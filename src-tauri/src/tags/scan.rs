//! The background tag scan.
//!
//! One task reads one file at a time, album by album, through [`Reader`]
//! (lowest priority on the player's transfer gate, waiting while playback
//! downloads). A file is read once per part key; a changed file has a new
//! key and is read again. Network or server trouble pauses the scan and it
//! tries again a minute later; a file that can't be read is marked and left
//! until it changes. Progress goes to Settings ("Reading tags: 812 of 1,439").

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::Notify;

use crate::audio::StreamSource;
use crate::db::DbHandle;
use crate::error::AppResult;

use super::fetch::{FetchError, Reader, read_tags};
use super::store::{self, Counts, Status};
use super::FileTags;

/// Files per batch between progress and album refreshes.
const BATCH: usize = 25;
/// Re-check for new or changed files this often when idle.
const IDLE_RECHECK: Duration = Duration::from_secs(60);
/// Wait after network or server trouble.
const BACKOFF: Duration = Duration::from_secs(60);
const PROGRESS_EVERY: Duration = Duration::from_millis(750);

/// Where a track's part key can be read: its server's direct file URL plus
/// the token header. The track id names the server (part keys don't).
/// Fails while there's no usable connection (offline, or only the relay,
/// which is too slow to spend on tags).
#[async_trait]
pub trait PartSource: Send + Sync {
    async fn part(&self, track_id: &str, part_key: &str) -> AppResult<StreamSource>;
}

/// Scan progress for Settings (`tag_scan_status`, `tags://progress`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TagScanStatus {
    /// The setting is on.
    pub enabled: bool,
    /// Reading files right now.
    pub running: bool,
    /// Files read (any outcome) out of `total`.
    pub done: u64,
    pub total: u64,
    /// Files that couldn't be read.
    pub failed: u64,
    /// Waiting after network or server trouble.
    pub paused: bool,
}

/// What the scan tells the app.
pub trait TagEvents: Send + Sync {
    fn progress(&self, status: &TagScanStatus);
    /// Tags changed: badges, editions and filters should be read again.
    fn changed(&self);
}

struct Inner {
    db: DbHandle,
    parts: Arc<dyn PartSource>,
    reader: Reader,
    events: Option<Arc<dyn TagEvents>>,
    enabled: AtomicBool,
    wake: Notify,
    status: Mutex<TagScanStatus>,
}

/// Handle to the scan task. Cheap to clone.
#[derive(Clone)]
pub struct TagScanner(Arc<Inner>);

fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// How one file read ended, for the cache.
fn outcome(r: Result<FileTags, FetchError>) -> Result<(Status, FileTags), String> {
    match r {
        Ok(t) => Ok((Status::Ok, t)),
        Err(FetchError::File(e)) => {
            tracing::debug!("tag read failed: {e}");
            Ok((Status::Failed, FileTags::default()))
        }
        Err(FetchError::RangeRefused) => Ok((Status::Skipped, FileTags::default())),
        Err(FetchError::Transient(e)) => Err(e),
    }
}

impl TagScanner {
    pub fn new(db: DbHandle, parts: Arc<dyn PartSource>, reader: Reader, events: Option<Arc<dyn TagEvents>>) -> Self {
        Self(Arc::new(Inner {
            db,
            parts,
            reader,
            events,
            enabled: AtomicBool::new(true),
            wake: Notify::new(),
            status: Mutex::new(TagScanStatus { enabled: true, ..Default::default() }),
        }))
    }

    /// Starts the scan task on the current runtime.
    pub fn spawn(&self) {
        let me = self.clone();
        tokio::spawn(async move { me.run().await });
    }

    /// Turns reading on or off (the "Read extra tags from files" setting).
    pub fn set_enabled(&self, on: bool) {
        let was = self.0.enabled.swap(on, Ordering::SeqCst);
        self.0.status.lock().enabled = on;
        if was != on {
            self.emit_progress();
            self.0.wake.notify_one();
        }
    }

    /// Look for new or changed files now (after a library sync).
    pub fn poke(&self) {
        self.0.wake.notify_one();
    }

    pub fn status(&self) -> TagScanStatus {
        *self.0.status.lock()
    }

    fn enabled(&self) -> bool {
        self.0.enabled.load(Ordering::SeqCst)
    }

    fn emit_progress(&self) {
        if let Some(ev) = &self.0.events {
            ev.progress(&self.status());
        }
    }

    fn set_counts(&self, c: Counts) {
        let mut s = self.0.status.lock();
        s.done = c.done;
        s.total = c.total;
        s.failed = c.failed;
    }

    async fn counts(&self) -> Option<Counts> {
        self.0.db.read_async(store::counts).await.ok()
    }

    async fn sleep_or_wake(&self, d: Duration) {
        let _ = tokio::time::timeout(d, self.0.wake.notified()).await;
    }

    /// Rebuilds the album view of the tags and tells the UI.
    async fn refresh(&self) {
        if let Err(e) = self.0.db.write_async(|c| store::refresh_albums(c)).await {
            tracing::warn!("album tags refresh failed: {e}");
        }
        if let Some(ev) = &self.0.events {
            ev.changed();
        }
    }

    async fn run(self) {
        let mut pruned_once = false;
        loop {
            if !self.enabled() {
                self.0.status.lock().running = false;
                self.emit_progress();
                self.0.wake.notified().await;
                continue;
            }
            if !pruned_once {
                pruned_once = true;
                match self.0.db.write_async(|c| store::prune(c)).await {
                    Ok(n) if n > 0 => self.refresh().await,
                    _ => {}
                }
            }
            match self.pass().await {
                Pass::Idle => {
                    self.0.status.lock().running = false;
                    self.emit_progress();
                    self.sleep_or_wake(IDLE_RECHECK).await;
                    // Files may have changed or gone: drop rows for old keys.
                    if let Ok(n) = self.0.db.write_async(|c| store::prune(c)).await
                        && n > 0
                    {
                        self.refresh().await;
                    }
                }
                Pass::Trouble => {
                    {
                        let mut s = self.0.status.lock();
                        s.running = false;
                        s.paused = true;
                    }
                    self.emit_progress();
                    self.sleep_or_wake(BACKOFF).await;
                    self.0.status.lock().paused = false;
                }
                Pass::More => {}
            }
        }
    }

    /// Reads one batch of files.
    async fn pass(&self) -> Pass {
        let batch = match self.0.db.read_async(|c| store::pending(c, BATCH)).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("tag scan: {e}");
                return Pass::Trouble;
            }
        };
        if let Some(c) = self.counts().await {
            self.set_counts(c);
        }
        if batch.is_empty() {
            return Pass::Idle;
        }
        self.0.status.lock().running = true;
        self.emit_progress();
        let mut last_emit = Instant::now();
        let mut wrote = false;
        let mut read_ids: Vec<String> = Vec::new();
        let mut result = Pass::More;
        for item in batch {
            if !self.enabled() {
                break;
            }
            let src = match self.0.parts.part(&item.track_id, &item.part_key).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!("tag scan paused: {e}");
                    result = Pass::Trouble;
                    break;
                }
            };
            let read = read_tags(&self.0.reader, &src, item.container.as_deref()).await;
            let (status, tags) = match outcome(read) {
                Ok(o) => o,
                Err(e) => {
                    tracing::debug!("tag scan paused: {e}");
                    result = Pass::Trouble;
                    break;
                }
            };
            let key = item.part_key.clone();
            let now = now_s();
            if let Err(e) = self.0.db.write_async(move |c| store::save(c, &key, status, &tags, now)).await {
                tracing::warn!("tag scan: {e}");
                result = Pass::Trouble;
                break;
            }
            wrote = true;
            read_ids.push(item.track_id.clone());
            {
                let mut s = self.0.status.lock();
                s.done = (s.done + 1).min(s.total);
                if status != Status::Ok {
                    s.failed += 1;
                }
            }
            if last_emit.elapsed() >= PROGRESS_EVERY {
                last_emit = Instant::now();
                self.emit_progress();
            }
        }
        if wrote {
            self.refresh().await;
            // ISRCs, recording ids and versions feed the song index: redo
            // the rows of just these songs' artists (after album_tags, whose
            // versions decide release status).
            if let Err(e) = self.0.db.write_async(move |c| crate::identity::index::refresh_for_tracks(c, &read_ids)).await {
                tracing::warn!("song index refresh failed: {e}");
            }
        }
        if let Some(c) = self.counts().await {
            self.set_counts(c);
        }
        self.emit_progress();
        result
    }
}

enum Pass {
    /// Nothing left to read.
    Idle,
    /// A batch was read; go on.
    More,
    /// No connection or a server problem: wait, then try again.
    Trouble,
}
