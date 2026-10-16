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
use super::mbpick::tests::{ARTIST, FakeMb, RG_EP, RG_FIRST, RG_LIVE};
use super::request::RequestInput;
use super::request_tests::{NoMbid, Mbid};
use super::status::RequestState;
use super::swap::{self, BackupPlace, Failure, LocalFs, LoftyTags, NoteEdit, Planned, ShareFs, SwapHost, TagIo};
use super::upgrade::{Hosts, OfferStatus, PlexFolders, UpgradeDeps, Upgrades};
use super::provenance::ArchiveStatus;
use super::{Lidarr, LidarrDeps, PlexMusic};
use crate::artistinfo::http::HttpResponse;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::musicshare::{FolderPair, ShareMap};
use crate::plex::keychain::MemoryStore;
use crate::tags::FileTags;
use super::tagcarry::AudioInfo;
use crate::web::{Body, Method, Web, WebRequest};

const R1: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const R2: &str = "aaaaaaaa-0000-4000-8000-000000000002";
const STAGING: &str = "/staging/upgrades";
const LIB_ROOT: &str = "/downloads/lidarr";
const ALBUM_DIR: &str = "/data/music/Northlight/First Light";

// ---------------------------------------------------------------- audio files

/// A FLAC file with no audio frames: the stream header says 44.1 kHz
/// stereo at `bits`, `ms` long.
fn flac_bytes(bits: u64, ms: u64) -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    v.extend([0x80, 0, 0, 34]); // last block, STREAMINFO, 34 bytes
    v.extend([0x10, 0x00, 0x10, 0x00]); // block sizes 4096
    v.extend([0, 0, 0, 0, 0, 0]); // frame sizes unknown
    let packed: u64 = (44_100u64 << 44) | (1 << 41) | ((bits - 1) << 36) | (44_100 * ms / 1000);
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
    write_flac_ms(path, bits, 200_000, pairs, picture)
}

fn write_flac_ms(path: &Path, bits: u64, ms: u64, pairs: &[(&str, &str)], picture: Option<Picture>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, flac_bytes(bits, ms)).unwrap();
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
    old_mp3_with(path, title, recording, archive, None, None)
}

fn old_mp3_with(path: &Path, title: &str, recording: &str, archive: Option<&str>, isrc: Option<&str>, source: Option<&str>) {
    write_mp3(path, 320, |t| {
        if let Some(i) = isrc {
            t.insert_user_text("ISRC".into(), i.into());
        }
        if let Some(s) = source {
            t.insert_user_text("ARCHIVE_SOURCE".into(), s.into());
        }
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
    new_flac_with(path, title, recording, None, 200_000)
}

fn new_flac_with(path: &Path, title: &str, recording: &str, isrc: Option<&str>, ms: u64) {
    let t = format!("{title} (Remastered)");
    let mut pairs = vec![("TITLE", t.as_str()), ("MUSICBRAINZ_TRACKID", recording), ("REPLAYGAIN_TRACK_GAIN", "-6.20 dB"), ("COMMENT", "lidarr")];
    if let Some(i) = isrc {
        pairs.push(("ISRC", i));
    }
    write_flac_ms(path, 16, ms, &pairs, Some(cover(9)));
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
    super::tagcarry::carry(&old, &new).unwrap();

    let got = super::tagcarry::read(&new).unwrap();
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
    let (a, b) = (super::provenance::read_local(&old).unwrap(), super::provenance::read_local(&new).unwrap());
    assert_eq!((a.version.as_deref(), a.era.as_deref(), a.group.as_deref()), (Some("alternate"), Some("First Light era"), Some("first-light")));
    assert_eq!((a.version, a.era, a.group, a.recording_mbid, a.album), (b.version, b.era, b.group, b.recording_mbid, b.album));
    // Still the new audio.
    let audio = super::tagcarry::audio(&new).unwrap();
    assert_eq!((audio.codec.as_str(), audio.bit_depth), ("flac", Some(16)));
}

#[test]
fn flac_to_flac_carries_everything_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let (old, new) = (dir.path().join("a.flac"), dir.path().join("b.flac"));
    let pairs = [("TITLE", "Noon"), ("ARTIST", "Northlight"), ("ARCHIVE_EDITION", "Deluxe"), ("ARCHIVE_FLAGS", "fanedit"), ("GENRE", "Rock"), ("GENRE", "Folk")];
    write_flac(&old, 16, &pairs, Some(cover(3)));
    write_flac(&new, 24, &[("TITLE", "x"), ("ENCODER", "new encoder")], None);
    super::tagcarry::carry(&old, &new).unwrap();
    let got = super::tagcarry::read(&new).unwrap();
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
    let e = super::tagcarry::carry(&old, &new).unwrap_err().to_string();
    assert!(e.contains("PRIV") && e.contains("can't move to FLAC"), "{e}");
    assert_eq!(hash(&new), before, "nothing written");
}

#[test]
fn the_provenance_note_can_be_set_on_any_format() {
    let dir = tempfile::tempdir().unwrap();
    let (flac, mp3) = (dir.path().join("a.flac"), dir.path().join("a.mp3"));
    write_flac(&flac, 16, &[("TITLE", "Dawn"), ("ARCHIVE_SOURCE", "Official CD")], Some(cover(1)));
    old_mp3_with(&mp3, "Dawn", R1, Some("released"), None, Some("Official CD"));
    for p in [&flac, &mp3] {
        super::tagcarry::set_text(p, "ARCHIVE_SOURCE", "Official CD; Upgraded to FLAC from Lidarr, 2026-10-16").unwrap();
        let t = super::provenance::read_local(p).unwrap();
        assert_eq!(t.source.as_deref(), Some("Official CD; Upgraded to FLAC from Lidarr, 2026-10-16"));
        let bag = super::tagcarry::read(p).unwrap();
        assert!(bag.pairs.iter().any(|(k, v)| k == "TITLE" && v == "Dawn"), "everything else stays");
        assert_eq!(bag.pictures.len(), 1);
    }
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
    note_fails: bool,
    /// Renames onto these names fail.
    rename_fails: HashSet<String>,
    /// What the audio comparison says (None: it fails).
    correlation: Option<f64>,
}

struct FaultFs {
    share: PathBuf,
    faults: Arc<Mutex<Faults>>,
}

impl FaultFs {
    fn on_share(&self, p: &Path) -> bool {
        p.starts_with(&self.share)
    }
    fn backup(&self, p: &Path) -> bool {
        p.components().any(|c| c.as_os_str() == swap::TO_DELETE)
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
        if self.faults.lock().backup_unwritable && self.backup(to) {
            return Err(io::Error::other("read only"));
        }
        LocalFs.copy(from, to)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let f = self.faults.lock();
        if f.backup_unwritable && self.backup(to) {
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
    fn read_text(&self, p: &Path) -> io::Result<Option<String>> {
        LocalFs.read_text(p)
    }
    fn write_text(&self, p: &Path, text: &str) -> io::Result<()> {
        LocalFs.write_text(p, text)
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
    fn set_text(&self, p: &Path, key: &str, value: &str) -> AppResult<()> {
        if self.0.lock().note_fails {
            return Err(AppError::Invalid("The tags couldn't be copied: the note didn't take.".into()));
        }
        LoftyTags.set_text(p, key, value)
    }
    fn correlate(&self, _a: &Path, _b: &Path) -> AppResult<f64> {
        self.0.lock().correlation.ok_or_else(|| AppError::Invalid("no".into()))
    }
}

/// A share, a staging folder and an outside folder in one temporary folder.
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
    fn backup_place(&self, old: &Path) -> Result<BackupPlace, Failure> {
        swap::backup_place(&self.map.lock(), old)
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

const ISRC_DAWN: &str = "USRC17607839";

/// The album on the share: "01 Dawn.mp3" (an ISRC it shares with the new
/// copy), "02 Noon.mp3" (released by tag, an official source note, no ISRC)
/// and "03 Sketch.mp3" (a demo), with the better copies in staging.
fn share_box() -> Box_ {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let faults = Arc::new(Mutex::new(Faults { correlation: Some(0.998), ..Default::default() }));
    let map = ShareMap {
        music: vec![FolderPair { server: "/data/music".into(), local: root.join("share").to_string_lossy().into_owned() }],
        staging: root.join("staging").to_string_lossy().into_owned(),
        backup: String::new(),
    };
    let host = Arc::new(TestHost {
        map: Mutex::new(map),
        fs: FaultFs { share: root.join("share"), faults: faults.clone() },
        tags: FaultTags(faults.clone()),
        root,
    });
    let b = Box_ { _dir: dir, host, faults };
    old_mp3_with(&b.share("Northlight/First Light/01 Dawn.mp3"), "Dawn", R1, None, Some(ISRC_DAWN), None);
    old_mp3_with(&b.share("Northlight/First Light/02 Noon.mp3"), "Noon", R2, Some("released"), None, Some("Official CD, 2015"));
    old_mp3_with(&b.share("Northlight/First Light/03 Sketch.mp3"), "Sketch", R2, Some("demo"), None, Some("Leak"));
    new_flac_with(&b.staging("Northlight/First Light (2015)/01 - Dawn.flac"), "Dawn", R1, Some(ISRC_DAWN), 200_000);
    new_flac_with(&b.staging("Northlight/First Light (2015)/02 - Noon.flac"), "Noon", R2, None, 180_300);
    b
}

fn dawn() -> Planned {
    Planned {
        track_id: "t1".into(),
        plex_path: format!("{ALBUM_DIR}/01 Dawn.mp3"),
        lidarr_path: format!("{STAGING}/Northlight/First Light (2015)/01 - Dawn.flac"),
        note: None,
    }
}

fn noon() -> Planned {
    Planned {
        track_id: "t2".into(),
        plex_path: format!("{ALBUM_DIR}/02 Noon.mp3"),
        lidarr_path: format!("{STAGING}/Northlight/First Light (2015)/02 - Noon.flac"),
        note: Some(NoteEdit { key: "ARCHIVE_SOURCE".into(), value: "Official CD, 2015; Upgraded to FLAC from Lidarr, 2026-10-16".into() }),
    }
}

/// Every file of the album and of staging, by hash.
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
fn a_swap_backs_up_into_an_ignored_folder_and_undo_puts_it_back_byte_for_byte() {
    let b = share_box();
    let old = b.share("Northlight/First Light/02 Noon.mp3");
    let before = std::fs::read(&old).unwrap();
    let staged = hash(&b.staging("Northlight/First Light (2015)/02 - Noon.flac"));
    let ran = swap::run(b.host.as_ref(), &swap::backup_folder("Northlight", "First Light", "abcd1234-x"), &[noon()]).unwrap();
    let done = ran.done;
    assert_eq!(done.len(), 1);
    let placed = b.share("Northlight/First Light/02 Noon.flac");
    assert!(placed.exists() && !old.exists(), "the new copy takes the old name, with its own extension");
    assert_eq!(std::fs::read(&done[0].backup).unwrap(), before, "the old file is in the backup folder, untouched");
    let backup = PathBuf::from(&done[0].backup);
    assert!(backup.starts_with(b.share("_to_delete/Northlight - First Light (abcd1234)")), "{}", backup.display());
    // Plex is told to ignore it, once.
    assert_eq!(ran.ignored, vec![b.share("")]);
    let rule = std::fs::read_to_string(b.share(".plexignore")).unwrap();
    assert!(rule.lines().any(|l| l == swap::IGNORE_RULE), "{rule}");
    // The note says where the copy came from; every other tag carried over.
    let t = super::provenance::read_local(&placed).unwrap();
    assert_eq!(t.source.as_deref(), Some("Official CD, 2015; Upgraded to FLAC from Lidarr, 2026-10-16"));
    assert_eq!((t.version.as_deref(), t.recording_mbid.as_deref()), (Some("released"), Some(R2)));
    assert_eq!(hash(&b.staging("Northlight/First Light (2015)/02 - Noon.flac")), staged, "staging is only read");
    let leftovers: Vec<String> =
        std::fs::read_dir(b.share("Northlight/First Light")).unwrap().map(|e| swap::name(&e.unwrap().path())).filter(|n| n.starts_with('.')).collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    swap::undo(b.host.as_ref(), &done).unwrap();
    assert_eq!(std::fs::read(&old).unwrap(), before, "byte for byte");
    assert!(!placed.exists());
    assert!(b.share("_to_delete/Northlight - First Light (abcd1234)/Replaced/02 Noon.flac").exists(), "the new copy is kept, not deleted");

    // A second swap finds the rule already there.
    let ran = swap::run(b.host.as_ref(), "again", &[dawn()]).unwrap();
    assert!(ran.ignored.is_empty());
    assert_eq!(std::fs::read_to_string(b.share(".plexignore")).unwrap(), rule);
}

#[test]
fn an_existing_plexignore_keeps_its_rules() {
    let b = share_box();
    std::fs::write(b.share(".plexignore"), "Scans/*").unwrap();
    swap::run(b.host.as_ref(), "x", &[dawn()]).unwrap();
    let rule = std::fs::read_to_string(b.share(".plexignore")).unwrap();
    assert!(rule.starts_with("Scans/*\n") && rule.contains(swap::IGNORE_RULE), "{rule}");
}

#[test]
fn a_chosen_backup_folder_must_be_outside_the_music_folder() {
    let b = share_box();
    let before = snapshot(&b);
    b.host.map.lock().backup = b.share("Old files").to_string_lossy().into_owned();
    assert!(matches!(swap::run(b.host.as_ref(), "x", &[dawn()]).unwrap_err(), Failure::BackupInLibrary { .. }));
    assert_eq!(snapshot(&b), before);
    // Outside it: used as is, with no .plexignore needed.
    let outside = b.host.root.join("backups");
    b.host.map.lock().backup = outside.to_string_lossy().into_owned();
    let ran = swap::run(b.host.as_ref(), "x", &[dawn()]).unwrap();
    assert!(PathBuf::from(&ran.done[0].backup).starts_with(&outside));
    assert!(ran.ignored.is_empty() && !b.share(".plexignore").exists());
}

/// Run the swap with a fault and check that nothing changed.
fn fails_cleanly(set: impl FnOnce(&mut Faults), items: &[Planned]) -> Failure {
    let b = share_box();
    set(&mut b.faults.lock());
    let before = snapshot(&b);
    let f = swap::run(b.host.as_ref(), "x", items).unwrap_err();
    b.faults.lock().unreachable = false;
    assert_eq!(snapshot(&b), before, "{f:?} left every file as it was");
    f
}

#[test]
fn every_failure_leaves_the_old_files_in_place() {
    assert!(matches!(fails_cleanly(|f| f.unreachable = true, &[dawn()]), Failure::ShareUnreachable { .. }));
    assert_eq!(fails_cleanly(|f| { f.locked.insert("01 Dawn.mp3".into()); }, &[dawn()]), Failure::FileLocked { name: "01 Dawn.mp3".into() });
    assert!(matches!(fails_cleanly(|f| f.carry_fails = true, &[dawn()]), Failure::TagCopyFailed { .. }));
    assert!(matches!(fails_cleanly(|f| f.note_fails = true, &[noon()]), Failure::TagCopyFailed { .. }));
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
    assert_eq!(swap::run(b.host.as_ref(), "x", &[dawn()]).unwrap_err(), Failure::NoMapping);
    assert_eq!(snapshot(&b), before);
    assert!(!b.share(".plexignore").exists());
}

#[test]
fn a_new_name_that_is_taken_stops_the_swap() {
    let b = share_box();
    std::fs::write(b.share("Northlight/First Light/01 Dawn.flac"), b"someone else's").unwrap();
    let before = snapshot(&b);
    let f = swap::run(b.host.as_ref(), "x", &[dawn()]).unwrap_err();
    assert_eq!(f, Failure::NameTaken { name: "01 Dawn.flac".into() });
    assert_eq!(snapshot(&b), before);
}

#[test]
fn undo_refuses_when_the_kept_copy_is_gone() {
    let b = share_box();
    let done = swap::run(b.host.as_ref(), "u", &[dawn()]).unwrap().done;
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
    /// Lidarr's import lists exclude these artist ids.
    exclusions: Vec<String>,
    /// Lidarr puts added artists here instead of the root asked for.
    misplace_artists: Option<String>,
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
            (Method::Get, "/importlistexclusion") => {
                (200, Value::Array(st.exclusions.iter().map(|id| json!({"foreignId": id, "artistName": "Northlight"})).collect()))
            }
            (Method::Get, "/metadataprofile") => (200, json!([{"id": 3, "name": "Standard"}])),
            (Method::Get, "/filesystem") => (200, json!({"directories": [], "files": []})),
            (Method::Get, "/album/lookup") => {
                let rgid = q("term").unwrap_or_default().trim_start_matches("lidarr:").to_string();
                (200, json!([{"id": 0, "title": "First Light", "foreignAlbumId": rgid,
                    "artist": {"artistName": "Northlight", "foreignArtistId": ARTIST, "folder": "Northlight"}}]))
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
            (Method::Get, "/trackfile") if q("unmapped").is_some() => (200, json!([])),
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
                let folder = st.misplace_artists.clone().unwrap_or_else(|| b["rootFolderPath"].as_str().unwrap().to_string());
                let artist = json!({"id": id, "artistName": b["artistName"], "foreignArtistId": b["foreignArtistId"],
                    "path": format!("{folder}/Northlight"), "qualityProfileId": b["qualityProfileId"],
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
    mb: Arc<FakeMb>,
    plex: Arc<FakeRescan>,
    hosts: Arc<TestHosts>,
    up: Upgrades,
    lidarr: Lidarr,
}

/// The official release's tracklist on the faked MusicBrainz.
fn first_light_releases() -> Value {
    json!({"releases": [
        {"id": "rel-w", "status": "Withdrawn", "date": "2014-01-01", "media": [{"position": 1, "tracks": [{"position": 1, "title": "Wrong", "length": 1}]}]},
        {"id": "rel-1", "status": "Official", "date": "2015-03-01", "media": [{"position": 1, "tracks": [
            {"position": 1, "title": "Dawn", "length": 200000}, {"position": 2, "title": "Noon", "length": 180000}]}]}
    ]})
}

fn rig() -> Rig {
    let b = share_box();
    let db = DbHandle::open(b.host.root.join("u.db")).unwrap();
    let s = crate::settings::Settings { lidarr_url: "http://10.20.30.40:8686".into(), lidarr_staging_folder: STAGING.into(), ..Default::default() };
    crate::settings::save(&db, &s).unwrap();
    db.write(|c| {
        c.execute_batch(&format!(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('a1', 'First Light', 'First Light', 'ar1', 'Northlight', 'Album', '[]', 1),
               ('a2', 'Quiet Sketches (Build 2)', 'Quiet Sketches', 'ar1', 'Northlight', NULL, '[]', 0),
               ('a3', 'First Light [Leak]', 'First Light', 'ar1', 'Northlight', NULL, '[]', 0);
             INSERT INTO album_tags (album_id, version) VALUES ('a2', 'build');
             INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, disc_number, duration_ms, part_key, codec, bitrate, file) VALUES
               ('t1', 'Dawn', 'a1', 'First Light', 'ar1', 'Northlight', 1, 1, 200000, '/p/1', 'mp3', 320, '{ALBUM_DIR}/01 Dawn.mp3'),
               ('t2', 'Noon', 'a1', 'First Light', 'ar1', 'Northlight', 2, 1, 180000, '/p/2', 'mp3', 320, '{ALBUM_DIR}/02 Noon.mp3'),
               ('t3', 'Sketch', 'a1', 'First Light', 'ar1', 'Northlight', 3, 1, 150000, '/p/3', 'mp3', 320, '{ALBUM_DIR}/03 Sketch.mp3');"
        ))?;
        Ok(())
    })
    .unwrap();
    let mb = FakeMb::northlight();
    mb.route(
        "/release-group?query=releasegroup%3A%22first+light%22",
        Some(json!({"release-groups": [
            {"id": RG_FIRST, "title": "First Light", "primary-type": "Album", "first-release-date": "2015-03-01"},
            {"id": RG_LIVE, "title": "First Light", "primary-type": "Album", "first-release-date": "2022"}
        ]})),
    );
    mb.route(&format!("/release?release-group={RG_FIRST}"), Some(first_light_releases()));
    let web = FakeLidarr::new();
    let lidarr = Lidarr::new(LidarrDeps {
        web: web.clone(),
        keys: Arc::new(MemoryStore::new()),
        db: db.clone(),
        plex: Arc::new(Plex),
        clock: Arc::new(|| 1_800_000_000),
        mb: mb.clone(),
        artist_mbid: Arc::new(Mbid),
        album_wait: std::time::Duration::ZERO,
    });
    lidarr.set_key(Some("abc123")).unwrap();
    let plex = Arc::new(FakeRescan::default());
    let hosts = Arc::new(TestHosts(b.host.clone(), Mutex::new(true)));
    let up = Upgrades::new(UpgradeDeps {
        lidarr: lidarr.clone(),
        db: db.clone(),
        hosts: hosts.clone(),
        plex: plex.clone(),
        clock: Arc::new(|| 1_800_000_000),
        status: Arc::new(ArchiveStatus),
    });
    Rig { b, db, web, mb, plex, hosts, up, lidarr }
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

/// The user confirms First Light is RG_FIRST.
async fn confirmed(r: &Rig) {
    r.up.confirm("a1", Some(RG_FIRST)).await.unwrap();
}

#[tokio::test]
async fn upgrades_wait_for_the_user_to_confirm_the_release() {
    let r = rig();
    let e = r.up.request("a1").await.unwrap_err().to_string();
    assert!(e.contains("Confirm which MusicBrainz release"), "{e}");
    assert!(r.web.writes().is_empty());
    // The candidates: the studio album with its official tracklist (not the
    // withdrawn release), never the live one; the folder's songs beside it.
    let c = r.up.candidates("a1").await.unwrap();
    assert_eq!(c.candidates.len(), 1);
    assert_eq!(c.candidates[0].group.id, RG_FIRST);
    assert_eq!(c.candidates[0].tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), vec!["Dawn", "Noon"]);
    assert_eq!(c.songs.len(), 3);
    assert!(c.confirmed.is_none());
    // A live group can't be confirmed.
    assert!(r.up.confirm("a1", Some(RG_LIVE)).await.is_err());
    confirmed(&r).await;
    assert_eq!(r.up.for_album("a1").await.unwrap().confirmed.unwrap().release_group_id, RG_FIRST);
    // The leaked build with the same base title doesn't block it any more:
    // the user said which release this is, and every file is proven first.
    assert!(r.up.request("a1").await.is_ok());
    // Forgetting it.
    r.up.confirm("a1", None).await.unwrap();
    assert!(r.up.for_album("a1").await.unwrap().confirmed.is_none());
}

#[tokio::test]
async fn an_upgrade_goes_only_to_staging_with_monitoring_limited_and_a_profile_aiming_higher() {
    let r = rig();
    confirmed(&r).await;
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
    // The setup check ran before the first write, and again before the search.
    let log = r.web.log.lock().clone();
    let first = log.iter().position(|(m, _, _)| *m != Method::Get).unwrap();
    let search = log.iter().position(|(_, p, _)| p == "/api/v1/command").unwrap();
    assert!(log[..first].iter().any(|(_, p, _)| p == "/api/v1/rootfolder"));
    assert!(log[first..search].iter().any(|(_, p, _)| p == "/api/v1/rootfolder"), "checked again after the writes");
    // Asking again sends nothing.
    let n = r.web.writes().len();
    r.up.request("a1").await.unwrap();
    assert_eq!(r.web.writes().len(), n);
}

#[tokio::test]
async fn no_profile_aiming_higher_means_nothing_is_sent() {
    let r = rig();
    confirmed(&r).await;
    r.web.st.lock().profiles = json!([profiles()[0].clone()]);
    let e = r.up.request("a1").await.unwrap_err().to_string();
    assert!(e.contains("None of Lidarr's quality profiles aims higher than MP3 320"), "{e}");
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn an_artist_lidarr_keeps_outside_staging_is_refused() {
    let r = rig();
    confirmed(&r).await;
    r.web.st.lock().artists.push(json!({"id": 7, "artistName": "Northlight", "foreignArtistId": ARTIST, "path": format!("{LIB_ROOT}/Northlight"), "qualityProfileId": 2}));
    let e = r.up.request("a1").await.unwrap_err().to_string();
    assert!(e.contains("outside the staging folder"), "{e}");
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn excluded_artists_are_never_requested_or_added() {
    let r = rig();
    confirmed(&r).await;
    r.web.st.lock().exclusions = vec![ARTIST.to_string()];
    let e = r.up.request("a1").await.unwrap_err().to_string();
    assert!(e.contains("import list exclusions include Northlight"), "{e}");
    let input = RequestInput {
        release_group_id: "10000000-0000-0000-0000-000000000002".into(),
        title: "Second Wind".into(),
        artist: "Northlight".into(),
        artist_id: Some("ar1".into()),
        typed: Some("Second Wind".into()),
        ..Default::default()
    };
    assert!(r.lidarr.request(input).await.unwrap_err().to_string().contains("exclusions"));
    assert!(r.web.writes().is_empty());
    // The setup check says so.
    let report = r.lidarr.check().await.unwrap();
    assert!(report.findings.iter().any(|f| f.id == "importExclusions" && f.detail.contains("Northlight")));
}

#[tokio::test]
async fn an_added_artist_that_lands_in_the_plex_folder_stops_everything() {
    let r = rig();
    confirmed(&r).await;
    r.web.st.lock().misplace_artists = Some("/data/music".into());
    let rec = r.up.request("a1").await.unwrap();
    assert_eq!(rec.request.state, RequestState::Failed);
    assert!(rec.request.message.as_deref().unwrap().contains("inside your Plex music folder"), "{:?}", rec.request.message);
    let shape: Vec<String> = r.web.writes().iter().map(|(_, p, _)| p.clone()).collect();
    assert_eq!(shape, vec!["/api/v1/artist"], "nothing marked for download, nothing searched");
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
async fn an_allowlist_limits_what_an_artist_can_get() {
    let r = rig();
    r.lidarr.set_allowlist("Northlight", Some("ar1"), vec![RG_EP.into()]).await.unwrap();
    assert_eq!(r.lidarr.allowlist("Northlight", Some("ar1")).await, vec![RG_EP.to_string()]);
    let input = |rg: &str, t: &str| RequestInput {
        release_group_id: rg.into(),
        title: t.into(),
        artist: "Northlight".into(),
        artist_id: Some("ar1".into()),
        typed: Some(t.into()),
        ..Default::default()
    };
    let e = r.lidarr.request(input("10000000-0000-0000-0000-000000000002", "Second Wind")).await.unwrap_err().to_string();
    assert!(e.contains("limited this artist"), "{e}");
    assert!(r.web.writes().is_empty());
    assert!(r.lidarr.request(input(RG_EP, "Small Hours")).await.is_ok());
    // Upgrades too: First Light isn't on the list.
    assert!(r.up.confirm("a1", Some(RG_FIRST)).await.unwrap_err().to_string().contains("limited this artist"));
    r.lidarr.set_allowlist("Northlight", Some("ar1"), vec![]).await.unwrap();
    assert!(r.lidarr.allowlist("Northlight", Some("ar1")).await.is_empty());
}

#[tokio::test]
async fn unreleased_albums_and_a_missing_staging_root_are_refused() {
    let r = rig();
    let e = r.up.request("a2").await.unwrap_err().to_string();
    assert!(e.contains("counts as unreleased"), "{e}");
    confirmed(&r).await;
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

/// Confirmed, requested, imported by Lidarr, and offered.
async fn imported(r: &Rig) -> String {
    confirmed(r).await;
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
    assert_eq!(o.summary.as_deref(), Some("Upgrade 2 of 3 songs (MP3 320 to FLAC)"));
    let up: Vec<(&str, Option<&str>)> = o.upgrade.iter().map(|s| (s.title.as_str(), s.by.as_deref())).collect();
    assert_eq!(up, vec![("Dawn", Some("isrc")), ("Noon", Some("sound"))]);
    assert_eq!(o.kept.iter().map(|s| (s.title.as_str(), s.reason.as_deref())).collect::<Vec<_>>(), vec![("Sketch", Some("Unreleased"))]);
    assert!(o.ask_each, "a kept unreleased file makes the album ask per song");
    assert!(o.backup_dir.contains("_to_delete"));
    // Without the ISRC, the sound has to match too.
    r.b.faults.lock().correlation = Some(0.97);
    let o = r.up.offer(&id).await.unwrap();
    assert!(o.kept.iter().any(|s| s.title == "Noon" && s.reason.as_deref().unwrap().contains("doesn't sound the same")));
    assert!(o.upgrade.iter().any(|s| s.title == "Dawn"), "the ISRC match doesn't need the sound");
    r.b.faults.lock().correlation = Some(0.998);
    // A different official prefix: the note no longer counts.
    let s = crate::settings::Settings { upgrade_official_prefix: "Retail".into(), ..crate::settings::load(&r.db) };
    crate::settings::save(&r.db, &s).unwrap();
    let o = r.up.offer(&id).await.unwrap();
    assert!(o.kept.iter().any(|s| s.title == "Noon" && s.reason.as_deref().unwrap().contains("source note")));
    // The folder's other files are listed too.
    std::fs::write(r.b.share("Northlight/First Light/04 Leak.mp3"), mp3_bytes(128)).unwrap();
    let o = r.up.offer(&id).await.unwrap();
    assert!(o.kept.iter().any(|s| s.title == "04 Leak.mp3" && s.reason.as_deref() == Some("Not one of this album's songs in Plex")));
    // Without the mapping there's nothing to show yet.
    *r.hosts.1.lock() = false;
    assert_eq!(r.up.offer(&id).await.unwrap().status, OfferStatus::NeedsMapping);
}

#[tokio::test]
async fn swapping_asks_per_song_rescans_the_folder_notes_the_source_and_undoes_exactly() {
    let r = rig();
    let id = imported(&r).await;
    let old = r.b.share("Northlight/First Light/02 Noon.mp3");
    let before = std::fs::read(&old).unwrap();
    // The album holds a kept unreleased file: no swapping everything at once.
    let e = r.up.swap(&id, None).await.unwrap_err().to_string();
    assert!(e.contains("asks about each song"), "{e}");
    // A kept song can't be picked.
    assert!(r.up.swap(&id, Some(vec!["t3".into()])).await.unwrap_err().to_string().contains("Pick at least one"));
    let writes_before = r.web.writes().len();
    let res = r.up.swap(&id, Some(vec!["t2".into()])).await.unwrap();
    assert_eq!(res.swapped, 1);
    assert_eq!(res.summary, "Upgraded 1 song (MP3 320 to FLAC)");
    let placed = r.b.share("Northlight/First Light/02 Noon.flac");
    assert!(placed.exists() && !old.exists());
    let note = super::provenance::read_local(&placed).unwrap().source.unwrap();
    assert_eq!(note, "Official CD, 2015; Upgraded to FLAC from Lidarr, 2027-01-15");
    assert!(r.b.share("Northlight/First Light/03 Sketch.mp3").exists(), "the kept file is untouched");
    assert_eq!(*r.plex.scanned.lock(), vec![ALBUM_DIR.to_string()], "just that folder");
    assert_eq!(r.web.writes().len(), writes_before, "never through Lidarr");
    let rec = r.up.records().into_iter().next().unwrap();
    assert!(rec.request.notes.iter().any(|n| n.contains(".plexignore")), "{:?}", rec.request.notes);
    let o = r.up.offer(&id).await.unwrap();
    assert_eq!(o.status, OfferStatus::Swapped);
    assert!(o.last_swap.as_ref().unwrap().backup_dir.contains("_to_delete"));

    let rec = r.up.undo(&id).await.unwrap();
    assert!(rec.swaps[0].undone);
    assert_eq!(std::fs::read(&old).unwrap(), before, "byte for byte");
    assert!(!placed.exists());
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

/// Take the demo out of the album (file and song).
fn drop_sketch(r: &Rig) {
    std::fs::remove_file(r.b.share("Northlight/First Light/03 Sketch.mp3")).unwrap();
    r.db.write(|c| {
        c.execute("DELETE FROM tracks WHERE id = 't3'", [])?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn with_asking_off_a_clean_album_swaps_on_its_own_and_a_mixed_one_waits() {
    let r = rig();
    let s = crate::settings::Settings { upgrades_ask: false, ..crate::settings::load(&r.db) };
    crate::settings::save(&r.db, &s).unwrap();
    let id = imported(&r).await;
    assert_eq!(r.up.auto_swap().await, 0, "the unreleased file makes it wait for a person");
    assert!(r.b.share("Northlight/First Light/01 Dawn.mp3").exists());
    drop_sketch(&r);
    assert_eq!(r.up.auto_swap().await, 1);
    assert!(r.b.share("Northlight/First Light/01 Dawn.flac").exists());
    assert!(r.b.share("Northlight/First Light/02 Noon.flac").exists());
    assert!(r.up.offer(&id).await.unwrap().last_swap.is_some());
    // Asking on: nothing swaps on its own.
    let r2 = rig();
    imported(&r2).await;
    drop_sketch(&r2);
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
    assert_eq!(super::upgrade::ymd(1_800_000_000), "2027-01-15");
    let _ = (&r.mb, NoMbid);
}
