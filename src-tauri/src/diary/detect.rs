//! Albums played start to finish, from the listen log.
//!
//! A sitting is the plays started from an album with no pause longer than
//! [`SESSION_GAP_MS`] between them (the same rule as Home's records). It
//! makes an entry once it has heard [`MOST`] of the album's songs, the last
//! song among them, on an album of [`MIN_SONGS`] or more. A song counts as
//! heard when it finished or played at least half way. Hearing the album
//! through again straight after starts a new sitting, so it makes a second
//! entry.
//!
//! Only the listen log counts: plays imported from elsewhere (Plex history,
//! Tautulli, files) never make entries, though they do count for the first
//! time an album was heard.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::listens;
use crate::db::records::SESSION_GAP_MS;
use crate::error::AppResult;

/// Albums with fewer songs never make entries on their own (singles).
pub const MIN_SONGS: usize = 3;
/// The share of an album's songs a sitting must hear.
pub const MOST: f64 = 0.8;
/// How far back a check after a play looks for sittings.
pub const LOOKBACK_MS: i64 = 24 * 3600 * 1000;

/// A sitting that heard the album through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub album_id: String,
    /// The listen log row of the play that completed it.
    pub play_id: i64,
    pub started_at: i64,
    pub ended_at: i64,
    pub songs_played: u32,
    pub songs_total: u32,
    pub ms_listened: i64,
}

impl Finished {
    /// The entry's key in the sync store: one per sitting, named by the
    /// album and when the sitting started. Listen log row ids are this
    /// computer's own, so two devices syncing the diary could share one;
    /// a start time to the millisecond they won't.
    pub fn key(&self) -> String {
        entry_key_auto(&self.album_id, self.started_at)
    }

    /// The key older builds gave the same sitting (by listen log row), read
    /// so an entry or tombstone written under it still counts.
    pub fn legacy_key(&self) -> String {
        format!("auto:{}:{}", self.album_id, self.play_id)
    }
}

/// `auto:<album>:s<when the sitting started, unix ms>`.
pub fn entry_key_auto(album_id: &str, started_at: i64) -> String {
    format!("auto:{album_id}:s{started_at}")
}

/// Whether a play counts as heard.
pub fn heard(completed: bool, ms_listened: i64, duration_ms: i64) -> bool {
    completed || (duration_ms > 0 && ms_listened * 2 >= duration_ms)
}

/// An album's songs, in order.
pub fn album_songs(c: &Connection, album_id: &str) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached("SELECT id FROM tracks WHERE album_id = ?1 ORDER BY disc_number, track_number, pk")?;
    Ok(st.query_map([album_id], |r| r.get(0))?.collect::<Result<_, _>>()?)
}

struct Play {
    id: i64,
    track: String,
    started: i64,
    ended: i64,
    ms: i64,
    heard: bool,
}

/// Sittings started from `album_id` since `since_ms` that heard it through,
/// oldest first.
pub fn finished_sessions(c: &Connection, album_id: &str, since_ms: i64) -> AppResult<Vec<Finished>> {
    let songs = album_songs(c, album_id)?;
    if songs.len() < MIN_SONGS {
        return Ok(vec![]);
    }
    let on_album: HashSet<&str> = songs.iter().map(String::as_str).collect();
    let last = songs.last().map(String::as_str).unwrap_or_default();
    let need = ((songs.len() as f64) * MOST).ceil() as usize;
    let since_ms = sitting_start(c, album_id, since_ms)?;

    let mut st = c.prepare_cached(
        "SELECT id, rating_key, started_at, coalesce(ended_at, started_at + ms_listened), ms_listened, duration_ms, completed
         FROM listen_log
         WHERE context_kind = 'album' AND context_id = ?1 AND started_at >= ?2
         ORDER BY started_at, id",
    )?;
    let plays = st
        .query_map(params![album_id, since_ms], |r| {
            let (ms, dur, done): (i64, i64, i64) = (r.get(4)?, r.get(5)?, r.get(6)?);
            Ok(Play { id: r.get(0)?, track: r.get(1)?, started: r.get(2)?, ended: r.get(3)?, ms, heard: heard(done != 0, ms, dur) })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    let mut heard_songs: HashSet<&str> = HashSet::new();
    let mut start: Option<i64> = None;
    let mut last_end = 0i64;
    let mut ms_total = 0i64;
    for p in &plays {
        if !on_album.contains(p.track.as_str()) {
            continue;
        }
        if start.is_some() && p.started - last_end > SESSION_GAP_MS {
            heard_songs.clear();
            start = None;
            ms_total = 0;
        }
        let begun = *start.get_or_insert(p.started);
        last_end = last_end.max(p.ended.max(p.started));
        ms_total += p.ms.max(0);
        if p.heard {
            heard_songs.insert(p.track.as_str());
        }
        if p.heard && heard_songs.len() >= need && heard_songs.contains(last) {
            out.push(Finished {
                album_id: album_id.to_owned(),
                play_id: p.id,
                started_at: begun,
                ended_at: p.ended.max(p.started),
                songs_played: heard_songs.len() as u32,
                songs_total: songs.len() as u32,
                ms_listened: ms_total,
            });
            heard_songs.clear();
            start = None;
            ms_total = 0;
        }
    }
    Ok(out)
}

/// Where to start reading so a sitting the lookback cuts is read whole:
/// back past any play from the album that ends within [`SESSION_GAP_MS`] of
/// the start, until nothing before it is that close.
fn sitting_start(c: &Connection, album_id: &str, since_ms: i64) -> AppResult<i64> {
    let mut st = c.prepare_cached(
        "SELECT min(started_at) FROM listen_log
         WHERE context_kind = 'album' AND context_id = ?1 AND started_at < ?2
           AND coalesce(ended_at, started_at + ms_listened) >= ?2 - ?3",
    )?;
    let mut at = since_ms;
    // Each step goes back at least one play; a sitting is far fewer.
    for _ in 0..1000 {
        let earlier: Option<i64> = st.query_row(params![album_id, at, SESSION_GAP_MS], |r| r.get(0))?;
        match earlier {
            Some(e) if e < at => at = e,
            _ => break,
        }
    }
    Ok(at)
}

/// The earliest play of any of the album's songs: here, or imported from a
/// source that isn't left out of plays. None when it was never played.
pub fn first_play(c: &Connection, album_id: &str) -> AppResult<Option<i64>> {
    let here: Option<i64> = c
        .query_row(
            "SELECT min(started_at) FROM listen_log WHERE rating_key IN (SELECT id FROM tracks WHERE album_id = ?1)",
            [album_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let excluded: Vec<String> = listens::excluded_sources(c)?.into_iter().map(|s| s.as_str().to_owned()).collect();
    let excluded = serde_json::to_string(&excluded).unwrap_or_else(|_| "[]".into());
    let imported: Option<i64> = c
        .query_row(
            "SELECT min(played_at) FROM imported_plays
             WHERE rating_key IN (SELECT id FROM tracks WHERE album_id = ?1)
               AND source NOT IN (SELECT value FROM json_each(?2))",
            params![album_id, excluded],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(match (here, imported) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    })
}

/// What an entry shows about the album: title, artist, year, cover.
pub type AlbumBasics = (String, String, Option<i32>, Option<String>);

pub fn album_basics(c: &Connection, album_id: &str) -> AppResult<Option<AlbumBasics>> {
    let mut st = c.prepare_cached("SELECT title, artist_title, year, thumb FROM albums WHERE id = ?1")?;
    Ok(st.query_row([album_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?)
}
