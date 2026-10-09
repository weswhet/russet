//! Parallel range transfers for large resources.
//!
//! The original response keeps streaming from the start of the file while
//! range workers take fixed-size chunks from the end. Each side claims chunks
//! under one lock, so no byte range has two writers. Workers send `Range`
//! and `If-Range` with the pinned strong ETag and accept only an exact `206`
//! for the same representation. On any anomaly the workers are cancelled and
//! joined, and the original response takes over every chunk it has not
//! passed. If it has already stopped, the transfer restarts as one stream.
use super::native::{self, Failure, Sent};
use super::Headers;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Client, Response, StatusCode};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Concurrent range requests across all transfers.
const MAX_REQUESTS: usize = 8;
/// Concurrent range requests to one origin. With the original response this
/// allows four streams per origin.
const MAX_REQUESTS_PER_ORIGIN: usize = 3;

/// When and how a transfer is split.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChunkPolicy {
    /// The smallest resource that is split.
    pub(crate) min_size: u64,
    pub(crate) chunk_size: u64,
    /// Range workers alongside the original response.
    pub(crate) workers: usize,
}
impl Default for ChunkPolicy {
    fn default() -> Self {
        Self {
            min_size: 64 << 20,
            chunk_size: 8 << 20,
            workers: 3,
        }
    }
}

/// A validated split of one representation.
pub(crate) struct Plan {
    length: u64,
    chunk: u64,
    count: u64,
    etag: HeaderValue,
}
impl Plan {
    fn range(&self, index: u64) -> (u64, u64) {
        let start = index * self.chunk;
        (start, (start + self.chunk).min(self.length))
    }
}

/// Decides from the first response whether splitting is safe: a known
/// length large enough to matter, a strong ETag to pin the representation,
/// and no content coding.
pub(crate) fn plan(headers: &Headers, policy: &ChunkPolicy) -> Option<Plan> {
    if policy.workers == 0 || policy.chunk_size == 0 {
        return None;
    }
    let length = native::content_length(headers)?;
    if length < policy.min_size || length <= policy.chunk_size || native::is_chunked(headers) {
        return None;
    }
    if headers
        .get("content-encoding")
        .is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
        || headers
            .get("accept-ranges")
            .is_some_and(|v| v.eq_ignore_ascii_case("none"))
    {
        return None;
    }
    let etag = headers.get("etag")?;
    if !etag.starts_with('"') || etag.len() < 2 || !etag.ends_with('"') {
        return None;
    }
    Some(Plan {
        length,
        chunk: policy.chunk_size,
        count: length.div_ceil(policy.chunk_size),
        etag: HeaderValue::from_str(etag).ok()?,
    })
}

pub(crate) enum Error {
    /// The transfer failed as a single stream would have.
    Failed(Failure),
    /// The parallel attempt was abandoned after the original stream ended.
    Restart(String),
}

/// Which chunks each side owns. The original stream owns `[0, front)` and
/// workers own `[back, count)`; the gap is unclaimed.
struct Claims {
    front: u64,
    back: u64,
}

#[cfg(unix)]
fn write_at(file: &File, bytes: &[u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, bytes, offset)
}
#[cfg(windows)]
fn write_at(file: &File, mut bytes: &[u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !bytes.is_empty() {
        let written = file.seek_write(bytes, offset)?;
        if written == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        bytes = &bytes[written..];
        offset += written as u64;
    }
    Ok(())
}

fn permits(origin: &str, wanted: usize) -> Vec<(OwnedSemaphorePermit, OwnedSemaphorePermit)> {
    static GLOBAL: OnceLock<Arc<Semaphore>> = OnceLock::new();
    static ORIGINS: OnceLock<Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();
    let global = GLOBAL.get_or_init(|| Arc::new(Semaphore::new(MAX_REQUESTS)));
    let Ok(mut origins) = ORIGINS.get_or_init(Default::default).lock() else {
        return Vec::new();
    };
    let local = origins
        .entry(origin.to_string())
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_REQUESTS_PER_ORIGIN)))
        .clone();
    let mut granted = Vec::new();
    while granted.len() < wanted {
        let (Ok(a), Ok(b)) = (
            global.clone().try_acquire_owned(),
            local.clone().try_acquire_owned(),
        ) else {
            break;
        };
        granted.push((a, b));
    }
    granted
}

/// The original response, writing its chunks in order until it reaches one
/// a worker owns. Returns the bytes written from offset zero and whether it
/// reached the end.
async fn original(
    mut response: Response,
    file: Arc<File>,
    claims: Arc<Mutex<Claims>>,
    length: u64,
    chunk: u64,
) -> Result<(u64, bool), (Failure, u64)> {
    let mut position = 0u64;
    let mut limit = chunk.min(length);
    loop {
        let bytes = match response.chunk().await {
            Ok(Some(bytes)) => bytes,
            Ok(None) if position == length => return Ok((position, true)),
            Ok(None) => {
                let failure = native::short_body(length - position);
                return Err((failure, position));
            }
            Err(error) => {
                let failure = native::body_failure(&error, position, Some(length), false);
                return Err((failure, position));
            }
        };
        let mut data = &bytes[..];
        while !data.is_empty() {
            if position == limit {
                let mut claims = claims
                    .lock()
                    .map_err(|_| (native::write_failure(), position))?;
                if position < length && claims.front < claims.back {
                    claims.front += 1;
                    limit = (claims.front * chunk).min(length);
                } else {
                    return Ok((position, position == length));
                }
            }
            let count = data.len().min((limit - position) as usize);
            write_at(&file, &data[..count], position)
                .map_err(|_| (native::write_failure(), position))?;
            position += count as u64;
            data = &data[count..];
        }
    }
}

/// A range worker. It takes chunks from the end until none remain, and
/// returns a reason for the first anomaly it sees.
#[allow(clippy::too_many_arguments)]
async fn worker(
    client: Client,
    url: url::Url,
    headers: HeaderMap,
    plan: Arc<Plan>,
    claims: Arc<Mutex<Claims>>,
    done: Arc<Mutex<Vec<bool>>>,
    file: Arc<File>,
    _permits: (OwnedSemaphorePermit, OwnedSemaphorePermit),
) -> Result<(), String> {
    loop {
        let index = {
            let mut claims = claims.lock().map_err(|_| "a poisoned lock")?;
            if claims.back <= claims.front {
                return Ok(());
            }
            claims.back -= 1;
            claims.back
        };
        let (start, end) = plan.range(index);
        let mut response = client
            .get(url.clone())
            .headers(headers.clone())
            .header(header::RANGE, format!("bytes={start}-{}", end - 1))
            .header(header::IF_RANGE, plan.etag.clone())
            .send()
            .await
            .map_err(|_| "a range request failed")?;
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(format!(
                "a range request returned {}",
                response.status().as_u16()
            ));
        }
        let field = |name| {
            response
                .headers()
                .get(name)
                .map(|v: &HeaderValue| v.as_bytes().to_vec())
        };
        let expected_range = format!("bytes {start}-{}/{}", end - 1, plan.length);
        if field(header::CONTENT_RANGE).as_deref() != Some(expected_range.as_bytes()) {
            return Err("a mismatched Content-Range".into());
        }
        if field(header::ETAG).as_deref() != Some(plan.etag.as_bytes()) {
            return Err("a changed ETag".into());
        }
        if field(header::CONTENT_ENCODING).is_some_and(|v| !v.eq_ignore_ascii_case(b"identity")) {
            return Err("a coded range".into());
        }
        if response
            .content_length()
            .is_some_and(|length| length != end - start)
        {
            return Err("a mismatched range length".into());
        }
        let mut position = start;
        while let Some(bytes) = response.chunk().await.map_err(|_| "a short range")? {
            if bytes.len() as u64 > end - position {
                return Err("a range longer than requested".into());
            }
            write_at(&file, &bytes, position).map_err(|_| "a failed write")?;
            position += bytes.len() as u64;
        }
        if position != end {
            return Err("a short range".into());
        }
        done.lock().map_err(|_| "a poisoned lock")?[index as usize] = true;
    }
}

/// Splits a transfer whose first response is `sent`. Returns a description
/// of how the transfer completed.
pub(crate) async fn download(
    client: &Client,
    sent: Sent,
    plan: Plan,
    path: &Path,
    policy: &ChunkPolicy,
) -> Result<String, Error> {
    let file = Arc::new(File::create(path).map_err(|_| Error::Failed(native::write_failure()))?);
    let origin = sent.url.origin().ascii_serialization();
    let granted = permits(&origin, policy.workers.min((plan.count - 1) as usize));
    let mut headers = sent.headers.clone();
    headers.remove(header::IF_NONE_MATCH);
    headers.remove(header::IF_MODIFIED_SINCE);
    let plan = Arc::new(plan);
    let claims = Arc::new(Mutex::new(Claims {
        front: 1,
        back: plan.count,
    }));
    let done = Arc::new(Mutex::new(vec![false; plan.count as usize]));
    let workers = granted.len();
    let mut set = tokio::task::JoinSet::new();
    for permit in granted {
        set.spawn(worker(
            client.clone(),
            sent.url.clone(),
            headers.clone(),
            plan.clone(),
            claims.clone(),
            done.clone(),
            file.clone(),
            permit,
        ));
    }
    let mut first = tokio::spawn(original(
        sent.response,
        file.clone(),
        claims.clone(),
        plan.length,
        plan.chunk,
    ));
    let mut result = None;
    let mut anomaly = None;
    loop {
        tokio::select! {
            outcome = &mut first, if result.is_none() => {
                let outcome = outcome.unwrap_or_else(|_| Err((native::write_failure(), 0)));
                if let Err((failure, position)) = outcome {
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                    // Keep only what one stream would have written.
                    let _ = file.set_len(position);
                    return Err(Error::Failed(failure));
                }
                result = Some(outcome);
            }
            joined = set.join_next(), if !set.is_empty() => {
                let failed = match joined {
                    Some(Ok(Ok(()))) | None => None,
                    Some(Ok(Err(reason))) => Some(reason),
                    Some(Err(_)) => Some("a worker stopped".to_string()),
                };
                if let Some(reason) = failed {
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                    // Every worker has stopped writing. Hand all chunks the
                    // original stream has not passed back to it.
                    if let Ok(mut claims) = claims.lock() {
                        claims.back = plan.count;
                    }
                    anomaly = Some(reason);
                }
            }
            else => break,
        }
        if result.is_some() && set.is_empty() {
            break;
        }
    }
    let Some(Ok((position, complete))) = result else {
        return Err(Error::Restart("the original stream ended".into()));
    };
    if complete {
        return Ok(match anomaly {
            Some(reason) => format!("sequential after {reason}"),
            None => format!("parallel, {} chunks, {workers} range workers", plan.count),
        });
    }
    if let Some(reason) = anomaly {
        return Err(Error::Restart(reason));
    }
    let first_worker_chunk = position.div_ceil(plan.chunk);
    let covered = done
        .lock()
        .map(|done| done[first_worker_chunk as usize..].iter().all(|d| *d))
        .unwrap_or(false);
    if position % plan.chunk != 0 && position != plan.length || !covered {
        return Err(Error::Restart("incomplete coverage".into()));
    }
    Ok(format!(
        "parallel, {} chunks, {workers} range workers",
        plan.count
    ))
}
