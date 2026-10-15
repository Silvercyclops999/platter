//! Downloads: albums, playlists, Liked Songs and mixes kept on this computer
//! for playing without the server.
//!
//! The user turns items on; each item's songs go in a queue that fetches one
//! file at a time through the player's transfer gate at background priority,
//! so playback always comes first. A file is the Plex part as it is, or an
//! MP3 from the server's transcoder when the quality setting says so. Files
//! live in one folder, named by track id, shared by every item that holds the
//! song. Playlists and Liked Songs follow the library: new songs queue, and a
//! removed song's file goes once no downloaded item needs it.
//!
//! The queue waits while the server can't be reached or Go offline is on,
//! away from the home network when "Download only on home network" is on,
//! and when the storage limit or the disk is full. It survives restarts: a
//! half-fetched file resumes by Range. Failures retry with backoff.

pub mod art;
pub mod commands;
pub mod store;
mod transfer;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::audio::StreamSource;
use crate::audio::cache::PrecacheHost;
use crate::audio::gate::TransferGate;
use crate::audio::source::LocalAudio;
use crate::db::{ArtFetcher, DbHandle, queries};
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::plex::types::ConnectionKind;

pub use store::{ItemKey, ItemKind};

const GB: u64 = 1024 * 1024 * 1024;
/// Re-read playlists and Liked Songs this often for new and removed songs.
const RESYNC_EVERY: Duration = Duration::from_secs(30);
/// How often a waiting queue looks again on its own.
const WAIT_POLL: Duration = Duration::from_secs(5);
/// Progress events at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// Space kept free on the disk beyond the song being fetched.
const DISK_MARGIN: u64 = 256 * 1024 * 1024;
pub const LIMIT_MIN_GB: u32 = 1;
pub const LIMIT_MAX_GB: u32 = 4096;

/// Download quality: the file as it is, or an MP3 from the transcoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DownloadQuality {
    #[default]
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "320")]
    Kbps320,
    #[serde(rename = "192")]
    Kbps192,
    #[serde(rename = "128")]
    Kbps128,
}

impl DownloadQuality {
    pub fn kbps(self) -> Option<u32> {
        match self {
            Self::Original => None,
            Self::Kbps320 => Some(320),
            Self::Kbps192 => Some(192),
            Self::Kbps128 => Some(128),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Kbps320 => "320",
            Self::Kbps192 => "192",
            Self::Kbps128 => "128",
        }
    }
}

/// The settings the queue follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prefs {
    pub quality: DownloadQuality,
    pub limit_bytes: u64,
    /// Fetch only over the local connection to the server.
    pub home_only: bool,
    /// Go offline is on: no network at all.
    pub offline: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            quality: DownloadQuality::Original,
            limit_bytes: 20 * GB,
            home_only: true,
            offline: false,
        }
    }
}

/// One file request.
#[derive(Debug, Clone)]
pub struct Fetch {
    pub src: StreamSource,
    /// The server honours Range (the original part). A transcode starts over.
    pub resumable: bool,
    /// Transcoder session to stop afterwards.
    pub session: Option<String>,
}

/// Where downloads come from (Plex, or a fake in tests).
#[async_trait]
pub trait DownloadSource: Send + Sync {
    /// How the server is reached now, or None when it can't be.
    async fn connection(&self) -> Option<ConnectionKind>;
    /// The request for a track: the original part, or a transcode at `kbps`.
    async fn fetch(&self, track: &Track, kbps: Option<u32>) -> AppResult<Fetch>;
    /// Called after every fetch (stops a transcode session).
    async fn finished(&self, _fetch: &Fetch) {}
}

/// Tells the UI what changed.
pub trait DownloadEvents: Send + Sync {
    /// `downloads://status`.
    fn status(&self, s: &DownloadStatus);
    /// `downloads://changed`: the set of downloaded songs changed.
    fn files_changed(&self);
}

pub type FreeSpace = Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

pub fn os_free_space() -> FreeSpace {
    Arc::new(|p: &Path| fs4::available_space(p).ok())
}

/// Why the queue is waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseReason {
    /// Go offline is on, or the server can't be reached.
    Offline,
    /// Away from home with "Download only on home network" on.
    NotHome,
    /// The storage limit is reached.
    Full,
    /// The disk has no room left.
    NoSpace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemState {
    Queued,
    Downloading,
    Downloaded,
    /// Every song that could be fetched is; some failed or went missing.
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemStatus {
    pub kind: ItemKind,
    pub id: String,
    pub title: String,
    pub state: ItemState,
    pub total: u32,
    pub done: u32,
    pub failed: u32,
    /// Finished songs whose file has gone from disk.
    pub missing: u32,
    /// 0 to 1, counting the song in flight.
    pub progress: f32,
    /// What removing it would free.
    pub bytes: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentFile {
    pub track_id: String,
    pub received: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    pub items: Vec<ItemStatus>,
    pub used_bytes: u64,
    pub limit_bytes: u64,
    /// Free space on the disk holding the folder.
    pub free_bytes: Option<u64>,
    pub folder: String,
    pub paused: Option<PauseReason>,
    /// Songs waiting to be fetched.
    pub queued: u32,
    pub current: Option<CurrentFile>,
}

/// What [`Downloads::new`] needs.
pub struct Deps {
    pub db: DbHandle,
    pub dir: PathBuf,
    pub source: Arc<dyn DownloadSource>,
    pub free_space: FreeSpace,
    /// Album art kept beside the files.
    pub art: Option<Arc<dyn ArtFetcher>>,
}

#[derive(Clone)]
struct Transfer {
    gate: Arc<TransferGate>,
    host: Option<Arc<dyn PrecacheHost>>,
    client: reqwest::Client,
}

struct Inner {
    db: DbHandle,
    dir: RwLock<PathBuf>,
    source: Arc<dyn DownloadSource>,
    free_space: FreeSpace,
    art: Option<Arc<dyn ArtFetcher>>,
    events: RwLock<Option<Arc<dyn DownloadEvents>>>,
    prefs: Mutex<Prefs>,
    transfer: RwLock<Option<Transfer>>,
    wake: Notify,
    /// Item song lists need a fresh look.
    dirty: AtomicBool,
    /// Bumped by removals, so a fetch for a song no longer wanted stops.
    epoch: AtomicU64,
    current: Mutex<Option<CurrentFile>>,
    paused: Mutex<Option<PauseReason>>,
    /// Finished files by track id (the play path reads this).
    files: RwLock<HashMap<String, PathBuf>>,
    last_progress: Mutex<Option<Instant>>,
}

/// Cheap to clone; clones share state.
#[derive(Clone)]
pub struct Downloads {
    inner: Arc<Inner>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A track id made safe for a file name.
fn file_stem(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect()
}

/// File extension for a downloaded song.
fn extension(track: &Track, quality: DownloadQuality) -> String {
    if quality.kbps().is_some() {
        return "mp3".into();
    }
    let from_part = Path::new(track.part_key.split('?').next().unwrap_or(""))
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()));
    from_part
        .or_else(|| track.container.clone().map(|c| c.to_ascii_lowercase()))
        .or_else(|| track.codec.clone().map(|c| c.to_ascii_lowercase()))
        .unwrap_or_else(|| "audio".into())
}

/// A song's likely size, for the storage check before it starts.
pub fn estimate_bytes(track: &Track, quality: DownloadQuality) -> u64 {
    let kbps = quality.kbps().or(track.bitrate).unwrap_or(1000) as u64;
    kbps * 1000 / 8 * track.duration_ms / 1000
}

impl Downloads {
    pub fn new(deps: Deps) -> Self {
        let _ = std::fs::create_dir_all(&deps.dir);
        let me = Self {
            inner: Arc::new(Inner {
                db: deps.db,
                dir: RwLock::new(deps.dir),
                source: deps.source,
                free_space: deps.free_space,
                art: deps.art,
                events: RwLock::new(None),
                prefs: Mutex::new(Prefs::default()),
                transfer: RwLock::new(None),
                wake: Notify::new(),
                dirty: AtomicBool::new(true),
                epoch: AtomicU64::new(0),
                current: Mutex::new(None),
                paused: Mutex::new(None),
                files: RwLock::new(HashMap::new()),
                last_progress: Mutex::new(None),
            }),
        };
        me.reload_files();
        me
    }

    pub fn set_events(&self, events: Arc<dyn DownloadEvents>) {
        *self.inner.events.write() = Some(events);
    }

    /// Hooks the queue to the player's transfer gate. Until then nothing downloads.
    pub fn attach(&self, gate: Arc<TransferGate>, host: Option<Arc<dyn PrecacheHost>>, client: reqwest::Client) {
        *self.inner.transfer.write() = Some(Transfer { gate, host, client });
        self.inner.wake.notify_one();
    }

    pub fn dir(&self) -> PathBuf {
        self.inner.dir.read().clone()
    }

    pub fn prefs(&self) -> Prefs {
        *self.inner.prefs.lock()
    }

    pub fn set_prefs(&self, p: Prefs) {
        let mut cur = self.inner.prefs.lock();
        if *cur == p {
            return;
        }
        *cur = p;
        drop(cur);
        self.inner.wake.notify_one();
        self.emit_status();
    }

    /// The library may have changed (sync, likes, playlist edits).
    pub fn poke(&self) {
        self.inner.dirty.store(true, Ordering::SeqCst);
        self.inner.wake.notify_one();
    }

    fn events(&self) -> Option<Arc<dyn DownloadEvents>> {
        self.inner.events.read().clone()
    }

    fn emit_status(&self) {
        if let Some(ev) = self.events()
            && let Ok(s) = self.status()
        {
            ev.status(&s);
        }
    }

    fn files_changed(&self) {
        self.reload_files();
        if let Some(ev) = self.events() {
            ev.files_changed();
        }
        self.emit_status();
    }

    fn reload_files(&self) {
        let dir = self.dir();
        let files = self.inner.db.read(store::done_files).unwrap_or_default();
        *self.inner.files.write() = files.into_iter().map(|(id, f)| (id, dir.join(f))).collect();
    }

    /// Ids of every downloaded song.
    pub fn downloaded_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.inner.files.read().keys().cloned().collect();
        ids.sort();
        ids
    }

    // ---------------------------------------------------------------- items

    /// Turns an item on. Albums, playlists and Liked Songs take their songs
    /// from the library cache; a mix needs `track_ids`.
    pub async fn add(&self, key: ItemKey, title: String, track_ids: Option<Vec<String>>) -> AppResult<()> {
        let liked_min = crate::settings::load(&self.inner.db).liked_min_rating();
        let k = key.clone();
        let deleted = self
            .inner
            .db
            .write_async(move |c| {
                let tracks = match (k.kind, track_ids) {
                    (ItemKind::Mix, Some(ids)) => ids,
                    (ItemKind::Mix, None) => return Err(AppError::Invalid("A mix needs its songs.".into())),
                    // The page's own list wins (a playlist not synced yet).
                    (_, ids) => match store::tracks_of_library_item(c, &k, liked_min)? {
                        Some(t) => t,
                        None => ids.ok_or_else(|| AppError::NotFound("That isn't in your library.".into()))?,
                    },
                };
                let tx = c.transaction()?;
                store::set_item(&tx, &k, &title, &tracks, now())?;
                let deleted = store::reconcile(&tx)?;
                tx.commit()?;
                Ok(deleted)
            })
            .await?;
        self.delete_files(deleted);
        self.inner.wake.notify_one();
        self.files_changed();
        Ok(())
    }

    /// Turns an item off; songs no other item needs are deleted.
    pub async fn remove(&self, key: ItemKey) -> AppResult<()> {
        let deleted = self
            .inner
            .db
            .write_async(move |c| {
                let tx = c.transaction()?;
                store::remove_item(&tx, &key)?;
                let deleted = store::reconcile(&tx)?;
                tx.commit()?;
                Ok(deleted)
            })
            .await?;
        self.inner.epoch.fetch_add(1, Ordering::SeqCst);
        self.delete_files(deleted);
        self.files_changed();
        Ok(())
    }

    /// Deletes every download. Returns the bytes freed.
    pub async fn remove_all(&self) -> AppResult<u64> {
        let before = self.inner.db.read_async(store::used_bytes).await?;
        let deleted = self.inner.db.write_async(|c| store::remove_all(c)).await?;
        self.inner.epoch.fetch_add(1, Ordering::SeqCst);
        self.delete_files(deleted);
        // Leftover partial files and art.
        let dir = self.dir();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "part") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let _ = std::fs::remove_dir_all(dir.join(art::ART_DIR));
        self.files_changed();
        Ok(before)
    }

    /// Failed and missing songs (of one item, or all) try again.
    pub async fn retry(&self, key: Option<ItemKey>) -> AppResult<()> {
        self.inner.db.write_async(move |c| store::retry(c, key.as_ref())).await?;
        self.inner.wake.notify_one();
        self.emit_status();
        Ok(())
    }

    pub async fn is_on(&self, key: ItemKey) -> AppResult<bool> {
        self.inner.db.read_async(move |c| store::has_item(c, &key)).await
    }

    fn delete_files(&self, names: Vec<String>) {
        let dir = self.dir();
        for n in names {
            let p = dir.join(&n);
            if let Err(e) = std::fs::remove_file(&p)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!("could not delete download {n}: {e}");
            }
            let _ = std::fs::remove_file(dir.join(format!("{n}.part")));
        }
    }

    /// Re-reads the songs of downloaded albums, playlists and Liked Songs
    /// from the library cache, queueing new songs and dropping removed ones.
    pub async fn resync(&self) -> AppResult<()> {
        self.inner.dirty.store(false, Ordering::SeqCst);
        let liked_min = crate::settings::load(&self.inner.db).liked_min_rating();
        let (deleted, changed) = self
            .inner
            .db
            .write_async(move |c| {
                let tx = c.transaction()?;
                let mut changed = false;
                for item in store::items(&tx)? {
                    if let Some(now_ids) = store::tracks_of_library_item(&tx, &item.key, liked_min)?
                        && now_ids != store::item_tracks(&tx, &item.key)?
                    {
                        store::set_tracks(&tx, &item.key, &now_ids)?;
                        changed = true;
                    }
                }
                let before: i64 = tx.query_row("SELECT COUNT(*) FROM download_files WHERE state = 'queued'", [], |r| r.get(0))?;
                let deleted = store::reconcile(&tx)?;
                let after: i64 = tx.query_row("SELECT COUNT(*) FROM download_files WHERE state = 'queued'", [], |r| r.get(0))?;
                tx.commit()?;
                Ok((deleted, changed || before != after))
            })
            .await?;
        let any_deleted = !deleted.is_empty();
        self.delete_files(deleted);
        if any_deleted {
            self.inner.epoch.fetch_add(1, Ordering::SeqCst);
            self.files_changed();
        } else if changed {
            self.emit_status();
        }
        Ok(())
    }

    /// Finished files that are gone from disk become "missing" (offered
    /// again in the UI). Returns how many.
    pub fn check_files(&self) -> usize {
        let gone: Vec<String> = self
            .inner
            .files
            .read()
            .iter()
            .filter(|(_, p)| !p.is_file())
            .map(|(id, _)| id.clone())
            .collect();
        if gone.is_empty() {
            return 0;
        }
        let n = self
            .inner
            .db
            .write(|c| {
                let mut n = 0;
                for id in &gone {
                    n += store::mark_missing(c, id)? as usize;
                }
                Ok(n)
            })
            .unwrap_or(0);
        self.files_changed();
        n
    }

    /// Moves every file to `to` and uses it from now on.
    pub async fn move_to(&self, to: PathBuf) -> AppResult<()> {
        let from = self.dir();
        if from == to {
            return Ok(());
        }
        std::fs::create_dir_all(&to)?;
        let names: Vec<String> = self
            .inner
            .db
            .read_async(store::done_files)
            .await?
            .into_iter()
            .map(|(_, f)| f)
            .collect();
        let (f2, t2) = (from.clone(), to.clone());
        tokio::task::spawn_blocking(move || -> AppResult<()> {
            for n in names {
                move_file(&f2.join(&n), &t2.join(&n))?;
            }
            let art_from = f2.join(art::ART_DIR);
            if art_from.is_dir() {
                let art_to = t2.join(art::ART_DIR);
                std::fs::create_dir_all(&art_to)?;
                for e in std::fs::read_dir(&art_from)?.flatten() {
                    move_file(&e.path(), &art_to.join(e.file_name()))?;
                }
                let _ = std::fs::remove_dir(&art_from);
            }
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
        *self.inner.dir.write() = to;
        self.inner.epoch.fetch_add(1, Ordering::SeqCst);
        self.files_changed();
        Ok(())
    }

    // ---------------------------------------------------------------- status

    pub fn status(&self) -> AppResult<DownloadStatus> {
        let current = self.inner.current.lock().clone();
        let prefs = self.prefs();
        let paused = *self.inner.paused.lock();
        let cur_id = current.as_ref().map(|c| c.track_id.clone());
        let fraction = current
            .as_ref()
            .and_then(|c| c.total.filter(|t| *t > 0).map(|t| (c.received as f64 / t as f64).min(1.0)))
            .unwrap_or(0.0);
        let (items, used, queued) = self.inner.db.read(|c| {
            let mut out = Vec::new();
            for item in store::items(c)? {
                let n = store::item_counts(c, &item.key)?;
                let has_current = match &cur_id {
                    Some(id) => c
                        .query_row(
                            "SELECT 1 FROM download_item_tracks WHERE kind = ?1 AND id = ?2 AND track_id = ?3",
                            rusqlite::params![item.key.kind.as_str(), item.key.id, id],
                            |_| Ok(()),
                        )
                        .is_ok(),
                    None => false,
                };
                let settled = n.done + n.failed + n.missing;
                let state = if n.done >= n.total {
                    ItemState::Downloaded
                } else if settled >= n.total {
                    ItemState::Error
                } else if has_current || (n.done > 0 && paused.is_none()) {
                    ItemState::Downloading
                } else {
                    ItemState::Queued
                };
                let progress = if n.total == 0 {
                    1.0
                } else {
                    ((n.done as f64 + if has_current { fraction } else { 0.0 }) / n.total as f64).min(1.0) as f32
                };
                out.push(ItemStatus {
                    kind: item.key.kind,
                    id: item.key.id,
                    title: item.title,
                    state,
                    total: n.total,
                    done: n.done,
                    failed: n.failed,
                    missing: n.missing,
                    progress,
                    bytes: n.own_bytes,
                    error: n.error,
                });
            }
            Ok((out, store::used_bytes(c)?, store::queued_count(c, now())?))
        })?;
        let dir = self.dir();
        Ok(DownloadStatus {
            items,
            used_bytes: used,
            limit_bytes: prefs.limit_bytes,
            free_bytes: (self.inner.free_space)(&dir),
            folder: dir.to_string_lossy().into_owned(),
            paused,
            queued,
            current,
        })
    }

    fn set_paused(&self, p: Option<PauseReason>) {
        let changed = {
            let mut cur = self.inner.paused.lock();
            let changed = *cur != p;
            *cur = p;
            changed
        };
        if changed {
            self.emit_status();
        }
    }

    fn set_current(&self, c: Option<CurrentFile>, force: bool) {
        *self.inner.current.lock() = c;
        let mut last = self.inner.last_progress.lock();
        let due = force || last.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
        if due {
            *last = Some(Instant::now());
            drop(last);
            self.emit_status();
        }
    }

    // ---------------------------------------------------------------- queue

    /// Why the queue can't fetch now, if it can't.
    async fn blocked(&self) -> Option<PauseReason> {
        let p = self.prefs();
        if p.offline {
            return Some(PauseReason::Offline);
        }
        match self.inner.source.connection().await {
            None => Some(PauseReason::Offline),
            Some(ConnectionKind::Local) => None,
            Some(_) if p.home_only => Some(PauseReason::NotHome),
            Some(_) => None,
        }
    }

    async fn wait(&self, max: Duration) {
        let _ = tokio::time::timeout(max, self.inner.wake.notified()).await;
    }

    /// The queue's loop. Runs until the app quits.
    pub async fn run(self) {
        // Files deleted while the app was closed.
        if self.check_files() > 0 {
            tracing::info!("some downloaded files are missing");
        }
        let mut last_sync: Option<Instant> = None;
        loop {
            if self.inner.dirty.load(Ordering::SeqCst) || last_sync.is_none_or(|t| t.elapsed() >= RESYNC_EVERY) {
                if let Err(e) = self.resync().await {
                    tracing::warn!("downloads resync failed: {e}");
                }
                last_sync = Some(Instant::now());
            }
            if let Some(why) = self.blocked().await {
                self.set_paused(Some(why));
                self.wait(WAIT_POLL).await;
                continue;
            }
            if self.inner.transfer.read().is_none() {
                self.wait(WAIT_POLL).await;
                continue;
            }
            match self.step().await {
                Ok(Step::Idle) => {
                    self.set_paused(None);
                    let retry = self
                        .inner
                        .db
                        .read_async(|c| store::next_retry_in(c, now()))
                        .await
                        .ok()
                        .flatten()
                        .map(|s| Duration::from_secs(s.max(1) as u64));
                    self.wait(retry.map_or(RESYNC_EVERY, |r| r.min(RESYNC_EVERY))).await;
                }
                Ok(Step::Worked) => {}
                Ok(Step::Paused(why)) => {
                    self.set_paused(Some(why));
                    self.wait(WAIT_POLL).await;
                }
                Err(e) => {
                    tracing::warn!("downloads: {e}");
                    self.wait(WAIT_POLL).await;
                }
            }
        }
    }

    /// Fetches the next queued song, if any. Public for tests.
    pub async fn step(&self) -> AppResult<Step> {
        let Some(track_id) = self.inner.db.read_async(|c| store::next_queued(c, now())).await? else {
            return Ok(Step::Idle);
        };
        let id = track_id.clone();
        let track = self
            .inner
            .db
            .read_async(move |c| queries::tracks_by_ids(c, &[id]))
            .await?
            .into_iter()
            .next();
        let Some(track) = track else {
            let id = track_id.clone();
            self.inner
                .db
                .write_async(move |c| store::mark_failed(c, &id, "This song is no longer in your library.", now()))
                .await?;
            self.emit_status();
            return Ok(Step::Worked);
        };
        let prefs = self.prefs();
        // Room for it under the limit and on the disk.
        let estimate = estimate_bytes(&track, prefs.quality);
        let used = self.inner.db.read_async(store::used_bytes).await?;
        if used + estimate > prefs.limit_bytes {
            return Ok(Step::Paused(PauseReason::Full));
        }
        let dir = self.dir();
        if (self.inner.free_space)(&dir).is_some_and(|free| free < estimate + DISK_MARGIN) {
            return Ok(Step::Paused(PauseReason::NoSpace));
        }
        self.set_paused(None);
        let Some(xfer) = self.inner.transfer.read().clone() else {
            return Ok(Step::Idle);
        };
        let fetch = match self.inner.source.fetch(&track, prefs.quality.kbps()).await {
            Ok(f) => f,
            Err(e) => {
                let msg = e.to_string();
                let id = track.id.clone();
                self.inner.db.write_async(move |c| store::mark_failed(c, &id, &msg, now())).await?;
                self.emit_status();
                return Ok(Step::Worked);
            }
        };
        let name = format!("{}.{}", file_stem(&track.id), extension(&track, prefs.quality));
        let part = dir.join(format!("{name}.part"));
        // A partial file from another quality can't be continued.
        if !fetch.resumable {
            let _ = tokio::fs::remove_file(&part).await;
        }
        let epoch = self.inner.epoch.load(Ordering::SeqCst);
        self.set_current(Some(CurrentFile { track_id: track.id.clone(), received: 0, total: None }), true);
        let room = prefs.limit_bytes.saturating_sub(used);
        let outcome = {
            let me = self.clone();
            let id = track.id.clone();
            let ctx = transfer::Ctx {
                client: &xfer.client,
                gate: &xfer.gate,
                host: xfer.host.as_deref(),
                room,
                keep_going: &|| me.keep_going(epoch, &id),
                progress: &|received, total| {
                    me.set_current(Some(CurrentFile { track_id: id.clone(), received, total }), false)
                },
            };
            transfer::fetch_file(&ctx, &fetch, &part).await
        };
        self.inner.source.finished(&fetch).await;
        self.set_current(None, false);
        let id = track.id.clone();
        match outcome {
            transfer::Outcome::Done(size) => {
                let fin = dir.join(&name);
                if let Err(e) = std::fs::rename(&part, &fin) {
                    let msg = format!("Couldn't save the file: {e}");
                    self.inner.db.write_async(move |c| store::mark_failed(c, &id, &msg, now())).await?;
                    self.emit_status();
                    return Ok(Step::Worked);
                }
                let (pk, q, n) = (track.part_key.clone(), prefs.quality.as_str().to_string(), name.clone());
                let old = self
                    .inner
                    .db
                    .write_async(move |c| store::mark_done(c, &id, &pk, &q, &n, size, now()))
                    .await?;
                if let Some(old) = old {
                    self.delete_files(vec![old]);
                }
                if let (Some(art), Some(thumb)) = (&self.inner.art, &track.thumb) {
                    art::keep(art.as_ref(), &dir, thumb).await;
                }
                self.files_changed();
                Ok(Step::Worked)
            }
            transfer::Outcome::Failed(msg) => {
                self.inner.db.write_async(move |c| store::mark_failed(c, &id, &msg, now())).await?;
                self.emit_status();
                Ok(Step::Worked)
            }
            transfer::Outcome::Full => {
                let _ = tokio::fs::remove_file(&part).await;
                Ok(Step::Paused(PauseReason::Full))
            }
            transfer::Outcome::Stopped => {
                // No longer wanted (deleted with its row), or the queue paused
                // (the partial file stays for later).
                if !self.still_wanted(&id) {
                    let _ = tokio::fs::remove_file(&part).await;
                }
                self.emit_status();
                Ok(Step::Worked)
            }
        }
    }

    fn still_wanted(&self, track_id: &str) -> bool {
        self.inner.db.read(|c| store::file(c, track_id)).ok().flatten().is_some()
    }

    /// Whether a fetch in flight should carry on.
    fn keep_going(&self, epoch: u64, track_id: &str) -> bool {
        let p = self.prefs();
        if p.offline {
            return false;
        }
        if self.inner.epoch.load(Ordering::SeqCst) != epoch && !self.still_wanted(track_id) {
            return false;
        }
        true
    }
}

/// What one step of the queue did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Idle,
    Worked,
    Paused(PauseReason),
}

fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.exists() {
        return Ok(());
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    // Another disk: copy, then delete.
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

impl LocalAudio for Downloads {
    fn path(&self, track: &Track) -> Option<PathBuf> {
        let p = self.inner.files.read().get(&track.id).cloned()?;
        if p.is_file() {
            return Some(p);
        }
        // Gone from disk: say so (the UI offers to download it again).
        let id = track.id.clone();
        if self.inner.db.write(|c| store::mark_missing(c, &id)).unwrap_or(false) {
            self.files_changed();
        }
        None
    }

    fn ids(&self) -> Vec<String> {
        self.downloaded_ids()
    }
}
