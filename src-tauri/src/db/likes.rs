//! Likes in the local cache and the queue of rating writes waiting to reach
//! Plex (the `likes` module replays it).
//!
//! A like is a Plex user rating: 10 (five stars) when liked, cleared when
//! unliked. Settings choose whether four stars count as liked too. Every
//! change is applied here first so the app shows it at once and works
//! offline. Each track has at most one queued write, holding the rating it
//! replaced, so a write Plex refuses can be rolled back.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::AppResult;
use crate::model::Track;

use super::store::{TRACK_COLS, row_track};

/// The rating a like writes: five stars.
pub const LIKED_RATING: f64 = 10.0;

fn now_s() -> i64 {
    super::queries::now_s()
}

/// A rating write waiting for Plex.
#[derive(Debug, Clone, PartialEq)]
pub struct RatingOp {
    pub seq: i64,
    pub track_id: String,
    /// None clears the rating.
    pub rating: Option<f64>,
    pub prev_rating: Option<f64>,
    pub prev_rated_at: Option<i64>,
    pub attempts: u32,
}

fn same(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => (x - y).abs() < 1e-6,
        _ => false,
    }
}

/// Set a track's rating here and queue the write. Returns false when the
/// track is unknown or already has that rating. Going back to the rating
/// Plex still has before the write was sent drops the write.
pub fn set_rating(c: &Connection, track_id: &str, rating: Option<f64>) -> AppResult<bool> {
    let current: Option<(Option<f64>, Option<i64>)> = c
        .query_row("SELECT user_rating, rated_at FROM tracks WHERE id = ?1", [track_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let Some((now_rating, now_at)) = current else { return Ok(false) };
    if same(now_rating, rating) {
        return Ok(false);
    }
    let rated_at = rating.map(|_| now_s());
    let pending: Option<(Option<f64>, Option<i64>)> = c
        .query_row("SELECT prev_rating, prev_rated_at FROM rating_ops WHERE track_id = ?1", [track_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    match pending {
        Some((prev, prev_at)) if same(prev, rating) => {
            // Back to what Plex has: nothing to send, and its own time stays.
            c.execute("DELETE FROM rating_ops WHERE track_id = ?1", [track_id])?;
            c.execute("UPDATE tracks SET user_rating = ?2, rated_at = ?3 WHERE id = ?1", params![track_id, prev, prev_at])?;
            return Ok(true);
        }
        Some(_) => {
            c.execute("UPDATE rating_ops SET rating = ?2, attempts = 0 WHERE track_id = ?1", params![track_id, rating])?;
        }
        None => {
            c.execute(
                "INSERT INTO rating_ops (track_id, rating, prev_rating, prev_rated_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![track_id, rating, now_rating, now_at, now_s()],
            )?;
        }
    }
    c.execute("UPDATE tracks SET user_rating = ?2, rated_at = ?3 WHERE id = ?1", params![track_id, rating, rated_at])?;
    Ok(true)
}

/// Queued writes, oldest first.
pub fn queued(c: &Connection) -> AppResult<Vec<RatingOp>> {
    let mut st = c.prepare_cached(
        "SELECT seq, track_id, rating, prev_rating, prev_rated_at, attempts FROM rating_ops ORDER BY seq",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok(RatingOp {
                seq: r.get(0)?,
                track_id: r.get(1)?,
                rating: r.get(2)?,
                prev_rating: r.get(3)?,
                prev_rated_at: r.get(4)?,
                attempts: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The write reached Plex. Only drops it if it wasn't changed meanwhile.
pub fn sent(c: &Connection, op: &RatingOp) -> AppResult<()> {
    c.execute(
        "DELETE FROM rating_ops WHERE seq = ?1 AND rating IS ?2",
        params![op.seq, op.rating],
    )?;
    // Changed while it was being sent: what Plex has now is what was sent.
    c.execute(
        "UPDATE rating_ops SET prev_rating = ?2, prev_rated_at = NULL WHERE seq = ?1",
        params![op.seq, op.rating],
    )?;
    Ok(())
}

/// One more failed try; returns the new count.
pub fn bump_attempts(c: &Connection, seq: i64) -> AppResult<u32> {
    c.execute("UPDATE rating_ops SET attempts = attempts + 1 WHERE seq = ?1", [seq])?;
    Ok(c.query_row("SELECT attempts FROM rating_ops WHERE seq = ?1", [seq], |r| r.get(0)).optional()?.unwrap_or(0))
}

/// Plex refused the write: put the rating back as Plex has it and forget
/// the write.
pub fn roll_back(c: &Connection, op: &RatingOp) -> AppResult<()> {
    c.execute("DELETE FROM rating_ops WHERE seq = ?1", [op.seq])?;
    c.execute(
        "UPDATE tracks SET user_rating = ?2, rated_at = ?3 WHERE id = ?1",
        params![op.track_id, op.prev_rating, op.prev_rated_at],
    )?;
    Ok(())
}

/// Ids of liked tracks (rating at or above `min`), newest like first.
pub fn liked_ids(c: &Connection, min: f64) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached(
        "SELECT id FROM tracks WHERE user_rating >= ?1 ORDER BY rated_at IS NULL, rated_at DESC, pk",
    )?;
    let ids = st.query_map([min], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(ids)
}

/// Liked tracks, newest like first.
pub fn liked_tracks(c: &Connection, min: f64) -> AppResult<Vec<Track>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {TRACK_COLS} FROM tracks t WHERE t.user_rating >= ?1
         ORDER BY t.rated_at IS NULL, t.rated_at DESC, t.pk"
    ))?;
    let rows = st.query_map([min], row_track)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Playlists holding the track, most recently changed first.
pub fn playlists_with(c: &Connection, track_id: &str) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached(
        "SELECT p.id FROM playlists p
         WHERE EXISTS (SELECT 1 FROM playlist_items pi WHERE pi.playlist_id = p.id AND pi.track_id = ?1)
         ORDER BY p.updated_at IS NULL, p.updated_at DESC, p.title COLLATE NOCASE",
    )?;
    let ids = st.query_map([track_id], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(ids)
}

/// A rated track as Plex has it (sync).
#[derive(Debug, Clone, PartialEq)]
pub struct Rated {
    pub track_id: String,
    pub rating: f64,
    pub rated_at: Option<i64>,
}

/// Make the cache's ratings match Plex's list of every rated track: new
/// ratings in, changed ones updated, ratings Plex no longer has cleared.
/// Tracks with a write still queued here keep theirs. Returns the ids that
/// changed.
pub fn apply_rated(tx: &Transaction<'_>, rated: &[Rated]) -> AppResult<Vec<String>> {
    let mut st = tx.prepare_cached(
        "SELECT id, user_rating, rated_at FROM tracks
         WHERE user_rating IS NOT NULL AND id NOT IN (SELECT track_id FROM rating_ops)",
    )?;
    let mut have: HashMap<String, (Option<f64>, Option<i64>)> = st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))?
        .collect::<Result<_, _>>()?;
    drop(st);
    let mut changed = Vec::new();
    let mut set = tx.prepare_cached(
        "UPDATE tracks SET user_rating = ?2, rated_at = ?3
         WHERE id = ?1 AND id NOT IN (SELECT track_id FROM rating_ops)",
    )?;
    for r in rated {
        let now = have.remove(&r.track_id);
        let up_to_date = now.is_some_and(|(rating, at)| same(rating, Some(r.rating)) && at == r.rated_at);
        if !up_to_date && set.execute(params![r.track_id, r.rating, r.rated_at])? > 0 {
            changed.push(r.track_id.clone());
        }
    }
    // Rated here before, not in Plex's list any more: cleared elsewhere.
    for id in have.into_keys() {
        set.execute(params![id, Option::<f64>::None, Option::<i64>::None])?;
        changed.push(id);
    }
    Ok(changed)
}
