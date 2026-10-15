//! Windows/macOS glue: the system media controls, media keys, the tray or
//! menu bar item, close to tray, launch at login and Discord status.
//!
//! Player state reaches this module through [`Tap`], an event sink that
//! sits in front of the UI's sink. OS commands go back to the player
//! through its public handle. The UI is not involved beyond Settings.

pub mod art;
pub mod commands;
pub mod discord;
pub mod keys;
pub mod logs;
pub mod media;
pub mod menubar;
pub mod miniplayer;
pub mod power;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub mod session;
#[cfg(target_os = "windows")]
pub mod thumbbar;
pub mod tray;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::watch;

use crate::audio::events::{ErrorPayload, EventSink, OutputLost, PlayerState, PositionPayload};
use crate::audio::output::OutputState;
use crate::audio::player::PlayerHandle;
use crate::db::ArtCache;
use media::{Controller, OsCommand};

/// The App settings this module acts on. All are per device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformSettings {
    pub media_keys: bool,
    pub media_session: bool,
    pub close_to_tray: bool,
    pub launch_at_login: bool,
    /// Go offline (the menu bar's check follows it).
    pub go_offline: bool,
}

impl Default for PlatformSettings {
    fn default() -> Self {
        Self { media_keys: true, media_session: true, close_to_tray: false, launch_at_login: false, go_offline: false }
    }
}

/// Shared switch the settings code writes and this module watches.
#[derive(Clone)]
pub struct Switches(Arc<watch::Sender<PlatformSettings>>);

impl Default for Switches {
    fn default() -> Self {
        Self(Arc::new(watch::channel(PlatformSettings::default()).0))
    }
}

impl Switches {
    pub fn set(&self, s: PlatformSettings) {
        self.0.send_if_modified(|cur| std::mem::replace(cur, s) != s);
    }
    pub fn get(&self) -> PlatformSettings {
        *self.0.borrow()
    }
    pub fn subscribe(&self) -> watch::Receiver<PlatformSettings> {
        self.0.subscribe()
    }
}

/// What the window does when the user closes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Hide to the tray / menu bar; playback carries on.
    Hide,
    Quit,
}

pub fn close_action(s: &PlatformSettings) -> CloseAction {
    if s.close_to_tray { CloseAction::Hide } else { CloseAction::Quit }
}

/// Passed by launch at login so the app can start out of the way.
pub const AUTOSTART_ARG: &str = "--autostart";

/// How the window starts when launched at login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    Normal,
    Hidden,
    Minimised,
}

pub fn start_mode(args: &[String], s: &PlatformSettings) -> StartMode {
    if !args.iter().any(|a| a == AUTOSTART_ARG) {
        StartMode::Normal
    } else if s.close_to_tray {
        StartMode::Hidden
    } else {
        StartMode::Minimised
    }
}

enum Msg {
    State(Box<PlayerState>),
    Position(PositionPayload),
    Settings(PlatformSettings),
    Cover(String, PathBuf),
}

/// An [`EventSink`] that passes everything to the UI's sink and copies
/// playback state to the platform worker.
pub struct Tap {
    inner: Arc<dyn EventSink>,
    tx: mpsc::Sender<Msg>,
    tray: watch::Sender<tray::TrayView>,
    menu: watch::Sender<menubar::MenuView>,
}

impl EventSink for Tap {
    fn state(&self, state: &PlayerState) {
        self.inner.state(state);
        let _ = self.tx.send(Msg::State(Box::new(state.clone())));
        self.tray.send_if_modified(|v| {
            let next = tray::tray_view(state);
            std::mem::replace(v, next.clone()) != next
        });
        self.menu.send_if_modified(|v| {
            let next = menubar::menu_view(state);
            std::mem::replace(v, next) != next
        });
    }
    fn position(&self, pos: &PositionPayload) {
        self.inner.position(pos);
        let _ = self.tx.send(Msg::Position(*pos));
    }
    fn error(&self, err: &ErrorPayload) {
        self.inner.error(err);
    }
    fn output(&self, state: &OutputState) {
        self.inner.output(state);
    }
    fn output_lost(&self, lost: &OutputLost) {
        self.inner.output_lost(lost);
    }
}

/// Built before the player (it needs the sink), started after it.
pub struct Platform {
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    tray: watch::Sender<tray::TrayView>,
    menu: watch::Sender<menubar::MenuView>,
}

impl Platform {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            tray: watch::channel(tray::TrayView::default()).0,
            menu: watch::channel(menubar::MenuView::default()).0,
        }
    }

    /// The sink to give the player, wrapping the UI's sink.
    pub fn tap(&self, inner: Arc<dyn EventSink>) -> Arc<dyn EventSink> {
        Arc::new(Tap { inner, tx: self.tx.clone(), tray: self.tray.clone(), menu: self.menu.clone() })
    }

    /// Start the worker, tray, keys and launch at login.
    pub fn start<R: Runtime>(self, app: &AppHandle<R>, player: PlayerHandle, art: ArtCache, cover_dir: PathBuf, switches: Switches) {
        let Self { tx, rx, tray: tray_tx, menu: menu_tx } = self;
        app.manage(tray::Tray::<R>::default());
        app.manage(menubar::MenuBar::<R>::default());
        menubar::install(app);

        // OS commands: player commands run on the async runtime; window
        // commands on the app.
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<OsCommand>();
        let on_command = move |c: OsCommand| {
            let _ = cmd_tx.send(c);
        };
        {
            let app = app.clone();
            let player = player.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(c) = cmd_rx.recv().await {
                    match c {
                        OsCommand::Show => show_window(&app),
                        OsCommand::Quit => app.exit(0),
                        c => {
                            if let Err(e) = commands::run(&player, c).await {
                                tracing::debug!("OS command {c:?} failed: {e}");
                            }
                        }
                    }
                }
            });
        }

        // The session worker.
        let controller = Controller::new(session_factory(app), Arc::new(on_command.clone()));
        let worker_tx = tx.clone();
        let rt = tauri::async_runtime::handle();
        let keys_app = app.clone();
        let keys_cmd = on_command.clone();
        let set_keys = move |on: bool| keys::apply(&keys_app, on, keys_cmd.clone());
        let _ = std::thread::Builder::new()
            .name("platter-media".into())
            .spawn(move || run_worker(controller, rx, worker_tx, art, cover_dir, rt, set_keys));

        // Seed the worker with the restored queue (the player doesn't emit
        // until something changes).
        {
            let tx = tx.clone();
            let player = player.clone();
            let tray_tx = tray_tx.clone();
            let menu_tx = menu_tx.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(s) = player.state().await {
                    tray_tx.send_replace(tray::tray_view(&s));
                    menu_tx.send_replace(menubar::menu_view(&s));
                    let _ = tx.send(Msg::State(Box::new(s)));
                }
            });
        }

        // Tray menu follows playback.
        {
            let app = app.clone();
            let mut rx = tray_tx.subscribe();
            tauri::async_runtime::spawn(async move {
                loop {
                    let v = rx.borrow_and_update().clone();
                    app.state::<tray::Tray<R>>().update(v);
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            });
        }

        // The menu bar's checkmarks follow playback (macOS).
        {
            let app = app.clone();
            let mut rx = menu_tx.subscribe();
            tauri::async_runtime::spawn(async move {
                loop {
                    let v = *rx.borrow_and_update();
                    app.state::<menubar::MenuBar<R>>().update(v);
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            });
        }

        // Settings.
        #[cfg(target_os = "windows")]
        thumbbar::install(app, on_command.clone(), tray_tx.subscribe());
        let app = app.clone();
        let mut rx = switches.subscribe();
        tauri::async_runtime::spawn(async move {
            let mut first = true;
            loop {
                let s = *rx.borrow_and_update();
                let _ = tx.send(Msg::Settings(s));
                if let Err(e) = app.state::<tray::Tray<R>>().set_shown(&app, s.close_to_tray, on_command.clone()) {
                    tracing::warn!("tray icon: {e}");
                }
                if !s.close_to_tray && !first {
                    // Without a tray a hidden window would be unreachable.
                    if let Some(w) = app.get_webview_window("main")
                        && !w.is_visible().unwrap_or(true)
                    {
                        show_window(&app);
                    }
                }
                sync_autostart(&app, s.launch_at_login);
                app.state::<menubar::MenuBar<R>>().set_offline(s.go_offline);
                first = false;
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });
    }
}

impl Default for Platform {
    fn default() -> Self {
        Self::new()
    }
}

fn run_worker(
    mut c: Controller,
    rx: mpsc::Receiver<Msg>,
    tx: mpsc::Sender<Msg>,
    art: ArtCache,
    cover_dir: PathBuf,
    rt: tauri::async_runtime::RuntimeHandle,
    set_keys: impl Fn(bool),
) {
    while let Ok(msg) = rx.recv() {
        let at = Instant::now();
        match msg {
            Msg::State(s) => {
                let before = c.track_id().map(str::to_owned);
                c.on_state(&s, at);
                // New track: fetch its cover from the art cache.
                if c.track_id() != before.as_deref()
                    && let (Some(id), Some(thumb)) = (c.track_id(), c.thumb())
                {
                    let (id, thumb) = (id.to_owned(), thumb.to_owned());
                    let (art, dir, tx) = (art.clone(), cover_dir.clone(), tx.clone());
                    rt.spawn(async move {
                        match art::cover_file(&art, &dir, &thumb).await {
                            Ok(p) => {
                                let _ = tx.send(Msg::Cover(id, p));
                            }
                            Err(e) => tracing::debug!("no cover for the system controls: {e}"),
                        }
                    });
                }
            }
            Msg::Position(p) => c.on_position(&p, at),
            Msg::Settings(s) => {
                c.set_enabled(s.media_session);
                set_keys(keys::wants_shortcuts(&s, c.is_attached()));
            }
            Msg::Cover(id, path) => c.on_cover(&id, path),
        }
    }
}

/// Creates the real session on the main thread (where the window lives) and
/// hands it to the worker.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn session_factory<R: Runtime>(app: &AppHandle<R>) -> Box<dyn media::SessionFactory> {
    let app = app.clone();
    Box::new(move |cb: media::CommandFn| -> Result<Box<dyn media::MediaSession>, String> {
        #[cfg(target_os = "windows")]
        let hwnd = app
            .get_webview_window("main")
            .and_then(|w| w.hwnd().ok())
            .map(|h| h.0 as isize);
        #[cfg(not(target_os = "windows"))]
        let hwnd = None;
        let (tx, rx) = mpsc::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(session::OsSession::create(hwnd, cb));
        })
        .map_err(|e| e.to_string())?;
        let s = rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|e| e.to_string())??;
        Ok(Box::new(s))
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn session_factory<R: Runtime>(_app: &AppHandle<R>) -> Box<dyn media::SessionFactory> {
    Box::new(|_cb: media::CommandFn| -> Result<Box<dyn media::MediaSession>, String> {
        Err("no system media controls on this platform".into())
    })
}

/// Bring the main window back from the tray or the taskbar.
pub fn show_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Make the OS's launch-at-login entry match the setting. Only touches it
/// when they differ.
fn sync_autostart<R: Runtime>(app: &AppHandle<R>, on: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    match al.is_enabled() {
        Ok(cur) if cur == on => {}
        Ok(_) => {
            let r = if on { al.enable() } else { al.disable() };
            if let Err(e) = r {
                tracing::warn!("launch at login: {e}");
            }
        }
        Err(e) => tracing::warn!("launch at login state: {e}"),
    }
}

/// Apply the launch-at-login start mode to the main window.
pub fn apply_start_mode<R: Runtime>(app: &AppHandle<R>, mode: StartMode) {
    let Some(w) = app.get_webview_window("main") else { return };
    match mode {
        StartMode::Normal => {}
        StartMode::Hidden => {
            let _ = w.hide();
        }
        StartMode::Minimised => {
            let _ = w.minimize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_to_tray_hides_instead_of_quitting() {
        let mut s = PlatformSettings::default();
        assert_eq!(close_action(&s), CloseAction::Quit, "off by default");
        s.close_to_tray = true;
        assert_eq!(close_action(&s), CloseAction::Hide);
    }

    #[test]
    fn launch_at_login_starts_hidden_to_the_tray_or_minimised() {
        let mut s = PlatformSettings::default();
        let auto = vec!["platter.exe".to_string(), AUTOSTART_ARG.to_string()];
        let normal = vec!["platter.exe".to_string()];
        assert_eq!(start_mode(&normal, &s), StartMode::Normal);
        assert_eq!(start_mode(&auto, &s), StartMode::Minimised);
        s.close_to_tray = true;
        assert_eq!(start_mode(&auto, &s), StartMode::Hidden);
        assert_eq!(start_mode(&normal, &s), StartMode::Normal);
    }

    #[test]
    fn switches_notify_only_on_change() {
        let sw = Switches::default();
        let mut rx = sw.subscribe();
        rx.borrow_and_update();
        sw.set(PlatformSettings::default());
        assert!(!rx.has_changed().unwrap());
        sw.set(PlatformSettings { close_to_tray: true, ..Default::default() });
        assert!(rx.has_changed().unwrap());
        assert!(sw.get().close_to_tray);
    }

    struct Null;
    impl EventSink for Null {
        fn state(&self, _: &PlayerState) {}
        fn position(&self, _: &PositionPayload) {}
        fn error(&self, _: &ErrorPayload) {}
    }

    #[test]
    fn tap_copies_state_to_the_worker_tray_and_menu_bar() {
        use crate::audio::events::PlayerStatus;
        use media::tests::{state, track};
        let p = Platform::new();
        let sink = p.tap(Arc::new(Null));
        let mut tray_rx = p.tray.subscribe();
        let mut menu_rx = p.menu.subscribe();
        sink.state(&state(Some(track("1")), PlayerStatus::Playing, 0));
        sink.position(&PositionPayload { position_ms: 1, duration_ms: 2, buffered_ms: 0 });
        assert!(matches!(p.rx.try_recv(), Ok(Msg::State(_))));
        assert!(matches!(p.rx.try_recv(), Ok(Msg::Position(_))));
        assert!(tray_rx.has_changed().unwrap());
        assert!(menu_rx.has_changed().unwrap());
        assert!(menu_rx.borrow_and_update().playing);
        assert_eq!(tray_rx.borrow_and_update().label, "Song 1 · Album Artist");
    }
}
