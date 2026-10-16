//! What a file says about where it came from: its release status and its
//! provenance note, as upgrades see them.
//!
//! Upgrades only ask through [`StatusReader`]. [`ArchiveStatus`] reads the
//! `ARCHIVE_*` tags the scan already parses (`ARCHIVE_VERSION` for the
//! status, `ARCHIVE_SOURCE` for the note); a fuller reader (plain STATUS and
//! VERSION tags, Picard's release status) plugs in behind the same trait.

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

/// The stub over the `ARCHIVE_*` fields.
pub struct ArchiveStatus;

impl StatusReader for ArchiveStatus {
    fn file_status(&self, tags: &FileTags) -> FileStatus {
        let status = match tags.version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            Some(v) if v.eq_ignore_ascii_case("released") => ReleaseStatus::Released,
            Some(_) => ReleaseStatus::Unreleased,
            None => ReleaseStatus::Unknown,
        };
        FileStatus { status, note: tags.source.clone().filter(|s| !s.trim().is_empty()), note_tag: NoteTag::ArchiveSource }
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
    fn the_stub_reads_archive_version_and_source() {
        let r = ArchiveStatus;
        let t = |v: Option<&str>, s: Option<&str>| FileTags { version: v.map(str::to_string), source: s.map(str::to_string), ..Default::default() };
        assert_eq!(r.file_status(&t(Some("released"), Some("Official CD"))).status, ReleaseStatus::Released);
        assert_eq!(r.file_status(&t(Some("leak"), None)).status, ReleaseStatus::Unreleased);
        assert_eq!(r.file_status(&t(None, None)).status, ReleaseStatus::Unknown);
        assert_eq!(r.file_status(&t(None, Some("Official CD"))).note.as_deref(), Some("Official CD"));
        assert!(note_is_official(Some("  official CD rip"), "Official"));
        assert!(!note_is_official(Some("Unofficial bootleg"), "Official"));
        assert!(!note_is_official(None, "Official"));
        assert!(!note_is_official(Some("Official"), " "));
    }
}
