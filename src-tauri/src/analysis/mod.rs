//! The sound of each song, worked out on this computer: tempo and beat
//! grids, bars, key, energy, loudness over time, cue points and a feature
//! vector ([`features`]). Automix plans its transitions from it, and the
//! sound map will place songs by the feature vector.
//!
//! - [`features`] the analysis itself (pure Rust: an FFT and arithmetic)
//! - [`store`]    results per song, cached until the file changes
//! - [`commands`] the Tauri commands, and what the player reads
//!
//! The servers Platter plays from don't share beats or keys (Plex's sonic
//! analysis gives loudness, which leveling already uses, and similar
//! songs), so everything here is local.
//!
//! Work is light and only while the computer is otherwise idle: one song at
//! a time on one worker thread at the lowest priority, the file read at the
//! lowest priority on the player's transfer gate (from the pre-cache when
//! it's there), a breather between songs, waiting while the computer is
//! busy (the same busy check and away mode as lyrics transcription), and
//! each file done once. Songs about to be mixed go first. A song in
//! progress stops when Automix is turned off or the computer gets busy.

pub mod commands;
pub mod features;
pub mod store;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::{Notify, oneshot};

use crate::audio::automix::MixInfoSource;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::lyrics::transcribe::SongReader;
use crate::lyrics::transcribe::audio::AudioError;
use crate::lyrics::transcribe::pace::{Load, Pacer};
use features::{Analysis, Decoded, MIN_SECONDS};
use store::Todo;

/// Fired when songs get analysed (payload: their ids).
pub const EV_CHANGED: &str = "analysis://changed";

/// Startup work goes first.
const START_DELAY: Duration = Duration::from_secs(120);
const LOOK_EVERY: Duration = Duration::from_secs(300);
/// Songs per round, and the breather between them.
const PER_ROUND: usize = 10;
const GAP: Duration = Duration::from_secs(4);
/// After the server couldn't be read.
const BACKOFF: Duration = Duration::from_secs(300);
/// Songs asked for first, at most.
const PRIORITY_MAX: usize = 200;
/// Most of one file read into memory; bigger counts as too long.
const MAX_BYTES: u64 = 150 * 1024 * 1024;
/// How often songs no longer in the library are cleared out.
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How often a song in progress looks at whether to stop.
const STOP_POLL: Duration = Duration::from_millis(250);

static WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);
/// Automix is on (pushed by `settings::publish`).
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Analysis runs while Automix is on.
pub fn set_enabled(on: bool) {
    if ENABLED.swap(on, Ordering::SeqCst) != on && on {
        WAKE.notify_one();
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

type Job = Box<dyn FnOnce() + Send>;

/// One thread at the lowest priority that does the decoding and the maths.
struct Worker(Mutex<std::sync::mpsc::Sender<Job>>);

impl Worker {
    fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new().name("analysis".into()).spawn(move || {
            lower_priority();
            while let Ok(job) = rx.recv() {
                job();
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("song analysis: couldn't start its thread: {e}");
        }
        Self(Mutex::new(tx))
    }

    /// Runs `f` on the worker thread. The inner Err is a panic (what it
    /// said): the job is lost, the thread carries on with the next one.
    async fn run<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> AppResult<Result<T, String>> {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(panic_message);
            let _ = tx.send(r);
        });
        self.0.lock().send(job).map_err(|_| AppError::Internal("analysis worker stopped".into()))?;
        rx.await.map_err(|_| AppError::Internal("analysis worker stopped".into()))
    }
}

fn panic_message(p: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "panicked".into()
    }
}

/// This thread runs behind everything else on the computer, for the
/// processor and (where the system has it) for disk and memory too.
fn lower_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain syscalls on the calling thread.
    unsafe {
        let tid = libc::gettid();
        libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 19);
    }
    #[cfg(target_os = "macos")]
    // SAFETY: sets the quality of service of the calling thread only.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
    }
    #[cfg(target_os = "windows")]
    // SAFETY: the pseudo handle for the calling thread.
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_PRIORITY_IDLE};
        // Background mode lowers its disk and memory priority as well.
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_IDLE);
    }
}

/// How one song went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Too long to analyse; not tried again until it changes.
    Skipped,
    /// Couldn't be read (tried again a day later) or decoded (tried again
    /// when it changes or the analysis does).
    Failed,
    /// The server couldn't be reached: stop for now.
    Later,
    /// Stopped part way (Automix off, or the computer got busy): nothing
    /// kept, the song is done another time.
    Stopped,
}

/// What the worker made of a file.
enum Found {
    Analysis(Analysis),
    TooShort,
    TooLong,
    Stopped,
}

/// Progress for Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub on: bool,
    pub running: bool,
    #[serde(flatten)]
    pub counts: store::Counts,
}

/// The background analysis.
#[derive(Clone)]
pub struct Analyzer(Arc<Inner>);

struct Inner {
    db: DbHandle,
    songs: Arc<dyn SongReader>,
    pacer: Option<Arc<Pacer>>,
    worker: Worker,
    running: AtomicBool,
    priority: Mutex<VecDeque<String>>,
    on_done: Mutex<Option<OnDone>>,
    /// Most bytes read for one song.
    max_bytes: u64,
    /// When songs no longer in the library were last cleared out.
    pruned: Mutex<Option<Instant>>,
    /// Tests: the song whose analysis panics.
    #[cfg(test)]
    panic_on: Mutex<Option<String>>,
}

/// Called with the ids of songs once they're analysed.
pub type OnDone = Arc<dyn Fn(Vec<String>) + Send + Sync>;

impl Analyzer {
    /// `pacer`: the busy check shared with transcription (None: never wait).
    pub fn new(db: DbHandle, songs: Arc<dyn SongReader>, pacer: Option<Arc<Pacer>>) -> Self {
        Self::with_max_bytes(db, songs, pacer, MAX_BYTES)
    }

    fn with_max_bytes(db: DbHandle, songs: Arc<dyn SongReader>, pacer: Option<Arc<Pacer>>, max_bytes: u64) -> Self {
        Self(Arc::new(Inner {
            db,
            songs,
            pacer,
            worker: Worker::start(),
            running: AtomicBool::new(false),
            priority: Mutex::new(VecDeque::new()),
            on_done: Mutex::new(None),
            max_bytes,
            pruned: Mutex::new(None),
            #[cfg(test)]
            panic_on: Mutex::new(None),
        }))
    }

    pub fn db(&self) -> &DbHandle {
        &self.0.db
    }

    /// Called with the ids of songs once they're analysed.
    pub fn set_on_done(&self, f: OnDone) {
        *self.0.on_done.lock() = Some(f);
    }

    pub fn is_on(&self) -> bool {
        enabled()
    }

    /// Analyse these songs before any others (they're about to be mixed).
    pub fn want(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        {
            let mut p = self.0.priority.lock();
            for id in ids.iter().rev() {
                p.retain(|x| x != id);
                p.push_front(id.clone());
            }
            p.truncate(PRIORITY_MAX);
        }
        WAKE.notify_one();
    }

    pub async fn status(&self) -> AppResult<Status> {
        let counts = self.0.db.read_async(store::counts).await?;
        Ok(Status { on: self.is_on(), running: self.0.running.load(Ordering::Relaxed), counts })
    }

    /// Start the background work for the life of the app.
    pub fn spawn(&self) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.watch().await });
    }

    async fn watch(self) {
        tokio::time::sleep(START_DELAY).await;
        loop {
            let _ = tokio::time::timeout(LOOK_EVERY, WAKE.notified()).await;
            if !self.is_on() {
                continue;
            }
            self.0.running.store(true, Ordering::Relaxed);
            let me = self.clone();
            let r = self.run_round(PER_ROUND, GAP, &move || me.is_on()).await;
            self.0.running.store(false, Ordering::Relaxed);
            match r {
                Ok(Some(Outcome::Later)) => tokio::time::sleep(BACKOFF).await,
                Ok(_) => {
                    let more = !self.0.priority.lock().is_empty()
                        || self.0.db.read_async(|c| store::pending(c, 1)).await.is_ok_and(|v| !v.is_empty());
                    if more {
                        WAKE.notify_one();
                    }
                }
                Err(e) => tracing::warn!("song analysis: {e}"),
            }
        }
    }

    /// The songs to do next: the wanted ones still to analyse, then the
    /// newest of the rest.
    async fn todo(&self, limit: usize) -> AppResult<Vec<Todo>> {
        let wanted: Vec<String> = self.0.priority.lock().iter().cloned().collect();
        let mut out = if wanted.is_empty() { Vec::new() } else { self.0.db.read_async(move |c| store::pending_of(c, &wanted)).await? };
        // What's done (or gone) leaves the wanted list.
        {
            let left: std::collections::HashSet<&str> = out.iter().map(|t| t.track_id.as_str()).collect();
            self.0.priority.lock().retain(|id| left.contains(id.as_str()));
        }
        out.truncate(limit);
        if out.len() < limit {
            let rest = self.0.db.read_async(move |c| store::pending(c, limit)).await?;
            for t in rest {
                if out.len() >= limit {
                    break;
                }
                if !out.iter().any(|o| o.track_id == t.track_id) {
                    out.push(t);
                }
            }
        }
        Ok(out)
    }

    /// One round: up to `limit` songs, `gap` apart, each waiting until the
    /// computer is calm. Some(Later) when the server couldn't be read,
    /// Some(Stopped) when a song was stopped part way.
    pub async fn run_round(&self, limit: usize, gap: Duration, keep_going: &(dyn Fn() -> bool + Sync)) -> AppResult<Option<Outcome>> {
        let _pace = self.0.pacer.as_ref().map(|p| p.want());
        let todo = self.todo(limit).await?;
        let mut done = Vec::new();
        let mut failed = 0usize;
        let mut stop = None;
        let mut error = None;
        for t in todo {
            if !keep_going() {
                break;
            }
            if let Some(p) = &self.0.pacer {
                p.wait_calm().await;
            }
            match self.analyse(&t, keep_going).await {
                Ok(Outcome::Done) => done.push(t.track_id.clone()),
                Ok(Outcome::Skipped) => {}
                Ok(Outcome::Failed) => failed += 1,
                Ok(o @ (Outcome::Later | Outcome::Stopped)) => {
                    stop = Some(o);
                    break;
                }
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
            if !gap.is_zero() {
                tokio::time::sleep(gap).await;
            }
        }
        if failed > 0 {
            tracing::warn!("song analysis: {failed} song(s) couldn't be read or decoded this round");
        }
        // What's saved is announced even when a later song couldn't be.
        if !done.is_empty() {
            let f = self.0.on_done.lock().clone();
            if let Some(f) = f {
                f(done);
            }
        }
        if let Some(e) = error {
            return Err(e);
        }
        self.prune_now_and_then().await;
        Ok(stop)
    }

    /// Clears out songs no longer in the library, at most once a day and
    /// never while the library is syncing.
    async fn prune_now_and_then(&self) {
        if self.0.pruned.lock().is_some_and(|at| at.elapsed() < PRUNE_EVERY) || self.0.db.sync_running() {
            return;
        }
        match self.0.db.write_async(|c| store::prune(c)).await {
            Ok(n) => {
                *self.0.pruned.lock() = Some(Instant::now());
                if n > 0 {
                    tracing::debug!("song analysis: cleared {n} song(s) no longer in the library");
                }
            }
            Err(e) => tracing::warn!("song analysis: couldn't clear out old songs: {e}"),
        }
    }

    /// The computer is busy (with pausing while busy on).
    fn busy(&self) -> bool {
        self.0.pacer.as_ref().is_some_and(|p| p.load() == Load::Busy)
    }

    /// Waits for `fut`, setting `stop` as soon as Automix is turned off
    /// (`keep_going`) or the computer gets busy.
    async fn watching<F: std::future::Future>(&self, fut: F, stop: &AtomicBool, keep_going: &(dyn Fn() -> bool + Sync)) -> F::Output {
        tokio::pin!(fut);
        loop {
            tokio::select! {
                biased;
                r = &mut fut => return r,
                _ = tokio::time::sleep(STOP_POLL) => {
                    if !keep_going() || self.busy() {
                        stop.store(true, Ordering::SeqCst);
                    }
                }
            }
        }
    }

    /// Reads, decodes and analyses one song, and keeps what it found.
    pub async fn analyse(&self, t: &Todo, keep_going: &(dyn Fn() -> bool + Sync)) -> AppResult<Outcome> {
        let save_status = |status: &'static str| {
            let t = t.clone();
            self.0.db.write_async(move |c| store::save_status(c, &t, status, now_s()))
        };
        if t.duration_ms > store::MAX_MS {
            save_status(store::LONG).await?;
            return Ok(Outcome::Skipped);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let read = self.watching(self.0.songs.read_up_to(&t.track_id, &stop, self.0.max_bytes), &stop, keep_going).await;
        let audio = match read {
            Ok(Some(a)) => a,
            Ok(None) => {
                tracing::debug!("song analysis: {} is too big to read", t.track_id);
                save_status(store::LONG).await?;
                return Ok(Outcome::Skipped);
            }
            Err(AudioError::Unreachable(m)) => {
                tracing::info!("song analysis: the server can't be reached, trying again later: {m}");
                return Ok(Outcome::Later);
            }
            Err(AudioError::Cancelled) => return Ok(Outcome::Stopped),
            Err(AudioError::Unreadable(m)) => {
                tracing::warn!("song analysis: {} couldn't be read: {m}", t.track_id);
                save_status(store::UNREAD).await?;
                return Ok(Outcome::Failed);
            }
        };
        // A file much longer than the library says isn't decoded to the end.
        let limit_s = (t.duration_ms as f64 / 1000.0 * 1.5 + 30.0).max(MIN_SECONDS as f64);
        let job_stop = stop.clone();
        #[cfg(test)]
        let panics = self.0.panic_on.lock().as_deref() == Some(t.track_id.as_str());
        let job = self.0.worker.run(move || -> Result<Found, String> {
            #[cfg(test)]
            if panics {
                panic!("analysis blew up");
            }
            let mono = match features::decode_mono(audio.bytes, audio.hint.as_deref(), limit_s, &job_stop)? {
                Decoded::Audio(m) => m,
                Decoded::TooLong => return Ok(Found::TooLong),
                Decoded::Stopped => return Ok(Found::Stopped),
            };
            Ok(match features::analyse_until(&mono, &job_stop) {
                Ok(Some(a)) => Found::Analysis(a),
                Ok(None) => Found::TooShort,
                Err(features::Stopped) => Found::Stopped,
            })
        });
        let result = self.watching(job, &stop, keep_going).await?;
        match result {
            Ok(Ok(Found::Analysis(a))) => {
                let t = t.clone();
                self.0.db.write_async(move |c| store::save(c, &t, &a, now_s())).await?;
                Ok(Outcome::Done)
            }
            Ok(Ok(Found::TooLong)) => {
                save_status(store::LONG).await?;
                Ok(Outcome::Skipped)
            }
            Ok(Ok(Found::Stopped)) => Ok(Outcome::Stopped),
            Ok(Ok(Found::TooShort)) => {
                tracing::warn!("song analysis: {} decoded too short to analyse", t.track_id);
                save_status(store::FAILED).await?;
                Ok(Outcome::Failed)
            }
            Ok(Err(e)) => {
                tracing::warn!("song analysis: {} couldn't be decoded: {e}", t.track_id);
                save_status(store::FAILED).await?;
                Ok(Outcome::Failed)
            }
            Err(panic) => {
                tracing::warn!("song analysis: {} crashed the analysis: {panic}", t.track_id);
                save_status(store::FAILED).await?;
                Ok(Outcome::Failed)
            }
        }
    }
}

#[async_trait]
impl MixInfoSource for Analyzer {
    async fn infos(&self, ids: &[String]) -> AppResult<HashMap<String, Analysis>> {
        let ids = ids.to_vec();
        self.0.db.read_async(move |c| store::get_many(c, &ids)).await
    }

    fn want(&self, ids: &[String]) {
        Analyzer::want(self, ids)
    }
}
