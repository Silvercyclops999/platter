//! Tauri commands for Library and Search, per
//! docs/IPC.md. Registered by the integrator in `lib.rs`:
//!
//! ```ignore
//! use platter_lib::db::commands as dbc;
//! .manage(db_handle)
//! .invoke_handler(tauri::generate_handler![
//!     dbc::sync_status, dbc::sync_now, dbc::home, dbc::artists, dbc::artist_names, dbc::albums,
//!     dbc::playlists, dbc::artist_page, dbc::album_page, dbc::playlist_page,
//!     dbc::search, dbc::recent_searches, dbc::recent_search_add,
//!     dbc::recent_searches_clear,
//! ])
//! ```
//!
//! All reads run on the blocking pool against read-only connections, so a
//! running sync never stalls the UI.

use tauri::State;

use crate::error::AppResult;
use crate::model::{Playlist, ReleaseSection};

use super::DbHandle;
use super::queries::{self, now_s};
use super::search;
use super::types::*;

#[tauri::command]
pub async fn sync_status(db: State<'_, DbHandle>) -> AppResult<SyncStatus> {
    let db = db.inner().clone();
    tokio::task::spawn_blocking(move || db.sync_status())
        .await
        .map_err(|e| crate::error::AppError::Internal(e.to_string()))
}

/// Starts a background sync (no-op if one is running). Progress arrives as
/// `sync://progress`; `library://changed` fires as data lands.
#[tauri::command]
pub async fn sync_now(db: State<'_, DbHandle>, full: Option<bool>) -> AppResult<()> {
    db.start_sync(full.unwrap_or(false))?;
    Ok(())
}

#[tauri::command]
pub async fn home(db: State<'_, DbHandle>) -> AppResult<HomeData> {
    db.read_async(|c| queries::home(c, now_s())).await
}

#[tauri::command]
pub async fn artists(
    db: State<'_, DbHandle>,
    sort: Option<ArtistSort>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> AppResult<Page<crate::model::Artist>> {
    let (sort, offset, limit) = (sort.unwrap_or_default(), offset.unwrap_or(0), limit.unwrap_or(100));
    db.read_async(move |c| queries::artists(c, sort, offset, limit)).await
}

#[tauri::command]
pub async fn albums(
    db: State<'_, DbHandle>,
    sort: Option<AlbumSort>,
    offset: Option<u32>,
    limit: Option<u32>,
    section: Option<ReleaseSection>,
) -> AppResult<Page<crate::model::Album>> {
    let (sort, offset, limit) = (sort.unwrap_or_default(), offset.unwrap_or(0), limit.unwrap_or(100));
    db.read_async(move |c| queries::albums(c, sort, offset, limit, section)).await
}

#[tauri::command]
pub async fn artist_names(db: State<'_, DbHandle>) -> AppResult<Vec<ArtistName>> {
    db.read_async(queries::artist_names).await
}

#[tauri::command]
pub async fn playlists(db: State<'_, DbHandle>) -> AppResult<Vec<Playlist>> {
    db.read_async(queries::playlists).await
}

#[tauri::command]
pub async fn artist_page(db: State<'_, DbHandle>, id: String) -> AppResult<ArtistPage> {
    let source = db.popular_source();
    db.read_async(move |c| queries::artist_page_by(c, &id, source)).await
}

#[tauri::command]
pub async fn album_page(db: State<'_, DbHandle>, id: String) -> AppResult<AlbumPage> {
    db.read_async(move |c| queries::album_page(c, &id)).await
}

#[tauri::command]
pub async fn playlist_page(db: State<'_, DbHandle>, id: String) -> AppResult<PlaylistPage> {
    db.read_async(move |c| queries::playlist_page(c, &id)).await
}

#[tauri::command]
pub async fn search(
    db: State<'_, DbHandle>,
    query: String,
    filter: Option<SearchFilter>,
    limit: Option<u32>,
) -> AppResult<SearchResults> {
    search::search_with_fallback(db.inner(), &query, filter.unwrap_or_default(), limit).await
}

#[tauri::command]
pub async fn recent_searches(db: State<'_, DbHandle>) -> AppResult<Vec<String>> {
    db.read_async(search::recent_list).await
}

#[tauri::command]
pub async fn recent_search_add(db: State<'_, DbHandle>, query: String) -> AppResult<()> {
    db.write_async(move |c| search::recent_add(c, &query)).await
}

#[tauri::command]
pub async fn recent_searches_clear(db: State<'_, DbHandle>) -> AppResult<()> {
    db.write_async(|c| search::recent_clear(c)).await
}
