//! Analysis results per song (`track_analysis`), keyed by track with the
//! file they came from: a changed file, or a newer analysis version, is
//! done again. Songs that couldn't be decoded are kept as failed so they
//! aren't read again until their file changes (or the analysis improves);
//! songs that couldn't be read at all are tried again a day later.

use std::collections::HashMap;

use rusqlite::{Connection, Row, params};
use serde::Serialize;

use super::features::{Analysis, Grid, MAX_SECONDS, MIN_SECONDS, VERSION};
use crate::error::AppResult;

pub const OK: &str = "ok";
/// The file couldn't be decoded (tried again with a new file or version).
pub const FAILED: &str = "failed";
/// The file couldn't be read (tried again after [`RETRY_UNREAD_S`]).
pub const UNREAD: &str = "unread";
/// How long before a song that couldn't be read is tried again.
pub const RETRY_UNREAD_S: i64 = 24 * 60 * 60;
/// Too long to analyse (a long mix, a concert).
pub const LONG: &str = "long";

/// A song to analyse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Todo {
    pub track_id: String,
    pub part_key: String,
    pub duration_ms: u64,
}

/// Never analysed, a new file, an older analysis version (failed ones get
/// one more try per version), or unreadable a day ago.
fn pending_where() -> String {
    format!(
        "t.part_key <> '' AND t.duration_ms >= ?1
         AND (a.track_id IS NULL OR a.part_key <> t.part_key
              OR (a.status <> '{UNREAD}' AND a.version < ?2)
              OR (a.status = '{UNREAD}' AND a.analysed_at <= CAST(strftime('%s', 'now') AS INTEGER) - {RETRY_UNREAD_S}))"
    )
}

/// Up to `limit` songs to analyse, newest first.
pub fn pending(c: &Connection, limit: usize) -> AppResult<Vec<Todo>> {
    let sql = format!(
        "SELECT t.id, t.part_key, t.duration_ms FROM tracks t LEFT JOIN track_analysis a ON a.track_id = t.id
         WHERE {} ORDER BY t.pk DESC LIMIT ?3",
        pending_where()
    );
    let mut st = c.prepare_cached(&sql)?;
    let min_ms = MIN_SECONDS as i64 * 1000;
    let out = st
        .query_map(params![min_ms, VERSION, limit as i64], |r| Ok(Todo { track_id: r.get(0)?, part_key: r.get(1)?, duration_ms: r.get::<_, i64>(2)?.max(0) as u64 }))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Of these songs, the ones still to analyse, in the order given.
pub fn pending_of(c: &Connection, ids: &[String]) -> AppResult<Vec<Todo>> {
    let sql = format!(
        "SELECT t.id, t.part_key, t.duration_ms FROM tracks t LEFT JOIN track_analysis a ON a.track_id = t.id
         WHERE t.id = ?3 AND {}",
        pending_where()
    );
    let mut st = c.prepare_cached(&sql)?;
    let min_ms = MIN_SECONDS as i64 * 1000;
    let mut out = Vec::new();
    for id in ids {
        let rows = st
            .query_map(params![min_ms, VERSION, id], |r| Ok(Todo { track_id: r.get(0)?, part_key: r.get(1)?, duration_ms: r.get::<_, i64>(2)?.max(0) as u64 }))?
            .collect::<Result<Vec<_>, _>>()?;
        out.extend(rows);
    }
    Ok(out)
}

fn json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

pub fn save(c: &Connection, t: &Todo, a: &Analysis, now: i64) -> AppResult<()> {
    let g = |g: &Option<Grid>| g.map(|g| (g.downbeat_ms, g.beat_ms, g.conf as f64, g.downbeat_conf as f64));
    let (i, o) = (g(&a.intro), g(&a.outro));
    c.execute(
        "INSERT OR REPLACE INTO track_analysis (track_id, part_key, status, version, source, duration_ms, bpm, beat_conf,
             in_downbeat_ms, in_beat_ms, in_conf, in_down_conf, out_downbeat_ms, out_beat_ms, out_conf, out_down_conf,
             musical_key, key_conf, energy, loudness_db, cue_in_ms, cue_out_ms, intro_end_ms, outro_start_ms,
             starts_hot, ends_hot, sections, features, analysed_at)
         VALUES (?1, ?2, 'ok', ?3, 'local', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
             ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
        params![
            t.track_id,
            t.part_key,
            VERSION,
            a.duration_ms as i64,
            a.bpm.map(|b| b as f64),
            a.beat_conf as f64,
            i.map(|g| g.0),
            i.map(|g| g.1),
            i.map(|g| g.2),
            i.map(|g| g.3),
            o.map(|g| g.0),
            o.map(|g| g.1),
            o.map(|g| g.2),
            o.map(|g| g.3),
            a.key.map(|k| k as i64),
            a.key_conf as f64,
            a.energy as f64,
            a.loudness_db as f64,
            a.cue_in_ms as i64,
            a.cue_out_ms as i64,
            a.intro_end_ms as i64,
            a.outro_start_ms as i64,
            a.starts_hot,
            a.ends_hot,
            json(&a.sections),
            json(&a.features),
            now,
        ],
    )?;
    Ok(())
}

/// A song that couldn't be analysed (`status`: failed, unread or long).
pub fn save_status(c: &Connection, t: &Todo, status: &str, now: i64) -> AppResult<()> {
    c.execute(
        "INSERT OR REPLACE INTO track_analysis (track_id, part_key, status, version, analysed_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![t.track_id, t.part_key, status, VERSION, now],
    )?;
    Ok(())
}

const COLUMNS: &str = "a.track_id, a.duration_ms, a.bpm, a.beat_conf, a.in_downbeat_ms, a.in_beat_ms, a.in_conf, a.in_down_conf,
     a.out_downbeat_ms, a.out_beat_ms, a.out_conf, a.out_down_conf, a.musical_key, a.key_conf, a.energy, a.loudness_db,
     a.cue_in_ms, a.cue_out_ms, a.intro_end_ms, a.outro_start_ms, a.starts_hot, a.ends_hot, a.sections, a.features";

fn grid(r: &Row, at: usize) -> rusqlite::Result<Option<Grid>> {
    let (d, b, c, dc): (Option<f64>, Option<f64>, Option<f64>, Option<f64>) = (r.get(at)?, r.get(at + 1)?, r.get(at + 2)?, r.get(at + 3)?);
    Ok(match (d, b) {
        (Some(d), Some(b)) if b > 0.0 => Some(Grid {
            downbeat_ms: d,
            beat_ms: b,
            conf: c.unwrap_or(0.0) as f32,
            downbeat_conf: dc.unwrap_or(0.0) as f32,
        }),
        _ => None,
    })
}

/// A stored JSON column; a damaged one is logged and read as empty.
fn parse_json<T: serde::de::DeserializeOwned + Default>(v: Option<String>, what: &str) -> T {
    match v {
        None => T::default(),
        Some(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            tracing::warn!("song analysis: stored {what} unreadable: {e}");
            T::default()
        }),
    }
}

fn row(r: &Row) -> rusqlite::Result<(String, Analysis)> {
    let u = |v: Option<i64>| v.unwrap_or(0).max(0) as u64;
    let sections: Option<String> = r.get(22)?;
    let features: Option<String> = r.get(23)?;
    Ok((
        r.get(0)?,
        Analysis {
            duration_ms: u(r.get(1)?),
            bpm: r.get::<_, Option<f64>>(2)?.map(|b| b as f32),
            beat_conf: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0) as f32,
            intro: grid(r, 4)?,
            outro: grid(r, 8)?,
            key: r.get::<_, Option<i64>>(12)?.filter(|k| (0..24).contains(k)).map(|k| k as u8),
            key_conf: r.get::<_, Option<f64>>(13)?.unwrap_or(0.0) as f32,
            energy: r.get::<_, Option<f64>>(14)?.unwrap_or(0.5) as f32,
            loudness_db: r.get::<_, Option<f64>>(15)?.unwrap_or(-14.0) as f32,
            cue_in_ms: u(r.get(16)?),
            cue_out_ms: u(r.get(17)?),
            intro_end_ms: u(r.get(18)?),
            outro_start_ms: u(r.get(19)?),
            starts_hot: r.get(20)?,
            ends_hot: r.get(21)?,
            sections: parse_json(sections, "sections"),
            features: parse_json(features, "features"),
        },
    ))
}

/// What's known for these songs (only analyses of their current file).
pub fn get_many(c: &Connection, ids: &[String]) -> AppResult<HashMap<String, Analysis>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM track_analysis a JOIN tracks t ON t.id = a.track_id AND t.part_key = a.part_key
         WHERE a.track_id = ?1 AND a.status = 'ok'"
    );
    let mut st = c.prepare_cached(&sql)?;
    let mut out = HashMap::new();
    for id in ids {
        if out.contains_key(id) {
            continue;
        }
        match st.query_map([id], row)?.next() {
            Some(Ok((k, a))) => {
                out.insert(k, a);
            }
            Some(Err(e)) => tracing::warn!("song analysis: stored row for {id} unreadable: {e}"),
            None => {}
        }
    }
    Ok(out)
}

/// Drop rows for songs no longer in the library. Does nothing while the
/// library is empty (not loaded yet, or being rebuilt).
pub fn prune(c: &Connection) -> AppResult<usize> {
    let any: bool = c.query_row("SELECT EXISTS (SELECT 1 FROM tracks)", [], |r| r.get(0))?;
    if !any {
        return Ok(0);
    }
    Ok(c.execute("DELETE FROM track_analysis WHERE NOT EXISTS (SELECT 1 FROM tracks t WHERE t.id = track_analysis.track_id)", [])?)
}

/// Progress for Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub total: u64,
    /// Analysed, or tried (current file, current version).
    pub done: u64,
}

pub fn counts(c: &Connection) -> AppResult<Counts> {
    Ok(c.query_row(
        &format!(
            "SELECT count(*), COALESCE(sum(a.part_key = t.part_key AND (a.status = '{UNREAD}' OR a.version >= ?2)), 0)
             FROM tracks t LEFT JOIN track_analysis a ON a.track_id = t.id
             WHERE t.part_key <> '' AND t.duration_ms >= ?1"
        ),
        params![MIN_SECONDS as i64 * 1000, VERSION],
        |r| Ok(Counts { total: r.get::<_, i64>(0)? as u64, done: r.get::<_, i64>(1)? as u64 }),
    )?)
}

/// Longest song worth reading (ms), for skipping long ones before reading.
pub const MAX_MS: u64 = MAX_SECONDS as u64 * 1000;
