//! The `share_links` table: one row per lookup (kind plus the folded search
//! fields), with the page found or NULL when there was no confident match.
//! Keyed by the fields rather than the Plex id, so retagging a song looks it
//! up again.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;

/// How long a found link is reused.
pub const TTL_S: i64 = 30 * 24 * 3600;
/// How long "not found" is remembered before asking again.
pub const MISS_TTL_S: i64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub url: Option<String>,
    pub expires_at: i64,
}

pub fn get(c: &Connection, key: &str) -> AppResult<Option<Row>> {
    Ok(c.query_row("SELECT url, expires_at FROM share_links WHERE key = ?1", params![key], |r| {
        Ok(Row { url: r.get(0)?, expires_at: r.get(1)? })
    })
    .optional()?)
}

pub fn put(c: &Connection, key: &str, url: Option<&str>, now: i64) -> AppResult<()> {
    let ttl = if url.is_some() { TTL_S } else { MISS_TTL_S };
    c.execute(
        "INSERT INTO share_links (key, url, fetched_at, expires_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (key) DO UPDATE SET url = excluded.url, fetched_at = excluded.fetched_at, expires_at = excluded.expires_at",
        params![key, url, now, now + ttl],
    )?;
    Ok(())
}

/// Drop expired rows.
pub fn prune(c: &Connection, now: i64) -> AppResult<usize> {
    Ok(c.execute("DELETE FROM share_links WHERE expires_at <= ?1", params![now])?)
}
