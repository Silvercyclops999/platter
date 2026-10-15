//! What identifying by sound found, per song (`sound_ids`), keyed by track
//! with the file it was made from: a changed file has a new part key and is
//! done again. A fingerprint is kept until its lookup is done, so a lookup
//! that has to wait (no key, network down) never reads the file twice.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::Scope;
use super::acoustid::Hit;
use crate::error::AppResult;

/// A hit below this score isn't trusted (kept as no match).
pub const MIN_SCORE: f32 = 0.8;

/// Row states.
pub const OK: &str = "ok";
pub const NONE: &str = "none";
pub const FAILED: &str = "failed";
/// Fingerprinted, waiting for its lookup.
pub const PRINTED: &str = "printed";

/// A song to fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Todo {
    pub track_id: String,
    pub part_key: String,
    pub duration_ms: u64,
    pub bitrate: Option<u32>,
    pub codec: Option<String>,
    pub container: Option<String>,
}

/// The songs [`Scope`] covers: every song, or those whose file has no
/// recording id or ISRC (files whose tags aren't read yet count, after the
/// ones that are).
fn scope_filter(scope: Scope) -> &'static str {
    match scope {
        Scope::All => "1",
        Scope::Untagged => "(f.part_key IS NULL OR (f.recording_mbid IS NULL AND f.isrc IS NULL))",
    }
}

/// Up to `limit` songs to fingerprint, newest first: never done, or their
/// file changed since.
pub fn pending(c: &Connection, scope: Scope, limit: usize) -> AppResult<Vec<Todo>> {
    let sql = format!(
        "SELECT t.id, t.part_key, t.duration_ms, t.bitrate, t.codec, t.container
         FROM tracks t
         LEFT JOIN file_tags f ON f.part_key = t.part_key
         LEFT JOIN sound_ids s ON s.track_id = t.id
         WHERE t.part_key <> '' AND t.duration_ms >= ?1
           AND (s.track_id IS NULL OR s.part_key <> t.part_key)
           AND {}
         ORDER BY f.part_key IS NULL, t.pk DESC
         LIMIT ?2",
        scope_filter(scope)
    );
    let mut st = c.prepare_cached(&sql)?;
    let min_ms = super::print::MIN_SECONDS as i64 * 1000;
    let out = st
        .query_map(params![min_ms, limit as i64], |r| {
            Ok(Todo {
                track_id: r.get(0)?,
                part_key: r.get(1)?,
                duration_ms: r.get::<_, i64>(2)?.max(0) as u64,
                bitrate: r.get(3)?,
                codec: r.get(4)?,
                container: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Fingerprints waiting for a lookup: (track id, fingerprint, seconds).
pub fn printed(c: &Connection, limit: usize) -> AppResult<Vec<(String, String, u32)>> {
    let mut st = c.prepare_cached(
        "SELECT track_id, fingerprint, duration_s FROM sound_ids
         WHERE status = 'printed' AND fingerprint IS NOT NULL ORDER BY checked_at LIMIT ?1",
    )?;
    let out = st
        .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)?.max(0) as u32)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Keep a fingerprint until its lookup is done.
pub fn save_print(c: &Connection, t: &Todo, fingerprint: &str, duration_s: u32, now: i64) -> AppResult<()> {
    c.execute(
        "INSERT OR REPLACE INTO sound_ids (track_id, part_key, status, fingerprint, duration_s, checked_at)
         VALUES (?1, ?2, 'printed', ?3, ?4, ?5)",
        params![t.track_id, t.part_key, fingerprint, duration_s, now],
    )?;
    Ok(())
}

/// A file that couldn't be fingerprinted (not tried again until it changes).
pub fn save_failed(c: &Connection, t: &Todo, now: i64) -> AppResult<()> {
    c.execute(
        "INSERT OR REPLACE INTO sound_ids (track_id, part_key, status, checked_at) VALUES (?1, ?2, 'failed', ?3)",
        params![t.track_id, t.part_key, now],
    )?;
    Ok(())
}

/// What the lookup said. The best hit at or above [`MIN_SCORE`] is kept;
/// the fingerprint is dropped once it has an answer.
pub fn save_result(c: &Connection, track_id: &str, hits: &[Hit], now: i64) -> AppResult<bool> {
    let best = hits.iter().filter(|h| h.score >= MIN_SCORE).max_by(|a, b| a.score.total_cmp(&b.score));
    match best {
        Some(h) => c.execute(
            "UPDATE sound_ids SET status = 'ok', fingerprint = NULL, recording_mbid = ?2, title = ?3, artist = ?4,
                 album = ?5, score = ?6, checked_at = ?7 WHERE track_id = ?1",
            params![track_id, h.recording_mbid, h.title, h.artist, h.album, h.score as f64, now],
        )?,
        None => c.execute(
            "UPDATE sound_ids SET status = 'none', fingerprint = NULL, recording_mbid = NULL, title = NULL, artist = NULL,
                 album = NULL, score = ?2, checked_at = ?3 WHERE track_id = ?1",
            params![track_id, hits.first().map(|h| h.score as f64), now],
        )?,
    };
    Ok(best.is_some())
}

/// A lookup AcoustID couldn't use: no match, not asked again.
pub fn save_rejected(c: &Connection, track_id: &str, now: i64) -> AppResult<()> {
    save_result(c, track_id, &[], now).map(|_| ())
}

/// Drop rows for songs no longer in the library.
pub fn prune(c: &Connection) -> AppResult<usize> {
    Ok(c.execute("DELETE FROM sound_ids WHERE track_id NOT IN (SELECT id FROM tracks)", [])?)
}

/// Progress for Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// Songs the setting covers.
    pub total: u64,
    /// Of those, done (identified, no match, or couldn't be read).
    pub done: u64,
    /// Identified.
    pub identified: u64,
}

pub fn counts(c: &Connection, scope: Scope) -> AppResult<Counts> {
    let sql = format!(
        "SELECT count(*),
                COALESCE(sum(s.status IN ('ok', 'none', 'failed') AND s.part_key = t.part_key), 0),
                COALESCE(sum(s.status = 'ok' AND s.part_key = t.part_key), 0)
         FROM tracks t
         LEFT JOIN file_tags f ON f.part_key = t.part_key
         LEFT JOIN sound_ids s ON s.track_id = t.id
         WHERE t.part_key <> '' AND t.duration_ms >= ?1 AND {}",
        scope_filter(scope)
    );
    let min_ms = super::print::MIN_SECONDS as i64 * 1000;
    Ok(c.query_row(&sql, [min_ms], |r| {
        Ok(Counts { total: r.get::<_, i64>(0)? as u64, done: r.get::<_, i64>(1)? as u64, identified: r.get::<_, i64>(2)? as u64 })
    })?)
}

/// The recording a song sounds like (a hint; IPC `SoundsLike`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundsLike {
    pub track_id: String,
    pub recording_mbid: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// 0 to 100.
    pub score: u8,
    /// The song itself is unreleased: this is only a hint, never a match.
    pub unreleased: bool,
    /// A released copy of that recording in the library, to open.
    pub in_library: Option<LibraryCopy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryCopy {
    pub track_id: String,
    pub album_id: String,
}

/// What `track_id` sounds like, if it was identified.
pub fn sounds_like(c: &Connection, track_id: &str) -> AppResult<Option<SoundsLike>> {
    let row = c
        .query_row(
            "SELECT s.recording_mbid, s.title, s.artist, s.album, s.score,
                    COALESCE((SELECT max(unreleased) FROM song_index WHERE track_id = t.id), 0)
             FROM sound_ids s JOIN tracks t ON t.id = s.track_id AND t.part_key = s.part_key
             WHERE s.track_id = ?1 AND s.status = 'ok'",
            [track_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<f64>>(4)?,
                    r.get::<_, i64>(5)? != 0,
                ))
            },
        )
        .optional()?;
    let Some((mbid, title, artist, album, score, unreleased)) = row else { return Ok(None) };
    let in_library = c
        .query_row(
            "SELECT t.id, t.album_id FROM song_index s JOIN tracks t ON t.id = s.track_id
             WHERE (s.recording_mbid = ?1 OR s.sound_mbid = ?1) AND s.unreleased = 0 AND t.id <> ?2
             ORDER BY t.id LIMIT 1",
            params![mbid, track_id],
            |r| Ok(LibraryCopy { track_id: r.get(0)?, album_id: r.get(1)? }),
        )
        .optional()?;
    Ok(Some(SoundsLike {
        track_id: track_id.to_string(),
        recording_mbid: mbid,
        title,
        artist: artist.unwrap_or_default(),
        album,
        score: (score.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0).round() as u8,
        unreleased,
        in_library,
    }))
}

/// An identified song, with what its file says, for the library doctor.
#[derive(Debug, Clone, PartialEq)]
pub struct Identified {
    pub track_id: String,
    pub title: String,
    pub artist_id: String,
    /// Album artist.
    pub artist: String,
    pub track_artist: Option<String>,
    pub album_id: String,
    pub album: String,
    pub thumb: Option<String>,
    pub tag_recording_mbid: Option<String>,
    pub unreleased: bool,
    pub sounds: SoundsLike,
}

/// Every identified song whose file is unchanged since.
pub fn identified(c: &Connection) -> AppResult<Vec<Identified>> {
    let mut st = c.prepare_cached(
        "SELECT t.id, t.title, t.artist_id, t.artist_title, t.track_artist, t.album_id, t.album_title, al.thumb,
                f.recording_mbid, COALESCE((SELECT max(unreleased) FROM song_index WHERE track_id = t.id), 0),
                s.recording_mbid, s.title, s.artist, s.album, s.score
         FROM sound_ids s
         JOIN tracks t ON t.id = s.track_id AND t.part_key = s.part_key
         LEFT JOIN albums al ON al.id = t.album_id
         LEFT JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'
         WHERE s.status = 'ok' AND s.recording_mbid IS NOT NULL
         ORDER BY t.artist_title COLLATE NOCASE, t.album_title COLLATE NOCASE, t.id",
    )?;
    let rows = st.query_map([], |r| {
        let track_id: String = r.get(0)?;
        let unreleased = r.get::<_, i64>(9)? != 0;
        Ok(Identified {
            track_id: track_id.clone(),
            title: r.get(1)?,
            artist_id: r.get(2)?,
            artist: r.get(3)?,
            track_artist: r.get(4)?,
            album_id: r.get(5)?,
            album: r.get(6)?,
            thumb: r.get::<_, Option<String>>(7)?.filter(|t| !t.is_empty()),
            tag_recording_mbid: r.get::<_, Option<String>>(8)?.and_then(|m| crate::tags::clean_mbid(&m)),
            unreleased,
            sounds: SoundsLike {
                track_id,
                recording_mbid: r.get(10)?,
                title: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
                artist: r.get::<_, Option<String>>(12)?.unwrap_or_default(),
                album: r.get(13)?,
                score: (r.get::<_, Option<f64>>(14)?.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0).round() as u8,
                unreleased,
                in_library: None,
            },
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
