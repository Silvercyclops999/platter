//! Tauri side of lyrics (docs/IPC.md `lyrics_get`) and the Plex source.

use tauri::State;

use super::{LyricsService, PlexLyrics};
use crate::error::AppResult;
use crate::plex::PlexState;
use crate::plex::details::{Lyrics, fetch_lyrics};

/// Lyrics streams from the signed-in Plex server.
pub struct PlexSource(pub PlexState);

#[async_trait::async_trait]
impl PlexLyrics for PlexSource {
    async fn lyrics(&self, track_id: &str) -> AppResult<Option<Lyrics>> {
        let client = self.0.client().await?;
        fetch_lyrics(&client, track_id).await
    }
}

#[tauri::command]
pub async fn lyrics_get(lyrics: State<'_, LyricsService>, track_id: String) -> AppResult<Option<Lyrics>> {
    lyrics.get(&track_id).await
}
