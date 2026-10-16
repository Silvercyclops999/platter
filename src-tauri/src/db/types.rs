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

/// Every library artist's name and id: the UI splits track artist lines
/// into links with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistName {
    pub id: String,
    pub title: String,
}

/// How the artist page ranks Popular (the `popularSource` setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PopularSource {
    /// Global listener counts first, then your own plays.
    #[default]
    Global,
    /// Only your own play counts (then most recent play).
    MyPlays,
}

/// Kind of a row in the library sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LibraryKind {
    Playlist,
    Album,
    Artist,
}

/// One playlist, album or artist in the library sidebar (`library_items`).
/// Small on purpose: the sidebar lists the whole library at once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub kind: LibraryKind,
    pub id: String,
    pub title: String,
    /// Title without a leading article, for alphabetical order.
    pub sort_title: String,
    /// Album artist for albums; None for artists and playlists.
    pub creator: Option<String>,
    /// Album artist id (albums only), for "Go to artist".
    pub creator_id: Option<String>,
    pub thumb: Option<String>,
    /// Plex smart playlist.
    pub smart: bool,
    /// Lower-case Plex album format ("ep", "single"), albums only.
    pub format: Option<String>,
    /// Unix seconds it entered the library: Plex addedAt for albums, the
    /// earliest of their albums for artists. None for playlists.
    pub added_at: Option<i64>,
    /// Unix seconds of the latest play: Plex history, the local listen log
    /// and Plex lastViewedAt for albums; their newest album for artists;
    /// local plays started from the playlist for playlists.
    pub played_at: Option<i64>,
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

/// One album, artist or playlist on Home (shortcut tile or mixed shelf).
// Albums carry more than the others; Home lists are short, so no boxing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HomeItem {
    Album { album: Album },
    Artist { artist: Artist },
    Playlist { playlist: Playlist },
}

impl HomeItem {
    /// `(kind, id)`, unique across the three kinds.
    pub fn key(&self) -> (&'static str, &str) {
        match self {
            HomeItem::Album { album } => ("album", &album.id),
            HomeItem::Artist { artist } => ("artist", &artist.id),
            HomeItem::Playlist { playlist } => ("playlist", &playlist.id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeData {
    /// Up to 8 shortcut tiles: hot first, then most played, de-duplicated.
    pub shortcuts: Vec<HomeItem>,
    /// Albums, artists and playlists by most recent play.
    pub recently_played: Vec<HomeItem>,
    pub jump_back_in: Vec<Album>,
    /// Playlists, most recently played first, then by title.
    pub playlists: Vec<Playlist>,
    pub recently_added: Vec<Album>,
    /// Artists by play count.
    pub top_artists: Vec<Artist>,
}

/// A Home shelf, for its "Show all" page (`home_shelf`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HomeShelf {
    RecentlyPlayed,
    JumpBackIn,
    Playlists,
    RecentlyAdded,
    TopArtists,
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
    /// A section of the user's own, by name (`section` is then Albums and
    /// means nothing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistPage {
    pub artist: Artist,
    pub popular: Vec<Track>,
    pub sections: Vec<ArtistSection>,
    /// Sum of play counts over the artist's own tracks.
    pub plays: u64,
    /// Plex artist biography, when it has one.
    pub about: Option<String>,
    /// Plex similar artists that are in this library, in Plex's order.
    pub similar: Vec<Artist>,
    /// Albums by others with tracks that credit this artist, newest first.
    #[serde(default)]
    pub appears_on: Vec<Album>,
    /// Popular ranks by the global list, none has been fetched yet and a
    /// fetch is on its way: the page shows placeholder rows a moment
    /// rather than plays order that reshuffles when the list lands.
    #[serde(default)]
    pub popular_pending: bool,
    /// The artist's eras (ARCHIVE_ERA), oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eras: Vec<crate::archive::eras::Era>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumPage {
    pub album: Album,
    pub tracks: Vec<Track>,
    pub total_duration_ms: u64,
    pub more_by_artist: Vec<Album>,
    /// The album artist's photo, for the header.
    #[serde(default)]
    pub artist_thumb: Option<String>,
    /// Release date (YYYY-MM-DD) and label from Plex, for the footer.
    #[serde(default)]
    pub released: Option<String>,
    #[serde(default)]
    pub studio: Option<String>,
    /// Era, date, edition and source for an unreleased build's banner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<crate::archive::eras::AlbumArchive>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistPage {
    pub playlist: Playlist,
    pub tracks: Vec<Track>,
    /// One id per track, same order: what edits use to name a row (a song
    /// can be in a playlist twice).
    pub item_ids: Vec<String>,
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
    /// Other versions of the songs in `songs`, under the song's id
    /// (`archive::versions::group_search`).
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub versions: std::collections::HashMap<String, Vec<Track>>,
    /// The query the results are for when the typo fallback changed a word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrected: Option<String>,
}
