//! SQLite cache, FTS5 search index and library sync (specs 03, 05, 06).
//!
//! Layout:
//! - [`schema`]   connection setup (WAL, pragmas) and versioned migrations
//! - [`source`]   the `LibrarySource` / `ArtFetcher` traits the integrator
//!   implements with the Plex client (this module never depends on `plex/`)
//! - [`store`]    row mapping and upserts
//! - [`queries`]  every Library read in docs/IPC.md
//! - [`sync`]     first / incremental / weekly full sync, deletions, library
//!   replaced detection
//! - [`search`]   local search (spec 05)
//! - [`listens`]  local listen log + offline report queue (spec 06)
//! - [`artcache`] artwork disk cache (LRU, size cap, request coalescing)
//! - [`commands`] Tauri command fns (registered by the integrator in lib.rs)
//!
//! Units: library timestamps (`added_at`, `last_viewed_at`, history
//! `viewed_at`) are unix **seconds** as Plex returns them; listen-log and
//! report-queue timestamps are unix **milliseconds**.

pub mod artcache;
pub mod commands;
pub mod listens;
pub mod queries;
pub mod release;
pub mod schema;
pub mod search;
pub mod source;
pub mod store;
pub mod sync;
pub mod types;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::Mutex;
use rusqlite::Connection;

use crate::error::{AppError, AppResult};

pub use artcache::ArtCache;
pub use release::release_section;
pub use source::{ArtFetcher, DbEvents, LibrarySource};
pub use types::*;

/// Number of idle read connections kept around.
const READ_POOL_MAX: usize = 4;

/// Shared database handle. Cheap to clone; manage one in Tauri state.
///
/// Writes (sync, listen log, recent searches) go through a single writer
/// connection behind a mutex. Reads use a small pool of read-only
/// connections; with WAL they never wait on a long sync write.
#[derive(Clone)]
pub struct DbHandle {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
    /// Bumped after every committed library write; search uses it to know
    /// when its in-memory vocabulary is stale.
    generation: AtomicU64,
    pub(crate) sync: sync::SyncController,
    source: Mutex<Option<Arc<dyn LibrarySource>>>,
    events: Mutex<Option<Arc<dyn DbEvents>>>,
    pub(crate) vocab: search::VocabCache,
}

impl DbHandle {
    /// Open (or create) the database at `path` and run migrations.
    pub fn open(path: impl AsRef<Path>) -> AppResult<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut writer = schema::open_writer(&path)?;
        schema::migrate(&mut writer)?;
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                writer: Mutex::new(writer),
                readers: Mutex::new(Vec::new()),
                generation: AtomicU64::new(1),
                sync: sync::SyncController::default(),
                source: Mutex::new(None),
                events: Mutex::new(None),
                vocab: search::VocabCache::default(),
            }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Plug in the Plex-backed library source (integrator).
    pub fn set_source(&self, source: Arc<dyn LibrarySource>) {
        *self.inner.source.lock() = Some(source);
    }

    pub fn source(&self) -> Option<Arc<dyn LibrarySource>> {
        self.inner.source.lock().clone()
    }

    /// Plug in the event sink (an `AppHandle` implements [`DbEvents`]).
    pub fn set_events(&self, events: Arc<dyn DbEvents>) {
        *self.inner.events.lock() = Some(events);
    }

    pub(crate) fn events(&self) -> Option<Arc<dyn DbEvents>> {
        self.inner.events.lock().clone()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::Acquire)
    }

    pub(crate) fn bump_generation(&self) {
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Run `f` on a pooled read-only connection.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
        let conn = match self.inner.readers.lock().pop() {
            Some(c) => c,
            None => schema::open_reader(&self.inner.path)?,
        };
        let out = f(&conn);
        let mut pool = self.inner.readers.lock();
        if pool.len() < READ_POOL_MAX {
            pool.push(conn);
        }
        out
    }

    /// Run `f` on the writer connection.
    pub fn write<T>(&self, f: impl FnOnce(&mut Connection) -> AppResult<T>) -> AppResult<T> {
        let mut conn = self.inner.writer.lock();
        f(&mut conn)
    }

    /// [`read`](Self::read) on the blocking pool, for async callers.
    pub async fn read_async<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> AppResult<T> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.read(f))
            .await
            .map_err(|e| AppError::Internal(format!("db task: {e}")))?
    }

    /// [`write`](Self::write) on the blocking pool, for async callers.
    pub async fn write_async<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.write(f))
            .await
            .map_err(|e| AppError::Internal(format!("db task: {e}")))?
    }

    /// Current sync status (for the `sync_status` command).
    pub fn sync_status(&self) -> SyncStatus {
        let mut s = self.inner.sync.status();
        if s.last_sync_at.is_none() {
            s.last_sync_at = self
                .read(|c| store::sync_state(c).map(|st| st.and_then(|s| s.last_sync_at)))
                .ok()
                .flatten();
        }
        s
    }

    /// Start a sync on a background tokio task unless one is already running.
    /// Must be called from within a tokio runtime (Tauri commands are).
    /// Returns `false` when a sync was already running.
    pub fn start_sync(&self, full: bool) -> AppResult<bool> {
        let source = self
            .source()
            .ok_or_else(|| AppError::Invalid("no library source configured".into()))?;
        Ok(sync::spawn(self.clone(), source, full))
    }

    /// Ask a running sync to stop after the current page.
    pub fn cancel_sync(&self) {
        self.inner.sync.cancel();
    }

    /// Run a sync inline (awaits completion). Used by tests and by callers
    /// that want the outcome.
    pub async fn sync_once(
        &self,
        source: Arc<dyn LibrarySource>,
        full: bool,
    ) -> AppResult<SyncOutcome> {
        sync::run(self.clone(), source, full, Arc::new(AtomicBool::new(false))).await
    }
}
