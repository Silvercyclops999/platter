//! Tauri commands for mixes (docs/IPC.md "Mixes and radio"). Radio plays
//! through `play_context` with `{ kind: "radio", id: "track:<id>" }` or
//! `"artist:<id>"`; a mix with `{ kind: "mix", id, trackIds? }`.

use tauri::State;

use crate::error::AppResult;

use super::{MixPage, MixService, MixShelves, MixSteer};

/// Home's mix shelves. `tz_offset_min`: the UI's time zone, minutes east of UTC.
#[tauri::command]
pub async fn mixes_home(mixes: State<'_, MixService>, tz_offset_min: Option<i32>) -> AppResult<MixShelves> {
    if let Some(m) = tz_offset_min {
        mixes.set_tz(m);
    }
    mixes.shelves().await
}

/// A mix and its songs; `steer` makes it again leaning one way.
#[tauri::command]
pub async fn mix_page(
    mixes: State<'_, MixService>,
    id: String,
    steer: Option<MixSteer>,
    tz_offset_min: Option<i32>,
) -> AppResult<MixPage> {
    if let Some(m) = tz_offset_min {
        mixes.set_tz(m);
    }
    mixes.page(id, steer).await
}

/// A song or artist radio page (`track:<id>` or `artist:<id>`).
#[tauri::command]
pub async fn radio_page(
    mixes: State<'_, MixService>,
    seed: String,
    steer: Option<MixSteer>,
    salt: Option<u64>,
) -> AppResult<MixPage> {
    mixes.radio_page(seed, steer, salt).await
}

/// Make one mix again with new songs.
#[tauri::command]
pub async fn mix_refresh(mixes: State<'_, MixService>, id: String) -> AppResult<MixPage> {
    mixes.refresh(id).await
}
