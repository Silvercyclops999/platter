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
//! - [`pace`]   threads, the busy pause and the breather between songs
//! - [`lines`]  timed words to lyric lines and LRC
//! - [`store`]  the `lyrics_auto` table
//! - [`commands`] Tauri commands, events and the app's side of the traits

pub mod audio;
pub mod commands;
pub mod engine;
pub mod helper;
pub mod lines;
pub mod model;
pub mod pace;
pub mod store;

#[cfg(test)]
mod tests;

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
use audio::AudioError;
use engine::{RecognizeError, Recognizer};
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
    /// The chosen model.
    pub model: ModelKind,
    pub models: Vec<ModelInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentJob>,
    /// Songs waiting (not counting the current one).
    pub queued: usize,
    pub batch: BatchProgress,
    /// About how long each song of a long run takes, breather included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secs_per_song: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<PauseReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
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
}

/// Whether a song already has lyrics from another source (Plex, LRCLIB when
/// on, or the user's own). Asked before transcribing an album or the batch.
#[async_trait]
pub trait OtherLyrics: Send + Sync {
    async fn has_lyrics(&self, track_id: &str) -> bool;
}

/// What pauses the background batch.
#[async_trait]
pub trait Conditions: Send + Sync {
    async fn pause_reason(&self) -> Option<PauseReason>;
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
}

struct Download {
    kind: ModelKind,
    done: u64,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    enabled: bool,
    model: ModelKind,
    queue: VecDeque<Job>,
    current: Option<(CurrentJob, Arc<AtomicBool>)>,
    batch: BatchProgress,
    paused: Option<PauseReason>,
    error: Option<Failure>,
    download: Option<Download>,
    last_emit: Option<Instant>,
    /// No background song starts before this (the breather).
    rest_until: Option<Instant>,
    /// Time taken by the background songs transcribed in this run, and how many.
    pace: (Duration, u32),
    /// A background song's decoded audio, kept when a song someone asked
    /// for stopped it, so it needn't be read again.
    held: Option<(String, Arc<Vec<f32>>)>,
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
}

struct Inner {
    deps: Deps,
    st: Mutex<State>,
    wake: Notify,
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
    }
}

fn mb(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

impl Transcriber {
    pub fn new(deps: Deps) -> Self {
        Self { inner: Arc::new(Inner { deps, st: Mutex::new(State::default()), wake: Notify::new() }) }
    }

    /// Start the worker on the current runtime.
    pub fn spawn(&self) {
        let me = self.clone();
        tokio::spawn(async move { me.run().await });
    }

    /// Settings: the switch and the chosen model. Turning it off stops
    /// everything (queue, current song, a download).
    pub fn set_options(&self, enabled: bool, model: ModelKind) {
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
        if let Some((_, c)) = &st.current {
            c.store(true, Ordering::Relaxed);
        }
        st.batch = BatchProgress::default();
        st.paused = None;
    }

    pub fn status(&self) -> TranscribeStatus {
        let reason = self.inner.deps.recognizer.unavailable();
        let st = self.inner.st.lock();
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
                }
            })
            .collect();
        TranscribeStatus {
            enabled: st.enabled,
            available: reason.is_none(),
            unavailable_reason: reason,
            model: st.model,
            models,
            current: st.current.as_ref().map(|c| c.0.clone()),
            queued: st.queue.len(),
            batch: st.batch,
            secs_per_song: (st.batch.total > 1 && st.pace.1 > 0).then(|| (st.pace.0.as_secs() / st.pace.1 as u64) as u32),
            paused: st.paused,
            error: st.error.clone(),
        }
    }

    fn emit(&self, force: bool) {
        {
            let mut st = self.inner.st.lock();
            let now = Instant::now();
            if !force && st.last_emit.is_some_and(|t| now.duration_since(t) < EMIT_GAP) {
                return;
            }
            st.last_emit = Some(now);
        }
        self.inner.deps.events.status(&self.status());
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
                    let waiting = st.current.as_ref().is_some_and(|c| c.0.stage == Stage::WaitingForModel);
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
            if st.current.is_some() && st.model == kind {
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
            self.inner.deps.models.installed(st.model).is_none() && st.download.is_none()
        };
        if start_download {
            let model = self.inner.st.lock().model;
            self.download_model(model)?;
        }
        let added = {
            let mut st = self.inner.st.lock();
            let mut added = 0;
            let mut fronts = Vec::new();
            for id in ids {
                if st.current.as_ref().is_some_and(|c| c.0.track_id == id) {
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
                let job = Job { track_id: id, background, check_other, now: front };
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
            st.enabled && self.inner.deps.models.installed(st.model).is_some()
        };
        let rec = &self.inner.deps.recognizer;
        if ready && rec.unavailable().is_none() && !rec.crashed_too_often() {
            let _ = self.enqueue(vec![track_id.to_owned()], true, false, false);
        }
    }

    /// An album's songs without lyrics.
    pub async fn transcribe_album(&self, album_id: &str) -> AppResult<usize> {
        self.check_background()?;
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
                    if let Some((c, flag)) = &st.current
                        && c.track_id == id
                    {
                        flag.store(true, Ordering::Relaxed);
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
        self.inner.deps.db.write_async(move |c| store::forget(c, &id)).await
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
                let job = if st.enabled { st.queue.pop_front() } else { None };
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

    fn set_stage(&self, stage: Stage, percent: u8) {
        if let Some((c, _)) = self.inner.st.lock().current.as_mut() {
            c.stage = stage;
            c.percent = percent;
        }
        self.emit(stage != Stage::Transcribing || percent == 0);
    }

    fn fail(&self, track_id: &str, message: &str) {
        self.inner.st.lock().error =
            Some(Failure { kind: FailureKind::Transcribe, message: message.to_owned(), track_id: Some(track_id.to_owned()) });
    }

    async fn process(&self, job: Job) {
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut st = self.inner.st.lock();
            let current = CurrentJob { track_id: job.track_id.clone(), stage: Stage::Reading, percent: 0, background: job.background };
            st.current = Some((current, cancel.clone()));
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
                Outcome::Cancelled => {}
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
    async fn wait_conditions(&self, cancel: &AtomicBool, battery: bool) -> Waited {
        let pacer = &self.inner.deps.pacer;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Waited::Cancelled;
            }
            let asked_for = self.inner.st.lock().queue.front().is_some_and(|j| j.now);
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

    /// One song. A background song stops (and goes back in the queue) when
    /// someone asks for a song, or when someone is back at the computer
    /// while it runs with the extra threads.
    async fn work(&self, job: &Job, cancel: &Arc<AtomicBool>) -> Outcome {
        let stop = Arc::new(AtomicBool::new(false));
        let again = Arc::new(AtomicBool::new(false));
        let boosted = Arc::new(AtomicBool::new(false));
        let watch = {
            let (me, cancel, stop, again, boosted, now) =
                (self.clone(), cancel.clone(), stop.clone(), again.clone(), boosted.clone(), job.now);
            tokio::spawn(async move {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        stop.store(true, Ordering::Relaxed);
                    }
                    let asked_for = !now && me.inner.st.lock().queue.front().is_some_and(|j| j.now);
                    let back = boosted.load(Ordering::Relaxed) && !me.inner.deps.pacer.away();
                    if asked_for || back {
                        if back {
                            tracing::info!("someone is back at the computer: starting the song again with fewer threads");
                        }
                        again.store(true, Ordering::Relaxed);
                        stop.store(true, Ordering::Relaxed);
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
        };
        let out = self.run_job(job, &stop, &boosted).await;
        watch.abort();
        match out {
            Outcome::Cancelled if again.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) => Outcome::Again,
            o => o,
        }
    }

    async fn run_job(&self, job: &Job, cancel: &Arc<AtomicBool>, boosted: &AtomicBool) -> Outcome {
        let d = &self.inner.deps;
        let id = job.track_id.clone();
        // Already done (an earlier run or another trigger).
        let done_before = {
            let id = id.clone();
            d.db.read_async(move |c| store::done(c, &id)).await.unwrap_or(false)
        };
        if done_before {
            return Outcome::Skipped;
        }
        // The model: wait for a download in progress.
        let path = loop {
            if cancel.load(Ordering::Relaxed) {
                return Outcome::Cancelled;
            }
            let (kind, downloading) = {
                let st = self.inner.st.lock();
                (st.model, st.download.is_some())
            };
            if let Some(p) = d.models.installed(kind) {
                break (p, kind);
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
        // moment someone is (the song starts again).
        let (threads, boost) = d.pacer.threads_boosted(job.now);
        boosted.store(boost, Ordering::Relaxed);
        let started = Instant::now();
        let seconds = pcm.len() / audio::RATE as usize;
        let res = {
            let rec = d.recognizer.clone();
            let (cancel, pcm) = (cancel.clone(), pcm.clone());
            let me = self.clone();
            let progress: engine::Progress = Arc::new(move |p| me.set_stage(Stage::Transcribing, p));
            tokio::task::spawn_blocking(move || rec.transcribe(&model_path, kind, &pcm, threads, engine::LANGUAGE, cancel, progress))
                .await
        };
        let segments = match res {
            Ok(Ok(s)) => s,
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
    async fn read_song(&self, job: &Job, cancel: &Arc<AtomicBool>) -> Result<Vec<f32>, Outcome> {
        let d = &self.inner.deps;
        let id = job.track_id.clone();
        self.set_stage(Stage::Reading, 0);
        let song = match d.songs.read(&id, cancel).await {
            Ok(s) => s,
            Err(AudioError::Cancelled) => return Err(Outcome::Cancelled),
            Err(AudioError::Unreachable(m)) => {
                tracing::info!("transcription can't read the song now: {m}");
                if job.background {
                    self.inner.st.lock().paused = Some(PauseReason::Offline);
                    self.emit(true);
                    tokio::select! {
                        _ = tokio::time::sleep(OFFLINE_RETRY) => {}
                        _ = wait_flag(cancel) => {}
                    }
                    self.inner.st.lock().paused = None;
                    return Err(if cancel.load(Ordering::Relaxed) { Outcome::Cancelled } else { Outcome::Retry });
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
            let cancel = cancel.clone();
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
        cancel: &AtomicBool,
    ) -> Outcome {
        let d = &self.inner.deps;
        let id = job.track_id.clone();
        if cancel.load(Ordering::Relaxed) {
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
            "song transcribed"
        );
        let saved = {
            let (id, model, now) = (id.clone(), model_name(kind), now_s());
            d.db.write_async(move |c| store::save(c, &id, &body, model, now)).await
        };
        match saved {
            Ok(true) => d.events.done(&id, !found_lines.is_empty()),
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
