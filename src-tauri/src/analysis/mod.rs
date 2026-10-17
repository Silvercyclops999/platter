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
//! each file done once. Songs about to be mixed go first.

pub mod commands;
pub mod features;
pub mod store;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::{Notify, oneshot};

use crate::audio::automix::MixInfoSource;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::lyrics::transcribe::SongReader;
use crate::lyrics::transcribe::audio::AudioError;
use crate::lyrics::transcribe::pace::Pacer;
use features::Analysis;
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
        let _ = std::thread::Builder::new().name("analysis".into()).spawn(move || {
            lower_priority();
            while let Ok(job) = rx.recv() {
                job();
            }
        });
        Self(Mutex::new(tx))
    }

    async fn run<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> AppResult<T> {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move || {
            let _ = tx.send(f());
        });
        self.0.lock().send(job).map_err(|_| AppError::Internal("analysis worker stopped".into()))?;
        rx.await.map_err(|_| AppError::Internal("analysis worker stopped".into()))
    }
}

/// This thread runs behind everything else on the computer.
fn lower_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain syscalls on the calling thread.
    unsafe {
        let tid = libc::gettid();
        libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 19);
    }
    #[cfg(target_os = "windows")]
    // SAFETY: the pseudo handle for the calling thread.
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_IDLE};
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_IDLE);
    }
}

/// How one song went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Couldn't be read or decoded, or too long; not tried again until it changes.
    Skipped,
    /// The server couldn't be reached: stop for now.
    Later,
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
}

/// Called with the ids of songs once they're analysed.
pub type OnDone = Arc<dyn Fn(Vec<String>) + Send + Sync>;

impl Analyzer {
    /// `pacer`: the busy check shared with transcription (None: never wait).
    pub fn new(db: DbHandle, songs: Arc<dyn SongReader>, pacer: Option<Arc<Pacer>>) -> Self {
        Self(Arc::new(Inner {
            db,
            songs,
            pacer,
            worker: Worker::start(),
            running: AtomicBool::new(false),
            priority: Mutex::new(VecDeque::new()),
            on_done: Mutex::new(None),
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
    /// computer is calm. Some(Later) when the server couldn't be read.
    pub async fn run_round(&self, limit: usize, gap: Duration, keep_going: &(dyn Fn() -> bool + Sync)) -> AppResult<Option<Outcome>> {
        let _pace = self.0.pacer.as_ref().map(|p| p.want());
        let todo = self.todo(limit).await?;
        let mut done = Vec::new();
        let mut stop = None;
        for t in todo {
            if !keep_going() {
                break;
            }
            if let Some(p) = &self.0.pacer {
                p.wait_calm().await;
            }
            match self.analyse(&t).await? {
                Outcome::Done => done.push(t.track_id.clone()),
                Outcome::Skipped => {}
                Outcome::Later => {
                    stop = Some(Outcome::Later);
                    break;
                }
            }
            if !gap.is_zero() {
                tokio::time::sleep(gap).await;
            }
        }
        self.0.db.write_async(|c| store::prune(c)).await?;
        if !done.is_empty() {
            let f = self.0.on_done.lock().clone();
            if let Some(f) = f {
                f(done);
            }
        }
        Ok(stop)
    }

    /// Reads, decodes and analyses one song, and keeps what it found.
    pub async fn analyse(&self, t: &Todo) -> AppResult<Outcome> {
        if t.duration_ms > store::MAX_MS {
            let t = t.clone();
            self.0.db.write_async(move |c| store::save_status(c, &t, store::LONG, now_s())).await?;
            return Ok(Outcome::Skipped);
        }
        let cancel = AtomicBool::new(false);
        let audio = match self.0.songs.read(&t.track_id, &cancel).await {
            Ok(a) => a,
            Err(AudioError::Unreachable(m)) => {
                tracing::debug!("song analysis: {m}");
                return Ok(Outcome::Later);
            }
            Err(AudioError::Cancelled) => return Ok(Outcome::Later),
            Err(AudioError::Unreadable(m)) => {
                tracing::debug!("song analysis: {} unreadable: {m}", t.track_id);
                let t = t.clone();
                self.0.db.write_async(move |c| store::save_status(c, &t, store::FAILED, now_s())).await?;
                return Ok(Outcome::Skipped);
            }
        };
        let result = self
            .0
            .worker
            .run(move || -> Result<Option<Analysis>, String> {
                let Some(mono) = features::decode_mono(audio.bytes, audio.hint.as_deref())? else {
                    return Ok(None);
                };
                Ok(features::analyse(&mono))
            })
            .await?;
        let t2 = t.clone();
        match result {
            Ok(Some(a)) => {
                self.0.db.write_async(move |c| store::save(c, &t2, &a, now_s())).await?;
                Ok(Outcome::Done)
            }
            Ok(None) => {
                self.0.db.write_async(move |c| store::save_status(c, &t2, store::LONG, now_s())).await?;
                Ok(Outcome::Skipped)
            }
            Err(e) => {
                tracing::debug!("song analysis: {} couldn't be decoded: {e}", t.track_id);
                self.0.db.write_async(move |c| store::save_status(c, &t2, store::FAILED, now_s())).await?;
                Ok(Outcome::Skipped)
            }
        }
    }
}

#[async_trait]
impl MixInfoSource for Analyzer {
    async fn infos(&self, ids: &[String]) -> HashMap<String, Analysis> {
        let ids = ids.to_vec();
        self.0.db.read_async(move |c| store::get_many(c, &ids)).await.unwrap_or_default()
    }

    fn want(&self, ids: &[String]) {
        Analyzer::want(self, ids)
    }
}
