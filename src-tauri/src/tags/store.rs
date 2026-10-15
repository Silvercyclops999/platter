//! The file tag cache in SQLite and what's derived from it.
//!
//! `file_tags` has one row per media part key. Plex part keys carry the
//! part's update stamp (`/library/parts/{id}/{stamp}/file.ext`), so a file
//! that changes gets a new key: the old row is pruned and the new one read.
//! `album_tags` is rebuilt from it after each scan batch: an album's explicit
//! flag, its most common version, edition and group, whether every read song
//! is a fan or AI edit, and which album is the face of its group.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::db::release::is_unreleased;
use crate::error::AppResult;
use crate::model::Track;

use super::{FileTags, TagView, is_edit};

/// How a read ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// The file couldn't be read or parsed. Tried again when it changes.
    Failed,
    /// The server refused Range requests and the file is too big to read whole.
    Skipped,
}

impl Status {
    fn key(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Failed => "failed",
            Status::Skipped => "skipped",
        }
    }
}

/// A track whose file hasn't been read yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub track_id: String,
    pub part_key: String,
    pub container: Option<String>,
}

pub fn save(c: &Connection, part_key: &str, status: Status, t: &FileTags, now: i64) -> AppResult<()> {
    c.execute(
        "INSERT OR REPLACE INTO file_tags
             (part_key, status, explicit, version, era, quality, date, edition, grp, source, flags, scanned_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            part_key,
            status.key(),
            t.explicit.map(|e| e as i64),
            t.version,
            t.era,
            t.quality,
            t.date,
            t.edition,
            t.group,
            t.source,
            t.flags,
            now
        ],
    )?;
    Ok(())
}

/// Read tags for one part key (tests and diagnostics).
pub fn get(c: &Connection, part_key: &str) -> AppResult<Option<(String, FileTags)>> {
    Ok(c.query_row(
        "SELECT status, explicit, version, era, quality, date, edition, grp, source, flags
         FROM file_tags WHERE part_key = ?1",
        [part_key],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                FileTags {
                    explicit: r.get::<_, Option<i64>>(1)?.map(|e| e != 0),
                    version: r.get(2)?,
                    era: r.get(3)?,
                    quality: r.get(4)?,
                    date: r.get(5)?,
                    edition: r.get(6)?,
                    group: r.get(7)?,
                    source: r.get(8)?,
                    flags: r.get(9)?,
                },
            ))
        },
    )
    .optional()?)
}

/// Tracks still to read, album by album.
pub fn pending(c: &Connection, limit: usize) -> AppResult<Vec<Pending>> {
    let mut st = c.prepare_cached(
        "SELECT t.id, t.part_key, t.container FROM tracks t
         LEFT JOIN file_tags f ON f.part_key = t.part_key
         WHERE f.part_key IS NULL AND t.part_key <> ''
         ORDER BY t.album_id, t.disc_number, t.track_number LIMIT ?1",
    )?;
    let rows = st.query_map([limit as i64], |r| {
        Ok(Pending { track_id: r.get(0)?, part_key: r.get(1)?, container: r.get(2)? })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Drops rows for files no longer in the library (or changed: new key).
pub fn prune(c: &Connection) -> AppResult<usize> {
    Ok(c.execute("DELETE FROM file_tags WHERE part_key NOT IN (SELECT part_key FROM tracks)", [])?)
}

/// Scan progress: tracks read (any outcome), all tracks, reads that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub done: u64,
    pub total: u64,
    pub failed: u64,
}

pub fn counts(c: &Connection) -> AppResult<Counts> {
    let total: i64 = c.query_row("SELECT count(*) FROM tracks", [], |r| r.get(0))?;
    let (done, failed): (i64, i64) = c.query_row(
        "SELECT count(*), COALESCE(sum(f.status <> 'ok'), 0)
         FROM tracks t JOIN file_tags f ON f.part_key = t.part_key",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(Counts { done: done as u64, total: total as u64, failed: failed as u64 })
}

/// The most common value (first seen wins a tie).
fn most_common(values: &[String]) -> Option<String> {
    let mut best: Option<(&String, usize)> = None;
    for v in values {
        let n = values.iter().filter(|x| *x == v).count();
        if best.is_none_or(|b| n > b.1) {
            best = Some((v, n));
        }
    }
    best.map(|b| b.0.clone())
}

#[derive(Default)]
struct AlbumAcc {
    explicit: bool,
    versions: Vec<String>,
    editions: Vec<String>,
    groups: Vec<String>,
    read: usize,
    edits: usize,
}

/// One album of a group, for choosing the face.
#[derive(Debug, Clone)]
pub struct Member {
    pub album_id: String,
    pub edition: Option<String>,
    pub version: Option<String>,
    pub unreleased_type: bool,
    pub year: Option<i32>,
    pub added_at: i64,
}

/// The album a group shows as: the edition marked Standard, else a
/// released one, else the earliest.
pub fn face(members: &[Member]) -> Option<&Member> {
    members.iter().min_by_key(|m| {
        let standard = m.edition.as_deref().is_some_and(|e| e.trim().eq_ignore_ascii_case("standard"));
        let released = match m.version.as_deref() {
            Some(v) => v == "released",
            None => !m.unreleased_type,
        };
        let rank = if standard { 0 } else if released { 1 } else { 2 };
        (rank, m.year.is_none(), m.year, m.added_at, m.album_id.clone())
    })
}

/// Rebuilds `album_tags` from the file tags read so far.
pub fn refresh_albums(c: &Connection) -> AppResult<()> {
    let mut acc: HashMap<String, AlbumAcc> = HashMap::new();
    {
        let mut st = c.prepare_cached(
            "SELECT t.album_id, f.explicit, f.version, f.edition, f.grp, f.flags
             FROM tracks t JOIN file_tags f ON f.part_key = t.part_key WHERE f.status = 'ok'",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let a = acc.entry(r.get::<_, String>(0)?).or_default();
            a.read += 1;
            if r.get::<_, Option<i64>>(1)? == Some(1) {
                a.explicit = true;
            }
            if let Some(v) = r.get::<_, Option<String>>(2)? {
                a.versions.push(v);
            }
            if let Some(v) = r.get::<_, Option<String>>(3)? {
                a.editions.push(v);
            }
            if let Some(v) = r.get::<_, Option<String>>(4)? {
                a.groups.push(v);
            }
            if is_edit(r.get::<_, Option<String>>(5)?.as_deref()) {
                a.edits += 1;
            }
        }
    }
    // Album facts for choosing each group's face.
    let mut info = c.prepare_cached(
        "SELECT al.year, al.added_at, al.format, al.subformats, al.studio, al.matched, mc.found FROM albums al
         LEFT JOIN album_mb_check mc ON mc.album_id = al.id WHERE al.id = ?1",
    )?;
    let mut groups: HashMap<String, Vec<Member>> = HashMap::new();
    for (id, a) in &acc {
        if let Some(g) = most_common(&a.groups) {
            let row = info
                .query_row([id], |r| {
                    Ok((
                        r.get::<_, Option<i32>>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<bool>>(5)?,
                        r.get::<_, Option<bool>>(6)?,
                    ))
                })
                .optional()?;
            let Some((year, added_at, format, subs, studio, matched, mb)) = row else { continue };
            let subs: Vec<String> = serde_json::from_str(&subs).unwrap_or_default();
            groups.entry(g).or_default().push(Member {
                album_id: id.clone(),
                edition: most_common(&a.editions),
                version: most_common(&a.versions),
                unreleased_type: is_unreleased(format.as_deref(), &subs, matched, studio.as_deref(), None, mb),
                year,
                added_at,
            });
        }
    }
    let faces: HashMap<String, String> = groups
        .iter()
        .filter_map(|(g, m)| face(m).map(|f| (g.clone(), f.album_id.clone())))
        .collect();
    c.execute("DELETE FROM album_tags", [])?;
    let mut ins = c.prepare_cached(
        "INSERT INTO album_tags (album_id, explicit, version, edition, grp, flagged, face_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for (id, a) in acc {
        let grp = most_common(&a.groups);
        let face_id = grp.as_ref().and_then(|g| faces.get(g)).cloned();
        let flagged = a.read > 0 && a.edits == a.read;
        if !a.explicit && a.versions.is_empty() && a.editions.is_empty() && grp.is_none() && !flagged {
            continue;
        }
        ins.execute(params![
            id,
            a.explicit as i64,
            most_common(&a.versions),
            most_common(&a.editions),
            grp,
            flagged as i64,
            face_id
        ])?;
    }
    Ok(())
}

/// SQL condition (on albums aliased `al`) that leaves out albums the view
/// hides: editions behind their group's face, and albums of only fan or AI
/// edits. The view's switches go in as literal 0/1.
pub fn visible_clause(view: TagView) -> String {
    format!(
        "al.id NOT IN (SELECT a.album_id FROM album_tags a WHERE
            ({g} AND a.grp IS NOT NULL AND a.face_id IS NOT NULL AND a.face_id <> a.album_id
                 AND a.face_id IN (SELECT id FROM albums))
            OR ({e} AND a.flagged = 1))",
        g = view.group_editions as u8,
        e = (!view.show_edits) as u8,
    )
}

/// Album ids the view hides (see [`visible_clause`]).
pub fn hidden_albums(c: &Connection, view: TagView) -> AppResult<HashSet<String>> {
    let sql = format!(
        "SELECT al.id FROM albums al WHERE NOT ({})",
        visible_clause(view)
    );
    let mut st = c.prepare_cached(&sql)?;
    Ok(st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
}

/// What was left out of a play context.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skipped {
    pub explicit: usize,
    pub edits: usize,
}

/// Takes explicit songs (when filtered) and fan or AI edits (when hidden)
/// out of a list about to play.
pub fn drop_hidden(c: &Connection, tracks: &mut Vec<Track>, view: TagView) -> AppResult<Skipped> {
    let mut skipped = Skipped::default();
    if !view.filter_explicit && view.show_edits {
        return Ok(skipped);
    }
    let mut st = c.prepare_cached("SELECT explicit, flags FROM file_tags WHERE part_key = ?1 AND status = 'ok'")?;
    let mut keep = Vec::with_capacity(tracks.len());
    for t in tracks.drain(..) {
        let row: Option<(Option<i64>, Option<String>)> =
            st.query_row([&t.part_key], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let (explicit, flags) = row.unwrap_or_default();
        if view.filter_explicit && explicit == Some(1) {
            skipped.explicit += 1;
        } else if !view.show_edits && is_edit(flags.as_deref()) {
            skipped.edits += 1;
        } else {
            keep.push(t);
        }
    }
    *tracks = keep;
    Ok(skipped)
}

/// Takes fan and AI edits out of a list shown in the UI (album pages).
pub fn drop_edits(c: &Connection, tracks: &mut Vec<Track>, view: TagView) -> AppResult<()> {
    if view.show_edits {
        return Ok(());
    }
    let mut st = c.prepare_cached("SELECT flags FROM file_tags WHERE part_key = ?1 AND status = 'ok'")?;
    let mut keep = Vec::with_capacity(tracks.len());
    for t in tracks.drain(..) {
        let flags: Option<Option<String>> = st.query_row([&t.part_key], |r| r.get(0)).optional()?;
        if !is_edit(flags.flatten().as_deref()) {
            keep.push(t);
        }
    }
    *tracks = keep;
    Ok(())
}

/// ARCHIVE_VERSION of a track's file, if read.
pub fn track_version(c: &Connection, track_id: &str) -> AppResult<Option<String>> {
    Ok(c.query_row(
        "SELECT f.version FROM tracks t JOIN file_tags f ON f.part_key = t.part_key WHERE t.id = ?1",
        [track_id],
        |r| r.get(0),
    )
    .optional()?
    .flatten())
}

/// The most common ARCHIVE_VERSION on an album's files, if any.
pub fn album_version(c: &Connection, album_id: &str) -> AppResult<Option<String>> {
    Ok(c.query_row("SELECT version FROM album_tags WHERE album_id = ?1", [album_id], |r| r.get(0))
        .optional()?
        .flatten())
}

// ------------------------------------------------------------------ UI index

/// Badges for one song (only songs with something to show are listed).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackBadges {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub explicit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    /// A fan or AI edit.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub edit: bool,
}

/// What the tags say about one album.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumBadges {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub explicit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
}

/// One edition in a group, for the album page's picker.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edition {
    pub album_id: String,
    pub title: String,
    /// ARCHIVE_EDITION, when tagged.
    pub label: Option<String>,
    pub year: Option<i32>,
}

/// Everything the UI needs for badges, edition cards and the picker
/// (`tags_index`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagIndex {
    pub tracks: HashMap<String, TrackBadges>,
    pub albums: HashMap<String, AlbumBadges>,
    /// Groups with two or more albums, face first.
    pub groups: HashMap<String, Vec<Edition>>,
}

pub fn index(c: &Connection) -> AppResult<TagIndex> {
    let mut out = TagIndex::default();
    {
        let mut st = c.prepare_cached(
            "SELECT t.id, f.explicit, f.version, f.quality, f.flags
             FROM tracks t JOIN file_tags f ON f.part_key = t.part_key
             WHERE f.status = 'ok' AND (f.explicit = 1 OR f.version IS NOT NULL OR f.quality IS NOT NULL OR f.flags IS NOT NULL)",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let flags: Option<String> = r.get(4)?;
            out.tracks.insert(
                r.get(0)?,
                TrackBadges {
                    explicit: r.get::<_, Option<i64>>(1)? == Some(1),
                    version: r.get(2)?,
                    quality: r.get(3)?,
                    edit: is_edit(flags.as_deref()),
                },
            );
        }
    }
    let mut faces: HashMap<String, String> = HashMap::new();
    {
        let mut st = c.prepare_cached(
            "SELECT a.album_id, a.explicit, a.version, a.grp, a.edition, a.face_id, al.title, al.year, al.added_at
             FROM album_tags a JOIN albums al ON al.id = a.album_id",
        )?;
        let mut rows = st.query([])?;
        let mut members: HashMap<String, Vec<(Edition, i64)>> = HashMap::new();
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let group: Option<String> = r.get(3)?;
            let edition: Option<String> = r.get(4)?;
            if let Some(g) = &group {
                if let Some(f) = r.get::<_, Option<String>>(5)? {
                    faces.insert(g.clone(), f);
                }
                members.entry(g.clone()).or_default().push((
                    Edition { album_id: id.clone(), title: r.get(6)?, label: edition.clone(), year: r.get(7)? },
                    r.get(8)?,
                ));
            }
            out.albums.insert(
                id,
                AlbumBadges { explicit: r.get::<_, i64>(1)? != 0, version: r.get(2)?, group, edition },
            );
        }
        for (g, mut m) in members {
            if m.len() < 2 {
                continue;
            }
            let face = faces.get(&g).cloned().unwrap_or_default();
            m.sort_by(|a, b| {
                (a.0.album_id != face, a.0.year.is_none(), a.0.year, a.1, &a.0.album_id).cmp(&(
                    b.0.album_id != face,
                    b.0.year.is_none(),
                    b.0.year,
                    b.1,
                    &b.0.album_id,
                ))
            });
            out.groups.insert(g, m.into_iter().map(|x| x.0).collect());
        }
    }
    Ok(out)
}
