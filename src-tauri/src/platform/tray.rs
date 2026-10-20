//! Tray icon (Windows) / menu bar item (macOS), shown while "close to tray"
//! is on. Left click shows the window; the menu has the current track,
//! play/pause, next, previous, Go offline, Show Platter and Quit.

use parking_lot::Mutex;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Runtime};

use super::media::OsCommand;
use crate::audio::events::{PlayerState, PlayerStatus};

const TRAY_ID: &str = "platter-tray";
/// The Go offline check. The UI flips the setting, the same as its own switch.
const OFFLINE_ID: &str = "tray-offline";

/// The tray glyph (icons/tray.svg drawn at 32 px): the app icon's P, whose
/// bowl is a record, on its graphite tile. On macOS the menu bar takes the black template
/// copy, which the system colours for light and dark menu bars.
const TRAY_PNG: &[u8] = if cfg!(target_os = "macos") {
    include_bytes!("../../icons/tray-template.png")
} else {
    include_bytes!("../../icons/tray.png")
};

/// The tray glyph as an image, or None if it can't be read (the app icon is used then).
fn tray_glyph() -> Option<tauri::image::Image<'static>> {
    let img = image::load_from_memory_with_format(TRAY_PNG, image::ImageFormat::Png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some(tauri::image::Image::new_owned(img.into_raw(), w, h))
}

/// What the tray menu shows about playback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayView {
    /// The disabled first line: the track, or "Nothing playing".
    pub label: String,
    pub playing: bool,
    pub has_track: bool,
}

impl Default for TrayView {
    fn default() -> Self {
        Self { label: "Nothing playing".into(), playing: false, has_track: false }
    }
}

pub fn tray_view(state: &PlayerState) -> TrayView {
    match &state.track {
        None => TrayView::default(),
        Some(t) => {
            let artist = t.track_artist.as_deref().unwrap_or(&t.artist_title);
            let mut label = format!("{} · {artist}", t.title);
            // Menus don't wrap; keep it to a sensible width.
            if label.chars().count() > 60 {
                label = label.chars().take(59).collect::<String>() + "…";
            }
            TrayView {
                label,
                playing: matches!(state.status, PlayerStatus::Playing | PlayerStatus::Loading | PlayerStatus::Buffering),
                has_track: true,
            }
        }
    }
}

/// The UI action a tray item runs instead of a player command, if any.
pub fn menu_ui_action(id: &str) -> Option<&'static str> {
    (id == OFFLINE_ID).then_some("goOffline")
}

/// Map a tray menu item id to a command.
pub fn menu_command(id: &str) -> Option<OsCommand> {
    Some(match id {
        "tray-toggle" => OsCommand::Toggle,
        "tray-next" => OsCommand::Next,
        "tray-previous" => OsCommand::Previous,
        "tray-show" => OsCommand::Show,
        "tray-quit" => OsCommand::Quit,
        _ => return None,
    })
}

struct Items<R: Runtime> {
    label: MenuItem<R>,
    toggle: MenuItem<R>,
    next: MenuItem<R>,
    previous: MenuItem<R>,
    offline: CheckMenuItem<R>,
}

/// The tray while it exists, plus the last view so a new tray starts current.
pub struct Tray<R: Runtime> {
    items: Mutex<Option<Items<R>>>,
    view: Mutex<TrayView>,
    offline: Mutex<bool>,
}

impl<R: Runtime> Default for Tray<R> {
    fn default() -> Self {
        Self { items: Mutex::new(None), view: Mutex::new(TrayView::default()), offline: Mutex::new(false) }
    }
}

impl<R: Runtime> Tray<R> {
    /// Show or remove the tray icon.
    pub fn set_shown(
        &self,
        app: &AppHandle<R>,
        on: bool,
        on_command: impl Fn(OsCommand) + Send + Sync + Clone + 'static,
    ) -> tauri::Result<()> {
        let mut items = self.items.lock();
        if !on {
            if items.take().is_some()
                && let Some(t) = app.remove_tray_by_id(TRAY_ID)
            {
                let _ = t.set_visible(false);
            }
            return Ok(());
        }
        if items.is_some() {
            return Ok(());
        }
        let v = self.view.lock().clone();
        let label = MenuItem::with_id(app, "tray-label", &v.label, false, None::<&str>)?;
        let toggle = MenuItem::with_id(app, "tray-toggle", if v.playing { "Pause" } else { "Play" }, v.has_track, None::<&str>)?;
        let next = MenuItem::with_id(app, "tray-next", "Next", v.has_track, None::<&str>)?;
        let previous = MenuItem::with_id(app, "tray-previous", "Previous", v.has_track, None::<&str>)?;
        let offline = CheckMenuItem::with_id(app, OFFLINE_ID, "Go offline", true, *self.offline.lock(), None::<&str>)?;
        let show = MenuItem::with_id(app, "tray-show", "Show Platter", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "tray-quit", "Quit Platter", true, None::<&str>)?;
        let menu = Menu::with_items(
            app,
            &[
                &label,
                &PredefinedMenuItem::separator(app)?,
                &toggle,
                &next,
                &previous,
                &PredefinedMenuItem::separator(app)?,
                &offline,
                &PredefinedMenuItem::separator(app)?,
                &show,
                &quit,
            ],
        )?;
        let menu_cmd = on_command.clone();
        let mut builder = TrayIconBuilder::with_id(TRAY_ID)
            .tooltip("Platter")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(move |app, e| {
                let id = e.id().as_ref();
                if let Some(action) = menu_ui_action(id) {
                    let _ = app.emit(super::menubar::ACTION_EVENT, super::menubar::ActionPayload { action });
                } else if let Some(c) = menu_command(id) {
                    menu_cmd(c)
                }
            })
            .on_tray_icon_event(move |_, e| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    on_command(OsCommand::Show)
                }
            });
        if let Some(icon) = tray_glyph() {
            builder = builder.icon(icon).icon_as_template(cfg!(target_os = "macos"));
        } else if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        builder.build(app)?;
        *items = Some(Items { label, toggle, next, previous, offline });
        Ok(())
    }

    /// The Go offline setting changed: move the check to match.
    pub fn set_offline(&self, on: bool) {
        *self.offline.lock() = on;
        if let Some(items) = self.items.lock().as_ref() {
            let _ = items.offline.set_checked(on);
        }
    }

    /// Player state changed: refresh the menu text if it differs.
    pub fn update(&self, view: TrayView) {
        {
            let mut cur = self.view.lock();
            if *cur == view {
                return;
            }
            *cur = view.clone();
        }
        if let Some(items) = self.items.lock().as_ref() {
            let _ = items.label.set_text(&view.label);
            let _ = items.toggle.set_text(if view.playing { "Pause" } else { "Play" });
            for i in [&items.toggle, &items.next, &items.previous] {
                let _ = i.set_enabled(view.has_track);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::media::tests::{state, track};

    #[test]
    fn tray_glyph_reads_at_32_pixels() {
        let img = tray_glyph().expect("tray glyph");
        assert_eq!((img.width(), img.height()), (32, 32));
        // Not blank: some pixels are drawn.
        assert!(img.rgba().chunks(4).any(|p| p[3] > 0));
    }

    #[test]
    fn tray_label_follows_the_track_and_status() {
        assert_eq!(tray_view(&state(None, PlayerStatus::Idle, 0)), TrayView::default());
        let v = tray_view(&state(Some(track("1")), PlayerStatus::Playing, 0));
        assert_eq!(v.label, "Song 1 · Album Artist");
        assert!(v.playing && v.has_track);
        let mut t = track("2");
        t.title = "x".repeat(100);
        let v = tray_view(&state(Some(t), PlayerStatus::Paused, 0));
        assert!(!v.playing);
        assert_eq!(v.label.chars().count(), 60);
    }

    #[test]
    fn menu_items_map_to_commands() {
        assert_eq!(menu_command("tray-toggle"), Some(OsCommand::Toggle));
        assert_eq!(menu_command("tray-next"), Some(OsCommand::Next));
        assert_eq!(menu_command("tray-previous"), Some(OsCommand::Previous));
        assert_eq!(menu_command("tray-show"), Some(OsCommand::Show));
        assert_eq!(menu_command("tray-quit"), Some(OsCommand::Quit));
        assert_eq!(menu_command("tray-label"), None);
    }

    #[test]
    fn go_offline_is_a_ui_action_not_a_player_command() {
        assert_eq!(menu_ui_action("tray-offline"), Some("goOffline"));
        assert_eq!(menu_command("tray-offline"), None);
        assert_eq!(menu_ui_action("tray-toggle"), None);
    }
}
