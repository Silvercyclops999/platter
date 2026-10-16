//! Tauri commands for play history from Plex, Tautulli and Last.fm
//! (docs/IPC.md "Play history").

use std::sync::Arc;

use tauri::State;

use super::tautulli::{Tautulli, TautulliStatus};
use super::{HistoryStatus, PlayHistory};
use crate::db::listens::ImportSource;
use crate::error::{AppError, AppResult};

/// Managed state.
#[derive(Clone)]
pub struct PlayHistoryState {
    pub history: PlayHistory,
    pub tautulli: Arc<Tautulli>,
}

#[tauri::command]
pub async fn play_history_status(state: State<'_, PlayHistoryState>) -> AppResult<HistoryStatus> {
    state.history.status().await
}

/// Look for new plays from every source now (failures are tried again).
#[tauri::command]
pub async fn play_history_update(state: State<'_, PlayHistoryState>) -> AppResult<HistoryStatus> {
    super::update_now();
    state.history.status().await
}

/// Stop the read going on now; it goes on from the same page later.
#[tauri::command]
pub async fn play_history_cancel(state: State<'_, PlayHistoryState>) -> AppResult<HistoryStatus> {
    state.history.cancel();
    state.history.status().await
}

/// Delete the plays brought in from one source (only that source's).
#[tauri::command]
pub async fn play_history_remove(state: State<'_, PlayHistoryState>, source: ImportSource) -> AppResult<HistoryStatus> {
    state.history.remove(source).await?;
    state.history.status().await
}

#[tauri::command]
pub async fn tautulli_status(state: State<'_, PlayHistoryState>) -> AppResult<TautulliStatus> {
    let t = state.tautulli.clone();
    tokio::task::spawn_blocking(move || Ok(t.status())).await.map_err(|e| AppError::Internal(e.to_string()))?
}

/// Save (or with null remove) Tautulli's API key in the OS keychain.
#[tauri::command]
pub async fn tautulli_key_set(state: State<'_, PlayHistoryState>, key: Option<String>) -> AppResult<TautulliStatus> {
    let t = state.tautulli.clone();
    let st = tokio::task::spawn_blocking(move || t.set_key(key.as_deref())).await.map_err(|e| AppError::Internal(e.to_string()))??;
    if st.ready() {
        PlayHistory::poke();
    }
    Ok(st)
}
