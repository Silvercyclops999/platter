//! Running device links: a listener per link on its relay inbox and a
//! worker per link that does that link's sending and answering, so one
//! device that can't be reached never holds up another. A round with
//! every linked device on start, after reconnecting (not more than once a
//! while) and every few minutes, and local changes sent a moment after the
//! last edit.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use tokio::sync::{mpsc, watch};

use super::clock;
use super::lan::LanSend;
use super::protocol::{Copying, Paused, SyncPeer};
use crate::homeside::link::{self, Link};
use crate::homeside::peer::{self as relay_peer, Incoming, Wire};
use crate::homeside::wire::{Msg, PROTO};
use crate::social::listen::relay::RelayError;
use crate::sync::store::SETTINGS;

/// Anything bigger than this, deflated, goes over the local network only
/// (or waits until the devices meet): the relay is for small changes.
pub const RELAY_MAX: usize = 300 * 1024;
/// Work waiting per link at most; more is dropped (the next round repairs).
const WORK_MAX: usize = 64;

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Quiet time after a local change before it's sent.
    pub debounce: Duration,
    /// A round with every linked device at least this often.
    pub every: Duration,
    /// A relay that reconnects starts a round at most this often per link.
    pub reopen: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { debounce: Duration::from_secs(2), every: Duration::from_secs(5 * 60), reopen: Duration::from_secs(30) }
    }
}

/// How the last send on a link went.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PathState {
    #[default]
    Unknown,
    Ok,
    /// The relay is turning messages away for a while.
    RelayBusy,
    /// Neither path reached the other device.
    Away,
    /// Something big is waiting for the local network.
    WaitingForLan,
}

/// One linked device, for Settings > Sync.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    pub id: String,
    pub name: String,
    /// When the two were last in step (ms, this device's clock).
    pub last_synced_at: Option<u64>,
    /// One plain line.
    pub words: String,
    /// A newer Platter on the other device.
    pub update_other: Option<String>,
    /// The clocks are far apart.
    pub clock: Option<String>,
    /// Entries held back from a clock far ahead.
    pub held: usize,
}

/// What a link's worker does next.
enum Work {
    /// Its relay listener (re)connected.
    Opened,
    Msg(Msg),
    Round,
    Push,
}

/// A link's relay listener and its worker.
struct LinkTasks {
    listen: watch::Sender<bool>,
    stop: watch::Sender<bool>,
    work: mpsc::Sender<Work>,
}

impl LinkTasks {
    fn stop(&self) {
        let _ = self.listen.send(true);
        let _ = self.stop.send(true);
    }
}

pub struct DeviceSync {
    pub peer: Arc<SyncPeer>,
    pub wire: Arc<Wire>,
    timing: Timing,
    paths: Mutex<HashMap<String, PathState>>,
    tasks: Mutex<HashMap<String, LinkTasks>>,
    lan: RwLock<Option<Arc<dyn LanSend>>>,
    /// One answer to a request from the local network at a time, per link.
    replies: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// When each link last had a round.
    rounds: Mutex<HashMap<String, tokio::time::Instant>>,
    /// A link was added, or "Sync now".
    kick: tokio::sync::Notify,
    /// Keep my devices in sync (Settings); on when not set.
    switch: RwLock<Option<Switch>>,
    on_status: Mutex<Option<OnStatus>>,
    last_status: Mutex<Vec<DeviceStatus>>,
    /// The sync loop stopped on a fault and is starting again.
    stopped: AtomicBool,
    /// Where listeners hand what arrives (while the loop runs).
    incoming: Mutex<Option<mpsc::Sender<Incoming>>>,
}

type Switch = Arc<dyn Fn() -> bool + Send + Sync>;
type OnStatus = Arc<dyn Fn(&[DeviceStatus]) + Send + Sync>;

impl DeviceSync {
    pub fn new(peer: Arc<SyncPeer>, wire: Arc<Wire>, timing: Timing) -> Self {
        Self {
            peer,
            wire,
            timing,
            paths: Mutex::default(),
            tasks: Mutex::default(),
            lan: RwLock::default(),
            replies: Mutex::default(),
            rounds: Mutex::default(),
            kick: tokio::sync::Notify::new(),
            switch: RwLock::default(),
            on_status: Mutex::default(),
            last_status: Mutex::default(),
            stopped: AtomicBool::new(false),
            incoming: Mutex::default(),
        }
    }

    /// Follow Keep my devices in sync: while it's off nothing is sent,
    /// taken or listened for.
    pub fn set_switch(&self, f: Switch) {
        *self.switch.write() = Some(f);
    }

    pub fn is_on(&self) -> bool {
        self.switch.read().as_ref().is_none_or(|f| f())
    }

    /// Called with every linked device's status when any of it changes.
    pub fn set_on_status(&self, f: OnStatus) {
        *self.on_status.lock() = Some(f);
    }

    /// Tell the window if the status changed.
    pub fn status_changed(&self) {
        let now = self.status();
        let mut last = self.last_status.lock();
        if *last != now {
            *last = now.clone();
            drop(last);
            if let Some(f) = self.on_status.lock().clone() {
                f(&now);
            }
        }
    }

    /// The sync loop stopped on a fault (true) or runs again (false).
    pub fn set_stopped(&self, stopped: bool) {
        self.stopped.store(stopped, Ordering::Relaxed);
        self.status_changed();
    }

    /// A device was paired, or the user asked to sync now: listen on any
    /// new link and run a round with every device.
    pub fn kick(&self) {
        self.kick.notify_one();
    }

    /// Use the local network when the other device answers there.
    pub fn set_lan(&self, lan: Arc<dyn LanSend>) {
        *self.lan.write() = Some(lan);
    }

    pub(crate) fn lan(&self) -> Option<Arc<dyn LanSend>> {
        self.lan.read().clone()
    }

    pub(crate) fn reply_lock(&self, link_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.replies.lock().entry(link_id.to_string()).or_default().clone()
    }

    /// The other device answered on the local network.
    pub(crate) fn lan_ok(&self, link_id: &str) {
        self.set_path(link_id, PathState::Ok);
    }

    fn set_path(&self, id: &str, s: PathState) {
        self.paths.lock().insert(id.to_string(), s);
    }

    /// Send messages on a link through the relay, in order. Stops at the
    /// first that doesn't go (the next round repairs what's missing).
    /// Entries past [`RELAY_MAX`] in one go wait for the local network or
    /// a later round.
    pub async fn deliver(&self, link: &Link, msgs: Vec<Msg>) {
        let mut budget = RELAY_MAX;
        let mut waiting = false;
        let mut sent = Vec::new();
        let mut failed = None;
        for m in msgs {
            let size = link::packed_len(&m);
            let bulk = matches!(m, Msg::SyncEntries { .. } | Msg::SyncDiff { .. });
            if size > RELAY_MAX || (bulk && size > budget) {
                tracing::debug!(kind = m.kind(), "device sync: too big for the relay, waiting for the local network");
                waiting = true;
                continue;
            }
            budget = budget.saturating_sub(size);
            match self.wire.send(link, &m).await {
                Ok(()) => sent.push(m),
                Err(e) => {
                    let busy = e.to_string().contains(&relay_peer::relay_error(RelayError::Limited));
                    tracing::info!("device sync: {e}");
                    failed = Some(if busy { PathState::RelayBusy } else { PathState::Away });
                    break;
                }
            }
        }
        if !sent.is_empty() {
            let link2 = link.clone();
            self.peer.blocking(move |p| p.sent_all(&link2, &sent)).await;
        }
        if let Some(f) = failed {
            self.set_path(&link.id, f);
            return;
        }
        let before = self.paths.lock().get(&link.id).copied();
        self.set_path(&link.id, if waiting { PathState::WaitingForLan } else { PathState::Ok });
        // The relay is back: listen again now rather than after the
        // listener's growing wait, and pick up what came meanwhile.
        if matches!(before, Some(PathState::Away | PathState::RelayBusy)) {
            self.listen_again(link);
        }
    }

    /// A new relay listener for a link in place of its old one.
    fn listen_again(&self, link: &Link) {
        let Some(tx) = self.incoming.lock().clone() else { return };
        let mut tasks = self.tasks.lock();
        let Some(t) = tasks.get_mut(&link.id) else { return };
        let _ = t.listen.send(true);
        let (listen, rx) = watch::channel(false);
        t.listen = listen;
        tokio::spawn(relay_peer::listen(self.wire.clone(), link.clone(), tx, rx));
    }

    /// A round with one linked device.
    pub async fn round(&self, link: &Link) {
        self.rounds.lock().insert(link.id.clone(), tokio::time::Instant::now());
        match self.peer.hello(false).await {
            Some(h) => self.send(link, vec![h]).await,
            // Signed out: nothing goes, and the device says why.
            None => self.peer.signed_out(&link.id),
        }
    }

    /// A round with every linked device ("Sync now").
    pub async fn sync_now(&self) {
        if !self.is_on() {
            return;
        }
        for link in self.wire.links.links() {
            self.round(&link).await;
        }
    }

    /// "Copy settings from my computer": take the synced settings a linked
    /// computer has, as they are there, so they win here and everywhere.
    /// The computer that was in step most recently is asked (a phone or
    /// tablet only when there's no computer). Returns its name.
    pub async fn copy_settings(&self, wait: Duration) -> crate::error::AppResult<String> {
        use crate::error::AppError;
        if !self.is_on() {
            return Err(AppError::Invalid("Turn on Keep my devices in sync first.".into()));
        }
        if !self.peer.keeps_settings() {
            return Err(AppError::Invalid("Settings aren't kept in sync on this device. Turn on Settings under What to sync first.".into()));
        }
        let mobile = |p: &str| matches!(p.to_ascii_lowercase().as_str(), "ios" | "ipados" | "android");
        let mut all = self.wire.links.all();
        all.sort_by_key(|p| {
            let seen = self.peer.seen(&p.id).unwrap_or_default();
            (mobile(&seen.platform), std::cmp::Reverse(p.last_synced_at))
        });
        let Some(p) = all.first() else {
            return Err(AppError::NotFound("Pair this device with your computer first, in Settings under Keep my devices in sync.".into()));
        };
        let link = self.wire.links.link(&p.id).ok_or_else(|| AppError::NotFound("That device isn't linked here.".into()))?;
        let seen = self.peer.seen(&p.id).unwrap_or_default();
        let name = if seen.name.is_empty() { p.name.clone() } else { seen.name };
        if self.peer.peer_skips(&link.id, SETTINGS.name()) {
            return Err(AppError::Invalid(format!("{name} doesn't keep its settings in sync. Turn on Settings under What to sync there first.")));
        }
        // The other end answers only once it has checked this one's account:
        // a hello goes first when this run hasn't met it yet.
        let mut msgs = Vec::new();
        if !self.peer.verified(&link.id) {
            msgs.extend(self.peer.hello(false).await);
        }
        msgs.push(self.peer.copy_settings(&link));
        self.send(&link, msgs).await;
        let until = tokio::time::Instant::now() + wait;
        while self.peer.copying(&link.id) == Some(Copying::Waiting) && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let r = self.peer.copying(&link.id);
        self.peer.stop_copying(&link.id);
        match r {
            Some(Copying::Done) => Ok(name),
            Some(Copying::Failed(m)) => Err(AppError::Invalid(m)),
            _ => Err(AppError::Network(format!("{name} didn't answer. Open Platter there and try again."))),
        }
    }

    /// Local changes out to every linked device.
    pub async fn push_all(&self) {
        if !self.is_on() {
            return;
        }
        for link in self.wire.links.links() {
            let msgs = self.peer.push(&link).await;
            self.send(&link, msgs).await;
        }
    }

    /// One message in from a link: answered on the same link.
    pub async fn incoming(&self, link: &Link, msg: Msg) {
        if !self.is_on() {
            return;
        }
        let replies = self.peer.handle(link, msg).await;
        self.send(link, replies).await;
    }

    /// Unpair: tell the other device, then forget it here (also when it
    /// can't be reached).
    pub async fn forget(&self, link_id: &str) -> crate::error::AppResult<()> {
        if let Some(link) = self.wire.links.link(link_id) {
            let _ = tokio::time::timeout(Duration::from_secs(10), self.wire.send(&link, &Msg::SyncBye)).await;
        }
        self.drop_link(link_id);
        self.peer.forget(link_id)
    }

    /// Stop a link's tasks and forget what this run kept for it.
    fn drop_link(&self, link_id: &str) {
        if let Some(t) = self.tasks.lock().remove(link_id) {
            t.stop();
        }
        self.paths.lock().remove(link_id);
        self.rounds.lock().remove(link_id);
        self.replies.lock().remove(link_id);
    }

    /// Every linked device, for Settings > Sync.
    pub fn status(&self) -> Vec<DeviceStatus> {
        let paths = self.paths.lock().clone();
        let on = self.is_on();
        let stopped = self.stopped.load(Ordering::Relaxed);
        let now = self.peer.deps().store.now();
        // Not heard from for two rounds and more: no longer "In sync".
        let stale = |t: u64| now.saturating_sub(t) > 2 * self.timing.every.as_millis() as u64;
        self.wire
            .links
            .all()
            .into_iter()
            .map(|p| {
                let seen = self.peer.seen(&p.id).unwrap_or_default();
                let name = if seen.name.is_empty() { p.name.clone() } else { seen.name.clone() };
                let path = paths.get(&p.id).copied().unwrap_or_default();
                let words = match (seen.paused, path) {
                    _ if !on => "Keep my devices in sync is off".to_string(),
                    _ if stopped => "Sync stopped, starting again".to_string(),
                    (Some(Paused::OtherAccount), _) => "This device is signed in to a different account. Sync paused".to_string(),
                    (Some(Paused::SignedOut), _) => "Sign in to Plex to keep your devices in sync".to_string(),
                    _ if seen.save_failed => format!("Couldn't save changes from {name}"),
                    (None, PathState::RelayBusy) => "The relay is busy, trying again later".to_string(),
                    (None, PathState::Away) => format!("{name} is away, changes are kept and will go when it's back"),
                    (None, PathState::WaitingForLan) => format!("Waiting for {name}"),
                    _ if seen.verified && p.last_synced_at.is_some_and(stale) => format!("Not in touch with {name} lately"),
                    _ if seen.verified && p.last_synced_at.is_some() => "In sync".to_string(),
                    _ => format!("Waiting for {name}"),
                };
                DeviceStatus {
                    id: p.id.clone(),
                    last_synced_at: p.last_synced_at,
                    update_other: (seen.proto > PROTO).then(|| format!("Update Platter on {name} to sync everything")),
                    clock: seen.clock_offset_ms.and_then(|o| clock::peer_words(&name, o)),
                    held: seen.held,
                    words,
                    name,
                }
            })
            .collect()
    }

    /// Stop every link's tasks (switched off, or the loop is ending).
    fn stop_links(&self) {
        for (_, t) in self.tasks.lock().drain() {
            t.stop();
        }
    }

    /// A listener and a worker for every link that has none (or whose
    /// tasks ended), and none for links that are gone.
    fn listen_new(self: &Arc<Self>, tx: &mpsc::Sender<Incoming>) {
        if !self.is_on() {
            // Off: stop listening until it's on again.
            self.stop_links();
            return;
        }
        let links = self.wire.links.links();
        let mut tasks = self.tasks.lock();
        tasks.retain(|id, t| {
            let keep = links.iter().any(|l| &l.id == id);
            let alive = !t.listen.is_closed() && !t.work.is_closed();
            if keep && !alive {
                tracing::warn!("device sync: a link's tasks stopped, starting them again");
            }
            if !keep || !alive {
                t.stop();
            }
            keep && alive
        });
        for link in links {
            if tasks.contains_key(&link.id) {
                continue;
            }
            let (listen, listen_rx) = watch::channel(false);
            let (stop_tx, stop) = watch::channel(false);
            let (work_tx, work_rx) = mpsc::channel(WORK_MAX);
            tasks.insert(link.id.clone(), LinkTasks { listen, stop: stop_tx, work: work_tx });
            tokio::spawn(relay_peer::listen(self.wire.clone(), link.clone(), tx.clone(), listen_rx));
            tokio::spawn(self.clone().work(link.id, work_rx, stop));
        }
    }

    /// Hand work to a link's worker (dropped when it has too much waiting).
    fn dispatch(&self, id: &str, w: Work) {
        let tasks = self.tasks.lock();
        let Some(t) = tasks.get(id) else { return };
        if let Err(e) = t.work.try_send(w) {
            tracing::debug!("device sync: a link is busy, work left for the next round: {e}");
        }
    }

    fn dispatch_all(&self, w: impl Fn() -> Work) {
        let ids: Vec<String> = self.tasks.lock().keys().cloned().collect();
        for id in ids {
            self.dispatch(&id, w());
        }
    }

    /// One link's sending and answering, in order, until `stop`.
    async fn work(self: Arc<Self>, id: String, mut rx: mpsc::Receiver<Work>, mut stop: watch::Receiver<bool>) {
        loop {
            let w = tokio::select! {
                w = rx.recv() => match w {
                    Some(w) => w,
                    None => return,
                },
                _ = stop.changed() => return,
            };
            if !self.is_on() {
                continue;
            }
            let Some(link) = self.wire.links.link(&id) else { return };
            match w {
                Work::Opened => {
                    // A relay that keeps reconnecting doesn't start a round each time.
                    let recent = self.rounds.lock().get(&id).is_some_and(|t| t.elapsed() < self.timing.reopen);
                    if !recent {
                        self.round(&link).await;
                    }
                }
                Work::Round => self.round(&link).await,
                Work::Push => {
                    let msgs = self.peer.push(&link).await;
                    self.send(&link, msgs).await;
                }
                Work::Msg(m) => self.incoming(&link, m).await,
            }
            self.status_changed();
        }
    }

    /// Listeners and workers for every link, rounds, local changes out.
    /// Runs until `stop`. Waits on its events only; the work itself is
    /// each link's worker's.
    pub async fn run(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        // Tasks from an earlier run send to a channel that's gone.
        self.stop_links();
        self.set_stopped(false);
        let (tx, mut rx) = mpsc::channel::<Incoming>(64);
        *self.incoming.lock() = Some(tx.clone());
        let mut changes = self.peer.deps().store.subscribe();
        let mut changes_open = true;
        let mut dirty: Option<tokio::time::Instant> = None;
        let mut every = tokio::time::interval(self.timing.every);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            self.listen_new(&tx);
            let push_at = dirty.map(|d| d + self.timing.debounce);
            tokio::select! {
                Some(inc) = rx.recv() => match inc {
                    Incoming::Opened(id) => self.dispatch(&id, Work::Opened),
                    Incoming::Msg(id, msg) => self.dispatch(&id, Work::Msg(msg)),
                },
                _ = every.tick() => {
                    if self.is_on() {
                        self.dispatch_all(|| Work::Round);
                    }
                }
                _ = self.kick.notified() => {
                    self.listen_new(&tx);
                    self.dispatch_all(|| Work::Round);
                }
                c = changes.recv(), if changes_open => match c {
                    // Missed some: there were changes all the same.
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if !self.wire.links.all().is_empty() {
                            dirty = Some(tokio::time::Instant::now());
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::warn!("device sync: the store's change feed closed; changes go with each round");
                        changes_open = false;
                    }
                },
                _ = async { if let Some(t) = push_at { tokio::time::sleep_until(t).await } else { std::future::pending::<()>().await } } => {
                    dirty = None;
                    if self.is_on() {
                        self.dispatch_all(|| Work::Push);
                    }
                }
                _ = stop.changed() => {
                    self.stop_links();
                    *self.incoming.lock() = None;
                    return;
                }
            }
            self.status_changed();
        }
    }
}

/// Run a long-lived task, starting it again (waiting longer each time) if
/// it stops on a fault; a normal end ends it. `on_stop(true)` while it's
/// down, `on_stop(false)` once it runs again.
pub async fn supervise<F, Fut>(name: &'static str, on_stop: impl Fn(bool) + Send + Sync + 'static, make: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let mut wait = Duration::from_secs(2);
    loop {
        match tokio::spawn(make()).await {
            Ok(()) => return,
            Err(e) => {
                tracing::error!("{name} stopped: {e}. Starting it again.");
                on_stop(true);
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(300));
                on_stop(false);
            }
        }
    }
}
