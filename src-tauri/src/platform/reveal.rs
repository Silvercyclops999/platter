//! Opening the main window without a flash.
//!
//! The window is made hidden (`visible: false` in tauri.conf.json) and shown
//! once the page has drawn its first real frame: the UI calls `window_ready`
//! after the shell and the look are on screen. If that never comes, the
//! window is shown anyway after [`SHOW_ANYWAY_AFTER`]. Launched at login it
//! stays in the tray, or opens minimised, as before.
//!
//! The window opens where it was last (`window.json` in the app data folder,
//! shared by every Home user on this computer): same place, size, screen
//! and maximised state, and with the look's background colour behind the
//! page. Switching Home users restarts the app; the restart is noted here
//! so the new window opens normally even when the app was launched at login.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::window::Color;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, Window, WindowEvent};

use super::miniplayer::{ScreenRect, WindowBounds, on_a_screen};
use super::{StartMode, visibility};

/// Where the window's place and colour are kept.
pub const PLACE_FILE: &str = "window.json";
/// The window is shown after this even if the page never says it's ready.
pub const SHOW_ANYWAY_AFTER: Duration = Duration::from_secs(3);
/// A switch older than this (a restart that never happened) is ignored.
const SWITCH_FRESH_SECS: u64 = 120;
/// The window's colour before any look is known (the dark look).
pub const DEFAULT_BACKGROUND: Color = Color(0x0E, 0x0D, 0x0C, 0xFF);
/// Smallest saved size taken back (smaller is a minimised or broken one).
const MIN_SAVED: u32 = 200;

/// The main window's place on screen, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// What `window.json` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Saved {
    pub place: Option<Place>,
    /// The look's background as `#rrggbb`.
    pub background: Option<String>,
    /// Set just before a Home user switch restarts the app (unix seconds).
    pub switching_at: Option<u64>,
}

impl Saved {
    pub fn load(root: &Path) -> Self {
        std::fs::read(root.join(PLACE_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let tmp = root.join(format!("{PLACE_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(&tmp, root.join(PLACE_FILE))
    }

    /// Whether this launch is the restart after a Home user switch.
    pub fn switching(&self, now: u64) -> bool {
        self.switching_at.is_some_and(|at| at <= now && now - at <= SWITCH_FRESH_SECS)
    }
}

/// What happens when the window is first shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstShow {
    Show,
    /// Shown in the taskbar, minimised.
    Minimised,
    /// Stays in the tray until the user opens it.
    StayHidden,
}

/// The first show for how the app was started. The restart after a Home
/// user switch always opens the window: the user was looking at it.
pub fn first_show(mode: StartMode, switching: bool) -> FirstShow {
    match mode {
        _ if switching => FirstShow::Show,
        StartMode::Normal => FirstShow::Show,
        StartMode::Minimised => FirstShow::Minimised,
        StartMode::Hidden => FirstShow::StayHidden,
    }
}

/// `#rrggbb` (or `#rgb`) as a window colour.
pub fn parse_color(s: &str) -> Option<Color> {
    let hex = s.trim().strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = |i: usize, n: usize| u8::from_str_radix(&hex[i..i + n], 16).ok();
    match hex.len() {
        6 => Some(Color(v(0, 2)?, v(2, 2)?, v(4, 2)?, 0xFF)),
        3 => {
            let d = |i| v(i, 1).map(|x| x * 17);
            Some(Color(d(0)?, d(1)?, d(2)?, 0xFF))
        }
        _ => None,
    }
}

/// The saved place when enough of it is on one of the screens (work areas,
/// physical pixels) to grab; None opens the window centred at its default
/// size.
pub fn usable_place(saved: Option<Place>, screens: &[ScreenRect]) -> Option<Place> {
    let p = saved?;
    if p.width < MIN_SAVED || p.height < MIN_SAVED {
        return None;
    }
    let b = WindowBounds { x: p.x as f64, y: p.y as f64, width: p.width as f64, height: p.height as f64 };
    // With no screens known there's nothing to check against.
    (screens.is_empty() || on_a_screen(&b, screens)).then_some(p)
}

/// The place to remember after the window moved or resized: a maximised
/// window keeps the place it goes back to; minimised or hidden changes
/// nothing.
pub fn next_place(last: Option<Place>, now: Option<Place>, minimized: bool, visible: bool, maximized: bool) -> Option<Place> {
    if minimized || !visible {
        return last;
    }
    if maximized {
        return last.map(|p| Place { maximized: true, ..p }).or(now.map(|p| Place { maximized: true, ..p }));
    }
    match now {
        Some(p) if p.width >= MIN_SAVED && p.height >= MIN_SAVED => Some(Place { maximized: false, ..p }),
        _ => last,
    }
}

struct Inner {
    root: PathBuf,
    saved: Saved,
    plan: FirstShow,
    /// The first show has happened (or was skipped for the tray).
    done: bool,
}

/// Live state, managed by the app.
pub struct Reveal(Mutex<Inner>);

impl Reveal {
    pub fn new(root: PathBuf, saved: Saved, plan: FirstShow) -> Self {
        Self(Mutex::new(Inner { root, saved, plan, done: false }))
    }

    /// The first show's plan, once: the ready signal and the timeout both
    /// ask, only the first gets it.
    pub fn take(&self) -> Option<FirstShow> {
        let mut g = self.0.lock();
        if g.done {
            return None;
        }
        g.done = true;
        Some(g.plan)
    }

    /// Remember the look's background; true when it changed.
    pub fn set_background(&self, hex: &str) -> bool {
        let mut g = self.0.lock();
        let hex = hex.trim().to_ascii_lowercase();
        if g.saved.background.as_deref() == Some(hex.as_str()) {
            return false;
        }
        g.saved.background = Some(hex);
        true
    }

    pub fn background(&self) -> Color {
        self.0.lock().saved.background.as_deref().and_then(parse_color).unwrap_or(DEFAULT_BACKGROUND)
    }

    fn update_place(&self, f: impl FnOnce(Option<Place>) -> Option<Place>) {
        let mut g = self.0.lock();
        g.saved.place = f(g.saved.place);
    }

    /// Note that the next launch is a switch's restart.
    pub fn mark_switching(&self, now: u64) {
        self.0.lock().saved.switching_at = Some(now);
    }

    /// Write `window.json`.
    pub fn save(&self) {
        let (root, saved) = {
            let g = self.0.lock();
            (g.root.clone(), g.saved.clone())
        };
        if let Err(e) = saved.save(&root) {
            tracing::warn!("window place not saved: {e}");
        }
    }
}

fn now_s() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(desktop)]
fn screens<R: Runtime>(app: &AppHandle<R>) -> Vec<ScreenRect> {
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let a = m.work_area();
            ScreenRect {
                x: a.position.x as f64,
                y: a.position.y as f64,
                width: a.size.width as f64,
                height: a.size.height as f64,
            }
        })
        .collect()
}

/// Called from `setup` while the window is still hidden: put it back where
/// it was, colour it, and plan the first show. The window is shown when the
/// page says it's ready, or after [`SHOW_ANYWAY_AFTER`].
pub fn install<R: Runtime>(app: &AppHandle<R>, root: &Path, mode: StartMode) {
    let mut saved = Saved::load(root);
    let switching = saved.switching(now_s());
    saved.switching_at = None;
    let plan = first_show(mode, switching);
    // A phone's window has no place, size or minimised state to restore.
    #[cfg(desktop)]
    if let Some(w) = app.get_webview_window("main") {
        let color = saved.background.as_deref().and_then(parse_color).unwrap_or(DEFAULT_BACKGROUND);
        let _ = w.set_background_color(Some(color));
        if let Some(p) = usable_place(saved.place, &screens(app)) {
            let _ = w.set_position(PhysicalPosition::new(p.x, p.y));
            let _ = w.set_size(PhysicalSize::new(p.width, p.height));
            // On a hidden window this only takes effect when it's shown.
            if p.maximized {
                let _ = w.maximize();
            }
        }
        match plan {
            FirstShow::StayHidden => visibility::set_page_visible(&w, false),
            FirstShow::Minimised => {
                let _ = w.minimize();
            }
            FirstShow::Show => {}
        }
    }
    let reveal = Reveal::new(root.to_path_buf(), saved, plan);
    if switching {
        // The switch is used up.
        reveal.save();
    }
    app.manage(reveal);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SHOW_ANYWAY_AFTER).await;
        if show_first(&app) {
            tracing::info!("main window shown before the page said it was ready");
        }
    });
}

/// Show the window for the first time, as planned. True when this call did
/// it (later calls do nothing).
pub fn show_first<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(plan) = app.try_state::<Reveal>().and_then(|r| r.take()) else { return false };
    let Some(w) = app.get_webview_window("main") else { return false };
    #[cfg(mobile)]
    let _ = (w, plan);
    #[cfg(desktop)]
    match plan {
        FirstShow::Show => {
            let _ = w.show();
            let _ = w.set_focus();
        }
        // Already minimised while hidden, so it appears in the taskbar only.
        FirstShow::Minimised => {
            let _ = w.show();
        }
        FirstShow::StayHidden => {}
    }
    true
}

/// The page in the main window has drawn its first frame with the look on.
/// `background` is the look's background colour (`#rrggbb`).
#[tauri::command]
pub async fn window_ready(app: AppHandle, window: Window, background: Option<String>) -> crate::error::AppResult<()> {
    if window.label() != "main" {
        return Ok(());
    }
    if let (Some(bg), Some(r)) = (background.as_deref(), app.try_state::<Reveal>())
        && let Some(c) = parse_color(bg)
        && r.set_background(bg)
    {
        #[cfg(desktop)]
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.set_background_color(Some(c));
        }
        #[cfg(mobile)]
        let _ = c;
        r.save();
    }
    show_first(&app);
    Ok(())
}

/// The look's background colour, for other windows (the miniplayer).
pub fn background<R: Runtime>(app: &AppHandle<R>) -> Color {
    app.try_state::<Reveal>().map(|r| r.background()).unwrap_or(DEFAULT_BACKGROUND)
}

/// Follow the main window's place; written when it closes or the app quits.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }
    let Some(r) = window.try_state::<Reveal>() else { return };
    match event {
        #[cfg(desktop)]
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            let now = match (window.outer_position(), window.inner_size()) {
                (Ok(p), Ok(s)) => Some(Place { x: p.x, y: p.y, width: s.width, height: s.height, maximized: false }),
                _ => None,
            };
            let minimized = window.is_minimized().unwrap_or(false);
            let visible = window.is_visible().unwrap_or(true);
            let maximized = window.is_maximized().unwrap_or(false);
            r.update_place(|last| next_place(last, now, minimized, visible, maximized));
        }
        WindowEvent::CloseRequested { .. } => r.save(),
        _ => {}
    }
}

/// Write the place now (the app is quitting).
pub fn save<R: Runtime>(app: &AppHandle<R>) {
    if let Some(r) = app.try_state::<Reveal>() {
        r.save();
    }
}

/// A Home user switch is about to restart the app: the new window opens
/// where this one is, shown, even if this run started in the tray.
pub fn mark_switching<R: Runtime>(app: &AppHandle<R>) {
    if let Some(r) = app.try_state::<Reveal>() {
        r.mark_switching(now_s());
        r.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(x: i32, y: i32) -> Place {
        Place { x, y, width: 1280, height: 820, maximized: false }
    }

    fn screen() -> ScreenRect {
        ScreenRect { x: 0.0, y: 0.0, width: 1920.0, height: 1040.0 }
    }

    #[test]
    fn every_launch_shows_the_window_except_into_the_tray() {
        assert_eq!(first_show(StartMode::Normal, false), FirstShow::Show);
        assert_eq!(first_show(StartMode::Minimised, false), FirstShow::Minimised);
        assert_eq!(first_show(StartMode::Hidden, false), FirstShow::StayHidden);
    }

    #[test]
    fn the_restart_after_a_switch_always_shows_the_window() {
        assert_eq!(first_show(StartMode::Hidden, true), FirstShow::Show);
        assert_eq!(first_show(StartMode::Minimised, true), FirstShow::Show);
    }

    #[test]
    fn the_ready_signal_and_the_timeout_show_it_only_once() {
        let r = Reveal::new(PathBuf::new(), Saved::default(), FirstShow::Show);
        assert_eq!(r.take(), Some(FirstShow::Show), "whichever comes first");
        assert_eq!(r.take(), None, "the other does nothing");
        let tray = Reveal::new(PathBuf::new(), Saved::default(), FirstShow::StayHidden);
        assert_eq!(tray.take(), Some(FirstShow::StayHidden));
        assert_eq!(tray.take(), None);
    }

    #[test]
    fn a_switch_counts_only_for_the_restart_right_after_it() {
        let s = Saved { switching_at: Some(1_000), ..Default::default() };
        assert!(s.switching(1_005));
        assert!(!s.switching(1_000 + SWITCH_FRESH_SECS + 1), "stale");
        assert!(!s.switching(999), "clock went back");
        assert!(!Saved::default().switching(1_000));
    }

    #[test]
    fn colours_come_from_the_look() {
        assert_eq!(parse_color("#F4F1EA"), Some(Color(0xF4, 0xF1, 0xEA, 0xFF)));
        assert_eq!(parse_color("#000"), Some(Color(0, 0, 0, 0xFF)));
        assert_eq!(parse_color("rgb(1, 2, 3)"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#gg0000"), None);
        let r = Reveal::new(PathBuf::new(), Saved::default(), FirstShow::Show);
        assert_eq!(r.background(), DEFAULT_BACKGROUND, "dark until a look is known");
        assert!(r.set_background("#000000"));
        assert!(!r.set_background("#000000"), "unchanged");
        assert_eq!(r.background(), Color(0, 0, 0, 0xFF));
    }

    #[test]
    fn the_window_opens_where_it_was_when_thats_on_a_screen() {
        let screens = [screen()];
        assert_eq!(usable_place(Some(place(100, 50)), &screens), Some(place(100, 50)));
        // A second screen that's gone: centred instead.
        assert_eq!(usable_place(Some(place(3000, 50)), &screens), None);
        assert_eq!(usable_place(None, &screens), None);
        let tiny = Place { width: 10, height: 10, ..place(0, 0) };
        assert_eq!(usable_place(Some(tiny), &screens), None);
        // A second screen to the left (negative coordinates) counts.
        let left = ScreenRect { x: -1920.0, ..screen() };
        assert_eq!(usable_place(Some(place(-1500, 0)), &[screen(), left]), Some(place(-1500, 0)));
    }

    #[test]
    fn maximised_keeps_the_place_it_goes_back_to() {
        let normal = Some(place(100, 50));
        let max_now = Some(Place { x: -8, y: -8, width: 1936, height: 1056, maximized: false });
        let m = next_place(normal, max_now, false, true, true).unwrap();
        assert_eq!(m, Place { maximized: true, ..place(100, 50) });
        // Back to normal: the new place, not maximised.
        assert_eq!(next_place(Some(m), Some(place(200, 60)), false, true, false), Some(place(200, 60)));
        // Minimised or in the tray: nothing changes.
        assert_eq!(next_place(normal, Some(place(-32000, -32000)), true, true, false), normal);
        assert_eq!(next_place(normal, Some(place(5, 5)), false, false, false), normal);
        // First seen maximised: that's all there is to keep.
        assert!(next_place(None, max_now, false, true, true).unwrap().maximized);
    }

    #[test]
    fn the_place_file_round_trips_and_a_broken_one_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Saved::load(dir.path()), Saved::default());
        let s = Saved { place: Some(place(1, 2)), background: Some("#ffffff".into()), switching_at: Some(5) };
        s.save(dir.path()).unwrap();
        assert_eq!(Saved::load(dir.path()), s);
        std::fs::write(dir.path().join(PLACE_FILE), b"{nope").unwrap();
        assert_eq!(Saved::load(dir.path()), Saved::default());
    }
}
