//! Upgrades against a faked Lidarr, a faked Plex and a "music share" in a
//! temporary folder, with real FLAC and MP3 files. Nothing here reaches a
//! real Lidarr, Plex, MusicBrainz or share.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use lofty::config::WriteOptions;
use lofty::id3::v2::{Frame, Id3v2Tag, PrivateFrame};
use lofty::ogg::OggPictureStorage;
use lofty::ogg::tag::VorbisComments;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, TagExt};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::check::{PlexFile, PlexFiles};
use super::mbpick::tests::{ARTIST, FakeMb, RG_FIRST};
use super::request::RequestInput;
use super::request_tests::{NoMbid, Mbid};
use super::status::RequestState;
use super::swap::{self, Failure, LocalFs, LoftyTags, Planned, ShareFs, SwapHost, TagIo};
use super::upgrade::{Hosts, OfferStatus, PlexFolders, UpgradeDeps, Upgrades};
use super::{Lidarr, LidarrDeps, PlexMusic};
use crate::artistinfo::http::HttpResponse;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::musicshare::{FolderPair, ShareMap};
use crate::plex::keychain::MemoryStore;
use crate::tags::FileTags;
use crate::tags::carry::AudioInfo;
use crate::web::{Body, Method, Web, WebRequest};

const R1: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const R2: &str = "aaaaaaaa-0000-4000-8000-000000000002";
const STAGING: &str = "/staging/upgrades";
const LIB_ROOT: &str = "/downloads/lidarr";
const ALBUM_DIR: &str = "/data/music/Northlight/First Light";

// ---------------------------------------------------------------- audio files

/// A FLAC file with no audio frames: the stream header says 44.1 kHz stereo at `bits`.
fn flac_bytes(bits: u64) -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    v.extend([0x80, 0, 0, 34]); // last block, STREAMINFO, 34 bytes
    v.extend([0x10, 0x00, 0x10, 0x00]); // block sizes 4096
    v.extend([0, 0, 0, 0, 0, 0]); // frame sizes unknown
    let packed: u64 = (44_100u64 << 44) | (1 << 41) | ((bits - 1) << 36) | 441_000;
    v.extend(packed.to_be_bytes());
    v.extend([0u8; 16]); // MD5
    v.extend(vec![0u8; 4096]);
    v
}

/// MPEG-1 layer III frames at `kbps` (128 or 320), 44.1 kHz.
fn mp3_bytes(kbps: u32) -> Vec<u8> {
    let (index, len) = match kbps {
        320 => (0xE0u8, 1044usize),
        _ => (0x90u8, 417usize),
    };
    let mut v = Vec::new();
    for _ in 0..40 {
        let mut frame = vec![0u8; len];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, index, 0x00]);
        v.extend(frame);
    }
    v
}

fn cover(byte: u8) -> Picture {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(vec![byte; 64]);
    Picture::unchecked(png).pic_type(PictureType::CoverFront).mime_type(MimeType::Png).build()
}

/// An MP3 with ID3 tags from `f`.
fn write_mp3(path: &Path, kbps: u32, f: impl FnOnce(&mut Id3v2Tag)) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, mp3_bytes(kbps)).unwrap();
    let mut t = Id3v2Tag::new();
    f(&mut t);
    t.save_to_path(path, WriteOptions::default()).unwrap();
}

/// A FLAC with these Vorbis comments.
fn write_flac(path: &Path, bits: u64, pairs: &[(&str, &str)], picture: Option<Picture>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, flac_bytes(bits)).unwrap();
    let mut vc = VorbisComments::new();
    for (k, v) in pairs {
        vc.push(k.to_string(), v.to_string());
    }
    if let Some(p) = picture {
        vc.insert_picture(p, None).unwrap();
    }
    vc.save_to_path(path, WriteOptions::default()).unwrap();
}

/// A tagged old MP3 as Picard and the archive would leave it.
fn old_mp3(path: &Path, title: &str, recording: &str, archive: Option<&str>) {
    write_mp3(path, 320, |t| {
        t.set_title(title.into());
        t.set_artist("Northlight".into());
        t.set_album("First Light".into());
        t.set_track(1);
        t.insert_user_text("MusicBrainz Track Id".into(), recording.into());
        t.insert_user_text("ALBUMARTIST".into(), "Northlight".into());
        t.insert_user_text("RELEASETYPE".into(), "album".into());
        t.insert_user_text("REPLAYGAIN_TRACK_GAIN".into(), "-7.10 dB".into());
        if let Some(v) = archive {
            t.insert_user_text("ARCHIVE_VERSION".into(), v.into());
        }
        t.insert_picture(cover(7));
    });
}

/// The better copy as Lidarr imported it: its own tags, its own ReplayGain.
fn new_flac(path: &Path, title: &str, recording: &str) {
    write_flac(
        path,
        16,
        &[("TITLE", &format!("{title} (Remastered)")), ("MUSICBRAINZ_TRACKID", recording), ("REPLAYGAIN_TRACK_GAIN", "-6.20 dB"), ("COMMENT", "lidarr")],
        Some(cover(9)),
    );
}

fn hash(p: &Path) -> String {
    LocalFs.hash(p).unwrap()
}

// ---------------------------------------------------------------- carrying tags

#[test]
fn mp3_tags_carry_onto_flac_with_archive_tags_and_the_new_copy_keeps_its_technical_tags() {
    let dir = tempfile::tempdir().unwrap();
    let (old, new) = (dir.path().join("01 Dawn.mp3"), dir.path().join("01 Dawn.flac"));
    old_mp3(&old, "Dawn", R1, Some("alternate"));
    write_mp3(&old, 320, |t| {
        t.set_title("Dawn".into());
        t.set_album("First Light".into());
        t.insert_user_text("MusicBrainz Track Id".into(), R1.into());
        t.insert_user_text("ARCHIVE_VERSION".into(), "alternate".into());
        t.insert_user_text("ARCHIVE_ERA".into(), "First Light era".into());
        t.insert_user_text("ARCHIVE_GROUP".into(), "first-light".into());
        t.insert_user_text("PLEX_MOOD".into(), "Calm".into());
        t.insert_user_text("REPLAYGAIN_TRACK_GAIN".into(), "-7.10 dB".into());
        t.insert_picture(cover(7));
    });
    new_flac(&new, "Dawn", R1);
    crate::tags::carry::carry(&old, &new).unwrap();

    let got = crate::tags::carry::read(&new).unwrap();
    let get = |k: &str| got.pairs.iter().filter(|(n, _)| n == k).map(|(_, v)| v.as_str()).collect::<Vec<_>>();
    assert_eq!(get("TITLE"), vec!["Dawn"], "the old title, not Lidarr's");
    assert_eq!(get("ALBUM"), vec!["First Light"]);
    assert_eq!(get("ARCHIVE_VERSION"), vec!["alternate"]);
    assert_eq!(get("ARCHIVE_ERA"), vec!["First Light era"]);
    assert_eq!(get("ARCHIVE_GROUP"), vec!["first-light"]);
    assert_eq!(get("PLEX_MOOD"), vec!["Calm"]);
    assert_eq!(get("REPLAYGAIN_TRACK_GAIN"), vec!["-6.20 dB"], "the new copy's own ReplayGain");
    assert!(get("COMMENT").is_empty(), "Lidarr's tags don't stay");
    assert_eq!(got.pictures.len(), 1);
    assert_eq!(got.pictures[0].data(), cover(7).data(), "the old cover");
    // The library's own reader sees the same archive tags and ids.
    let (a, b) = (crate::tags::read_local(&old).unwrap(), crate::tags::read_local(&new).unwrap());
    assert_eq!((a.version.as_deref(), a.era.as_deref(), a.group.as_deref()), (Some("alternate"), Some("First Light era"), Some("first-light")));
    assert_eq!((a.version, a.era, a.group, a.recording_mbid, a.album), (b.version, b.era, b.group, b.recording_mbid, b.album));
    // Still the new audio.
    let audio = crate::tags::carry::audio(&new).unwrap();
    assert_eq!((audio.codec.as_str(), audio.bit_depth), ("flac", Some(16)));
}

#[test]
fn flac_to_flac_carries_everything_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let (old, new) = (dir.path().join("a.flac"), dir.path().join("b.flac"));
    let pairs = [("TITLE", "Noon"), ("ARTIST", "Northlight"), ("ARCHIVE_EDITION", "Deluxe"), ("ARCHIVE_FLAGS", "fanedit"), ("GENRE", "Rock"), ("GENRE", "Folk")];
    write_flac(&old, 16, &pairs, Some(cover(3)));
    write_flac(&new, 24, &[("TITLE", "x"), ("ENCODER", "new encoder")], None);
    crate::tags::carry::carry(&old, &new).unwrap();
    let got = crate::tags::carry::read(&new).unwrap();
    for (k, v) in pairs {
        assert!(got.pairs.contains(&(k.to_string(), v.to_string())), "{k}={v}");
    }
    assert!(got.pairs.contains(&("ENCODER".to_string(), "new encoder".to_string())));
    assert!(!got.pairs.contains(&("TITLE".to_string(), "x".to_string())));
    assert_eq!(got.pictures.len(), 1);
}

#[test]
fn tags_only_mp3_can_hold_stop_the_copy_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let (old, new) = (dir.path().join("a.mp3"), dir.path().join("b.flac"));
    write_mp3(&old, 320, |t| {
        t.set_title("Dawn".into());
        t.insert(Frame::Private(PrivateFrame::new("com.example.player", vec![1u8, 2, 3])));
    });
    new_flac(&new, "Dawn", R1);
    let before = hash(&new);
    let e = crate::tags::carry::carry(&old, &new).unwrap_err().to_string();
    assert!(e.contains("PRIV") && e.contains("can't move to FLAC"), "{e}");
    assert_eq!(hash(&new), before, "nothing written");
}

// ---------------------------------------------------------------- the swap, with faults

#[derive(Default)]
struct Faults {
    /// The share doesn't answer.
    unreachable: bool,
    /// File names in use by another program.
    locked: HashSet<String>,
    backup_unwritable: bool,
    carry_fails: bool,
    /// Renames onto these names fail.
    rename_fails: HashSet<String>,
}

struct FaultFs {
    share: PathBuf,
    backup: PathBuf,
    faults: Arc<Mutex<Faults>>,
}

impl FaultFs {
    fn on_share(&self, p: &Path) -> bool {
        p.starts_with(&self.share)
    }
}

impl ShareFs for FaultFs {
    fn exists(&self, p: &Path) -> bool {
        !(self.faults.lock().unreachable && self.on_share(p)) && LocalFs.exists(p)
    }
    fn is_dir(&self, p: &Path) -> bool {
        !(self.faults.lock().unreachable && self.on_share(p)) && LocalFs.is_dir(p)
    }
    fn list(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        LocalFs.list(dir)
    }
    fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
        if self.faults.lock().backup_unwritable && to.starts_with(&self.backup) {
            return Err(io::Error::other("read only"));
        }
        LocalFs.copy(from, to)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let f = self.faults.lock();
        if f.backup_unwritable && to.starts_with(&self.backup) {
            return Err(io::Error::other("read only"));
        }
        if f.rename_fails.contains(&swap::name(to)) {
            return Err(io::Error::other("no"));
        }
        drop(f);
        LocalFs.rename(from, to)
    }
    fn remove(&self, p: &Path) -> io::Result<()> {
        LocalFs.remove(p)
    }
    fn create_dir_all(&self, p: &Path) -> io::Result<()> {
        LocalFs.create_dir_all(p)
    }
    fn hash(&self, p: &Path) -> io::Result<String> {
        LocalFs.hash(p)
    }
    fn check_unlocked(&self, p: &Path) -> io::Result<()> {
        if self.faults.lock().locked.contains(&swap::name(p)) {
            return Err(io::Error::other("in use"));
        }
        LocalFs.check_unlocked(p)
    }
    fn probe_writable(&self, dir: &Path) -> io::Result<()> {
        if self.faults.lock().backup_unwritable {
            return Err(io::Error::other("read only"));
        }
        LocalFs.probe_writable(dir)
    }
}

struct FaultTags(Arc<Mutex<Faults>>);

impl TagIo for FaultTags {
    fn read(&self, p: &Path) -> AppResult<FileTags> {
        LoftyTags.read(p)
    }
    fn audio(&self, p: &Path) -> AppResult<AudioInfo> {
        LoftyTags.audio(p)
    }
    fn carry(&self, old: &Path, new: &Path) -> AppResult<()> {
        if self.0.lock().carry_fails {
            return Err(AppError::Invalid("The tags couldn't be copied: the disk said no.".into()));
        }
        LoftyTags.carry(old, new)
    }
}

/// A share, a staging folder and a backup folder in one temporary folder.
struct TestHost {
    map: Mutex<ShareMap>,
    root: PathBuf,
    fs: FaultFs,
    tags: FaultTags,
}

impl SwapHost for TestHost {
    fn label(&self) -> String {
        "this computer".into()
    }
    fn music_path(&self, plex_path: &str) -> Option<PathBuf> {
        self.map.lock().music_path(plex_path)
    }
    fn staging_path(&self, lidarr_path: &str) -> Option<PathBuf> {
        self.map.lock().staging_path(STAGING, lidarr_path)
    }
    fn backup_root(&self) -> PathBuf {
        self.root.join("backup")
    }
    fn fs(&self) -> &dyn ShareFs {
        &self.fs
    }
    fn tags(&self) -> &dyn TagIo {
        &self.tags
    }
}

struct Box_ {
    _dir: tempfile::TempDir,
    host: Arc<TestHost>,
    faults: Arc<Mutex<Faults>>,
}

impl Box_ {
    fn share(&self, rel: &str) -> PathBuf {
        self.host.root.join("share").join(rel)
    }
    fn staging(&self, rel: &str) -> PathBuf {
        self.host.root.join("staging").join(rel)
    }
}

/// The album on the share: "01 Dawn.mp3" (released, recording R1) and
/// "02 Noon.mp3" (ARCHIVE_VERSION), with the better copies in staging.
fn share_box() -> Box_ {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let faults = Arc::new(Mutex::new(Faults::default()));
    let map = ShareMap {
        music: vec![FolderPair { server: "/data/music".into(), local: root.join("share").to_string_lossy().into_owned() }],
        staging: root.join("staging").to_string_lossy().into_owned(),
        backup: String::new(),
    };
    let host = Arc::new(TestHost {
        map: Mutex::new(map),
        fs: FaultFs { share: root.join("share"), backup: root.join("backup"), faults: faults.clone() },
        tags: FaultTags(faults.clone()),
        root,
    });
    let b = Box_ { _dir: dir, host, faults };
    old_mp3(&b.share("Northlight/First Light/01 Dawn.mp3"), "Dawn", R1, None);
    old_mp3(&b.share("Northlight/First Light/02 Noon.mp3"), "Noon", R2, Some("released"));
    new_flac(&b.staging("Northlight/First Light (2015)/01 - Dawn.flac"), "Dawn", R1);
    new_flac(&b.staging("Northlight/First Light (2015)/02 - Noon.flac"), "Noon", R2);
    b
}

fn dawn() -> Planned {
    Planned {
        track_id: "t1".into(),
        plex_path: format!("{ALBUM_DIR}/01 Dawn.mp3"),
        lidarr_path: format!("{STAGING}/Northlight/First Light (2015)/01 - Dawn.flac"),
    }
}

fn noon() -> Planned {
    Planned {
        track_id: "t2".into(),
        plex_path: format!("{ALBUM_DIR}/02 Noon.mp3"),
        lidarr_path: format!("{STAGING}/Northlight/First Light (2015)/02 - Noon.flac"),
    }
}

/// Every file of the album and of staging, by hash, plus no leftovers.
fn snapshot(b: &Box_) -> Vec<(String, String)> {
    let mut v = Vec::new();
    for d in [b.share("Northlight/First Light"), b.staging("Northlight/First Light (2015)")] {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            v.push((swap::name(&p), hash(&p)));
        }
    }
    v.sort();
    v
}

#[test]
fn a_swap_backs_up_the_old_file_and_undo_puts_it_back_byte_for_byte() {
    let b = share_box();
    let old = b.share("Northlight/First Light/01 Dawn.mp3");
    let before = std::fs::read(&old).unwrap();
    let staged = hash(&b.staging("Northlight/First Light (2015)/01 - Dawn.flac"));
    let backup = swap::backup_folder(&b.host.backup_root(), "Northlight", "First Light", "abcd1234-x");
    let done = swap::run(b.host.as_ref(), &backup, &[dawn()]).unwrap();
    assert_eq!(done.len(), 1);
    let placed = b.share("Northlight/First Light/01 Dawn.flac");
    assert!(placed.exists() && !old.exists(), "the new copy takes the old name, with its own extension");
    assert_eq!(std::fs::read(&done[0].backup).unwrap(), before, "the old file is in the backup folder, untouched");
    assert!(done[0].backup.contains("Northlight - First Light (abcd1234)"));
    assert_eq!(crate::tags::read_local(&placed).unwrap().recording_mbid.as_deref(), Some(R1));
    assert_eq!(hash(&b.staging("Northlight/First Light (2015)/01 - Dawn.flac")), staged, "staging is only read");
    let leftovers: Vec<String> =
        std::fs::read_dir(b.share("Northlight/First Light")).unwrap().map(|e| swap::name(&e.unwrap().path())).filter(|n| n.starts_with('.')).collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    swap::undo(b.host.as_ref(), &done).unwrap();
    assert_eq!(std::fs::read(&old).unwrap(), before, "byte for byte");
    assert!(!placed.exists());
    let replaced = backup.join("Replaced").join("01 Dawn.flac");
    assert!(replaced.exists(), "the new copy is kept, not deleted");
}

/// Run the swap with a fault and check that nothing changed.
fn fails_cleanly(set: impl FnOnce(&mut Faults), items: &[Planned]) -> Failure {
    let b = share_box();
    set(&mut b.faults.lock());
    let before = snapshot(&b);
    let backup = b.host.backup_root().join("x");
    let f = swap::run(b.host.as_ref(), &backup, items).unwrap_err();
    b.faults.lock().unreachable = false;
    assert_eq!(snapshot(&b), before, "{f:?} left every file as it was");
    f
}

#[test]
fn every_failure_leaves_the_old_files_in_place() {
    assert!(matches!(fails_cleanly(|f| f.unreachable = true, &[dawn()]), Failure::ShareUnreachable { .. }));
    assert_eq!(fails_cleanly(|f| { f.locked.insert("01 Dawn.mp3".into()); }, &[dawn()]), Failure::FileLocked { name: "01 Dawn.mp3".into() });
    assert!(matches!(fails_cleanly(|f| f.carry_fails = true, &[dawn()]), Failure::TagCopyFailed { .. }));
    assert!(matches!(fails_cleanly(|f| f.backup_unwritable = true, &[dawn()]), Failure::BackupNotWritable { .. }));
    // The second file fails after the first was swapped: the first goes back too.
    assert!(matches!(fails_cleanly(|f| { f.rename_fails.insert("02 Noon.flac".into()); }, &[dawn(), noon()]), Failure::MoveFailed { .. }));
    let f = fails_cleanly(|f| { f.locked.insert("02 Noon.mp3".into()); }, &[dawn(), noon()]);
    assert_eq!(f, Failure::FileLocked { name: "02 Noon.mp3".into() });
    for f in [Failure::ShareUnreachable { path: "x".into() }, Failure::BackupNotWritable { path: "y".into() }, Failure::RescanFailed] {
        assert!(!f.message().is_empty());
    }
}

#[test]
fn nothing_is_written_without_the_share_mapping() {
    let b = share_box();
    b.host.map.lock().music.clear();
    let before = snapshot(&b);
    assert_eq!(swap::run(b.host.as_ref(), &b.host.backup_root(), &[dawn()]).unwrap_err(), Failure::NoMapping);
    assert_eq!(snapshot(&b), before);
}

#[test]
fn a_new_name_that_is_taken_stops_the_swap() {
    let b = share_box();
    std::fs::write(b.share("Northlight/First Light/01 Dawn.flac"), b"someone else's").unwrap();
    let before = snapshot(&b);
    let f = swap::run(b.host.as_ref(), &b.host.backup_root(), &[dawn()]).unwrap_err();
    assert_eq!(f, Failure::NameTaken { name: "01 Dawn.flac".into() });
    assert_eq!(snapshot(&b), before);
}

#[test]
fn undo_refuses_when_the_kept_copy_is_gone() {
    let b = share_box();
    let done = swap::run(b.host.as_ref(), &b.host.backup_root().join("u"), &[dawn()]).unwrap();
    std::fs::remove_file(&done[0].backup).unwrap();
    assert!(matches!(swap::undo(b.host.as_ref(), &done).unwrap_err(), Failure::BackupMissing { .. }));
    assert!(b.share("Northlight/First Light/01 Dawn.flac").exists(), "the new copy stays put");
}

// ---------------------------------------------------------------- the whole flow

#[derive(Default)]
struct State {
    artists: Vec<Value>,
    albums: Vec<Value>,
    next_id: i64,
    profiles: Value,
    /// The staging folder is one of Lidarr's root folders.
    staging_root: bool,
    /// An artist Lidarr already has, at this path.
    existing_artist: Option<String>,
}

struct FakeLidarr {
    st: Mutex<State>,
    log: Mutex<Vec<(Method, String, String)>>,
}

fn profiles() -> Value {
    json!([
        {"id": 1, "name": "Standard", "upgradeAllowed": false, "cutoff": 4, "items": [
            {"quality": {"id": 1, "name": "MP3-192"}, "allowed": true}, {"quality": {"id": 4, "name": "MP3-320"}, "allowed": true}]},
        {"id": 2, "name": "Lossless", "upgradeAllowed": false, "cutoff": 1005, "items": [
            {"quality": {"id": 4, "name": "MP3-320"}, "allowed": false},
            {"id": 1005, "name": "Lossless", "allowed": true, "items": [
                {"quality": {"id": 6, "name": "FLAC"}, "allowed": true}, {"quality": {"id": 7, "name": "ALAC"}, "allowed": true}]}]}
    ])
}

impl FakeLidarr {
    fn new() -> Arc<Self> {
        Arc::new(Self { st: Mutex::new(State { next_id: 100, profiles: profiles(), staging_root: true, ..Default::default() }), log: Mutex::new(vec![]) })
    }

    fn writes(&self) -> Vec<(Method, String, String)> {
        self.log.lock().iter().filter(|(m, _, _)| *m != Method::Get).cloned().collect()
    }

    /// Lidarr finished: the album has its files in staging.
    fn import(&self) {
        let mut st = self.st.lock();
        for a in st.albums.iter_mut() {
            a["statistics"] = json!({"trackFileCount": 2, "trackCount": 2, "percentOfTracks": 100.0});
        }
    }

    fn handle(&self, method: Method, u: &url::Url, body: &str) -> (u16, Value) {
        let mut st = self.st.lock();
        let path = u.path().trim_start_matches("/api/v1");
        let q = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.to_string());
        let root = |id: i64, p: &str| json!({"id": id, "path": p, "accessible": true, "unmappedFolders": [{"name": "Elsewhere", "path": format!("{p}/Elsewhere")}],
            "defaultQualityProfileId": 1, "defaultMetadataProfileId": 3});
        match (method, path) {
            (Method::Get, "/system/status") => (200, json!({"appName": "Lidarr", "version": "2.8.2.4493"})),
            (Method::Get, "/rootfolder") => {
                // The staging root has the lower id: plain requests must still skip it.
                let mut v = vec![root(2, LIB_ROOT)];
                if st.staging_root {
                    v.insert(0, root(1, STAGING));
                }
                (200, Value::Array(v))
            }
            (Method::Get, "/artist") => (200, Value::Array(st.artists.clone())),
            (Method::Get, "/config/naming") => (200, json!({"renameTracks": false})),
            (Method::Get, "/config/metadataprovider") => (200, json!({"writeAudioTags": "no"})),
            (Method::Get, "/config/mediamanagement") => (200, json!({"deleteEmptyFolders": false, "recycleBin": "/recycle"})),
            (Method::Get, "/qualityprofile") => (200, st.profiles.clone()),
            (Method::Get, "/importlist") => (200, json!([])),
            (Method::Get, "/metadataprofile") => (200, json!([{"id": 3, "name": "Standard"}])),
            (Method::Get, "/filesystem") => (200, json!({"directories": [], "files": []})),
            (Method::Get, "/album/lookup") => {
                let rgid = q("term").unwrap_or_default().trim_start_matches("lidarr:").to_string();
                let artist = match &st.existing_artist {
                    Some(p) => json!({"artistName": "Northlight", "foreignArtistId": ARTIST, "folder": "Northlight", "path": p}),
                    None => json!({"artistName": "Northlight", "foreignArtistId": ARTIST, "folder": "Northlight"}),
                };
                (200, json!([{"id": 0, "title": "First Light", "foreignAlbumId": rgid, "artist": artist}]))
            }
            (Method::Get, "/album") => {
                if let Some(f) = q("foreignAlbumId") {
                    return (200, Value::Array(st.albums.iter().filter(|a| a["foreignAlbumId"] == f.as_str()).cloned().collect()));
                }
                let aid: i64 = q("artistId").and_then(|a| a.parse().ok()).unwrap_or(-1);
                (200, Value::Array(st.albums.iter().filter(|a| a["artistId"] == aid).cloned().collect()))
            }
            (Method::Get, p) if p.starts_with("/artist/") => {
                let id: i64 = p.trim_start_matches("/artist/").parse().unwrap_or(-1);
                (200, st.artists.iter().find(|a| a["id"] == id).cloned().unwrap_or(json!({})))
            }
            (Method::Get, "/queue/details") => (200, json!([])),
            (Method::Get, "/history") => (200, json!({"records": []})),
            (Method::Get, "/track") => (
                200,
                json!([
                    {"id": 1, "title": "Dawn", "foreignRecordingId": R1, "mediumNumber": 1, "absoluteTrackNumber": 1, "duration": 200000, "trackFileId": 11},
                    {"id": 2, "title": "Noon", "foreignRecordingId": R2, "mediumNumber": 1, "absoluteTrackNumber": 2, "duration": 180000, "trackFileId": 12}
                ]),
            ),
            (Method::Get, "/trackfile") => (
                200,
                json!([
                    {"id": 11, "path": format!("{STAGING}/Northlight/First Light (2015)/01 - Dawn.flac")},
                    {"id": 12, "path": format!("{STAGING}/Northlight/First Light (2015)/02 - Noon.flac")}
                ]),
            ),
            (Method::Post, "/artist") => {
                let b: Value = serde_json::from_str(body).unwrap();
                st.next_id += 1;
                let id = st.next_id;
                let artist = json!({"id": id, "artistName": b["artistName"], "foreignArtistId": b["foreignArtistId"],
                    "path": format!("{}/Northlight", b["rootFolderPath"].as_str().unwrap()), "qualityProfileId": b["qualityProfileId"],
                    "monitored": true, "monitorNewItems": b["monitorNewItems"]});
                st.artists.push(artist.clone());
                for (rg, title) in [(RG_FIRST, "First Light"), ("10000000-0000-0000-0000-000000000002", "Second Wind")] {
                    st.next_id += 1;
                    let nid = st.next_id;
                    st.albums.push(json!({"id": nid, "title": title, "foreignAlbumId": rg, "artistId": id, "monitored": false,
                        "statistics": {"trackFileCount": 0, "trackCount": 2, "percentOfTracks": 0}}));
                }
                (201, artist)
            }
            (Method::Put, "/album/monitor") => {
                let b: Value = serde_json::from_str(body).unwrap();
                for id in b["albumIds"].as_array().unwrap() {
                    if let Some(a) = st.albums.iter_mut().find(|a| a["id"] == *id) {
                        a["monitored"] = b["monitored"].clone();
                    }
                }
                (202, json!([]))
            }
            (Method::Post, "/command") => (201, json!({"id": 1})),
            _ => (405, json!({})),
        }
    }
}

#[async_trait]
impl Web for FakeLidarr {
    async fn send(&self, req: WebRequest) -> AppResult<HttpResponse> {
        let body = match &req.body {
            Body::Json(j) => j.clone(),
            _ => String::new(),
        };
        let u = url::Url::parse(&req.url).unwrap();
        self.log.lock().push((req.method, format!("{}{}", u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default()), body.clone()));
        let (status, v) = self.handle(req.method, &u, &body);
        Ok(HttpResponse { status, body: Bytes::from(v.to_string()), ..Default::default() })
    }
}

struct Plex;

#[async_trait]
impl PlexMusic for Plex {
    async fn music_files(&self) -> AppResult<PlexFiles> {
        Ok(PlexFiles { locations: vec!["/data/music".into()], files: vec![PlexFile { path: format!("{ALBUM_DIR}/01 Dawn.mp3"), size: Some(41_760) }] })
    }
}

#[derive(Default)]
struct FakeRescan {
    scanned: Mutex<Vec<String>>,
    down: Mutex<bool>,
    scan_fails: Mutex<bool>,
}

#[async_trait]
impl PlexFolders for FakeRescan {
    async fn can_scan(&self, _folders: &[String]) -> AppResult<()> {
        if *self.down.lock() { Err(AppError::Network("down".into())) } else { Ok(()) }
    }
    async fn scan(&self, folder: &str) -> AppResult<()> {
        if *self.scan_fails.lock() {
            return Err(AppError::Server("no".into()));
        }
        self.scanned.lock().push(folder.to_string());
        Ok(())
    }
}

struct TestHosts(Arc<TestHost>, Mutex<bool>);

impl Hosts for TestHosts {
    fn host(&self) -> Option<Arc<dyn SwapHost>> {
        (*self.1.lock()).then(|| self.0.clone() as Arc<dyn SwapHost>)
    }
}

struct Rig {
    b: Box_,
    db: DbHandle,
    web: Arc<FakeLidarr>,
    plex: Arc<FakeRescan>,
    hosts: Arc<TestHosts>,
    up: Upgrades,
    lidarr: Lidarr,
}

fn rig_with(mb: Arc<FakeMb>) -> Rig {
    let b = share_box();
    let db = DbHandle::open(b.host.root.join("u.db")).unwrap();
    let s = crate::settings::Settings { lidarr_url: "http://10.20.30.40:8686".into(), lidarr_staging_folder: STAGING.into(), ..Default::default() };
    crate::settings::save(&db, &s).unwrap();
    db.write(|c| {
        c.execute_batch(&format!(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('a1', 'First Light', 'First Light', 'ar1', 'Northlight', 'Album', '[]', 1),
               ('a2', 'Quiet Sketches (Build 2)', 'Quiet Sketches', 'ar1', 'Northlight', NULL, '[]', 0);
             INSERT INTO album_tags (album_id, version) VALUES ('a2', 'build');
             INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, disc_number, duration_ms, part_key, codec, bitrate, file) VALUES
               ('t1', 'Dawn', 'a1', 'First Light', 'ar1', 'Northlight', 1, 1, 200000, '/p/1', 'mp3', 320, '{ALBUM_DIR}/01 Dawn.mp3'),
               ('t2', 'Noon', 'a1', 'First Light', 'ar1', 'Northlight', 2, 1, 180000, '/p/2', 'mp3', 320, '{ALBUM_DIR}/02 Noon.mp3');"
        ))?;
        Ok(())
    })
    .unwrap();
    mb.route(
        "/release-group?query=releasegroup%3A%22first+light%22",
        Some(json!({"release-groups": [{"id": RG_FIRST, "title": "First Light", "primary-type": "Album", "first-release-date": "2015-03-01"}]})),
    );
    let web = FakeLidarr::new();
    let lidarr = Lidarr::new(LidarrDeps {
        web: web.clone(),
        keys: Arc::new(MemoryStore::new()),
        db: db.clone(),
        plex: Arc::new(Plex),
        clock: Arc::new(|| 1_800_000_000),
        mb,
        artist_mbid: Arc::new(Mbid),
        album_wait: std::time::Duration::ZERO,
    });
    lidarr.set_key(Some("abc123")).unwrap();
    let plex = Arc::new(FakeRescan::default());
    let hosts = Arc::new(TestHosts(b.host.clone(), Mutex::new(true)));
    let up = Upgrades::new(UpgradeDeps { lidarr: lidarr.clone(), db: db.clone(), hosts: hosts.clone(), plex: plex.clone(), clock: Arc::new(|| 1_800_000_000) });
    Rig { b, db, web, plex, hosts, up, lidarr }
}

fn rig() -> Rig {
    rig_with(FakeMb::northlight())
}

fn assert_only_allowed_writes(web: &FakeLidarr) {
    for (m, p, _) in web.log.lock().iter() {
        let ok = match m {
            Method::Get => p.starts_with("/api/v1/"),
            Method::Post => p == "/api/v1/artist" || p == "/api/v1/command",
            Method::Put => p == "/api/v1/album/monitor",
        };
        assert!(ok, "Lidarr was sent {m:?} {p}");
    }
}

#[tokio::test]
async fn an_upgrade_goes_only_to_staging_with_monitoring_limited_and_a_profile_aiming_higher() {
    let r = rig();
    let rec = r.up.request("a1").await.unwrap();
    assert_eq!(rec.request.state, RequestState::Searching, "{rec:?}");
    assert_eq!(rec.request.release_group_id, RG_FIRST);
    assert_eq!(rec.request.destination.as_deref(), Some("/staging/upgrades/Northlight"));
    assert_eq!(rec.from_quality, "MP3 320");
    let writes = r.web.writes();
    let shape: Vec<(Method, &str)> = writes.iter().map(|(m, p, _)| (*m, p.as_str())).collect();
    assert_eq!(shape, vec![(Method::Post, "/api/v1/artist"), (Method::Put, "/api/v1/album/monitor"), (Method::Post, "/api/v1/command")]);
    let add: Value = serde_json::from_str(&writes[0].2).unwrap();
    assert_eq!(add["rootFolderPath"], STAGING, "only into staging");
    assert_eq!(add["monitorNewItems"], "none");
    assert_eq!(add["addOptions"], json!({"monitor": "none", "searchForMissingAlbums": false}));
    assert_eq!(add["qualityProfileId"], 2, "Lossless: its cutoff is above MP3 320 (Standard's isn't)");
    let on: Vec<String> = r.web.st.lock().albums.iter().filter(|a| a["monitored"] == true).map(|a| a["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(on, vec!["First Light"]);
    assert!(rec.request.notes.iter().any(|n| n.contains("FLAC or better with its Lossless profile")), "{:?}", rec.request.notes);
    assert_only_allowed_writes(&r.web);
    // The setup check ran before the first write.
    let log = r.web.log.lock().clone();
    let first = log.iter().position(|(m, _, _)| *m != Method::Get).unwrap();
    assert!(log[..first].iter().any(|(_, p, _)| p == "/api/v1/rootfolder"));
    // Asking again sends nothing.
    let n = r.web.writes().len();
    r.up.request("a1").await.unwrap();
    assert_eq!(r.web.writes().len(), n);
}

#[tokio::test]
async fn no_profile_aiming_higher_means_nothing_is_sent() {
    let r = rig();
    r.web.st.lock().profiles = json!([profiles()[0].clone()]);
    let e = r.up.request("a1").await.unwrap_err().to_string();
    assert!(e.contains("None of Lidarr's quality profiles aims higher than MP3 320"), "{e}");
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn an_artist_lidarr_keeps_outside_staging_is_refused() {
    let r = rig();
    r.web.st.lock().artists.push(json!({"id": 7, "artistName": "Northlight", "foreignArtistId": ARTIST, "path": format!("{LIB_ROOT}/Northlight"), "qualityProfileId": 2}));
    let e = r.up.request("a1").await.unwrap_err().to_string();
    assert!(e.contains("outside the staging folder"), "{e}");
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn plain_requests_never_land_in_staging() {
    let r = rig();
    let input = RequestInput {
        release_group_id: "10000000-0000-0000-0000-000000000002".into(),
        title: "Second Wind".into(),
        artist: "Northlight".into(),
        artist_id: Some("ar1".into()),
        typed: Some("Second Wind".into()),
        ..Default::default()
    };
    let rec = r.lidarr.request(input).await.unwrap();
    assert_eq!(rec.destination.as_deref(), Some("/downloads/lidarr/Northlight"));
    let add: Value = serde_json::from_str(&r.web.writes()[0].2).unwrap();
    assert_eq!(add["rootFolderPath"], LIB_ROOT);
}

#[tokio::test]
async fn unreleased_albums_and_a_missing_staging_root_are_refused() {
    let r = rig();
    let e = r.up.request("a2").await.unwrap_err().to_string();
    assert!(e.contains("counts as unreleased"), "{e}");
    // Staging isn't one of Lidarr's root folders: the setup check is red.
    r.web.st.lock().staging_root = false;
    let e = r.up.request("a1").await.unwrap_err().to_string();
    assert!(e.contains("red items") && e.contains("isn't one of Lidarr's root folders"), "{e}");
    assert!(r.web.writes().is_empty());
    // Without a staging folder there are no upgrades at all.
    let s = crate::settings::Settings { lidarr_staging_folder: String::new(), ..crate::settings::load(&r.db) };
    crate::settings::save(&r.db, &s).unwrap();
    assert!(r.up.request("a1").await.unwrap_err().to_string().contains("staging folder"));
    assert!(!r.up.for_album("a1").await.unwrap().available);
}

#[tokio::test]
async fn a_title_clash_with_another_album_refuses_the_upgrade() {
    let r = rig();
    r.db.write(|c| {
        c.execute_batch(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('a3', 'First Light [Leak]', 'First Light', 'ar1', 'Northlight', NULL, '[]', 0);",
        )?;
        Ok(())
    })
    .unwrap();
    let e = r.up.request("a1").await.unwrap_err().to_string();
    assert!(e.contains("same title as \"First Light [Leak]\""), "{e}");
    assert!(r.web.writes().is_empty());
}

/// Requested, imported by Lidarr, and offered.
async fn imported(r: &Rig) -> String {
    let rec = r.up.request("a1").await.unwrap();
    r.web.import();
    let list = r.up.refresh().await.unwrap();
    assert_eq!(list[0].request.state, RequestState::Imported);
    rec.request.id
}

#[tokio::test]
async fn the_offer_says_what_upgrades_and_what_is_kept_and_why() {
    let r = rig();
    let id = imported(&r).await;
    let o = r.up.offer(&id).await.unwrap();
    assert_eq!(o.status, OfferStatus::Ready, "{o:?}");
    assert_eq!(o.summary.as_deref(), Some("Upgrade 1 of 2 songs (MP3 320 to FLAC)"));
    assert_eq!(o.upgrade.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), vec!["Dawn"]);
    assert_eq!(o.upgrade[0].by.as_deref(), Some("recordingId"));
    assert_eq!(o.kept.iter().map(|s| (s.title.as_str(), s.reason.as_deref())).collect::<Vec<_>>(), vec![("Noon", Some("Has archive tags"))]);
    assert!(o.ask_each, "a kept archive file makes the album ask per song");
    assert!(o.ask_reason.as_deref().unwrap().contains("asks about each song"));
    // The folder's other files are listed too.
    std::fs::write(r.b.share("Northlight/First Light/03 Leak.mp3"), mp3_bytes(128)).unwrap();
    let o = r.up.offer(&id).await.unwrap();
    assert!(o.kept.iter().any(|s| s.title == "03 Leak.mp3" && s.reason.as_deref() == Some("Not one of this album's songs in Plex")));
    // Without the mapping there's nothing to show yet.
    *r.hosts.1.lock() = false;
    assert_eq!(r.up.offer(&id).await.unwrap().status, OfferStatus::NeedsMapping);
}

#[tokio::test]
async fn swapping_asks_per_song_rescans_the_folder_and_undoes_exactly() {
    let r = rig();
    let id = imported(&r).await;
    let old = r.b.share("Northlight/First Light/01 Dawn.mp3");
    let before = std::fs::read(&old).unwrap();
    // The album holds a kept archive file: no swapping everything at once.
    let e = r.up.swap(&id, None).await.unwrap_err().to_string();
    assert!(e.contains("asks about each song"), "{e}");
    // A kept song can't be picked.
    assert!(r.up.swap(&id, Some(vec!["t2".into()])).await.unwrap_err().to_string().contains("Pick at least one"));
    let writes_before = r.web.writes().len();
    let res = r.up.swap(&id, Some(vec!["t1".into()])).await.unwrap();
    assert_eq!(res.swapped, 1);
    assert_eq!(res.summary, "Upgraded 1 song (MP3 320 to FLAC)");
    assert!(r.b.share("Northlight/First Light/01 Dawn.flac").exists() && !old.exists());
    assert!(r.b.share("Northlight/First Light/02 Noon.mp3").exists(), "the kept file is untouched");
    assert_eq!(*r.plex.scanned.lock(), vec![ALBUM_DIR.to_string()], "just that folder");
    assert_eq!(r.web.writes().len(), writes_before, "never through Lidarr");
    let o = r.up.offer(&id).await.unwrap();
    assert_eq!(o.status, OfferStatus::Swapped);
    assert!(o.last_swap.as_ref().unwrap().backup_dir.contains("Northlight - First Light"));

    let rec = r.up.undo(&id).await.unwrap();
    assert!(rec.swaps[0].undone);
    assert_eq!(std::fs::read(&old).unwrap(), before, "byte for byte");
    assert!(!r.b.share("Northlight/First Light/01 Dawn.flac").exists());
    assert_eq!(r.plex.scanned.lock().len(), 2, "rescanned after the undo too");
    assert!(r.up.undo(&id).await.is_err(), "nothing left to undo");
}

#[tokio::test]
async fn a_failed_rescan_puts_the_old_files_back() {
    let r = rig();
    let id = imported(&r).await;
    let before = snapshot(&r.b);
    *r.plex.scan_fails.lock() = true;
    let e = r.up.swap(&id, Some(vec!["t1".into()])).await.unwrap_err().to_string();
    assert!(e.contains("Plex didn't take the request to rescan"), "{e}");
    assert_eq!(snapshot(&r.b), before);
    assert_eq!(r.up.records()[0].failure, Some(Failure::RescanFailed));
    // Plex unreachable before the swap: nothing changes at all.
    *r.plex.scan_fails.lock() = false;
    *r.plex.down.lock() = true;
    let e = r.up.swap(&id, Some(vec!["t1".into()])).await.unwrap_err().to_string();
    assert!(e.contains("changed nothing"), "{e}");
    assert_eq!(snapshot(&r.b), before);
    assert_eq!(r.up.records()[0].failure, Some(Failure::PlexUnreachable));
}

#[tokio::test]
async fn a_locked_file_or_failed_tag_copy_is_recorded_and_changes_nothing() {
    let r = rig();
    let id = imported(&r).await;
    let before = snapshot(&r.b);
    r.b.faults.lock().locked.insert("01 Dawn.mp3".into());
    let e = r.up.swap(&id, Some(vec!["t1".into()])).await.unwrap_err().to_string();
    assert!(e.contains("in use by another program"), "{e}");
    let o = r.up.offer(&id).await.unwrap();
    assert_eq!(o.failure, Some(Failure::FileLocked { name: "01 Dawn.mp3".into() }));
    assert!(o.failure_message.unwrap().contains("Nothing was changed"));
    r.b.faults.lock().locked.clear();
    r.b.faults.lock().carry_fails = true;
    assert!(r.up.swap(&id, Some(vec!["t1".into()])).await.unwrap_err().to_string().contains("tags couldn't be copied"));
    r.b.faults.lock().carry_fails = false;
    r.b.faults.lock().backup_unwritable = true;
    assert!(r.up.swap(&id, Some(vec!["t1".into()])).await.unwrap_err().to_string().contains("backup folder"));
    assert_eq!(snapshot(&r.b), before);
}

#[tokio::test]
async fn with_asking_off_a_clean_album_swaps_on_its_own_and_a_mixed_one_waits() {
    let r = rig();
    let s = crate::settings::Settings { upgrades_ask: false, ..crate::settings::load(&r.db) };
    crate::settings::save(&r.db, &s).unwrap();
    let id = imported(&r).await;
    assert_eq!(r.up.auto_swap().await, 0, "the archive-tagged file makes it wait for a person");
    assert!(r.b.share("Northlight/First Light/01 Dawn.mp3").exists());
    // Without the archive file, the same rules let it swap on its own.
    std::fs::remove_file(r.b.share("Northlight/First Light/02 Noon.mp3")).unwrap();
    r.db.write(|c| {
        c.execute("DELETE FROM tracks WHERE id = 't2'", [])?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r.up.auto_swap().await, 1);
    assert!(r.b.share("Northlight/First Light/01 Dawn.flac").exists());
    assert!(r.up.offer(&id).await.unwrap().last_swap.is_some());
    // Asking on: nothing swaps on its own.
    let r2 = rig();
    imported(&r2).await;
    std::fs::remove_file(r2.b.share("Northlight/First Light/02 Noon.mp3")).unwrap();
    r2.db.write(|c| {
        c.execute("DELETE FROM tracks WHERE id = 't2'", [])?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r2.up.auto_swap().await, 0);
}

#[tokio::test]
async fn the_album_menu_shows_the_action_only_when_it_can_run() {
    let r = rig();
    // No setup check yet: hidden.
    assert!(!r.up.for_album("a1").await.unwrap().available);
    r.lidarr.check().await.unwrap();
    let a = r.up.for_album("a1").await.unwrap();
    assert!(a.available, "{a:?}");
    assert_eq!(a.quality.as_deref(), Some("MP3 320"));
    assert!(!r.up.for_album("a2").await.unwrap().available, "never for an unreleased album");
    let _ = NoMbid;
}
