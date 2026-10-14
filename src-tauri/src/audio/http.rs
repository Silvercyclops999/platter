//! A symphonia `MediaSource` over HTTP range requests.
//!
//! A background tokio task fills an in-memory byte window ahead of the read
//! position. Reads block (on the decoder thread) until bytes arrive. A
//! dropped connection is resumed from the last received byte with a new
//! `Range` request, so the decoder never sees a gap. Seeks inside the window
//! are free; far seeks restart the fetch at the new offset; reads near the end
//! of the file (trailing tags, MP4 `moov`) are served from a separately
//! fetched tail so probing doesn't throw away the window.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, HeaderMap, HeaderName, HeaderValue, RANGE};
use symphonia::core::io::MediaSource;
use tokio::runtime::Handle;
use tokio::sync::Notify;

use super::source::StreamSource;
use crate::error::{AppError, AppResult};

/// Tuning for [`HttpSource`].
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Stop fetching when this many bytes are buffered ahead of the reader.
    pub max_ahead: usize,
    /// Keep at most this many bytes behind the reader (for cheap back-seeks).
    pub keep_behind: usize,
    /// Give up after this long without receiving a byte.
    pub retry_window: Duration,
    /// First retry delay; doubles up to `max_backoff`.
    pub min_backoff: Duration,
    pub max_backoff: Duration,
    /// How long `open` waits for the first response.
    pub open_timeout: Duration,
    /// Seeks less than this far past the buffered end just wait for the data.
    pub jump_threshold: u64,
    /// Size of the separately fetched file tail.
    pub tail_bytes: u64,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            max_ahead: 48 * 1024 * 1024,
            keep_behind: 16 * 1024 * 1024,
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
}

impl State {
    fn end(&self) -> u64 {
        self.base + self.data.len() as u64
    }
}

struct Shared {
    st: Mutex<State>,
    cv: Condvar,
    /// Wakes the fetch task (reader progressed, restart, close).
    wake: Notify,
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
}

/// Snapshot of what's buffered, for `bufferedMs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferStats {
    pub read_pos: u64,
    pub buffered_end: u64,
    pub total: Option<u64>,
    pub complete: bool,
    pub resumes: u32,
}

impl HttpSource {
    /// Starts fetching and waits for the first response so a 404 or auth
    /// error surfaces here. Blocking; call from a non-runtime thread.
    pub fn open(
        handle: Handle,
        client: reqwest::Client,
        src: StreamSource,
        cfg: HttpConfig,
    ) -> AppResult<Self> {
        let shared = Arc::new(Shared {
            st: Mutex::new(State {
                total: src.size,
                ..Default::default()
            }),
            cv: Condvar::new(),
            wake: Notify::new(),
        });
        let src = Arc::new(src);
        handle.spawn(fetch_task(
            shared.clone(),
            client.clone(),
            src.clone(),
            cfg.clone(),
        ));
        let deadline = Instant::now() + cfg.open_timeout;
        {
            let mut st = shared.st.lock();
            loop {
                if let Some(f) = &st.fatal {
                    let e = f.to_app();
                    st.closed = true;
                    drop(st);
                    shared.wake.notify_one();
                    return Err(e);
                }
                if st.opened {
                    break;
                }
                if shared.cv.wait_until(&mut st, deadline).timed_out() && !st.opened {
                    st.closed = true;
                    drop(st);
                    shared.wake.notify_one();
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
        })
    }

    pub fn stats(&self) -> BufferStats {
        let st = self.shared.st.lock();
        BufferStats {
            read_pos: st.read_pos,
            buffered_end: st.end(),
            total: st.total,
            complete: st.base == 0 && st.total == Some(st.data.len() as u64),
            resumes: st.resumes,
        }
    }

    /// Shared stats handle that stays valid after the source moves into the
    /// decoder.
    pub fn stats_handle(&self) -> HttpStats {
        HttpStats(self.shared.clone())
    }

    /// The whole file, if it was fetched contiguously from byte 0.
    pub fn completed_bytes(&self) -> Option<Vec<u8>> {
        let st = self.shared.st.lock();
        (st.base == 0 && st.total == Some(st.data.len() as u64)).then(|| st.data.clone())
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
        self.handle.spawn(async move {
            let res = fetch_range(&client, &src, start, Some(total - 1)).await;
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
        let st = self.0.st.lock();
        BufferStats {
            read_pos: st.read_pos,
            buffered_end: st.end(),
            total: st.total,
            complete: st.base == 0 && st.total == Some(st.data.len() as u64),
            resumes: st.resumes,
        }
    }

    /// The whole file, if it was fetched contiguously from byte 0.
    pub fn completed_bytes(&self) -> Option<Vec<u8>> {
        let st = self.0.st.lock();
        (st.base == 0 && st.total == Some(st.data.len() as u64)).then(|| st.data.clone())
    }
}

impl Drop for HttpSource {
    fn drop(&mut self) {
        self.shared.st.lock().closed = true;
        self.shared.wake.notify_one();
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
                self.shared.wake.notify_one();
            }
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

fn parse_total(h: &HeaderMap) -> Option<u64> {
    // "bytes 100-199/1234"
    let v = h.get(CONTENT_RANGE)?.to_str().ok()?;
    v.rsplit('/').next()?.trim().parse().ok()
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
    if resp.status() != StatusCode::PARTIAL_CONTENT {
        return Err(format!("http {}", resp.status().as_u16()));
    }
    let b = resp.bytes().await.map_err(|e| e.without_url().to_string())?;
    Ok(b.to_vec())
}

async fn fetch_task(shared: Arc<Shared>, client: reqwest::Client, src: Arc<StreamSource>, cfg: HttpConfig) {
    let mut backoff = cfg.min_backoff;
    let mut last_progress = Instant::now();
    'outer: loop {
        // Where to fetch from, and wait while there's nothing to do.
        let (generation, from) = loop {
            let notified = shared.wake.notified();
            {
                let st = shared.st.lock();
                if st.closed {
                    return;
                }
                let complete = st.eof || st.total.is_some_and(|t| st.end() >= t);
                let ahead = st.end().saturating_sub(st.read_pos) as usize;
                if st.fatal.is_none() && !complete && ahead < cfg.max_ahead {
                    break (st.generation, st.end());
                }
            }
            notified.await;
            last_progress = Instant::now();
        };

        let range = format!("bytes={from}-");
        let resp = client
            .get(&src.url)
            .headers(header_map(&src))
            .header(RANGE, range)
            .send()
            .await;
        let mut resp = match resp {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("stream request failed: {}", e.without_url());
                if !retry_wait(&shared, &mut backoff, last_progress, &cfg).await {
                    break 'outer;
                }
                continue;
            }
        };
        let status = resp.status();
        if let Some(f) = classify(status) {
            let mut st = shared.st.lock();
            st.fatal = Some(f);
            shared.cv.notify_all();
            return;
        }
        if !status.is_success() {
            // 5xx: transient server trouble, retry.
            tracing::debug!("stream http {}", status.as_u16());
            if !retry_wait(&shared, &mut backoff, last_progress, &cfg).await {
                break 'outer;
            }
            continue;
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
            let chunk = resp.chunk().await;
            match chunk {
                Ok(Some(mut bytes)) => {
                    if skip > 0 {
                        let n = (skip as usize).min(bytes.len());
                        skip -= n as u64;
                        let _ = bytes.split_to(n);
                        if bytes.is_empty() {
                            continue;
                        }
                    }
                    last_progress = Instant::now();
                    backoff = cfg.min_backoff;
                    let mut st = shared.st.lock();
                    if st.closed {
                        return;
                    }
                    if st.generation != generation {
                        continue 'outer;
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
                    if ahead >= cfg.max_ahead {
                        // Pause: drop the connection, resume later by range.
                        continue 'outer;
                    }
                }
                Ok(None) => {
                    let early_close = {
                        let mut st = shared.st.lock();
                        if st.generation != generation {
                            continue 'outer;
                        }
                        match st.total {
                            Some(t) if st.end() < t => true,
                            _ => {
                                st.eof = true;
                                if st.total.is_none() {
                                    st.total = Some(st.end());
                                }
                                shared.cv.notify_all();
                                false
                            }
                        }
                    };
                    // Connection closed early: resume from the last byte.
                    if early_close
                        && !retry_wait(&shared, &mut backoff, last_progress, &cfg).await
                    {
                        break 'outer;
                    }
                    continue 'outer;
                }
                Err(e) => {
                    tracing::debug!("stream dropped: {}", e.without_url());
                    if !retry_wait(&shared, &mut backoff, last_progress, &cfg).await {
                        break 'outer;
                    }
                    continue 'outer;
                }
            }
        }
    }
    let mut st = shared.st.lock();
    if st.fatal.is_none() {
        st.fatal = Some(Fatal::Network("stream lost".into()));
    }
    shared.cv.notify_all();
}

/// Sleeps for the backoff; returns false once the retry window is exhausted.
async fn retry_wait(
    shared: &Arc<Shared>,
    backoff: &mut Duration,
    last_progress: Instant,
    cfg: &HttpConfig,
) -> bool {
    if last_progress.elapsed() >= cfg.retry_window {
        return false;
    }
    {
        let mut st = shared.st.lock();
        if st.closed {
            return false;
        }
        st.resumes += 1;
    }
    tokio::time::sleep(*backoff).await;
    *backoff = (*backoff * 2).min(cfg.max_backoff);
    true
}
