//! Tauri commands for playback & queue (docs/IPC.md). Not registered here:
//! the integrator adds them to `generate_handler!` and `manage`s a
//! [`PlayerHandle`]. Argument names are snake_case in Rust and arrive as
//! camelCase from JS (`positionMs`, `trackIds`, `itemId`, `toIndex`...).

use tauri::State;

use super::events::{AbLoop, PlayerState, Problem};
use super::output::OutputState;
use crate::settings::{Settings, SettingsState};
use super::eqprofile;
use super::player::PlayerHandle;
use super::queue::{AddAt, Repeat};
use super::source::PlayContext;
use crate::error::{AppError, AppResult};
use crate::remote::RemoteService;

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

/// Smart shuffle for the playing playlist or Liked Songs: suggestions from
/// the library mixed in. Off turns shuffle off too, keeping the order.
#[tauri::command]
pub async fn set_smart_shuffle(player: State<'_, PlayerHandle>, on: bool) -> AppResult<()> {
    player.set_smart_shuffle(on).await
}

/// Smart shuffle for a playlist or Liked Songs by id. Applies now if it's
/// playing, otherwise the next time it plays.
#[tauri::command]
pub async fn set_context_smart_shuffle(player: State<'_, PlayerHandle>, context_id: String, on: bool) -> AppResult<()> {
    player.set_context_smart_shuffle(context_id, on).await
}

/// Skips the playing smart shuffle suggestion and picks new ones.
#[tauri::command]
pub async fn smart_suggestion_remove(player: State<'_, PlayerHandle>) -> AppResult<()> {
    player.smart_remove().await
}

/// The playing suggestion was added to its playlist or liked: it stays as
/// one of the playlist's songs.
#[tauri::command]
pub async fn smart_suggestion_keep(player: State<'_, PlayerHandle>, item_id: String) -> AppResult<()> {
    player.smart_keep(item_id).await
}

/// Crossfade for the playing context only (a mix's smooth transitions).
#[tauri::command]
pub async fn set_smooth_transitions(player: State<'_, PlayerHandle>, on: bool) -> AppResult<()> {
    player.set_smooth_transitions(on).await
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

/// Sets the A-B loop on the playing track (`abLoop: null` clears it).
#[tauri::command]
pub async fn set_ab_loop(player: State<'_, PlayerHandle>, ab_loop: Option<AbLoop>) -> AppResult<()> {
    player.set_ab_loop(ab_loop).await
}

#[tauri::command]
pub async fn queue_add(
    player: State<'_, PlayerHandle>,
    remote: State<'_, RemoteService>,
    track_ids: Vec<String>,
    at: AddAt,
) -> AppResult<()> {
    player.queue_add(track_ids, at).await?;
    edited(&remote).await
}

#[tauri::command]
pub async fn queue_remove(
    player: State<'_, PlayerHandle>,
    remote: State<'_, RemoteService>,
    item_id: String,
) -> AppResult<()> {
    player.queue_remove(item_id).await?;
    edited(&remote).await
}

#[tauri::command]
pub async fn queue_move(
    player: State<'_, PlayerHandle>,
    remote: State<'_, RemoteService>,
    item_id: String,
    to_index: usize,
) -> AppResult<()> {
    player.queue_move(item_id, to_index).await?;
    edited(&remote).await
}

#[tauri::command]
pub async fn queue_clear(player: State<'_, PlayerHandle>, remote: State<'_, RemoteService>) -> AppResult<()> {
    player.queue_clear().await?;
    edited(&remote).await
}

/// While another device plays, it gets the edit too.
async fn edited(remote: &RemoteService) -> AppResult<()> {
    if remote.is_remote() { remote.queue_edited().await } else { Ok(()) }
}

#[tauri::command]
pub async fn queue_jump(player: State<'_, PlayerHandle>, item_id: String) -> AppResult<()> {
    player.queue_jump(item_id).await
}

#[tauri::command]
pub async fn player_state(player: State<'_, PlayerHandle>) -> AppResult<PlayerState> {
    player.state().await.map(|s| s.for_ui().into_owned())
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

// ------------------------------------------------------------------- EQ

/// Saves the current EQ curve as a named profile (replacing one with that
/// name) and selects it. Returns the new settings.
#[tauri::command]
pub async fn eq_profile_save(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
    name: String,
) -> AppResult<Settings> {
    change_eq(&settings, &player, |s| eqprofile::save_as(s, &name)).await
}

/// Renames a profile, keeping the outputs that use it.
#[tauri::command]
pub async fn eq_profile_rename(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
    from: String,
    to: String,
) -> AppResult<Settings> {
    change_eq(&settings, &player, |s| eqprofile::rename(s, &from, &to)).await
}

/// Deletes a profile; outputs using it lose their binding.
#[tauri::command]
pub async fn eq_profile_delete(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
    name: String,
) -> AppResult<Settings> {
    change_eq(&settings, &player, |s| eqprofile::delete(s, &name)).await
}

/// Uses the selected preset or profile whenever the current output device
/// plays (`on: false` removes that).
#[tauri::command]
pub async fn eq_use_for_output(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
    on: bool,
) -> AppResult<Settings> {
    let device = player
        .output_state()
        .await?
        .current
        .ok_or_else(|| AppError::NotFound("No output device is in use.".into()))?;
    change_eq(&settings, &player, |s| eqprofile::use_for_output(s, &device, on)).await
}

async fn change_eq(
    settings: &SettingsState,
    player: &PlayerHandle,
    f: impl FnOnce(&mut Settings) -> AppResult<()>,
) -> AppResult<Settings> {
    let s = match settings.modify(|s| f(s).map(|_| true)).await? {
        Some(s) => s,
        None => settings.get().await?,
    };
    player.set_dsp(s.dsp()).await?;
    Ok(s)
}

/// Settings changed by the core itself (an output switch moved the EQ).
pub const EV_SETTINGS: &str = "settings://changed";

/// Switches the EQ to the profile bound to the output device in use, now
/// and whenever the device changes, and tells the UI (`settings://changed`).
pub async fn follow_output<R: tauri::Runtime>(app: tauri::AppHandle<R>, player: PlayerHandle, settings: SettingsState) {
    use tauri::Emitter;
    use tokio::sync::broadcast::error::RecvError;
    let mut rx = player.subscribe_output();
    let mut last: Option<Option<String>> = None;
    let mut next = player.output_state().await.ok();
    loop {
        if let Some(state) = next.take() {
            let id = state.current.as_ref().map(|d| d.id.clone());
            if last.as_ref() != Some(&id) {
                last = Some(id);
                let device = state.current.clone();
                match settings.modify(|s| Ok(eqprofile::output_changed(s, device.as_ref()))).await {
                    Ok(Some(s)) => {
                        let _ = player.set_dsp(s.dsp()).await;
                        let _ = app.emit(EV_SETTINGS, &s);
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("EQ output switch failed: {e}"),
                }
            }
        }
        match rx.recv().await {
            Ok(state) => next = Some(state),
            Err(RecvError::Lagged(_)) => next = player.output_state().await.ok(),
            Err(RecvError::Closed) => return,
        }
    }
}
