//! The miniplayer: a small second window that shows what's playing and stays
//! above other windows (docs/IPC.md "Miniplayer").
//!
//! It loads the same UI as the main window, which sees its window label and
//! shows only the miniplayer. Player state reaches it through the same
//! events. Its size and position are kept per device in the settings
//! (`miniplayerBounds`), saved a moment after it stops moving and when it
//! closes. A saved place that is no longer on any screen is dropped and the
//! window opens in the bottom-right corner of the main screen.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder, Window};

use crate::error::{AppError, AppResult};
use crate::settings::{Settings, SettingsState};

/// The window's label (also in `capabilities/miniplayer.json`).
pub const LABEL: &str = "miniplayer";
/// Size when it first opens.
pub const DEFAULT_SIZE: (f64, f64) = (340.0, 365.0);
/// Smallest size the grip allows.
pub const MIN_SIZE: (f64, f64) = (240.0, 260.0);
/// Largest size the window can be resized to (and kept). It's a small
/// player; past this the main window does the job better.
pub const MAX_SIZE: (f64, f64) = (1000.0, 1000.0);
/// Height the queue needs. Turning the queue on in a shorter window grows it
/// to this (the UI hides the queue below it).
pub const QUEUE_HEIGHT: f64 = 500.0;
/// Gap from the screen edges when it opens in the corner.
const CORNER_GAP: f64 = 24.0;
/// How long the window has to sit still before its place is saved.
const SAVE_AFTER: Duration = Duration::from_millis(800);
/// How much of the top strip has to be on a screen to count as visible.
const VISIBLE_STRIP: (f64, f64) = (80.0, 24.0);

/// Where the window is, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A screen's usable area, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What the UI is told (`miniplayer_state`, `miniplayer://changed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniplayerState {
    pub open: bool,
}

/// Size kept inside the allowed range; None for values that aren't numbers.
pub fn clamp_size(width: f64, height: f64) -> Option<(f64, f64)> {
    if !width.is_finite() || !height.is_finite() {
        return None;
    }
    Some((width.clamp(MIN_SIZE.0, MAX_SIZE.0), height.clamp(MIN_SIZE.1, MAX_SIZE.1)))
}

/// Whether enough of the window's top strip is on one of the screens to
/// grab and move it.
pub fn on_a_screen(b: &WindowBounds, screens: &[ScreenRect]) -> bool {
    screens.iter().any(|s| {
        let left = b.x.max(s.x);
        let right = (b.x + b.width).min(s.x + s.width);
        let top = b.y.max(s.y);
        let bottom = (b.y + VISIBLE_STRIP.1).min(s.y + s.height);
        right - left >= VISIBLE_STRIP.0 && bottom - top >= VISIBLE_STRIP.1
    })
}

/// The bottom-right corner of a screen for a window of this size.
pub fn corner(screen: &ScreenRect, width: f64, height: f64) -> (f64, f64) {
    (
        (screen.x + screen.width - width - CORNER_GAP).max(screen.x),
        (screen.y + screen.height - height - CORNER_GAP).max(screen.y),
    )
}

/// Where to open: the saved place when it's sane and on a screen, else the
/// default size in the corner of `main` (the main screen). None places it
/// wherever the system likes.
pub fn place(saved: Option<WindowBounds>, screens: &[ScreenRect], main: Option<&ScreenRect>) -> (f64, f64, Option<(f64, f64)>) {
    if let Some(b) = saved
        && let Some((w, h)) = clamp_size(b.width, b.height)
        && b.x.is_finite()
        && b.y.is_finite()
    {
        let b = WindowBounds { width: w, height: h, ..b };
        if on_a_screen(&b, screens) {
            return (w, h, Some((b.x, b.y)));
        }
    }
    let (w, h) = DEFAULT_SIZE;
    (w, h, main.map(|m| corner(m, w, h)))
}

/// Where the window goes when the queue is turned on: as tall as the queue
/// needs, growing upwards when that would run past the bottom of its screen
/// (but never above the screen's top). None when it's tall enough already.
pub fn grown_for_queue(b: WindowBounds, screen: Option<&ScreenRect>) -> Option<WindowBounds> {
    if !b.height.is_finite() || b.height >= QUEUE_HEIGHT {
        return None;
    }
    let mut g = WindowBounds { height: QUEUE_HEIGHT, ..b };
    if let Some(s) = screen {
        // Running off the bottom: grow upwards instead, keeping the bottom
        // edge where it was (the corner it opens in stays its corner).
        if g.y + g.height > s.y + s.height {
            g.y = (b.y + b.height - g.height).min(s.y + s.height - g.height).max(s.y);
        }
    }
    Some(g)
}

/// The queue was turned on: grow the open window to fit it.
#[cfg(desktop)]
pub fn grow_for_queue<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = app.get_webview_window(LABEL) else { return };
    let Ok(scale) = w.scale_factor() else { return };
    let (Ok(pos), Ok(size)) = (w.outer_position(), w.inner_size()) else { return };
    let (pos, size) = (pos.to_logical::<f64>(scale), size.to_logical::<f64>(scale));
    let b = WindowBounds { x: pos.x, y: pos.y, width: size.width, height: size.height };
    let screen = w.current_monitor().ok().flatten().map(|m| rect_of(&m));
    let Some(g) = grown_for_queue(b, screen.as_ref()) else { return };
    if let Err(e) = w.set_size(tauri::LogicalSize::new(g.width, g.height)) {
        tracing::warn!("miniplayer grow: {e}");
        return;
    }
    if g.y != b.y {
        let _ = w.set_position(tauri::LogicalPosition::new(g.x, g.y));
    }
}

/// Live state: the last place seen, and a counter so only the last move saves.
#[derive(Default, Clone)]
pub struct Miniplayer {
    last: Arc<Mutex<Option<WindowBounds>>>,
    moves: Arc<AtomicU64>,
}

/// No miniplayer window on a phone.
#[cfg(mobile)]
pub fn grow_for_queue<R: Runtime>(_app: &AppHandle<R>) {}

/// A monitor's usable area in logical pixels.
#[cfg(desktop)]
fn rect_of(m: &tauri::Monitor) -> ScreenRect {
    let scale = m.scale_factor();
    let area = m.work_area();
    ScreenRect {
        x: area.position.x as f64 / scale,
        y: area.position.y as f64 / scale,
        width: area.size.width as f64 / scale,
        height: area.size.height as f64 / scale,
    }
}

#[cfg(desktop)]
fn screens<R: Runtime>(app: &AppHandle<R>) -> (Vec<ScreenRect>, Option<ScreenRect>) {
    let all = app.available_monitors().unwrap_or_default().iter().map(rect_of).collect();
    let main = app
        .get_webview_window("main")
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
        .map(|m| rect_of(&m));
    (all, main)
}

fn emit<R: Runtime>(app: &AppHandle<R>, open: bool) {
    if let Err(e) = app.emit("miniplayer://changed", MiniplayerState { open }) {
        tracing::warn!("emit miniplayer://changed: {e}");
    }
}

/// Whether the window is open now.
pub fn is_open<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_webview_window(LABEL).is_some()
}

/// Open the window (or bring it forward when it's open).
#[cfg(desktop)]
pub async fn open<R: Runtime>(app: &AppHandle<R>) -> AppResult<()> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let s = match app.try_state::<SettingsState>() {
        Some(st) => st.get().await.unwrap_or_default(),
        None => Settings::default(),
    };
    let (all, main) = screens(app);
    let (w, h, at) = place(s.miniplayer_bounds, &all, main.as_ref());
    let mut b = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Platter miniplayer")
        .inner_size(w, h)
        .min_inner_size(MIN_SIZE.0, MIN_SIZE.1)
        .max_inner_size(MAX_SIZE.0, MAX_SIZE.1)
        .decorations(false)
        .resizable(true)
        // No maximize: a double click on the top strip (the drag region)
        // would otherwise fill the screen.
        .maximizable(false)
        // The queue's rows drag to reorder (the page's own drag and drop).
        .disable_drag_drop_handler()
        .always_on_top(s.miniplayer_on_top)
        .background_color(super::reveal::background(app))
        .visible(true)
        .focused(true);
    if let Some((x, y)) = at {
        b = b.position(x, y);
    }
    b.build().map_err(|e| {
        tracing::warn!("miniplayer window: {e}");
        AppError::Internal("The miniplayer couldn't open.".into())
    })?;
    emit(app, true);
    Ok(())
}

/// A phone has one window: the miniplayer is the phone's own mini player.
#[cfg(mobile)]
pub async fn open<R: Runtime>(_app: &AppHandle<R>) -> AppResult<()> {
    Err(AppError::Invalid("There's no miniplayer window on a phone.".into()))
}

/// Close the window if it's open.
pub fn close<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(desktop)]
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.destroy();
    }
    #[cfg(mobile)]
    let _ = app;
}

/// A miniplayer setting changed: both windows get the settings, and the
/// window goes on top or not.
pub fn settings_changed<R: Runtime>(app: &AppHandle<R>, s: &Settings) {
    #[cfg(desktop)]
    if let Some(w) = app.get_webview_window(LABEL)
        && let Err(e) = w.set_always_on_top(s.miniplayer_on_top)
    {
        tracing::warn!("miniplayer on top: {e}");
    }
    let _ = app.emit("settings://changed", s);
}

#[cfg(desktop)]
fn bounds_of<R: Runtime>(w: &Window<R>) -> Option<WindowBounds> {
    let scale = w.scale_factor().ok()?;
    let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
    let size = w.inner_size().ok()?.to_logical::<f64>(scale);
    Some(WindowBounds { x: pos.x, y: pos.y, width: size.width, height: size.height })
}

#[cfg(desktop)]
async fn save<R: Runtime>(app: &AppHandle<R>, b: WindowBounds) {
    let Some(st) = app.try_state::<SettingsState>() else { return };
    let res = st
        .modify(move |s| {
            if s.miniplayer_bounds == Some(b) {
                return Ok(false);
            }
            s.miniplayer_bounds = Some(b);
            Ok(true)
        })
        .await;
    if let Err(e) = res {
        tracing::warn!("miniplayer place not saved: {e}");
    }
}

/// Window events for the miniplayer (and the main window closing).
#[cfg(desktop)]
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &tauri::WindowEvent) {
    let app = window.app_handle().clone();
    if window.label() == "main" {
        // The miniplayer doesn't outlive the main window.
        if let tauri::WindowEvent::Destroyed = event {
            close(&app);
        }
        return;
    }
    if window.label() != LABEL {
        return;
    }
    let Some(mp) = app.try_state::<Miniplayer>().map(|m| m.inner().clone()) else { return };
    match event {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
            let Some(b) = bounds_of(window) else { return };
            // Minimised windows report a size of 0; keep the last real place.
            if b.width < 1.0 || b.height < 1.0 {
                return;
            }
            *mp.last.lock() = Some(b);
            let n = mp.moves.fetch_add(1, Ordering::AcqRel) + 1;
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(SAVE_AFTER).await;
                if mp.moves.load(Ordering::Acquire) == n {
                    save(&app, b).await;
                }
            });
        }
        tauri::WindowEvent::Destroyed => {
            emit(&app, false);
            let last = *mp.last.lock();
            if let Some(b) = last {
                tauri::async_runtime::spawn(async move { save(&app, b).await });
            }
        }
        _ => {}
    }
}

/// A phone has no miniplayer window to follow.
#[cfg(mobile)]
pub fn on_window_event<R: Runtime>(_window: &Window<R>, _event: &tauri::WindowEvent) {}

#[tauri::command]
pub async fn miniplayer_state(app: AppHandle) -> AppResult<MiniplayerState> {
    Ok(MiniplayerState { open: is_open(&app) })
}

/// Open or close the miniplayer. Answers whether it's open now.
#[tauri::command]
pub async fn miniplayer_set(app: AppHandle, open: bool) -> AppResult<MiniplayerState> {
    if open {
        self::open(&app).await?;
    } else {
        close(&app);
    }
    Ok(MiniplayerState { open: is_open(&app) })
}

/// A page the miniplayer can open in the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoKind {
    Album,
    Artist,
}

/// What the main window is told to open (`miniplayer://go`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniplayerGo {
    pub kind: GoKind,
    pub id: String,
}

/// Open a page in the main window (from the miniplayer's queue menu): bring
/// the main window forward and tell it which page.
#[tauri::command]
pub async fn miniplayer_go(app: AppHandle, kind: GoKind, id: String) -> AppResult<()> {
    if id.is_empty() {
        return Err(AppError::Invalid("Nothing to open.".into()));
    }
    crate::share::commands::focus_main(&app);
    if let Err(e) = app.emit_to("main", "miniplayer://go", MiniplayerGo { kind, id }) {
        tracing::warn!("emit miniplayer://go: {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: ScreenRect = ScreenRect { x: 0.0, y: 0.0, width: 1920.0, height: 1040.0 };
    const SECOND: ScreenRect = ScreenRect { x: 1920.0, y: 0.0, width: 1280.0, height: 1024.0 };

    fn at(x: f64, y: f64) -> WindowBounds {
        WindowBounds { x, y, width: 340.0, height: 365.0 }
    }

    #[test]
    fn first_open_goes_in_the_corner_at_the_default_size() {
        let (w, h, pos) = place(None, &[SCREEN], Some(&SCREEN));
        assert_eq!((w, h), DEFAULT_SIZE);
        assert_eq!(pos, Some((1920.0 - 340.0 - 24.0, 1040.0 - 365.0 - 24.0)));
    }

    #[test]
    fn a_saved_place_on_a_screen_is_kept() {
        let b = WindowBounds { x: 2000.0, y: 100.0, width: 500.0, height: 600.0 };
        assert_eq!(place(Some(b), &[SCREEN, SECOND], Some(&SCREEN)), (500.0, 600.0, Some((2000.0, 100.0))));
    }

    #[test]
    fn a_place_on_a_screen_thats_gone_falls_back_to_the_corner() {
        let b = at(2000.0, 100.0);
        let (_, _, pos) = place(Some(b), &[SCREEN], Some(&SCREEN));
        assert_eq!(pos, Some(corner(&SCREEN, 340.0, 365.0)));
    }

    #[test]
    fn the_top_strip_has_to_be_reachable() {
        // Only 40 px of the strip on screen: too little to grab.
        assert!(!on_a_screen(&at(1880.0, 10.0), &[SCREEN]));
        // Above the top edge: the strip is off screen.
        assert!(!on_a_screen(&at(100.0, -100.0), &[SCREEN]));
        assert!(on_a_screen(&at(1500.0, 10.0), &[SCREEN]));
    }

    #[test]
    fn sizes_are_kept_in_range() {
        assert_eq!(clamp_size(10.0, 99_999.0), Some((MIN_SIZE.0, MAX_SIZE.1)));
        assert_eq!(clamp_size(f64::NAN, 300.0), None);
        let b = WindowBounds { x: 10.0, y: 10.0, width: 50.0, height: 50.0 };
        let (w, h, _) = place(Some(b), &[SCREEN], Some(&SCREEN));
        assert_eq!((w, h), MIN_SIZE);
    }

    #[test]
    fn a_saved_size_past_the_maximum_comes_back_clamped() {
        let b = WindowBounds { x: 10.0, y: 10.0, width: 2000.0, height: 1200.0 };
        let (w, h, _) = place(Some(b), &[SCREEN], Some(&SCREEN));
        assert_eq!((w, h), MAX_SIZE);
        assert!(MAX_SIZE.0 >= DEFAULT_SIZE.0 && MAX_SIZE.1 >= DEFAULT_SIZE.1);
    }

    #[test]
    fn no_screens_known_lets_the_system_place_it() {
        assert_eq!(place(None, &[], None), (DEFAULT_SIZE.0, DEFAULT_SIZE.1, None));
    }

    #[test]
    fn turning_the_queue_on_grows_a_short_window() {
        let screen = ScreenRect { x: 0.0, y: 0.0, width: 1920.0, height: 1040.0 };
        let b = WindowBounds { x: 100.0, y: 100.0, width: 340.0, height: 365.0 };
        let g = grown_for_queue(b, Some(&screen)).unwrap();
        assert_eq!((g.x, g.y, g.width, g.height), (100.0, 100.0, 340.0, QUEUE_HEIGHT));
        // Tall enough already: left alone.
        assert_eq!(grown_for_queue(WindowBounds { height: 600.0, ..b }, Some(&screen)), None);
    }

    #[test]
    fn a_window_in_the_bottom_corner_grows_upwards() {
        let screen = ScreenRect { x: 0.0, y: 0.0, width: 1920.0, height: 1040.0 };
        let (x, y) = corner(&screen, 340.0, 365.0);
        let g = grown_for_queue(WindowBounds { x, y, width: 340.0, height: 365.0 }, Some(&screen)).unwrap();
        assert_eq!(g.y + g.height, y + 365.0);
        // A screen shorter than the queue keeps the top strip on it.
        let small = ScreenRect { height: 400.0, ..screen };
        let g = grown_for_queue(WindowBounds { x: 0.0, y: 50.0, width: 340.0, height: 300.0 }, Some(&small)).unwrap();
        assert_eq!(g.y, 0.0);
    }

    #[test]
    fn go_reads_page_kinds_in_camel_case() {
        let k: GoKind = serde_json::from_str("\"album\"").unwrap();
        assert_eq!(k, GoKind::Album);
        let v = serde_json::to_value(MiniplayerGo { kind: GoKind::Artist, id: "7".into() }).unwrap();
        assert_eq!(v, serde_json::json!({ "kind": "artist", "id": "7" }));
    }

    #[test]
    fn bounds_go_over_the_wire_in_camel_case() {
        let v = serde_json::to_value(at(1.0, 2.0)).unwrap();
        assert_eq!(v["width"], 340.0);
        assert_eq!(serde_json::to_value(MiniplayerState { open: true }).unwrap()["open"], true);
    }
}
