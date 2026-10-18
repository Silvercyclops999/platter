//! Tauri side of downloads (docs/IPC.md "Downloads"): the commands, the
//! events, and the Plex source.

use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use super::{DownloadEvents, DownloadSource, DownloadStatus, Downloads, Fetch, ItemKey, ItemKind};
use crate::audio::StreamSource;
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::plex::PlexState;
use crate::plex::client::StreamRequest;
use crate::plex::types::ConnectionKind;
use crate::settings::SettingsState;

pub const EV_STATUS: &str = "downloads://status";
pub const EV_CHANGED: &str = "downloads://changed";

impl<R: Runtime> DownloadEvents for AppHandle<R> {
    fn status(&self, s: &DownloadStatus) {
        let _ = self.emit(EV_STATUS, s);
    }
    fn files_changed(&self) {
        let _ = self.emit(EV_CHANGED, ());
    }
}

/// Downloads from the signed-in Plex server: GETs of the media part, or of
/// the universal transcoder for MP3 qualities. The token goes in headers only.
pub struct PlexSource(pub PlexState);

fn source_of(req: StreamRequest, codec: Option<String>) -> StreamSource {
    StreamSource {
        url: req.url.to_string(),
        headers: req
            .headers
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect(),
        size: None,
        codec_hint: codec,
    }
}

#[async_trait]
impl DownloadSource for PlexSource {
    async fn connection(&self) -> Option<ConnectionKind> {
        if self.0.offline_by_choice() || !self.0.connected() {
            return None;
        }
        self.0.client().await.ok()?.connection()
    }

    async fn fetch(&self, track: &Track, kbps: Option<u32>) -> AppResult<Fetch> {
        let client = self.0.client_for(&track.id).await?;
        match kbps {
            None => Ok(Fetch {
                src: source_of(client.stream_request(&track.part_key)?, track.codec.clone()),
                resumable: true,
                session: None,
            }),
            Some(k) => {
                let session = uuid::Uuid::new_v4().to_string();
                Ok(Fetch {
                    src: source_of(client.transcode_request(&track.id, k, &session)?, Some("mp3".into())),
                    resumable: false,
                    session: Some(session),
                })
            }
        }
    }

    async fn finished(&self, fetch: &Fetch) {
        if let Some(s) = &fetch.session
            && let Ok(c) = self.0.client().await
            && let Err(e) = c.transcode_stop(s).await
        {
            tracing::debug!("transcode stop: {e}");
        }
    }
}

async fn status_of(d: Downloads) -> AppResult<DownloadStatus> {
    tokio::task::spawn_blocking(move || d.status())
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn downloads_status(downloads: State<'_, Downloads>) -> AppResult<DownloadStatus> {
    status_of(downloads.inner().clone()).await
}

/// Ids of every downloaded song.
#[tauri::command]
pub async fn downloaded_tracks(downloads: State<'_, Downloads>) -> AppResult<Vec<String>> {
    Ok(downloads.downloaded_ids())
}

/// Turn an item's download on or off. Liked Songs is the "Download Liked
/// Songs" setting, so it changes that too. A mix sends its songs.
#[tauri::command]
pub async fn downloads_set(
    downloads: State<'_, Downloads>,
    settings: State<'_, SettingsState>,
    kind: String,
    id: Option<String>,
    on: bool,
    title: Option<String>,
    track_ids: Option<Vec<String>>,
) -> AppResult<DownloadStatus> {
    let kind = ItemKind::parse(&kind)?;
    let key = ItemKey::new(kind, id.as_deref().unwrap_or_default());
    if kind != ItemKind::Liked && key.id.is_empty() {
        return Err(AppError::Invalid("Which one to download?".into()));
    }
    if kind == ItemKind::Liked {
        settings
            .modify(|s| {
                let changed = s.download_liked != on;
                s.download_liked = on;
                Ok(changed)
            })
            .await?;
    }
    if on {
        let title = title.unwrap_or_else(|| if kind == ItemKind::Liked { "Liked Songs".into() } else { String::new() });
        downloads.add(key, title, track_ids).await?;
    } else {
        downloads.remove(key).await?;
    }
    status_of(downloads.inner().clone()).await
}

/// Failed and missing songs try again (one item's, or every one).
#[tauri::command]
pub async fn downloads_retry(downloads: State<'_, Downloads>, kind: Option<String>, id: Option<String>) -> AppResult<()> {
    let key = match kind {
        Some(k) => Some(ItemKey::new(ItemKind::parse(&k)?, id.as_deref().unwrap_or_default())),
        None => None,
    };
    downloads.retry(key).await
}

/// Delete every download. Returns the bytes freed.
#[tauri::command]
pub async fn downloads_remove_all(downloads: State<'_, Downloads>, settings: State<'_, SettingsState>) -> AppResult<u64> {
    settings
        .modify(|s| {
            let changed = s.download_liked;
            s.download_liked = false;
            Ok(changed)
        })
        .await?;
    downloads.remove_all().await
}

/// Pick a new folder for downloads; the files move there. Returns the folder,
/// or None when the picker was cancelled.
#[tauri::command]
pub async fn downloads_choose_folder(
    app: AppHandle,
    downloads: State<'_, Downloads>,
    settings: State<'_, SettingsState>,
) -> AppResult<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let d = app.dialog().file().set_title("Choose where downloads go").set_directory(downloads.dir());
    crate::platform::pick_folder(d, move |p| {
        let _ = tx.send(p);
    });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let dir = picked.into_path().map_err(|e| AppError::Io(format!("folder: {e}")))?;
    let dir = dir.join("Platter downloads");
    downloads.move_to(dir.clone()).await?;
    let saved = dir.to_string_lossy().into_owned();
    let s2 = saved.clone();
    settings
        .modify(move |s| {
            s.download_folder = s2;
            Ok(true)
        })
        .await?;
    Ok(Some(saved))
}
