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

/// One row of Plex play history (`/status/sessions/history/all`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub rating_key: String,
    /// Unix seconds.
    pub viewed_at: i64,
}

/// The library as seen through Plex. All methods are paged/cheap where
/// Plex allows; see docs/PLEX-API.md for the endpoints behind each.
#[async_trait]
pub trait LibrarySource: Send + Sync {
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

    /// Every current ratingKey of a kind (deletion detection).
    async fn all_ids(&self, section_id: &str, kind: ItemKind) -> AppResult<Vec<String>>;

    /// Play history for the section, newest first.
    async fn history(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<HistoryEntry>>;

    /// Audio playlists (`/playlists?playlistType=audio`).
    async fn playlists(&self) -> AppResult<Vec<Playlist>>;

    /// Track ratingKeys of a playlist, in order.
    async fn playlist_items(&self, playlist_id: &str) -> AppResult<Vec<String>>;
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
}
