//! Sync to a device: playlists, albums, artists and Liked Songs copied to a
//! USB drive, an SD card or a music player, as they are or converted to
//! FLAC, MP3 or Opus on this computer.
//!
//! A device is a drive (recognised again by its volume serial or UUID, or
//! by the manifest Platter keeps on it) plus a music folder on it. Devices
//! are remembered on this computer only. A sync works out what the device
//! should hold, compares that with the manifest of what Platter wrote there
//! before, and copies only what's new or changed. Files Platter didn't write
//! are never touched. Conversion runs in the background at low priority,
//! two songs at a time at most, and waits while playback or the rest of the
//! computer is busy.

pub mod commands;
pub mod drives;
pub mod encode;
pub mod io;
pub mod m3u;
pub mod manifest;
pub mod names;
pub mod plan;
pub mod run;
pub mod store;
pub mod tags;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

pub use run::{SyncControl, SyncStatus};

/// What songs become on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DeviceFormat {
    /// The file as it is on the server.
    #[default]
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "flac")]
    Flac,
    #[serde(rename = "mp3-320")]
    Mp3Cbr320,
    #[serde(rename = "mp3-v0")]
    Mp3V0,
    /// Opus at about 160 kbps.
    #[serde(rename = "opus")]
    Opus,
}

impl DeviceFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Flac => "flac",
            Self::Mp3Cbr320 => "mp3-320",
            Self::Mp3V0 => "mp3-v0",
            Self::Opus => "opus",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }

    /// The converted file's extension (None: the original's).
    pub fn ext(self) -> Option<&'static str> {
        match self {
            Self::Original => None,
            Self::Flac => Some("flac"),
            Self::Mp3Cbr320 | Self::Mp3V0 => Some("mp3"),
            Self::Opus => Some("opus"),
        }
    }

    /// The codec name a source must have to be copied instead of converted.
    pub fn codec(self) -> Option<&'static str> {
        match self {
            Self::Original => None,
            Self::Flac => Some("flac"),
            Self::Mp3Cbr320 | Self::Mp3V0 => Some("mp3"),
            Self::Opus => Some("opus"),
        }
    }
}

/// The kinds of things a device can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelKind {
    Playlist,
    Album,
    Artist,
    /// Liked Songs (its id is always empty).
    Liked,
}

impl SelKind {
    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }
}

/// One thing chosen for a device, in the order it was chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelItem {
    pub kind: SelKind,
    pub id: String,
    pub title: String,
}

impl SelItem {
    pub fn same(&self, kind: SelKind, id: &str) -> bool {
        self.kind == kind && (kind == SelKind::Liked || self.id == id)
    }
}

/// A device remembered on this computer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    /// The drive's volume serial (Windows) or UUID (macOS), when it has one.
    #[serde(default)]
    pub volume_id: Option<String>,
    /// The drive's own name when it was added.
    #[serde(default)]
    pub volume_label: String,
    /// The music folder below the drive's top, `/` separated ("" is the top).
    #[serde(default)]
    pub folder: String,
    /// None follows the default format setting.
    #[serde(default)]
    pub format: Option<DeviceFormat>,
    /// Fill at most this much of the device, in GB.
    #[serde(default)]
    pub budget_gb: Option<f64>,
    #[serde(default)]
    pub selection: Vec<SelItem>,
    /// Unix seconds of the last finished sync.
    #[serde(default)]
    pub last_sync: Option<i64>,
    /// Where the drive was mounted last time (shown while it's away).
    #[serde(default)]
    pub last_root: Option<String>,
}

pub const NAME_MAX: usize = 60;
pub const BUDGET_MAX_GB: f64 = 100_000.0;
/// Embedded covers on the device are at most this many pixels wide or tall.
pub const COVER_PX: u32 = 500;
/// The manifest's file name in the device's music folder.
pub const MANIFEST: &str = ".platter-sync.json";

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A music folder as typed or picked: `/` separated, no leading or trailing
/// slash, no `..`, every part a safe name.
pub fn clean_folder(s: &str) -> Result<String, String> {
    let mut parts = Vec::new();
    for p in s.split(['/', '\\']) {
        let p = p.trim();
        if p.is_empty() || p == "." {
            continue;
        }
        if p == ".." {
            return Err("The music folder has to be on the device.".into());
        }
        parts.push(names::component(p, "Music", names::MAX_COMPONENT));
    }
    Ok(parts.join("/"))
}
