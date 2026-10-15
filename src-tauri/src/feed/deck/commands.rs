//! Tauri commands for the Stream Deck control API (docs/IPC.md "Stream Deck").
//! It's turned on and off with `settings_set` (`allowStreamDeck`).

use tauri::State;

use super::DeckStatus;
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
