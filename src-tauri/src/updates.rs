//! Auto-update from GitHub releases. Checks shortly after launch, every
//! 6 hours, and when the window comes back to the front if the last check was
//! over an hour ago (the app often lives in the tray). Downloads in the background, then tells the UI an update is
//! ready. Installing only happens when the user asks (it restarts the app),
//! so playback is never cut off. A failed check or download changes nothing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::{AppError, AppResult};

const FIRST_CHECK: Duration = Duration::from_secs(10);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// While offline by choice, how often to see whether that changed.
const OFFLINE_RECHECK: Duration = Duration::from_secs(60);
/// Bringing the window back checks again once the last check is this old.
const STALE: Duration = Duration::from_secs(60 * 60);

#[derive(Default)]
pub struct Updates {
    /// A downloaded update waiting for the user to restart.
    ready: Mutex<Option<(Update, Vec<u8>)>>,
    /// When the last check started.
    last: Mutex<Option<Instant>>,
    /// Wakes the loop when the window comes back to the front.
    shown: Arc<tokio::sync::Notify>,
}

/// Whether a window coming back should start a check now.
fn is_stale(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.duration_since(t) >= STALE)
}

/// The main window came back to the front (opened from the tray, restored,
/// focused). Wakes the update loop, which checks if the last check is stale.
pub fn window_shown(app: &AppHandle) {
    if let Some(u) = app.try_state::<Updates>() {
        u.shown.notify_one();
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Ready {
    version: String,
}

fn err(e: tauri_plugin_updater::Error) -> AppError {
    AppError::Network(format!("update: {e}"))
}

/// Check, and download if there's something new. Returns what was found.
async fn check_and_download(app: &AppHandle) -> AppResult<UpdateInfo> {
    let state = app.state::<Updates>();
    *state.last.lock() = Some(Instant::now());
    if let Some((u, _)) = state.ready.lock().as_ref() {
        return Ok(UpdateInfo { available: true, version: Some(u.version.clone()), notes: u.body.clone() });
    }
    let Some(update) = app.updater().map_err(err)?.check().await.map_err(err)? else {
        return Ok(UpdateInfo { available: false, version: None, notes: None });
    };
    let info = UpdateInfo { available: true, version: Some(update.version.clone()), notes: update.body.clone() };
    let bytes = update.download(|_, _| {}, || {}).await.map_err(err)?;
    let version = update.version.clone();
    *state.ready.lock() = Some((update, bytes));
    let _ = app.emit("update://ready", Ready { version });
    Ok(info)
}

/// Background loop. Debug builds don't check on their own (no releases match
/// a dev version); `update_check` still works there.
pub fn start(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            // Go offline: no update checks.
            let offline = app
                .try_state::<crate::plex::PlexState>()
                .is_some_and(|p| p.offline_by_choice());
            if offline {
                // Look again once a minute, not every few seconds.
                tokio::time::sleep(OFFLINE_RECHECK).await;
                continue;
            }
            if let Err(e) = check_and_download(&app).await {
                tracing::info!("update check failed: {e}");
            }
            // Next check: the 6-hourly one, or sooner when the window comes
            // back after more than an hour.
            let shown = app.state::<Updates>().shown.clone();
            let deadline = tokio::time::Instant::now() + EVERY;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    _ = shown.notified() => {
                        let last = *app.state::<Updates>().last.lock();
                        if is_stale(last, Instant::now()) { break; }
                    }
                }
            }
        }
    });
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> AppResult<UpdateInfo> {
    check_and_download(&app).await
}

/// Install the downloaded update and restart. Called when the user clicks
/// "Restart to install"; on Windows the installer exits the app itself.
#[tauri::command]
pub async fn update_install_on_restart(app: AppHandle, state: State<'_, Updates>) -> AppResult<()> {
    let Some((update, bytes)) = state.ready.lock().take() else {
        return Err(AppError::Invalid("no update has been downloaded".into()));
    };
    if let Some(player) = app.try_state::<crate::audio::player::PlayerHandle>() {
        player.flush().await;
    }
    update.install(bytes).map_err(err)?;
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_coming_back_checks_only_when_the_last_check_is_old() {
        let now = Instant::now();
        assert!(is_stale(None, now));
        assert!(!is_stale(Some(now), now));
        assert!(!is_stale(Some(now - Duration::from_secs(59 * 60)), now));
        assert!(is_stale(Some(now - STALE), now));
    }
}
