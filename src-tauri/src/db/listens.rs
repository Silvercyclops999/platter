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
use crate::model::Track;

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

/// What a song was when it was played: kept on the play so history and
/// stats still read after the song leaves the library or is retagged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaySnapshot {
    pub title: Option<String>,
    /// The song's own artist (the track artist when it has one).
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
}

impl PlaySnapshot {
    pub fn of_track(t: &Track) -> Self {
        Self {
            title: Some(t.title.clone()),
            artist: Some(t.track_artist.clone().unwrap_or_else(|| t.artist_title.clone())),
            album: Some(t.album_title.clone()),
            album_artist: Some(t.artist_title.clone()),
        }
    }
}

/// Log a play. The snapshot comes from the track cache.
pub fn start_play(c: &Connection, p: &PlayStart) -> AppResult<i64> {
    start_play_with(c, p, None)
}

/// Log a play with what the player knew about the song. Anything the
/// snapshot leaves out comes from the track cache.
pub fn start_play_with(c: &Connection, p: &PlayStart, snap: Option<&PlaySnapshot>) -> AppResult<i64> {
    let snap = snap.cloned().unwrap_or_default();
    c.execute(
        "INSERT INTO listen_log (rating_key, part_key, started_at, duration_ms, context_kind,
           context_id, device, offline, snap_title, snap_artist, snap_album, snap_album_artist)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
           coalesce(?9, t.title), coalesce(?10, t.track_artist, t.artist_title),
           coalesce(?11, t.album_title), coalesce(?12, t.artist_title)
         FROM (SELECT 1) LEFT JOIN tracks t ON t.id = ?1",
        params![
            p.rating_key,
            p.part_key,
            p.started_at,
            p.duration_ms as i64,
            p.context_kind.map(ContextKind::as_str),
            p.context_id,
            p.device,
            p.offline,
            snap.title,
            snap.artist,
            snap.album,
            snap.album_artist
        ],
    )?;
    Ok(c.last_insert_rowid())
}

/// Plays filled per [`backfill_snapshots`] pass.
pub const BACKFILL_BATCH: u32 = 500;

/// Fill the snapshot on up to `limit` older plays whose song is still in
/// the track cache, oldest first. Returns how many were filled; 0 means
/// nothing is left that can be filled now. Plays of songs no longer in the
/// cache stay empty and read as unknown.
pub fn backfill_snapshots(c: &Connection, limit: u32) -> AppResult<usize> {
    Ok(c.execute(
        "UPDATE listen_log SET
           snap_title = t.title,
           snap_artist = coalesce(t.track_artist, t.artist_title),
           snap_album = t.album_title,
           snap_album_artist = t.artist_title,
           duration_ms = CASE WHEN listen_log.duration_ms > 0 THEN listen_log.duration_ms ELSE t.duration_ms END
         FROM tracks t
         WHERE t.id = listen_log.rating_key AND listen_log.id IN (
           SELECT l.id FROM listen_log l JOIN tracks t2 ON t2.id = l.rating_key
           WHERE l.snap_title IS NULL ORDER BY l.id LIMIT ?1)",
        [limit as i64],
    )?)
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

// ---------------------------------------------------------------- imported plays

/// Where an imported play came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportSource {
    /// The server's own play history.
    PlexHistory,
    Tautulli,
    /// Last.fm scrobbles.
    Lastfm,
    /// A listening history file the user picked.
    File,
}

impl ImportSource {
    pub const ALL: [ImportSource; 4] = [ImportSource::PlexHistory, ImportSource::Tautulli, ImportSource::Lastfm, ImportSource::File];

    pub fn as_str(self) -> &'static str {
        match self {
            ImportSource::PlexHistory => "plex_history",
            ImportSource::Tautulli => "tautulli",
            ImportSource::Lastfm => "lastfm",
            ImportSource::File => "file",
        }
    }

    pub fn parse(s: &str) -> Option<ImportSource> {
        ImportSource::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

/// A play brought in from elsewhere. `source_id` names it within its
/// source, so importing the same history twice adds nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedPlay {
    pub source: ImportSource,
    pub source_id: String,
    /// Unix ms.
    pub played_at: i64,
    pub ms_played: Option<u64>,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// The library song it was matched to, when known.
    pub rating_key: Option<String>,
}

/// Add imported plays, skipping any already imported from the same source.
/// Returns how many were new.
pub fn insert_imported(c: &mut Connection, plays: &[ImportedPlay], now_ms: i64) -> AppResult<usize> {
    let tx = c.transaction()?;
    let mut added = 0;
    {
        let mut st = tx.prepare_cached(
            "INSERT OR IGNORE INTO imported_plays
               (source, source_id, played_at, ms_played, title, artist, album, rating_key, matched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for p in plays {
            added += st.execute(params![
                p.source.as_str(),
                p.source_id,
                p.played_at,
                p.ms_played.map(|v| v as i64),
                p.title,
                p.artist,
                p.album,
                p.rating_key,
                p.rating_key.as_ref().map(|_| now_ms)
            ])?;
        }
    }
    tx.commit()?;
    Ok(added)
}

/// Record (or clear) the library song an imported play matches.
pub fn set_imported_match(
    c: &Connection,
    source: ImportSource,
    source_id: &str,
    rating_key: Option<&str>,
    now_ms: i64,
) -> AppResult<bool> {
    Ok(c.execute(
        "UPDATE imported_plays SET rating_key = ?3, matched_at = ?4 WHERE source = ?1 AND source_id = ?2",
        params![source.as_str(), source_id, rating_key, now_ms],
    )? > 0)
}

// ---------------------------------------------------------------- all plays

/// How far apart (ms) an imported play and a local play of the same song
/// can be and still be one play. Other apps log a play when it started, when
/// it counted or when it ended, so the window runs from this long before the
/// local play started to this long after it ended.
pub const SAME_PLAY_MS: i64 = 180_000;

/// Where the import sources left out of every play are kept (`kv`, a JSON
/// list of source names). Settings turn a source off here; its rows stay.
pub const EXCLUDED_KEY: &str = "plays.excludedSources";

/// Every play, local and imported, as one table: the one place stats, mixes,
/// recaps and smart playlists read play history from. Columns: `source`
/// ('local' or the import source), `source_id`, `played_at` (unix ms),
/// `ms_played`, `duration_ms`, `rating_key`, `title`, `artist`, `album`,
/// `album_artist`, `skipped`. Text comes from the snapshot, else the track
/// cache, else NULL.
///
/// Nothing counts twice:
/// - an imported play of the same song as a local (non-skip) play, from
///   [`SAME_PLAY_MS`] before it started to [`SAME_PLAY_MS`] after it ended,
///   is that play reported elsewhere (Platter reports its plays to the
///   server and scrobbles them), so it is left out;
/// - the same play imported from two sources (the server's history,
///   Tautulli and Last.fm all see a play from another app) counts once, from
///   the first source in that order: same song, within [`SAME_PLAY_MS`] plus
///   the song's length. Unmatched plays compare by title and artist.
///
/// Sources listed under [`EXCLUDED_KEY`] are left out entirely. The checks
/// look plays up by time (the `started_at` and `played_at` indexes), so the
/// view stays quick with long histories.
/// Use as `WITH plays AS (ALL_PLAYS) SELECT ... FROM plays`.
pub const ALL_PLAYS: &str = "
    SELECT 'local' AS source, CAST(l.id AS TEXT) AS source_id, l.started_at AS played_at,
           l.ms_listened AS ms_played,
           CASE WHEN l.duration_ms > 0 THEN l.duration_ms ELSE t.duration_ms END AS duration_ms,
           l.rating_key AS rating_key,
           coalesce(l.snap_title, t.title) AS title,
           coalesce(l.snap_artist, t.track_artist, t.artist_title) AS artist,
           coalesce(l.snap_album, t.album_title) AS album,
           coalesce(l.snap_album_artist, t.artist_title) AS album_artist,
           l.skipped AS skipped
    FROM listen_log l LEFT JOIN tracks t ON t.id = l.rating_key
    UNION ALL
    SELECT i.source, i.source_id, i.played_at, i.ms_played, t.duration_ms, i.rating_key,
           i.title, i.artist, i.album, t.artist_title, 0
    FROM imported_plays i LEFT JOIN tracks t ON t.id = i.rating_key
    WHERE i.source NOT IN (SELECT value FROM json_each(coalesce(
            (SELECT value FROM kv WHERE key = 'plays.excludedSources' AND json_valid(value)), '[]')))
      AND (i.rating_key IS NULL OR NOT EXISTS (
        SELECT 1 FROM listen_log l
        WHERE l.started_at BETWEEN i.played_at - 14400000 AND i.played_at + 180000
          AND +l.rating_key = i.rating_key AND l.skipped = 0
          AND coalesce(l.ended_at, l.started_at + l.duration_ms) + 180000 >= i.played_at))
      AND NOT EXISTS (
        SELECT 1 FROM imported_plays j
        WHERE j.played_at BETWEEN i.played_at - 180000 - coalesce(t.duration_ms, 0)
                              AND i.played_at + 180000 + coalesce(t.duration_ms, 0)
          AND (CASE j.source WHEN 'plex_history' THEN 1 WHEN 'tautulli' THEN 2 WHEN 'lastfm' THEN 3 ELSE 4 END)
            < (CASE i.source WHEN 'plex_history' THEN 1 WHEN 'tautulli' THEN 2 WHEN 'lastfm' THEN 3 ELSE 4 END)
          AND (+j.rating_key = i.rating_key
               OR (i.rating_key IS NULL AND +j.rating_key IS NULL
                   AND lower(j.title) = lower(i.title) AND lower(coalesce(j.artist, '')) = lower(coalesce(i.artist, ''))))
          AND j.source NOT IN (SELECT value FROM json_each(coalesce(
                (SELECT value FROM kv WHERE key = 'plays.excludedSources' AND json_valid(value)), '[]'))))";

/// The import sources left out of every play.
pub fn excluded_sources(c: &Connection) -> AppResult<Vec<ImportSource>> {
    let raw: Option<String> = c.query_row("SELECT value FROM kv WHERE key = ?1", [EXCLUDED_KEY], |r| r.get(0)).optional()?;
    let names: Vec<String> = raw.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
    Ok(ImportSource::ALL.into_iter().filter(|s| names.iter().any(|n| n == s.as_str())).collect())
}

/// Leave these import sources out of every play (an empty list includes
/// them all). The days with plays from a source that changed are counted
/// again. Returns whether anything changed.
pub fn set_excluded_sources(c: &mut Connection, sources: &[ImportSource]) -> AppResult<bool> {
    let before = excluded_sources(c)?;
    let after: Vec<ImportSource> = ImportSource::ALL.into_iter().filter(|s| sources.contains(s)).collect();
    if before == after {
        return Ok(false);
    }
    let names: Vec<&str> = after.iter().map(|s| s.as_str()).collect();
    let json = serde_json::to_string(&names).map_err(|e| AppError::Internal(e.to_string()))?;
    let tx = c.transaction()?;
    tx.execute(
        "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![EXCLUDED_KEY, json],
    )?;
    {
        let mut mark = tx.prepare_cached(
            "INSERT OR IGNORE INTO stats_dirty
             SELECT DISTINCT CAST(julianday(date(played_at / 1000, 'unixepoch', 'localtime')) - 2440587.5 AS INTEGER)
             FROM imported_plays WHERE source = ?1",
        )?;
        for s in ImportSource::ALL {
            if before.contains(&s) != after.contains(&s) {
                mark.execute([s.as_str()])?;
            }
        }
    }
    tx.commit()?;
    Ok(true)
}

/// One row of [`ALL_PLAYS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRecord {
    pub source: String,
    pub source_id: String,
    pub played_at: i64,
    pub ms_played: Option<u64>,
    pub duration_ms: Option<u64>,
    pub rating_key: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub skipped: bool,
}

/// Every play from `since_ms` (inclusive) to `until_ms` (exclusive), oldest
/// first, skips included (stats decide whether to count them).
pub fn all_plays(c: &Connection, since_ms: i64, until_ms: i64) -> AppResult<Vec<PlayRecord>> {
    let mut st = c.prepare_cached(&format!(
        "WITH plays AS ({ALL_PLAYS})
         SELECT source, source_id, played_at, ms_played, duration_ms, rating_key, title, artist,
                album, album_artist, skipped
         FROM plays WHERE played_at >= ?1 AND played_at < ?2
         ORDER BY played_at, source, source_id"
    ))?;
    let u = |v: Option<i64>| v.map(|v| v.max(0) as u64);
    let rows = st.query_map(params![since_ms, until_ms], |r| {
        Ok(PlayRecord {
            source: r.get(0)?,
            source_id: r.get(1)?,
            played_at: r.get(2)?,
            ms_played: u(r.get(3)?),
            duration_ms: u(r.get(4)?),
            rating_key: r.get(5)?,
            title: r.get(6)?,
            artist: r.get(7)?,
            album: r.get(8)?,
            album_artist: r.get(9)?,
            skipped: r.get(10)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
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

    use crate::db::schema::{SCHEMA_VERSION, migrate, migrate_to};

    fn has_snapshot(c: &Connection) -> bool {
        c.query_row("SELECT count(*) FROM pragma_table_info('listen_log') WHERE name = 'snap_title'", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
            > 0
    }

    /// The schema version just before the snapshot columns arrived.
    fn version_before_snapshot() -> usize {
        (1..=SCHEMA_VERSION as usize)
            .find(|&v| {
                let mut c = Connection::open_in_memory().unwrap();
                migrate_to(&mut c, v).unwrap();
                has_snapshot(&c)
            })
            .unwrap()
            - 1
    }

    fn db() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        migrate(&mut c).unwrap();
        c
    }

    fn add_track(c: &Connection, id: &str, title: &str, track_artist: Option<&str>) {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_artist, part_key, duration_ms)
             VALUES (?1, ?2, 'al', 'Album', 'ar', 'Album Artist', ?3, '/p', 180000)",
            params![id, title, track_artist],
        )
        .unwrap();
    }

    fn start(key: &str, at: i64) -> PlayStart {
        PlayStart {
            rating_key: key.into(),
            part_key: None,
            started_at: at,
            duration_ms: 180_000,
            context_kind: None,
            context_id: None,
            device: None,
            offline: false,
        }
    }

    fn snap_of(c: &Connection, id: i64) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
        c.query_row(
            "SELECT snap_title, snap_artist, snap_album, snap_album_artist FROM listen_log WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
    }

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn old_plays_survive_the_migration_and_still_read() {
        let mut c = Connection::open_in_memory().unwrap();
        migrate_to(&mut c, version_before_snapshot()).unwrap();
        assert!(!has_snapshot(&c));
        c.execute_batch(
            "INSERT INTO listen_log (rating_key, started_at, ended_at, duration_ms, ms_listened, completed, scrobbled)
             VALUES ('1', 1000, 181000, 180000, 180000, 1, 1), ('gone', 2000, NULL, 0, 5000, 0, 0);",
        )
        .unwrap();
        migrate(&mut c).unwrap();
        let p = get_play(&c, 1).unwrap().unwrap();
        assert_eq!((p.rating_key.as_str(), p.ms_listened, p.completed, p.scrobbled), ("1", 180_000, true, true));
        assert_eq!(snap_of(&c, 1), (None, None, None, None), "filled later, not by the migration");
        // Readable through the union even before the backfill: the cache
        // fills in what it can, a song no longer in the library reads as unknown.
        add_track(&c, "1", "Song", None);
        let all = all_plays(&c, 0, i64::MAX).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].title.clone(), all[0].artist.clone()), (s("Song"), s("Album Artist")));
        assert_eq!((all[1].title.clone(), all[1].rating_key.clone()), (None, s("gone")));
    }

    #[test]
    fn a_new_play_keeps_a_snapshot() {
        let c = db();
        add_track(&c, "1", "Cached", Some("Guest"));
        // From the cache when the player gives nothing.
        let a = start_play(&c, &start("1", 1)).unwrap();
        assert_eq!(snap_of(&c, a), (s("Cached"), s("Guest"), s("Album"), s("Album Artist")));
        // What the player knew wins over the cache.
        let snap = PlaySnapshot { title: s("Live"), artist: s("Band"), album: s("Tour"), album_artist: s("Band") };
        let b = start_play_with(&c, &start("1", 2), Some(&snap)).unwrap();
        assert_eq!(snap_of(&c, b), (s("Live"), s("Band"), s("Tour"), s("Band")));
        // A song that isn't cached still logs.
        let d = start_play(&c, &start("nope", 3)).unwrap();
        assert_eq!(snap_of(&c, d), (None, None, None, None));
        // The snapshot outlives the song.
        c.execute("DELETE FROM tracks", []).unwrap();
        let all = all_plays(&c, 0, 10).unwrap();
        assert_eq!(all.iter().map(|p| p.title.clone()).collect::<Vec<_>>(), vec![s("Cached"), s("Live"), None]);
    }

    #[test]
    fn backfill_fills_old_plays_in_batches_and_skips_songs_that_are_gone() {
        let c = db();
        c.execute_batch(
            "INSERT INTO listen_log (rating_key, started_at, duration_ms) VALUES
               ('1', 1, 0), ('1', 2, 170000), ('2', 3, 0), ('gone', 4, 0);",
        )
        .unwrap();
        add_track(&c, "1", "One", None);
        add_track(&c, "2", "Two", Some("Feat"));
        assert_eq!(backfill_snapshots(&c, 2).unwrap(), 2);
        assert_eq!(backfill_snapshots(&c, 2).unwrap(), 1);
        assert_eq!(backfill_snapshots(&c, 2).unwrap(), 0, "the missing song is left alone");
        assert_eq!(snap_of(&c, 1), (s("One"), s("Album Artist"), s("Album"), s("Album Artist")));
        assert_eq!(snap_of(&c, 3).1, s("Feat"));
        assert_eq!(snap_of(&c, 4), (None, None, None, None));
        let dur = |id: i64| c.query_row("SELECT duration_ms FROM listen_log WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!((dur(1), dur(2)), (180_000, 170_000), "only an unknown length is filled");
        // Later retags don't rewrite history.
        c.execute("UPDATE tracks SET title = 'Renamed' WHERE id = '1'", []).unwrap();
        assert_eq!(backfill_snapshots(&c, 10).unwrap(), 0);
        assert_eq!(snap_of(&c, 1).0, s("One"));
    }

    fn imported(source: ImportSource, id: &str, at: i64, key: Option<&str>) -> ImportedPlay {
        ImportedPlay {
            source,
            source_id: id.into(),
            played_at: at,
            ms_played: Some(60_000),
            title: format!("Imported {id}"),
            artist: s("Someone"),
            album: None,
            rating_key: key.map(str::to_string),
        }
    }

    #[test]
    fn importing_the_same_plays_twice_adds_nothing() {
        let mut c = db();
        let batch = vec![
            imported(ImportSource::PlexHistory, "a", 1, Some("1")),
            imported(ImportSource::PlexHistory, "b", 2, None),
            imported(ImportSource::Tautulli, "a", 1, None),
        ];
        assert_eq!(insert_imported(&mut c, &batch, 99).unwrap(), 3, "same id from another source is a different row");
        assert_eq!(insert_imported(&mut c, &batch, 100).unwrap(), 0);
        let mut more = batch.clone();
        more.push(imported(ImportSource::File, "x", 5, None));
        assert_eq!(insert_imported(&mut c, &more, 101).unwrap(), 1);
        let n: i64 = c.query_row("SELECT count(*) FROM imported_plays", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 4);
        let matched = |id: &str| {
            c.query_row(
                "SELECT rating_key, matched_at FROM imported_plays WHERE source = 'plex_history' AND source_id = ?1",
                [id],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .unwrap()
        };
        assert_eq!(matched("a"), (s("1"), Some(99)));
        assert_eq!(matched("b"), (None, None));
        assert!(set_imported_match(&c, ImportSource::PlexHistory, "b", Some("2"), 200).unwrap());
        assert_eq!(matched("b"), (s("2"), Some(200)));
        assert!(!set_imported_match(&c, ImportSource::PlexHistory, "zzz", None, 200).unwrap());
    }

    #[test]
    fn all_plays_unions_local_and_imported_without_counting_one_play_twice() {
        let mut c = db();
        add_track(&c, "1", "One", None);
        let p = start_play(&c, &start("1", 1_000_000)).unwrap();
        end_play(&c, p, 1_180_000, 180_000, 180_000, true).unwrap();
        let skip = start_play(&c, &start("1", 2_000_000)).unwrap();
        end_play(&c, skip, 2_005_000, 5_000, 5_000, false).unwrap();
        insert_imported(
            &mut c,
            &[
                // The same play as reported to the server when it counted.
                imported(ImportSource::PlexHistory, "same", 1_090_000, Some("1")),
                // During the skip: a skip isn't a play, so this one stands.
                imported(ImportSource::Tautulli, "during-skip", 2_002_000, Some("1")),
                // Same song, another day.
                imported(ImportSource::PlexHistory, "older", 500_000, Some("1")),
                // Unmatched, from a file.
                imported(ImportSource::File, "f1", 3_000_000, None),
            ],
            0,
        )
        .unwrap();
        let all = all_plays(&c, 0, i64::MAX).unwrap();
        let got: Vec<_> = all.iter().map(|p| (p.source.as_str(), p.source_id.as_str(), p.skipped)).collect();
        let (local, skipped) = (p.to_string(), skip.to_string());
        assert_eq!(
            got,
            vec![
                ("plex_history", "older", false),
                ("local", local.as_str(), false),
                ("local", skipped.as_str(), true),
                ("tautulli", "during-skip", false),
                ("file", "f1", false),
            ]
        );
        let older = &all[0];
        assert_eq!((older.title.clone(), older.album_artist.clone(), older.duration_ms), (s("Imported older"), s("Album Artist"), Some(180_000)));
        assert_eq!(all_plays(&c, 1_000_000, 2_000_000).unwrap().len(), 1, "since is inclusive, until exclusive");
        // Stats can build on the shared query.
        let n: i64 = c
            .query_row(&format!("WITH plays AS ({ALL_PLAYS}) SELECT count(*) FROM plays WHERE skipped = 0"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 4);
    }

    #[test]
    fn skip_rule() {
        assert!(is_skip(29_999, false));
        assert!(!is_skip(30_000, false));
        assert!(!is_skip(10_000, true), "a short track played to the end is not a skip");
    }
}
