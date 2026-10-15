//! Transcription without a real model: fake downloads, a fake recognizer,
//! songs made of a generated tone. Each failure state in docs/APP-STATES.md
//! (download failed, checksum mismatch, out of disk, transcription failed)
//! is faked here. The real-model run is `#[ignore]`d at the bottom.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use parking_lot::Mutex;

use super::audio::{AudioError, RATE};
use super::engine::{Progress, RecognizeError, Recognizer};
use super::lines::{Segment, Word};
use super::model::{DownloadError, ModelFetch, ModelKind, ModelSpec, ModelStore, sha256_hex};
use super::*;
use crate::lyrics::user;

// ---------------------------------------------------------------- fakes

/// Serves fixed bytes, or fails after some of them.
struct FakeFetch {
    body: Mutex<Vec<u8>>,
    fail_after: Mutex<Option<usize>>,
    refuse: AtomicBool,
    /// Wait this long between chunks (so a test can cancel mid-way).
    gap: Duration,
}

impl FakeFetch {
    fn new(body: Vec<u8>) -> Arc<Self> {
        Arc::new(Self { body: Mutex::new(body), fail_after: Mutex::new(None), refuse: AtomicBool::new(false), gap: Duration::ZERO })
    }
}

#[async_trait]
impl ModelFetch for FakeFetch {
    async fn open(&self, _url: &str) -> Result<BoxStream<'static, Result<Bytes, String>>, String> {
        if self.refuse.load(Ordering::Relaxed) {
            return Err("http 503".into());
        }
        let body = self.body.lock().clone();
        let fail_after = *self.fail_after.lock();
        let gap = self.gap;
        let mut chunks: Vec<Result<Bytes, String>> = body.chunks(1000).map(|c| Ok(Bytes::copy_from_slice(c))).collect();
        if let Some(n) = fail_after {
            chunks.truncate(n);
            chunks.push(Err("connection reset".into()));
        }
        Ok(futures::stream::iter(chunks)
            .then(move |c| async move {
                if !gap.is_zero() {
                    tokio::time::sleep(gap).await;
                }
                c
            })
            .boxed())
    }
}

#[derive(Clone)]
enum Behaviour {
    Words(Vec<Segment>),
    Fail,
    /// Runs until cancelled.
    Block,
}

struct FakeRecognizer {
    behaviour: Mutex<Behaviour>,
    runs: AtomicUsize,
    /// Samples given on the last run.
    last_len: AtomicUsize,
    last_threads: AtomicUsize,
}

impl FakeRecognizer {
    fn new(b: Behaviour) -> Arc<Self> {
        Arc::new(Self { behaviour: Mutex::new(b), runs: AtomicUsize::new(0), last_len: AtomicUsize::new(0), last_threads: AtomicUsize::new(0) })
    }
}

impl Recognizer for FakeRecognizer {
    fn unavailable(&self) -> Option<String> {
        None
    }
    fn transcribe(
        &self,
        model: &Path,
        _kind: ModelKind,
        pcm: &[f32],
        threads: usize,
        cancel: Arc<AtomicBool>,
        progress: Progress,
    ) -> Result<Vec<Segment>, RecognizeError> {
        assert!(model.is_file(), "the model is on disk");
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.last_len.store(pcm.len(), Ordering::SeqCst);
        self.last_threads.store(threads, Ordering::SeqCst);
        progress(50);
        match self.behaviour.lock().clone() {
            Behaviour::Words(s) => Ok(s),
            Behaviour::Fail => Err(RecognizeError::Failed("boom".into())),
            Behaviour::Block => {
                while !cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(RecognizeError::Cancelled)
            }
        }
    }
}

/// Two seconds of a 44.1 kHz stereo tone as a WAV file.
fn wav() -> Vec<u8> {
    let mut cur = std::io::Cursor::new(Vec::new());
    {
        let spec = hound::WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
        for i in 0..88_200 {
            let s = ((i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 8000.0) as i16;
            w.write_sample(s).unwrap();
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }
    cur.into_inner()
}

struct FakeSongs {
    unreachable: AtomicBool,
    unreadable: AtomicBool,
    reads: AtomicUsize,
}

#[async_trait]
impl SongReader for FakeSongs {
    async fn read(&self, _track_id: &str, _cancel: &AtomicBool) -> Result<SongAudio, AudioError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.unreachable.load(Ordering::Relaxed) {
            return Err(AudioError::Unreachable("offline".into()));
        }
        if self.unreadable.load(Ordering::Relaxed) {
            return Ok(SongAudio { bytes: b"not audio at all".to_vec(), hint: None });
        }
        Ok(SongAudio { bytes: wav(), hint: Some("wav".into()) })
    }
}

#[derive(Default)]
struct FakeOther(Mutex<Vec<String>>);

#[async_trait]
impl OtherLyrics for FakeOther {
    async fn has_lyrics(&self, track_id: &str) -> bool {
        self.0.lock().iter().any(|t| t == track_id)
    }
}

#[derive(Default)]
struct FakeConditions(Mutex<Option<PauseReason>>);

#[async_trait]
impl Conditions for FakeConditions {
    async fn pause_reason(&self) -> Option<PauseReason> {
        *self.0.lock()
    }
}

#[derive(Default)]
struct FakeEvents {
    statuses: Mutex<Vec<TranscribeStatus>>,
    done: Mutex<Vec<(String, bool)>>,
}

impl TranscribeEvents for FakeEvents {
    fn status(&self, s: &TranscribeStatus) {
        self.statuses.lock().push(s.clone());
    }
    fn done(&self, track_id: &str, found: bool) {
        self.done.lock().push((track_id.to_owned(), found));
    }
}

// ---------------------------------------------------------------- fixture

const MODEL_BYTES: usize = 5_000;

fn model_body() -> Vec<u8> {
    (0..MODEL_BYTES).map(|i| (i % 251) as u8).collect()
}

fn specs() -> Vec<ModelSpec> {
    let body = model_body();
    let spec = |kind, file: &str| ModelSpec {
        kind,
        file: file.into(),
        url: format!("https://models.invalid/{file}"),
        size: body.len() as u64,
        sha256: sha256_hex(&body),
    };
    vec![spec(ModelKind::Small, "small.bin"), spec(ModelKind::Medium, "medium.bin")]
}

struct Fx {
    t: Transcriber,
    db: DbHandle,
    models: ModelStore,
    fetch: Arc<FakeFetch>,
    rec: Arc<FakeRecognizer>,
    songs: Arc<FakeSongs>,
    other: Arc<FakeOther>,
    cond: Arc<FakeConditions>,
    events: Arc<FakeEvents>,
    free: Arc<Mutex<Option<u64>>>,
    dir: tempfile::TempDir,
}

fn words(text: &str, at: u32) -> Segment {
    Segment {
        words: text
            .split(' ')
            .enumerate()
            .map(|(i, w)| Word { start_ms: at + i as u32 * 300, end_ms: at + i as u32 * 300 + 250, text: format!(" {w}") })
            .collect(),
    }
}

fn fx_with(fetch: Arc<FakeFetch>) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            r#"
            INSERT INTO artists (id, title, sort_title) VALUES ('7', 'Test Artist', 'Test Artist');
            INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, track_count)
              VALUES ('50', 'First Light', 'First Light', '7', 'Test Artist', 'album', '[]', 2),
                     ('80', 'Basement Tapes', 'Basement Tapes', '7', 'Test Artist', 'album', '["demo"]', 3);
            INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, track_number)
              VALUES ('501', 'Opening', '50', 'First Light', '7', 'Test Artist', 200500, '/p/1', 1),
                     ('502', 'Closing', '50', 'First Light', '7', 'Test Artist', 200500, '/p/2', 2),
                     ('801', 'Tape One', '80', 'Basement Tapes', '7', 'Test Artist', 190000, '/p/3', 1),
                     ('802', 'Tape Two', '80', 'Basement Tapes', '7', 'Test Artist', 190000, '/p/4', 2),
                     ('803', 'Tape Three', '80', 'Basement Tapes', '7', 'Test Artist', 190000, '/p/5', 3);
            "#,
        )?;
        Ok(())
    })
    .unwrap();
    let free: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(None));
    let models = {
        let free = free.clone();
        ModelStore::new(dir.path().join("models"), specs(), Arc::new(move |_: &Path| *free.lock()))
    };
    let rec = FakeRecognizer::new(Behaviour::Words(vec![words("hello there", 1000), words("second line", 4000)]));
    let songs = Arc::new(FakeSongs { unreachable: AtomicBool::new(false), unreadable: AtomicBool::new(false), reads: AtomicUsize::new(0) });
    let other = Arc::new(FakeOther::default());
    let cond = Arc::new(FakeConditions::default());
    let events = Arc::new(FakeEvents::default());
    let t = Transcriber::new(Deps {
        db: db.clone(),
        models: models.clone(),
        fetch: fetch.clone(),
        recognizer: rec.clone(),
        songs: songs.clone(),
        other: other.clone(),
        conditions: cond.clone(),
        events: events.clone(),
    });
    t.set_options(true, ModelKind::Small);
    t.spawn();
    Fx { t, db, models, fetch, rec, songs, other, cond, events, free, dir }
}

fn fx() -> Fx {
    fx_with(FakeFetch::new(model_body()))
}

/// Waits (up to 5 s) until the status matches.
async fn until(t: &Transcriber, what: &str, f: impl Fn(&TranscribeStatus) -> bool) -> TranscribeStatus {
    for _ in 0..500 {
        let s = t.status();
        if f(&s) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}: {:?}", t.status());
}

fn idle(s: &TranscribeStatus) -> bool {
    s.current.is_none() && s.queued == 0
}

async fn install(f: &Fx) {
    f.t.download_model(ModelKind::Small).unwrap();
    until(&f.t, "model", |s| s.models[0].installed && !s.models[0].downloading).await;
}

// ---------------------------------------------------------------- models

#[tokio::test]
async fn model_downloads_with_progress_and_is_verified() {
    let f = fx();
    let s = f.t.status();
    assert_eq!(s.models.len(), 2);
    assert_eq!(s.models[0].size_bytes, MODEL_BYTES as u64, "the size is known before downloading");
    assert!(!s.models[0].installed);
    install(&f).await;
    assert!(f.models.installed(ModelKind::Small).is_some());
    assert!(f.events.statuses.lock().iter().any(|s| s.models[0].downloading), "progress was reported");
    assert!(!f.dir.path().join("models/small.bin.part").exists());
    // Delete removes it.
    f.t.delete_model(ModelKind::Small).unwrap();
    assert!(!f.t.status().models[0].installed);
}

#[tokio::test]
async fn checksum_mismatch_deletes_the_download() {
    let mut bad = model_body();
    bad[10] ^= 0xff;
    let f = fx_with(FakeFetch::new(bad));
    f.t.download_model(ModelKind::Small).unwrap();
    let s = until(&f.t, "failure", |s| s.error.is_some()).await;
    assert_eq!(s.error.unwrap().kind, FailureKind::Checksum);
    assert!(!s.models[0].installed);
    assert!(!f.dir.path().join("models/small.bin").exists());
    assert!(!f.dir.path().join("models/small.bin.part").exists());
}

#[tokio::test]
async fn failed_download_reports_and_leaves_nothing() {
    let f = fx();
    *f.fetch.fail_after.lock() = Some(2);
    f.t.download_model(ModelKind::Small).unwrap();
    let s = until(&f.t, "failure", |s| s.error.is_some()).await;
    assert_eq!(s.error.unwrap().kind, FailureKind::Download);
    assert!(!f.dir.path().join("models/small.bin.part").exists());
    // The server refusing is the same failure; trying again works.
    *f.fetch.fail_after.lock() = None;
    f.fetch.refuse.store(true, Ordering::Relaxed);
    f.t.download_model(ModelKind::Small).unwrap();
    let s = until(&f.t, "failure", |s| s.error.is_some() && s.models[0].downloading.eq(&false)).await;
    assert_eq!(s.error.unwrap().kind, FailureKind::Download);
    f.fetch.refuse.store(false, Ordering::Relaxed);
    install(&f).await;
    assert!(f.t.status().error.is_none());
}

#[tokio::test]
async fn out_of_disk_stops_before_downloading() {
    let f = fx();
    *f.free.lock() = Some(1000);
    f.t.download_model(ModelKind::Small).unwrap();
    let s = until(&f.t, "failure", |s| s.error.is_some()).await;
    let e = s.error.unwrap();
    assert_eq!(e.kind, FailureKind::Disk);
    assert!(e.message.contains("MB"), "{}", e.message);
    assert!(!f.dir.path().join("models").join("small.bin.part").exists());
}

#[tokio::test]
async fn download_can_be_cancelled() {
    let fetch = Arc::new(FakeFetch {
        body: Mutex::new(model_body()),
        fail_after: Mutex::new(None),
        refuse: AtomicBool::new(false),
        gap: Duration::from_millis(20),
    });
    let f = fx_with(fetch);
    f.t.download_model(ModelKind::Small).unwrap();
    until(&f.t, "downloading", |s| s.models[0].downloading).await;
    assert_eq!(f.t.download_model(ModelKind::Medium).unwrap_err().kind(), "invalid", "one at a time");
    f.t.cancel_download();
    let s = until(&f.t, "stopped", |s| !s.models[0].downloading).await;
    assert!(s.error.is_none(), "cancelling isn't an error");
    assert!(!s.models[0].installed);
}

#[tokio::test]
async fn store_download_maps_disk_and_checksum_directly() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path().to_path_buf(), specs(), Arc::new(|_: &Path| Some(10)));
    let never = AtomicBool::new(false);
    let r = store.download(ModelKind::Small, FakeFetch::new(model_body()).as_ref(), &never, &|_, _| {}).await;
    assert!(matches!(r, Err(DownloadError::Disk { .. })));
    let store = ModelStore::new(dir.path().to_path_buf(), specs(), Arc::new(|_: &Path| None));
    let mut long = model_body();
    long.push(0);
    let r = store.download(ModelKind::Small, FakeFetch::new(long).as_ref(), &never, &|_, _| {}).await;
    assert_eq!(r, Err(DownloadError::Checksum), "more bytes than expected");
}

#[test]
fn the_catalogue_is_pinned_and_checksummed() {
    for s in model::catalogue() {
        assert!(s.url.starts_with("https://huggingface.co/ggerganov/whisper.cpp/resolve/"));
        assert!(!s.url.contains("/main/"), "pinned to a commit");
        assert_eq!(s.sha256.len(), 64);
        assert!(s.size > 100_000_000);
    }
}

// ---------------------------------------------------------------- songs

#[tokio::test]
async fn a_song_is_transcribed_and_opens_in_the_editor() {
    let f = fx();
    install(&f).await;
    f.t.transcribe_track("801").unwrap();
    until(&f.t, "done", |s| idle(s) && !f.events.done.lock().is_empty()).await;
    assert_eq!(*f.events.done.lock(), vec![("801".to_string(), true)]);
    // 2 s of audio at 16 kHz mono.
    let n = f.rec.last_len.load(Ordering::SeqCst);
    assert!((RATE as usize * 2 - 10..=RATE as usize * 2 + 10).contains(&n), "{n}");
    let own = f.db.read(|c| user::get(c, "801")).unwrap().unwrap();
    assert_eq!(own.auto.as_deref(), Some("small"));
    assert!(own.text.starts_with("[00:01.00]<00:01.00>Hello <00:01.30>there"), "{}", own.text);
    let l = own.lyrics().unwrap();
    assert_eq!(l.source, crate::plex::details::LyricsSource::Auto);
    assert_eq!(l.timed.len(), 2);
    // Asked again: already done, nothing runs.
    f.t.transcribe_track("801").unwrap();
    until(&f.t, "idle", idle).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn no_words_found_is_remembered() {
    let f = fx();
    install(&f).await;
    *f.rec.behaviour.lock() = Behaviour::Words(vec![words("[Music]", 0)]);
    f.t.transcribe_track("801").unwrap();
    until(&f.t, "done", |s| idle(s) && !f.events.done.lock().is_empty()).await;
    assert_eq!(*f.events.done.lock(), vec![("801".to_string(), false)]);
    assert!(f.db.read(|c| store::found_nothing(c, "801")).unwrap());
    assert!(f.db.read(|c| user::get(c, "801")).unwrap().is_none());
    assert_eq!(f.db.read(store::unreleased_todo).unwrap(), vec!["802", "803"]);
    // Forgetting lets it run again.
    assert!(f.t.forget("801").await.unwrap());
    assert!(!f.db.read(|c| store::done(c, "801")).unwrap());
}

#[test]
fn the_todo_list_picks_up_albums_plex_couldnt_match() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            r#"
            INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, studio, matched)
              VALUES ('50', 'First Light', 'First Light', '7', 'A', 'album', '[]', 'Night Records', 1),
                     ('60', 'Loose Ends', 'Loose Ends', '7', 'A', NULL, '[]', NULL, 0),
                     ('70', 'Old Cache', 'Old Cache', '7', 'A', NULL, '[]', NULL, NULL),
                     ('71', 'Old Label', 'Old Label', '7', 'A', NULL, '[]', 'Night Records', NULL);
            INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, track_number)
              VALUES ('501', 'Opening', '50', 'First Light', '7', 'A', 1000, '/p/1', 1),
                     ('601', 'Loose One', '60', 'Loose Ends', '7', 'A', 1000, '/p/6', 1),
                     ('602', 'Loose Two', '60', 'Loose Ends', '7', 'A', 1000, '/p/7', 2),
                     ('701', 'Old One', '70', 'Old Cache', '7', 'A', 1000, '/p/8', 1),
                     ('711', 'Labelled', '71', 'Old Label', '7', 'A', 1000, '/p/9', 1);
            INSERT INTO file_tags (part_key, status, version, scanned_at) VALUES ('/p/7', 'ok', 'released', 1);
            "#,
        )?;
        Ok(())
    })
    .unwrap();
    // A file tagged released wins over the guess.
    assert_eq!(db.read(store::unreleased_todo).unwrap(), vec!["601", "701"]);
    crate::db::release::set_count_unmatched(false);
    assert!(db.read(store::unreleased_todo).unwrap().is_empty(), "off: only real tags count");
    crate::db::release::set_count_unmatched(true);
}

#[tokio::test]
async fn transcription_failure_is_reported_for_the_song() {
    let f = fx();
    install(&f).await;
    *f.rec.behaviour.lock() = Behaviour::Fail;
    f.t.transcribe_track("801").unwrap();
    let s = until(&f.t, "failure", |s| s.error.is_some() && idle(s)).await;
    let e = s.error.unwrap();
    assert_eq!(e.kind, FailureKind::Transcribe);
    assert_eq!(e.track_id.as_deref(), Some("801"));
    assert!(!f.db.read(|c| store::done(c, "801")).unwrap(), "a failure isn't remembered as done");
}

#[tokio::test]
async fn unreadable_or_unreachable_files_fail_the_song() {
    let f = fx();
    install(&f).await;
    f.songs.unreadable.store(true, Ordering::Relaxed);
    f.t.transcribe_track("801").unwrap();
    let s = until(&f.t, "failure", |s| s.error.is_some() && idle(s)).await;
    assert!(s.error.unwrap().message.contains("couldn't be"));
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 0);
    f.songs.unreadable.store(false, Ordering::Relaxed);
    f.songs.unreachable.store(true, Ordering::Relaxed);
    f.t.transcribe_track("802").unwrap();
    let s = until(&f.t, "failure", |s| s.error.as_ref().is_some_and(|e| e.track_id.as_deref() == Some("802")) && idle(s)).await;
    assert!(s.error.unwrap().message.contains("server"));
}

#[tokio::test]
async fn asking_without_a_model_downloads_it_first() {
    let f = fx_with(Arc::new(FakeFetch {
        body: Mutex::new(model_body()),
        fail_after: Mutex::new(None),
        refuse: AtomicBool::new(false),
        gap: Duration::from_millis(20),
    }));
    f.t.transcribe_track("801").unwrap();
    until(&f.t, "done", |s| idle(s) && !f.events.done.lock().is_empty()).await;
    assert!(f.models.installed(ModelKind::Small).is_some());
    // The song ran after the model arrived. The "waiting for the model" status
    // is only there for a moment, so a busy machine can miss it; it isn't
    // asserted here.
    assert!(f.rec.runs.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn a_failed_model_download_drops_the_waiting_song() {
    let f = fx();
    f.fetch.refuse.store(true, Ordering::Relaxed);
    f.t.transcribe_track("801").unwrap();
    let s = until(&f.t, "failure", |s| s.error.is_some() && idle(s)).await;
    assert_eq!(s.error.unwrap().kind, FailureKind::Download);
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn album_skips_songs_with_lyrics_elsewhere() {
    let f = fx();
    install(&f).await;
    f.other.0.lock().push("802".into());
    assert_eq!(f.t.transcribe_album("80").await.unwrap(), 3);
    until(&f.t, "done", |s| idle(s) && f.events.done.lock().len() == 2).await;
    let done: Vec<String> = f.events.done.lock().iter().map(|d| d.0.clone()).collect();
    assert_eq!(done, vec!["801", "803"]);
    assert_eq!(f.t.status().batch, BatchProgress::default(), "progress resets once the queue empties");
}

#[tokio::test]
async fn the_background_batch_takes_unreleased_songs_and_waits_on_battery() {
    let f = fx();
    install(&f).await;
    *f.cond.0.lock() = Some(PauseReason::Battery);
    assert_eq!(f.t.transcribe_unreleased().await.unwrap(), 3, "the demo album's songs only");
    let s = until(&f.t, "paused", |s| s.paused == Some(PauseReason::Battery)).await;
    assert_eq!(s.batch.total, 3);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(f.songs.reads.load(Ordering::SeqCst), 0, "nothing is read while paused");
    // Plugged in: the batch runs at the lower thread count.
    *f.cond.0.lock() = None;
    let s = until(&f.t, "done", |s| idle(s) && f.events.done.lock().len() == 3).await;
    assert_eq!(s.paused, None);
    assert_eq!(f.rec.last_threads.load(Ordering::SeqCst), engine::threads(true));
    assert!(f.db.read(store::unreleased_todo).unwrap().is_empty());
}

#[tokio::test]
async fn songs_asked_for_go_before_the_batch() {
    let f = fx();
    install(&f).await;
    *f.cond.0.lock() = Some(PauseReason::Battery);
    f.t.transcribe_unreleased().await.unwrap();
    until(&f.t, "paused", |s| s.paused.is_some()).await;
    f.t.transcribe_track("501").unwrap();
    let s = f.t.status();
    assert_eq!(s.queued, 3, "two batch songs and the one asked for");
    // Cancel the paused batch song; the asked-for song runs next even on battery.
    let current = s.current.unwrap().track_id;
    f.t.cancel(Some(&current));
    until(&f.t, "501 done", |_| f.events.done.lock().iter().any(|d| d.0 == "501")).await;
    assert_eq!(f.rec.last_threads.load(Ordering::SeqCst), engine::threads(false));
    f.t.cancel(None);
}

#[tokio::test]
async fn cancel_stops_the_running_song_and_turning_off_clears_the_queue() {
    let f = fx();
    install(&f).await;
    *f.rec.behaviour.lock() = Behaviour::Block;
    f.t.transcribe_album("80").await.unwrap();
    until(&f.t, "transcribing", |s| s.current.as_ref().is_some_and(|c| c.stage == Stage::Transcribing)).await;
    f.t.cancel(Some("801"));
    let s = until(&f.t, "next song", |s| s.current.as_ref().is_some_and(|c| c.track_id == "802" && c.stage == Stage::Transcribing)).await;
    assert_eq!(s.queued, 1);
    f.t.set_options(false, ModelKind::Small);
    let s = until(&f.t, "stopped", idle).await;
    assert!(!s.enabled);
    assert!(f.events.done.lock().is_empty());
    assert!(!f.db.read(|c| store::done(c, "801")).unwrap());
    // Off refuses new work.
    assert_eq!(f.t.transcribe_track("801").unwrap_err().kind(), "invalid");
}

#[tokio::test]
async fn a_transcription_never_replaces_lyrics_the_user_saved() {
    let f = fx();
    install(&f).await;
    let now = crate::db::queries::now_s();
    f.db.write(|c| user::put(c, "801", "Mine", false, now)).unwrap();
    f.t.transcribe_track("801").unwrap();
    until(&f.t, "idle", idle).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 0, "a song with the user's lyrics is skipped");
    assert!(!f.db.write(|c| store::save(c, "801", "[00:01.00]x\n", "small", now)).unwrap());
    assert_eq!(f.db.read(|c| user::get(c, "801")).unwrap().unwrap().text, "Mine");
}

// ---------------------------------------------------------------- audio

#[test]
fn decodes_to_16k_mono() {
    let never = AtomicBool::new(false);
    let pcm = audio::to_16k_mono(wav(), Some("wav"), &never).unwrap();
    assert_eq!(pcm.len(), 32_000);
    let peak = pcm.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.2 && peak < 0.3, "{peak}");
    assert!(matches!(audio::to_16k_mono(b"nope".to_vec(), None, &never), Err(AudioError::Unreadable(_))));
}

mod read_whole {
    use super::*;
    use crate::audio::StreamSource;
    use crate::audio::gate::{Priority, TransferGate};
    use crate::tags::fetch::Reader;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    struct Ranged(Vec<u8>);

    impl Respond for Ranged {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let f = &self.0;
            let (a, b) = req
                .headers
                .get("range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    let (a, b) = v.strip_prefix("bytes=")?.split_once('-')?;
                    Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                })
                .unwrap();
            let end = b.min(f.len() - 1);
            ResponseTemplate::new(206)
                .insert_header("content-range", format!("bytes {a}-{end}/{}", f.len()).as_str())
                .set_body_bytes(f[a..=end].to_vec())
        }
    }

    #[tokio::test]
    async fn reads_the_whole_file_in_ranges_at_background_priority() {
        let body: Vec<u8> = (0..(audio::CHUNK * 2 + 1234)).map(|i| (i % 255) as u8).collect();
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("GET")).respond_with(Ranged(body.clone())).mount(&server).await;
        let gate = TransferGate::new();
        let reader = Reader::new(crate::audio::http::http_client(), gate.clone(), None);
        let src = StreamSource { url: format!("{}/f.flac", server.uri()), ..Default::default() };
        let never = AtomicBool::new(false);
        let got = audio::read_whole(&reader, &src, &never).await.unwrap();
        assert_eq!(got, body);
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
        // While playback holds the gate, the read waits.
        let held = gate.acquire(&Priority::Current.cell()).await;
        let r2 = reader.clone();
        let s2 = src.clone();
        let t = tokio::spawn(async move {
            let never = AtomicBool::new(false);
            audio::read_whole(&r2, &s2, &never).await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!t.is_finished());
        drop(held);
        assert_eq!(t.await.unwrap().unwrap().len(), body.len());
    }
}

// ---------------------------------------------------------------- real model

/// A real run, for timing. Needs the `transcribe` feature, a model file and
/// a WAV clip:
/// `PLATTER_WHISPER_MODEL=path PLATTER_TEST_AUDIO=path cargo test --release
/// --features transcribe real_model -- --ignored --nocapture`
#[cfg(feature = "transcribe")]
#[test]
#[ignore]
fn real_model_speed() {
    let model = std::env::var("PLATTER_WHISPER_MODEL").expect("PLATTER_WHISPER_MODEL");
    let clip = std::env::var("PLATTER_TEST_AUDIO").expect("PLATTER_TEST_AUDIO");
    let kind = if model.contains("medium") { ModelKind::Medium } else { ModelKind::Small };
    let threads: usize = std::env::var("PLATTER_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(engine::threads(false));
    let never = AtomicBool::new(false);
    let pcm = audio::to_16k_mono(std::fs::read(&clip).unwrap(), None, &never).unwrap();
    let rec = engine::default_recognizer();
    assert_eq!(rec.unavailable(), None);
    let started = std::time::Instant::now();
    let segs = rec
        .transcribe(Path::new(&model), kind, &pcm, threads, Arc::new(AtomicBool::new(false)), Arc::new(|_| {}))
        .unwrap();
    let took = started.elapsed();
    let lines = lines::to_lines(&segs);
    println!("audio {:.0} s, {threads} threads, took {:.1} s, {} lines", pcm.len() as f32 / RATE as f32, took.as_secs_f32(), lines.len());
    for l in lines.iter().take(8) {
        println!("{:>7} ms  {}", l.start_ms, l.text());
    }
}
