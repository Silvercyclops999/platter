//! Discover's Tauri commands (docs/IPC.md "Discover").

use serde::Serialize;
use tauri::State;

use super::{CatArtist, CatalogueAlbumPage, CatalogueArtistPage, CataloguePart, CataloguePhoto, CatalogueSearch, Discover, FeaturedArtist, GetAlbumsPlan, NewToYou};
use crate::error::AppResult;
use crate::lidarr::waves::{QueuedGet, Waves};

/// Music from MusicBrainz that isn't in the library, for the search page.
#[tauri::command]
pub async fn catalogue_search(discover: State<'_, Discover>, query: String) -> AppResult<CatalogueSearch> {
    discover.search(&query).await
}

/// One kind (`artists`, `albums`, `songs`) of music that isn't in the
/// library, for search; the UI asks for all three at once and shows each as
/// it arrives.
#[tauri::command]
pub async fn catalogue_search_part(discover: State<'_, Discover>, query: String, part: String) -> AppResult<CataloguePart> {
    discover.search_part(&query, &part).await
}

/// A photo of an artist outside the library (asked as their card scrolls
/// into view).
#[tauri::command]
pub async fn catalogue_artist_photo(discover: State<'_, Discover>, mbid: String, name: String) -> AppResult<CataloguePhoto> {
    discover.artist_photo(&mbid, &name).await
}

/// What the catalogue knows about a featured guest (a `credit:` artist
/// page): a photo and their releases by section, with Get. Empty while
/// offline.
#[tauri::command]
pub async fn featured_artist(discover: State<'_, Discover>, id: String, name: String) -> AppResult<FeaturedArtist> {
    discover.featured_artist(&id, &name).await
}

/// Just a featured guest's photo (search's top result card). Empty while
/// offline.
#[tauri::command]
pub async fn featured_artist_photo(discover: State<'_, Discover>, id: String, name: String) -> AppResult<CataloguePhoto> {
    discover.featured_photo(&id, &name).await
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

/// "Get albums": what it would queue for an artist, most popular first.
#[tauri::command]
pub async fn catalogue_get_albums_plan(discover: State<'_, Discover>, mbid: String, name: String, kinds: Vec<String>) -> AppResult<GetAlbumsPlan> {
    discover.get_albums_plan(&mbid, &name, &kinds).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAlbumsQueued {
    /// Albums added to the queue (ones queued or asked for already are left out).
    pub queued: usize,
    pub plan: GetAlbumsPlan,
}

/// "Get albums": work the list out again and queue it, paced like Bring in
/// my listening (each album still goes through every check).
#[tauri::command]
pub async fn catalogue_get_albums(discover: State<'_, Discover>, waves: State<'_, Waves>, mbid: String, name: String, kinds: Vec<String>) -> AppResult<GetAlbumsQueued> {
    let plan = discover.get_albums_plan(&mbid, &name, &kinds).await?;
    let queued = queue_plan(waves.inner(), &plan)?;
    Ok(GetAlbumsQueued { queued, plan })
}

/// Put a plan's albums on the waves' queue, in its order.
pub fn queue_plan(waves: &Waves, plan: &GetAlbumsPlan) -> AppResult<usize> {
    let items = plan
        .albums
        .iter()
        .map(|a| QueuedGet {
            release_group_id: a.release_group_id.clone(),
            title: a.title.clone(),
            artist: plan.artist.clone(),
            artist_mbid: Some(plan.artist_mbid.clone()),
            artist_id: None,
        })
        .collect();
    waves.enqueue(items, &plan.artist)
}
