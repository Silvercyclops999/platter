//! Shared data model: the shapes that cross the IPC boundary to the UI.
//!
//! Every struct here is mirrored in `src/lib/types.ts`. Field names are
//! camelCase on the wire. IDs are Plex `ratingKey`s as strings. Artwork is a
//! Plex image path (e.g. `/library/metadata/1/thumb/123`); the UI turns it into
//! an `art://` URL that Rust serves from its disk cache, so the token never
//! reaches the webview.

use serde::{Deserialize, Serialize};

/// Release-type section on an artist page, derived from Plex `Format` /
/// `Subformat` (see docs/PLEX-API.md and docs/TAGGING.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReleaseSection {
    Albums,
    SinglesEps,
    Live,
    Compilations,
    Soundtracks,
    Remixes,
    Demos,
    Mixtapes,
    /// Unreleased material (release status, not a release type): set on the
    /// artist page only, never stored by a sync.
    Unreleased,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub id: String,
    pub title: String,
    pub sort_title: String,
    pub thumb: Option<String>,
    pub art: Option<String>,
    pub view_count: u32,
    /// Plex artist biography. Stored by sync and sent only on the artist
    /// page (as `ArtistPage.about`), never with artist lists.
    #[serde(skip)]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    pub title: String,
    pub sort_title: String,
    pub artist_id: String,
    pub artist_title: String,
    pub year: Option<i32>,
    pub thumb: Option<String>,
    /// Lower-case Plex format, e.g. "album", "ep", "single". None when untagged.
    pub format: Option<String>,
    /// Lower-case Plex subformats, e.g. ["live"], ["soundtrack"].
    pub subformats: Vec<String>,
    pub section: ReleaseSection,
    pub genres: Vec<String>,
    pub track_count: u32,
    pub added_at: i64,
    pub view_count: u32,
    pub last_viewed_at: Option<i64>,
    /// Server-computed colors for header tint (hex without '#'), if present.
    pub tint: Option<String>,
    /// Release date as Plex has it (`originallyAvailableAt`, YYYY-MM-DD).
    /// Stored for the album page footer; not sent with every album.
    #[serde(skip)]
    pub released: Option<String>,
    /// Record label (Plex `studio`). Album page footer only, like `released`.
    #[serde(skip)]
    pub studio: Option<String>,
    /// Whether Plex matched the album to its metadata agent (`guid`
    /// `plex://album/...`) or not (`local://...`). None when unknown.
    /// Feeds the unreleased guess; not sent to the UI.
    #[serde(skip)]
    pub matched: Option<bool>,
    /// Plex style tags (finer genres). Stored for mixes; not sent to the UI.
    #[serde(skip)]
    pub styles: Vec<String>,
    /// Plex mood tags. Stored for mixes; not sent to the UI.
    #[serde(skip)]
    pub moods: Vec<String>,
    /// MusicBrainz release id (Plex `Guid` `mbid://`), when Plex matched
    /// the album. Stored to find the same album on another server; not sent
    /// to the UI.
    #[serde(skip)]
    pub mbid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub title: String,
    pub album_id: String,
    pub album_title: String,
    /// Album artist (Plex grandparent).
    pub artist_id: String,
    pub artist_title: String,
    /// Track artist(s) when different from the album artist (Plex originalTitle).
    pub track_artist: Option<String>,
    pub track_number: u32,
    pub disc_number: u32,
    pub duration_ms: u64,
    pub thumb: Option<String>,
    /// Plex media part key, e.g. `/library/parts/1/2/file.flac`.
    pub part_key: String,
    pub codec: Option<String>,
    pub container: Option<String>,
    pub bitrate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
    pub user_rating: Option<f32>,
    pub view_count: u32,
    pub last_viewed_at: Option<i64>,
    /// Plex ratingCount. Only on tracks Plex matched to MusicBrainz and not
    /// on one scale, so it only breaks ties in Popular (db::popular).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_count: Option<u32>,
    /// When the user last rated the track, unix seconds (Plex lastRatedAt).
    /// For a liked song this is when it was liked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rated_at: Option<i64>,
    /// Plex genre and mood tags on the track itself (the Liked Songs
    /// filter chips).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    /// MusicBrainz track id (Plex `Guid` `mbid://`), when Plex matched the
    /// track. Stored to find the same song on another server; not sent to
    /// the UI.
    #[serde(skip)]
    pub mbid: Option<String>,
    /// The file's path on the server (Plex Part `file`). Stored for the
    /// folder name hints on release status; not sent to the UI.
    #[serde(skip)]
    pub file: Option<String>,
    /// How many other versions of the song the library has
    /// (`archive::versions`), on pages that fill it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_versions: Option<u32>,
    /// The song's own release status, on pages that fill it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub song_status: Option<SongStatus>,
}

/// One song's release status (`archive::songs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SongStatus {
    Released,
    /// Tagged or marked unreleased.
    Unreleased,
    /// On an album that's probably unreleased, and no released song in the
    /// library has its title.
    ProbablyUnreleased,
    /// On an album that's probably unreleased, with the title of a released
    /// song in the library: probably another version of it. Still unreleased.
    ProbablyAlternate,
}

impl SongStatus {
    pub fn key(self) -> &'static str {
        match self {
            Self::Released => "released",
            Self::Unreleased => "unreleased",
            Self::ProbablyUnreleased => "probablyUnreleased",
            Self::ProbablyAlternate => "probablyAlternate",
        }
    }

    pub fn from_key(k: &str) -> Self {
        match k {
            "released" => Self::Released,
            "probablyUnreleased" => Self::ProbablyUnreleased,
            "probablyAlternate" => Self::ProbablyAlternate,
            _ => Self::Unreleased,
        }
    }

    pub fn is_released(self) -> bool {
        self == Self::Released
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub smart: bool,
    pub track_count: u32,
    pub duration_ms: u64,
    pub thumb: Option<String>,
    /// The playlist's description (Plex `summary`).
    #[serde(default)]
    pub summary: Option<String>,
    /// Last change, unix seconds (Plex `updatedAt`, bumped by local edits).
    #[serde(default)]
    pub updated_at: Option<i64>,
}
