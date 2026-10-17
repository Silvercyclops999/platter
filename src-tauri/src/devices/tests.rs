//! Syncs to a temp folder standing in for the device: what lands where,
//! re-syncs, other people's files, removing, the size limit, conversion,
//! and each way writing to a device fails.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;

use super::drives::Volume;
use super::io::{DeviceIo, StdIo};
use super::manifest::Manifest;
use super::run::{Deps, Devices, Opts, Pace, Phase, SyncControl, SyncStatus, TrackFiles, Volumes};
use super::*;
use crate::db::DbHandle;
use crate::db::fake;
use crate::db::source::PlaylistEntry;
use crate::db::store::{set_playlist_items, upsert_albums, upsert_playlist, upsert_tracks};
use crate::model::Track;

const RATE: u32 = 44_100;

fn wav(path: &Path, freq: f32, secs: f32) {
    let spec = hound::WavSpec { channels: 2, sample_rate: RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    let n = (RATE as f32 * secs) as u32;
    for i in 0..n {
        let v = ((i as f32 / RATE as f32 * freq * std::f32::consts::TAU).sin() * 12_000.0) as i16;
        w.write_sample(v).unwrap();
        w.write_sample(v).unwrap();
    }
    w.finalize().unwrap();
}

/// Songs as short WAV files. `on_fetch` runs before each one (with how
/// many came before), to fake trouble partway through.
struct FakeFiles {
    fetched: AtomicUsize,
    on_fetch: Box<dyn Fn(usize) + Send + Sync>,
    fail: Option<String>,
    /// This song's fetch panics.
    panic_on: Option<String>,
    /// From this fetch on, each waits until it's told to stop.
    hold_from: Option<usize>,
    /// Fetches running now.
    in_flight: Arc<AtomicUsize>,
    /// Where each song was fetched to.
    dests: Arc<parking_lot::Mutex<Vec<PathBuf>>>,
}

impl FakeFiles {
    fn new() -> Self {
        Self {
            fetched: AtomicUsize::new(0),
            on_fetch: Box::new(|_| {}),
            fail: None,
            panic_on: None,
            hold_from: None,
            in_flight: Arc::default(),
            dests: Arc::default(),
        }
    }
}

struct InFlight(Arc<AtomicUsize>);
impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl TrackFiles for FakeFiles {
    async fn fetch(&self, t: &Track, dest: &Path, stop: &(dyn Fn() -> bool + Sync)) -> Result<(), String> {
        let n = self.fetched.fetch_add(1, Ordering::SeqCst);
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        let _done = InFlight(self.in_flight.clone());
        self.dests.lock().push(dest.to_path_buf());
        (self.on_fetch)(n);
        if self.panic_on.as_deref() == Some(t.id.as_str()) {
            panic!("the fake source fell over");
        }
        if self.fail.as_deref() == Some(t.id.as_str()) {
            return Err("The file isn't on the server any more.".into());
        }
        if self.hold_from.is_some_and(|h| n >= h) {
            let start = std::time::Instant::now();
            while !stop() && start.elapsed() < std::time::Duration::from_secs(60) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            return Err(encode::STOPPED.into());
        }
        let freq = 220.0 + 20.0 * (t.track_number as f32);
        wav(dest, freq, 0.3);
        Ok(())
    }
}

struct NoPace;
impl Pace for NoPace {
    fn busy(&self) -> bool {
        false
    }
}

struct FakeVolumes {
    root: PathBuf,
    removable: bool,
}
impl FakeVolumes {
    fn new(root: PathBuf) -> Self {
        Self { root, removable: true }
    }
}
impl Volumes for FakeVolumes {
    fn list(&self) -> Vec<Volume> {
        if !self.root.is_dir() {
            return vec![];
        }
        vec![Volume {
            root: self.root.clone(),
            label: "PLAYER".into(),
            id: Some("ABCD-1234".into()),
            removable: self.removable,
            free: Some(64 * 1024 * 1024 * 1024),
            total: Some(128 * 1024 * 1024 * 1024),
        }]
    }
}

/// The device's file system, with faults.
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    ReadOnly,
    /// Full once this many songs are on it.
    FullAfter(usize),
    /// Pulled out (once) when this many songs are on it.
    PulledAfter(usize),
}

struct FaultyIo {
    fault: Fault,
    copies: AtomicUsize,
    tripped: AtomicBool,
    /// The drive's top: once it's gone nothing can be written (a pulled-out
    /// drive can't be made again by creating folders).
    root: PathBuf,
    /// Set when the test pulls the drive out.
    pulled: Arc<AtomicBool>,
}

impl FaultyIo {
    fn gone(&self) -> io::Result<()> {
        if self.pulled.load(Ordering::SeqCst) || !self.root.is_dir() { Err(io::Error::from(io::ErrorKind::NotFound)) } else { Ok(()) }
    }
}

impl DeviceIo for FaultyIo {
    fn copy_in(&self, from: &Path, to: &Path, stop: &dyn Fn() -> bool, progress: &mut dyn FnMut(u64)) -> io::Result<u64> {
        self.gone()?;
        match self.fault {
            Fault::ReadOnly => return Err(io::Error::from(io::ErrorKind::ReadOnlyFilesystem)),
            Fault::FullAfter(n) if self.copies.load(Ordering::SeqCst) >= n => return Err(io::Error::from(io::ErrorKind::StorageFull)),
            Fault::PulledAfter(n) if self.copies.load(Ordering::SeqCst) >= n && !self.tripped.swap(true, Ordering::SeqCst) => {
                self.pulled.store(true, Ordering::SeqCst);
                return Err(io::Error::from(io::ErrorKind::NotFound));
            }
            _ => {}
        }
        let r = StdIo.copy_in(from, to, stop, progress);
        if r.is_ok() {
            self.copies.fetch_add(1, Ordering::SeqCst);
        }
        r
    }
    fn write_bytes(&self, to: &Path, bytes: &[u8]) -> io::Result<()> {
        self.gone()?;
        if self.fault == Fault::ReadOnly {
            return Err(io::Error::from(io::ErrorKind::ReadOnlyFilesystem));
        }
        StdIo.write_bytes(to, bytes)
    }
    fn append(&self, to: &Path, bytes: &[u8]) -> io::Result<()> {
        self.gone()?;
        if self.fault == Fault::ReadOnly {
            return Err(io::Error::from(io::ErrorKind::ReadOnlyFilesystem));
        }
        StdIo.append(to, bytes)
    }
    fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        StdIo.read(p)
    }
    fn size(&self, p: &Path) -> Option<u64> {
        StdIo.size(p)
    }
    fn remove(&self, p: &Path) -> io::Result<()> {
        StdIo.remove(p)
    }
    fn prune(&self, dir: &Path, top: &Path) {
        StdIo.prune(dir, top)
    }
    fn is_dir(&self, p: &Path) -> bool {
        if p == self.root && self.pulled.load(Ordering::SeqCst) {
            return false;
        }
        StdIo.is_dir(p)
    }
}

struct World {
    _dir: tempfile::TempDir,
    db: DbHandle,
    root: PathBuf,
    devices: Devices,
    device: Device,
}

fn wav_track(id: &str, title: &str, al: &crate::model::Album, n: u32) -> Track {
    let mut t = fake::track(id, title, al, 1, n);
    t.part_key = format!("/library/parts/{id}/1/file.wav");
    t.codec = Some("pcm".into());
    t.container = Some("wav".into());
    t.bitrate = Some(1411);
    t.duration_ms = 300;
    t
}

fn world_with(files: FakeFiles, fault: Fault) -> World {
    world_pulled(files, fault, Arc::default())
}

fn world_pulled(files: FakeFiles, fault: Fault, pulled: Arc<AtomicBool>) -> World {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("lib.db")).unwrap();
    let ar = fake::artist("ar1", "The Band");
    let al = fake::album("al1", "First: Album", &ar, 2001, None, &[]);
    let al2 = fake::album("al2", "Second", &ar, 2003, None, &[]);
    let tracks = vec![
        wav_track("t1", "Opening", &al, 1),
        wav_track("t2", "Middle?", &al, 2),
        wav_track("t3", "Closing", &al, 3),
        wav_track("t4", "Later", &al2, 1),
    ];
    db.write(|c| {
        let tx = c.transaction()?;
        upsert_albums(&tx, &[al.clone(), al2.clone()], 1)?;
        upsert_tracks(&tx, &tracks, 1)?;
        upsert_playlist(&tx, &fake::playlist("p1", "Road Trip", 2))?;
        set_playlist_items(
            &tx,
            "p1",
            &[
                PlaylistEntry { track_id: "t4".into(), item_id: Some("i1".into()) },
                PlaylistEntry { track_id: "t2".into(), item_id: Some("i2".into()) },
            ],
        )?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let root = dir.path().join("device");
    std::fs::create_dir_all(&root).unwrap();
    let devices = Devices::new(Deps {
        db: db.clone(),
        files: Arc::new(files),
        volumes: Arc::new(FakeVolumes::new(root.clone())),
        io: Arc::new(FaultyIo { fault, copies: AtomicUsize::new(0), tripped: AtomicBool::new(false), root: root.clone(), pulled }),
        pace: Arc::new(NoPace),
        scratch: dir.path().join("scratch"),
    });
    let mut device = devices.add(&root, "Player", "Music").unwrap();
    device = devices
        .update(&device.id, |d| {
            store::set_selected(d, SelKind::Album, "al1", "First: Album", true);
            store::set_selected(d, SelKind::Playlist, "p1", "Road Trip", true);
            Ok(())
        })
        .unwrap();
    World { _dir: dir, db, root, devices, device }
}

fn world() -> World {
    world_with(FakeFiles::new(), Fault::None)
}

fn opts(format: DeviceFormat, delete_removed: bool) -> Opts {
    Opts { default_format: format, delete_removed, liked_min: 10.0 }
}

impl World {
    fn music(&self) -> PathBuf {
        self.root.join("Music")
    }
    fn current(&self) -> Device {
        store::load(&self.db).into_iter().find(|d| d.id == self.device.id).unwrap()
    }
    async fn sync(&self, o: Opts) -> SyncStatus {
        self.devices.sync(self.current(), o, Arc::new(SyncControl::default())).await;
        self.devices.status().unwrap()
    }
    fn manifest(&self) -> Manifest {
        Manifest::parse(&std::fs::read(self.music().join(MANIFEST)).unwrap()).unwrap()
    }
    fn files(&self) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(dir: &Path, top: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, top, out);
                } else {
                    out.push(p.strip_prefix(top).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        if self.music().is_dir() {
            walk(&self.music(), &self.music(), &mut out);
        }
        out.sort();
        out
    }
}

#[tokio::test]
async fn a_first_sync_lays_out_folders_playlists_and_the_manifest() {
    let w = world();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Done, "{:?}", s.error);
    assert!(s.safe_to_unplug);
    assert_eq!((s.total, s.done, s.kept), (4, 4, 0));
    assert_eq!(
        w.files(),
        vec![
            ".platter-sync.json",
            "Road Trip.m3u8",
            "The Band/First_ Album/01 Opening.wav",
            "The Band/First_ Album/02 Middle_.wav",
            "The Band/First_ Album/03 Closing.wav",
            "The Band/Second/01 Later.wav",
        ]
    );
    let m3u = std::fs::read_to_string(w.music().join("Road Trip.m3u8")).unwrap();
    assert_eq!(
        m3u,
        "#EXTM3U\n#EXTINF:1,The Band - Later\nThe Band/Second/01 Later.wav\n#EXTINF:1,The Band - Middle?\nThe Band/First_ Album/02 Middle_.wav\n"
    );
    let m = w.manifest();
    assert_eq!(m.device_id, w.device.id);
    assert_eq!(m.files.len(), 4);
    assert!(w.current().last_sync.is_some());
}

#[tokio::test]
async fn a_re_sync_copies_only_what_changed() {
    let w = world();
    w.sync(opts(DeviceFormat::Original, false)).await;
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!((s.phase, s.total, s.kept), (Phase::Done, 0, 4));

    // A song's file changes on the server: only it is made again.
    w.db.write(|c| {
        c.execute("UPDATE tracks SET part_key = '/library/parts/t3/2/new.wav' WHERE id = 't3'", [])?;
        Ok(())
    })
    .unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!((s.total, s.kept), (1, 3));

    // A song deleted from the device by hand comes back.
    std::fs::remove_file(w.music().join("The Band/Second/01 Later.wav")).unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!((s.total, s.kept), (1, 3));
    assert!(w.music().join("The Band/Second/01 Later.wav").exists());
}

#[tokio::test]
async fn files_platter_did_not_write_are_never_touched() {
    let w = world();
    let mine = w.music().join("The Band/First_ Album/01 Opening.wav");
    std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
    std::fs::write(&mine, b"my own file").unwrap();
    std::fs::write(w.music().join("Road Trip.m3u8"), b"my list").unwrap();
    let other = w.music().join("Podcasts/episode.mp3");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::write(&other, b"podcast").unwrap();

    let s = w.sync(opts(DeviceFormat::Original, true)).await;
    assert_eq!(s.phase, Phase::Done);
    assert_eq!(std::fs::read(&mine).unwrap(), b"my own file");
    assert_eq!(std::fs::read(w.music().join("Road Trip.m3u8")).unwrap(), b"my list");
    assert!(w.music().join("The Band/First_ Album/01 Opening (2).wav").exists());
    assert!(w.music().join("Road Trip (2).m3u8").exists());

    // Removing everything from the selection removes only Platter's files.
    w.devices
        .update(&w.device.id, |d| {
            d.selection.clear();
            Ok(())
        })
        .unwrap();
    let s = w.sync(opts(DeviceFormat::Original, true)).await;
    assert_eq!(s.removed, 4);
    assert_eq!(std::fs::read(&mine).unwrap(), b"my own file");
    assert_eq!(std::fs::read(&other).unwrap(), b"podcast");
    assert_eq!(std::fs::read(w.music().join("Road Trip.m3u8")).unwrap(), b"my list");
    assert!(!w.music().join("Road Trip (2).m3u8").exists());
    assert!(!w.music().join("The Band/Second").exists(), "empty folders go");
    assert!(w.manifest().files.is_empty());
}

#[tokio::test]
async fn removed_songs_stay_unless_removing_is_on() {
    let w = world();
    w.sync(opts(DeviceFormat::Original, false)).await;
    w.devices
        .update(&w.device.id, |d| {
            store::set_selected(d, SelKind::Album, "al1", "", false);
            Ok(())
        })
        .unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!((s.removed, s.kept), (0, 2));
    assert!(w.music().join("The Band/First_ Album/01 Opening.wav").exists());
    assert_eq!(w.manifest().files.len(), 4, "still Platter's, so a later sync can remove them");

    let s = w.sync(opts(DeviceFormat::Original, true)).await;
    assert_eq!(s.removed, 2);
    assert!(!w.music().join("The Band/First_ Album/01 Opening.wav").exists());
    assert!(w.music().join("The Band/First_ Album/02 Middle_.wav").exists(), "still in the playlist");
}

#[tokio::test]
async fn the_size_limit_fills_in_selection_order() {
    let w = world();
    // About two songs' worth (each estimate is about 177 KB).
    w.devices
        .update(&w.device.id, |d| {
            d.budget_gb = Some(400_000.0 / (1024.0 * 1024.0 * 1024.0));
            Ok(())
        })
        .unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Done);
    assert_eq!(s.done, 2);
    assert_eq!(s.left_out_songs, 2);
    assert_eq!(s.left_out.iter().map(|l| (l.title.as_str(), l.songs)).collect::<Vec<_>>(), vec![("First: Album", 1), ("Road Trip", 1)]);
    assert!(w.music().join("The Band/First_ Album/01 Opening.wav").exists());
    assert!(w.music().join("The Band/First_ Album/02 Middle_.wav").exists());
    // The playlist lists only what's there.
    let m3u = std::fs::read_to_string(w.music().join("Road Trip.m3u8")).unwrap();
    assert!(!m3u.contains("Later") && m3u.contains("Middle"));
}

#[tokio::test]
async fn a_write_protected_device_says_so() {
    let w = world_with(FakeFiles::new(), Fault::ReadOnly);
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Failed);
    let e = s.error.unwrap();
    assert_eq!(e.kind, "writeProtected");
    assert!(e.message.contains("write-protected"));
    assert_eq!(s.done, 0);
}

#[tokio::test]
async fn a_full_device_keeps_what_got_there() {
    let w = world_with(FakeFiles::new(), Fault::FullAfter(2));
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Failed);
    assert_eq!(s.error.unwrap().kind, "full");
    assert_eq!(w.manifest().files.len(), 2);
    assert!(w.files().iter().all(|f| !f.contains("platter-part")));
}

#[tokio::test]
async fn a_device_pulled_out_mid_sync_says_it_was_removed() {
    let pulled: Arc<AtomicBool> = Arc::default();
    let pull = pulled.clone();
    let files = FakeFiles {
        on_fetch: Box::new(move |n| {
            if n == 2 {
                pull.store(true, Ordering::SeqCst);
            }
        }),
        ..FakeFiles::new()
    };
    let w = world_pulled(files, Fault::None, pulled);
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Failed);
    let e = s.error.unwrap();
    assert_eq!(e.kind, "removed");
    assert!(!s.safe_to_unplug);
}

#[tokio::test]
async fn a_device_that_is_not_plugged_in_says_so() {
    let w = world();
    std::fs::remove_dir_all(&w.root).unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Failed);
    let e = s.error.unwrap();
    assert_eq!(e.kind, "notConnected");
    assert!(e.message.contains("Player"));
}

#[tokio::test]
async fn a_song_that_cannot_be_fetched_is_listed_and_the_rest_go_on() {
    let files = FakeFiles { fail: Some("t2".into()), ..FakeFiles::new() };
    let w = world_with(files, Fault::None);
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Done);
    assert_eq!(s.failed.len(), 1);
    assert_eq!(s.failed[0].title, "Middle?");
    assert_eq!(w.manifest().files.len(), 3);
}

#[tokio::test]
async fn cancelling_stops_and_leaves_the_device_safe() {
    let w = world();
    let ctl = Arc::new(SyncControl::default());
    ctl.cancel();
    w.devices.sync(w.current(), opts(DeviceFormat::Original, false), ctl).await;
    let s = w.devices.status().unwrap();
    assert_eq!(s.phase, Phase::Cancelled);
    assert!(s.safe_to_unplug);
    assert_eq!(s.done, 0);
    assert!(w.current().last_sync.is_none());
}

#[tokio::test]
async fn a_device_is_found_by_its_manifest_without_a_volume_id() {
    let w = world();
    w.sync(opts(DeviceFormat::Original, false)).await;
    let mut d = w.current();
    d.volume_id = Some("other-serial".into());
    let vols = FakeVolumes::new(w.root.clone()).list();
    let found = run::find_root(&d, &vols, &StdIo).unwrap();
    assert_eq!(found.root, w.root);
    d.id = "someone-else".into();
    assert!(run::find_root(&d, &vols, &StdIo).is_none());
}

#[tokio::test]
async fn the_view_shows_plugged_in_devices_and_new_drives() {
    let w = world();
    let v = w.devices.view();
    assert_eq!(v.devices.len(), 1);
    assert!(v.devices[0].root.is_some());
    assert!(v.drives.is_empty(), "the drive is the known device");
    std::fs::remove_dir_all(&w.root).unwrap();
    let v = w.devices.view();
    assert!(v.devices[0].root.is_none());
    w.devices.forget(&w.device.id).unwrap();
    assert!(w.devices.view().devices.is_empty());
}

fn decode(path: &Path) -> (u32, usize, usize) {
    let ext = path.extension().unwrap().to_string_lossy().into_owned();
    let hint = if ext == "opus" { "ogg".to_string() } else { ext };
    let mut d = crate::audio::decode::TrackDecoder::open(Box::new(std::fs::File::open(path).unwrap()), Some(&hint)).unwrap();
    let (rate, ch) = (d.rate, d.channels);
    let mut n = 0;
    while let Some(b) = d.next_block().unwrap() {
        n += b.len() / ch;
    }
    (rate, ch, n)
}

async fn converted(format: DeviceFormat, ext: &str) -> (World, PathBuf) {
    let w = world();
    let s = w.sync(opts(format, false)).await;
    assert_eq!(s.phase, Phase::Done, "{:?} {:?}", s.error, s.failed);
    let p = w.music().join(format!("The Band/First_ Album/01 Opening.{ext}"));
    assert!(p.exists(), "{:?}", w.files());
    (w, p)
}

fn title_of(p: &Path) -> Option<String> {
    use lofty::file::TaggedFileExt;
    use lofty::tag::Accessor;
    let f = lofty::read_from_path(p).ok()?;
    f.primary_tag().and_then(|t| t.title().map(|s| s.into_owned()))
}

#[tokio::test]
async fn songs_convert_to_flac() {
    let (_w, p) = converted(DeviceFormat::Flac, "flac").await;
    let (rate, ch, n) = decode(&p);
    assert_eq!((rate, ch), (RATE, 2));
    assert_eq!(n, (RATE as f32 * 0.3) as usize);
    assert_eq!(title_of(&p).as_deref(), Some("Opening"));
}

#[tokio::test]
async fn songs_convert_to_mp3() {
    let (_w, p) = converted(DeviceFormat::Mp3V0, "mp3").await;
    let (rate, ch, n) = decode(&p);
    assert_eq!((rate, ch), (RATE, 2));
    assert!(n > 10_000, "{n}");
    assert_eq!(title_of(&p).as_deref(), Some("Opening"));
}

#[tokio::test]
async fn songs_convert_to_opus() {
    let (_w, p) = converted(DeviceFormat::Opus, "opus").await;
    assert_eq!(&std::fs::read(&p).unwrap()[..4], b"OggS");
    assert_eq!(title_of(&p).as_deref(), Some("Opening"));
}

#[tokio::test]
async fn a_device_format_wins_over_the_default() {
    let w = world();
    w.devices
        .update(&w.device.id, |d| {
            d.format = Some(DeviceFormat::Mp3Cbr320);
            Ok(())
        })
        .unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Done, "{:?}", s.failed);
    assert!(w.music().join("The Band/Second/01 Later.mp3").exists());
    // Changing the format makes the songs again and replaces the old files.
    w.devices
        .update(&w.device.id, |d| {
            d.format = None;
            Ok(())
        })
        .unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.total, 4);
    assert!(w.music().join("The Band/Second/01 Later.wav").exists());
    assert!(!w.music().join("The Band/Second/01 Later.mp3").exists());
}

#[test]
fn the_device_settings_start_as_the_original_and_off() {
    use crate::settings::{Settings, SettingsPatch};
    let mut s = Settings::default();
    assert_eq!((s.device_format, s.device_delete_removed), (DeviceFormat::Original, false));
    let p: SettingsPatch = serde_json::from_value(serde_json::json!({ "deviceFormat": "mp3-v0", "deviceDeleteRemoved": true })).unwrap();
    s.apply(&p).unwrap();
    assert_eq!((s.device_format, s.device_delete_removed), (DeviceFormat::Mp3V0, true));
    let wire = serde_json::to_value(&s).unwrap();
    assert_eq!((wire["deviceFormat"].as_str(), wire["deviceDeleteRemoved"].as_bool()), (Some("mp3-v0"), Some(true)));
    assert!(serde_json::from_value::<SettingsPatch>(serde_json::json!({ "deviceFormat": "wma" })).is_err());
    for f in ["original", "flac", "mp3-320", "mp3-v0", "opus"] {
        assert_eq!(DeviceFormat::parse(f).unwrap().as_str(), f);
    }
}

#[test]
fn a_cancelled_flag_is_seen() {
    let c = SyncControl::default();
    assert!(!c.cancelled() && !c.paused());
    c.pause(true);
    assert!(c.paused());
    c.cancel();
    assert!(c.cancelled());
}

/// Waits (up to a generous limit) for something to become true.
async fn eventually(what: &str, f: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    while !f() {
        assert!(start.elapsed() < std::time::Duration::from_secs(20), "timed out waiting for {what}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

fn the_four_songs() -> Vec<&'static str> {
    vec![
        "The Band/First_ Album/01 Opening.wav",
        "The Band/First_ Album/02 Middle_.wav",
        "The Band/First_ Album/03 Closing.wav",
        "The Band/Second/01 Later.wav",
    ]
}

#[tokio::test]
async fn a_damaged_manifest_is_kept_aside_and_the_songs_are_matched_again() {
    let w = world();
    w.sync(opts(DeviceFormat::Original, false)).await;
    std::fs::write(w.music().join(MANIFEST), b"{\"version\":1,\"deviceId\":\"x\",\"fil").unwrap();

    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Done, "{:?}", s.error);
    assert!(s.notice.as_deref().is_some_and(|n| n.contains(".bak")), "{:?}", s.notice);
    assert_eq!(std::fs::read(w.music().join(".platter-sync.json.bak")).unwrap(), b"{\"version\":1,\"deviceId\":\"x\",\"fil");
    // Nothing copied a second time under another name.
    assert!(w.files().iter().all(|f| !f.contains(" (2)")), "{:?}", w.files());
    let m = w.manifest();
    assert_eq!(m.files.keys().map(String::as_str).collect::<Vec<_>>(), the_four_songs());
    assert_eq!(m.playlists.keys().collect::<Vec<_>>(), vec!["Road Trip.m3u8"]);
    // The next sync is an ordinary one.
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!((s.total, s.kept, s.notice), (0, 4, None));
}

#[tokio::test]
async fn a_manifest_from_a_newer_platter_stops_the_sync_untouched() {
    let w = world();
    w.sync(opts(DeviceFormat::Original, false)).await;
    let newer = br#"{"version":99,"deviceId":"x","files":{"a":"b"}}"#;
    std::fs::write(w.music().join(MANIFEST), newer).unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Failed);
    assert!(s.error.unwrap().message.contains("newer version of Platter"));
    assert_eq!(std::fs::read(w.music().join(MANIFEST)).unwrap(), newer);
    assert!(w.files().iter().all(|f| !f.contains(" (2)")));
}

#[tokio::test]
async fn songs_written_before_the_drive_was_pulled_are_not_copied_twice() {
    let pulled: Arc<AtomicBool> = Arc::default();
    let w = world_pulled(FakeFiles::new(), Fault::PulledAfter(2), pulled.clone());
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.error.unwrap().kind, "removed");
    // The manifest was saved before any song; the journal has the two.
    assert!(w.manifest().files.is_empty());
    let journal = std::fs::read_to_string(w.music().join(manifest::JOURNAL)).unwrap();
    assert_eq!(journal.lines().count(), 2);

    pulled.store(false, Ordering::SeqCst);
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Done, "{:?}", s.error);
    assert_eq!((s.kept, s.total), (2, 2));
    assert!(w.files().iter().all(|f| !f.contains(" (2)")), "{:?}", w.files());
    assert_eq!(w.manifest().files.len(), 4);
    assert!(!w.music().join(manifest::JOURNAL).exists(), "folded into the manifest");
}

#[tokio::test]
async fn an_untracked_song_of_the_right_size_is_taken_back() {
    let w = world();
    w.sync(opts(DeviceFormat::Original, false)).await;
    // The manifest and journal lost (say, deleted by hand): the files are
    // at their places and the same size as what would be written.
    std::fs::remove_file(w.music().join(MANIFEST)).unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Done);
    assert_eq!(s.total, 4);
    assert!(w.files().iter().all(|f| !f.contains(" (2)")), "{:?}", w.files());
    assert_eq!(w.manifest().files.keys().map(String::as_str).collect::<Vec<_>>(), the_four_songs());
}

#[tokio::test]
async fn a_failed_sync_stops_the_songs_still_being_made() {
    let files = FakeFiles { hold_from: Some(2), ..FakeFiles::new() };
    let in_flight = files.in_flight.clone();
    let w = world_with(files, Fault::FullAfter(1));
    let ctl = Arc::new(SyncControl::default());
    w.devices.sync(w.current(), opts(DeviceFormat::Original, false), ctl.clone()).await;
    let s = w.devices.status().unwrap();
    assert_eq!(s.error.unwrap().kind, "full");
    assert!(ctl.cancelled(), "the run's work was told to stop");
    eventually("the held fetch to stop", || in_flight.load(Ordering::SeqCst) == 0).await;
    let scratch = w._dir.path().join("scratch").join(&w.device.id);
    eventually("the run's scratch folder to go", || std::fs::read_dir(&scratch).map(|r| r.count() == 0).unwrap_or(true)).await;
}

#[tokio::test]
async fn each_run_has_its_own_scratch_folder() {
    let files = FakeFiles::new();
    let dests = files.dests.clone();
    let w = world_with(files, Fault::None);
    w.sync(opts(DeviceFormat::Original, false)).await;
    std::fs::remove_dir_all(w.music()).unwrap();
    w.sync(opts(DeviceFormat::Original, false)).await;
    let dests = dests.lock().clone();
    assert_eq!(dests.len(), 8);
    let first = dests[0].parent().unwrap().to_path_buf();
    let second = dests[4].parent().unwrap().to_path_buf();
    assert_ne!(first, second);
    assert_eq!(first.parent(), second.parent(), "both below the device's scratch folder");
}

#[tokio::test]
async fn a_case_only_rename_keeps_the_song_just_written() {
    let w = world();
    w.sync(opts(DeviceFormat::Original, false)).await;
    w.db.write(|c| {
        c.execute("UPDATE tracks SET album_title = 'FIRST: Album' WHERE album_id = 'al1'", [])?;
        Ok(())
    })
    .unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Done, "{:?}", s.error);
    assert_eq!(s.total, 3);
    let m = w.manifest();
    assert_eq!(m.files.len(), 4);
    for rel in m.files.keys() {
        assert!(w.music().join(rel).is_file(), "{rel} is listed but gone: {:?}", w.files());
    }
}

#[tokio::test]
async fn a_song_that_panics_while_being_made_is_a_failed_song() {
    let files = FakeFiles { panic_on: Some("t3".into()), ..FakeFiles::new() };
    let w = world_with(files, Fault::None);
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Done, "not shown as cancelled");
    assert_eq!(s.failed.len(), 1);
    assert_eq!(s.failed[0].title, "Closing");
    assert_eq!(w.manifest().files.len(), 3);
}

#[tokio::test]
async fn partial_files_from_an_earlier_sync_are_swept() {
    let w = world();
    let part = w.music().join("The Band/Old/.01 x.wav.platter-part");
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, b"half").unwrap();
    let mine = w.music().join("The Band/Old/keep.platter-partial");
    std::fs::write(&mine, b"mine").unwrap();
    let s = w.sync(opts(DeviceFormat::Original, false)).await;
    assert_eq!(s.phase, Phase::Done);
    assert!(!part.exists());
    assert!(mine.exists());
}

#[tokio::test]
async fn a_fixed_drive_is_never_a_device_from_its_top() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("drive");
    std::fs::create_dir_all(root.join("Music")).unwrap();
    let devices = Devices::new(Deps {
        db: DbHandle::open(dir.path().join("lib.db")).unwrap(),
        files: Arc::new(FakeFiles::new()),
        volumes: Arc::new(FakeVolumes { root: root.clone(), removable: false }),
        io: Arc::new(StdIo),
        pace: Arc::new(NoPace),
        scratch: dir.path().join("scratch"),
    });
    assert!(devices.add(&root, "Disk", "").is_err());
    // A path that climbs back to the top is the top.
    assert!(devices.add(&root.join("Music").join(".."), "Disk", "/").is_err());
    let d = devices.add(&root.join("Music").join(".."), "Disk", "Music").unwrap();
    assert_eq!(d.folder, "Music");
    assert!(devices.add(&dir.path().join("nowhere"), "Disk", "Music").is_err());
}

#[test]
fn music_dir_never_climbs_out() {
    let root = Path::new("/media/usb");
    let p = super::run::music_dir(root, "../../etc/C:/x\\..\\y/./Music");
    assert!(p.starts_with(root));
    assert_eq!(p, root.join("etc").join("Music"));
}
