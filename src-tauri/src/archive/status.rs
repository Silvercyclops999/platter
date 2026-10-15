//! Each album's release status with the evidence behind it.
//!
//! The rule is `db::release::assess`; this gathers what it decides from
//! (the album's tags, Plex's match, the MusicBrainz check, the user's mark,
//! folder names and song lengths) and keeps the answer in `album_status`, so
//! artist pages and search can sort by it without working it out again.
//! "Why?" works it out fresh from the same facts.

use std::collections::HashMap;

use rusqlite::{Connection, ToSql, params, params_from_iter};
use serde::Serialize;

use crate::db::release::{self, AlbumFacts, Evidence, Mark, ReleaseStatus, SNIPPET_MS};
use crate::error::AppResult;

const ALBUMS: &str = "SELECT al.id, al.artist_id, al.format, al.subformats, al.matched, al.studio, mc.found, at.version
     FROM albums al
     LEFT JOIN album_mb_check mc ON mc.album_id = al.id
     LEFT JOIN album_tags at ON at.album_id = al.id";

/// The facts for the albums `album_where` picks, with the folder and length
/// hints from the songs `track_where` picks (both take the same arguments).
pub fn load_facts(
    c: &Connection,
    album_where: &str,
    track_where: &str,
    args: &[&dyn ToSql],
) -> AppResult<Vec<(String, String, AlbumFacts)>> {
    let mut out: Vec<(String, String, AlbumFacts)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    {
        let mut st = c.prepare_cached(&format!("{ALBUMS} {album_where}"))?;
        let mut rows = st.query(params_from_iter(args.iter()))?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let subs: Option<String> = r.get(3)?;
            let f = AlbumFacts {
                format: r.get(2)?,
                subformats: serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default(),
                matched: r.get(4)?,
                studio: r.get(5)?,
                mb_found: r.get(6)?,
                version: r.get(7)?,
                mark: release::mark_for(&id, None),
                ..Default::default()
            };
            at.insert(id.clone(), out.len());
            out.push((id, r.get(1)?, f));
        }
    }
    let keywords = release::folder_keywords();
    let mut st = c.prepare_cached(&format!("SELECT t.album_id, t.file, t.duration_ms FROM tracks t {track_where}"))?;
    let mut rows = st.query(params_from_iter(args.iter()))?;
    while let Some(r) = rows.next()? {
        let album: String = r.get(0)?;
        let Some(&i) = at.get(&album) else { continue };
        let f = &mut out[i].2;
        let file: Option<String> = r.get(1)?;
        let ms = r.get::<_, i64>(2)?.max(0) as u64;
        f.songs += 1;
        if ms > 0 && ms < SNIPPET_MS {
            f.short_songs += 1;
        }
        if let Some(path) = file.as_deref() {
            for k in release::folder_hits(path, &keywords) {
                if !f.folders.contains(&k) {
                    f.folders.push(k);
                }
            }
        }
    }
    Ok(out)
}

fn write(c: &Connection, rows: &[(String, String, AlbumFacts)]) -> AppResult<usize> {
    let mut ins = c.prepare_cached(
        "INSERT OR REPLACE INTO album_status (album_id, artist_id, status, evidence, score) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (id, artist, f) in rows {
        let (s, ev) = release::assess(f);
        ins.execute(params![id, artist, s.key(), serde_json::to_string(&ev).unwrap_or_else(|_| "[]".into()), release::score(&ev)])?;
    }
    Ok(rows.len())
}

/// Work out and store the status of every album (None), or of these album
/// artists' albums. Returns the albums written.
pub fn refresh(c: &Connection, artists: Option<&[String]>) -> AppResult<usize> {
    match artists {
        None => {
            c.execute("DELETE FROM album_status", [])?;
            let rows = load_facts(c, "", "", &[])?;
            write(c, &rows)
        }
        Some(ids) => {
            let mut n = 0;
            for id in ids {
                c.execute("DELETE FROM album_status WHERE artist_id = ?1", [id])?;
                let rows = load_facts(c, "WHERE al.artist_id = ?1", "WHERE t.artist_id = ?1", &[id])?;
                n += write(c, &rows)?;
            }
            Ok(n)
        }
    }
}

/// What "Why?" shows (IPC `AlbumReleaseStatus`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumReleaseStatus {
    pub id: String,
    pub artist_id: String,
    pub status: ReleaseStatus,
    /// The evidence as plain sentences (what older pages read).
    pub reasons: Vec<String>,
    /// Every piece of evidence, with its weight.
    pub evidence: Vec<Evidence>,
    /// The user's mark on the album, if any.
    pub mark: Option<Mark>,
    /// The weights summed: how strongly it leans unreleased.
    pub score: i32,
}

/// An album's status worked out now, with everything behind it.
pub fn album_status(c: &Connection, album_id: &str) -> AppResult<Option<AlbumReleaseStatus>> {
    let rows = load_facts(c, "WHERE al.id = ?1", "WHERE t.album_id = ?1", &[&album_id])?;
    Ok(rows.into_iter().next().map(|(id, artist_id, f)| {
        let (status, evidence) = release::assess(&f);
        AlbumReleaseStatus {
            reasons: evidence.iter().map(|e| e.text.clone()).collect(),
            score: release::score(&evidence),
            mark: f.mark,
            id,
            artist_id,
            status,
            evidence,
        }
    }))
}

/// Stored statuses of an artist's albums; albums not stored yet are worked
/// out now.
pub fn for_artist(c: &Connection, artist_id: &str, album_ids: &[String]) -> AppResult<HashMap<String, ReleaseStatus>> {
    let mut out: HashMap<String, ReleaseStatus> = HashMap::new();
    {
        let mut st = c.prepare_cached("SELECT album_id, status FROM album_status WHERE artist_id = ?1")?;
        for row in st.query_map([artist_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, s) = row?;
            out.insert(id, ReleaseStatus::from_key(&s));
        }
    }
    for id in album_ids {
        if !out.contains_key(id)
            && let Some(s) = album_status(c, id)?
        {
            out.insert(id.clone(), s.status);
        }
    }
    Ok(out)
}
