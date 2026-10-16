//! One-click fixes. Each one first shows every change it would make; the
//! changes chosen are then saved through the editor, so they're checked,
//! logged and can be undone like any other edit.
//!
//! - Fill disc tags: `1/1` on single-disc folders only. A folder whose
//!   file names or tags say it has more discs is left for the editor.
//! - Release status: suggestions only, one file at a time, from the other
//!   songs on the same disc. Albums often mix statuses on purpose, so
//!   nothing is ticked until the user ticks it.
//! - ISRCs for released songs, from one official release the user picks:
//!   matched by title, artist and length within 1.5 s. A song whose length
//!   differs (a re-edit) or without a confident match is kept as it is.

use std::collections::BTreeMap;

use serde::Serialize;

use super::check::{NamePattern, parse_name, to_file_tags};
use super::editor::{EditFile, EditorData};
use super::fields::Field;
use super::lookup::{CatTrack, Candidate};
use crate::tags::provenance::{self, FileRelease};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FixKind {
    DiscTags,
    ReleaseStatus,
    Isrc,
    Explicit,
}

impl FixKind {
    pub fn label(self, album: &str) -> String {
        match self {
            FixKind::DiscTags => format!("Filled disc tags on {album}"),
            FixKind::ReleaseStatus => format!("Filled release status on {album}"),
            FixKind::Isrc => format!("Added ISRCs on {album}"),
            FixKind::Explicit => format!("Marked {album}"),
        }
    }
}

/// One change a fix would make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub track_id: String,
    pub file_name: String,
    /// The file as read, for the save.
    pub stamp: String,
    pub field: Field,
    pub before: Option<String>,
    pub after: String,
    /// Why this value ("from 4 songs on disc 1", "matched 3:12 vs 3:12").
    pub note: Option<String>,
}

/// A file a fix leaves as it is, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    pub track_id: String,
    pub file_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub kind: FixKind,
    pub label: String,
    pub changes: Vec<Change>,
    pub kept: Vec<Kept>,
    /// Changes start unticked: the user picks each one.
    pub pick_each: bool,
    /// What the preview found, in a sentence.
    pub message: String,
}

fn kept(f: &EditFile, reason: impl Into<String>) -> Kept {
    Kept { track_id: f.track_id.clone(), file_name: f.file_name.clone(), reason: reason.into() }
}

fn change(f: &EditFile, field: Field, after: &str, note: Option<String>) -> Option<Change> {
    Some(Change {
        track_id: f.track_id.clone(),
        file_name: f.file_name.clone(),
        stamp: f.stamp.clone()?,
        field,
        before: f.values.get(&field).cloned(),
        after: after.to_string(),
        note,
    })
}

fn finish(kind: FixKind, title: &str, changes: Vec<Change>, kept: Vec<Kept>, pick_each: bool) -> Preview {
    let message = match (changes.len(), kept.len()) {
        (0, 0) => "Nothing to fill: every song already has it.".to_string(),
        (0, k) => format!("Nothing to fill. {} kept as they are.", super::editor::files(k)),
        (c, 0) => format!("{c} {} to make.", if c == 1 { "change" } else { "changes" }),
        (c, k) => format!("{c} {} to make. {} kept as they are.", if c == 1 { "change" } else { "changes" }, super::editor::files(k)),
    };
    Preview { kind, label: kind.label(title), changes, kept, pick_each, message }
}

fn num(f: &EditFile, k: Field) -> Option<u32> {
    f.values.get(&k).and_then(|v| v.trim().parse().ok())
}

/// Disc tags: `1/1` where a folder is one disc.
pub fn disc_tags(d: &EditorData) -> Preview {
    let files: Vec<&EditFile> = d.files.iter().filter(|f| f.stamp.is_some()).collect();
    // By folder: one album can span folders, and a folder is what's named.
    let mut by_folder: BTreeMap<String, Vec<&EditFile>> = BTreeMap::new();
    for f in &files {
        by_folder.entry(f.server_path.as_deref().map(super::editor::server_folder).unwrap_or_default()).or_default().push(f);
    }
    let mut changes = Vec::new();
    let mut keep = Vec::new();
    for fs in by_folder.values() {
        let need: Vec<&&EditFile> = fs.iter().filter(|f| num(f, Field::DiscNumber).is_none() || num(f, Field::DiscTotal).is_none()).collect();
        if need.is_empty() {
            continue;
        }
        let named_discs = fs.iter().any(|f| parse_name(&f.file_name).0 == NamePattern::DiscTrack);
        let tagged_many = fs.iter().any(|f| num(f, Field::DiscNumber).is_some_and(|d| d > 1) || num(f, Field::DiscTotal).is_some_and(|d| d > 1));
        let folder_named = fs.iter().any(|f| {
            f.server_path.as_deref().map(super::editor::server_folder).is_some_and(|p| {
                let last = p.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
                last.starts_with("disc ") || last.starts_with("cd ") || (last.starts_with("cd") && last[2..].chars().all(|c| c.is_ascii_digit()))
            })
        });
        if named_discs || tagged_many || folder_named {
            for f in need {
                keep.push(kept(f, "This album has more than one disc. Set its disc numbers in the editor."));
            }
            continue;
        }
        for f in need {
            if num(f, Field::DiscNumber).is_none() {
                changes.extend(change(f, Field::DiscNumber, "1", None));
            }
            if num(f, Field::DiscTotal).is_none() {
                changes.extend(change(f, Field::DiscTotal, "1", None));
            }
        }
    }
    finish(FixKind::DiscTags, &d.title, changes, keep, false)
}

/// Release status suggestions from the other songs on the same disc.
pub fn release_status(d: &EditorData) -> Preview {
    let field = d.status_field;
    let disc = |f: &EditFile| num(f, Field::DiscNumber).unwrap_or(1);
    let mut known: BTreeMap<u32, BTreeMap<String, usize>> = BTreeMap::new();
    for f in &d.files {
        if let Some(v) = f.values.get(&field).filter(|v| !v.trim().is_empty()) {
            *known.entry(disc(f)).or_default().entry(v.clone()).or_default() += 1;
        }
    }
    let mut changes = Vec::new();
    let mut keep = Vec::new();
    for f in d.files.iter().filter(|f| f.stamp.is_some() && f.values.get(&field).is_none_or(|v| v.trim().is_empty())) {
        let on_disc = known.get(&disc(f));
        let best = on_disc.and_then(|m| m.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))));
        match best {
            Some((v, n)) => {
                let all: usize = on_disc.map(|m| m.values().sum()).unwrap_or(0);
                let note = if *n == all {
                    format!("Like the other {} on disc {}", if *n == 1 { "song".to_string() } else { format!("{n} songs") }, disc(f))
                } else {
                    format!("{n} of {all} songs on disc {} say so; others differ", disc(f))
                };
                changes.extend(change(f, field, v, Some(note)));
            }
            None => keep.push(kept(f, format!("No other song on disc {} has a status to go by.", disc(f)))),
        }
    }
    finish(FixKind::ReleaseStatus, &d.title, changes, keep, true)
}

/// Released songs without an ISRC: those the ISRC fix looks up.
pub fn wants_isrc(f: &EditFile) -> bool {
    f.stamp.is_some()
        && !f.values.contains_key(&Field::Isrc)
        && provenance::resolve(&to_file_tags(&f.values), &provenance::config()).status == Some(FileRelease::Released)
}

/// Titles compared: any case, featured artists, brackets and punctuation set aside.
pub fn title_key(t: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in t.to_lowercase().chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let cut = [" feat. ", " ft. ", " featuring "].iter().filter_map(|p| out.find(p)).min();
    if let Some(i) = cut {
        out.truncate(i);
    }
    let kept: String = out.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first named artist, compared the same way.
pub fn lead_key(a: &str) -> String {
    let a = a.to_lowercase();
    let cut = [", ", " & ", " feat. ", " ft. ", " featuring ", " x ", "; ", " / ", " and "].iter().filter_map(|p| a.find(p)).min().unwrap_or(a.len());
    title_key(&a[..cut])
}

/// Within this many ms, lengths are the same.
pub const LENGTH_SLACK_MS: i64 = 1500;

fn mmss(ms: i64) -> String {
    let s = (ms + 500) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// ISRCs from the release the user confirmed.
pub fn isrcs(d: &EditorData, release: &Candidate, tracks: &[CatTrack]) -> Preview {
    let mut changes = Vec::new();
    let mut keep = Vec::new();
    for f in d.files.iter().filter(|f| wants_isrc(f)) {
        let title = title_key(f.values.get(&Field::Title).map(String::as_str).unwrap_or(""));
        let artist = lead_key(f.values.get(&Field::Artist).or(f.values.get(&Field::AlbumArtist)).map(String::as_str).unwrap_or(""));
        let same: Vec<&CatTrack> = tracks.iter().filter(|t| title_key(&t.title) == title && !title.is_empty()).collect();
        let by_artist: Vec<&CatTrack> =
            same.iter().copied().filter(|t| artist.is_empty() || lead_key(&t.artist) == artist || lead_key(&release.artist) == artist).collect();
        if by_artist.is_empty() {
            keep.push(kept(f, format!("No song on \"{}\" matches its title and artist.", release.title)));
            continue;
        }
        let within: Vec<&CatTrack> =
            by_artist.iter().copied().filter(|t| t.duration_ms.is_some_and(|l| (l - f.duration_ms).abs() <= LENGTH_SLACK_MS)).collect();
        let Some(first) = within.first() else {
            let theirs = by_artist.iter().find_map(|t| t.duration_ms).map(mmss).unwrap_or_else(|| "unknown".into());
            keep.push(kept(f, format!("Its length ({}) differs from the release's ({theirs}): likely another edit.", mmss(f.duration_ms))));
            continue;
        };
        let isrcs: std::collections::BTreeSet<&str> = within.iter().filter_map(|t| t.isrc.as_deref()).collect();
        match (isrcs.len(), first.isrc.as_deref()) {
            (0, _) => keep.push(kept(f, "The release has no ISRC for this song.")),
            (1, _) => {
                let isrc = isrcs.into_iter().next().unwrap_or_default();
                let note = Some(format!("{} on \"{}\", {}", first.title, release.title, mmss(first.duration_ms.unwrap_or(0))));
                changes.extend(change(f, Field::Isrc, isrc, note));
            }
            _ => keep.push(kept(f, "More than one song on the release could be this one.")),
        }
    }
    finish(FixKind::Isrc, &d.title, changes, keep, false)
}

/// The changes picked, as a save request.
pub fn to_request(label: &str, changes: &[Change]) -> super::editor::SaveRequest {
    let mut by: BTreeMap<String, super::editor::SaveFile> = BTreeMap::new();
    for c in changes {
        let e = by.entry(c.track_id.clone()).or_insert_with(|| super::editor::SaveFile {
            track_id: c.track_id.clone(),
            stamp: c.stamp.clone(),
            edits: Default::default(),
        });
        e.edits.insert(c.field, c.after.clone());
    }
    super::editor::SaveRequest { label: label.to_string(), files: by.into_values().collect() }
}

// ---------------------------------------------------------------- running them

use super::editor::TagEditor;
use super::lookup::LookupError;
use crate::error::{AppError, AppResult};

/// Release candidates for the ISRC fix, and what to say about them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Releases {
    pub candidates: Vec<Candidate>,
    /// Released songs without an ISRC on this album.
    pub wanted: usize,
    pub message: String,
}

pub const LOOKUP_DOWN: &str = "MusicBrainz and Deezer couldn't be reached. Nothing was changed; try again later.";

impl TagEditor {
    fn opened(&self, album_id: &str) -> AppResult<EditorData> {
        let d = self.open(Some(album_id), &[])?;
        if d.needs_music_folder {
            return Err(AppError::Invalid(super::editor::NEEDS_FOLDER.into()));
        }
        Ok(d)
    }

    pub fn fix_preview(&self, kind: FixKind, album_id: &str) -> AppResult<Preview> {
        let d = self.opened(album_id)?;
        Ok(match kind {
            FixKind::DiscTags => disc_tags(&d),
            FixKind::ReleaseStatus => release_status(&d),
            FixKind::Isrc | FixKind::Explicit => return Err(AppError::Invalid("That fix needs more to go on.".into())),
        })
    }

    /// Official releases the album could be, for the user to confirm one.
    pub async fn isrc_releases(&self, album_id: &str) -> AppResult<Releases> {
        let d = self.opened(album_id)?;
        let wanted = d.files.iter().filter(|f| wants_isrc(f)).count();
        if wanted == 0 {
            return Ok(Releases { candidates: Vec::new(), wanted, message: "No released song on this album is missing an ISRC.".into() });
        }
        let album = shared_value(&d, Field::Album).unwrap_or_else(|| d.title.clone());
        let artist = shared_value(&d, Field::AlbumArtist).unwrap_or_default();
        match self.catalog().releases(&artist, &album).await {
            Ok(c) if c.is_empty() => Ok(Releases { candidates: c, wanted, message: "No official release matches this album.".into() }),
            Ok(c) => Ok(Releases { candidates: c, wanted, message: "Pick the release these files are from.".into() }),
            Err(LookupError::Down(e)) => {
                tracing::warn!("ISRC lookup: {e}");
                Err(AppError::Network(LOOKUP_DOWN.into()))
            }
        }
    }

    pub async fn isrc_preview(&self, album_id: &str, release: &Candidate) -> AppResult<Preview> {
        let d = self.opened(album_id)?;
        match self.catalog().tracks(release).await {
            Ok(t) => Ok(isrcs(&d, release, &t)),
            Err(LookupError::Down(e)) => {
                tracing::warn!("ISRC lookup: {e}");
                Err(AppError::Network(LOOKUP_DOWN.into()))
            }
        }
    }

    /// Mark albums explicit or clean: a change for each file that says otherwise.
    pub fn explicit_preview(&self, album_ids: &[String], explicit: bool) -> AppResult<Preview> {
        let want = if explicit { "explicit" } else { "clean" };
        let mut changes = Vec::new();
        let mut keep = Vec::new();
        let mut title = String::new();
        for id in album_ids {
            let d = self.opened(id)?;
            if title.is_empty() {
                title = d.title.clone();
            }
            for f in &d.files {
                if f.stamp.is_none() {
                    keep.push(kept(f, f.error.clone().unwrap_or_else(|| "It couldn't be read.".into())));
                } else if f.values.get(&Field::Explicit).map(String::as_str) != Some(want) {
                    changes.extend(change(f, Field::Explicit, want, None));
                }
            }
        }
        if album_ids.len() > 1 {
            title = format!("{} albums", album_ids.len());
        }
        let mut p = finish(FixKind::Explicit, &title, changes, keep, false);
        p.label = format!("Marked {title} as {want}");
        Ok(p)
    }

    /// An artist's albums whose files carry this era.
    pub fn era_albums(&self, artist_id: &str, era: &str) -> AppResult<Vec<String>> {
        self.db().read(|c| {
            let mut st = c.prepare(
                "SELECT DISTINCT t.album_id FROM tracks t JOIN file_tags ft ON ft.part_key = t.part_key
                 JOIN albums a ON a.id = t.album_id
                 WHERE a.artist_id = ?1 AND ft.era = ?2 ORDER BY a.year, a.sort_title",
            )?;
            let ids = st.query_map(rusqlite::params![artist_id, era], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok(ids)
        })
    }
}

fn shared_value(d: &EditorData, f: Field) -> Option<String> {
    let v: Vec<&super::Values> = d.files.iter().map(|x| &x.values).collect();
    super::editor::shared(&v, f).filter(|s| !s.is_empty())
}
