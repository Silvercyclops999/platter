//! Lidarr's Tauri commands (docs/IPC.md).

use tauri::State;

use super::check::CheckReport;
use super::mbpick::Picked;
use super::request::{MissingReleases, Preview, RequestInput, RequestRecord};
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

/// Which release group to request for an album or a song (MusicBrainz).
#[tauri::command]
pub async fn lidarr_pick(
    lidarr: State<'_, Lidarr>,
    artist: String,
    artist_id: Option<String>,
    title: String,
    song: bool,
    isrc: Option<String>,
) -> AppResult<Picked> {
    lidarr.pick(&artist, artist_id.as_deref(), &title, song, isrc.as_deref()).await
}

/// What a request will do and where its files will land. Reads only.
#[tauri::command]
pub async fn lidarr_request_preview(lidarr: State<'_, Lidarr>, input: RequestInput) -> AppResult<Preview> {
    lidarr.preview(&input).await
}

/// Send a request (or keep it waiting when Lidarr can't be reached).
#[tauri::command]
pub async fn lidarr_request(lidarr: State<'_, Lidarr>, input: RequestInput) -> AppResult<RequestRecord> {
    lidarr.request(input).await
}

/// Requests on this device, newest first, with their latest state.
#[tauri::command]
pub async fn lidarr_requests(lidarr: State<'_, Lidarr>, refresh: bool) -> AppResult<Vec<RequestRecord>> {
    if refresh {
        let _ = lidarr.refresh_statuses().await;
    }
    Ok(lidarr.requests())
}

/// Cancel a waiting request, or clear a finished one.
#[tauri::command]
pub async fn lidarr_request_cancel(lidarr: State<'_, Lidarr>, id: String) -> AppResult<Vec<RequestRecord>> {
    let l = lidarr.inner().clone();
    blocking(move || l.cancel(&id)).await
}

/// An artist's official releases the library lacks, for the artist page.
#[tauri::command]
pub async fn lidarr_missing_releases(lidarr: State<'_, Lidarr>, artist_id: String, artist: String) -> AppResult<MissingReleases> {
    lidarr.missing_releases(&artist_id, &artist).await
}
