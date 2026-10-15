//! Compare stats with a friend. Each side makes a small summary of its
//! recent taste: its top artists and songs by name, each with a weight
//! (its share of the plays counted). No play times, no counts, no history.
//! Unreleased songs stay out of a summary unless both sides opted in.
//! Comparing two summaries gives a taste overlap, the artists in common and
//! the friend's songs you haven't played.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::queries;
use crate::error::{AppError, AppResult};
use crate::identity::fold::{fold_artist, fold_title};
use crate::model::Track;

pub const SUMMARY_VERSION: u32 = 1;
/// A summary covers about the last six months.
pub const SUMMARY_DAYS: i64 = 182;
pub const SUMMARY_ARTISTS: usize = 50;
pub const SUMMARY_SONGS: usize = 50;
/// Their songs you haven't played, at most.
pub const NEW_SONGS_MAX: usize = 20;
pub const COMMON_MAX: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightedArtist {
    pub name: String,
    /// Share of the summary's plays, 0 to 1.
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightedSong {
    pub artist: String,
    pub title: String,
    pub weight: f64,
}

/// What one side shares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareSummary {
    pub version: u32,
    pub name: Option<String>,
    pub artists: Vec<WeightedArtist>,
    pub songs: Vec<WeightedSong>,
    /// This side agreed to include unreleased songs.
    pub unreleased_ok: bool,
}

/// One weight per name, rounded so the summary says nothing finer.
fn weigh<T>(rows: Vec<(T, i64)>, max: usize) -> Vec<(T, f64)> {
    let total: i64 = rows.iter().map(|r| r.1).sum();
    if total <= 0 {
        return Vec::new();
    }
    rows.into_iter()
        .take(max)
        .map(|(t, p)| (t, (p as f64 / total as f64 * 1000.0).round() / 1000.0))
        .filter(|(_, w)| *w > 0.0)
        .collect()
}

/// Your summary. `with_unreleased` only when both sides opted in.
pub fn summary(c: &Connection, today: i64, name: Option<String>, unreleased_ok: bool, with_unreleased: bool) -> AppResult<CompareSummary> {
    let lo = today - SUMMARY_DAYS + 1;
    // Songs counted in the window, released ones only unless both opted in.
    let released = if with_unreleased {
        ""
    } else {
        "AND (s.rating_key IS NULL OR NOT EXISTS (SELECT 1 FROM song_index i WHERE i.track_id = s.rating_key AND i.unreleased = 1))"
    };
    let mut st = c.prepare(&format!(
        "SELECT s.artist_key, max(a.name), sum(d.plays) AS p FROM stats_song_day d
         JOIN stats_song s ON s.song_key = d.song_key JOIN stats_artist a ON a.artist_key = s.artist_key
         WHERE d.day BETWEEN ?1 AND ?2 {released}
         GROUP BY s.artist_key ORDER BY p DESC, s.artist_key"
    ))?;
    let artists: Vec<(String, i64)> =
        st.query_map(params![lo, today], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?.collect::<Result<_, _>>()?;
    let mut st = c.prepare(&format!(
        "SELECT s.artist, s.title, sum(d.plays) AS p FROM stats_song_day d JOIN stats_song s ON s.song_key = d.song_key
         WHERE d.day BETWEEN ?1 AND ?2 {released}
         GROUP BY s.song_key ORDER BY p DESC, s.song_key"
    ))?;
    let songs: Vec<((String, String), i64)> = st
        .query_map(params![lo, today], |r| Ok(((r.get::<_, String>(0)?, r.get::<_, String>(1)?), r.get::<_, i64>(2)?)))?
        .collect::<Result<_, _>>()?;
    Ok(CompareSummary {
        version: SUMMARY_VERSION,
        name: name.filter(|n| !n.trim().is_empty()),
        artists: weigh(artists, SUMMARY_ARTISTS).into_iter().map(|(name, weight)| WeightedArtist { name, weight }).collect(),
        songs: weigh(songs, SUMMARY_SONGS)
            .into_iter()
            .map(|((artist, title), weight)| WeightedSong { artist, title, weight })
            .collect(),
        unreleased_ok,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommonArtist {
    pub name: String,
    pub artist_id: Option<String>,
    pub thumb: Option<String>,
    pub mine: f64,
    pub theirs: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSong {
    pub artist: String,
    pub title: String,
    /// The library song, when you have it.
    pub track: Option<Track>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareResult {
    pub friend: Option<String>,
    /// Taste overlap, 0 to 100.
    pub overlap: u32,
    pub common_artists: Vec<CommonArtist>,
    pub their_songs_new: Vec<NewSong>,
}

/// Folded name to weight, normalised so the weights add up to 1.
fn normalised(artists: &[WeightedArtist]) -> HashMap<String, (f64, String)> {
    let total: f64 = artists.iter().map(|a| a.weight.max(0.0)).sum();
    let mut out: HashMap<String, (f64, String)> = HashMap::new();
    if total <= 0.0 {
        return out;
    }
    for a in artists {
        let k = fold_artist(&a.name);
        if k.is_empty() {
            continue;
        }
        let e = out.entry(k).or_insert((0.0, a.name.clone()));
        e.0 += a.weight.max(0.0) / total;
    }
    out
}

/// Taste overlap: the weight both sides give the same artists.
pub fn overlap(mine: &[WeightedArtist], theirs: &[WeightedArtist]) -> u32 {
    let a = normalised(mine);
    let b = normalised(theirs);
    let shared: f64 = a.iter().filter_map(|(k, (w, _))| b.get(k).map(|(v, _)| w.min(*v))).sum();
    (shared * 100.0).round().clamp(0.0, 100.0) as u32
}

fn song_fold(artist: &str, title: &str) -> (String, String) {
    (fold_artist(artist), fold_title(title).title)
}

pub fn validate(s: &CompareSummary) -> AppResult<()> {
    if s.version != SUMMARY_VERSION {
        return Err(AppError::Invalid("That stats summary is from a different version of Platter.".into()));
    }
    if s.artists.len() > SUMMARY_ARTISTS * 2 || s.songs.len() > SUMMARY_SONGS * 2 {
        return Err(AppError::Invalid("That stats summary is too big.".into()));
    }
    Ok(())
}

/// Compare your summary with a friend's.
pub fn compare(c: &Connection, mine: &CompareSummary, theirs: &CompareSummary) -> AppResult<CompareResult> {
    validate(theirs)?;
    let a = normalised(&mine.artists);
    let b = normalised(&theirs.artists);
    let mut common: Vec<CommonArtist> = a
        .iter()
        .filter_map(|(k, (w, name))| {
            b.get(k).map(|(v, _)| CommonArtist { name: name.clone(), artist_id: None, thumb: None, mine: *w, theirs: *v })
        })
        .collect();
    common.sort_by(|x, y| y.mine.min(y.theirs).total_cmp(&x.mine.min(x.theirs)).then_with(|| x.name.cmp(&y.name)));
    common.truncate(COMMON_MAX);
    let mut lib = c.prepare_cached("SELECT id, thumb FROM artists WHERE lower(title) = lower(?1) LIMIT 1")?;
    for ca in &mut common {
        if let Some((id, thumb)) = lib.query_row([&ca.name], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))).optional()? {
            ca.artist_id = Some(id);
            ca.thumb = thumb;
        }
    }

    // Everything you've ever played, by folded artist and title.
    let mut played: HashSet<(String, String)> = HashSet::new();
    let mut st = c.prepare("SELECT artist, title FROM stats_song WHERE plays > 0")?;
    for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (artist, title) = row?;
        played.insert(song_fold(&artist, &title));
    }
    let both_unreleased = mine.unreleased_ok && theirs.unreleased_ok;
    let mut find = c.prepare_cached(
        "SELECT track_id FROM song_index WHERE artist_fold = ?1 AND title_fold = ?2
         AND (?3 OR unreleased = 0) ORDER BY version_fold = '' DESC LIMIT 1",
    )?;
    let mut new_songs = Vec::new();
    let mut seen = HashSet::new();
    for s in &theirs.songs {
        let key = song_fold(&s.artist, &s.title);
        if key.0.is_empty() || key.1.is_empty() || played.contains(&key) || !seen.insert(key.clone()) {
            continue;
        }
        let id: Option<String> = find.query_row(params![key.0, key.1, both_unreleased], |r| r.get(0)).optional()?;
        let track = match id {
            Some(id) => queries::tracks_by_ids(c, &[id])?.pop(),
            None => None,
        };
        new_songs.push(NewSong { artist: s.artist.clone(), title: s.title.clone(), track });
        if new_songs.len() >= NEW_SONGS_MAX {
            break;
        }
    }
    Ok(CompareResult { friend: theirs.name.clone(), overlap: overlap(&mine.artists, &theirs.artists), common_artists: common, their_songs_new: new_songs })
}

// ---------------------------------------------------------------- friends' summaries

const FRIENDS_KEY: &str = "stats.friends";
/// Friends' summaries kept, at most.
pub const FRIENDS_MAX: usize = 20;

/// Summaries friends shared with you, newest first. Only what they shared
/// is kept: names and weights.
pub fn friends(c: &Connection) -> AppResult<Vec<CompareSummary>> {
    Ok(crate::db::store::kv_get(c, FRIENDS_KEY)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
}

fn save_friends(c: &Connection, all: &[CompareSummary]) -> AppResult<()> {
    let json = serde_json::to_string(all).map_err(|e| AppError::Internal(format!("friends: {e}")))?;
    crate::db::store::kv_set(c, FRIENDS_KEY, &json)
}

/// Keep a friend's summary, replacing an older one with the same name.
/// Whatever delivers a friend's summary to this computer calls this.
pub fn friend_add(c: &Connection, s: CompareSummary) -> AppResult<Vec<CompareSummary>> {
    validate(&s)?;
    let mut all = friends(c)?;
    all.retain(|f| f.name.is_none() || f.name != s.name);
    all.insert(0, s);
    all.truncate(FRIENDS_MAX);
    save_friends(c, &all)?;
    Ok(all)
}

/// Forget a friend's summary.
pub fn friend_remove(c: &Connection, name: &str) -> AppResult<Vec<CompareSummary>> {
    let mut all = friends(c)?;
    all.retain(|f| f.name.as_deref() != Some(name));
    save_friends(c, &all)?;
    Ok(all)
}
