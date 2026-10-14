//! Artwork disk cache.
//!
//! Files live under the cache dir, keyed by a hash of (path, w, h); a small
//! SQLite index (`index.db`) holds size, MIME type and an LRU access stamp.
//! Total size is capped (default 2 GB): least recently used images are
//! evicted first. Concurrent requests for the same key share one fetch.
//! When the disk is full, the cache evicts and retries once; if that fails
//! the store errors (the image is still served to the caller).

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use bytes::Bytes;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{AppError, AppResult};

use super::source::{ArtBytes, ArtFetcher};

pub const DEFAULT_CAP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_DIM: u32 = 4096;
const MAX_PATH_LEN: usize = 2048;

type SharedFetch = Shared<BoxFuture<'static, Result<ArtBytes, Arc<AppError>>>>;

#[derive(Clone)]
pub struct ArtCache {
    inner: Arc<Inner>,
}

struct Inner {
    dir: PathBuf,
    index: Mutex<Connection>,
    cap: AtomicU64,
    total: AtomicU64,
    clock: AtomicI64,
    fetcher: Arc<dyn ArtFetcher>,
    inflight: Mutex<HashMap<String, SharedFetch>>,
    #[cfg(test)]
    fail_writes: std::sync::atomic::AtomicU32,
}

fn clone_err(e: &AppError) -> AppError {
    match e {
        AppError::Network(m) => AppError::Network(m.clone()),
        AppError::Auth(m) => AppError::Auth(m.clone()),
        AppError::NotFound(m) => AppError::NotFound(m.clone()),
        AppError::Server(m) => AppError::Server(m.clone()),
        AppError::Io(m) => AppError::Io(m.clone()),
        AppError::Decode(m) => AppError::Decode(m.clone()),
        AppError::Invalid(m) => AppError::Invalid(m.clone()),
        AppError::Internal(m) => AppError::Internal(m.clone()),
    }
}

/// FNV-1a 64 with a seed; two seeds give a 128-bit file name.
fn fnv(seed: u64, s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn cache_key(path: &str, w: u32, h: u32) -> String {
    format!("{w}x{h}:{path}")
}

fn file_name(key: &str) -> String {
    let a = fnv(0, key);
    let b = fnv(0x9e37_79b9_7f4a_7c15, key);
    format!("{:02x}/{a:016x}{b:016x}.img", a >> 56)
}

/// Guess a MIME type from magic bytes.
pub fn sniff_content_type(b: &[u8]) -> &'static str {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if b.starts_with(b"\x89PNG") {
        "image/png"
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else if b.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "application/octet-stream"
    }
}

fn is_disk_full(e: &std::io::Error) -> bool {
    // ENOSPC (28), ERROR_HANDLE_DISK_FULL (39), ERROR_DISK_FULL (112).
    e.kind() == ErrorKind::StorageFull || matches!(e.raw_os_error(), Some(28) | Some(39) | Some(112))
}

impl ArtCache {
    /// Open the cache in `dir` with a size cap in bytes.
    pub fn open(dir: impl AsRef<Path>, cap_bytes: u64, fetcher: Arc<dyn ArtFetcher>) -> AppResult<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let index = Connection::open(dir.join("index.db"))?;
        index.busy_timeout(std::time::Duration::from_secs(5))?;
        let _: String = index.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
        index.pragma_update(None, "synchronous", "NORMAL")?;
        index.execute_batch(
            "CREATE TABLE IF NOT EXISTS art (
                key TEXT PRIMARY KEY, file TEXT NOT NULL, size INTEGER NOT NULL,
                content_type TEXT NOT NULL, last_access INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS art_lru ON art (last_access);",
        )?;
        let (total, clock): (i64, i64) = index.query_row(
            "SELECT COALESCE(sum(size), 0), COALESCE(max(last_access), 0) FROM art",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let cache = Self {
            inner: Arc::new(Inner {
                dir,
                index: Mutex::new(index),
                cap: AtomicU64::new(cap_bytes),
                total: AtomicU64::new(total.max(0) as u64),
                clock: AtomicI64::new(clock + 1),
                fetcher,
                inflight: Mutex::new(HashMap::new()),
                #[cfg(test)]
                fail_writes: std::sync::atomic::AtomicU32::new(0),
            }),
        };
        cache.evict_to(cache.cap())?;
        Ok(cache)
    }

    pub fn cap(&self) -> u64 {
        self.inner.cap.load(Ordering::Acquire)
    }

    /// Bytes currently cached.
    pub fn size(&self) -> u64 {
        self.inner.total.load(Ordering::Acquire)
    }

    /// Change the cap (Settings); evicts immediately if over.
    pub fn set_cap(&self, cap_bytes: u64) -> AppResult<()> {
        self.inner.cap.store(cap_bytes, Ordering::Release);
        self.evict_to(cap_bytes)
    }

    /// Remove everything.
    pub fn clear(&self) -> AppResult<()> {
        self.evict_to(0)
    }

    fn tick(&self) -> i64 {
        self.inner.clock.fetch_add(1, Ordering::AcqRel)
    }

    /// Cached bytes for a key, touching its LRU stamp. Blocking.
    pub fn lookup(&self, path: &str, w: u32, h: u32) -> AppResult<Option<ArtBytes>> {
        let key = cache_key(path, w, h);
        let row: Option<(String, String)> = self
            .inner
            .index
            .lock()
            .query_row("SELECT file, content_type FROM art WHERE key = ?1", [&key], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((file, content_type)) = row else { return Ok(None) };
        match std::fs::read(self.inner.dir.join(&file)) {
            Ok(bytes) => {
                let t = self.tick();
                self.inner.index.lock().execute("UPDATE art SET last_access = ?2 WHERE key = ?1", params![key, t])?;
                Ok(Some(ArtBytes { bytes: Bytes::from(bytes), content_type }))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                self.forget(&key)?;
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn forget(&self, key: &str) -> AppResult<()> {
        let idx = self.inner.index.lock();
        let size: Option<i64> = idx
            .query_row("SELECT size FROM art WHERE key = ?1", [key], |r| r.get(0))
            .optional()?;
        if let Some(size) = size {
            idx.execute("DELETE FROM art WHERE key = ?1", [key])?;
            self.inner.total.fetch_sub(size.max(0) as u64, Ordering::AcqRel);
        }
        Ok(())
    }

    /// Evict least recently used entries until the total is ≤ `target`.
    fn evict_to(&self, target: u64) -> AppResult<()> {
        while self.size() > target {
            let victims: Vec<(String, String, i64)> = {
                let idx = self.inner.index.lock();
                let mut st = idx.prepare_cached("SELECT key, file, size FROM art ORDER BY last_access LIMIT 64")?;
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<Result<_, _>>()?
            };
            if victims.is_empty() {
                self.inner.total.store(0, Ordering::Release);
                break;
            }
            for (key, file, size) in victims {
                if self.size() <= target {
                    break;
                }
                match std::fs::remove_file(self.inner.dir.join(&file)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::NotFound => {}
                    Err(e) => tracing::warn!("art cache evict {file}: {e}"),
                }
                self.inner.index.lock().execute("DELETE FROM art WHERE key = ?1", [&key])?;
                self.inner.total.fetch_sub(size.max(0) as u64, Ordering::AcqRel);
            }
        }
        Ok(())
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        #[cfg(test)]
        {
            let left = self.inner.fail_writes.load(Ordering::Acquire);
            if left > 0 {
                self.inner.fail_writes.store(left - 1, Ordering::Release);
                return Err(std::io::Error::new(ErrorKind::StorageFull, "disk full (test)"));
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// Store bytes for a key. Blocking. Images larger than the cap are not
    /// cached. On disk-full: evict a quarter of the cache (at least 2× the
    /// image) and retry once.
    pub fn store(&self, path: &str, w: u32, h: u32, art: &ArtBytes) -> AppResult<()> {
        let size = art.bytes.len() as u64;
        let cap = self.cap();
        if size > cap {
            return Ok(());
        }
        let key = cache_key(path, w, h);
        self.forget(&key)?;
        self.evict_to(cap.saturating_sub(size))?;
        let file = file_name(&key);
        let full = self.inner.dir.join(&file);
        if let Err(e) = self.write_file(&full, &art.bytes) {
            if !is_disk_full(&e) {
                return Err(e.into());
            }
            let free = (self.size() / 4).max(size * 2);
            tracing::warn!("art cache: disk full, evicting {free} bytes and retrying");
            self.evict_to(self.size().saturating_sub(free))?;
            self.write_file(&full, &art.bytes)
                .map_err(|e| AppError::Io(format!("art cache: disk full after eviction: {e}")))?;
        }
        let t = self.tick();
        self.inner.index.lock().execute(
            "INSERT INTO art (key, file, size, content_type, last_access) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (key) DO UPDATE SET file = excluded.file, size = excluded.size,
               content_type = excluded.content_type, last_access = excluded.last_access",
            params![key, file, size as i64, art.content_type, t],
        )?;
        self.inner.total.fetch_add(size, Ordering::AcqRel);
        Ok(())
    }

    /// Cached image, or fetch it once (concurrent callers for the same key
    /// share the fetch), store it, and return it.
    pub async fn get_or_fetch(&self, path: &str, w: u32, h: u32) -> AppResult<ArtBytes> {
        if path.is_empty() || path.len() > MAX_PATH_LEN || !path.starts_with('/') {
            return Err(AppError::Invalid(format!("bad art path: {path:.64}")));
        }
        let (w, h) = (w.clamp(1, MAX_DIM), h.clamp(1, MAX_DIM));
        {
            let me = self.clone();
            let p = path.to_owned();
            let hit = tokio::task::spawn_blocking(move || me.lookup(&p, w, h))
                .await
                .map_err(|e| AppError::Internal(format!("art task: {e}")))??;
            if let Some(hit) = hit {
                return Ok(hit);
            }
        }
        let key = cache_key(path, w, h);
        let fut = {
            let mut inflight = self.inner.inflight.lock();
            if let Some(f) = inflight.get(&key) {
                f.clone()
            } else {
                let me = self.clone();
                let p = path.to_owned();
                let k = key.clone();
                let f: BoxFuture<'static, Result<ArtBytes, Arc<AppError>>> = async move {
                    let res = me.fetch_and_store(p, w, h).await;
                    me.inner.inflight.lock().remove(&k);
                    res.map_err(Arc::new)
                }
                .boxed();
                let shared = f.shared();
                inflight.insert(key, shared.clone());
                shared
            }
        };
        fut.await.map_err(|e| clone_err(&e))
    }

    async fn fetch_and_store(&self, path: String, w: u32, h: u32) -> AppResult<ArtBytes> {
        let mut art = self.inner.fetcher.fetch(&path, w, h).await?;
        if art.content_type.is_empty() || art.content_type == "application/octet-stream" {
            art.content_type = sniff_content_type(&art.bytes).to_owned();
        }
        let me = self.clone();
        let a = art.clone();
        let stored = tokio::task::spawn_blocking(move || me.store(&path, w, h, &a))
            .await
            .map_err(|e| AppError::Internal(format!("art task: {e}")))?;
        if let Err(e) = stored {
            // Serve the image anyway; only caching failed.
            tracing::warn!("art cache store failed: {e}");
        }
        Ok(art)
    }
}

/// Parse `art://localhost/<urlencoded path>?w=&h=` (or the Windows form
/// `http://art.localhost/...`) into (path, w, h).
pub fn parse_art_uri(uri: &str) -> Option<(String, u32, u32)> {
    let url = url::Url::parse(uri).ok()?;
    let raw = url.path().trim_start_matches('/');
    let decoded = percent_encoding::percent_decode_str(raw).decode_utf8().ok()?.into_owned();
    if decoded.is_empty() {
        return None;
    }
    let path = if decoded.starts_with('/') { decoded } else { format!("/{decoded}") };
    let mut w = None;
    let mut h = None;
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "w" => w = v.parse::<u32>().ok(),
            "h" => h = v.parse::<u32>().ok(),
            _ => {}
        }
    }
    let w = w.unwrap_or(300);
    let h = h.unwrap_or(w);
    Some((path, w.clamp(1, MAX_DIM), h.clamp(1, MAX_DIM)))
}

/// Serve one request from a Tauri async URI scheme handler:
///
/// ```ignore
/// .register_asynchronous_uri_scheme_protocol("art", |ctx, req, responder| {
///     let cache = ctx.app_handle().state::<ArtCache>().inner().clone();
///     platter_lib::db::artcache::respond(cache, req.uri().to_string(), responder);
/// })
/// ```
pub fn respond(cache: ArtCache, uri: String, responder: tauri::UriSchemeResponder) {
    tauri::async_runtime::spawn(async move {
        use tauri::http::{Response, StatusCode, header};
        let resp = match parse_art_uri(&uri) {
            None => Response::builder().status(StatusCode::BAD_REQUEST).body(Vec::new()),
            Some((path, w, h)) => match cache.get_or_fetch(&path, w, h).await {
                Ok(art) => Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, art.content_type)
                    .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
                    .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                    .body(art.bytes.to_vec()),
                Err(e) => {
                    let status = match e {
                        AppError::NotFound(_) => StatusCode::NOT_FOUND,
                        AppError::Invalid(_) => StatusCode::BAD_REQUEST,
                        _ => StatusCode::BAD_GATEWAY,
                    };
                    Response::builder().status(status).body(Vec::new())
                }
            },
        };
        match resp {
            Ok(r) => responder.respond(r),
            Err(e) => tracing::warn!("art response: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    struct CountingFetcher {
        calls: AtomicUsize,
        size: usize,
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl ArtFetcher for CountingFetcher {
        async fn fetch(&self, path: &str, _w: u32, _h: u32) -> AppResult<ArtBytes> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            if path.contains("missing") {
                return Err(AppError::NotFound(path.into()));
            }
            let mut b = vec![0u8; self.size];
            b[..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
            Ok(ArtBytes { bytes: Bytes::from(b), content_type: String::new() })
        }
    }

    fn fetcher(size: usize, delay_ms: u64) -> Arc<CountingFetcher> {
        Arc::new(CountingFetcher { calls: AtomicUsize::new(0), size, delay: Duration::from_millis(delay_ms) })
    }

    #[tokio::test]
    async fn caches_and_serves_hits() {
        let dir = tempfile::tempdir().unwrap();
        let f = fetcher(1000, 0);
        let c = ArtCache::open(dir.path(), DEFAULT_CAP_BYTES, f.clone()).unwrap();
        let a = c.get_or_fetch("/library/metadata/1/thumb/9", 300, 300).await.unwrap();
        assert_eq!(a.content_type, "image/jpeg");
        assert_eq!(a.bytes.len(), 1000);
        let b = c.get_or_fetch("/library/metadata/1/thumb/9", 300, 300).await.unwrap();
        assert_eq!(a, b);
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        // A different size is a different key.
        c.get_or_fetch("/library/metadata/1/thumb/9", 600, 600).await.unwrap();
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
        assert_eq!(c.size(), 2000);
        // Survives reopen.
        drop(c);
        let c = ArtCache::open(dir.path(), DEFAULT_CAP_BYTES, f.clone()).unwrap();
        assert_eq!(c.size(), 2000);
        c.get_or_fetch("/library/metadata/1/thumb/9", 300, 300).await.unwrap();
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn coalesces_concurrent_requests() {
        let dir = tempfile::tempdir().unwrap();
        let f = fetcher(500, 50);
        let c = ArtCache::open(dir.path(), DEFAULT_CAP_BYTES, f.clone()).unwrap();
        let futs = (0..10).map(|_| {
            let c = c.clone();
            tokio::spawn(async move { c.get_or_fetch("/a", 100, 100).await })
        });
        for r in futures::future::join_all(futs).await {
            assert_eq!(r.unwrap().unwrap().bytes.len(), 500);
        }
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        // Errors are shared too, and not cached.
        let e = c.get_or_fetch("/missing", 1, 1).await.unwrap_err();
        assert_eq!(e.kind(), "notFound");
        assert!(c.get_or_fetch("relative", 1, 1).await.is_err());
    }

    #[tokio::test]
    async fn evicts_least_recently_used_over_cap() {
        let dir = tempfile::tempdir().unwrap();
        let f = fetcher(1000, 0);
        let c = ArtCache::open(dir.path(), 3500, f.clone()).unwrap();
        for p in ["/1", "/2", "/3"] {
            c.get_or_fetch(p, 10, 10).await.unwrap();
        }
        // Touch /1 so /2 is now the least recently used.
        c.get_or_fetch("/1", 10, 10).await.unwrap();
        assert_eq!(f.calls.load(Ordering::SeqCst), 3);
        c.get_or_fetch("/4", 10, 10).await.unwrap();
        assert!(c.size() <= 3500);
        assert!(c.lookup("/2", 10, 10).unwrap().is_none(), "LRU entry evicted");
        assert!(c.lookup("/1", 10, 10).unwrap().is_some());
        assert!(c.lookup("/3", 10, 10).unwrap().is_some());
        assert!(c.lookup("/4", 10, 10).unwrap().is_some());
        // Shrinking the cap evicts immediately.
        c.set_cap(1000).unwrap();
        assert!(c.size() <= 1000);
        // Bigger than the cap: served, not cached.
        let big = ArtCache::open(dir.path().join("big"), 10, f.clone()).unwrap();
        assert_eq!(big.get_or_fetch("/x", 1, 1).await.unwrap().bytes.len(), 1000);
        assert_eq!(big.size(), 0);
    }

    #[tokio::test]
    async fn disk_full_evicts_then_retries_else_errors() {
        let dir = tempfile::tempdir().unwrap();
        let f = fetcher(1000, 0);
        let c = ArtCache::open(dir.path(), DEFAULT_CAP_BYTES, f.clone()).unwrap();
        for p in ["/1", "/2", "/3", "/4"] {
            c.get_or_fetch(p, 10, 10).await.unwrap();
        }
        let art = ArtBytes { bytes: Bytes::from(vec![1u8; 1000]), content_type: "image/png".into() };
        // One failure: evict + retry succeeds.
        c.inner.fail_writes.store(1, Ordering::SeqCst);
        c.store("/5", 10, 10, &art).unwrap();
        assert!(c.lookup("/5", 10, 10).unwrap().is_some());
        assert!(c.lookup("/1", 10, 10).unwrap().is_none(), "evicted to make room");
        // Two failures: error.
        c.inner.fail_writes.store(2, Ordering::SeqCst);
        let e = c.store("/6", 10, 10, &art).unwrap_err();
        assert_eq!(e.kind(), "io");
        assert!(c.lookup("/6", 10, 10).unwrap().is_none());
        // get_or_fetch still serves the image when caching fails.
        c.inner.fail_writes.store(2, Ordering::SeqCst);
        assert_eq!(c.get_or_fetch("/7", 10, 10).await.unwrap().bytes.len(), 1000);
    }

    #[test]
    fn parses_both_uri_forms() {
        let p = parse_art_uri("art://localhost/%2Flibrary%2Fmetadata%2F1%2Fthumb%2F2?w=300&h=200").unwrap();
        assert_eq!(p, ("/library/metadata/1/thumb/2".into(), 300, 200));
        let p = parse_art_uri("http://art.localhost/%2Flibrary%2Fmetadata%2F1%2Fthumb%2F2?w=99999").unwrap();
        assert_eq!(p, ("/library/metadata/1/thumb/2".into(), 4096, 4096));
        assert!(parse_art_uri("art://localhost/").is_none());
        assert_eq!(sniff_content_type(b"\x89PNG...."), "image/png");
    }
}
