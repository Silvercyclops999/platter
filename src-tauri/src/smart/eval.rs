//! Smart playlist rules evaluated on this computer, from the library cache
//! (and the local listen log for "played on this computer"). Used for the
//! editor's preview count and for Platter smart playlists.

use std::cmp::Ordering;
use std::collections::HashMap;

use rusqlite::Connection;

use super::rules::{Day, Field, Kind, LimitUnit, Op, Rule, SmartRules, SortBy, parse_number_range, parse_range, parse_span};
use crate::error::AppResult;

/// One song as the rules see it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub artist_id: String,
    pub artist: String,
    pub track_artist: Option<String>,
    pub album: String,
    pub disc: u32,
    pub track: u32,
    pub year: Option<i32>,
    /// The album's, unix seconds.
    pub added_at: i64,
    pub plays: u32,
    /// Unix seconds.
    pub last_played: Option<i64>,
    /// Newest play in the local listen log, unix seconds.
    pub played_here: Option<i64>,
    /// Plex rating, 0 to 10.
    pub rating: Option<f64>,
    pub duration_ms: u64,
    pub section: String,
    pub genres: Vec<String>,
    pub styles: Vec<String>,
    pub moods: Vec<String>,
    pub explicit: bool,
    pub version: Option<String>,
    /// Demos and archive versions (never sent to an AI provider).
    pub unreleased: bool,
    pub album_id: String,
    /// The file tags' era.
    pub era: Option<String>,
    pub lossless: bool,
    /// Times skipped in the listen history.
    pub skips: u32,
    /// Every play in the listen history (skips left out), unix seconds.
    pub plays_at: Vec<i64>,
    /// Every song on the album has been played.
    pub album_finished: bool,
    /// Newest skip in the listen history, unix seconds.
    pub last_skipped: Option<i64>,
    /// The playlists it's on, by name (folded).
    pub playlists: Vec<String>,
    /// Hz and bits, from the file details.
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u32>,
}

/// FLAC, ALAC, WAV and the other lossless codecs Plex reports.
pub fn lossless_codec(codec: Option<&str>) -> bool {
    let c = codec.unwrap_or("").to_ascii_lowercase();
    matches!(c.as_str(), "flac" | "alac" | "wav" | "pcm" | "aiff" | "ape" | "wavpack" | "wv" | "tta" | "dsd")
}

fn json_vec(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

/// Every song in the cache.
pub fn read_rows(c: &Connection) -> AppResult<Vec<Row>> {
    // The listen log keeps unix ms; rules compare seconds.
    let mut here: HashMap<String, i64> = HashMap::new();
    {
        let mut st = c.prepare("SELECT rating_key, max(started_at) FROM listen_log GROUP BY rating_key")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for r in rows {
            let (k, at) = r?;
            here.insert(k, at / 1000);
        }
    }
    let mut history: HashMap<String, (Vec<i64>, u32, Option<i64>)> = HashMap::new();
    {
        let sql = format!("WITH plays AS ({}) SELECT rating_key, played_at, skipped FROM plays WHERE rating_key IS NOT NULL", crate::db::listens::ALL_PLAYS);
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
        for r in rows {
            let (k, at, skipped) = r?;
            let e = history.entry(k).or_default();
            if skipped != 0 {
                e.1 += 1;
                e.2 = e.2.max(Some(at / 1000));
            } else {
                e.0.push(at / 1000);
            }
        }
    }
    let mut lists: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut st = c.prepare("SELECT DISTINCT i.track_id, p.title FROM playlist_items i JOIN playlists p ON p.id = i.playlist_id")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for r in rows {
            let (k, title) = r?;
            lists.entry(k).or_default().push(fold(&title));
        }
    }
    let mut st = c.prepare(
        "SELECT t.id, t.artist_id, t.artist_title, t.track_artist, t.album_title, t.disc_number, t.track_number,
                al.year, coalesce(al.added_at, 0), t.view_count, t.last_viewed_at, t.user_rating, t.duration_ms,
                coalesce(al.section, 'albums'), coalesce(al.genres, '[]'), coalesce(al.styles, '[]'),
                coalesce(al.moods, '[]'), t.genres, coalesce(f.explicit, (SELECT x.explicit FROM explicit_found x WHERE x.track_id = t.id)), f.version, al.format, coalesce(al.subformats, '[]'), t.title, al.studio, al.matched, mc.found, t.album_id,
                f.era, t.codec, coalesce(f.version, f.release, CASE WHEN at.version IS NULL THEN at.release END),
                t.sample_rate, t.bit_depth
         FROM tracks t
         LEFT JOIN albums al ON al.id = t.album_id
         LEFT JOIN album_mb_check mc ON mc.album_id = t.album_id
         LEFT JOIN album_tags at ON at.album_id = t.album_id
         LEFT JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'",
    )?;
    let mut rows = st.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        let track_tags = json_vec(&r.get::<_, String>(17)?);
        let mut genres = json_vec(&r.get::<_, String>(14)?);
        let mut moods = json_vec(&r.get::<_, String>(16)?);
        // A song's own tags are genres or moods; both lists get them.
        genres.extend(track_tags.iter().cloned());
        moods.extend(track_tags);
        let version: Option<String> = r.get(19)?;
        let format: Option<String> = r.get(20)?;
        let subformats = json_vec(&r.get::<_, String>(21)?);
        let studio: Option<String> = r.get(23)?;
        let matched: Option<bool> = r.get(24)?;
        let mb: Option<bool> = r.get(25)?;
        let album_id: String = r.get(26)?;
        // The rule reads the file's status tags too; the Version field stays ARCHIVE_VERSION.
        let status_version: Option<String> = r.get(29)?;
        let unreleased = crate::db::release::track_unreleased(
            &id,
            &album_id,
            format.as_deref(),
            &subformats,
            matched,
            studio.as_deref(),
            status_version.as_deref(),
            mb,
        );
        let era: Option<String> = r.get(27)?;
        let codec: Option<String> = r.get(28)?;
        let (plays_at, skips, last_skipped) = history.remove(&id).unwrap_or_default();
        out.push(Row {
            played_here: here.get(&id).copied(),
            album_id,
            era: era.filter(|e| !e.trim().is_empty()),
            lossless: lossless_codec(codec.as_deref()),
            skips,
            plays_at,
            album_finished: false,
            last_skipped,
            playlists: lists.remove(&id).unwrap_or_default(),
            sample_rate: r.get(30)?,
            bit_depth: r.get(31)?,
            id,
            title: r.get(22)?,
            artist_id: r.get(1)?,
            artist: r.get(2)?,
            track_artist: r.get(3)?,
            album: r.get(4)?,
            disc: r.get(5)?,
            track: r.get(6)?,
            year: r.get(7)?,
            added_at: r.get(8)?,
            plays: r.get(9)?,
            last_played: r.get(10)?,
            rating: r.get(11)?,
            duration_ms: r.get::<_, i64>(12)?.max(0) as u64,
            section: r.get(13)?,
            genres,
            styles: json_vec(&r.get::<_, String>(15)?),
            moods,
            explicit: r.get::<_, Option<i64>>(18)? == Some(1),
            version,
            unreleased,
        });
    }
    mark_finished(&mut out);
    Ok(out)
}

/// Every playlist's name (for rules that name one).
pub fn playlist_names(c: &Connection) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached("SELECT title FROM playlists")?;
    let names = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(names)
}

/// Albums whose every song has a play.
pub fn mark_finished(rows: &mut [Row]) {
    let mut open: HashMap<String, bool> = HashMap::new();
    for r in rows.iter() {
        let played = r.plays > 0 || !r.plays_at.is_empty();
        let e = open.entry(r.album_id.clone()).or_insert(true);
        *e = *e && played;
    }
    for r in rows.iter_mut() {
        r.album_finished = open.get(&r.album_id).copied().unwrap_or(false);
    }
}

/// Unix seconds when a local calendar day starts. `tz_min`: minutes east of UTC.
pub fn day_start(d: Day, tz_min: i32) -> i64 {
    crate::stats::days_from_civil(d.0 as i64, d.1, d.2) * 86_400 - tz_min as i64 * 60
}

fn next_day(d: Day) -> Day {
    let (y, m, dd) = crate::stats::civil_from_days(crate::stats::days_from_civil(d.0 as i64, d.1, d.2) + 1);
    (y as i32, m, dd)
}

fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

fn text(op: Op, have: &[&str], want: &str) -> bool {
    let want = fold(want);
    let hit = |f: &dyn Fn(&str) -> bool| have.iter().any(|h| f(&fold(h)));
    match op {
        Op::Is => hit(&|h| h == want),
        Op::IsNot => !hit(&|h| h == want),
        Op::Contains => hit(&|h| h.contains(&want)),
        Op::NotContains => !hit(&|h| h.contains(&want)),
        Op::BeginsWith => hit(&|h| h.starts_with(&want)),
        Op::EndsWith => hit(&|h| h.ends_with(&want)),
        _ => false,
    }
}

/// A number rule. A song without the number (no year) matches only "is not".
/// `range` is the "between" value.
fn number(op: Op, have: Option<f64>, want: f64, range: &str) -> bool {
    let Some(h) = have else { return op == Op::IsNot };
    if op == Op::Between {
        return parse_number_range(range).is_some_and(|(a, b)| h >= a - 1e-9 && h <= b + 1e-9);
    }
    match op {
        Op::Is => (h - want).abs() < 1e-9,
        Op::IsNot => (h - want).abs() >= 1e-9,
        Op::Gt => h > want,
        Op::Lt => h < want,
        _ => false,
    }
}

fn date(op: Op, at: Option<i64>, span: i64, now: i64) -> bool {
    let within = at.is_some_and(|t| t >= now - span);
    match op {
        Op::InLast => within,
        Op::NotInLast => !within,
        _ => false,
    }
}

/// Whether a song matches one rule. `liked_min` is the Plex rating that
/// counts as liked.
pub fn matches(r: &Rule, row: &Row, now: i64, liked_min: f64) -> bool {
    matches_tz(r, row, now, liked_min, 0)
}

/// [`matches`] with the time zone date ranges are read in (minutes east of UTC).
pub fn matches_tz(r: &Rule, row: &Row, now: i64, liked_min: f64, tz_min: i32) -> bool {
    let v = r.value.trim();
    match r.field.kind() {
        Kind::Tag => {
            let list = match r.field {
                Field::Genre => &row.genres,
                Field::Style => &row.styles,
                _ => &row.moods,
            };
            let refs: Vec<&str> = list.iter().map(String::as_str).collect();
            text(r.op, &refs, v)
        }
        Kind::Text => match r.field {
            Field::Artist => {
                let mut have = vec![row.artist.as_str()];
                if let Some(t) = row.track_artist.as_deref() {
                    have.push(t);
                }
                text(r.op, &have, v)
            }
            Field::Album => text(r.op, &[row.album.as_str()], v),
            Field::Title => text(r.op, &[row.title.as_str()], v),
            Field::Era => text(r.op, &row.era.as_deref().into_iter().collect::<Vec<_>>(), v),
            _ => text(r.op, &row.version.as_deref().into_iter().collect::<Vec<_>>(), v),
        },
        Kind::Number => {
            let want: f64 = v.parse().unwrap_or(0.0);
            let have = match r.field {
                Field::Year => row.year.map(f64::from),
                Field::PlayCount => Some(row.plays as f64),
                // Stars against Plex's 0 to 10.
                Field::Rating => Some(row.rating.unwrap_or(0.0) / 2.0),
                Field::Skips => Some(row.skips as f64),
                Field::TrackNumber => Some(row.track as f64).filter(|n| *n > 0.0),
                // kHz.
                Field::SampleRate => row.sample_rate.filter(|n| *n > 0).map(|n| n as f64 / 1000.0),
                Field::BitDepth => row.bit_depth.filter(|n| *n > 0).map(f64::from),
                _ => Some(row.duration_ms as f64 / 60_000.0),
            };
            number(r.op, have, want, v)
        }
        Kind::Decade => {
            let want: i32 = v.parse().unwrap_or(0);
            let is = row.year.is_some_and(|y| y - y.rem_euclid(10) == want);
            if r.op == Op::Is { is } else { !is }
        }
        Kind::Date => {
            let times: Vec<i64> = match r.field {
                Field::AddedAt => Some(row.added_at).filter(|a| *a > 0).into_iter().collect(),
                Field::LastPlayed => row.last_played.into_iter().collect(),
                Field::Played => row.plays_at.clone(),
                Field::LastSkipped => row.last_skipped.into_iter().collect(),
                _ => row.played_here.into_iter().collect(),
            };
            if r.op == Op::Between {
                let Some((a, b)) = parse_range(v) else { return false };
                let (from, to) = (day_start(a, tz_min), day_start(next_day(b), tz_min));
                return times.iter().any(|t| (from..to).contains(t));
            }
            let span = parse_span(v).unwrap_or(0);
            date(r.op, times.iter().max().copied(), span, now)
        }
        Kind::Bool => {
            let want = v == "true";
            let is = match r.field {
                Field::Liked => row.rating.is_some_and(|x| x >= liked_min),
                Field::Unreleased => row.unreleased,
                Field::Lossless => row.lossless,
                Field::Finished => row.album_finished,
                _ => row.explicit,
            };
            is == want
        }
        Kind::Choice => {
            let is = row.section == v;
            if r.op == Op::Is { is } else { !is }
        }
        Kind::List => {
            let is = row.playlists.contains(&fold(v));
            if r.op == Op::Is { is } else { !is }
        }
    }
}

/// Whether a song matches the rules and groups together.
pub fn matches_all_tz(rules: &SmartRules, row: &Row, now: i64, liked_min: f64, tz_min: i32) -> bool {
    if rules.rules.is_empty() && rules.groups.is_empty() {
        return true;
    }
    let one = |r: &Rule| matches_tz(r, row, now, liked_min, tz_min);
    let group = |g: &super::rules::RuleGroup| if g.match_all { g.rules.iter().all(one) } else { g.rules.iter().any(one) };
    let mut each = rules.rules.iter().map(one).chain(rules.groups.iter().map(group));
    if rules.match_all { each.all(|b| b) } else { each.any(|b| b) }
}

/// A small stable hash, for random order that holds until the next update.
fn mix(seed: u64, s: &str) -> u64 {
    let mut h = seed ^ 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The songs, in order, after rules, sort and limit. `seed` decides a
/// random order.
pub fn evaluate<'a>(rows: &'a [Row], rules: &SmartRules, now: i64, liked_min: f64, seed: u64) -> Vec<&'a Row> {
    evaluate_tz(rows, rules, now, liked_min, seed, 0)
}

/// [`evaluate`] with the time zone date ranges are read in (minutes east of UTC).
pub fn evaluate_tz<'a>(rows: &'a [Row], rules: &SmartRules, now: i64, liked_min: f64, seed: u64, tz_min: i32) -> Vec<&'a Row> {
    let mut hit: Vec<&Row> = rows
        .iter()
        .filter(|row| matches_all_tz(rules, row, now, liked_min, tz_min))
        .collect();
    let library = |a: &&Row, b: &&Row| {
        fold(&a.artist)
            .cmp(&fold(&b.artist))
            .then(a.year.cmp(&b.year))
            .then(fold(&a.album).cmp(&fold(&b.album)))
            .then(a.disc.cmp(&b.disc))
            .then(a.track.cmp(&b.track))
            .then(a.id.cmp(&b.id))
    };
    let desc_opt = |a: Option<f64>, b: Option<f64>| b.unwrap_or(-1.0).partial_cmp(&a.unwrap_or(-1.0)).unwrap_or(Ordering::Equal);
    match rules.sort {
        SortBy::None => hit.sort_by(library),
        SortBy::Random => hit.sort_by_key(|r| mix(seed, &r.id)),
        SortBy::MostPlayed => hit.sort_by(|a, b| b.plays.cmp(&a.plays).then(library(a, b))),
        SortBy::LeastPlayed => hit.sort_by(|a, b| a.plays.cmp(&b.plays).then(library(a, b))),
        SortBy::RecentlyAdded => hit.sort_by(|a, b| b.added_at.cmp(&a.added_at).then(library(a, b))),
        SortBy::RecentlyPlayed => hit.sort_by(|a, b| b.last_played.cmp(&a.last_played).then(library(a, b))),
        SortBy::Rating => hit.sort_by(|a, b| desc_opt(a.rating, b.rating).then(library(a, b))),
    }
    if let Some(l) = rules.limit {
        match l.unit {
            LimitUnit::Songs => hit.truncate(l.value as usize),
            LimitUnit::Hours => {
                let cap = l.value as u64 * 3_600_000;
                let mut total = 0u64;
                hit.retain(|r| {
                    if total + r.duration_ms > cap {
                        return false;
                    }
                    total += r.duration_ms;
                    true
                });
            }
        }
    }
    hit
}
