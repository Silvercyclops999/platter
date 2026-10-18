//! Tauri side of Sync to a device (docs/IPC.md "Devices"): the commands,
//! the events, and the real song source, pace and drive list.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use super::drives::{self, Volume};
use super::run::{DevicesView, Opts, Pace, SyncEvents, SyncStatus, TrackFiles, Volumes};
use super::{BUDGET_MAX_GB, Device, DeviceFormat, SelKind, store};
use crate::audio::cache::PrecacheHost;
use crate::audio::gate::TransferGate;
use crate::audio::source::LocalAudio;
use crate::db::ArtFetcher;
use crate::downloads::DownloadSource;
use crate::downloads::transfer::{self, Ctx, Outcome};
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::settings::SettingsState;

pub use super::run::Devices;

pub const EV_PROGRESS: &str = "devices://progress";
pub const EV_CHANGED: &str = "devices://changed";

impl<R: Runtime> SyncEvents for AppHandle<R> {
    fn progress(&self, s: &SyncStatus) {
        let _ = self.emit(EV_PROGRESS, s);
    }
    fn changed(&self) {
        let _ = self.emit(EV_CHANGED, ());
    }
}

/// Songs from each one's server through the player's transfer gate at
/// background priority, or the downloaded file when it's the original.
pub struct ServerFiles {
    pub source: Arc<dyn DownloadSource>,
    pub local: Arc<dyn LocalAudio>,
    pub gate: Arc<TransferGate>,
    pub host: Arc<dyn PrecacheHost>,
    pub client: reqwest::Client,
    pub art: Option<Arc<dyn ArtFetcher>>,
}

#[async_trait]
impl TrackFiles for ServerFiles {
    async fn fetch(&self, t: &Track, dest: &Path, stop: &(dyn Fn() -> bool + Sync)) -> Result<(), String> {
        let want_ext = super::names::original_ext(&t.part_key, t.codec.as_deref(), t.container.as_deref());
        if let Some(p) = self.local.path(t)
            && p.extension().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(&want_ext))
            && tokio::fs::copy(&p, dest).await.is_ok()
        {
            return Ok(());
        }
        let fetch = self.source.fetch(t, None).await.map_err(|e| e.to_string())?;
        let keep_going = || !stop();
        let progress = |_: u64, _: Option<u64>| {};
        let ctx = Ctx {
            client: &self.client,
            gate: &self.gate,
            host: Some(&*self.host),
            room: u64::MAX,
            keep_going: &keep_going,
            progress: &progress,
        };
        let out = transfer::fetch_file(&ctx, &fetch, dest).await;
        self.source.finished(&fetch).await;
        match out {
            Outcome::Done(_) => Ok(()),
            Outcome::Failed(e) => Err(e),
            Outcome::Full => Err("There isn't room on this computer to fetch the song.".into()),
            Outcome::Stopped => Err(super::encode::STOPPED.into()),
        }
    }

    async fn art(&self, t: &Track) -> Option<Vec<u8>> {
        let thumb = t.thumb.as_deref()?;
        let px = super::COVER_PX;
        self.art.as_ref()?.fetch(thumb, px, px).await.ok().map(|a| a.bytes.to_vec())
    }
}

/// Waits while playback is still filling its buffers, a fullscreen game or
/// presentation is in front, the computer runs hot, or other programs keep
/// the processor busy.
pub struct SystemPace {
    pub host: Arc<dyn PrecacheHost>,
    pub load: crate::platform::cpu::SystemLoad,
}

/// Other programs above this share of the processor count as busy.
const BUSY_PERCENT: f32 = 60.0;

impl Pace for SystemPace {
    fn busy(&self) -> bool {
        !self.host.precache_allowed()
            || crate::platform::cpu::fullscreen_in_front()
            || crate::platform::cpu::running_hot()
            || self.load.others_percent().is_some_and(|p| p > BUSY_PERCENT)
    }
}

pub struct SystemVolumes;

impl Volumes for SystemVolumes {
    fn list(&self) -> Vec<Volume> {
        drives::list()
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::Internal(e.to_string()))?
}

async fn opts(settings: &SettingsState) -> AppResult<Opts> {
    let s = settings.get().await?;
    Ok(Opts { default_format: s.device_format, delete_removed: s.device_delete_removed, liked_min: s.liked_min_rating() })
}

/// Known devices (and where each is plugged in), new removable drives,
/// and the sync running now.
#[tauri::command]
pub async fn devices_list(devices: State<'_, Devices>) -> AppResult<DevicesView> {
    let d = devices.inner().clone();
    blocking(move || Ok(d.view())).await
}

/// Remember a drive as a device: its name and the music folder on it.
#[tauri::command]
pub async fn devices_add(devices: State<'_, Devices>, root: String, name: String, folder: String) -> AppResult<Device> {
    let d = devices.inner().clone();
    blocking(move || d.add(Path::new(&root), &name, &folder)).await
}

/// Pick the music folder on a drive. Returns it below the drive's top, or
/// None when the picker was cancelled.
#[tauri::command]
pub async fn devices_choose_folder(app: AppHandle, root: String) -> AppResult<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let d = app.dialog().file().set_title("Choose the music folder on the device").set_directory(&root);
    crate::platform::pick_folder(d, move |p| {
        let _ = tx.send(p);
    });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let picked: PathBuf = picked.into_path().map_err(|e| AppError::Io(format!("folder: {e}")))?;
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let rel = lower(&picked)
        .strip_prefix(lower(Path::new(&root)))
        .map(|_| picked.components().skip(Path::new(&root).components().count()).collect::<PathBuf>())
        .map_err(|_| AppError::Invalid("Choose a folder on the device.".into()))?;
    let rel = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
    Ok(Some(super::clean_folder(&rel).map_err(AppError::Invalid)?))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePatch {
    pub name: Option<String>,
    pub folder: Option<String>,
    /// A format, or "default" to follow the Default format setting.
    pub format: Option<String>,
    /// GB; 0 removes the limit.
    pub budget_gb: Option<f64>,
}

/// Apply a patch to a device (name, music folder, format, size limit).
pub fn apply(d: &mut Device, p: &DevicePatch) -> AppResult<()> {
    if let Some(n) = &p.name {
        d.name = store::clean_name(n)?;
    }
    if let Some(f) = &p.folder {
        d.folder = super::clean_folder(f).map_err(AppError::Invalid)?;
    }
    if let Some(f) = &p.format {
        d.format = match f.as_str() {
            "default" => None,
            other => Some(DeviceFormat::parse(other).ok_or_else(|| AppError::Invalid(format!("Unknown format {other}.")))?),
        };
    }
    if let Some(gb) = p.budget_gb {
        if !gb.is_finite() || !(0.0..=BUDGET_MAX_GB).contains(&gb) {
            return Err(AppError::Invalid("Size limits go from 0 (no limit) up.".into()));
        }
        d.budget_gb = (gb > 0.0).then_some(gb);
    }
    Ok(())
}

#[tauri::command]
pub async fn devices_update(devices: State<'_, Devices>, id: String, patch: DevicePatch) -> AppResult<Device> {
    let d = devices.inner().clone();
    blocking(move || d.update(&id, |x| apply(x, &patch))).await
}

/// Add something to a device's selection, or take it off.
#[tauri::command]
pub async fn devices_select(
    devices: State<'_, Devices>,
    id: String,
    kind: String,
    item_id: Option<String>,
    title: Option<String>,
    on: bool,
) -> AppResult<Device> {
    let kind = SelKind::parse(&kind).ok_or_else(|| AppError::Invalid(format!("Can't put a {kind} on a device.")))?;
    let item_id = item_id.unwrap_or_default();
    if kind != SelKind::Liked && item_id.is_empty() {
        return Err(AppError::Invalid("What should go on the device?".into()));
    }
    let title = title.unwrap_or_default();
    let d = devices.inner().clone();
    blocking(move || {
        d.update(&id, |x| {
            store::set_selected(x, kind, &item_id, &title, on);
            Ok(())
        })
    })
    .await
}

/// Forget a device on this computer (its files stay on it).
#[tauri::command]
pub async fn devices_forget(devices: State<'_, Devices>, id: String) -> AppResult<()> {
    let d = devices.inner().clone();
    blocking(move || d.forget(&id)).await
}

/// Start syncing a device; progress comes as `devices://progress`.
#[tauri::command]
pub async fn devices_sync(devices: State<'_, Devices>, settings: State<'_, SettingsState>, id: String) -> AppResult<SyncStatus> {
    let o = opts(&settings).await?;
    devices.start(&id, o)
}

#[tauri::command]
pub async fn devices_pause(devices: State<'_, Devices>, on: bool) -> AppResult<()> {
    devices.pause(on);
    Ok(())
}

#[tauri::command]
pub async fn devices_cancel(devices: State<'_, Devices>) -> AppResult<()> {
    devices.cancel();
    Ok(())
}

#[tauri::command]
pub async fn devices_status(devices: State<'_, Devices>) -> AppResult<Option<SyncStatus>> {
    Ok(devices.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Device {
        Device {
            id: "d".into(),
            name: "Old".into(),
            volume_id: None,
            volume_label: String::new(),
            folder: String::new(),
            format: None,
            budget_gb: None,
            selection: vec![],
            last_sync: None,
            last_root: None,
        }
    }

    #[test]
    fn patches_check_what_they_set() {
        let mut d = device();
        apply(
            &mut d,
            &DevicePatch { name: Some(" Car ".into()), folder: Some("\\Music\\Synced/".into()), format: Some("opus".into()), budget_gb: Some(8.0) },
        )
        .unwrap();
        assert_eq!((d.name.as_str(), d.folder.as_str(), d.format, d.budget_gb), ("Car", "Music/Synced", Some(DeviceFormat::Opus), Some(8.0)));
        apply(&mut d, &DevicePatch { format: Some("default".into()), budget_gb: Some(0.0), ..Default::default() }).unwrap();
        assert_eq!((d.format, d.budget_gb), (None, None));
        assert!(apply(&mut d, &DevicePatch { format: Some("wma".into()), ..Default::default() }).is_err());
        assert!(apply(&mut d, &DevicePatch { budget_gb: Some(-1.0), ..Default::default() }).is_err());
        assert!(apply(&mut d, &DevicePatch { folder: Some("../up".into()), ..Default::default() }).is_err());
        assert!(apply(&mut d, &DevicePatch { name: Some("  ".into()), ..Default::default() }).is_err());
    }
}
