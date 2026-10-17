//! Home's history shelves. On this day: what you played on today's date in
//! earlier years, one group per year. Time travel: a mix per release decade
//! of the library, from the songs you play most, and a mix of the year you
//! listened most.
//!
//! Both read the stats day tables (see the module above), so they cover the
//! local listen log and imported plays, with a play both sides know about
//! counted once. Each costs a handful of indexed queries, and the commands
//! keep the answer for the rest of the day (until the counted plays change).

use std::collections::{BTreeMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::{Days, TopAlbum, TopKind, TopSong, albums_for, civil_from_days, data_span, days_from_civil, format_day, month_len, play_ids, songs_for, top_keys};
use crate::error::AppResult;

/// Most songs listed for one year's day.
pub const DAY_SONGS: u32 = 50;
/// Most albums listed for one year's day.
pub const DAY_ALBUMS: u32 = 10;
/// Songs in a decade or year mix.
pub const MIX_SONGS: usize = 50;
/// Covers kept for a card's collage.
const COVERS: usize = 4;
/// The earliest release year counted as real (0 and junk years are left out).
const FIRST_YEAR: i64 = 1900;

/// One earlier year's plays on today's date.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnThisDayYear {
    pub year: i64,
    /// The day it was, "YYYY-MM-DD" (Feb 28 of a non-leap year also takes
    /// in Feb 29 of a leap year; see [`matching_days`]).
    pub date: String,
    pub plays: i64,
    pub ms: i64,
    /// Most played first. Songs no longer in the library have no `trackId`.
    pub songs: Vec<TopSong>,
    pub albums: Vec<TopAlbum>,
    /// What the card plays: the library songs only, in the order above.
    pub track_ids: Vec<String>,
    pub covers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnThisDay {
    /// False while the setting is off.
    pub shown: bool,
    /// Today, "YYYY-MM-DD".
    pub date: String,
    /// The most played day first (ties: the later year).
    pub years: Vec<OnThisDayYear>,
}

/// A Time travel card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMix {
    /// "decade:1990" or "year:2021".
    pub id: String,
    /// "decade" or "year".
    pub kind: String,
    /// The decade's first year, or the year.
    pub year: i64,
    /// Plays the mix's songs have (a decade), or the year's plays (a year).
    pub plays: i64,
    pub track_ids: Vec<String>,
    pub covers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeTravel {
    /// False while the setting is off.
    pub shown: bool,
    /// The year with the most plays, when it has library songs to play.
    pub your_year: Option<HistoryMix>,
    /// Oldest decade first.
    pub decades: Vec<HistoryMix>,
}

/// What changes when plays are counted again: the day tables' size and total.
pub type Stamp = (i64, i64);

pub fn stamp(c: &Connection) -> AppResult<Stamp> {
    Ok(c.query_row("SELECT count(*), coalesce(sum(plays), 0) FROM stats_hour", [], |r| Ok((r.get(0)?, r.get(1)?)))?)
}

fn is_leap(y: i64) -> bool {
    month_len(y, 2) == 29
}

/// The days of `year` that match today's month and day `(m, d)` in
/// `this_year`. Feb 29 matches only Feb 29. Feb 28 also takes in Feb 29 of
/// a leap year when this year has no Feb 29, so those plays still come up
/// in the years between.
pub fn matching_days(m: u32, d: u32, this_year: i64, year: i64) -> Vec<i64> {
    if m == 2 && d == 29 {
        return if is_leap(year) { vec![days_from_civil(year, 2, 29)] } else { Vec::new() };
    }
    let mut out = vec![days_from_civil(year, m, d)];
    if m == 2 && d == 28 && !is_leap(this_year) && is_leap(year) {
        out.push(days_from_civil(year, 2, 29));
    }
    out
}

/// Album covers for songs, distinct, in order, at most [`COVERS`].
fn covers_for(c: &Connection, ids: &[String]) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached("SELECT coalesce(a.thumb, t.thumb) FROM tracks t LEFT JOIN albums a ON a.id = t.album_id WHERE t.id = ?1")?;
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if out.len() >= COVERS {
            break;
        }
        let thumb: Option<String> = st.query_row([id], |r| r.get(0)).optional()?.flatten();
        if let Some(t) = thumb.filter(|t| !out.contains(t)) {
            out.push(t);
        }
    }
    Ok(out)
}

/// Today's date in every earlier year with plays on it.
pub fn on_this_day(c: &Connection, today: i64) -> AppResult<OnThisDay> {
    let (this_year, m, d) = civil_from_days(today);
    let mut out = OnThisDay { shown: true, date: format_day(today), years: Vec::new() };
    let Some((first, _)) = data_span(c)? else { return Ok(out) };
    let mut totals = c.prepare_cached("SELECT coalesce(sum(plays), 0), coalesce(sum(ms), 0) FROM stats_hour WHERE day BETWEEN ?1 AND ?2")?;
    for year in civil_from_days(first).0..this_year {
        let days = matching_days(m, d, this_year, year);
        let (Some(&lo), Some(&hi)) = (days.iter().min(), days.iter().max()) else { continue };
        let (plays, ms): (i64, i64) = totals.query_row([lo, hi], |r| Ok((r.get(0)?, r.get(1)?)))?;
        if plays == 0 {
            continue;
        }
        let range = Days { lo, hi, all: false };
        let songs = songs_for(c, &top_keys(c, "stats_song_day", "song_key", "stats_song", range, DAY_SONGS)?)?;
        if songs.is_empty() {
            continue;
        }
        let albums = albums_for(c, &top_keys(c, "stats_album_day", "album_key", "stats_album", range, DAY_ALBUMS)?)?;
        let track_ids: Vec<String> = songs.iter().filter_map(|s| s.track_id.clone()).collect();
        let mut covers: Vec<String> = albums.iter().filter(|a| a.album_id.is_some()).filter_map(|a| a.thumb.clone()).collect();
        covers.dedup();
        for t in covers_for(c, &track_ids)? {
            if !covers.contains(&t) {
                covers.push(t);
            }
        }
        covers.truncate(COVERS);
        out.years.push(OnThisDayYear { year, date: format_day(lo), plays, ms, songs, albums, track_ids, covers });
    }
    out.years.sort_by(|a, b| b.plays.cmp(&a.plays).then(b.ms.cmp(&a.ms)).then(b.year.cmp(&a.year)));
    Ok(out)
}

/// The decades of the library's release years, and the year listened most.
pub fn time_travel(c: &Connection, today: i64) -> AppResult<TimeTravel> {
    let this_year = civil_from_days(today).0;
    let mut decades: BTreeMap<i64, (Vec<String>, i64)> = BTreeMap::new();
    let mut st = c.prepare_cached("SELECT DISTINCT (year / 10) * 10 FROM albums WHERE year BETWEEN ?1 AND ?2")?;
    for dec in st.query_map([FIRST_YEAR, this_year], |r| r.get::<_, i64>(0))? {
        decades.insert(dec?, (Vec::new(), 0));
    }
    // Your most played songs, each into its release decade until that is full.
    let mut st = c.prepare_cached(
        "SELECT t.id, s.plays, a.year FROM stats_song s
         JOIN tracks t ON t.id = s.rating_key JOIN albums a ON a.id = t.album_id
         WHERE s.plays > 0 AND a.year BETWEEN ?1 AND ?2
         ORDER BY s.plays DESC, s.ms DESC, s.song_key",
    )?;
    let mut rows = st.query(params![FIRST_YEAR, this_year])?;
    while let Some(r) = rows.next()? {
        let (id, plays, year): (String, i64, i64) = (r.get(0)?, r.get(1)?, r.get(2)?);
        let slot = decades.entry(year / 10 * 10).or_default();
        if slot.0.len() < MIX_SONGS {
            slot.0.push(id);
            slot.1 += plays;
        }
    }
    // A decade with few plays fills up with its songs played most on the server.
    let mut fill = c.prepare_cached(
        "SELECT t.id FROM albums a JOIN tracks t ON t.album_id = a.id
         WHERE a.year BETWEEN ?1 AND ?2 ORDER BY t.view_count DESC, t.id LIMIT ?3",
    )?;
    let mut mixes = Vec::new();
    for (dec, (mut ids, plays)) in decades {
        if ids.len() < MIX_SONGS {
            let have: HashSet<String> = ids.iter().cloned().collect();
            let more = fill.query_map(params![dec, dec + 9, (MIX_SONGS * 2) as i64], |r| r.get::<_, String>(0))?;
            for id in more {
                let id = id?;
                if ids.len() >= MIX_SONGS {
                    break;
                }
                if !have.contains(&id) {
                    ids.push(id);
                }
            }
        }
        if ids.is_empty() {
            continue;
        }
        let covers = covers_for(c, &ids)?;
        mixes.push(HistoryMix { id: format!("decade:{dec}"), kind: "decade".into(), year: dec, plays, track_ids: ids, covers });
    }
    Ok(TimeTravel { shown: true, your_year: your_year(c)?, decades: mixes })
}

/// The calendar year with the most plays (ties: the later one), as a mix of
/// its most played library songs.
pub fn your_year(c: &Connection) -> AppResult<Option<HistoryMix>> {
    let mut by_year: BTreeMap<i64, i64> = BTreeMap::new();
    let mut st = c.prepare_cached("SELECT day, sum(plays) FROM stats_hour GROUP BY day")?;
    for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (day, plays) = row?;
        *by_year.entry(civil_from_days(day).0).or_default() += plays;
    }
    let Some((&year, &plays)) = by_year.iter().filter(|(_, p)| **p > 0).max_by_key(|(y, p)| (**p, **y)) else { return Ok(None) };
    let range = Days { lo: days_from_civil(year, 1, 1), hi: days_from_civil(year, 12, 31), all: false };
    let ids = play_ids(c, range, TopKind::Songs, MIX_SONGS as u32)?;
    if ids.is_empty() {
        return Ok(None);
    }
    let covers = covers_for(c, &ids)?;
    Ok(Some(HistoryMix { id: format!("year:{year}"), kind: "year".into(), year, plays, track_ids: ids, covers }))
}
