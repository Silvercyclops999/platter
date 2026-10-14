//! Tauri commands for docs/IPC.md "Auth & servers" (spec 02) and `health`
//! (spec 07). Registration happens in `lib.rs`:
//!
//! ```ignore
//! .setup(|app| {
//!     let dir = app.path().app_data_dir()?;
//!     let plex = plex::PlexState::new(dir)?;
//!     plex.start(app.handle().clone());
//!     app.manage(plex);
//!     Ok(())
//! })
//! .invoke_handler(tauri::generate_handler![
//!     plex::commands::auth_status,
//!     plex::commands::auth_start,
//!     plex::commands::auth_cancel,
//!     plex::commands::auth_sign_out,
//!     plex::commands::servers_list,
//!     plex::commands::server_select,
//!     plex::commands::health,
//! ])
//! ```
//!
//! Events: `auth://changed` (AuthStatus), `health://changed` (Health),
//! `toast` ({ kind, message }).

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::state::PlexState;
use super::types::{AuthStatus, Health, PinInfo, ServerInfo};
use crate::error::{AppError, AppResult};

/// Names of the commands in this module, for the integrator.
pub const COMMANDS: &[&str] = &[
    "auth_status",
    "auth_start",
    "auth_cancel",
    "auth_sign_out",
    "servers_list",
    "server_select",
    "health",
];

#[tauri::command]
pub async fn auth_status(state: State<'_, PlexState>) -> AppResult<AuthStatus> {
    Ok(state.auth_status().await)
}

/// Creates a PIN, opens the Plex sign-in page in the default browser and
/// polls in the background. Result arrives as `auth://changed`.
#[tauri::command]
pub async fn auth_start(app: AppHandle, state: State<'_, PlexState>) -> AppResult<PinInfo> {
    state
        .start_sign_in(|url| {
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|e| AppError::Internal(format!("open browser: {e}")))
        })
        .await
}

#[tauri::command]
pub async fn auth_cancel(state: State<'_, PlexState>) -> AppResult<()> {
    state.cancel_sign_in();
    Ok(())
}

#[tauri::command]
pub async fn auth_sign_out(state: State<'_, PlexState>) -> AppResult<()> {
    state.sign_out().await
}

#[tauri::command]
pub async fn servers_list(state: State<'_, PlexState>) -> AppResult<Vec<ServerInfo>> {
    Ok(state.servers().await)
}

#[tauri::command]
pub async fn server_select(state: State<'_, PlexState>, machine_id: String) -> AppResult<()> {
    state.select_server(&machine_id).await
}

#[tauri::command]
pub async fn health(state: State<'_, PlexState>) -> AppResult<Health> {
    Ok(state.health())
}
