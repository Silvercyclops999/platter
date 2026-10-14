//! Local listen log and offline report queue.
//!
//! The caller (playback/tracking) owns policy: when a private session is
//! on it simply doesn't call [`start_play`]; when "report to Plex" is off it
//! doesn't enqueue reports. Times are unix milliseconds.
//!
//! Scrobble rule: once per play, when the listened time reaches
//! [`scrobble_threshold_ms`] — half the track or 4 minutes, whichever comes
//! first, but never under 30 s (so a skip under 30 s is never scrobbled);
//! a track shorter than 30 s must be listened to the end.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const SKIP_THRESHOLD_MS: u64 = 30_000;
pub const SCROBBLE_CAP_MS: u64 = 240_000;

/// Listened time needed before a play counts as a scrobble.
pub fn scrobble_threshold_ms(duration_ms: u64) -> u64 {
    if duration_ms == 0 {
        return SKIP_THRESHOLD_MS;
    }
    (duration_ms / 2).min(SCROBBLE_CAP_MS).max(SKIP_THRESHOLD_MS.min(duration_ms))
}

/// Should this play be scrobbled now?
pub fn should_scrobble(listened_ms: u64, duration_ms: u64, already_scrobbled: bool) -> bool {
    !already_scrobbled && listened_ms >= scrobble_threshold_ms(duration_ms)
}

/// A play that ended before completion with under 30 s listened is a skip.
pub fn is_skip(listened_ms: u64, completed: bool) -> bool {
    !completed && listened_ms < SKIP_THRESHOLD_MS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextKind {
    Album,
    Playlist,
    Artist,
    Mix,
    Radio,
    Search,
    Tracks,
    Liked,
}

impl ContextKind {
    fn as_str(self) -> &'static str {
        match self {
            ContextKind::Album => "album",
            ContextKind::Playlist => "playlist",
            ContextKind::Artist => "artist",
            ContextKind::Mix => "mix",
            ContextKind::Radio => "radio",
            ContextKind::Search => "search",
            ContextKind::Tracks => "tracks",
            ContextKind::Liked => "liked",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayStart {
    pub rating_key: String,
    pub part_key: Option<String>,
    pub started_at: i64,
    pub duration_ms: u64,
    pub context_kind: Option<ContextKind>,
    pub context_id: Option<String>,
    pub device: Option<String>,
    pub offline: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenRow {
    pub id: i64,
    pub rating_key: String,
    pub part_key: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_ms: u64,
    pub ms_listened: u64,
    pub completed: bool,
    pub skipped: bool,
    pub skipped_at_ms: Option<u64>,
    pub context_kind: Option<String>,
    pub context_id: Option<String>,
    pub device: Option<String>,
    pub offline: bool,
    pub scrobbled: bool,
    pub reported_to_plex: bool,
}

/// What [`progress`] / [`end_play`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayUpdate {
    /// True exactly once per play: the caller should send `/:/scrobble`
    /// (or enqueue it when offline).
    pub scrobble_now: bool,
    pub skipped: bool,
}

pub fn start_play(c: &Connection, p: &PlayStart) -> AppResult<i64> {
    c.execute(
        "INSERT INTO listen_log (rating_key, part_key, started_at, duration_ms, context_kind,
           context_id, device, offline)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            p.rating_key,
            p.part_key,
            p.started_at,
            p.duration_ms as i64,
            p.context_kind.map(ContextKind::as_str),
            p.context_id,
            p.device,
            p.offline
        ],
    )?;
    Ok(c.last_insert_rowid())
}

fn scrobble_state(c: &Connection, id: i64) -> AppResult<(u64, bool)> {
    c.query_row("SELECT duration_ms, scrobbled FROM listen_log WHERE id = ?1", [id], |r| {
        Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get(1)?))
    })
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("play {id}")))
}

/// Record listened time so far. Marks the play scrobbled the first time it
/// crosses the threshold and tells the caller to scrobble.
pub fn progress(c: &Connection, id: i64, ms_listened: u64) -> AppResult<PlayUpdate> {
    let (duration, scrobbled) = scrobble_state(c, id)?;
    let now = should_scrobble(ms_listened, duration, scrobbled);
    c.execute(
        "UPDATE listen_log SET ms_listened = max(ms_listened, ?2), scrobbled = scrobbled OR ?3 WHERE id = ?1",
        params![id, ms_listened as i64, now],
    )?;
    Ok(PlayUpdate { scrobble_now: now, skipped: false })
}

/// Close a play. `position_ms` is where playback stopped (logged as the
/// skip point when it was a skip).
pub fn end_play(
    c: &Connection,
    id: i64,
    ended_at: i64,
    ms_listened: u64,
    position_ms: u64,
    completed: bool,
) -> AppResult<PlayUpdate> {
    let (duration, scrobbled) = scrobble_state(c, id)?;
    let skipped = is_skip(ms_listened, completed);
    let now = !skipped && should_scrobble(ms_listened, duration, scrobbled);
    c.execute(
        "UPDATE listen_log SET ended_at = ?2, ms_listened = max(ms_listened, ?3), completed = ?4,
           skipped = ?5, skipped_at_ms = ?6, scrobbled = scrobbled OR ?7
         WHERE id = ?1",
        params![
            id,
            ended_at,
            ms_listened as i64,
            completed,
            skipped,
            skipped.then_some(position_ms as i64),
            now
        ],
    )?;
    Ok(PlayUpdate { scrobble_now: now, skipped })
}

pub fn mark_reported(c: &Connection, play_id: i64) -> AppResult<()> {
    c.execute("UPDATE listen_log SET reported_to_plex = 1 WHERE id = ?1", [play_id])?;
    Ok(())
}

pub fn get_play(c: &Connection, id: i64) -> AppResult<Option<ListenRow>> {
    Ok(c.query_row(
        "SELECT id, rating_key, part_key, started_at, ended_at, duration_ms, ms_listened, completed,
           skipped, skipped_at_ms, context_kind, context_id, device, offline, scrobbled, reported_to_plex
         FROM listen_log WHERE id = ?1",
        [id],
        |r| {
            Ok(ListenRow {
                id: r.get(0)?,
                rating_key: r.get(1)?,
                part_key: r.get(2)?,
                started_at: r.get(3)?,
                ended_at: r.get(4)?,
                duration_ms: r.get::<_, i64>(5)?.max(0) as u64,
                ms_listened: r.get::<_, i64>(6)?.max(0) as u64,
                completed: r.get(7)?,
                skipped: r.get(8)?,
                skipped_at_ms: r.get::<_, Option<i64>>(9)?.map(|v| v.max(0) as u64),
                context_kind: r.get(10)?,
                context_id: r.get(11)?,
                device: r.get(12)?,
                offline: r.get(13)?,
                scrobbled: r.get(14)?,
                reported_to_plex: r.get(15)?,
            })
        },
    )
    .optional()?)
}

// ---------------------------------------------------------------- report queue

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReportKind {
    /// `/:/timeline` (state + time).
    Timeline,
    /// `/:/scrobble`.
    Scrobble,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingReport {
    pub id: i64,
    pub play_id: Option<i64>,
    pub kind: ReportKind,
    pub rating_key: String,
    /// "playing" | "paused" | "stopped" for timeline reports.
    pub state: Option<String>,
    pub time_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    /// When it originally happened (unix ms), for replay with original times.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReport {
    pub play_id: Option<i64>,
    pub kind: ReportKind,
    pub rating_key: String,
    pub state: Option<String>,
    pub time_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub at: i64,
}

pub fn enqueue_report(c: &Connection, r: &NewReport) -> AppResult<i64> {
    let kind = match r.kind {
        ReportKind::Timeline => "timeline",
        ReportKind::Scrobble => "scrobble",
    };
    c.execute(
        "INSERT INTO report_queue (play_id, kind, rating_key, state, time_ms, duration_ms, at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            r.play_id,
            kind,
            r.rating_key,
            r.state,
            r.time_ms.map(|v| v as i64),
            r.duration_ms.map(|v| v as i64),
            r.at
        ],
    )?;
    Ok(c.last_insert_rowid())
}

/// Unsent reports in the order they happened (then insertion order).
pub fn pending_reports(c: &Connection, limit: u32) -> AppResult<Vec<PendingReport>> {
    let mut st = c.prepare_cached(
        "SELECT id, play_id, kind, rating_key, state, time_ms, duration_ms, at FROM report_queue
         WHERE sent_at IS NULL ORDER BY at, id LIMIT ?1",
    )?;
    let rows = st.query_map([limit as i64], |r| {
        let kind: String = r.get(2)?;
        Ok(PendingReport {
            id: r.get(0)?,
            play_id: r.get(1)?,
            kind: if kind == "scrobble" { ReportKind::Scrobble } else { ReportKind::Timeline },
            rating_key: r.get(3)?,
            state: r.get(4)?,
            time_ms: r.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u64),
            duration_ms: r.get::<_, Option<i64>>(6)?.map(|v| v.max(0) as u64),
            at: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Mark reports as sent; a play whose scrobble was sent is marked reported.
pub fn mark_sent(c: &mut Connection, ids: &[i64], sent_at: i64) -> AppResult<()> {
    let tx = c.transaction()?;
    {
        let mut st = tx.prepare_cached("UPDATE report_queue SET sent_at = ?2 WHERE id = ?1")?;
        let mut rep = tx.prepare_cached(
            "UPDATE listen_log SET reported_to_plex = 1 WHERE id =
               (SELECT play_id FROM report_queue WHERE id = ?1 AND kind = 'scrobble')",
        )?;
        for id in ids {
            st.execute(params![id, sent_at])?;
            rep.execute([id])?;
        }
    }
    // Keep a week of sent rows for debugging, drop older ones.
    tx.execute(
        "DELETE FROM report_queue WHERE sent_at IS NOT NULL AND sent_at < ?1",
        [sent_at - 7 * 24 * 3600 * 1000],
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrobble_thresholds() {
        // Long track: half, capped at 4 min.
        assert_eq!(scrobble_threshold_ms(600_000), 240_000);
        assert_eq!(scrobble_threshold_ms(300_000), 150_000);
        // Short track: at least 30 s.
        assert_eq!(scrobble_threshold_ms(40_000), 30_000);
        // Under 30 s: must be heard to the end.
        assert_eq!(scrobble_threshold_ms(20_000), 20_000);

        assert!(!should_scrobble(149_999, 300_000, false));
        assert!(should_scrobble(150_000, 300_000, false));
        assert!(should_scrobble(240_000, 900_000, false));
        assert!(!should_scrobble(239_999, 900_000, false));
        assert!(!should_scrobble(300_000, 300_000, true), "once per play");
        assert!(!should_scrobble(29_999, 40_000, false), "under 30 s never scrobbles");
    }

    #[test]
    fn skip_rule() {
        assert!(is_skip(29_999, false));
        assert!(!is_skip(30_000, false));
        assert!(!is_skip(10_000, true), "a short track played to the end is not a skip");
    }
}
