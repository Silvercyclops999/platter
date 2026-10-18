//! "Bring in my listening": albums from the Missing page's "Played a lot"
//! list, asked of Lidarr in waves, a few a day, so the library slowly
//! catches up with what the user listens to.
//!
//! - Waves go by fewest plays (100, then 50, then 20 by default), most
//!   played first inside a wave, at most `bring_in_per_day` albums a local
//!   day. Progress is kept on this device and picked up after a restart.
//! - Every album goes the usual way: MusicBrainz picks one official release
//!   group, then the full request path (setup check, Managed and Protected
//!   rules, staging, no-overlap recheck). In practice mode a wave only lists
//!   what it would ask for. Albums that can't be picked, or that a check
//!   refuses, go to "Needs you" with the reason and the wave moves on.
//! - Guards pause it: Lidarr's queue past `bring_in_max_queue`, less than
//!   `bring_in_min_free_gb` free where Lidarr downloads, Lidarr out of
//!   reach, a red setup check, or three failed requests in a row. Each pass
//!   looks again and carries on once they clear.
//! - Never asks again for an album it, or anyone, asked for before, that
//!   Lidarr already has, or that the library gained (those leave the list).
//! - Pauses, resumes, requests and practice runs go to the Changes log; a
//!   notification sums up each day.
//! - "Get albums" on an artist's page queues that artist's albums here; a
//!   pass asks for queued albums first, with the same guards and daily
//!   pace, whether or not Bring in my listening is on.
//! - With a Platter Bridge paired, the Bridge runs the waves instead, from
//!   an album list this computer hands it (`waves_bridge`).

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::Lidarr;
use super::request::{RequestInput, RequestRecord};
use super::status::RequestState;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::identity::fold::{fold_album, fold_artist};
use crate::import::played::{self, Kind, Query};
use crate::social::Notifier;

pub const DEFAULT_WAVES: [u32; 3] = [100, 50, 20];
pub const DEFAULT_PER_DAY: u32 = 30;
pub const MAX_PER_DAY: u32 = 200;
pub const DEFAULT_MIN_FREE_GB: u32 = 50;
pub const DEFAULT_MAX_QUEUE: u32 = 20;
pub const MAX_WAVES: usize = 5;
/// Failed requests in a row that pause it.
pub const FAILS_TO_PAUSE: u32 = 3;
/// Albums looked at in one pass at most.
pub const PER_PASS: usize = 10;
/// How often it looks for work while Platter is open.
pub const LOOK_EVERY: Duration = Duration::from_secs(10 * 60);
const STATE_KEY: &str = "lidarr.bringIn";
/// Most "Needs you" and practice rows kept.
const KEEP: usize = 500;
const GB: i64 = 1_000_000_000;

/// The waves as the setting keeps them: highest first, each once, 1 to
/// 100,000 plays, at most [`MAX_WAVES`].
pub fn clean_waves(w: &[u32]) -> AppResult<Vec<u32>> {
    let mut v: Vec<u32> = w.iter().copied().filter(|n| (1..=100_000).contains(n)).collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v.dedup();
    if v.is_empty() || v.len() != w.len() || v.len() > MAX_WAVES {
        return Err(AppError::Invalid(format!("Waves are 1 to {MAX_WAVES} different play counts, like 100, 50, 20.")));
    }
    Ok(v)
}

/// An album that needs the user: it couldn't be picked, or a check refused it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsYou {
    pub key: String,
    pub artist: String,
    pub album: String,
    pub plays: u64,
    pub reason: String,
    /// Unix seconds.
    pub at: i64,
    /// For a "Get albums" album: what to ask for again on a retry.
    #[serde(default)]
    pub get: Option<QueuedGet>,
}

/// An album "Get albums" queued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedGet {
    pub release_group_id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub artist_mbid: Option<String>,
    #[serde(default)]
    pub artist_id: Option<String>,
}

impl QueuedGet {
    fn key(&self) -> String {
        format!("get:{}", self.release_group_id.to_ascii_lowercase())
    }

    fn input(&self) -> RequestInput {
        RequestInput {
            release_group_id: self.release_group_id.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            artist_id: self.artist_id.clone(),
            artist_mbid: self.artist_mbid.clone(),
            typed: None,
        }
    }
}

/// What practice mode would have asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Practiced {
    pub key: String,
    pub artist: String,
    pub album: String,
    pub release_group_id: String,
    pub at: i64,
}

/// Progress, kept on this device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WaveState {
    /// Which wave (index into the setting).
    pub wave: usize,
    /// The local day the counts below are for.
    pub day: i64,
    /// Albums asked for (or listed, in practice) on that day.
    pub today: u32,
    pub today_needs: u32,
    /// Albums dealt with, by their "Played a lot" key.
    pub done: Vec<String>,
    pub needs_you: Vec<NeedsYou>,
    pub practiced: Vec<Practiced>,
    /// Failed requests in a row.
    pub fails: u32,
    /// Why a guard paused it.
    pub paused: Option<String>,
    /// The user paused it.
    pub user_paused: bool,
    /// Every wave is done.
    pub finished: bool,
    /// The last day a summary went out for.
    pub summary_day: i64,
    pub requested_total: u32,
    /// Albums "Get albums" queued, in the order they go out.
    pub queued: Vec<QueuedGet>,
    /// What the last daily summary said (for the day in `summary_day`).
    pub last_summary: Option<String>,
    /// On a Bridge: changes from the desktop already made, by id.
    pub applied: Vec<String>,
}

/// What the progress card shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WavesStatus {
    pub on: bool,
    /// Practice mode: albums are only listed.
    pub practice: bool,
    /// Lidarr is set up and its last check passed.
    pub ready: bool,
    pub waves: Vec<u32>,
    pub wave: usize,
    /// The current wave's fewest plays; None once every wave is done.
    pub threshold: Option<u32>,
    /// Albums dealt with so far.
    pub done: u64,
    /// Albums left in the current wave.
    pub remaining: u64,
    pub today: u32,
    pub per_day: u32,
    pub paused: Option<String>,
    pub user_paused: bool,
    pub finished: bool,
    pub requested_total: u32,
    pub needs_you: Vec<NeedsYou>,
    pub practiced: Vec<Practiced>,
    /// Albums "Get albums" queued and not asked for yet.
    pub queued: Vec<QueuedGet>,
    /// The paired Bridge's name while it runs the waves.
    pub bridge: Option<String>,
}

/// One album the waves can ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveAlbum {
    /// Its key on "Played a lot".
    pub key: String,
    pub artist: String,
    pub album: String,
    pub plays: u64,
    /// Its songs already in the library.
    pub in_library: u64,
    /// The release group, when it's known already.
    pub release_group_id: Option<String>,
}

/// Where the waves' albums come from: "Played a lot" on a desktop, the
/// album list a desktop handed over on a Bridge.
pub trait AlbumSource: Send + Sync {
    /// The most played album with at least `min` plays whose key isn't in
    /// `skip`, and how many such albums there are.
    fn first(&self, min: u32, skip: &[String]) -> AppResult<(Option<WaveAlbum>, u64)>;

    /// Release groups asked for elsewhere (never asked for again).
    fn requested(&self) -> Vec<String> {
        Vec::new()
    }
}

/// "Played a lot" on this computer.
pub struct PlayedAlbums {
    pub db: crate::db::DbHandle,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl AlbumSource for PlayedAlbums {
    fn first(&self, min: u32, skip: &[String]) -> AppResult<(Option<WaveAlbum>, u64)> {
        let now_ms = (self.clock)() * 1000;
        let page = self.db.read(|c| played::list(c, &album_query(min), skip, now_ms))?;
        let total = page.total;
        let row = page.rows.into_iter().next().map(|r| WaveAlbum {
            key: r.key,
            artist: r.artist,
            album: r.title,
            plays: r.plays,
            in_library: r.in_library.unwrap_or(0),
            release_group_id: None,
        });
        Ok((row, total))
    }
}

/// What one pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassOutcome {
    pub requested: Vec<String>,
    pub practiced: Vec<String>,
    pub needs_you: Vec<String>,
    pub paused: Option<String>,
}

/// Lidarr's queue and free space, read before each pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gauges {
    pub queue: u64,
    /// Bytes free where Lidarr downloads; None when it doesn't say.
    pub free: Option<i64>,
}

/// Why the guards stop a pass, or None.
pub fn guard(g: &Gauges, max_queue: u32, min_free_gb: u32) -> Option<String> {
    if g.queue > max_queue as u64 {
        return Some(format!("Lidarr's queue has {} items. It carries on once it's down to {max_queue}.", g.queue));
    }
    if let Some(free) = g.free
        && min_free_gb > 0
        && free < min_free_gb as i64 * GB
    {
        return Some(format!(
            "Only {} GB is free where Lidarr downloads. It carries on once there's {min_free_gb} GB.",
            free.max(0) / GB
        ));
    }
    None
}

/// The key an album has on "Played a lot".
fn album_key(artist: &str, album: &str) -> String {
    played::hide_key("album", &fold_artist(artist), &fold_album(album))
}

pub struct WavesDeps {
    pub lidarr: Lidarr,
    pub notifier: Arc<dyn Notifier>,
    /// The local day number of a unix time (seconds).
    pub day_of: Arc<dyn Fn(i64) -> i64 + Send + Sync>,
    /// Hide keys of rows the user hid from "Played a lot".
    pub hidden: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
}

/// Called when something the Bridge should hear about changed.
pub type OnChange = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct Waves {
    pub(super) inner: Arc<WavesDeps>,
    pub(super) albums: Arc<dyn AlbumSource>,
    pub(super) lock: Arc<tokio::sync::Mutex<()>>,
    wake: Arc<tokio::sync::Notify>,
    on_change: Arc<parking_lot::RwLock<Option<OnChange>>>,
    /// When this run started (unix seconds).
    pub(super) started: i64,
    /// The waves settings last seen, so only their changes count.
    seen_settings: Arc<parking_lot::Mutex<String>>,
}

/// How a request came out, for the pacing.
enum Sent {
    Done(Box<RequestRecord>),
    /// Lidarr couldn't be reached, or its check is red: pause.
    Pause(String),
    /// MusicBrainz or a check refused it.
    Refused(String),
    /// Something else went wrong.
    Failed(String),
}

impl Waves {
    /// Albums from "Played a lot" in this computer's database.
    pub fn new(deps: WavesDeps) -> Self {
        let albums = Arc::new(PlayedAlbums { db: deps.lidarr.inner.db.clone(), clock: deps.lidarr.inner.clock.clone() });
        Self::with_albums(deps, albums)
    }

    pub fn with_albums(deps: WavesDeps, albums: Arc<dyn AlbumSource>) -> Self {
        let started = (deps.lidarr.inner.clock)();
        let deps_db = deps.lidarr.inner.db.clone();
        Self {
            inner: Arc::new(deps),
            albums,
            lock: Arc::new(tokio::sync::Mutex::new(())),
            wake: Arc::new(tokio::sync::Notify::new()),
            on_change: Arc::default(),
            started,
            seen_settings: Arc::new(parking_lot::Mutex::new(settings_sig(&deps_db))),
        }
    }

    pub(super) fn l(&self) -> &Lidarr {
        &self.inner.lidarr
    }

    pub fn state(&self) -> WaveState {
        self.l().inner.db.read(|c| kv_get(c, STATE_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub(super) fn save(&self, st: &WaveState) -> AppResult<()> {
        let json = serde_json::to_string(st).map_err(|e| AppError::Internal(e.to_string()))?;
        self.l().inner.db.write(|c| kv_set(c, STATE_KEY, &json))
    }

    pub(super) fn now(&self) -> i64 {
        (self.l().inner.clock)()
    }

    /// Run a pass soon (a setting changed, the user resumed it).
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    /// Waits for [`Self::poke`].
    pub async fn woken(&self) {
        self.wake.notified().await;
    }

    /// Tell `f` whenever something a paired Bridge should hear changed.
    pub fn set_on_change(&self, f: OnChange) {
        *self.on_change.write() = Some(f);
    }

    pub(super) fn changed(&self) {
        if let Some(f) = self.on_change.read().clone() {
            f();
        }
    }

    /// Settings changed: when the waves' own did, run a pass soon and tell
    /// the Bridge.
    pub fn settings_changed(&self) {
        let now = settings_sig(&self.l().inner.db);
        let changed = {
            let mut seen = self.seen_settings.lock();
            let changed = *seen != now;
            *seen = now;
            changed
        };
        if changed {
            self.poke();
            self.changed();
        }
    }

    /// The progress card.
    pub fn status(&self) -> AppResult<WavesStatus> {
        let s = crate::settings::load(&self.l().inner.db);
        let own = self.state();
        // While a Bridge runs the waves, its progress is what counts.
        let lease = self.on_bridge();
        let st = match lease.as_ref().and_then(|l| l.mirror.clone()) {
            Some(mut m) => {
                m.user_paused = own.user_paused;
                m.queued = own.queued.clone();
                m.needs_you.extend(own.needs_you.iter().filter(|n| n.get.is_some()).cloned());
                m
            }
            None => own,
        };
        let waves = clean_waves(&s.bring_in_waves).unwrap_or_else(|_| DEFAULT_WAVES.to_vec());
        let threshold = waves.get(st.wave).copied().filter(|_| !st.finished);
        let remaining = match threshold {
            Some(t) => self.albums.first(t, &self.skip_keys(&st))?.1,
            None => 0,
        };
        let today = if st.day == (self.inner.day_of)(self.now()) { st.today } else { 0 };
        Ok(WavesStatus {
            on: s.bring_in,
            practice: self.l().practice_active(),
            ready: self.l().status().ready,
            waves,
            wave: st.wave,
            threshold,
            done: st.done.len() as u64,
            remaining,
            today,
            per_day: s.bring_in_per_day,
            paused: st.paused.clone(),
            user_paused: st.user_paused,
            finished: st.finished,
            requested_total: st.requested_total,
            needs_you: st.needs_you.clone(),
            practiced: st.practiced.clone(),
            queued: st.queued.clone(),
            bridge: lease.map(|l| l.name),
        })
    }

    /// "Get albums": queue albums to ask Lidarr for, paced like the waves.
    /// Albums already queued, or asked for before (and not failed), are
    /// left out. Returns how many were added.
    pub fn enqueue(&self, albums: Vec<QueuedGet>, artist: &str) -> AppResult<usize> {
        let records = self.l().records();
        let mut st = self.state();
        let mut added = 0;
        for a in albums {
            let rg = a.release_group_id.trim().to_ascii_lowercase();
            if !crate::artistinfo::is_mbid(&rg)
                || st.queued.iter().any(|q| q.release_group_id.eq_ignore_ascii_case(&rg))
                || records.iter().any(|r| r.release_group_id == rg && r.state != RequestState::Failed)
            {
                continue;
            }
            st.needs_you.retain(|n| n.key != a.key());
            st.queued.push(QueuedGet { release_group_id: rg, ..a });
            added += 1;
        }
        if added > 0 {
            self.save(&st)?;
            let what = format!("Queued {added} album{} by {artist} (Get albums)", if added == 1 { "" } else { "s" });
            self.l().log_change(&what, "", "", true, None)?;
            tracing::info!(added, "get albums: queued");
            self.poke();
        }
        Ok(added)
    }

    /// Take albums off the "Get albums" queue (all of them when `rgids` is empty).
    pub fn unqueue(&self, rgids: &[String]) -> AppResult<WavesStatus> {
        let mut st = self.state();
        if rgids.is_empty() {
            st.queued.clear();
        } else {
            st.queued.retain(|q| !rgids.iter().any(|r| r.eq_ignore_ascii_case(&q.release_group_id)));
        }
        self.save(&st)?;
        self.status()
    }

    /// Pause it, or carry on.
    pub fn set_paused(&self, paused: bool) -> AppResult<WavesStatus> {
        let mut st = self.state();
        if st.user_paused != paused {
            st.user_paused = paused;
            self.save(&st)?;
            let what = if paused { "Paused Bring in my listening" } else { "Resumed Bring in my listening" };
            self.l().log_change(what, "", "You asked", true, None)?;
        }
        if !paused {
            self.poke();
        }
        self.changed();
        self.status()
    }

    /// A "Needs you" album: try it again on the next pass, or leave it be.
    pub fn needs_you(&self, key: &str, retry: bool) -> AppResult<WavesStatus> {
        let mut st = self.state();
        let get = st.needs_you.iter().find(|n| n.key == key).and_then(|n| n.get.clone());
        if get.is_none() {
            // The Bridge's list, while it runs the waves: it hears next.
            self.bridge_edit(key, retry)?;
        }
        st.needs_you.retain(|n| n.key != key);
        if let (true, Some(g)) = (retry, get) {
            st.queued.insert(0, g);
        }
        if retry {
            st.done.retain(|k| k != key);
            st.finished = false;
        }
        self.save(&st)?;
        if retry {
            self.poke();
        }
        self.status()
    }

    /// Start over from the first wave (the waves setting changed).
    pub fn restart_waves(&self) -> AppResult<()> {
        let mut st = self.state();
        st.wave = 0;
        st.finished = false;
        self.save(&st)
    }

    pub(super) fn skip_keys(&self, st: &WaveState) -> Vec<String> {
        let mut keys = (self.inner.hidden)();
        keys.extend(st.done.iter().cloned());
        keys.extend(st.needs_you.iter().map(|n| n.key.clone()));
        keys
    }

    /// The next album: the most played of the current wave not dealt with,
    /// moving to the next wave when one runs out.
    fn next(&self, waves: &[u32], st: &mut WaveState) -> AppResult<Option<WaveAlbum>> {
        while let Some(&t) = waves.get(st.wave) {
            let skip = self.skip_keys(st);
            if let (Some(r), _) = self.albums.first(t, &skip)? {
                return Ok(Some(r));
            }
            self.l().log_change(&format!("Finished the {t}+ plays wave of Bring in my listening"), "", "", false, None)?;
            st.wave += 1;
        }
        st.finished = true;
        Ok(None)
    }

    fn pause(&self, st: &mut WaveState, why: String) -> AppResult<()> {
        if st.paused.as_deref() != Some(why.as_str()) {
            tracing::info!("Bring in my listening paused: {why}");
            self.l().log_change("Paused Bring in my listening", "", &why, false, None)?;
            st.paused = Some(why);
        }
        Ok(())
    }

    fn resume(&self, st: &mut WaveState) -> AppResult<()> {
        if let Some(was) = st.paused.take() {
            tracing::info!("Bring in my listening carried on");
            self.l().log_change("Bring in my listening carried on", &was, "Clear again", false, None)?;
        }
        Ok(())
    }

    fn summary(&self, st: &mut WaveState, day: i64, practice: bool) {
        if st.summary_day == day || (st.today == 0 && st.today_needs == 0) {
            return;
        }
        st.summary_day = day;
        let albums = if st.today == 1 { "1 album".to_string() } else { format!("{} albums", st.today) };
        let did = if practice { format!("{albums} listed (practice mode)") } else { format!("{albums} requested") };
        let body = if st.today_needs > 0 { format!("{did}, {} need you.", st.today_needs) } else { format!("{did}.") };
        st.last_summary = Some(body.clone());
        self.inner.notifier.notify("Bring in my listening", &body);
    }

    /// Lidarr's queue and the free space where it downloads (reads only).
    async fn gauges(&self) -> AppResult<Gauges> {
        let (client, _) = self.l().connection().await?;
        let (queue, roots) = futures::try_join!(client.queue_total(), client.root_folders())?;
        let staging = self.l().staging_norm();
        let free = roots
            .iter()
            .filter(|r| r.accessible && staging.as_ref().is_none_or(|s| super::paths::NormPath::parse(&r.path) != *s))
            .filter_map(|r| r.free_space)
            .min();
        Ok(Gauges { queue, free })
    }

    /// Ask for one album the way a request by hand does, without the typed
    /// name (turning this on stands in for it).
    async fn send(&self, input: RequestInput) -> Sent {
        let l = self.l();
        let _one = l.lock.lock().await;
        let rgid = input.release_group_id.trim().to_ascii_lowercase();
        if let Some(r) = l.records().into_iter().find(|r| r.release_group_id == rgid && r.state != RequestState::Failed) {
            return Sent::Done(Box::new(r));
        }
        let rec = l.new_record(&input, rgid, true);
        match l.send(rec).await {
            Ok(r) => match l.upsert(&r) {
                Ok(()) => Sent::Done(Box::new(r)),
                Err(e) => Sent::Failed(e.message()),
            },
            Err(e) => classify(&e),
        }
    }

    /// One pass: guards, then albums until today's count is reached, the
    /// pass's share is done, or something pauses it.
    pub async fn pass(&self) -> AppResult<PassOutcome> {
        let _g = self.lock.lock().await;
        let s = crate::settings::load(&self.l().inner.db);
        let mut st = self.state();
        let mut out = PassOutcome::default();
        let now = self.now();
        let today = (self.inner.day_of)(now);
        let practice = self.l().practice_active();
        if st.day != today {
            let day = st.day;
            self.summary(&mut st, day, practice);
            st.day = today;
            st.today = 0;
            st.today_needs = 0;
        }
        let waves = clean_waves(&s.bring_in_waves).unwrap_or_else(|_| DEFAULT_WAVES.to_vec());
        // A Bridge that stopped answering gives the waves back to this computer.
        self.take_back(&mut st)?;
        // Bring in my listening's own albums (unless a Bridge runs them);
        // "Get albums" goes on regardless.
        let bring = s.bring_in && !st.user_paused && !(st.finished && st.wave >= waves.len()) && self.on_bridge().is_none();
        if !bring && st.queued.is_empty() {
            self.save(&st)?;
            return Ok(out);
        }
        if bring {
            st.finished = false;
        }
        if !self.l().status().ready {
            self.pause(&mut st, "Lidarr isn't set up, or its setup check hasn't passed.".into())?;
            out.paused = st.paused.clone();
            self.save(&st)?;
            return Ok(out);
        }
        if st.today >= s.bring_in_per_day {
            self.summary(&mut st, today, practice);
            self.save(&st)?;
            return Ok(out);
        }
        match self.gauges().await {
            Err(e) if e.kind() == "network" => self.pause(&mut st, UNREACHABLE.into())?,
            Err(e) => self.pause(&mut st, format!("Platter couldn't read Lidarr's queue: {}", e.message()))?,
            Ok(g) => match guard(&g, s.bring_in_max_queue, s.bring_in_min_free_gb) {
                Some(why) => self.pause(&mut st, why)?,
                None => self.resume(&mut st)?,
            },
        }
        if st.paused.is_some() {
            out.paused = st.paused.clone();
            self.save(&st)?;
            return Ok(out);
        }
        for _ in 0..PER_PASS {
            if st.today >= s.bring_in_per_day {
                self.summary(&mut st, today, practice);
                break;
            }
            if let Some(q) = st.queued.first().cloned() {
                if !self.queued_album(&q, practice, &mut st, &mut out).await? {
                    break;
                }
                self.save(&st)?;
                continue;
            }
            if !bring {
                break;
            }
            let Some(row) = self.next(&waves, &mut st)? else { break };
            if !self.album(&row, practice, &mut st, &mut out).await? {
                break;
            }
            self.save(&st)?;
        }
        st.needs_you.truncate(KEEP);
        st.practiced.truncate(KEEP);
        out.paused = st.paused.clone();
        self.save(&st)?;
        Ok(out)
    }

    /// One album. False stops the pass (paused, or a failure to try later).
    async fn album(&self, row: &WaveAlbum, practice: bool, st: &mut WaveState, out: &mut PassOutcome) -> AppResult<bool> {
        let needs = |st: &mut WaveState, out: &mut PassOutcome, reason: String| {
            tracing::info!(album = %row.album, "Bring in my listening: needs you ({reason})");
            st.needs_you.insert(0, NeedsYou { key: row.key.clone(), artist: row.artist.clone(), album: row.album.clone(), plays: row.plays, reason, at: self.now(), get: None });
            st.today_needs += 1;
            out.needs_you.push(row.album.clone());
        };
        if row.in_library > 0 {
            needs(
                st,
                out,
                "Some of its songs are in your library already. Platter won't ask Lidarr for an album that shares a title with one you have; a store has the rest.".into(),
            );
            return Ok(true);
        }
        // Asked for before (by hand or by a wave).
        let fa = fold_artist(&row.artist);
        let fal = fold_album(&row.album);
        if self.l().records().iter().any(|r| r.state != RequestState::Failed && fold_artist(&r.artist) == fa && fold_album(&r.title) == fal) {
            st.done.push(row.key.clone());
            return Ok(true);
        }
        // The release group: known already (picked on a desktop), else
        // MusicBrainz picks one now. Either way it's checked again on sending.
        let (group_id, group_title) = match row.release_group_id.clone().filter(|g| crate::artistinfo::is_mbid(g)) {
            Some(g) => (g, row.album.clone()),
            None => {
                let picked = match self.l().pick(&row.artist, None, &row.album, false, None).await {
                    Ok(p) => p,
                    Err(e) if e.kind() == "network" => {
                        st.fails += 1;
                        if st.fails >= FAILS_TO_PAUSE {
                            self.pause(st, format!("MusicBrainz didn't answer {FAILS_TO_PAUSE} times in a row. It tries again later."))?;
                        }
                        return Ok(false);
                    }
                    Err(e) => {
                        needs(st, out, e.message());
                        return Ok(true);
                    }
                };
                let Some(group) = picked.group else {
                    needs(st, out, picked.reason.unwrap_or_else(|| "MusicBrainz has no official release of this album for Lidarr to get.".into()));
                    return Ok(true);
                };
                self.remember_group(&row.key, &group.id);
                (group.id, group.title)
            }
        };
        let input = RequestInput {
            release_group_id: group_id.clone(),
            title: group_title.clone(),
            artist: row.artist.clone(),
            artist_id: None,
            artist_mbid: None,
            typed: None,
        };
        let elsewhere = self.albums.requested();
        if self.l().records().iter().any(|r| r.release_group_id.eq_ignore_ascii_case(&group_id) && r.state != RequestState::Failed)
            || elsewhere.iter().any(|g| g.eq_ignore_ascii_case(&group_id))
        {
            st.done.push(row.key.clone());
            return Ok(true);
        }
        if practice {
            match self.l().preview(&input).await {
                Ok(p) if p.existing.is_some() => st.done.push(row.key.clone()),
                Ok(p) => {
                    let to = p.destination.clone().unwrap_or_default();
                    self.l().log_practice(&format!("Would request {} by {} (Bring in my listening)", input.title, row.artist), "", &to)?;
                    tracing::info!(album = %input.title, "Bring in my listening: would request (practice mode)");
                    st.practiced.insert(0, Practiced { key: row.key.clone(), artist: row.artist.clone(), album: input.title.clone(), release_group_id: input.release_group_id.clone(), at: self.now() });
                    st.done.push(row.key.clone());
                    st.today += 1;
                    st.fails = 0;
                    out.practiced.push(input.title.clone());
                }
                Err(e) => return self.after_error(classify(&e), st, out, needs),
            }
            return Ok(true);
        }
        match self.send(input).await {
            Sent::Done(rec) => {
                st.done.push(row.key.clone());
                st.fails = 0;
                if !rec.already_in_lidarr && rec.auto {
                    st.today += 1;
                    st.requested_total += 1;
                    out.requested.push(rec.title.clone());
                    let to = rec.destination.clone().unwrap_or_default();
                    self.l().log_change(&format!("Requested {} by {} (Bring in my listening)", rec.title, rec.artist), "", &to, false, None)?;
                    tracing::info!(album = %rec.title, "Bring in my listening: requested");
                }
                Ok(true)
            }
            other => self.after_error(other, st, out, needs),
        }
    }

    /// One "Get albums" album. False stops the pass.
    async fn queued_album(&self, q: &QueuedGet, practice: bool, st: &mut WaveState, out: &mut PassOutcome) -> AppResult<bool> {
        let take = |st: &mut WaveState| st.queued.retain(|x| x.release_group_id != q.release_group_id);
        let needs = |st: &mut WaveState, out: &mut PassOutcome, reason: String| {
            st.queued.retain(|x| x.release_group_id != q.release_group_id);
            st.needs_you.insert(0, NeedsYou { key: q.key(), artist: q.artist.clone(), album: q.title.clone(), plays: 0, reason, at: self.now(), get: Some(q.clone()) });
            st.today_needs += 1;
            out.needs_you.push(q.title.clone());
        };
        if practice {
            match self.l().preview(&q.input()).await {
                Ok(p) => {
                    take(st);
                    if p.existing.is_none() {
                        let to = p.destination.clone().unwrap_or_default();
                        self.l().log_practice(&format!("Would request {} by {} (Get albums)", q.title, q.artist), "", &to)?;
                        st.practiced.insert(0, Practiced { key: q.key(), artist: q.artist.clone(), album: q.title.clone(), release_group_id: q.release_group_id.clone(), at: self.now() });
                        st.today += 1;
                        out.practiced.push(q.title.clone());
                    }
                    st.fails = 0;
                    Ok(true)
                }
                Err(e) => self.after_error(classify(&e), st, out, needs),
            }
        } else {
            match self.send(q.input()).await {
                Sent::Done(rec) => {
                    take(st);
                    st.fails = 0;
                    if !rec.already_in_lidarr && rec.auto {
                        st.today += 1;
                        out.requested.push(rec.title.clone());
                        let to = rec.destination.clone().unwrap_or_default();
                        self.l().log_change(&format!("Requested {} by {} (Get albums)", rec.title, rec.artist), "", &to, false, None)?;
                    }
                    Ok(true)
                }
                other => self.after_error(other, st, out, needs),
            }
        }
    }

    fn after_error(
        &self,
        sent: Sent,
        st: &mut WaveState,
        out: &mut PassOutcome,
        needs: impl Fn(&mut WaveState, &mut PassOutcome, String),
    ) -> AppResult<bool> {
        match sent {
            Sent::Done(_) => Ok(true),
            Sent::Pause(why) => {
                self.pause(st, why)?;
                Ok(false)
            }
            Sent::Refused(why) => {
                needs(st, out, why);
                st.fails = 0;
                Ok(true)
            }
            Sent::Failed(why) => {
                st.fails += 1;
                if st.fails >= FAILS_TO_PAUSE {
                    self.pause(st, format!("{FAILS_TO_PAUSE} requests in a row didn't go through. The last said: {why}"))?;
                }
                Ok(false)
            }
        }
    }
}

/// The settings the waves follow, in one line.
fn settings_sig(db: &crate::db::DbHandle) -> String {
    let s = crate::settings::load(db);
    format!("{}|{:?}|{}|{}|{}|{}", s.bring_in, s.bring_in_waves, s.bring_in_per_day, s.bring_in_min_free_gb, s.bring_in_max_queue, s.lidarr_practice_until)
}

const UNREACHABLE: &str = "Lidarr can't be reached. It carries on once Platter can reach it.";

fn album_query(min: u32) -> Query {
    Query { kind: Kind::Album, offset: 0, min_plays: min, period: "all".into() }
}

fn classify(e: &AppError) -> Sent {
    let m = e.message();
    match e.kind() {
        "network" => Sent::Pause(UNREACHABLE.into()),
        _ if m.contains("red items") => Sent::Pause(m),
        "invalid" | "notFound" => Sent::Refused(m),
        _ => Sent::Failed(m),
    }
}

/// While Platter is open: a pass every [`LOOK_EVERY`], or sooner when poked.
pub async fn watch(w: Waves) {
    tokio::time::sleep(Duration::from_secs(90)).await;
    loop {
        if (crate::settings::load(&w.l().inner.db).bring_in || !w.state().queued.is_empty())
            && let Err(e) = w.pass().await
        {
            tracing::warn!("bring in my listening: {e}");
        }
        tokio::select! {
            _ = tokio::time::sleep(LOOK_EVERY) => {}
            _ = w.wake.notified() => {}
        }
    }
}

/// The local day number of a unix time, from SQLite's local time.
pub fn local_day(db: &crate::db::DbHandle, at: i64) -> i64 {
    db.read(|c| Ok(c.query_row("SELECT CAST(julianday(date(?1, 'unixepoch', 'localtime')) - 2440587.5 AS INTEGER)", [at], |r| r.get(0))?))
        .unwrap_or(at.div_euclid(86_400))
}

/// Key of an album on "Played a lot", for the UI and tests.
pub fn key_of(artist: &str, album: &str) -> String {
    album_key(artist, album)
}
