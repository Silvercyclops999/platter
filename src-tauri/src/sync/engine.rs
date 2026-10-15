//! Keeps the local store and the sync file in step while sync is on.
//!
//! A pass reads the file (and any conflicted copies), takes the entries that
//! win into the local store, adds this device's entries that win, and writes
//! the file back when anything changed. Passes run 2 s after the last local
//! change, when the file changes on disk (checked every 3 s), every 60 s,
//! and on "Sync now". When the folder can't be reached, changes simply stay
//! in the local store and go out with the next pass that can write: that is
//! the offline queue.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};

use super::file::{self, DeviceSeen, ReadError, SyncFileData};
use super::store::{self, SyncStore};
use crate::error::{AppError, AppResult};

/// What to sync (Settings → Sync → What to sync).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Groups {
    /// Settings marked Synced.
    pub settings: bool,
    /// Library folders and pins, release status marks and artist page
    /// sections.
    pub library: bool,
    /// Followed artists, hidden and snoozed items, bookmarks.
    pub follows: bool,
    /// Lyrics you wrote.
    pub lyrics: bool,
}

impl Default for Groups {
    fn default() -> Self {
        Self { settings: true, library: true, follows: true, lyrics: true }
    }
}

impl Groups {
    /// The store namespaces these groups cover.
    pub fn namespaces(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.settings {
            out.push(store::SETTINGS.name());
        }
        if self.library {
            out.extend([
                store::PINS.name(),
                store::FOLDERS.name(),
                store::FOLDER_ITEMS.name(),
                store::SMART.name(),
                store::RELEASE_MARKS.name(),
                store::ALBUM_SECTIONS.name(),
            ]);
        }
        if self.follows {
            out.extend([store::FOLLOWS.name(), store::HIDDEN.name(), store::BOOKMARKS.name()]);
        }
        if self.lyrics {
            out.push(store::LYRICS.name());
        }
        out
    }
}

/// The sync settings, as the engine follows them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncConfig {
    pub enabled: bool,
    pub folder: Option<PathBuf>,
    pub groups: Groups,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncState {
    /// Sync is off (or on with no folder chosen).
    #[default]
    Off,
    /// Not signed in to Plex, so there's no account to name the file by.
    SignedOut,
    Ok,
    /// The chosen folder isn't there (drive unplugged, cloud app signed out).
    FolderMissing,
    /// The file isn't a sync file Platter can read.
    Unreadable,
    /// Written by a newer Platter.
    Newer,
    /// Reading or writing failed.
    Error,
}

/// Another device that writes the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeenDevice {
    pub name: String,
    pub last_seen: u64,
    pub this_device: bool,
}

/// For Settings → Sync (`sync_file_status`, `sync-file://status`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFileStatus {
    pub state: SyncState,
    pub folder: Option<String>,
    /// The file's full path.
    pub file: Option<String>,
    pub last_synced_at: Option<u64>,
    pub devices: Vec<SeenDevice>,
    /// Conflicted copies merged in by the last pass.
    pub merged_copies: u32,
    /// Plain words for a problem.
    pub message: Option<String>,
}

/// The account's tag (a hash of the Plex account id), or None when signed out.
#[async_trait]
pub trait AccountTag: Send + Sync {
    async fn tag(&self) -> Option<String>;
}

/// Where status and changes go (the window, in the app).
pub trait SyncEvents: Send + Sync {
    fn status(&self, status: &SyncFileStatus);
}

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Quiet time after a local change before writing.
    pub debounce: Duration,
    /// How often the file is checked for changes.
    pub poll: Duration,
    /// A full pass at least this often.
    pub reread: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { debounce: Duration::from_secs(2), poll: Duration::from_secs(3), reread: Duration::from_secs(60) }
    }
}

/// What one pass did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PassOutcome {
    /// Namespaces the file changed here.
    pub changed: Vec<String>,
    /// Whether the file was written.
    pub wrote: bool,
    pub merged_copies: u32,
    pub devices: Vec<SeenDevice>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassError {
    FolderMissing,
    Unreadable,
    Newer,
    Io(String),
}

impl PassError {
    fn state(&self) -> (SyncState, String) {
        match self {
            PassError::FolderMissing => (
                SyncState::FolderMissing,
                "The sync folder isn't there. Changes are kept on this computer until it's back.".into(),
            ),
            PassError::Unreadable => (
                SyncState::Unreadable,
                "The sync file can't be read. Changes are kept on this computer. Start a new file to sync again.".into(),
            ),
            PassError::Newer => (SyncState::Newer, "A newer Platter wrote the sync file. Update Platter to keep syncing.".into()),
            PassError::Io(e) => (SyncState::Error, format!("Couldn't sync: {e}")),
        }
    }
}

/// A device's own record is refreshed at least this often.
const SEEN_EVERY_MS: u64 = 60 * 60 * 1000;

/// One sync pass (blocking file and database work).
pub fn pass(store: &SyncStore, folder: &Path, tag: &str, groups: Groups, device_name: &str, now: u64) -> Result<PassOutcome, PassError> {
    if !folder.is_dir() {
        return Err(PassError::FolderMissing);
    }
    let dir = file::dir(folder);
    std::fs::create_dir_all(&dir).map_err(|e| PassError::Io(e.to_string()))?;
    let path = dir.join(file::file_name(tag));
    let main = match file::read(&path) {
        Ok(m) => m,
        Err(ReadError::Unreadable) => return Err(PassError::Unreadable),
        Err(ReadError::Newer) => return Err(PassError::Newer),
        Err(ReadError::Io(e)) => return Err(PassError::Io(e)),
    };
    let had_file = main.is_some();
    let main = main.unwrap_or_default();
    let mut combined = main.entries.clone();
    let mut devices = main.devices.clone();
    let mut copies = Vec::new();
    for c in file::conflicted_copies(&dir, tag) {
        // A copy that can't be read stays where it is.
        if let Ok(Some(d)) = file::read(&c) {
            file::merge_into(&mut combined, &d.entries);
            for (id, seen) in d.devices {
                let newer = devices.get(&id).is_none_or(|s| s.last_seen < seen.last_seen);
                if newer {
                    devices.insert(id, seen);
                }
            }
            copies.push(c);
        }
    }
    let namespaces = groups.namespaces();
    let changed = store.merge(&combined, &namespaces).map_err(|e| PassError::Io(e.to_string()))?;
    let local = store.entries(&namespaces).map_err(|e| PassError::Io(e.to_string()))?;
    let mut out = combined;
    file::merge_into(&mut out, &local);
    file::prune(&mut out, now);
    let _ = store.prune(now.saturating_sub(file::TOMBSTONE_MS));

    let me = store.device().to_string();
    let stale = devices
        .get(&me)
        .is_none_or(|d| d.name != device_name || now.saturating_sub(d.last_seen) >= SEEN_EVERY_MS);
    let entries_changed = out != main.entries;
    if entries_changed || stale || !had_file {
        devices.insert(me.clone(), DeviceSeen { name: device_name.to_string(), last_seen: now });
    }
    let wrote = entries_changed || stale || !had_file || !copies.is_empty() || devices != main.devices;
    if wrote {
        let data = SyncFileData { format: file::FORMAT, app: "Platter".into(), devices: devices.clone(), entries: out };
        file::write_atomic(&path, &data).map_err(|e| PassError::Io(e.to_string()))?;
        for c in &copies {
            let _ = std::fs::remove_file(c);
        }
    }
    let mut seen: Vec<SeenDevice> = devices
        .into_iter()
        .map(|(id, d)| SeenDevice { name: d.name, last_seen: d.last_seen, this_device: id == me })
        .collect();
    seen.sort_by(|a, b| b.this_device.cmp(&a.this_device).then(b.last_seen.cmp(&a.last_seen)));
    Ok(PassOutcome { changed, wrote, merged_copies: copies.len() as u32, devices: seen, path })
}

/// The file's size and time, to notice another device writing it.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

struct Inner {
    store: SyncStore,
    account: Arc<dyn AccountTag>,
    config: watch::Receiver<SyncConfig>,
    device_name: Arc<dyn Fn() -> String + Send + Sync>,
    timing: Timing,
    status: Mutex<SyncFileStatus>,
    events: Mutex<Option<Arc<dyn SyncEvents>>>,
    kick: Notify,
}

/// The background sync. Cheap to clone.
#[derive(Clone)]
pub struct SyncEngine {
    inner: Arc<Inner>,
}

impl SyncEngine {
    pub fn new(
        store: SyncStore,
        account: Arc<dyn AccountTag>,
        config: watch::Receiver<SyncConfig>,
        device_name: Arc<dyn Fn() -> String + Send + Sync>,
        timing: Timing,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                account,
                config,
                device_name,
                timing,
                status: Mutex::new(SyncFileStatus::default()),
                events: Mutex::new(None),
                kick: Notify::new(),
            }),
        }
    }

    pub fn set_events(&self, events: Arc<dyn SyncEvents>) {
        *self.inner.events.lock() = Some(events);
    }

    pub fn store(&self) -> &SyncStore {
        &self.inner.store
    }

    pub fn status(&self) -> SyncFileStatus {
        self.inner.status.lock().clone()
    }

    /// Run a pass now ("Sync now").
    pub fn kick(&self) {
        self.inner.kick.notify_one();
    }

    fn set_status(&self, s: SyncFileStatus) {
        let changed = {
            let mut cur = self.inner.status.lock();
            let changed = *cur != s;
            *cur = s.clone();
            changed
        };
        if changed && let Some(ev) = self.inner.events.lock().clone() {
            ev.status(&s);
        }
    }

    /// One pass with the current settings; returns the file's path when it
    /// ran against a file.
    pub async fn sync_now(&self) -> Option<PathBuf> {
        let cfg = self.inner.config.borrow().clone();
        let folder_text = cfg.folder.as_ref().map(|f| f.to_string_lossy().to_string());
        let Some(folder) = cfg.folder.clone().filter(|_| cfg.enabled) else {
            self.set_status(SyncFileStatus { folder: folder_text, ..Default::default() });
            return None;
        };
        let Some(tag) = self.inner.account.tag().await else {
            self.set_status(SyncFileStatus {
                state: SyncState::SignedOut,
                folder: folder_text,
                message: Some("Sign in to Plex to sync.".into()),
                ..Default::default()
            });
            return None;
        };
        let store = self.inner.store.clone();
        let name = (self.inner.device_name)();
        let groups = cfg.groups;
        let f2 = folder.clone();
        let t2 = tag.clone();
        let result = tokio::task::spawn_blocking(move || {
            let now = store.now();
            pass(&store, &f2, &t2, groups, &name, now).map(|o| (o, now))
        })
        .await
        .unwrap_or_else(|e| Err(PassError::Io(e.to_string())));
        let path = file::dir(&folder).join(file::file_name(&tag));
        let prev = self.status();
        match result {
            Ok((o, now)) => {
                if !o.changed.is_empty() {
                    tracing::info!(namespaces = ?o.changed, "took changes from the sync file");
                }
                self.set_status(SyncFileStatus {
                    state: SyncState::Ok,
                    folder: folder_text,
                    file: Some(o.path.to_string_lossy().to_string()),
                    last_synced_at: Some(now),
                    devices: o.devices,
                    merged_copies: o.merged_copies,
                    message: None,
                });
            }
            Err(e) => {
                let (state, message) = e.state();
                if prev.state != state {
                    tracing::info!(?state, "sync file problem");
                }
                self.set_status(SyncFileStatus {
                    state,
                    folder: folder_text,
                    file: Some(path.to_string_lossy().to_string()),
                    last_synced_at: prev.last_synced_at,
                    devices: prev.devices,
                    merged_copies: 0,
                    message: Some(message),
                });
            }
        }
        Some(path)
    }

    /// Start a new file: the unreadable one is moved aside (kept), then a pass.
    pub async fn replace_file(&self) -> AppResult<()> {
        let cfg = self.inner.config.borrow().clone();
        let folder = cfg.folder.filter(|_| cfg.enabled).ok_or_else(|| AppError::Invalid("Sync is off.".into()))?;
        let tag = self.inner.account.tag().await.ok_or_else(|| AppError::Auth("Sign in to Plex to sync.".into()))?;
        let path = file::dir(&folder).join(file::file_name(&tag));
        let now = self.inner.store.now();
        if path.exists() {
            tokio::task::spawn_blocking(move || file::set_aside(&path, now))
                .await
                .map_err(|e| AppError::Internal(e.to_string()))??;
        }
        self.sync_now().await;
        Ok(())
    }

    /// Run until the config channel closes.
    pub async fn run(self) {
        let t = self.inner.timing;
        let mut config = self.inner.config.clone();
        loop {
            let path = self.sync_now().await;
            let mut last = path.as_deref().and_then(stamp);
            let started = tokio::time::Instant::now();
            // Wait for the next reason to sync.
            loop {
                tokio::select! {
                    r = config.changed() => {
                        if r.is_err() { return; }
                        break;
                    }
                    _ = self.inner.kick.notified() => break,
                    _ = self.inner.store.edited().notified() => {
                        // Quiet for `debounce` before writing.
                        loop {
                            tokio::select! {
                                _ = self.inner.store.edited().notified() => continue,
                                _ = tokio::time::sleep(t.debounce) => break,
                            }
                        }
                        break;
                    }
                    // Sync off (no file): nothing to poll, so no timer.
                    _ = tokio::time::sleep(t.poll), if path.is_some() => {
                        let now = path.as_deref().and_then(stamp);
                        if now != last {
                            last = now;
                            break;
                        }
                        if started.elapsed() >= t.reread {
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// The sync settings, pushed from Settings to the engine.
#[derive(Clone)]
pub struct ConfigSwitch(Arc<watch::Sender<SyncConfig>>);

impl Default for ConfigSwitch {
    fn default() -> Self {
        Self(Arc::new(watch::channel(SyncConfig::default()).0))
    }
}

impl ConfigSwitch {
    pub fn set(&self, c: SyncConfig) {
        self.0.send_if_modified(|cur| {
            let changed = *cur != c;
            *cur = c;
            changed
        });
    }

    pub fn get(&self) -> SyncConfig {
        self.0.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<SyncConfig> {
        self.0.subscribe()
    }
}
