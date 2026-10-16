//! Listening stats: totals, top lists, streaks and per-item numbers for any
//! range of days, built from every play (db::listens::ALL_PLAYS: the local
//! listen log plus imported plays).
//!
//! Plays are summed into small per-day tables (schema v28): one row per day
//! and song, artist, album or genre, and per day and hour. Triggers on the
//! listen log and imported plays mark the days that changed in
//! `stats_dirty`; [`refresh`] recounts only those days, so keeping up costs
//! one day's plays, not the whole history. Queries read the day tables, so
//! a range query touches at most one row per item per day in the range.
//!
//! Days are local calendar days, counted from 1970-01-01. A play counts
//! when it wasn't a skip. Private-session plays never reach the listen log,
//! so they never count.

pub mod commands;
pub mod compare;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::listens::ALL_PLAYS;
use crate::db::queries;
use crate::error::{AppError, AppResult};
use crate::model::Track;

/// The local day (days since 1970-01-01) of a unix ms column, in SQL.
macro_rules! day_of {
    ($col:literal) => {
        concat!("CAST(julianday(date(", $col, " / 1000, 'unixepoch', 'localtime')) - 2440587.5 AS INTEGER)")
    };
}

const DAY_MS: i64 = 86_400_000;
/// Ranges up to this many days chart by day; longer ones by month.
pub const DAILY_SERIES_MAX_DAYS: i64 = 92;
/// The longest top list a query returns.
pub const TOP_MAX: u32 = 100;
/// The longest calendar one call returns.
pub const DAYS_MAX: i64 = 400;
/// Months in an item's plays-over-time line.
pub const ITEM_SERIES_MONTHS: u32 = 12;

// ---------------------------------------------------------------- calendar

/// Days since 1970-01-01 for a calendar date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The calendar date of a day number.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// 0 is Sunday.
pub fn weekday(day: i64) -> u32 {
    (day + 4).rem_euclid(7) as u32
}

pub fn format_day(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn format_month(day: i64) -> String {
    let (y, m, _) = civil_from_days(day);
    format!("{y:04}-{m:02}")
}

fn bad_date(s: &str) -> AppError {
    AppError::Invalid(format!("Not a date: {s}"))
}

/// "YYYY-MM-DD" to a day number.
pub fn parse_day(s: &str) -> AppResult<i64> {
    let mut it = s.trim().splitn(3, '-');
    let (Some(y), Some(m), Some(d)) = (it.next(), it.next(), it.next()) else {
        return Err(bad_date(s));
    };
    let y: i64 = y.parse().map_err(|_| bad_date(s))?;
    let m: u32 = m.parse().map_err(|_| bad_date(s))?;
    let d: u32 = d.parse().map_err(|_| bad_date(s))?;
    if !(1..=12).contains(&m) || d < 1 || d > month_len(y, m) || !(1900..=9999).contains(&y) {
        return Err(bad_date(s));
    }
    Ok(days_from_civil(y, m, d))
}

/// "YYYY-MM" to the month's first and last day.
pub fn parse_month(s: &str) -> AppResult<(i64, i64)> {
    let (y, m) = s.trim().split_once('-').ok_or_else(|| bad_date(s))?;
    let y: i64 = y.parse().map_err(|_| bad_date(s))?;
    let m: u32 = m.parse().map_err(|_| bad_date(s))?;
    if !(1..=12).contains(&m) || !(1900..=9999).contains(&y) {
        return Err(bad_date(s));
    }
    Ok(month_bounds(y, m))
}

pub fn month_len(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

pub fn month_bounds(y: i64, m: u32) -> (i64, i64) {
    let first = days_from_civil(y, m, 1);
    (first, first + month_len(y, m) as i64 - 1)
}

/// The first day of the month before.
fn prev_month(y: i64, m: u32) -> (i64, u32) {
    if m == 1 { (y - 1, 12) } else { (y, m - 1) }
}

/// Today, as a local day number.
pub fn today(c: &Connection) -> AppResult<i64> {
    Ok(c.query_row("SELECT CAST(julianday(date('now', 'localtime')) - 2440587.5 AS INTEGER)", [], |r| r.get(0))?)
}

// ---------------------------------------------------------------- refresh

/// Whether some days wait to be counted again.
pub fn needs_refresh(c: &Connection) -> AppResult<bool> {
    Ok(c.query_row("SELECT EXISTS (SELECT 1 FROM stats_dirty)", [], |r| r.get(0))?)
}

/// Count again every day marked dirty. Returns how many days.
pub fn refresh(c: &mut Connection) -> AppResult<usize> {
    let (lo, hi, n): (Option<i64>, Option<i64>, i64) =
        c.query_row("SELECT min(day), max(day), count(*) FROM stats_dirty", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let (Some(lo), Some(hi)) = (lo, hi) else { return Ok(0) };
    // Local days are at most 14 hours off UTC days; a day either side covers it.
    let (lo_ms, hi_ms) = ((lo - 1) * DAY_MS, (hi + 2) * DAY_MS);
    let tx = c.transaction()?;
    tx.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS stats_touched (kind TEXT NOT NULL, key TEXT NOT NULL, PRIMARY KEY (kind, key)) WITHOUT ROWID;
         DELETE FROM temp.stats_touched;
         DROP TABLE IF EXISTS temp.stats_p;
         INSERT OR IGNORE INTO temp.stats_touched SELECT 's', song_key FROM stats_song_day WHERE day IN (SELECT day FROM stats_dirty);
         INSERT OR IGNORE INTO temp.stats_touched SELECT 'a', artist_key FROM stats_artist_day WHERE day IN (SELECT day FROM stats_dirty);
         INSERT OR IGNORE INTO temp.stats_touched SELECT 'b', album_key FROM stats_album_day WHERE day IN (SELECT day FROM stats_dirty);
         DELETE FROM stats_hour WHERE day IN (SELECT day FROM stats_dirty);
         DELETE FROM stats_song_day WHERE day IN (SELECT day FROM stats_dirty);
         DELETE FROM stats_artist_day WHERE day IN (SELECT day FROM stats_dirty);
         DELETE FROM stats_album_day WHERE day IN (SELECT day FROM stats_dirty);
         DELETE FROM stats_genre_day WHERE day IN (SELECT day FROM stats_dirty);",
    )?;
    // One row per counted play on a dirty day, with its keys. The artist is
    // the album artist, except on compilations ("Various Artists"), where
    // it's the song's own. Songs in the library are keyed by rating key,
    // others (unmatched imports) by artist and title.
    tx.execute(
        &format!(
            "CREATE TEMP TABLE stats_p AS
             WITH plays AS ({ALL_PLAYS}),
             p0 AS (
               SELECT {day} AS day,
                      CAST(strftime('%H', played_at / 1000, 'unixepoch', 'localtime') AS INTEGER) AS hour,
                      played_at, max(coalesce(ms_played, duration_ms, 0), 0) AS ms, rating_key,
                      coalesce(nullif(trim(title), ''), 'Unknown song') AS title,
                      coalesce(nullif(trim(artist), ''), nullif(trim(album_artist), ''), 'Unknown artist') AS artist,
                      nullif(trim(album), '') AS album,
                      nullif(trim(album_artist), '') AS album_artist
               FROM plays WHERE played_at >= ?1 AND played_at < ?2 AND skipped = 0),
             p1 AS (
               SELECT p0.*, t.artist_id AS t_artist_id, t.album_id AS t_album_id, t.genres AS genres,
                      (p0.album_artist IS NULL OR lower(p0.album_artist) IN ('various artists', 'various', 'va')) AS various,
                      coalesce(p0.album_artist, p0.artist) AS album_owner
               FROM p0 LEFT JOIN tracks t ON t.id = p0.rating_key
               WHERE p0.day IN (SELECT day FROM stats_dirty))
             SELECT day, hour, played_at, ms, rating_key, title, artist, album, genres,
                    CASE WHEN rating_key IS NOT NULL THEN 'k:' || rating_key
                         ELSE 'n:' || lower(artist) || '|' || lower(title) END AS song_key,
                    CASE WHEN various THEN artist ELSE album_artist END AS artist_name,
                    lower(CASE WHEN various THEN artist ELSE album_artist END) AS artist_key,
                    CASE WHEN various THEN NULL ELSE t_artist_id END AS artist_id,
                    album_owner,
                    CASE WHEN album IS NULL THEN NULL ELSE lower(album_owner) || '|' || lower(album) END AS album_key,
                    t_album_id AS album_id
             FROM p1",
            day = day_of!("played_at"),
        ),
        params![lo_ms, hi_ms],
    )?;
    tx.execute_batch(
        "INSERT INTO stats_hour SELECT day, hour, count(*), sum(ms) FROM temp.stats_p GROUP BY day, hour;
         INSERT INTO stats_song_day SELECT day, song_key, count(*), sum(ms), min(played_at), max(played_at)
           FROM temp.stats_p GROUP BY day, song_key;
         INSERT INTO stats_artist_day SELECT day, artist_key, count(*), sum(ms) FROM temp.stats_p GROUP BY day, artist_key;
         INSERT INTO stats_album_day SELECT day, album_key, count(*), sum(ms) FROM temp.stats_p
           WHERE album_key IS NOT NULL GROUP BY day, album_key;
         INSERT INTO stats_genre_day SELECT p.day, trim(g.value), count(*), sum(p.ms)
           FROM temp.stats_p p, json_each(CASE WHEN json_valid(p.genres) THEN p.genres ELSE '[]' END) g
           WHERE g.type = 'text' AND trim(g.value) <> '' GROUP BY p.day, trim(g.value);

         INSERT INTO stats_song (song_key, rating_key, title, artist, album, artist_key, album_key, seen_at)
           SELECT song_key, rating_key, title, artist, album, artist_key, album_key, max(played_at)
           FROM temp.stats_p WHERE true GROUP BY song_key
           ON CONFLICT (song_key) DO UPDATE SET rating_key = excluded.rating_key, title = excluded.title,
             artist = excluded.artist, album = excluded.album, artist_key = excluded.artist_key,
             album_key = excluded.album_key, seen_at = excluded.seen_at
           WHERE excluded.seen_at >= stats_song.seen_at;
         INSERT INTO stats_artist (artist_key, name, artist_id, seen_at)
           SELECT artist_key, artist_name, artist_id, max(played_at) FROM temp.stats_p WHERE true GROUP BY artist_key
           ON CONFLICT (artist_key) DO UPDATE SET
             name = CASE WHEN excluded.seen_at >= stats_artist.seen_at THEN excluded.name ELSE stats_artist.name END,
             artist_id = coalesce(excluded.artist_id, stats_artist.artist_id),
             seen_at = max(excluded.seen_at, stats_artist.seen_at);
         INSERT INTO stats_album (album_key, title, artist, album_id, seen_at)
           SELECT album_key, album, album_owner, album_id, max(played_at) FROM temp.stats_p
           WHERE album_key IS NOT NULL GROUP BY album_key
           ON CONFLICT (album_key) DO UPDATE SET
             title = CASE WHEN excluded.seen_at >= stats_album.seen_at THEN excluded.title ELSE stats_album.title END,
             artist = CASE WHEN excluded.seen_at >= stats_album.seen_at THEN excluded.artist ELSE stats_album.artist END,
             album_id = coalesce(excluded.album_id, stats_album.album_id),
             seen_at = max(excluded.seen_at, stats_album.seen_at);

         INSERT OR IGNORE INTO temp.stats_touched SELECT DISTINCT 's', song_key FROM temp.stats_p;
         INSERT OR IGNORE INTO temp.stats_touched SELECT DISTINCT 'a', artist_key FROM temp.stats_p;
         INSERT OR IGNORE INTO temp.stats_touched SELECT DISTINCT 'b', album_key FROM temp.stats_p WHERE album_key IS NOT NULL;

         UPDATE stats_song SET plays = 0, ms = 0, first_at = NULL, last_at = NULL
           WHERE song_key IN (SELECT key FROM temp.stats_touched WHERE kind = 's');
         UPDATE stats_song SET plays = x.p, ms = x.m, first_at = x.f, last_at = x.l
           FROM (SELECT song_key AS k, sum(plays) AS p, sum(ms) AS m, min(first_at) AS f, max(last_at) AS l
                 FROM stats_song_day WHERE song_key IN (SELECT key FROM temp.stats_touched WHERE kind = 's')
                 GROUP BY song_key) x
           WHERE stats_song.song_key = x.k;
         UPDATE stats_artist SET plays = 0, ms = 0
           WHERE artist_key IN (SELECT key FROM temp.stats_touched WHERE kind = 'a');
         UPDATE stats_artist SET plays = x.p, ms = x.m
           FROM (SELECT artist_key AS k, sum(plays) AS p, sum(ms) AS m
                 FROM stats_artist_day WHERE artist_key IN (SELECT key FROM temp.stats_touched WHERE kind = 'a')
                 GROUP BY artist_key) x
           WHERE stats_artist.artist_key = x.k;
         UPDATE stats_album SET plays = 0, ms = 0
           WHERE album_key IN (SELECT key FROM temp.stats_touched WHERE kind = 'b');
         UPDATE stats_album SET plays = x.p, ms = x.m
           FROM (SELECT album_key AS k, sum(plays) AS p, sum(ms) AS m
                 FROM stats_album_day WHERE album_key IN (SELECT key FROM temp.stats_touched WHERE kind = 'b')
                 GROUP BY album_key) x
           WHERE stats_album.album_key = x.k;
         UPDATE stats_artist SET
             first_at = (SELECT min(s.first_at) FROM stats_song s WHERE s.artist_key = stats_artist.artist_key AND s.plays > 0),
             last_at = (SELECT max(s.last_at) FROM stats_song s WHERE s.artist_key = stats_artist.artist_key AND s.plays > 0)
           WHERE artist_key IN (SELECT key FROM temp.stats_touched WHERE kind IN ('a', 's'))
              OR artist_key IN (SELECT s.artist_key FROM stats_song s
                                WHERE s.song_key IN (SELECT key FROM temp.stats_touched WHERE kind = 's'));
         UPDATE stats_album SET
             first_at = (SELECT min(s.first_at) FROM stats_song s WHERE s.album_key = stats_album.album_key AND s.plays > 0),
             last_at = (SELECT max(s.last_at) FROM stats_song s WHERE s.album_key = stats_album.album_key AND s.plays > 0)
           WHERE album_key IN (SELECT key FROM temp.stats_touched WHERE kind = 'b')
              OR album_key IN (SELECT s.album_key FROM stats_song s
                               WHERE s.song_key IN (SELECT key FROM temp.stats_touched WHERE kind = 's'));

         DELETE FROM stats_dirty;
         DELETE FROM temp.stats_touched;
         DROP TABLE temp.stats_p;",
    )?;
    tx.commit()?;
    Ok(n as usize)
}

// ---------------------------------------------------------------- ranges

/// Days to count, both ends included, as "YYYY-MM-DD". No ends: all time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsRange {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// A resolved range of day numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Days {
    pub lo: i64,
    pub hi: i64,
    /// All time: whole-history tables answer it.
    pub all: bool,
}

/// The first and last day with plays.
pub fn data_span(c: &Connection) -> AppResult<Option<(i64, i64)>> {
    let (lo, hi): (Option<i64>, Option<i64>) = c.query_row("SELECT min(day), max(day) FROM stats_hour", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(lo.zip(hi))
}

pub fn resolve(c: &Connection, r: &StatsRange) -> AppResult<Days> {
    let span = data_span(c)?;
    let today = today(c)?;
    let all = r.from.is_none() && r.to.is_none();
    let lo = match &r.from {
        Some(s) => parse_day(s)?,
        None => span.map(|s| s.0).unwrap_or(today),
    };
    let hi = match &r.to {
        Some(s) => parse_day(s)?,
        None => span.map(|s| s.1.max(today)).unwrap_or(today),
    };
    if hi < lo {
        return Err(AppError::Invalid("The range ends before it starts.".into()));
    }
    Ok(Days { lo, hi, all })
}

// ---------------------------------------------------------------- overview

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub ms: i64,
    pub plays: i64,
    pub songs: i64,
    pub artists: i64,
    pub albums: i64,
    /// Days with at least one play.
    pub days: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Bucket {
    Day,
    Month,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    /// "YYYY-MM-DD" for days, "YYYY-MM" for months.
    pub key: String,
    pub plays: i64,
    pub ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Streak {
    pub days: i64,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopSong {
    pub key: String,
    pub track_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub plays: i64,
    pub ms: i64,
    /// Counts in your own stats; never leaves the device in anything shared.
    pub unreleased: bool,
    /// The library song, when it's still there (rows play it).
    pub track: Option<Track>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopArtist {
    pub key: String,
    pub artist_id: Option<String>,
    pub name: String,
    pub thumb: Option<String>,
    pub plays: i64,
    pub ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopAlbum {
    pub key: String,
    pub album_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub thumb: Option<String>,
    pub plays: i64,
    pub ms: i64,
    pub unreleased: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopGenre {
    pub name: String,
    pub plays: i64,
    pub ms: i64,
    /// The cover of the range's most played album with songs in the genre.
    pub thumb: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsOverview {
    /// The range counted, resolved ("YYYY-MM-DD").
    pub from: String,
    pub to: String,
    pub totals: Totals,
    pub bucket: Bucket,
    pub series: Vec<Point>,
    pub streak: Option<Streak>,
    /// Listening time per hour of the day (0 to 23).
    pub hours: Vec<i64>,
    /// Listening time per weekday, 0 is Sunday.
    pub weekdays: Vec<i64>,
    pub busiest_hour: Option<u32>,
    pub busiest_weekday: Option<u32>,
    pub top_songs: Vec<TopSong>,
    pub top_artists: Vec<TopArtist>,
    pub top_albums: Vec<TopAlbum>,
    pub top_genres: Vec<TopGenre>,
}

/// Plays and listening time per day with plays, oldest first.
fn daily(c: &Connection, d: Days) -> AppResult<Vec<(i64, i64, i64)>> {
    let mut st = c.prepare_cached("SELECT day, sum(plays), sum(ms) FROM stats_hour WHERE day BETWEEN ?1 AND ?2 GROUP BY day ORDER BY day")?;
    let rows = st.query_map([d.lo, d.hi], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// The longest run of days in a row with plays (the latest when tied).
pub fn longest_streak(days: &[i64]) -> Option<Streak> {
    let mut best: Option<(i64, i64)> = None;
    let mut start = None;
    let mut prev = None;
    for &d in days {
        if prev.is_none_or(|p| d != p + 1) {
            start = Some(d);
        }
        let s = start.unwrap_or(d);
        if best.is_none_or(|(bs, be)| d - s >= be - bs) {
            best = Some((s, d));
        }
        prev = Some(d);
    }
    best.map(|(s, e)| Streak { days: e - s + 1, from: format_day(s), to: format_day(e) })
}

/// The chart: every day in short ranges, every month in long ones, gaps as zero.
pub fn series(days: &[(i64, i64, i64)], d: Days) -> (Bucket, Vec<Point>) {
    if d.hi - d.lo < DAILY_SERIES_MAX_DAYS {
        let by: HashMap<i64, (i64, i64)> = days.iter().map(|&(day, p, m)| (day, (p, m))).collect();
        let pts = (d.lo..=d.hi)
            .map(|day| {
                let (plays, ms) = by.get(&day).copied().unwrap_or((0, 0));
                Point { key: format_day(day), plays, ms }
            })
            .collect();
        return (Bucket::Day, pts);
    }
    let mut pts: Vec<Point> = Vec::new();
    let (mut y, mut m, _) = civil_from_days(d.lo);
    let (ey, em, _) = civil_from_days(d.hi);
    while (y, m) <= (ey, em) {
        pts.push(Point { key: format!("{y:04}-{m:02}"), plays: 0, ms: 0 });
        (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    }
    let first = civil_from_days(d.lo);
    for &(day, p, ms) in days {
        let (yy, mm, _) = civil_from_days(day);
        let i = ((yy - first.0) * 12 + mm as i64 - first.1 as i64) as usize;
        if let Some(pt) = pts.get_mut(i) {
            pt.plays += p;
            pt.ms += ms;
        }
    }
    (Bucket::Month, pts)
}

fn top_keys(c: &Connection, table: &str, key: &str, dims: &str, d: Days, limit: u32) -> AppResult<Vec<(String, i64, i64)>> {
    let sql = if d.all {
        format!("SELECT {key}, plays, ms FROM {dims} WHERE plays > 0 ORDER BY plays DESC, ms DESC, {key} LIMIT ?1")
    } else {
        format!(
            "SELECT {key}, sum(plays) AS p, sum(ms) AS m FROM {table} WHERE day BETWEEN ?1 AND ?2
             GROUP BY {key} ORDER BY p DESC, m DESC, {key} LIMIT ?3"
        )
    };
    let mut st = c.prepare_cached(&sql)?;
    let rows = if d.all {
        st.query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?
    } else {
        st.query_map(params![d.lo, d.hi, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?
    };
    Ok(rows)
}

fn track_unreleased(c: &Connection, id: &str) -> AppResult<bool> {
    let mut st = c.prepare_cached("SELECT EXISTS (SELECT 1 FROM song_index WHERE track_id = ?1 AND unreleased = 1)")?;
    Ok(st.query_row([id], |r| r.get(0))?)
}

fn album_unreleased(c: &Connection, album_id: &str) -> AppResult<bool> {
    let mut st = c.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM tracks t JOIN song_index s ON s.track_id = t.id WHERE t.album_id = ?1 AND s.unreleased = 1)",
    )?;
    Ok(st.query_row([album_id], |r| r.get(0))?)
}

/// Songs for keys with their counts, in the given order.
pub fn songs_for(c: &Connection, keys: &[(String, i64, i64)]) -> AppResult<Vec<TopSong>> {
    let mut st = c.prepare_cached("SELECT rating_key, title, artist, album FROM stats_song WHERE song_key = ?1")?;
    let mut out = Vec::with_capacity(keys.len());
    for (k, plays, ms) in keys {
        let Some((rk, title, artist, album)) = st
            .query_row([k], |r| Ok((r.get::<_, Option<String>>(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .optional()?
        else {
            continue;
        };
        let track = match &rk {
            Some(id) => queries::tracks_by_ids(c, std::slice::from_ref(id))?.pop(),
            None => None,
        };
        let unreleased = match &rk {
            Some(id) => track_unreleased(c, id)?,
            None => false,
        };
        out.push(TopSong { key: k.clone(), track_id: track.as_ref().map(|t| t.id.clone()), title, artist, album, plays: *plays, ms: *ms, unreleased, track });
    }
    Ok(out)
}

/// The library artist for an artist key: its id and picture.
fn library_artist(c: &Connection, id: Option<&str>, key: &str) -> AppResult<Option<(String, Option<String>)>> {
    if let Some(id) = id {
        let mut st = c.prepare_cached("SELECT id, thumb FROM artists WHERE id = ?1")?;
        if let Some(a) = st.query_row([id], |r| Ok((r.get(0)?, r.get(1)?))).optional()? {
            return Ok(Some(a));
        }
    }
    let mut st = c.prepare_cached("SELECT id, thumb FROM artists WHERE lower(title) = ?1 LIMIT 1")?;
    Ok(st.query_row([key], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
}

pub fn artists_for(c: &Connection, keys: &[(String, i64, i64)]) -> AppResult<Vec<TopArtist>> {
    let mut st = c.prepare_cached("SELECT name, artist_id FROM stats_artist WHERE artist_key = ?1")?;
    let mut out = Vec::with_capacity(keys.len());
    for (k, plays, ms) in keys {
        let Some((name, id)) = st.query_row([k], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))).optional()? else {
            continue;
        };
        let lib = library_artist(c, id.as_deref(), k)?;
        out.push(TopArtist {
            key: k.clone(),
            artist_id: lib.as_ref().map(|a| a.0.clone()),
            thumb: lib.and_then(|a| a.1),
            name,
            plays: *plays,
            ms: *ms,
        });
    }
    Ok(out)
}

pub fn albums_for(c: &Connection, keys: &[(String, i64, i64)]) -> AppResult<Vec<TopAlbum>> {
    let mut st = c.prepare_cached(
        "SELECT s.title, s.artist, a.id, a.thumb FROM stats_album s LEFT JOIN albums a ON a.id = s.album_id WHERE s.album_key = ?1",
    )?;
    let mut out = Vec::with_capacity(keys.len());
    for (k, plays, ms) in keys {
        let Some((title, artist, id, thumb)) = st
            .query_row([k], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get(3)?)))
            .optional()?
        else {
            continue;
        };
        let unreleased = match &id {
            Some(id) => album_unreleased(c, id)?,
            None => false,
        };
        out.push(TopAlbum { key: k.clone(), album_id: id, title, artist, thumb, plays: *plays, ms: *ms, unreleased });
    }
    Ok(out)
}

pub fn top_genres(c: &Connection, d: Days, limit: u32) -> AppResult<Vec<TopGenre>> {
    let mut st = c.prepare_cached(
        "SELECT genre, sum(plays) AS p, sum(ms) AS m FROM stats_genre_day WHERE day BETWEEN ?1 AND ?2
         GROUP BY genre ORDER BY p DESC, m DESC, genre LIMIT ?3",
    )?;
    let rows = st.query_map(params![d.lo, d.hi, limit], |r| Ok(TopGenre { name: r.get(0)?, plays: r.get(1)?, ms: r.get(2)?, thumb: None }))?;
    let mut out = rows.collect::<Result<Vec<_>, _>>()?;
    if !out.is_empty() {
        let covers = genre_covers(c, d)?;
        for g in &mut out {
            g.thumb = covers.get(&g.name.to_lowercase()).cloned();
        }
    }
    Ok(out)
}

/// Albums looked through for genre covers.
const GENRE_COVER_ALBUMS: u32 = 200;

/// The cover for each genre: the range's most played album (of its top
/// [`GENRE_COVER_ALBUMS`]) with a song in that genre. Keyed by lower-case genre.
fn genre_covers(c: &Connection, d: Days) -> AppResult<HashMap<String, String>> {
    let mut st = c.prepare_cached(
        "WITH top AS (SELECT album_key, sum(plays) AS p FROM stats_album_day WHERE day BETWEEN ?1 AND ?2
                      GROUP BY album_key ORDER BY p DESC LIMIT ?3)
         SELECT DISTINCT lower(trim(g.value)), a.thumb, top.p
         FROM top JOIN stats_album s ON s.album_key = top.album_key
         JOIN albums a ON a.id = s.album_id
         JOIN tracks t ON t.album_id = a.id,
         json_each(CASE WHEN json_valid(t.genres) THEN t.genres ELSE '[]' END) g
         WHERE a.thumb IS NOT NULL AND g.type = 'text'
         ORDER BY top.p DESC",
    )?;
    let mut out = HashMap::new();
    for row in st.query_map(params![d.lo, d.hi, GENRE_COVER_ALBUMS], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (g, thumb) = row?;
        out.entry(g).or_insert(thumb);
    }
    Ok(out)
}

pub fn top_songs(c: &Connection, d: Days, limit: u32) -> AppResult<Vec<TopSong>> {
    songs_for(c, &top_keys(c, "stats_song_day", "song_key", "stats_song", d, limit)?)
}

pub fn top_artists(c: &Connection, d: Days, limit: u32) -> AppResult<Vec<TopArtist>> {
    artists_for(c, &top_keys(c, "stats_artist_day", "artist_key", "stats_artist", d, limit)?)
}

pub fn top_albums(c: &Connection, d: Days, limit: u32) -> AppResult<Vec<TopAlbum>> {
    albums_for(c, &top_keys(c, "stats_album_day", "album_key", "stats_album", d, limit)?)
}

fn distinct(c: &Connection, table: &str, key: &str, dims: &str, d: Days) -> AppResult<i64> {
    if d.all {
        return Ok(c.query_row(&format!("SELECT count(*) FROM {dims} WHERE plays > 0"), [], |r| r.get(0))?);
    }
    Ok(c.prepare_cached(&format!("SELECT count(DISTINCT {key}) FROM {table} WHERE day BETWEEN ?1 AND ?2"))?
        .query_row([d.lo, d.hi], |r| r.get(0))?)
}

pub fn overview(c: &Connection, d: Days, limit: u32) -> AppResult<StatsOverview> {
    let limit = limit.clamp(1, TOP_MAX);
    let days = daily(c, d)?;
    let mut totals = Totals {
        plays: days.iter().map(|x| x.1).sum(),
        ms: days.iter().map(|x| x.2).sum(),
        days: days.len() as i64,
        ..Default::default()
    };
    totals.songs = distinct(c, "stats_song_day", "song_key", "stats_song", d)?;
    totals.artists = distinct(c, "stats_artist_day", "artist_key", "stats_artist", d)?;
    totals.albums = distinct(c, "stats_album_day", "album_key", "stats_album", d)?;

    let mut hours = vec![0i64; 24];
    let mut st = c.prepare_cached("SELECT hour, sum(ms) FROM stats_hour WHERE day BETWEEN ?1 AND ?2 GROUP BY hour")?;
    for row in st.query_map([d.lo, d.hi], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (h, ms) = row?;
        if let Some(x) = hours.get_mut(h as usize) {
            *x = ms;
        }
    }
    let mut weekdays = vec![0i64; 7];
    for &(day, _, ms) in &days {
        weekdays[weekday(day) as usize] += ms;
    }
    let busiest = |v: &[i64]| {
        v.iter().enumerate().filter(|(_, ms)| **ms > 0).max_by_key(|(i, ms)| (**ms, std::cmp::Reverse(*i))).map(|(i, _)| i as u32)
    };
    let (bucket, series) = series(&days, d);
    let day_list: Vec<i64> = days.iter().map(|x| x.0).collect();
    Ok(StatsOverview {
        from: format_day(d.lo),
        to: format_day(d.hi),
        totals,
        bucket,
        series,
        streak: longest_streak(&day_list),
        busiest_hour: busiest(&hours),
        busiest_weekday: busiest(&weekdays),
        hours,
        weekdays,
        top_songs: top_songs(c, d, limit)?,
        top_artists: top_artists(c, d, limit)?,
        top_albums: top_albums(c, d, limit)?,
        top_genres: top_genres(c, d, limit.min(20))?,
    })
}

/// What the range pickers offer: the first and last day with plays and
/// every month with plays, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSpan {
    pub first: Option<String>,
    pub last: Option<String>,
    pub months: Vec<String>,
    pub today: String,
}

pub fn span(c: &Connection) -> AppResult<StatsSpan> {
    let s = data_span(c)?;
    let mut st = c.prepare_cached("SELECT DISTINCT day FROM stats_hour ORDER BY day DESC")?;
    let mut months: Vec<String> = Vec::new();
    for day in st.query_map([], |r| r.get::<_, i64>(0))? {
        let m = format_month(day?);
        if months.last() != Some(&m) {
            months.push(m);
        }
    }
    Ok(StatsSpan { first: s.map(|s| format_day(s.0)), last: s.map(|s| format_day(s.1)), months, today: format_day(today(c)?) })
}

// ---------------------------------------------------------------- play these

/// What "Play these" plays for a top list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TopKind {
    Songs,
    Artists,
    Albums,
    Genres,
}

/// Songs to play for a top list: the top songs themselves, or the range's
/// most played songs by its top artists, albums or genres. Library songs
/// only, most played first.
pub fn play_ids(c: &Connection, d: Days, kind: TopKind, limit: u32) -> AppResult<Vec<String>> {
    let limit = limit.clamp(1, TOP_MAX);
    let filter = match kind {
        TopKind::Songs => String::new(),
        TopKind::Artists => "AND s.artist_key IN (SELECT k FROM top)".into(),
        TopKind::Albums => "AND s.album_key IN (SELECT k FROM top)".into(),
        TopKind::Genres => "AND EXISTS (SELECT 1 FROM tracks t, json_each(CASE WHEN json_valid(t.genres) THEN t.genres ELSE '[]' END) g
                             WHERE t.id = s.rating_key AND g.value IN (SELECT k FROM top))"
            .into(),
    };
    let top = match kind {
        TopKind::Songs => "SELECT NULL AS k".to_string(),
        TopKind::Artists => "SELECT artist_key AS k FROM stats_artist_day WHERE day BETWEEN ?1 AND ?2 GROUP BY artist_key ORDER BY sum(plays) DESC, sum(ms) DESC LIMIT 10".into(),
        TopKind::Albums => "SELECT album_key AS k FROM stats_album_day WHERE day BETWEEN ?1 AND ?2 GROUP BY album_key ORDER BY sum(plays) DESC, sum(ms) DESC LIMIT 10".into(),
        TopKind::Genres => "SELECT genre AS k FROM stats_genre_day WHERE day BETWEEN ?1 AND ?2 GROUP BY genre ORDER BY sum(plays) DESC, sum(ms) DESC LIMIT 5".into(),
    };
    let sql = format!(
        "WITH top AS ({top})
         SELECT s.rating_key FROM stats_song_day d JOIN stats_song s ON s.song_key = d.song_key
         JOIN tracks t0 ON t0.id = s.rating_key
         WHERE d.day BETWEEN ?1 AND ?2 {filter}
         GROUP BY s.song_key ORDER BY sum(d.plays) DESC, sum(d.ms) DESC, s.song_key LIMIT ?3"
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(params![d.lo, d.hi, limit], |r| r.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

// ---------------------------------------------------------------- one item

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Song,
    Artist,
    Album,
}

/// Where a play happened: the album, playlist or artist it was played from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayedFrom {
    /// album | playlist | artist | mix | radio | search | tracks | liked |
    /// imported (another app's history; `id` then names where from:
    /// plex_history, tautulli, lastfm or file).
    pub kind: String,
    pub id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstListen {
    /// Unix ms.
    pub at: i64,
    pub song: String,
    pub track_id: Option<String>,
    pub from: Option<PlayedFrom>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemStats {
    pub kind: ItemKind,
    pub plays: i64,
    pub ms: i64,
    pub first: Option<FirstListen>,
    /// Unix ms.
    pub last_at: Option<i64>,
    /// Place in your all-time top for its kind (1 is most played).
    pub rank: Option<i64>,
    /// Plays per month, the last [`ITEM_SERIES_MONTHS`] months, oldest first.
    pub series: Vec<Point>,
}

/// A play brought in from elsewhere, named by its source.
fn imported_from(source: String) -> PlayedFrom {
    PlayedFrom { kind: "imported".into(), id: Some(source), title: None }
}

fn played_from(c: &Connection, rating_key: &str, at: i64) -> AppResult<Option<PlayedFrom>> {
    let local: Option<(Option<String>, Option<String>)> = c
        .prepare_cached("SELECT context_kind, context_id FROM listen_log WHERE rating_key = ?1 AND started_at = ?2 LIMIT 1")?
        .query_row(params![rating_key, at], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((kind, id)) = local else {
        let source: Option<String> = c
            .prepare_cached("SELECT source FROM imported_plays WHERE rating_key = ?1 AND played_at = ?2 ORDER BY id LIMIT 1")?
            .query_row(params![rating_key, at], |r| r.get(0))
            .optional()?;
        return Ok(source.map(imported_from));
    };
    let Some(kind) = kind else { return Ok(None) };
    let table = match kind.as_str() {
        "album" => Some("albums"),
        "playlist" => Some("playlists"),
        "artist" => Some("artists"),
        _ => None,
    };
    let title = match (table, &id) {
        (Some(t), Some(id)) => c
            .prepare_cached(&format!("SELECT title FROM {t} WHERE id = ?1"))?
            .query_row([id], |r| r.get::<_, String>(0))
            .optional()?,
        _ => None,
    };
    Ok(Some(PlayedFrom { kind, id, title }))
}

fn first_listen(c: &Connection, song_key: &str) -> AppResult<Option<FirstListen>> {
    let row: Option<(Option<String>, String, Option<i64>)> = c
        .prepare_cached("SELECT rating_key, title, first_at FROM stats_song WHERE song_key = ?1")?
        .query_row([song_key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    let Some((rk, song, Some(at))) = row else { return Ok(None) };
    let from = match &rk {
        Some(k) => played_from(c, k, at)?,
        None => {
            let source: Option<String> = c
                .prepare_cached("SELECT source FROM imported_plays WHERE rating_key IS NULL AND played_at = ?1 ORDER BY id LIMIT 1")?
                .query_row([at], |r| r.get(0))
                .optional()?;
            Some(imported_from(source.unwrap_or_else(|| "file".into())))
        }
    };
    let track_id = match &rk {
        Some(k) => queries::tracks_by_ids(c, std::slice::from_ref(k))?.pop().map(|t| t.id),
        None => None,
    };
    Ok(Some(FirstListen { at, song, track_id, from }))
}

/// The artist key for a library artist.
fn artist_key_for(c: &Connection, id: &str) -> AppResult<Option<String>> {
    if let Some(k) = c
        .prepare_cached("SELECT artist_key FROM stats_artist WHERE artist_id = ?1 ORDER BY plays DESC LIMIT 1")?
        .query_row([id], |r| r.get::<_, String>(0))
        .optional()?
    {
        return Ok(Some(k));
    }
    Ok(c.prepare_cached("SELECT lower(title) FROM artists WHERE id = ?1")?.query_row([id], |r| r.get(0)).optional()?)
}

fn album_key_for(c: &Connection, id: &str) -> AppResult<Option<String>> {
    if let Some(k) = c
        .prepare_cached("SELECT album_key FROM stats_album WHERE album_id = ?1 ORDER BY plays DESC LIMIT 1")?
        .query_row([id], |r| r.get::<_, String>(0))
        .optional()?
    {
        return Ok(Some(k));
    }
    Ok(c.prepare_cached("SELECT lower(artist_title) || '|' || lower(title) FROM albums WHERE id = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?)
}

pub fn item(c: &Connection, kind: ItemKind, id: &str, today: i64) -> AppResult<ItemStats> {
    let (dims, day_table, key_col, key) = match kind {
        ItemKind::Song => ("stats_song", "stats_song_day", "song_key", Some(format!("k:{id}"))),
        ItemKind::Artist => ("stats_artist", "stats_artist_day", "artist_key", artist_key_for(c, id)?),
        ItemKind::Album => ("stats_album", "stats_album_day", "album_key", album_key_for(c, id)?),
    };
    let (ty, tm, _) = civil_from_days(today);
    let mut months: Vec<(i64, u32)> = vec![(ty, tm)];
    for _ in 1..ITEM_SERIES_MONTHS {
        let (y, m) = *months.last().unwrap_or(&(ty, tm));
        months.push(prev_month(y, m));
    }
    months.reverse();
    let mut series: Vec<Point> = months.iter().map(|(y, m)| Point { key: format!("{y:04}-{m:02}"), plays: 0, ms: 0 }).collect();
    let empty = ItemStats { kind, plays: 0, ms: 0, first: None, last_at: None, rank: None, series: series.clone() };
    let Some(key) = key else { return Ok(empty) };
    let row: Option<(i64, i64, Option<i64>)> = c
        .prepare_cached(&format!("SELECT plays, ms, last_at FROM {dims} WHERE {key_col} = ?1"))?
        .query_row([&key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    let Some((plays, ms, last_at)) = row.filter(|r| r.0 > 0) else { return Ok(empty) };
    let rank: i64 = c
        .prepare_cached(&format!("SELECT count(*) + 1 FROM {dims} WHERE plays > ?1 OR (plays = ?1 AND ms > ?2)"))?
        .query_row(params![plays, ms], |r| r.get(0))?;
    let first_song: Option<String> = match kind {
        ItemKind::Song => Some(key.clone()),
        ItemKind::Artist => c
            .prepare_cached("SELECT song_key FROM stats_song WHERE artist_key = ?1 AND plays > 0 ORDER BY first_at LIMIT 1")?
            .query_row([&key], |r| r.get(0))
            .optional()?,
        ItemKind::Album => c
            .prepare_cached("SELECT song_key FROM stats_song WHERE album_key = ?1 AND plays > 0 ORDER BY first_at LIMIT 1")?
            .query_row([&key], |r| r.get(0))
            .optional()?,
    };
    let first = match first_song {
        Some(k) => first_listen(c, &k)?,
        None => None,
    };
    let lo = days_from_civil(months[0].0, months[0].1, 1);
    let mut st = c.prepare_cached(&format!("SELECT day, plays, ms FROM {day_table} WHERE {key_col} = ?1 AND day >= ?2"))?;
    for row in st.query_map(params![key, lo], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
        let (day, p, m) = row?;
        let (y, mo, _) = civil_from_days(day);
        let i = ((y - months[0].0) * 12 + mo as i64 - months[0].1 as i64) as usize;
        if let Some(pt) = series.get_mut(i) {
            pt.plays += p;
            pt.ms += m;
        }
    }
    Ok(ItemStats { kind, plays, ms, first, last_at, rank: Some(rank), series })
}

// ---------------------------------------------------------------- monthly recap

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthRecap {
    /// "YYYY-MM".
    pub month: String,
    pub totals: Totals,
    /// Listening time the month before, for the change.
    pub prev_ms: i64,
    pub top_songs: Vec<TopSong>,
    pub top_artists: Vec<TopArtist>,
    pub top_album: Option<TopAlbum>,
    /// The month's most played albums (up to [`RECAP_COVERS`]), for its covers.
    pub top_albums: Vec<TopAlbum>,
    pub top_genre: Option<String>,
    /// Listening time per day of the month.
    pub daily: Vec<Point>,
    /// The day with the most listening.
    pub biggest_day: Option<Point>,
}

pub const RECAP_TOP: u32 = 5;
/// Covers on a recap card.
pub const RECAP_COVERS: u32 = 4;

pub fn month_recap(c: &Connection, month: &str) -> AppResult<MonthRecap> {
    let (lo, hi) = parse_month(month)?;
    let d = Days { lo, hi, all: false };
    let (y, m, _) = civil_from_days(lo);
    let (py, pm) = prev_month(y, m);
    let (plo, phi) = month_bounds(py, pm);
    let prev_ms: i64 = c
        .prepare_cached("SELECT coalesce(sum(ms), 0) FROM stats_hour WHERE day BETWEEN ?1 AND ?2")?
        .query_row([plo, phi], |r| r.get(0))?;
    let days = daily(c, d)?;
    let mut totals = Totals {
        plays: days.iter().map(|x| x.1).sum(),
        ms: days.iter().map(|x| x.2).sum(),
        days: days.len() as i64,
        ..Default::default()
    };
    totals.songs = distinct(c, "stats_song_day", "song_key", "stats_song", d)?;
    totals.artists = distinct(c, "stats_artist_day", "artist_key", "stats_artist", d)?;
    totals.albums = distinct(c, "stats_album_day", "album_key", "stats_album", d)?;
    let (_, daily_pts) = series(&days, d);
    let biggest_day = daily_pts.iter().filter(|p| p.ms > 0).max_by_key(|p| p.ms).cloned();
    let top_albums = top_albums(c, d, RECAP_COVERS)?;
    Ok(MonthRecap {
        month: format!("{y:04}-{m:02}"),
        totals,
        prev_ms,
        top_songs: top_songs(c, d, RECAP_TOP)?,
        top_artists: top_artists(c, d, RECAP_TOP)?,
        top_album: top_albums.first().cloned(),
        top_albums,
        top_genre: top_genres(c, d, 1)?.pop().map(|g| g.name),
        daily: daily_pts,
        biggest_day,
    })
}

// ---------------------------------------------------------------- days

/// Plays and listening time for each day with plays from `lo` to `hi`
/// (the calendar), oldest first.
pub fn days(c: &Connection, lo: i64, hi: i64) -> AppResult<Vec<Point>> {
    Ok(daily(c, Days { lo, hi, all: false })?.into_iter().map(|(day, plays, ms)| Point { key: format_day(day), plays, ms }).collect())
}

// ---------------------------------------------------------------- moments

/// A song at a moment of the range.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomentSong {
    /// Its plays and minutes are the moment's own (that day).
    pub song: TopSong,
    /// "YYYY-MM-DD".
    pub day: String,
    /// Unix ms of the play, when it's one play.
    pub at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    /// Its plays and minutes are the range's.
    pub artist: TopArtist,
    /// The day you first played them.
    pub day: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comeback {
    pub song: MomentSong,
    /// Days since you'd played it before.
    pub gap_days: i64,
}

/// Small stories from a range: the first song, the day one song was played
/// most, the artist found in it that you played most, and a song you came
/// back to after a long break.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Moments {
    pub first_song: Option<MomentSong>,
    pub replayed: Option<MomentSong>,
    pub discovery: Option<Discovery>,
    pub comeback: Option<Comeback>,
    /// Artists first played in the range. None for all time.
    pub new_artists: Option<i64>,
}

/// A one-song day replays at least this often to be a moment.
pub const REPLAY_MIN: i64 = 3;
/// A break this many days or longer counts as coming back.
pub const COMEBACK_MIN_DAYS: i64 = 30;

fn one_song(c: &Connection, key: String, plays: i64, ms: i64) -> AppResult<Option<TopSong>> {
    Ok(songs_for(c, &[(key, plays, ms)])?.pop())
}

pub fn moments(c: &Connection, d: Days) -> AppResult<Moments> {
    let first: Option<(i64, String, i64, i64, i64)> = c
        .prepare_cached(
            "SELECT day, song_key, plays, ms, first_at FROM stats_song_day
             WHERE day = (SELECT min(day) FROM stats_hour WHERE day BETWEEN ?1 AND ?2)
             ORDER BY first_at, song_key LIMIT 1",
        )?
        .query_row([d.lo, d.hi], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .optional()?;
    let first_song = match first {
        Some((day, key, plays, ms, at)) => one_song(c, key, plays, ms)?.map(|song| MomentSong { song, day: format_day(day), at: Some(at) }),
        None => None,
    };

    let replay: Option<(i64, String, i64, i64)> = c
        .prepare_cached(
            "SELECT day, song_key, plays, ms FROM stats_song_day WHERE day BETWEEN ?1 AND ?2 AND plays >= ?3
             ORDER BY plays DESC, ms DESC, day DESC LIMIT 1",
        )?
        .query_row(params![d.lo, d.hi, REPLAY_MIN], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .optional()?;
    let replayed = match replay {
        Some((day, key, plays, ms)) => one_song(c, key, plays, ms)?.map(|song| MomentSong { song, day: format_day(day), at: None }),
        None => None,
    };

    // New artists: played in the range and never before it. All time has none.
    let (discovery, new_artists) = if d.all {
        (None, None)
    } else {
        let mut st = c.prepare_cached(
            "SELECT n.artist_key, sum(n.plays) AS p, sum(n.ms) AS m, min(n.day) AS f FROM stats_artist_day n
             WHERE n.day BETWEEN ?1 AND ?2
               AND NOT EXISTS (SELECT 1 FROM stats_artist_day o WHERE o.artist_key = n.artist_key AND o.day < ?1)
             GROUP BY n.artist_key ORDER BY p DESC, m DESC, n.artist_key",
        )?;
        let rows = st
            .query_map([d.lo, d.hi], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let found = match rows.first() {
            Some((k, p, m, f)) => artists_for(c, &[(k.clone(), *p, *m)])?.pop().map(|artist| Discovery { artist, day: format_day(*f) }),
            None => None,
        };
        (found, Some(rows.len() as i64))
    };

    // Coming back: the longest break before a day in the range, for songs
    // played in the range.
    let back: Option<(String, i64, i64, i64, i64)> = c
        .prepare_cached(
            "WITH played AS (SELECT DISTINCT song_key FROM stats_song_day WHERE day BETWEEN ?1 AND ?2),
             gaps AS (SELECT x.song_key, x.day, x.plays, x.ms,
                             x.day - lag(x.day) OVER (PARTITION BY x.song_key ORDER BY x.day) AS gap
                      FROM stats_song_day x WHERE x.song_key IN (SELECT song_key FROM played) AND x.day <= ?2)
             SELECT song_key, day, plays, ms, gap FROM gaps WHERE day >= ?1 AND gap >= ?3
             ORDER BY gap DESC, day DESC, song_key LIMIT 1",
        )?
        .query_row(params![d.lo, d.hi, COMEBACK_MIN_DAYS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .optional()?;
    let comeback = match back {
        Some((key, day, plays, ms, gap)) => {
            one_song(c, key, plays, ms)?.map(|song| Comeback { song: MomentSong { song, day: format_day(day), at: None }, gap_days: gap })
        }
        None => None,
    };
    Ok(Moments { first_song, replayed, discovery, comeback, new_artists })
}
