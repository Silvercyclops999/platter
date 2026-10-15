//! Tags read from the music files themselves (docs/TAGGING.md).
//!
//! Plex ignores the `ARCHIVE_*` tags and the explicit flag, so Platter reads
//! them from each file's header through Plex's own part URL with small Range
//! requests. Nothing is installed on the server and no file is ever written.
//!
//! - [`parse`]  header parsers (FLAC, ID3v2, MP4, Ogg) over partial reads
//! - [`fetch`]  Range reads through the player's one-transfer gate
//! - [`store`]  the SQLite cache (keyed by part key) and what's derived from it
//! - [`scan`]   the background scanner: lowest priority, yields to playback
//! - [`view`]   shapes library pages by the tags (editions, fan edits)
//! - [`commands`] Tauri commands and events (docs/IPC.md "File tags")

pub mod commands;
pub mod fetch;
pub mod parse;
pub mod scan;
pub mod store;
pub mod view;

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

/// The tags kept from one file. Text values are trimmed; version, quality
/// and flags are lower case.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTags {
    /// Some(true) explicit, Some(false) clean, None not tagged.
    pub explicit: Option<bool>,
    pub version: Option<String>,
    pub era: Option<String>,
    pub quality: Option<String>,
    pub date: Option<String>,
    pub edition: Option<String>,
    pub group: Option<String>,
    pub source: Option<String>,
    /// Comma-separated, e.g. "fanedit" or "fanedit,aiedit".
    pub flags: Option<String>,
    /// ISRC, upper case without dashes (ID3 TSRC, Vorbis ISRC, MP4 freeform ISRC).
    pub isrc: Option<String>,
    /// MusicBrainz recording id, lower case (Vorbis MUSICBRAINZ_TRACKID, ID3
    /// UFID owned by musicbrainz.org, MP4 freeform "MusicBrainz Track Id").
    pub recording_mbid: Option<String>,
    /// The album name the file carries (ALBUM, ID3 TALB, MP4 "©alb").
    pub album: Option<String>,
    /// The album artist the file carries (ALBUMARTIST, ID3 TPE2, MP4 aART).
    pub album_artist: Option<String>,
    /// The year its date starts with (DATE or YEAR, ID3 TDRC or TYER, MP4 "©day").
    pub year: Option<i32>,
}

/// Raised whenever the parsers start keeping something new, so files read
/// by an older build are read again (at the scan's background priority).
/// 1: ISRC and the MusicBrainz recording id.
/// 2: album, album artist and year (the library doctor compares them).
pub const CACHE_VERSION: i64 = 2;

const KEYS: [&str; 20] = [
    "ARCHIVE_VERSION",
    "ARCHIVE_ERA",
    "ARCHIVE_QUALITY",
    "ARCHIVE_DATE",
    "ARCHIVE_EDITION",
    "ARCHIVE_GROUP",
    "ARCHIVE_SOURCE",
    "ARCHIVE_FLAGS",
    "ITUNESADVISORY",
    "EXPLICIT",
    "RTNG",
    "ISRC",
    "MUSICBRAINZ_TRACKID",
    "MUSICBRAINZ TRACK ID",
    "ALBUM",
    "ALBUMARTIST",
    "ALBUM ARTIST",
    "ALBUM_ARTIST",
    "DATE",
    "YEAR",
];

impl FileTags {
    /// Whether a tag name is one we keep (any case).
    pub fn wants(key: &str) -> bool {
        let k = key.trim();
        KEYS.iter().any(|w| w.eq_ignore_ascii_case(k))
    }

    pub fn set_explicit(&mut self, v: Option<bool>) {
        if self.explicit.is_none() {
            self.explicit = v;
        }
    }

    /// Records one tag. The first value of a field wins; flags add up.
    pub fn set(&mut self, key: &str, value: &str) {
        let v = value.trim().trim_end_matches('\0').trim();
        if v.is_empty() {
            return;
        }
        let key = key.trim().to_ascii_uppercase();
        let first = |slot: &mut Option<String>, v: String| {
            if slot.is_none() {
                *slot = Some(v);
            }
        };
        match key.as_str() {
            "ARCHIVE_VERSION" => first(&mut self.version, v.to_lowercase()),
            "ARCHIVE_ERA" => first(&mut self.era, v.to_string()),
            "ARCHIVE_QUALITY" => first(&mut self.quality, v.to_lowercase()),
            "ARCHIVE_DATE" => first(&mut self.date, v.to_string()),
            "ARCHIVE_EDITION" => first(&mut self.edition, v.to_string()),
            "ARCHIVE_GROUP" => first(&mut self.group, v.to_string()),
            "ARCHIVE_SOURCE" => first(&mut self.source, v.to_string()),
            "ARCHIVE_FLAGS" => {
                let mut all: Vec<String> = self
                    .flags
                    .as_deref()
                    .map(|f| f.split(',').map(str::to_string).collect())
                    .unwrap_or_default();
                for f in v.split([',', ';', '/', ' ']).map(|f| f.trim().to_lowercase()) {
                    if !f.is_empty() && !all.contains(&f) {
                        all.push(f);
                    }
                }
                self.flags = (!all.is_empty()).then(|| all.join(","));
            }
            // iTunes advisory: 1 (or 4) explicit, 2 clean, 0 none.
            "ITUNESADVISORY" | "RTNG" => match v {
                "1" | "4" => self.set_explicit(Some(true)),
                "2" => self.set_explicit(Some(false)),
                _ => {}
            },
            "ISRC" => {
                if self.isrc.is_none() {
                    self.isrc = v.split([';', '/', ',']).find_map(clean_isrc);
                }
            }
            "MUSICBRAINZ_TRACKID" | "MUSICBRAINZ TRACK ID" => {
                if self.recording_mbid.is_none() {
                    self.recording_mbid = v.split([';', '/', ',']).find_map(clean_mbid);
                }
            }
            "ALBUM" => first(&mut self.album, v.to_string()),
            "ALBUMARTIST" | "ALBUM ARTIST" | "ALBUM_ARTIST" => first(&mut self.album_artist, v.to_string()),
            "DATE" | "YEAR" => {
                if self.year.is_none() {
                    self.year = year_of(v);
                }
            }
            "EXPLICIT" => match v.to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "explicit" => self.set_explicit(Some(true)),
                "0" | "false" | "no" | "clean" => self.set_explicit(Some(false)),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn has_flag(&self, flag: &str) -> bool {
        has_flag(self.flags.as_deref(), flag)
    }
}

/// An ISRC in its plain form ("USRC17607839"), or None when it isn't one.
/// Dashes and spaces some taggers add are dropped.
pub fn clean_isrc(v: &str) -> Option<String> {
    let s = v.chars().filter(|c| !matches!(c, '-' | ' ' | '\0')).collect::<String>().to_ascii_uppercase();
    let b = s.as_bytes();
    let ok = b.len() == 12
        && b[..2].iter().all(u8::is_ascii_alphabetic)
        && b[2..5].iter().all(u8::is_ascii_alphanumeric)
        && b[5..].iter().all(u8::is_ascii_digit);
    ok.then_some(s)
}

/// A MusicBrainz id in lower case, or None when it isn't one (36 characters:
/// hex digits and four dashes).
pub fn clean_mbid(v: &str) -> Option<String> {
    let s = v.trim().trim_matches('\0').trim().to_ascii_lowercase();
    let ok = s.len() == 36
        && s.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() });
    ok.then_some(s)
}

/// The year a date tag starts with ("2016", "2016-05-04", "2016/05",
/// "20160504"), or None when it doesn't start with one.
pub fn year_of(v: &str) -> Option<i32> {
    let v = v.trim();
    let d: String = v.chars().take_while(char::is_ascii_digit).collect();
    if d.len() != 4 && d.len() != 8 {
        return None;
    }
    d[..4].parse().ok().filter(|y| (1000..=2999).contains(y))
}

/// Whether a comma-separated flag list holds `flag` (fan or AI edit).
pub fn has_flag(flags: Option<&str>, flag: &str) -> bool {
    flags.is_some_and(|f| f.split(',').any(|x| x.trim().eq_ignore_ascii_case(flag)))
}

/// A fan or AI edit (hidden unless Settings shows them).
pub fn is_edit(flags: Option<&str>) -> bool {
    has_flag(flags, "fanedit") || has_flag(flags, "aiedit")
}

/// How the library shows what the tags say (from Settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagView {
    /// Albums sharing ARCHIVE_GROUP show as one card.
    pub group_editions: bool,
    /// Fan and AI edits show (off hides them).
    pub show_edits: bool,
    /// Explicit songs are left out of what plays.
    pub filter_explicit: bool,
}

impl Default for TagView {
    fn default() -> Self {
        Self { group_editions: true, show_edits: false, filter_explicit: false }
    }
}

impl TagView {
    pub fn bits(self) -> u8 {
        self.group_editions as u8 | (self.show_edits as u8) << 1 | (self.filter_explicit as u8) << 2
    }

    pub fn from_bits(b: u8) -> Self {
        Self { group_editions: b & 1 != 0, show_edits: b & 2 != 0, filter_explicit: b & 4 != 0 }
    }
}
