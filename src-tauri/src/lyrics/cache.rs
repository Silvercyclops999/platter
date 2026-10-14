//! The `lyrics_cache` table: one row per LRCLIB lookup, keyed by the folded
//! artist, title, album and duration, with the lyrics text found (NULL when
//! none) and whether it was matched by title only.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;

/// How long found lyrics are reused.
pub const TTL_S: i64 = 30 * 24 * 3600;
/// How long "none found" is remembered before asking again.
pub const MISS_TTL_S: i64 = 7 * 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub body: Option<String>,
    pub matched: bool,
    pub expires_at: i64,
}

pub fn get(c: &Connection, key: &str) -> AppResult<Option<Row>> {
    Ok(c.query_row("SELECT body, matched, expires_at FROM lyrics_cache WHERE key = ?1", params![key], |r| {
        Ok(Row { body: r.get(0)?, matched: r.get::<_, i64>(1)? != 0, expires_at: r.get(2)? })
    })
    .optional()?)
}

pub fn put(c: &Connection, key: &str, body: Option<&str>, matched: bool, now: i64) -> AppResult<()> {
    let ttl = if body.is_some() { TTL_S } else { MISS_TTL_S };
    c.execute(
        "INSERT INTO lyrics_cache (key, body, matched, fetched_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (key) DO UPDATE SET body = excluded.body, matched = excluded.matched,
           fetched_at = excluded.fetched_at, expires_at = excluded.expires_at",
        params![key, body, matched as i64, now, now + ttl],
    )?;
    Ok(())
}

/// Drop expired rows.
pub fn prune(c: &Connection, now: i64) -> AppResult<usize> {
    Ok(c.execute("DELETE FROM lyrics_cache WHERE expires_at <= ?1", params![now])?)
}
