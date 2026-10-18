//! Bring in my listening on Platter Bridge, so the waves keep going while
//! every computer is off.
//!
//! - The desktop hands its paired Bridge a plan (`WavesPlan`): the albums
//!   from "Played a lot" still to deal with (artist, album, release group
//!   when known, plays), the waves settings, the release groups already
//!   asked for, and, on a new handover, how far the waves got. Nothing else
//!   from the listening history leaves the desktop.
//! - Exactly one side runs the waves. Once the desktop has handed over, it
//!   stops running them and the Bridge runs them through the same request
//!   path as a desktop (the setup check, Managed and Protected rules,
//!   staging, practice mode, daily pace and every pause). The Bridge
//!   answers each plan, and after each pass, with its progress
//!   (`WavesProgress`), which the desktop shows and keeps.
//! - The desktop takes the waves back when the switch goes off (the Bridge
//!   is told to stop), when the Bridge never answers a handover, or when it
//!   hasn't been heard from for a while. Progress merges both ways, and a
//!   request is never sent twice: what one side asked for is done on both.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::status::RequestState;
use super::waves::{AlbumSource, NeedsYou, Practiced, WaveAlbum, WaveState, Waves, clean_waves, key_of};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::identity::fold::{fold_album, fold_artist};
use crate::import::played;

/// The plan, on the Bridge.
const PLAN_KEY: &str = "lidarr.bringIn.plan";
/// Who runs the waves, on the desktop.
const LEASE_KEY: &str = "lidarr.bringIn.bridge";
/// Bridges that didn't answer a handover, and until when they're left be.
const REFUSED_KEY: &str = "lidarr.bringIn.bridgeRefused";
/// Release groups the desktop picked, by album key.
const GROUPS_KEY: &str = "lidarr.bringIn.groups";
/// Most albums in one plan (the rest follow as these are done).
pub const MAX_PLAN: usize = 1000;
/// A handover the Bridge hasn't answered within this long goes back (s).
pub const HANDOVER_WAIT: i64 = 15 * 60;
/// A Bridge not heard from for this long gives the waves back (s). It
/// reports after every pass and says it's alive every half hour.
pub const SILENT_S: i64 = crate::homeside::desktop::ONLINE_S;
/// After Platter starts, the Bridge has this long to say it's still
/// running before this computer takes over (s).
pub const STARTUP_GRACE: i64 = 10 * 60;
/// A Bridge that didn't answer a handover isn't asked again for this long (s).
pub const RETRY_AFTER: i64 = 6 * 3600;
const KEEP_GROUPS: usize = 5000;
const KEEP_APPLIED: usize = 200;
const KEEP: usize = 500;

/// One album of the plan: all the Bridge learns of the listening history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanAlbum {
    pub artist: String,
    pub album: String,
    #[serde(default)]
    pub release_group_id: Option<String>,
    pub plays: u64,
}

/// A change the user made on the desktop to the Bridge's "Needs you" list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEdit {
    pub id: String,
    pub key: String,
    /// Try it again (else leave it be).
    pub retry: bool,
}

/// What a desktop hands its Bridge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WavesPlan {
    /// One per handover; a resend keeps it.
    pub id: String,
    /// The Bridge runs the waves. False: stop.
    pub run: bool,
    pub user_paused: bool,
    pub waves: Vec<u32>,
    pub per_day: u32,
    pub min_free_gb: u32,
    pub max_queue: u32,
    /// Practice mode, as the Lidarr setting keeps it.
    pub practice_until: i64,
    /// Most played first.
    pub albums: Vec<PlanAlbum>,
    /// Release groups already asked for from a desktop.
    #[serde(default)]
    pub requested: Vec<String>,
    /// How far the waves got, on a new handover.
    #[serde(default)]
    pub progress: Option<WaveState>,
    #[serde(default)]
    pub edits: Vec<PlanEdit>,
}

/// What the Bridge sends back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WavesProgress {
    pub plan_id: String,
    /// The Bridge is running the waves.
    pub running: bool,
    pub state: WaveState,
}

/// On the desktop: a Bridge was handed the waves.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Lease {
    pub link: String,
    pub name: String,
    pub plan_id: String,
    /// Unix seconds.
    pub handed_at: i64,
    /// The Bridge answered this plan.
    pub acked: bool,
    pub seen_at: i64,
    pub running: bool,
    /// The Bridge's progress, as it last said.
    pub mirror: Option<WaveState>,
    /// Changes the Bridge hasn't confirmed yet.
    pub edits: Vec<PlanEdit>,
    /// The last daily summary shown here.
    pub summary_day: i64,
}

/// On the Bridge: the plan it runs from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Kept {
    id: String,
    run: bool,
    albums: Vec<PlanAlbum>,
    requested: Vec<String>,
}

fn read<T: serde::de::DeserializeOwned + Default>(db: &DbHandle, key: &str) -> T {
    db.read(|c| kv_get(c, key)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write<T: Serialize>(db: &DbHandle, key: &str, v: &T) -> AppResult<()> {
    let json = serde_json::to_string(v).map_err(|e| AppError::Internal(e.to_string()))?;
    db.write(|c| kv_set(c, key, &json))
}

/// The plan's albums, on the Bridge.
pub struct PlanAlbums {
    pub db: DbHandle,
}

impl AlbumSource for PlanAlbums {
    fn first(&self, min: u32, skip: &[String]) -> AppResult<(Option<WaveAlbum>, u64)> {
        let kept: Kept = read(&self.db, PLAN_KEY);
        if !kept.run {
            return Ok((None, 0));
        }
        let min = u64::from(min.clamp(played::MIN_PLAYS_LOW, played::MIN_PLAYS_HIGH));
        let mut left: Vec<(String, &PlanAlbum)> =
            kept.albums.iter().filter(|a| a.plays >= min).map(|a| (key_of(&a.artist, &a.album), a)).filter(|(k, _)| !skip.contains(k)).collect();
        left.sort_by_key(|a| std::cmp::Reverse(a.1.plays));
        let total = left.len() as u64;
        let Some((key, a)) = left.into_iter().next() else { return Ok((None, 0)) };
        // Songs of it already in the library, from the Bridge's own copy of it.
        let (af, alf) = (fold_artist(&a.artist), fold_album(&a.album));
        let in_library = self.db.read(|c| played::album_in_library(c, &af, &alf)).unwrap_or(0);
        Ok((
            Some(WaveAlbum { key, artist: a.artist.clone(), album: a.album.clone(), plays: a.plays, in_library, release_group_id: a.release_group_id.clone() }),
            total,
        ))
    }

    fn requested(&self) -> Vec<String> {
        read::<Kept>(&self.db, PLAN_KEY).requested
    }
}

/// Fold `from`'s progress into `into`: what either side dealt with stays
/// dealt with.
pub fn merge_progress(into: &mut WaveState, from: &WaveState) {
    for k in &from.done {
        if !into.done.contains(k) {
            into.done.push(k.clone());
        }
    }
    for n in from.needs_you.iter().filter(|n| n.get.is_none()) {
        if !into.needs_you.iter().any(|x| x.key == n.key) && !into.done.contains(&n.key) {
            into.needs_you.push(n.clone());
        }
    }
    into.needs_you.sort_by_key(|n: &NeedsYou| std::cmp::Reverse(n.at));
    into.needs_you.truncate(KEEP);
    for p in &from.practiced {
        if !into.practiced.iter().any(|x| x.key == p.key) {
            into.practiced.push(p.clone());
        }
    }
    into.practiced.sort_by_key(|p: &Practiced| std::cmp::Reverse(p.at));
    into.practiced.truncate(KEEP);
    // Later waves take in earlier ones, so the further one is safe.
    into.wave = into.wave.max(from.wave);
    into.finished = into.finished && from.finished;
    if from.day > into.day {
        into.day = from.day;
        into.today = from.today;
        into.today_needs = from.today_needs;
    } else if from.day == into.day {
        into.today = into.today.max(from.today);
        into.today_needs = into.today_needs.max(from.today_needs);
    }
    if from.summary_day > into.summary_day {
        into.summary_day = from.summary_day;
        into.last_summary = from.last_summary.clone();
    }
    into.requested_total = into.requested_total.max(from.requested_total);
}

impl Waves {
    // ------------------------------------------------------------ desktop

    pub fn lease(&self) -> Option<Lease> {
        let l: Lease = read(&self.l().inner.db, LEASE_KEY);
        (!l.plan_id.is_empty()).then_some(l)
    }

    fn save_lease(&self, l: Option<&Lease>) -> AppResult<()> {
        match l {
            Some(l) => write(&self.l().inner.db, LEASE_KEY, l),
            None => self.l().inner.db.write(|c| {
                c.execute("DELETE FROM kv WHERE key = ?1", [LEASE_KEY])?;
                Ok(())
            }),
        }
    }

    fn lease_holds(&self, l: &Lease) -> bool {
        let now = self.now();
        if !l.acked {
            return now - l.handed_at < HANDOVER_WAIT;
        }
        l.running && (now - l.seen_at < SILENT_S || now - self.started < STARTUP_GRACE)
    }

    /// The Bridge running the waves now, if one is.
    pub fn on_bridge(&self) -> Option<Lease> {
        self.lease().filter(|l| self.lease_holds(l))
    }

    /// Take the waves back from a Bridge that stopped answering (or with
    /// `force`, from any Bridge): its progress joins this computer's.
    fn take_back_with(&self, st: &mut WaveState, force: bool) -> AppResult<bool> {
        let Some(l) = self.lease() else { return Ok(false) };
        if !force && self.lease_holds(&l) {
            return Ok(false);
        }
        if let Some(m) = &l.mirror {
            merge_progress(st, m);
        }
        if !l.acked && !force {
            let mut refused: HashMap<String, i64> = read(&self.l().inner.db, REFUSED_KEY);
            refused.insert(l.link.clone(), self.now() + RETRY_AFTER);
            write(&self.l().inner.db, REFUSED_KEY, &refused)?;
        }
        self.save_lease(None)?;
        self.save(st)?;
        tracing::info!(bridge = %l.name, "Bring in my listening: back on this computer");
        Ok(true)
    }

    pub(super) fn take_back(&self, st: &mut WaveState) -> AppResult<()> {
        if self.take_back_with(st, false)? {
            self.changed();
        }
        Ok(())
    }

    /// Release groups picked here, by album key.
    pub(super) fn remember_group(&self, key: &str, rgid: &str) {
        let db = &self.l().inner.db;
        let mut m: Vec<(String, String)> = read(db, GROUPS_KEY);
        m.retain(|(k, _)| k != key);
        m.push((key.to_string(), rgid.to_ascii_lowercase()));
        if m.len() > KEEP_GROUPS {
            m.drain(..m.len() - KEEP_GROUPS);
        }
        let _ = write(db, GROUPS_KEY, &m);
    }

    /// "Needs you" changed on the desktop while a Bridge runs the waves:
    /// shown at once, and sent with the next plan.
    pub(super) fn bridge_edit(&self, key: &str, retry: bool) -> AppResult<()> {
        let Some(mut l) = self.lease() else { return Ok(()) };
        if let Some(m) = l.mirror.as_mut() {
            m.needs_you.retain(|n| n.key != key);
            if retry {
                m.done.retain(|k| k != key);
                m.finished = false;
            }
        }
        l.edits.push(PlanEdit { id: uuid::Uuid::new_v4().simple().to_string(), key: key.to_string(), retry });
        self.save_lease(Some(&l))?;
        self.changed();
        Ok(())
    }

    /// The Bridge on `link` was unpaired: the waves come back here.
    pub async fn bridge_gone(&self, link: &str) -> AppResult<()> {
        let _g = self.lock.lock().await;
        if self.lease().is_some_and(|l| l.link == link) {
            let mut st = self.state();
            self.take_back_with(&mut st, true)?;
            self.poke();
        }
        Ok(())
    }

    /// Whether a Bridge on `link` may be handed the waves now.
    pub fn may_hand_to(&self, link: &str) -> bool {
        let refused: HashMap<String, i64> = read(&self.l().inner.db, REFUSED_KEY);
        refused.get(link).is_none_or(|until| *until <= self.now())
    }

    /// The plan for the Bridge on `link` (named `name`), handing the waves
    /// over if it doesn't have them. None: nothing to send it (another
    /// Bridge runs them, or this one didn't answer last time).
    pub async fn bridge_plan(&self, link: &str, name: &str) -> AppResult<Option<WavesPlan>> {
        let _g = self.lock.lock().await;
        let s = crate::settings::load(&self.l().inner.db);
        let mut st = self.state();
        let lease = self.lease();
        if let Some(l) = &lease
            && l.link != link
            && self.lease_holds(l)
        {
            return Ok(None);
        }
        let waves = clean_waves(&s.bring_in_waves).unwrap_or_else(|_| super::waves::DEFAULT_WAVES.to_vec());
        let user_paused = st.user_paused;
        let stop = |id: String| WavesPlan {
            id,
            run: false,
            user_paused,
            waves: waves.clone(),
            per_day: s.bring_in_per_day,
            min_free_gb: s.bring_in_min_free_gb,
            max_queue: s.bring_in_max_queue,
            practice_until: s.lidarr_practice_until,
            albums: Vec::new(),
            requested: Vec::new(),
            progress: None,
            edits: Vec::new(),
        };
        if !s.bring_in {
            // Off: this computer keeps what the Bridge did, and the Bridge stops.
            let id = lease.as_ref().map(|l| l.plan_id.clone()).unwrap_or_default();
            if lease.is_some() {
                self.take_back_with(&mut st, true)?;
            }
            return Ok(Some(stop(id)));
        }
        let mut l = match lease {
            Some(l) if l.link == link => l,
            Some(_) => {
                // The other Bridge stopped answering: its progress comes back first.
                self.take_back_with(&mut st, false)?;
                Lease::default()
            }
            None => Lease::default(),
        };
        if l.plan_id.is_empty() {
            if !self.may_hand_to(link) {
                return Ok(None);
            }
            l = Lease { link: link.to_string(), name: name.to_string(), plan_id: uuid::Uuid::new_v4().to_string(), handed_at: self.now(), summary_day: st.summary_day, ..Default::default() };
            tracing::info!(bridge = %name, "Bring in my listening: handing the waves to the Bridge");
        }
        l.name = name.to_string();
        let mut seen = st.clone();
        if let Some(m) = &l.mirror {
            merge_progress(&mut seen, m);
        }
        let albums = self.plan_albums(&waves, &seen)?;
        let requested: Vec<String> = self.l().records().into_iter().filter(|r| r.state != RequestState::Failed).map(|r| r.release_group_id).collect();
        let progress = (!l.acked).then(|| {
            let mut p = st.clone();
            p.queued.clear();
            p.needs_you.retain(|n| n.get.is_none());
            p.paused = None;
            p.fails = 0;
            p.applied.clear();
            p
        });
        let plan = WavesPlan {
            id: l.plan_id.clone(),
            run: true,
            user_paused: st.user_paused,
            waves,
            per_day: s.bring_in_per_day,
            min_free_gb: s.bring_in_min_free_gb,
            max_queue: s.bring_in_max_queue,
            practice_until: s.lidarr_practice_until,
            albums,
            requested,
            progress,
            edits: l.edits.clone(),
        };
        self.save_lease(Some(&l))?;
        Ok(Some(plan))
    }

    /// "Played a lot" albums still to deal with, most played first.
    fn plan_albums(&self, waves: &[u32], st: &WaveState) -> AppResult<Vec<PlanAlbum>> {
        let Some(&min) = waves.iter().min() else { return Ok(Vec::new()) };
        let skip = self.skip_keys(st);
        let groups: HashMap<String, String> = read::<Vec<(String, String)>>(&self.l().inner.db, GROUPS_KEY).into_iter().collect();
        let now_ms = self.now() * 1000;
        let mut out = Vec::new();
        let mut offset = 0;
        while out.len() < MAX_PLAN {
            let q = played::Query { kind: played::Kind::Album, offset, min_plays: min, period: "all".into() };
            let page = self.l().inner.db.read(|c| played::list(c, &q, &skip, now_ms))?;
            if page.rows.is_empty() {
                break;
            }
            offset += page.rows.len();
            for r in page.rows {
                out.push(PlanAlbum { release_group_id: groups.get(&r.key).cloned(), artist: r.artist, album: r.title, plays: r.plays });
            }
        }
        out.truncate(MAX_PLAN);
        Ok(out)
    }

    /// The Bridge's progress came in. True when it should get a plan now
    /// (it runs from one this computer doesn't know).
    pub async fn bridge_progress(&self, link: &str, p: WavesProgress) -> AppResult<bool> {
        let _g = self.lock.lock().await;
        let Some(mut l) = self.lease().filter(|l| l.link == link && l.plan_id == p.plan_id) else {
            return Ok(p.running);
        };
        let now = self.now();
        if !l.acked {
            tracing::info!(bridge = %l.name, running = p.running, "Bring in my listening: the Bridge took the waves");
        }
        l.acked = true;
        l.seen_at = now;
        l.running = p.running;
        l.edits.retain(|e| !p.state.applied.contains(&e.id));
        if p.state.summary_day > l.summary_day
            && let Some(text) = &p.state.last_summary
        {
            self.inner.notifier.notify("Bring in my listening", text);
            l.summary_day = p.state.summary_day;
        }
        l.mirror = Some(p.state);
        self.save_lease(Some(&l))?;
        if !l.running {
            // The Bridge stopped on its own: the waves come back here.
            let mut st = self.state();
            self.take_back_with(&mut st, true)?;
            self.poke();
        }
        Ok(false)
    }

    // ------------------------------------------------------------ Bridge

    /// On the Bridge: take a plan from the desktop.
    pub async fn take_plan(&self, plan: &WavesPlan) -> AppResult<()> {
        let _g = self.lock.lock().await;
        let db = &self.l().inner.db;
        let mut s = crate::settings::try_load(db)?;
        s.bring_in = plan.run;
        if let Ok(w) = clean_waves(&plan.waves) {
            s.bring_in_waves = w;
        }
        s.bring_in_per_day = plan.per_day.clamp(1, super::waves::MAX_PER_DAY);
        s.bring_in_min_free_gb = plan.min_free_gb.min(100_000);
        s.bring_in_max_queue = plan.max_queue.clamp(1, 1_000);
        s.lidarr_practice_until = plan.practice_until;
        crate::settings::save(db, &s)?;
        let mut kept: Kept = read(db, PLAN_KEY);
        let mut st = self.state();
        if !plan.run {
            if kept.run {
                tracing::info!("Bring in my listening: stopped (turned off, or back on a desktop)");
            }
            kept.run = false;
            kept.albums.clear();
            write(db, PLAN_KEY, &kept)?;
            return Ok(());
        }
        if kept.id != plan.id {
            if let Some(p) = &plan.progress {
                merge_progress(&mut st, p);
            }
            st.paused = None;
            st.fails = 0;
            tracing::info!(albums = plan.albums.len(), "Bring in my listening: running here now, from the desktop's list");
        }
        for e in &plan.edits {
            if st.applied.contains(&e.id) {
                continue;
            }
            st.needs_you.retain(|n| n.key != e.key);
            if e.retry {
                st.done.retain(|k| k != &e.key);
                st.finished = false;
            }
            st.applied.push(e.id.clone());
        }
        if st.applied.len() > KEEP_APPLIED {
            st.applied.drain(..st.applied.len() - KEEP_APPLIED);
        }
        st.user_paused = plan.user_paused;
        self.save(&st)?;
        let requested = plan.requested.iter().map(|r| r.trim().to_ascii_lowercase()).filter(|r| crate::artistinfo::is_mbid(r)).collect();
        let albums = plan.albums.iter().take(MAX_PLAN).cloned().collect();
        write(db, PLAN_KEY, &Kept { id: plan.id.clone(), run: true, albums, requested })?;
        Ok(())
    }

    /// On the Bridge: whether it has the waves to run.
    pub fn plan_running(&self) -> bool {
        read::<Kept>(&self.l().inner.db, PLAN_KEY).run
    }

    /// On the Bridge: progress for the desktops.
    pub fn progress(&self) -> WavesProgress {
        let kept: Kept = read(&self.l().inner.db, PLAN_KEY);
        WavesProgress { plan_id: kept.id, running: kept.run, state: self.state() }
    }
}
