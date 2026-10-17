//! The devices service: the remembered devices, finding them among the
//! mounted drives, and the sync itself (one at a time, in the background).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use tokio::sync::Notify;

use super::drives::Volume;
use super::encode::{self, Make};
use super::io::{DeviceIo, Failure, classify};
use super::manifest::{self, Entry, Manifest};
use super::plan::{self, Placed, Want};
use super::{Device, DeviceFormat, MANIFEST, SelItem, SelKind, m3u, names, store};
use crate::db::DbHandle;
use crate::downloads::{ItemKey, ItemKind};
use crate::error::{AppError, AppResult};
use crate::model::Track;

/// Songs made ahead of the copy: two encoder threads at most.
pub const AHEAD: usize = 2;
/// How often a busy computer is looked at again.
const BUSY_POLL: Duration = Duration::from_secs(2);
/// The manifest is saved after this many songs (and at the end).
const SAVE_EVERY: u32 = 10;
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Where songs come from.
#[async_trait]
pub trait TrackFiles: Send + Sync {
    /// Puts the song's original file at `dest`.
    async fn fetch(&self, t: &Track, dest: &Path, stop: &(dyn Fn() -> bool + Sync)) -> Result<(), String>;
    /// Album art for a song without a picture of its own.
    async fn art(&self, _t: &Track) -> Option<Vec<u8>> {
        None
    }
}

/// Whether background work should wait: playback still filling its
/// buffers, a fullscreen game, a hot or busy computer.
pub trait Pace: Send + Sync {
    fn busy(&self) -> bool;
}

/// The mounted drives.
pub trait Volumes: Send + Sync {
    fn list(&self) -> Vec<Volume>;
}

pub trait SyncEvents: Send + Sync {
    /// `devices://progress`.
    fn progress(&self, s: &SyncStatus);
    /// `devices://changed`: the device list changed.
    fn changed(&self);
}

pub struct Deps {
    pub db: DbHandle,
    pub files: Arc<dyn TrackFiles>,
    pub volumes: Arc<dyn Volumes>,
    pub io: Arc<dyn DeviceIo>,
    pub pace: Arc<dyn Pace>,
    /// Where originals and converted files wait before the copy.
    pub scratch: PathBuf,
}

/// The settings a sync follows.
#[derive(Debug, Clone, Copy)]
pub struct Opts {
    pub default_format: DeviceFormat,
    pub delete_removed: bool,
    /// Liked Songs' rating threshold.
    pub liked_min: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Planning,
    Copying,
    /// Waiting while playback or the computer is busy.
    Waiting,
    Paused,
    Done,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedSong {
    pub title: String,
    pub error: String,
}

/// An item some (or all) of whose songs didn't fit.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeftOut {
    pub title: String,
    pub songs: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncError {
    /// full, writeProtected, removed, notConnected, other.
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub device_id: String,
    pub device_name: String,
    pub phase: Phase,
    /// Songs to copy this time.
    pub total: u32,
    pub done: u32,
    /// Songs already on the device as they should be.
    pub kept: u32,
    /// Platter's files removed from the device.
    pub removed: u32,
    pub current: Option<String>,
    pub bytes: u64,
    pub failed: Vec<FailedSong>,
    pub left_out: Vec<LeftOut>,
    pub left_out_songs: u32,
    pub error: Option<SyncError>,
    /// Everything written is flushed to the device.
    pub safe_to_unplug: bool,
}

impl SyncStatus {
    fn new(d: &Device) -> Self {
        Self {
            device_id: d.id.clone(),
            device_name: d.name.clone(),
            phase: Phase::Planning,
            total: 0,
            done: 0,
            kept: 0,
            removed: 0,
            current: None,
            bytes: 0,
            failed: vec![],
            left_out: vec![],
            left_out_songs: 0,
            error: None,
            safe_to_unplug: false,
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.phase, Phase::Planning | Phase::Copying | Phase::Waiting | Phase::Paused)
    }
}

/// Pause and cancel for the running sync.
#[derive(Default)]
pub struct SyncControl {
    paused: AtomicBool,
    cancel: AtomicBool,
    wake: Notify,
}

impl SyncControl {
    pub fn pause(&self, on: bool) {
        self.paused.store(on, Ordering::SeqCst);
        self.wake.notify_waiters();
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.wake.notify_waiters();
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

/// A device as the UI shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    #[serde(flatten)]
    pub device: Device,
    /// Plugged in now: where.
    pub root: Option<String>,
    pub free: Option<u64>,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicesView {
    pub devices: Vec<DeviceView>,
    /// Removable drives that aren't a known device.
    pub drives: Vec<Volume>,
    pub sync: Option<SyncStatus>,
}

struct Inner {
    deps: Deps,
    events: RwLock<Option<Arc<dyn SyncEvents>>>,
    status: Mutex<Option<SyncStatus>>,
    control: Mutex<Option<Arc<SyncControl>>>,
    last_progress: Mutex<Option<Instant>>,
}

/// Cheap to clone; clones share state.
#[derive(Clone)]
pub struct Devices {
    inner: Arc<Inner>,
}

/// Finds a device among the mounted drives: by volume id, else by the
/// manifest in its music folder.
pub fn find_root(d: &Device, volumes: &[Volume], io: &dyn DeviceIo) -> Option<Volume> {
    if let Some(id) = &d.volume_id
        && let Some(v) = volumes.iter().find(|v| v.id.as_deref() == Some(id.as_str()))
    {
        return Some(v.clone());
    }
    volumes
        .iter()
        .find(|v| {
            io.read(&music_dir(&v.root, &d.folder).join(MANIFEST))
                .ok()
                .and_then(|b| Manifest::parse(&b))
                .is_some_and(|m| m.device_id == d.id)
        })
        .cloned()
}

pub fn music_dir(root: &Path, folder: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in folder.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

fn rel_path(music: &Path, rel: &str) -> PathBuf {
    music_dir(music, rel)
}

/// An item's songs now, from the library cache, in their order.
pub fn item_tracks(c: &rusqlite::Connection, it: &SelItem, liked_min: f64) -> AppResult<Vec<String>> {
    let lib = |kind| crate::downloads::store::tracks_of_library_item(c, &ItemKey::new(kind, &it.id), liked_min);
    Ok(match it.kind {
        SelKind::Playlist => lib(ItemKind::Playlist)?.unwrap_or_default(),
        SelKind::Album => lib(ItemKind::Album)?.unwrap_or_default(),
        SelKind::Liked => lib(ItemKind::Liked)?.unwrap_or_default(),
        SelKind::Artist => {
            let mut st = c.prepare_cached(
                "SELECT t.id FROM tracks t LEFT JOIN albums a ON a.id = t.album_id WHERE t.artist_id = ?1
                 ORDER BY a.year, a.sort_title, t.album_id, t.disc_number, t.track_number, t.title",
            )?;
            let ids = st.query_map([&it.id], |r| r.get(0))?;
            ids.collect::<Result<_, _>>()?
        }
    })
}

/// Everything a sync needs from the library.
struct Resolved {
    /// Song ids per selection item, in order.
    items: Vec<Vec<String>>,
    tracks: HashMap<String, Track>,
    multi_disc: HashSet<String>,
}

fn resolve(c: &rusqlite::Connection, sel: &[SelItem], liked_min: f64) -> AppResult<Resolved> {
    let mut items = Vec::new();
    let mut all = Vec::new();
    let mut seen = HashSet::new();
    for it in sel {
        let ids = item_tracks(c, it, liked_min)?;
        for id in &ids {
            if seen.insert(id.clone()) {
                all.push(id.clone());
            }
        }
        items.push(ids);
    }
    let list = crate::db::queries::tracks_by_ids(c, &all)?;
    let albums: HashSet<&str> = list.iter().map(|t| t.album_id.as_str()).collect();
    let mut multi_disc = HashSet::new();
    let mut st = c.prepare_cached("SELECT MAX(disc_number) FROM tracks WHERE album_id = ?1")?;
    for a in albums {
        let max: Option<i64> = st.query_row([a], |r| r.get(0))?;
        if max.unwrap_or(1) > 1 {
            multi_disc.insert(a.to_string());
        }
    }
    let tracks = list.into_iter().map(|t| (t.id.clone(), t)).collect();
    Ok(Resolved { items, tracks, multi_disc })
}

fn want_for(t: &Track, item: usize, format: DeviceFormat, multi: bool) -> (Want, String) {
    let src_ext = names::original_ext(&t.part_key, t.codec.as_deref(), t.container.as_deref());
    let how = encode::make_of(format, t, &src_ext);
    let ext = format.ext().map(str::to_string).unwrap_or_else(|| src_ext.clone());
    let natural = names::track_path(&t.artist_title, &t.album_title, t.disc_number, multi, t.track_number, &t.title, &ext);
    let w = Want {
        track: t.clone(),
        item,
        format,
        natural,
        source: manifest::fingerprint(t, format),
        est: plan::estimate(t, format, how != Make::Convert),
    };
    (w, src_ext)
}

/// One song made and waiting in the scratch folder.
struct Ready {
    placed: Placed,
    file: PathBuf,
    cleanup: Vec<PathBuf>,
}

enum Prepared {
    Ready(Ready),
    Failed(Placed, String),
    Stopped,
}

/// Runs the encoder on its own thread, below normal priority.
async fn encode_on_thread(
    src: PathBuf,
    src_ext: String,
    out: PathBuf,
    format: DeviceFormat,
    t: Track,
    art: Option<Vec<u8>>,
    ctl: Arc<SyncControl>,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("device-encode".into())
        .spawn(move || {
            lower_priority();
            let stop = || ctl.cancelled();
            let r = encode::make(&src, &src_ext, &out, format, &t, art.as_deref(), &stop);
            let _ = tx.send(r);
        })
        .map_err(|e| format!("couldn't start converting ({e})"))?;
    rx.await.unwrap_or_else(|_| Err("converting stopped unexpectedly".into()))
}

/// This thread runs behind everything else on the computer.
fn lower_priority() {
    #[cfg(target_os = "windows")]
    // SAFETY: the pseudo handle for the calling thread.
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST};
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
    }
    #[cfg(target_os = "linux")]
    // SAFETY: plain syscalls on the calling thread.
    unsafe {
        let tid = libc::gettid();
        libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 15);
    }
}

impl Devices {
    pub fn new(deps: Deps) -> Self {
        Self {
            inner: Arc::new(Inner {
                deps,
                events: RwLock::new(None),
                status: Mutex::new(None),
                control: Mutex::new(None),
                last_progress: Mutex::new(None),
            }),
        }
    }

    pub fn set_events(&self, e: Arc<dyn SyncEvents>) {
        *self.inner.events.write() = Some(e);
    }

    fn db(&self) -> &DbHandle {
        &self.inner.deps.db
    }

    fn changed(&self) {
        if let Some(e) = self.inner.events.read().clone() {
            e.changed();
        }
    }

    // ------------------------------------------------------------ devices

    pub fn devices(&self) -> Vec<Device> {
        store::load(self.db())
    }

    /// The devices, where each is plugged in, and the new drives.
    pub fn view(&self) -> DevicesView {
        let volumes = self.inner.deps.volumes.list();
        let io = &*self.inner.deps.io;
        let mut used = HashSet::new();
        let devices = store::load(self.db())
            .into_iter()
            .map(|d| {
                let v = find_root(&d, &volumes, io);
                if let Some(v) = &v {
                    used.insert(v.root.clone());
                }
                DeviceView {
                    root: v.as_ref().map(|v| v.root.to_string_lossy().into_owned()),
                    free: v.as_ref().and_then(|v| v.free),
                    total: v.as_ref().and_then(|v| v.total),
                    device: d,
                }
            })
            .collect();
        let drives = volumes.into_iter().filter(|v| v.removable && !used.contains(&v.root)).collect();
        DevicesView { devices, drives, sync: self.status() }
    }

    /// Remembers a drive (at `root`) as a device with its music folder.
    pub fn add(&self, root: &Path, name: &str, folder: &str) -> AppResult<Device> {
        let name = store::clean_name(name)?;
        let folder = super::clean_folder(folder).map_err(AppError::Invalid)?;
        let volumes = self.inner.deps.volumes.list();
        let v = super::drives::volume_of(root, &volumes)
            .ok_or_else(|| AppError::NotFound("That drive isn't plugged in any more.".into()))?;
        let mut list = store::load(self.db());
        if let Some(id) = &v.id
            && let Some(d) = list.iter().find(|d| d.volume_id.as_deref() == Some(id) && d.folder.eq_ignore_ascii_case(&folder))
        {
            return Err(AppError::Invalid(format!("That folder is already the device {}.", d.name)));
        }
        let d = Device {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            volume_id: v.id.clone(),
            volume_label: v.label.clone(),
            folder,
            format: None,
            budget_gb: None,
            selection: vec![],
            last_sync: None,
            last_root: Some(v.root.to_string_lossy().into_owned()),
        };
        list.push(d.clone());
        store::save(self.db(), &list)?;
        self.changed();
        Ok(d)
    }

    pub fn update(&self, id: &str, f: impl FnOnce(&mut Device) -> AppResult<()>) -> AppResult<Device> {
        let d = store::update(self.db(), id, f)?;
        self.changed();
        Ok(d)
    }

    /// Forgets a device on this computer. Its files stay on the device.
    pub fn forget(&self, id: &str) -> AppResult<()> {
        if self.status().is_some_and(|s| s.running() && s.device_id == id) {
            return Err(AppError::Invalid("Cancel the sync to this device first.".into()));
        }
        let mut list = store::load(self.db());
        list.retain(|d| d.id != id);
        store::save(self.db(), &list)?;
        self.changed();
        Ok(())
    }

    // ------------------------------------------------------------ status

    pub fn status(&self) -> Option<SyncStatus> {
        self.inner.status.lock().clone()
    }

    fn set_status(&self, f: impl FnOnce(&mut SyncStatus), force: bool) {
        let snapshot = {
            let mut g = self.inner.status.lock();
            let Some(s) = g.as_mut() else { return };
            f(s);
            s.clone()
        };
        let mut last = self.inner.last_progress.lock();
        if !force && last.is_some_and(|t| t.elapsed() < PROGRESS_EVERY) {
            return;
        }
        *last = Some(Instant::now());
        drop(last);
        if let Some(e) = self.inner.events.read().clone() {
            e.progress(&snapshot);
        }
    }

    pub fn pause(&self, on: bool) {
        if let Some(c) = self.inner.control.lock().clone() {
            c.pause(on);
            self.set_status(
                |s| {
                    if s.running() {
                        s.phase = if on { Phase::Paused } else { Phase::Copying };
                    }
                },
                true,
            );
        }
    }

    pub fn cancel(&self) {
        if let Some(c) = self.inner.control.lock().clone() {
            c.cancel();
        }
    }

    // ------------------------------------------------------------ sync

    /// Starts syncing a device in the background.
    pub fn start(&self, id: &str, opts: Opts) -> AppResult<SyncStatus> {
        let d = store::load(self.db())
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| AppError::NotFound("That device isn't on this computer any more.".into()))?;
        let ctl = {
            let mut c = self.inner.control.lock();
            if self.status().is_some_and(|s| s.running()) {
                return Err(AppError::Invalid("A sync is already running. Wait for it, or cancel it.".into()));
            }
            let ctl = Arc::new(SyncControl::default());
            *c = Some(ctl.clone());
            ctl
        };
        let st = SyncStatus::new(&d);
        *self.inner.status.lock() = Some(st.clone());
        self.set_status(|_| {}, true);
        let me = self.clone();
        tokio::spawn(async move { me.sync(d, opts, ctl).await });
        Ok(st)
    }

    /// Runs one sync to the end (the background task; tests call it directly).
    pub async fn sync(&self, d: Device, opts: Opts, ctl: Arc<SyncControl>) {
        if self.status().is_none_or(|s| s.device_id != d.id) {
            *self.inner.status.lock() = Some(SyncStatus::new(&d));
        }
        let outcome = self.sync_inner(&d, opts, &ctl).await;
        let finished = matches!(outcome, Ok(true));
        self.set_status(
            |s| {
                s.current = None;
                match &outcome {
                    Ok(true) => {
                        s.phase = Phase::Done;
                        s.safe_to_unplug = true;
                    }
                    Ok(false) => {
                        s.phase = Phase::Cancelled;
                        s.safe_to_unplug = true;
                    }
                    Err(f) => {
                        s.phase = Phase::Failed;
                        // Nothing is mid-write once it has stopped.
                        s.safe_to_unplug = !matches!(f, SyncFail::Device(Failure::Removed));
                        s.error = Some(f.to_error());
                    }
                }
            },
            true,
        );
        if finished {
            let _ = store::update(self.db(), &d.id, |x| {
                x.last_sync = Some(super::now());
                Ok(())
            });
            self.changed();
        }
        *self.inner.control.lock() = None;
        let _ = std::fs::remove_dir_all(self.inner.deps.scratch.join(&d.id));
    }

    async fn wait_turn(&self, ctl: &SyncControl) -> bool {
        loop {
            if ctl.cancelled() {
                return false;
            }
            if ctl.paused() {
                self.set_status(|s| s.phase = Phase::Paused, true);
                let _ = tokio::time::timeout(BUSY_POLL, ctl.wake.notified()).await;
                continue;
            }
            if self.inner.deps.pace.busy() {
                self.set_status(|s| s.phase = Phase::Waiting, false);
                let _ = tokio::time::timeout(BUSY_POLL, ctl.wake.notified()).await;
                continue;
            }
            self.set_status(
                |s| {
                    if matches!(s.phase, Phase::Paused | Phase::Waiting | Phase::Planning) {
                        s.phase = Phase::Copying;
                    }
                },
                false,
            );
            return true;
        }
    }

    /// Ok(true): finished; Ok(false): cancelled.
    async fn sync_inner(&self, d: &Device, opts: Opts, ctl: &Arc<SyncControl>) -> Result<bool, SyncFail> {
        let io = self.inner.deps.io.clone();
        let volumes = self.inner.deps.volumes.list();
        let vol = find_root(d, &volumes, &*io).ok_or(SyncFail::NotConnected(d.name.clone()))?;
        let root = vol.root.clone();
        let music = music_dir(&root, &d.folder);
        let present = {
            let io = io.clone();
            let root = root.clone();
            move || io.is_dir(&root)
        };
        let dev_err = |e: std::io::Error| SyncFail::Device(classify(&e, present()));

        // What Platter wrote before.
        let mut man = io
            .read(&music.join(MANIFEST))
            .ok()
            .and_then(|b| Manifest::parse(&b))
            .unwrap_or_else(|| Manifest::new(&d.id));
        man.device_id = d.id.clone();

        // What the device should hold.
        let sel = d.selection.clone();
        let liked_min = opts.liked_min;
        let res = self
            .db()
            .read_async(move |c| resolve(c, &sel, liked_min))
            .await
            .map_err(|e| SyncFail::Other(e.to_string()))?;
        let format = d.format.unwrap_or(opts.default_format);
        let mut wanted = Vec::new();
        let mut src_exts = HashMap::new();
        for (i, ids) in res.items.iter().enumerate() {
            for id in ids {
                if let Some(t) = res.tracks.get(id) {
                    let (w, ext) = want_for(t, i, format, res.multi_disc.contains(&t.album_id));
                    src_exts.insert(t.id.clone(), ext);
                    wanted.push(w);
                }
            }
        }
        let budget = d.budget_gb.filter(|g| *g > 0.0).map(|g| (g * 1024.0 * 1024.0 * 1024.0) as u64);
        let free = vol.free.or_else(|| fs4::available_space(&root).ok());
        let on_disk = |rel: &str| io.size(&rel_path(&music, rel));
        let p = plan::plan(wanted, &plan::Device { manifest: &man, on_disk: &on_disk, free }, budget, opts.delete_removed);

        let mut left: Vec<(usize, u32)> = Vec::new();
        for w in &p.left_out {
            match left.iter_mut().find(|(i, _)| *i == w.item) {
                Some((_, n)) => *n += 1,
                None => left.push((w.item, 1)),
            }
        }
        let left_out: Vec<LeftOut> =
            left.into_iter().map(|(i, songs)| LeftOut { title: d.selection.get(i).map(|s| s.title.clone()).unwrap_or_default(), songs }).collect();
        let (total, kept, left_songs) = (p.writes.len() as u32, p.keep.len() as u32, p.left_out.len() as u32);
        self.set_status(
            |s| {
                s.total = total;
                s.kept = kept;
                s.left_out = left_out;
                s.left_out_songs = left_songs;
            },
            true,
        );

        // Forget files that are gone; this first write also shows a
        // write-protected device straight away.
        for rel in &p.stale {
            man.files.remove(rel);
        }
        save_manifest(&*io, &music, &man).map_err(dev_err)?;

        // Platter's files for songs no longer chosen.
        let mut removed = 0u32;
        for rel in &p.deletes {
            if ctl.cancelled() {
                break;
            }
            let path = rel_path(&music, rel);
            io.remove(&path).map_err(dev_err)?;
            man.files.remove(rel);
            if let Some(dir) = path.parent() {
                io.prune(dir, &music);
            }
            removed += 1;
        }
        if removed > 0 {
            save_manifest(&*io, &music, &man).map_err(dev_err)?;
            self.set_status(|s| s.removed = removed, true);
        }

        // Make and copy the new songs, two being made ahead at most.
        let scratch = self.inner.deps.scratch.join(&d.id);
        let _ = std::fs::create_dir_all(&scratch);
        let mut on_device: HashMap<String, String> = p.keep.iter().map(|k| (k.want.track.id.clone(), k.rel.clone())).collect();
        let mut stopped = ctl.cancelled();
        let mut since_save = 0u32;
        let files = self.inner.deps.files.clone();
        let jobs = p.writes.into_iter().enumerate().map(|(n, placed)| {
            let me = self.clone();
            let ctl = ctl.clone();
            let files = files.clone();
            let scratch = scratch.clone();
            let src_ext = src_exts.get(&placed.want.track.id).cloned().unwrap_or_else(|| "audio".into());
            tokio::spawn(async move { me.prepare(n, placed, src_ext, &scratch, &*files, ctl).await })
        });
        let mut ready = futures::stream::iter(jobs).buffered(AHEAD);
        while let Some(job) = ready.next().await {
            let job = job.unwrap_or(Prepared::Stopped);
            if ctl.cancelled() {
                if let Prepared::Ready(r) = &job {
                    clean(&r.cleanup);
                }
                stopped = true;
                break;
            }
            let r = match job {
                Prepared::Stopped => {
                    stopped = true;
                    break;
                }
                Prepared::Failed(placed, error) => {
                    self.set_status(
                        |s| {
                            s.done += 1;
                            s.failed.push(FailedSong { title: placed.want.track.title.clone(), error });
                        },
                        true,
                    );
                    continue;
                }
                Prepared::Ready(r) => r,
            };
            if !self.wait_turn(ctl).await {
                clean(&r.cleanup);
                stopped = true;
                break;
            }
            let title = r.placed.want.track.title.clone();
            self.set_status(|s| s.current = Some(title), false);
            let dest = rel_path(&music, &r.placed.rel);
            let copy = {
                let io = io.clone();
                let file = r.file.clone();
                let dest = dest.clone();
                let ctl = ctl.clone();
                tokio::task::spawn_blocking(move || io.copy_in(&file, &dest, &|| ctl.cancelled(), &mut |_| {}))
                    .await
                    .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())))
            };
            clean(&r.cleanup);
            let size = match copy {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    stopped = true;
                    break;
                }
                Err(e) => {
                    let f = classify(&e, present());
                    // Keep what got there.
                    if !matches!(f, Failure::Removed) {
                        let _ = save_manifest(&*io, &music, &man);
                    }
                    return Err(SyncFail::Device(f));
                }
            };
            let w = &r.placed.want;
            // Older copies of this song go now that the new one is there.
            let old: Vec<String> =
                man.files.iter().filter(|(p, e)| e.track_id == w.track.id && **p != r.placed.rel).map(|(p, _)| p.clone()).collect();
            for rel in old {
                let path = rel_path(&music, &rel);
                if io.remove(&path).is_ok() {
                    man.files.remove(&rel);
                    if let Some(dir) = path.parent() {
                        io.prune(dir, &music);
                    }
                }
            }
            man.files.insert(r.placed.rel.clone(), Entry { track_id: w.track.id.clone(), source: w.source.clone(), format: w.format, size });
            on_device.insert(w.track.id.clone(), r.placed.rel.clone());
            self.set_status(
                |s| {
                    s.done += 1;
                    s.bytes += size;
                },
                false,
            );
            since_save += 1;
            if since_save >= SAVE_EVERY {
                since_save = 0;
                save_manifest(&*io, &music, &man).map_err(dev_err)?;
            }
        }
        drop(ready);

        // Playlists, with what's on the device now.
        self.write_playlists(d, &res.items, &res.tracks, &on_device, &music, &mut man, opts.delete_removed)
            .map_err(dev_err)?;
        save_manifest(&*io, &music, &man).map_err(dev_err)?;
        Ok(!stopped)
    }

    async fn prepare(&self, n: usize, placed: Placed, src_ext: String, scratch: &Path, files: &dyn TrackFiles, ctl: Arc<SyncControl>) -> Prepared {
        if !self.wait_turn(&ctl).await {
            return Prepared::Stopped;
        }
        let t = placed.want.track.clone();
        let src = scratch.join(format!("{n}-src.{src_ext}"));
        let _ = std::fs::remove_file(&src);
        let stop = || ctl.cancelled();
        if let Err(e) = files.fetch(&t, &src, &stop).await {
            let _ = std::fs::remove_file(&src);
            return if ctl.cancelled() { Prepared::Stopped } else { Prepared::Failed(placed, e) };
        }
        let format = placed.want.format;
        if encode::make_of(format, &t, &src_ext) == Make::AsIs {
            return Prepared::Ready(Ready { placed, file: src.clone(), cleanup: vec![src] });
        }
        let art = files.art(&t).await;
        let out = scratch.join(format!("{n}-out.{}", format.ext().unwrap_or("bin")));
        let made = encode_on_thread(src.clone(), src_ext, out.clone(), format, t, art, ctl.clone()).await;
        let _ = std::fs::remove_file(&src);
        match made {
            Ok(()) => Prepared::Ready(Ready { placed, file: out.clone(), cleanup: vec![out] }),
            Err(_) if ctl.cancelled() => {
                let _ = std::fs::remove_file(&out);
                Prepared::Stopped
            }
            Err(e) => {
                let _ = std::fs::remove_file(&out);
                Prepared::Failed(placed, e)
            }
        }
    }

    /// An M3U8 for each chosen playlist and for Liked Songs, listing the
    /// songs on the device in the item's order.
    #[allow(clippy::too_many_arguments)]
    fn write_playlists(
        &self,
        d: &Device,
        items: &[Vec<String>],
        tracks: &HashMap<String, Track>,
        on_device: &HashMap<String, String>,
        music: &Path,
        man: &mut Manifest,
        delete_removed: bool,
    ) -> std::io::Result<()> {
        let io = &*self.inner.deps.io;
        let mut written: HashSet<String> = HashSet::new();
        let ours: HashSet<String> = man.files.keys().chain(man.playlists.keys()).map(|p| names::key(p)).collect();
        for (i, it) in d.selection.iter().enumerate() {
            if !matches!(it.kind, SelKind::Playlist | SelKind::Liked) {
                continue;
            }
            let mut seen = HashSet::new();
            let lines: Vec<m3u::Line<'_>> = items[i]
                .iter()
                .filter(|id| seen.insert(id.as_str()))
                .filter_map(|id| {
                    let t = tracks.get(id)?;
                    let rel = on_device.get(id)?;
                    Some(m3u::Line {
                        rel,
                        duration_ms: t.duration_ms,
                        artist: t.track_artist.as_deref().filter(|a| !a.is_empty()).unwrap_or(&t.artist_title),
                        title: &t.title,
                    })
                })
                .collect();
            let natural = names::playlist_path(&it.title);
            let mut n = 1;
            let rel = loop {
                let rel = if n == 1 { natural.clone() } else { names::with_suffix(&natural, n) };
                let k = names::key(&rel);
                let foreign = !ours.contains(&k) && io.size(&rel_path(music, &rel)).is_some();
                if !written.contains(&k) && !foreign {
                    break rel;
                }
                n += 1;
            };
            written.insert(names::key(&rel));
            let text = m3u::write(&lines);
            let hash = manifest::hash_text(&text);
            let path = rel_path(music, &rel);
            if man.playlists.get(&rel) != Some(&hash) || io.size(&path).is_none() {
                io.write_bytes(&path, text.as_bytes())?;
                man.playlists.insert(rel, hash);
            }
        }
        // Playlist files from before that weren't written this time.
        let old: Vec<String> = man.playlists.keys().filter(|p| !written.contains(&names::key(p))).cloned().collect();
        for rel in old {
            let path = rel_path(music, &rel);
            if io.size(&path).is_none() {
                man.playlists.remove(&rel);
            } else if delete_removed {
                io.remove(&path)?;
                man.playlists.remove(&rel);
            }
        }
        Ok(())
    }
}

fn clean(paths: &[PathBuf]) {
    for p in paths {
        let _ = std::fs::remove_file(p);
    }
}

fn save_manifest(io: &dyn DeviceIo, music: &Path, m: &Manifest) -> std::io::Result<()> {
    io.write_bytes(&music.join(MANIFEST), &m.to_bytes())
}

/// Why a sync stopped early.
#[derive(Debug)]
pub enum SyncFail {
    NotConnected(String),
    Device(Failure),
    Other(String),
}

impl SyncFail {
    fn to_error(&self) -> SyncError {
        match self {
            Self::NotConnected(name) => SyncError { kind: "notConnected".into(), message: format!("Plug in {name} to sync it.") },
            Self::Device(f) => SyncError { kind: f.kind().into(), message: f.message() },
            Self::Other(m) => SyncError { kind: "other".into(), message: m.clone() },
        }
    }
}
