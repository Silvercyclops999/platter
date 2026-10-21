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
use super::engine::{Hints, Progress, RecognizeError, Recognizer, Step, Stop};
use super::lines::{Segment, Word};
use super::model::{DownloadError, ModelFetch, ModelKind, ModelSpec, ModelStore, sha256_hex};
use super::pace::fake::FakeLoad;
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
    /// The helper process stopped.
    Crash,
    /// Runs with fewer threads than this block until cancelled (a long
    /// background song); runs with more give these words.
    BlockBelow(usize, Vec<Segment>),
}

struct FakeRecognizer {
    behaviour: Mutex<Behaviour>,
    runs: AtomicUsize,
    /// Samples given on the last run.
    last_len: AtomicUsize,
    last_threads: AtomicUsize,
    crashed_too_often: AtomicBool,
    /// Runs after this many block until cancelled.
    hold_after: AtomicUsize,
    /// What the last run was told about the song.
    last_hints: Mutex<Option<Hints>>,
}

impl FakeRecognizer {
    fn new(b: Behaviour) -> Arc<Self> {
        Arc::new(Self {
            behaviour: Mutex::new(b),
            runs: AtomicUsize::new(0),
            last_len: AtomicUsize::new(0),
            last_threads: AtomicUsize::new(0),
            crashed_too_often: AtomicBool::new(false),
            hold_after: AtomicUsize::new(usize::MAX),
            last_hints: Mutex::new(None),
        })
    }
}

impl Recognizer for FakeRecognizer {
    fn unavailable(&self) -> Option<String> {
        None
    }
    fn transcribe(
        &self,
        model: &Path,
        kind: ModelKind,
        pcm: &[f32],
        threads: usize,
        hints: &Hints,
        cancel: Stop,
        progress: Progress,
    ) -> Result<Vec<Segment>, RecognizeError> {
        assert!(model.is_file(), "the model is on disk");
        assert_eq!(hints.language, if kind.multilingual() { engine::DETECT } else { engine::LANGUAGE });
        *self.last_hints.lock() = Some(hints.clone());
        let run = self.runs.fetch_add(1, Ordering::SeqCst) + 1;
        if run > self.hold_after.load(Ordering::SeqCst) {
            *self.behaviour.lock() = Behaviour::Block;
        }
        self.last_len.store(pcm.len(), Ordering::SeqCst);
        self.last_threads.store(threads, Ordering::SeqCst);
        progress(Step::Percent(50));
        let behaviour = self.behaviour.lock().clone();
        match behaviour {
            Behaviour::Words(s) => Ok(s),
            Behaviour::Fail => Err(RecognizeError::Failed("boom".into())),
            Behaviour::Block => {
                while !cancel.is_set() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(RecognizeError::Cancelled)
            }
            Behaviour::Crash => Err(RecognizeError::Crashed),
            Behaviour::BlockBelow(n, s) => {
                if threads >= n {
                    return Ok(s);
                }
                while !cancel.is_set() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(RecognizeError::Cancelled)
            }
        }
    }
    fn crashed_too_often(&self) -> bool {
        self.crashed_too_often.load(Ordering::SeqCst)
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
    /// Reads wait while this is set (playback has the connection).
    hold: AtomicBool,
}

#[async_trait]
impl SongReader for FakeSongs {
    async fn read(&self, _track_id: &str, _cancel: &AtomicBool) -> Result<SongAudio, AudioError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.hold.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        if self.unreachable.load(Ordering::Relaxed) {
            return Err(AudioError::Unreachable("offline".into()));
        }
        if self.unreadable.load(Ordering::Relaxed) {
            return Ok(SongAudio { bytes: b"not audio at all".to_vec(), hint: None });
        }
        Ok(SongAudio { bytes: wav(), hint: Some("wav".into()) })
    }
    fn yielding(&self) -> bool {
        self.hold.load(Ordering::SeqCst)
    }
}

/// Songs with lyrics elsewhere; what looking for official synced lyrics
/// finds per song (none found when not listed); the songs it was asked about.
#[derive(Default)]
struct FakeOther(Mutex<Vec<String>>, Mutex<Vec<(String, Official)>>, Mutex<Vec<String>>);

#[async_trait]
impl OtherLyrics for FakeOther {
    async fn has_lyrics(&self, track_id: &str) -> bool {
        self.0.lock().iter().any(|t| t == track_id)
    }
    async fn official_synced(&self, track_id: &str) -> Official {
        self.2.lock().push(track_id.to_owned());
        self.1.lock().iter().find(|(t, _)| t == track_id).map_or(Official::NoneMatched, |(_, o)| *o)
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

/// "Separates" by handing back a voice of a length the test can spot.
#[derive(Default)]
struct FakeSeparator {
    runs: AtomicUsize,
    /// Stereo samples given on the last run.
    last_len: AtomicUsize,
    fail: AtomicBool,
}

/// The fake voice's length, so the recognizer's input shows it was heard.
const VOICE_LEN: usize = 12_345;

impl voice::Separator for FakeSeparator {
    fn vocals(&self, weights: &Path, song: &[f32], _stop: Stop, progress: Progress) -> Result<Vec<f32>, voice::VoiceError> {
        assert!(weights.is_file(), "the voice model is on disk");
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.last_len.store(song.len(), Ordering::SeqCst);
        progress(Step::Percent(50));
        if self.fail.load(Ordering::SeqCst) {
            return Err(voice::VoiceError::Crashed);
        }
        Ok(vec![0.1; VOICE_LEN])
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
    vec![spec(ModelKind::Small, "small.bin"), spec(ModelKind::Medium, "medium.bin"), spec(ModelKind::Turbo, "turbo.bin")]
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
    load: Arc<FakeLoad>,
    free: Arc<Mutex<Option<u64>>>,
    gpu: Arc<gpu::Gpu>,
    sep: Arc<FakeSeparator>,
    voice_models: ModelStore<voice::Voice>,
    dir: tempfile::TempDir,
}

fn words(text: &str, at: u32) -> Segment {
    Segment {
        words: text
            .split(' ')
            .enumerate()
            .map(|(i, w)| Word { start_ms: at + i as u32 * 300, end_ms: at + i as u32 * 300 + 250, text: format!(" {w}") })
            .collect(),
        sure: None,
    }
}

fn fx_with(fetch: Arc<FakeFetch>) -> Fx {
    fx_build(fetch, None, gpu::Gpu::none())
}

/// A computer with these graphics cards.
fn fx_gpu(cards: Vec<gpu::Device>) -> Fx {
    fx_build(FakeFetch::new(model_body()), None, gpu_with(cards))
}

fn gpu_with(cards: Vec<gpu::Device>) -> Arc<gpu::Gpu> {
    let g = gpu::Gpu::new(Arc::new(move || cards.clone()));
    g.wait(Duration::from_secs(5));
    g
}

fn card(name: &str, kind: gpu::DeviceKind) -> gpu::Device {
    gpu::Device { index: 0, name: name.into(), kind, memory: 8 << 30 }
}

/// `over`: another recognizer; `gpu`: the graphics cards found.
fn fx_build(fetch: Arc<FakeFetch>, over: Option<Arc<dyn Recognizer>>, gpu: Arc<gpu::Gpu>) -> Fx {
    fx_timed(fetch, over, gpu, pace::fake::timing())
}

/// With these pacing times (upkeep waits an hour in the others).
fn fx_timed(fetch: Arc<FakeFetch>, over: Option<Arc<dyn Recognizer>>, gpu: Arc<gpu::Gpu>, timing: pace::Timing) -> Fx {
    fx_full(fetch, over, gpu, timing, false)
}

/// A graphics card, and the voice separated first.
fn fx_voice() -> Fx {
    fx_full(FakeFetch::new(model_body()), None, gpu_with(vec![card("RTX 5090", gpu::DeviceKind::Discrete)]), pace::fake::timing(), true)
}

fn voice_specs() -> Vec<ModelSpec<voice::Voice>> {
    let body = model_body();
    vec![ModelSpec {
        kind: voice::Voice::Demucs,
        file: "voice.safetensors".into(),
        url: "https://models.invalid/voice.safetensors".into(),
        size: body.len() as u64,
        sha256: sha256_hex(&body),
    }]
}

fn fx_full(fetch: Arc<FakeFetch>, over: Option<Arc<dyn Recognizer>>, gpu: Arc<gpu::Gpu>, timing: pace::Timing, voiced: bool) -> Fx {
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
    let songs = Arc::new(FakeSongs {
        unreachable: AtomicBool::new(false),
        unreadable: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
        hold: AtomicBool::new(false),
    });
    let other = Arc::new(FakeOther::default());
    let cond = Arc::new(FakeConditions::default());
    let events = Arc::new(FakeEvents::default());
    let load = FakeLoad::new(5.0);
    let sep = Arc::new(FakeSeparator::default());
    let voice_models = ModelStore::new(dir.path().join("models"), voice_specs(), Arc::new(|_: &Path| None));
    let t = Transcriber::new(Deps {
        db: db.clone(),
        models: models.clone(),
        fetch: fetch.clone(),
        recognizer: over.unwrap_or_else(|| rec.clone()),
        songs: songs.clone(),
        other: other.clone(),
        conditions: cond.clone(),
        events: events.clone(),
        // A 16-thread computer: Light is 2 threads in the background, 4 for a song asked for.
        pacer: pace::Pacer::with_cores(load.clone(), timing, 16),
        gpu: gpu.clone(),
        speeds: Arc::new(speed::Speeds::load(Some(dir.path().join("speeds.json")))),
        voice: voiced.then(|| VoiceDeps { separator: sep.clone(), models: voice_models.clone() }),
    });
    t.set_options(true, Some(ModelKind::Small));
    t.spawn();
    Fx { t, db, models, fetch, rec, songs, other, cond, events, load, free, gpu, sep, voice_models, dir }
}

fn fx() -> Fx {
    fx_with(FakeFetch::new(model_body()))
}

/// How long a wait gives up after: long, so a slow machine only makes a test slower.
const WAIT: Duration = Duration::from_secs(20);

/// Waits (up to `WAIT`) until the status matches.
async fn until(t: &Transcriber, what: &str, f: impl Fn(&TranscribeStatus) -> bool) -> TranscribeStatus {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let s = t.status();
        if f(&s) {
            return s;
        }
        if std::time::Instant::now() > deadline {
            panic!("timed out waiting for {what}: {s:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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
    assert_eq!(s.models.len(), 3);
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

#[tokio::test]
async fn a_model_changed_after_its_check_isnt_used_and_an_old_one_is_checked_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path().to_path_buf(), specs(), Arc::new(|_: &Path| None));
    let never = AtomicBool::new(false);
    let p = store.download(ModelKind::Small, FakeFetch::new(model_body()).as_ref(), &never, &|_, _| {}).await.unwrap();
    assert!(store.installed(ModelKind::Small).is_some() && !store.needs_verify(ModelKind::Small));
    // Swapped for other bytes of the same size: no longer the checked file.
    let mut other = model_body();
    other[0] ^= 0xff;
    let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
    std::io::Write::write_all(&mut &f, &other).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
    drop(f);
    assert!(store.installed(ModelKind::Small).is_none());
    // A file from before checks were noted: checked once, then noted.
    std::fs::remove_file(dir.path().join("small.bin.verified")).unwrap();
    assert!(store.needs_verify(ModelKind::Small));
    assert!(!store.verify(ModelKind::Small), "the swapped bytes fail");
    assert!(!p.exists(), "and are removed");
    std::fs::write(&p, model_body()).unwrap();
    assert!(store.verify(ModelKind::Small));
    assert!(store.installed(ModelKind::Small).is_some() && !store.needs_verify(ModelKind::Small));
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
async fn the_model_hears_the_title_and_unsure_lines_are_left_out() {
    let f = fx();
    install(&f).await;
    let mut guess = words("banana river", 3000);
    guess.sure = Some(0.1);
    let mut sung = words("hello there friend", 1000);
    sung.sure = Some(0.9);
    *f.rec.behaviour.lock() = Behaviour::Words(vec![sung, guess, words("second line", 5000)]);
    f.t.transcribe_track("801").unwrap();
    until(&f.t, "done", |s| idle(s) && !f.events.done.lock().is_empty()).await;
    assert_eq!(*f.rec.last_hints.lock(), Some(Hints { language: "en".into(), prompt: "Tape One by Test Artist.".into() }));
    let own = f.db.read(|c| user::get(c, "801")).unwrap().unwrap();
    assert!(own.text.contains("Hello") && own.text.contains("Second"), "{}", own.text);
    assert!(!own.text.contains("Banana"), "the unsure line is left out: {}", own.text);
}

#[tokio::test]
async fn a_mostly_unsure_song_is_saved_with_nothing_found() {
    let f = fx();
    install(&f).await;
    let mut guess = words("la la la la", 1000);
    guess.sure = Some(0.2);
    *f.rec.behaviour.lock() = Behaviour::Words(vec![guess, words("hey", 5000)]);
    f.t.transcribe_track("801").unwrap();
    until(&f.t, "done", |s| idle(s) && !f.events.done.lock().is_empty()).await;
    assert_eq!(*f.events.done.lock(), vec![("801".to_string(), false)]);
    assert!(f.db.read(|c| user::get(c, "801")).unwrap().is_none(), "no wrong lyrics shown");
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
    assert_eq!(f.rec.last_threads.load(Ordering::SeqCst), 2);
    assert!(f.db.read(store::unreleased_todo).unwrap().is_empty());
}

#[tokio::test]
async fn a_bridge_doing_the_lyrics_holds_the_batch_but_not_songs_asked_for() {
    let f = fx();
    install(&f).await;
    let bridge = Arc::new(AtomicBool::new(true));
    {
        let b = bridge.clone();
        f.t.set_defer(Arc::new(move || b.load(Ordering::SeqCst)));
    }
    assert_eq!(f.t.transcribe_unreleased().await.unwrap(), 3);
    let s = until(&f.t, "left to the Bridge", |s| s.paused == Some(PauseReason::Bridge)).await;
    assert_eq!(s.batch.total, 3);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(f.songs.reads.load(Ordering::SeqCst), 0, "this computer doesn't do the Bridge's work");
    // The song someone plays (the Bridge hasn't got to it yet) runs here, at once.
    f.t.transcribe_track("501").unwrap();
    until(&f.t, "501 done", |_| f.events.done.lock().iter().any(|d| d.0 == "501")).await;
    // So does the quiet one for a song whose lyrics didn't match.
    f.t.transcribe_quietly("502");
    until(&f.t, "502 done", |_| f.events.done.lock().iter().any(|d| d.0 == "502")).await;
    // Once that song is through (its done event comes just before the
    // worker settles), the batch still waits.
    until(&f.t, "the batch still waits", |s| s.paused == Some(PauseReason::Bridge)).await;
    assert_eq!(f.events.done.lock().len(), 2, "and nothing of the batch ran here");
    // The Bridge went quiet (or a problem): the batch goes on here.
    bridge.store(false, Ordering::SeqCst);
    f.t.poke();
    until(&f.t, "batch done", |s| idle(s) && f.events.done.lock().len() == 5).await;
    assert!(f.db.read(store::unreleased_todo).unwrap().is_empty());
}

// ---------------------------------------------------------------- following playback

#[tokio::test]
async fn the_playing_song_and_the_next_without_lyrics_are_written_down_model_first() {
    let f = fx();
    // "502" has lyrics elsewhere; no model yet: following downloads it.
    f.other.0.lock().push("502".into());
    f.t.follow(Some("501".into()), vec!["502".into(), "503".into(), "504".into()]).await;
    let s = f.t.status();
    assert!(s.models[0].downloading || s.models[0].installed, "the model downloads by itself");
    until(&f.t, "501 and 503 done", |s| idle(s) && f.events.done.lock().len() == 2).await;
    let done: Vec<String> = f.events.done.lock().iter().map(|d| d.0.clone()).collect();
    assert_eq!(done, ["501", "503"], "the playing song first, the one with lyrics skipped, two ahead at most");
    // Already done: asking again does nothing.
    f.t.follow(Some("501".into()), vec![]).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(idle(&f.t.status()));
    assert_eq!(f.events.done.lock().len(), 2);
}

#[tokio::test]
async fn following_waits_while_paused_and_skipping_past_stops_the_song() {
    let f = fx();
    install(&f).await;
    let words = f.rec.behaviour.lock().clone();
    // Paused: songs followed for playback wait like background work.
    f.t.pause();
    f.t.follow(Some("601".into()), vec!["602".into()]).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 0, "nothing runs while paused");
    assert_eq!(f.t.status().queued, 2);
    // Blocked before resuming, so 601 can't finish before the skip.
    *f.rec.behaviour.lock() = Behaviour::Block;
    f.t.resume();
    until(&f.t, "601 running", |s| s.current.as_ref().is_some_and(|c| c.track_id == "601" && c.stage == Stage::Transcribing)).await;
    *f.rec.behaviour.lock() = words;
    // The listener skipped ahead: 601 stops and 602 leaves the queue.
    f.t.follow(Some("701".into()), vec![]).await;
    until(&f.t, "701 done", |s| idle(s) && f.events.done.lock().iter().any(|d| d.0 == "701")).await;
    assert!(!f.events.done.lock().iter().any(|d| d.0 == "601" || d.0 == "602"));
}

#[tokio::test]
async fn asking_for_a_followed_song_makes_it_the_users() {
    let f = fx();
    install(&f).await;
    f.t.pause();
    f.t.follow(Some("601".into()), vec!["602".into()]).await;
    // Asked for while it waits as an upcoming song: it runs now and isn't
    // dropped when playback moves on.
    f.t.transcribe_track("602").unwrap();
    f.t.follow(Some("701".into()), vec![]).await;
    until(&f.t, "602 done", |_| f.events.done.lock().iter().any(|d| d.0 == "602")).await;
}

#[tokio::test]
async fn a_cancelled_or_failed_song_isnt_followed_again() {
    let f = fx();
    install(&f).await;
    let words = f.rec.behaviour.lock().clone();
    *f.rec.behaviour.lock() = Behaviour::Fail;
    f.t.follow(Some("601".into()), vec![]).await;
    let s = until(&f.t, "601 failed", |s| idle(s) && s.error.is_some()).await;
    assert_eq!(s.error.as_ref().and_then(|e| e.track_id.as_deref()), Some("601"));
    *f.rec.behaviour.lock() = words.clone();
    // Playing it again neither retries it nor clears the failure.
    f.t.follow(Some("601".into()), vec![]).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let s = f.t.status();
    assert!(idle(&s) && s.error.is_some());
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 1);
    // Cancelled by the user: the same.
    *f.rec.behaviour.lock() = Behaviour::Block;
    f.t.follow(Some("602".into()), vec![]).await;
    until(&f.t, "602 running", |s| s.current.as_ref().is_some_and(|c| c.track_id == "602" && c.stage == Stage::Transcribing)).await;
    f.t.cancel(Some("602"));
    *f.rec.behaviour.lock() = words;
    until(&f.t, "stopped", idle).await;
    f.t.follow(Some("602".into()), vec![]).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(idle(&f.t.status()));
    // Asking for it again clears that.
    f.t.transcribe_track("602").unwrap();
    until(&f.t, "602 done", |_| f.events.done.lock().iter().any(|d| d.0 == "602")).await;
}

#[tokio::test]
async fn following_is_quiet_when_transcription_is_off() {
    let f = fx();
    f.t.set_options(false, None);
    f.t.follow(Some("501".into()), vec!["502".into()]).await;
    let s = f.t.status();
    assert!(idle(&s) && s.error.is_none() && !s.models[0].downloading);
}

/// Results from the user's other devices: "802" was transcribed elsewhere.
#[derive(Default)]
struct FakeShared {
    saved: Mutex<Vec<(String, String)>>,
    forgot: Mutex<Vec<String>>,
}

#[async_trait]
impl SharedResults for FakeShared {
    async fn take(&self, track_id: &str) -> Option<bool> {
        (track_id == "802").then_some(true)
    }
    async fn saved(&self, track_id: &str, _body: &str, model: &str) {
        self.saved.lock().push((track_id.to_owned(), model.to_owned()));
    }
    async fn forgot(&self, track_id: &str) {
        self.forgot.lock().push(track_id.to_owned());
    }
}

#[tokio::test]
async fn the_batch_skips_songs_transcribed_on_another_device() {
    let f = fx();
    let shared = Arc::new(FakeShared::default());
    f.t.set_shared(shared.clone());
    install(&f).await;
    assert_eq!(f.t.transcribe_unreleased().await.unwrap(), 3);
    until(&f.t, "done", |s| idle(s) && f.events.done.lock().len() == 3).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 2, "802 came from another device");
    let mut saved = shared.saved.lock().clone();
    saved.sort();
    assert_eq!(saved, vec![("801".to_string(), "small".to_string()), ("803".to_string(), "small".to_string())]);
    assert!(f.events.done.lock().contains(&("802".to_string(), true)));
    f.t.forget("801").await.unwrap();
    assert_eq!(*shared.forgot.lock(), vec!["801".to_string()]);
}

#[tokio::test]
async fn songs_asked_for_go_before_the_batch() {
    let f = fx();
    install(&f).await;
    *f.cond.0.lock() = Some(PauseReason::Battery);
    f.t.transcribe_unreleased().await.unwrap();
    let s = until(&f.t, "paused", |s| s.paused.is_some()).await;
    let waiting = s.current.unwrap().track_id;
    // The asked-for song runs at once, even on battery, with more threads.
    f.t.transcribe_track("501").unwrap();
    until(&f.t, "501 done", |_| f.events.done.lock().iter().any(|d| d.0 == "501")).await;
    assert_eq!(f.rec.last_threads.load(Ordering::SeqCst), 4);
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 1, "only the song asked for ran");
    // Then the batch song it gave way to waits again.
    let s = until(&f.t, "paused again", |s| s.paused == Some(PauseReason::Battery)).await;
    assert_eq!(s.current.unwrap().track_id, waiting);
    assert_eq!((s.batch.total, s.queued), (4, 2));
    f.t.cancel(None);
}

#[tokio::test]
async fn a_song_asked_for_stops_the_batch_song_in_progress_which_resumes_after() {
    let f = fx();
    install(&f).await;
    // Background songs (two threads) take forever; the song asked for (four) doesn't.
    *f.rec.behaviour.lock() = Behaviour::BlockBelow(4, vec![words("hello there", 1000)]);
    f.t.transcribe_album("80").await.unwrap();
    until(&f.t, "801 transcribing", |s| s.current.as_ref().is_some_and(|c| c.track_id == "801" && c.stage == Stage::Transcribing)).await;
    let reads = f.songs.reads.load(Ordering::SeqCst);
    f.t.transcribe_track("501").unwrap();
    // The asked-for song runs (the batch song would block forever), and it finishes.
    until_true("the song asked for started", || {
        f.t.status().current.is_some_and(|c| c.track_id == "501") || f.events.done.lock().iter().any(|d| d.0 == "501")
    })
    .await;
    until(&f.t, "501 done", |_| f.events.done.lock().iter().any(|d| d.0 == "501")).await;
    // Then the batch song goes on, from the audio already read.
    let s = until(&f.t, "801 again", |s| s.current.as_ref().is_some_and(|c| c.track_id == "801" && c.stage == Stage::Transcribing)).await;
    assert_eq!(f.songs.reads.load(Ordering::SeqCst), reads + 1, "only the asked-for song was read");
    assert_eq!(s.queued, 2);
    assert!(s.error.is_none());
    f.t.cancel(None);
    until(&f.t, "stopped", idle).await;
}

#[tokio::test]
async fn background_work_waits_while_the_computer_is_busy() {
    let f = fx();
    install(&f).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(f.load.reads(), 0, "nothing reads the processor while the queue is empty");
    f.load.set(90.0);
    f.t.transcribe_album("80").await.unwrap();
    let s = until(&f.t, "paused", |s| s.paused == Some(PauseReason::Busy)).await;
    assert_eq!(s.current.unwrap().track_id, "801");
    assert_eq!(f.songs.reads.load(Ordering::SeqCst), 0, "nothing is read while paused");
    // The song someone asks for still runs.
    f.t.transcribe_track("501").unwrap();
    until(&f.t, "501 done", |_| f.events.done.lock().iter().any(|d| d.0 == "501")).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 1);
    until(&f.t, "paused again", |s| s.paused == Some(PauseReason::Busy)).await;
    // Calm again (for the resume time): the album goes on.
    f.load.set(5.0);
    until(&f.t, "album done", |s| idle(s) && f.events.done.lock().len() == 4).await;
    assert_eq!(f.t.status().paused, None);
    assert_eq!(f.rec.last_threads.load(Ordering::SeqCst), 2);
    // Done: the readings stop.
    tokio::time::sleep(Duration::from_millis(30)).await;
    let n = f.load.reads();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(f.load.reads(), n);
}

#[tokio::test]
async fn a_fullscreen_game_pauses_background_work() {
    let f = fx();
    install(&f).await;
    f.load.in_front.store(true, Ordering::SeqCst);
    f.t.transcribe_album("80").await.unwrap();
    until(&f.t, "paused", |s| s.paused == Some(PauseReason::Busy)).await;
    f.load.in_front.store(false, Ordering::SeqCst);
    until(&f.t, "album done", |s| idle(s) && f.events.done.lock().len() == 3).await;
}

#[tokio::test]
async fn the_busy_pause_can_be_turned_off() {
    let f = fx();
    install(&f).await;
    f.t.set_pace(pace::ProcessorUse::Balanced, false, false);
    f.load.set(95.0);
    f.t.transcribe_album("80").await.unwrap();
    until(&f.t, "album done", |s| idle(s) && f.events.done.lock().len() == 3).await;
    assert_eq!(f.load.reads(), 0);
    assert_eq!(f.rec.last_threads.load(Ordering::SeqCst), 4, "Balanced");
}

#[tokio::test]
async fn away_mode_uses_more_threads_and_drops_back_when_someone_returns() {
    let f = fx();
    install(&f).await;
    *f.rec.behaviour.lock() = Behaviour::Block;
    f.load.idle_for(Some(Duration::from_secs(10)));
    f.t.transcribe_album("80").await.unwrap();
    until(&f.t, "a run with 4 threads", |_| f.rec.last_threads.load(Ordering::SeqCst) == 4).await;
    // Someone's back: the song starts again with the usual two.
    *f.rec.behaviour.lock() = Behaviour::Words(vec![words("hello there", 1000)]);
    f.load.idle_for(Some(Duration::from_millis(1)));
    until(&f.t, "album done", |s| idle(s) && f.events.done.lock().len() == 3).await;
    assert_eq!(f.rec.last_threads.load(Ordering::SeqCst), 2);
    assert_eq!(f.events.done.lock()[0].0, "801", "the song that started again isn't skipped");
    assert!(f.t.status().error.is_none());
}

#[tokio::test]
async fn a_crash_fails_the_song_and_the_queue_goes_on() {
    let f = fx();
    install(&f).await;
    *f.rec.behaviour.lock() = Behaviour::Crash;
    f.t.transcribe_album("80").await.unwrap();
    let s = until(&f.t, "failure", |s| s.error.is_some()).await;
    assert_eq!(s.error.unwrap().message, "The speech model stopped unexpectedly on this song.");
    *f.rec.behaviour.lock() = Behaviour::Words(vec![words("hello there", 1000)]);
    until(&f.t, "the rest", |s| idle(s) && f.events.done.lock().len() == 2).await;
}

#[tokio::test]
async fn repeated_crashes_stop_background_work_for_the_session() {
    let f = fx();
    install(&f).await;
    *f.rec.behaviour.lock() = Behaviour::Crash;
    f.rec.crashed_too_often.store(true, Ordering::SeqCst);
    f.t.transcribe_album("80").await.unwrap_err();
    f.rec.crashed_too_often.store(false, Ordering::SeqCst);
    f.t.transcribe_album("80").await.unwrap();
    f.rec.crashed_too_often.store(true, Ordering::SeqCst);
    let s = until(&f.t, "stopped", |s| idle(s) && s.error.as_ref().is_some_and(|e| e.track_id.is_none())).await;
    assert!(s.error.unwrap().message.contains("until Platter restarts"));
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 1, "the rest of the album was dropped");
    assert_eq!(f.t.transcribe_unreleased().await.unwrap_err().kind(), "invalid");
    // A song someone asks for still runs.
    *f.rec.behaviour.lock() = Behaviour::Words(vec![words("hello there", 1000)]);
    f.t.transcribe_track("501").unwrap();
    until(&f.t, "501 done", |_| f.events.done.lock().iter().any(|d| d.0 == "501")).await;
}

#[tokio::test]
async fn long_runs_show_their_pace() {
    let f = fx();
    install(&f).await;
    f.rec.hold_after.store(1, Ordering::SeqCst);
    f.t.transcribe_album("80").await.unwrap();
    assert_eq!(f.t.status().secs_per_song, None, "not known before a song is done");
    let s = until(&f.t, "the second song", |s| s.current.as_ref().is_some_and(|c| c.track_id == "802")).await;
    assert!(s.secs_per_song.is_some(), "{s:?}");
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
    f.t.set_options(false, Some(ModelKind::Small));
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

// ---------------------------------------------------------------- official lyrics first

/// Waits (up to `WAIT`) for a condition outside the status.
async fn until_true(what: &str, f: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !f() {
        if std::time::Instant::now() > deadline {
            panic!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn matching_official_synced_lyrics_stop_a_transcription() {
    let f = fx();
    install(&f).await;
    f.other.1.lock().push(("801".into(), Official::Matched));
    // Asked for, and in the background batch: neither transcribes 801.
    f.t.transcribe_track("801").unwrap();
    until(&f.t, "idle", |s| idle(s) && f.other.2.lock().contains(&"801".to_string())).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 0, "official lyrics came first");
    assert!(f.events.done.lock().is_empty());
    assert!(f.db.read(|c| user::get(c, "801")).unwrap().is_none());
    assert_eq!(f.t.transcribe_unreleased().await.unwrap(), 3);
    until(&f.t, "batch", |s| idle(s) && f.events.done.lock().len() == 2).await;
    let done: Vec<String> = f.events.done.lock().iter().map(|d| d.0.clone()).collect();
    assert_eq!(done, vec!["802", "803"], "songs whose official lyrics don't match are transcribed");
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn without_a_match_the_song_is_transcribed() {
    let f = fx();
    install(&f).await;
    // Couldn't look (the check is off): transcribed as before.
    f.other.1.lock().push(("801".into(), Official::Unknown));
    f.t.transcribe_track("801").unwrap();
    until(&f.t, "done", |s| idle(s) && f.events.done.lock().len() == 1).await;
    // None matched: transcribed too.
    f.t.transcribe_track("802").unwrap();
    until(&f.t, "done", |s| idle(s) && f.events.done.lock().len() == 2).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 2);
    assert_eq!(*f.other.2.lock(), vec!["801", "802"], "official lyrics are looked for first");
    // Freshly transcribed songs aren't due another look for a week.
    let due = f.db.read(|c| store::official_due(c, crate::db::queries::now_s() - OFFICIAL_RECHECK_S, 10)).unwrap();
    assert!(due.is_empty(), "{due:?}");
}

#[tokio::test]
async fn transcribed_songs_are_looked_at_again_weekly_and_a_users_choice_is_kept() {
    let f = fx();
    let now = crate::db::queries::now_s();
    let week = OFFICIAL_RECHECK_S;
    f.db.write(|c| {
        for id in ["801", "802", "803"] {
            store::save(c, id, "[00:01.00]<00:01.00>Hello\n", "small", now)?;
        }
        // 801: looked at eight days ago; 802: yesterday; 803: chosen by the user.
        store::official_checked(c, "801", now - week - 86_400)?;
        store::official_checked(c, "802", now - 86_400)?;
        store::official_checked(c, "803", now - week - 86_400)?;
        crate::lyrics::choice::put(c, "803", &crate::lyrics::choice::Choice { pick: Some("auto".into()), offset_ms: 0 }, now)?;
        Ok(())
    })
    .unwrap();
    f.other.1.lock().push(("801".into(), Official::Matched));
    assert_eq!(f.t.recheck_official().await, Some(1));
    assert_eq!(*f.other.2.lock(), vec!["801"], "only the song due, never the one the user chose");
    // Swapped by the lyrics view (the match outranks a transcription); the
    // transcription stays to pick.
    assert!(f.db.read(|c| user::get(c, "801")).unwrap().unwrap().auto.is_some());
    // Looked at: not due again this week.
    assert_eq!(f.t.recheck_official().await, Some(0));
    assert_eq!(f.other.2.lock().len(), 1);
    // A look that can't run leaves the song due.
    f.db.write(|c| store::official_checked(c, "802", now - week - 1)).unwrap();
    f.other.1.lock().push(("802".into(), Official::Unknown));
    assert_eq!(f.t.recheck_official().await, None);
    let due = f.db.read(|c| store::official_due(c, now - week, 10)).unwrap();
    assert_eq!(due, vec!["802"]);
}

#[tokio::test]
async fn transcriptions_with_the_first_timing_are_moved_once_at_idle() {
    let timing = pace::Timing { upkeep: Duration::from_millis(20), ..pace::fake::timing() };
    let f = fx_timed(FakeFetch::new(model_body()), None, gpu::Gpu::none(), timing);
    let now = crate::db::queries::now_s();
    let old = "[00:12.34]<00:12.34>Hello <00:12.80>world\n";
    f.db.write(|c| {
        user::put_auto_timed(c, "801", old, "small", now, 1)?;
        // Already current: left as it is.
        user::put_auto(c, "802", old, "small", now)?;
        // The user's own: never moved.
        user::put(c, "803", old, false, now)?;
        // Nothing official to look at, so only the timing is upkeep's work.
        for id in ["801", "802"] {
            store::official_checked(c, id, now)?;
        }
        Ok(())
    })
    .unwrap();
    let timing = |id: &str| f.db.read(|c| user::auto_timing(c, id)).unwrap();
    assert_eq!(timing("801"), Some(1));
    // Upkeep finds it on its own once the computer is calm.
    until_true("moved", || timing("801") == Some(engine::TIMING_VERSION)).await;
    let text = |id: &str| f.db.read(|c| user::get(c, id)).unwrap().unwrap().text;
    assert_eq!(text("801"), "[00:11.99]<00:11.99>Hello <00:12.61>world
");
    assert_eq!(text("802"), old);
    assert_eq!(text("803"), old);
    // Once: nothing is left to move.
    assert_eq!(f.t.retime_old().await, 0);
    assert_eq!(text("801"), "[00:11.99]<00:11.99>Hello <00:12.61>world
");
}

// ---------------------------------------------------------------- pause, playback, model switch

#[tokio::test]
async fn the_batch_pauses_and_resumes_and_a_pause_survives_a_restart() {
    let f = fx();
    install(&f).await;
    // The second song runs until it's stopped.
    f.rec.hold_after.store(1, Ordering::SeqCst);
    assert_eq!(f.t.transcribe_unreleased().await.unwrap(), 3);
    until(&f.t, "second song", |s| f.rec.runs.load(Ordering::SeqCst) == 2 && s.current.is_some()).await;
    f.t.pause();
    let s = until(&f.t, "paused", |s| s.user_paused && s.current.is_none()).await;
    assert_eq!((s.batch.done, s.batch.total, s.queued), (1, 3, 2), "the stopped song went back in the queue");
    // Kept for a restart.
    let kept = f.db.read(|c| crate::db::store::kv_get(c, PAUSED_KEY)).unwrap().unwrap();
    let p: PausedBatch = serde_json::from_str(&kept).unwrap();
    assert_eq!((p.ids.len(), p.done, p.total), (2, 1, 3));
    // A song asked for still runs while paused.
    *f.rec.behaviour.lock() = Behaviour::Words(vec![words("hello there", 1000)]);
    f.rec.hold_after.store(usize::MAX, Ordering::SeqCst);
    f.t.transcribe_track("501").unwrap();
    until(&f.t, "501", |s| s.current.is_none() && f.events.done.lock().iter().any(|d| d.0 == "501")).await;
    let s = f.t.status();
    assert!(s.user_paused);
    assert_eq!(s.queued, 2, "the batch still waits");
    // A restart: another transcriber on the same database comes back paused.
    let g = fx();
    g.db.write(|c| crate::db::store::kv_set(c, PAUSED_KEY, &kept)).unwrap();
    g.t.restore_pause().await;
    let s = g.t.status();
    assert!(s.user_paused);
    assert_eq!((s.batch.done, s.batch.total, s.queued), (1, 3, 2));
    // Resume: the rest is done and nothing is kept any more.
    f.t.resume();
    until(&f.t, "batch done", |s| idle(s) && !s.user_paused && f.events.done.lock().len() == 4).await;
    let kept = f.db.read(|c| crate::db::store::kv_get(c, PAUSED_KEY)).unwrap().unwrap_or_default();
    assert!(kept.is_empty());
}

#[tokio::test]
async fn reading_a_song_says_when_it_waits_for_playback() {
    let f = fx();
    install(&f).await;
    f.songs.hold.store(true, Ordering::SeqCst);
    f.t.transcribe_track("801").unwrap();
    let s = until(&f.t, "waiting", |s| s.current.as_ref().is_some_and(|c| c.waiting_for_playback)).await;
    let c = s.current.unwrap();
    assert_eq!((c.stage, c.title.as_deref()), (Stage::Reading, Some("Tape One")));
    f.songs.hold.store(false, Ordering::SeqCst);
    until(&f.t, "done", |s| idle(s) && f.events.done.lock().len() == 1).await;
}

#[tokio::test]
async fn while_a_new_model_downloads_the_installed_one_carries_on_and_its_songs_can_be_redone() {
    let fetch = Arc::new(FakeFetch {
        body: Mutex::new(model_body()),
        fail_after: Mutex::new(None),
        refuse: AtomicBool::new(false),
        gap: Duration::from_millis(40),
    });
    let f = fx_with(fetch);
    f.t.download_model(ModelKind::Small).unwrap();
    until(&f.t, "small", |s| s.models[0].installed && !s.models[0].downloading).await;
    // Medium is picked: it downloads, and songs meanwhile use Small.
    f.t.set_options(true, Some(ModelKind::Medium));
    f.t.transcribe_track("801").unwrap();
    until(&f.t, "801", |s| idle(s) && f.events.done.lock().len() == 1).await;
    assert!(f.t.status().models[1].downloading, "{:?}", f.t.status().models);
    let own = f.db.read(|c| user::get(c, "801")).unwrap().unwrap();
    assert_eq!(own.auto.as_deref(), Some("small"));
    assert_eq!(f.t.status().redo, vec![RedoInfo { model: ModelKind::Small, count: 1 }]);
    assert!(f.t.redo_other_model().is_err(), "not before Medium is here");
    // Medium arrives: the next song uses it, and 801 can be done again.
    until(&f.t, "medium", |s| s.models[1].installed).await;
    f.t.transcribe_track("802").unwrap();
    until(&f.t, "802", |s| idle(s) && f.events.done.lock().len() == 2).await;
    assert_eq!(f.db.read(|c| user::get(c, "802")).unwrap().unwrap().auto.as_deref(), Some("medium"));
    assert_eq!(f.t.redo_other_model().unwrap(), 1);
    until(&f.t, "redone", |s| idle(s) && s.redo.is_empty()).await;
    assert_eq!(f.db.read(|c| user::get(c, "801")).unwrap().unwrap().auto.as_deref(), Some("medium"));
}

// ---------------------------------------------------------------- the voice

async fn transcribe(f: &Fx, id: &str) {
    let before = f.events.done.lock().len();
    f.t.transcribe_track(id).unwrap();
    until(&f.t, "done", |s| idle(s) && f.events.done.lock().len() > before).await;
}

#[tokio::test]
async fn on_a_graphics_card_the_voice_model_downloads_quietly_and_then_the_voice_is_heard() {
    let f = fx_voice();
    install(&f).await;
    // The first song is heard as it is while the voice model downloads.
    transcribe(&f, "801").await;
    assert_eq!(f.sep.runs.load(Ordering::SeqCst), 0);
    assert!(f.rec.last_len.load(Ordering::SeqCst) > 30_000, "the whole mix");
    until_true("the voice model", || f.voice_models.installed(voice::Voice::Demucs).is_some()).await;
    assert!(f.t.status().error.is_none(), "nothing to say about it");
    // Then the voice alone, from the song in stereo at the voice model's rate.
    transcribe(&f, "802").await;
    assert_eq!(f.sep.runs.load(Ordering::SeqCst), 1);
    assert_eq!(f.sep.last_len.load(Ordering::SeqCst), 2 * 88_200);
    assert_eq!(f.rec.last_len.load(Ordering::SeqCst), VOICE_LEN);
    assert_eq!(*f.events.done.lock(), vec![("801".to_string(), true), ("802".to_string(), true)]);
}

#[tokio::test]
async fn a_separation_that_goes_wrong_hears_the_mix_and_stops_trying_this_session() {
    let f = fx_voice();
    install(&f).await;
    f.voice_models.download(voice::Voice::Demucs, &*f.fetch, &AtomicBool::new(false), &|_, _| {}).await.unwrap();
    f.sep.fail.store(true, Ordering::SeqCst);
    transcribe(&f, "801").await;
    assert_eq!(*f.events.done.lock(), vec![("801".to_string(), true)], "the song is still written down");
    assert!(f.rec.last_len.load(Ordering::SeqCst) > 30_000, "from the whole mix");
    assert!(f.gpu.active().is_some(), "the speech model keeps the card");
    transcribe(&f, "802").await;
    assert_eq!(f.sep.runs.load(Ordering::SeqCst), 1, "not tried again");
}

#[tokio::test]
async fn without_a_graphics_card_the_voice_isnt_separated() {
    let f = fx_voice();
    install(&f).await;
    f.voice_models.download(voice::Voice::Demucs, &*f.fetch, &AtomicBool::new(false), &|_, _| {}).await.unwrap();
    f.t.set_gpu(gpu::GpuMode::Off);
    transcribe(&f, "801").await;
    assert_eq!(f.sep.runs.load(Ordering::SeqCst), 0);
    assert!(f.rec.last_len.load(Ordering::SeqCst) > 30_000);
}

#[tokio::test]
async fn a_failed_voice_model_download_is_quiet_and_waits_before_trying_again() {
    let f = fx_voice();
    install(&f).await;
    f.fetch.refuse.store(true, Ordering::Relaxed);
    transcribe(&f, "801").await;
    until_true("the download gave up", || !f.t.inner.voice.fetching.load(Ordering::SeqCst)).await;
    assert!(f.t.status().error.is_none());
    f.fetch.refuse.store(false, Ordering::Relaxed);
    transcribe(&f, "802").await;
    assert!(f.voice_models.installed(voice::Voice::Demucs).is_none(), "not tried again so soon");
    assert_eq!(*f.events.done.lock(), vec![("801".to_string(), true), ("802".to_string(), true)]);
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

#[test]
fn decodes_the_stereo_for_the_voice_too_unless_the_song_is_too_long() {
    let never = AtomicBool::new(false);
    let d = audio::decode(wav(), Some("wav"), &never, Some((44_100, 10))).unwrap();
    assert_eq!(d.speech.len(), 32_000);
    assert_eq!(d.stereo.as_ref().map(Vec::len), Some(2 * 88_200));
    let d = audio::decode(wav(), Some("wav"), &never, Some((44_100, 1))).unwrap();
    assert_eq!(d.speech.len(), 32_000, "the speech model still hears it all");
    assert!(d.stereo.is_none());
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

// ---------------------------------------------------------------- through a helper

/// What runs in the fake helper "process": background songs (fewer than
/// four threads) send 10% and run until cancelled, the first `blocks` of
/// them; songs asked for send 40% and wait for `gate` (or a cancel, unless
/// `deaf`, when only ending the helper stops them).
#[derive(Default)]
struct Script {
    blocks: AtomicUsize,
    gate: AtomicBool,
    deaf: AtomicBool,
    killed: Arc<AtomicBool>,
    loaded: Mutex<Option<std::path::PathBuf>>,
    loads: AtomicUsize,
    threads: Mutex<Vec<usize>>,
    busy: AtomicUsize,
    most_busy: AtomicUsize,
}

impl Recognizer for Script {
    fn unavailable(&self) -> Option<String> {
        None
    }
    fn transcribe(
        &self,
        model: &Path,
        _kind: ModelKind,
        _pcm: &[f32],
        threads: usize,
        _hints: &Hints,
        cancel: Stop,
        progress: Progress,
    ) -> Result<Vec<Segment>, RecognizeError> {
        if self.loaded.lock().as_deref() != Some(model) {
            progress(Step::Loading);
            std::thread::sleep(Duration::from_millis(20));
            *self.loaded.lock() = Some(model.to_path_buf());
            self.loads.fetch_add(1, Ordering::SeqCst);
        }
        self.threads.lock().push(threads);
        let n = self.busy.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_busy.fetch_max(n, Ordering::SeqCst);
        let res = self.run(threads, &cancel, &progress);
        self.busy.fetch_sub(1, Ordering::SeqCst);
        res
    }
    fn unload(&self) {
        *self.loaded.lock() = None;
    }
}

impl Script {
    fn run(&self, threads: usize, cancel: &Stop, progress: &Progress) -> Result<Vec<Segment>, RecognizeError> {
        progress(Step::Percent(0));
        if threads < 4 {
            if self.blocks.load(Ordering::SeqCst) > 0 {
                self.blocks.fetch_sub(1, Ordering::SeqCst);
                progress(Step::Percent(10));
                let started = std::time::Instant::now();
                while !cancel.is_set() && started.elapsed() < Duration::from_secs(20) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                return Err(RecognizeError::Cancelled);
            }
            return Ok(vec![words("in the background", 1000)]);
        }
        progress(Step::Percent(40));
        // A failed test mustn't leave this running (the runtime waits for it).
        let started = std::time::Instant::now();
        loop {
            if started.elapsed() > Duration::from_secs(20) {
                return Err(RecognizeError::Failed("the test gave up".into()));
            }
            if self.deaf.load(Ordering::SeqCst) {
                if self.killed.load(Ordering::SeqCst) {
                    panic!("ended by force");
                }
            } else if cancel.is_set() {
                return Err(RecognizeError::Cancelled);
            }
            if self.gate.load(Ordering::SeqCst) {
                return Ok(vec![words("asked for", 1000)]);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[derive(Default)]
struct Processes {
    started: AtomicUsize,
    ended: Arc<AtomicUsize>,
    most_alive: AtomicUsize,
}

struct ScriptLaunch {
    script: Arc<Script>,
    n: Arc<Processes>,
}

struct ScriptProcess(Arc<AtomicUsize>, Arc<AtomicBool>);

impl helper::WorkerProcess for ScriptProcess {
    fn kill(&mut self) {
        self.1.store(true, Ordering::SeqCst);
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl helper::Launch for ScriptLaunch {
    fn launch(&self, _lowest: bool) -> std::io::Result<helper::Worker> {
        let started = self.n.started.fetch_add(1, Ordering::SeqCst) + 1;
        self.n.most_alive.fetch_max(started - self.n.ended.load(Ordering::SeqCst), Ordering::SeqCst);
        self.script.killed.store(false, Ordering::SeqCst);
        *self.script.loaded.lock() = None;
        let (req_r, req_w) = std::io::pipe()?;
        let (rep_r, rep_w) = std::io::pipe()?;
        let rec: Arc<dyn Recognizer> = self.script.clone();
        std::thread::spawn(move || {
            let _ = helper::worker::serve(Box::new(req_r), Box::new(rep_w), rec, None, &|_| {});
        });
        Ok(helper::Worker {
            input: Box::new(req_w),
            output: Box::new(rep_r),
            process: Box::new(ScriptProcess(self.n.ended.clone(), self.script.killed.clone())),
        })
    }
}

/// The real helper, available in builds without the speech library too.
struct Available(Arc<helper::Helper>);

impl Recognizer for Available {
    fn unavailable(&self) -> Option<String> {
        None
    }
    fn transcribe(
        &self,
        model: &Path,
        kind: ModelKind,
        pcm: &[f32],
        threads: usize,
        hints: &Hints,
        cancel: Stop,
        progress: Progress,
    ) -> Result<Vec<Segment>, RecognizeError> {
        self.0.transcribe(model, kind, pcm, threads, hints, cancel, progress)
    }
    fn unload(&self) {
        self.0.unload()
    }
    fn wanted(&self) -> bool {
        self.0.wanted()
    }
    fn crashed_too_often(&self) -> bool {
        self.0.crashed_too_often()
    }
}

struct Hx {
    f: Fx,
    script: Arc<Script>,
    n: Arc<Processes>,
    helper: Arc<helper::Helper>,
}

async fn hx(idle: Duration) -> Hx {
    hx_timed(idle, pace::fake::timing()).await
}

async fn hx_timed(idle: Duration, timing: pace::Timing) -> Hx {
    let script = Arc::new(Script::default());
    let n = Arc::new(Processes::default());
    let helper =
        Arc::new(helper::Helper::new(Arc::new(ScriptLaunch { script: script.clone(), n: n.clone() }), Arc::new(|| false), idle));
    let rec: Arc<dyn Recognizer> = Arc::new(Available(helper.clone()));
    let f = fx_timed(FakeFetch::new(model_body()), Some(rec), gpu::Gpu::none(), timing);
    install(&f).await;
    Hx { f, script, n, helper }
}

fn is(s: &TranscribeStatus, id: &str, stage: Stage, percent: u8) -> bool {
    s.current.as_ref().is_some_and(|c| c.track_id == id && c.stage == stage && c.percent == percent)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn progress_from_the_helper_reaches_the_status_and_events() {
    // "Quiet" after a second: far longer than a slow machine takes to send the
    // move, so the first event can't already be quiet; the test then waits it out.
    let timing = pace::Timing { stuck: Duration::from_secs(1), ..pace::fake::timing() };
    let h = hx_timed(IDLE_EXIT_TEST, timing).await;
    let f = &h.f;
    f.t.transcribe_track("501").unwrap();
    until(&f.t, "40%", |s| is(s, "501", Stage::Transcribing, 40)).await;
    let seen: Vec<(Stage, u8)> = f
        .events
        .statuses
        .lock()
        .iter()
        .filter_map(|s| s.current.as_ref().filter(|c| c.track_id == "501").map(|c| (c.stage, c.percent)))
        .collect();
    assert!(seen.contains(&(Stage::Loading, 0)), "loading the model was said: {seen:?}");
    // The 40% came right after the 0%, inside the emit gap: the next tick sends it.
    until_true("an event says 40%", || f.events.statuses.lock().iter().any(|s| is(s, "501", Stage::Transcribing, 40))).await;
    let moved = f.events.statuses.lock().iter().filter(|s| is(s, "501", Stage::Transcribing, 40)).all(|s| s.current.as_ref().unwrap().quiet_secs.is_none());
    assert!(moved, "not quiet right after it moved");
    // Nothing moves for a while (here a second): the status says so.
    let s = until(&f.t, "quiet", |s| s.current.as_ref().is_some_and(|c| c.quiet_secs.is_some())).await;
    assert_eq!(s.current.unwrap().percent, 40);
    // The next tick (every 50 ms) sends it.
    until_true("an event saying it's quiet", || {
        f.events.statuses.lock().iter().any(|s| s.current.as_ref().is_some_and(|c| c.quiet_secs.is_some()))
    })
    .await;
    h.script.gate.store(true, Ordering::SeqCst);
    until(&f.t, "done", |_| f.events.done.lock().iter().any(|d| d.0 == "501")).await;
    assert_eq!(*h.script.threads.lock(), vec![4], "the thread budget reaches the speech library as is");
}

/// Short, so a test can watch the helper end.
const IDLE_EXIT_TEST: Duration = Duration::from_millis(150);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_song_asked_for_pauses_the_batch_in_the_same_helper() {
    let h = hx(IDLE_EXIT_TEST).await;
    let f = &h.f;
    h.script.blocks.store(1, Ordering::SeqCst);
    h.script.gate.store(true, Ordering::SeqCst);
    f.t.transcribe_album("80").await.unwrap();
    until(&f.t, "801 at 10%", |s| is(s, "801", Stage::Transcribing, 10)).await;
    let asked = std::time::Instant::now();
    f.t.transcribe_track("501").unwrap();
    // The batch song is stopped within a moment and the asked-for song runs.
    until(&f.t, "501 running", |s| s.current.as_ref().is_some_and(|c| c.track_id == "501")).await;
    assert!(asked.elapsed() < Duration::from_secs(1), "the batch gave way in {:?}", asked.elapsed());
    until(&f.t, "501 done", |_| f.events.done.lock().iter().any(|d| d.0 == "501")).await;
    // Then the batch song goes on, first.
    until(&f.t, "801 done", |_| f.events.done.lock().iter().any(|d| d.0 == "801")).await;
    until(&f.t, "all done", idle).await;
    let done: Vec<String> = f.events.done.lock().iter().map(|d| d.0.clone()).collect();
    assert_eq!(done, vec!["501", "801", "802", "803"]);
    assert_eq!(h.n.started.load(Ordering::SeqCst), 1, "one helper all along");
    assert_eq!(h.n.most_alive.load(Ordering::SeqCst), 1);
    assert_eq!(h.script.loads.load(Ordering::SeqCst), 1, "the model was loaded once");
    assert_eq!(h.script.most_busy.load(Ordering::SeqCst), 1, "never two songs at once");
    let threads = h.script.threads.lock().clone();
    assert_eq!(threads[1], 4, "the asked-for song got its four threads: {threads:?}");
    assert!(threads[0] < 4 && threads[2] < 4, "{threads:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lyrics_check_waiting_for_the_helper_pauses_the_batch() {
    let h = hx(IDLE_EXIT_TEST).await;
    let f = &h.f;
    h.script.blocks.store(1, Ordering::SeqCst);
    f.t.transcribe_album("80").await.unwrap();
    until(&f.t, "801 at 10%", |s| is(s, "801", Stage::Transcribing, 10)).await;
    let check = h.helper.priority();
    let model = f.models.installed(ModelKind::Small).unwrap();
    let asked = std::time::Instant::now();
    let res = tokio::task::spawn_blocking(move || {
        // A check's song is "asked for" by the script (four threads), and finishes.
        check.transcribe(&model, ModelKind::Small, &[0.0; 16], 4, &Hints::english(), Stop::new(), Arc::new(|_| {}))
    });
    h.script.gate.store(true, Ordering::SeqCst);
    res.await.unwrap().unwrap();
    assert!(asked.elapsed() < Duration::from_secs(2), "{:?}", asked.elapsed());
    until(&f.t, "801 done", |_| f.events.done.lock().iter().any(|d| d.0 == "801")).await;
    assert_eq!(h.n.started.load(Ordering::SeqCst), 1);
    assert_eq!(h.script.loads.load(Ordering::SeqCst), 1);
    f.t.cancel(None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_reaches_the_helper_and_resets_the_song() {
    let h = hx(IDLE_EXIT_TEST).await;
    let f = &h.f;
    f.t.transcribe_track("501").unwrap();
    until(&f.t, "40%", |s| is(s, "501", Stage::Transcribing, 40)).await;
    let cancelled = std::time::Instant::now();
    f.t.cancel(Some("501"));
    let s = until(&f.t, "reset", idle).await;
    assert!(cancelled.elapsed() < Duration::from_secs(1), "{:?}", cancelled.elapsed());
    assert!(s.error.is_none() && s.up_next.is_none());
    assert!(f.events.done.lock().is_empty());
    assert_eq!(h.n.ended.load(Ordering::SeqCst), 0, "it stopped when asked: the helper stays a moment");
    // Nothing else to do: it ends on its idle timer.
    until_true("the helper ended on its idle timer", || !h.helper.running()).await;
    // A helper that doesn't stop when asked is ended within about a second.
    h.script.deaf.store(true, Ordering::SeqCst);
    f.t.transcribe_track("502").unwrap();
    until(&f.t, "40%", |s| is(s, "502", Stage::Transcribing, 40)).await;
    let cancelled = std::time::Instant::now();
    f.t.cancel(Some("502"));
    let s = until(&f.t, "reset", idle).await;
    assert!(cancelled.elapsed() < Duration::from_secs(3), "{:?}", cancelled.elapsed());
    assert!(s.error.is_none(), "{s:?}");
    assert!(!h.helper.running() && !h.helper.crashed_too_often());
    assert_eq!(h.n.most_alive.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_of_a_queued_song_leaves_the_running_one() {
    let h = hx(IDLE_EXIT_TEST).await;
    let f = &h.f;
    f.t.transcribe_track("501").unwrap();
    until(&f.t, "40%", |s| is(s, "501", Stage::Transcribing, 40)).await;
    f.t.transcribe_track("502").unwrap();
    f.t.transcribe_track("801").unwrap();
    let s = f.t.status();
    assert_eq!((s.queued, s.up_next.as_deref()), (2, Some("801")));
    f.t.cancel(Some("502"));
    let s = f.t.status();
    assert_eq!(s.queued, 1);
    assert!(is(&s, "501", Stage::Transcribing, 40), "the running song is untouched: {s:?}");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(is(&f.t.status(), "501", Stage::Transcribing, 40));
    h.script.gate.store(true, Ordering::SeqCst);
    until(&f.t, "all done", idle).await;
    let done: Vec<String> = f.events.done.lock().iter().map(|d| d.0.clone()).collect();
    assert_eq!(done, vec!["501", "801"]);
}

// ---------------------------------------------------------------- graphics card

async fn install_kind(f: &Fx, kind: ModelKind) {
    f.t.download_model(kind).unwrap();
    until(&f.t, "model", |s| s.models.iter().any(|m| m.kind == kind && m.installed && !m.downloading)).await;
}

#[tokio::test]
async fn with_a_graphics_card_new_users_get_turbo_and_settings_name_the_card() {
    let f = fx_gpu(vec![card("RTX 5090", gpu::DeviceKind::Discrete)]);
    f.t.set_options(true, None);
    let s = f.t.status();
    assert_eq!((s.model, s.model_picked), (ModelKind::Turbo, false));
    assert_eq!(s.gpu.as_deref(), Some("RTX 5090"));
    // A model someone picked stays.
    f.t.set_options(true, Some(ModelKind::Small));
    let s = f.t.status();
    assert_eq!((s.model, s.model_picked), (ModelKind::Small, true));
    // "Use the graphics card" Off: the processor, and Small by default.
    f.t.set_options(true, None);
    f.t.set_gpu(gpu::GpuMode::Off);
    let s = f.t.status();
    assert_eq!((s.model, s.gpu.as_deref(), s.gpu_found.as_deref()), (ModelKind::Small, None, Some("RTX 5090")));
    // A failure on the card this session: the processor too.
    f.t.set_gpu(gpu::GpuMode::Auto);
    assert_eq!(f.t.status().model, ModelKind::Turbo);
    f.gpu.fail();
    assert_eq!((f.t.status().model, f.t.status().gpu), (ModelKind::Small, None));
}

#[tokio::test]
async fn without_a_graphics_card_small_stays_the_default_and_every_model_is_offered() {
    let f = fx();
    f.t.set_options(true, None);
    let s = f.t.status();
    assert_eq!((s.model, s.gpu, s.gpu_found), (ModelKind::Small, None, None));
    assert_eq!(s.models.iter().map(|m| m.kind).collect::<Vec<_>>(), ModelKind::ALL.to_vec());
    assert_eq!(ModelKind::ALL[0], ModelKind::Small, "the lyrics check takes the smallest first");
}

#[tokio::test]
async fn on_the_graphics_card_a_song_uses_a_few_processor_threads() {
    let f = fx_gpu(vec![card("Radeon Graphics", gpu::DeviceKind::Integrated)]);
    f.t.set_options(true, None);
    install_kind(&f, ModelKind::Turbo).await;
    f.t.transcribe_track("501").unwrap();
    until(&f.t, "501 done", |_| f.events.done.lock().iter().any(|d| d.0 == "501")).await;
    assert_eq!(f.rec.last_threads.load(Ordering::SeqCst), 3, "Light: enough to keep the card fed");
    f.t.set_pace(pace::ProcessorUse::Fast, true, true);
    f.t.transcribe_track("502").unwrap();
    until(&f.t, "502 done", |_| f.events.done.lock().iter().any(|d| d.0 == "502")).await;
    assert_eq!(f.rec.last_threads.load(Ordering::SeqCst), 4, "Fast: still a few, the card does the work");
    let own = f.db.read(|c| user::get(c, "502")).unwrap().unwrap();
    assert_eq!(own.auto.as_deref(), Some("turbo"));
}

#[tokio::test]
async fn work_on_the_graphics_card_still_pauses_for_a_fullscreen_game() {
    let f = fx_gpu(vec![card("RTX 5090", gpu::DeviceKind::Discrete)]);
    f.t.set_options(true, None);
    install_kind(&f, ModelKind::Turbo).await;
    f.load.in_front.store(true, Ordering::SeqCst);
    f.t.transcribe_album("80").await.unwrap();
    until(&f.t, "paused", |s| s.paused == Some(PauseReason::Busy)).await;
    assert_eq!(f.rec.runs.load(Ordering::SeqCst), 0);
    f.load.in_front.store(false, Ordering::SeqCst);
    until(&f.t, "album done", |s| idle(s) && f.events.done.lock().len() == 3).await;
}

#[tokio::test]
async fn the_model_picker_shows_the_speed_measured_on_the_device_in_use() {
    let f = fx_gpu(vec![card("RTX 5090", gpu::DeviceKind::Discrete)]);
    let speeds = &f.t.inner.deps.speeds;
    // Ten seconds for a 210 s song on the card; four minutes for Small on the processor.
    speeds.record(ModelKind::Turbo, true, 210.0, Duration::from_secs(10));
    speeds.record(ModelKind::Small, false, 210.0, Duration::from_secs(240));
    let by_kind = |s: &TranscribeStatus, k| s.models.iter().find(|m| m.kind == k).unwrap().secs_per_song;
    let s = f.t.status();
    assert_eq!((by_kind(&s, ModelKind::Turbo), by_kind(&s, ModelKind::Small)), (Some(10), None));
    f.t.set_gpu(gpu::GpuMode::Off);
    let s = f.t.status();
    assert_eq!((by_kind(&s, ModelKind::Turbo), by_kind(&s, ModelKind::Small)), (None, Some(240)));
    // Kept on disk for next time.
    let again = speed::Speeds::load(Some(f.dir.path().join("speeds.json")));
    assert_eq!(again.secs_per_song(ModelKind::Turbo, true), Some(10));
}

#[test]
fn speeds_follow_the_latest_runs_and_skip_short_ones() {
    let s = speed::Speeds::load(None);
    s.record(ModelKind::Small, false, 10.0, Duration::from_secs(60));
    assert_eq!(s.secs_per_song(ModelKind::Small, false), None, "a check's short window doesn't count");
    s.record(ModelKind::Small, false, 210.0, Duration::from_secs(100));
    assert_eq!(s.secs_per_song(ModelKind::Small, false), Some(100));
    s.record(ModelKind::Small, false, 210.0, Duration::from_secs(200));
    assert_eq!(s.secs_per_song(ModelKind::Small, false), Some(130), "leaning on the newest run");
}

#[test]
fn the_default_model_follows_the_graphics_card() {
    assert_eq!(ModelKind::chosen(None, true), ModelKind::Turbo);
    assert_eq!(ModelKind::chosen(None, false), ModelKind::Small);
    assert_eq!(ModelKind::chosen(Some(ModelKind::Medium), true), ModelKind::Medium);
    assert_eq!(pace::gpu_thread_budget(16, pace::ProcessorUse::Light), 3);
    assert_eq!(pace::gpu_thread_budget(16, pace::ProcessorUse::Balanced), 4);
    assert_eq!(pace::gpu_thread_budget(2, pace::ProcessorUse::Fast), 1, "never more than half the cores");
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
    // PLATTER_SPEECH_GPU=1 runs it on the graphics card.
    gpu::allow_from_env();
    let kind = if model.contains("turbo") {
        ModelKind::Turbo
    } else if model.contains("medium") {
        ModelKind::Medium
    } else {
        ModelKind::Small
    };
    let threads: usize = std::env::var("PLATTER_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(4);
    let never = AtomicBool::new(false);
    let pcm = audio::to_16k_mono(std::fs::read(&clip).unwrap(), None, &never).unwrap();
    let rec = engine::in_process();
    assert_eq!(rec.unavailable(), None);
    let started = std::time::Instant::now();
    let segs = rec
        .transcribe(Path::new(&model), kind, &pcm, threads, &Hints::english(), Stop::new(), Arc::new(|_| {}))
        .unwrap();
    let took = started.elapsed();
    let lines = lines::to_lines(&segs);
    println!("audio {:.0} s, {threads} threads, took {:.1} s, {} lines", pcm.len() as f32 / RATE as f32, took.as_secs_f32(), lines.len());
    for l in lines.iter().take(8) {
        println!("{:>7} ms  {}", l.start_ms, l.text());
    }
}

/// Line timing with real models against spoken lines at known times.
/// `PLATTER_TIMING_AUDIO`: the audio; `PLATTER_TIMING_TRUTH`: JSON
/// `[["line text", start ms], ...]`; `PLATTER_MODELS`: a folder holding any of
/// the three model files (missing ones are skipped). Two threads, processor
/// unless `PLATTER_SPEECH_GPU=1`. `cargo test --lib --features transcribe
/// real_model_timing -- --ignored --nocapture`
#[cfg(feature = "transcribe")]
#[test]
#[ignore]
fn real_model_timing() {
    let (Ok(clip), Ok(truth), Ok(dir)) =
        (std::env::var("PLATTER_TIMING_AUDIO"), std::env::var("PLATTER_TIMING_TRUTH"), std::env::var("PLATTER_MODELS"))
    else {
        println!("skipped: set PLATTER_TIMING_AUDIO, PLATTER_TIMING_TRUTH and PLATTER_MODELS");
        return;
    };
    gpu::allow_from_env();
    let truth: Vec<(String, u32)> = serde_json::from_str(&std::fs::read_to_string(truth).unwrap()).unwrap();
    let never = AtomicBool::new(false);
    let pcm = audio::to_16k_mono(std::fs::read(&clip).unwrap(), None, &never).unwrap();
    let rec = engine::in_process();
    for spec in model::catalogue() {
        let path = Path::new(&dir).join(&spec.file);
        if !path.is_file() {
            println!("{:?}: not here, skipped", spec.kind);
            continue;
        }
        let segs = rec.transcribe(&path, spec.kind, &pcm, 2, &Hints::english(), Stop::new(), Arc::new(|_| {})).unwrap();
        let e = super::timing_tests::errors(&truth, &super::timing_tests::line_starts(&lines::to_lines(&segs)));
        println!("{:?}: line start errors {e:?} ms", spec.kind);
        assert!(e.iter().all(|x| x.abs() <= 300), "{:?}: {e:?}", spec.kind);
        rec.unload();
    }
}

/// Each song heard as the whole mix and as its separated voice, side by
/// side, for the before and after check. `PLATTER_WHISPER_MODEL`: the
/// speech model; `PLATTER_VOICE_MODEL`: htdemucs.safetensors;
/// `PLATTER_TEST_AUDIO`: one song, or a folder of them. Graphics card with
/// `PLATTER_SPEECH_GPU=1`. `cargo test --release --lib --features transcribe
/// real_voice_before_after -- --ignored --nocapture`
#[cfg(feature = "transcribe")]
#[test]
#[ignore]
fn real_voice_before_after() {
    let (Ok(model), Ok(weights), Ok(audio_at)) =
        (std::env::var("PLATTER_WHISPER_MODEL"), std::env::var("PLATTER_VOICE_MODEL"), std::env::var("PLATTER_TEST_AUDIO"))
    else {
        println!("skipped: set PLATTER_WHISPER_MODEL, PLATTER_VOICE_MODEL and PLATTER_TEST_AUDIO");
        return;
    };
    gpu::allow_from_env();
    let kind = if model.contains("turbo") { ModelKind::Turbo } else if model.contains("medium") { ModelKind::Medium } else { ModelKind::Small };
    let at = Path::new(&audio_at);
    let mut songs: Vec<std::path::PathBuf> =
        if at.is_dir() { std::fs::read_dir(at).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_file()).collect() } else { vec![at.into()] };
    songs.sort();
    let rec = engine::in_process();
    let sep = voice::in_process().expect("this build separates");
    let never = AtomicBool::new(false);
    for song in songs {
        let title = song.file_stem().map(|s| s.to_string_lossy().into_owned());
        let hints = Hints::for_song(kind, title.as_deref(), None);
        let Ok(d) = audio::decode(std::fs::read(&song).unwrap(), None, &never, Some((voice::RATE, voice::MAX_SECONDS))) else {
            println!("== {}: not decodable, skipped", song.display());
            continue;
        };
        println!("== {} ({:.0} s)", song.display(), d.speech.len() as f32 / RATE as f32);
        let started = std::time::Instant::now();
        let vocals = d.stereo.as_deref().map(|s| sep.vocals(Path::new(&weights), s, Stop::new(), Arc::new(|_| {})));
        let separated_in = started.elapsed();
        let mut heard = vec![("mix", d.speech)];
        match vocals {
            Some(Ok(v)) => heard.push(("voice", v)),
            Some(Err(e)) => println!("   separation failed: {e:?}"),
            None => println!("   too long to separate"),
        }
        for (what, pcm) in heard {
            let started = std::time::Instant::now();
            let segs = rec.transcribe(Path::new(&model), kind, &pcm, 4, &hints, Stop::new(), Arc::new(|_| {})).unwrap();
            let kept = lines::to_lines(&lines::keep_sure(segs, &hints.prompt));
            let extra = if what == "voice" { format!(" (+{:.1} s separating)", separated_in.as_secs_f32()) } else { String::new() };
            println!("-- {what}: {} lines in {:.1} s{extra}", kept.len(), started.elapsed().as_secs_f32());
            for l in &kept {
                println!("{:>7} ms  {}", l.start_ms, l.text());
            }
        }
    }
}
