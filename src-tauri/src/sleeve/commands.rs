//! Tauri commands for sleeve notes (docs/IPC.md "Sleeve notes"), and the
//! adapters that let the service read Plex's song credits and the user's
//! Discogs access.

use async_trait::async_trait;
use serde_json::Value;
use tauri::State;

use super::gather::PlexCredit;
use super::service::{DiscogsAccess, PlexCredits, SleeveService, SongCredits};
use super::store::CreditPage;
use super::SleeveNotes;
use crate::error::AppResult;

/// An album's sleeve notes from the cache; `sleeve://changed` follows when
/// a lookup it started finishes.
#[tauri::command]
pub async fn sleeve_notes(sleeve: State<'_, SleeveService>, id: String) -> AppResult<SleeveNotes> {
    sleeve.notes(&id).await
}

/// One song's credits, for the Credits dialog.
#[tauri::command]
pub async fn song_credits(sleeve: State<'_, SleeveService>, id: String) -> AppResult<SongCredits> {
    sleeve.song(&id).await
}

/// Everything in the library a person is credited on, by role. Null when
/// nothing credits them.
#[tauri::command]
pub async fn credit_page(sleeve: State<'_, SleeveService>, name: String) -> AppResult<Option<CreditPage>> {
    sleeve.credit_page(&name).await
}

/// [`PlexCredits`] through the Plex connection.
pub struct PlexSongCredits(pub crate::plex::PlexState);

#[async_trait]
impl PlexCredits for PlexSongCredits {
    async fn track_credits(&self, track_id: &str) -> AppResult<Vec<PlexCredit>> {
        let client = self.0.client_for(track_id).await?;
        Ok(crate::plex::details::fetch_credits(&client, track_id)
            .await?
            .into_iter()
            .map(|c| PlexCredit { name: c.name, roles: c.roles })
            .collect())
    }
}

#[async_trait]
impl DiscogsAccess for crate::physical::commands::Physical {
    async fn has_token(&self) -> bool {
        self.has_discogs_token().await
    }

    async fn owned_release(&self, album_id: &str) -> Option<u64> {
        self.discogs_release_of(album_id).await
    }

    async fn search(&self, artist: &str, title: &str) -> AppResult<Value> {
        self.discogs_search(artist, title).await
    }

    async fn release(&self, id: u64) -> AppResult<Option<Value>> {
        self.discogs_release(id).await
    }
}
