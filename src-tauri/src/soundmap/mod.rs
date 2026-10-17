//! The sound map: every analysed song as a point on a flat map, placed so
//! songs that sound alike sit together. Made on this computer from the
//! feature vector song analysis keeps ([`crate::analysis`]), so it works
//! offline and nothing is sent anywhere.
//!
//! - [`project`]  the projection (plain arithmetic)
//! - [`store`]    the points and the projection, cached in the database
//! - [`commands`] the Tauri commands
//!
//! The work is light (a 20 by 20 matrix and a couple of passes over the
//! songs) and runs on one thread of its own at the lowest priority. Songs
//! analysed later are placed with the projection already in use, so the
//! map holds still; it's worked out again only when the library of analysed
//! songs has grown by half, or the projection changes, and then turned to
//! match the old one so the map doesn't flip over.

pub mod commands;
pub mod project;
pub mod store;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::{Notify, oneshot};

use crate::db::DbHandle;
use crate::error::{AppError, AppResult};

/// Fired when points were added, moved or removed.
pub const EV_CHANGED: &str = "soundmap://changed";

/// Startup work goes first.
const START_DELAY: Duration = Duration::from_secs(90);
/// Songs are analysed one at a time: gather a few before placing them.
const SETTLE: Duration = Duration::from_secs(5);

fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// What one update did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Update {
    /// The projection was worked out again and every point redone.
    pub refit: bool,
    /// Points written.
    pub placed: usize,
    /// Points dropped (song gone, or its file changed).
    pub removed: usize,
    /// Analyses newly marked as unusable.
    pub skipped: usize,
}

impl Update {
    pub fn changed(&self) -> bool {
        self.refit || self.placed > 0 || self.removed > 0 || self.skipped > 0
    }
}

/// What an update will write, worked out on a read connection.
#[derive(Default)]
struct Plan {
    /// A new projection (every point is redone), or None to add to the one in use.
    layout: Option<project::Basis>,
    points: Vec<store::Point>,
    skips: Vec<store::Skip>,
}

fn plan(c: &rusqlite::Connection) -> AppResult<Option<Plan>> {
    // One snapshot for the passes below.
    let _snapshot = c.unchecked_transaction()?;
    let current = store::basis(c)?;
    let count = store::analysed_count(c)?;
    let (basis, version) = match &current {
        Some((b, v)) => (Some(b), *v),
        None => (None, project::LAYOUT_VERSION),
    };
    let mut out = Plan::default();
    if project::needs_refit(basis, version, count) {
        // First pass: the projection, a vector at a time. Second: the points.
        let mut fit = project::Fit::default();
        store::each_analysed(c, &mut out.skips, |a| fit.add(&a.features))?;
        if let Some(b) = fit.finish(basis) {
            store::each_analysed(c, &mut Vec::new(), |a| {
                let (x, y) = project::place(&b, &a.features);
                out.points.push((a.track_id, a.part_key, x, y));
            })?;
            out.layout = Some(b);
        }
    } else if let Some(b) = basis {
        store::each_unplaced(c, &mut out.skips, |a| {
            let (x, y) = project::place(b, &a.features);
            out.points.push((a.track_id, a.part_key, x, y));
        })?;
    }
    Ok((out.layout.is_some() || !out.points.is_empty() || !out.skips.is_empty()).then_some(out))
}

/// Brings the map up to date with the analyses: drops points of songs
/// that left or changed, places new ones, and works the projection out
/// again when it's due. Points go in in chunks, each its own write.
pub fn update(db: &DbHandle) -> AppResult<Update> {
    let removed = db.write(|c| store::prune(c))?;
    let mut out = Update { removed, ..Default::default() };
    let Some(p) = db.read(plan)? else { return Ok(out) };
    if !p.skips.is_empty() {
        out.skipped = p.skips.len();
        db.write(|c| store::skip(c, &p.skips))?;
    }
    for chunk in p.points.chunks(store::WRITE_CHUNK) {
        db.write(|c| store::add_points(c, chunk))?;
    }
    out.placed = p.points.len();
    if let Some(b) = p.layout {
        out.refit = true;
        db.write(|c| store::finish_layout(c, &b, now_s()))?;
    }
    Ok(out)
}

type Job = Box<dyn FnOnce() + Send>;

/// What a panic said.
fn panic_message(p: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "panicked".into()
    }
}

/// One thread at the lowest priority for the maths. A job that panics is
/// logged and reported to its caller; the thread carries on.
struct Worker(Mutex<std::sync::mpsc::Sender<Job>>);

impl Worker {
    fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new().name("soundmap".into()).spawn(move || {
            lower_priority();
            while let Ok(job) = rx.recv() {
                if let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
                    tracing::error!("sound map: a job panicked: {}", panic_message(p));
                }
            }
        });
        if let Err(e) = spawned {
            tracing::error!("sound map: couldn't start its thread: {e}");
        }
        Self(Mutex::new(tx))
    }

    async fn run<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> AppResult<T> {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(panic_message);
            if let Err(m) = &r {
                tracing::error!("sound map: update panicked: {m}");
            }
            let _ = tx.send(r);
        });
        self.0.lock().send(job).map_err(|_| AppError::Internal("sound map worker stopped".into()))?;
        rx.await
            .map_err(|_| AppError::Internal("sound map worker stopped".into()))?
            .map_err(|m| AppError::Internal(format!("sound map update failed: {m}")))
    }
}

/// This thread runs behind everything else on the computer. Failures are
/// logged; the map still works, just at normal priority.
fn lower_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain syscalls on the calling thread.
    unsafe {
        let tid = libc::gettid();
        if libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 19) != 0 {
            tracing::warn!("sound map: couldn't lower its priority: {}", std::io::Error::last_os_error());
        }
    }
    #[cfg(target_os = "macos")]
    // SAFETY: sets the quality of service of the calling thread only.
    unsafe {
        let r = libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
        if r != 0 {
            tracing::warn!("sound map: couldn't lower its priority: {}", std::io::Error::from_raw_os_error(r));
        }
    }
    #[cfg(target_os = "windows")]
    // SAFETY: the pseudo handle for the calling thread.
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_IDLE};
        if let Err(e) = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_IDLE) {
            tracing::warn!("sound map: couldn't lower its priority: {e}");
        }
    }
}

/// Told when the map changed.
pub type OnChanged = Arc<dyn Fn() + Send + Sync>;

/// Counts updates under way; held for one update and let go however it
/// ends (finished, failed, or the caller gave up waiting), so the map can't
/// be left saying it's busy, and one update ending doesn't clear another's.
struct Building<'a>(&'a AtomicUsize);

impl<'a> Building<'a> {
    fn start(n: &'a AtomicUsize) -> Self {
        n.fetch_add(1, Ordering::AcqRel);
        Self(n)
    }
}

impl Drop for Building<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Keeps the map up to date in the background.
#[derive(Clone)]
pub struct SoundMap(Arc<Inner>);

struct Inner {
    db: DbHandle,
    worker: Worker,
    wake: Notify,
    building: AtomicUsize,
    on_changed: Mutex<Option<OnChanged>>,
}

impl SoundMap {
    pub fn new(db: DbHandle) -> Self {
        Self(Arc::new(Inner { db, worker: Worker::start(), wake: Notify::new(), building: AtomicUsize::new(0), on_changed: Mutex::new(None) }))
    }

    pub fn db(&self) -> &DbHandle {
        &self.0.db
    }

    pub fn set_on_changed(&self, f: OnChanged) {
        *self.0.on_changed.lock() = Some(f);
    }

    /// Songs were analysed (or the map was asked for): update soon.
    pub fn kick(&self) {
        self.0.wake.notify_one();
    }

    pub fn building(&self) -> bool {
        self.0.building.load(Ordering::Acquire) > 0
    }

    /// One update on the low priority thread; tells listeners when it changed something.
    pub async fn refresh(&self) -> AppResult<Update> {
        let u = {
            let _busy = Building::start(&self.0.building);
            let db = self.0.db.clone();
            self.0.worker.run(move || update(&db)).await??
        };
        if u.changed()
            && let Some(f) = self.0.on_changed.lock().clone()
        {
            f();
        }
        Ok(u)
    }

    /// Start the background work for the life of the app.
    pub fn spawn(&self) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            // A kick before the start delay still counts.
            let _ = tokio::time::timeout(START_DELAY, me.0.wake.notified()).await;
            loop {
                tokio::time::sleep(SETTLE).await;
                if let Err(e) = me.refresh().await {
                    tracing::warn!("sound map: {e}");
                }
                me.0.wake.notified().await;
            }
        });
    }
}
