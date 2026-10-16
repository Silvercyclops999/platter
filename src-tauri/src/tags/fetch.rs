//! Range reads of file headers through the player's transfer gate.
//!
//! Plex cuts an older file transfer when a new one starts (docs/PLEX-API.md),
//! so every read here holds the player's gate at [`Priority::Background`],
//! below pre-caching. It also waits while playback's own streams are still
//! downloading, and drops its request the moment anything else wants the
//! gate, trying again later from the start (the reads are small).

use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, RANGE};

use crate::audio::StreamSource;
use crate::audio::cache::PrecacheHost;
use crate::audio::gate::{Priority, TransferGate};

use super::FileTags;
use super::parse::{FIRST_READ, MAX_BLOCK, MORE_READ, Sparse, Step, parse};

/// A server that ignores Range gets read whole only below this size.
pub const FULL_READ_MAX: u64 = 2 * 1024 * 1024;
/// Reads after the first, per file.
pub const MAX_EXTRA_READS: usize = 6;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const IDLE_POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// Network, sign-in or server trouble: stop and try again later.
    Transient(String),
    /// This file can't be read (gone, or the server refuses it).
    File(String),
    /// The server ignores Range and the file is too big to read whole.
    RangeRefused,
}

/// Bytes from one read.
#[derive(Debug, Clone)]
pub struct Got {
    pub bytes: Vec<u8>,
    /// File size, when the server said.
    pub total: Option<u64>,
    /// The whole file came back (the server ignored Range).
    pub whole: bool,
}

/// Reads byte ranges one at a time behind playback.
#[derive(Clone)]
pub struct Reader {
    client: reqwest::Client,
    gate: Arc<TransferGate>,
    host: Option<Arc<dyn PrecacheHost>>,
}

/// `bytes a-b/total` → total.
fn content_total(h: &reqwest::header::HeaderMap) -> Option<u64> {
    h.get(CONTENT_RANGE)?.to_str().ok()?.rsplit('/').next()?.trim().parse().ok()
}

impl Reader {
    pub fn new(client: reqwest::Client, gate: Arc<TransferGate>, host: Option<Arc<dyn PrecacheHost>>) -> Self {
        Self { client, gate, host }
    }

    /// Playback has nothing left to download right now.
    fn idle(&self) -> bool {
        self.host.as_ref().is_none_or(|h| h.precache_allowed())
    }

    /// Waits until playback has nothing left to download. False when cancelled.
    pub async fn wait_idle(&self, cancel: &std::sync::atomic::AtomicBool) -> bool {
        while !self.idle() {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return false;
            }
            tokio::time::sleep(IDLE_POLL).await;
        }
        !cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// `len` bytes from `start` (fewer at the end of the file).
    pub async fn range(&self, src: &StreamSource, start: u64, len: usize) -> Result<Got, FetchError> {
        let prio = Priority::Background.cell();
        loop {
            while !self.idle() {
                tokio::time::sleep(IDLE_POLL).await;
            }
            let permit = self.gate.acquire(&prio).await;
            if !self.idle() {
                continue;
            }
            let r = tokio::select! {
                r = self.once(src, start, len) => Some(r),
                _ = permit.preempted() => None,
            };
            drop(permit);
            if let Some(r) = r {
                return r;
            }
            // Something better wanted the gate: wait for it to finish.
            tokio::time::sleep(IDLE_POLL).await;
        }
    }

    async fn once(&self, src: &StreamSource, start: u64, len: usize) -> Result<Got, FetchError> {
        let end = start + len.max(1) as u64 - 1;
        let mut req = self.client.get(&src.url).timeout(REQUEST_TIMEOUT);
        for (k, v) in &src.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .header(RANGE, format!("bytes={start}-{end}"))
            .send()
            .await
            .map_err(|e| FetchError::Transient(e.without_url().to_string()))?;
        let status = resp.status();
        match status {
            StatusCode::PARTIAL_CONTENT => {
                let total = content_total(resp.headers());
                let bytes = body(resp, len as u64).await?;
                Ok(Got { bytes, total, whole: false })
            }
            StatusCode::OK => {
                if start > 0 || resp.content_length().is_some_and(|n| n > FULL_READ_MAX) {
                    return Err(FetchError::RangeRefused);
                }
                let bytes = body(resp, FULL_READ_MAX + 1).await?;
                if bytes.len() as u64 > FULL_READ_MAX {
                    return Err(FetchError::RangeRefused);
                }
                let total = Some(bytes.len() as u64);
                Ok(Got { bytes, total, whole: true })
            }
            StatusCode::NOT_FOUND | StatusCode::GONE => Err(FetchError::File("file unavailable".into())),
            StatusCode::RANGE_NOT_SATISFIABLE => Err(FetchError::File("range not satisfiable".into())),
            s if s == StatusCode::UNAUTHORIZED
                || s == StatusCode::FORBIDDEN
                || s == StatusCode::TOO_MANY_REQUESTS
                || s.is_server_error() =>
            {
                Err(FetchError::Transient(format!("http {}", s.as_u16())))
            }
            s => Err(FetchError::File(format!("http {}", s.as_u16()))),
        }
    }
}

/// Up to `cap` bytes of a response body.
async fn body(mut resp: reqwest::Response, cap: u64) -> Result<Vec<u8>, FetchError> {
    let mut out = Vec::new();
    while (out.len() as u64) < cap {
        match resp.chunk().await {
            Ok(Some(c)) => out.extend_from_slice(&c),
            Ok(None) => break,
            Err(e) => return Err(FetchError::Transient(e.without_url().to_string())),
        }
    }
    out.truncate(cap as usize);
    Ok(out)
}

/// Reads one file's tags: the first 64 KB, then what the parser asks for.
pub async fn read_tags(reader: &Reader, src: &StreamSource, container: Option<&str>) -> Result<FileTags, FetchError> {
    let first = reader.range(src, 0, FIRST_READ).await?;
    let whole = first.whole;
    let mut s = Sparse::new(first.bytes, first.total);
    s.container = container.map(str::to_string);
    if !whole {
        for _ in 0..MAX_EXTRA_READS {
            match parse(&s, false) {
                Step::Done(t) => return Ok(t),
                Step::Need { offset, len } => {
                    if s.total.is_some_and(|t| offset >= t) {
                        break;
                    }
                    let want = len.clamp(MORE_READ, MAX_BLOCK.max(len));
                    let got = reader.range(src, offset, want).await?;
                    if got.bytes.is_empty() {
                        break;
                    }
                    if s.total.is_none() {
                        s.total = got.total;
                    }
                    s.add(offset, got.bytes);
                }
            }
        }
    }
    match parse(&s, true) {
        Step::Done(t) => Ok(t),
        Step::Need { .. } => Ok(FileTags::for_read()),
    }
}
