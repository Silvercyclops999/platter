//! The background tag scan.
//!
//! One task reads one file at a time, album by album, through [`Reader`]
//! (lowest priority on the player's transfer gate, waiting while playback
//! downloads). A file is read once per part key; a changed file has a new
//! key and is read again. Network or server trouble pauses the scan and it
//! tries again a minute later; a file that can't be read is marked and left
//! until it changes. Progress goes to Settings ("Reading tags: 812 of 1,439").

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

/// Files per batch between progress and refreshes of the batch's albums.
const BATCH: usize = 25;
/// Re-check for new or changed files this often when idle.
const IDLE_RECHECK: Duration = Duration::from_secs(60);
/// Wait after network or server trouble.
const BACKOFF: Duration = Duration::from_secs(60);
const PROGRESS_EVERY: Duration = Duration::from_millis(750);
/// Tell the UI tags changed at most this often while a pass runs; the end
/// of the pass always tells it.
const CHANGED_EVERY: Duration = Duration::from_secs(5);

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
    /// When the UI was last told tags changed.
    last_changed: Mutex<Option<Instant>>,
    /// Batches were read since the last full album rebuild.
    dirty: AtomicBool,
    /// Full album rebuilds so far (tests check a big scan does one, not one
    /// per batch).
    full_refreshes: AtomicUsize,
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
            Ok((Status::Failed, FileTags::for_read()))
        }
        Err(FetchError::RangeRefused) => Ok((Status::Skipped, FileTags::for_read())),
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
            last_changed: Mutex::new(None),
            dirty: AtomicBool::new(false),
            full_refreshes: AtomicUsize::new(0),
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

    /// The status tag or its values changed in Settings: work every file's
    /// status out again from the tags already read (then tell the UI), and
    /// look for files to read again for a new tag name.
    pub fn restatus(&self) {
        let me = self.clone();
        std::thread::spawn(move || {
            let r = me.0.db.write(|c| {
                let mut changed = false;
                crate::archive::in_tx(c, |c| {
                    changed = store::restatus(c)?;
                    Ok(())
                })?;
                Ok(changed)
            });
            match r {
                Ok(true) => {
                    if let Some(ev) = &me.0.events {
                        ev.changed();
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("file status refresh failed: {e}"),
            }
            me.0.wake.notify_one();
        });
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

    /// How many full album rebuilds the scan has done.
    pub fn full_refreshes(&self) -> usize {
        self.0.full_refreshes.load(Ordering::SeqCst)
    }

    /// Tells the UI tags changed now.
    fn changed_now(&self) {
        *self.0.last_changed.lock() = Some(Instant::now());
        if let Some(ev) = &self.0.events {
            ev.changed();
        }
    }

    /// Tells the UI tags changed unless it was told a moment ago (the end
    /// of the pass tells it again).
    fn changed_soon(&self) {
        let due = self.0.last_changed.lock().is_none_or(|t| t.elapsed() >= CHANGED_EVERY);
        if due {
            self.changed_now();
        }
    }

    /// Rebuilds the whole album view of the tags and tells the UI.
    async fn refresh(&self) {
        self.0.dirty.store(false, Ordering::SeqCst);
        self.0.full_refreshes.fetch_add(1, Ordering::SeqCst);
        if let Err(e) = self.0.db.write_async(|c| store::refresh_albums(c)).await {
            tracing::warn!("album tags refresh failed: {e}");
        }
        self.changed_now();
    }

    /// The end of a pass (nothing left, or trouble): one full rebuild if
    /// batches were read since the last one.
    async fn settle(&self) {
        if self.0.dirty.load(Ordering::SeqCst) {
            self.refresh().await;
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
                    self.settle().await;
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
                    self.settle().await;
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
            self.0.dirty.store(true, Ordering::SeqCst);
            // The album view of just these songs' albums (and their groups),
            // then the song index: ISRCs, recording ids and versions, redone
            // for these songs' artists (after album_tags, whose versions
            // decide release status). Then release status, versions and eras
            // for the same artists. The whole view is rebuilt once at the
            // end of the pass.
            let refreshed = self
                .0
                .db
                .write_async(move |c| {
                    crate::archive::in_tx(c, |c| {
                        store::refresh_albums_for(c, &read_ids)?;
                        crate::identity::index::refresh_for_tracks(c, &read_ids)?;
                        crate::archive::refresh_for_tracks(c, &read_ids)
                    })
                })
                .await;
            if let Err(e) = refreshed {
                tracing::warn!("song index refresh failed: {e}");
            }
            self.changed_soon();
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
