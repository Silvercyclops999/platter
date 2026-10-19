//! What this device feeds into the sync store for its other devices, and
//! what it takes from them, for the parts of the app that don't write the
//! store themselves: saved looks (the themes folder), what's downloaded
//! here (an intent only) and plays brought in from elsewhere.
//!
//! Local state is checked every minute and stored when it changed; changes
//! from other devices are applied a moment after they stop arriving. Each
//! part follows its group (Settings > Sync).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::watch;

use crate::downloads::store::ItemRow;
use crate::error::AppResult;
use crate::sync::SyncStore;
use crate::sync::engine::Groups;
use crate::sync::store::{DOWNLOAD_INTENT, IMPORTED_PLAYS, LOOKS};
use crate::sync::{download_intent, imported_plays, looks};

#[derive(Debug, Clone, Copy)]
pub struct FeedTiming {
    /// How often local state is checked.
    pub every: Duration,
    /// Quiet time after changes from another device before applying them.
    pub settle: Duration,
}

impl Default for FeedTiming {
    fn default() -> Self {
        Self { every: Duration::from_secs(60), settle: Duration::from_secs(3) }
    }
}

/// What applying other devices' changes did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Look files written or deleted here.
    pub looks: usize,
    /// Plays added to this device's history.
    pub plays: usize,
}

/// What was last stored, so an unchanged check writes nothing.
#[derive(Default)]
struct Marks {
    downloads: Option<Vec<(String, String, String, i64)>>,
    plays: Option<(i64, i64)>,
}

pub struct Feeds {
    pub store: SyncStore,
    /// The themes folder (None where there isn't one).
    pub themes: Option<PathBuf>,
    /// This device's downloads.
    pub downloads: Arc<dyn Fn() -> Vec<ItemRow> + Send + Sync>,
    pub device_name: Arc<dyn Fn() -> String + Send + Sync>,
    pub groups: Arc<dyn Fn() -> Groups + Send + Sync>,
    pub timing: FeedTiming,
    marks: Mutex<Marks>,
    on_applied: Mutex<Option<OnApplied>>,
}

type OnApplied = Arc<dyn Fn(&Applied) + Send + Sync>;

impl Feeds {
    pub fn new(
        store: SyncStore,
        themes: Option<PathBuf>,
        downloads: Arc<dyn Fn() -> Vec<ItemRow> + Send + Sync>,
        device_name: Arc<dyn Fn() -> String + Send + Sync>,
        groups: Arc<dyn Fn() -> Groups + Send + Sync>,
        timing: FeedTiming,
    ) -> Arc<Self> {
        Arc::new(Self { store, themes, downloads, device_name, groups, timing, marks: Mutex::default(), on_applied: Mutex::default() })
    }

    /// Called when other devices' changes were applied here.
    pub fn set_on_applied(&self, f: OnApplied) {
        *self.on_applied.lock() = Some(f);
    }

    /// How many imported plays there are, and the newest row: changes when
    /// any import adds plays.
    fn plays_mark(&self) -> AppResult<(i64, i64)> {
        self.store.db().read(|c| Ok(c.query_row("SELECT COUNT(*), COALESCE(MAX(id), 0) FROM imported_plays", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
    }

    /// Store what changed here since the last check (blocking).
    pub fn publish(&self) -> AppResult<usize> {
        let g = (self.groups)();
        let mut n = 0;
        if g.looks
            && let Some(dir) = &self.themes
        {
            n += looks::publish(&self.store, dir)?;
        }
        if g.downloads {
            let items = (self.downloads)();
            let mark: Vec<_> = items.iter().map(|r| (r.key.kind.as_str().to_string(), r.key.id.clone(), r.title.clone(), r.added_at)).collect();
            if self.marks.lock().downloads.as_ref() != Some(&mark) {
                n += download_intent::publish(&self.store, &(self.device_name)(), &items)?;
                self.marks.lock().downloads = Some(mark);
            }
        }
        if g.plays {
            let mark = self.plays_mark()?;
            if self.marks.lock().plays != Some(mark) {
                n += imported_plays::publish(&self.store)?;
                // Read again: what was just stored is the new mark.
                self.marks.lock().plays = Some(self.plays_mark()?);
            }
        }
        Ok(n)
    }

    /// Apply what other devices brought, for the namespaces that changed
    /// (blocking).
    pub fn apply(&self, namespaces: &[String]) -> AppResult<Applied> {
        let g = (self.groups)();
        let has = |ns: &str| namespaces.iter().any(|n| n == ns);
        let mut out = Applied::default();
        if g.looks
            && has(LOOKS.name())
            && let Some(dir) = &self.themes
        {
            out.looks = looks::apply(&self.store, dir)?;
        }
        if g.plays && has(IMPORTED_PLAYS.name()) {
            out.plays = imported_plays::apply(&self.store, self.store.now() as i64)?;
            // Share the union back, so a history split across devices ends
            // whole everywhere.
            self.marks.lock().plays = None;
        }
        // Download intents are only read (as suggestions), never applied.
        let _ = has(DOWNLOAD_INTENT.name());
        Ok(out)
    }

    async fn blocking<R: Send + 'static>(self: &Arc<Self>, f: impl FnOnce(&Feeds) -> AppResult<R> + Send + 'static) -> Option<R> {
        let me = self.clone();
        match tokio::task::spawn_blocking(move || f(&me)).await {
            Ok(Ok(r)) => Some(r),
            Ok(Err(e)) => {
                tracing::warn!("device sync: {e}");
                None
            }
            Err(e) => {
                tracing::warn!("device sync: {e}");
                None
            }
        }
    }

    /// Check local state every so often, apply other devices' changes as
    /// they settle. Runs until `stop`.
    pub async fn run(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut changes = self.store.subscribe();
        let mut every = tokio::time::interval(self.timing.every);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut pending: Vec<String> = Vec::new();
        let mut due: Option<tokio::time::Instant> = None;
        loop {
            tokio::select! {
                _ = every.tick() => {
                    self.blocking(|f| f.publish()).await;
                }
                c = changes.recv() => match c {
                    Ok(c) if c.remote => {
                        for n in c.namespaces {
                            if !pending.contains(&n) {
                                pending.push(n);
                            }
                        }
                        due = Some(tokio::time::Instant::now() + self.timing.settle);
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        pending = vec![LOOKS.name().into(), IMPORTED_PLAYS.name().into()];
                        due = Some(tokio::time::Instant::now() + self.timing.settle);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
                _ = async { if let Some(t) = due { tokio::time::sleep_until(t).await } else { std::future::pending::<()>().await } } => {
                    due = None;
                    let ns = std::mem::take(&mut pending);
                    if let Some(applied) = self.blocking(move |f| f.apply(&ns)).await {
                        if applied != Applied::default()
                            && let Some(cb) = self.on_applied.lock().clone()
                        {
                            cb(&applied);
                        }
                        self.blocking(|f| f.publish()).await;
                    }
                }
                _ = stop.changed() => return,
            }
        }
    }
}
