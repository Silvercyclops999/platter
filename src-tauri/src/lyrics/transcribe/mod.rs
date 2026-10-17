//! Lyrics transcribed on this computer (docs/IPC.md "Lyrics transcription").
//!
//! Opt-in. A speech model is downloaded once when asked (never in the
//! installer). Songs are transcribed one at a time when the user asks: one
//! song, an album, or every unreleased song without lyrics in the background.
//! The audio never leaves the machine: the file is read from the server like
//! playback reads it (or taken from the pre-cache), decoded with the player's
//! decoder and given to whisper.cpp. The result is stored as enhanced LRC in
//! `lyrics_auto` and shown with an "auto-transcribed" note.
//!
//! - [`model`]  the model files: catalogue, download, checksum, delete
//! - [`audio`]  reading the whole file behind playback, decoding to 16 kHz mono
//! - [`engine`] the recognizer (whisper.cpp in builds with `transcribe`)
//! - [`helper`] the helper process the recognizer runs in
//! - [`gpu`]    the graphics card: finding one, the setting, the check's route
//! - [`speed`]  how fast each model has been here, per device
//! - [`pace`]   threads, the busy pause and the breather between songs
//! - [`lines`]  timed words to lyric lines and LRC
//! - [`store`]  the `lyrics_auto` table
//! - [`commands`] Tauri commands, events and the app's side of the traits

pub mod audio;
pub mod commands;
pub mod engine;
pub mod gpu;
pub mod helper;
pub mod lines;
pub mod model;
pub mod pace;
pub mod speed;
pub mod store;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod timing_tests;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::Notify;

use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::{AppError, AppResult};
use crate::lyrics::user;
use audio::AudioError;
use engine::{RecognizeError, Recognizer, Step, Stop};
use model::{DownloadError, ModelFetch, ModelKind, ModelStore};

/// Progress events at most this often (state changes go out at once).
const EMIT_GAP: Duration = Duration::from_millis(250);
/// A batch that can't reach the server waits this long before trying again.
const OFFLINE_RETRY: Duration = Duration::from_secs(60);
/// After repeated helper crashes.
const STOPPED: &str = "Transcribing in the background is off until Platter restarts, because the speech model kept stopping.";

/// Why the background batch is waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseReason {
    /// The computer is on battery power.
    Battery,
    /// The server can't be reached.
    Offline,
    /// Other programs are keeping the processor busy, a fullscreen game
    /// or presentation is in front, or the computer is running warm.
    Busy,
}

/// What went wrong last (shown in Settings and the lyrics view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    /// The model download failed (network or server).
    Download,
    /// The downloaded model didn't match its checksum and was deleted.
    Checksum,
    /// Not enough free disk space for the model.
    Disk,
    /// A song couldn't be read or transcribed.
    Transcribe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub kind: ModelKind,
    pub size_bytes: u64,
    pub installed: bool,
    pub downloading: bool,
    /// Bytes downloaded so far (while downloading).
    pub done_bytes: u64,
    /// About how long a song took here lately, on the device used now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secs_per_song: Option<u32>,
}

/// Where the song being worked on is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    /// Waiting for the model download to finish.
    WaitingForModel,
    /// Reading the file from the server.
    Reading,
    Decoding,
    /// The speech helper is loading the model.
    Loading,
    Transcribing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentJob {
    pub track_id: String,
    pub stage: Stage,
    /// Percent of the transcribing stage.
    pub percent: u8,
    pub background: bool,
    /// Seconds since the stage or percent last moved, once that's been
    /// long enough to say so (the computer may be busy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_secs: Option<u32>,
    /// The song's title, when the library has it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Reading the song waits while music is playing (playback's downloads come first).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub waiting_for_playback: bool,
    /// The model this song uses (another than the chosen one while that downloads).
    pub model: ModelKind,
}

/// Progress through the songs asked for since the queue was last empty.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchProgress {
    pub done: usize,
    pub total: usize,
}

/// `transcribe_status` and the `transcribe://status` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeStatus {
    /// The Settings switch.
    pub enabled: bool,
    /// This build and this computer can transcribe.
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    /// The model used: the one picked, else the default for this computer.
    pub model: ModelKind,
    /// A model was picked in Settings (else the default is used).
    pub model_picked: bool,
    /// The graphics card transcription uses, by name (None: the processor).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    /// A usable graphics card on this computer, used or not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_found: Option<String>,
    pub models: Vec<ModelInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentJob>,
    /// Songs waiting (not counting the current one).
    pub queued: usize,
    /// The song that goes next.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up_next: Option<String>,
    pub batch: BatchProgress,
    /// About how long each song of a long run takes, breather included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secs_per_song: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<PauseReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
    /// The user paused the batch (songs asked for one at a time still run).
    pub user_paused: bool,
    /// Songs done with another model than the chosen one since this run
    /// started (it was downloading): "Redo" offers them again.
    pub redo: Vec<RedoInfo>,
}

/// Songs transcribed with a model other than the one chosen now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedoInfo {
    pub model: ModelKind,
    pub count: usize,
}

/// A song's file, ready to decode.
pub struct SongAudio {
    pub bytes: Vec<u8>,
    /// Container or codec, to help the probe.
    pub hint: Option<String>,
}

/// Reads a song's whole file (pre-cache first, then the server behind playback).
#[async_trait]
pub trait SongReader: Send + Sync {
    async fn read(&self, track_id: &str, cancel: &AtomicBool) -> Result<SongAudio, AudioError>;

    /// [`read`](Self::read), or Ok(None) when the file is bigger than
    /// `max_bytes` (stopping as soon as that's known, where it can).
    async fn read_up_to(&self, track_id: &str, cancel: &AtomicBool, max_bytes: u64) -> Result<Option<SongAudio>, AudioError> {
        let a = self.read(track_id, cancel).await?;
        Ok((a.bytes.len() as u64 <= max_bytes).then_some(a))
    }

    /// Playback is using the connection, so reads wait for it.
    fn yielding(&self) -> bool {
        false
    }
}

/// Where a paused batch is kept so it survives a restart.
const PAUSED_KEY: &str = "transcribe.pausedBatch";

/// A paused batch as kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
struct PausedBatch {
    ids: Vec<String>,
    done: usize,
    total: usize,
}

/// Whether a song already has lyrics from another source (Plex, LRCLIB when
/// on, or the user's own). Asked before transcribing an album or the batch.
#[async_trait]
pub trait OtherLyrics: Send + Sync {
    async fn has_lyrics(&self, track_id: &str) -> bool;

    /// Synced lyrics from the song's sources (its server, files next to it,
    /// LRCLIB), checked against the recording now if they haven't been.
    /// Asked before transcribing, and again each week for transcribed songs.
    async fn official_synced(&self, _track_id: &str) -> Official {
        Official::Unknown
    }
}

/// What looking for official synced lyrics found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Official {
    /// A synced set matches the recording: it's shown, no transcription needed.
    Matched,
    /// None found, or none matches.
    NoneMatched,
    /// Couldn't look (the setting or the check is off, or the user chose
    /// the transcription for this song).
    Unknown,
}

/// A transcribed song is looked at again for official synced lyrics this often.
pub const OFFICIAL_RECHECK_S: i64 = 7 * 24 * 3600;
/// Songs looked at per round of upkeep.
const OFFICIAL_PER_ROUND: usize = 10;
/// Stored transcriptions moved to the current timing per step.
const RETIME_BATCH: usize = 50;

/// What pauses the background batch.
#[async_trait]
pub trait Conditions: Send + Sync {
    async fn pause_reason(&self) -> Option<PauseReason>;
}

/// Results made on the user's other devices (the sync store). A song
/// transcribed elsewhere for the same file isn't transcribed again here.
#[async_trait]
pub trait SharedResults: Send + Sync {
    /// Save here a result another device made for this song's file. Some
    /// (words found) when there was one.
    async fn take(&self, track_id: &str) -> Option<bool>;
    /// A result was saved here (an empty body: no words found).
    async fn saved(&self, track_id: &str, body: &str, model: &str);
    /// A song's transcription was forgotten here.
    async fn forgot(&self, track_id: &str);
}

pub trait TranscribeEvents: Send + Sync {
    fn status(&self, s: &TranscribeStatus);
    /// A song was transcribed (`found`: words came out).
    fn done(&self, track_id: &str, found: bool);
}

#[derive(Debug, Clone)]
struct Job {
    track_id: String,
    background: bool,
    /// Skip it when another source already has lyrics.
    check_other: bool,
    /// The single song someone is waiting on ("Transcribe this song").
    now: bool,
    /// Done again with the chosen model (it was done with another).
    redo: bool,
}

struct Download {
    kind: ModelKind,
    done: u64,
    cancel: Arc<AtomicBool>,
}

/// The song being worked on.
struct Running {
    job: CurrentJob,
    cancel: Stop,
    /// When the stage or percent last moved.
    moved: Instant,
}

#[derive(Default)]
struct State {
    enabled: bool,
    /// Picked in Settings (None: the default for this computer).
    model: Option<ModelKind>,
    queue: VecDeque<Job>,
    current: Option<Running>,
    batch: BatchProgress,
    paused: Option<PauseReason>,
    error: Option<Failure>,
    download: Option<Download>,
    last_emit: Option<Instant>,
    /// A progress event was held back by the emit rate.
    dirty: bool,
    /// The quiet minutes the last event said.
    quiet_said: Option<u32>,
    /// No background song starts before this (the breather).
    rest_until: Option<Instant>,
    /// Time taken by the background songs transcribed in this run, and how many.
    pace: (Duration, u32),
    /// A background song's decoded audio, kept when a song someone asked
    /// for stopped it, so it needn't be read again.
    held: Option<(String, Arc<Vec<f32>>)>,
    /// The user paused the batch.
    user_paused: bool,
    /// Songs done this session with a model other than the chosen one.
    other_model: Vec<(String, ModelKind)>,
    /// The paused batch as last kept in the database.
    pause_kept: Option<String>,
}

/// The parts the service works with.
pub struct Deps {
    pub db: DbHandle,
    pub models: ModelStore,
    pub fetch: Arc<dyn ModelFetch>,
    pub recognizer: Arc<dyn Recognizer>,
    pub songs: Arc<dyn SongReader>,
    pub other: Arc<dyn OtherLyrics>,
    pub conditions: Arc<dyn Conditions>,
    pub events: Arc<dyn TranscribeEvents>,
    /// Threads, the busy check and the breather (shared with the lyrics check).
    pub pacer: Arc<pace::Pacer>,
    /// The graphics card (shared with the lyrics check and the helper).
    pub gpu: Arc<gpu::Gpu>,
    /// How fast each model has been here.
    pub speeds: Arc<speed::Speeds>,
}

struct Inner {
    deps: Deps,
    st: Mutex<State>,
    wake: Notify,
    shared: parking_lot::RwLock<Option<Arc<dyn SharedResults>>>,
}

/// The transcription queue and model manager. Cheap to clone.
#[derive(Clone)]
pub struct Transcriber {
    inner: Arc<Inner>,
}

fn model_name(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Small => "small",
        ModelKind::Medium => "medium",
        ModelKind::Turbo => "turbo",
    }
}

fn mb(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

impl Transcriber {
    pub fn new(deps: Deps) -> Self {
        let me = Self {
            inner: Arc::new(Inner { deps, st: Mutex::new(State::default()), wake: Notify::new(), shared: Default::default() }),
        };
        // Once the card is known (or fails), the default model and Settings follow.
        let weak = Arc::downgrade(&me.inner);
        me.inner.deps.gpu.on_found(Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                let t = Transcriber { inner };
                t.inner.wake.notify_one();
                t.emit(true);
            }
        }));
        me
    }

    /// The model used now.
    fn model(&self, st: &State) -> ModelKind {
        ModelKind::chosen(st.model, self.inner.deps.gpu.active().is_some())
    }

    /// Settings: "Use the graphics card".
    pub fn set_gpu(&self, mode: gpu::GpuMode) {
        let d = &self.inner.deps;
        if d.gpu.mode() == mode {
            return;
        }
        d.gpu.set_mode(mode);
        self.inner.wake.notify_one();
        self.emit(true);
    }

    /// Waits (a little) for the graphics card to be known, so the default
    /// model fits this computer before a download starts.
    pub async fn settle_gpu(&self) {
        let g = self.inner.deps.gpu.clone();
        if g.known() || g.mode() == gpu::GpuMode::Off {
            return;
        }
        let _ = tokio::task::spawn_blocking(move || g.wait(Duration::from_secs(25))).await;
    }

    /// Connect results from the user's other devices.
    pub fn set_shared(&self, s: Arc<dyn SharedResults>) {
        *self.inner.shared.write() = Some(s);
    }

    fn shared(&self) -> Option<Arc<dyn SharedResults>> {
        self.inner.shared.read().clone()
    }

    /// Start the worker on the current runtime.
    pub fn spawn(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            me.restore_pause().await;
            me.run().await
        });
        let me = self.clone();
        tokio::spawn(async move { me.upkeep().await });
    }

    /// A song is being worked on or waits.
    fn working(&self) -> bool {
        let st = self.inner.st.lock();
        st.current.is_some() || !st.queue.is_empty()
    }

    /// Idle upkeep, every little while once nothing is queued and the
    /// computer is calm: transcriptions stored with an older timing are
    /// moved into place, and transcribed songs not looked at for a week are
    /// looked at for official synced lyrics again.
    async fn upkeep(self) {
        let pacer = self.inner.deps.pacer.clone();
        // Rounds to skip the official look after it couldn't run.
        let mut official_wait = 0u32;
        loop {
            tokio::time::sleep(pacer.timing().upkeep).await;
            if self.working() {
                continue;
            }
            official_wait = official_wait.saturating_sub(1);
            let (retime, official) = self.upkeep_due(official_wait == 0).await;
            if !retime && !official {
                // Nothing to do: the processor isn't read.
                continue;
            }
            let _readings = pacer.want();
            pacer.wait_calm().await;
            if self.working() {
                continue;
            }
            if retime {
                self.retime_old().await;
            }
            if official && self.recheck_official().await.is_none() {
                official_wait = 6;
            }
        }
    }

    /// Whether upkeep has work: old timing to move, songs due a look.
    async fn upkeep_due(&self, official: bool) -> (bool, bool) {
        let before = now_s() - OFFICIAL_RECHECK_S;
        self.inner
            .deps
            .db
            .read_async(move |c| {
                let retime = !user::auto_timed_before(c, engine::TIMING_VERSION, 1)?.is_empty();
                let due = official && !store::official_due(c, before, 1)?.is_empty();
                Ok((retime, due))
            })
            .await
            .unwrap_or((false, false))
    }

    /// Moves transcriptions made with an older timing into place (from the
    /// stored words, without hearing the songs again). Each is done once.
    /// Returns how many changed.
    pub async fn retime_old(&self) -> usize {
        let db = &self.inner.deps.db;
        let mut moved = 0;
        // Bounded: a row that keeps changing underneath can't hold this up.
        for _ in 0..10_000 {
            let rows = match db.read_async(|c| user::auto_timed_before(c, engine::TIMING_VERSION, RETIME_BATCH)).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("old transcriptions couldn't be read: {e}");
                    break;
                }
            };
            if rows.is_empty() {
                break;
            }
            let done = db
                .write_async(move |c| {
                    let mut n = 0;
                    for (id, body) in rows {
                        let new = lines::retime_v1(&body);
                        if user::retimed(c, &id, &body, new.as_deref(), engine::TIMING_VERSION)? && new.is_some() {
                            n += 1;
                        }
                    }
                    Ok(n)
                })
                .await;
            match done {
                Ok(n) => moved += n,
                Err(e) => {
                    tracing::warn!("old transcriptions couldn't be moved: {e}");
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
        if moved > 0 {
            tracing::info!(moved, "transcriptions moved to the current timing");
        }
        moved
    }

    /// Transcribed songs due a look for official synced lyrics, a few per
    /// round, stopping as soon as a song is asked for. How many now show
    /// official lyrics; None when the look couldn't run.
    pub async fn recheck_official(&self) -> Option<usize> {
        let d = &self.inner.deps;
        let before = now_s() - OFFICIAL_RECHECK_S;
        let due = d.db.read_async(move |c| store::official_due(c, before, OFFICIAL_PER_ROUND)).await.unwrap_or_default();
        let mut matched = 0;
        for id in due {
            if self.working() {
                break;
            }
            let found = d.other.official_synced(&id).await;
            if found == Official::Unknown {
                // The check can't run now: looked at again later.
                return None;
            }
            if found == Official::Matched {
                tracing::info!("official synced lyrics now shown in place of a transcription");
                matched += 1;
            }
            let now = now_s();
            let _ = d.db.write_async(move |c| store::official_checked(c, &id, now)).await;
        }
        Some(matched)
    }

    /// Settings: the switch and the chosen model. Turning it off stops
    /// everything (queue, current song, a download).
    pub fn set_options(&self, enabled: bool, model: Option<ModelKind>) {
        {
            let mut st = self.inner.st.lock();
            let changed = st.enabled != enabled || st.model != model;
            st.enabled = enabled;
            st.model = model;
            if !enabled {
                Self::stop_all(&mut st);
                if let Some(d) = st.download.take() {
                    d.cancel.store(true, Ordering::Relaxed);
                }
            }
            if !changed {
                return;
            }
        }
        self.inner.wake.notify_one();
        self.emit(true);
    }

    /// Settings: "Processor use", "Pause while the computer is busy" and
    /// "Work faster while I'm away".
    pub fn set_pace(&self, usage: pace::ProcessorUse, pause_when_busy: bool, faster_away: bool) {
        self.inner.deps.pacer.set_options(usage, pause_when_busy, faster_away);
        self.inner.wake.notify_one();
    }

    fn stop_all(st: &mut State) {
        st.queue.clear();
        if let Some(r) = &st.current {
            r.cancel.now();
        }
        st.batch = BatchProgress::default();
        st.paused = None;
        st.user_paused = false;
    }

    /// Pause the batch: the song in progress stops at the speech library's
    /// next check and goes back in the queue; the queue and progress stay,
    /// across a restart too. Songs asked for one at a time still run.
    pub fn pause(&self) {
        self.inner.st.lock().user_paused = true;
        self.inner.wake.notify_one();
        self.emit(true);
    }

    /// Carry on with a paused batch.
    pub fn resume(&self) {
        self.inner.st.lock().user_paused = false;
        self.inner.wake.notify_one();
        self.emit(true);
    }

    /// The batch as kept while paused (None: nothing to keep).
    fn pause_snapshot(st: &State) -> Option<PausedBatch> {
        if !st.user_paused {
            return None;
        }
        let mut ids: Vec<String> = st.current.iter().map(|r| r.job.track_id.clone()).collect();
        ids.extend(st.queue.iter().filter(|j| !j.now).map(|j| j.track_id.clone()));
        ids.dedup();
        (!ids.is_empty()).then_some(PausedBatch { ids, done: st.batch.done, total: st.batch.total })
    }

    /// Keeps the paused batch in the database when it changed.
    fn keep_pause(&self) {
        let body = {
            let mut st = self.inner.st.lock();
            let body = Self::pause_snapshot(&st).and_then(|p| serde_json::to_string(&p).ok()).unwrap_or_default();
            if st.pause_kept.as_deref() == Some(body.as_str()) {
                return;
            }
            st.pause_kept = Some(body.clone());
            body
        };
        if let Err(e) = self.inner.deps.db.write(|c| crate::db::store::kv_set(c, PAUSED_KEY, &body)) {
            tracing::warn!("paused transcription batch not kept: {e}");
        }
    }

    /// A batch paused before the last restart comes back, still paused.
    async fn restore_pause(&self) {
        let kept = self.inner.deps.db.read_async(|c| crate::db::store::kv_get(c, PAUSED_KEY)).await.ok().flatten();
        let Some(p) = kept.filter(|b| !b.is_empty()).and_then(|b| serde_json::from_str::<PausedBatch>(&b).ok()) else { return };
        {
            let mut st = self.inner.st.lock();
            for id in &p.ids {
                if !st.queue.iter().any(|j| &j.track_id == id) {
                    st.queue.push_back(Job { track_id: id.clone(), background: true, check_other: true, now: false, redo: false });
                }
            }
            st.batch = BatchProgress { done: p.done.min(p.total), total: p.total.max(p.ids.len()) };
            st.user_paused = true;
            st.pause_kept = serde_json::to_string(&p).ok();
        }
        self.inner.wake.notify_one();
        self.emit(true);
    }

    /// Songs done with another model than the chosen one go again with it.
    /// Returns how many were queued.
    pub fn redo_other_model(&self) -> AppResult<usize> {
        self.check_background()?;
        let added = {
            let mut st = self.inner.st.lock();
            let chosen = self.model(&st);
            if self.inner.deps.models.installed(chosen).is_none() {
                return Err(AppError::Invalid("Wait for the model download to finish first.".into()));
            }
            let ids: Vec<String> = st.other_model.iter().filter(|(_, k)| *k != chosen).map(|(id, _)| id.clone()).collect();
            let mut added = 0;
            for id in ids {
                if st.queue.iter().any(|j| j.track_id == id) || st.current.as_ref().is_some_and(|r| r.job.track_id == id) {
                    continue;
                }
                st.queue.push_back(Job { track_id: id, background: true, check_other: false, now: false, redo: true });
                added += 1;
            }
            st.batch.total += added;
            added
        };
        self.inner.wake.notify_one();
        self.emit(true);
        Ok(added)
    }

    pub fn status(&self) -> TranscribeStatus {
        let reason = self.inner.deps.recognizer.unavailable();
        let d = &self.inner.deps;
        let st = self.inner.st.lock();
        if st.enabled && reason.is_none() {
            d.gpu.look();
        }
        let card = d.gpu.active();
        let on_gpu = card.is_some();
        let models = ModelKind::ALL
            .iter()
            .filter_map(|k| self.inner.deps.models.spec(*k).map(|s| (k, s)))
            .map(|(k, s)| {
                let dl = st.download.as_ref().filter(|d| d.kind == *k);
                ModelInfo {
                    kind: *k,
                    size_bytes: s.size,
                    installed: self.inner.deps.models.installed(*k).is_some(),
                    downloading: dl.is_some(),
                    done_bytes: dl.map_or(0, |d| d.done),
                    secs_per_song: d.speeds.secs_per_song(*k, on_gpu),
                }
            })
            .collect();
        TranscribeStatus {
            enabled: st.enabled,
            available: reason.is_none(),
            unavailable_reason: reason,
            model: ModelKind::chosen(st.model, on_gpu),
            model_picked: st.model.is_some(),
            gpu: card.map(|c| c.name),
            gpu_found: d.gpu.device().map(|c| c.name),
            models,
            current: st.current.as_ref().map(|r| {
                let quiet = r.moved.elapsed();
                let said = quiet >= self.inner.deps.pacer.timing().stuck;
                CurrentJob { quiet_secs: said.then_some(quiet.as_secs() as u32), ..r.job.clone() }
            }),
            queued: st.queue.len(),
            up_next: st.queue.front().map(|j| j.track_id.clone()),
            batch: st.batch,
            secs_per_song: (st.batch.total > 1 && st.pace.1 > 0).then(|| (st.pace.0.as_secs() / st.pace.1 as u64) as u32),
            paused: st.paused,
            error: st.error.clone(),
            user_paused: st.user_paused,
            redo: {
                let chosen = ModelKind::chosen(st.model, on_gpu);
                let mut out: Vec<RedoInfo> = Vec::new();
                for (_, k) in st.other_model.iter().filter(|(_, k)| *k != chosen) {
                    match out.iter_mut().find(|r| r.model == *k) {
                        Some(r) => r.count += 1,
                        None => out.push(RedoInfo { model: *k, count: 1 }),
                    }
                }
                out
            },
        }
    }

    fn emit(&self, force: bool) {
        {
            let mut st = self.inner.st.lock();
            let now = Instant::now();
            if !force && st.last_emit.is_some_and(|t| now.duration_since(t) < EMIT_GAP) {
                // Sent by the next tick instead.
                st.dirty = true;
                return;
            }
            st.last_emit = Some(now);
            st.dirty = false;
        }
        self.keep_pause();
        let s = self.status();
        self.inner.st.lock().quiet_said = s.current.as_ref().and_then(|c| c.quiet_secs).map(|q| q / 60);
        self.inner.deps.events.status(&s);
    }

    /// While a song runs: sends progress the emit rate held back, and says
    /// when it has gone quiet (then each minute it stays quiet).
    fn tick(&self) {
        let (dirty, quiet_changed) = {
            let st = self.inner.st.lock();
            let quiet = st.current.as_ref().and_then(|r| {
                let q = r.moved.elapsed();
                (q >= self.inner.deps.pacer.timing().stuck).then_some(q.as_secs() as u32 / 60)
            });
            (st.dirty, quiet != st.quiet_said)
        };
        if quiet_changed {
            self.emit(true);
        } else if dirty {
            self.emit(false);
        }
    }

    fn check_usable(&self) -> AppResult<()> {
        if let Some(r) = self.inner.deps.recognizer.unavailable() {
            return Err(AppError::Invalid(r));
        }
        if !self.inner.st.lock().enabled {
            return Err(AppError::Invalid("Turn on transcription in Settings first.".into()));
        }
        Ok(())
    }

    /// Albums and the batch also stop for the session after repeated crashes.
    fn check_background(&self) -> AppResult<()> {
        self.check_usable()?;
        if self.inner.deps.recognizer.crashed_too_often() {
            return Err(AppError::Invalid(STOPPED.into()));
        }
        Ok(())
    }

    // ------------------------------------------------------------ models

    /// Start downloading a model. Progress and the outcome arrive as status events.
    pub fn download_model(&self, kind: ModelKind) -> AppResult<()> {
        if let Some(r) = self.inner.deps.recognizer.unavailable() {
            return Err(AppError::Invalid(r));
        }
        if self.inner.deps.models.spec(kind).is_none() {
            return Err(AppError::Invalid("unknown model".into()));
        }
        if self.inner.deps.models.installed(kind).is_some() {
            return Ok(());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut st = self.inner.st.lock();
            if st.download.is_some() {
                return Err(AppError::Invalid("A model is already downloading.".into()));
            }
            st.download = Some(Download { kind, done: 0, cancel: cancel.clone() });
            st.error = None;
        }
        self.emit(true);
        let me = self.clone();
        tokio::spawn(async move {
            let progress = {
                let me = me.clone();
                move |done: u64, _total: u64| {
                    if let Some(d) = me.inner.st.lock().download.as_mut() {
                        d.done = done;
                    }
                    me.emit(false);
                }
            };
            let res = me.inner.deps.models.download(kind, me.inner.deps.fetch.as_ref(), &cancel, &progress).await;
            {
                let mut st = me.inner.st.lock();
                st.download = None;
                st.error = match res {
                    Ok(_) => None,
                    Err(e) => Self::download_failure(e),
                };
                if st.error.is_some() {
                    // Songs waiting for this model can't go ahead.
                    let waiting = st.current.as_ref().is_some_and(|r| r.job.stage == Stage::WaitingForModel);
                    if waiting {
                        Self::stop_all(&mut st);
                    }
                }
            }
            me.inner.wake.notify_one();
            me.emit(true);
        });
        Ok(())
    }

    fn download_failure(e: DownloadError) -> Option<Failure> {
        let f = |kind, message: String| Some(Failure { kind, message, track_id: None });
        match e {
            DownloadError::Cancelled => None,
            DownloadError::Network(m) => {
                tracing::info!("model download failed: {m}");
                f(FailureKind::Download, "The model download failed. Check your connection and try again.".into())
            }
            DownloadError::Io(m) => {
                tracing::warn!("model download couldn't be saved: {m}");
                f(FailureKind::Download, "The model couldn't be saved on this computer. Try again.".into())
            }
            DownloadError::Checksum => f(
                FailureKind::Checksum,
                "The downloaded model didn't match its checksum, so it was deleted. Try again.".into(),
            ),
            DownloadError::Disk { needed, free } => f(
                FailureKind::Disk,
                format!("Not enough free space. The model needs {} MB free and {} MB is available.", mb(needed), mb(free)),
            ),
        }
    }

    pub fn cancel_download(&self) {
        if let Some(d) = self.inner.st.lock().download.as_ref() {
            d.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn delete_model(&self, kind: ModelKind) -> AppResult<()> {
        {
            let st = self.inner.st.lock();
            if st.download.as_ref().is_some_and(|d| d.kind == kind) {
                return Err(AppError::Invalid("Cancel the download first.".into()));
            }
            if st.current.is_some() && self.model(&st) == kind {
                return Err(AppError::Invalid("Cancel transcription first.".into()));
            }
        }
        self.inner.deps.recognizer.unload();
        self.inner.deps.models.delete(kind)?;
        self.emit(true);
        Ok(())
    }

    // ------------------------------------------------------------ queue

    /// Queue songs. Songs asked for now go before the background batch; a
    /// single song goes first. Returns how many were added.
    fn enqueue(&self, ids: Vec<String>, background: bool, check_other: bool, front: bool) -> AppResult<usize> {
        if front { self.check_usable()? } else { self.check_background()? }
        let start_download = {
            let st = self.inner.st.lock();
            self.inner.deps.models.installed(self.model(&st)).is_none() && st.download.is_none()
        };
        if start_download {
            let model = self.model(&self.inner.st.lock());
            self.download_model(model)?;
        }
        let added = {
            let mut st = self.inner.st.lock();
            let mut added = 0;
            let mut fronts = Vec::new();
            for id in ids {
                if st.current.as_ref().is_some_and(|r| r.job.track_id == id) {
                    continue;
                }
                if let Some(pos) = st.queue.iter().position(|j| j.track_id == id) {
                    if background || !st.queue[pos].background {
                        continue;
                    }
                    // Asked for now: lift it out of the batch.
                    st.queue.remove(pos);
                    st.batch.total = st.batch.total.saturating_sub(1);
                }
                let job = Job { track_id: id, background, check_other, now: front, redo: false };
                if front {
                    fronts.push(job);
                } else if background {
                    st.queue.push_back(job);
                } else {
                    let at = st.queue.iter().position(|j| j.background).unwrap_or(st.queue.len());
                    st.queue.insert(at, job);
                }
                added += 1;
            }
            for job in fronts.into_iter().rev() {
                st.queue.push_front(job);
            }
            st.batch.total += added;
            st.error = None;
            added
        };
        self.inner.wake.notify_one();
        self.emit(true);
        Ok(added)
    }

    /// One song, now (the lyrics view's "Transcribe this song").
    pub fn transcribe_track(&self, track_id: &str) -> AppResult<()> {
        self.enqueue(vec![track_id.to_owned()], false, false, true).map(|_| ())
    }

    /// One song in the background, when transcription is on and its model
    /// is installed (every lyrics candidate disagreed with the recording).
    /// Never downloads a model or says anything when it can't.
    pub fn transcribe_quietly(&self, track_id: &str) {
        let ready = {
            let st = self.inner.st.lock();
            st.enabled && self.inner.deps.models.installed(self.model(&st)).is_some()
        };
        let rec = &self.inner.deps.recognizer;
        if ready && rec.unavailable().is_none() && !rec.crashed_too_often() {
            let _ = self.enqueue(vec![track_id.to_owned()], true, false, false);
        }
    }

    /// An album's songs without lyrics.
    pub async fn transcribe_album(&self, album_id: &str) -> AppResult<usize> {
        self.check_background()?;
        self.settle_gpu().await;
        let id = album_id.to_owned();
        let ids = self
            .inner
            .deps
            .db
            .read_async(move |c| {
                let all = store::album_tracks(c, &id)?;
                let mut out = Vec::new();
                for t in all {
                    if !store::done(c, &t)? {
                        out.push(t);
                    }
                }
                Ok(out)
            })
            .await?;
        self.enqueue(ids, false, true, false)
    }

    /// Every unreleased song without lyrics, in the background.
    pub async fn transcribe_unreleased(&self) -> AppResult<usize> {
        self.check_background()?;
        self.settle_gpu().await;
        let ids = self.inner.deps.db.read_async(store::unreleased_todo).await?;
        self.enqueue(ids, true, true, false)
    }

    /// Cancel one song (queued or running), or everything.
    pub fn cancel(&self, track_id: Option<&str>) {
        {
            let mut st = self.inner.st.lock();
            match track_id {
                None => Self::stop_all(&mut st),
                Some(id) => {
                    let before = st.queue.len();
                    st.queue.retain(|j| j.track_id != id);
                    let removed = before - st.queue.len();
                    st.batch.total = st.batch.total.saturating_sub(removed);
                    if let Some(r) = &st.current
                        && r.job.track_id == id
                    {
                        // The helper gets about a second to stop.
                        r.cancel.now();
                    }
                }
            }
        }
        self.inner.wake.notify_one();
        self.emit(true);
    }

    /// Forget a song's transcription.
    pub async fn forget(&self, track_id: &str) -> AppResult<bool> {
        let id = track_id.to_owned();
        let had = self.inner.deps.db.write_async(move |c| store::forget(c, &id)).await?;
        if let Some(s) = self.shared() {
            s.forgot(track_id).await;
        }
        Ok(had)
    }

    // ------------------------------------------------------------ worker

    async fn run(self) {
        // Held while background songs wait or run (the busy check reads the
        // processor only then).
        let mut pace: Option<pace::PaceGuard> = None;
        loop {
            let notified = self.inner.wake.notified();
            // A breather between background songs so the processor cools down.
            let rest = {
                let st = self.inner.st.lock();
                match (st.enabled, st.queue.front(), st.rest_until) {
                    (true, Some(j), Some(until)) if !j.now => until.checked_duration_since(Instant::now()),
                    _ => None,
                }
            };
            if let Some(d) = rest.filter(|d| !d.is_zero()) {
                if pace.is_none() {
                    pace = Some(self.inner.deps.pacer.want());
                }
                tokio::select! {
                    _ = tokio::time::sleep(d) => {}
                    _ = notified => {}
                }
                continue;
            }
            let job = {
                let mut st = self.inner.st.lock();
                // Paused: only songs asked for one at a time.
                let at = if !st.enabled {
                    None
                } else if st.user_paused {
                    st.queue.iter().position(|j| j.now)
                } else {
                    (!st.queue.is_empty()).then_some(0)
                };
                let job = at.and_then(|i| st.queue.remove(i));
                job.map(|j| {
                    let background_left = !j.now || st.queue.iter().any(|q| !q.now);
                    (j, background_left)
                })
            };
            match job {
                Some((job, background_left)) => {
                    if !background_left {
                        drop(pace.take());
                    } else if pace.is_none() {
                        pace = Some(self.inner.deps.pacer.want());
                    }
                    self.process(job).await
                }
                None => {
                    drop(pace.take());
                    {
                        let mut st = self.inner.st.lock();
                        if st.queue.is_empty() && st.current.is_none() {
                            st.batch = BatchProgress::default();
                            st.paused = None;
                            st.rest_until = None;
                            st.pace = (Duration::ZERO, 0);
                            st.held = None;
                        }
                    }
                    self.inner.deps.recognizer.unload();
                    self.emit(true);
                    notified.await;
                }
            }
        }
    }

    /// Whether reading the song waits for playback (said while reading only).
    fn set_waiting(&self, yielding: bool) {
        let changed = {
            let mut st = self.inner.st.lock();
            let Some(r) = st.current.as_mut() else { return };
            let w = yielding && r.job.stage == Stage::Reading;
            let changed = r.job.waiting_for_playback != w;
            r.job.waiting_for_playback = w;
            changed
        };
        if changed {
            self.emit(true);
        }
    }

    fn set_stage(&self, stage: Stage, percent: u8) {
        let new_stage = {
            let mut st = self.inner.st.lock();
            let Some(r) = st.current.as_mut() else { return };
            let new_stage = r.job.stage != stage;
            if new_stage || r.job.percent != percent {
                r.moved = Instant::now();
            }
            r.job.stage = stage;
            r.job.percent = percent;
            new_stage
        };
        // Stage changes go out at once, percents at the emit rate.
        self.emit(new_stage);
    }

    /// The speech helper's progress, as the stage and percent.
    fn helper_step(&self, step: Step) {
        match step {
            Step::Loading => self.set_stage(Stage::Loading, 0),
            Step::Percent(p) => self.set_stage(Stage::Transcribing, p),
        }
    }

    fn fail(&self, track_id: &str, message: &str) {
        self.inner.st.lock().error =
            Some(Failure { kind: FailureKind::Transcribe, message: message.to_owned(), track_id: Some(track_id.to_owned()) });
    }

    async fn process(&self, job: Job) {
        let cancel = Stop::new();
        {
            let mut st = self.inner.st.lock();
            let current = CurrentJob {
                track_id: job.track_id.clone(),
                stage: Stage::Reading,
                percent: 0,
                background: job.background,
                quiet_secs: None,
                title: self.inner.deps.db.read(|c| store::title(c, &job.track_id)).ok().flatten(),
                waiting_for_playback: false,
                model: self.model(&st),
            };
            st.current = Some(Running { job: current, cancel: cancel.clone(), moved: Instant::now() });
        }
        self.emit(true);
        let started = Instant::now();
        let outcome = self.work(&job, &cancel).await;
        {
            let mut st = self.inner.st.lock();
            st.current = None;
            if matches!(outcome, Outcome::Done) && !job.now {
                let rest = self.inner.deps.pacer.timing().rest;
                st.rest_until = Some(Instant::now() + rest);
                st.pace.0 += started.elapsed() + rest;
                st.pace.1 += 1;
            }
            match outcome {
                Outcome::Retry => {
                    st.queue.push_back(job.clone());
                }
                Outcome::Again => {
                    // After the songs someone is waiting on.
                    let at = st.queue.iter().position(|j| !j.now).unwrap_or(st.queue.len());
                    st.queue.insert(at, job.clone());
                }
                Outcome::Cancelled => {
                    // Its audio isn't needed again.
                    if st.held.as_ref().is_some_and(|h| h.0 == job.track_id) {
                        st.held = None;
                    }
                }
                Outcome::Done | Outcome::Skipped | Outcome::Failed => {
                    st.batch.done = (st.batch.done + 1).min(st.batch.total);
                }
            }
            if matches!(outcome, Outcome::Failed) && self.inner.deps.recognizer.crashed_too_often() {
                // Background work stops for the session; songs asked for still run.
                let before = st.queue.len();
                st.queue.retain(|j| j.now);
                let dropped = before - st.queue.len();
                st.batch.total = st.batch.total.saturating_sub(dropped);
                st.error = Some(Failure { kind: FailureKind::Transcribe, message: STOPPED.into(), track_id: None });
            }
        }
        self.emit(true);
    }

    /// Waits out the pause conditions for work nobody is waiting on: the
    /// computer being busy, and battery power for the background batch.
    /// Gives way to a song someone asks for meanwhile.
    async fn wait_conditions(&self, cancel: &Stop, battery: bool) -> Waited {
        let pacer = &self.inner.deps.pacer;
        loop {
            if cancel.is_set() {
                return Waited::Cancelled;
            }
            let asked_for = {
                let st = self.inner.st.lock();
                st.user_paused || st.queue.front().is_some_and(|j| j.now)
            };
            if asked_for {
                self.inner.st.lock().paused = None;
                return Waited::GiveWay;
            }
            let mut reason = if battery { self.inner.deps.conditions.pause_reason().await } else { None };
            let load = pacer.load();
            if reason.is_none() && load == pace::Load::Busy {
                reason = Some(PauseReason::Busy);
            }
            // Not enough readings yet: wait a moment without saying paused.
            let settling = reason.is_none() && load == pace::Load::Unknown;
            let changed = {
                let mut st = self.inner.st.lock();
                let changed = st.paused != reason;
                st.paused = reason;
                changed
            };
            if changed {
                self.emit(true);
            }
            if reason.is_none() && !settling {
                return Waited::Go;
            }
            let poll = if settling { pacer.timing().sample } else { pacer.timing().poll };
            tokio::select! {
                _ = tokio::time::sleep(poll) => {}
                _ = self.inner.wake.notified() => {}
            }
        }
    }

    /// One song. A background song stops (and goes back at the front of the
    /// background songs, its audio kept) when someone asks for a song or a
    /// lyrics check needs the speech helper, or when someone is back at the
    /// computer while it runs with the extra threads. It stops at the speech
    /// library's next abort check and the helper, model loaded, goes on with
    /// what came first.
    async fn work(&self, job: &Job, cancel: &Stop) -> Outcome {
        let stop = Stop::new();
        let again = Arc::new(AtomicBool::new(false));
        let boosted = Arc::new(AtomicBool::new(false));
        let watch = {
            let (me, cancel, stop, again, boosted, now) =
                (self.clone(), cancel.clone(), stop.clone(), again.clone(), boosted.clone(), job.now);
            tokio::spawn(async move {
                loop {
                    if cancel.is_now() {
                        stop.now();
                    } else if cancel.is_set() {
                        stop.give_way();
                    }
                    let asked_for = !now && {
                        let st = me.inner.st.lock();
                        st.user_paused || st.queue.front().is_some_and(|j| j.now)
                    } || !now && me.inner.deps.recognizer.wanted();
                    me.set_waiting(me.inner.deps.songs.yielding());
                    let back = boosted.load(Ordering::Relaxed) && !me.inner.deps.pacer.away();
                    if (asked_for || back) && !stop.is_set() {
                        if back {
                            tracing::info!("someone is back at the computer: starting the song again with fewer threads");
                        } else {
                            tracing::debug!("a background song gives way");
                        }
                        again.store(true, Ordering::Relaxed);
                        stop.give_way();
                    }
                    me.tick();
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
        };
        let out = self.run_job(job, &stop, &boosted).await;
        watch.abort();
        match out {
            Outcome::Cancelled if again.load(Ordering::Relaxed) && !cancel.is_set() => Outcome::Again,
            o => o,
        }
    }

    async fn run_job(&self, job: &Job, cancel: &Stop, boosted: &AtomicBool) -> Outcome {
        let d = &self.inner.deps;
        let id = job.track_id.clone();
        // Already done (an earlier run or another trigger).
        let done_before = {
            let id = id.clone();
            d.db.read_async(move |c| store::done(c, &id)).await.unwrap_or(false)
        };
        if done_before && !job.redo {
            return Outcome::Skipped;
        }
        // Transcribed on another of the user's devices for this file.
        if let Some(s) = self.shared()
            && let Some(found) = s.take(&id).await
        {
            d.events.done(&id, found);
            return Outcome::Skipped;
        }
        // The model: wait for a download in progress.
        let path = loop {
            if cancel.is_set() {
                return Outcome::Cancelled;
            }
            let (kind, downloading) = {
                let st = self.inner.st.lock();
                (self.model(&st), st.download.is_some())
            };
            if let Some(p) = d.models.installed(kind) {
                break (p, kind);
            }
            // While the chosen model downloads, an installed one carries on.
            if downloading
                && !job.redo
                && let Some(other) = [ModelKind::Turbo, ModelKind::Medium, ModelKind::Small]
                    .into_iter()
                    .filter(|k| *k != kind)
                    .find_map(|k| d.models.installed(k).map(|p| (p, k)))
            {
                break other;
            }
            if !downloading {
                // A failed download already said why; keep that message.
                let said = self.inner.st.lock().error.as_ref().is_some_and(|e| e.kind != FailureKind::Transcribe);
                if !said {
                    self.fail(&id, "Download the transcription model in Settings first.");
                }
                return Outcome::Failed;
            }
            self.set_stage(Stage::WaitingForModel, 0);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                _ = self.inner.wake.notified() => {}
            }
        };
        let (model_path, kind) = path;
        if let Some(r) = self.inner.st.lock().current.as_mut() {
            r.job.model = kind;
        }
        if !job.now {
            match self.wait_conditions(cancel, job.background).await {
                Waited::Go => {}
                Waited::Cancelled => return Outcome::Cancelled,
                Waited::GiveWay => return Outcome::Again,
            }
        }
        if job.check_other && d.other.has_lyrics(&id).await {
            return Outcome::Skipped;
        }
        // Official synced lyrics that match the recording win: no transcription.
        if d.other.official_synced(&id).await == Official::Matched {
            tracing::info!("official synced lyrics match the song: not transcribed");
            let (id, now) = (id.clone(), now_s());
            let _ = d.db.write_async(move |c| store::official_checked(c, &id, now)).await;
            return Outcome::Skipped;
        }
        if cancel.is_set() {
            return Outcome::Cancelled;
        }
        let held = {
            let mut st = self.inner.st.lock();
            if st.held.as_ref().is_some_and(|h| h.0 == id) { st.held.take().map(|h| h.1) } else { None }
        };
        let pcm = match held {
            Some(p) => p,
            None => match self.read_song(job, cancel).await {
                Ok(p) => Arc::new(p),
                Err(o) => return o,
            },
        };
        self.set_stage(Stage::Transcribing, 0);
        // More threads because nobody's at the computer: back to fewer the
        // moment someone is (the song starts again). On the graphics card
        // the processor only feeds it: one or two threads.
        let on_gpu = d.gpu.active().is_some();
        let (threads, boost) = if on_gpu { (d.pacer.gpu_threads(), false) } else { d.pacer.threads_boosted(job.now) };
        boosted.store(boost, Ordering::Relaxed);
        let started = Instant::now();
        let seconds = pcm.len() / audio::RATE as usize;
        let res = {
            let rec = d.recognizer.clone();
            let (cancel, pcm) = (cancel.clone(), pcm.clone());
            let me = self.clone();
            let progress: engine::Progress = Arc::new(move |step| me.helper_step(step));
            tokio::task::spawn_blocking(move || rec.transcribe(&model_path, kind, &pcm, threads, engine::LANGUAGE, cancel, progress))
                .await
        };
        let segments = match res {
            Ok(Ok(s)) => {
                d.speeds.record(kind, on_gpu, pcm.len() as f64 / audio::RATE as f64, started.elapsed());
                s
            }
            Ok(Err(RecognizeError::Cancelled)) => {
                // Stopped for a song asked for: keep the audio for later.
                if !job.now {
                    self.inner.st.lock().held = Some((id.clone(), pcm));
                }
                return Outcome::Cancelled;
            }
            Ok(Err(RecognizeError::Crashed)) => {
                self.fail(&id, "The speech model stopped unexpectedly on this song.");
                return Outcome::Failed;
            }
            Ok(Err(RecognizeError::Failed(m))) => {
                tracing::warn!("transcription failed: {m}");
                self.fail(&id, "Transcription failed for this song.");
                return Outcome::Failed;
            }
            Err(e) => {
                tracing::warn!("transcription task failed: {e}");
                self.fail(&id, "Transcription failed for this song.");
                return Outcome::Failed;
            }
        };
        self.finish_song(job, kind, segments, seconds, started, threads, cancel).await
    }

    /// Reads and decodes a song.
    async fn read_song(&self, job: &Job, cancel: &Stop) -> Result<Vec<f32>, Outcome> {
        let d = &self.inner.deps;
        let id = job.track_id.clone();
        self.set_stage(Stage::Reading, 0);
        let song = match d.songs.read(&id, cancel.flag()).await {
            Ok(s) => s,
            Err(AudioError::Cancelled) => return Err(Outcome::Cancelled),
            Err(AudioError::Unreachable(m)) => {
                tracing::info!("transcription can't read the song now: {m}");
                if job.background {
                    self.inner.st.lock().paused = Some(PauseReason::Offline);
                    self.emit(true);
                    tokio::select! {
                        _ = tokio::time::sleep(OFFLINE_RETRY) => {}
                        _ = wait_flag(cancel.flag()) => {}
                    }
                    self.inner.st.lock().paused = None;
                    return Err(if cancel.is_set() { Outcome::Cancelled } else { Outcome::Retry });
                }
                self.fail(&id, "Couldn't read this song from your server. Try again when it's connected.");
                return Err(Outcome::Failed);
            }
            Err(AudioError::Unreadable(m)) => {
                tracing::info!("transcription can't use this file: {m}");
                self.fail(&id, "This song's file couldn't be read for transcription.");
                return Err(Outcome::Failed);
            }
        };
        self.set_stage(Stage::Decoding, 0);
        let pcm = {
            let cancel = cancel.flag().clone();
            let SongAudio { bytes, hint, .. } = song;
            tokio::task::spawn_blocking(move || audio::to_16k_mono(bytes, hint.as_deref(), &cancel)).await
        };
        match pcm {
            Ok(Ok(p)) => Ok(p),
            Ok(Err(AudioError::Cancelled)) => Err(Outcome::Cancelled),
            Ok(Err(e)) => {
                tracing::info!("transcription decode failed: {e:?}");
                self.fail(&id, "This song's file couldn't be decoded for transcription.");
                Err(Outcome::Failed)
            }
            Err(e) => {
                tracing::warn!("decode task failed: {e}");
                self.fail(&id, "Transcription failed.");
                Err(Outcome::Failed)
            }
        }
    }

    /// Lines from the words, saved.
    #[allow(clippy::too_many_arguments)]
    async fn finish_song(
        &self,
        job: &Job,
        kind: ModelKind,
        segments: Vec<lines::Segment>,
        seconds: usize,
        started: Instant,
        threads: usize,
        cancel: &Stop,
    ) -> Outcome {
        let d = &self.inner.deps;
        let id = job.track_id.clone();
        if cancel.is_set() {
            return Outcome::Cancelled;
        }
        let found_lines = lines::to_lines(&segments);
        let body = lines::to_lrc(&found_lines);
        tracing::info!(
            seconds,
            took_ms = started.elapsed().as_millis() as u64,
            threads,
            lines = found_lines.len(),
            model = model_name(kind),
            gpu = self.inner.deps.gpu.active().is_some(),
            "song transcribed"
        );
        let saved = {
            let (id, body, model, now) = (id.clone(), body.clone(), model_name(kind), now_s());
            d.db.write_async(move |c| store::save(c, &id, &body, model, now)).await
        };
        match saved {
            Ok(true) => {
                {
                    let mut st = self.inner.st.lock();
                    let chosen = self.model(&st);
                    st.other_model.retain(|(t, _)| *t != id);
                    if kind != chosen {
                        st.other_model.push((id.clone(), kind));
                    }
                }
                // Official lyrics were looked for just before.
                let (id2, now) = (id.clone(), now_s());
                let _ = d.db.write_async(move |c| store::official_checked(c, &id2, now)).await;
                if let Some(s) = self.shared() {
                    s.saved(&id, &body, model_name(kind)).await;
                }
                d.events.done(&id, !found_lines.is_empty())
            }
            // The user saved lyrics of their own meanwhile: theirs stay.
            Ok(false) => {}
            Err(e) => {
                tracing::warn!("transcription not saved: {e}");
                self.fail(&id, "The transcription couldn't be saved.");
                return Outcome::Failed;
            }
        }
        Outcome::Done
    }
}

enum Outcome {
    Done,
    Skipped,
    Failed,
    Cancelled,
    /// Put back at the end of the queue (the server was unreachable).
    Retry,
    /// Put back after the songs someone is waiting on (it gave way to one,
    /// or starts again with fewer threads).
    Again,
}

enum Waited {
    Go,
    Cancelled,
    /// A song someone is waiting on came first.
    GiveWay,
}

async fn wait_flag(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
