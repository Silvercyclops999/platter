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

use super::provenance::{self, FileRelease};
use super::{CACHE_VERSION, FileTags, TagView, is_edit};

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
    let p = provenance::resolve(t, &provenance::config());
    c.execute(
        "INSERT OR REPLACE INTO file_tags
             (part_key, status, explicit, version, era, quality, date, edition, grp, source, flags, scanned_at,
              isrc, recording_mbid, ver, album, album_artist, year,
              tag_status, version_note, mb_status, mb_type, custom_status, custom_key, release, provenance, sleeve)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
                 ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
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
            now,
            t.isrc,
            t.recording_mbid,
            CACHE_VERSION,
            t.album,
            t.album_artist,
            t.year,
            t.status_tag,
            t.version_note,
            t.mb_status,
            t.mb_type,
            t.custom_status,
            t.custom_key.as_deref().unwrap_or(""),
            p.status.map(FileRelease::key),
            p.note,
            (!t.sleeve.is_empty()).then(|| serde_json::to_string(&t.sleeve).ok()).flatten()
        ],
    )?;
    save_credits(c, part_key, &t.sleeve)?;
    Ok(())
}

/// The people a file credits, for credit pages (replaces what was there).
fn save_credits(c: &Connection, part_key: &str, s: &super::sleeve::TagSleeve) -> AppResult<()> {
    c.execute("DELETE FROM tag_credits WHERE part_key = ?1", [part_key])?;
    let mut st = c.prepare_cached(
        "INSERT OR IGNORE INTO tag_credits (part_key, person_key, name, role, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for cr in &s.credits {
        let key = crate::artistinfo::fold_name(&cr.name);
        if key.is_empty() {
            continue;
        }
        st.execute(params![part_key, key, cr.name, cr.role, cr.detail.as_deref().unwrap_or("")])?;
    }
    Ok(())
}

/// What a file carries for the sleeve notes (empty when nothing or not read).
pub fn sleeve(c: &Connection, part_key: &str) -> AppResult<super::sleeve::TagSleeve> {
    let json: Option<String> = c
        .prepare_cached("SELECT sleeve FROM file_tags WHERE part_key = ?1 AND status = 'ok'")?
        .query_row([part_key], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
}

/// The file tags kept for working out a file's status again.
const RAW: &str = "version, source, tag_status, version_note, mb_status, mb_type, custom_status, custom_key";

fn raw_tags(r: &rusqlite::Row, at: usize) -> rusqlite::Result<FileTags> {
    let key: String = r.get(at + 7)?;
    Ok(FileTags {
        version: r.get(at)?,
        source: r.get(at + 1)?,
        status_tag: r.get(at + 2)?,
        version_note: r.get(at + 3)?,
        mb_status: r.get(at + 4)?,
        mb_type: r.get(at + 5)?,
        custom_status: r.get(at + 6)?,
        custom_key: (!key.is_empty()).then_some(key),
        ..Default::default()
    })
}

/// Works out every file's status and note again from the tags already
/// read, after the status settings change. Returns the files whose status
/// or note changed.
pub fn recompute(c: &Connection) -> AppResult<usize> {
    let cfg = provenance::config();
    let mut changed: Vec<(String, Option<&'static str>, Option<String>)> = Vec::new();
    {
        let mut st = c.prepare(&format!("SELECT part_key, release, provenance, {RAW} FROM file_tags WHERE status = 'ok'"))?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let p = provenance::resolve(&raw_tags(r, 3)?, &cfg);
            let key = p.status.map(FileRelease::key);
            let (was, note): (Option<String>, Option<String>) = (r.get(1)?, r.get(2)?);
            if was.as_deref() != key || note != p.note {
                changed.push((r.get(0)?, key, p.note));
            }
        }
    }
    let mut up = c.prepare_cached("UPDATE file_tags SET release = ?2, provenance = ?3 WHERE part_key = ?1")?;
    for (k, rel, note) in &changed {
        up.execute(params![k, rel, note])?;
    }
    Ok(changed.len())
}

/// After the status settings change: every file's status again, then what's
/// derived from it (album tags, the song index, the archive). Returns
/// whether any file's status or note changed.
pub fn restatus(c: &Connection) -> AppResult<bool> {
    if recompute(c)? == 0 {
        return Ok(false);
    }
    refresh_albums(c)?;
    crate::identity::index::rebuild(c)?;
    crate::archive::refresh_all(c)?;
    Ok(true)
}

/// [`restatus`] on its own thread (no tag scan running).
pub fn restatus_soon(db: &crate::db::DbHandle) {
    let db = db.clone();
    std::thread::spawn(move || {
        if let Err(e) = db.write(|c| crate::archive::in_tx(c, |c| restatus(c).map(|_| ()))) {
            tracing::warn!("file status refresh failed: {e}");
        }
    });
}

/// The custom tag name rows are compared with ('' for Auto).
fn custom_key_now() -> String {
    provenance::custom_key().unwrap_or_default()
}

/// Read tags for one part key (tests and diagnostics).
pub fn get(c: &Connection, part_key: &str) -> AppResult<Option<(String, FileTags)>> {
    Ok(c.query_row(
        "SELECT status, explicit, version, era, quality, date, edition, grp, source, flags, isrc, recording_mbid,
                album, album_artist, year, version, source, tag_status, version_note, mb_status, mb_type,
                custom_status, custom_key
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
                    isrc: r.get(10)?,
                    recording_mbid: r.get(11)?,
                    album: r.get(12)?,
                    album_artist: r.get(13)?,
                    year: r.get(14)?,
                    ..raw_tags(r, 15)?
                },
            ))
        },
    )
    .optional()?)
}

/// Tracks still to read, album by album: files never read first, then files
/// an older build read (see [`CACHE_VERSION`]) or read for another custom
/// status tag. Their old row stays in use until the new read replaces it.
pub fn pending(c: &Connection, limit: usize) -> AppResult<Vec<Pending>> {
    let mut st = c.prepare_cached(
        "SELECT t.id, t.part_key, t.container FROM tracks t
         LEFT JOIN file_tags f ON f.part_key = t.part_key
         WHERE (f.part_key IS NULL OR f.ver < ?2 OR f.custom_key <> ?3) AND t.part_key <> ''
         ORDER BY f.part_key IS NOT NULL, t.album_id, t.disc_number, t.track_number LIMIT ?1",
    )?;
    let rows = st.query_map(params![limit as i64, CACHE_VERSION, custom_key_now()], |r| {
        Ok(Pending { track_id: r.get(0)?, part_key: r.get(1)?, container: r.get(2)? })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Drops rows for files no longer in the library (or changed: new key).
pub fn prune(c: &Connection) -> AppResult<usize> {
    let n = c.execute("DELETE FROM file_tags WHERE part_key NOT IN (SELECT part_key FROM tracks)", [])?;
    c.execute("DELETE FROM tag_credits WHERE part_key NOT IN (SELECT part_key FROM file_tags)", [])?;
    Ok(n)
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
         FROM tracks t JOIN file_tags f ON f.part_key = t.part_key AND f.ver >= ?1 AND f.custom_key = ?2",
        params![CACHE_VERSION, custom_key_now()],
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
    /// (disc, status) of each file read.
    releases: Vec<(u32, Option<FileRelease>)>,
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
            "SELECT t.album_id, f.explicit, f.version, f.edition, f.grp, f.flags, t.disc_number, f.release
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
            let disc = r.get::<_, Option<i64>>(6)?.unwrap_or(0).max(0) as u32;
            a.releases.push((disc, r.get::<_, Option<String>>(7)?.as_deref().and_then(FileRelease::from_key)));
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
                version: most_common(&a.versions)
                    .or_else(|| provenance::album_release(&a.releases).status.map(|r| r.key().to_string())),
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
        "INSERT INTO album_tags (album_id, explicit, version, edition, grp, flagged, face_id, release, mixed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    for (id, a) in acc {
        let grp = most_common(&a.groups);
        let face_id = grp.as_ref().and_then(|g| faces.get(g)).cloned();
        let flagged = a.read > 0 && a.edits == a.read;
        let rel = provenance::album_release(&a.releases);
        if !a.explicit && a.versions.is_empty() && a.editions.is_empty() && grp.is_none() && !flagged && rel.status.is_none() {
            continue;
        }
        ins.execute(params![
            id,
            a.explicit as i64,
            most_common(&a.versions),
            most_common(&a.editions),
            grp,
            flagged as i64,
            face_id,
            rel.status.map(FileRelease::key),
            rel.mixed as i64
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

// What the release rule (`db::release`) reads as a "version" in SQL: for a
// file `coalesce(f.version, f.release)` (its ARCHIVE_VERSION, else its
// status from whichever tags it has), for an album `coalesce(at.version,
// at.release)`, and for a song `coalesce(f.version, f.release, at.version,
// at.release)`. Where a check read only the file before, the album's status
// joins only when the album has no ARCHIVE_VERSION, so Platter's own tags
// keep the same answers.

/// One file's status and provenance note, if read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackRelease {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<FileRelease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
}

impl TrackRelease {
    pub fn is_empty(&self) -> bool {
        self.status.is_none() && self.provenance.is_none()
    }
}

/// A track's own status by its file's tags (unknown when not read).
pub fn track_release(c: &Connection, track_id: &str) -> AppResult<TrackRelease> {
    let row: Option<(Option<String>, Option<String>)> = c
        .prepare_cached(
            "SELECT f.release, f.provenance FROM tracks t JOIN file_tags f ON f.part_key = t.part_key
             WHERE t.id = ?1 AND f.status = 'ok'",
        )?
        .query_row([track_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let (s, p) = row.unwrap_or_default();
    Ok(TrackRelease { status: s.as_deref().and_then(FileRelease::from_key), provenance: p })
}

/// An album's status by its files' tags, and whether it's mixed.
pub fn album_release(c: &Connection, album_id: &str) -> AppResult<provenance::AlbumRelease> {
    let row: Option<(Option<String>, i64)> = c
        .prepare_cached("SELECT release, mixed FROM album_tags WHERE album_id = ?1")?
        .query_row([album_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(row
        .map(|(s, m)| provenance::AlbumRelease { status: s.as_deref().and_then(FileRelease::from_key), mixed: m != 0 })
        .unwrap_or_default())
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
    /// The file's release status from whichever tags it has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<FileRelease>,
    /// Where the file came from, as tagged (VERSION, else ARCHIVE_SOURCE).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
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
    /// The album's status by its files' tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<FileRelease>,
    /// Some files are released and some aren't.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mixed: bool,
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
            "SELECT t.id, f.explicit, f.version, f.quality, f.flags, f.release, f.provenance
             FROM tracks t JOIN file_tags f ON f.part_key = t.part_key
             WHERE f.status = 'ok' AND (f.explicit = 1 OR f.version IS NOT NULL OR f.quality IS NOT NULL OR f.flags IS NOT NULL
                   OR f.release IS NOT NULL OR f.provenance IS NOT NULL)",
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
                    release: r.get::<_, Option<String>>(5)?.as_deref().and_then(FileRelease::from_key),
                    provenance: r.get(6)?,
                },
            );
        }
    }
    let mut faces: HashMap<String, String> = HashMap::new();
    {
        let mut st = c.prepare_cached(
            "SELECT a.album_id, a.explicit, a.version, a.grp, a.edition, a.face_id, al.title, al.year, al.added_at,
                    a.release, a.mixed
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
                AlbumBadges {
                    explicit: r.get::<_, i64>(1)? != 0,
                    version: r.get(2)?,
                    group,
                    edition,
                    release: r.get::<_, Option<String>>(9)?.as_deref().and_then(FileRelease::from_key),
                    mixed: r.get::<_, i64>(10)? != 0,
                },
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
