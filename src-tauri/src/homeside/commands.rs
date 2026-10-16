//! Settings > Bridge's Tauri commands (docs/IPC.md "Platter Bridge").

use std::sync::Arc;

use tauri::State;

use super::desktop::{BridgeOverview, Desktop, PairStart};
use crate::error::AppResult;
use crate::lidarr::Lidarr;

/// Paired Bridges, a code waiting to be used, and queued Lidarr changes.
#[tauri::command]
pub async fn bridge_overview(bridge: State<'_, Arc<Desktop>>) -> AppResult<BridgeOverview> {
    Ok(bridge.overview())
}

/// "Pair a Bridge": a new code, good for an hour.
#[tauri::command]
pub async fn bridge_pair_start(bridge: State<'_, Arc<Desktop>>, db: State<'_, crate::db::DbHandle>) -> AppResult<PairStart> {
    let relay = crate::social::listen::invite::relay_url(&crate::settings::load(&db).listen_relay);
    bridge.inner().pair_start(relay)
}

#[tauri::command]
pub async fn bridge_pair_cancel(bridge: State<'_, Arc<Desktop>>) -> AppResult<()> {
    bridge.pair_cancel();
    Ok(())
}

#[tauri::command]
pub async fn bridge_unpair(bridge: State<'_, Arc<Desktop>>, id: String) -> AppResult<BridgeOverview> {
    bridge.unpair(&id).await?;
    Ok(bridge.overview())
}

/// Say yes to a Lidarr settings change a Bridge queued.
#[tauri::command]
pub async fn bridge_queued_apply(bridge: State<'_, Arc<Desktop>>, lidarr: State<'_, Lidarr>, id: String) -> AppResult<BridgeOverview> {
    lidarr.apply_queued(&id).await?;
    Ok(bridge.overview())
}

#[tauri::command]
pub async fn bridge_queued_dismiss(bridge: State<'_, Arc<Desktop>>, lidarr: State<'_, Lidarr>, id: String) -> AppResult<BridgeOverview> {
    lidarr.dismiss_queued(&id)?;
    Ok(bridge.overview())
}

/// Send the Bridges the current Lidarr and Plex settings (after a change).
#[tauri::command]
pub async fn bridge_send_settings(bridge: State<'_, Arc<Desktop>>) -> AppResult<()> {
    bridge.send_config().await;
    Ok(())
}
