//! Running device links: a listener per link on its relay inbox, a round
//! with every linked device on start, after reconnecting and every few
//! minutes, and local changes sent a moment after the last edit.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use tokio::sync::{mpsc, watch};

use super::clock;
use super::lan::LanSend;
use super::protocol::{Paused, SyncPeer};
use crate::homeside::link::{self, Link};
use crate::homeside::peer::{self as relay_peer, Incoming, Wire};
use crate::homeside::wire::{Msg, PROTO};
use crate::social::listen::relay::RelayError;

/// Anything bigger than this, deflated, goes over the local network only
/// (or waits until the devices meet): the relay is for small changes.
pub const RELAY_MAX: usize = 300 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Quiet time after a local change before it's sent.
    pub debounce: Duration,
    /// A round with every linked device at least this often.
    pub every: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { debounce: Duration::from_secs(2), every: Duration::from_secs(5 * 60) }
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

pub struct DeviceSync {
    pub peer: Arc<SyncPeer>,
    pub wire: Arc<Wire>,
    timing: Timing,
    paths: Mutex<HashMap<String, PathState>>,
    listeners: Mutex<HashMap<String, watch::Sender<bool>>>,
    lan: RwLock<Option<Arc<dyn LanSend>>>,
    /// One answer to a request from the local network at a time, per link.
    replies: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl DeviceSync {
    pub fn new(peer: Arc<SyncPeer>, wire: Arc<Wire>, timing: Timing) -> Self {
        Self {
            peer,
            wire,
            timing,
            paths: Mutex::default(),
            listeners: Mutex::default(),
            lan: RwLock::default(),
            replies: Mutex::default(),
        }
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
                Ok(()) => self.peer.sent(link, &m),
                Err(e) => {
                    let busy = e.to_string().contains(&relay_peer::relay_error(RelayError::Limited));
                    tracing::info!("device sync: {e}");
                    self.set_path(&link.id, if busy { PathState::RelayBusy } else { PathState::Away });
                    return;
                }
            }
        }
        let before = self.paths.lock().get(&link.id).copied();
        self.set_path(&link.id, if waiting { PathState::WaitingForLan } else { PathState::Ok });
        // The relay is back: listen again now rather than after the
        // listener's growing wait, and pick up what came meanwhile.
        if matches!(before, Some(PathState::Away | PathState::RelayBusy))
            && let Some(stop) = self.listeners.lock().remove(&link.id)
        {
            let _ = stop.send(true);
        }
    }

    /// A round with one linked device.
    pub async fn round(&self, link: &Link) {
        if let Some(h) = self.peer.hello(false).await {
            self.send(link, vec![h]).await;
        }
    }

    /// A round with every linked device ("Sync now").
    pub async fn sync_now(&self) {
        for link in self.wire.links.links() {
            self.round(&link).await;
        }
    }

    /// Local changes out to every linked device.
    pub async fn push_all(&self) {
        for link in self.wire.links.links() {
            let msgs = self.peer.push(&link).await;
            self.send(&link, msgs).await;
        }
    }

    /// One message in from a link: answered on the same link.
    pub async fn incoming(&self, link: &Link, msg: Msg) {
        let replies = self.peer.handle(link, msg).await;
        self.send(link, replies).await;
    }

    /// Unpair: tell the other device, then forget it here (also when it
    /// can't be reached).
    pub async fn forget(&self, link_id: &str) -> crate::error::AppResult<()> {
        if let Some(link) = self.wire.links.link(link_id) {
            let _ = tokio::time::timeout(Duration::from_secs(10), self.wire.send(&link, &Msg::SyncBye)).await;
        }
        if let Some(stop) = self.listeners.lock().remove(link_id) {
            let _ = stop.send(true);
        }
        self.paths.lock().remove(link_id);
        self.peer.forget(link_id)
    }

    /// Every linked device, for Settings > Sync.
    pub fn status(&self) -> Vec<DeviceStatus> {
        let paths = self.paths.lock().clone();
        self.wire
            .links
            .all()
            .into_iter()
            .map(|p| {
                let seen = self.peer.seen(&p.id).unwrap_or_default();
                let name = if seen.name.is_empty() { p.name.clone() } else { seen.name.clone() };
                let path = paths.get(&p.id).copied().unwrap_or_default();
                let words = match (seen.paused, path) {
                    (Some(Paused::OtherAccount), _) => "This device is signed in to a different account. Sync paused".to_string(),
                    (Some(Paused::SignedOut), _) => "Sign in to Plex to keep your devices in sync".to_string(),
                    (None, PathState::RelayBusy) => "The relay is busy, trying again later".to_string(),
                    (None, PathState::Away) => format!("{name} is away, changes are kept and will go when it's back"),
                    (None, PathState::WaitingForLan) => format!("Waiting for {name}"),
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

    fn listen_new(&self, tx: &mpsc::Sender<Incoming>) {
        for link in self.wire.links.links() {
            let mut l = self.listeners.lock();
            if !l.contains_key(&link.id) {
                let (stop_tx, stop) = watch::channel(false);
                l.insert(link.id.clone(), stop_tx);
                tokio::spawn(relay_peer::listen(self.wire.clone(), link, tx.clone(), stop));
            }
        }
    }

    /// Listeners for every link, rounds, local changes out. Runs until
    /// `stop`.
    pub async fn run(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let (tx, mut rx) = mpsc::channel::<Incoming>(64);
        let mut changes = self.peer.deps().store.subscribe();
        let mut dirty: Option<tokio::time::Instant> = None;
        let mut every = tokio::time::interval(self.timing.every);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            self.listen_new(&tx);
            let push_at = dirty.map(|d| d + self.timing.debounce);
            tokio::select! {
                Some(inc) = rx.recv() => match inc {
                    Incoming::Opened(id) => {
                        if let Some(link) = self.wire.links.link(&id) {
                            self.round(&link).await;
                        }
                    }
                    Incoming::Msg(id, msg) => {
                        if let Some(link) = self.wire.links.link(&id) {
                            self.incoming(&link, msg).await;
                        }
                    }
                },
                _ = every.tick() => self.sync_now().await,
                c = changes.recv() => {
                    if c.is_ok() && !self.wire.links.all().is_empty() {
                        dirty = Some(tokio::time::Instant::now());
                    }
                }
                _ = async { if let Some(t) = push_at { tokio::time::sleep_until(t).await } else { std::future::pending::<()>().await } } => {
                    dirty = None;
                    self.push_all().await;
                }
                _ = stop.changed() => {
                    for (_, s) in self.listeners.lock().drain() {
                        let _ = s.send(true);
                    }
                    return;
                }
            }
        }
    }
}
