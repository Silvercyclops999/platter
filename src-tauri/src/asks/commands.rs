//! Requests' Tauri commands (docs/IPC.md).

use tauri::State;

use super::wire::AskItem;
use super::{Answered, Asks, AsksView, Incoming, MyAsk};
use crate::error::AppResult;

/// The asking side for the connected server: whether Request shows, and
/// my requests.
#[tauri::command]
pub async fn asks_view(asks: State<'_, Asks>) -> AppResult<AsksView> {
    Ok(asks.view().await)
}

/// Request an album (or a song, with its album) from the server's owner.
#[tauri::command]
pub async fn asks_request(asks: State<'_, Asks>, item: AskItem) -> AppResult<MyAsk> {
    asks.request(item).await
}

/// Take a request back before the owner answers.
#[tauri::command]
pub async fn asks_cancel(asks: State<'_, Asks>, id: String) -> AppResult<()> {
    asks.cancel(&id).await
}

/// The owner's side: requests people made.
#[tauri::command]
pub async fn asks_incoming(asks: State<'_, Asks>) -> AppResult<Vec<Incoming>> {
    Ok(asks.incoming())
}

/// The owner's yes or no.
#[tauri::command]
pub async fn asks_answer(asks: State<'_, Asks>, id: String, yes: bool) -> AppResult<Answered> {
    asks.answer(&id, yes).await
}
