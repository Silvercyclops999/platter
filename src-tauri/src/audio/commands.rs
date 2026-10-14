//! Tauri commands for playback & queue (docs/IPC.md). Not registered here:
//! the integrator adds them to `generate_handler!` and `manage`s a
//! [`PlayerHandle`]. Argument names are snake_case in Rust and arrive as
//! camelCase from JS (`positionMs`, `trackIds`, `itemId`, `toIndex`...).

use tauri::State;

use super::events::{PlayerState, Problem};
use super::output::OutputState;
use crate::settings::SettingsState;
use super::player::PlayerHandle;
use super::queue::{AddAt, Repeat};
use super::source::PlayContext;
use crate::error::AppResult;

#[tauri::command]
pub async fn play_context(
    player: State<'_, PlayerHandle>,
    context: PlayContext,
    start_track_id: Option<String>,
    shuffle: Option<bool>,
) -> AppResult<()> {
    player.play_context(context, start_track_id, shuffle).await
}

#[tauri::command]
pub async fn play_toggle(player: State<'_, PlayerHandle>) -> AppResult<()> {
    player.toggle().await
}

#[tauri::command]
pub async fn play_pause(player: State<'_, PlayerHandle>) -> AppResult<()> {
    player.pause().await
}

#[tauri::command]
pub async fn play_resume(player: State<'_, PlayerHandle>) -> AppResult<()> {
    player.resume().await
}

#[tauri::command]
pub async fn play_next(player: State<'_, PlayerHandle>) -> AppResult<()> {
    player.next().await
}

#[tauri::command]
pub async fn play_previous(player: State<'_, PlayerHandle>) -> AppResult<()> {
    player.previous().await
}

#[tauri::command]
pub async fn play_seek(player: State<'_, PlayerHandle>, position_ms: u64) -> AppResult<()> {
    player.seek(position_ms).await
}

#[tauri::command]
pub async fn set_volume(player: State<'_, PlayerHandle>, volume: f32) -> AppResult<()> {
    player.set_volume(volume).await
}

#[tauri::command]
pub async fn set_muted(player: State<'_, PlayerHandle>, muted: bool) -> AppResult<()> {
    player.set_muted(muted).await
}

#[tauri::command]
pub async fn set_shuffle(player: State<'_, PlayerHandle>, shuffle: bool) -> AppResult<()> {
    player.set_shuffle(shuffle).await
}

/// Shuffle preference for an album/playlist/artist by id. Applies now if
/// it's the playing context, otherwise the next time it plays.
#[tauri::command]
pub async fn set_context_shuffle(
    player: State<'_, PlayerHandle>,
    context_id: String,
    shuffle: bool,
) -> AppResult<()> {
    player.set_context_shuffle(context_id, shuffle).await
}

#[tauri::command]
pub async fn set_repeat(player: State<'_, PlayerHandle>, repeat: Repeat) -> AppResult<()> {
    player.set_repeat(repeat).await
}

#[tauri::command]
pub async fn queue_add(
    player: State<'_, PlayerHandle>,
    track_ids: Vec<String>,
    at: AddAt,
) -> AppResult<()> {
    player.queue_add(track_ids, at).await
}

#[tauri::command]
pub async fn queue_remove(player: State<'_, PlayerHandle>, item_id: String) -> AppResult<()> {
    player.queue_remove(item_id).await
}

#[tauri::command]
pub async fn queue_move(
    player: State<'_, PlayerHandle>,
    item_id: String,
    to_index: usize,
) -> AppResult<()> {
    player.queue_move(item_id, to_index).await
}

#[tauri::command]
pub async fn queue_clear(player: State<'_, PlayerHandle>) -> AppResult<()> {
    player.queue_clear().await
}

#[tauri::command]
pub async fn queue_jump(player: State<'_, PlayerHandle>, item_id: String) -> AppResult<()> {
    player.queue_jump(item_id).await
}

#[tauri::command]
pub async fn player_state(player: State<'_, PlayerHandle>) -> AppResult<PlayerState> {
    player.state().await
}

/// Audio output devices, the chosen one and the one in use.
#[tauri::command]
pub async fn output_devices(player: State<'_, PlayerHandle>) -> AppResult<OutputState> {
    player.output_state().await
}

/// Plays on another output device (`deviceId: null` follows the system
/// default), keeping position and play state, and remembers the choice.
#[tauri::command]
pub async fn output_select(
    player: State<'_, PlayerHandle>,
    settings: State<'_, SettingsState>,
    device_id: Option<String>,
) -> AppResult<OutputState> {
    let state = player.select_output(device_id).await?;
    settings.set_output_device(state.selected.clone()).await?;
    Ok(state)
}

/// Not in docs/IPC.md yet: the Problems list ("Skipped: file
/// unavailable" entries).
#[tauri::command]
pub async fn player_problems(player: State<'_, PlayerHandle>) -> AppResult<Vec<Problem>> {
    Ok(player.problems())
}
