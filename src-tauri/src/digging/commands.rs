//! Tauri commands for crates (docs/IPC.md "Crates"). The index is worked out
//! once and kept until the library, the credits or the view change.

use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

use super::{CrateEntry, CrateKind, Index, PREVIEW, SHELF_N, Stamp};
use crate::db::DbHandle;
use crate::db::merge::{ServerView, ViewFilter};
use crate::db::queries::albums_by_ids;
use crate::error::AppResult;
use crate::model::Album;
use crate::sleeve::service::SleeveService;
use crate::tags::TagView;

/// One crate on the Crates page: its first sleeves and how many it holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crate {
    pub kind: CrateKind,
    pub key: String,
    pub name: String,
    pub count: usize,
    pub artists: usize,
    pub from_year: Option<i32>,
    pub to_year: Option<i32>,
    /// The first [`PREVIEW`] albums, by year.
    pub albums: Vec<Album>,
}

/// IPC `CrateList`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrateList {
    pub kind: CrateKind,
    /// Biggest first.
    pub crates: Vec<Crate>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrateAlbum {
    pub album: Album,
    /// Its songs have places on the sound map.
    pub on_map: bool,
}

/// IPC `CratePage`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CratePage {
    pub kind: CrateKind,
    pub key: String,
    pub name: String,
    pub artists: usize,
    pub from_year: Option<i32>,
    pub to_year: Option<i32>,
    pub albums: Vec<CrateAlbum>,
    /// For an era: its years that are crates themselves.
    pub years: Vec<i32>,
    /// Every song, album by album, for Play all and Shuffle ([`PLAY_MAX`] at most).
    pub track_ids: Vec<String>,
}

/// Songs a crate's Play all queues at most.
pub const PLAY_MAX: usize = 5000;

/// IPC `AlbumShelf`: "More from <label>" and the like on an album page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumShelf {
    pub kind: CrateKind,
    pub key: String,
    pub name: String,
    /// Albums in the whole crate, this one included.
    pub count: usize,
    /// The crate's other albums: other artists first, then by year.
    pub albums: Vec<Album>,
}

type Key = (Stamp, TagView, ServerView);

static INDEX: parking_lot::Mutex<Option<(Key, Arc<Index>)>> = parking_lot::const_mutex(None);

fn crates_on(db: &DbHandle) -> bool {
    crate::settings::load(db).show_crates
}

/// The index for the library as shown, from the cache when nothing moved.
async fn index(db: &DbHandle, sleeve: &SleeveService) -> AppResult<Arc<Index>> {
    sleeve.backfill_facts().await;
    let view = db.tag_view();
    let server = db.server_view();
    let stamp = db.read_async(super::stamp).await?;
    let key: Key = (stamp, view, server.clone());
    if let Some((k, ix)) = INDEX.lock().as_ref()
        && *k == key
    {
        return Ok(ix.clone());
    }
    let ix = Arc::new(db.read_async(move |c| index_for(c, view, &server)).await?);
    *INDEX.lock() = Some((key, ix.clone()));
    Ok(ix)
}

/// The index for one view: hidden editions and other servers' copies leave.
pub fn index_for(c: &Connection, view: TagView, server: &ServerView) -> AppResult<Index> {
    let hidden: HashSet<String> = crate::tags::store::hidden_albums(c, view)?;
    let filter = ViewFilter::load(c, server)?;
    super::build(c, &|id: &str| !hidden.contains(id) && filter.shows(id))
}

fn span_of(e: &CrateEntry) -> (Option<i32>, Option<i32>) {
    (e.years.map(|y| y.0), e.years.map(|y| y.1))
}

/// Every crate of one kind, biggest first.
pub fn list(c: &Connection, ix: &Index, kind: CrateKind) -> AppResult<CrateList> {
    let mut entries: Vec<&CrateEntry> = ix.of_kind(kind).collect();
    match kind {
        // Eras and years read in time order.
        CrateKind::Era | CrateKind::Year => entries.sort_by(|a, b| b.key.cmp(&a.key)),
        _ => entries.sort_by(|a, b| b.album_ids.len().cmp(&a.album_ids.len()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))),
    }
    let mut crates = Vec::with_capacity(entries.len());
    for e in entries {
        let ids: Vec<String> = e.album_ids.iter().take(PREVIEW).cloned().collect();
        let (from_year, to_year) = span_of(e);
        crates.push(Crate {
            kind,
            key: e.key.clone(),
            name: e.name.clone(),
            count: e.album_ids.len(),
            artists: e.artists,
            from_year,
            to_year,
            albums: albums_by_ids(c, &ids)?,
        });
    }
    Ok(CrateList { kind, crates })
}

/// One crate's page. None when there's no such crate.
pub fn page(c: &Connection, ix: &Index, kind: CrateKind, key: &str) -> AppResult<Option<CratePage>> {
    let Some(e) = ix.find(kind, key) else { return Ok(None) };
    let mut albums = Vec::with_capacity(e.album_ids.len());
    let mut track_ids: Vec<String> = Vec::new();
    let mut songs = c.prepare_cached("SELECT id FROM tracks WHERE album_id = ?1 ORDER BY disc_number, track_number")?;
    for a in albums_by_ids(c, &e.album_ids)? {
        let on_map = super::on_map(c, &a.id)?;
        if track_ids.len() < PLAY_MAX {
            for id in songs.query_map([&a.id], |r| r.get::<_, String>(0))? {
                track_ids.push(id?);
            }
            track_ids.truncate(PLAY_MAX);
        }
        albums.push(CrateAlbum { album: a, on_map });
    }
    let years: Vec<i32> = if kind == CrateKind::Era {
        let mut ys: Vec<i32> = ix.of_kind(CrateKind::Year).filter(|y| super::decade(y.key.parse().ok()).map(|d| d.to_string()) == Some(e.key.clone())).filter_map(|y| y.key.parse().ok()).collect();
        ys.sort();
        ys
    } else {
        vec![]
    };
    let (from_year, to_year) = span_of(e);
    Ok(Some(CratePage { kind, key: e.key.clone(), name: e.name.clone(), artists: e.artists, from_year, to_year, albums, years, track_ids }))
}

/// The album's label, producer and studio crates as shelves of its other
/// albums. Crates with nothing else in them never show (a crate holds two
/// albums at least).
pub fn shelves(c: &Connection, ix: &Index, album_id: &str) -> AppResult<Vec<AlbumShelf>> {
    let artist: Option<String> = albums_by_ids(c, &[album_id.to_owned()])?.into_iter().next().map(|a| a.artist_id);
    let mut out = Vec::new();
    for e in ix.digging_from(album_id) {
        let others: Vec<String> = e.album_ids.iter().filter(|id| *id != album_id).cloned().collect();
        let mut albums = albums_by_ids(c, &others)?;
        // Other artists first: digging is for finding something new.
        albums.sort_by_key(|a| Some(&a.artist_id) == artist.as_ref());
        albums.truncate(SHELF_N);
        if albums.is_empty() {
            continue;
        }
        out.push(AlbumShelf { kind: e.kind, key: e.key.clone(), name: e.name.clone(), count: e.album_ids.len(), albums });
    }
    Ok(out)
}

/// Every crate of one kind (empty while Show crates is off).
#[tauri::command]
pub async fn crates(db: State<'_, DbHandle>, sleeve: State<'_, SleeveService>, kind: CrateKind) -> AppResult<CrateList> {
    if !crates_on(&db) {
        return Ok(CrateList { kind, crates: vec![] });
    }
    let ix = index(&db, &sleeve).await?;
    db.read_async(move |c| list(c, &ix, kind)).await
}

/// One crate with all its albums. Null when there's no such crate.
#[tauri::command]
pub async fn crate_page(db: State<'_, DbHandle>, sleeve: State<'_, SleeveService>, kind: CrateKind, key: String) -> AppResult<Option<CratePage>> {
    if !crates_on(&db) {
        return Ok(None);
    }
    let ix = index(&db, &sleeve).await?;
    db.read_async(move |c| page(c, &ix, kind, &key)).await
}

/// An album page's crate shelves (empty while either setting is off).
#[tauri::command]
pub async fn album_crates(db: State<'_, DbHandle>, sleeve: State<'_, SleeveService>, id: String) -> AppResult<Vec<AlbumShelf>> {
    let s = crate::settings::load(&db);
    if !s.show_crates || !s.crate_shelves {
        return Ok(vec![]);
    }
    let ix = index(&db, &sleeve).await?;
    db.read_async(move |c| shelves(c, &ix, &id)).await
}
