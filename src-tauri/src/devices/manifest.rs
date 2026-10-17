//! The manifest: a small JSON file in the device's music folder listing
//! every file Platter wrote there, which song it holds and how it was made.
//! A re-sync compares against it to copy only what changed, and anything
//! not in it belongs to someone else and is left alone.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::DeviceFormat;
use crate::model::Track;

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub version: u32,
    /// The device this music folder belongs to (finds the drive again when
    /// it has no volume id).
    pub device_id: String,
    /// Songs by path below the music folder (`/` separated).
    #[serde(default)]
    pub files: BTreeMap<String, Entry>,
    /// Playlist files by path, with a hash of what was written.
    #[serde(default)]
    pub playlists: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub track_id: String,
    /// What the file was made from ([`fingerprint`]).
    pub source: String,
    pub format: DeviceFormat,
    /// The file's size when written.
    pub size: u64,
}

impl Manifest {
    pub fn new(device_id: &str) -> Self {
        Self { version: VERSION, device_id: device_id.to_string(), files: BTreeMap::new(), playlists: BTreeMap::new() }
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let m: Self = serde_json::from_slice(bytes).ok()?;
        (m.version <= VERSION).then_some(m)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).unwrap_or_default()
    }

    /// Bytes of every song file Platter wrote.
    pub fn bytes(&self) -> u64 {
        self.files.values().map(|e| e.size).sum()
    }
}

/// What a song file is made from: the song, its file on the server and
/// how it was converted. Any change means the file is made again.
pub fn fingerprint(t: &Track, format: DeviceFormat) -> String {
    let mut h = Sha256::new();
    for part in [
        t.id.as_str(),
        t.part_key.as_str(),
        t.codec.as_deref().unwrap_or(""),
        t.container.as_deref().unwrap_or(""),
        &t.bitrate.unwrap_or(0).to_string(),
        &t.duration_ms.to_string(),
        format.as_str(),
        // Tags written from the library when the file has none.
        t.title.as_str(),
        t.album_title.as_str(),
        t.artist_title.as_str(),
        &t.track_number.to_string(),
        &t.disc_number.to_string(),
    ] {
        h.update(part.as_bytes());
        h.update([0]);
    }
    let d = h.finalize();
    d.iter().take(10).map(|b| format!("{b:02x}")).collect()
}

pub fn hash_text(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    d.iter().take(10).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::fake;

    #[test]
    fn manifests_round_trip() {
        let mut m = Manifest::new("d1");
        m.files.insert(
            "A/B/01 C.mp3".into(),
            Entry { track_id: "t1".into(), source: "abc".into(), format: DeviceFormat::Mp3V0, size: 10 },
        );
        m.playlists.insert("Road.m3u8".into(), "h".into());
        let back = Manifest::parse(&m.to_bytes()).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.bytes(), 10);
        assert!(Manifest::parse(b"not json").is_none());
        assert!(Manifest::parse(br#"{"version":99,"deviceId":"x"}"#).is_none());
    }

    #[test]
    fn fingerprints_change_with_the_file_or_the_format() {
        let ar = fake::artist("ar", "Artist");
        let al = fake::album("al", "Album", &ar, 2000, None, &[]);
        let t = fake::track("t1", "Song", &al, 1, 1);
        let a = fingerprint(&t, DeviceFormat::Flac);
        assert_eq!(a, fingerprint(&t, DeviceFormat::Flac));
        assert_ne!(a, fingerprint(&t, DeviceFormat::Opus));
        let mut t2 = t.clone();
        t2.part_key = "/library/parts/9/1/new.flac".into();
        assert_ne!(a, fingerprint(&t2, DeviceFormat::Flac));
    }
}
