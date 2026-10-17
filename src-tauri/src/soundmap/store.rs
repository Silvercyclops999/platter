//! The map as stored: a point per analysed song (`sound_map`, keyed by
//! track with the file it came from) and the projection that placed them
//! (`sound_map_layout`). Reads the analyses straight from `track_analysis`.

use rusqlite::{Connection, OptionalExtension, ToSql, params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::project::{self, Basis, LAYOUT_VERSION};
use crate::analysis::features::MIN_SECONDS;
use crate::db::release::track_unreleased;
use crate::error::AppResult;

/// A song's analysis, current for its file.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysed {
    pub track_id: String,
    pub part_key: String,
    pub features: Vec<f32>,
}

const ANALYSED: &str = "SELECT a.track_id, a.part_key, a.features FROM track_analysis a
     JOIN tracks t ON t.id = a.track_id AND t.part_key = a.part_key
     WHERE a.status = 'ok' AND a.features IS NOT NULL";

fn analysed_rows(c: &Connection, sql: &str) -> AppResult<Vec<Analysed>> {
    let mut st = c.prepare(sql)?;
    let rows = st.query_map([], |r| {
        let f: String = r.get(2)?;
        Ok(Analysed { track_id: r.get(0)?, part_key: r.get(1)?, features: serde_json::from_str(&f).unwrap_or_default() })
    })?;
    let mut out = Vec::new();
    for r in rows {
        let a = r?;
        if project::usable(&a.features) {
            out.push(a);
        }
    }
    Ok(out)
}

/// Every song with a usable analysis.
pub fn analysed(c: &Connection) -> AppResult<Vec<Analysed>> {
    analysed_rows(c, ANALYSED)
}

/// Analysed songs that aren't on the map yet (or were placed from an older file).
pub fn unplaced(c: &Connection) -> AppResult<Vec<Analysed>> {
    analysed_rows(
        c,
        &format!("{ANALYSED} AND NOT EXISTS (SELECT 1 FROM sound_map m WHERE m.track_id = a.track_id AND m.part_key = a.part_key)"),
    )
}

/// How many songs have an analysis the map can use (roughly: the vector is checked when read).
pub fn analysed_count(c: &Connection) -> AppResult<usize> {
    Ok(c.query_row(
        "SELECT count(*) FROM track_analysis a JOIN tracks t ON t.id = a.track_id AND t.part_key = a.part_key
         WHERE a.status = 'ok' AND a.features IS NOT NULL",
        [],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

/// The projection in use and the version it was made with.
pub fn basis(c: &Connection) -> AppResult<Option<(Basis, i64)>> {
    let row: Option<(String, i64)> =
        c.query_row("SELECT basis, version FROM sound_map_layout WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    Ok(row.and_then(|(b, v)| serde_json::from_str(&b).ok().map(|b| (b, v))))
}

/// A new projection: every point is replaced by `points`.
pub fn save_layout(c: &mut Connection, b: &Basis, points: &[(String, String, f32, f32)], now: i64) -> AppResult<()> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM sound_map", [])?;
    tx.execute(
        "INSERT OR REPLACE INTO sound_map_layout (id, version, basis, built_at) VALUES (1, ?1, ?2, ?3)",
        params![LAYOUT_VERSION, serde_json::to_string(b).unwrap_or_default(), now],
    )?;
    insert(&tx, points)?;
    tx.commit()?;
    Ok(())
}

/// More points with the projection in use.
pub fn add_points(c: &mut Connection, points: &[(String, String, f32, f32)]) -> AppResult<()> {
    let tx = c.transaction()?;
    insert(&tx, points)?;
    tx.commit()?;
    Ok(())
}

fn insert(c: &Connection, points: &[(String, String, f32, f32)]) -> AppResult<()> {
    let mut st = c.prepare_cached("INSERT OR REPLACE INTO sound_map (track_id, part_key, x, y) VALUES (?1, ?2, ?3, ?4)")?;
    for (id, pk, x, y) in points {
        st.execute(params![id, pk, *x as f64, *y as f64])?;
    }
    Ok(())
}

/// Drop points whose song left the library or whose file changed.
pub fn prune(c: &Connection) -> AppResult<usize> {
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
    let mut sql = String::from(
        "SELECT m.track_id, m.x, m.y, t.genres, al.genres, al.year, t.album_id, coalesce(f.version, f.release),
                al.format, al.subformats, al.matched, al.studio, mc.found, coalesce(at.version, at.release), al.id IS NOT NULL
         FROM sound_map m JOIN tracks t ON t.id = m.track_id
         LEFT JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'
         LEFT JOIN albums al ON al.id = t.album_id
         LEFT JOIN album_mb_check mc ON mc.album_id = t.album_id
         LEFT JOIN album_tags at ON at.album_id = t.album_id
         WHERE m.track_id > ?1",
    );
    let after = after.unwrap_or("").to_string();
    let mut args: Vec<Box<dyn ToSql>> = vec![Box::new(after)];
    if let Some(a) = area {
        sql.push_str(" AND m.x >= ?2 AND m.x <= ?3 AND m.y >= ?4 AND m.y <= ?5");
        for v in [a.min_x, a.max_x, a.min_y, a.max_y] {
            args.push(Box::new(v as f64));
        }
    }
    sql.push_str(&format!(" ORDER BY m.track_id LIMIT {}", limit + 1));
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
        let genre = first_genre(r.get(3)?).or_else(|| first_genre(r.get(4).ok().flatten()));
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
