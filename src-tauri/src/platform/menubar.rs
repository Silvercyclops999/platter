//! The menu bar on macOS: the Platter, File, Edit, Playback, View, Window
//! and Help menus a Mac app is expected to have. Windows keeps its own
//! window chrome and gets no menu bar.
//!
//! The menus are one table ([`MENUS`]) so tests can check ids, labels and
//! keys on any OS. Most items hand an action name to the UI as the
//! `menu://action` event, which runs the same code as the app's own
//! buttons and shortcuts. Quit, the project and issue pages and the log
//! folder are done here.

use parking_lot::Mutex;
use serde::Serialize;
use tauri::menu::{CheckMenuItem, MenuItem};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::audio::events::{PlayerState, PlayerStatus};
use crate::audio::queue::Repeat;

/// The UI listens for this.
pub const ACTION_EVENT: &str = "menu://action";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ActionPayload {
    pub action: &'static str,
}

/// What a menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Run a UI action. `front` brings the main window forward first.
    Ui { name: &'static str, front: bool },
    /// Quit (saves the queue on the way out, like the tray's Quit).
    Quit,
    /// Open the project page in the browser.
    OpenRepo,
    /// Open the issues page in the browser.
    ReportIssue,
    /// Show the log folder in Finder.
    ShowLogs,
}

/// Items macOS provides itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Native {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    CloseWindow,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Minimize,
    Zoom,
    BringAllToFront,
}

impl Native {
    /// The keys the system gives it, for the duplicate check.
    pub fn accelerator(self) -> Option<&'static str> {
        Some(match self {
            Native::Hide => "Cmd+H",
            Native::HideOthers => "Alt+Cmd+H",
            Native::CloseWindow => "Cmd+W",
            Native::Undo => "Cmd+Z",
            Native::Redo => "Shift+Cmd+Z",
            Native::Cut => "Cmd+X",
            Native::Copy => "Cmd+C",
            Native::Paste => "Cmd+V",
            Native::SelectAll => "Cmd+A",
            Native::Minimize => "Cmd+M",
            Native::About | Native::Services | Native::ShowAll | Native::Zoom | Native::BringAllToFront => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub id: &'static str,
    pub label: &'static str,
    pub accelerator: Option<&'static str>,
    /// Shows a checkmark that follows playback (or Go offline).
    pub check: bool,
    pub action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Item(Item),
    Native(Native),
    Separator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    App,
    Window,
    Help,
    Plain,
}

pub struct MenuDef {
    pub title: &'static str,
    pub role: Role,
    pub entries: &'static [Entry],
}

const fn ui(id: &'static str, label: &'static str, accelerator: Option<&'static str>, name: &'static str) -> Entry {
    Entry::Item(Item { id, label, accelerator, check: false, action: Action::Ui { name, front: true } })
}

/// A playback item: works with the window hidden, so it stays hidden.
const fn play(id: &'static str, label: &'static str, accelerator: Option<&'static str>, name: &'static str, check: bool) -> Entry {
    Entry::Item(Item { id, label, accelerator, check, action: Action::Ui { name, front: false } })
}

pub const PLAY_PAUSE_ID: &str = "menu-play-pause";
pub const SHUFFLE_ID: &str = "menu-shuffle";
pub const REPEAT_ID: &str = "menu-repeat";
pub const GO_OFFLINE_ID: &str = "menu-go-offline";

/// Every menu, in menu bar order. Keys match the app's own shortcuts
/// (Space for play/pause is the webview's, so that item shows none).
pub const MENUS: &[MenuDef] = &[
    MenuDef {
        title: "Platter",
        role: Role::App,
        entries: &[
            Entry::Native(Native::About),
            ui("menu-check-updates", "Check for Updates…", None, "checkUpdates"),
            Entry::Separator,
            ui("menu-settings", "Settings…", Some("Cmd+,"), "settings"),
            Entry::Separator,
            Entry::Native(Native::Services),
            Entry::Separator,
            Entry::Native(Native::Hide),
            Entry::Native(Native::HideOthers),
            Entry::Native(Native::ShowAll),
            Entry::Separator,
            Entry::Item(Item { id: "menu-quit", label: "Quit Platter", accelerator: Some("Cmd+Q"), check: false, action: Action::Quit }),
        ],
    },
    MenuDef {
        title: "File",
        role: Role::Plain,
        entries: &[
            ui("menu-new-playlist", "New Playlist", Some("Cmd+N"), "newPlaylist"),
            ui("menu-new-folder", "New Playlist Folder", Some("Shift+Cmd+N"), "newFolder"),
            Entry::Separator,
            // The same switch as the profile menu's; the check follows the setting.
            Entry::Item(Item { id: GO_OFFLINE_ID, label: "Go Offline", accelerator: None, check: true, action: Action::Ui { name: "goOffline", front: false } }),
            ui("menu-sign-out", "Sign Out…", None, "signOut"),
            Entry::Separator,
            Entry::Native(Native::CloseWindow),
        ],
    },
    MenuDef {
        title: "Edit",
        role: Role::Plain,
        entries: &[
            Entry::Native(Native::Undo),
            Entry::Native(Native::Redo),
            Entry::Separator,
            Entry::Native(Native::Cut),
            Entry::Native(Native::Copy),
            Entry::Native(Native::Paste),
            Entry::Native(Native::SelectAll),
            Entry::Separator,
            ui("menu-search", "Search", Some("Cmd+L"), "search"),
            ui("menu-filter-library", "Filter Your Library", Some("Cmd+F"), "filterLibrary"),
        ],
    },
    MenuDef {
        title: "Playback",
        role: Role::Plain,
        entries: &[
            play(PLAY_PAUSE_ID, "Play", None, "playPause", false),
            Entry::Separator,
            play("menu-next", "Next", Some("Cmd+Right"), "next", false),
            play("menu-previous", "Previous", Some("Cmd+Left"), "previous", false),
            Entry::Separator,
            play("menu-volume-up", "Volume Up", Some("Cmd+Up"), "volumeUp", false),
            play("menu-volume-down", "Volume Down", Some("Cmd+Down"), "volumeDown", false),
            Entry::Separator,
            play(SHUFFLE_ID, "Shuffle", Some("Cmd+S"), "shuffle", true),
            play(REPEAT_ID, "Repeat", Some("Cmd+R"), "repeat", true),
            play("menu-shuffle-albums", "Shuffle Albums", None, "shuffleAlbums", false),
        ],
    },
    MenuDef {
        title: "View",
        role: Role::Plain,
        entries: &[
            play("menu-miniplayer", "Miniplayer", None, "miniplayer", false),
            ui("menu-full-window", "Full-Window Now Playing", None, "fullWindow"),
            ui("menu-lyrics", "Lyrics", None, "lyrics"),
            Entry::Separator,
            ui("menu-queue", "Queue", Some("Alt+Shift+Q"), "queue"),
            ui("menu-toggle-library", "Show or Hide Your Library", Some("Alt+Shift+L"), "toggleLibrary"),
        ],
    },
    MenuDef {
        title: "Window",
        role: Role::Window,
        entries: &[
            Entry::Native(Native::Minimize),
            Entry::Native(Native::Zoom),
            Entry::Separator,
            Entry::Native(Native::BringAllToFront),
        ],
    },
    MenuDef {
        title: "Help",
        role: Role::Help,
        entries: &[
            ui("menu-shortcuts", "Keyboard Shortcuts", Some("Cmd+/"), "shortcuts"),
            Entry::Separator,
            Entry::Item(Item { id: "menu-report-issue", label: "Report an Issue…", accelerator: None, check: false, action: Action::ReportIssue }),
            Entry::Item(Item { id: "menu-show-logs", label: "Show Logs in Finder", accelerator: None, check: false, action: Action::ShowLogs }),
            Entry::Separator,
            Entry::Item(Item { id: "menu-github", label: "Platter on GitHub", accelerator: None, check: false, action: Action::OpenRepo }),
            Entry::Native(Native::About),
        ],
    },
];

/// Every item of ours in the table.
pub fn items() -> impl Iterator<Item = &'static Item> {
    MENUS.iter().flat_map(|m| m.entries.iter()).filter_map(|e| match e {
        Entry::Item(i) => Some(i),
        _ => None,
    })
}

/// What a clicked menu id does. Ids from other menus (the tray) are `None`.
pub fn action_for(id: &str) -> Option<Action> {
    items().find(|i| i.id == id).map(|i| i.action)
}

/// What the menus show about playback.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MenuView {
    pub playing: bool,
    pub shuffle: bool,
    pub repeat: bool,
}

pub fn menu_view(state: &PlayerState) -> MenuView {
    MenuView {
        playing: state.track.is_some()
            && matches!(state.status, PlayerStatus::Playing | PlayerStatus::Loading | PlayerStatus::Buffering),
        shuffle: state.shuffle,
        repeat: state.repeat != Repeat::Off,
    }
}

pub fn play_pause_label(v: &MenuView) -> &'static str {
    if v.playing { "Pause" } else { "Play" }
}

/// Everything the menus' checks and the play/pause label follow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MenuChecks {
    pub view: MenuView,
    /// Go offline is on.
    pub offline: bool,
}

impl MenuChecks {
    /// Player state changed. Returns whether anything did.
    pub fn set_view(&mut self, v: MenuView) -> bool {
        std::mem::replace(&mut self.view, v) != v
    }

    /// The Go offline setting changed. Returns whether it did.
    pub fn set_offline(&mut self, on: bool) -> bool {
        std::mem::replace(&mut self.offline, on) != on
    }
}

struct Live<R: Runtime> {
    play_pause: MenuItem<R>,
    shuffle: CheckMenuItem<R>,
    repeat: CheckMenuItem<R>,
    go_offline: CheckMenuItem<R>,
}

impl<R: Runtime> Live<R> {
    fn show(&self, c: &MenuChecks) {
        let _ = self.play_pause.set_text(play_pause_label(&c.view));
        let _ = self.shuffle.set_checked(c.view.shuffle);
        let _ = self.repeat.set_checked(c.view.repeat);
        let _ = self.go_offline.set_checked(c.offline);
    }
}

/// The menu bar's items that follow playback and Go offline (none on Windows).
pub struct MenuBar<R: Runtime> {
    live: Mutex<Option<Live<R>>>,
    checks: Mutex<MenuChecks>,
}

impl<R: Runtime> Default for MenuBar<R> {
    fn default() -> Self {
        Self { live: Mutex::new(None), checks: Mutex::new(MenuChecks::default()) }
    }
}

impl<R: Runtime> MenuBar<R> {
    /// Player state changed.
    pub fn update(&self, v: MenuView) {
        let changed = self.checks.lock().set_view(v);
        if changed {
            self.reshow();
        }
    }

    /// The Go offline setting changed.
    pub fn set_offline(&self, on: bool) {
        let changed = self.checks.lock().set_offline(on);
        if changed {
            self.reshow();
        }
    }

    /// Put the checkmarks back to the player's state. A check item flips
    /// itself when clicked; the player's next state (or the setting) sets
    /// it for real.
    fn reshow(&self) {
        let c = *self.checks.lock();
        if let Some(live) = self.live.lock().as_ref() {
            live.show(&c);
        }
    }
}

/// Build the menu bar and route its clicks. Only macOS gets one.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        match build(app) {
            Ok((menu, live)) => {
                let bar = app.state::<MenuBar<R>>();
                *bar.live.lock() = Some(live);
                bar.reshow();
                if let Err(e) = app.set_menu(menu) {
                    tracing::warn!("menu bar: {e}");
                }
            }
            Err(e) => tracing::warn!("menu bar: {e}"),
        }
        app.on_menu_event(|app, e| on_menu(app, e.id().as_ref()));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// A menu item was clicked.
pub fn on_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let Some(action) = action_for(id) else { return };
    match action {
        Action::Quit => app.exit(0),
        Action::OpenRepo => open_url(app, crate::settings::REPO_URL),
        Action::ReportIssue => open_url(app, crate::settings::ISSUES_URL),
        Action::ShowLogs => super::logs::show(app),
        Action::Ui { name, front } => {
            if front {
                super::show_window(app);
            }
            if let Some(bar) = app.try_state::<MenuBar<R>>() {
                bar.reshow();
            }
            let _ = app.emit(ACTION_EVENT, ActionPayload { action: name });
        }
    }
}

fn open_url<R: Runtime>(app: &AppHandle<R>, url: &str) {
    use tauri_plugin_opener::OpenerExt;
    if let Err(e) = app.opener().open_url(url, None::<&str>) {
        tracing::warn!("open {url}: {e}");
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<(tauri::menu::Menu<R>, Live<R>)> {
    use tauri::menu::{AboutMetadata, Menu, PredefinedMenuItem, Submenu};

    let mut play_pause = None;
    let mut shuffle = None;
    let mut repeat = None;
    let mut go_offline = None;
    let menu = Menu::new(app)?;
    for def in MENUS {
        let sub = Submenu::new(app, def.title, true)?;
        for entry in def.entries {
            match entry {
                Entry::Separator => sub.append(&PredefinedMenuItem::separator(app)?)?,
                Entry::Native(n) => {
                    let item = match n {
                        Native::About => PredefinedMenuItem::about(
                            app,
                            Some("About Platter"),
                            Some(AboutMetadata {
                                name: Some("Platter".into()),
                                version: Some(app.package_info().version.to_string()),
                                website: Some(crate::settings::REPO_URL.into()),
                                website_label: Some("Platter on GitHub".into()),
                                ..Default::default()
                            }),
                        )?,
                        Native::Services => PredefinedMenuItem::services(app, None)?,
                        Native::Hide => PredefinedMenuItem::hide(app, Some("Hide Platter"))?,
                        Native::HideOthers => PredefinedMenuItem::hide_others(app, None)?,
                        Native::ShowAll => PredefinedMenuItem::show_all(app, None)?,
                        Native::CloseWindow => PredefinedMenuItem::close_window(app, None)?,
                        Native::Undo => PredefinedMenuItem::undo(app, None)?,
                        Native::Redo => PredefinedMenuItem::redo(app, None)?,
                        Native::Cut => PredefinedMenuItem::cut(app, None)?,
                        Native::Copy => PredefinedMenuItem::copy(app, None)?,
                        Native::Paste => PredefinedMenuItem::paste(app, None)?,
                        Native::SelectAll => PredefinedMenuItem::select_all(app, None)?,
                        Native::Minimize => PredefinedMenuItem::minimize(app, None)?,
                        Native::Zoom => PredefinedMenuItem::maximize(app, Some("Zoom"))?,
                        Native::BringAllToFront => PredefinedMenuItem::bring_all_to_front(app, None)?,
                    };
                    sub.append(&item)?;
                }
                Entry::Item(i) if i.check => {
                    let item = CheckMenuItem::with_id(app, i.id, i.label, true, false, i.accelerator)?;
                    sub.append(&item)?;
                    match i.id {
                        SHUFFLE_ID => shuffle = Some(item),
                        REPEAT_ID => repeat = Some(item),
                        GO_OFFLINE_ID => go_offline = Some(item),
                        _ => {}
                    }
                }
                Entry::Item(i) => {
                    let item = MenuItem::with_id(app, i.id, i.label, true, i.accelerator)?;
                    sub.append(&item)?;
                    if i.id == PLAY_PAUSE_ID {
                        play_pause = Some(item);
                    }
                }
            }
        }
        menu.append(&sub)?;
        #[cfg(target_os = "macos")]
        match def.role {
            Role::Window => sub.set_as_windows_menu_for_nsapp()?,
            Role::Help => sub.set_as_help_menu_for_nsapp()?,
            Role::App | Role::Plain => {}
        }
    }
    let missing = || tauri::Error::AssetNotFound("menu item missing from the table".into());
    let live = Live {
        play_pause: play_pause.ok_or_else(missing)?,
        shuffle: shuffle.ok_or_else(missing)?,
        repeat: repeat.ok_or_else(missing)?,
        go_offline: go_offline.ok_or_else(missing)?,
    };
    Ok((menu, live))
}

/// One accelerator in a fixed form ("ALT+CMD+H"), for comparing. `None`
/// when it has a token the menu wouldn't understand.
pub fn normalize(accel: &str) -> Option<String> {
    const NAMED: &[&str] = &["UP", "DOWN", "LEFT", "RIGHT", "SPACE", "ENTER", "ESCAPE", "TAB", "BACKSPACE", "DELETE"];
    let mut mods = Vec::new();
    let mut key = None;
    for t in accel.split('+') {
        let t = t.trim().to_uppercase();
        let m = match t.as_str() {
            "CMD" | "COMMAND" | "META" | "SUPER" | "CMDORCTRL" | "COMMANDORCONTROL" => Some("CMD"),
            "ALT" | "OPTION" => Some("ALT"),
            "SHIFT" => Some("SHIFT"),
            "CTRL" | "CONTROL" => Some("CTRL"),
            _ => None,
        };
        match m {
            Some(m) if !mods.contains(&m) => mods.push(m),
            Some(_) => return None,
            None if key.is_some() => return None,
            None => {
                let named = match t.as_str() {
                    "ARROWUP" => "UP",
                    "ARROWDOWN" => "DOWN",
                    "ARROWLEFT" => "LEFT",
                    "ARROWRIGHT" => "RIGHT",
                    other => other,
                };
                let ok = NAMED.contains(&named)
                    || (named.chars().count() == 1 && named.chars().all(|c| c.is_ascii_alphanumeric() || ",./;[]\\-='`".contains(c)));
                if !ok {
                    return None;
                }
                key = Some(named.to_string());
            }
        }
    }
    let key = key?;
    mods.sort_unstable();
    mods.push(key.as_str());
    Some(mods.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::PlatformSettings;
    use crate::platform::media::tests::{state, track};
    use std::collections::HashSet;

    #[test]
    fn menus_are_in_mac_order() {
        let titles: Vec<_> = MENUS.iter().map(|m| m.title).collect();
        assert_eq!(titles, ["Platter", "File", "Edit", "Playback", "View", "Window", "Help"]);
        assert_eq!(MENUS[0].role, Role::App);
        assert_eq!(MENUS.iter().filter(|m| m.role == Role::Window).count(), 1);
        assert_eq!(MENUS.iter().filter(|m| m.role == Role::Help).count(), 1);
    }

    #[test]
    fn ids_and_labels_are_unique_and_set() {
        let mut ids = HashSet::new();
        for i in items() {
            assert!(i.id.starts_with("menu-"), "{} isn't namespaced away from the tray", i.id);
            assert!(ids.insert(i.id), "{} is used twice", i.id);
            assert!(!i.label.trim().is_empty());
            assert!(!i.label.contains("..."), "{}: use the ellipsis character", i.label);
        }
        for m in MENUS {
            let labels: Vec<_> = m
                .entries
                .iter()
                .filter_map(|e| if let Entry::Item(i) = e { Some(i.label) } else { None })
                .collect();
            let set: HashSet<_> = labels.iter().collect();
            assert_eq!(set.len(), labels.len(), "a label repeats in {}", m.title);
            assert!(!matches!(m.entries.first(), Some(Entry::Separator)));
            assert!(!matches!(m.entries.last(), Some(Entry::Separator)));
            assert!(!m.entries.windows(2).any(|w| w[0] == Entry::Separator && w[1] == Entry::Separator));
        }
    }

    #[test]
    fn accelerators_parse_and_never_repeat() {
        let mut seen = HashSet::new();
        let all = MENUS.iter().flat_map(|m| m.entries.iter()).filter_map(|e| match e {
            Entry::Item(i) => i.accelerator.map(|a| (i.label, a)),
            Entry::Native(n) => n.accelerator().map(|a| ("system item", a)),
            Entry::Separator => None,
        });
        for (label, a) in all {
            let n = normalize(a).unwrap_or_else(|| panic!("{label}: {a} doesn't parse"));
            assert!(seen.insert(n.clone()), "{label}: {a} is already used");
        }
        assert!(seen.contains("CMD+,") && seen.contains("CMD+Q") && seen.contains("ALT+CMD+H"));
    }

    #[test]
    fn accelerators_match_the_apps_shortcuts() {
        let keys: Vec<_> = items().map(|i| (i.id, i.accelerator.and_then(normalize))).collect();
        let want = [
            ("menu-settings", "CMD+,"),
            ("menu-quit", "CMD+Q"),
            ("menu-new-playlist", "CMD+N"),
            ("menu-new-folder", "CMD+SHIFT+N"),
            ("menu-search", "CMD+L"),
            ("menu-filter-library", "CMD+F"),
            ("menu-next", "CMD+RIGHT"),
            ("menu-previous", "CMD+LEFT"),
            ("menu-volume-up", "CMD+UP"),
            ("menu-volume-down", "CMD+DOWN"),
            ("menu-shuffle", "CMD+S"),
            ("menu-repeat", "CMD+R"),
            ("menu-queue", "ALT+SHIFT+Q"),
            ("menu-toggle-library", "ALT+SHIFT+L"),
            ("menu-shortcuts", "CMD+/"),
        ];
        for (id, k) in want {
            let got = keys.iter().find(|(i, _)| *i == id).unwrap_or_else(|| panic!("{id} missing"));
            assert_eq!(got.1.as_deref(), Some(k), "{id}");
        }
        // Space is the webview's; a menu key would take it from text fields.
        assert_eq!(items().find(|i| i.id == PLAY_PAUSE_ID).unwrap().accelerator, None);
        assert_eq!(items().filter(|i| i.accelerator.is_some()).count(), want.len());
    }

    #[test]
    fn normalize_rejects_bad_keys() {
        assert_eq!(normalize("Shift+Cmd+N").as_deref(), Some("CMD+SHIFT+N"));
        assert_eq!(normalize("Cmd+ArrowRight").as_deref(), Some("CMD+RIGHT"));
        assert_eq!(normalize("Cmd+Cmd+N"), None);
        assert_eq!(normalize("Cmd+N+M"), None);
        assert_eq!(normalize("Cmd+"), None);
        assert_eq!(normalize("Hyper+N"), None);
    }

    #[test]
    fn menu_ids_map_to_actions() {
        assert_eq!(action_for("menu-settings"), Some(Action::Ui { name: "settings", front: true }));
        assert_eq!(action_for("menu-check-updates"), Some(Action::Ui { name: "checkUpdates", front: true }));
        assert_eq!(action_for("menu-new-playlist"), Some(Action::Ui { name: "newPlaylist", front: true }));
        assert_eq!(action_for("menu-next"), Some(Action::Ui { name: "next", front: false }));
        assert_eq!(action_for(PLAY_PAUSE_ID), Some(Action::Ui { name: "playPause", front: false }));
        assert_eq!(action_for(SHUFFLE_ID), Some(Action::Ui { name: "shuffle", front: false }));
        assert_eq!(action_for("menu-shortcuts"), Some(Action::Ui { name: "shortcuts", front: true }));
        assert_eq!(action_for("menu-quit"), Some(Action::Quit));
        assert_eq!(action_for("menu-github"), Some(Action::OpenRepo));
        assert_eq!(action_for(GO_OFFLINE_ID), Some(Action::Ui { name: "goOffline", front: false }));
        assert_eq!(action_for("menu-sign-out"), Some(Action::Ui { name: "signOut", front: true }));
        assert_eq!(action_for("menu-report-issue"), Some(Action::ReportIssue));
        assert_eq!(action_for("menu-show-logs"), Some(Action::ShowLogs));
        // The tray's ids and unknown ones do nothing here.
        assert_eq!(action_for("tray-quit"), None);
        assert_eq!(action_for("nope"), None);
        // Every UI action name is distinct.
        let names: Vec<_> = items()
            .filter_map(|i| if let Action::Ui { name, .. } = i.action { Some(name) } else { None })
            .collect();
        assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len());
    }

    fn labels(title: &str) -> Vec<String> {
        let m = MENUS.iter().find(|m| m.title == title).unwrap();
        m.entries
            .iter()
            .map(|e| match e {
                Entry::Item(i) => i.label.to_string(),
                Entry::Native(n) => format!("{n:?}"),
                Entry::Separator => "-".into(),
            })
            .collect()
    }

    #[test]
    fn file_has_go_offline_and_sign_out() {
        assert_eq!(
            labels("File"),
            ["New Playlist", "New Playlist Folder", "-", "Go Offline", "Sign Out…", "-", "CloseWindow"]
        );
        let go = items().find(|i| i.id == GO_OFFLINE_ID).unwrap();
        assert!(go.check, "Go Offline shows a checkmark");
        assert_eq!(go.accelerator, None);
    }

    #[test]
    fn help_reports_issues_shows_logs_and_about() {
        assert_eq!(
            labels("Help"),
            ["Keyboard Shortcuts", "-", "Report an Issue…", "Show Logs in Finder", "-", "Platter on GitHub", "About"]
        );
        assert!(crate::settings::ISSUES_URL.starts_with(crate::settings::REPO_URL));
        assert!(crate::settings::ISSUES_URL.ends_with("/issues"));
    }

    #[test]
    fn the_go_offline_check_follows_the_setting() {
        let mut c = MenuChecks::default();
        assert!(!c.offline);
        assert!(c.set_offline(true));
        assert!(!c.set_offline(true), "no change, no redraw");
        // Playback changes leave it alone.
        assert!(c.set_view(MenuView { playing: true, shuffle: false, repeat: false }));
        assert!(c.offline);
        assert!(c.set_offline(false));
        assert!(!c.offline && c.view.playing);
        assert!(!PlatformSettings::default().go_offline);
    }

    #[test]
    fn checks_follow_shuffle_and_repeat() {
        assert_eq!(menu_view(&state(None, PlayerStatus::Idle, 0)), MenuView::default());
        let mut s = state(Some(track("1")), PlayerStatus::Playing, 0);
        s.shuffle = true;
        s.repeat = Repeat::One;
        let v = menu_view(&s);
        assert_eq!(v, MenuView { playing: true, shuffle: true, repeat: true });
        assert_eq!(play_pause_label(&v), "Pause");
        s.status = PlayerStatus::Paused;
        s.shuffle = false;
        s.repeat = Repeat::Off;
        let v = menu_view(&s);
        assert_eq!(v, MenuView::default());
        assert_eq!(play_pause_label(&v), "Play");
        s.repeat = Repeat::All;
        assert!(menu_view(&s).repeat);
    }
}
