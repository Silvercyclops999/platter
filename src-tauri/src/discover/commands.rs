//! Discover's Tauri commands (docs/IPC.md "Discover").

use tauri::State;

use super::{CatArtist, CatalogueAlbumPage, CatalogueArtistPage, CatalogueSearch, Discover, NewToYou};
use crate::error::AppResult;

/// Music from MusicBrainz that isn't in the library, for the search page.
#[tauri::command]
pub async fn catalogue_search(discover: State<'_, Discover>, query: String) -> AppResult<CatalogueSearch> {
    discover.search(&query).await
}

/// A page for an artist outside the library (by MusicBrainz id).
#[tauri::command]
pub async fn catalogue_artist(discover: State<'_, Discover>, mbid: String) -> AppResult<CatalogueArtistPage> {
    discover.artist_page(&mbid).await
}

/// A page for an album outside the library (by release group id).
#[tauri::command]
pub async fn catalogue_album(discover: State<'_, Discover>, release_group_id: String) -> AppResult<CatalogueAlbumPage> {
    discover.album_page(&release_group_id).await
}

/// Artists outside the library for a library artist's "Fans also like".
#[tauri::command]
pub async fn catalogue_fans_also_like(discover: State<'_, Discover>, artist_id: String, name: String) -> AppResult<Vec<CatArtist>> {
    discover.fans_also_like(&artist_id, &name).await
}

/// Artists like the ones played most that aren't in the library.
#[tauri::command]
pub async fn new_to_you(discover: State<'_, Discover>) -> AppResult<NewToYou> {
    discover.new_to_you().await
}
