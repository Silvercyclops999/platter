//! Auto-update from GitHub releases. Checks shortly after launch, every
//! 6 hours, and when the window comes back to the front if the last check was
//! over an hour ago (the app often lives in the tray). Downloads in the
//! background. With "Update by itself" on (the default), an update found just
//! after launch installs straight away when nothing is playing (a short note
//! first, which can put it off), and any other waiting update installs when
//! Platter quits. Playback is never cut off. With it off, or while music
//! plays, the UI offers a restart. A failed check, download or install
//! changes nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::{AppError, AppResult};
use crate::plex::types::{EVENT_TOAST, Toast};

const FIRST_CHECK: Duration = Duration::from_secs(10);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// While offline by choice, how often to see whether that changed.
const OFFLINE_RECHECK: Duration = Duration::from_secs(60);
/// Bringing the window back checks again once the last check is this old.
const STALE: Duration = Duration::from_secs(60 * 60);
/// An update downloaded this soon after launch installs straight away.
const LAUNCH_WINDOW: Duration = Duration::from_secs(3 * 60);
/// How long "Updating Platter" shows before the restart, so it can be put off.
const NOTICE: Duration = Duration::from_secs(8);

pub struct Updates {
    /// A downloaded update waiting to be installed.
    ready: Mutex<Option<(Update, Vec<u8>)>>,
    /// When the last check started.
    last: Mutex<Option<Instant>>,
    /// Wakes the loop when the window comes back to the front.
    shown: Arc<tokio::sync::Notify>,
    /// Update by itself (a setting, this device only).
    pub auto: Arc<AtomicBool>,
    /// The user said Not now to installing: it waits for quit.
    put_off: AtomicBool,
    started: Instant,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            ready: Mutex::default(),
            last: Mutex::default(),
            shown: Arc::default(),
            auto: Arc::new(AtomicBool::new(true)),
            put_off: AtomicBool::new(false),
            started: Instant::now(),
        }
    }
}

/// What to do with an update that just finished downloading.
#[derive(Debug, PartialEq, Eq)]
enum Install {
    /// Just launched and nothing playing: say so, then restart into it.
    Now,
    /// Offer a restart; it installs when Platter quits.
    OnQuit,
    /// Update by itself is off: offer a restart, nothing more.
    Ask,
}

fn install_when(auto: bool, since_launch: Duration, playing: bool, put_off: bool) -> Install {
    if !auto {
        Install::Ask
    } else if since_launch <= LAUNCH_WINDOW && !playing && !put_off {
        Install::Now
    } else {
        Install::OnQuit
    }
}

/// Whether an exit should install the waiting update first. Not on a
/// restart: the Windows installer would leave the app closed.
fn install_on_exit(auto: bool, code: Option<i32>) -> bool {
    auto && code != Some(tauri::RESTART_EXIT_CODE)
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
#[serde(rename_all = "camelCase")]
struct Ready {
    version: String,
    /// It installs when Platter quits (Update by itself is on).
    on_quit: bool,
}

#[derive(Debug, Clone, Serialize)]
struct Installing {
    version: String,
    seconds: u64,
}

fn err(e: tauri_plugin_updater::Error) -> AppError {
    AppError::Network(format!("update: {e}"))
}

/// What a check means for the update already downloaded, if any.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// The one waiting is still the newest.
    Keep,
    /// The release went away: the one waiting is dropped.
    Forget,
    /// Something newer (or the first one): download it.
    Download,
    Nothing,
}

fn next(waiting: Option<&str>, found: Option<&str>) -> Next {
    match (waiting, found) {
        (Some(w), Some(f)) if w == f => Next::Keep,
        (_, Some(_)) => Next::Download,
        (Some(_), None) => Next::Forget,
        (None, None) => Next::Nothing,
    }
}

/// Takes what is in `slot` and installs it; if installing fails it goes
/// back, so the user can try again without downloading it again.
fn install_or_keep<T>(slot: &Mutex<Option<T>>, install: impl FnOnce(&T) -> AppResult<()>) -> AppResult<()> {
    let Some(item) = slot.lock().take() else {
        return Err(AppError::Invalid("no update has been downloaded".into()));
    };
    install(&item).inspect_err(|_| *slot.lock() = Some(item))
}

/// Check, and download if there's something new. Returns what was found,
/// and the version when it was downloaded just now.
async fn check_and_download(app: &AppHandle) -> AppResult<(UpdateInfo, Option<String>)> {
    let state = app.state::<Updates>();
    *state.last.lock() = Some(Instant::now());
    let waiting = state
        .ready
        .lock()
        .as_ref()
        .map(|(u, _)| UpdateInfo { available: true, version: Some(u.version.clone()), notes: u.body.clone() });
    let found = match app.updater().map_err(err)?.check().await {
        Ok(f) => f,
        // Can't ask now: the one already downloaded still stands.
        Err(e) => return waiting.map(|w| (w, None)).ok_or_else(|| err(e)),
    };
    let none = UpdateInfo { available: false, version: None, notes: None };
    match next(waiting.as_ref().and_then(|w| w.version.as_deref()), found.as_ref().map(|u| u.version.as_str())) {
        Next::Keep => return Ok((waiting.unwrap_or(none), None)),
        Next::Forget => {
            *state.ready.lock() = None;
            return Ok((none, None));
        }
        Next::Nothing => return Ok((none, None)),
        Next::Download => {}
    }
    let Some(update) = found else {
        return Ok((none, None));
    };
    let info = UpdateInfo { available: true, version: Some(update.version.clone()), notes: update.body.clone() };
    let bytes = update.download(|_, _| {}, || {}).await.map_err(err)?;
    let version = update.version.clone();
    *state.ready.lock() = Some((update, bytes));
    Ok((info, Some(version)))
}

fn say_ready(app: &AppHandle, version: String) {
    let on_quit = app.state::<Updates>().auto.load(Ordering::Relaxed);
    let _ = app.emit("update://ready", Ready { version, on_quit });
}

async fn playing(app: &AppHandle) -> bool {
    use crate::audio::events::PlayerStatus;
    let Some(player) = app.try_state::<crate::audio::player::PlayerHandle>() else {
        return false;
    };
    player
        .state()
        .await
        .is_ok_and(|s| matches!(s.status, PlayerStatus::Playing | PlayerStatus::Loading | PlayerStatus::Buffering))
}

/// An update just finished downloading in the background: install it now
/// (just launched, nothing playing), or say it's ready.
async fn downloaded(app: &AppHandle, version: String) {
    let state = app.state::<Updates>();
    let when = install_when(
        state.auto.load(Ordering::Relaxed),
        state.started.elapsed(),
        playing(app).await,
        state.put_off.load(Ordering::Relaxed),
    );
    if when != Install::Now {
        return say_ready(app, version);
    }
    let _ = app.emit("update://installing", Installing { version: version.clone(), seconds: NOTICE.as_secs() });
    tokio::time::sleep(NOTICE).await;
    // Put off, or music started meanwhile: it waits for quit.
    if state.put_off.load(Ordering::Relaxed) || playing(app).await {
        return say_ready(app, version);
    }
    if let Err(e) = install_and_restart(app).await {
        tracing::warn!("installing the update failed: {e}");
        let toast = Toast { kind: "error".into(), message: "Couldn't install the update. Platter tries again when you quit.".into() };
        let _ = app.emit(EVENT_TOAST, toast);
    }
}

/// Flush playback state, install the waiting update and restart into it.
/// A failed install keeps the update.
async fn install_and_restart(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<Updates>();
    if state.ready.lock().is_none() {
        return Err(AppError::Invalid("no update has been downloaded".into()));
    }
    if let Some(player) = app.try_state::<crate::audio::player::PlayerHandle>() {
        player.flush().await;
    }
    install_or_keep(&state.ready, |(update, bytes)| update.install(bytes).map_err(err))?;
    app.restart();
}

/// Platter is quitting: with Update by itself on, install the waiting update
/// so the next launch is the new version. On Windows the installer runs on
/// its own (a small progress window) and doesn't reopen the app. Called after
/// playback state is saved.
pub fn quitting(app: &AppHandle, code: Option<i32>) {
    let Some(state) = app.try_state::<Updates>() else { return };
    if !install_on_exit(state.auto.load(Ordering::Relaxed), code) {
        return;
    }
    let Some((update, bytes)) = state.ready.lock().take() else { return };
    tracing::info!(version = %update.version, "installing the update on the way out");
    #[cfg(windows)]
    app.cleanup_before_exit();
    if let Err(e) = update.restart_after_install(false).install(&bytes) {
        tracing::warn!("installing the update on quit failed: {e}");
    }
}

#[tauri::command]
pub fn update_put_off(state: State<'_, Updates>) {
    state.put_off.store(true, Ordering::Relaxed);
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
            match check_and_download(&app).await {
                Ok((_, Some(version))) => downloaded(&app, version).await,
                Ok(_) => {}
                Err(e) => tracing::info!("update check failed: {e}"),
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
    let (info, fresh) = check_and_download(&app).await?;
    if let Some(version) = fresh {
        say_ready(&app, version);
    }
    Ok(info)
}

/// Install the downloaded update and restart. Called when the user clicks
/// "Restart to install"; on Windows the installer exits the app itself.
#[tauri::command]
pub async fn update_install_on_restart(app: AppHandle) -> AppResult<()> {
    // The waiting update may have been downloaded hours ago. Ask once more so a
    // newer release replaces it here, instead of installing the old one and
    // offering the new one straight after the restart. A failed check keeps
    // the one already downloaded.
    if let Err(e) = check_and_download(&app).await {
        tracing::info!("update check before install failed: {e}");
    }
    install_and_restart(&app).await
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

    #[test]
    fn a_failed_install_keeps_the_update_for_another_try() {
        let slot = Mutex::new(Some(("1.4.0", vec![1u8, 2, 3])));
        let failed = install_or_keep(&slot, |_| Err(AppError::Io("the installer couldn't start".into())));
        assert!(failed.is_err());
        assert_eq!(slot.lock().as_ref().map(|(v, b)| (*v, b.len())), Some(("1.4.0", 3)), "still there");
        let mut got = None;
        install_or_keep(&slot, |(v, _)| {
            got = Some(*v);
            Ok(())
        })
        .unwrap();
        assert_eq!(got, Some("1.4.0"));
        assert!(slot.lock().is_none(), "installed: taken");
        assert!(install_or_keep(&slot, |_: &(&str, Vec<u8>)| Ok(())).is_err(), "nothing to install");
    }

    #[test]
    fn just_after_launch_with_nothing_playing_it_installs_straight_away() {
        let soon = Duration::from_secs(30);
        assert_eq!(install_when(true, soon, false, false), Install::Now);
        assert_eq!(install_when(true, LAUNCH_WINDOW, false, false), Install::Now);
    }

    #[test]
    fn music_playing_a_later_find_or_not_now_waits_for_quit() {
        let soon = Duration::from_secs(30);
        assert_eq!(install_when(true, soon, true, false), Install::OnQuit, "never cuts a song off");
        assert_eq!(install_when(true, LAUNCH_WINDOW + Duration::from_secs(1), false, false), Install::OnQuit, "hours later");
        assert_eq!(install_when(true, soon, false, true), Install::OnQuit, "the user said Not now");
    }

    #[test]
    fn with_update_by_itself_off_it_only_asks() {
        assert_eq!(install_when(false, Duration::from_secs(30), false, false), Install::Ask);
        assert!(!install_on_exit(false, None), "quitting doesn't install either");
    }

    #[test]
    fn quitting_installs_but_a_restart_does_not() {
        assert!(install_on_exit(true, None), "last window closed");
        assert!(install_on_exit(true, Some(0)), "Quit");
        assert!(!install_on_exit(true, Some(tauri::RESTART_EXIT_CODE)), "switching user restarts: the app must come back");
    }

    #[test]
    fn a_downloaded_update_is_checked_again_against_the_newest_release() {
        assert_eq!(next(Some("1.4.0"), Some("1.4.0")), Next::Keep);
        assert_eq!(next(Some("1.4.0"), Some("1.5.0")), Next::Download, "a newer one replaces it");
        assert_eq!(next(Some("1.4.0"), None), Next::Forget, "the release was pulled");
        assert_eq!(next(None, Some("1.4.0")), Next::Download);
        assert_eq!(next(None, None), Next::Nothing);
    }
}
