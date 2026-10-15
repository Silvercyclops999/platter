//! What the db layer needs from the outside world. The integrator implements
//! these with the Plex client; `db/` never depends on `plex/`.

use async_trait::async_trait;
use bytes::Bytes;

use crate::error::AppResult;
use crate::model::{Album, Artist, Playlist, Track};

use super::types::{SyncOutcome, SyncStatus};

/// A Plex music library section (`/library/sections`, `type: artist`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicSection {
    pub id: String,
    /// Stable section uuid; a change means "library replaced" → rebuild.
    pub uuid: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// Plex type 8.
    Artist,
    /// Plex type 9.
    Album,
    /// Plex type 10.
    Track,
}

/// Items added or updated since a point in time.
#[derive(Debug, Clone, Default)]
pub struct Changes {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
}

/// One row of the server's play history (Plex:
/// `/status/sessions/history/all`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub track_id: String,
    /// Unix seconds.
    pub viewed_at: i64,
}

/// One playlist row: the track and Plex's playlistItemID for that row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistEntry {
    pub track_id: String,
    pub item_id: Option<String>,
}

/// A similar artist from Plex artist detail (`Similar` tag). Stored as JSON
/// in `artists.similar`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SimilarArtist {
    /// Plex tag id, when present.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
}

/// The library as seen through Plex. All methods are paged/cheap where
/// Plex allows; see docs/PLEX-API.md for the endpoints behind each.
#[async_trait]
pub trait LibrarySource: Send + Sync {
    /// The server's tag (`crate::sid`), which its ids carry. A cache from
    /// before tags is rewritten with it before the first sync. None: the
    /// source's ids are plain (tests).
    async fn server_tag(&self) -> Option<String> {
        None
    }

    /// Music sections on the selected server (empty = "no music library").
    async fn music_sections(&self) -> AppResult<Vec<MusicSection>>;

    /// Total items of a kind (`X-Plex-Container-Size=0` → `totalSize`).
    async fn count(&self, section_id: &str, kind: ItemKind) -> AppResult<u64>;

    /// Paged lists (`/library/sections/{id}/all?type=8|9|10`). Albums must
    /// come back with `format`/`subformats`/`section` already mapped (use
    /// [`release_section`](super::release_section)).
    async fn artists(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Artist>>;
    async fn albums(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Album>>;
    async fn tracks(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Track>>;

    /// Items whose updatedAt or addedAt is ≥ `since` (unix seconds).
    async fn changed_since(&self, section_id: &str, since: i64) -> AppResult<Changes>;

    /// These items as they are now (live updates from server events), sorted
    /// by kind; ids the server no longer has are left out. Albums come back
    /// with their release section mapped. None: not supported, so the caller
    /// runs a sync instead.
    async fn items(&self, _section_id: &str, _ids: &[String]) -> AppResult<Option<Changes>> {
        Ok(None)
    }

    /// Every current ratingKey of a kind (deletion detection).
    async fn all_ids(&self, section_id: &str, kind: ItemKind) -> AppResult<Vec<String>>;

    /// Play history for the section, newest first.
    async fn history(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<HistoryEntry>>;

    /// Audio playlists (`/playlists?playlistType=audio`).
    async fn playlists(&self) -> AppResult<Vec<Playlist>>;

    /// Track ratingKeys of a playlist, in order.
    async fn playlist_items(&self, playlist_id: &str) -> AppResult<Vec<String>>;

    /// A playlist's items in order with their playlistItemIDs (needed to
    /// remove or move one). Defaults to [`Self::playlist_items`] without ids.
    async fn playlist_entries(&self, playlist_id: &str) -> AppResult<Vec<PlaylistEntry>> {
        Ok(self
            .playlist_items(playlist_id)
            .await?
            .into_iter()
            .map(|track_id| PlaylistEntry { track_id, item_id: None })
            .collect())
    }

    /// Every liked (or, on a server with star ratings, rated) track in the
    /// section with its rating and when it was liked. Read on every sync so
    /// likes made in other apps arrive. A server with likes only gives
    /// [`super::likes::LIKED_RATING`]. None: not supported, likes come only
    /// with the track lists. Defaults to [`Self::rated_tracks`].
    async fn liked_tracks(&self, section_id: &str) -> AppResult<Option<Vec<super::likes::Rated>>> {
        self.rated_tracks(section_id).await
    }

    /// Star ratings, for servers that have them (Plex:
    /// `/library/sections/{id}/all?type=10&userRating>=1`).
    async fn rated_tracks(&self, _section_id: &str) -> AppResult<Option<Vec<super::likes::Rated>>> {
        Ok(None)
    }

    /// Similar artists from one artist's detail (`/library/metadata/{id}`).
    /// Not part of sync: called when an artist page opens (db::similar).
    async fn artist_similar(&self, _artist_id: &str) -> AppResult<Vec<SimilarArtist>> {
        Ok(Vec::new())
    }
}

/// Raw matches from the server's own search, before ranking.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteHits {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
}

/// Server-side search, used only while the first library sync is still
/// filling the cache (`/hubs/search`).
#[async_trait]
pub trait RemoteSearch: Send + Sync {
    async fn search(&self, query: &str, section_id: Option<&str>, limit: u32) -> AppResult<RemoteHits>;
}

/// Image bytes plus MIME type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtBytes {
    pub bytes: Bytes,
    pub content_type: String,
}

/// Fetches a resized Plex image (`/photo/:/transcode?url=…&width=…&height=…`).
#[async_trait]
pub trait ArtFetcher: Send + Sync {
    async fn fetch(&self, path: &str, width: u32, height: u32) -> AppResult<ArtBytes>;
}

/// Event sink for the db layer. Implemented for `tauri::AppHandle`, which
/// emits `sync://progress` and `library://changed` (docs/IPC.md).
pub trait DbEvents: Send + Sync {
    fn sync_progress(&self, status: &SyncStatus);
    fn library_changed(&self);
    /// Called once per sync run; map to `Health.banner`.
    fn sync_outcome(&self, _outcome: &SyncOutcome) {}
    /// One artist's page data changed outside a sync (similar artists).
    fn artist_changed(&self, _id: &str) {}
}

impl<R: tauri::Runtime> DbEvents for tauri::AppHandle<R> {
    fn sync_progress(&self, status: &SyncStatus) {
        use tauri::Emitter;
        if let Err(e) = self.emit("sync://progress", status) {
            tracing::warn!("emit sync://progress: {e}");
        }
    }

    fn library_changed(&self) {
        use tauri::Emitter;
        if let Err(e) = self.emit("library://changed", ()) {
            tracing::warn!("emit library://changed: {e}");
        }
    }

    fn artist_changed(&self, id: &str) {
        use tauri::Emitter;
        if let Err(e) = self.emit("artist://changed", serde_json::json!({ "id": id })) {
            tracing::warn!("emit artist://changed: {e}");
        }
    }
}
