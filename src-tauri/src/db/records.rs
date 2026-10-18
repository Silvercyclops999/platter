//! Home led by records (the Studio looks): the album you were in the middle
//! of, and four shelves of albums.
//!
//! - **Resume**: the album whose latest listening session (plays started
//!   from the album, no gap over [`SESSION_GAP_MS`]) stopped partway, most
//!   recent first, within [`RESUME_WINDOW_MS`]. It resumes at the song after
//!   the last one played when that one finished or was skipped, else at the
//!   same song (it was stopped in the middle).
//! - **Finish what you started**: the other albums stopped partway, within
//!   [`UNFINISHED_WINDOW_MS`]. "Partway" means the session played at least
//!   [`MIN_INTO`] songs, the album has [`MIN_TRACKS`] or more, and there is
//!   a song left where it stopped.
//! - **New arrivals**: albums added in the last [`NEW_WINDOW_S`], newest
//!   first.
//! - **Spun this week**: albums with at least [`SPUN_MIN_PLAYS`] counted
//!   plays in the last 7 days (the stats day tables), most played first.
//! - **Not played in a year**: albums you played at least
//!   [`LAPSED_MIN_PLAYS`] times whose last play anywhere (here, imported, or
//!   Plex on any device) is over a year ago, most played first.
//!
//! Every read is a handful of indexed queries over a bounded window; the
//! command keeps the answer until the plays or the library change.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::store::{ALBUM_COLS, row_album};
use crate::error::AppResult;
use crate::model::Album;

/// Cards on each shelf.
pub const SHELF_N: usize = 12;
/// A pause longer than this between two plays starts a new session.
pub const SESSION_GAP_MS: i64 = 2 * 3600 * 1000;
/// The hero only offers albums played this recently.
pub const RESUME_WINDOW_MS: i64 = 14 * 86_400_000;
/// Finish what you started looks back this far.
pub const UNFINISHED_WINDOW_MS: i64 = 60 * 86_400_000;
/// A session counts as started once it played this many songs.
pub const MIN_INTO: u32 = 2;
/// Albums with fewer songs than this are never "partway" (singles, short EPs).
pub const MIN_TRACKS: u32 = 4;
/// New arrivals look back this far.
pub const NEW_WINDOW_S: i64 = 90 * 86_400;
pub const SPUN_MIN_PLAYS: i64 = 2;
pub const LAPSED_MIN_PLAYS: i64 = 5;
const YEAR_MS: i64 = 365 * 86_400_000;
/// Plays read for sessions, at most (newest first).
const SESSION_ROWS: i64 = 5000;

/// An album you stopped partway through, and where to pick it up.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordProgress {
    pub album: Album,
    /// The song Resume starts at.
    pub next_track_id: String,
    pub next_title: String,
    /// Its place in the album, from 1.
    pub next_number: u32,
    /// Songs on the album.
    pub total: u32,
    /// The session's last play, unix ms.
    pub played_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordSpin {
    pub album: Album,
    /// Counted plays in the last 7 days.
    pub plays: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordLapsed {
    pub album: Album,
    /// All-time plays.
    pub plays: i64,
    /// The last play anywhere, unix ms.
    pub last_played: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordHome {
    /// False while the setting is off.
    pub shown: bool,
    pub resume: Option<RecordProgress>,
    pub new_arrivals: Vec<Album>,
    pub spun_this_week: Vec<RecordSpin>,
    pub unfinished: Vec<RecordProgress>,
    pub not_in_a_year: Vec<RecordLapsed>,
}

/// One play from an album, oldest first within an album.
#[derive(Debug, Clone)]
struct AlbumPlay {
    track: String,
    started: i64,
    ended: i64,
    /// Finished or skipped: the next song came after it.
    moved_on: bool,
}

/// Where the latest session of each album stopped: (album id, the session's
/// plays oldest first). Newest session first.
fn latest_sessions(c: &Connection, since_ms: i64) -> AppResult<Vec<(String, Vec<AlbumPlay>)>> {
    let mut st = c.prepare_cached(
        "SELECT context_id, rating_key, started_at,
                coalesce(ended_at, started_at + duration_ms), completed OR skipped
         FROM listen_log
         WHERE started_at >= ?1 AND context_kind = 'album' AND context_id IS NOT NULL
         ORDER BY started_at DESC, id DESC LIMIT ?2",
    )?;
    let rows = st.query_map(params![since_ms, SESSION_ROWS], |r| {
        Ok((
            r.get::<_, String>(0)?,
            AlbumPlay { track: r.get(1)?, started: r.get(2)?, ended: r.get(3)?, moved_on: r.get::<_, i64>(4)? != 0 },
        ))
    })?;
    // Newest first: each album's session grows backwards until a long gap.
    let mut order: Vec<String> = Vec::new();
    let mut sessions: HashMap<String, (Vec<AlbumPlay>, bool)> = HashMap::new();
    for row in rows {
        let (album, play) = row?;
        let e = sessions.entry(album.clone()).or_insert_with(|| {
            order.push(album);
            (Vec::new(), false)
        });
        let (plays, closed) = e;
        if *closed {
            continue;
        }
        if let Some(prev) = plays.last()
            && prev.started - play.ended.max(play.started) > SESSION_GAP_MS
        {
            *closed = true;
            continue;
        }
        plays.push(play);
    }
    Ok(order
        .into_iter()
        .filter_map(|id| {
            let (mut plays, _) = sessions.remove(&id)?;
            plays.reverse();
            Some((id, plays))
        })
        .collect())
}

/// An album's songs in order: (id, title).
fn album_tracks(c: &Connection, album: &str) -> AppResult<Vec<(String, String)>> {
    let mut st = c.prepare_cached("SELECT id, title FROM tracks WHERE album_id = ?1 ORDER BY disc_number, track_number, pk")?;
    Ok(st.query_map([album], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?)
}

/// Where a session stopped, when it stopped partway (see the module docs).
/// `tracks` is the album in order.
pub fn stopped_at(tracks: &[(String, String)], session: &[(String, bool)]) -> Option<usize> {
    if (tracks.len() as u32) < MIN_TRACKS {
        return None;
    }
    let pos = |id: &str| tracks.iter().position(|(t, _)| t == id);
    let (last, moved_on) = session.last()?;
    let at = pos(last)?;
    let next = if *moved_on { at + 1 } else { at };
    if next >= tracks.len() || (session.len() as u32) < MIN_INTO {
        return None;
    }
    Some(next)
}

/// Albums stopped partway, newest first (the hero is the first one inside
/// [`RESUME_WINDOW_MS`]).
pub fn unfinished(c: &Connection, now_ms: i64) -> AppResult<Vec<RecordProgress>> {
    let mut out = Vec::new();
    let mut album_st = c.prepare_cached(&format!("SELECT {ALBUM_COLS} FROM albums al WHERE al.id = ?1"))?;
    for (album_id, plays) in latest_sessions(c, now_ms - UNFINISHED_WINDOW_MS)? {
        if out.len() > SHELF_N {
            break;
        }
        let tracks = album_tracks(c, &album_id)?;
        let session: Vec<(String, bool)> = plays.iter().map(|p| (p.track.clone(), p.moved_on)).collect();
        let Some(next) = stopped_at(&tracks, &session) else { continue };
        let Some(album) = album_st.query_row([&album_id], row_album).optional()? else { continue };
        let (next_track_id, next_title) = tracks[next].clone();
        out.push(RecordProgress {
            album,
            next_track_id,
            next_title,
            next_number: next as u32 + 1,
            total: tracks.len() as u32,
            played_at: plays.last().map(|p| p.ended.max(p.started)).unwrap_or(0),
        });
    }
    Ok(out)
}

/// Albums added in the last [`NEW_WINDOW_S`], newest first.
pub fn new_arrivals(c: &Connection, now_s: i64) -> AppResult<Vec<Album>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {ALBUM_COLS} FROM albums al WHERE al.added_at >= ?1 ORDER BY al.added_at DESC, al.pk DESC LIMIT ?2"
    ))?;
    Ok(st.query_map(params![now_s - NEW_WINDOW_S, SHELF_N as i64], row_album)?.collect::<Result<_, _>>()?)
}

/// Albums by counted plays over the 7 days ending `today` (a stats day).
pub fn spun_this_week(c: &Connection, today: i64) -> AppResult<Vec<RecordSpin>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {ALBUM_COLS}, w.n FROM (
             SELECT s.album_id AS id, sum(d.plays) AS n, max(d.day) AS last
             FROM stats_album_day d JOIN stats_album s ON s.album_key = d.album_key
             WHERE d.day BETWEEN ?1 AND ?2 AND s.album_id IS NOT NULL
             GROUP BY s.album_id HAVING n >= ?3
         ) w JOIN albums al ON al.id = w.id
         ORDER BY w.n DESC, w.last DESC, al.pk LIMIT ?4"
    ))?;
    let rows = st.query_map(params![today - 6, today, SPUN_MIN_PLAYS, SHELF_N as i64], |r| {
        Ok(RecordSpin { album: row_album(r)?, plays: r.get(16)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Albums you used to play and haven't in a year, most played first. Counts
/// come from the stats tables, or Plex's own when those know more; the last
/// play is the later of the two.
pub fn not_in_a_year(c: &Connection, now_ms: i64) -> AppResult<Vec<RecordLapsed>> {
    let cutoff = now_ms - YEAR_MS;
    let mut st = c.prepare_cached(&format!(
        "SELECT {ALBUM_COLS}, x.plays, x.last FROM (
             SELECT al.id AS id,
                    max(max(coalesce(s.plays, 0)), al.view_count) AS plays,
                    max(max(coalesce(s.last_at, 0)), coalesce(al.last_viewed_at, 0) * 1000) AS last
             FROM albums al LEFT JOIN stats_album s ON s.album_id = al.id
             WHERE coalesce(s.plays, 0) >= ?1 OR al.view_count >= ?1
             GROUP BY al.id
         ) x JOIN albums al ON al.id = x.id
         WHERE x.plays >= ?1 AND x.last > 0 AND x.last < ?2
         ORDER BY x.plays DESC, x.last DESC, al.pk LIMIT ?3"
    ))?;
    let rows = st.query_map(params![LAPSED_MIN_PLAYS, cutoff, SHELF_N as i64], |r| {
        Ok(RecordLapsed { album: row_album(r)?, plays: r.get(16)?, last_played: r.get(17)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Everything Home shows first in the Studio looks. `today` is the local
/// stats day; the stats tables should be counted first.
pub fn record_home(c: &Connection, now_ms: i64, today: i64) -> AppResult<RecordHome> {
    let mut unfinished = unfinished(c, now_ms)?;
    let resume = match unfinished.first() {
        Some(p) if p.played_at >= now_ms - RESUME_WINDOW_MS => Some(unfinished.remove(0)),
        _ => None,
    };
    unfinished.truncate(SHELF_N);
    Ok(RecordHome {
        shown: true,
        resume,
        new_arrivals: new_arrivals(c, now_ms / 1000)?,
        spun_this_week: spun_this_week(c, today)?,
        unfinished,
        not_in_a_year: not_in_a_year(c, now_ms)?,
    })
}

/// What changes the answer: recent album plays and how they ended, the
/// counted plays, and the library's albums.
pub type Stamp = (i64, i64, i64, i64, i64, i64);

pub fn stamp(c: &Connection, now_ms: i64) -> AppResult<Stamp> {
    let (n, last, ended): (i64, i64, i64) = c.query_row(
        "SELECT count(*), coalesce(max(id), 0), coalesce(sum((ended_at IS NOT NULL) + completed + skipped), 0)
         FROM listen_log WHERE started_at >= ?1",
        [now_ms - UNFINISHED_WINDOW_MS],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let plays: i64 = c.query_row("SELECT coalesce(sum(plays), 0) FROM stats_hour", [], |r| r.get(0))?;
    let library: i64 = c.query_row(
        "SELECT count(*) * 1000003 + coalesce(max(added_at), 0) + coalesce(max(last_viewed_at), 0) FROM albums",
        [],
        |r| r.get(0),
    )?;
    Ok((n, last, ended, plays, library, now_ms / 3_600_000))
}
