//! Disk pre-cache: the next few tracks are downloaded whole into a
//! size-capped LRU directory so playback keeps going when the network drops.
//! Recency is the file's modified time (touched on every hit), so no index
//! file can get out of sync.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use parking_lot::Mutex;
use reqwest::StatusCode;
use reqwest::header::RANGE;
use tokio::io::AsyncWriteExt;
use tokio::runtime::Handle;
use tokio::sync::Notify;

use super::gate::{Permit, Priority, TransferGate};
use super::http::parse_total;
use super::source::{StreamSource, TrackResolver};
use crate::model::Track;

/// Stable cache key for a track's current file (the part key changes when the
/// file is replaced, so a stale copy is never used).
pub fn cache_key(track: &Track) -> String {
    // FNV-1a: tiny, stable across runs and platforms.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in track.part_key.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let id: String = track
        .id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(32)
        .collect();
    format!("{id}-{h:016x}")
}

pub struct DiskCache {
    dir: PathBuf,
    cap_bytes: u64,
    /// Keys that must not be evicted (current + upcoming).
    pinned: Mutex<HashSet<String>>,
}

impl DiskCache {
    pub fn new(dir: impl Into<PathBuf>, cap_bytes: u64) -> std::io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        // Leftovers from an interrupted download.
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "part") {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
        Ok(Self {
            dir,
            cap_bytes,
            pinned: Mutex::new(HashSet::new()),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.audio"))
    }

    /// The cached file for `key`, touching it for LRU.
    pub fn get(&self, key: &str) -> Option<PathBuf> {
        let p = self.path(key);
        if p.is_file() {
            if let Ok(f) = fs::File::options().append(true).open(&p) {
                let _ = f.set_modified(SystemTime::now());
            }
            Some(p)
        } else {
            None
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.path(key).is_file()
    }

    pub fn set_pinned(&self, keys: impl IntoIterator<Item = String>) {
        *self.pinned.lock() = keys.into_iter().collect();
    }

    /// Stores a complete file and evicts old entries past the cap.
    pub fn insert_bytes(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        // Distinct from a pre-cache download's partial file for the same key.
        let tmp = self.dir.join(format!("{key}.ins.part"));
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, self.path(key))?;
        self.evict();
        Ok(())
    }

    fn part_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.part"))
    }

    fn commit(&self, key: &str) -> std::io::Result<()> {
        fs::rename(self.part_path(key), self.path(key))?;
        self.evict();
        Ok(())
    }

    /// Total bytes of complete entries.
    pub fn used_bytes(&self) -> u64 {
        self.entries().iter().map(|e| e.1).sum()
    }

    fn entries(&self) -> Vec<(PathBuf, u64, SystemTime, String)> {
        let mut v = Vec::new();
        if let Ok(rd) = fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_none_or(|x| x != "audio") {
                    continue;
                }
                if let Ok(m) = e.metadata() {
                    let key = p
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    v.push((p, m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH), key));
                }
            }
        }
        v
    }

    /// Deletes least-recently-used, unpinned entries until under the cap.
    pub fn evict(&self) {
        let mut entries = self.entries();
        let mut used: u64 = entries.iter().map(|e| e.1).sum();
        if used <= self.cap_bytes {
            return;
        }
        entries.sort_by_key(|e| e.2);
        let pinned = self.pinned.lock().clone();
        for (p, len, _, key) in entries {
            if used <= self.cap_bytes {
                break;
            }
            if pinned.contains(&key) {
                continue;
            }
            if fs::remove_file(&p).is_ok() {
                used = used.saturating_sub(len);
            }
        }
    }
}

/// What the pre-cacher needs to know about the engine.
pub trait PrecacheHost: Send + Sync {
    /// True when the playing and preloaded streams are fully buffered (or
    /// paused with a full read-ahead window), so pre-caching won't compete.
    fn precache_allowed(&self) -> bool;
    /// The engine is already streaming this track (don't download it twice).
    fn is_streaming(&self, track: &Track) -> bool;
}

/// Downloads wanted tracks into the cache one at a time, in order, through
/// the transfer gate at the lowest priority. A preempted download keeps its
/// partial file and resumes by `Range`.
pub struct Precacher {
    want: Mutex<Vec<Track>>,
    wake: Arc<Notify>,
}

/// Collaborators of the pre-cache task.
struct Deps {
    cache: Arc<DiskCache>,
    resolver: Arc<dyn TrackResolver>,
    client: reqwest::Client,
    gate: Arc<TransferGate>,
    host: Option<Arc<dyn PrecacheHost>>,
}

impl Deps {
    fn allowed(&self) -> bool {
        self.host.as_ref().is_none_or(|h| h.precache_allowed())
    }
    fn streaming(&self, t: &Track) -> bool {
        self.host.as_ref().is_some_and(|h| h.is_streaming(t))
    }
}

impl Precacher {
    pub fn spawn(
        handle: &Handle,
        cache: Arc<DiskCache>,
        resolver: Arc<dyn TrackResolver>,
        client: reqwest::Client,
        gate: Arc<TransferGate>,
        host: Option<Arc<dyn PrecacheHost>>,
    ) -> Arc<Self> {
        let me = Arc::new(Self {
            want: Mutex::new(Vec::new()),
            wake: Arc::new(Notify::new()),
        });
        let deps = Deps {
            cache,
            resolver,
            client,
            gate,
            host,
        };
        let weak = Arc::downgrade(&me);
        let wake = me.wake.clone();
        handle.spawn(async move {
            loop {
                let notified = wake.notified();
                let Some(me) = weak.upgrade() else { return };
                let next = if deps.allowed() {
                    me.want
                        .lock()
                        .iter()
                        .find(|t| !deps.cache.contains(&cache_key(t)) && !deps.streaming(t))
                        .cloned()
                } else {
                    None
                };
                match next {
                    Some(t) => {
                        if let Err(e) = download(&deps, &t, &me).await {
                            tracing::debug!("precache {} failed: {e}", t.id);
                            // Don't hammer: drop it from the wish list.
                            me.want.lock().retain(|w| w.id != t.id);
                        }
                    }
                    None => {
                        drop(me);
                        // Re-check periodically: the engine's buffering
                        // state changes without telling us.
                        let _ = tokio::time::timeout(Duration::from_millis(200), notified).await;
                    }
                }
            }
        });
        me
    }

    /// Replaces the wish list (upcoming tracks, in priority order).
    pub fn set_wanted(&self, tracks: Vec<Track>) {
        *self.want.lock() = tracks;
        self.wake.notify_one();
    }

    fn still_wanted(&self, id: &str) -> bool {
        self.want.lock().iter().any(|t| t.id == id)
    }
}

impl Drop for Precacher {
    fn drop(&mut self) {
        // Wake the task so it notices the handle is gone and exits.
        self.wake.notify_one();
    }
}

/// How one download attempt ended.
enum Attempt {
    Done,
    /// Preempted: resume later by range.
    Yield,
    Failed(String),
}

async fn download(d: &Deps, track: &Track, pre: &Precacher) -> Result<(), String> {
    let key = cache_key(track);
    let src = d.resolver.stream(track).await.map_err(|e| e.to_string())?;
    let part = d.cache.part_path(&key);
    let prio = Priority::Precache.cell();
    let mut failures = 0u32;
    let result = loop {
        if !pre.still_wanted(&track.id) {
            break Err("no longer wanted".to_string());
        }
        if d.streaming(track) {
            break Err("being streamed".to_string());
        }
        if !d.allowed() {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        let permit = d.gate.acquire(&prio).await;
        let a = attempt(d, &src, &part, pre, &track.id, &permit).await;
        drop(permit);
        match a {
            Attempt::Done => break Ok(()),
            Attempt::Yield => {}
            Attempt::Failed(e) => {
                failures += 1;
                if failures >= 3 {
                    break Err(e);
                }
                tokio::time::sleep(Duration::from_millis(250 * failures as u64)).await;
            }
        }
    };
    match result {
        Ok(()) => d.cache.commit(&key).map_err(|e| e.to_string()),
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

/// One request, continuing the partial file from where it ends. Checks for
/// preemption only between chunks, so a file write is never cut short.
async fn attempt(
    d: &Deps,
    src: &StreamSource,
    part: &Path,
    pre: &Precacher,
    id: &str,
    permit: &Permit,
) -> Attempt {
    let have = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
    let mut req = d.client.get(&src.url);
    for (k, v) in &src.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let sent = tokio::select! {
        r = req.header(RANGE, format!("bytes={have}-")).send() => r,
        _ = permit.preempted() => return Attempt::Yield,
    };
    let mut resp = match sent {
        Ok(r) => r,
        Err(e) => return Attempt::Failed(e.without_url().to_string()),
    };
    let status = resp.status();
    if status == StatusCode::RANGE_NOT_SATISFIABLE && have > 0 {
        return Attempt::Done; // every byte is already here
    }
    if !status.is_success() {
        return Attempt::Failed(format!("http {}", status.as_u16()));
    }
    let total = parse_total(resp.headers()).or_else(|| resp.content_length());
    // 200 means the server ignored the range: start over.
    let append = status == StatusCode::PARTIAL_CONTENT;
    let file = if append {
        tokio::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(part)
            .await
    } else {
        tokio::fs::File::create(part).await
    };
    let mut f = match file {
        Ok(f) => f,
        Err(e) => return Attempt::Failed(e.to_string()),
    };
    let mut written = if append { have } else { 0 };
    loop {
        let next = tokio::select! {
            c = resp.chunk() => c,
            _ = permit.preempted() => {
                let _ = f.flush().await;
                return Attempt::Yield;
            }
        };
        match next {
            Ok(Some(chunk)) => {
                if let Err(e) = f.write_all(&chunk).await {
                    let _ = f.flush().await;
                    return Attempt::Failed(e.to_string());
                }
                written += chunk.len() as u64;
                if !pre.still_wanted(id) {
                    let _ = f.flush().await;
                    return Attempt::Failed("no longer wanted".into());
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = f.flush().await;
                return Attempt::Failed(e.without_url().to_string());
            }
        }
    }
    if let Err(e) = f.flush().await {
        return Attempt::Failed(e.to_string());
    }
    match total {
        Some(t) if written < t => Attempt::Failed("short body".into()),
        _ => Attempt::Done,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, part: &str) -> Track {
        crate::audio::testutil::track(id, "a", part)
    }

    #[test]
    fn key_changes_with_part() {
        assert_ne!(cache_key(&track("1", "/a")), cache_key(&track("1", "/b")));
        assert_eq!(cache_key(&track("1", "/a")), cache_key(&track("1", "/a")));
    }

    #[test]
    fn lru_evicts_oldest_unpinned() {
        let dir = tempfile::tempdir().unwrap();
        let c = DiskCache::new(dir.path(), 250).unwrap();
        c.insert_bytes("a", &[0u8; 100]).unwrap();
        let old = SystemTime::now() - std::time::Duration::from_secs(100);
        fs::File::options()
            .append(true)
            .open(c.path("a"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        c.insert_bytes("b", &[0u8; 100]).unwrap();
        fs::File::options()
            .append(true)
            .open(c.path("b"))
            .unwrap()
            .set_modified(old + std::time::Duration::from_secs(10))
            .unwrap();
        c.set_pinned(["a".to_string()]);
        c.insert_bytes("c", &[0u8; 100]).unwrap();
        // a is older but pinned, so b goes.
        assert!(c.contains("a"));
        assert!(!c.contains("b"));
        assert!(c.contains("c"));
        assert!(c.used_bytes() <= 250);
    }
}
