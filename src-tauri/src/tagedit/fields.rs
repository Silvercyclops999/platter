//! The fields the tag editor reads and writes (docs/TAGGING.md), and how a
//! value is checked before it's written.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// One editable field. Only these are ever written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    AlbumArtist,
    Album,
    /// DATE (Vorbis), TDRC (ID3), ©day (MP4): the album year or full date.
    Year,
    Genre,
    /// RELEASETYPE, e.g. `album;demo`.
    ReleaseType,
    Edition,
    Group,
    Era,
    Quality,
    /// `explicit`, `clean`, or not set.
    Explicit,
    TrackNumber,
    TrackTotal,
    DiscNumber,
    DiscTotal,
    Title,
    Artist,
    /// ARCHIVE_VERSION.
    ArchiveVersion,
    /// A plain STATUS tag.
    Status,
    /// Picard's RELEASESTATUS ("MusicBrainz Album Status" in ID3 and MP4).
    PicardStatus,
    /// The custom release status tag named in Settings.
    CustomStatus,
    /// A plain VERSION tag: the free-text provenance note.
    VersionNote,
    /// ARCHIVE_DATE.
    ArchiveDate,
    /// ARCHIVE_SOURCE.
    Source,
    /// ARCHIVE_FLAGS.
    Flags,
    Isrc,
}

impl Field {
    pub const ALL: [Field; 25] = [
        Field::AlbumArtist,
        Field::Album,
        Field::Year,
        Field::Genre,
        Field::ReleaseType,
        Field::Edition,
        Field::Group,
        Field::Era,
        Field::Quality,
        Field::Explicit,
        Field::TrackNumber,
        Field::TrackTotal,
        Field::DiscNumber,
        Field::DiscTotal,
        Field::Title,
        Field::Artist,
        Field::ArchiveVersion,
        Field::Status,
        Field::PicardStatus,
        Field::CustomStatus,
        Field::VersionNote,
        Field::ArchiveDate,
        Field::Source,
        Field::Flags,
        Field::Isrc,
    ];

    /// Fields a file can hold several values of. They read joined with
    /// `"; "` and are written back as separate values in the file's own way.
    pub fn multi_value(self) -> bool {
        matches!(self, Field::AlbumArtist | Field::Artist | Field::Genre)
    }

    /// The Vorbis comment name, which is also the TXXX description and MP4
    /// freeform name for the fields that have no frame or atom of their own.
    /// None for the custom tag (its name comes from Settings).
    pub fn vorbis_key(self) -> Option<&'static str> {
        Some(match self {
            Field::AlbumArtist => "ALBUMARTIST",
            Field::Album => "ALBUM",
            Field::Year => "DATE",
            Field::Genre => "GENRE",
            Field::ReleaseType => "RELEASETYPE",
            Field::Edition => "ARCHIVE_EDITION",
            Field::Group => "ARCHIVE_GROUP",
            Field::Era => "ARCHIVE_ERA",
            Field::Quality => "ARCHIVE_QUALITY",
            Field::Explicit => "ITUNESADVISORY",
            Field::TrackNumber => "TRACKNUMBER",
            Field::TrackTotal => "TRACKTOTAL",
            Field::DiscNumber => "DISCNUMBER",
            Field::DiscTotal => "DISCTOTAL",
            Field::Title => "TITLE",
            Field::Artist => "ARTIST",
            Field::ArchiveVersion => "ARCHIVE_VERSION",
            Field::Status => "STATUS",
            Field::PicardStatus => "RELEASESTATUS",
            Field::CustomStatus => return None,
            Field::VersionNote => "VERSION",
            Field::ArchiveDate => "ARCHIVE_DATE",
            Field::Source => "ARCHIVE_SOURCE",
            Field::Flags => "ARCHIVE_FLAGS",
            Field::Isrc => "ISRC",
        })
    }

    /// Fields that are the same on every file of an album.
    pub fn album_level(self) -> bool {
        matches!(
            self,
            Field::AlbumArtist
                | Field::Album
                | Field::Year
                | Field::Genre
                | Field::ReleaseType
                | Field::Edition
                | Field::Group
                | Field::Era
                | Field::Quality
        )
    }

    /// How the UI and messages name it.
    pub fn label(self) -> &'static str {
        match self {
            Field::AlbumArtist => "Album artist",
            Field::Album => "Album",
            Field::Year => "Year",
            Field::Genre => "Genre",
            Field::ReleaseType => "Release type",
            Field::Edition => "Edition",
            Field::Group => "Group",
            Field::Era => "Era",
            Field::Quality => "Quality",
            Field::Explicit => "Explicit",
            Field::TrackNumber => "Track",
            Field::TrackTotal => "Tracks",
            Field::DiscNumber => "Disc",
            Field::DiscTotal => "Discs",
            Field::Title => "Title",
            Field::Artist => "Artist",
            Field::ArchiveVersion | Field::Status | Field::PicardStatus | Field::CustomStatus => "Release status",
            Field::VersionNote => "Version note",
            Field::ArchiveDate => "Date",
            Field::Source => "Source",
            Field::Flags => "Flags",
            Field::Isrc => "ISRC",
        }
    }
}

/// Where an album keeps its release status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scheme {
    /// Platter's own ARCHIVE_VERSION.
    Archive,
    /// A plain STATUS tag (VERSION as the note).
    Plain,
    /// MusicBrainz Picard's RELEASESTATUS.
    Picard,
    /// The custom tag named in Settings.
    Custom,
}

impl Scheme {
    pub fn field(self) -> Field {
        match self {
            Scheme::Archive => Field::ArchiveVersion,
            Scheme::Plain => Field::Status,
            Scheme::Picard => Field::PicardStatus,
            Scheme::Custom => Field::CustomStatus,
        }
    }

    /// The values the editor offers for it.
    pub fn choices(self) -> &'static [&'static str] {
        match self {
            Scheme::Archive => &["released", "alternate", "demo", "session", "leak", "snippet", "remix", "live", "build"],
            Scheme::Plain | Scheme::Custom => &["Released", "Unreleased", "Leaked", "Alternate"],
            Scheme::Picard => &["official", "promotion", "bootleg"],
        }
    }
}

/// The scheme an album's files already use: whichever status tag most of
/// them carry (ARCHIVE_VERSION first on a tie, as the reader has it). With
/// none, the Settings choice: the custom tag when one is set, else
/// ARCHIVE_VERSION.
pub fn detect_scheme<'a>(files: impl IntoIterator<Item = &'a crate::tagedit::Values>, custom_set: bool) -> Scheme {
    let mut n = [0usize; 4];
    let order = [Scheme::Archive, Scheme::Custom, Scheme::Plain, Scheme::Picard];
    for v in files {
        for (i, s) in order.iter().enumerate() {
            if (*s != Scheme::Custom || custom_set) && v.get(&s.field()).is_some_and(|x| !x.trim().is_empty()) {
                n[i] += 1;
            }
        }
    }
    let best = (0..4).fold(None::<usize>, |b, i| match b {
        _ if n[i] == 0 => b,
        Some(j) if n[j] >= n[i] => Some(j),
        _ => Some(i),
    });
    match best {
        Some(i) => order[i],
        None if custom_set => Scheme::Custom,
        None => Scheme::Archive,
    }
}

/// The values ARCHIVE_* fields take (docs/TAGGING.md). Anything else is
/// "unknown" to the tag check.
pub const ARCHIVE_VERSIONS: &[&str] =
    &["released", "alternate", "demo", "session", "leak", "snippet", "remix", "live", "build", "unreleased"];
pub const ARCHIVE_QUALITIES: &[&str] = &["lossless", "cdq", "lq", "snippet"];
pub const ARCHIVE_FLAGS: &[&str] = &["fanedit", "aiedit"];
/// Every ARCHIVE_* tag Platter knows.
pub const ARCHIVE_KEYS: &[&str] = &[
    "ARCHIVE_VERSION",
    "ARCHIVE_ERA",
    "ARCHIVE_QUALITY",
    "ARCHIVE_DATE",
    "ARCHIVE_EDITION",
    "ARCHIVE_GROUP",
    "ARCHIVE_SOURCE",
    "ARCHIVE_FLAGS",
];

/// Release type parts (RELEASETYPE): a primary one, then secondary ones.
pub const PRIMARY_TYPES: &[&str] = &["album", "ep", "single", "broadcast", "other"];
pub const SECONDARY_TYPES: &[&str] = &[
    "compilation",
    "soundtrack",
    "spokenword",
    "interview",
    "audiobook",
    "live",
    "remix",
    "dj-mix",
    "mixtape/street",
    "demo",
];

/// Whether an ARCHIVE_DATE is one of the three forms the reader takes.
pub fn archive_date_ok(v: &str) -> bool {
    let b = v.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    match b.len() {
        4 => digits(0..4),
        7 => digits(0..4) && b[4] == b'-' && digits(5..7),
        10 => digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10),
        _ => false,
    }
}

/// A value as it will be written: trimmed, checked, and in the form the
/// field takes. Empty means remove the tag.
pub fn clean_value(f: Field, v: &str) -> AppResult<String> {
    let v = v.trim();
    if v.is_empty() {
        return Ok(String::new());
    }
    if v.chars().any(|c| c.is_control()) {
        return Err(AppError::Invalid(format!("{} can't hold control characters.", f.label())));
    }
    if v.chars().count() > 1000 {
        return Err(AppError::Invalid(format!("{} is too long.", f.label())));
    }
    match f {
        Field::TrackNumber | Field::TrackTotal | Field::DiscNumber | Field::DiscTotal => match v.parse::<u32>() {
            Ok(n) if (1..=999).contains(&n) => Ok(n.to_string()),
            _ => Err(AppError::Invalid(format!("{} is a number from 1 to 999.", f.label()))),
        },
        Field::Isrc => crate::tags::clean_isrc(v)
            .ok_or_else(|| AppError::Invalid(format!("\"{v}\" isn't an ISRC (like USRC17607839)."))),
        Field::Explicit => match v.to_ascii_lowercase().as_str() {
            "explicit" | "1" | "true" | "yes" => Ok("explicit".into()),
            "clean" | "2" | "0" | "false" | "no" => Ok("clean".into()),
            _ => Err(AppError::Invalid("Explicit is explicit, clean or not set.".into())),
        },
        Field::Year => {
            let ok = crate::tags::year_of(v).is_some() && (v.len() == 4 || archive_date_ok(v));
            if ok { Ok(v.to_string()) } else { Err(AppError::Invalid("Year is like 2019 or 2019-05-31.".into())) }
        }
        Field::ArchiveDate => {
            if archive_date_ok(v) {
                Ok(v.to_string())
            } else {
                Err(AppError::Invalid("Date is like 2019, 2019-05 or 2019-05-31.".into()))
            }
        }
        Field::ReleaseType => {
            let parts: Vec<String> =
                v.split([';', '/']).map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty()).collect();
            // "mixtape/street" holds a slash of its own.
            let joined = parts.join(";").replace("mixtape;street", "mixtape/street");
            Ok(joined)
        }
        Field::ArchiveVersion | Field::Quality => Ok(v.to_lowercase()),
        Field::Flags => {
            let mut out: Vec<String> = Vec::new();
            for f in v.split([',', ';', '/', ' ']).map(|f| f.trim().to_lowercase()).filter(|f| !f.is_empty()) {
                if !out.contains(&f) {
                    out.push(f);
                }
            }
            Ok(out.join(","))
        }
        _ => Ok(v.to_string()),
    }
}
