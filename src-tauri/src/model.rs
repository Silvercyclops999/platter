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
    /// Global listener count Plex attaches to a track (Plex ratingCount).
    /// Missing on unreleased and rare songs. Ranks an artist's popular songs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_count: Option<u32>,
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
}
