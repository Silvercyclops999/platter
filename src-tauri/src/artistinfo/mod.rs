//! Optional artist info from outside Plex: Wikipedia and Wikidata, Last.fm,
//! Fanart.tv and TheAudioDB. Plex stays the main source; these fill gaps on
//! the artist page (a longer bio, origin, years active, members, tags,
//! listener counts, a photo or banner when Plex has none). ListenBrainz (or
//! Last.fm top tracks) gives the global Popular list, and MusicBrainz the id
//! of a credited artist that has no Plex entry.
//!
//! Layout:
//! - [`http`]     the `InfoHttp` trait (faked in tests) and the real client
//! - [`limit`]    per-source pacing and 429/5xx backoff
//! - [`wiki`], [`lastfm`], [`fanart`], [`audiodb`]  one lookup per source
//! - [`listenbrainz`] top recordings; [`musicbrainz`] artist id by name
//! - [`cache`]    SQLite rows per artist and source (`artist_info` table)
//! - [`merge`]    what the page shows, from Plex plus the cached sources
//! - [`service`]  lazy fetch, background refresh, keys, switches
//! - [`art`]      serves source images through the `art` cache
//! - [`commands`] Tauri commands (docs/IPC.md "Artist info")
//!
//! Only the artist's name or MusicBrainz id ever leaves the computer.

pub mod art;
pub mod audiodb;
pub mod cache;
pub mod commands;
pub mod fanart;
pub mod http;
pub mod lastfm;
pub mod limit;
pub mod listenbrainz;
pub mod merge;
pub mod musicbrainz;
pub mod service;
pub mod wiki;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

pub use service::{ArtistInfoService, InfoEvents, MbidLookup, Switches};

/// The outside sources, in bio fallback order after Plex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InfoSource {
    Wikipedia,
    Lastfm,
    Fanart,
    Audiodb,
    /// Popular songs (not part of the merged info).
    Listenbrainz,
}

impl InfoSource {
    /// The sources merged into [`ArtistInfo`].
    pub const ALL: [InfoSource; 4] = [Self::Wikipedia, Self::Lastfm, Self::Fanart, Self::Audiodb];

    /// Row name in the cache table.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wikipedia => "wikipedia",
            Self::Lastfm => "lastfm",
            Self::Fanart => "fanart",
            Self::Audiodb => "audiodb",
            Self::Listenbrainz => crate::db::popular::GLOBAL_ROW,
        }
    }

    /// Whether the source can't work without the user's API key.
    pub fn needs_key(self) -> bool {
        !matches!(self, Self::Wikipedia | Self::Listenbrainz)
    }

    /// Whether the source can use a key (ListenBrainz: an optional token).
    pub fn takes_key(self) -> bool {
        !matches!(self, Self::Wikipedia)
    }

    /// Keychain account for the source's key.
    pub fn key_account(self) -> &'static str {
        match self {
            Self::Wikipedia => "wikipedia-unused",
            Self::Lastfm => "lastfm-api-key",
            Self::Fanart => "fanart-api-key",
            Self::Audiodb => "audiodb-api-key",
            Self::Listenbrainz => "listenbrainz-token",
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            Self::Wikipedia => "Wikipedia",
            Self::Lastfm => "Last.fm",
            Self::Fanart => "Fanart.tv",
            Self::Audiodb => "TheAudioDB",
            Self::Listenbrainz => "ListenBrainz",
        }
    }
}

/// Wikipedia article lead plus Wikidata facts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WikiData {
    /// Wikidata item id, like "Q44190".
    pub qid: String,
    /// The article's lead section as plain text.
    pub bio: Option<String>,
    pub article_url: Option<String>,
    pub origin: Option<String>,
    pub years_active: Option<String>,
    pub members: Vec<String>,
    /// MusicBrainz artist id from Wikidata (used when Plex has none).
    pub mbid: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LastfmData {
    pub url: Option<String>,
    pub listeners: Option<u64>,
    pub playcount: Option<u64>,
    pub tags: Vec<String>,
    pub bio: Option<String>,
    /// The artist's MusicBrainz id, when Last.fm has one.
    pub mbid: Option<String>,
}

/// Image URLs from Fanart.tv (the most liked of each kind).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FanartData {
    pub mbid: String,
    pub background: Option<String>,
    pub thumb: Option<String>,
    pub logo: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioDbData {
    pub url: Option<String>,
    pub bio: Option<String>,
    pub thumb: Option<String>,
}

/// Why a lookup gave nothing to cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The service refused the API key (wrong, revoked, suspended).
    KeyRejected,
    /// Offline, busy or broken; try again on a later page open.
    Unavailable(String),
}

impl From<crate::error::AppError> for FetchError {
    fn from(e: crate::error::AppError) -> Self {
        FetchError::Unavailable(e.to_string())
    }
}

/// `Ok(None)`: the source has nothing for this artist (cached as a miss).
pub type Fetch<T> = Result<Option<T>, FetchError>;

/// A bio's source, for the credit line under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BioSource {
    Plex,
    Wikipedia,
    Lastfm,
    Audiodb,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub name: String,
    /// Set when the member is an artist in this library.
    pub artist_id: Option<String>,
}

/// One "where this came from" line in the About dialog.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Credit {
    pub source: String,
    pub text: String,
    pub url: Option<String>,
    pub licence: Option<String>,
    pub licence_url: Option<String>,
}

/// What the artist page shows from the outside sources (IPC `ArtistInfo`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistInfo {
    pub id: String,
    /// Best biography (may be Plex's own summary).
    pub bio: Option<String>,
    pub bio_source: Option<BioSource>,
    pub origin: Option<String>,
    pub years_active: Option<String>,
    pub members: Vec<Member>,
    pub tags: Vec<String>,
    /// Last.fm listeners and plays.
    pub listeners: Option<u64>,
    pub scrobbles: Option<u64>,
    /// Art path for a photo, only when Plex has none.
    pub photo: Option<String>,
    /// Art path for the header banner, only when Plex has no background art.
    pub banner: Option<String>,
    pub logo: Option<String>,
    pub credits: Vec<Credit>,
}

/// Plain-text cleanup shared by the sources: drop tags, decode the common
/// entities, trim, and keep paragraph breaks.
pub fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("\r\n", "\n");
    // At most one blank line between paragraphs.
    let mut lines: Vec<&str> = Vec::new();
    for line in out.split('\n').map(str::trim) {
        if line.is_empty() && lines.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Case, width and spacing folded, for comparing artist names.
pub fn fold_name(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkc().collect::<String>().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A MusicBrainz id: 36 characters of hex and dashes in the UUID layout.
pub fn is_mbid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// The art path that routes a source image through the art cache.
pub fn ext_art_path(url: &str) -> String {
    format!("{}{url}", art::EXT_PREFIX)
}
