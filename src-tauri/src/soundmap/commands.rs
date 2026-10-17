//! Tauri commands for the sound map: how far along it is, the points in
//! chunks (optionally one area at a time), finding a song or an artist on
//! it, and the hover card. All read the local database only.

use serde::Serialize;
use tauri::State;

use super::SoundMap;
use super::store::{self, Area, Chunk, Counts, Found, Song};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    #[serde(flatten)]
    pub counts: Counts,
    /// Song analysis is running (Automix on), so more songs will come.
    pub analysing: bool,
    /// The map is being updated right now.
    pub building: bool,
}

/// Progress, and a nudge to place any songs analysed since the last update.
#[tauri::command]
pub async fn soundmap_info(map: State<'_, SoundMap>) -> AppResult<Info> {
    let counts = map.db().read_async(store::counts).await?;
    if counts.analysed > counts.placed {
        map.kick();
    }
    Ok(Info { counts, analysing: crate::analysis::enabled(), building: map.building() })
}

/// Up to `limit` points (default and most [`store::CHUNK_MAX`]) after
/// `after`, within `area` when given.
#[tauri::command]
pub async fn soundmap_points(map: State<'_, SoundMap>, after: Option<String>, limit: Option<u32>, area: Option<Area>) -> AppResult<Chunk> {
    let limit = limit.map(|l| l as usize).unwrap_or(store::CHUNK_MAX);
    map.db().read_async(move |c| store::points(c, after.as_deref(), limit, area)).await
}

/// Artists and songs on the map matching `query`.
#[tauri::command]
pub async fn soundmap_find(map: State<'_, SoundMap>, query: String) -> AppResult<Vec<Found>> {
    map.db().read_async(move |c| store::find(c, &query)).await
}

/// The title, artist and cover for the hover card.
#[tauri::command]
pub async fn soundmap_song(map: State<'_, SoundMap>, id: String) -> AppResult<Song> {
    let missing = id.clone();
    map.db().read_async(move |c| store::song(c, &id)).await?.ok_or(AppError::NotFound(missing))
}
