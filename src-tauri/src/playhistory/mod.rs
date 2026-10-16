//! Listening from before Platter and from other apps: the signed-in user's
//! play history on their Plex server, Tautulli's history and Last.fm
//! scrobbles, brought into `imported_plays` so stats, mixes, recaps and
//! smart playlists count it (they all read `db::listens::ALL_PLAYS`, which
//! also keeps a play reported to several places from counting twice).
//!
//! - [`plex`]     the server's history, the user's own account only
//! - [`tautulli`] Tautulli's history (set up in Settings > Integrations)
//! - [`lastfm`]   the scrobbles of the account connected for scrobbling
//! - [`store`]    mapping plays to library songs, saving, progress, removing
//! - [`commands`] Tauri commands
//!
//! Each source runs on its own once it's set up and its Include setting is
//! on. The first run reads the whole history in the background, a page at
//! a time with a pause between pages, and can be stopped (it goes on from
//! the same page later). After that, each run reads only what's newer than
//! the newest play already saved, after every library sync and every few
//! hours. A run that fails says why in Settings and is tried again later.
//! Plays already saved are never saved twice.
//!
//! Turning a source's Include setting off leaves its plays out of every
//! count straight away (they stay saved); Remove deletes them.

pub mod commands;
pub mod lastfm;
pub mod plex;
pub mod store;
pub mod tautulli;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Serialize;

use crate::db::DbHandle;
use crate::db::listens::{ImportSource, set_excluded_sources};
use crate::error::{AppError, AppResult};
use crate::plex::state::EventSink;
use crate::settings::Settings;

pub const EVENT_STATUS: &str = "playhistory://status";
/// Plays asked for per page.
pub const PAGE_SIZE: u64 = 200;
/// The pause between pages, so a long first read stays in the background.
pub const PAGE_PAUSE: Duration = Duration::from_secs(1);
/// A failed run is tried again after this long (or on the next sync).
pub const RETRY_MS: i64 = 30 * 60 * 1000;
/// Without a sync, each source looks for new plays this often.
pub const REFRESH_MS: i64 = 6 * 3600 * 1000;
/// How often the watcher looks whether something is due.
const LOOK_EVERY: Duration = Duration::from_secs(60);
/// Wait after startup before the first look.
const START_DELAY: Duration = Duration::from_secs(45);

/// The sources this module brings in, in the order they run.
pub const SOURCES: [ImportSource; 3] = [ImportSource::PlexHistory, ImportSource::Tautulli, ImportSource::Lastfm];

/// One page from a source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    pub plays: Vec<Fetched>,
    /// More pages follow.
    pub more: bool,
    /// Plays the source has in all, when it says.
    pub total: Option<u64>,
    /// Plays on this page left out for being too short to count.
    pub short_plays: u64,
}

pub use store::Fetched;

/// A history source.
#[async_trait]
pub trait Feed: Send + Sync {
    fn source(&self) -> ImportSource;
    /// Set up and reachable in principle (signed in, address and key saved).
    async fn ready(&self) -> bool;
    /// Page `page` (from 0) of the history, newest first. With `since_ms`,
    /// only plays after it are needed (a source may still send older ones;
    /// they're skipped as already saved).
    async fn page(&self, page: u64, size: u64, since_ms: Option<i64>) -> AppResult<Page>;
}

/// Whether a source's Include setting is on.
pub fn included(s: &Settings, source: ImportSource) -> bool {
    match source {
        ImportSource::PlexHistory => s.include_plex_history,
        ImportSource::Tautulli => s.include_tautulli_history,
        ImportSource::Lastfm => s.include_lastfm_history,
        ImportSource::File => true,
    }
}

/// Wakes the watcher (a setting changed, the user asked for an update).
static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Follow the Include settings: sources turned off leave every count (their
/// plays stay saved), sources turned back on count again and may be brought
/// in again after a Remove. Called whenever settings are published.
pub fn follow_settings(db: &DbHandle, s: &Settings) {
    let off: Vec<ImportSource> = SOURCES.into_iter().filter(|x| !included(s, *x)).collect();
    let r = db.write(|c| {
        let before = crate::db::listens::excluded_sources(c)?;
        set_excluded_sources(c, &off)?;
        for src in SOURCES {
            if before.contains(&src) && !off.contains(&src) {
                let mut p = store::progress(c, src)?;
                if p.removed {
                    p.removed = false;
                    store::put_progress(c, src, &p)?;
                }
            }
        }
        Ok(())
    });
    if let Err(e) = r {
        tracing::warn!("play history settings not applied: {e}");
    }
    // An address or switch may have changed: look whether anything is due.
    WAKE.notify_one();
}

/// One source in Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    pub source: ImportSource,
    /// Set up: a Plex server, Tautulli's address and key, Last.fm connected.
    pub set_up: bool,
    /// The Include setting.
    pub included: bool,
    /// Plays brought in from it (saved; left out of counts while not included).
    pub plays: u64,
    /// Of those, plays of songs that aren't in the library.
    pub not_in_library: u64,
    /// Plays left out for being too short to count.
    pub short_plays: u64,
    /// Last finished run (unix ms).
    pub updated_at: Option<i64>,
    /// The whole history has been read once.
    pub first_read_done: bool,
    /// Reading now.
    pub importing: bool,
    /// Plays read so far in this run, and the source's total when known.
    pub done: u64,
    pub total: Option<u64>,
    /// Why the last run stopped, in words. Tried again later.
    pub error: Option<String>,
    /// The user removed its plays (nothing comes in until it's turned on again).
    pub removed: bool,
    /// The user stopped a read; it goes on when they ask for an update.
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStatus {
    pub sources: Vec<SourceStatus>,
}

/// The run going on now.
#[derive(Debug, Clone, Default)]
struct Running {
    source: Option<ImportSource>,
    done: u64,
    total: Option<u64>,
}

pub struct Deps {
    pub db: DbHandle,
    pub feeds: Vec<Arc<dyn Feed>>,
    pub events: Arc<dyn EventSink>,
    /// Unix ms.
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub pause: Duration,
    pub page_size: u64,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Read up to date; this many plays were new.
    Done(usize),
    /// Stopped by the user; it goes on later.
    Stopped,
    Failed,
}

#[derive(Clone)]
pub struct PlayHistory {
    inner: Arc<Deps>,
    running: Arc<Mutex<Running>>,
    cancel: Arc<AtomicBool>,
    lock: Arc<tokio::sync::Mutex<()>>,
}

/// What a failed run says in Settings.
fn plain(source: ImportSource, e: &AppError) -> String {
    let fallback = || match source {
        ImportSource::PlexHistory => "Your Plex history isn't available right now. Platter will try again later.".to_string(),
        ImportSource::Tautulli => "Couldn't reach Tautulli. Platter will try again later.".to_string(),
        ImportSource::Lastfm => "Couldn't reach Last.fm. Platter will try again later.".to_string(),
        ImportSource::File => "Couldn't read that history. Try again later.".to_string(),
    };
    match (source, e) {
        (ImportSource::PlexHistory, AppError::Auth(_)) => {
            "Plex didn't let Platter read your play history (only the server's owner and Home users can). Platter will try again later."
                .into()
        }
        (ImportSource::PlexHistory, AppError::Invalid(m)) => m.clone(),
        (ImportSource::PlexHistory, _) => fallback(),
        (_, AppError::Auth(m) | AppError::Invalid(m) | AppError::Network(m) | AppError::Server(m)) if !m.trim().is_empty() => m.clone(),
        _ => fallback(),
    }
}

impl PlayHistory {
    pub fn new(deps: Deps) -> Self {
        Self {
            inner: Arc::new(deps),
            running: Arc::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    fn feed(&self, source: ImportSource) -> Option<&Arc<dyn Feed>> {
        self.inner.feeds.iter().find(|f| f.source() == source)
    }

    pub async fn status(&self) -> AppResult<HistoryStatus> {
        let settings = crate::settings::load(&self.inner.db);
        let run = self.running.lock().clone();
        let mut out = Vec::new();
        for f in &self.inner.feeds {
            let source = f.source();
            let set_up = f.ready().await;
            let (counts, p) = self.inner.db.read_async(move |c| Ok((store::counts(c, source)?, store::progress(c, source)?))).await?;
            let now = run.source == Some(source);
            out.push(SourceStatus {
                source,
                set_up,
                included: included(&settings, source),
                plays: counts.0,
                not_in_library: counts.1,
                short_plays: p.short_plays,
                updated_at: p.updated_at,
                first_read_done: p.full_done,
                importing: now,
                done: if now { run.done } else { 0 },
                total: if now { run.total } else { None },
                error: p.error,
                removed: p.removed,
                paused: p.paused,
            });
        }
        Ok(HistoryStatus { sources: out })
    }

    async fn emit(&self) {
        if let Ok(s) = self.status().await {
            self.inner.events.emit(EVENT_STATUS, serde_json::to_value(s).unwrap_or_default());
        }
    }

    /// Stop the run going on now. It waits until the user asks for an
    /// update, then goes on from the same page.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Delete the plays brought in from `source` (nothing else).
    pub async fn remove(&self, source: ImportSource) -> AppResult<usize> {
        if !SOURCES.contains(&source) {
            return Err(AppError::Invalid("That source can't be removed here.".into()));
        }
        if self.running.lock().source == Some(source) {
            self.cancel();
        }
        let _g = self.lock.lock().await;
        let n = self.inner.db.write_async(move |c| store::remove(c, source)).await?;
        self.emit().await;
        Ok(n)
    }

    /// Run every source that is due. `synced`: the library synced since the
    /// last look. `forced`: the user asked (failures and timers are ignored).
    pub async fn run_due(&self, synced: bool, forced: bool) -> Vec<(ImportSource, Outcome)> {
        let settings = crate::settings::load(&self.inner.db);
        if settings.go_offline {
            return Vec::new();
        }
        let mut out = Vec::new();
        for f in self.inner.feeds.clone() {
            let source = f.source();
            if !included(&settings, source) || !f.ready().await {
                continue;
            }
            let Ok(p) = self.inner.db.read_async(move |c| store::progress(c, source)).await else {
                continue;
            };
            if p.removed || (p.paused && !forced) {
                continue;
            }
            let now = self.now();
            let failed_lately = p.error_at.is_some_and(|t| now - t < RETRY_MS);
            let due = forced
                || (!failed_lately && (!p.full_done || p.error.is_some() || synced || p.updated_at.is_none_or(|t| now - t >= REFRESH_MS)));
            if due {
                out.push((source, self.run(source).await));
            }
        }
        out
    }

    /// Read one source up to date.
    pub async fn run(&self, source: ImportSource) -> Outcome {
        let Some(feed) = self.feed(source).cloned() else {
            return Outcome::Failed;
        };
        let _g = self.lock.lock().await;
        self.cancel.store(false, Ordering::SeqCst);
        *self.running.lock() = Running { source: Some(source), done: 0, total: None };
        self.emit().await;
        let r = self.read(feed.as_ref()).await;
        *self.running.lock() = Running::default();
        let now = self.now();
        let outcome = match r {
            Ok(o) => o,
            Err(e) => {
                tracing::info!(source = source.as_str(), kind = e.kind(), "play history import failed");
                let msg = plain(source, &e);
                let _ = self
                    .inner
                    .db
                    .write_async(move |c| {
                        let mut p = store::progress(c, source)?;
                        p.error = Some(msg);
                        p.error_at = Some(now);
                        store::put_progress(c, source, &p)
                    })
                    .await;
                Outcome::Failed
            }
        };
        self.emit().await;
        outcome
    }

    async fn read(&self, feed: &dyn Feed) -> AppResult<Outcome> {
        let source = feed.source();
        let db = &self.inner.db;
        let start = db.read_async(move |c| Ok((store::progress(c, source)?, store::newest(c, source)?))).await?;
        let (mut prog, newest) = start;
        prog.paused = false;
        let since = if prog.full_done { newest } else { None };
        let mut page = if prog.full_done { 0 } else { prog.resume };
        let mut added = 0;
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                prog.paused = true;
                db.write_async(move |c| store::put_progress(c, source, &prog)).await?;
                return Ok(Outcome::Stopped);
            }
            let got = feed.page(page, self.inner.page_size, since).await?;
            let reached = since.is_some_and(|s| got.plays.iter().any(|p| p.played_at <= s));
            let ranks = db.source_ranks();
            let now = self.now();
            let plays = got.plays;
            let n = plays.len() as u64;
            added += db.write_async(move |c| store::save(c, source, &plays, ranks, now)).await?;
            prog.short_plays += got.short_plays;
            {
                let mut r = self.running.lock();
                r.done += n;
                r.total = got.total.or(r.total);
            }
            let more = got.more && !reached;
            if !prog.full_done {
                prog.resume = page + 1;
            }
            let p = prog.clone();
            db.write_async(move |c| store::put_progress(c, source, &p)).await?;
            self.emit().await;
            if !more {
                break;
            }
            page += 1;
            if !self.inner.pause.is_zero() {
                tokio::time::sleep(self.inner.pause).await;
            }
        }
        prog.full_done = true;
        prog.resume = 0;
        prog.updated_at = Some(self.now());
        prog.error = None;
        prog.error_at = None;
        db.write_async(move |c| store::put_progress(c, source, &prog)).await?;
        Ok(Outcome::Done(added))
    }

    /// Ask for an update now (Settings' Update button).
    pub fn poke() {
        WAKE.notify_one();
    }
}

/// Look for due sources for the life of the app: after startup, after each
/// library sync, every few hours and when poked.
pub async fn watch(ph: PlayHistory) {
    tokio::time::sleep(START_DELAY).await;
    let mut last_sync = ph.inner.db.sync_status().last_sync_at;
    let mut first = true;
    loop {
        let poked = if first {
            first = false;
            false
        } else {
            tokio::time::timeout(LOOK_EVERY, WAKE.notified()).await.is_ok()
        };
        let db = &ph.inner.db;
        if db.sync_running() {
            continue;
        }
        let now_sync = db.sync_status().last_sync_at;
        let synced = now_sync != last_sync;
        last_sync = now_sync;
        ph.run_due(synced, poked && FORCE.swap(false, Ordering::SeqCst)).await;
    }
}

/// The next poke is the user asking for an update (not a setting change).
static FORCE: AtomicBool = AtomicBool::new(false);

/// Settings' Update button: run every source now.
pub fn update_now() {
    FORCE.store(true, Ordering::SeqCst);
    WAKE.notify_one();
}
