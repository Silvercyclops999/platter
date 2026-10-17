//! The map as stored: a point per analysed song (`sound_map`, keyed by
//! track with the file it came from) and the projection that placed them
//! (`sound_map_layout`). Reads the analyses straight from `track_analysis`.

use rusqlite::{Connection, OptionalExtension, ToSql, params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::project::{self, Basis, LAYOUT_VERSION};
use crate::analysis::features::MIN_SECONDS;
use crate::db::release::track_unreleased;
use crate::error::{AppError, AppResult};

/// A song's analysis, current for its file.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysed {
    pub track_id: String,
    pub part_key: String,
    pub features: Vec<f32>,
}

/// An analysis the map can't use (unreadable, or the wrong shape), marked so
/// it isn't counted as waiting to be placed. Keyed by the analysis it had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    pub track_id: String,
    pub part_key: String,
    pub analysed_at: i64,
}

/// A point to write: song, file, x, y.
pub type Point = (String, String, f32, f32);

/// Points written per transaction, so other writers aren't held up for long.
pub const WRITE_CHUNK: usize = 2000;

/// Current analyses (for the song's file as it is now) that aren't marked
/// as unusable.
const ANALYSED: &str = "FROM track_analysis a
     JOIN tracks t ON t.id = a.track_id AND t.part_key = a.part_key
     WHERE a.status = 'ok' AND a.features IS NOT NULL
       AND NOT EXISTS (SELECT 1 FROM sound_map_skip s
                       WHERE s.track_id = a.track_id AND s.part_key = a.part_key AND s.analysed_at = a.analysed_at)";

const NOT_PLACED: &str = "AND NOT EXISTS (SELECT 1 FROM sound_map m WHERE m.track_id = a.track_id AND m.part_key = a.part_key)";

/// Goes through the analyses one row at a time, handing each usable vector
/// to `each`. Unusable ones go in `skips` (a corrupt one is logged).
fn scan(c: &Connection, only_unplaced: bool, skips: &mut Vec<Skip>, mut each: impl FnMut(Analysed)) -> AppResult<()> {
    let sql = format!(
        "SELECT a.track_id, a.part_key, a.features, a.analysed_at {ANALYSED} {}",
        if only_unplaced { NOT_PLACED } else { "" }
    );
    let mut st = c.prepare(&sql)?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let track_id: String = r.get(0)?;
        let part_key: String = r.get(1)?;
        let f: String = r.get(2)?;
        let features = match serde_json::from_str::<Vec<f32>>(&f) {
            Ok(v) if project::usable(&v) => Some(v),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(track = %track_id, "sound map: unreadable analysis, left off the map: {e}");
                None
            }
        };
        match features {
            Some(features) => each(Analysed { track_id, part_key, features }),
            None => skips.push(Skip { track_id, part_key, analysed_at: r.get(3)? }),
        }
    }
    Ok(())
}

/// Every song with a usable analysis.
pub fn each_analysed(c: &Connection, skips: &mut Vec<Skip>, each: impl FnMut(Analysed)) -> AppResult<()> {
    scan(c, false, skips, each)
}

/// Analysed songs that aren't on the map yet (or were placed from an older file).
pub fn each_unplaced(c: &Connection, skips: &mut Vec<Skip>, each: impl FnMut(Analysed)) -> AppResult<()> {
    scan(c, true, skips, each)
}

/// Every song with a usable analysis, all at once (tests and small libraries).
pub fn analysed(c: &Connection) -> AppResult<Vec<Analysed>> {
    let mut out = Vec::new();
    each_analysed(c, &mut Vec::new(), |a| out.push(a))?;
    Ok(out)
}

/// How many songs have an analysis the map can use. One read but not yet
/// checked counts too, until the map has looked at it.
pub fn analysed_count(c: &Connection) -> AppResult<usize> {
    Ok(c.query_row(&format!("SELECT count(*) {ANALYSED}"), [], |r| r.get::<_, i64>(0))? as usize)
}

/// The projection in use and the version it was made with. One that can't
/// be read is logged and treated as none, so the map is worked out again.
pub fn basis(c: &Connection) -> AppResult<Option<(Basis, i64)>> {
    let row: Option<(String, i64)> =
        c.query_row("SELECT basis, version FROM sound_map_layout WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((b, v)) = row else { return Ok(None) };
    match serde_json::from_str::<Basis>(&b) {
        Ok(b) if b.whole() => Ok(Some((b, v))),
        Ok(_) => {
            tracing::warn!("sound map: stored projection is the wrong shape; working it out again");
            Ok(None)
        }
        Err(e) => {
            tracing::warn!("sound map: stored projection unreadable ({e}); working it out again");
            Ok(None)
        }
    }
}

/// Marks analyses the map can't use.
pub fn skip(c: &mut Connection, skips: &[Skip]) -> AppResult<()> {
    let tx = c.transaction()?;
    {
        let mut st = tx.prepare_cached("INSERT OR REPLACE INTO sound_map_skip (track_id, part_key, analysed_at) VALUES (?1, ?2, ?3)")?;
        let mut del = tx.prepare_cached("DELETE FROM sound_map WHERE track_id = ?1")?;
        for s in skips {
            st.execute(params![s.track_id, s.part_key, s.analysed_at])?;
            del.execute([&s.track_id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// The last step of a new projection, once its points are written (in
/// chunks, with [`add_points`]): keeps the projection and drops points of
/// songs whose analysis is gone, so only the new layout's songs remain.
pub fn finish_layout(c: &mut Connection, b: &Basis, now: i64) -> AppResult<()> {
    let basis = serde_json::to_string(b).map_err(|e| AppError::Internal(format!("sound map projection: {e}")))?;
    let tx = c.transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO sound_map_layout (id, version, basis, built_at) VALUES (1, ?1, ?2, ?3)",
        params![LAYOUT_VERSION, basis, now],
    )?;
    tx.execute(
        "DELETE FROM sound_map WHERE NOT EXISTS (SELECT 1 FROM track_analysis a
             WHERE a.track_id = sound_map.track_id AND a.part_key = sound_map.part_key AND a.status = 'ok' AND a.features IS NOT NULL)",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

/// Points with the projection in use, in transactions of [`WRITE_CHUNK`].
/// Each chunk is its own write, so the database is free in between.
pub fn add_points(c: &mut Connection, points: &[Point]) -> AppResult<()> {
    let tx = c.transaction()?;
    {
        let mut st = tx.prepare_cached("INSERT OR REPLACE INTO sound_map (track_id, part_key, x, y) VALUES (?1, ?2, ?3, ?4)")?;
        for (id, pk, x, y) in points {
            st.execute(params![id, pk, *x as f64, *y as f64])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Drop points whose song left the library or whose file changed, and
/// unusable marks for analyses that have since been redone.
pub fn prune(c: &Connection) -> AppResult<usize> {
    c.execute(
        "DELETE FROM sound_map_skip WHERE NOT EXISTS (SELECT 1 FROM track_analysis a
             WHERE a.track_id = sound_map_skip.track_id AND a.part_key = sound_map_skip.part_key AND a.analysed_at = sound_map_skip.analysed_at)",
        [],
    )?;
    Ok(c.execute(
        "DELETE FROM sound_map WHERE NOT EXISTS (SELECT 1 FROM tracks t WHERE t.id = sound_map.track_id AND t.part_key = sound_map.part_key)",
        [],
    )?)
}

/// How far along the map is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// Songs on the map.
    pub placed: u64,
    /// Songs with an analysis.
    pub analysed: u64,
    /// Songs that can be analysed.
    pub total: u64,
}

pub fn counts(c: &Connection) -> AppResult<Counts> {
    let placed: i64 = c.query_row("SELECT count(*) FROM sound_map", [], |r| r.get(0))?;
    let total: i64 = c.query_row(
        "SELECT count(*) FROM tracks WHERE part_key <> '' AND duration_ms >= ?1",
        [MIN_SECONDS as i64 * 1000],
        |r| r.get(0),
    )?;
    Ok(Counts { placed: placed as u64, analysed: analysed_count(c)? as u64, total: total as u64 })
}

/// Part of the map, for loading it in pieces.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Area {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

/// Points in columns (one entry per song in each), so tens of thousands
/// travel small. `genre` indexes `genres` (-1: none); `year` 0 is unknown.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    pub ids: Vec<String>,
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub genre: Vec<i32>,
    pub genres: Vec<String>,
    pub year: Vec<i32>,
    pub unreleased: Vec<bool>,
    /// Pass back as `after` for the next chunk; None when this was the last.
    pub next: Option<String>,
}

pub const CHUNK_MAX: usize = 20_000;

fn first_genre(json: Option<String>) -> Option<String> {
    let v: Vec<String> = serde_json::from_str(json.as_deref().unwrap_or("[]")).ok()?;
    v.into_iter().map(|g| g.trim().to_string()).find(|g| !g.is_empty())
}

/// Up to `limit` points after the song id `after` (in id order), within
/// `area` when given.
pub fn points(c: &Connection, after: Option<&str>, limit: usize, area: Option<Area>) -> AppResult<Chunk> {
    let limit = limit.clamp(1, CHUNK_MAX);
    let after = after.unwrap_or("").to_string();
    let mut args: Vec<Box<dyn ToSql>> = vec![Box::new(after)];
    // An area is picked out of the map alone first (by its x, y index), so
    // only its points are joined to the library.
    let from = match area {
        Some(a) => {
            for v in [a.min_x, a.max_x, a.min_y, a.max_y] {
                args.push(Box::new(v as f64));
            }
            "(SELECT track_id, x, y FROM sound_map WHERE x >= ?2 AND x <= ?3 AND y >= ?4 AND y <= ?5 AND track_id > ?1)"
        }
        None => "sound_map",
    };
    let sql = format!(
        "SELECT m.track_id, m.x, m.y, t.genres, al.genres, al.year, t.album_id, coalesce(f.version, f.release),
                al.format, al.subformats, al.matched, al.studio, mc.found, coalesce(at.version, at.release), al.id IS NOT NULL
         FROM {from} m JOIN tracks t ON t.id = m.track_id
         LEFT JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'
         LEFT JOIN albums al ON al.id = t.album_id
         LEFT JOIN album_mb_check mc ON mc.album_id = t.album_id
         LEFT JOIN album_tags at ON at.album_id = t.album_id
         WHERE m.track_id > ?1
         ORDER BY m.track_id LIMIT {}",
        limit + 1
    );
    let mut st = c.prepare(&sql)?;
    let mut rows = st.query(params_from_iter(args.iter().map(|b| b.as_ref())))?;
    let mut out = Chunk::default();
    let mut names: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    while let Some(r) = rows.next()? {
        if out.ids.len() == limit {
            out.next = out.ids.last().cloned();
            break;
        }
        let id: String = r.get(0)?;
        let album_genres: Option<String> = r.get(4)?;
        let genre = first_genre(r.get(3)?).or_else(|| first_genre(album_genres));
        let g = match genre {
            Some(name) => {
                let n = names.len() as i32;
                *names.entry(name.clone()).or_insert_with(|| {
                    out.genres.push(name);
                    n
                })
            }
            None => -1,
        };
        let album_id: String = r.get(6)?;
        let subs: Option<String> = r.get(9)?;
        let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
        let known: bool = r.get(14)?;
        // An album not synced yet says nothing either way.
        let matched: Option<bool> = if known { r.get(10)? } else { Some(true) };
        let version: Option<String> = r.get::<_, Option<String>>(7)?.or(r.get(13)?);
        let format: Option<String> = r.get(8)?;
        let studio: Option<String> = r.get(11)?;
        let unreleased =
            track_unreleased(&id, &album_id, format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref(), r.get(12)?);
        out.x.push(r.get::<_, f64>(1)? as f32);
        out.y.push(r.get::<_, f64>(2)? as f32);
        out.genre.push(g);
        out.year.push(r.get::<_, Option<i32>>(5)?.unwrap_or(0));
        out.unreleased.push(unreleased);
        out.ids.push(id);
    }
    Ok(out)
}

/// A song or an artist to fly to.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    /// "song" or "artist".
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    /// The artist for a song, the number of songs on the map for an artist.
    pub sub: String,
    pub x: f32,
    pub y: f32,
}

fn like(q: &str) -> String {
    let mut s = String::from("%");
    for ch in q.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            s.push('\\');
        }
        s.push(ch);
    }
    s.push('%');
    s
}

/// Artists (at the middle of their songs) and songs on the map matching `q`.
pub fn find(c: &Connection, q: &str) -> AppResult<Vec<Found>> {
    let q = q.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let pat = like(q);
    let start = like(q).trim_start_matches('%').to_string();
    let mut out = Vec::new();
    let mut st = c.prepare_cached(
        "SELECT t.artist_id, t.artist_title, avg(m.x), avg(m.y), count(*) FROM sound_map m JOIN tracks t ON t.id = m.track_id
         WHERE t.artist_title LIKE ?1 ESCAPE '\\' GROUP BY t.artist_id
         ORDER BY t.artist_title LIKE ?2 ESCAPE '\\' DESC, count(*) DESC LIMIT 3",
    )?;
    let rows = st.query_map(params![pat, start], |r| {
        let n: i64 = r.get(4)?;
        Ok(Found {
            kind: "artist",
            id: r.get(0)?,
            title: r.get(1)?,
            sub: if n == 1 { "1 song".into() } else { format!("{n} songs") },
            x: r.get::<_, f64>(2)? as f32,
            y: r.get::<_, f64>(3)? as f32,
        })
    })?;
    for r in rows {
        out.push(r?);
    }
    let mut st = c.prepare_cached(
        "SELECT t.id, t.title, coalesce(t.track_artist, t.artist_title), m.x, m.y FROM sound_map m JOIN tracks t ON t.id = m.track_id
         WHERE t.title LIKE ?1 ESCAPE '\\'
         ORDER BY t.title LIKE ?2 ESCAPE '\\' DESC, t.view_count DESC LIMIT 6",
    )?;
    let rows = st.query_map(params![pat, start], |r| {
        Ok(Found {
            kind: "song",
            id: r.get(0)?,
            title: r.get(1)?,
            sub: r.get(2)?,
            x: r.get::<_, f64>(3)? as f32,
            y: r.get::<_, f64>(4)? as f32,
        })
    })?;
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// What the hover card shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album_title: String,
    pub thumb: Option<String>,
}

pub fn song(c: &Connection, id: &str) -> AppResult<Option<Song>> {
    Ok(c
        .query_row(
            "SELECT t.id, t.title, coalesce(t.track_artist, t.artist_title), t.album_title, coalesce(t.thumb, al.thumb)
             FROM tracks t LEFT JOIN albums al ON al.id = t.album_id WHERE t.id = ?1",
            [id],
            |r| Ok(Song { id: r.get(0)?, title: r.get(1)?, artist: r.get(2)?, album_title: r.get(3)?, thumb: r.get(4)? }),
        )
        .optional()?)
}
