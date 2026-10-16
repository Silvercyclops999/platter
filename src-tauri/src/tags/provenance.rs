//! Each file's release status, from whichever tags the library uses
//! (docs/TAGGING.md "Release status").
//!
//! Libraries mark official and unofficial material in different ways:
//! Platter's own ARCHIVE_VERSION, a plain STATUS tag (with VERSION as a
//! free-text note), or the release status MusicBrainz Picard writes. The
//! reader keeps the raw values ([`FileTags`]); [`resolve`] turns them into
//! one [`FileRelease`] per file, tried in that order, and [`album_release`]
//! rates a whole album from its files. A value no scheme knows stays
//! unknown: it is never taken as released.
//!
//! Settings can point the reader at a custom tag instead of STATUS and
//! RELEASESTATUS, and map its values ([`Config`]). ARCHIVE_VERSION always
//! comes first, so Platter's own tags work as before.

use serde::{Deserialize, Serialize};

use super::FileTags;

/// One file's release status. Unknown is `None` wherever it's kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileRelease {
    Released,
    Unreleased,
    Leaked,
    /// A demo, alternate take, reference track, snippet or original file.
    Alternate,
}

impl FileRelease {
    pub const ALL: [FileRelease; 4] = [Self::Released, Self::Unreleased, Self::Leaked, Self::Alternate];

    /// Stable string stored in SQLite and sent over IPC.
    pub fn key(self) -> &'static str {
        match self {
            Self::Released => "released",
            Self::Unreleased => "unreleased",
            Self::Leaked => "leaked",
            Self::Alternate => "alternate",
        }
    }

    pub fn from_key(k: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == k.trim())
    }

    pub fn is_released(self) -> bool {
        self == Self::Released
    }

    /// How "Why?" names it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Released => "released",
            Self::Unreleased => "unreleased",
            Self::Leaked => "leaked",
            Self::Alternate => "alternate versions",
        }
    }
}

/// One custom value and what it means (Settings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueMap {
    pub value: String,
    pub status: FileRelease,
}

/// Where the release status is read from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// A custom tag name, upper case. None tries ARCHIVE_VERSION, STATUS,
    /// RELEASESTATUS and RELEASETYPE in that order.
    pub tag: Option<String>,
    /// Values mapped by the user, checked before the built-in ones.
    pub values: Vec<ValueMap>,
}

impl Config {
    /// From the settings: an empty or "auto" tag name means Auto.
    pub fn new(tag: &str, values: &[ValueMap]) -> Self {
        Self { tag: clean_tag(tag), values: values.to_vec() }
    }
}

/// A tag name as the reader compares it (upper case, trimmed), or None for
/// Auto.
pub fn clean_tag(tag: &str) -> Option<String> {
    let t = tag.trim().trim_end_matches('\0').trim();
    (!t.is_empty() && !t.eq_ignore_ascii_case("auto")).then(|| t.to_ascii_uppercase())
}

// The settings in effect. Tests keep them per thread.
#[cfg(not(test))]
static CONFIG: std::sync::LazyLock<parking_lot::RwLock<Config>> = std::sync::LazyLock::new(Default::default);
#[cfg(test)]
thread_local! {
    static CONFIG: std::cell::RefCell<Config> = Default::default();
}

pub fn config() -> Config {
    #[cfg(not(test))]
    return CONFIG.read().clone();
    #[cfg(test)]
    return CONFIG.with(|c| c.borrow().clone());
}

/// Set from the settings. Returns whether anything changed.
pub fn set_config(cfg: Config) -> bool {
    #[cfg(not(test))]
    {
        let mut cur = CONFIG.write();
        let changed = *cur != cfg;
        *cur = cfg;
        changed
    }
    #[cfg(test)]
    CONFIG.with(|c| {
        let changed = *c.borrow() != cfg;
        *c.borrow_mut() = cfg;
        changed
    })
}

/// The custom tag the reader looks for now, if any.
pub fn custom_key() -> Option<String> {
    config().tag
}

/// A value folded for comparing: lower case, `_` and `-` as spaces, one
/// space between words, outer quotes and punctuation dropped.
pub fn fold_value(v: &str) -> String {
    let s: String = v.trim().to_lowercase().chars().map(|c| if matches!(c, '_' | '-') { ' ' } else { c }).collect();
    let s = s.trim_matches(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '.' | ':' | ';' | ',' | '!' | '*'));
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The value with anything in brackets left out: "Official release (Tidal
/// rip)" is "official release".
fn without_brackets(v: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in v.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    fold_value(&out)
}

/// The built-in values (folded), any case.
const SYNONYMS: &[(&str, FileRelease)] = &[
    ("released", FileRelease::Released),
    ("release", FileRelease::Released),
    ("official", FileRelease::Released),
    ("official release", FileRelease::Released),
    ("officially released", FileRelease::Released),
    ("retail", FileRelease::Released),
    ("retail release", FileRelease::Released),
    ("commercial", FileRelease::Released),
    ("commercial release", FileRelease::Released),
    ("promotion", FileRelease::Released),
    ("promo", FileRelease::Released),
    ("promotional", FileRelease::Released),
    ("withdrawn", FileRelease::Released),
    ("unreleased", FileRelease::Unreleased),
    ("not released", FileRelease::Unreleased),
    ("never released", FileRelease::Unreleased),
    ("unofficial", FileRelease::Unreleased),
    ("unofficial release", FileRelease::Unreleased),
    ("bootleg", FileRelease::Unreleased),
    ("cancelled", FileRelease::Unreleased),
    ("canceled", FileRelease::Unreleased),
    ("scrapped", FileRelease::Unreleased),
    ("shelved", FileRelease::Unreleased),
    ("vault", FileRelease::Unreleased),
    ("leak", FileRelease::Leaked),
    ("leaked", FileRelease::Leaked),
    ("leaks", FileRelease::Leaked),
    ("alt", FileRelease::Alternate),
    ("alt version", FileRelease::Alternate),
    ("alternate", FileRelease::Alternate),
    ("alternate version", FileRelease::Alternate),
    ("alternative", FileRelease::Alternate),
    ("alternative version", FileRelease::Alternate),
    ("alternate take", FileRelease::Alternate),
    ("demo", FileRelease::Alternate),
    ("og", FileRelease::Alternate),
    ("og file", FileRelease::Alternate),
    ("og version", FileRelease::Alternate),
    ("reference", FileRelease::Alternate),
    ("reference track", FileRelease::Alternate),
    ("ref", FileRelease::Alternate),
    ("snippet", FileRelease::Alternate),
    ("session", FileRelease::Alternate),
    ("session file", FileRelease::Alternate),
    ("early version", FileRelease::Alternate),
    ("rough mix", FileRelease::Alternate),
];

/// A built-in value: the whole value, else the value without its brackets.
pub fn synonym(v: &str) -> Option<FileRelease> {
    let find = |f: &str| SYNONYMS.iter().find(|(s, _)| *s == f).map(|x| x.1);
    find(&fold_value(v)).or_else(|| find(&without_brackets(v)))
}

/// A value by the user's mapping first, then the built-in values.
pub fn map_value(v: &str, cfg: &Config) -> Option<FileRelease> {
    let f = fold_value(v);
    if f.is_empty() {
        return None;
    }
    let b = without_brackets(v);
    let user = |x: &str| cfg.values.iter().find(|m| fold_value(&m.value) == x).map(|m| m.status);
    user(&f).or_else(|| user(&b)).or_else(|| synonym(v))
}

/// Platter's own ARCHIVE_VERSION. Its unreleased kinds (see
/// `db::release::version_is_unreleased`) map to their status; every other
/// version is a released one (remix, live), as the album rule has it.
pub fn from_archive_version(v: &str) -> Option<FileRelease> {
    let f = fold_value(v);
    if f.is_empty() {
        return None;
    }
    Some(match f.as_str() {
        "leak" | "leaked" => FileRelease::Leaked,
        "alternate" => FileRelease::Alternate,
        _ if crate::db::release::version_is_unreleased(&f) => FileRelease::Unreleased,
        _ => FileRelease::Released,
    })
}

/// MusicBrainz Picard's release status (RELEASESTATUS, or "MusicBrainz
/// Album Status"). A pseudo-release (a translated track list) says nothing
/// about where the music came from.
pub fn from_release_status(v: &str, cfg: &Config) -> Option<FileRelease> {
    match fold_value(v).as_str() {
        "pseudo release" | "pseudo" => None,
        _ => map_value(v, cfg),
    }
}

/// MusicBrainz Picard's release type ("album; demo"): only a demo
/// secondary type says anything about status.
pub fn from_release_type(v: &str) -> Option<FileRelease> {
    v.split([';', ',']).any(|p| fold_value(p) == "demo").then_some(FileRelease::Alternate)
}

/// One file's status and its provenance note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provenance {
    pub status: Option<FileRelease>,
    /// VERSION as written (free text), else ARCHIVE_SOURCE.
    pub note: Option<String>,
}

/// One file's status from its tags: ARCHIVE_VERSION first, then (Auto)
/// STATUS, RELEASESTATUS and RELEASETYPE, or the custom tag. The first one
/// with a known value decides.
pub fn resolve(t: &FileTags, cfg: &Config) -> Provenance {
    let note = t.version_note.clone().or_else(|| t.source.clone()).filter(|n| !n.trim().is_empty());
    let archive = t.version.as_deref().and_then(from_archive_version);
    let status = archive.or_else(|| match &cfg.tag {
        Some(key) => {
            // A value read for another tag name waits for the file to be read again.
            let fresh = t.custom_key.as_deref() == Some(key.as_str());
            t.custom_status.as_deref().filter(|_| fresh).and_then(|v| map_value(v, cfg))
        }
        None => t
            .status_tag
            .as_deref()
            .and_then(|v| map_value(v, cfg))
            .or_else(|| t.mb_status.as_deref().and_then(|v| from_release_status(v, cfg)))
            .or_else(|| t.mb_type.as_deref().and_then(from_release_type)),
    });
    Provenance { status, note }
}

/// What an album's files say, all together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlbumRelease {
    /// None when no file has a known status.
    pub status: Option<FileRelease>,
    /// Some files are released and some aren't (an official album with a
    /// disc of extras, or a build with a few released songs).
    pub mixed: bool,
}

/// Rates an album from its files' (disc, status). Disc 1 decides (disc 0,
/// not tagged, counts as disc 1): when more than half of its files with a
/// known status are released, the album is official, whatever later discs
/// hold. With nothing known on disc 1, every disc counts. Otherwise the
/// most common other status wins (unreleased, then leaked, then alternate
/// on a tie).
pub fn album_release(files: &[(u32, Option<FileRelease>)]) -> AlbumRelease {
    let known: Vec<(u32, FileRelease)> = files.iter().filter_map(|(d, s)| s.map(|s| (*d, s))).collect();
    if known.is_empty() {
        return AlbumRelease::default();
    }
    let first: Vec<FileRelease> = known.iter().filter(|(d, _)| *d <= 1).map(|x| x.1).collect();
    let base: Vec<FileRelease> = if first.is_empty() { known.iter().map(|x| x.1).collect() } else { first };
    let count = |s: FileRelease| base.iter().filter(|x| **x == s).count();
    let status = if count(FileRelease::Released) * 2 > base.len() {
        FileRelease::Released
    } else {
        [FileRelease::Unreleased, FileRelease::Leaked, FileRelease::Alternate]
            .into_iter()
            .fold((FileRelease::Unreleased, 0), |best, s| if count(s) > best.1 { (s, count(s)) } else { best })
            .0
    };
    let any_released = known.iter().any(|x| x.1.is_released());
    let any_other = known.iter().any(|x| !x.1.is_released());
    AlbumRelease { status: Some(status), mixed: any_released && any_other }
}

/// How many songs the MusicBrainz check compares with an official
/// release's length: the songs tagged released, else disc 1, else all.
pub fn compare_count(released: usize, disc_one: usize, all: usize) -> usize {
    if released > 0 {
        released
    } else if disc_one > 0 {
        disc_one
    } else {
        all
    }
}
