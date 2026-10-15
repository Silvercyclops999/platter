//! The release radar's Tauri commands (docs/IPC.md) and app glue.

use std::sync::Arc;

use async_trait::async_trait;
use tauri::State;

use super::{AutoArtist, Radar, RadarPage};
use crate::error::{AppError, AppResult};
use crate::lidarr::ArtistMbid;
use crate::lidarr::mbpick::Picker;

/// An artist's MusicBrainz id: the artist info cache's (Plex's, else
/// Wikidata's), else an exact name match on MusicBrainz (cached a week).
pub struct RadarMbids {
    pub info: Arc<dyn ArtistMbid>,
    pub picker: Picker,
}

#[async_trait]
impl ArtistMbid for RadarMbids {
    async fn artist_mbid(&self, artist_id: &str, name: &str) -> Option<String> {
        if !artist_id.is_empty()
            && let Some(m) = self.info.artist_mbid(artist_id, name).await
        {
            return Some(m);
        }
        self.picker.artist_mbid(name).await.ok().flatten()
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::Internal(e.to_string()))?
}

/// New releases found (empty while the setting is off).
#[tauri::command]
pub async fn release_radar(radar: State<'_, Radar>) -> AppResult<RadarPage> {
    let r = radar.inner().clone();
    blocking(move || r.page()).await
}

/// Look now instead of waiting for the daily look.
#[tauri::command]
pub async fn release_radar_check(radar: State<'_, Radar>) -> AppResult<RadarPage> {
    let r = radar.inner().clone();
    r.check().await?;
    blocking(move || r.page()).await
}

/// Artists with auto-request on.
#[tauri::command]
pub async fn radar_auto_artists(radar: State<'_, Radar>) -> AppResult<Vec<AutoArtist>> {
    let r = radar.inner().clone();
    blocking(move || Ok(r.auto_artists())).await
}

/// Turn auto-request on or off for an artist.
#[tauri::command]
pub async fn radar_auto_set(radar: State<'_, Radar>, id: String, name: String, on: bool) -> AppResult<Vec<AutoArtist>> {
    let r = radar.inner().clone();
    blocking(move || r.set_auto(&id, &name, on)).await
}
