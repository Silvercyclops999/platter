//! When mixes are made, and the snapshot they're made from.
//!
//! - Every mix is made again on the first check of a new day (days start at
//!   4 am local time), when the mix settings change, or when there were none
//!   and the library has filled in since.
//! - The daylist is made again when its block of hours changes (the Refresh
//!   daylist setting).
//! - Refresh on a mix makes just that one again with a new seed.
//!
//! All of it runs on the blocking pool. The library snapshot is kept while
//! the library, the listen log and the likes stay the same, so radio refills
//! don't reload it.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use parking_lot::Mutex;

use crate::db::DbHandle;
use crate::error::{AppError, AppResult};

use super::engine::{self, Ctx};
use super::library::{self, Lib};
use super::radio::{self, Seed};
use super::store::{self, Stored};
use super::{MixKind, MixOptions, MixPage, MixShelves, MixSteer, Variety};

/// Tells the UI the mixes changed (`mixes://changed`).
pub trait MixEvents: Send + Sync {
    fn changed(&self);
}

impl<R: tauri::Runtime> MixEvents for tauri::AppHandle<R> {
    fn changed(&self) {
        use tauri::Emitter;
        if let Err(e) = self.emit("mixes://changed", ()) {
            tracing::warn!("emit mixes://changed: {e}");
        }
    }
}

/// What a cached snapshot was built from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapKey {
    generation: u64,
    listens: i64,
    history: i64,
    likes: String,
    tracks: i64,
    options: String,
    hidden: u64,
}

struct Inner {
    db: DbHandle,
    opts: Mutex<MixOptions>,
    /// One generation at a time.
    gate: Mutex<()>,
    snap: Mutex<Option<(SnapKey, Arc<Lib>)>>,
    events: Mutex<Option<Arc<dyn MixEvents>>>,
    /// Minutes east of UTC, from the UI (i32::MIN until known).
    tz: AtomicI32,
    /// Hidden songs and artists (skipped by mixes, radio and autoplay).
    hidden: Mutex<crate::hidden::HiddenIds>,
}

#[derive(Clone)]
pub struct MixService {
    inner: Arc<Inner>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::Internal(format!("mix task: {e}")))?
}

/// A mix page without songs hidden since it was made.
fn without_hidden(mut page: MixPage, hidden: &crate::hidden::HiddenSet) -> MixPage {
    if hidden.is_empty() {
        return page;
    }
    let keep: Vec<bool> = page.tracks.iter().map(|t| !hidden.hides(&t.id, &t.artist_id)).collect();
    if keep.iter().all(|k| *k) {
        return page;
    }
    let mut k = keep.iter();
    page.tracks.retain(|_| *k.next().unwrap_or(&true));
    let mut k = keep.iter();
    page.familiar.retain(|_| *k.next().unwrap_or(&true));
    page.mix.track_count = page.tracks.len() as u32;
    page.mix.duration_ms = page.tracks.iter().map(|t| t.duration_ms).sum();
    page
}

fn day_seed(day: &str) -> u64 {
    day.bytes().fold(0x84222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Order on Home's first shelf.
fn made_for_you_rank(kind: MixKind) -> Option<u8> {
    Some(match kind {
        MixKind::Daily => 0,
        MixKind::Daylist => 1,
        MixKind::Discover => 2,
        MixKind::NewInLibrary => 3,
        MixKind::OnRepeat => 4,
        MixKind::Rediscover => 5,
        MixKind::TopSongs => 6,
        MixKind::Genre | MixKind::Mood | MixKind::Radio => return None,
    })
}

impl MixService {
    pub fn new(db: DbHandle) -> Self {
        let tz = db
            .read(|c| store::get_kv(c, store::KEY_TZ))
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(i32::MIN);
        Self {
            inner: Arc::new(Inner {
                db,
                opts: Mutex::new(MixOptions::default()),
                gate: Mutex::new(()),
                snap: Mutex::new(None),
                events: Mutex::new(None),
                tz: AtomicI32::new(tz),
                hidden: Mutex::new(Default::default()),
            }),
        }
    }

    pub fn set_events(&self, events: Arc<dyn MixEvents>) {
        *self.inner.events.lock() = Some(events);
    }

    /// New mix settings. Mixes are made again on the next check.
    pub fn set_options(&self, opts: MixOptions) {
        *self.inner.opts.lock() = opts;
    }

    pub fn options(&self) -> MixOptions {
        let mut o = self.inner.opts.lock().clone();
        o.hidden = self.hidden();
        o
    }

    /// Share the hidden songs and artists (the Hide service owns them).
    pub fn set_hidden_ids(&self, ids: crate::hidden::HiddenIds) {
        *self.inner.hidden.lock() = ids;
    }

    pub fn hidden(&self) -> Arc<crate::hidden::HiddenSet> {
        self.inner.hidden.lock().get()
    }

    /// The UI's time zone, minutes east of UTC (remembered for checks made
    /// before the UI asks).
    pub fn set_tz(&self, minutes: i32) {
        let minutes = minutes.clamp(-14 * 60, 14 * 60);
        if self.inner.tz.swap(minutes, Ordering::Relaxed) != minutes {
            let _ = self.inner.db.write(|c| store::set_kv(c, store::KEY_TZ, &minutes.to_string()));
        }
    }

    fn tz(&self) -> i32 {
        match self.inner.tz.load(Ordering::Relaxed) {
            i32::MIN => 0,
            m => m,
        }
    }

    fn ctx(&self, now_ms: i64, opts: &MixOptions) -> Ctx {
        let mut ctx = Ctx {
            now_ms,
            tz_offset_min: self.tz(),
            variety: opts.variety,
            seed: 0,
            daylist_hours: opts.daylist_hours.max(1),
        };
        ctx.seed = day_seed(&ctx.mix_day());
        ctx
    }

    fn snap_key(&self, opts: &MixOptions) -> AppResult<SnapKey> {
        let db = &self.inner.db;
        let (listens, history, likes, tracks) = db.read(|c| {
            Ok(c.query_row(
                "SELECT (SELECT coalesce(max(id), 0) FROM listen_log),
                        (SELECT count(*) FROM play_history),
                        (SELECT count(*) || ':' || coalesce(max(rated_at), 0) FROM tracks WHERE user_rating IS NOT NULL),
                        (SELECT count(*) FROM tracks)",
                [],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)),
            )?)
        })?;
        Ok(SnapKey { generation: db.generation(), listens, history, likes, tracks, options: opts.fingerprint(), hidden: opts.hidden.version })
    }

    /// The library snapshot (blocking).
    pub fn lib(&self) -> AppResult<Arc<Lib>> {
        let opts = self.options();
        let key = self.snap_key(&opts)?;
        if let Some((k, lib)) = self.inner.snap.lock().as_ref()
            && *k == key
        {
            return Ok(lib.clone());
        }
        let raw = self.inner.db.read(library::read_raw)?;
        let lib = Arc::new(Lib::build(raw, &opts, opts.liked_min));
        *self.inner.snap.lock() = Some((key, lib.clone()));
        Ok(lib)
    }

    fn emit(&self) {
        if let Some(e) = self.inner.events.lock().clone() {
            e.changed();
        }
    }

    /// Make whatever is due (blocking). True when something changed.
    pub fn ensure_fresh_blocking(&self, now_ms: i64) -> AppResult<bool> {
        let _g = self.inner.gate.lock();
        let opts = self.options();
        let ctx = self.ctx(now_ms, &opts);
        let db = &self.inner.db;
        let (day, slot, fp, tracks_then, n, tracks_now) = db.read(|c| {
            Ok((
                store::get_kv(c, store::KEY_DAY)?,
                store::get_kv(c, store::KEY_SLOT)?,
                store::get_kv(c, store::KEY_OPTIONS)?,
                store::get_kv(c, store::KEY_TRACKS)?,
                store::count(c)?,
                c.query_row("SELECT count(*) FROM tracks", [], |r| r.get::<_, i64>(0))?,
            ))
        })?;
        let today = ctx.mix_day();
        let filled_since = n == 0 && tracks_now > 0 && tracks_then.as_deref() != Some(&tracks_now.to_string());
        let all_due = day.as_deref() != Some(&today) || fp.as_deref() != Some(&opts.fingerprint()) || filled_since;
        if all_due {
            let lib = self.lib()?;
            let made: Vec<Stored> = engine::generate_all(&lib, &ctx).into_iter().map(|g| Stored::from_generated(&lib, g)).collect();
            let (today, slot, fp) = (today.clone(), ctx.daylist_slot(), opts.fingerprint());
            db.write(|c| {
                store::save_all(c, &made, &today, now_ms / 1000)?;
                store::set_kv(c, store::KEY_DAY, &today)?;
                store::set_kv(c, store::KEY_SLOT, &slot)?;
                store::set_kv(c, store::KEY_OPTIONS, &fp)?;
                store::set_kv(c, store::KEY_TRACKS, &tracks_now.to_string())
            })?;
            tracing::info!(mixes = made.len(), "mixes made");
            drop(_g);
            self.emit();
            return Ok(true);
        }
        if slot.as_deref() != Some(&ctx.daylist_slot()) {
            let lib = self.lib()?;
            let made = engine::daylist(&lib, &ctx).map(|g| Stored::from_generated(&lib, g));
            let slot = ctx.daylist_slot();
            db.write(|c| {
                match &made {
                    Some(m) => store::save_one(c, m, 0, &today, now_ms / 1000)?,
                    None => store::remove(c, "daylist")?,
                }
                store::set_kv(c, store::KEY_SLOT, &slot)
            })?;
            drop(_g);
            self.emit();
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn ensure_fresh(&self, now_ms: i64) -> AppResult<bool> {
        let s = self.clone();
        blocking(move || s.ensure_fresh_blocking(now_ms)).await
    }

    /// Home's shelves (making what's due first).
    pub async fn shelves(&self) -> AppResult<MixShelves> {
        self.ensure_fresh(now_ms()).await?;
        let db = self.inner.db.clone();
        let next = self.ctx(now_ms(), &self.options()).next_daylist_update();
        blocking(move || {
            let mut all = db.read(store::all)?;
            for (m, _, _) in all.iter_mut().filter(|(m, _, _)| m.kind == MixKind::Daylist) {
                m.next_update_at = Some(next);
            }
            let mut out = MixShelves::default();
            let mut first: Vec<(u8, usize, super::MixSummary)> = Vec::new();
            for (i, (m, _, _)) in all.into_iter().enumerate() {
                match (m.kind, made_for_you_rank(m.kind)) {
                    (_, Some(r)) => first.push((r, i, m)),
                    (MixKind::Mood, _) => out.moods.push(m),
                    _ => out.genres.push(m),
                }
            }
            first.sort_by_key(|(r, i, _)| (*r, *i));
            out.made_for_you = first.into_iter().map(|(_, _, m)| m).collect();
            Ok(out)
        })
        .await
    }

    /// A mix page. With a steering chip the same mix is made again leaning
    /// towards favourites or less played songs (not stored).
    pub async fn page(&self, id: String, steer: Option<MixSteer>) -> AppResult<MixPage> {
        self.ensure_fresh(now_ms()).await?;
        let s = self.clone();
        blocking(move || {
            let db = &s.inner.db;
            let (mut mix, tracks, familiar) =
                db.read(|c| store::one(c, &id))?.ok_or_else(|| AppError::NotFound(format!("mix {id}")))?;
            if mix.kind == MixKind::Daylist {
                mix.next_update_at = Some(s.ctx(now_ms(), &s.options()).next_daylist_update());
            }
            let Some(steer) = steer else {
                return Ok(without_hidden(MixPage { mix, tracks, familiar, steer: None }, &s.hidden()));
            };
            let opts = s.options();
            let mut ctx = s.ctx(now_ms(), &opts);
            ctx.variety = match steer {
                MixSteer::Familiar => Variety::Familiar,
                MixSteer::New => Variety::Adventurous,
            };
            let (stored, tracks, familiar) = s.remake(&id, &ctx)?;
            let mix = db.read(|c| store::summary_of(c, &stored, &tracks))?;
            Ok(without_hidden(MixPage { mix, tracks, familiar, steer: Some(steer) }, &s.hidden()))
        })
        .await
    }

    /// One mix made again (blocking): the stored mix, its songs and favourites.
    fn remake(&self, id: &str, ctx: &Ctx) -> AppResult<(Stored, Vec<crate::model::Track>, Vec<bool>)> {
        let lib = self.lib()?;
        let db = &self.inner.db;
        let others: HashSet<u32> = db
            .read(|c| store::daily_track_ids(c, id))?
            .into_iter()
            .filter_map(|t| lib.index.get(&t).copied())
            .collect();
        let g = engine::regenerate_one(&lib, ctx, id, &others).ok_or_else(|| AppError::NotFound(format!("mix {id}")))?;
        let stored = Stored::from_generated(&lib, g);
        let ids: Vec<String> = stored.tracks.iter().map(|(t, _)| t.clone()).collect();
        let fam: std::collections::HashMap<&str, bool> = stored.tracks.iter().map(|(t, f)| (t.as_str(), *f)).collect();
        let tracks = db.read(|c| crate::db::queries::tracks_by_ids(c, &ids))?;
        let familiar = tracks.iter().map(|t| fam.get(t.id.as_str()).copied().unwrap_or(true)).collect();
        Ok((stored, tracks, familiar))
    }

    /// Refresh: make one mix again with a new seed and keep it.
    pub async fn refresh(&self, id: String) -> AppResult<MixPage> {
        let s = self.clone();
        let page = blocking(move || {
            let _g = s.inner.gate.lock();
            let now = now_ms();
            let opts = s.options();
            let mut ctx = s.ctx(now, &opts);
            ctx.seed ^= now as u64;
            let (stored, tracks, familiar) = s.remake(&id, &ctx)?;
            let day = ctx.mix_day();
            let db = &s.inner.db;
            db.write(|c| store::save_one(c, &stored, usize::MAX >> 1, &day, now / 1000))?;
            let mix = db.read(|c| store::summary_of(c, &stored, &tracks))?;
            Ok(MixPage { mix, tracks, familiar, steer: None })
        })
        .await?;
        self.emit();
        Ok(page)
    }

    /// Radio songs (blocking): ids of the next `n`, none in `avoid`.
    pub fn radio_ids(&self, seed: &Seed, avoid: &HashSet<String>, n: usize, first: bool) -> AppResult<Vec<String>> {
        let lib = self.lib()?;
        let taste = engine::Taste::new(&lib);
        let salt = now_ms() as u64;
        Ok(radio::batch(&lib, &taste, seed, avoid, n, salt, first, radio::LEAN_PLAIN)
            .into_iter()
            .map(|t| lib.tracks[t as usize].id.clone())
            .collect())
    }

    /// Smart shuffle suggestions (see [`radio::suggestions`]), as track ids.
    pub fn suggestion_ids(
        &self,
        seed_ids: &[String],
        avoid: &HashSet<String>,
        avoid_artists: &HashSet<String>,
        n: usize,
    ) -> AppResult<Vec<String>> {
        let lib = self.lib()?;
        let taste = engine::Taste::new(&lib);
        let salt = now_ms() as u64;
        Ok(radio::suggestions(&lib, &taste, seed_ids, avoid, avoid_artists, n, salt)
            .into_iter()
            .map(|t| lib.tracks[t as usize].id.clone())
            .collect())
    }
}

impl MixService {
    /// A radio page: about 50 songs like the seed, starting with it (made
    /// fresh each time it opens; the same `salt` gives the same page; a
    /// steering chip leans it one way).
    pub async fn radio_page(&self, seed_id: String, steer: Option<MixSteer>, salt: Option<u64>) -> AppResult<MixPage> {
        let s = self.clone();
        blocking(move || {
            let seed = Seed::parse(&seed_id).ok_or_else(|| AppError::Invalid(format!("not a radio: {seed_id}")))?;
            let lib = s.lib()?;
            let taste = engine::Taste::new(&lib);
            let lean = match steer {
                Some(MixSteer::Familiar) => radio::LEAN_FAMILIAR,
                Some(MixSteer::New) => radio::LEAN_NEW,
                None => radio::LEAN_PLAIN,
            };
            let salt = salt.unwrap_or_else(|| now_ms() as u64);
            let picked = radio::batch(&lib, &taste, &seed, &HashSet::new(), radio::RADIO_PAGE_LEN, salt, true, lean);
            let ids: Vec<String> = picked.iter().map(|t| lib.tracks[*t as usize].id.clone()).collect();
            let familiar: std::collections::HashMap<&str, bool> =
                picked.iter().map(|t| (lib.tracks[*t as usize].id.as_str(), lib.familiar(*t))).collect();
            let db = &s.inner.db;
            let tracks = db.read(|c| crate::db::queries::tracks_by_ids(c, &ids))?;
            let familiar = tracks.iter().map(|t| familiar.get(t.id.as_str()).copied().unwrap_or(false)).collect();
            // The seed's name and picture, then the artists the radio leans on.
            let seed_artist = match &seed {
                Seed::Track(id) => lib.index.get(id).map(|t| lib.tracks[*t as usize].artist),
                Seed::Artist(id) => lib.artists.iter().position(|a| a.id == *id).map(|a| a as u32),
                Seed::Tracks(_) => None,
            };
            let (name, picture) = match &seed {
                Seed::Track(id) => {
                    let t = tracks.iter().find(|t| t.id == *id).cloned();
                    (t.as_ref().map(|t| t.title.clone()), t.and_then(|t| t.thumb))
                }
                Seed::Artist(_) => {
                    let a = seed_artist.map(|a| &lib.artists[a as usize]);
                    (a.map(|a| a.title.clone()), a.and_then(|a| a.thumb.clone()))
                }
                Seed::Tracks(_) => (None, None),
            };
            let name = name.ok_or_else(|| AppError::NotFound(format!("radio {seed_id}")))?;
            let mut lead: Vec<u32> = seed_artist.into_iter().collect();
            let mut count: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
            for t in &picked {
                *count.entry(lib.tracks[*t as usize].artist).or_default() += 1;
            }
            let mut others: Vec<(u32, usize)> = count.into_iter().filter(|(a, _)| Some(*a) != seed_artist).collect();
            others.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            lead.extend(others.iter().map(|(a, _)| *a));
            let names: Vec<&str> = lead.iter().take(3).map(|a| lib.artists[*a as usize].title.as_str()).filter(|n| !n.is_empty()).collect();
            let description = if names.is_empty() { String::new() } else { format!("with {} and more", names.join(", ")) };
            let mut covers: Vec<String> = picture.iter().cloned().collect();
            covers.extend(others.iter().filter_map(|(a, _)| lib.artists[*a as usize].thumb.clone()).take(2));
            let mix = super::MixSummary {
                id: seed_id.clone(),
                kind: MixKind::Radio,
                title: format!("{name} Radio"),
                description,
                color: engine::PALETTE[seed_id.len() % engine::PALETTE.len()].to_string(),
                art: picture,
                covers,
                artist_ids: lead.iter().take(6).map(|a| lib.artists[*a as usize].id.clone()).collect(),
                tag: None,
                track_count: tracks.len() as u32,
                duration_ms: tracks.iter().map(|t| t.duration_ms).sum(),
                next_update_at: None,
            };
            Ok(MixPage { mix, tracks, familiar, steer })
        })
        .await
    }
}

/// Checks for due mixes now and then (a new day, the next daylist block).
pub fn spawn_schedule(service: MixService) {
    const EVERY: std::time::Duration = std::time::Duration::from_secs(10 * 60);
    tauri::async_runtime::spawn(async move {
        // Let startup and the first sync settle first.
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        loop {
            if let Err(e) = service.ensure_fresh(now_ms()).await {
                tracing::warn!("mixes not made: {e}");
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}
