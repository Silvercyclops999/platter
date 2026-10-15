//! Library sync engine.
//!
//! - **First / full sync**: pages artists (8), albums (9), tracks (10) at
//!   [`PAGE_SIZE`] per page, one transaction per page, so everything synced so
//!   far is browsable while the rest loads. Every row is stamped with the
//!   run's generation; after a complete pass, rows not stamped are deleted.
//! - **Incremental**: items changed since the last sync (minus a skew
//!   margin), then deletion detection by comparing id sets.
//! - **Weekly full re-sync**: a run becomes full when the last full sync is
//!   older than [`FULL_RESYNC_AFTER_S`].
//! - **Library replaced**: the section uuid changed → wipe and rebuild.
//! - **No music library**: no music section → [`SyncOutcome::NoMusicLibrary`].
//!
//! Runs on a tokio task; DB writes go to the blocking pool. Cancellable
//! between pages. Progress goes to [`DbEvents::sync_progress`] and the
//! `sync_status` command; `library://changed` is throttled to one per
//! [`CHANGED_THROTTLE`] during long syncs plus one at the end.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::error::{AppError, AppResult};

use super::DbHandle;
use super::queries::now_s;
use super::source::{ItemKind, LibrarySource, MusicSection};
use super::store::{self, SyncState};
use super::types::{SyncOutcome, SyncPhase, SyncStatus};

pub const PAGE_SIZE: u64 = 500;
pub const FULL_RESYNC_AFTER_S: i64 = 7 * 24 * 3600;
/// Re-fetch a little before the last sync to cover clock skew and items
/// that changed while the previous sync ran.
const INCREMENTAL_SKEW_S: i64 = 120;
const HISTORY_PAGES: u64 = 2;
const CHANGED_THROTTLE: Duration = Duration::from_secs(2);
const SELECTED_SECTION_KEY: &str = "library.sectionId";

#[derive(Default)]
pub(crate) struct SyncController {
    status: Mutex<SyncStatus>,
    running: AtomicBool,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

impl SyncController {
    pub fn status(&self) -> SyncStatus {
        self.status.lock().clone()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        if let Some(c) = self.cancel.lock().as_ref() {
            c.store(true, Ordering::Release);
        }
    }
}

/// Spawn a background sync. Returns `false` if one is already running.
pub(crate) fn spawn(db: DbHandle, source: Arc<dyn LibrarySource>, full: bool) -> bool {
    if db.inner.sync.running.swap(true, Ordering::AcqRel) {
        return false;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *db.inner.sync.cancel.lock() = Some(cancel.clone());
    tokio::spawn(async move {
        let res = run_inner(db.clone(), source, full, cancel).await;
        if let Err(e) = res {
            tracing::warn!("library sync failed: {e}");
        }
        db.inner.sync.running.store(false, Ordering::Release);
        *db.inner.sync.cancel.lock() = None;
    });
    true
}

/// Run a sync to completion on the current task.
pub(crate) async fn run(
    db: DbHandle,
    source: Arc<dyn LibrarySource>,
    full: bool,
    cancel: Arc<AtomicBool>,
) -> AppResult<SyncOutcome> {
    if db.inner.sync.running.swap(true, Ordering::AcqRel) {
        return Err(AppError::Invalid("a sync is already running".into()));
    }
    *db.inner.sync.cancel.lock() = Some(cancel.clone());
    let res = run_inner(db.clone(), source, full, cancel).await;
    db.inner.sync.running.store(false, Ordering::Release);
    *db.inner.sync.cancel.lock() = None;
    res
}

struct Ctx {
    db: DbHandle,
    source: Arc<dyn LibrarySource>,
    cancel: Arc<AtomicBool>,
    last_changed_emit: Mutex<Option<Instant>>,
    last_sync_at: Option<i64>,
}

impl Ctx {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    fn progress(&self, phase: SyncPhase, done: u64, total: u64) {
        let status = SyncStatus { phase, done, total, last_sync_at: self.last_sync_at };
        *self.db.inner.sync.status.lock() = status.clone();
        if let Some(ev) = self.db.events() {
            ev.sync_progress(&status);
        }
    }

    /// Library rows changed: bump the generation and maybe tell the UI.
    fn changed(&self, force: bool) {
        self.db.bump_generation();
        let mut last = self.last_changed_emit.lock();
        let due = force || last.is_none_or(|t| t.elapsed() >= CHANGED_THROTTLE);
        if due {
            *last = Some(Instant::now());
            if let Some(ev) = self.db.events() {
                ev.library_changed();
            }
        }
    }
}

async fn run_inner(
    db: DbHandle,
    source: Arc<dyn LibrarySource>,
    full: bool,
    cancel: Arc<AtomicBool>,
) -> AppResult<SyncOutcome> {
    let prev = db.read_async(store::sync_state).await?;
    let ctx = Ctx {
        db: db.clone(),
        source,
        cancel,
        last_changed_emit: Mutex::new(None),
        last_sync_at: prev.as_ref().and_then(|p| p.last_sync_at),
    };
    let res = run_ctx(&ctx, prev, full).await;
    // Always settle back to idle, even on error.
    let last = db.read_async(store::sync_state).await.ok().flatten().and_then(|s| s.last_sync_at);
    let idle = SyncStatus { phase: SyncPhase::Idle, done: 0, total: 0, last_sync_at: last };
    *db.inner.sync.status.lock() = idle.clone();
    if let Some(ev) = db.events() {
        ev.sync_progress(&idle);
        if let Ok(outcome) = &res {
            if !matches!(outcome, SyncOutcome::Cancelled) {
                ev.library_changed();
            }
            ev.sync_outcome(outcome);
        }
    }
    res
}

async fn pick_section(ctx: &Ctx, prev: Option<&SyncState>) -> AppResult<Option<MusicSection>> {
    let sections = ctx.source.music_sections().await?;
    if sections.is_empty() {
        return Ok(None);
    }
    let wanted = ctx.db.read_async(|c| store::kv_get(c, SELECTED_SECTION_KEY)).await?;
    let by_id = |id: &str| sections.iter().find(|s| s.id == id).cloned();
    Ok(wanted
        .as_deref()
        .and_then(by_id)
        .or_else(|| prev.and_then(|p| sections.iter().find(|s| s.uuid == p.section_uuid).cloned()))
        .or_else(|| prev.and_then(|p| by_id(&p.section_id)))
        .or_else(|| sections.first().cloned()))
}

async fn run_ctx(ctx: &Ctx, prev: Option<SyncState>, full: bool) -> AppResult<SyncOutcome> {
    let started = now_s();
    let Some(section) = pick_section(ctx, prev.as_ref()).await? else {
        return Ok(SyncOutcome::NoMusicLibrary);
    };

    let replaced = prev.as_ref().is_some_and(|p| p.section_uuid != section.uuid);
    if replaced {
        tracing::info!(old = ?prev.as_ref().map(|p| &p.section_uuid), new = %section.uuid, "library replaced; rebuilding cache");
        ctx.db
            .write_async(|c| {
                let tx = c.transaction()?;
                store::wipe_library(&tx)?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        ctx.changed(true);
    }
    let prev = if replaced { None } else { prev };

    let weekly_due = prev
        .as_ref()
        .is_none_or(|p| p.last_full_sync_at.is_none_or(|t| started - t >= FULL_RESYNC_AFTER_S));
    let do_full = full || weekly_due || prev.as_ref().and_then(|p| p.last_sync_at).is_none();

    let mut state = prev.clone().unwrap_or(SyncState {
        section_id: section.id.clone(),
        section_uuid: section.uuid.clone(),
        section_title: Some(section.title.clone()),
        last_sync_at: None,
        last_full_sync_at: None,
        generation: 0,
    });
    state.section_id = section.id.clone();
    state.section_uuid = section.uuid.clone();
    state.section_title = Some(section.title.clone());

    let outcome = if do_full {
        state.generation += 1;
        // Record the section before paging so a restart mid-sync knows it.
        {
            let s = state.clone();
            ctx.db.write_async(move |c| store::put_sync_state(c, &s)).await?;
        }
        match full_pass(ctx, &section.id, state.generation).await? {
            None => return Ok(SyncOutcome::Cancelled),
            Some((artists, albums, tracks, deleted)) => {
                state.last_full_sync_at = Some(started);
                if replaced {
                    SyncOutcome::LibraryReplaced { artists, albums, tracks }
                } else {
                    SyncOutcome::Full { artists, albums, tracks, deleted }
                }
            }
        }
    } else {
        let since = state.last_sync_at.unwrap_or(0) - INCREMENTAL_SKEW_S;
        match incremental_pass(ctx, &section.id, since, state.generation).await? {
            None => return Ok(SyncOutcome::Cancelled),
            Some((changed, deleted)) => SyncOutcome::Incremental { changed, deleted },
        }
    };

    if ctx.cancelled() {
        return Ok(SyncOutcome::Cancelled);
    }
    sync_history(ctx, &section.id).await?;
    sync_playlists(ctx, do_full).await?;
    // Ratings changed in other Plex apps don't always show in updatedAt.
    if let Err(e) = sync_ratings(ctx, &section.id).await {
        tracing::warn!("ratings not synced: {e}");
    }

    state.last_sync_at = Some(started);
    let s = state.clone();
    ctx.db.write_async(move |c| store::put_sync_state(c, &s)).await?;
    ctx.changed(true);
    // Build the typo vocabulary now so the first search doesn't pay for it.
    let db = ctx.db.clone();
    if let Err(e) = ctx.db.read_async(move |c| super::search::warm(&db, c)).await {
        tracing::warn!("search vocab warm-up: {e}");
    }
    Ok(outcome)
}

/// Returns `None` when cancelled.
async fn full_pass(ctx: &Ctx, section_id: &str, gen_: i64) -> AppResult<Option<(u64, u64, u64, u64)>> {
    let mut counts = [0u64; 3];
    for (i, kind) in [ItemKind::Artist, ItemKind::Album, ItemKind::Track].into_iter().enumerate() {
        let phase = match kind {
            ItemKind::Artist => SyncPhase::Artists,
            ItemKind::Album => SyncPhase::Albums,
            ItemKind::Track => SyncPhase::Tracks,
        };
        let total = ctx.source.count(section_id, kind).await?;
        ctx.progress(phase, 0, total);
        let mut done = 0u64;
        loop {
            if ctx.cancelled() {
                return Ok(None);
            }
            let n = fetch_and_store_page(ctx, section_id, kind, done, gen_).await?;
            if n == 0 {
                break;
            }
            done += n;
            ctx.progress(phase, done, total.max(done));
            ctx.changed(false);
            if n < PAGE_SIZE {
                break;
            }
        }
        counts[i] = done;
        if kind == ItemKind::Album {
            ctx.db
                .write_async(|c| {
                    let tx = c.transaction()?;
                    store::refresh_artist_recency(&tx)?;
                    tx.commit()?;
                    Ok(())
                })
                .await?;
        }
    }
    if ctx.cancelled() {
        return Ok(None);
    }
    let deleted = ctx
        .db
        .write_async(move |c| {
            let tx = c.transaction()?;
            let mut n = 0;
            for k in [ItemKind::Track, ItemKind::Album, ItemKind::Artist] {
                n += store::delete_unseen(&tx, k, gen_)?;
            }
            tx.commit()?;
            Ok(n)
        })
        .await?;
    Ok(Some((counts[0], counts[1], counts[2], deleted)))
}

async fn fetch_and_store_page(
    ctx: &Ctx,
    section_id: &str,
    kind: ItemKind,
    start: u64,
    gen_: i64,
) -> AppResult<u64> {
    match kind {
        ItemKind::Artist => {
            let items = ctx.source.artists(section_id, start, PAGE_SIZE).await?;
            let n = items.len() as u64;
            ctx.db.write_async(move |c| {
                let tx = c.transaction()?;
                store::upsert_artists(&tx, &items, gen_)?;
                tx.commit()?;
                Ok(())
            })
            .await?;
            Ok(n)
        }
        ItemKind::Album => {
            let items = ctx.source.albums(section_id, start, PAGE_SIZE).await?;
            let n = items.len() as u64;
            ctx.db.write_async(move |c| {
                let tx = c.transaction()?;
                store::upsert_albums(&tx, &items, gen_)?;
                tx.commit()?;
                Ok(())
            })
            .await?;
            Ok(n)
        }
        ItemKind::Track => {
            let items = ctx.source.tracks(section_id, start, PAGE_SIZE).await?;
            let n = items.len() as u64;
            ctx.db.write_async(move |c| {
                let tx = c.transaction()?;
                store::upsert_tracks(&tx, &items, gen_)?;
                tx.commit()?;
                Ok(())
            })
            .await?;
            Ok(n)
        }
    }
}

/// Returns `None` when cancelled.
async fn incremental_pass(ctx: &Ctx, section_id: &str, since: i64, gen_: i64) -> AppResult<Option<(u64, u64)>> {
    ctx.progress(SyncPhase::Incremental, 0, 0);
    let changes = ctx.source.changed_since(section_id, since).await?;
    let changed = (changes.artists.len() + changes.albums.len() + changes.tracks.len()) as u64;
    ctx.progress(SyncPhase::Incremental, 0, changed);
    if ctx.cancelled() {
        return Ok(None);
    }
    if changed > 0 {
        ctx.db
            .write_async(move |c| {
                let tx = c.transaction()?;
                store::upsert_artists(&tx, &changes.artists, gen_)?;
                store::upsert_albums(&tx, &changes.albums, gen_)?;
                store::upsert_tracks(&tx, &changes.tracks, gen_)?;
                if !changes.albums.is_empty() {
                    store::refresh_artist_recency(&tx)?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        ctx.progress(SyncPhase::Incremental, changed, changed);
        ctx.changed(true);
    }

    // Deletions: compare id sets per kind.
    let mut keep = Vec::with_capacity(3);
    for kind in [ItemKind::Track, ItemKind::Album, ItemKind::Artist] {
        if ctx.cancelled() {
            return Ok(None);
        }
        let ids: HashSet<String> = ctx.source.all_ids(section_id, kind).await?.into_iter().collect();
        keep.push((kind, ids));
    }
    let deleted = ctx
        .db
        .write_async(move |c| {
            let tx = c.transaction()?;
            let mut n = 0;
            for (kind, ids) in &keep {
                n += store::delete_missing(&tx, *kind, ids)?;
            }
            tx.commit()?;
            Ok(n)
        })
        .await?;
    if deleted > 0 {
        ctx.changed(true);
    }
    Ok(Some((changed, deleted)))
}

async fn sync_history(ctx: &Ctx, section_id: &str) -> AppResult<()> {
    for page in 0..HISTORY_PAGES {
        let items = ctx.source.history(section_id, page * PAGE_SIZE, PAGE_SIZE).await?;
        let n = items.len() as u64;
        if n > 0 {
            ctx.db
                .write_async(move |c| {
                    let tx = c.transaction()?;
                    store::insert_history(&tx, &items)?;
                    tx.commit()?;
                    Ok(())
                })
                .await?;
        }
        if n < PAGE_SIZE {
            break;
        }
    }
    Ok(())
}

async fn sync_playlists(ctx: &Ctx, force_items: bool) -> AppResult<()> {
    let lists = ctx.source.playlists().await?;
    let stale = ctx
        .db
        .write_async(move |c| {
            let tx = c.transaction()?;
            let stale = store::replace_playlists(&tx, &lists, force_items)?;
            tx.commit()?;
            Ok(stale)
        })
        .await?;
    for id in stale {
        if ctx.cancelled() {
            break;
        }
        let items = match ctx.source.playlist_entries(&id).await {
            Ok(items) => items,
            // Orphaned smart playlists (left over from a deleted library)
            // answer 500. Drop them instead of failing the whole sync.
            Err(AppError::Server(_) | AppError::NotFound(_)) => {
                tracing::warn!("playlist {id} can't be listed by the server; skipping it");
                ctx.db
                    .write_async(move |c| {
                        c.execute("DELETE FROM playlists WHERE id = ?1", [&id])?;
                        Ok(())
                    })
                    .await?;
                continue;
            }
            Err(e) => return Err(e),
        };
        ctx.db
            .write_async(move |c| {
                let tx = c.transaction()?;
                // An edit made here while the items were loading wins.
                if !store::playlists_with_local_changes(&tx)?.contains(&id) {
                    store::set_playlist_items(&tx, &id, &items)?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
    }
    Ok(())
}

async fn sync_ratings(ctx: &Ctx, section_id: &str) -> AppResult<()> {
    let Some(rated) = ctx.source.rated_tracks(section_id).await? else { return Ok(()) };
    let changed = ctx
        .db
        .write_async(move |c| {
            let tx = c.transaction()?;
            let changed = super::likes::apply_rated(&tx, &rated)?;
            tx.commit()?;
            Ok(changed)
        })
        .await?;
    if !changed.is_empty() {
        tracing::debug!(n = changed.len(), "ratings changed in Plex");
    }
    Ok(())
}

/// Choose which music section to mirror (multi-library servers). Takes
/// effect on the next sync; a different section counts as "library
/// replaced" and rebuilds the cache.
pub fn select_section(db: &DbHandle, section_id: &str) -> AppResult<()> {
    db.write(|c| store::kv_set(c, SELECTED_SECTION_KEY, section_id))
}
