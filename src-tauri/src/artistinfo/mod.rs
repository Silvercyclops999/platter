//! Optional artist info from outside Plex: Wikipedia and Wikidata, Last.fm,
//! Fanart.tv and TheAudioDB. Plex stays the main source; these fill gaps on
//! the artist page (a longer bio, origin, years active, members, tags,
//! listener counts, a photo or banner when Plex has none). ListenBrainz (or
//! Last.fm top tracks) gives the global Popular list and listener numbers
//! when Last.fm has none. MusicBrainz gives the id of a credited artist that
//! has no Plex entry, members with their years and other relationships, and
//! an album's original release date and credits.
//!
//! Layout:
//! - [`http`]     the `InfoHttp` trait (faked in tests) and the real client
//! - [`limit`]    per-source pacing and 429/5xx backoff
//! - [`wiki`], [`lastfm`], [`fanart`], [`audiodb`]  one lookup per source
//! - [`listenbrainz`] top recordings and listener stats; [`musicbrainz`]
//!   artist id by name, artist and release lookups
//! - [`mbdetails`] parsing MusicBrainz members, relationships and credits
//! - [`cache`]    SQLite rows per artist and source (`artist_info` table)
//! - [`merge`]    what the page shows, from Plex plus the cached sources
//! - [`service`]  lazy fetch, background refresh, keys, switches
//! - [`art`]      serves source images through the `art` cache
//! - [`commands`] Tauri commands (docs/IPC.md "Artist info")
//!
//! Only the artist's name or MusicBrainz id ever leaves the computer (for an
//! album: its MusicBrainz release id, or its artist and title).

pub mod art;
pub mod audiodb;
pub mod cache;
pub mod commands;
pub mod fanart;
pub mod http;
pub mod lastfm;
pub mod limit;
pub mod listenbrainz;
pub mod mbdetails;
pub mod merge;
pub mod musicbrainz;
pub mod service;
pub mod wiki;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod mb_tests;

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
    /// Members with their years, relationships, album dates and credits.
    Musicbrainz,
    /// Listener numbers from ListenBrainz (when Last.fm has none).
    ListenbrainzStats,
}

impl InfoSource {
    /// The sources merged into [`ArtistInfo`].
    pub const ALL: [InfoSource; 6] =
        [Self::Wikipedia, Self::Lastfm, Self::Fanart, Self::Audiodb, Self::Musicbrainz, Self::ListenbrainzStats];

    /// Row name in the cache table.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wikipedia => "wikipedia",
            Self::Lastfm => "lastfm",
            Self::Fanart => "fanart",
            Self::Audiodb => "audiodb",
            Self::Listenbrainz => crate::db::popular::GLOBAL_ROW,
            Self::Musicbrainz => "musicbrainz",
            Self::ListenbrainzStats => "listenbrainz-stats",
        }
    }

    /// Whether the source can't work without the user's API key.
    pub fn needs_key(self) -> bool {
        matches!(self, Self::Lastfm | Self::Fanart | Self::Audiodb)
    }

    /// Whether the source can use a key (ListenBrainz: an optional token).
    pub fn takes_key(self) -> bool {
        matches!(self, Self::Lastfm | Self::Fanart | Self::Audiodb | Self::Listenbrainz)
    }

    /// Keychain account for the source's key.
    pub fn key_account(self) -> &'static str {
        match self {
            Self::Wikipedia => "wikipedia-unused",
            Self::Lastfm => "lastfm-api-key",
            Self::Fanart => "fanart-api-key",
            Self::Audiodb => "audiodb-api-key",
            Self::Listenbrainz => "listenbrainz-token",
            Self::Musicbrainz => "musicbrainz-unused",
            Self::ListenbrainzStats => "listenbrainz-token",
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            Self::Wikipedia => "Wikipedia",
            Self::Lastfm => "Last.fm",
            Self::Fanart => "Fanart.tv",
            Self::Audiodb => "TheAudioDB",
            Self::Listenbrainz | Self::ListenbrainzStats => "ListenBrainz",
            Self::Musicbrainz => "MusicBrainz",
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
    /// Official website and well-known profiles, https only.
    pub links: Vec<InfoLink>,
}

/// A link in the About dialog's side column: the official website or a
/// profile on a well-known site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoLink {
    /// "website", "wikipedia", "lastfm", "instagram", "x", "facebook",
    /// "youtube", "bandcamp" or "soundcloud".
    pub kind: String,
    /// What the link says ("Official website", "Instagram").
    pub label: String,
    pub url: String,
}

/// An artist in the library named in the bio, for linking it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BioLink {
    /// The name exactly as the bio writes it.
    pub text: String,
    pub artist_id: String,
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
    /// Every photo and background, most liked first (for the gallery).
    pub thumbs: Vec<String>,
    pub backgrounds: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioDbData {
    pub url: Option<String>,
    pub bio: Option<String>,
    pub thumb: Option<String>,
    /// Wide fan art images (for the gallery).
    pub fanart: Vec<String>,
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
    /// "1991–present", "1991–2004" (MusicBrainz), when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub years: Option<String>,
}

/// Another artist this one is related to (MusicBrainz).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Related {
    pub name: String,
    pub artist_id: Option<String>,
    /// "Member of", "Subgroup", "Collaboration", "Formerly", …
    pub relation: String,
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
    /// Last.fm listeners and plays, else ListenBrainz's.
    pub listeners: Option<u64>,
    pub scrobbles: Option<u64>,
    /// Where the listener numbers come from: "lastfm" or "listenbrainz".
    pub listeners_from: Option<String>,
    /// Other artists this one is related to (MusicBrainz).
    pub related: Vec<Related>,
    /// Art path for a photo, only when Plex has none.
    pub photo: Option<String>,
    /// Art path for the header banner, only when Plex has no background art.
    pub banner: Option<String>,
    pub logo: Option<String>,
    pub credits: Vec<Credit>,
    /// Art paths for the About dialog's gallery: Plex's images first, then
    /// the sources'.
    pub gallery: Vec<String>,
    /// Official website and profiles (Wikidata), then Wikipedia and Last.fm.
    pub links: Vec<InfoLink>,
    /// Library artists the bio names, in the order they first appear.
    pub bio_links: Vec<BioLink>,
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
