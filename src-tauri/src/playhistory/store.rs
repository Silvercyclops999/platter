//! Imported plays on disk: mapping each fetched play to a library song,
//! saving it (once per source id), each source's progress, and removing a
//! source's plays.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::listens::{ImportSource, ImportedPlay, insert_imported};
use crate::db::merge::Rank;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::import::SongIn;
use crate::import::matching::{Matcher, Status};

/// One play as a source gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The play's id within its source (a history entry id, a row id).
    pub source_id: String,
    /// Unix ms.
    pub played_at: i64,
    pub ms_played: Option<u64>,
    /// The library song the source named, when it names one.
    pub rating_key: Option<String>,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// The library song a play is: its own song when the library still has it,
/// else the song its names match (as a history file's plays are matched),
/// else none (kept by its names).
fn resolve(c: &Connection, matcher: &mut Matcher<'_>, p: &Fetched) -> AppResult<Option<String>> {
    if let Some(k) = &p.rating_key {
        let known: bool = c.prepare_cached("SELECT EXISTS (SELECT 1 FROM tracks WHERE id = ?1)")?.query_row([k], |r| r.get(0))?;
        if known {
            return Ok(Some(k.clone()));
        }
    }
    let Some(artist) = p.artist.as_deref().filter(|a| !a.trim().is_empty()) else {
        return Ok(None);
    };
    let song = SongIn { artist: artist.to_string(), title: p.title.clone(), album: p.album.clone(), ..Default::default() };
    let o = matcher.find(&song)?;
    Ok(if o.status == Status::Matched { o.track_id } else { None })
}

/// Save fetched plays from `source`, each mapped to a library song where
/// one fits. Plays already saved from the source are skipped. Returns how
/// many were new.
pub fn save(c: &mut Connection, source: ImportSource, plays: &[Fetched], ranks: HashMap<String, Rank>, now_ms: i64) -> AppResult<usize> {
    if plays.is_empty() {
        return Ok(0);
    }
    let rows = {
        let mut known = c.prepare_cached("SELECT EXISTS (SELECT 1 FROM imported_plays WHERE source = ?1 AND source_id = ?2)")?;
        let mut matcher = Matcher::new(c, ranks);
        let mut rows = Vec::with_capacity(plays.len());
        for p in plays {
            if p.title.trim().is_empty() || known.query_row(params![source.as_str(), p.source_id], |r| r.get::<_, bool>(0))? {
                continue;
            }
            rows.push(ImportedPlay {
                source,
                source_id: p.source_id.clone(),
                played_at: p.played_at,
                ms_played: p.ms_played,
                title: p.title.trim().to_string(),
                artist: p.artist.clone().filter(|a| !a.trim().is_empty()),
                album: p.album.clone().filter(|a| !a.trim().is_empty()),
                rating_key: resolve(c, &mut matcher, p)?,
            });
        }
        rows
    };
    insert_imported(c, &rows, now_ms)
}

/// Plays saved from a source, and how many of them aren't in the library.
pub fn counts(c: &Connection, source: ImportSource) -> AppResult<(u64, u64)> {
    Ok(c.query_row("SELECT count(*), count(*) - count(rating_key) FROM imported_plays WHERE source = ?1", [source.as_str()], |r| {
        Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get::<_, i64>(1)?.max(0) as u64))
    })?)
}

/// The newest play saved from a source (unix ms).
pub fn newest(c: &Connection, source: ImportSource) -> AppResult<Option<i64>> {
    Ok(c.query_row("SELECT max(played_at) FROM imported_plays WHERE source = ?1", [source.as_str()], |r| r.get(0)).optional()?.flatten())
}

/// Delete every play saved from a source (nothing else) and forget its
/// progress. Returns how many plays went.
pub fn remove(c: &mut Connection, source: ImportSource) -> AppResult<usize> {
    let tx = c.transaction()?;
    let n = tx.execute("DELETE FROM imported_plays WHERE source = ?1", [source.as_str()])?;
    let p = Progress { removed: true, ..Progress::default() };
    put_progress(&tx, source, &p)?;
    tx.commit()?;
    Ok(n)
}

/// Where a source's import stands (kept in `kv`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Progress {
    /// The whole history has been read once; later runs only read what's new.
    pub full_done: bool,
    /// The page the first read goes on from.
    pub resume: u64,
    /// Last run that finished (unix ms).
    pub updated_at: Option<i64>,
    /// Why the last run stopped, in words for Settings.
    pub error: Option<String>,
    /// When it failed (unix ms); it's tried again later.
    pub error_at: Option<i64>,
    /// Plays left out for being too short to count, all runs.
    pub short_plays: u64,
    /// The user removed this source's plays; nothing is brought in again
    /// until the source is turned back on.
    pub removed: bool,
    /// The user stopped a read; it waits until they ask for an update.
    pub paused: bool,
}

fn key(source: ImportSource) -> String {
    format!("playhistory.{}", source.as_str())
}

pub fn progress(c: &Connection, source: ImportSource) -> AppResult<Progress> {
    Ok(kv_get(c, &key(source))?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
}

pub fn put_progress(c: &Connection, source: ImportSource, p: &Progress) -> AppResult<()> {
    let json = serde_json::to_string(p).map_err(|e| AppError::Internal(e.to_string()))?;
    kv_set(c, &key(source), &json)
}
