//! Tauri commands for Library and Search, per
//! docs/IPC.md. Registered by the integrator in `lib.rs`:
//!
//! ```ignore
//! use platter_lib::db::commands as dbc;
//! .manage(db_handle)
//! .invoke_handler(tauri::generate_handler![
//!     dbc::sync_status, dbc::sync_now, dbc::home, dbc::home_shelf, dbc::artists, dbc::artist_names, dbc::albums,
//!     dbc::playlists, dbc::library_items, dbc::artist_page, dbc::album_page, dbc::playlist_page,
//!     dbc::recent_tracks, dbc::recents,
//!     dbc::search, dbc::search_completions, dbc::search_recents,
//!     dbc::search_recent_add, dbc::search_recent_remove, dbc::search_recents_clear,
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
use super::history;
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
    let view = db.tag_view();
    let server = db.server_view();
    db.read_async(move |c| {
        let mut data = queries::home(c, now_s())?;
        crate::tags::view::home(c, &mut data, view)?;
        super::merge::ViewFilter::load(c, &server)?.home(&mut data);
        Ok(data)
    })
    .await
}

/// The last answer [`home_records`] worked out, until its stamp changes.
static RECORDS: parking_lot::Mutex<Option<(super::records::Stamp, super::records::RecordHome)>> = parking_lot::const_mutex(None);

/// Home led by records in the Studio looks (empty with `shown: false` while
/// the setting is off). Hidden editions and other servers' copies leave.
#[tauri::command]
pub async fn home_records(db: State<'_, DbHandle>) -> AppResult<super::records::RecordHome> {
    use super::records::{self, RecordHome};
    if !crate::settings::load(&db).home_records_first {
        return Ok(RecordHome::default());
    }
    crate::stats::commands::fresh(&db).await?;
    let now_ms = now_s() * 1000;
    let stamp = db.read_async(move |c| records::stamp(c, now_ms)).await?;
    let cached = RECORDS.lock().as_ref().filter(|(s, _)| *s == stamp).map(|(_, v)| v.clone());
    let mut data = match cached {
        Some(v) => v,
        None => {
            let v = db
                .read_async(move |c| {
                    let today = crate::stats::today(c)?;
                    records::record_home(c, now_ms, today)
                })
                .await?;
            *RECORDS.lock() = Some((stamp, v.clone()));
            v
        }
    };
    let view = db.tag_view();
    let server = db.server_view();
    db.read_async(move |c| {
        let hidden = crate::tags::store::hidden_albums(c, view)?;
        let filter = super::merge::ViewFilter::load(c, &server)?;
        let shows = |id: &str| !hidden.contains(id) && filter.shows(id);
        data.resume = data.resume.take().filter(|p| shows(&p.album.id));
        data.new_arrivals.retain(|a| shows(&a.id));
        data.spun_this_week.retain(|s| shows(&s.album.id));
        data.unfinished.retain(|p| shows(&p.album.id));
        data.not_in_a_year.retain(|l| shows(&l.album.id));
        Ok(data)
    })
    .await
}

/// Everything in one Home shelf, for its "Show all" page.
#[tauri::command]
pub async fn home_shelf(db: State<'_, DbHandle>, shelf: HomeShelf, limit: Option<u32>) -> AppResult<Vec<HomeItem>> {
    let limit = limit.unwrap_or(queries::SHELF_ALL_MAX as u32);
    let view = db.tag_view();
    let server = db.server_view();
    db.read_async(move |c| {
        let mut items = queries::home_shelf(c, now_s(), shelf, limit)?;
        let albums_shelf = matches!(shelf, HomeShelf::RecentlyAdded | HomeShelf::JumpBackIn);
        crate::tags::view::home_items(c, &mut items, view, albums_shelf)?;
        super::merge::ViewFilter::load(c, &server)?.home_items(&mut items);
        Ok(items)
    })
    .await
}

#[tauri::command]
pub async fn artists(
    db: State<'_, DbHandle>,
    sort: Option<ArtistSort>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> AppResult<Page<crate::model::Artist>> {
    let (sort, offset, limit) = (sort.unwrap_or_default(), offset.unwrap_or(0), limit.unwrap_or(100));
    let server = db.server_view();
    db.read_async(move |c| queries::artists_on(c, sort, offset, limit, &server)).await
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
    let view = db.tag_view();
    let server = db.server_view();
    db.read_async(move |c| queries::albums_view_on(c, sort, offset, limit, section, view, &server)).await
}

#[tauri::command]
pub async fn artist_names(db: State<'_, DbHandle>) -> AppResult<Vec<ArtistName>> {
    db.read_async(queries::artist_names).await
}

#[tauri::command]
pub async fn playlists(db: State<'_, DbHandle>) -> AppResult<Vec<Playlist>> {
    let server = db.server_view();
    db.read_async(move |c| {
        let mut out = queries::playlists(c)?;
        super::merge::ViewFilter::load(c, &server)?.playlists(&mut out);
        Ok(out)
    })
    .await
}

/// Every playlist, album and artist for the library sidebar.
#[tauri::command]
pub async fn library_items(db: State<'_, DbHandle>) -> AppResult<Vec<LibraryItem>> {
    let server = db.server_view();
    db.read_async(move |c| {
        let mut out = queries::library_items(c)?;
        super::merge::ViewFilter::load(c, &server)?.library_items(&mut out);
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn artist_page(
    app: tauri::AppHandle,
    db: State<'_, DbHandle>,
    info: State<'_, crate::artistinfo::ArtistInfoService>,
    hidden: State<'_, crate::hidden::Hidden>,
    id: String,
) -> AppResult<ArtistPage> {
    use tauri::Manager;
    let source = db.popular_source();
    let key = id.clone();
    let view = db.tag_view();
    let server = db.server_view();
    let hidden = hidden.ids().get();
    let started = std::time::Instant::now();
    // The user's own sections for this artist's albums.
    let custom = match app.try_state::<crate::sync::commands::SyncService>() {
        Some(s) => {
            let artist = id.clone();
            s.store.run(move |s| crate::archive::marks::custom_map(s, Some(&artist))).await.unwrap_or_default()
        }
        None => Default::default(),
    };
    let mut page = db
        .read_async(move |c| {
            let mut page = queries::artist_page_with(c, &key, source, Some(&hidden))?;
            super::merge::ViewFilter::load(c, &server)?.artist_page(c, &mut page)?;
            crate::tags::view::artist_page(c, &mut page, view)?;
            crate::archive::sections::apply(c, &mut page, &custom)?;
            if super::popular::credit_key(&key).is_none() {
                page.eras = crate::archive::eras::artist_eras(c, &key, view.show_edits)?;
            }
            crate::archive::versions::annotate(c, &mut page.popular, view.show_edits)?;
            Ok(page)
        })
        .await?;
    tracing::debug!(ms = started.elapsed().as_millis() as u64, popular = page.popular.len(), "artist page");
    // Nothing will fetch a list with every source for it off.
    page.popular_pending &= info.popular_wanted();
    // Cached similar artists show at once; stale or missing ones refresh in
    // the background and `artist://changed` tells the page. Credited
    // artists have no Plex entry to ask.
    if super::popular::credit_key(&id).is_none() {
        super::similar::spawn_refresh_if_stale(db.inner().clone(), id);
    }
    Ok(page)
}

#[tauri::command]
pub async fn album_page(db: State<'_, DbHandle>, id: String) -> AppResult<AlbumPage> {
    let view = db.tag_view();
    db.read_async(move |c| {
        let mut page = queries::album_page(c, &id)?;
        crate::tags::view::album_page(c, &mut page, view)?;
        crate::archive::versions::annotate(c, &mut page.tracks, view.show_edits)?;
        page.archive = crate::archive::eras::album_archive(c, &id)?;
        Ok(page)
    })
    .await
}

#[tauri::command]
pub async fn playlist_page(db: State<'_, DbHandle>, id: String) -> AppResult<PlaylistPage> {
    let view = db.tag_view();
    db.read_async(move |c| {
        let mut page = queries::playlist_page(c, &id)?;
        crate::archive::versions::annotate(c, &mut page.tracks, view.show_edits)?;
        Ok(page)
    })
    .await
}

#[tauri::command]
pub async fn search(
    db: State<'_, DbHandle>,
    query: String,
    filter: Option<SearchFilter>,
    limit: Option<u32>,
) -> AppResult<SearchResults> {
    let mut out = search::search_with_fallback(db.inner(), &query, filter.unwrap_or_default(), limit).await?;
    let server = db.server_view();
    let f = db.read_async(move |c| super::merge::ViewFilter::load(c, &server)).await?;
    f.search(&mut out);
    // One row per song, the released version first (archive::versions);
    // for an artist's name, only the versions that artist is on.
    let show = db.tag_view().show_edits;
    let artist = search::named_artist(&query, &out.artists);
    db.read_async(move |c| {
        crate::archive::versions::group_search(c, &mut out, show, artist.as_deref())?;
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn search_completions(db: State<'_, DbHandle>, query: String, limit: Option<u32>) -> AppResult<Vec<String>> {
    let n = limit.map_or(search::COMPLETIONS_MAX, |l| (l as usize).min(10));
    db.read_async(move |c| search::completions(c, &query, n)).await
}

/// Most songs `recent_tracks` returns.
const RECENT_TRACKS_MAX: usize = 200;

#[tauri::command]
pub async fn recent_tracks(db: State<'_, DbHandle>, limit: Option<usize>) -> AppResult<Vec<crate::model::Track>> {
    let n = limit.unwrap_or(50).clamp(1, RECENT_TRACKS_MAX);
    db.read_async(move |c| queries::recent_tracks(c, n)).await
}

/// The Recents page: listening sessions and likes from the last
/// [`history::HISTORY_DAYS`] days, newest first.
#[tauri::command]
pub async fn recents(db: State<'_, DbHandle>, limit: Option<usize>) -> AppResult<Vec<history::RecentsEntry>> {
    let n = limit.unwrap_or(history::HISTORY_MAX).clamp(1, history::HISTORY_MAX);
    let liked_min = crate::settings::load(db.inner()).liked_min_rating();
    let since = (now_s() - history::HISTORY_DAYS * 24 * 3600) * 1000;
    db.read_async(move |c| history::recents(c, since, liked_min, n)).await
}

#[tauri::command]
pub async fn search_recents(db: State<'_, DbHandle>) -> AppResult<Vec<SearchHit>> {
    db.read_async(search::recent_list).await
}

#[tauri::command]
pub async fn search_recent_add(db: State<'_, DbHandle>, kind: SearchKind, id: String) -> AppResult<()> {
    db.write_async(move |c| search::recent_add(c, kind, &id)).await
}

#[tauri::command]
pub async fn search_recent_remove(db: State<'_, DbHandle>, kind: SearchKind, id: String) -> AppResult<()> {
    db.write_async(move |c| search::recent_remove(c, kind, &id)).await
}

#[tauri::command]
pub async fn search_recents_clear(db: State<'_, DbHandle>) -> AppResult<()> {
    db.write_async(|c| search::recent_clear(c)).await
}
