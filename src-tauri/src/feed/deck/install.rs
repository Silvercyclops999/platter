//! The Stream Deck plugin that ships inside Platter (a Tauri resource on
//! Windows and macOS): whether the Stream Deck app is on this computer,
//! whether the plugin is installed and at which version, and opening the
//! bundled `.streamDeckPlugin` so the Stream Deck app installs it.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The plugin's id, which is also its folder name in the Stream Deck app.
pub const PLUGIN_UUID: &str = "app.platter.streamdeck";
/// The bundled plugin, relative to the app's resources.
pub const RESOURCE: &str = "streamdeck/app.platter.streamdeck.streamDeckPlugin";

/// Elgato's download page for the Stream Deck app.
pub const DOWNLOAD_URL: &str = "https://www.elgato.com/downloads";

/// The manifest the bundled plugin was built from (same commit).
const MANIFEST: &str = include_str!("../../../../streamdeck/app.platter.streamdeck.sdPlugin/manifest.json");

/// The version of the plugin built into this Platter.
pub fn bundled_version() -> Option<String> {
    manifest_version(MANIFEST)
}

fn manifest_version(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json.trim_start_matches('\u{feff}')).ok()?;
    v.get("Version")?.as_str().map(str::to_owned)
}

/// Where to look for the Stream Deck app and its plugins.
#[derive(Debug, Clone, Default)]
pub struct Places {
    /// Any of these existing means the Stream Deck app is installed.
    pub app: Vec<PathBuf>,
    /// The Stream Deck app's plugins folder.
    pub plugins: Option<PathBuf>,
}

impl Places {
    /// This computer's places.
    pub fn system() -> Self {
        #[cfg(windows)]
        {
            let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
            let data = env("APPDATA").map(|a| a.join("Elgato").join("StreamDeck"));
            let mut app = vec![];
            for pf in [env("ProgramFiles"), env("ProgramW6432"), Some(PathBuf::from(r"C:\Program Files"))].into_iter().flatten() {
                app.push(pf.join("Elgato").join("StreamDeck").join("StreamDeck.exe"));
            }
            app.extend(data.clone());
            Self { app, plugins: data.map(|d| d.join("Plugins")) }
        }
        #[cfg(target_os = "macos")]
        {
            let support = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/com.elgato.StreamDeck"));
            let mut app = vec![PathBuf::from("/Applications/Elgato Stream Deck.app")];
            app.extend(support.clone());
            Self { app, plugins: support.map(|s| s.join("Plugins")) }
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            // There is no Stream Deck app for Linux.
            Self::default()
        }
    }
}

/// The Settings row (`deck_plugin_status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum PluginStatus {
    /// The Stream Deck app isn't on this computer.
    NoApp,
    /// This build of Platter has no plugin inside (a development build).
    NotBundled,
    /// The Stream Deck app is here and the plugin isn't.
    Install { version: String },
    /// The same version (or a newer one) is installed.
    Installed { version: String, installed: String },
    /// An older version is installed.
    Update { version: String, installed: String },
}

/// The installed plugin's version, from its manifest.
fn installed_version(plugins: &Path) -> Option<String> {
    let dir = plugins.join(format!("{PLUGIN_UUID}.sdPlugin"));
    let json = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    manifest_version(&json).or(Some(String::new()))
}

/// Dotted numbers, missing parts as 0; anything else as 0.
fn parts(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

/// Whether version `a` is older than `b`.
pub fn older(a: &str, b: &str) -> bool {
    let (a, b) = (parts(a), parts(b));
    let n = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..n).map(|i| at(&a, i).cmp(&at(&b, i))).find(|o| o.is_ne()) == Some(std::cmp::Ordering::Less)
}

/// What the Settings row shows. `bundled` is the plugin inside this Platter
/// (None when its file is missing).
pub fn detect(places: &Places, bundled: Option<&str>) -> PluginStatus {
    if !places.app.iter().any(|p| p.exists()) {
        return PluginStatus::NoApp;
    }
    let Some(version) = bundled.map(str::to_owned) else { return PluginStatus::NotBundled };
    match places.plugins.as_deref().and_then(installed_version) {
        None => PluginStatus::Install { version },
        Some(installed) if older(&installed, &version) => PluginStatus::Update { version, installed },
        Some(installed) => PluginStatus::Installed { version, installed },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(dir: &Path) -> Places {
        Places { app: vec![dir.join("Elgato Stream Deck.app")], plugins: Some(dir.join("Plugins")) }
    }

    fn install(dir: &Path, version: &str) {
        let p = dir.join("Plugins").join("app.platter.streamdeck.sdPlugin");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("manifest.json"), format!(r#"{{"UUID":"app.platter.streamdeck","Version":"{version}"}}"#)).unwrap();
    }

    #[test]
    fn the_bundled_version_is_the_manifests() {
        let v = bundled_version().expect("the plugin manifest has a version");
        assert_eq!(parts(&v).len(), 4, "{v}");
    }

    #[test]
    fn no_stream_deck_app() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(detect(&places(d.path()), Some("1.0.0.0")), PluginStatus::NoApp);
        assert_eq!(detect(&Places::default(), Some("1.0.0.0")), PluginStatus::NoApp);
    }

    #[test]
    fn install_update_and_installed() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Elgato Stream Deck.app")).unwrap();
        let p = places(d.path());
        assert_eq!(detect(&p, None), PluginStatus::NotBundled);
        assert_eq!(detect(&p, Some("0.2.0.0")), PluginStatus::Install { version: "0.2.0.0".into() });
        install(d.path(), "0.1.0.0");
        assert_eq!(detect(&p, Some("0.2.0.0")), PluginStatus::Update { version: "0.2.0.0".into(), installed: "0.1.0.0".into() });
        install(d.path(), "0.2.0.0");
        assert_eq!(detect(&p, Some("0.2.0.0")), PluginStatus::Installed { version: "0.2.0.0".into(), installed: "0.2.0.0".into() });
        // A newer one (built by hand) is left as installed, not "updated" down.
        install(d.path(), "0.3.0.0");
        assert!(matches!(detect(&p, Some("0.2.0.0")), PluginStatus::Installed { .. }));
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(older("0.1.0.0", "0.2.0.0"));
        assert!(older("0.9", "0.10.0.0"));
        assert!(older("", "0.1"));
        assert!(!older("0.2.0.0", "0.2"));
        assert!(!older("1.0.0.0", "0.9.9.9"));
    }

    #[test]
    fn the_status_reads_as_the_ui_expects() {
        let s = PluginStatus::Update { version: "0.2.0.0".into(), installed: "0.1.0.0".into() };
        assert_eq!(serde_json::to_value(s).unwrap(), serde_json::json!({ "state": "update", "version": "0.2.0.0", "installed": "0.1.0.0" }));
        assert_eq!(serde_json::to_value(PluginStatus::NoApp).unwrap(), serde_json::json!({ "state": "noApp" }));
    }
}
