//! The downloads tables (schema v17): what the user turned on, the songs in
//! each, and one file row per song shared by every item that holds it.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Tries before a failed song waits for the user to retry.
pub const MAX_ATTEMPTS: u32 = 5;

/// What can be downloaded as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Album,
    Playlist,
    /// Liked Songs (its id is always empty).
    Liked,
    /// A mix; its songs come from the caller, not the library cache.
    Mix,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Playlist => "playlist",
            Self::Liked => "liked",
            Self::Mix => "mix",
        }
    }

    pub fn parse(s: &str) -> AppResult<Self> {
        Ok(match s {
            "album" => Self::Album,
            "playlist" => Self::Playlist,
            "liked" => Self::Liked,
            "mix" => Self::Mix,
            _ => return Err(AppError::Invalid(format!("can't download a {s}"))),
        })
    }
}

/// An item key: kind plus id ("" for Liked Songs).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemKey {
    pub kind: ItemKind,
    pub id: String,
}

impl ItemKey {
    pub fn new(kind: ItemKind, id: &str) -> Self {
        let id = if kind == ItemKind::Liked { String::new() } else { id.to_string() };
        Self { kind, id }
    }
}

/// Wait before trying a failed song again, by how many times it failed.
pub fn backoff_secs(attempts: u32) -> i64 {
    match attempts {
        0 | 1 => 30,
        2 => 120,
        3 => 600,
        _ => 3600,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemRow {
    pub key: ItemKey,
    pub title: String,
    pub added_at: i64,
}

pub fn items(c: &Connection) -> AppResult<Vec<ItemRow>> {
    let mut st = c.prepare_cached("SELECT kind, id, title, added_at FROM download_items ORDER BY added_at, rowid")?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (kind, id, title, added_at) = row?;
        if let Ok(kind) = ItemKind::parse(&kind) {
            out.push(ItemRow { key: ItemKey { kind, id }, title, added_at });
        }
    }
    Ok(out)
}

pub fn item_tracks(c: &Connection, key: &ItemKey) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached("SELECT track_id FROM download_item_tracks WHERE kind = ?1 AND id = ?2 ORDER BY position")?;
    let ids = st.query_map(params![key.kind.as_str(), key.id], |r| r.get(0))?;
    Ok(ids.collect::<Result<_, _>>()?)
}

pub fn has_item(c: &Connection, key: &ItemKey) -> AppResult<bool> {
    Ok(c.query_row(
        "SELECT 1 FROM download_items WHERE kind = ?1 AND id = ?2",
        params![key.kind.as_str(), key.id],
        |_| Ok(()),
    )
    .optional()?
    .is_some())
}

/// Turns an item on (or refreshes its title and songs). Call [`reconcile`] after.
pub fn set_item(c: &Connection, key: &ItemKey, title: &str, tracks: &[String], now: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO download_items (kind, id, title, added_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (kind, id) DO UPDATE SET title = CASE WHEN excluded.title = '' THEN title ELSE excluded.title END",
        params![key.kind.as_str(), key.id, title, now],
    )?;
    set_tracks(c, key, tracks)
}

/// Replaces an item's songs (duplicates keep their first place).
pub fn set_tracks(c: &Connection, key: &ItemKey, tracks: &[String]) -> AppResult<()> {
    c.execute(
        "DELETE FROM download_item_tracks WHERE kind = ?1 AND id = ?2",
        params![key.kind.as_str(), key.id],
    )?;
    let mut ins = c.prepare_cached(
        "INSERT OR IGNORE INTO download_item_tracks (kind, id, track_id, position) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (i, t) in tracks.iter().enumerate() {
        ins.execute(params![key.kind.as_str(), key.id, t, i as i64])?;
    }
    Ok(())
}

pub fn remove_item(c: &Connection, key: &ItemKey) -> AppResult<()> {
    c.execute(
        "DELETE FROM download_item_tracks WHERE kind = ?1 AND id = ?2",
        params![key.kind.as_str(), key.id],
    )?;
    c.execute("DELETE FROM download_items WHERE kind = ?1 AND id = ?2", params![key.kind.as_str(), key.id])?;
    Ok(())
}

/// Brings the file rows in line with the items: songs that are wanted and
/// have no row are queued (in the order their items were turned on), rows no
/// item needs any more are dropped, and finished files whose song was
/// replaced on the server are queued again. Returns the file names to delete.
pub fn reconcile(c: &Connection) -> AppResult<Vec<String>> {
    let mut seq: i64 = c.query_row("SELECT COALESCE(MAX(seq), 0) FROM download_files", [], |r| r.get(0))?;
    let wanted: Vec<String> = {
        let mut st = c.prepare_cached(
            "SELECT t.track_id FROM download_item_tracks t
             JOIN download_items i ON i.kind = t.kind AND i.id = t.id
             WHERE NOT EXISTS (SELECT 1 FROM download_files f WHERE f.track_id = t.track_id)
             ORDER BY i.added_at, i.rowid, t.position",
        )?;
        st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
    };
    let mut seen = HashSet::new();
    let mut ins = c.prepare_cached("INSERT OR IGNORE INTO download_files (track_id, seq, state) VALUES (?1, ?2, 'queued')")?;
    for t in wanted {
        if seen.insert(t.clone()) {
            seq += 1;
            ins.execute(params![t, seq])?;
        }
    }
    let gone: Vec<(String, Option<String>)> = {
        let mut st = c.prepare_cached(
            "SELECT track_id, file FROM download_files f
             WHERE NOT EXISTS (SELECT 1 FROM download_item_tracks t WHERE t.track_id = f.track_id)",
        )?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
    };
    let mut del = c.prepare_cached("DELETE FROM download_files WHERE track_id = ?1")?;
    let mut files = Vec::new();
    for (track, file) in gone {
        del.execute([&track])?;
        files.extend(file);
    }
    c.execute(
        "UPDATE download_files SET state = 'queued', attempts = 0, next_try_at = 0, error = NULL
         WHERE state = 'done' AND EXISTS (
             SELECT 1 FROM tracks t WHERE t.id = download_files.track_id
             AND t.part_key != '' AND t.part_key != download_files.part_key)",
        [],
    )?;
    Ok(files)
}

/// The next song to fetch: queued ones in order, then failed ones whose wait is over.
pub fn next_queued(c: &Connection, now: i64) -> AppResult<Option<String>> {
    Ok(c.query_row(
        "SELECT track_id FROM download_files
         WHERE state = 'queued' OR (state = 'failed' AND attempts < ?1 AND next_try_at <= ?2)
         ORDER BY state = 'failed', seq LIMIT 1",
        params![MAX_ATTEMPTS, now],
        |r| r.get(0),
    )
    .optional()?)
}

/// Seconds until a failed song can be tried again, if any is waiting.
pub fn next_retry_in(c: &Connection, now: i64) -> AppResult<Option<i64>> {
    let at: Option<i64> = c.query_row(
        "SELECT MIN(next_try_at) FROM download_files WHERE state = 'failed' AND attempts < ?1",
        params![MAX_ATTEMPTS],
        |r| r.get(0),
    )?;
    Ok(at.map(|t| (t - now).max(0)))
}

/// One file row.
#[derive(Debug, Clone, PartialEq)]
pub struct FileRow {
    pub track_id: String,
    pub state: String,
    pub part_key: String,
    pub quality: String,
    pub file: Option<String>,
    pub size: u64,
    pub error: Option<String>,
    pub attempts: u32,
}

pub fn file(c: &Connection, track_id: &str) -> AppResult<Option<FileRow>> {
    Ok(c.query_row(
        "SELECT track_id, state, part_key, quality, file, size, error, attempts FROM download_files WHERE track_id = ?1",
        [track_id],
        |r| {
            Ok(FileRow {
                track_id: r.get(0)?,
                state: r.get(1)?,
                part_key: r.get(2)?,
                quality: r.get(3)?,
                file: r.get(4)?,
                size: r.get::<_, i64>(5)?.max(0) as u64,
                error: r.get(6)?,
                attempts: r.get::<_, i64>(7)?.max(0) as u32,
            })
        },
    )
    .optional()?)
}

/// A song finished. Returns the old file's name when it had a different one.
pub fn mark_done(
    c: &Connection,
    track_id: &str,
    part_key: &str,
    quality: &str,
    file_name: &str,
    size: u64,
    now: i64,
) -> AppResult<Option<String>> {
    let old: Option<Option<String>> = c
        .query_row("SELECT file FROM download_files WHERE track_id = ?1", [track_id], |r| r.get(0))
        .optional()?;
    let n = c.execute(
        "UPDATE download_files SET state = 'done', part_key = ?2, quality = ?3, file = ?4, size = ?5,
         error = NULL, attempts = 0, next_try_at = 0, done_at = ?6 WHERE track_id = ?1",
        params![track_id, part_key, quality, file_name, size as i64, now],
    )?;
    if n == 0 {
        // Removed while it downloaded: the caller deletes the file.
        return Ok(Some(file_name.to_string()));
    }
    Ok(old.flatten().filter(|f| f != file_name))
}

pub fn mark_failed(c: &Connection, track_id: &str, error: &str, now: i64) -> AppResult<()> {
    let attempts: Option<i64> = c
        .query_row("SELECT attempts FROM download_files WHERE track_id = ?1", [track_id], |r| r.get(0))
        .optional()?;
    let attempts = attempts.unwrap_or(0) as u32 + 1;
    c.execute(
        "UPDATE download_files SET state = 'failed', error = ?2, attempts = ?3, next_try_at = ?4 WHERE track_id = ?1",
        params![track_id, error, attempts, now + backoff_secs(attempts)],
    )?;
    Ok(())
}

/// A finished file is gone from disk.
pub fn mark_missing(c: &Connection, track_id: &str) -> AppResult<bool> {
    Ok(c.execute(
        "UPDATE download_files SET state = 'missing', error = 'The file is missing from this computer.'
         WHERE track_id = ?1 AND state = 'done'",
        [track_id],
    )? > 0)
}

/// Failed and missing songs (of one item, or all) go back in the queue.
pub fn retry(c: &Connection, key: Option<&ItemKey>) -> AppResult<usize> {
    let sql = "UPDATE download_files SET state = 'queued', attempts = 0, next_try_at = 0, error = NULL
               WHERE state IN ('failed', 'missing')";
    Ok(match key {
        None => c.execute(sql, [])?,
        Some(k) => c.execute(
            &format!(
                "{sql} AND track_id IN (SELECT track_id FROM download_item_tracks WHERE kind = ?1 AND id = ?2)"
            ),
            params![k.kind.as_str(), k.id],
        )?,
    })
}

/// Every finished file: (track id, file name).
pub fn done_files(c: &Connection) -> AppResult<Vec<(String, String)>> {
    let mut st = c.prepare_cached("SELECT track_id, file FROM download_files WHERE state = 'done' AND file IS NOT NULL")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn used_bytes(c: &Connection) -> AppResult<u64> {
    let n: i64 = c.query_row("SELECT COALESCE(SUM(size), 0) FROM download_files WHERE state = 'done'", [], |r| r.get(0))?;
    Ok(n.max(0) as u64)
}

pub fn queued_count(c: &Connection, now: i64) -> AppResult<u32> {
    let n: i64 = c.query_row(
        "SELECT COUNT(*) FROM download_files
         WHERE state = 'queued' OR (state = 'failed' AND attempts < ?1 AND next_try_at <= ?2)",
        params![MAX_ATTEMPTS, now],
        |r| r.get(0),
    )?;
    Ok(n as u32)
}

/// Per item counts for the status.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemCounts {
    pub total: u32,
    pub done: u32,
    pub failed: u32,
    pub missing: u32,
    /// Bytes of finished files only this item holds (what removing it frees).
    pub own_bytes: u64,
    /// The first failure's reason.
    pub error: Option<String>,
}

pub fn item_counts(c: &Connection, key: &ItemKey) -> AppResult<ItemCounts> {
    let mut st = c.prepare_cached(
        "SELECT
            COUNT(*),
            COALESCE(SUM(f.state = 'done'), 0),
            COALESCE(SUM(f.state = 'failed' AND f.attempts >= ?3), 0),
            COALESCE(SUM(f.state = 'missing'), 0),
            COALESCE(SUM(CASE WHEN f.state = 'done' AND NOT EXISTS (
                SELECT 1 FROM download_item_tracks o
                WHERE o.track_id = t.track_id AND (o.kind != t.kind OR o.id != t.id)) THEN f.size ELSE 0 END), 0),
            (SELECT f2.error FROM download_item_tracks t2 JOIN download_files f2 ON f2.track_id = t2.track_id
             WHERE t2.kind = ?1 AND t2.id = ?2 AND f2.state IN ('failed', 'missing') AND f2.error IS NOT NULL
             ORDER BY t2.position LIMIT 1)
         FROM download_item_tracks t LEFT JOIN download_files f ON f.track_id = t.track_id
         WHERE t.kind = ?1 AND t.id = ?2",
    )?;
    Ok(st.query_row(params![key.kind.as_str(), key.id, MAX_ATTEMPTS], |r| {
        Ok(ItemCounts {
            total: r.get::<_, i64>(0)? as u32,
            done: r.get::<_, i64>(1)? as u32,
            failed: r.get::<_, i64>(2)? as u32,
            missing: r.get::<_, i64>(3)? as u32,
            own_bytes: r.get::<_, i64>(4)?.max(0) as u64,
            error: r.get(5)?,
        })
    })?)
}

/// Forgets every download. Returns the file names to delete.
pub fn remove_all(c: &Connection) -> AppResult<Vec<String>> {
    let files: Vec<String> = {
        let mut st = c.prepare_cached("SELECT file FROM download_files WHERE file IS NOT NULL")?;
        st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
    };
    c.execute_batch("DELETE FROM download_item_tracks; DELETE FROM download_items; DELETE FROM download_files;")?;
    Ok(files)
}

/// An album's, playlist's or Liked Songs' songs now, from the library cache.
/// None when the item isn't in the cache (or for a mix, whose songs come from
/// the caller): its stored list is kept.
pub fn tracks_of_library_item(c: &Connection, key: &ItemKey, liked_min: f64) -> AppResult<Option<Vec<String>>> {
    match key.kind {
        ItemKind::Album => {
            let exists = c
                .query_row("SELECT 1 FROM albums WHERE id = ?1", [&key.id], |_| Ok(()))
                .optional()?
                .is_some();
            if !exists {
                return Ok(None);
            }
            let mut st = c.prepare_cached(
                "SELECT id FROM tracks WHERE album_id = ?1 ORDER BY disc_number, track_number, title",
            )?;
            let ids = st.query_map([&key.id], |r| r.get(0))?;
            Ok(Some(ids.collect::<Result<_, _>>()?))
        }
        ItemKind::Playlist => {
            let id = crate::db::playlists::resolve_id(c, &key.id)?;
            let exists = c
                .query_row("SELECT 1 FROM playlists WHERE id = ?1", [&id], |_| Ok(()))
                .optional()?
                .is_some();
            if !exists {
                return Ok(None);
            }
            let mut st = c.prepare_cached("SELECT track_id FROM playlist_items WHERE playlist_id = ?1 ORDER BY position")?;
            let ids = st.query_map([&id], |r| r.get(0))?;
            Ok(Some(ids.collect::<Result<_, _>>()?))
        }
        ItemKind::Liked => Ok(Some(crate::db::likes::liked_ids(c, liked_min)?)),
        ItemKind::Mix => Ok(None),
    }
}
