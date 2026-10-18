//! Downloads: albums, playlists, Liked Songs and mixes kept on this computer
//! for playing without the server.
//!
//! The user turns items on; each item's songs go in a queue that fetches one
//! file at a time through the player's transfer gate at background priority,
//! so playback always comes first. A file is the Plex part as it is, or an
//! MP3 from the server's transcoder when the quality setting says so. Files
//! live in one folder, named by track id, shared by every item that holds the
//! song. Playlists, Liked Songs and mixes follow the library: new songs
//! queue, and a removed song's file goes once no downloaded item needs it. A
//! song replaced on the server keeps playing from its old file until the new
//! one lands. The app pokes the queue on every library or mix change
//! ([`FOLLOW_EVENTS`]), so this happens within a moment; a slow re-read
//! every [`RESYNC_EVERY`] catches the rest.
//!
//! The queue waits while the server can't be reached or Go offline is on,
//! away from the home network when "Download only on home network" is on,
//! and when the storage limit or the disk is full. It survives restarts: a
//! half-fetched file resumes by Range. Failures retry with backoff.

pub mod art;
pub mod commands;
pub mod store;
pub(crate) mod transfer;

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
/// App events after which downloaded playlists, albums, Liked Songs and
/// mixes are read again (live library updates, playlist edits, likes, mixes
/// made again).
pub const FOLLOW_EVENTS: [&str; 3] = ["library://changed", "likes://changed", "mixes://changed"];
/// Re-read playlists and Liked Songs this often for new and removed songs.
const RESYNC_EVERY: Duration = Duration::from_secs(30);
/// How often a waiting queue looks again on its own.
const WAIT_POLL: Duration = Duration::from_secs(5);
/// How often finished files are checked on disk, and deletes that failed
/// (a file in use) are tried again.
const CHECK_EVERY: Duration = Duration::from_secs(60);
/// How far down the queue a step looks for a song that fits and whose
/// server can be reached.
const CANDIDATES: usize = 50;
/// Of those, how many servers it asks about.
const REACH_TRIES: usize = 8;
/// Files whose delete failed, tried again later (kv, JSON list of names).
const KEY_PENDING_DELETE: &str = "downloads.pendingDelete";
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
    /// How the server holding `track` is reached now.
    async fn connection_for(&self, _track: &Track) -> Option<ConnectionKind> {
        self.connection().await
    }
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
    /// The downloads folder can't be reached (its drive is unplugged).
    FolderMissing,
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
    /// Songs that turned out too big for the room left, with the (limit,
    /// bytes used) they were too big for. They aren't fetched again until
    /// one changes.
    too_big: Mutex<Option<TooBig>>,
    /// How often [`CHECK_EVERY`] runs (shorter in tests).
    check_every: Mutex<Duration>,
    /// Held by each queue step and by a folder move, so a move never runs
    /// while a song is being saved.
    step_lock: tokio::sync::Mutex<()>,
    /// A folder move is under way: fetches stop and wait for it.
    moving: AtomicBool,
}

/// (limit, bytes used) and the songs too big for that room.
type TooBig = ((u64, u64), std::collections::HashSet<String>);

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

/// FNV-1a, 32 bits, as 8 hex digits.
fn short_hash(s: &str) -> String {
    let mut h: u32 = 0x811c9dc5;
    for b in s.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    format!("{h:08x}")
}

/// A track id made safe for a file name. An id that had to change (other
/// characters, or too long) gets a hash of the whole id, so two ids never
/// share a file.
fn file_stem(id: &str) -> String {
    let clean: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect();
    if clean == id { clean } else { format!("{clean}-{}", short_hash(id)) }
}

/// The partial file for `name` while it comes from `part_key`. A partial
/// file of another version of the song (the file changed on the server) has
/// another name, so it is never continued.
fn part_name(name: &str, part_key: &str) -> String {
    format!("{name}.{}.part", short_hash(part_key))
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `s` is something [`file_stem`] can make: the id as it is (up to
/// 64 safe characters), or the cleaned id plus a dash and its hash.
fn is_stem(s: &str) -> bool {
    let safe = |t: &str| t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if s.is_empty() || !safe(s) {
        return false;
    }
    if s.len() <= 64 {
        return true;
    }
    s.len() <= 73 && s.rsplit_once('-').is_some_and(|(clean, h)| clean.len() <= 64 && is_lower_hex(h, 8))
}

/// Whether `s` is something [`extension`] can give.
fn is_ext(s: &str) -> bool {
    (1..=5).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// The stem of a partial file this app made (`{stem}.{ext}.{hash}.part`, as
/// [`file_stem`], [`extension`] and [`part_name`] make it). None for any
/// other name, so files that aren't ours are never taken for ours.
fn part_stem(file: &str) -> Option<&str> {
    let rest = file.strip_suffix(".part")?;
    let (rest, hash) = rest.rsplit_once('.')?;
    let (stem, ext) = rest.rsplit_once('.')?;
    (is_lower_hex(hash, 8) && is_ext(ext) && is_stem(stem)).then_some(stem)
}

/// Whether `file` is a partial file of the song stored as `stem`.
fn is_part_of(file: &str, stem: &str) -> bool {
    part_stem(file) == Some(stem)
}

/// Whether a stored file name is one plain name in the folder (no folders,
/// `..`, drive or root), so joining it to the folder stays in the folder.
fn is_plain_name(name: &str) -> bool {
    use std::path::Component;
    if name.is_empty() || name.contains(['/', '\\', ':']) {
        return false;
    }
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(Component::Normal(n)), None) if n == name)
}

/// The file in `dir` a stored name stands for, or None (and a warning) for a
/// name that isn't one plain name.
fn stored_path(dir: &Path, name: &str) -> Option<PathBuf> {
    if is_plain_name(name) {
        Some(dir.join(name))
    } else {
        tracing::warn!("ignoring a stored download name that isn't a plain file name");
        None
    }
}

/// Left in a downloads folder this app made or uses. Nothing in a folder
/// without it is ever swept.
pub const MARKER: &str = ".platter-downloads";

fn has_marker(dir: &Path) -> bool {
    dir.join(MARKER).is_file()
}

fn write_marker(dir: &Path) {
    if let Err(e) = std::fs::write(dir.join(MARKER), b"Platter keeps its downloads in this folder.\n") {
        tracing::warn!("could not mark the downloads folder: {e}");
    }
}

/// Makes the folder if it isn't there, marking it as ours when it is new.
fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    write_marker(dir);
    Ok(())
}

/// File extension for a downloaded song.
fn extension(track: &Track, quality: DownloadQuality) -> String {
    if quality.kbps().is_some() {
        return "mp3".into();
    }
    let clean = |e: &str| Some(e.to_ascii_lowercase()).filter(|e| is_ext(e));
    let from_part = Path::new(track.part_key.split('?').next().unwrap_or(""))
        .extension()
        .and_then(|e| e.to_str())
        .and_then(clean);
    from_part
        .or_else(|| track.container.as_deref().and_then(clean))
        .or_else(|| track.codec.as_deref().and_then(clean))
        .unwrap_or_else(|| "audio".into())
}

/// A song's likely size, for the storage check before it starts.
pub fn estimate_bytes(track: &Track, quality: DownloadQuality) -> u64 {
    let kbps = quality.kbps().or(track.bitrate).unwrap_or(1000) as u64;
    kbps * 1000 / 8 * track.duration_ms / 1000
}

impl Downloads {
    pub fn new(deps: Deps) -> Self {
        let _ = ensure_dir(&deps.dir);
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
                too_big: Mutex::new(None),
                check_every: Mutex::new(CHECK_EVERY),
                step_lock: tokio::sync::Mutex::new(()),
                moving: AtomicBool::new(false),
            }),
        };
        me.reload_files();
        me.adopt_folder();
        me
    }

    /// A folder from before folders were marked is taken as ours once one of
    /// the files this app saved there is found in it.
    fn adopt_folder(&self) {
        let dir = self.dir();
        if dir.is_dir() && !has_marker(&dir) && self.inner.files.read().values().any(|p| p.is_file()) {
            write_marker(&dir);
        }
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
        *self.inner.files.write() =
            files.into_iter().filter_map(|(id, f)| Some((id, stored_path(&dir, &f)?))).collect();
    }

    /// Ids of every downloaded song. None while the folder can't be reached
    /// (an unplugged drive): nothing there can play.
    pub fn downloaded_ids(&self) -> Vec<String> {
        if !self.dir().is_dir() {
            return Vec::new();
        }
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
                    // A mix follows its stored songs (what resync reads too);
                    // a radio, which isn't stored, keeps the page's.
                    (ItemKind::Mix, ids) => match store::tracks_of_library_item(c, &k, liked_min)? {
                        Some(t) if !t.is_empty() && (ids.is_none() || store::library_list_complete(c, &k)?) => t,
                        _ => ids.ok_or_else(|| AppError::Invalid("A mix needs its songs.".into()))?,
                    },
                    // The library's list wins; the page's own list when it
                    // isn't there yet (a playlist not synced).
                    // Half read (a resync under way): the page's list, else
                    // what's already downloaded stays as it is.
                    (_, ids) => match store::tracks_of_library_item(c, &k, liked_min)? {
                        Some(t) if !t.is_empty() && store::library_list_complete(c, &k)? => t,
                        found => match ids {
                            Some(ids) => ids,
                            None if store::has_item(c, &k)? => store::item_tracks(c, &k)?,
                            None => found.ok_or_else(|| AppError::NotFound("That isn't in your library.".into()))?,
                        },
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
        // Leftover partial files and art, only in a folder that is ours and
        // only by the names this app gives them.
        let dir = self.dir();
        if has_marker(&dir) {
            let mut failed = Vec::new();
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if part_stem(&name).is_some() && e.path().is_file() {
                    remove_logged(&e.path(), &name, &mut failed);
                }
            }
            let art_dir = dir.join(art::ART_DIR);
            for e in std::fs::read_dir(&art_dir).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if art::is_art_name(&name) && e.path().is_file() {
                    remove_logged(&e.path(), &name, &mut Vec::new());
                }
            }
            // Only goes when empty: anything else in it stays.
            let _ = std::fs::remove_dir(&art_dir);
            self.queue_pending(&dir, failed);
        }
        self.retry_pending_deletes();
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

    /// File names and stems some row still uses, which no delete may touch.
    fn in_use(&self) -> AppResult<InUse> {
        let rows = self.inner.db.read(store::row_files)?;
        let mut used = InUse::default();
        for (id, file) in rows {
            used.stems.insert(file_stem(&id));
            if let Some(f) = file {
                used.names.insert(f);
            }
        }
        Ok(used)
    }

    /// Deletes the files of rows no item needs: the finished file and any
    /// partial ones. A file that can't be deleted now (in use) is tried again
    /// later ([`Self::retry_pending_deletes`]). Names a row uses again are
    /// left alone.
    fn delete_files(&self, gone: Vec<store::Gone>) {
        if gone.is_empty() {
            return;
        }
        let used = match self.in_use() {
            Ok(u) => u,
            Err(e) => {
                // Not deleted now, not forgotten either: the retry checks
                // again once the library can be read.
                tracing::warn!("downloads: not deleting yet, the library can't be read: {e}");
                let names = gone.iter().filter_map(|g| g.file.clone()).filter(|n| is_plain_name(n)).collect();
                self.queue_pending(&self.dir(), names);
                return;
            }
        };
        let dir = self.dir();
        let mut failed = Vec::new();
        for g in &gone {
            if let Some(n) = &g.file
                && !used.names.contains(n)
                && let Some(p) = stored_path(&dir, n)
            {
                remove_logged(&p, n, &mut failed);
            }
        }
        let stems: std::collections::HashSet<String> =
            gone.iter().map(|g| file_stem(&g.track_id)).filter(|s| !used.stems.contains(s)).collect();
        if !stems.is_empty() {
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if part_stem(&name).is_some_and(|s| stems.contains(s)) && e.path().is_file() {
                    remove_logged(&e.path(), &name, &mut failed);
                }
            }
        }
        self.queue_pending(&dir, failed);
    }

    /// Keeps names whose delete failed, with their folder, for later.
    fn queue_pending(&self, dir: &Path, names: Vec<String>) {
        if names.is_empty() {
            return;
        }
        let folder = dir.to_string_lossy().into_owned();
        let res = self.inner.db.write(move |c| {
            let mut pending = pending_deletes(c)?;
            pending.extend(names.into_iter().map(|name| PendingDelete { dir: folder.clone(), name }));
            pending.sort();
            pending.dedup();
            crate::db::store::kv_set(c, KEY_PENDING_DELETE, &pending_json(&pending))
        });
        if let Err(e) = res {
            tracing::warn!("downloads: could not note files to delete later: {e}");
        }
    }

    /// Tries again to delete files whose delete failed, each in the folder it
    /// was in, unless a song uses that name again. Returns how many are
    /// still left. When the library can't be read nothing is deleted.
    pub fn retry_pending_deletes(&self) -> usize {
        let pending = match self.inner.db.read(pending_deletes) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("downloads: files to delete later can't be read: {e}");
                return 0;
            }
        };
        if pending.is_empty() {
            return 0;
        }
        let used = match self.in_use() {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("downloads: not deleting, the library can't be read: {e}");
                return pending.len();
            }
        };
        let left: Vec<PendingDelete> = pending
            .into_iter()
            .filter(|p| {
                if used.names.contains(&p.name) || part_stem(&p.name).is_some_and(|s| used.stems.contains(s)) {
                    return false;
                }
                let dir = PathBuf::from(&p.dir);
                if !dir.is_dir() {
                    // Its drive is away: try again when it is back.
                    return true;
                }
                let Some(path) = stored_path(&dir, &p.name) else { return false };
                match std::fs::remove_file(path) {
                    Ok(()) => false,
                    Err(e) => e.kind() != std::io::ErrorKind::NotFound,
                }
            })
            .collect();
        let n = left.len();
        let res = self.inner.db.write(move |c| crate::db::store::kv_set(c, KEY_PENDING_DELETE, &pending_json(&left)));
        if let Err(e) = res {
            tracing::warn!("downloads: could not note files to delete later: {e}");
        }
        n
    }

    /// At startup: partial files no row needs (from an item removed before
    /// they finished, or an older version), deletes left over, and album art
    /// no downloaded song uses. Runs only in a folder that is ours, only on
    /// names this app gives, and not at all while the library holds no
    /// downloads (a new, reset or restored library knows nothing of them).
    pub fn sweep(&self) {
        let dir = self.dir();
        if !dir.is_dir() {
            return;
        }
        if !has_marker(&dir) {
            tracing::info!("downloads folder isn't marked as Platter's; not sweeping it");
            return;
        }
        let Ok((rows, items, tracks)) = self.inner.db.read(|c| {
            let mut st =
                c.prepare("SELECT f.track_id, t.thumb, t.id IS NOT NULL FROM download_files f LEFT JOIN tracks t ON t.id = f.track_id")?;
            let rows = st
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, bool>(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            let items: i64 = c.query_row("SELECT COUNT(*) FROM download_items", [], |r| r.get(0))?;
            let tracks: i64 = c.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?;
            Ok((rows, items, tracks))
        }) else {
            return;
        };
        if rows.is_empty() && items == 0 {
            return;
        }
        self.retry_pending_deletes();
        let stems: std::collections::HashSet<String> = rows.iter().map(|(id, _, _)| file_stem(id)).collect();
        let mut failed = Vec::new();
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if part_stem(&name).is_some_and(|s| !stems.contains(s)) && e.path().is_file() {
                remove_logged(&e.path(), &name, &mut failed);
            }
        }
        self.queue_pending(&dir, failed);
        // A downloaded song the library doesn't have (not in it yet, or
        // mid resync) means which covers are used isn't known: keep them
        // all. A song that simply has no cover needs none.
        if tracks == 0 || rows.iter().any(|(_, _, known)| !known) {
            return;
        }
        let keep: std::collections::HashSet<String> =
            rows.iter().filter_map(|(_, t, _)| t.as_deref()).map(art::file_name).collect();
        for e in std::fs::read_dir(dir.join(art::ART_DIR)).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if art::is_art_name(&name) && !keep.contains(&name) && e.path().is_file() {
                remove_logged(&e.path(), &name, &mut Vec::new());
            }
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
                    // An empty answer is far more likely a half-read library
                    // than an item the user emptied, so its files stay until
                    // the library lists songs for it again.
                    // A list with songs missing (the library read again
                    // halfway) waits too.
                    if let Some(now_ids) = store::tracks_of_library_item(&tx, &item.key, liked_min)?
                        && !now_ids.is_empty()
                        && store::library_list_complete(&tx, &item.key)?
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
        // The whole folder gone is an unplugged drive or a network folder
        // not there yet, not every file deleted: leave them be.
        if !self.dir().is_dir() {
            tracing::warn!("downloads folder unavailable; not checking its files");
            return 0;
        }
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

    /// Moves every file to `to` and uses it from now on. The queue waits
    /// meanwhile. If a file can't be moved, the ones already moved go back
    /// and the folder stays as it was.
    pub async fn move_to(&self, to: PathBuf) -> AppResult<()> {
        let from = self.dir();
        if from == to {
            return Ok(());
        }
        self.inner.moving.store(true, Ordering::SeqCst);
        let res = async {
            // Waits for a song being fetched to stop (keep_going says no).
            let _step = self.inner.step_lock.lock().await;
            ensure_dir(&to)?;
            let mut names: Vec<String> = self
                .inner
                .db
                .read_async(store::done_files)
                .await?
                .into_iter()
                .map(|(_, f)| f)
                .filter(|f| is_plain_name(f))
                .collect();
            names.sort();
            let (f2, t2) = (from.clone(), to.clone());
            tokio::task::spawn_blocking(move || move_all(&f2, &t2, &names))
                .await
                .map_err(|e| AppError::Internal(e.to_string()))??;
            // The folder the user picked holds the downloads from now on.
            if !has_marker(&to) {
                write_marker(&to);
            }
            *self.inner.dir.write() = to;
            self.inner.epoch.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        .await;
        self.inner.moving.store(false, Ordering::SeqCst);
        self.files_changed();
        self.inner.wake.notify_one();
        res
    }

    /// Moves the files to `to`, then saves the new folder with `save`. When
    /// saving fails the files go back, so the saved folder and the files
    /// never part ways.
    pub async fn move_and_save<F, Fut>(&self, to: PathBuf, save: F) -> AppResult<()>
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = AppResult<()>>,
    {
        let before = self.dir();
        self.move_to(to.clone()).await?;
        if let Err(e) = save(to.to_string_lossy().into_owned()).await {
            if let Err(back) = self.move_to(before).await {
                tracing::warn!("downloads not moved back after the folder couldn't be saved: {back}");
            }
            return Err(e);
        }
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
            let mut counts = store::all_item_counts(c)?;
            let holding = match &cur_id {
                Some(id) => store::items_holding(c, id)?,
                None => Default::default(),
            };
            for item in store::items(c)? {
                let k = (item.key.kind.as_str().to_string(), item.key.id.clone());
                let n = counts.remove(&k).unwrap_or_default();
                let has_current = holding.contains(&k);
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

    /// Why the queue can't fetch anything now, if it can't. Whether a song's
    /// own server can be reached is asked per song ([`Self::unreachable`]).
    async fn blocked(&self) -> Option<PauseReason> {
        if self.prefs().offline {
            return Some(PauseReason::Offline);
        }
        let dir = self.dir();
        if !dir.is_dir() && ensure_dir(&dir).is_err() {
            return Some(PauseReason::FolderMissing);
        }
        None
    }

    /// Why `track` can't be fetched from its server now, if it can't.
    async fn unreachable(&self, track: &Track) -> Option<PauseReason> {
        let p = self.prefs();
        if p.offline {
            return Some(PauseReason::Offline);
        }
        match self.inner.source.connection_for(track).await {
            None => Some(PauseReason::Offline),
            Some(ConnectionKind::Local) => None,
            Some(_) if p.home_only => Some(PauseReason::NotHome),
            Some(_) => None,
        }
    }

    /// Tests check the disk this often.
    #[cfg(test)]
    pub fn set_check_every(&self, d: Duration) {
        *self.inner.check_every.lock() = d;
    }

    /// Waits up to `max`, but never past the next disk check.
    async fn nap(&self, max: Duration) {
        let every = *self.inner.check_every.lock();
        self.wait(max.min(every)).await;
    }

    /// Whether anything is downloaded or set to download. Public for tests.
    pub async fn has_items(&self) -> bool {
        self.inner.db.read_async(|c| Ok(!store::items(c)?.is_empty())).await.unwrap_or(true)
    }

    async fn wait(&self, max: Duration) {
        let _ = tokio::time::timeout(max, self.inner.wake.notified()).await;
    }

    /// The queue's loop. Runs until the app quits.
    pub async fn run(self) {
        // Files deleted while the app was closed, and leftovers.
        if self.check_files() > 0 {
            tracing::info!("some downloaded files are missing");
        }
        self.sweep();
        let mut last_sync: Option<Instant> = None;
        let mut last_check = Instant::now();
        loop {
            // Files deleted or a drive unplugged while running: offline
            // plays stop counting on them within a minute.
            if last_check.elapsed() >= *self.inner.check_every.lock() {
                self.check_files();
                self.retry_pending_deletes();
                last_check = Instant::now();
            }
            if self.inner.dirty.load(Ordering::SeqCst) || last_sync.is_none_or(|t| t.elapsed() >= RESYNC_EVERY) {
                if let Err(e) = self.resync().await {
                    tracing::warn!("downloads resync failed: {e}");
                }
                last_sync = Some(Instant::now());
            }
            // Nothing downloaded or wanted: no timer, wait for a download
            // to be added (or the library to change).
            if !self.has_items().await {
                self.set_paused(None);
                self.inner.wake.notified().await;
                continue;
            }
            if let Some(why) = self.blocked().await {
                self.set_paused(Some(why));
                self.nap(WAIT_POLL).await;
                continue;
            }
            if self.inner.transfer.read().is_none() {
                self.nap(WAIT_POLL).await;
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
                    self.nap(retry.map_or(RESYNC_EVERY, |r| r.min(RESYNC_EVERY))).await;
                }
                Ok(Step::Worked) => {}
                Ok(Step::Paused(why)) => {
                    self.set_paused(Some(why));
                    self.nap(WAIT_POLL).await;
                }
                Err(e) => {
                    tracing::warn!("downloads: {e}");
                    self.nap(WAIT_POLL).await;
                }
            }
        }
    }

    /// Fetches the next queued song, if any. Public for tests.
    pub async fn step(&self) -> AppResult<Step> {
        let _step = self.inner.step_lock.lock().await;
        if self.inner.moving.load(Ordering::SeqCst) {
            return Ok(Step::Idle);
        }
        let ids = self.inner.db.read_async(|c| store::queue(c, now(), CANDIDATES)).await?;
        if ids.is_empty() {
            return Ok(Step::Idle);
        }
        let lookup = ids.clone();
        let (tracks, used, kept) = self
            .inner
            .db
            .read_async(move |c| {
                let tracks = queries::tracks_by_ids(c, &lookup)?;
                // A finished file fetched again frees its old size when the
                // new one lands.
                let mut kept = HashMap::new();
                for id in &lookup {
                    if let Some(row) = store::file(c, id)?
                        && row.state == "done"
                    {
                        kept.insert(id.clone(), row.size);
                    }
                }
                Ok((tracks, store::used_bytes(c)?, kept))
            })
            .await?;
        let by_id: HashMap<&str, &Track> = tracks.iter().map(|t| (t.id.as_str(), t)).collect();
        let gone: Vec<String> = ids.iter().filter(|id| !by_id.contains_key(id.as_str())).cloned().collect();
        if !gone.is_empty() {
            self.inner
                .db
                .write_async(move |c| {
                    for id in &gone {
                        store::mark_failed(c, id, "This song is no longer in your library.", now())?;
                    }
                    Ok(())
                })
                .await?;
            self.emit_status();
            return Ok(Step::Worked);
        }
        let prefs = self.prefs();
        // The first song that fits under the limit (one too big doesn't hold
        // up smaller ones behind it) and whose server can be reached.
        let mut wait: Option<PauseReason> = None;
        let mut asked = 0;
        let mut pick = None;
        for id in &ids {
            let track = by_id[id.as_str()];
            let base = used.saturating_sub(kept.get(id).copied().unwrap_or(0));
            let estimate = estimate_bytes(track, prefs.quality);
            let too_big = self
                .inner
                .too_big
                .lock()
                .as_ref()
                .is_some_and(|(room, ids)| *room == (prefs.limit_bytes, base) && ids.contains(id));
            if base + estimate > prefs.limit_bytes || too_big {
                wait.get_or_insert(PauseReason::Full);
                continue;
            }
            if asked == REACH_TRIES {
                break;
            }
            asked += 1;
            if let Some(why) = self.unreachable(track).await {
                wait.get_or_insert(why);
                continue;
            }
            pick = Some((track.clone(), base, estimate));
            break;
        }
        let Some((track, used, estimate)) = pick else {
            return Ok(Step::Paused(wait.unwrap_or(PauseReason::Full)));
        };
        let room_key = (prefs.limit_bytes, used);
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
        let stem = file_stem(&track.id);
        let name = format!("{stem}.{}", extension(&track, prefs.quality));
        let part_file = part_name(&name, &track.part_key);
        let part = dir.join(&part_file);
        // Partial files from another version of the song or another quality
        // can't be continued, and a transcode always starts over.
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if is_part_of(&n, &stem)
                && (n != part_file || !fetch.resumable)
                && let Err(err) = tokio::fs::remove_file(e.path()).await
                && err.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!("could not delete download {n}: {err}");
            }
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
                min_len: (!fetch.resumable).then(|| shortest_transcode(&track, prefs.quality)).flatten(),
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
                let id_for_old = id.clone();
                let done = move |c: &mut rusqlite::Connection| store::mark_done(c, &id, &pk, &q, &n, size, now());
                let mut old = self.inner.db.write_async(done.clone()).await?;
                // Removed while it downloaded, then turned on again before
                // the file was let go: the new row takes the file.
                if old.as_deref() == Some(name.as_str()) && self.still_wanted(&id_for_old) {
                    old = self.inner.db.write_async(done).await?;
                }
                if let Some(old) = old {
                    self.delete_files(vec![store::Gone { track_id: id_for_old, file: Some(old) }]);
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
                let mut tb = self.inner.too_big.lock();
                match tb.as_mut() {
                    Some((room, ids)) if *room == room_key => {
                        ids.insert(id);
                    }
                    _ => *tb = Some((room_key, [id].into_iter().collect())),
                }
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
        if p.offline || self.inner.moving.load(Ordering::SeqCst) {
            return false;
        }
        if self.inner.epoch.load(Ordering::SeqCst) != epoch && !self.still_wanted(track_id) {
            return false;
        }
        true
    }
}

/// The least a transcode at `quality` should come to: most of the size its
/// bitrate and length give. None when that can't be told (no length, or no
/// bitrate for a server that may send the file as it is when it's smaller).
fn shortest_transcode(track: &Track, quality: DownloadQuality) -> Option<u64> {
    let kbps = quality.kbps()?.min(track.bitrate?) as u64;
    if track.duration_ms == 0 || kbps == 0 {
        return None;
    }
    Some(kbps * 1000 / 8 * track.duration_ms / 1000 * 7 / 10)
}

/// A file whose delete failed, with the folder it is in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct PendingDelete {
    dir: String,
    name: String,
}

/// File names and track stems the download rows still use.
#[derive(Default)]
struct InUse {
    names: std::collections::HashSet<String>,
    stems: std::collections::HashSet<String>,
}

/// The list of files to delete later. An older list held bare names with no
/// folder; those are dropped, since which folder they were in isn't known.
fn pending_deletes(c: &rusqlite::Connection) -> AppResult<Vec<PendingDelete>> {
    let Some(v) = crate::db::store::kv_get(c, KEY_PENDING_DELETE)? else { return Ok(Vec::new()) };
    match serde_json::from_str(&v) {
        Ok(list) => Ok(list),
        Err(_) => {
            tracing::info!("downloads: dropping an older list of files to delete later");
            Ok(Vec::new())
        }
    }
}

fn pending_json(list: &[PendingDelete]) -> String {
    serde_json::to_string(list).unwrap_or_else(|_| "[]".into())
}

/// Deletes one file. A failure other than "already gone" is logged and its
/// name kept in `failed`.
fn remove_logged(path: &Path, name: &str, failed: &mut Vec<String>) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("could not delete download {name}: {e}");
        failed.push(name.to_string());
    }
}

/// What one step of the queue did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Idle,
    Worked,
    Paused(PauseReason),
}

/// Moves the named files, the art folder and any partial files from `from`
/// to `to`. On a failure the files already moved go back.
fn move_all(from: &Path, to: &Path, names: &[String]) -> AppResult<()> {
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let go = |a: PathBuf, b: PathBuf, moved: &mut Vec<(PathBuf, PathBuf)>| -> std::io::Result<()> {
        if !a.exists() {
            return Ok(());
        }
        move_file(&a, &b)?;
        moved.push((a, b));
        Ok(())
    };
    let mut result = Ok(());
    for n in names {
        if let Err(e) = go(from.join(n), to.join(n), &mut moved) {
            result = Err(AppError::Io(format!("Couldn't move {n}: {e}")));
            break;
        }
    }
    let art_from = from.join(art::ART_DIR);
    if result.is_ok() && art_from.is_dir() {
        let art_to = to.join(art::ART_DIR);
        result = std::fs::create_dir_all(&art_to).map_err(AppError::from);
        if result.is_ok() {
            for e in std::fs::read_dir(&art_from).into_iter().flatten().flatten() {
                // Only the covers this app kept; anything else stays put.
                if !art::is_art_name(&e.file_name().to_string_lossy()) {
                    continue;
                }
                if let Err(err) = go(e.path(), art_to.join(e.file_name()), &mut moved) {
                    result = Err(AppError::Io(format!("Couldn't move album art: {err}")));
                    break;
                }
            }
        }
    }
    if let Err(e) = result {
        for (a, b) in moved.iter().rev() {
            if let Err(err) = move_file(b, a) {
                tracing::warn!("download not moved back to {}: {err}", a.display());
            }
        }
        return Err(e);
    }
    let _ = std::fs::remove_dir(&art_from);
    // Half-fetched songs carry on in the new folder; losing one only means
    // fetching it again.
    for e in std::fs::read_dir(from).into_iter().flatten().flatten() {
        if part_stem(&e.file_name().to_string_lossy()).is_some() && e.path().is_file() {
            let _ = move_file(&e.path(), &to.join(e.file_name()));
        }
    }
    Ok(())
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
        // The whole folder away (an unplugged drive): nothing to mark.
        if !p.parent().is_some_and(Path::is_dir) {
            return None;
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
