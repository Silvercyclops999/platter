//! Songs that left the server, so the library can say plainly what went and
//! when instead of songs quietly vanishing.
//!
//! A sync that removes songs keeps a copy of each one's name here first.
//! After the sync, songs that came back (an upgraded copy, a re-tag, the
//! same song on another album) are dropped again, so only music that is
//! really gone stays listed. Rows are kept 30 days. Playlists the server
//! lost are kept by `db::playlists::keep_deleted` and counted alongside.

use rusqlite::{Connection, Transaction, params};
use serde::Serialize;
use tauri::State;

use super::DbHandle;
use crate::error::AppResult;

/// How long a gone song stays listed.
pub const KEEP_S: i64 = 30 * 86_400;
/// The library line only counts what went this recently.
pub const FRESH_S: i64 = 7 * 86_400;
/// Most rows a list returns.
pub const LIST_MAX: u32 = 2000;

fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

const COPY: &str = "INSERT OR REPLACE INTO tracks_gone (track_id, title, album_id, album_title, artist_title, thumb, gone_at, seen)
     SELECT id, title, album_id, album_title, COALESCE(NULLIF(track_artist, ''), artist_title), thumb, ?1, 0 FROM tracks";

/// Keep the names of these songs before the sync deletes them.
pub fn record<'a>(tx: &Transaction<'_>, ids: impl Iterator<Item = &'a str>) -> AppResult<()> {
    let now = now_s();
    let mut st = tx.prepare_cached(&format!("{COPY} WHERE id = ?2"))?;
    for id in ids {
        st.execute(params![now, id])?;
    }
    Ok(())
}

/// Keep the names of the songs a full pass didn't see, before they go.
pub fn record_unseen(tx: &Transaction<'_>, gen_: i64, src: &str) -> AppResult<()> {
    tx.execute(&format!("{COPY} WHERE src = ?2 AND seen_gen <> ?3"), params![now_s(), src, gen_])?;
    Ok(())
}

/// A copy by the same artist on another album counts as the song coming
/// back only when that album arrived this recently (an upgrade or a re-tag
/// lands as a new album). Otherwise losing one "Intro" would hide behind
/// another album's "Intro".
pub const BACK_WITHIN_S: i64 = 7 * 86_400;

/// After a sync: drop songs that are back (same title on the same album, or
/// by the same artist on an album added lately) and ones older than 30
/// days. Returns rows dropped.
pub fn reconcile(c: &Connection, now: i64) -> AppResult<usize> {
    Ok(c.execute(
        "DELETE FROM tracks_gone WHERE gone_at < ?1
           OR (lower(title), lower(album_title)) IN (SELECT lower(title), lower(album_title) FROM tracks)
           OR (lower(title), lower(artist_title)) IN
              (SELECT lower(t.title), lower(COALESCE(NULLIF(t.track_artist, ''), t.artist_title))
                 FROM tracks t JOIN albums a ON a.id = t.album_id WHERE a.added_at >= ?2)",
        params![now - KEEP_S, now - BACK_WITHIN_S],
    )?)
}

/// What the library line says: songs and playlists that left the server
/// this week and haven't been looked at yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Safety {
    pub songs: u32,
    pub playlists: u32,
}

pub fn safety(c: &Connection, now: i64) -> AppResult<Safety> {
    let since = now - FRESH_S;
    let songs: i64 = c.query_row("SELECT count(*) FROM tracks_gone WHERE seen = 0 AND gone_at >= ?1", [since], |r| r.get(0))?;
    let playlists: i64 = c.query_row(
        "SELECT count(DISTINCT playlist_id) FROM playlist_trash WHERE gone = 1 AND seen = 0 AND deleted_at >= ?1",
        [since],
        |r| r.get(0),
    )?;
    Ok(Safety { songs: songs as u32, playlists: playlists as u32 })
}

/// A song that left the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoneSong {
    pub id: String,
    pub title: String,
    pub album_id: String,
    pub album_title: String,
    pub artist_title: String,
    pub thumb: Option<String>,
    /// Unix seconds.
    pub gone_at: i64,
    pub seen: bool,
}

/// Songs that left the server in the last 30 days, newest first, then by
/// album and title.
pub fn list(c: &Connection, now: i64) -> AppResult<Vec<GoneSong>> {
    let mut st = c.prepare_cached(
        "SELECT track_id, title, album_id, album_title, artist_title, thumb, gone_at, seen FROM tracks_gone
         WHERE gone_at >= ?1 ORDER BY gone_at DESC, lower(artist_title), lower(album_title), lower(title) LIMIT ?2",
    )?;
    let rows = st
        .query_map(params![now - KEEP_S, LIST_MAX], |r| {
            Ok(GoneSong {
                id: r.get(0)?,
                title: r.get(1)?,
                album_id: r.get(2)?,
                album_title: r.get(3)?,
                artist_title: r.get(4)?,
                thumb: r.get(5)?,
                gone_at: r.get(6)?,
                seen: r.get::<_, i64>(7)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The listener looked: the line goes back to saying all is fine.
pub fn mark_seen(c: &Connection) -> AppResult<()> {
    c.execute_batch("UPDATE tracks_gone SET seen = 1 WHERE seen = 0; UPDATE playlist_trash SET seen = 1 WHERE gone = 1 AND seen = 0;")?;
    Ok(())
}

#[tauri::command]
pub async fn library_safety(db: State<'_, DbHandle>) -> AppResult<Safety> {
    db.read_async(|c| safety(c, now_s())).await
}

#[tauri::command]
pub async fn library_gone(db: State<'_, DbHandle>) -> AppResult<Vec<GoneSong>> {
    db.read_async(|c| list(c, now_s())).await
}

#[tauri::command]
pub async fn library_gone_seen(db: State<'_, DbHandle>) -> AppResult<()> {
    db.write_async(|c| mark_seen(c)).await
}

#[cfg(test)]
#[path = "gone_tests.rs"]
mod tests;
