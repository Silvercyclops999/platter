//! The `artist_info` table: one row per artist and source with the source's
//! JSON (NULL when it had nothing), when it was fetched and when it goes
//! stale. A stale row is still shown while a refresh runs.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::AppResult;

/// How long a found entry is reused.
pub const TTL_S: i64 = 30 * 24 * 3600;
/// How long "nothing found" is remembered before asking again.
pub const MISS_TTL_S: i64 = 7 * 24 * 3600;
/// Shortest reuse for a reply whose cache headers ask for less.
pub const MIN_TTL_S: i64 = 24 * 3600;

/// Row name for the MusicBrainz id read from Plex.
pub const MBID_ROW: &str = "mbid";

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub data: Option<String>,
    pub fetched_at: i64,
    pub expires_at: i64,
}

impl Row {
    pub fn stale(&self, now: i64) -> bool {
        now >= self.expires_at
    }

    pub fn parse<T: DeserializeOwned>(&self) -> Option<T> {
        self.data.as_deref().and_then(|d| serde_json::from_str(d).ok())
    }
}

pub fn get(c: &Connection, artist_id: &str, source: &str) -> AppResult<Option<Row>> {
    Ok(c.query_row(
        "SELECT data, fetched_at, expires_at FROM artist_info WHERE artist_id = ?1 AND source = ?2",
        params![artist_id, source],
        |r| Ok(Row { data: r.get(0)?, fetched_at: r.get(1)?, expires_at: r.get(2)? }),
    )
    .optional()?)
}

/// Store a source's answer. Returns whether the stored data changed.
pub fn put<T: Serialize>(c: &Connection, artist_id: &str, source: &str, data: Option<&T>, now: i64, ttl: i64) -> AppResult<bool> {
    let json = data.and_then(|d| serde_json::to_string(d).ok());
    put_raw(c, artist_id, source, json, now, ttl)
}

pub fn put_raw(c: &Connection, artist_id: &str, source: &str, json: Option<String>, now: i64, ttl: i64) -> AppResult<bool> {
    let before = get(c, artist_id, source)?.map(|r| r.data);
    c.execute(
        "INSERT INTO artist_info (artist_id, source, data, fetched_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (artist_id, source) DO UPDATE SET data = excluded.data, fetched_at = excluded.fetched_at, expires_at = excluded.expires_at",
        params![artist_id, source, json, now, now + ttl],
    )?;
    Ok(before != Some(json))
}

/// Delete every row (album credits too). Returns how many artists had info.
pub fn clear(c: &Connection) -> AppResult<u64> {
    let n: i64 = c.query_row(
        "SELECT count(DISTINCT artist_id) FROM artist_info WHERE artist_id NOT LIKE 'album:%'",
        [],
        |r| r.get(0),
    )?;
    c.execute("DELETE FROM artist_info", [])?;
    Ok(n as u64)
}

/// The TTL for a reply: the cache headers' max-age when given, kept between
/// a day and 30 days.
pub fn ttl_for(max_age: Option<u64>, found: bool) -> i64 {
    let base = if found { TTL_S } else { MISS_TTL_S };
    match max_age {
        Some(s) => (s as i64).clamp(MIN_TTL_S, base),
        None => base,
    }
}
