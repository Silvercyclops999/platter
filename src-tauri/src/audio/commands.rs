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
    start_index: Option<usize>,
    shuffle: Option<bool>,
) -> AppResult<()> {
    player.play_context_from(context, start_track_id, start_index, shuffle).await
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
    remote.edit_queue(|| player.queue_add(track_ids, at)).await
}

#[tauri::command]
pub async fn queue_remove(
    player: State<'_, PlayerHandle>,
    remote: State<'_, RemoteService>,
    item_id: String,
) -> AppResult<()> {
    remote.edit_queue(|| player.queue_remove(item_id)).await
}

#[tauri::command]
pub async fn queue_move(
    player: State<'_, PlayerHandle>,
    remote: State<'_, RemoteService>,
    item_id: String,
    to_index: usize,
) -> AppResult<()> {
    remote.edit_queue(|| player.queue_move(item_id, to_index)).await
}

// While another device plays, it gets each edit too (`edit_queue`).
#[tauri::command]
pub async fn queue_clear(player: State<'_, PlayerHandle>, remote: State<'_, RemoteService>) -> AppResult<()> {
    remote.edit_queue(|| player.queue_clear()).await
}

#[tauri::command]
pub async fn queue_jump(player: State<'_, PlayerHandle>, item_id: String) -> AppResult<()> {
    player.queue_jump(item_id).await
}

#[tauri::command]
pub async fn player_state(player: State<'_, PlayerHandle>) -> AppResult<PlayerState> {
    player.state().await.map(|s| s.for_ui().into_owned())
}

/// What happens to the sound now, from the file to the output device
/// (also sent as `audio://signal-path` when it changes).
#[tauri::command]
pub async fn signal_path(player: State<'_, PlayerHandle>) -> AppResult<super::signal::SignalPath> {
    player.signal_path().await
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
                let changed = settings
                    .modify(|s| {
                        let before = s.eq_last_output.clone();
                        let eq = eqprofile::output_changed(s, device.as_ref());
                        Ok(eq || s.eq_last_output != before)
                    })
                    .await;
                // The device's own EQ, headphone correction and bit-perfect choice.
                match changed {
                    Ok(Some(s)) => {
                        let _ = super::hifi::apply_for_output(&player, &s).await;
                        let _ = app.emit(EV_SETTINGS, &s);
                    }
                    Ok(None) => {
                        if let Ok(s) = settings.get().await {
                            let _ = super::hifi::apply_for_output(&player, &s).await;
                        }
                    }
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

/// The battery state for the visualizer: read from a cached value, so the
/// command never waits on the system. A stale value starts one refresh in
/// the background (never two at once), which gives up after a few seconds
/// (on a Mac asking runs a tool).
pub struct BatteryCache {
    on: std::sync::atomic::AtomicBool,
    /// Seconds since the epoch of the last answer (0: never).
    at: std::sync::atomic::AtomicU64,
    refreshing: std::sync::atomic::AtomicBool,
}

/// How old the cached battery state may get.
pub const BATTERY_FRESH_S: u64 = 30;
/// How long one refresh may take before it is given up.
pub const BATTERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// A refresh to run on the async runtime.
pub type Refresh = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

impl BatteryCache {
    pub const fn new() -> Self {
        use std::sync::atomic::{AtomicBool, AtomicU64};
        Self { on: AtomicBool::new(false), at: AtomicU64::new(0), refreshing: AtomicBool::new(false) }
    }

    /// The cached state now. When it is stale, `spawn` gets a future that
    /// asks `probe` (on a blocking thread, with a timeout) and stores the
    /// answer; only one runs at a time.
    pub fn get(&'static self, now_s: u64, spawn: impl FnOnce(Refresh), probe: fn() -> bool) -> bool {
        use std::sync::atomic::Ordering;
        let stale = now_s.saturating_sub(self.at.load(Ordering::Relaxed)) >= BATTERY_FRESH_S;
        if stale && !self.refreshing.swap(true, Ordering::SeqCst) {
            spawn(Box::pin(async move {
                let mut task = tokio::task::spawn_blocking(probe);
                match tokio::time::timeout(BATTERY_TIMEOUT, &mut task).await {
                    Ok(Ok(v)) => self.on.store(v, Ordering::Relaxed),
                    Ok(Err(_)) => {}
                    // A probe that hangs keeps the last answer for another
                    // stretch, and no second probe starts until it returns,
                    // so a stuck one can't pile up blocking threads.
                    Err(_) => {
                        self.at.store(now_s, Ordering::Relaxed);
                        let _ = task.await;
                        self.refreshing.store(false, Ordering::SeqCst);
                        return;
                    }
                }
                self.at.store(now_s, Ordering::Relaxed);
                self.refreshing.store(false, Ordering::SeqCst);
            }));
        }
        self.on.load(Ordering::Relaxed)
    }
}

impl Default for BatteryCache {
    fn default() -> Self {
        Self::new()
    }
}

static BATTERY: BatteryCache = BatteryCache::new();

fn now_s() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether a visualizer watch may run: the asking window is on screen, and
/// on battery only when the setting allows it.
pub fn visualizer_allowed(on: bool, window_seen: bool, pause_on_battery: bool, on_battery: impl FnOnce() -> bool) -> bool {
    on && window_seen && !(pause_on_battery && on_battery())
}

/// Starts or renews this window's visualizer watch: `player://spectrum`
/// frames come at `fps` (1 to 60) until it is turned off or not renewed
/// within 3 s. Each window has its own watch. `scope` adds the stereo
/// pairs. Returns whether it runs: not while the window is minimised or
/// hidden, nor on battery with `pauseOnBattery`.
#[tauri::command]
pub async fn visualizer_watch(
    player: State<'_, PlayerHandle>,
    window: tauri::WebviewWindow,
    on: bool,
    fps: u32,
    scope: Option<bool>,
    pause_on_battery: Option<bool>,
) -> AppResult<bool> {
    #[cfg(desktop)]
    let seen = window.is_visible().unwrap_or(true) && !window.is_minimized().unwrap_or(false);
    #[cfg(mobile)]
    let seen = true;
    let rest_on_battery = pause_on_battery.unwrap_or(true);
    let battery = || BATTERY.get(now_s(), |f| drop(tauri::async_runtime::spawn(f)), crate::platform::power::visualizer_should_rest);
    let run = visualizer_allowed(on, seen, rest_on_battery, battery);
    Ok(player.visualizer_watch(window.label(), run, fps, scope.unwrap_or(false)) && run)
}

#[cfg(test)]
mod visualizer_tests {
    use super::visualizer_allowed;

    #[test]
    fn the_visualizer_runs_only_when_seen_and_not_resting_on_battery() {
        assert!(visualizer_allowed(true, true, true, || false));
        assert!(!visualizer_allowed(true, false, true, || false), "minimised or hidden");
        assert!(!visualizer_allowed(true, true, true, || true), "on battery, resting");
        assert!(visualizer_allowed(true, true, false, || true), "on battery, allowed");
        assert!(!visualizer_allowed(false, true, false, || false));
    }

    #[tokio::test]
    async fn the_battery_state_comes_from_a_cache_refreshed_once_at_a_time() {
        use super::{BATTERY_FRESH_S, BatteryCache};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CACHE: BatteryCache = BatteryCache::new();
        static ASKED: AtomicUsize = AtomicUsize::new(0);
        fn probe() -> bool {
            ASKED.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(50));
            true
        }
        let mut spawned = Vec::new();
        // Stale at first: the answer now is the cached one, one refresh starts.
        assert!(!CACHE.get(1_000, |f| spawned.push(tokio::spawn(f)), probe));
        // A second ask while it runs starts nothing more.
        assert!(!CACHE.get(1_000, |f| spawned.push(tokio::spawn(f)), probe));
        assert_eq!(spawned.len(), 1);
        for h in spawned.drain(..) {
            h.await.unwrap();
        }
        assert_eq!(ASKED.load(Ordering::SeqCst), 1);
        // Fresh: read from the cache, nothing asked.
        assert!(CACHE.get(1_010, |f| spawned.push(tokio::spawn(f)), probe));
        assert!(spawned.is_empty());
        // Stale again: refreshed again.
        assert!(CACHE.get(1_000 + BATTERY_FRESH_S, |f| spawned.push(tokio::spawn(f)), probe));
        assert_eq!(spawned.len(), 1);
    }
}
