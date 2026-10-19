//! Reads behind every Library command in docs/IPC.md.
//!
//! Home definitions:
//! - **Recently played**: albums, artists and playlists by their most recent
//!   play, merging Plex history (`play_history`, all devices) with the local
//!   listen log (catches offline plays not yet reported, and knows which
//!   artist or playlist a play started from). See [`recently_played`].
//! - **Shortcuts**: the grid of 8 at the top (the UI puts Liked Songs
//!   before them): what you're into right now first, then long-time
//!   favourites, then other recent items, de-duplicated. "Right now" ranks
//!   albums, artists and playlists by their plays in the last week, each
//!   play worth half as much per day of age, so a few plays today beat one,
//!   and the order changes as you listen. See [`hot_keys`].
//! - **Your playlists**: most recently played here first, then by title.
//! - **Your top artists**: artists by play count.
//! - **Recently added**: albums by `added_at`, newest first.
//! - **Jump back in**: albums you were into lately but haven't finished.
//!   Window: last 28 days. An album qualifies when it has ≥ 2 plays in the
//!   window and either fewer distinct tracks played than it has tracks (not
//!   finished) or it is among your most played. Ranked by plays in the window,
//!   then most recent play. Albums already in the first 6 of "Recently
//!   played" are left out so the two rows don't repeat each other.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{AppError, AppResult};
use crate::hidden::HiddenSet;
use crate::model::{Album, Artist, Playlist, ReleaseSection, Track};

use super::popular::{self, POPULAR_N};
use super::release::{SECTION_ORDER, section_key};
use super::store::{
    ALBUM_COLS, ARTIST_COLS, PLAYLIST_COLS, TRACK_COLS, row_album, row_artist, row_playlist, row_track,
};
use super::types::*;

pub const HOME_ROW: usize = 12;
/// Tiles in Home's shortcut grid.
pub const HOME_SHORTCUTS: usize = 8;
/// Items you're into right now that lead the shortcut grid before long-time
/// favourites join (with Liked Songs first, about half the grid).
const SHORTCUT_HOT: usize = 4;
/// Plays older than this don't count towards "right now".
const HOT_WINDOW_S: i64 = 7 * 24 * 3600;
/// A play's weight halves every this many seconds.
const HOT_HALF_LIFE_S: f64 = 24.0 * 3600.0;
/// Most items a "Show all" page lists.
pub const SHELF_ALL_MAX: usize = 200;
const JUMP_BACK_WINDOW_S: i64 = 28 * 24 * 3600;
const MORE_BY_N: usize = 12;
const MAX_PAGE: u32 = 500;

pub fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn albums_by_ids(c: &Connection, ids: &[String]) -> AppResult<Vec<Album>> {
    let mut st = c.prepare_cached(&format!("SELECT {ALBUM_COLS} FROM albums al WHERE al.id = ?1"))?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(a) = st.query_row([id], row_album).optional()? {
            out.push(a);
        }
    }
    Ok(out)
}

/// Recently played albums, artists and playlists, newest first.
///
/// A play started from an artist or a playlist counts for that artist or
/// playlist only, not for each track's album, so playing a playlist doesn't
/// fill the row with its albums. Plex history (all devices) has no context,
/// so its plays count for the album, except those matching a local artist or
/// playlist play of the same track (reported to Plex from here). Albums' own
/// `last_viewed_at` stands in until Plex history has synced.
fn recent_keys(c: &Connection, limit: usize) -> AppResult<Vec<(String, String)>> {
    let mut st = c.prepare_cached(
        "WITH contexts AS (
             SELECT context_kind AS kind, context_id AS id, max(started_at) / 1000 AS at
             FROM listen_log
             WHERE context_kind IN ('album', 'artist', 'playlist') AND context_id IS NOT NULL
             GROUP BY context_kind, context_id
         ), track_plays AS (
             SELECT h.rating_key, h.viewed_at AS at FROM play_history h
             WHERE NOT EXISTS (
                 SELECT 1 FROM listen_log l
                 WHERE l.rating_key = h.rating_key
                   AND l.context_kind IN ('artist', 'playlist') AND l.context_id IS NOT NULL
                   AND h.viewed_at BETWEEN l.started_at / 1000 - 60
                       AND coalesce(l.ended_at, l.started_at + l.duration_ms) / 1000 + 600
             )
             UNION ALL
             SELECT rating_key, started_at / 1000 FROM listen_log
             WHERE context_id IS NULL OR context_kind IS NULL OR context_kind NOT IN ('artist', 'playlist')
         ), recent AS (
             SELECT rating_key, max(at) AS at FROM track_plays GROUP BY rating_key
             ORDER BY at DESC LIMIT 2000
         ), items AS (
             SELECT kind, id, at FROM contexts
             UNION ALL
             SELECT 'album', t.album_id, max(r.at) FROM recent r JOIN tracks t ON t.id = r.rating_key
             GROUP BY t.album_id
             UNION ALL
             SELECT 'album', id, last_viewed_at FROM albums
             WHERE last_viewed_at IS NOT NULL AND NOT EXISTS (SELECT 1 FROM play_history)
         )
         SELECT kind, id, max(at) AS at FROM items GROUP BY kind, id ORDER BY at DESC, kind, id LIMIT ?1",
    )?;
    let rows = st
        .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Look up `(kind, id)` keys; ones no longer in the library are skipped.
fn items_by_keys(c: &Connection, keys: &[(String, String)]) -> AppResult<Vec<HomeItem>> {
    let mut al = c.prepare_cached(&format!("SELECT {ALBUM_COLS} FROM albums al WHERE al.id = ?1"))?;
    let mut ar = c.prepare_cached(&format!("SELECT {ARTIST_COLS} FROM artists ar WHERE ar.id = ?1"))?;
    let mut pl = c.prepare_cached(&format!("SELECT {PLAYLIST_COLS} FROM playlists p WHERE p.id = ?1"))?;
    let mut out = Vec::with_capacity(keys.len());
    for (kind, id) in keys {
        let item = match kind.as_str() {
            "album" => al.query_row([id], row_album).optional()?.map(|album| HomeItem::Album { album }),
            "artist" => ar.query_row([id], row_artist).optional()?.map(|artist| HomeItem::Artist { artist }),
            "playlist" => pl.query_row([id], row_playlist).optional()?.map(|playlist| HomeItem::Playlist { playlist }),
            _ => None,
        };
        out.extend(item);
    }
    Ok(out)
}

/// Recently played albums, artists and playlists.
pub fn recently_played(c: &Connection, limit: usize) -> AppResult<Vec<HomeItem>> {
    items_by_keys(c, &recent_keys(c, limit)?)
}

/// Artists by play count (played ones only). Artists matched to one
/// MusicBrainz id (one artist under two names) are one row: their plays
/// added up, under the one played most.
pub fn top_artists(c: &Connection, limit: usize) -> AppResult<Vec<Artist>> {
    let mut st = c.prepare_cached(
        "SELECT ar.id, ar.title, ar.sort_title, ar.thumb, ar.art,
                CASE WHEN ar.mbid IS NULL THEN ar.view_count
                     ELSE (SELECT sum(o.view_count) FROM artists o WHERE o.mbid = ar.mbid) END AS plays
         FROM artists ar WHERE ar.view_count > 0
           AND NOT EXISTS (SELECT 1 FROM artists o WHERE ar.mbid IS NOT NULL AND o.mbid = ar.mbid
                           AND (o.view_count > ar.view_count OR (o.view_count = ar.view_count AND o.id < ar.id)))
         ORDER BY plays DESC, ar.sort_title COLLATE NOCASE LIMIT ?1",
    )?;
    Ok(st.query_map([limit as i64], row_artist)?.collect::<Result<Vec<_>, _>>()?)
}

/// Playlists, most recently played here first, then by title.
pub fn home_playlists(c: &Connection, limit: usize) -> AppResult<Vec<Playlist>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {PLAYLIST_COLS} FROM playlists p
         LEFT JOIN (SELECT context_id, max(started_at) AS at FROM listen_log
                    WHERE context_kind = 'playlist' GROUP BY context_id) l ON l.context_id = p.id
         ORDER BY l.at IS NULL, l.at DESC, p.title COLLATE NOCASE LIMIT ?1"
    ))?;
    Ok(st.query_map([limit as i64], row_playlist)?.collect::<Result<Vec<_>, _>>()?)
}

pub fn recently_added(c: &Connection, limit: usize) -> AppResult<Vec<Album>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {ALBUM_COLS} FROM albums al ORDER BY al.added_at DESC, al.pk DESC LIMIT ?1"
    ))?;
    Ok(st.query_map([limit as i64], row_album)?.collect::<Result<Vec<_>, _>>()?)
}

/// Most played albums, artists and playlists, taking turns (album, artist,
/// playlist) since their counts aren't on the same scale.
fn most_played(c: &Connection, per_kind: usize) -> AppResult<Vec<HomeItem>> {
    let albums: Vec<Album> = {
        let mut st = c.prepare_cached(&format!(
            "SELECT {ALBUM_COLS} FROM albums al WHERE al.view_count > 0
             ORDER BY al.view_count DESC, al.pk LIMIT ?1"
        ))?;
        st.query_map([per_kind as i64], row_album)?.collect::<Result<_, _>>()?
    };
    let artists = top_artists(c, per_kind)?;
    let playlists: Vec<Playlist> = {
        let mut st = c.prepare_cached(&format!(
            "SELECT {PLAYLIST_COLS} FROM playlists p
             JOIN (SELECT context_id, count(*) AS n FROM listen_log
                   WHERE context_kind = 'playlist' AND skipped = 0 GROUP BY context_id) l
               ON l.context_id = p.id
             ORDER BY l.n DESC, p.title COLLATE NOCASE LIMIT ?1"
        ))?;
        st.query_map([per_kind as i64], row_playlist)?.collect::<Result<_, _>>()?
    };
    let mut albums = albums.into_iter().map(|album| HomeItem::Album { album });
    let mut artists = artists.into_iter().map(|artist| HomeItem::Artist { artist });
    let mut playlists = playlists.into_iter().map(|playlist| HomeItem::Playlist { playlist });
    let mut out = Vec::new();
    loop {
        let before = out.len();
        out.extend(albums.next());
        out.extend(artists.next());
        out.extend(playlists.next());
        if out.len() == before {
            return Ok(out);
        }
    }
}

/// Albums, artists and playlists by how much they were played lately:
/// each play in the last [`HOT_WINDOW_S`] adds a weight that halves every
/// [`HOT_HALF_LIFE_S`]. Plays started from an artist or playlist count for
/// it, other plays for the song's album; plays from Liked Songs don't count
/// (Liked Songs has its own tile). Plex history from other devices counts
/// for the album unless it is a play from here already in the listen log.
fn hot_keys(c: &Connection, now: i64, limit: usize) -> AppResult<Vec<(String, String)>> {
    let since = now - HOT_WINDOW_S;
    let mut st = c.prepare_cached(
        "SELECT CASE WHEN l.context_kind IN ('artist', 'playlist') AND l.context_id IS NOT NULL
                     THEN l.context_kind ELSE 'album' END,
                CASE WHEN l.context_kind IN ('artist', 'playlist') AND l.context_id IS NOT NULL
                     THEN l.context_id ELSE t.album_id END,
                l.started_at / 1000
         FROM listen_log l JOIN tracks t ON t.id = l.rating_key
         WHERE l.started_at >= ?1 * 1000 AND l.skipped = 0 AND coalesce(l.context_kind, '') != 'liked'
         UNION ALL
         SELECT 'album', t.album_id, h.viewed_at FROM play_history h JOIN tracks t ON t.id = h.rating_key
         WHERE h.viewed_at >= ?1 AND NOT EXISTS (
             SELECT 1 FROM listen_log l
             WHERE l.rating_key = h.rating_key
               AND h.viewed_at BETWEEN l.started_at / 1000 - 60
                   AND coalesce(l.ended_at, l.started_at + l.duration_ms) / 1000 + 600
         )",
    )?;
    let rows = st.query_map([since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
    let mut score: HashMap<(String, String), (f64, i64)> = HashMap::new();
    for row in rows {
        let (kind, id, at) = row?;
        let age = (now - at).max(0) as f64;
        let e = score.entry((kind, id)).or_insert((0.0, i64::MIN));
        e.0 += 0.5f64.powf(age / HOT_HALF_LIFE_S);
        e.1 = e.1.max(at);
    }
    let mut ranked: Vec<_> = score.into_iter().collect();
    ranked.sort_by(|(ka, (sa, la)), (kb, (sb, lb))| sb.total_cmp(sa).then(lb.cmp(la)).then(ka.cmp(kb)));
    Ok(ranked.into_iter().take(limit).map(|(k, _)| k).collect())
}

/// The shortcut grid: up to [`SHORTCUT_HOT`] items you're into right now
/// lead, long-time favourites follow, then the rest of the hot items and
/// other recent ones fill any gap. No repeats, mixed types.
pub fn shortcuts(hot: &[HomeItem], most: &[HomeItem], recent: &[HomeItem]) -> Vec<HomeItem> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(HOME_SHORTCUTS);
    let lead = hot.iter().take(SHORTCUT_HOT);
    let rest = hot.iter().skip(SHORTCUT_HOT);
    for item in lead.chain(most.iter()).chain(rest).chain(recent.iter()) {
        if out.len() >= HOME_SHORTCUTS {
            break;
        }
        let (kind, id) = item.key();
        if seen.insert((kind, id.to_string())) {
            out.push(item.clone());
        }
    }
    out
}

/// Jump back in (see the module docs). Albums in `recent_albums` (the start
/// of Recently played) are left out so the two rows differ.
fn jump_back_in(c: &Connection, now: i64, recent_albums: &HashSet<String>, limit: usize) -> AppResult<Vec<Album>> {
    let since = now - JUMP_BACK_WINDOW_S;
    let mut st = c.prepare_cached(
        "WITH plays AS (
             SELECT rating_key, viewed_at AS at FROM play_history WHERE viewed_at >= ?1
             UNION ALL
             SELECT rating_key, started_at / 1000 FROM listen_log
             WHERE started_at >= ?1 * 1000 AND skipped = 0
         )
         SELECT t.album_id, count(*) AS n, count(DISTINCT t.id) AS distinct_tracks,
                max(p.at) AS last, al.track_count
         FROM plays p JOIN tracks t ON t.id = p.rating_key JOIN albums al ON al.id = t.album_id
         GROUP BY t.album_id HAVING n >= 2
         ORDER BY n DESC, last DESC LIMIT 500",
    )?;
    let rows: Vec<(String, i64, i64, i64)> = st
        .query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(4)?)))?
        .collect::<Result<_, _>>()?;
    let mut ids = Vec::new();
    for (rank, (id, _n, distinct, track_count)) in rows.into_iter().enumerate() {
        if recent_albums.contains(&id) {
            continue;
        }
        let unfinished = distinct < track_count;
        if unfinished || rank < HOME_ROW {
            ids.push(id);
        }
        if ids.len() >= limit {
            break;
        }
    }
    albums_by_ids(c, &ids)
}

/// Album ids among the first six recently played items.
fn first_recent_albums(recent: &[HomeItem]) -> HashSet<String> {
    recent
        .iter()
        .take(6)
        .filter_map(|i| match i {
            HomeItem::Album { album } => Some(album.id.clone()),
            _ => None,
        })
        .collect()
}

pub fn home(c: &Connection, now: i64) -> AppResult<HomeData> {
    home_sized(c, now, HOME_ROW)
}

/// Home with each shelf read up to `row` long (longer than it shows, when
/// filters are applied after).
pub fn home_sized(c: &Connection, now: i64, row: usize) -> AppResult<HomeData> {
    let mut recently_played = recently_played(c, row.max(HOME_SHORTCUTS))?;
    let most = most_played(c, HOME_SHORTCUTS)?;
    let hot = items_by_keys(c, &hot_keys(c, now, HOME_SHORTCUTS)?)?;
    let shortcuts = shortcuts(&hot, &most, &recently_played);
    let jump_back_in = jump_back_in(c, now, &first_recent_albums(&recently_played), row)?;
    recently_played.truncate(row);
    Ok(HomeData {
        shortcuts,
        recently_played,
        jump_back_in,
        playlists: home_playlists(c, row)?,
        recently_added: recently_added(c, row)?,
        top_artists: top_artists(c, row)?,
    })
}

/// The length of each shelf, in [`HomeData`] order (shortcuts left out).
pub fn home_lens(d: &HomeData) -> [usize; 5] {
    [d.recently_played.len(), d.jump_back_in.len(), d.playlists.len(), d.recently_added.len(), d.top_artists.len()]
}

/// Cuts every shelf to `n`.
pub fn trim_home(d: &mut HomeData, n: usize) {
    d.recently_played.truncate(n);
    d.jump_back_in.truncate(n);
    d.playlists.truncate(n);
    d.recently_added.truncate(n);
    d.top_artists.truncate(n);
}

/// Everything in one Home shelf, for its "Show all" page.
pub fn home_shelf(c: &Connection, now: i64, shelf: HomeShelf, limit: u32) -> AppResult<Vec<HomeItem>> {
    let n = (limit as usize).clamp(1, SHELF_ALL_MAX);
    Ok(match shelf {
        HomeShelf::RecentlyPlayed => recently_played(c, n)?,
        HomeShelf::JumpBackIn => {
            let recent = recently_played(c, 6)?;
            let albums = jump_back_in(c, now, &first_recent_albums(&recent), n)?;
            albums.into_iter().map(|album| HomeItem::Album { album }).collect()
        }
        HomeShelf::Playlists => {
            home_playlists(c, n)?.into_iter().map(|playlist| HomeItem::Playlist { playlist }).collect()
        }
        HomeShelf::RecentlyAdded => recently_added(c, n)?.into_iter().map(|album| HomeItem::Album { album }).collect(),
        HomeShelf::TopArtists => top_artists(c, n)?.into_iter().map(|artist| HomeItem::Artist { artist }).collect(),
    })
}

fn clamp_page(offset: u32, limit: u32) -> (i64, i64) {
    (offset as i64, limit.clamp(1, MAX_PAGE) as i64)
}

pub fn artists(c: &Connection, sort: ArtistSort, offset: u32, limit: u32) -> AppResult<Page<Artist>> {
    artists_on(c, sort, offset, limit, &super::merge::ServerView::All)
}

/// [`artists`] as one server, or all servers merged, show them.
pub fn artists_on(
    c: &Connection,
    sort: ArtistSort,
    offset: u32,
    limit: u32,
    server: &super::merge::ServerView,
) -> AppResult<Page<Artist>> {
    let shown = server.clause("ar", "artist");
    let order = match sort {
        ArtistSort::Name => "ar.sort_title COLLATE NOCASE, ar.pk",
        ArtistSort::Recent => "ar.latest_added_at DESC, ar.pk",
        ArtistSort::Plays => "ar.view_count DESC, ar.sort_title COLLATE NOCASE",
    };
    let (off, lim) = clamp_page(offset, limit);
    let mut st = c.prepare_cached(&format!(
        "SELECT {ARTIST_COLS} FROM artists ar WHERE {shown} ORDER BY {order} LIMIT ?1 OFFSET ?2"
    ))?;
    let items = st.query_map([lim, off], row_artist)?.collect::<Result<Vec<_>, _>>()?;
    let total: i64 = c.query_row(&format!("SELECT count(*) FROM artists ar WHERE {shown}"), [], |r| r.get(0))?;
    Ok(Page { items, total: total as u64 })
}

pub fn albums(
    c: &Connection,
    sort: AlbumSort,
    offset: u32,
    limit: u32,
    section: Option<ReleaseSection>,
) -> AppResult<Page<Album>> {
    albums_view(c, sort, offset, limit, section, crate::tags::TagView::default())
}

/// [`albums`] as the file tags shape it: editions behind their group's face
/// and albums of only fan or AI edits left out (per `view`).
pub fn albums_view(
    c: &Connection,
    sort: AlbumSort,
    offset: u32,
    limit: u32,
    section: Option<ReleaseSection>,
    view: crate::tags::TagView,
) -> AppResult<Page<Album>> {
    albums_view_on(c, sort, offset, limit, section, view, &super::merge::ServerView::All)
}

/// [`albums_view`] as one server, or all servers merged, show them.
pub fn albums_view_on(
    c: &Connection,
    sort: AlbumSort,
    offset: u32,
    limit: u32,
    section: Option<ReleaseSection>,
    view: crate::tags::TagView,
    server: &super::merge::ServerView,
) -> AppResult<Page<Album>> {
    let order = match sort {
        AlbumSort::Name => "al.sort_title COLLATE NOCASE, al.pk",
        AlbumSort::Artist => "al.artist_title COLLATE NOCASE, al.year, al.sort_title COLLATE NOCASE",
        AlbumSort::Year => "al.year DESC, al.sort_title COLLATE NOCASE",
        AlbumSort::Recent => "al.added_at DESC, al.pk DESC",
        AlbumSort::Plays => "al.view_count DESC, al.sort_title COLLATE NOCASE",
    };
    let (off, lim) = clamp_page(offset, limit);
    let section = section.map(section_key);
    let visible = format!("{} AND {}", crate::tags::store::visible_clause(view), server.clause("al", "album"));
    let filter = format!("(?3 IS NULL OR al.section = ?3) AND {visible}");
    let mut st = c.prepare_cached(&format!(
        "SELECT {ALBUM_COLS} FROM albums al WHERE {filter} ORDER BY {order} LIMIT ?1 OFFSET ?2"
    ))?;
    let items = st
        .query_map(params![lim, off, section], row_album)?
        .collect::<Result<Vec<_>, _>>()?;
    let total: i64 = c.query_row(
        &format!("SELECT count(*) FROM albums al WHERE (?1 IS NULL OR al.section = ?1) AND {visible}"),
        [section],
        |r| r.get(0),
    )?;
    Ok(Page { items, total: total as u64 })
}

/// All artist names and ids (small: two short strings per artist).
/// Learned names that contain a separator follow with `credit:` ids, so
/// track lines keep them whole too.
pub fn artist_names(c: &Connection) -> AppResult<Vec<ArtistName>> {
    let mut st = c.prepare_cached("SELECT id, title FROM artists ORDER BY pk")?;
    let rows = st.query_map([], |r| Ok(ArtistName { id: r.get(0)?, title: r.get(1)? }))?;
    let mut out = rows.collect::<Result<Vec<_>, _>>()?;
    out.extend(popular::learned_names(c)?.into_iter().map(|n| ArtistName { id: popular::credit_id(&n), title: n }));
    Ok(out)
}

pub fn playlists(c: &Connection) -> AppResult<Vec<Playlist>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {PLAYLIST_COLS} FROM playlists p ORDER BY p.title COLLATE NOCASE"
    ))?;
    Ok(st.query_map([], row_playlist)?.collect::<Result<Vec<_>, _>>()?)
}

pub fn artist_page(c: &Connection, id: &str) -> AppResult<ArtistPage> {
    artist_page_by(c, id, PopularSource::Global)
}

/// [`artist_page`] with Popular ranked by `source`. A `credit:` id gives a
/// credited artist's page (see [`credited_page`]).
pub fn artist_page_by(c: &Connection, id: &str, source: PopularSource) -> AppResult<ArtistPage> {
    artist_page_with(c, id, source, None)
}

/// Popular candidates without hidden songs, or songs by hidden artists
/// other than the page's own (a hidden artist's page still shows its songs:
/// you opened it yourself).
fn unhidden(tracks: Vec<Track>, page_id: &str, hidden: Option<&HiddenSet>) -> Vec<Track> {
    match hidden {
        Some(h) if !h.is_empty() => tracks
            .into_iter()
            .filter(|t| !h.tracks.contains(&t.id) && (t.artist_id == page_id || !h.artists.contains(&t.artist_id)))
            .collect(),
        _ => tracks,
    }
}

/// An artist's own albums in the page's order (albums, then EPs and the
/// other sections; newest first in each), at most `limit`.
pub fn artist_albums(c: &Connection, id: &str, limit: usize) -> AppResult<Vec<Album>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {ALBUM_COLS} FROM albums al WHERE al.artist_id = ?1
         ORDER BY al.section_rank, al.year IS NULL, al.year DESC, al.added_at DESC LIMIT ?2"
    ))?;
    Ok(st.query_map(params![id, limit as i64], row_album)?.collect::<Result<Vec<_>, _>>()?)
}

/// [`artist_page_by`] leaving hidden songs and artists out of Popular.
pub fn artist_page_with(c: &Connection, id: &str, source: PopularSource, hidden: Option<&HiddenSet>) -> AppResult<ArtistPage> {
    if let Some(key) = popular::credit_key(id) {
        return credited_page(c, id, key, source, hidden);
    }
    let artist = c
        .query_row(&format!("SELECT {ARTIST_COLS} FROM artists ar WHERE ar.id = ?1"), [id], row_artist)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("artist {id}")))?;
    let mut st = c.prepare_cached(&format!("SELECT {TRACK_COLS} FROM tracks t WHERE t.artist_id = ?1 ORDER BY t.pk"))?;
    let own = st.query_map([id], row_track)?.collect::<Result<Vec<_>, _>>()?;
    let credited = popular::credited_tracks(c, &artist.title, Some(id))?;
    let appears_on = appears_on(c, &credited)?;
    let global = global_for(c, id, source)?;
    let cands = unhidden(own.into_iter().chain(credited).collect(), id, hidden);
    let popular = popular::rank_with(cands, global.as_ref(), POPULAR_N, |t| popular::copy_info(c, t))?;
    let all = {
        let mut st = c.prepare_cached(&format!(
            "SELECT {ALBUM_COLS} FROM albums al WHERE al.artist_id = ?1
             ORDER BY al.section_rank, al.year IS NULL, al.year DESC, al.added_at DESC"
        ))?;
        st.query_map([id], row_album)?.collect::<Result<Vec<_>, _>>()?
    };
    let sections = SECTION_ORDER
        .iter()
        .filter_map(|s| {
            let albums: Vec<Album> = all.iter().filter(|a| a.section == *s).cloned().collect();
            (!albums.is_empty()).then_some(ArtistSection { section: *s, albums, custom: None })
        })
        .collect();
    let plays: i64 =
        c.query_row("SELECT COALESCE(sum(view_count), 0) FROM tracks WHERE artist_id = ?1", [id], |r| r.get(0))?;
    let (about, similar_names): (Option<String>, String) =
        c.query_row("SELECT summary, similar FROM artists WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let about = about.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let similar = similar_artists(c, id, &similar_names)?;
    let popular_pending = popular_pending(c, id, source)?;
    Ok(ArtistPage { artist, popular, sections, plays: plays.max(0) as u64, about, similar, appears_on, popular_pending, eras: Vec::new() })
}

/// Popular ranks by the global list and none was ever stored for `id` (a
/// "none found" counts as stored). The command also checks a source for the
/// list is on.
fn popular_pending(c: &Connection, id: &str, source: PopularSource) -> AppResult<bool> {
    if source != PopularSource::Global {
        return Ok(false);
    }
    let stored: bool = c.query_row(
        "SELECT EXISTS (SELECT 1 FROM artist_info WHERE artist_id = ?1 AND source = ?2)",
        params![id, popular::GLOBAL_ROW],
        |r| r.get(0),
    )?;
    Ok(!stored)
}

/// The global top songs when Popular ranks by them.
fn global_for(c: &Connection, id: &str, source: PopularSource) -> AppResult<Option<popular::GlobalTop>> {
    match source {
        PopularSource::Global => popular::global_top(c, id),
        PopularSource::MyPlays => Ok(None),
    }
}

/// Distinct albums of `tracks`, newest first: an artist's "Appears on".
fn appears_on(c: &Connection, tracks: &[Track]) -> AppResult<Vec<Album>> {
    let mut seen = HashSet::new();
    let ids: Vec<String> = tracks.iter().filter(|t| seen.insert(t.album_id.clone())).map(|t| t.album_id.clone()).collect();
    let mut albums = albums_by_ids(c, &ids)?;
    albums.sort_by(|a, b| b.year.cmp(&a.year).then(b.added_at.cmp(&a.added_at)));
    Ok(albums)
}

/// A page for a name that tracks credit but that has no library artist
/// (a featured guest). Built from the tracks that credit the name: Popular,
/// "Appears on" and the plays line. NotFound when nothing credits it.
pub fn credited_page(
    c: &Connection,
    id: &str,
    key: &str,
    source: PopularSource,
    hidden: Option<&HiddenSet>,
) -> AppResult<ArtistPage> {
    let tracks = popular::credited_tracks(c, key, None)?;
    if tracks.is_empty() {
        return Err(AppError::NotFound(format!("artist {id}")));
    }
    let title = popular::credited_display_name(c, key)?.unwrap_or_else(|| key.to_string());
    let plays: u64 = tracks.iter().map(|t| t.view_count as u64).sum();
    let appears_on = appears_on(c, &tracks)?;
    let global = global_for(c, id, source)?;
    let popular = popular::rank_with(unhidden(tracks, id, hidden), global.as_ref(), POPULAR_N, |t| popular::copy_info(c, t))?;
    let artist = Artist {
        id: id.to_string(),
        sort_title: title.clone(),
        title,
        thumb: None,
        art: None,
        view_count: plays.min(u32::MAX as u64) as u32,
        summary: None,
        mbid: None,
    };
    let popular_pending = popular_pending(c, id, source)?;
    Ok(ArtistPage { artist, popular, sections: Vec::new(), plays, about: None, similar: Vec::new(), appears_on, popular_pending, eras: Vec::new() })
}

/// The tracks a credited artist's play context adds after its Popular list.
/// Empty for library artists.
pub fn credited_context_tracks(c: &Connection, id: &str) -> AppResult<Vec<Track>> {
    match popular::credit_key(id) {
        Some(key) => popular::credited_tracks(c, key, None),
        None => Ok(Vec::new()),
    }
}

/// Most similar artists shown on an artist page.
const SIMILAR_N: usize = 12;

/// Library artists matching the stored similar artists by name (any case),
/// in the stored order, without the artist itself or repeats.
fn similar_artists(c: &Connection, id: &str, names_json: &str) -> AppResult<Vec<Artist>> {
    let stored: Vec<super::source::SimilarArtist> = serde_json::from_str(names_json).unwrap_or_default();
    let names = stored.into_iter().map(|s| s.name);
    let mut st = c.prepare_cached(&format!(
        "SELECT {ARTIST_COLS} FROM artists ar WHERE ar.title = ?1 COLLATE NOCASE AND ar.id <> ?2
         ORDER BY ar.view_count DESC, ar.pk LIMIT 1"
    ))?;
    let mut out: Vec<Artist> = Vec::new();
    for name in names {
        if out.len() == SIMILAR_N {
            break;
        }
        if let Some(a) = st.query_row(params![name.trim(), id], row_artist).optional()?
            && !out.iter().any(|o| o.id == a.id)
        {
            out.push(a);
        }
    }
    Ok(out)
}

pub fn album_page(c: &Connection, id: &str) -> AppResult<AlbumPage> {
    let album = c
        .query_row(&format!("SELECT {ALBUM_COLS} FROM albums al WHERE al.id = ?1"), [id], row_album)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("album {id}")))?;
    let tracks = album_tracks(c, id)?;
    let total_duration_ms = tracks.iter().map(|t| t.duration_ms).sum();
    let more_by_artist = {
        let mut st = c.prepare_cached(&format!(
            "SELECT {ALBUM_COLS} FROM albums al WHERE al.artist_id = ?1 AND al.id <> ?2
             ORDER BY al.year IS NULL, al.year DESC, al.added_at DESC LIMIT ?3"
        ))?;
        st.query_map(params![album.artist_id, id, MORE_BY_N as i64], row_album)?
            .collect::<Result<Vec<_>, _>>()?
    };
    let (released, studio) = c.query_row("SELECT released, studio FROM albums WHERE id = ?1", [id], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    let artist_thumb = c
        .query_row("SELECT thumb FROM artists WHERE id = ?1", [&album.artist_id], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten();
    Ok(AlbumPage { album, tracks, total_duration_ms, more_by_artist, artist_thumb, released, studio, archive: None })
}

pub fn album_tracks(c: &Connection, album_id: &str) -> AppResult<Vec<Track>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {TRACK_COLS} FROM tracks t WHERE t.album_id = ?1
         ORDER BY t.disc_number, t.track_number, t.title"
    ))?;
    Ok(st.query_map([album_id], row_track)?.collect::<Result<Vec<_>, _>>()?)
}

/// A playlist and its songs. An id made here that has since been created in
/// Plex still finds it (see db::playlists).
pub fn playlist_page(c: &Connection, id: &str) -> AppResult<PlaylistPage> {
    let id = super::playlists::resolve_id(c, id)?;
    let playlist = c
        .query_row(&format!("SELECT {PLAYLIST_COLS} FROM playlists p WHERE p.id = ?1"), [&id], row_playlist)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("playlist {id}")))?;
    let mut st = c.prepare_cached(&format!(
        "SELECT {TRACK_COLS}, pi.item_id FROM playlist_items pi JOIN tracks t ON t.id = pi.track_id
         WHERE pi.playlist_id = ?1 ORDER BY pi.position"
    ))?;
    let rows = st
        .query_map([&id], |r| Ok((row_track(r)?, r.get::<_, Option<String>>(super::store::TRACK_NCOLS)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let (tracks, item_ids) = rows.into_iter().map(|(t, item)| (t, item.unwrap_or_default())).unzip();
    Ok(PlaylistPage { playlist, tracks, item_ids })
}

/// Songs played on this device (the listen log), newest first. A song
/// played twice shows twice. Songs no longer in the library are skipped.
/// For the queue panel's Recently played tab.
pub fn recent_tracks(c: &Connection, limit: usize) -> AppResult<Vec<Track>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {TRACK_COLS} FROM listen_log l JOIN tracks t ON t.id = l.rating_key
         ORDER BY l.started_at DESC, l.id DESC LIMIT ?1"
    ))?;
    Ok(st.query_map([limit as i64], row_track)?.collect::<Result<Vec<_>, _>>()?)
}

/// Tracks by id, in the given order (missing ids skipped). For the player.
pub fn tracks_by_ids(c: &Connection, ids: &[String]) -> AppResult<Vec<Track>> {
    let mut st = c.prepare_cached(&format!("SELECT {TRACK_COLS} FROM tracks t WHERE t.id = ?1"))?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(t) = st.query_row([id], row_track).optional()? {
            out.push(t);
        }
    }
    Ok(out)
}

/// Every playlist, album and artist with what the library sidebar sorts and
/// shows by. Unordered; the UI sorts.
pub fn library_items(c: &Connection) -> AppResult<Vec<LibraryItem>> {
    // Latest play per album: Plex history (all devices) and the local listen log.
    let mut album_played: HashMap<String, i64> = HashMap::new();
    {
        let mut st = c.prepare_cached(
            "WITH plays AS (
                 SELECT rating_key, max(viewed_at) AS at FROM play_history GROUP BY rating_key
                 UNION ALL
                 SELECT rating_key, max(started_at) / 1000 AS at FROM listen_log GROUP BY rating_key
             )
             SELECT t.album_id, max(p.at) FROM plays p JOIN tracks t ON t.id = p.rating_key GROUP BY t.album_id",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (id, at) = row?;
            album_played.insert(id, at);
        }
    }

    // Per album: songs that aren't lossless, the most bits and highest rate.
    let mut album_quality: HashMap<String, AudioQuality> = HashMap::new();
    {
        let codecs = LOSSLESS_CODECS.iter().map(|c| format!("'{c}'")).collect::<Vec<_>>().join(",");
        let mut st = c.prepare_cached(&format!(
            "SELECT album_id,
                    sum(CASE WHEN lower(coalesce(codec, '')) IN ({codecs}) THEN 0 ELSE 1 END),
                    max(coalesce(bit_depth, 0)), max(coalesce(sample_rate, 0))
             FROM tracks GROUP BY album_id"
        ))?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)))?;
        for row in rows {
            let (id, lossy, bits, rate) = row?;
            album_quality.insert(id, AudioQuality::from_tracks(lossy, bits, rate));
        }
    }

    let mut out = Vec::new();
    // Per artist: earliest album added, latest album played.
    let mut artist_dates: HashMap<String, (Option<i64>, Option<i64>)> = HashMap::new();
    {
        let mut st = c.prepare_cached(
            "SELECT id, title, sort_title, artist_id, artist_title, thumb, format, added_at, last_viewed_at, tint, track_count, year, section FROM albums",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, i64>(10)?,
                r.get::<_, Option<i32>>(11)?,
                r.get::<_, Option<String>>(12)?,
            ))
        })?;
        for row in rows {
            let (id, title, sort_title, artist_id, artist_title, thumb, format, added, viewed, tint, tracks, year, section) = row?;
            let quality = album_quality.get(&id).copied();
            let played = match (viewed, album_played.get(&id).copied()) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            let added = (added > 0).then_some(added);
            let e = artist_dates.entry(artist_id.clone()).or_insert((None, None));
            e.0 = match (e.0, added) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            e.1 = e.1.max(played);
            out.push(LibraryItem {
                kind: LibraryKind::Album,
                id,
                title,
                sort_title,
                creator: Some(artist_title),
                creator_id: Some(artist_id),
                thumb,
                smart: false,
                format,
                added_at: added,
                played_at: played,
                tint,
                track_count: u32::try_from(tracks).ok().filter(|n| *n > 0),
                year,
                section,
                quality,
            });
        }
    }
    {
        let mut st = c.prepare_cached("SELECT id, title, sort_title, thumb FROM artists")?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?))
        })?;
        for row in rows {
            let (id, title, sort_title, thumb) = row?;
            let (added_at, played_at) = artist_dates.get(&id).copied().unwrap_or((None, None));
            out.push(LibraryItem {
                kind: LibraryKind::Artist,
                id,
                title,
                sort_title,
                creator: None,
                creator_id: None,
                thumb,
                smart: false,
                format: None,
                added_at,
                played_at,
                tint: None,
                track_count: None,
                year: None,
                section: None,
                quality: None,
            });
        }
    }
    {
        let mut st = c.prepare_cached(
            "SELECT p.id, p.title, p.thumb, p.smart,
                    (SELECT max(l.started_at) / 1000 FROM listen_log l
                      WHERE l.context_kind = 'playlist' AND l.context_id = p.id)
             FROM playlists p",
        )?;
        let rows = st.query_map([], |r| {
            Ok(LibraryItem {
                kind: LibraryKind::Playlist,
                id: r.get(0)?,
                sort_title: r.get(1)?,
                title: r.get(1)?,
                creator: None,
                creator_id: None,
                thumb: r.get(2)?,
                smart: r.get(3)?,
                format: None,
                added_at: None,
                played_at: r.get(4)?,
                tint: None,
                track_count: None,
                year: None,
                section: None,
                quality: None,
            })
        })?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}
