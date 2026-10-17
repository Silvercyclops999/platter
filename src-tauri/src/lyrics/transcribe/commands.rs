//! Tauri side of transcription (docs/IPC.md "Lyrics transcription"): the
//! commands, the events, and the app's implementations of the service's
//! traits (reading songs, other lyrics sources, pause conditions).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Runtime, State};

use super::audio::{self, AudioError};
use super::model::ModelKind;
use super::{Conditions, OtherLyrics, PauseReason, SongAudio, SongReader, TranscribeEvents, TranscribeStatus, Transcriber, store};
use crate::audio::PlayerHandle;
use crate::audio::cache::cache_key;
use crate::audio::decode::extension_hint;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::lyrics::LyricsService;
use crate::tags::fetch::Reader;

pub const EV_STATUS: &str = "transcribe://status";
pub const EV_DONE: &str = "transcribe://done";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Done<'a> {
    track_id: &'a str,
    found: bool,
}

impl<R: Runtime> TranscribeEvents for AppHandle<R> {
    fn status(&self, s: &TranscribeStatus) {
        let _ = self.emit(EV_STATUS, s);
    }
    fn done(&self, track_id: &str, found: bool) {
        let _ = self.emit(EV_DONE, Done { track_id, found });
    }
}

/// Songs from the pre-cache, else from the server behind playback (never
/// over the relay).
pub struct AppSongs {
    pub player: PlayerHandle,
    pub audio_dir: PathBuf,
    pub parts: std::sync::Arc<dyn crate::tags::scan::PartSource>,
    pub reader: Reader,
}

#[async_trait]
impl SongReader for AppSongs {
    async fn read(&self, track_id: &str, cancel: &AtomicBool) -> Result<SongAudio, AudioError> {
        self.read_up_to(track_id, cancel, audio::MAX_FILE).await?.ok_or_else(|| AudioError::Unreadable("file too big".into()))
    }

    async fn read_up_to(&self, track_id: &str, cancel: &AtomicBool, max_bytes: u64) -> Result<Option<SongAudio>, AudioError> {
        let track = self
            .player
            .lookup_tracks(&[track_id.to_owned()])
            .await
            .map_err(|e| AudioError::Unreachable(e.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| AudioError::Unreadable("not in the library".into()))?;
        let hint = extension_hint(track.codec.as_deref(), track.container.as_deref());
        let cached = self.audio_dir.join(format!("{}.audio", cache_key(&track)));
        if let Some(got) = read_cached(&cached, max_bytes, &hint).await {
            return Ok(got);
        }
        // Playback may be fetching this very song (it's playing or next).
        // Once it's done, a fully streamed song is on disk: look again before
        // reading it a second time over a slow connection.
        if !self.reader.wait_idle(cancel).await {
            return Err(AudioError::Cancelled);
        }
        if let Some(got) = read_cached(&cached, max_bytes, &hint).await {
            return Ok(got);
        }
        let src = self.parts.part(&track.id, &track.part_key).await.map_err(|e| AudioError::Unreachable(e.to_string()))?;
        let bytes = audio::read_whole_up_to(&self.reader, &src, cancel, max_bytes).await?;
        Ok(bytes.map(|bytes| SongAudio { bytes, hint }))
    }
}

/// A song from the pre-cache: None when it isn't there, Some(None) when
/// it's bigger than `max_bytes` (looked up before reading it).
async fn read_cached(path: &std::path::Path, max_bytes: u64, hint: &Option<String>) -> Option<Option<SongAudio>> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    if meta.len() > max_bytes {
        return Some(None);
    }
    let bytes = tokio::fs::read(path).await.ok()?;
    Some((bytes.len() as u64 <= max_bytes).then(|| SongAudio { bytes, hint: hint.clone() }))
}

/// Plex, LRCLIB (when on) and the user's own lyrics.
pub struct AppOther(pub LyricsService);

#[async_trait]
impl OtherLyrics for AppOther {
    async fn has_lyrics(&self, track_id: &str) -> bool {
        self.0.has_other(track_id).await
    }
}

/// Battery power. A slow connection doesn't pause the batch: its reads
/// already wait until playback has nothing left to download and give way the
/// moment it wants the connection, and songs on disk need no network at all.
pub struct AppConditions;

#[async_trait]
impl Conditions for AppConditions {
    async fn pause_reason(&self) -> Option<PauseReason> {
        let battery = tokio::task::spawn_blocking(crate::platform::power::on_battery).await.unwrap_or(false);
        battery.then_some(PauseReason::Battery)
    }
}

#[tauri::command]
pub async fn transcribe_status(t: State<'_, Transcriber>) -> AppResult<TranscribeStatus> {
    Ok(t.status())
}

#[tauri::command]
pub async fn transcribe_model_download(t: State<'_, Transcriber>, model: ModelKind) -> AppResult<()> {
    t.download_model(model)
}

#[tauri::command]
pub async fn transcribe_model_cancel(t: State<'_, Transcriber>) -> AppResult<()> {
    t.cancel_download();
    Ok(())
}

#[tauri::command]
pub async fn transcribe_model_delete(t: State<'_, Transcriber>, model: ModelKind) -> AppResult<()> {
    t.delete_model(model)
}

#[tauri::command]
pub async fn transcribe_track(t: State<'_, Transcriber>, track_id: String) -> AppResult<()> {
    if !crate::lyrics::user::valid_track_id(&track_id) {
        return Err(AppError::Invalid("track id".into()));
    }
    t.settle_gpu().await;
    t.transcribe_track(&track_id)
}

#[tauri::command]
pub async fn transcribe_album(t: State<'_, Transcriber>, album_id: String) -> AppResult<usize> {
    t.transcribe_album(&album_id).await
}

#[tauri::command]
pub async fn transcribe_unreleased(t: State<'_, Transcriber>) -> AppResult<usize> {
    t.transcribe_unreleased().await
}

#[tauri::command]
pub async fn transcribe_cancel(t: State<'_, Transcriber>, track_id: Option<String>) -> AppResult<()> {
    t.cancel(track_id.as_deref());
    Ok(())
}

/// True when the song was transcribed and no words were found.
#[tauri::command]
pub async fn transcribe_found_nothing(db: State<'_, DbHandle>, track_id: String) -> AppResult<bool> {
    db.read_async(move |c| store::found_nothing(c, &track_id)).await
}
