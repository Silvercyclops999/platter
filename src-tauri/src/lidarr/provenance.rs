//! What a file says about where it came from: its release status and its
//! provenance note, as upgrades see them.
//!
//! Upgrades only ask through [`StatusReader`]. [`TagStatus`] uses the
//! library's provenance reader (`tags::provenance`): ARCHIVE_VERSION first,
//! then a plain STATUS tag, Picard's release status and type, or the custom
//! tag set in Settings. The note is a plain VERSION tag, else ARCHIVE_SOURCE.

use std::path::Path;

use serde::Serialize;

use crate::tags::FileTags;
use crate::tags::parse::{self, Sparse, Step};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReleaseStatus {
    /// A tag says it's the released recording.
    Released,
    /// A tag says it isn't (a demo, leak, session, build...).
    Unreleased,
    /// Nothing says either way.
    Unknown,
}

/// Which tag holds a file's provenance note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteTag {
    /// `ARCHIVE_SOURCE`.
    ArchiveSource,
    /// A plain `VERSION` tag.
    Version,
}

impl NoteTag {
    pub fn key(self) -> &'static str {
        match self {
            NoteTag::ArchiveSource => "ARCHIVE_SOURCE",
            NoteTag::Version => "VERSION",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStatus {
    pub status: ReleaseStatus,
    /// The provenance note ("Official CD, 2015").
    pub note: Option<String>,
    /// Where the note lives, or where one would go.
    pub note_tag: NoteTag,
}

/// A file's release status and provenance note, from its tags.
pub trait StatusReader: Send + Sync {
    fn file_status(&self, tags: &FileTags) -> FileStatus;
}

/// The library's provenance reader, with the settings in effect.
pub struct TagStatus;

impl StatusReader for TagStatus {
    fn file_status(&self, tags: &FileTags) -> FileStatus {
        use crate::tags::provenance::{config, resolve};
        let p = resolve(tags, &config());
        let status = match p.status {
            Some(s) if s.is_released() => ReleaseStatus::Released,
            Some(_) => ReleaseStatus::Unreleased,
            None => ReleaseStatus::Unknown,
        };
        let note_tag = if tags.version_note.is_some() && tags.source.is_none() { NoteTag::Version } else { NoteTag::ArchiveSource };
        FileStatus { status, note: p.note, note_tag }
    }
}

/// Whether a note starts with the prefix that marks an official source
/// (any case, leading spaces ignored).
pub fn note_is_official(note: Option<&str>, prefix: &str) -> bool {
    let p = prefix.trim();
    !p.is_empty() && note.is_some_and(|n| n.trim_start().to_lowercase().starts_with(&p.to_lowercase()))
}

/// The tags of a file on this computer, read with the scan's own parsers
/// (only the parts of the file the parsers ask for are read).
pub fn read_local(path: &Path) -> std::io::Result<FileTags> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let total = f.metadata()?.len();
    let mut first = vec![0u8; (parse::MORE_READ * 4).min(total as usize)];
    f.read_exact(&mut first)?;
    let mut s = Sparse::new(first, Some(total));
    s.container = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    for _ in 0..64 {
        match parse::parse(&s, false) {
            Step::Done(t) => return Ok(t),
            Step::Need { offset, len } => {
                let len = len.min(parse::MAX_BLOCK * 16).min(total.saturating_sub(offset) as usize);
                if len == 0 {
                    break;
                }
                let mut b = vec![0u8; len];
                f.seek(SeekFrom::Start(offset))?;
                f.read_exact(&mut b)?;
                s.add(offset, b);
            }
        }
    }
    match parse::parse(&s, true) {
        Step::Done(t) => Ok(t),
        Step::Need { .. } => Ok(FileTags::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_and_note_come_from_whichever_tags_the_file_uses() {
        let r = TagStatus;
        let t = |v: Option<&str>, s: Option<&str>| FileTags { version: v.map(str::to_string), source: s.map(str::to_string), ..Default::default() };
        assert_eq!(r.file_status(&t(Some("released"), Some("Official CD"))).status, ReleaseStatus::Released);
        assert_eq!(r.file_status(&t(Some("leak"), None)).status, ReleaseStatus::Unreleased);
        assert_eq!(r.file_status(&t(None, None)).status, ReleaseStatus::Unknown);
        let archive = r.file_status(&t(None, Some("Official CD")));
        assert_eq!((archive.note.as_deref(), archive.note_tag), (Some("Official CD"), NoteTag::ArchiveSource));
        // A plain STATUS tag and a VERSION note.
        let plain = FileTags { status_tag: Some("Released".into()), version_note: Some("Official release (CD)".into()), ..Default::default() };
        let f = r.file_status(&plain);
        assert_eq!((f.status, f.note.as_deref(), f.note_tag), (ReleaseStatus::Released, Some("Official release (CD)"), NoteTag::Version));
        // Picard's release status.
        let picard = FileTags { mb_status: Some("bootleg".into()), ..Default::default() };
        assert_eq!(r.file_status(&picard).status, ReleaseStatus::Unreleased);
        // A value no scheme knows is never taken as released.
        let odd = FileTags { status_tag: Some("whatever".into()), ..Default::default() };
        assert_eq!(r.file_status(&odd).status, ReleaseStatus::Unknown);
        assert!(note_is_official(Some("  official CD rip"), "Official"));
        assert!(!note_is_official(Some("Unofficial bootleg"), "Official"));
        assert!(!note_is_official(None, "Official"));
        assert!(!note_is_official(Some("Official"), " "));
    }
}
