//! Settings > Bridge's Tauri commands (docs/IPC.md "Platter Bridge").

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use super::desktop::{BridgeInfo, BridgeOverview, Desktop, PairStart};
use crate::error::AppResult;
use crate::lidarr::Lidarr;
use crate::sync::devices::commands::DeviceService;
use crate::sync::devices::home::ComputerSide;

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

/// "Add the Bridge": the code a Bridge shows in its log (PBB-). Gives back
/// the Bridge's name once it answered.
#[tauri::command]
pub async fn bridge_join(bridge: State<'_, Arc<Desktop>>, code: String) -> AppResult<String> {
    bridge.join_bridge(&code).await
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

/// One home side, for Settings (Lidarr > Home side).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeSide {
    /// `computer` or `bridge`.
    pub kind: &'static str,
    pub name: String,
    /// One plain line: what it is and whether it's on.
    pub words: String,
    pub on: bool,
    /// The one that answers first when the home and away addresses don't.
    pub answers: bool,
}

/// Which home sides this device has, which answers, and what's wrong.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeSides {
    /// This device answers Lidarr for the user's other devices now.
    pub offering: bool,
    pub sides: Vec<HomeSide>,
    /// Nothing answers away from home: the words to show.
    pub problem: Option<String>,
}

/// The home sides in the order Lidarr tries them: computers at home, then
/// Bridges. `away_set`: this device has its own away address, so it isn't
/// stuck without a home side.
pub fn home_sides(computers: Vec<ComputerSide>, bridges: &[BridgeInfo], offering: bool, away_set: bool) -> HomeSides {
    let mut sides: Vec<HomeSide> = computers
        .into_iter()
        .map(|c| HomeSide {
            kind: "computer",
            words: if c.on { "Your computer at home, on now".into() } else { "Your computer at home, off or away now".into() },
            name: c.name,
            on: c.on,
            answers: false,
        })
        .collect();
    sides.extend(bridges.iter().map(|b| HomeSide {
        kind: "bridge",
        name: b.name.clone(),
        words: if b.online { "Platter Bridge, on all the time".into() } else { "Platter Bridge, not heard from lately".into() },
        on: b.online,
        answers: false,
    }));
    if let Some(first) = sides.iter_mut().find(|s| s.on) {
        first.answers = true;
    }
    let none_on = !sides.iter().any(|s| s.on);
    let words = if bridges.is_empty() { crate::lidarr::HOME_SIDE_OFF } else { crate::lidarr::BRIDGE_QUIET };
    HomeSides { offering, problem: (none_on && !offering && !away_set).then(|| words.to_string()), sides }
}

/// Settings > Lidarr > Home side.
#[tauri::command]
pub async fn lidarr_home_sides(
    bridge: State<'_, Arc<Desktop>>,
    lidarr: State<'_, Lidarr>,
    devices: State<'_, DeviceService>,
) -> AppResult<HomeSides> {
    let away = lidarr.status().away_set;
    let bridges = bridge.overview().bridges;
    if !bridges.is_empty() && !bridges.iter().any(|b| b.online) {
        // Not answering: look for it on the Plex account again now.
        bridge.relook_now();
    }
    Ok(home_sides(devices.sync.home_computers(), &bridges, devices.sync.peer.offers_lidarr(), away))
}
