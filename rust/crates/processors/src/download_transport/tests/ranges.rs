//! Native-only behavior: parallel ranges, their fallbacks, and TLS trust.
use super::super::chunks::ChunkPolicy;
use super::super::{run_with, Headers, Policy, TransportFailure};
use super::fixture::{content, serve, tls_identity, Reply, Request, Server};
use plist::{Dictionary, Value};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

const LENGTH: usize = 256 << 10;
const ETAG: &str = "\"strong-1\"";

fn small() -> ChunkPolicy {
    ChunkPolicy {
        min_size: 64 << 10,
        chunk_size: 32 << 10,
        workers: 3,
    }
}

fn native() -> Policy {
    Policy { native: Ok(()) }
}

/// How the fixture answers range requests.
#[derive(Clone, Copy, PartialEq)]
enum Ranges {
    Honored,
    Ignored,
    /// The representation changed: If-Range no longer matches.
    Changed,
    /// The last chunk arrives late and short.
    LateShortLastChunk,
    WrongContentRange,
}

fn ranged(
    data: Arc<Vec<u8>>,
    etag: &'static str,
    ranges: Ranges,
    pace: Duration,
) -> impl Fn(&Request) -> Reply {
    move |request| {
        let full = |body: &[u8], etag: &str| {
            Reply::new("200 OK")
                .header("ETag", etag)
                .header("Accept-Ranges", "bytes")
                .body(body.to_vec())
                .pace(pace)
        };
        let Some(range) = request.header("Range") else {
            return full(&data, etag);
        };
        let (start, end) = range
            .strip_prefix("bytes=")
            .and_then(|r| r.split_once('-'))
            .map(|(s, e)| (s.parse::<usize>().unwrap(), e.parse::<usize>().unwrap()))
            .unwrap();
        match ranges {
            Ranges::Ignored => full(&data, etag),
            Ranges::Changed => full(&content(LENGTH + 7), "\"strong-2\""),
            _ if request.header("If-Range") != Some(etag) => full(&data, etag),
            _ => {
                let mut reply = Reply::new("206 Partial Content")
                    .header("ETag", etag)
                    .header(
                        "Content-Range",
                        if ranges == Ranges::WrongContentRange {
                            format!("bytes {}-{end}/{}", start + 1, data.len())
                        } else {
                            format!("bytes {start}-{end}/{}", data.len())
                        },
                    )
                    .body(data[start..=end].to_vec());
                if ranges == Ranges::LateShortLastChunk && end + 1 == data.len() {
                    reply = reply.delay(Duration::from_millis(400)).cut(10);
                }
                reply
            }
        }
    }
}

struct Outcome {
    result: Result<(Headers, String), TransportFailure>,
    bytes: Vec<u8>,
    requests: Vec<Request>,
}

fn download(server: &Server, python: bool, policy: ChunkPolicy) -> Outcome {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("download");
    std::fs::write(&output, b"").unwrap();
    let env = Dictionary::from_iter([(
        "url".to_string(),
        Value::String(format!("{}/large.pkg", server.base)),
    )]);
    let operation = [
        "--fail".into(),
        "--output".into(),
        output.clone().into_os_string(),
    ];
    let (mut command, _trust) = crate::downloader::command(&env, python, &operation).unwrap();
    if python {
        command.args(["--write-out", super::super::options::EFFECTIVE_URL_FORMAT]);
    }
    let result = run_with(command, python, &native(), policy);
    Outcome {
        result,
        bytes: std::fs::read(&output).unwrap_or_default(),
        requests: server.requests(),
    }
}

fn range_requests(requests: &[Request]) -> usize {
    requests
        .iter()
        .filter(|r| r.header("Range").is_some())
        .count()
}

#[test]
fn parallel_ranges_reassemble_the_exact_representation() {
    let data = Arc::new(content(LENGTH));
    for python in [false, true] {
        let server = serve(
            ranged(
                data.clone(),
                ETAG,
                Ranges::Honored,
                Duration::from_millis(15),
            ),
            None,
        );
        let outcome = download(&server, python, small());
        let (headers, _) = outcome.result.unwrap();
        assert_eq!(headers["etag"], ETAG);
        assert_eq!(headers["content-length"], LENGTH.to_string());
        assert!(outcome.bytes == *data, "python={python}");
        let ranges = outcome
            .requests
            .iter()
            .filter(|r| r.header("Range").is_some())
            .collect::<Vec<_>>();
        assert!(!ranges.is_empty(), "python={python}");
        assert!(ranges.iter().all(|r| r.header("If-Range") == Some(ETAG)
            && r.header("If-None-Match").is_none()
            && r.header("User-Agent").is_some()));
    }
}

#[test]
fn small_files_and_weak_validators_use_one_stream() {
    let data = Arc::new(content(LENGTH));
    for (etag, policy) in [
        ("W/\"weak\"", small()),
        (
            ETAG,
            ChunkPolicy {
                min_size: LENGTH as u64 + 1,
                ..small()
            },
        ),
    ] {
        let server = serve(
            ranged(data.clone(), etag, Ranges::Honored, Duration::ZERO),
            None,
        );
        let outcome = download(&server, false, policy);
        outcome.result.unwrap();
        assert!(outcome.bytes == *data);
        assert_eq!(range_requests(&outcome.requests), 0);
    }
}

#[test]
fn range_anomalies_fall_back_to_one_consistent_stream() {
    let data = Arc::new(content(LENGTH));
    for ranges in [Ranges::Ignored, Ranges::Changed, Ranges::WrongContentRange] {
        let server = serve(
            ranged(data.clone(), ETAG, ranges, Duration::from_millis(15)),
            None,
        );
        let outcome = download(&server, false, small());
        let (headers, _) = outcome.result.unwrap();
        assert_eq!(headers["etag"], ETAG);
        // Bytes come only from the original representation, never mixed.
        assert!(outcome.bytes == *data);
        assert!(range_requests(&outcome.requests) > 0);
    }
}

#[test]
fn a_late_worker_failure_restarts_one_clean_stream() {
    let data = Arc::new(content(LENGTH));
    let server = serve(
        ranged(
            data.clone(),
            ETAG,
            Ranges::LateShortLastChunk,
            Duration::ZERO,
        ),
        None,
    );
    let outcome = download(&server, false, small());
    outcome.result.unwrap();
    assert!(outcome.bytes == *data);
    let whole = outcome
        .requests
        .iter()
        .filter(|r| r.header("Range").is_none())
        .collect::<Vec<_>>();
    assert_eq!(whole.len(), 2);
    assert!(whole[1].header("If-None-Match").is_none());
}

#[test]
fn a_short_original_stream_fails_like_one_stream() {
    let data = Arc::new(content(LENGTH));
    let handler = {
        let data = data.clone();
        move |request: &Request| {
            if request.header("Range").is_some() {
                return ranged(data.clone(), ETAG, Ranges::Honored, Duration::ZERO)(request);
            }
            Reply::new("200 OK")
                .header("ETag", ETAG)
                .body(data.to_vec())
                .cut(16 << 10)
        }
    };
    let server = serve(handler.clone(), None);
    let error = download(&server, false, small()).result.unwrap_err();
    assert_eq!(
        error.failure.message,
        super::super::native::short_body((LENGTH - (16 << 10)) as u64).curl_message()
    );
    // urllib accepts a body shorter than Content-Length; the file then holds
    // exactly the bytes the original stream delivered.
    let server = serve(handler, None);
    let outcome = download(&server, true, small());
    outcome.result.unwrap();
    assert!(outcome.bytes == data[..16 << 10]);
}

fn https_command(
    server: &Server,
    python: bool,
    ca: &std::path::Path,
    output: &std::path::Path,
) -> Command {
    let env = Dictionary::from_iter([(
        "url".to_string(),
        Value::String(format!("{}/secure.pkg", server.base)),
    )]);
    let operation = [
        "--fail".into(),
        "--output".into(),
        output.as_os_str().to_owned(),
    ];
    let (generated, _trust) = crate::downloader::command(&env, python, &operation).unwrap();
    let mut command = Command::new(generated.get_program());
    let mut arguments = generated
        .get_args()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if python {
        let index = arguments.iter().position(|a| a == "--cacert").unwrap();
        arguments[index + 1] = ca.as_os_str().to_owned();
        arguments.push("--write-out".into());
        arguments.push(super::super::options::EFFECTIVE_URL_FORMAT.into());
    } else {
        command.env("SSL_CERT_FILE", ca);
    }
    command.args(arguments);
    command
}

#[test]
fn tls_uses_each_processor_trust_source() {
    let (config, pem) = tls_identity();
    let (_, other) = tls_identity();
    let directory = tempfile::tempdir().unwrap();
    let trusted = directory.path().join("trusted.pem");
    let untrusted = directory.path().join("untrusted.pem");
    std::fs::write(&trusted, pem).unwrap();
    std::fs::write(&untrusted, other).unwrap();
    let output = directory.path().join("download");
    for python in [false, true] {
        let server = serve(
            |_| Reply::new("200 OK").body("secure"),
            Some(config.clone()),
        );
        let command = https_command(&server, python, &trusted, &output);
        let (headers, effective) = run_with(command, python, &native(), small()).unwrap();
        assert_eq!(headers["http_result_code"], "200");
        assert_eq!(std::fs::read(&output).unwrap(), b"secure");
        if python {
            assert_eq!(effective, format!("{}/secure.pkg", server.base));
        }
        let command = https_command(&server, python, &untrusted, &output);
        let error = run_with(command, python, &native(), small()).unwrap_err();
        assert!(
            error
                .failure
                .message
                .starts_with("curl: (60) SSL certificate problem"),
            "{}",
            error.failure.message
        );
    }
}
