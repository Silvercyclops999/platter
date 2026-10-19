//! A symphonia `MediaSource` over HTTP range requests.
//!
//! A background tokio task fills an in-memory byte window ahead of the read
//! position. Reads block (on the decoder thread) until bytes arrive. A
//! dropped connection is resumed from the last received byte with a new
//! `Range` request, so the decoder never sees a gap. Seeks inside the window
//! are free; far seeks restart the fetch at the new offset; reads near the end
//! of the file (trailing tags, MP4 `moov`) are served from a separately
//! fetched tail so probing doesn't throw away the window.
//!
//! Every body transfer goes through the player's [`TransferGate`] (one
//! transfer at a time). A preempted fetch drops its connection and resumes
//! by `Range` once it gets the gate back; that is not counted as a failure.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, HeaderMap, HeaderName, HeaderValue, RANGE};
use symphonia::core::io::MediaSource;
use tokio::runtime::Handle;
use tokio::sync::Notify;

use super::gate::{PriorityCell, TransferGate};
use super::source::StreamSource;
use crate::error::{AppError, AppResult};

/// Tuning for [`HttpSource`].
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Stop fetching when this many bytes are buffered ahead of the reader
    /// (resume below three quarters of it).
    pub max_ahead: usize,
    /// Keep at most this many bytes behind the reader (for cheap back-seeks).
    pub keep_behind: usize,
    /// Give up after failing this long without receiving a byte.
    pub retry_window: Duration,
    /// First retry delay; doubles up to `max_backoff`.
    pub min_backoff: Duration,
    pub max_backoff: Duration,
    /// How long `open` waits for the first response (time spent waiting
    /// for the transfer gate doesn't count).
    pub open_timeout: Duration,
    /// Seeks less than this far past the buffered end just wait for the data.
    pub jump_threshold: u64,
    /// Size of the separately fetched file tail.
    pub tail_bytes: u64,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            // 32 MB is about 4 minutes of CD-quality FLAC and 85 s of
            // 24/96, well past the 30 s retry window. Behind is trimmed at
            // twice `keep_behind`, so one stream holds at most 48 MB (was 80)
            // and the playing plus preloaded streams at most 96 MB.
            max_ahead: 32 * 1024 * 1024,
            keep_behind: 8 * 1024 * 1024,
            retry_window: Duration::from_secs(30),
            min_backoff: Duration::from_millis(200),
            max_backoff: Duration::from_secs(3),
            open_timeout: Duration::from_secs(15),
            jump_threshold: 512 * 1024,
            tail_bytes: 256 * 1024,
        }
    }
}

/// Builds the HTTP client used for streaming and pre-caching.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        // A pulled cable/Wi-Fi otherwise stalls a body read forever.
        .read_timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
enum Fatal {
    NotFound,
    Auth(String),
    Network(String),
    Server(String),
}

impl Fatal {
    fn to_app(&self) -> AppError {
        match self {
            Fatal::NotFound => AppError::NotFound("file unavailable".into()),
            Fatal::Auth(m) => AppError::Auth(m.clone()),
            Fatal::Network(m) => AppError::Network(m.clone()),
            Fatal::Server(m) => AppError::Server(m.clone()),
        }
    }
}

#[derive(Default)]
struct State {
    /// The window holds bytes `[base, base + data.len())`.
    base: u64,
    data: Vec<u8>,
    read_pos: u64,
    total: Option<u64>,
    /// The fetch for the current generation reached the end of the file.
    eof: bool,
    /// First response (headers) arrived.
    opened: bool,
    fatal: Option<Fatal>,
    /// Bumped when the reader wants the fetch restarted at `base`.
    generation: u64,
    closed: bool,
    tail: Option<(u64, Vec<u8>)>,
    tail_requested: bool,
    tail_failed: bool,
    /// Reconnects after a dropped connection (diagnostics / tests).
    resumes: u32,
    /// The fetch task doesn't currently need the network (complete, or
    /// paused with the read-ahead window full).
    idle: bool,
    /// The fetch task is waiting for the transfer gate.
    gate_wait: bool,
}

impl State {
    fn end(&self) -> u64 {
        self.base + self.data.len() as u64
    }
    fn complete(&self) -> bool {
        self.base == 0 && self.total == Some(self.data.len() as u64)
    }
}

struct Shared {
    st: Mutex<State>,
    cv: Condvar,
    /// Wakes the fetch task (reader progressed, restart, close).
    wake: Notify,
    /// Tells an in-flight request to drop (close, restart elsewhere).
    abort: Notify,
}

impl Shared {
    fn stats(&self) -> BufferStats {
        let st = self.st.lock();
        BufferStats {
            read_pos: st.read_pos,
            buffered_end: st.end(),
            total: st.total,
            complete: st.complete(),
            resumes: st.resumes,
            idle: st.idle && st.fatal.is_none(),
        }
    }
}

/// Seekable, read-ahead HTTP byte source. Must be read from a thread that is
/// not a tokio worker (reads block).
pub struct HttpSource {
    shared: Arc<Shared>,
    pos: u64,
    handle: Handle,
    client: reqwest::Client,
    src: Arc<StreamSource>,
    cfg: HttpConfig,
    gate: Arc<TransferGate>,
    prio: PriorityCell,
}

/// Snapshot of what's buffered, for `bufferedMs` and pre-cache gating.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferStats {
    pub read_pos: u64,
    pub buffered_end: u64,
    pub total: Option<u64>,
    pub complete: bool,
    pub resumes: u32,
    /// Not transferring and not wanting to (complete or window full).
    pub idle: bool,
}

impl HttpSource {
    /// Starts fetching and waits for the first response so a 404 or auth
    /// error surfaces here. Blocking; call from a non-runtime thread.
    /// `prio` may be changed later (e.g. promoted when the track becomes
    /// audible).
    pub fn open(
        handle: Handle,
        client: reqwest::Client,
        src: StreamSource,
        cfg: HttpConfig,
        gate: Arc<TransferGate>,
        prio: PriorityCell,
    ) -> AppResult<Self> {
        let shared = Arc::new(Shared {
            st: Mutex::new(State {
                total: src.size,
                ..Default::default()
            }),
            cv: Condvar::new(),
            wake: Notify::new(),
            abort: Notify::new(),
        });
        let src = Arc::new(src);
        handle.spawn(fetch_task(
            shared.clone(),
            client.clone(),
            src.clone(),
            cfg.clone(),
            gate.clone(),
            prio.clone(),
        ));
        let mut deadline = Instant::now() + cfg.open_timeout;
        {
            let mut st = shared.st.lock();
            loop {
                if let Some(f) = &st.fatal {
                    let e = f.to_app();
                    st.closed = true;
                    drop(st);
                    shared.wake.notify_one();
                    shared.abort.notify_one();
                    return Err(e);
                }
                if st.opened {
                    break;
                }
                if st.gate_wait {
                    // Waiting behind another transfer isn't the server's fault.
                    deadline = Instant::now() + cfg.open_timeout;
                }
                if shared
                    .cv
                    .wait_for(&mut st, Duration::from_millis(100))
                    .timed_out()
                    && !st.opened
                    && !st.gate_wait
                    && Instant::now() >= deadline
                {
                    st.closed = true;
                    drop(st);
                    shared.wake.notify_one();
                    shared.abort.notify_one();
                    return Err(AppError::Network("stream did not respond".into()));
                }
            }
        }
        Ok(Self {
            shared,
            pos: 0,
            handle,
            client,
            src,
            cfg,
            gate,
            prio,
        })
    }

    pub fn stats(&self) -> BufferStats {
        self.shared.stats()
    }

    /// Shared stats handle that stays valid after the source moves into the
    /// decoder.
    pub fn stats_handle(&self) -> HttpStats {
        HttpStats(self.shared.clone())
    }

    /// The whole file, if it was fetched contiguously from byte 0.
    pub fn completed_bytes(&self) -> Option<Vec<u8>> {
        self.stats_handle().completed_bytes()
    }

    fn request_tail(&self, st: &mut State, total: u64) {
        if st.tail_requested {
            return;
        }
        st.tail_requested = true;
        let start = total.saturating_sub(self.cfg.tail_bytes);
        let shared = self.shared.clone();
        let client = self.client.clone();
        let src = self.src.clone();
        let gate = self.gate.clone();
        let prio = self.prio.clone();
        self.handle.spawn(async move {
            let mut attempts = 0;
            let res = loop {
                if shared.st.lock().closed {
                    return;
                }
                let permit = gate.acquire(&prio).await;
                tokio::select! {
                    r = fetch_range(&client, &src, start, Some(total - 1)) => {
                        attempts += 1;
                        match r {
                            Ok(b) => break Ok(b),
                            Err(e) if attempts >= 3 => break Err(e),
                            Err(_) => {
                                drop(permit);
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                        }
                    }
                    // Preempted: retry once the gate is ours again.
                    _ = permit.preempted() => {}
                }
            };
            let mut st = shared.st.lock();
            match res {
                Ok(bytes) if bytes.len() as u64 == total - start => {
                    st.tail = Some((start, bytes));
                }
                _ => st.tail_failed = true,
            }
            shared.cv.notify_all();
        });
    }
}

/// Cloneable view of an [`HttpSource`]'s buffer state.
#[derive(Clone)]
pub struct HttpStats(Arc<Shared>);

impl HttpStats {
    pub fn get(&self) -> BufferStats {
        self.0.stats()
    }

    /// The whole file, if it was fetched contiguously from byte 0.
    pub fn completed_bytes(&self) -> Option<Vec<u8>> {
        let st = self.0.st.lock();
        st.complete().then(|| st.data.clone())
    }
}

impl Drop for HttpSource {
    fn drop(&mut self) {
        self.shared.st.lock().closed = true;
        self.shared.wake.notify_one();
        self.shared.abort.notify_one();
    }
}

impl Read for HttpSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut st = self.shared.st.lock();
        loop {
            let pos = self.pos;
            if let Some(total) = st.total
                && pos >= total
            {
                return Ok(0);
            }
            // Inside the window.
            if pos >= st.base && pos < st.end() {
                let off = (pos - st.base) as usize;
                let n = buf.len().min(st.data.len() - off);
                buf[..n].copy_from_slice(&st.data[off..off + n]);
                self.pos += n as u64;
                st.read_pos = self.pos;
                drop(st);
                self.shared.wake.notify_one();
                return Ok(n);
            }
            // Inside the tail.
            if let Some((tstart, tail)) = &st.tail
                && pos >= *tstart
                && pos < tstart + tail.len() as u64
            {
                let off = (pos - tstart) as usize;
                let n = buf.len().min(tail.len() - off);
                buf[..n].copy_from_slice(&tail[off..off + n]);
                self.pos += n as u64;
                return Ok(n);
            }
            if st.eof && pos >= st.end() && st.total.is_none() {
                return Ok(0);
            }
            if let Some(f) = &st.fatal {
                return Err(io::Error::other(f.to_app().to_string()));
            }
            let near_end = st
                .total
                .is_some_and(|t| pos + self.cfg.tail_bytes >= t && t > self.cfg.tail_bytes * 2);
            let far = pos < st.base || pos > st.end() + self.cfg.jump_threshold;
            if far && near_end && !st.tail_failed {
                let total = st.total.unwrap_or(0);
                self.request_tail(&mut st, total);
            } else if far {
                // Restart the fetch at the new position.
                st.generation += 1;
                st.base = pos;
                st.data.clear();
                st.eof = false;
                st.read_pos = pos;
                st.idle = false;
                self.shared.abort.notify_one();
            }
            // The reader is waiting: make sure the fetcher knows.
            self.shared.wake.notify_one();
            self.shared.cv.wait_for(&mut st, Duration::from_millis(100));
        }
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let total = self.shared.st.lock().total;
        let new = match pos {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => match total {
                Some(t) => t as i64 + d,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "length unknown",
                    ));
                }
            },
        };
        if new < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.shared.st.lock().total
    }
}

fn header_map(src: &StreamSource) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in &src.headers {
        if let (Ok(k), Ok(v)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            h.insert(k, v);
        }
    }
    h
}

/// Total size from `Content-Range: bytes a-b/total`.
pub(crate) fn parse_total(h: &HeaderMap) -> Option<u64> {
    let v = h.get(CONTENT_RANGE)?.to_str().ok()?;
    v.rsplit('/').next()?.trim().parse().ok()
}

/// An answer that is an API message, not audio: some servers (Subsonic)
/// report errors as a 200 with a JSON or XML body, and a proxy may answer
/// with a web page.
pub(crate) fn api_answer(h: &HeaderMap) -> bool {
    let ct = h.get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    ["application/json", "text/json", "application/xml", "text/xml", "text/html"].iter().any(|t| ct.starts_with(t))
}

/// What an API message in place of audio says went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApiError {
    /// Sign in again (Subsonic codes 40 to 44 and 50).
    Auth,
    /// The song isn't there (code 70).
    NotFound,
    /// Anything else, with the server's words when it gave some.
    Other(String),
}

/// Reads a Subsonic error from a JSON or XML body.
pub(crate) fn api_error(body: &[u8]) -> ApiError {
    let text = String::from_utf8_lossy(&body[..body.len().min(64 * 1024)]);
    let (code, message) = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => {
            let e = &v["subsonic-response"]["error"];
            (e["code"].as_i64(), e["message"].as_str().map(str::to_owned))
        }
        Err(_) => {
            // XML: <error code="40" message="..."/>
            let attr = |name: &str| {
                let key = format!("{name}=\"");
                text.find("<error").and_then(|i| {
                    let rest = &text[i..];
                    rest.find(&key).map(|j| rest[j + key.len()..].split('"').next().unwrap_or("").to_owned())
                })
            };
            (attr("code").and_then(|c| c.parse().ok()), attr("message"))
        }
    };
    match code {
        Some(40..=44) | Some(50) => ApiError::Auth,
        Some(70) => ApiError::NotFound,
        _ => ApiError::Other(message.unwrap_or_else(|| "The server sent a message instead of the song.".into())),
    }
}

fn classify(status: StatusCode) -> Option<Fatal> {
    match status.as_u16() {
        404 | 410 => Some(Fatal::NotFound),
        401 | 403 => Some(Fatal::Auth(format!("http {}", status.as_u16()))),
        400..=499 => Some(Fatal::Server(format!("http {}", status.as_u16()))),
        _ => None,
    }
}

async fn fetch_range(
    client: &reqwest::Client,
    src: &StreamSource,
    start: u64,
    end: Option<u64>,
) -> Result<Vec<u8>, String> {
    let range = match end {
        Some(e) => format!("bytes={start}-{e}"),
        None => format!("bytes={start}-"),
    };
    let resp = client
        .get(&src.url)
        .headers(header_map(src))
        .header(RANGE, range)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if resp.status() != StatusCode::PARTIAL_CONTENT || api_answer(resp.headers()) {
        return Err(format!("http {}", resp.status().as_u16()));
    }
    let b = resp.bytes().await.map_err(|e| e.without_url().to_string())?;
    Ok(b.to_vec())
}

/// What ended one request.
enum Stop {
    /// Go back and decide again (window full, restart, preempted, EOF).
    Again,
    /// The connection failed: back off and resume.
    Failed,
    /// Give up for good.
    Exit,
}

async fn fetch_task(
    shared: Arc<Shared>,
    client: reqwest::Client,
    src: Arc<StreamSource>,
    cfg: HttpConfig,
    gate: Arc<TransferGate>,
    prio: PriorityCell,
) {
    let mut backoff = cfg.min_backoff;
    // Set while requests keep failing; cleared by any received byte.
    let mut failing_since: Option<Instant> = None;
    let resume_below = cfg.max_ahead - cfg.max_ahead / 4;
    loop {
        // 1. Wait until bytes are needed. Holds no gate permit.
        let generation = loop {
            let notified = shared.wake.notified();
            {
                let mut st = shared.st.lock();
                if st.closed {
                    return;
                }
                let complete = st.eof || st.total.is_some_and(|t| st.end() >= t);
                let ahead = st.end().saturating_sub(st.read_pos) as usize;
                let need = st.fatal.is_none() && !complete && ahead < resume_below;
                st.idle = !need;
                if need {
                    break st.generation;
                }
            }
            notified.await;
        };

        // 2. Wait for the transfer gate.
        shared.st.lock().gate_wait = true;
        let permit = tokio::select! {
            p = gate.acquire(&prio) => Some(p),
            _ = shared.abort.notified() => None,
        };
        let from = {
            let mut st = shared.st.lock();
            st.gate_wait = false;
            shared.cv.notify_all();
            if st.closed {
                return;
            }
            if st.generation != generation {
                continue;
            }
            st.end()
        };
        let Some(permit) = permit else { continue };

        // 3. One request, until it stops for any reason.
        let stop = tokio::select! {
            s = transfer(&shared, &client, &src, &cfg, generation, from, &mut failing_since, &mut backoff) => s,
            _ = permit.preempted() => Stop::Again,
            _ = shared.abort.notified() => Stop::Again,
        };
        drop(permit);
        match stop {
            Stop::Again => {}
            Stop::Exit => return,
            Stop::Failed => {
                let since = *failing_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= cfg.retry_window {
                    break;
                }
                {
                    let mut st = shared.st.lock();
                    if st.closed {
                        return;
                    }
                    st.resumes += 1;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(cfg.max_backoff);
            }
        }
    }
    let mut st = shared.st.lock();
    if st.fatal.is_none() {
        st.fatal = Some(Fatal::Network("stream lost".into()));
    }
    st.idle = false;
    shared.cv.notify_all();
}

#[allow(clippy::too_many_arguments)]
async fn transfer(
    shared: &Arc<Shared>,
    client: &reqwest::Client,
    src: &StreamSource,
    cfg: &HttpConfig,
    generation: u64,
    from: u64,
    failing_since: &mut Option<Instant>,
    backoff: &mut Duration,
) -> Stop {
    let resp = client
        .get(&src.url)
        .headers(header_map(src))
        .header(RANGE, format!("bytes={from}-"))
        .send()
        .await;
    let mut resp = match resp {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("stream request failed: {}", e.without_url());
            return Stop::Failed;
        }
    };
    let status = resp.status();
    if let Some(f) = classify(status) {
        let body = resp.bytes().await.unwrap_or_default();
        tracing::warn!(
            "stream refused: http {} ({})",
            status.as_u16(),
            String::from_utf8_lossy(&body[..body.len().min(200)])
        );
        let mut st = shared.st.lock();
        st.fatal = Some(f);
        shared.cv.notify_all();
        return Stop::Exit;
    }
    if !status.is_success() {
        // 5xx: transient server trouble, retry.
        tracing::debug!("stream http {}", status.as_u16());
        return Stop::Failed;
    }
    if api_answer(resp.headers()) {
        let body = resp.bytes().await.unwrap_or_default();
        let fatal = match api_error(&body) {
            ApiError::Auth => Fatal::Auth("server asked to sign in".into()),
            ApiError::NotFound => Fatal::NotFound,
            ApiError::Other(m) => Fatal::Server(m),
        };
        let mut st = shared.st.lock();
        st.fatal = Some(fatal);
        shared.cv.notify_all();
        return Stop::Exit;
    }
    // A 200 means the server ignored the range: skip what we have.
    let mut skip = if status == StatusCode::OK { from } else { 0 };
    {
        let mut st = shared.st.lock();
        if let Some(t) = parse_total(resp.headers()) {
            st.total = Some(t);
        } else if status == StatusCode::OK
            && let Some(len) = resp.content_length()
        {
            st.total = Some(len);
        }
        st.opened = true;
        shared.cv.notify_all();
    }
    loop {
        match resp.chunk().await {
            Ok(Some(mut bytes)) => {
                if skip > 0 {
                    let n = (skip as usize).min(bytes.len());
                    skip -= n as u64;
                    let _ = bytes.split_to(n);
                    if bytes.is_empty() {
                        continue;
                    }
                }
                *failing_since = None;
                *backoff = cfg.min_backoff;
                let full = {
                    let mut st = shared.st.lock();
                    if st.closed {
                        return Stop::Exit;
                    }
                    if st.generation != generation {
                        return Stop::Again;
                    }
                    st.data.extend_from_slice(&bytes);
                    // Trim far-behind bytes so the window can't grow without bound.
                    let behind = st.read_pos.saturating_sub(st.base) as usize;
                    if behind > cfg.keep_behind * 2 {
                        let drop_n = behind - cfg.keep_behind;
                        st.data.drain(..drop_n);
                        st.base += drop_n as u64;
                    }
                    shared.cv.notify_all();
                    let ahead = st.end().saturating_sub(st.read_pos) as usize;
                    if ahead >= cfg.max_ahead && st.total.is_some() {
                        // Window full: drop the connection (and the gate),
                        // resume by range later.
                        return Stop::Again;
                    }
                    ahead >= cfg.max_ahead
                };
                if full {
                    // Length unknown (a transcode): a new request would
                    // start it over, so the connection waits for the reader.
                    let resume_below = cfg.max_ahead - cfg.max_ahead / 4;
                    loop {
                        let notified = shared.wake.notified();
                        {
                            let st = shared.st.lock();
                            if st.closed {
                                return Stop::Exit;
                            }
                            if st.generation != generation {
                                return Stop::Again;
                            }
                            if (st.end().saturating_sub(st.read_pos) as usize) < resume_below {
                                break;
                            }
                        }
                        notified.await;
                    }
                }
            }
            Ok(None) => {
                let mut st = shared.st.lock();
                if st.generation != generation {
                    return Stop::Again;
                }
                return match st.total {
                    // Connection closed early: resume from the last byte.
                    Some(t) if st.end() < t => Stop::Failed,
                    _ => {
                        st.eof = true;
                        if st.total.is_none() {
                            st.total = Some(st.end());
                        }
                        shared.cv.notify_all();
                        Stop::Again
                    }
                };
            }
            Err(e) => {
                tracing::debug!("stream dropped: {}", e.without_url());
                return Stop::Failed;
            }
        }
    }
}

#[cfg(test)]
mod api_answer_tests {
    use super::{ApiError, api_answer, api_error};
    use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};

    #[test]
    fn a_json_or_xml_answer_is_not_audio() {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
        assert!(api_answer(&h));
        h.insert(CONTENT_TYPE, HeaderValue::from_static("text/xml"));
        assert!(api_answer(&h));
        h.insert(CONTENT_TYPE, HeaderValue::from_static("audio/flac"));
        assert!(!api_answer(&h));
        assert!(!api_answer(&HeaderMap::new()));
    }

    #[test]
    fn subsonic_error_codes_map_to_what_went_wrong() {
        let json = |code: i64| format!(r#"{{"subsonic-response":{{"status":"failed","error":{{"code":{code},"message":"m{code}"}}}}}}"#);
        assert_eq!(api_error(json(40).as_bytes()), ApiError::Auth);
        assert_eq!(api_error(json(44).as_bytes()), ApiError::Auth);
        assert_eq!(api_error(json(70).as_bytes()), ApiError::NotFound);
        assert_eq!(api_error(json(0).as_bytes()), ApiError::Other("m0".into()));
        let xml = r#"<subsonic-response status="failed"><error code="70" message="Song not found"/></subsonic-response>"#;
        assert_eq!(api_error(xml.as_bytes()), ApiError::NotFound);
        assert!(matches!(api_error(b"<html>Sign in</html>"), ApiError::Other(_)));
    }
}
