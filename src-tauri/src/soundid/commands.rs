//! Tauri commands for identifying by sound (docs/IPC.md "Identify by sound").

use tauri::State;

use super::store::SoundsLike;
use super::{SoundId, Status};
use crate::error::{AppError, AppResult};

/// Whether there's a key, and progress.
#[tauri::command]
pub async fn sound_id_status(s: State<'_, SoundId>) -> AppResult<Status> {
    s.status().await
}

/// Save the user's own AcoustID application key in the keychain (null
/// removes it and goes back to the one the app came with).
#[tauri::command]
pub async fn sound_id_key_set(s: State<'_, SoundId>, key: Option<String>) -> AppResult<Status> {
    let me = s.inner().clone();
    tokio::task::spawn_blocking(move || me.set_key(key)).await.map_err(|e| AppError::Internal(e.to_string()))??;
    s.status().await
}

/// What a song sounds like, if it was identified. A hint only: an
/// unreleased song is never treated as the recording it sounds like.
#[tauri::command]
pub async fn track_sounds_like(s: State<'_, SoundId>, id: String) -> AppResult<Option<SoundsLike>> {
    s.db().read_async(move |c| super::store::sounds_like(c, &id)).await
}
