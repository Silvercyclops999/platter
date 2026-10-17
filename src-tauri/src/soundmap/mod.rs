//! The sound map: every analysed song as a point on a flat map, placed so
//! songs that sound alike sit together. Made on this computer from the
//! feature vector song analysis keeps ([`crate::analysis`]), so it works
//! offline and nothing is sent anywhere.
//!
//! - [`project`]  the projection (plain arithmetic)
//! - [`store`]    the points and the projection, cached in the database
//! - [`commands`] the Tauri commands
//!
//! The work is light (a 20 by 20 matrix and one pass over the songs) and
//! runs on one thread of its own at the lowest priority. Songs analysed
//! later are placed with the projection already in use, so the map holds
//! still; it's worked out again only when the library of analysed songs
//! has grown by half, or the projection changes.

pub mod commands;
pub mod project;
pub mod store;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl Update {
    pub fn changed(&self) -> bool {
        self.refit || self.placed > 0 || self.removed > 0
    }
}

/// What an update will write, worked out on a read connection.
enum Plan {
    Layout(project::Basis, Vec<(String, String, f32, f32)>),
    Add(Vec<(String, String, f32, f32)>),
}

fn plan(c: &rusqlite::Connection) -> AppResult<Option<Plan>> {
    let current = store::basis(c)?;
    let count = store::analysed_count(c)?;
    let (basis, version) = match &current {
        Some((b, v)) => (Some(b), *v),
        None => (None, project::LAYOUT_VERSION),
    };
    if project::needs_refit(basis, version, count) {
        let all = store::analysed(c)?;
        let vectors: Vec<&[f32]> = all.iter().map(|a| a.features.as_slice()).collect();
        let Some(b) = project::fit(&vectors) else { return Ok(None) };
        let points = all
            .into_iter()
            .map(|a| {
                let (x, y) = project::place(&b, &a.features);
                (a.track_id, a.part_key, x, y)
            })
            .collect();
        return Ok(Some(Plan::Layout(b, points)));
    }
    let Some(b) = basis else { return Ok(None) };
    let todo = store::unplaced(c)?;
    if todo.is_empty() {
        return Ok(None);
    }
    Ok(Some(Plan::Add(
        todo.into_iter()
            .map(|a| {
                let (x, y) = project::place(b, &a.features);
                (a.track_id, a.part_key, x, y)
            })
            .collect(),
    )))
}

/// Brings the map up to date with the analyses: drops points of songs
/// that left or changed, places new ones, and works the projection out
/// again when it's due.
pub fn update(db: &DbHandle) -> AppResult<Update> {
    let removed = db.write(|c| store::prune(c))?;
    let mut out = Update { removed, ..Default::default() };
    match db.read(plan)? {
        Some(Plan::Layout(b, points)) => {
            out.refit = true;
            out.placed = points.len();
            db.write(|c| store::save_layout(c, &b, &points, now_s()))?;
        }
        Some(Plan::Add(points)) => {
            out.placed = points.len();
            db.write(|c| store::add_points(c, &points))?;
        }
        None => {}
    }
    Ok(out)
}

type Job = Box<dyn FnOnce() + Send>;

/// One thread at the lowest priority for the maths.
struct Worker(Mutex<std::sync::mpsc::Sender<Job>>);

impl Worker {
    fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let _ = std::thread::Builder::new().name("soundmap".into()).spawn(move || {
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
        self.0.lock().send(job).map_err(|_| AppError::Internal("sound map worker stopped".into()))?;
        rx.await.map_err(|_| AppError::Internal("sound map worker stopped".into()))
    }
}

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

/// Told when the map changed.
pub type OnChanged = Arc<dyn Fn() + Send + Sync>;

/// Keeps the map up to date in the background.
#[derive(Clone)]
pub struct SoundMap(Arc<Inner>);

struct Inner {
    db: DbHandle,
    worker: Worker,
    wake: Notify,
    building: AtomicBool,
    on_changed: Mutex<Option<OnChanged>>,
}

impl SoundMap {
    pub fn new(db: DbHandle) -> Self {
        Self(Arc::new(Inner { db, worker: Worker::start(), wake: Notify::new(), building: AtomicBool::new(false), on_changed: Mutex::new(None) }))
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
        self.0.building.load(Ordering::Relaxed)
    }

    /// One update on the low priority thread; tells listeners when it changed something.
    pub async fn refresh(&self) -> AppResult<Update> {
        self.0.building.store(true, Ordering::Relaxed);
        let db = self.0.db.clone();
        let r = self.0.worker.run(move || update(&db)).await;
        self.0.building.store(false, Ordering::Relaxed);
        let u = r??;
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
