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
use crate::audio::events::PlayerStatus;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::lyrics::LyricsService;
use crate::plex::PlexState;
use crate::plex::types::ConnectionKind;
use crate::tags::fetch::Reader;
use crate::tags::scan::PartSource;

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
    pub parts: crate::tags::commands::PlexParts,
    pub reader: Reader,
}

#[async_trait]
impl SongReader for AppSongs {
    async fn read(&self, track_id: &str, cancel: &AtomicBool) -> Result<SongAudio, AudioError> {
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
        if let Ok(bytes) = tokio::fs::read(&cached).await {
            return Ok(SongAudio { bytes, hint });
        }
        let src = self.parts.part(&track.part_key).await.map_err(|e| AudioError::Unreachable(e.to_string()))?;
        let bytes = audio::read_whole(&self.reader, &src, cancel).await?;
        Ok(SongAudio { bytes, hint })
    }
}

/// Plex, LRCLIB (when on) and the user's own lyrics.
pub struct AppOther(pub LyricsService);

#[async_trait]
impl OtherLyrics for AppOther {
    async fn has_lyrics(&self, track_id: &str) -> bool {
        self.0.has_other(track_id).await
    }
}

/// Battery power, or music playing over remote access or the relay.
pub struct AppConditions {
    pub player: PlayerHandle,
    pub plex: PlexState,
}

#[async_trait]
impl Conditions for AppConditions {
    async fn pause_reason(&self) -> Option<PauseReason> {
        let battery = tokio::task::spawn_blocking(crate::platform::power::on_battery).await.unwrap_or(false);
        if battery {
            return Some(PauseReason::Battery);
        }
        let playing = self.player.state().await.is_ok_and(|s| s.status == PlayerStatus::Playing);
        if playing {
            let slow = match self.plex.client().await {
                Ok(c) => matches!(c.connection(), Some(ConnectionKind::Remote | ConnectionKind::Relay)),
                Err(_) => false,
            };
            if slow {
                return Some(PauseReason::SlowConnection);
            }
        }
        None
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
