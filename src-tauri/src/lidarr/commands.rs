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

// ---------------------------------------------------------------- upgrades

use super::upgrade::{AlbumUpgrade, BulkResult, Candidates, Confirmed, Offer, SwapResult, Under, UpgradeRecord, Upgrades};
use crate::musicshare::ShareMap;

/// Whether an album's menu offers "Look for a better copy", and its upgrade.
#[tauri::command]
pub async fn lidarr_upgrade_album(upgrades: State<'_, Upgrades>, album_id: String) -> AppResult<AlbumUpgrade> {
    upgrades.for_album(&album_id).await
}

/// Ask Lidarr for a better copy of an album, into the staging folder.
#[tauri::command]
pub async fn lidarr_upgrade_request(upgrades: State<'_, Upgrades>, album_id: String) -> AppResult<UpgradeRecord> {
    upgrades.request(&album_id).await
}

/// Ask for better copies of every released album under a quality.
#[tauri::command]
pub async fn lidarr_upgrade_bulk(upgrades: State<'_, Upgrades>, under: Under) -> AppResult<BulkResult> {
    upgrades.bulk(under).await
}

/// Upgrades on this computer, newest first; refresh follows them in Lidarr.
#[tauri::command]
pub async fn lidarr_upgrades(upgrades: State<'_, Upgrades>, refresh: bool) -> AppResult<Vec<UpgradeRecord>> {
    if refresh {
        let _ = upgrades.refresh().await;
    }
    Ok(upgrades.records())
}

/// What swapping would do now: songs that upgrade and songs kept, with why.
#[tauri::command]
pub async fn lidarr_upgrade_offer(upgrades: State<'_, Upgrades>, id: String) -> AppResult<Offer> {
    upgrades.offer(&id).await
}

/// Swap the better copies in (null: every song that upgrades).
#[tauri::command]
pub async fn lidarr_upgrade_swap(upgrades: State<'_, Upgrades>, id: String, track_ids: Option<Vec<String>>) -> AppResult<SwapResult> {
    upgrades.swap(&id, track_ids).await
}

/// Put the old files of the last swap back exactly.
#[tauri::command]
pub async fn lidarr_upgrade_undo(upgrades: State<'_, Upgrades>, id: String) -> AppResult<UpgradeRecord> {
    upgrades.undo(&id).await
}

/// Clear an upgrade from the list.
#[tauri::command]
pub async fn lidarr_upgrade_clear(upgrades: State<'_, Upgrades>, id: String) -> AppResult<Vec<UpgradeRecord>> {
    let u = upgrades.inner().clone();
    blocking(move || u.clear(&id)).await
}

/// Where this computer reaches the music share (this device only).
#[tauri::command]
pub async fn music_share_get(lidarr: State<'_, Lidarr>) -> AppResult<ShareMap> {
    let db = lidarr.inner.db.clone();
    blocking(move || Ok(crate::musicshare::load(&db))).await
}

#[tauri::command]
pub async fn music_share_set(lidarr: State<'_, Lidarr>, map: ShareMap) -> AppResult<ShareMap> {
    let db = lidarr.inner.db.clone();
    blocking(move || crate::musicshare::save(&db, &map)).await
}

/// Plex's music folders, as Plex sees them (to fill in the share mapping).
#[tauri::command]
pub async fn music_share_plex_folders(lidarr: State<'_, Lidarr>) -> AppResult<Vec<String>> {
    let mut l = lidarr.inner.plex.music_files().await?.locations;
    l.sort();
    l.dedup();
    Ok(l)
}

/// The album's songs and the releases it could be, to confirm one.
#[tauri::command]
pub async fn lidarr_upgrade_candidates(upgrades: State<'_, Upgrades>, album_id: String) -> AppResult<Candidates> {
    upgrades.candidates(&album_id).await
}

/// Confirm (or with null forget) which release group an album is.
#[tauri::command]
pub async fn lidarr_upgrade_confirm(upgrades: State<'_, Upgrades>, album_id: String, release_group_id: Option<String>) -> AppResult<Option<Confirmed>> {
    upgrades.confirm(&album_id, release_group_id.as_deref()).await
}

/// The release groups an artist is limited to (empty: no limit).
#[tauri::command]
pub async fn lidarr_allowlist(lidarr: State<'_, Lidarr>, artist: String, artist_id: Option<String>) -> AppResult<Vec<String>> {
    Ok(lidarr.allowlist(&artist, artist_id.as_deref()).await)
}

/// Limit an artist to these release groups (empty removes the limit).
#[tauri::command]
pub async fn lidarr_allowlist_set(lidarr: State<'_, Lidarr>, artist: String, artist_id: Option<String>, release_group_ids: Vec<String>) -> AppResult<Vec<String>> {
    lidarr.set_allowlist(&artist, artist_id.as_deref(), release_group_ids).await
}
