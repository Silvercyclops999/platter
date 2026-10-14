//! Reads behind every Library command in docs/IPC.md.
//!
//! Home definitions:
//! - **Recently played**: albums by the most recent play of any of their
//!   tracks, merging Plex history (`play_history`, all devices) with the local
//!   listen log (catches offline plays not yet reported). Falls back to the
//!   album's own `last_viewed_at`.
//! - **Recently added**: albums by `added_at`, newest first.
//! - **Jump back in**: albums you were into lately but haven't finished.
//!   Window: last 28 days. An album qualifies when it has ≥ 2 plays in the
//!   window and either fewer distinct tracks played than it has tracks (not
//!   finished) or it is among your most played. Ranked by plays in the window,
//!   then most recent play. Albums already in the first 6 of "Recently
//!   played" are left out so the two rows don't repeat each other.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{AppError, AppResult};
use crate::model::{Album, Artist, Playlist, ReleaseSection, Track};

use super::credits;
use super::release::{SECTION_ORDER, section_key};
use super::store::{
    ALBUM_COLS, ARTIST_COLS, PLAYLIST_COLS, TRACK_COLS, row_album, row_artist, row_playlist, row_track,
};
use super::types::*;

pub const HOME_ROW: usize = 12;
const JUMP_BACK_WINDOW_S: i64 = 28 * 24 * 3600;
const POPULAR_N: usize = 10;
const MORE_BY_N: usize = 12;
const MAX_PAGE: u32 = 500;

pub fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn albums_by_ids(c: &Connection, ids: &[String]) -> AppResult<Vec<Album>> {
    let mut st = c.prepare_cached(&format!("SELECT {ALBUM_COLS} FROM albums al WHERE al.id = ?1"))?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(a) = st.query_row([id], row_album).optional()? {
            out.push(a);
        }
    }
    Ok(out)
}

/// Album ids with their latest play time (seconds), newest first.
fn recent_album_plays(c: &Connection, limit: usize) -> AppResult<Vec<(String, i64)>> {
    let mut st = c.prepare_cached(
        "WITH plays AS (
             SELECT rating_key, viewed_at AS at FROM play_history
             UNION ALL
             SELECT rating_key, started_at / 1000 AS at FROM listen_log
         ), recent AS (
             SELECT rating_key, max(at) AS at FROM plays GROUP BY rating_key
             ORDER BY at DESC LIMIT 2000
         ), by_album AS (
             SELECT t.album_id AS album_id, max(r.at) AS at
             FROM recent r JOIN tracks t ON t.id = r.rating_key
             GROUP BY t.album_id
             UNION ALL
             SELECT id, last_viewed_at FROM albums WHERE last_viewed_at IS NOT NULL
         )
         SELECT album_id, max(at) AS at FROM by_album GROUP BY album_id ORDER BY at DESC LIMIT ?1",
    )?;
    let rows = st
        .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn home(c: &Connection, now: i64) -> AppResult<HomeData> {
    let recent_ids: Vec<String> = recent_album_plays(c, HOME_ROW)?.into_iter().map(|(id, _)| id).collect();
    let recently_played = albums_by_ids(c, &recent_ids)?;

    let recently_added = {
        let mut st = c.prepare_cached(&format!(
            "SELECT {ALBUM_COLS} FROM albums al ORDER BY al.added_at DESC, al.pk DESC LIMIT ?1"
        ))?;
        st.query_map([HOME_ROW as i64], row_album)?.collect::<Result<Vec<_>, _>>()?
    };

    let exclude: HashSet<&str> = recent_ids.iter().take(6).map(String::as_str).collect();
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
         ORDER BY n DESC, last DESC LIMIT 100",
    )?;
    let rows: Vec<(String, i64, i64, i64)> = st
        .query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(4)?)))?
        .collect::<Result<_, _>>()?;
    let mut ids = Vec::new();
    for (rank, (id, _n, distinct, track_count)) in rows.into_iter().enumerate() {
        if exclude.contains(id.as_str()) {
            continue;
        }
        let unfinished = distinct < track_count;
        if unfinished || rank < HOME_ROW {
            ids.push(id);
        }
        if ids.len() >= HOME_ROW {
            break;
        }
    }
    let jump_back_in = albums_by_ids(c, &ids)?;
    Ok(HomeData { recently_played, recently_added, jump_back_in })
}

fn clamp_page(offset: u32, limit: u32) -> (i64, i64) {
    (offset as i64, limit.clamp(1, MAX_PAGE) as i64)
}

pub fn artists(c: &Connection, sort: ArtistSort, offset: u32, limit: u32) -> AppResult<Page<Artist>> {
    let order = match sort {
        ArtistSort::Name => "ar.sort_title COLLATE NOCASE, ar.pk",
        ArtistSort::Recent => "ar.latest_added_at DESC, ar.pk",
        ArtistSort::Plays => "ar.view_count DESC, ar.sort_title COLLATE NOCASE",
    };
    let (off, lim) = clamp_page(offset, limit);
    let mut st = c.prepare_cached(&format!(
        "SELECT {ARTIST_COLS} FROM artists ar ORDER BY {order} LIMIT ?1 OFFSET ?2"
    ))?;
    let items = st.query_map([lim, off], row_artist)?.collect::<Result<Vec<_>, _>>()?;
    let total: i64 = c.query_row("SELECT count(*) FROM artists", [], |r| r.get(0))?;
    Ok(Page { items, total: total as u64 })
}

pub fn albums(
    c: &Connection,
    sort: AlbumSort,
    offset: u32,
    limit: u32,
    section: Option<ReleaseSection>,
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
    let filter = "(?3 IS NULL OR al.section = ?3)";
    let mut st = c.prepare_cached(&format!(
        "SELECT {ALBUM_COLS} FROM albums al WHERE {filter} ORDER BY {order} LIMIT ?1 OFFSET ?2"
    ))?;
    let items = st
        .query_map(params![lim, off, section], row_album)?
        .collect::<Result<Vec<_>, _>>()?;
    let total: i64 = c.query_row(
        "SELECT count(*) FROM albums al WHERE (?1 IS NULL OR al.section = ?1)",
        [section],
        |r| r.get(0),
    )?;
    Ok(Page { items, total: total as u64 })
}

/// All artist names and ids (small: two short strings per artist).
pub fn artist_names(c: &Connection) -> AppResult<Vec<ArtistName>> {
    let mut st = c.prepare_cached("SELECT id, title FROM artists ORDER BY pk")?;
    let rows = st.query_map([], |r| Ok(ArtistName { id: r.get(0)?, title: r.get(1)? }))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn playlists(c: &Connection) -> AppResult<Vec<Playlist>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {PLAYLIST_COLS} FROM playlists p ORDER BY p.title COLLATE NOCASE"
    ))?;
    Ok(st.query_map([], row_playlist)?.collect::<Result<Vec<_>, _>>()?)
}

pub fn artist_page(c: &Connection, id: &str) -> AppResult<ArtistPage> {
    let artist = c
        .query_row(&format!("SELECT {ARTIST_COLS} FROM artists ar WHERE ar.id = ?1"), [id], row_artist)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("artist {id}")))?;
    let popular = popular_tracks(c, id, &artist.title)?;
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
            (!albums.is_empty()).then_some(ArtistSection { section: *s, albums })
        })
        .collect();
    Ok(ArtistPage { artist, popular, sections })
}

/// Two copies of one song: same title (any case) and length within this.
const SAME_SONG_MS: u64 = 2_000;

/// An artist's popular songs: their own tracks plus tracks that credit them
/// in the track artist line. Songs with a global listener count come first,
/// most listened first; the rest follow by your own plays, then most recent
/// play. Repeat copies of a song (same title, about the same length) show
/// once, keeping the better ranked one.
fn popular_tracks(c: &Connection, id: &str, title: &str) -> AppResult<Vec<Track>> {
    let mut st = c.prepare_cached(&format!("SELECT {TRACK_COLS} FROM tracks t WHERE t.artist_id = ?1 ORDER BY t.pk"))?;
    let mut cands = st.query_map([id], row_track)?.collect::<Result<Vec<_>, _>>()?;

    // Other artists' tracks whose artist line mentions the name (the search
    // index finds them without a table scan), kept only when the line credits
    // the artist as a whole name.
    if title.chars().any(char::is_alphanumeric) {
        let phrase = format!("track_artist : \"{}\"", title.replace('"', "\"\""));
        let mut st = c.prepare_cached(&format!(
            "SELECT {TRACK_COLS} FROM tracks_fts JOIN tracks t ON t.pk = tracks_fts.rowid
             WHERE tracks_fts MATCH ?1 AND t.artist_id <> ?2 ORDER BY t.pk"
        ))?;
        let mentioned = st.query_map(params![phrase, id], row_track)?.collect::<Result<Vec<_>, _>>()?;
        let mut names_in = c.prepare_cached("SELECT title FROM artists WHERE instr(lower(?1), lower(title)) > 0")?;
        let me = title.to_lowercase();
        for t in mentioned {
            let line = t.track_artist.as_deref().unwrap_or_default();
            let known: Vec<String> = names_in
                .query_map([line], |r| r.get::<_, String>(0))?
                .map(|n| n.map(|n| n.to_lowercase()))
                .collect::<Result<_, _>>()?;
            let credited = credits::split_credits(line, |k| k == me || known.iter().any(|n| n == k))
                .iter()
                .any(|n| n.to_lowercase() == me);
            if credited {
                cands.push(t);
            }
        }
    }

    // Stable sort keeps library order for full ties.
    cands.sort_by(|a, b| {
        b.rating_count
            .is_some()
            .cmp(&a.rating_count.is_some())
            .then(b.rating_count.cmp(&a.rating_count))
            .then(b.view_count.cmp(&a.view_count))
            .then(b.last_viewed_at.cmp(&a.last_viewed_at))
    });

    let mut out: Vec<Track> = Vec::with_capacity(POPULAR_N);
    for t in cands {
        let dupe = out.iter().any(|o| {
            o.id == t.id
                || (o.title.to_lowercase() == t.title.to_lowercase()
                    && o.duration_ms.abs_diff(t.duration_ms) <= SAME_SONG_MS)
        });
        if !dupe {
            out.push(t);
            if out.len() == POPULAR_N {
                break;
            }
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
    Ok(AlbumPage { album, tracks, total_duration_ms, more_by_artist })
}

pub fn album_tracks(c: &Connection, album_id: &str) -> AppResult<Vec<Track>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {TRACK_COLS} FROM tracks t WHERE t.album_id = ?1
         ORDER BY t.disc_number, t.track_number, t.title"
    ))?;
    Ok(st.query_map([album_id], row_track)?.collect::<Result<Vec<_>, _>>()?)
}

pub fn playlist_page(c: &Connection, id: &str) -> AppResult<PlaylistPage> {
    let playlist = c
        .query_row(&format!("SELECT {PLAYLIST_COLS} FROM playlists p WHERE p.id = ?1"), [id], row_playlist)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("playlist {id}")))?;
    let mut st = c.prepare_cached(&format!(
        "SELECT {TRACK_COLS} FROM playlist_items pi JOIN tracks t ON t.id = pi.track_id
         WHERE pi.playlist_id = ?1 ORDER BY pi.position"
    ))?;
    let tracks = st.query_map([id], row_track)?.collect::<Result<Vec<_>, _>>()?;
    Ok(PlaylistPage { playlist, tracks })
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
