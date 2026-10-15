//! Row mapping and writes for the library cache.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::error::AppResult;
use crate::model::{Album, Artist, Playlist, Track};

use super::release::{section_from_key, section_key, section_rank};
use super::source::{HistoryEntry, ItemKind, PlaylistEntry};

// ---------------------------------------------------------------- columns

pub const ARTIST_COLS: &str = "ar.id, ar.title, ar.sort_title, ar.thumb, ar.art, ar.view_count";
pub const ALBUM_COLS: &str = "al.id, al.title, al.sort_title, al.artist_id, al.artist_title, al.year, \
     al.thumb, al.format, al.subformats, al.section, al.genres, al.track_count, al.added_at, \
     al.view_count, al.last_viewed_at, al.tint";
pub const TRACK_COLS: &str = "t.id, t.title, t.album_id, t.album_title, t.artist_id, t.artist_title, \
     t.track_artist, t.track_number, t.disc_number, t.duration_ms, t.thumb, t.part_key, t.codec, \
     t.container, t.bitrate, t.bit_depth, t.sample_rate, t.user_rating, t.view_count, t.last_viewed_at, t.rating_count, \
     t.rated_at, t.genres";
/// How many columns [`TRACK_COLS`] selects (extra columns after it start here).
pub const TRACK_NCOLS: usize = 23;
pub const PLAYLIST_COLS: &str = "p.id, p.title, p.smart, p.track_count, p.duration_ms, p.thumb, p.summary, p.updated_at";

pub fn row_artist(r: &Row<'_>) -> rusqlite::Result<Artist> {
    Ok(Artist {
        id: r.get(0)?,
        title: r.get(1)?,
        sort_title: r.get(2)?,
        thumb: r.get(3)?,
        art: r.get(4)?,
        view_count: r.get(5)?,
        summary: None,
    })
}

fn json_vec(s: String) -> Vec<String> {
    serde_json::from_str(&s).unwrap_or_default()
}

pub fn row_album(r: &Row<'_>) -> rusqlite::Result<Album> {
    let section: String = r.get(9)?;
    Ok(Album {
        id: r.get(0)?,
        title: r.get(1)?,
        sort_title: r.get(2)?,
        artist_id: r.get(3)?,
        artist_title: r.get(4)?,
        year: r.get(5)?,
        thumb: r.get(6)?,
        format: r.get(7)?,
        subformats: json_vec(r.get(8)?),
        section: section_from_key(&section),
        genres: json_vec(r.get(10)?),
        track_count: r.get(11)?,
        added_at: r.get(12)?,
        view_count: r.get(13)?,
        last_viewed_at: r.get(14)?,
        tint: r.get(15)?,
        released: None,
        studio: None,
        styles: Vec::new(),
        moods: Vec::new(),
    })
}

pub fn row_track(r: &Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        id: r.get(0)?,
        title: r.get(1)?,
        album_id: r.get(2)?,
        album_title: r.get(3)?,
        artist_id: r.get(4)?,
        artist_title: r.get(5)?,
        track_artist: r.get(6)?,
        track_number: r.get(7)?,
        disc_number: r.get(8)?,
        duration_ms: r.get::<_, i64>(9)?.max(0) as u64,
        thumb: r.get(10)?,
        part_key: r.get(11)?,
        codec: r.get(12)?,
        container: r.get(13)?,
        bitrate: r.get(14)?,
        bit_depth: r.get(15)?,
        sample_rate: r.get(16)?,
        user_rating: r.get::<_, Option<f64>>(17)?.map(|v| v as f32),
        view_count: r.get(18)?,
        last_viewed_at: r.get(19)?,
        rating_count: r.get(20)?,
        rated_at: r.get(21)?,
        genres: json_vec(r.get(22)?),
    })
}

pub fn row_playlist(r: &Row<'_>) -> rusqlite::Result<Playlist> {
    Ok(Playlist {
        id: r.get(0)?,
        title: r.get(1)?,
        smart: r.get(2)?,
        track_count: r.get(3)?,
        duration_ms: r.get::<_, i64>(4)?.max(0) as u64,
        thumb: r.get(5)?,
        summary: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

// ---------------------------------------------------------------- upserts

/// Upsert artists, stamping `seen_gen` (used to find deletions after a full
/// pass).
pub fn upsert_artists(tx: &Transaction<'_>, items: &[Artist], gen_: i64) -> AppResult<()> {
    let mut st = tx.prepare_cached(
        "INSERT INTO artists (id, title, sort_title, thumb, art, view_count, summary, seen_gen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (id) DO UPDATE SET title = excluded.title, sort_title = excluded.sort_title,
           thumb = excluded.thumb, art = excluded.art, view_count = excluded.view_count,
           summary = excluded.summary, seen_gen = excluded.seen_gen",
    )?;
    // `similar` isn't on list items; the artist page fills it (db::similar).
    for a in items {
        st.execute(params![a.id, a.title, a.sort_title, a.thumb, a.art, a.view_count, a.summary, gen_])?;
    }
    Ok(())
}

pub fn upsert_albums(tx: &Transaction<'_>, items: &[Album], gen_: i64) -> AppResult<()> {
    let mut st = tx.prepare_cached(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, year, thumb, format,
           subformats, section, section_rank, genres, track_count, added_at, view_count,
           last_viewed_at, tint, seen_gen, released, studio, styles, moods)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)
         ON CONFLICT (id) DO UPDATE SET title = excluded.title, sort_title = excluded.sort_title,
           artist_id = excluded.artist_id, artist_title = excluded.artist_title, year = excluded.year,
           thumb = excluded.thumb, format = excluded.format, subformats = excluded.subformats,
           section = excluded.section, section_rank = excluded.section_rank, genres = excluded.genres,
           track_count = excluded.track_count, added_at = excluded.added_at,
           view_count = excluded.view_count, last_viewed_at = excluded.last_viewed_at,
           tint = excluded.tint, seen_gen = excluded.seen_gen,
           released = excluded.released, studio = excluded.studio,
           styles = excluded.styles, moods = excluded.moods",
    )?;
    for a in items {
        st.execute(params![
            a.id,
            a.title,
            a.sort_title,
            a.artist_id,
            a.artist_title,
            a.year,
            a.thumb,
            a.format,
            serde_json::to_string(&a.subformats).unwrap_or_else(|_| "[]".into()),
            section_key(a.section),
            section_rank(a.section),
            serde_json::to_string(&a.genres).unwrap_or_else(|_| "[]".into()),
            a.track_count,
            a.added_at,
            a.view_count,
            a.last_viewed_at,
            a.tint,
            gen_,
            a.released,
            a.studio,
            serde_json::to_string(&a.styles).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&a.moods).unwrap_or_else(|_| "[]".into())
        ])?;
    }
    Ok(())
}

pub fn upsert_tracks(tx: &Transaction<'_>, items: &[Track], gen_: i64) -> AppResult<()> {
    let mut st = tx.prepare_cached(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_artist,
           track_number, disc_number, duration_ms, thumb, part_key, codec, container, bitrate,
           bit_depth, sample_rate, user_rating, view_count, last_viewed_at, seen_gen, rating_count,
           rated_at, genres)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
           ?19, ?20, ?21, ?22, ?23, ?24)
         ON CONFLICT (id) DO UPDATE SET title = excluded.title, album_id = excluded.album_id,
           album_title = excluded.album_title, artist_id = excluded.artist_id,
           artist_title = excluded.artist_title, track_artist = excluded.track_artist,
           track_number = excluded.track_number, disc_number = excluded.disc_number,
           duration_ms = excluded.duration_ms, thumb = excluded.thumb, part_key = excluded.part_key,
           codec = excluded.codec, container = excluded.container, bitrate = excluded.bitrate,
           bit_depth = COALESCE(excluded.bit_depth, tracks.bit_depth),
           sample_rate = COALESCE(excluded.sample_rate, tracks.sample_rate),
           view_count = excluded.view_count,
           last_viewed_at = excluded.last_viewed_at, seen_gen = excluded.seen_gen,
           rating_count = COALESCE(excluded.rating_count, tracks.rating_count),
           genres = excluded.genres,
           -- A like made here that hasn't reached Plex yet wins over Plex.
           user_rating = CASE WHEN EXISTS (SELECT 1 FROM rating_ops o WHERE o.track_id = excluded.id)
                         THEN tracks.user_rating ELSE excluded.user_rating END,
           rated_at = CASE WHEN EXISTS (SELECT 1 FROM rating_ops o WHERE o.track_id = excluded.id)
                      THEN tracks.rated_at ELSE excluded.rated_at END",
    )?;
    for t in items {
        st.execute(params![
            t.id,
            t.title,
            t.album_id,
            t.album_title,
            t.artist_id,
            t.artist_title,
            t.track_artist,
            t.track_number,
            t.disc_number,
            t.duration_ms as i64,
            t.thumb,
            t.part_key,
            t.codec,
            t.container,
            t.bitrate,
            t.bit_depth,
            t.sample_rate,
            t.user_rating.map(f64::from),
            t.view_count,
            t.last_viewed_at,
            gen_,
            t.rating_count,
            t.rated_at,
            serde_json::to_string(&t.genres).unwrap_or_else(|_| "[]".into())
        ])?;
    }
    Ok(())
}

/// Recompute each artist's newest album `added_at` (artists sort "recent").
pub fn refresh_artist_recency(tx: &Transaction<'_>) -> AppResult<()> {
    tx.execute_batch(
        "UPDATE artists SET latest_added_at = COALESCE(
            (SELECT max(added_at) FROM albums WHERE albums.artist_id = artists.id), 0)",
    )?;
    Ok(())
}

/// Delete items of `kind` whose ids are not in `keep`. Returns rows deleted.
pub fn delete_missing(tx: &Transaction<'_>, kind: ItemKind, keep: &HashSet<String>) -> AppResult<u64> {
    let table = table_for(kind);
    let existing: Vec<String> = {
        let mut st = tx.prepare(&format!("SELECT id FROM {table}"))?;
        st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
    };
    let mut del = tx.prepare_cached(&format!("DELETE FROM {table} WHERE id = ?1"))?;
    let mut n = 0;
    for id in existing.iter().filter(|id| !keep.contains(*id)) {
        n += del.execute([id])? as u64;
    }
    Ok(n)
}

/// Delete items of `kind` not stamped with `gen_` (not seen in a full pass).
pub fn delete_unseen(tx: &Transaction<'_>, kind: ItemKind, gen_: i64) -> AppResult<u64> {
    let table = table_for(kind);
    Ok(tx.execute(&format!("DELETE FROM {table} WHERE seen_gen <> ?1"), [gen_])? as u64)
}

fn table_for(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Artist => "artists",
        ItemKind::Album => "albums",
        ItemKind::Track => "tracks",
    }
}

/// Wipe the library cache (library replaced). Keeps the listen log,
/// recent searches and settings.
pub fn wipe_library(tx: &Transaction<'_>) -> AppResult<()> {
    tx.execute_batch(
        "DELETE FROM playlist_items; DELETE FROM playlists; DELETE FROM playlist_ops;
         DELETE FROM playlist_aliases; DELETE FROM tracks;
         DELETE FROM albums; DELETE FROM artists; DELETE FROM play_history;
         DELETE FROM sync_state;",
    )?;
    Ok(())
}

pub fn insert_history(tx: &Transaction<'_>, items: &[HistoryEntry]) -> AppResult<()> {
    let mut st = tx.prepare_cached(
        "INSERT OR IGNORE INTO play_history (rating_key, viewed_at) VALUES (?1, ?2)",
    )?;
    for h in items {
        st.execute(params![h.rating_key, h.viewed_at])?;
    }
    Ok(())
}

/// Playlist ids a sync must leave alone: ones with edits still waiting to
/// reach Plex, and ones made here that Plex doesn't have yet.
pub fn playlists_with_local_changes(c: &Connection) -> AppResult<HashSet<String>> {
    let mut st = c.prepare_cached(
        "SELECT playlist_id FROM playlist_ops UNION SELECT id FROM playlists WHERE local_only = 1",
    )?;
    let ids = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?;
    Ok(ids)
}

/// Replace the playlist list. Returns ids whose items need (re)fetching:
/// new playlists and ones whose count, duration or updatedAt changed (all
/// of them with `force_all`). Playlists with local changes that haven't
/// reached Plex yet are left as they are.
pub fn replace_playlists(tx: &Transaction<'_>, items: &[Playlist], force_all: bool) -> AppResult<Vec<String>> {
    let local = playlists_with_local_changes(tx)?;
    let items: Vec<&Playlist> = items.iter().filter(|p| !local.contains(&p.id)).collect();
    let mut stale = Vec::new();
    {
        let mut prev =
            tx.prepare_cached("SELECT track_count, duration_ms, updated_at FROM playlists WHERE id = ?1")?;
        for p in &items {
            let old: Option<(u32, i64, Option<i64>)> =
                prev.query_row([&p.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
            if force_all || old != Some((p.track_count, p.duration_ms as i64, p.updated_at)) {
                stale.push(p.id.clone());
            }
        }
    }
    let keep: HashSet<&str> = items.iter().map(|p| p.id.as_str()).collect();
    let existing: Vec<String> = {
        let mut st = tx.prepare("SELECT id FROM playlists")?;
        st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
    };
    for id in existing.iter().filter(|id| !keep.contains(id.as_str()) && !local.contains(*id)) {
        tx.execute("DELETE FROM playlists WHERE id = ?1", [id])?;
    }
    for p in items {
        upsert_playlist(tx, p)?;
    }
    Ok(stale)
}

/// Insert or update one playlist row as Plex has it.
pub fn upsert_playlist(c: &Connection, p: &Playlist) -> AppResult<()> {
    let mut st = c.prepare_cached(
        "INSERT INTO playlists (id, title, smart, track_count, duration_ms, thumb, summary, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (id) DO UPDATE SET title = excluded.title, smart = excluded.smart,
           track_count = excluded.track_count, duration_ms = excluded.duration_ms,
           thumb = excluded.thumb, summary = excluded.summary, updated_at = excluded.updated_at,
           local_only = 0",
    )?;
    st.execute(params![
        p.id,
        p.title,
        p.smart,
        p.track_count,
        p.duration_ms as i64,
        p.thumb,
        p.summary,
        p.updated_at
    ])?;
    Ok(())
}

/// Replace a playlist's items. Items without a Plex playlistItemID get a
/// local one so every row can still be named.
pub fn set_playlist_items(c: &Connection, playlist_id: &str, items: &[PlaylistEntry]) -> AppResult<()> {
    c.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", [playlist_id])?;
    let mut st = c.prepare_cached(
        "INSERT INTO playlist_items (playlist_id, position, track_id, item_id) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (i, e) in items.iter().enumerate() {
        let item_id = e.item_id.clone().unwrap_or_else(super::playlists::local_item_id);
        st.execute(params![playlist_id, i as i64, e.track_id, item_id])?;
    }
    Ok(())
}

// ---------------------------------------------------------------- sync state

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncState {
    pub section_id: String,
    pub section_uuid: String,
    pub section_title: Option<String>,
    pub last_sync_at: Option<i64>,
    pub last_full_sync_at: Option<i64>,
    pub generation: i64,
}

pub fn sync_state(c: &Connection) -> AppResult<Option<SyncState>> {
    Ok(c.query_row(
        "SELECT section_id, section_uuid, section_title, last_sync_at, last_full_sync_at, generation
         FROM sync_state WHERE id = 1",
        [],
        |r| {
            Ok(SyncState {
                section_id: r.get(0)?,
                section_uuid: r.get(1)?,
                section_title: r.get(2)?,
                last_sync_at: r.get(3)?,
                last_full_sync_at: r.get(4)?,
                generation: r.get(5)?,
            })
        },
    )
    .optional()?)
}

pub fn put_sync_state(c: &Connection, s: &SyncState) -> AppResult<()> {
    c.execute(
        "INSERT INTO sync_state (id, section_id, section_uuid, section_title, last_sync_at,
           last_full_sync_at, generation)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (id) DO UPDATE SET section_id = excluded.section_id,
           section_uuid = excluded.section_uuid, section_title = excluded.section_title,
           last_sync_at = excluded.last_sync_at, last_full_sync_at = excluded.last_full_sync_at,
           generation = excluded.generation",
        params![
            s.section_id,
            s.section_uuid,
            s.section_title,
            s.last_sync_at,
            s.last_full_sync_at,
            s.generation
        ],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- kv

pub fn kv_get(c: &Connection, key: &str) -> AppResult<Option<String>> {
    Ok(c.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()?)
}

pub fn kv_set(c: &Connection, key: &str, value: &str) -> AppResult<()> {
    c.execute(
        "INSERT INTO kv (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}
