//! Lyrics for every song on the Bridge, without a real model: songs are WAV
//! files in a temporary music folder, the model download and the speech
//! model are fakes, and the clock is set by hand. Each failure state is
//! faked here: the model download failing or not matching its checksum, a
//! song the Bridge can't read, the server busy, the night ending, and a
//! restart in the middle of the night.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use parking_lot::Mutex;

use super::lyrics::{self, BridgeLyrics, LyricsDeps, MountSongs, Opts};
use super::wire::{LyricsState, NightHours};
use crate::db::DbHandle;
use crate::db::store::kv_get;
use crate::lyrics::transcribe::engine::{Progress, RecognizeError, Recognizer, Stop};
use crate::lyrics::transcribe::lines::{Segment, Word};
use crate::lyrics::transcribe::model::{ModelFetch, ModelKind, ModelSpec, ModelStore, sha256_hex};
use crate::lyrics::transcribe::pace::fake::{FakeLoad, timing};
use crate::lyrics::transcribe::{OtherLyrics, PauseReason, TranscribeStatus, pace};
use crate::sync::SyncStore;
use crate::sync::store::TRANSCRIPTS;

/// 2 AM UTC on a day in 2027; the night hours (1 to 7) hold it.
const NIGHT: i64 = 1_799_971_200 + 2 * 3600;
/// Noon that day.
const NOON: i64 = 1_799_971_200 + 12 * 3600;
const PLEX_ROOT: &str = "/data/media/Music";

struct FakeFetch {
    body: Mutex<Vec<u8>>,
    refuse: AtomicBool,
    opened: AtomicUsize,
}

#[async_trait]
impl ModelFetch for FakeFetch {
    async fn open(&self, _url: &str) -> Result<BoxStream<'static, Result<Bytes, String>>, String> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        if self.refuse.load(Ordering::SeqCst) {
            return Err("http 503".into());
        }
        let body = self.body.lock().clone();
        let chunks: Vec<Result<Bytes, String>> = body.chunks(1000).map(|c| Ok(Bytes::copy_from_slice(c))).collect();
        Ok(Box::pin(futures::stream::iter(chunks)))
    }
}

struct FakeRec {
    runs: AtomicUsize,
    threads: AtomicUsize,
}

impl Recognizer for FakeRec {
    fn unavailable(&self) -> Option<String> {
        None
    }
    fn transcribe(&self, model: &Path, _: ModelKind, _: &[f32], threads: usize, _: &str, _: Stop, _: Progress) -> Result<Vec<Segment>, RecognizeError> {
        assert!(model.is_file());
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.threads.store(threads, Ordering::SeqCst);
        let w = |t: &str, at: u32| Word { start_ms: at, end_ms: at + 250, text: format!(" {t}") };
        Ok(vec![Segment { words: vec![w("night", 1000), w("song", 1300)] }])
    }
}

/// Songs with lyrics from Plex or LRCLIB.
#[derive(Default)]
struct FakeOther(Mutex<Vec<String>>);

#[async_trait]
impl OtherLyrics for FakeOther {
    async fn has_lyrics(&self, track_id: &str) -> bool {
        self.0.lock().iter().any(|t| t == track_id)
    }
}

fn model_body() -> Vec<u8> {
    (0..4_000).map(|i| (i % 249) as u8).collect()
}

fn specs() -> Vec<ModelSpec> {
    let b = model_body();
    vec![ModelSpec { kind: ModelKind::Small, file: "small.bin".into(), url: "https://models.invalid/small.bin".into(), size: b.len() as u64, sha256: sha256_hex(&b) }]
}

/// Half a second of a tone, as a WAV file.
fn wav() -> Vec<u8> {
    let mut cur = std::io::Cursor::new(Vec::new());
    {
        let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
        for i in 0..8_000 {
            w.write_sample(((i as f32 * 0.2).sin() * 6000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
    }
    cur.into_inner()
}

struct Fx {
    l: Arc<BridgeLyrics>,
    db: DbHandle,
    store: SyncStore,
    mount: PathBuf,
    clock: Arc<AtomicI64>,
    fetch: Arc<FakeFetch>,
    rec: Arc<FakeRec>,
    other: Arc<FakeOther>,
    load: Arc<FakeLoad>,
}

/// The library: four songs in the music folder (`t1` played most), one
/// whose file is gone (`gone`), one whose path climbs out of the folder
/// (`out`), one that isn't audio (`junk`).
fn library(db: &DbHandle, mount: &Path) {
    db.write(|c| {
        c.execute_batch(
            "INSERT INTO artists (id, title, sort_title) VALUES ('a', 'Night Band', 'Night Band');
             INSERT INTO albums (id, title, sort_title, artist_id, artist_title) VALUES ('al', 'Late', 'Late', 'a', 'Night Band');",
        )?;
        for (id, file, plays) in [
            ("t1", "Night Band/Late/01 One.wav", 9),
            ("t2", "Night Band/Late/02 Two.wav", 1),
            ("t3", "Night Band/Late/03 Three.wav", 0),
            ("t4", "Night Band/Late/04 Four.wav", 0),
            ("gone", "Night Band/Late/05 Gone.wav", 0),
            ("out", "../outside.wav", 0),
            ("junk", "Night Band/Late/07 Junk.wav", 0),
        ] {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, file, view_count, container)
                 VALUES (?1, ?1, 'al', 'Late', 'a', 'Night Band', 500, '/p/' || ?1, ?2, ?3, 'wav')",
                rusqlite::params![id, format!("{PLEX_ROOT}/{file}"), plays],
            )?;
        }
        Ok(())
    })
    .unwrap();
    let album = mount.join("Night Band").join("Late");
    std::fs::create_dir_all(&album).unwrap();
    for f in ["01 One.wav", "02 Two.wav", "03 Three.wav", "04 Four.wav"] {
        std::fs::write(album.join(f), wav()).unwrap();
    }
    std::fs::write(album.join("07 Junk.wav"), b"this is not audio at all").unwrap();
    std::fs::write(mount.parent().unwrap().join("outside.wav"), wav()).unwrap();
}

fn build(dir: &Path, db: DbHandle, store: SyncStore, music: bool, clock: Arc<AtomicI64>, retry: Duration) -> Fx {
    build_with_root(dir, db, store, music, clock, retry, PLEX_ROOT)
}

/// `root`: MUSIC_PLEX_PATH ("" when it isn't set).
fn build_with_root(dir: &Path, db: DbHandle, store: SyncStore, music: bool, clock: Arc<AtomicI64>, retry: Duration, root: &str) -> Fx {
    let mount = dir.join("music");
    let root: lyrics::MusicRoot = Arc::new(parking_lot::RwLock::new(root.to_string()));
    let fetch = Arc::new(FakeFetch { body: Mutex::new(model_body()), refuse: AtomicBool::new(false), opened: AtomicUsize::new(0) });
    let rec = Arc::new(FakeRec { runs: AtomicUsize::new(0), threads: AtomicUsize::new(0) });
    let other = Arc::new(FakeOther::default());
    let load = FakeLoad::new(5.0);
    let c = clock.clone();
    let l = BridgeLyrics::with_retry(
        LyricsDeps {
            db: db.clone(),
            store: store.clone(),
            models: ModelStore::new(dir.join("models"), specs(), Arc::new(|_: &Path| Some(u64::MAX / 2))),
            fetch: fetch.clone(),
            recognizer: rec.clone(),
            songs: Arc::new(MountSongs { db: db.clone(), plex_root: root.clone(), mount: mount.clone() }),
            other: other.clone(),
            // A 16-thread server: Light is 2 threads in the background.
            // Readings averaged over a wider window than the quick test timing,
            // so a slow test machine still sees the server as busy.
            pacer: pace::Pacer::with_cores(load.clone(), pace::Timing { sample: Duration::from_millis(20), window: Duration::from_millis(50), resume: Duration::from_millis(60), ..timing() }, 16),
            music,
            root,
            clock: Arc::new(move || c.load(Ordering::SeqCst)),
            lrclib: Arc::new(AtomicBool::new(false)),
        },
        retry,
    );
    l.spawn();
    Fx { l, db, store, mount, clock, fetch, rec, other, load }
}

fn fx_retry(retry: Duration) -> (Fx, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let db = DbHandle::open(tmp.path().join("bridge.db")).unwrap();
    let store = SyncStore::new(db.clone(), "bridge-0011");
    std::fs::create_dir_all(tmp.path().join("music")).unwrap();
    library(&db, &tmp.path().join("music"));
    let f = build(tmp.path(), db, store, true, Arc::new(AtomicI64::new(NIGHT)), retry);
    (f, tmp)
}

fn fx() -> (Fx, tempfile::TempDir) {
    fx_retry(lyrics::DOWNLOAD_RETRY)
}

async fn on(f: &Fx) {
    f.l.set_opts(Opts { on: true, hours: NightHours::Night, utc_offset_s: 0, lrclib: false }).await;
}

const WAIT: Duration = Duration::from_secs(20);

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !ok() {
        if std::time::Instant::now() > deadline {
            panic!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn status(f: &Fx) -> TranscribeStatus {
    f.l.transcriber().status()
}

fn idle(f: &Fx) -> bool {
    let s = status(f);
    s.current.is_none() && s.queued == 0 && !s.models.iter().any(|m| m.downloading)
}

/// Ticks until the queue has nothing more to take.
async fn run_night(f: &Fx) {
    for _ in 0..40 {
        until("the queue to empty", || idle(f)).await;
        if f.l.tick().await == 0 {
            until("the queue to empty", || idle(f)).await;
            if f.l.tick().await == 0 {
                return;
            }
        }
    }
    panic!("the night never ended");
}

fn has_lyrics(db: &DbHandle, id: &str) -> bool {
    let id = id.to_string();
    db.read(move |c| crate::lyrics::transcribe::store::done(c, &id)).unwrap()
}

fn files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if p.is_dir() {
            out.extend(files(&p));
        } else {
            out.push((p, e.metadata().unwrap().len()));
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------- pieces

#[test]
fn night_hours_wrap_round_midnight_and_follow_the_computers_time_zone() {
    let at = |h: i64| 1_799_971_200 + h * 3600;
    assert!(NightHours::Night.contains(at(1), 0) && NightHours::Night.contains(at(6), 0));
    assert!(!NightHours::Night.contains(at(7), 0) && !NightHours::Night.contains(at(0), 0));
    assert!(NightHours::Late.contains(at(23), 0) && NightHours::Late.contains(at(2), 0));
    assert!(!NightHours::Late.contains(at(6), 0) && !NightHours::Late.contains(at(22), 0));
    assert!(NightHours::Early.contains(at(7), 0) && !NightHours::Early.contains(at(8), 0));
    assert!(NightHours::Always.contains(at(12), 0));
    // A computer five hours behind UTC: 2 AM there is 7 AM UTC.
    assert!(NightHours::Night.contains(at(7), -5 * 3600));
    assert!(!NightHours::Night.contains(at(2), -5 * 3600));
    // The container's own time zone doesn't matter: only the unix time and the offset.
    assert!(NightHours::Night.contains(at(24 + 3), 0));
}

#[test]
fn a_songs_file_is_found_only_inside_the_music_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let mount = tmp.path().join("music");
    std::fs::create_dir_all(mount.join("A")).unwrap();
    std::fs::write(mount.join("A/song.flac"), b"x").unwrap();
    std::fs::write(tmp.path().join("secret.flac"), b"x").unwrap();
    let got = lyrics::resolve(PLEX_ROOT, &mount, &format!("{PLEX_ROOT}/A/song.flac")).unwrap();
    assert!(got.ends_with("song.flac"));
    assert!(lyrics::resolve(PLEX_ROOT, &mount, &format!("{PLEX_ROOT}/A/missing.flac")).is_none(), "missing");
    assert!(lyrics::resolve(PLEX_ROOT, &mount, &format!("{PLEX_ROOT}/../secret.flac")).is_none(), "climbing out");
    assert!(lyrics::resolve(PLEX_ROOT, &mount, "/elsewhere/A/song.flac").is_none(), "another Plex folder");
    assert!(lyrics::resolve(PLEX_ROOT, &mount, &format!("{PLEX_ROOT}/A")).is_none(), "a folder isn't a song");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(tmp.path().join("secret.flac"), mount.join("A/link.flac")).unwrap();
        assert!(lyrics::resolve(PLEX_ROOT, &mount, &format!("{PLEX_ROOT}/A/link.flac")).is_none(), "a link out of the folder");
    }
}

#[test]
fn the_next_songs_are_the_most_played_without_lyrics_anywhere() {
    let tmp = tempfile::tempdir().unwrap();
    let db = DbHandle::open(tmp.path().join("b.db")).unwrap();
    std::fs::create_dir_all(tmp.path().join("music")).unwrap();
    library(&db, &tmp.path().join("music"));
    let none = HashSet::new();
    let next = |have: &HashSet<String>, now| db.read(|c| lyrics::next_songs(c, have, now, 3)).unwrap();
    assert_eq!(next(&none, NIGHT), ["t1", "t2", "t3"].map(String::from), "most played first, then library order");
    // A computer has t1; Plex has lyrics for t2; t3 couldn't be read until next week.
    let have: HashSet<String> = ["t1".to_string()].into();
    db.write(|c| {
        lyrics::mark(c, "t2", "other")?;
        lyrics::mark(c, "gone", &format!("retry:{}", NIGHT + lyrics::RETRY_S))
    })
    .unwrap();
    assert_eq!(next(&have, NIGHT), ["t3", "t4", "out"].map(String::from));
    let later = db.read(|c| lyrics::next_songs(c, &have, NIGHT + lyrics::RETRY_S + 1, 0)).unwrap();
    assert!(later.contains(&"gone".to_string()), "a week later it's tried again");
    assert!(!db.read(|c| lyrics::next_songs(c, &have, NIGHT, 0)).unwrap().contains(&"gone".to_string()));
    // A replaced file (another part) is looked at again.
    db.write(|c| Ok(c.execute("UPDATE tracks SET part_key = '/p/t2-new' WHERE id = 't2'", [])?)).unwrap();
    assert!(next(&have, NIGHT).contains(&"t2".to_string()));
    let (done, left) = db.read(|c| lyrics::counts(c, &have, NIGHT)).unwrap();
    assert_eq!((done, left), (0, 5), "all but t1 (the computer has it) and gone (next week)");
}

// ---------------------------------------------------------------- the night

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bridge_transcribes_at_night_and_the_lyrics_go_to_every_device() {
    let (f, _tmp) = fx();
    // Not switched on yet: nothing happens.
    assert_eq!(f.l.tick().await, 0);
    assert_eq!(f.l.line().state, LyricsState::Off);
    f.other.0.lock().push("t4".into());
    f.l.set_have("link-1", vec!["t3".into(), "../bad".into()]).await;
    on(&f).await;
    let before = files(&f.mount);
    run_night(&f).await;
    // The model came down once, checked.
    assert_eq!(f.fetch.opened.load(Ordering::SeqCst), 1);
    for id in ["t1", "t2"] {
        assert!(has_lyrics(&f.db, id), "{id}");
    }
    assert!(!has_lyrics(&f.db, "t3"), "a computer has it already");
    assert!(!has_lyrics(&f.db, "t4"), "Plex has lyrics for it");
    assert_eq!(f.rec.threads.load(Ordering::SeqCst), 2, "the Light budget");
    // Each result is in the sync store, for every device, with its file version and model.
    let t: crate::sync::transcripts::Transcript = f.store.get(&TRANSCRIPTS, "t1|small").unwrap().unwrap();
    assert_eq!(t.part, "/p/t1");
    assert!(t.text.to_lowercase().contains("night") && t.text.contains("song"), "{}", t.text);
    // The songs it couldn't use wait a week; the music folder wasn't touched.
    for id in ["gone", "out", "junk"] {
        assert!(!has_lyrics(&f.db, id), "{id}");
    }
    assert_eq!(files(&f.mount), before, "nothing in the music folder was written");
    let left = f.db.read(|c| lyrics::next_songs(c, &HashSet::new(), NIGHT, 0)).unwrap();
    assert_eq!(left, ["t3"].map(String::from), "only the computer's song is left (the Bridge knows it has it)");
    let line = f.l.line();
    assert_eq!(line.state, LyricsState::Done, "{line:?}");
    assert_eq!(line.done, 2);
    assert_eq!(line.left, 0);
    assert!(lyrics::words(&line).contains("every song has lyrics"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn by_day_nothing_starts_and_songs_in_the_queue_wait_for_the_night() {
    let (f, _tmp) = fx();
    on(&f).await;
    f.clock.store(NOON, Ordering::SeqCst);
    assert_eq!(f.l.tick().await, 0, "nothing goes in by day");
    assert_eq!(f.l.line().state, LyricsState::Waiting);
    // Songs that went in at night wait once the night is over.
    f.l.transcriber().download_model(ModelKind::Small).unwrap();
    until("the model", || status(&f).models[0].installed).await;
    f.l.transcriber().transcribe_background(vec!["t1".into()]).unwrap();
    until("waiting for the night", || status(&f).paused == Some(PauseReason::Hours)).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 0);
    // Night again (in the computer's time zone, six hours behind UTC).
    f.l.set_opts(Opts { on: true, hours: NightHours::Night, utc_offset_s: -6 * 3600, lrclib: false }).await;
    f.clock.store(NOON - 4 * 3600, Ordering::SeqCst);
    until("t1 done", || has_lyrics(&f.db, "t1")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_model_download_is_said_and_tried_again_later() {
    let (f, _tmp) = fx_retry(Duration::from_millis(300));
    f.fetch.refuse.store(true, Ordering::SeqCst);
    on(&f).await;
    f.l.tick().await;
    until("the failure", || f.l.line().state == LyricsState::Problem).await;
    let line = f.l.line();
    assert!(line.problem.as_deref().unwrap().contains("didn't download"), "{line:?}");
    until("the queue to empty", || idle(&f)).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 0);
    assert_eq!(f.l.tick().await, 0, "not straight away");
    // No song was marked bad for it.
    assert_eq!(f.db.read(|c| lyrics::next_songs(c, &HashSet::new(), NIGHT, 0)).unwrap().len(), 7);
    // An hour later (here, a moment) it tries again, and works.
    f.fetch.refuse.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(350)).await;
    run_night(&f).await;
    assert!(has_lyrics(&f.db, "t1"));
    assert_eq!(f.fetch.opened.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_damaged_model_download_is_deleted_and_said() {
    let (f, _tmp) = fx();
    f.fetch.body.lock()[7] ^= 0xff;
    on(&f).await;
    f.l.tick().await;
    until("the failure", || f.l.line().state == LyricsState::Problem).await;
    assert!(f.l.line().problem.unwrap().contains("checksum"));
    assert!(!status(&f).models[0].installed, "the bad file is gone");
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_busy_server_holds_the_work_until_its_calm() {
    let (f, _tmp) = fx();
    on(&f).await;
    f.load.set(90.0);
    f.l.tick().await;
    until("busy", || f.l.line().state == LyricsState::Busy).await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 0, "nothing runs while the server is busy");
    f.load.set(5.0);
    until("t1 done once calm", || has_lyrics(&f.db, "t1")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_the_music_folder_it_waits_and_says_what_to_set() {
    let tmp = tempfile::tempdir().unwrap();
    let db = DbHandle::open(tmp.path().join("bridge.db")).unwrap();
    let f = build(tmp.path(), db.clone(), SyncStore::new(db, "bridge-1"), false, Arc::new(AtomicI64::new(NIGHT)), lyrics::DOWNLOAD_RETRY);
    on(&f).await;
    assert_eq!(f.l.tick().await, 0);
    let line = f.l.line();
    assert_eq!(line.state, LyricsState::NoMusic);
    assert!(line.problem.unwrap().contains("/music"));
    assert!(!line.state.takes_over(), "computers carry on with their own");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_music_plex_path_it_uses_its_own_match_of_the_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let db = DbHandle::open(tmp.path().join("bridge.db")).unwrap();
    std::fs::create_dir_all(tmp.path().join("music")).unwrap();
    library(&db, &tmp.path().join("music"));
    let f = build_with_root(tmp.path(), db.clone(), SyncStore::new(db, "bridge-1"), true, Arc::new(AtomicI64::new(NIGHT)), lyrics::DOWNLOAD_RETRY, "");
    on(&f).await;
    assert_eq!(f.l.tick().await, 0, "it doesn't know which of Plex's folders this is yet");
    assert_eq!(f.l.line().state, LyricsState::NoMusic);
    // Once it has read Plex's library it matches the folder (run::suggest_plex_path).
    f.l.suggest_root(PLEX_ROOT);
    f.l.suggest_root("/somewhere/else");
    run_night(&f).await;
    assert!(has_lyrics(&f.db, "t1"), "read with its own match");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bridge_restarted_in_the_night_carries_on_from_what_it_did() {
    let tmp = tempfile::tempdir().unwrap();
    let db = DbHandle::open(tmp.path().join("bridge.db")).unwrap();
    let store = SyncStore::new(db.clone(), "bridge-0011");
    std::fs::create_dir_all(tmp.path().join("music")).unwrap();
    library(&db, &tmp.path().join("music"));
    let clock = Arc::new(AtomicI64::new(NIGHT));
    {
        let f = build(tmp.path(), db.clone(), store.clone(), true, clock.clone(), lyrics::DOWNLOAD_RETRY);
        on(&f).await;
        f.l.set_have("link-1", vec!["t2".into()]).await;
        f.l.tick().await;
        until("t1 done", || has_lyrics(&f.db, "t1")).await;
        f.l.transcriber().cancel(None);
    }
    // Started again: the settings and the computer's list are kept, and done songs aren't redone.
    let g = build(tmp.path(), db.clone(), store, true, clock, lyrics::DOWNLOAD_RETRY);
    assert!(g.l.opts().on);
    run_night(&g).await;
    assert_eq!(g.fetch.opened.load(Ordering::SeqCst), 0, "the model was already there");
    assert!(!has_lyrics(&db, "t2"), "the computer's list was kept");
    assert!(has_lyrics(&db, "t3"));
}

fn junk_mark(db: &DbHandle) -> Option<String> {
    db.read(|c| kv_get(c, &format!("{}junk|/p/junk", lyrics::SKIP_PREFIX))).unwrap()
}

async fn until_mark(db: &DbHandle, want: &str) {
    let deadline = std::time::Instant::now() + WAIT;
    while junk_mark(db).as_deref() != Some(want) {
        assert!(std::time::Instant::now() < deadline, "mark is {:?}, not {want}", junk_mark(db));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_song_that_fails_again_after_its_week_gets_a_fresh_week() {
    let (f, _tmp) = fx();
    on(&f).await;
    run_night(&f).await;
    // The mark is written off the async threads.
    until_mark(&f.db, &format!("retry:{}", NIGHT + lyrics::RETRY_S)).await;
    // A week later (same hour of the night) it's tried again, fails again, and waits another week.
    let later = NIGHT + lyrics::RETRY_S + 60;
    f.clock.store(later, Ordering::SeqCst);
    run_night(&f).await;
    until_mark(&f.db, &format!("retry:{}", later + lyrics::RETRY_S)).await;
    let next = f.db.read(|c| lyrics::next_songs(c, &HashSet::new(), later, 0)).unwrap();
    assert!(!next.contains(&"junk".to_string()), "not served again tonight: {next:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_song_without_a_file_version_is_never_picked_and_a_huge_file_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let db = DbHandle::open(tmp.path().join("b.db")).unwrap();
    let mount = tmp.path().join("music");
    std::fs::create_dir_all(&mount).unwrap();
    library(&db, &mount);
    db.write(|c| Ok(c.execute("UPDATE tracks SET part_key = '' WHERE id = 't1'", [])?)).unwrap();
    let next = db.read(|c| lyrics::next_songs(c, &HashSet::new(), NIGHT, 0)).unwrap();
    assert!(!next.contains(&"t1".to_string()), "it could never be marked, so it would come back every tick");
    assert_eq!(db.read(|c| lyrics::counts(c, &HashSet::new(), NIGHT)).unwrap().1, next.len());
    // A file over the cap is never read into memory.
    let big = mount.join("Night Band").join("Late").join("02 Two.wav");
    std::fs::OpenOptions::new().write(true).open(&big).unwrap().set_len(lyrics::MAX_SONG_BYTES + 1).unwrap();
    let songs = MountSongs { db: db.clone(), plex_root: Arc::new(parking_lot::RwLock::new(PLEX_ROOT.into())), mount };
    let got = crate::lyrics::transcribe::SongReader::read(&songs, "t2", &AtomicBool::new(false)).await;
    assert!(matches!(got, Err(crate::lyrics::transcribe::audio::AudioError::Unreadable(_))), "skipped");
}
