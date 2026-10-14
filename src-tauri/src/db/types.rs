//! IPC payload shapes for the Library and Search commands (docs/IPC.md).
//! camelCase on the wire, mirrored in `src/lib/types.ts`.

use serde::{Deserialize, Serialize};

use crate::model::{Album, Artist, Playlist, ReleaseSection, Track};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SyncPhase {
    #[default]
    Idle,
    Artists,
    Albums,
    Tracks,
    Incremental,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub phase: SyncPhase,
    pub done: u64,
    pub total: u64,
    /// Unix seconds of the last completed sync.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<i64>,
}

/// What a sync run ended with. Not on the IPC surface directly; the
/// integrator maps it to `Health.banner` via
/// [`DbEvents::sync_outcome`](super::DbEvents::sync_outcome).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SyncOutcome {
    Full { artists: u64, albums: u64, tracks: u64, deleted: u64 },
    Incremental { changed: u64, deleted: u64 },
    /// Section uuid changed: cache wiped and rebuilt ("This library changed").
    LibraryReplaced { artists: u64, albums: u64, tracks: u64 },
    /// No music section on the server ("No music library found").
    NoMusicLibrary,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeData {
    pub recently_played: Vec<Album>,
    pub recently_added: Vec<Album>,
    pub jump_back_in: Vec<Album>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ArtistSort {
    #[default]
    Name,
    Recent,
    Plays,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AlbumSort {
    #[default]
    Name,
    Artist,
    Year,
    Recent,
    Plays,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistSection {
    pub section: ReleaseSection,
    pub albums: Vec<Album>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistPage {
    pub artist: Artist,
    pub popular: Vec<Track>,
    pub sections: Vec<ArtistSection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumPage {
    pub album: Album,
    pub tracks: Vec<Track>,
    pub total_duration_ms: u64,
    pub more_by_artist: Vec<Album>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistPage {
    pub playlist: Playlist,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SearchFilter {
    #[default]
    All,
    Songs,
    Artists,
    Albums,
    Playlists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchKind {
    Artist,
    Album,
    Track,
    Playlist,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub kind: SearchKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<Artist>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<Album>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<Track>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist: Option<Playlist>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SearchSource {
    #[default]
    Local,
    Server,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<SearchHit>,
    pub songs: Vec<Track>,
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub playlists: Vec<Playlist>,
    pub took_ms: f64,
    pub source: SearchSource,
}
