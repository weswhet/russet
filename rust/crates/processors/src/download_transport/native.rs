//! Russet's HTTP engine: one shared Tokio runtime and pooled reqwest clients
//! over rustls. It performs the requests [`super::options`] admits and
//! reports results and failures the way curl does, so the downloader cannot
//! tell the backends apart.
use super::chunks::{self, ChunkPolicy};
use super::options::{NativeRequest, Trust, UserAgent};
use super::Headers;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Client, Method, Response, StatusCode, Version};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use url::Url;

/// curl's default `--max-redirs`.
const MAX_REDIRECTS: u32 = 50;
/// curl's default connection timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(300);
/// curl's own fallback when its version cannot be read.
const FALLBACK_USER_AGENT: &str = "curl/8.7.1";
/// Redirect statuses the downloader reports as `http_redirected`.
const REDIRECTS: [u16; 5] = [301, 302, 303, 307, 308];

/// A failure carrying curl's exit code and message, plus what the response
/// had revealed when it failed.
#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) code: i32,
    pub(crate) message: String,
    pub(crate) headers: Headers,
    pub(crate) effective: String,
}
impl Failure {
    pub(crate) fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            headers: Headers::new(),
            effective: String::new(),
        }
    }
    fn with(mut self, headers: &Headers, effective: &str) -> Self {
        self.headers = headers.clone();
        self.effective = effective.into();
        self
    }
    /// curl's standard error line for this failure.
    pub(crate) fn curl_message(&self) -> String {
        format!("curl: ({}) {}\n", self.code, self.message)
    }
}

/// A completed native transfer.
pub(crate) struct Transfer {
    pub(crate) headers: Headers,
    pub(crate) effective: String,
    pub(crate) detail: String,
}

/// A request with its clients ready, built before any network activity.
pub(crate) struct Prepared {
    pub(crate) request: NativeRequest,
    client: Client,
    ranged: Client,
    user_agent: Option<HeaderValue>,
}

fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .thread_name("russet-download")
                .enable_all()
                .build()
                .map_err(|e| format!("the download runtime is unavailable: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The fallback curl's version, read once from `curl --version`.
struct CurlIdentity {
    user_agent: HeaderValue,
    version: (u32, u32),
}
static IDENTITY: OnceLock<CurlIdentity> = OnceLock::new();

fn curl_identity(program: &OsStr) -> &'static CurlIdentity {
    IDENTITY.get_or_init(|| {
        let version = std::process::Command::new(program)
            .arg("--version")
            .output()
            .ok()
            .and_then(|output| {
                let text = String::from_utf8(output.stdout).ok()?;
                let version = text.strip_prefix("curl ")?.split_whitespace().next()?;
                version
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
                    .then(|| version.to_string())
            })
            .unwrap_or_else(|| FALLBACK_USER_AGENT["curl/".len()..].to_string());
        let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
        CurlIdentity {
            user_agent: HeaderValue::from_str(&format!("curl/{version}"))
                .unwrap_or_else(|_| HeaderValue::from_static(FALLBACK_USER_AGENT)),
            version: (parts.next().unwrap_or(0), parts.next().unwrap_or(0)),
        }
    })
}

/// curl's message for a body shorter than its Content-Length. curl 8.9.0
/// changed the wording.
pub(crate) fn short_body(remaining: u64) -> Failure {
    let modern = IDENTITY.get().is_some_and(|i| i.version >= (8, 9));
    Failure::new(
        18,
        if modern {
            format!("end of response with {remaining} bytes missing")
        } else {
            format!("transfer closed with {remaining} bytes remaining to read")
        },
    )
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ClientKey {
    trust: Option<[u8; 32]>,
    low_speed_time: Option<Duration>,
}

fn tls(
    trust: &Trust,
    http1_only: bool,
) -> Result<(rustls::ClientConfig, Option<[u8; 32]>), String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    let (mut config, digest) = match trust {
        Trust::Files { files, directories } => {
            let (roots, digest) = crate::downloader::trust::native_roots(files, directories)?;
            (
                builder.with_root_certificates(roots).with_no_client_auth(),
                Some(digest),
            )
        }
        Trust::Platform => {
            let verifier =
                rustls_platform_verifier::Verifier::new(provider).map_err(|e| e.to_string())?;
            (
                builder
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(verifier))
                    .with_no_client_auth(),
                None,
            )
        }
    };
    config.alpn_protocols = if http1_only {
        vec![b"http/1.1".to_vec()]
    } else {
        vec![b"h2".to_vec(), b"http/1.1".to_vec()]
    };
    Ok((config, digest))
}

fn build(
    config: rustls::ClientConfig,
    request: &NativeRequest,
    http1_only: bool,
) -> Result<Client, String> {
    let mut builder = Client::builder()
        .tls_backend_preconfigured(config)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(CONNECT_TIMEOUT)
        .http1_title_case_headers()
        .pool_idle_timeout(Duration::from_secs(60));
    if let Some(timeout) = request.low_speed_time {
        builder = builder.read_timeout(timeout);
    }
    if http1_only {
        builder = builder.http1_only();
    }
    builder.build().map_err(|e| e.to_string())
}

/// Builds or reuses the clients for a request. Clients are shared by every
/// request with the same trust roots and timing, so connections are pooled
/// across downloads; nothing else that affects a connection varies.
pub(crate) fn prepare(request: NativeRequest, program: &OsStr) -> Result<Prepared, String> {
    static CLIENTS: OnceLock<Mutex<HashMap<ClientKey, (Client, Client)>>> = OnceLock::new();
    let runtime = runtime()?;
    let _guard = runtime.enter();
    let (config, digest) = tls(&request.trust, false)?;
    let key = ClientKey {
        trust: digest,
        low_speed_time: request.low_speed_time,
    };
    let clients = CLIENTS.get_or_init(Default::default);
    let cached = clients
        .lock()
        .map_err(|e| e.to_string())?
        .get(&key)
        .cloned();
    let (client, ranged) = match cached {
        Some(pair) => pair,
        None => {
            let (ranged_config, _) = tls(&request.trust, true)?;
            let pair = (
                build(config, &request, false)?,
                build(ranged_config, &request, true)?,
            );
            clients
                .lock()
                .map_err(|e| e.to_string())?
                .insert(key, pair.clone());
            pair
        }
    };
    let identity = curl_identity(program);
    let user_agent = match &request.user_agent {
        UserAgent::Curl => Some(identity.user_agent.clone()),
        UserAgent::Custom(value) => Some(value.clone()),
        UserAgent::Omitted => None,
    };
    Ok(Prepared {
        request,
        client,
        ranged,
        user_agent,
    })
}

/// Runs a prepared request to completion on the shared runtime.
pub(crate) fn execute(prepared: &Prepared, policy: ChunkPolicy) -> Result<Transfer, Failure> {
    let runtime = runtime().map_err(|e| Failure::new(2, e))?;
    runtime.block_on(transfer(prepared, policy))
}

impl Prepared {
    fn headers(&self) -> HeaderMap {
        let mut map = HeaderMap::new();
        if let Some(agent) = &self.user_agent {
            map.insert(header::USER_AGENT, agent.clone());
        }
        for (name, value) in &self.request.headers {
            map.append(name.clone(), value.clone());
        }
        map
    }
    fn effective(&self, url: &Url) -> String {
        if self.request.write_out {
            url.as_str().into()
        } else {
            String::new()
        }
    }
}

/// The scheme, host, and port curl compares before sending credentials to a
/// redirect target.
fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
        && a.port_or_known_default() == b.port_or_known_default()
}

pub(crate) struct Sent {
    pub(crate) response: Response,
    pub(crate) url: Url,
    pub(crate) headers: HeaderMap,
    redirected: Option<String>,
}

/// Sends a request and follows redirects as `--location` does. Recipe
/// `Authorization` and `Cookie` headers go only to the original origin.
pub(crate) async fn send(
    client: &Client,
    method: Method,
    start: &Url,
    headers: &HeaderMap,
    follow: bool,
) -> Result<Sent, Failure> {
    let mut url = start.clone();
    let mut redirected = None;
    let mut followed = 0;
    loop {
        let mut hop = headers.clone();
        if !same_origin(start, &url) {
            hop.remove(header::AUTHORIZATION);
            hop.remove(header::COOKIE);
        }
        let response = client
            .request(method.clone(), url.clone())
            .headers(hop.clone())
            .send()
            .await
            .map_err(|e| request_failure(&e, &url))?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).trim().to_string());
        if REDIRECTS.contains(&status) {
            redirected = location.clone();
        }
        match location {
            Some(location) if follow && (300..400).contains(&status) && status != 304 => {
                followed += 1;
                if followed > MAX_REDIRECTS {
                    return Err(Failure::new(
                        47,
                        format!("Maximum ({MAX_REDIRECTS}) redirects followed"),
                    ));
                }
                let next = url.join(&location).map_err(|_| {
                    Failure::new(3, "URL rejected: Malformed input to a URL function")
                })?;
                if !matches!(next.scheme(), "http" | "https") {
                    return Err(Failure::new(
                        1,
                        format!("Protocol \"{}\" not supported", next.scheme()),
                    ));
                }
                url = next;
            }
            _ => {
                return Ok(Sent {
                    response,
                    url,
                    headers: hop,
                    redirected,
                })
            }
        }
    }
}

/// The response as the downloader reads curl's header dump: the final
/// response's headers, lowercased, last value winning, with its status.
pub(crate) fn capture(sent: &Sent) -> Headers {
    let response = &sent.response;
    let mut headers = Headers::new();
    headers.insert("http_result_code".into(), response.status().as_str().into());
    // curl prints HTTP/2 and HTTP/3 status lines without a reason phrase.
    let reason = match response.version() {
        Version::HTTP_2 | Version::HTTP_3 => String::new(),
        _ => response
            .extensions()
            .get::<hyper::ext::ReasonPhrase>()
            .map(|r| String::from_utf8_lossy(r.as_bytes()).into_owned())
            .or_else(|| response.status().canonical_reason().map(Into::into))
            .unwrap_or_default(),
    };
    headers.insert("http_result_description".into(), reason);
    for (name, value) in response.headers() {
        headers.insert(
            name.as_str().to_ascii_lowercase(),
            String::from_utf8_lossy(value.as_bytes()).trim().to_string(),
        );
    }
    if let Some(location) = &sent.redirected {
        headers.insert("http_redirected".into(), location.clone());
    }
    headers
}

fn source_chain<'a>(
    error: &'a (dyn std::error::Error + 'static),
) -> Vec<&'a (dyn std::error::Error + 'static)> {
    let mut chain = vec![error];
    let mut current = error.source();
    while let Some(next) = current {
        chain.push(next);
        current = next.source();
    }
    chain
}

fn tls_failure(error: &(dyn std::error::Error + 'static)) -> Option<Failure> {
    for item in source_chain(error) {
        // Connectors wrap TLS errors in nested I/O errors, which report
        // their payload through get_ref rather than source.
        let mut current: &(dyn std::error::Error + 'static) = item;
        while let Some(inner) = current
            .downcast_ref::<std::io::Error>()
            .and_then(|io| io.get_ref())
        {
            current = inner;
        }
        let tls = current.downcast_ref::<rustls::Error>();
        if let Some(tls) = tls {
            return Some(match tls {
                rustls::Error::InvalidCertificate(_) => {
                    Failure::new(60, format!("SSL certificate problem: {tls}"))
                }
                _ => Failure::new(35, format!("TLS connect error: {tls}")),
            });
        }
    }
    None
}

fn request_failure(error: &reqwest::Error, url: &Url) -> Failure {
    let host = url.host_str().unwrap_or_default();
    if let Some(failure) = tls_failure(error) {
        return failure;
    }
    if error.is_timeout() {
        return Failure::new(28, slow_message());
    }
    let text = source_chain(error)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ");
    if error.is_connect() {
        if text.contains("dns error") || text.contains("failed to lookup") {
            return Failure::new(6, format!("Could not resolve host: {host}"));
        }
        return Failure::new(
            7,
            format!(
                "Failed to connect to {host} port {}: Couldn't connect to server",
                url.port_or_known_default().unwrap_or_default()
            ),
        );
    }
    Failure::new(52, "Empty reply from server")
}

pub(crate) fn slow_message() -> String {
    "Operation too slow. Less than 1 bytes/sec transferred the last 30 seconds".into()
}

/// Classifies a failure while reading a body, as curl would report it.
pub(crate) fn body_failure(
    error: &reqwest::Error,
    received: u64,
    expected: Option<u64>,
    chunked: bool,
) -> Failure {
    if error.is_timeout() {
        return Failure::new(28, slow_message());
    }
    match expected {
        Some(length) if received < length => short_body(length - received),
        _ if chunked => Failure::new(18, "transfer closed with outstanding read data remaining"),
        _ => Failure::new(56, "Failure when receiving data from the peer"),
    }
}

pub(crate) fn write_failure() -> Failure {
    Failure::new(23, "Failure writing output to destination")
}

pub(crate) fn content_length(headers: &Headers) -> Option<u64> {
    headers.get("content-length")?.parse().ok()
}
pub(crate) fn is_chunked(headers: &Headers) -> bool {
    headers
        .get("transfer-encoding")
        .is_some_and(|s| s.to_ascii_lowercase().contains("chunked"))
}

/// Streams a body to the output file. The file is opened, and truncated,
/// only when the first bytes arrive, as curl does.
pub(crate) async fn stream(
    mut response: Response,
    path: &Path,
    headers: &Headers,
) -> Result<u64, Failure> {
    let expected = content_length(headers);
    let mut file: Option<File> = None;
    let mut received = 0u64;
    loop {
        match response.chunk().await {
            Ok(Some(bytes)) => {
                if file.is_none() {
                    file = Some(File::create(path).map_err(|_| write_failure())?);
                }
                if let Some(file) = file.as_mut() {
                    file.write_all(&bytes).map_err(|_| write_failure())?;
                }
                received += bytes.len() as u64;
            }
            Ok(None) => break,
            Err(error) => {
                return Err(body_failure(
                    &error,
                    received,
                    expected,
                    is_chunked(headers),
                ))
            }
        }
    }
    match expected {
        Some(length) if received < length => Err(short_body(length - received)),
        _ => Ok(received),
    }
}

async fn transfer(prepared: &Prepared, policy: ChunkPolicy) -> Result<Transfer, Failure> {
    let request = &prepared.request;
    let method = if request.head {
        Method::HEAD
    } else {
        Method::GET
    };
    let sent = send(
        &prepared.client,
        method,
        &request.url,
        &prepared.headers(),
        request.follow,
    )
    .await?;
    let headers = capture(&sent);
    let effective = prepared.effective(&sent.url);
    let status = sent.response.status();
    if request.fail && status.as_u16() >= 400 {
        return Err(Failure::new(
            22,
            format!("The requested URL returned error: {}", status.as_u16()),
        )
        .with(&headers, &effective));
    }
    let Some(output) = request.output.as_deref().filter(|_| !request.head) else {
        return Ok(Transfer {
            headers,
            effective,
            detail: "headers only".into(),
        });
    };
    if request.chunkable && status == StatusCode::OK {
        if let Some(plan) = chunks::plan(&headers, &policy) {
            let outcome = chunks::download(&prepared.ranged, sent, plan, output, &policy).await;
            return match outcome {
                Ok(detail) => Ok(Transfer {
                    headers,
                    effective,
                    detail,
                }),
                Err(chunks::Error::Failed(failure)) => Err(failure.with(&headers, &effective)),
                Err(chunks::Error::Restart(reason)) => restart(prepared, output, &reason).await,
            };
        }
    }
    stream(sent.response, output, &headers)
        .await
        .map_err(|failure| failure.with(&headers, &effective))?;
    Ok(Transfer {
        headers,
        effective,
        detail: "sequential".into(),
    })
}

/// After a failed parallel attempt whose bytes cannot be reused, downloads
/// the resource again in one stream. The cache validators are left out:
/// the first response already established that the resource changed.
async fn restart(prepared: &Prepared, output: &Path, reason: &str) -> Result<Transfer, Failure> {
    let request = &prepared.request;
    let mut headers = prepared.headers();
    headers.remove(header::IF_NONE_MATCH);
    headers.remove(header::IF_MODIFIED_SINCE);
    let sent = send(
        &prepared.client,
        Method::GET,
        &request.url,
        &headers,
        request.follow,
    )
    .await?;
    let captured = capture(&sent);
    let effective = prepared.effective(&sent.url);
    let status = sent.response.status().as_u16();
    if request.fail && status >= 400 {
        return Err(
            Failure::new(22, format!("The requested URL returned error: {status}"))
                .with(&captured, &effective),
        );
    }
    File::create(output).map_err(|_| write_failure())?;
    stream(sent.response, output, &captured)
        .await
        .map_err(|failure| failure.with(&captured, &effective))?;
    Ok(Transfer {
        headers: captured,
        effective,
        detail: format!("sequential after parallel attempt ({reason})"),
    })
}
