//! "Played a lot, not in your library": songs and albums from imported
//! listening history (any source in `imported_plays`) that have no match in
//! the library, for the Missing page.
//!
//! - Songs group plays by folded artist and title, albums by folded artist
//!   and album. A play counts when it lasted at least 30 seconds (or its
//!   source doesn't say how long; scrobbles only exist past that).
//! - Ranked by plays, then total time. Pages of [`PAGE`] rows.
//! - Filters: fewest plays, a period (all time, last year, last 90 days),
//!   rows and whole artists the user hid (synced through the store's
//!   `playedHidden` namespace), and sources left out of every count.
//! - When the library gains a song, [`rematch`] matches its plays (after
//!   each library sync, and before the list loads), so stats count them and
//!   the row leaves the list.
//!
//! Plays are grouped through their folded columns (schema v41). Rows from
//! before those columns are folded by [`fill_folds`].

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::SongIn;
use super::matching::{Matcher, Status};
use super::store::{StoreLinks, album_store_links, store_links};
use crate::db::listens::play_folds;
use crate::db::merge::Rank;
use crate::error::{AppError, AppResult};
use crate::identity::fold;
use crate::lidarr::status::RequestState;

pub const MIN_PLAYS_DEFAULT: u32 = 5;
pub const MIN_PLAYS_LOW: u32 = 1;
pub const MIN_PLAYS_HIGH: u32 = 1_000;
/// Rows per page.
pub const PAGE: usize = 100;
/// A play shorter than this is a skip.
pub const MIN_MS: i64 = 30_000;
/// Rows folded per write when filling older rows.
const FOLD_BATCH: usize = 5_000;
const DAY_MS: i64 = 86_400_000;
/// Hide keys are cut to this many characters (the sync store's key limit
/// is 200).
const KEY_CHARS: usize = 190;
/// Where the last re-match remembers what it saw.
const REMATCH_KEY: &str = "import.playedRematch";

/// Which plays count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    All,
    Year,
    Days90,
}

impl Period {
    pub fn parse(s: &str) -> Option<Period> {
        match s {
            "all" => Some(Period::All),
            "year" => Some(Period::Year),
            "days90" => Some(Period::Days90),
            _ => None,
        }
    }

    /// The earliest play time that counts.
    pub fn since(self, now_ms: i64) -> i64 {
        match self {
            Period::All => i64::MIN,
            Period::Year => now_ms - 365 * DAY_MS,
            Period::Days90 => now_ms - 90 * DAY_MS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Song,
    Album,
}

/// What the list asks for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    pub kind: Kind,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_min")]
    pub min_plays: u32,
    /// `all`, `year` or `days90`.
    #[serde(default = "default_period")]
    pub period: String,
}

fn default_min() -> u32 {
    MIN_PLAYS_DEFAULT
}

fn default_period() -> String {
    "all".into()
}

/// A row hidden from the list (synced): a song, an album or a whole artist.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenRow {
    /// `song`, `album` or `artist`.
    pub kind: String,
    pub artist: String,
    /// The song or album; empty for an artist.
    #[serde(default)]
    pub name: String,
    /// Unix seconds.
    #[serde(default)]
    pub hidden_at: i64,
}

/// The key a hidden row is kept under, from folded names.
pub fn hide_key(kind: &str, artist_fold: &str, name_fold: &str) -> String {
    let k = if kind == "artist" { format!("artist:{artist_fold}") } else { format!("{kind}:{artist_fold}\u{1f}{name_fold}") };
    k.chars().take(KEY_CHARS).collect()
}

/// [`hide_key`] from names as shown.
pub fn hide_key_for(kind: &str, artist: &str, name: &str) -> AppResult<String> {
    let a = fold::fold_artist(artist);
    let n = match kind {
        "song" => fold::fold_title(name).title,
        "album" => fold::fold_album(name),
        "artist" => String::new(),
        _ => return Err(AppError::Invalid("Only songs, albums and artists can be hidden.".into())),
    };
    if a.is_empty() || (kind != "artist" && n.is_empty()) {
        return Err(AppError::Invalid("There's nothing to hide.".into()));
    }
    Ok(hide_key(kind, &a, &n))
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayedRow {
    pub kind: Kind,
    /// Its hide key, also a stable id for the row.
    pub key: String,
    pub artist: String,
    /// The song's title, or the album's.
    pub title: String,
    /// A song's album, when the history named it.
    pub album: Option<String>,
    pub plays: u64,
    /// Total time played, ms.
    pub ms: u64,
    /// Unix ms.
    pub first_played: i64,
    pub last_played: i64,
    /// An album's songs that were played.
    pub songs: Option<u64>,
    /// An album's songs already in the library.
    pub in_library: Option<u64>,
    pub links: StoreLinks,
    /// A cover, when Platter knows the album's release group.
    pub thumb: Option<String>,
    /// The Lidarr request made for its album, if any.
    pub request: Option<RequestState>,
}

/// A page of the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayedPage {
    pub kind: Kind,
    /// Rows matching the filters, over every page.
    pub total: u64,
    pub offset: usize,
    pub rows: Vec<PlayedRow>,
    /// Any listening history was imported at all.
    pub has_history: bool,
}

// ------------------------------------------------------------------ folding and matching

/// Folds imported plays that came in before the folded columns. Returns
/// how many rows it folded.
pub fn fill_folds(c: &mut Connection) -> AppResult<usize> {
    let mut done = 0;
    loop {
        let tx = c.transaction()?;
        let rows: Vec<(i64, String, Option<String>, Option<String>)> = {
            let mut st = tx.prepare_cached("SELECT id, title, artist, album FROM imported_plays WHERE title_fold IS NULL LIMIT ?1")?;
            st.query_map([FOLD_BATCH as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?
        };
        if rows.is_empty() {
            return Ok(done);
        }
        {
            let mut up = tx.prepare_cached("UPDATE imported_plays SET artist_fold = ?2, title_fold = ?3, album_fold = ?4 WHERE id = ?1")?;
            for (id, title, artist, album) in &rows {
                let (a, t, al) = play_folds(title, artist.as_deref(), album.as_deref());
                up.execute(params![id, a, t, al])?;
            }
        }
        tx.commit()?;
        done += rows.len();
        if rows.len() < FOLD_BATCH {
            return Ok(done);
        }
    }
}

/// Matches unmatched imported plays whose song the library now has (by
/// folded names in the song index, then the usual matcher). Skips the work
/// when neither the library nor the unmatched plays changed since the last
/// time. Returns how many plays it matched.
pub fn rematch(c: &mut Connection, ranks: HashMap<String, Rank>, now_ms: i64) -> AppResult<usize> {
    let sig: String = c.query_row(
        "SELECT (SELECT count(*) FROM song_index) || ':' || (SELECT count(*) FROM imported_plays WHERE rating_key IS NULL)",
        [],
        |r| r.get(0),
    )?;
    if crate::db::store::kv_get(c, REMATCH_KEY)?.as_deref() == Some(sig.as_str()) {
        return Ok(0);
    }
    // Every song with unmatched plays whose names the library has, and one
    // play's names to match it by.
    let groups: Vec<(String, String, String, String, Option<String>)> = {
        let mut st = c.prepare(
            "SELECT p.artist_fold, p.title_fold, max(p.title), max(coalesce(p.artist, '')), max(p.album)
             FROM imported_plays p
             WHERE p.rating_key IS NULL AND p.title_fold IS NOT NULL AND p.title_fold <> ''
               AND EXISTS (SELECT 1 FROM song_index s WHERE s.artist_fold = p.artist_fold AND s.title_fold = p.title_fold AND s.unreleased = 0)
             GROUP BY p.artist_fold, p.title_fold",
        )?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<_, _>>()?
    };
    let mut found: Vec<(String, String, String)> = Vec::new();
    {
        let mut m = Matcher::new(c, ranks);
        for (a, t, title, artist, album) in groups {
            let o = m.find(&SongIn { artist, title, album, ..Default::default() })?;
            if o.status == Status::Matched
                && let Some(id) = o.track_id
            {
                found.push((a, t, id));
            }
        }
    }
    let tx = c.transaction()?;
    let mut n = 0;
    {
        let mut up = tx.prepare_cached(
            "UPDATE imported_plays SET rating_key = ?3, matched_at = ?4 WHERE rating_key IS NULL AND artist_fold = ?1 AND title_fold = ?2",
        )?;
        for (a, t, id) in &found {
            n += up.execute(params![a, t, id, now_ms])?;
        }
    }
    let sig: String = tx.query_row(
        "SELECT (SELECT count(*) FROM song_index) || ':' || (SELECT count(*) FROM imported_plays WHERE rating_key IS NULL)",
        [],
        |r| r.get(0),
    )?;
    crate::db::store::kv_set(&tx, REMATCH_KEY, &sig)?;
    tx.commit()?;
    Ok(n)
}

/// [`fill_folds`] then [`rematch`], logged instead of failing (after a sync).
pub fn keep_up(c: &mut Connection, ranks: HashMap<String, Rank>, now_ms: i64) {
    if let Err(e) = fill_folds(c) {
        tracing::warn!("imported plays not folded: {e}");
        return;
    }
    match rematch(c, ranks, now_ms) {
        Ok(0) => {}
        Ok(n) => tracing::info!(plays = n, "imported plays matched to new library songs"),
        Err(e) => tracing::warn!("imported plays not matched again: {e}"),
    }
}

// ------------------------------------------------------------------ the report

/// What a source's imported history holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCounts {
    pub plays: u64,
    /// Plays of songs in the library.
    pub plays_matched: u64,
    /// Different songs played.
    pub songs: u64,
    /// Of those, songs in the library.
    pub songs_matched: u64,
    /// Unix ms; None without plays.
    pub first_played: Option<i64>,
    pub last_played: Option<i64>,
}

/// The history a file source imported (its plays' ids start with the source).
pub fn history_counts(c: &Connection, source: &str) -> AppResult<HistoryCounts> {
    Ok(c.query_row(
        "SELECT count(*), count(rating_key),
                count(DISTINCT coalesce(artist_fold, lower(coalesce(artist, ''))) || char(31) || coalesce(title_fold, lower(title))),
                count(DISTINCT CASE WHEN rating_key IS NOT NULL
                      THEN coalesce(artist_fold, lower(coalesce(artist, ''))) || char(31) || coalesce(title_fold, lower(title)) END),
                min(played_at), max(played_at)
         FROM imported_plays WHERE source = 'file' AND substr(source_id, 1, ?2) = ?1",
        params![format!("{source}:"), source.len() as i64 + 1],
        |r| {
            Ok(HistoryCounts {
                plays: r.get::<_, i64>(0)? as u64,
                plays_matched: r.get::<_, i64>(1)? as u64,
                songs: r.get::<_, i64>(2)? as u64,
                songs_matched: r.get::<_, i64>(3)? as u64,
                first_played: r.get(4)?,
                last_played: r.get(5)?,
            })
        },
    )?)
}

// ------------------------------------------------------------------ the list

/// The plays every grouping counts, as a WHERE clause over `imported_plays`
/// (?1 since, ?2 hidden artists as JSON).
const COUNTED: &str = "rating_key IS NULL AND title_fold IS NOT NULL AND title_fold <> ''
    AND (ms_played IS NULL OR ms_played >= 30000)
    AND played_at >= ?1
    AND artist_fold NOT IN (SELECT value FROM json_each(?2))
    AND source NOT IN (SELECT value FROM json_each(coalesce(
        (SELECT value FROM kv WHERE key = 'plays.excludedSources' AND json_valid(value)), '[]')))";

/// The cover and request state of the album a row is for, by folded artist
/// and album: from Lidarr requests made on this device and albums added to
/// Missing by hand.
struct Known {
    by_album: HashMap<(String, String), (String, Option<RequestState>)>,
}

impl Known {
    fn load(c: &Connection) -> AppResult<Known> {
        let mut by_album = HashMap::new();
        for w in super::store::wanted_albums(c)? {
            by_album.insert((fold::fold_artist(&w.artist), fold::fold_album(&w.title)), (w.release_group_id, None));
        }
        for r in crate::lidarr::request::saved_requests(c) {
            by_album.insert((fold::fold_artist(&r.artist), fold::fold_album(&r.title)), (r.release_group_id, Some(r.state)));
        }
        Ok(Known { by_album })
    }

    fn get(&self, artist_fold: &str, album_fold: &str) -> (Option<String>, Option<RequestState>) {
        match self.by_album.get(&(artist_fold.to_string(), album_fold.to_string())) {
            Some((rg, st)) => (crate::lidarr::request::cover_thumb(rg), *st),
            None => (None, None),
        }
    }
}

/// A page of the list. `hidden` holds the hide keys (rows and artists).
pub fn list(c: &Connection, q: &Query, hidden: &[String], now_ms: i64) -> AppResult<PlayedPage> {
    let period = Period::parse(&q.period).ok_or_else(|| AppError::Invalid("Pick all time, the last year or the last 90 days.".into()))?;
    let since = period.since(now_ms);
    let min = q.min_plays.clamp(MIN_PLAYS_LOW, MIN_PLAYS_HIGH) as i64;
    let artists: Vec<&str> = hidden.iter().filter_map(|k| k.strip_prefix("artist:")).collect();
    let artists = serde_json::to_string(&artists).unwrap_or_else(|_| "[]".into());
    let rows_hidden: Vec<&String> = hidden.iter().filter(|k| !k.starts_with("artist:")).collect();
    let rows_hidden = serde_json::to_string(&rows_hidden).unwrap_or_else(|_| "[]".into());
    let has_history: bool = c.query_row("SELECT EXISTS (SELECT 1 FROM imported_plays)", [], |r| r.get(0))?;
    let known = Known::load(c)?;
    let limit = PAGE as i64;
    let offset = q.offset as i64;
    let (kind_name, name_col, extra) = match q.kind {
        Kind::Song => ("song", "title_fold", "0"),
        Kind::Album => ("album", "album_fold", "count(DISTINCT title_fold)"),
    };
    let album_only = if q.kind == Kind::Album { "AND album_fold IS NOT NULL AND album_fold <> ''" } else { "" };
    let sql = format!(
        "WITH g AS (
           SELECT artist_fold AS a, {name_col} AS n, count(*) AS plays, sum(coalesce(ms_played, 0)) AS ms,
                  min(played_at) AS first, max(played_at) AS last, {extra} AS songs
           FROM imported_plays INDEXED BY imported_plays_unmatched_{kind_name}
           WHERE {COUNTED} {album_only}
           GROUP BY artist_fold, {name_col}
           HAVING count(*) >= ?3),
         v AS (SELECT * FROM g WHERE substr('{kind_name}:' || a || char(31) || n, 1, {KEY_CHARS}) NOT IN (SELECT value FROM json_each(?4)))
         SELECT a, n, plays, ms, first, last, songs, count(*) OVER () FROM v
         ORDER BY plays DESC, ms DESC, a, n
         LIMIT ?5 OFFSET ?6"
    );
    type Raw = (String, String, i64, i64, i64, i64, i64, i64);
    let raw: Vec<Raw> = {
        let mut st = c.prepare_cached(&sql)?;
        st.query_map(params![since, artists, min, rows_hidden, limit, offset], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
        })?
        .collect::<Result<_, _>>()?
    };
    let mut total = raw.first().map_or(0, |r| r.7 as u64);
    if raw.is_empty() && offset > 0 {
        // Past the end: count what there is.
        let count_sql = format!(
            "SELECT count(*) FROM (SELECT artist_fold AS a, {name_col} AS n FROM imported_plays INDEXED BY imported_plays_unmatched_{kind_name}
               WHERE {COUNTED} {album_only} GROUP BY artist_fold, {name_col} HAVING count(*) >= ?3)
             WHERE substr('{kind_name}:' || a || char(31) || n, 1, {KEY_CHARS}) NOT IN (SELECT value FROM json_each(?4))"
        );
        total = c.query_row(&count_sql, params![since, artists, min, rows_hidden], |r| r.get::<_, i64>(0))? as u64;
    }
    let mut rows = Vec::with_capacity(raw.len());
    for (a, n, plays, ms, first, last, songs, _) in raw {
        rows.push(match q.kind {
            Kind::Song => song_row(c, &known, a, n, plays, ms, first, last)?,
            Kind::Album => album_row(c, &known, a, n, plays, ms, first, last, songs)?,
        });
    }
    Ok(PlayedPage { kind: q.kind, total, offset: q.offset, rows, has_history })
}

#[allow(clippy::too_many_arguments)]
fn song_row(c: &Connection, known: &Known, a: String, t: String, plays: i64, ms: i64, first: i64, last: i64) -> AppResult<PlayedRow> {
    // The names of its latest play, as the history spelled them.
    let (title, artist, album): (String, Option<String>, Option<String>) = c
        .prepare_cached(
            "SELECT title, artist, album FROM imported_plays INDEXED BY imported_plays_unmatched_song
             WHERE rating_key IS NULL AND artist_fold = ?1 AND title_fold = ?2 ORDER BY played_at DESC LIMIT 1",
        )?
        .query_row(params![a, t], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?
        .unwrap_or_default();
    let artist = artist.unwrap_or_default();
    let album = album.filter(|x| !x.trim().is_empty());
    let (thumb, request) = album.as_deref().map(|al| known.get(&a, &fold::fold_album(al))).unwrap_or((None, None));
    Ok(PlayedRow {
        kind: Kind::Song,
        key: hide_key("song", &a, &t),
        links: store_links(&artist, &title),
        artist,
        title,
        album,
        plays: plays as u64,
        ms: ms.max(0) as u64,
        first_played: first,
        last_played: last,
        songs: None,
        in_library: None,
        thumb,
        request,
    })
}

#[allow(clippy::too_many_arguments)]
fn album_row(c: &Connection, known: &Known, a: String, al: String, plays: i64, ms: i64, first: i64, last: i64, songs: i64) -> AppResult<PlayedRow> {
    let (album, artist): (String, Option<String>) = c
        .prepare_cached(
            "SELECT album, artist FROM imported_plays INDEXED BY imported_plays_unmatched_album
             WHERE rating_key IS NULL AND artist_fold = ?1 AND album_fold = ?2 ORDER BY played_at DESC LIMIT 1",
        )?
        .query_row(params![a, al], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get(1)?)))
        .optional()?
        .unwrap_or_default();
    let artist = artist.unwrap_or_default();
    let in_library: i64 = c
        .prepare_cached("SELECT count(DISTINCT track_id) FROM song_index WHERE artist_fold = ?1 AND album_fold = ?2 AND unreleased = 0")?
        .query_row(params![a, al], |r| r.get(0))?;
    let (thumb, request) = known.get(&a, &al);
    Ok(PlayedRow {
        kind: Kind::Album,
        key: hide_key("album", &a, &al),
        links: album_store_links(&artist, &album),
        artist,
        title: album,
        album: None,
        plays: plays as u64,
        ms: ms.max(0) as u64,
        first_played: first,
        last_played: last,
        songs: Some(songs as u64),
        in_library: Some(in_library as u64),
        thumb,
        request,
    })
}
