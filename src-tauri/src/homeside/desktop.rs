//! The desktop's side of Platter Bridge: Settings > Bridge (pair, list,
//! unpair), sending the Bridge how to reach Lidarr and Plex (sealed), keeping
//! shared Lidarr state in step with it, and the route through it for a
//! desktop away from home.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, mpsc, watch};

use super::code::PairCode;
use super::link::Link;
use super::links::{Links, Outbox};
use super::pair::Offer;
use super::peer::{self, Clock, Incoming, Wire};
use super::route::{self, BridgeRoute, Pending};
use super::wire::{BridgeConfig, BridgeStatus, MusicAccess, Msg};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::lidarr::Lidarr;
use crate::lidarr::api::LidarrRoute;
use crate::lidarr::homeside::QueuedWrite;
use crate::lidarr::waves::Waves;
use crate::plex::PlexState;
use crate::plex::keychain::{ACCOUNT_TOKEN, TokenStore};
use crate::social::listen::relay::{RelayEvent, RelayFactory};
use crate::sync::SyncStore;
use crate::sync::store::LIDARR;

/// A Bridge heard from within this long counts as answering (it says
/// it's alive every half hour).
pub const ONLINE_S: i64 = 75 * 60;
const SETTLE: Duration = Duration::from_secs(3);

fn seen_key(id: &str) -> String {
    format!("bridge.seen.{id}")
}

/// What the desktop last heard from a Bridge (this device only).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
    pub last_seen: Option<i64>,
    #[serde(default)]
    pub status: Option<BridgeStatus>,
    #[serde(default)]
    pub music: MusicAccess,
    /// It can run Bring in my listening.
    #[serde(default)]
    pub waves: bool,
}

/// One paired Bridge, for Settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
    pub online: bool,
    pub status: Option<BridgeStatus>,
    pub music: MusicAccess,
}

/// A code waiting for its Bridge.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairStart {
    pub code: String,
    /// Unix seconds.
    pub expires_at: i64,
}

/// Everything Settings > Bridge shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeOverview {
    pub bridges: Vec<BridgeInfo>,
    pub pairing: Option<PairStart>,
    /// Lidarr settings changes a Bridge queued for a yes.
    pub queued: Vec<QueuedWrite>,
}

pub struct DesktopDeps {
    pub db: DbHandle,
    pub keys: Arc<dyn TokenStore>,
    pub relays: Arc<dyn RelayFactory>,
    pub clock: Clock,
    pub store: SyncStore,
    pub lidarr: Lidarr,
    pub plex: Option<PlexState>,
    /// This computer's name.
    pub name: Arc<dyn Fn() -> String + Send + Sync>,
}

pub struct Desktop {
    d: DesktopDeps,
    pub wire: Arc<Wire>,
    pending: Arc<Pending>,
    tx: mpsc::Sender<Incoming>,
    rx: Mutex<Option<mpsc::Receiver<Incoming>>>,
    listeners: Mutex<HashMap<String, watch::Sender<bool>>>,
    pairing: Mutex<Option<(PairStart, watch::Sender<bool>)>>,
    /// Welcomes waiting for their link's listener to open.
    welcomes: Mutex<HashMap<String, (String, String, String)>>,
    links_changed: Notify,
    notify: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// The settings last sent, per link (this run only), so a change goes out.
    sent_config: Mutex<HashMap<String, String>>,
    /// Bring in my listening, which a Bridge can run.
    waves: RwLock<Option<Waves>>,
    waves_changed: Arc<Notify>,
    /// The plan last sent, per link (this run only).
    sent_plan: Mutex<HashMap<String, String>>,
}

impl Desktop {
    pub fn new(d: DesktopDeps) -> Arc<Self> {
        let links = Links::new(d.db.clone(), d.keys.clone());
        let wire = Arc::new(Wire::new(links, d.relays.clone(), d.clock.clone()));
        let (tx, rx) = mpsc::channel(64);
        let me = Arc::new(Self {
            d,
            wire,
            pending: Arc::default(),
            tx,
            rx: Mutex::new(Some(rx)),
            listeners: Mutex::default(),
            pairing: Mutex::default(),
            welcomes: Mutex::default(),
            links_changed: Notify::new(),
            notify: RwLock::default(),
            sent_config: Mutex::default(),
            waves: RwLock::default(),
            waves_changed: Arc::new(Notify::new()),
            sent_plan: Mutex::default(),
        });
        // Lidarr's last route: through a Bridge that's answering.
        let weak = Arc::downgrade(&me);
        me.d.lidarr.set_bridge_route(Some(Arc::new(move || weak.upgrade().and_then(|d| d.route()))));
        me
    }

    /// Bring in my listening: handed to a paired Bridge that can run it.
    pub fn set_waves(&self, w: Waves) {
        let n = self.waves_changed.clone();
        w.set_on_change(Arc::new(move || n.notify_one()));
        *self.waves.write() = Some(w);
    }

    /// Send the Bridge Bring in my listening's plan (when it changed since
    /// the last one, or `always`): to the Bridge that has the waves, else
    /// the first one answering that can run them.
    pub async fn send_waves_plan(&self, always: bool) {
        let Some(w) = self.waves.read().clone() else { return };
        let now = self.now_s();
        let links = self.wire.links.links();
        let held = w.lease().and_then(|l| links.iter().find(|x| x.id == l.link).cloned());
        let link = held.or_else(|| {
            links.iter().find(|l| {
                let s = self.seen(&l.id);
                s.waves && s.last_seen.is_some_and(|t| now - t < ONLINE_S)
            }).cloned()
        });
        let Some(link) = link else { return };
        let name = self.wire.links.all().into_iter().find(|p| p.id == link.id).map(|p| p.name).unwrap_or_default();
        let plan = match w.bridge_plan(&link.id, &name).await {
            Ok(Some(p)) => p,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!("bring in my listening: {e}");
                return;
            }
        };
        let Ok(json) = serde_json::to_string(&plan) else { return };
        if !always && self.sent_plan.lock().get(&link.id) == Some(&json) {
            return;
        }
        match self.wire.send(&link, &Msg::WavesPlan(Box::new(plan))).await {
            Ok(()) => {
                self.sent_plan.lock().insert(link.id.clone(), json);
            }
            Err(e) => tracing::info!("bring in my listening: the Bridge didn't get the plan: {e}"),
        }
        self.changed();
    }

    /// Tell the UI something changed (set by the app).
    pub fn set_notify(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *self.notify.write() = Some(f);
    }

    fn changed(&self) {
        if let Some(f) = self.notify.read().clone() {
            f();
        }
    }

    fn now_s(&self) -> i64 {
        ((self.d.clock)() / 1000) as i64
    }

    fn seen(&self, id: &str) -> Seen {
        self.d.db.read(|c| kv_get(c, &seen_key(id))).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn set_seen(&self, id: &str, f: impl FnOnce(&mut Seen)) {
        let mut s = self.seen(id);
        f(&mut s);
        if let Ok(json) = serde_json::to_string(&s) {
            let _ = self.d.db.write(|c| kv_set(c, &seen_key(id), &json));
        }
    }

    /// The route through the first Bridge that's answering.
    pub(crate) fn route(&self) -> Option<Arc<dyn LidarrRoute>> {
        let now = self.now_s();
        let link = self.wire.links.links().into_iter().find(|l| self.seen(&l.id).last_seen.is_some_and(|t| now - t < ONLINE_S))?;
        Some(Arc::new(BridgeRoute { wire: self.wire.clone(), link, pending: self.pending.clone() }))
    }

    pub fn overview(&self) -> BridgeOverview {
        let now = self.now_s();
        let bridges = self
            .wire
            .links
            .all()
            .into_iter()
            .map(|p| {
                let s = self.seen(&p.id);
                BridgeInfo {
                    online: s.last_seen.is_some_and(|t| now - t < ONLINE_S),
                    id: p.id,
                    name: p.name,
                    version: p.version,
                    paired_at: p.paired_at,
                    last_seen: s.last_seen,
                    status: s.status,
                    music: s.music,
                }
            })
            .collect();
        let pairing = self.pairing.lock().as_ref().map(|(p, _)| p.clone()).filter(|p| p.expires_at > now);
        BridgeOverview { bridges, pairing, queued: self.d.lidarr.queued_writes() }
    }

    // ------------------------------------------------------------ pairing

    /// Show a new pair code and wait (up to an hour) for a Bridge to use it.
    pub fn pair_start(self: &Arc<Self>, relay: Option<String>) -> AppResult<PairStart> {
        let code = PairCode::new(relay, self.now_s());
        let start = PairStart { code: code.text(), expires_at: i64::from(code.expires) };
        let (stop_tx, stop) = watch::channel(false);
        if let Some((_, old)) = self.pairing.lock().replace((start.clone(), stop_tx)) {
            let _ = old.send(true);
        }
        let me = self.clone();
        tokio::spawn(async move {
            if let Err(e) = me.wait_for_bridge(code, stop).await {
                tracing::info!("bridge pairing: {e}");
            }
            me.changed();
        });
        Ok(start)
    }

    pub fn pair_cancel(&self) {
        if let Some((_, stop)) = self.pairing.lock().take() {
            let _ = stop.send(true);
        }
    }

    async fn wait_for_bridge(self: &Arc<Self>, code: PairCode, mut stop: watch::Receiver<bool>) -> AppResult<()> {
        let relay_url = code.relay.clone().unwrap_or_else(|| super::DEFAULT_RELAY.to_string());
        let relay = self.d.relays.relay(&relay_url);
        let mut offer = Offer::new(code.clone(), &(self.d.name)());
        let topic = offer.listen_topic();
        loop {
            let mut rx = relay.subscribe(std::slice::from_ref(&topic), None).await.map_err(|e| AppError::Network(peer::relay_error(e)))?;
            loop {
                let left = (i64::from(code.expires) - self.now_s()).max(1) as u64;
                tokio::select! {
                    _ = stop.changed() => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_secs(left)) => {
                        self.pairing.lock().take();
                        return Err(AppError::Invalid("The pair code expired.".into()));
                    }
                    ev = rx.recv() => match ev {
                        Some(RelayEvent::Message { body, .. }) => {
                            if let Some(((wtopic, wbody), paired)) = offer.answer(&body, (self.d.clock)()) {
                                self.wire.links.add(&paired.link, &paired.name, &paired.version, self.now_s())?;
                                self.welcomes.lock().insert(paired.link.id.clone(), (relay_url.clone(), wtopic, wbody));
                                self.pairing.lock().take();
                                self.links_changed.notify_one();
                                tracing::info!("Paired with a Bridge.");
                                return Ok(());
                            }
                        }
                        Some(RelayEvent::Open) => {}
                        None => break,
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// Forget a Bridge (and tell it to forget this desktop).
    pub async fn unpair(&self, id: &str) -> AppResult<()> {
        if let Some(link) = self.wire.links.link(id) {
            let _ = tokio::time::timeout(Duration::from_secs(10), self.wire.send(&link, &Msg::Unpair)).await;
        }
        self.forget(id)
    }

    fn forget(&self, id: &str) -> AppResult<()> {
        if let Some(stop) = self.listeners.lock().remove(id) {
            let _ = stop.send(true);
        }
        if let Some(w) = self.waves.read().clone() {
            // Its waves come back to this computer.
            let id = id.to_string();
            tokio::spawn(async move {
                if let Err(e) = w.bridge_gone(&id).await {
                    tracing::warn!("bring in my listening: {e}");
                }
            });
        }
        self.wire.links.remove(id)?;
        let _ = self.d.db.write(|c| {
            c.execute("DELETE FROM kv WHERE key = ?1", [seen_key(id)])?;
            Ok(())
        });
        self.changed();
        Ok(())
    }

    // ------------------------------------------------------------ messages

    /// What the Bridge needs, read now: never stored on the relay unsealed.
    pub async fn config(&self) -> BridgeConfig {
        let s = crate::settings::load(&self.d.db);
        let url = if s.lidarr_url.trim().is_empty() { s.lidarr_away_url.clone() } else { s.lidarr_url.clone() };
        let plex_server = match &self.d.plex {
            Some(p) => p.machine_id().await,
            None => None,
        };
        BridgeConfig {
            desktop: (self.d.name)(),
            lidarr_url: url,
            lidarr_key: self.d.keys.get(crate::lidarr::KEY_ACCOUNT).ok().flatten().map(|t| t.expose().to_string()),
            plex_token: self.d.keys.get(ACCOUNT_TOKEN).ok().flatten().map(|t| t.expose().to_string()),
            plex_server,
            staging_folder: s.lidarr_staging_folder,
            scan_after_import: s.lidarr_scan_after_import,
            free_staging: s.lidarr_free_staging,
            practice_until: s.lidarr_practice_until,
            upgrades_ask: s.upgrades_ask,
        }
    }

    /// Send every Bridge the settings again (after a Lidarr or Plex change).
    pub async fn send_config(&self) {
        for link in self.wire.links.links() {
            self.send_config_to(&link, true).await;
        }
    }

    /// Send the settings when they changed since the last time (or `always`).
    async fn send_config_to(&self, link: &Link, always: bool) {
        let cfg = self.config().await;
        let Ok(json) = serde_json::to_string(&cfg) else { return };
        if !always && self.sent_config.lock().get(&link.id) == Some(&json) {
            return;
        }
        if self.wire.send(link, &Msg::Config(Box::new(cfg))).await.is_ok() {
            self.sent_config.lock().insert(link.id.clone(), json);
        }
    }

    pub async fn flush_entries(&self) {
        for link in self.wire.links.links() {
            let ob = Outbox::new(&self.d.db, &link.id);
            let Ok(batches) = ob.pending(&self.d.store) else { continue };
            for b in batches {
                if self.wire.send(&link, &Msg::Entries { entries: b.clone() }).await.is_ok() {
                    let _ = ob.mark(&b);
                } else {
                    break;
                }
            }
        }
    }

    pub async fn handle(&self, link: &Link, msg: Msg) {
        let now = self.now_s();
        self.set_seen(&link.id, |s| s.last_seen = Some(now));
        match msg {
            Msg::Hello { name, version, music, waves } => {
                let _ = self.wire.links.update(&link.id, |p| {
                    p.name = name.chars().take(60).collect();
                    p.version = version.chars().take(20).collect();
                });
                self.set_seen(&link.id, |s| {
                    s.music = music;
                    s.waves = waves;
                });
                self.send_config_to(link, true).await;
                self.flush_entries().await;
                if waves {
                    self.send_waves_plan(true).await;
                }
                self.changed();
            }
            Msg::WavesProgress(p) => {
                self.set_seen(&link.id, |s| s.waves = true);
                let w = self.waves.read().clone();
                if let Some(w) = w {
                    match w.bridge_progress(&link.id, *p).await {
                        Ok(true) => self.send_waves_plan(true).await,
                        Ok(false) => {}
                        Err(e) => tracing::warn!("bring in my listening: {e}"),
                    }
                }
                self.changed();
            }
            Msg::Status(st) => {
                self.set_seen(&link.id, |s| {
                    s.music = st.music;
                    s.status = Some(*st);
                });
                if self.seen(&link.id).waves {
                    self.send_waves_plan(false).await;
                }
                self.changed();
            }
            Msg::Entries { entries } => {
                if let Err(e) = self.d.store.merge(&entries, &[LIDARR.name()]) {
                    tracing::debug!("bridge entries: {e}");
                }
                let _ = Outbox::new(&self.d.db, &link.id).mark(&entries);
            }
            Msg::Answer { id, status, body } => self.pending.resolve(id, Ok(route::response(status, &body))),
            Msg::Failed { id, message } => self.pending.resolve(id, Err(message)),
            Msg::Unpair => {
                tracing::info!("A Bridge unpaired itself.");
                let _ = self.forget(&link.id);
            }
            Msg::Config(_) | Msg::Get { .. } | Msg::Write { .. } | Msg::WavesPlan(_) => {}
        }
    }

    /// Listeners for every link, shared changes out, messages in. Runs for
    /// the life of the app.
    pub async fn run(self: Arc<Self>) {
        let Some(mut rx) = self.rx.lock().take() else { return };
        let mut changes = self.d.store.subscribe();
        let mut dirty: Option<tokio::time::Instant> = None;
        // Lidarr and Plex settings changed here reach the Bridges within minutes.
        let mut settings = tokio::time::interval(Duration::from_secs(5 * 60));
        settings.tick().await;
        loop {
            for link in self.wire.links.links() {
                let mut l = self.listeners.lock();
                if !l.contains_key(&link.id) {
                    let (stop_tx, stop) = watch::channel(false);
                    l.insert(link.id.clone(), stop_tx);
                    tokio::spawn(peer::listen(self.wire.clone(), link, self.tx.clone(), stop));
                }
            }
            let flush_at = dirty.map(|d| d + SETTLE);
            tokio::select! {
                Some(inc) = rx.recv() => match inc {
                    Incoming::Opened(id) => {
                        let w = self.welcomes.lock().remove(&id);
                        if let Some((relay, topic, body)) = w {
                            if let Err(e) = self.d.relays.relay(&relay).publish(&topic, &body).await {
                                tracing::info!("bridge pairing: {}", peer::relay_error(e));
                            }
                            self.changed();
                        } else {
                            // Back in touch: the Bridge running the waves says how they went.
                            self.send_waves_plan(true).await;
                        }
                    }
                    Incoming::Msg(id, msg) => {
                        if let Some(link) = self.wire.links.link(&id) {
                            self.handle(&link, msg).await;
                        }
                    }
                },
                _ = self.links_changed.notified() => {}
                _ = self.waves_changed.notified() => self.send_waves_plan(false).await,
                _ = settings.tick() => {
                    for link in self.wire.links.links() {
                        self.send_config_to(&link, false).await;
                    }
                    self.send_waves_plan(false).await;
                }
                c = changes.recv() => {
                    if c.is_ok_and(|c| c.namespaces.iter().any(|n| n == LIDARR.name())) && !self.wire.links.all().is_empty() {
                        dirty.get_or_insert_with(tokio::time::Instant::now);
                    }
                }
                _ = async { if let Some(t) = flush_at { tokio::time::sleep_until(t).await } else { std::future::pending::<()>().await } } => {
                    dirty = None;
                    self.flush_entries().await;
                }
            }
        }
    }
}
