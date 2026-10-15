//! Lidarr's Tauri commands (docs/IPC.md).

use tauri::State;

use super::check::CheckReport;
use super::{Lidarr, LidarrStatus};
use crate::error::{AppError, AppResult};

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn lidarr_status(lidarr: State<'_, Lidarr>) -> AppResult<LidarrStatus> {
    let l = lidarr.inner().clone();
    blocking(move || Ok(l.status())).await
}

/// Save (or with null remove) Lidarr's API key in the OS keychain.
#[tauri::command]
pub async fn lidarr_key_set(lidarr: State<'_, Lidarr>, key: Option<String>) -> AppResult<LidarrStatus> {
    let l = lidarr.inner().clone();
    blocking(move || l.set_key(key.as_deref())).await
}

/// Save extra headers for the away address (`Name: value` lines; null or
/// blank removes them), kept in the OS keychain.
#[tauri::command]
pub async fn lidarr_headers_set(lidarr: State<'_, Lidarr>, text: Option<String>) -> AppResult<LidarrStatus> {
    let l = lidarr.inner().clone();
    blocking(move || l.set_headers(text.as_deref())).await
}

/// The read-only setup check.
#[tauri::command]
pub async fn lidarr_check(lidarr: State<'_, Lidarr>) -> AppResult<CheckReport> {
    lidarr.check().await
}

/// The user says an empty root folder is (or isn't) separate from the Plex
/// music folder.
#[tauri::command]
pub async fn lidarr_confirm_root(lidarr: State<'_, Lidarr>, path: String, confirmed: bool) -> AppResult<()> {
    let l = lidarr.inner().clone();
    blocking(move || l.confirm_root(&path, confirmed)).await
}
