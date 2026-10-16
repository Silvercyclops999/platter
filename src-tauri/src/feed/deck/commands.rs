//! Tauri commands for the Stream Deck control API (docs/IPC.md "Stream Deck").
//! It's turned on and off with `settings_set` (`allowStreamDeck`). Also the
//! pairing prompt and installing the bundled plugin.

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use super::install;
use super::{DeckStatus, UiReport};
use super::server::DeckServer;
use crate::error::AppResult;

/// Whether the control API is on, its pairing code and whether a Stream
/// Deck is paired. Changes arrive as `deck://status`.
#[tauri::command]
pub async fn deck_status(deck: State<'_, DeckServer>) -> AppResult<DeckStatus> {
    let d = deck.inner().clone();
    tokio::task::spawn_blocking(move || d.status()).await.map_err(|e| crate::error::AppError::Internal(e.to_string()))
}

/// Forget the paired script or command line and show a new code.
#[tauri::command]
pub async fn automation_revoke(deck: State<'_, DeckServer>) -> AppResult<DeckStatus> {
    let d = deck.inner().clone();
    tokio::task::spawn_blocking(move || d.revoke_automation()).await.map_err(|e| crate::error::AppError::Internal(e.to_string()))?
}

/// Forget the paired Stream Deck and show a new code.
#[tauri::command]
pub async fn deck_revoke(deck: State<'_, DeckServer>) -> AppResult<DeckStatus> {
    let d = deck.inner().clone();
    tokio::task::spawn_blocking(move || d.revoke()).await.map_err(|e| crate::error::AppError::Internal(e.to_string()))?
}

/// The "Allow Stream Deck to control Platter?" prompt showing now, if any
/// (changes arrive as `deck://pair-request`).
#[tauri::command]
pub fn deck_pair_prompt(deck: State<'_, DeckServer>) -> Option<String> {
    deck.pair_prompt()
}

/// Answer that prompt. Only the window can allow a Stream Deck this way.
#[tauri::command]
pub async fn deck_pair_answer(deck: State<'_, DeckServer>, id: String, allow: bool) -> AppResult<bool> {
    let d = deck.inner().clone();
    tokio::task::spawn_blocking(move || d.pair_answer(&id, allow)).await.map_err(|e| crate::error::AppError::Internal(e.to_string()))?
}

fn bundled_plugin(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().resolve(install::RESOURCE, BaseDirectory::Resource).ok().filter(|p| p.is_file())
}

/// Whether the Stream Deck app is here, and the plugin installed and current.
#[tauri::command]
pub async fn deck_plugin_status(app: AppHandle) -> AppResult<install::PluginStatus> {
    let bundled = bundled_plugin(&app).and_then(|_| install::bundled_version());
    tokio::task::spawn_blocking(move || install::detect(&install::Places::system(), bundled.as_deref()))
        .await
        .map_err(|e| crate::error::AppError::Internal(e.to_string()))
}

/// Open the bundled plugin with the OS, so the Stream Deck app installs it.
#[tauri::command]
pub fn deck_plugin_install(app: AppHandle) -> AppResult<()> {
    let path = bundled_plugin(&app).ok_or_else(|| crate::error::AppError::NotFound("This copy of Platter has no Stream Deck plugin inside.".into()))?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| crate::error::AppError::Internal(format!("couldn't open the Stream Deck plugin: {e}")))
}

/// Open Elgato's download page for the Stream Deck app.
#[tauri::command]
pub fn deck_app_download(app: AppHandle) -> AppResult<()> {
    app.opener()
        .open_url(install::DOWNLOAD_URL, None::<&str>)
        .map_err(|e| crate::error::AppError::Internal(format!("open browser: {e}")))
}

/// What the window shows that a Stream Deck key reflects (the lyrics view).
#[tauri::command]
pub async fn deck_ui_report(deck: State<'_, DeckServer>, report: UiReport) -> AppResult<()> {
    deck.set_ui(report);
    Ok(())
}
