//! Wrapped (a year in story cards) and the Top Songs lists: one list per
//! year with enough plays, and one for all time.
//!
//! Both read the per-day tables like the rest of stats, so a year is the
//! local calendar year: a play at 23:59 on December 31 counts in that year
//! wherever the computer is. Songs are keyed by the file played, so each
//! version of a song is counted (and credited) on its own. Private-session
//! plays never reach the listen log, so they never count here either.
//! Unreleased songs count and are marked; what may leave the computer is
//! decided where things are exported (the summary card).

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::{
    Days, MomentSong, Point, REPLAY_MIN, TopAlbum, TopArtist, TopGenre, TopSong, Totals, civil_from_days, daily,
    days_from_civil, distinct, format_day, songs_for, top_albums, top_artists, top_genres, top_songs,
};
use crate::error::{AppError, AppResult};
use crate::model::Track;

/// Songs on a Top Songs list.
pub const TOP_SONGS_LEN: u32 = 100;
/// A year needs this many plays for its own list.
pub const YEAR_LIST_MIN_PLAYS: i64 = 30;
/// Artists, songs, albums and genres on Wrapped's cards.
pub const WRAPPED_TOP: u32 = 5;
/// A song played at least this often in the year is one of its repeats.
pub const REPEAT_PLAYS: i64 = 5;
/// Home shows the newest list until this day of January.
pub const PROMOTE_UNTIL_JANUARY: u32 = 15;

/// A calendar year as local day numbers.
pub fn year_days(year: i64) -> Days {
    Days { lo: days_from_civil(year, 1, 1), hi: days_from_civil(year, 12, 31), all: false }
}

/// Whether a year is over enough to show: past years always, this one
/// from December 1.
pub fn year_open(year: i64, today: i64) -> bool {
    let (ty, tm, _) = civil_from_days(today);
    year < ty || (year == ty && tm == 12)
}

/// The year whose list Home shows: this year's through December, last
/// year's until January 15, otherwise none.
pub fn promoted_year(today: i64) -> Option<i64> {
    match civil_from_days(today) {
        (y, 12, _) => Some(y),
        (y, 1, d) if d <= PROMOTE_UNTIL_JANUARY => Some(y - 1),
        _ => None,
    }
}

/// Plays and listening time per year with plays, oldest first.
pub fn years(c: &Connection) -> AppResult<Vec<(i64, i64, i64)>> {
    let mut st = c.prepare_cached("SELECT day, sum(plays), sum(ms) FROM stats_hour GROUP BY day ORDER BY day")?;
    let mut out: Vec<(i64, i64, i64)> = Vec::new();
    for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
        let (day, p, ms) = row?;
        let (y, _, _) = civil_from_days(day);
        match out.last_mut() {
            Some(last) if last.0 == y => {
                last.1 += p;
                last.2 += ms;
            }
            _ => out.push((y, p, ms)),
        }
    }
    Ok(out)
}

/// What exists: the years with a Top Songs list and the years Wrapped
/// covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopSongsIndex {
    /// Years with a list (open and with enough plays), newest first.
    pub years: Vec<i64>,
    /// Whether there are any plays at all (the All-Time list).
    pub all_time: bool,
    /// The year Home shows right now, when there is one.
    pub promoted: Option<i64>,
    /// Years with plays that Wrapped shows on the profile, newest first.
    pub wrapped_years: Vec<i64>,
    pub current_year: i64,
}

pub fn index(c: &Connection, today: i64) -> AppResult<TopSongsIndex> {
    let ys = years(c)?;
    let mut lists: Vec<i64> = ys.iter().filter(|y| y.1 >= YEAR_LIST_MIN_PLAYS && year_open(y.0, today)).map(|y| y.0).collect();
    lists.reverse();
    let mut wrapped: Vec<i64> = ys.iter().filter(|y| y.1 > 0 && year_open(y.0, today)).map(|y| y.0).collect();
    wrapped.reverse();
    let promoted = promoted_year(today).filter(|y| lists.contains(y));
    Ok(TopSongsIndex { years: lists, all_time: ys.iter().any(|y| y.1 > 0), promoted, wrapped_years: wrapped, current_year: civil_from_days(today).0 })
}

// ---------------------------------------------------------------- lists

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopSongsRow {
    pub track: Track,
    pub plays: i64,
    pub ms: i64,
    pub unreleased: bool,
    /// The year's first play (yearly lists only).
    pub first_of_year: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopSongsList {
    /// None for All-Time.
    pub year: Option<i64>,
    pub title: String,
    /// Most played first; the position is the rank.
    pub rows: Vec<TopSongsRow>,
    /// The songs' length together.
    pub duration_ms: i64,
}

pub fn list_title(year: Option<i64>) -> String {
    match year {
        Some(y) => format!("Your Top Songs {y}"),
        None => "All-Time Top Songs".into(),
    }
}

/// The most played library songs in a range: plays, then minutes, then key.
fn library_keys(c: &Connection, d: Days, limit: u32) -> AppResult<Vec<(String, i64, i64)>> {
    let sql = if d.all {
        "SELECT s.song_key, s.plays, s.ms FROM stats_song s JOIN tracks t ON t.id = s.rating_key
         WHERE s.plays > 0 ORDER BY s.plays DESC, s.ms DESC, s.song_key LIMIT ?3"
    } else {
        "SELECT d.song_key, sum(d.plays) AS p, sum(d.ms) AS m FROM stats_song_day d
         JOIN stats_song s ON s.song_key = d.song_key JOIN tracks t ON t.id = s.rating_key
         WHERE d.day BETWEEN ?1 AND ?2 GROUP BY d.song_key ORDER BY p DESC, m DESC, d.song_key LIMIT ?3"
    };
    let mut st = c.prepare_cached(sql)?;
    let (lo, hi) = if d.all { (0, 0) } else { (d.lo, d.hi) };
    let rows = st.query_map(params![lo, hi, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// The first play of a range: its day, song key, that day's plays and
/// minutes of the song, and when it started (unix ms).
type FirstPlay = (i64, String, i64, i64, i64);

fn first_song(c: &Connection, d: Days) -> AppResult<Option<FirstPlay>> {
    Ok(c.prepare_cached(
        "SELECT day, song_key, plays, ms, first_at FROM stats_song_day
         WHERE day = (SELECT min(day) FROM stats_hour WHERE day BETWEEN ?1 AND ?2)
         ORDER BY first_at, song_key LIMIT 1",
    )?
    .query_row([d.lo, d.hi], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
    .optional()?)
}

/// A year's list (None: All-Time). A year without a list is not found.
pub fn list(c: &Connection, year: Option<i64>, today: i64) -> AppResult<TopSongsList> {
    let d = match year {
        Some(y) => {
            let plays = years(c)?.into_iter().find(|x| x.0 == y).map(|x| x.1).unwrap_or(0);
            if plays < YEAR_LIST_MIN_PLAYS || !year_open(y, today) {
                return Err(AppError::NotFound(format!("Top Songs {y}")));
            }
            year_days(y)
        }
        None => Days { lo: 0, hi: 0, all: true },
    };
    let first = match year {
        Some(_) => first_song(c, d)?.map(|f| f.1),
        None => None,
    };
    let keys = library_keys(c, d, TOP_SONGS_LEN)?;
    let rows: Vec<TopSongsRow> = songs_for(c, &keys)?
        .into_iter()
        .filter_map(|s| {
            let first_of_year = first.as_deref() == Some(s.key.as_str());
            s.track.map(|track| TopSongsRow { track, plays: s.plays, ms: s.ms, unreleased: s.unreleased, first_of_year })
        })
        .collect();
    let duration_ms = rows.iter().map(|r| r.track.duration_ms as i64).sum();
    Ok(TopSongsList { year, title: list_title(year), rows, duration_ms })
}

// ---------------------------------------------------------------- personality

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeOfDay {
    /// 5 AM to noon.
    Morning,
    /// Noon to 5 PM.
    Afternoon,
    /// 5 PM to 10 PM.
    Evening,
    /// 10 PM to 5 AM.
    Night,
}

pub fn time_of_day(hour: usize) -> TimeOfDay {
    match hour {
        5..=11 => TimeOfDay::Morning,
        12..=16 => TimeOfDay::Afternoon,
        17..=21 => TimeOfDay::Evening,
        _ => TimeOfDay::Night,
    }
}

/// How the year was listened to. Shares are 0 to 1.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Personality {
    /// The part of the day with the most listening, and its share of the minutes.
    pub time: TimeOfDay,
    pub time_share: f64,
    /// Share of plays that went to songs played [`REPEAT_PLAYS`] times or more.
    pub repeat_share: f64,
    /// Half or more of the plays were repeats; otherwise deep cuts.
    pub repeats: bool,
    /// Share of plays that went to songs first heard this year.
    pub new_share: f64,
    /// Half or more of the plays were new finds; otherwise old favourites.
    pub mostly_new: bool,
}

/// The personality from listening time per hour (0 to 23) and plays.
/// Ties between parts of the day go to the earlier part.
pub fn personality(hours: &[i64], plays: i64, repeat_plays: i64, new_plays: i64) -> Option<Personality> {
    let total: i64 = hours.iter().sum();
    if total <= 0 || plays <= 0 {
        return None;
    }
    let mut parts = [0i64; 4];
    for (h, ms) in hours.iter().enumerate() {
        parts[time_of_day(h) as usize] += ms;
    }
    let order = [TimeOfDay::Morning, TimeOfDay::Afternoon, TimeOfDay::Evening, TimeOfDay::Night];
    let (i, ms) = parts.iter().enumerate().fold((0, 0), |best, (i, ms)| if *ms > best.1 { (i, *ms) } else { best });
    let share = |n: i64| (n.clamp(0, plays) as f64) / plays as f64;
    let repeat_share = share(repeat_plays);
    let new_share = share(new_plays);
    Some(Personality {
        time: order[i],
        time_share: ms as f64 / total as f64,
        repeat_share,
        repeats: repeat_share >= 0.5,
        new_share,
        mostly_new: new_share >= 0.5,
    })
}

// ---------------------------------------------------------------- wrapped

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Wrapped {
    pub year: i64,
    pub totals: Totals,
    /// Listening time the year before (0 when there was none).
    pub prev_ms: i64,
    pub top_artists: Vec<TopArtist>,
    pub top_songs: Vec<TopSong>,
    pub top_albums: Vec<TopAlbum>,
    pub top_genres: Vec<TopGenre>,
    pub personality: Option<Personality>,
    /// Listening time per hour of the day.
    pub hours: Vec<i64>,
    pub biggest_day: Option<Point>,
    /// The song played most in one day (at least [`REPLAY_MIN`] times); its
    /// plays are that day's.
    pub replayed: Option<MomentSong>,
    pub first_song: Option<MomentSong>,
    /// Artists first played this year.
    pub new_artists: i64,
}

/// One number from a query (0 when it's NULL).
fn one(c: &Connection, sql: &str, p: impl rusqlite::Params) -> AppResult<i64> {
    Ok(c.prepare_cached(sql)?.query_row(p, |r| r.get::<_, Option<i64>>(0))?.unwrap_or(0))
}

pub fn wrapped(c: &Connection, year: i64) -> AppResult<Wrapped> {
    if !(1900..=9999).contains(&year) {
        return Err(AppError::Invalid(format!("Not a year: {year}")));
    }
    let d = year_days(year);
    let days = daily(c, d)?;
    let mut totals = Totals { plays: days.iter().map(|x| x.1).sum(), ms: days.iter().map(|x| x.2).sum(), days: days.len() as i64, ..Default::default() };
    totals.songs = distinct(c, "stats_song_day", "song_key", "stats_song", d)?;
    totals.artists = super::distinct_artists(c, d)?;
    totals.albums = distinct(c, "stats_album_day", "album_key", "stats_album", d)?;
    let p = year_days(year - 1);
    let prev_ms = one(c, "SELECT sum(ms) FROM stats_hour WHERE day BETWEEN ?1 AND ?2", [p.lo, p.hi])?;

    let mut hours = vec![0i64; 24];
    let mut st = c.prepare_cached("SELECT hour, sum(ms) FROM stats_hour WHERE day BETWEEN ?1 AND ?2 GROUP BY hour")?;
    for row in st.query_map([d.lo, d.hi], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (h, ms) = row?;
        if let Some(x) = hours.get_mut(h as usize) {
            *x = ms;
        }
    }
    let repeat_plays = one(
        c,
        "SELECT sum(p) FROM (SELECT sum(plays) AS p FROM stats_song_day WHERE day BETWEEN ?1 AND ?2 GROUP BY song_key) WHERE p >= ?3",
        params![d.lo, d.hi, REPEAT_PLAYS],
    )?;
    let new_plays = one(
        c,
        "SELECT sum(n.p) FROM (SELECT song_key, sum(plays) AS p FROM stats_song_day WHERE day BETWEEN ?1 AND ?2 GROUP BY song_key) n
         WHERE NOT EXISTS (SELECT 1 FROM stats_song_day o WHERE o.song_key = n.song_key AND o.day < ?1)",
        [d.lo, d.hi],
    )?;

    let biggest_day = days.iter().max_by_key(|x| (x.2, std::cmp::Reverse(x.0))).map(|&(day, plays, ms)| Point { key: format_day(day), plays, ms });

    let replay: Option<(i64, String, i64, i64)> = c
        .prepare_cached(
            "SELECT day, song_key, plays, ms FROM stats_song_day WHERE day BETWEEN ?1 AND ?2 AND plays >= ?3
             ORDER BY plays DESC, ms DESC, day DESC LIMIT 1",
        )?
        .query_row(params![d.lo, d.hi, REPLAY_MIN], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .optional()?;
    let replayed = match replay {
        Some((day, key, plays, ms)) => songs_for(c, &[(key, plays, ms)])?.pop().map(|song| MomentSong { song, day: format_day(day), at: None }),
        None => None,
    };
    let first_song = match first_song(c, d)? {
        Some((day, key, plays, ms, at)) => songs_for(c, &[(key, plays, ms)])?.pop().map(|song| MomentSong { song, day: format_day(day), at: Some(at) }),
        None => None,
    };
    let new_artists = super::aliases::new_artists(c, d, &super::aliases::groups(c)?)?.len() as i64;
    Ok(Wrapped {
        year,
        prev_ms,
        top_artists: top_artists(c, d, WRAPPED_TOP)?,
        top_songs: top_songs(c, d, WRAPPED_TOP)?,
        top_albums: top_albums(c, d, WRAPPED_TOP)?,
        top_genres: top_genres(c, d, WRAPPED_TOP)?,
        personality: personality(&hours, totals.plays, repeat_plays, new_plays),
        hours,
        biggest_day,
        replayed,
        first_song,
        new_artists,
        totals,
    })
}
