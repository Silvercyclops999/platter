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

    /// Reads a manifest from the device. The file comes from removable media,
    /// so any entry whose path could leave the music folder is dropped.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let mut m: Self = serde_json::from_slice(bytes).ok()?;
        if m.version > VERSION {
            return None;
        }
        m.files.retain(|k, _| safe_rel(k));
        m.playlists.retain(|k, _| safe_rel(k));
        Some(m)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).unwrap_or_default()
    }

    /// Bytes of every song file Platter wrote.
    pub fn bytes(&self) -> u64 {
        self.files.values().fold(0u64, |a, e| a.saturating_add(e.size))
    }
}

/// True for a plain relative path below the music folder: `/`-separated
/// parts, none empty, `.` or `..`, and no drive letters or backslashes.
pub fn safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && rel.len() <= 1024
        && rel.split('/').all(|p| !p.is_empty() && p != "." && p != ".." && !p.contains([':', '\\', '\0']))
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

    #[test]
    fn manifest_entries_that_leave_the_folder_are_dropped() {
        let json = r#"{"version":1,"deviceId":"d","files":{
            "A/B/01 Song.flac":{"trackId":"1","source":"s","format":"original","size":3},
            "../../Users/x/doc.docx":{"trackId":"2","source":"s","format":"original","size":3},
            "C:/Windows/x.mp3":{"trackId":"3","source":"s","format":"original","size":3},
            "A/./x.mp3":{"trackId":"4","source":"s","format":"original","size":3},
            "A\\..\\x.mp3":{"trackId":"5","source":"s","format":"original","size":3},
            "/abs.mp3":{"trackId":"6","source":"s","format":"original","size":3}},
            "playlists":{"Mix.m3u8":"h","../evil.m3u8":"h"}}"#;
        let m = Manifest::parse(json.as_bytes()).expect("parses");
        assert_eq!(m.files.keys().collect::<Vec<_>>(), vec!["A/B/01 Song.flac"]);
        assert_eq!(m.playlists.keys().collect::<Vec<_>>(), vec!["Mix.m3u8"]);
        assert!(safe_rel("Artist/Album/01 Title.flac"));
        assert!(!safe_rel(""));
        assert!(!safe_rel("a//b"));
    }
}
