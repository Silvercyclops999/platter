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
    pub fn read(bytes: &[u8]) -> Result<Self, Unreadable> {
        // The version first, so a newer layout isn't taken for damage.
        #[derive(Deserialize)]
        struct Head {
            version: u32,
        }
        if let Ok(h) = serde_json::from_slice::<Head>(bytes)
            && h.version > VERSION
        {
            return Err(Unreadable::Newer);
        }
        let mut m: Self = serde_json::from_slice(bytes).map_err(|_| Unreadable::Damaged)?;
        m.files.retain(|k, _| safe_rel(k));
        m.playlists.retain(|k, _| safe_rel(k));
        Ok(m)
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        Self::read(bytes).ok()
    }

    /// Folds in the journal of songs written since the manifest was last
    /// saved. A torn last line (the drive pulled mid-write) is skipped.
    /// Returns how many songs it added.
    pub fn fold_journal(&mut self, bytes: &[u8]) -> usize {
        let mut n = 0;
        for line in bytes.split(|b| *b == b'\n') {
            let Ok(j) = serde_json::from_slice::<JournalLine>(line) else { continue };
            if safe_rel(&j.rel) {
                self.files.insert(j.rel, j.entry);
                n += 1;
            }
        }
        n
    }
}

/// Why a manifest on the device couldn't be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreadable {
    /// Not a manifest any more (cut short, edited, garbled).
    Damaged,
    /// Written by a newer Platter.
    Newer,
}

/// The journal next to the manifest: one line per song written since the
/// manifest was last saved, so nothing written goes untracked if the drive
/// is pulled out or the computer stops.
pub const JOURNAL: &str = ".platter-sync.journal";

#[derive(Serialize, Deserialize)]
struct JournalLine {
    rel: String,
    entry: Entry,
}

/// One journal line for a song just written.
pub fn journal_line(rel: &str, entry: &Entry) -> Vec<u8> {
    let mut v = serde_json::to_vec(&JournalLine { rel: rel.to_string(), entry: entry.clone() }).unwrap_or_default();
    v.push(b'\n');
    v
}

impl Manifest {

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
        assert_eq!(Manifest::read(b"{\"version\":1,\"devi").unwrap_err(), Unreadable::Damaged);
        assert_eq!(Manifest::read(br#"{"version":99,"deviceId":"x","files":"new"}"#).unwrap_err(), Unreadable::Newer);
    }

    #[test]
    fn journals_fold_in_and_skip_torn_lines() {
        let mut m = Manifest::new("d1");
        let e = Entry { track_id: "t1".into(), source: "s".into(), format: DeviceFormat::Flac, size: 5 };
        let mut j = journal_line("A/B/01 C.flac", &e);
        j.extend(journal_line("../escape.flac", &e));
        j.extend_from_slice(br#"{"rel":"A/B/02 D.flac","entry":{"trackId":"t2","sou"#);
        assert_eq!(m.fold_journal(&j), 1);
        assert_eq!(m.files.get("A/B/01 C.flac"), Some(&e));
        assert_eq!(m.files.len(), 1);
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
