//! The cache of generated mixes: one row per mix plus its songs in order.
//! Songs gone from the library since are skipped when read.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::store::{TRACK_COLS, kv_get, kv_set, row_track};
use crate::error::AppResult;
use crate::model::Track;

use super::engine::Generated;
use super::library::Lib;
use super::{MixKind, MixSummary};

pub const KEY_DAY: &str = "mixes.day";
pub const KEY_SLOT: &str = "mixes.daylistSlot";
pub const KEY_OPTIONS: &str = "mixes.options";
pub const KEY_TZ: &str = "mixes.tzOffsetMin";
pub const KEY_TRACKS: &str = "mixes.libraryTracks";
/// Covers a collage uses.
const COVERS: usize = 4;

/// A generated mix with its songs as ids, ready to store.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub id: String,
    pub kind: MixKind,
    pub title: String,
    pub description: String,
    pub color: String,
    pub artist_ids: Vec<String>,
    pub tag: Option<String>,
    pub tracks: Vec<(String, bool)>,
}

impl Stored {
    pub fn from_generated(lib: &Lib, g: Generated) -> Stored {
        Stored {
            tracks: g.tracks.iter().map(|(t, f)| (lib.tracks[*t as usize].id.clone(), *f)).collect(),
            id: g.id,
            kind: g.kind,
            title: g.title,
            description: g.description,
            color: g.color,
            artist_ids: g.artist_ids,
            tag: g.tag,
        }
    }
}

fn insert(c: &Connection, m: &Stored, position: usize, day: &str, now_s: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO mixes (id, kind, position, title, description, color, artist_ids, tag, day, generated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            m.id,
            m.kind.key(),
            position as i64,
            m.title,
            m.description,
            m.color,
            serde_json::to_string(&m.artist_ids).unwrap_or_else(|_| "[]".into()),
            m.tag,
            day,
            now_s
        ],
    )?;
    let mut st = c.prepare_cached("INSERT INTO mix_tracks (mix_id, position, track_id, familiar) VALUES (?1, ?2, ?3, ?4)")?;
    for (i, (t, f)) in m.tracks.iter().enumerate() {
        st.execute(params![m.id, i as i64, t, *f])?;
    }
    Ok(())
}

/// Replace every mix.
pub fn save_all(c: &mut Connection, mixes: &[Stored], day: &str, now_s: i64) -> AppResult<()> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM mixes", [])?;
    for (i, m) in mixes.iter().enumerate() {
        insert(&tx, m, i, day, now_s)?;
    }
    tx.commit()?;
    Ok(())
}

/// Replace one mix, keeping its place (or adding it at `position`).
pub fn save_one(c: &mut Connection, m: &Stored, position: usize, day: &str, now_s: i64) -> AppResult<()> {
    let tx = c.transaction()?;
    let pos: Option<i64> = tx.query_row("SELECT position FROM mixes WHERE id = ?1", [&m.id], |r| r.get(0)).optional()?;
    tx.execute("DELETE FROM mixes WHERE id = ?1", [&m.id])?;
    insert(&tx, m, pos.map_or(position, |p| p as usize), day, now_s)?;
    tx.commit()?;
    Ok(())
}

pub fn remove(c: &Connection, id: &str) -> AppResult<()> {
    c.execute("DELETE FROM mixes WHERE id = ?1", [id])?;
    Ok(())
}

pub fn count(c: &Connection) -> AppResult<i64> {
    Ok(c.query_row("SELECT count(*) FROM mixes", [], |r| r.get(0))?)
}

pub fn get_kv(c: &Connection, key: &str) -> AppResult<Option<String>> {
    kv_get(c, key)
}

pub fn set_kv(c: &Connection, key: &str, value: &str) -> AppResult<()> {
    kv_set(c, key, value)
}

/// Song ids of every Daily Mix except `but`.
pub fn daily_track_ids(c: &Connection, but: &str) -> AppResult<HashSet<String>> {
    let mut st = c.prepare_cached(
        "SELECT mt.track_id FROM mix_tracks mt JOIN mixes m ON m.id = mt.mix_id WHERE m.kind = 'daily' AND m.id <> ?1",
    )?;
    Ok(st.query_map([but], |r| r.get(0))?.collect::<Result<_, _>>()?)
}

/// A mix's songs in order, with whether each is a favourite.
pub fn tracks(c: &Connection, id: &str) -> AppResult<(Vec<Track>, Vec<bool>)> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {TRACK_COLS}, mt.familiar FROM mix_tracks mt JOIN tracks t ON t.id = mt.track_id
         WHERE mt.mix_id = ?1 ORDER BY mt.position"
    ))?;
    let n = crate::db::store::TRACK_NCOLS;
    let rows = st
        .query_map([id], |r| Ok((row_track(r)?, r.get::<_, bool>(n)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().unzip())
}

/// A summary from its stored row and songs.
pub fn summary_of(c: &Connection, s: &Stored, tracks: &[Track]) -> AppResult<MixSummary> {
    let mut art = None;
    let mut ar = c.prepare_cached("SELECT thumb FROM artists WHERE id = ?1")?;
    for a in &s.artist_ids {
        if let Some(Some(t)) = ar.query_row([a], |r| r.get::<_, Option<String>>(0)).optional()? {
            art = Some(t);
            break;
        }
    }
    let mut covers: Vec<String> = Vec::new();
    for t in tracks {
        if let Some(th) = &t.thumb
            && !covers.contains(th)
        {
            covers.push(th.clone());
            if covers.len() == COVERS {
                break;
            }
        }
    }
    Ok(MixSummary {
        id: s.id.clone(),
        kind: s.kind,
        title: s.title.clone(),
        description: s.description.clone(),
        color: s.color.clone(),
        art,
        covers,
        artist_ids: s.artist_ids.clone(),
        tag: s.tag.clone(),
        track_count: tracks.len() as u32,
        duration_ms: tracks.iter().map(|t| t.duration_ms).sum(),
        next_update_at: None,
    })
}

fn stored_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Stored> {
    let kind: String = r.get(1)?;
    let artists: String = r.get(5)?;
    Ok(Stored {
        id: r.get(0)?,
        kind: MixKind::from_key(&kind).unwrap_or(MixKind::Genre),
        title: r.get(2)?,
        description: r.get(3)?,
        color: r.get(4)?,
        artist_ids: serde_json::from_str(&artists).unwrap_or_default(),
        tag: r.get(6)?,
        tracks: Vec::new(),
    })
}

const MIX_COLS: &str = "id, kind, title, description, color, artist_ids, tag";

/// A mix, its songs, and which of them are favourites.
pub type MixRows = (MixSummary, Vec<Track>, Vec<bool>);

/// Every stored mix with its songs, in shelf order. Mixes left with too few
/// songs (the library changed) are skipped.
pub fn all(c: &Connection) -> AppResult<Vec<MixRows>> {
    let mut st = c.prepare_cached(&format!("SELECT {MIX_COLS} FROM mixes ORDER BY position"))?;
    let rows = st.query_map([], stored_row)?.collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for s in rows {
        let (tracks, familiar) = self::tracks(c, &s.id)?;
        if tracks.len() < super::engine::MIX_MIN {
            continue;
        }
        out.push((summary_of(c, &s, &tracks)?, tracks, familiar));
    }
    Ok(out)
}

/// One stored mix, if there is one.
pub fn one(c: &Connection, id: &str) -> AppResult<Option<MixRows>> {
    let row = c
        .prepare_cached(&format!("SELECT {MIX_COLS} FROM mixes WHERE id = ?1"))?
        .query_row([id], stored_row)
        .optional()?;
    let Some(s) = row else { return Ok(None) };
    let (tracks, familiar) = self::tracks(c, &s.id)?;
    Ok(Some((summary_of(c, &s, &tracks)?, tracks, familiar)))
}
