//! Requests from people the server is shared with.
//!
//! Only the server's owner gets music through Lidarr. Anyone else signed in
//! to the server (a friend it's shared with, someone on the owner's Plex
//! Home) sees Request where the owner sees Get. A request goes to the
//! owner's Platter over the relay ([`wire`]), the owner says yes or no with
//! one tap, and a yes goes through the owner's own Get, with every check
//! that has. The owner can mark someone "Straight away", and their requests
//! then go through by themselves, up to a weekly number of albums.
//!
//! Every Platter runs both sides and uses the one that fits who's signed in
//! ([`Role`]). The asking side keeps its requests and resends any the
//! owner's Platter hasn't seen yet, so a request made while the owner's
//! computer is off goes when it's back. The owner's side listens on every
//! person's channel, and on the answers too, so a yes given on the owner's
//! phone shows on the owner's computer.
//!
//! Nothing here reaches Lidarr except through [`AsksHost::get`] on the
//! owner's side, and nothing from the asking side is trusted: who asked is
//! known from the channel the message opened on, and every request is
//! checked again before it's sent.

pub mod commands;
pub mod live;
pub mod wire;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::lidarr::request::{GetNeeds, GetOutcome, RequestInput, RequestRecord};
use crate::lidarr::status::RequestState;
use crate::plex::token::Token;
use crate::social::listen::invite::relay_url;
use crate::social::listen::relay::{Relay, RelayEvent, RelayFactory};
use wire::{AskItem, AskState, Down, Pair, Up};

pub const EVENT_MINE: &str = "asks://mine";
pub const EVENT_INCOMING: &str = "asks://incoming";

/// Requests one person may have waiting on the owner at once.
pub const OPEN_MAX: usize = 10;
/// The window the weekly limit counts in.
pub const WEEK_MS: u64 = 7 * 24 * 3600 * 1000;
/// How often requests the owner hasn't seen are sent again.
pub const RESEND_EVERY: Duration = Duration::from_secs(120);
/// How often the owner's side follows requests Lidarr is getting.
pub const FOLLOW_EVERY: Duration = Duration::from_secs(20);
/// How often the owner's side reads who the server is shared with.
pub const PEOPLE_EVERY: Duration = Duration::from_secs(300);
/// A second request from the same person this soon doesn't notify again.
pub const GROUP_MS: u64 = 3 * 60 * 1000;
/// Settled requests are kept this long.
const KEEP_MS: u64 = 30 * 24 * 3600 * 1000;
/// A request sent this long before the owner's device started listening
/// is the relay catching it up: it's listed, but another of the owner's
/// devices may have answered it already, so nothing is sent or got for it
/// here. (The margin allows for clocks that disagree a little.)
pub const STALE_MS: u64 = 2 * 60 * 1000;
/// Messages one person's Platter gets answered in an hour.
const HEARD_PER_HOUR: usize = 60;
/// How often the asking side checks on requests still open, in resends.
const CHECK_EVERY: u32 = 15;
/// A request that came in this long ago still counts as already asked.
const READY_DEDUPE_MS: u64 = 24 * 3600 * 1000;

const MINE_KEY: &str = "asks.mine";
const INCOMING_KEY: &str = "asks.incoming";
const POLICY_KEY: &str = "asks.policy";
const SENT_KEY: &str = "asks.sent";
const SINCE_KEY: &str = "asks.since";

pub const OFF_NOTE: &str = "Requests are turned off for you on this server.";
pub const FULL_NOTE: &str = "You have 10 requests waiting. Take one back or wait for an answer.";
pub const PROTECTED_NOTE: &str = "This artist's music is looked after by hand on this server, so it can't be requested.";
pub const FAILED_NOTE: &str = "It didn't come through. You can request it again.";
pub const OVER_LIMIT_NOTE: &str = "That's this week's limit, so it went to the owner for a yes.";
pub const FIRST_YES_NOTE: &str = "The owner's first request needs a yes in Platter, so it's waiting for one.";

/// Who this device is on the connected server.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Role {
    /// Not signed in, or not on a Plex server.
    Nobody,
    Owner { machine_id: String },
    /// Signed in as someone the server is shared with: `token` is their own
    /// access token for it.
    Guest { machine_id: String, token: Token },
}

/// Someone the owner's server is shared with.
#[derive(Clone, Debug)]
pub struct Person {
    pub user_id: String,
    pub name: String,
    /// Their access token for the server (only to derive their channels).
    pub token: Token,
}

/// What the owner lets one person do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Rule {
    /// No Request button at all.
    Off,
    /// Each request waits for the owner's yes.
    #[default]
    Ask,
    /// Requests go through by themselves, up to the weekly limit.
    Straight,
}

/// The owner's choices.
#[derive(Debug, Clone, PartialEq)]
pub struct AskSettings {
    /// "Let people you share with request music".
    pub on: bool,
    /// Per person (by Plex user id); anyone not listed asks first.
    pub rules: Vec<(String, Rule)>,
    /// Albums a week a Straight away person gets without a yes.
    pub weekly_limit: u32,
    /// A second request from the same person within a few minutes doesn't
    /// notify again.
    pub group_notices: bool,
}

impl AskSettings {
    pub fn rule(&self, user_id: &str) -> Rule {
        if !self.on {
            return Rule::Off;
        }
        self.rules.iter().find(|(u, _)| u == user_id).map(|(_, r)| *r).unwrap_or_default()
    }
}

/// What the requests need from the app.
#[async_trait]
pub trait AsksHost: Send + Sync {
    async fn role(&self) -> Role;
    /// Owner: everyone the server is shared with who has accepted.
    async fn people(&self) -> AppResult<Vec<Person>>;
    /// Owner: Lidarr is set up for this device to use.
    fn lidarr_usable(&self) -> bool;
    /// Owner: the release group for something asked for, or why there's none.
    async fn pick(&self, item: &AskItem) -> Result<String, String>;
    /// Owner: the artist's music is kept by hand (Lidarr only stages it),
    /// or None when that can't be checked now.
    async fn protected(&self, input: &RequestInput) -> Option<bool>;
    /// Owner: send it through Get, with every check.
    async fn get(&self, input: RequestInput) -> AppResult<GetOutcome>;
    /// Owner: Lidarr's request for this release group, if there is one.
    fn record(&self, release_group_id: &str) -> Option<RequestRecord>;
    fn settings(&self) -> AskSettings;
    fn notify(&self, title: &str, body: &str);
    fn emit(&self, event: &str, payload: serde_json::Value);
    /// Asking side: something arrived, read the library again soon.
    fn refresh_library(&self) {}
    fn now_ms(&self) -> u64;
}

/// One of my requests (the asking side).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MyAsk {
    pub id: String,
    /// The server it was asked of.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub server: String,
    pub item: AskItem,
    pub state: AskState,
    #[serde(default)]
    pub progress: Option<u8>,
    #[serde(default)]
    pub note: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    /// The owner's Platter has seen it.
    #[serde(default)]
    pub seen: bool,
    /// The newest status from the owner (older ones are dropped).
    #[serde(default)]
    pub status_at: u64,
}

/// A request someone made of me (the owner's side).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Incoming {
    pub id: String,
    pub person_id: String,
    pub person_name: String,
    pub item: AskItem,
    #[serde(default)]
    pub release_group_id: Option<String>,
    pub state: AskState,
    #[serde(default)]
    pub progress: Option<u8>,
    #[serde(default)]
    pub note: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    /// It went through without a yes (Straight away, or already coming).
    #[serde(default)]
    pub by_itself: bool,
}

/// What the owner's Platter last said this device may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub can_ask: bool,
    pub straight_away: bool,
    pub at: u64,
}

/// `asks_view`: the asking side, for the current server.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsksView {
    /// Signed in as someone the server is shared with (not its owner).
    pub guest: bool,
    /// Request shows (signed in as someone the server is shared with, and
    /// the owner's Platter said they may).
    pub can_request: bool,
    pub straight_away: bool,
    pub mine: Vec<MyAsk>,
}

/// `asks_answer`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answered {
    pub incoming: Incoming,
    /// Get needs the owner first (the first request ever is confirmed by
    /// typing the album's name, or a check said no).
    pub needs: Option<GetNeeds>,
}

/// The channels in use now.
#[derive(Default)]
struct Live {
    relay: Option<Arc<dyn Relay>>,
    /// Owner: each person's channels, by Plex user id.
    people: HashMap<String, (String, Pair)>,
    /// Guest: my channels.
    mine: Option<(String, Pair)>,
}

struct Inner {
    host: Arc<dyn AsksHost>,
    db: DbHandle,
    relays: Arc<dyn RelayFactory>,
    /// The relay setting (listen along's); empty means the default.
    relay: Arc<RwLock<String>>,
    default_relay: String,
    mine: Mutex<Vec<MyAsk>>,
    incoming: Mutex<Vec<Incoming>>,
    policy: Mutex<HashMap<String, Policy>>,
    /// Straight away requests sent, (user id, ms), for the weekly limit.
    sent: Mutex<Vec<(String, u64)>>,
    /// Last notice per person, for grouping.
    noticed: Mutex<HashMap<String, u64>>,
    /// Messages answered per person in the last hour.
    heard: Mutex<HashMap<String, Vec<u64>>>,
    /// When the owner's side started listening after being away (anything
    /// sent well before is catch-up), and whether it's away now.
    listening_since: AtomicU64,
    away: AtomicBool,
    stale_ms: AtomicU64,
    live: Mutex<Live>,
    wake: Notify,
    /// The rules the people were last told about.
    told: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct Asks {
    inner: Arc<Inner>,
}

fn load<T: serde::de::DeserializeOwned + Default>(db: &DbHandle, key: &str) -> T {
    db.read(|c| kv_get(c, key)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn fold(s: &str) -> String {
    crate::lidarr::tap::fold(s)
}

/// Lidarr's state, as the asking side sees it.
fn from_record(r: &RequestRecord) -> (AskState, Option<u8>) {
    match r.state {
        RequestState::Queued | RequestState::Requested | RequestState::Searching | RequestState::Importing => (AskState::Getting, None),
        RequestState::Downloading => (AskState::Getting, r.progress),
        RequestState::Imported => (AskState::Ready, None),
        RequestState::NotFound => (AskState::NotFound, None),
        RequestState::Failed => (AskState::Failed, None),
    }
}

impl Asks {
    pub fn new(host: Arc<dyn AsksHost>, db: DbHandle, relays: Arc<dyn RelayFactory>, relay: Arc<RwLock<String>>, default_relay: String) -> Self {
        let mine = load(&db, MINE_KEY);
        let incoming = load(&db, INCOMING_KEY);
        let policy = load(&db, POLICY_KEY);
        let sent = load(&db, SENT_KEY);
        Self {
            inner: Arc::new(Inner {
                host,
                db,
                relays,
                relay,
                default_relay,
                mine: Mutex::new(mine),
                incoming: Mutex::new(incoming),
                policy: Mutex::new(policy),
                sent: Mutex::new(sent),
                noticed: Mutex::new(HashMap::new()),
                heard: Mutex::new(HashMap::new()),
                listening_since: AtomicU64::new(0),
                away: AtomicBool::new(true),
                stale_ms: AtomicU64::new(STALE_MS),
                live: Mutex::new(Live::default()),
                wake: Notify::new(),
                told: Mutex::new(None),
            }),
        }
    }

    fn now(&self) -> u64 {
        self.inner.host.now_ms()
    }

    /// Something changed (sign-in, server, settings): look again now.
    pub fn poke(&self) {
        self.inner.wake.notify_one();
    }

    // ---- storage ----------------------------------------------------------------

    fn save_mine(&self) {
        let text = serde_json::to_string(&*self.inner.mine.lock()).unwrap_or_default();
        if let Err(e) = self.inner.db.write(|c| kv_set(c, MINE_KEY, &text)) {
            tracing::warn!(kind = e.kind(), "requests: couldn't save mine");
        }
        self.inner.host.emit(EVENT_MINE, serde_json::json!({}));
    }

    fn save_incoming(&self) {
        let text = serde_json::to_string(&*self.inner.incoming.lock()).unwrap_or_default();
        if let Err(e) = self.inner.db.write(|c| kv_set(c, INCOMING_KEY, &text)) {
            tracing::warn!(kind = e.kind(), "requests: couldn't save incoming");
        }
        self.inner.host.emit(EVENT_INCOMING, serde_json::to_value(self.incoming()).unwrap_or_default());
    }

    fn save_policy(&self) {
        let text = serde_json::to_string(&*self.inner.policy.lock()).unwrap_or_default();
        let _ = self.inner.db.write(|c| kv_set(c, POLICY_KEY, &text));
        self.inner.host.emit(EVENT_MINE, serde_json::json!({}));
    }

    fn save_sent(&self) {
        let text = serde_json::to_string(&*self.inner.sent.lock()).unwrap_or_default();
        let _ = self.inner.db.write(|c| kv_set(c, SENT_KEY, &text));
    }

    fn since(&self, key: &str) -> Option<String> {
        self.inner.db.read(|c| kv_get(c, &format!("{SINCE_KEY}.{key}"))).ok().flatten()
    }

    fn set_since(&self, key: &str, id: &str) {
        let _ = self.inner.db.write(|c| kv_set(c, &format!("{SINCE_KEY}.{key}"), id));
    }

    /// Drop settled requests older than a month.
    fn prune(&self) {
        let now = self.now();
        let old = |at: u64| now.saturating_sub(at) > KEEP_MS;
        self.inner.mine.lock().retain(|a| !(a.state.settled() && old(a.updated_at)));
        self.inner.incoming.lock().retain(|a| !(a.state.settled() && old(a.updated_at)));
        self.inner.sent.lock().retain(|(_, at)| now.saturating_sub(*at) < WEEK_MS);
    }

    // ---- the relay ----------------------------------------------------------------

    fn relay_base(&self) -> String {
        relay_url(&self.inner.relay.read()).unwrap_or_else(|| self.inner.default_relay.clone())
    }

    async fn publish(&self, topic: &str, body: String) -> bool {
        let relay = self.inner.live.lock().relay.clone();
        let Some(relay) = relay else { return false };
        match relay.publish(topic, &body).await {
            Ok(()) => true,
            Err(e) => {
                tracing::info!(?e, "requests: the relay didn't take a message");
                false
            }
        }
    }

    async fn tell(&self, user_id: &str, m: &Down) {
        let pair = self.inner.live.lock().people.get(user_id).map(|(_, p)| p.clone());
        let Some(pair) = pair else { return };
        match wire::seal_down(&pair, m) {
            Ok(body) => {
                self.publish(&pair.down.topic, body).await;
            }
            Err(e) => tracing::warn!(kind = e.kind(), "requests: couldn't seal an answer"),
        }
    }

    async fn send_up(&self, m: &Up) -> bool {
        let pair = self.inner.live.lock().mine.as_ref().map(|(_, p)| p.clone());
        let Some(pair) = pair else { return false };
        match wire::seal_up(&pair, m) {
            Ok(body) => self.publish(&pair.up.topic, body).await,
            Err(_) => false,
        }
    }

    // ---- the loop ----------------------------------------------------------------

    /// Runs for the app's life: listens as the owner or as a guest, whichever
    /// the sign-in is, and switches when that changes.
    pub async fn run(self) {
        let mut backoff = Duration::from_secs(2);
        loop {
            let role = self.inner.host.role().await;
            let ok = match &role {
                Role::Nobody => {
                    self.inner.away.store(true, Ordering::SeqCst);
                    self.idle();
                    tokio::select! {
                        _ = self.inner.wake.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                    }
                    continue;
                }
                Role::Owner { machine_id } => {
                    let ok = self.run_owner(machine_id.clone(), &role).await;
                    if !ok {
                        self.inner.away.store(true, Ordering::SeqCst);
                    }
                    ok
                }
                Role::Guest { machine_id, token } => {
                    self.inner.away.store(true, Ordering::SeqCst);
                    self.run_guest(machine_id.clone(), token.clone(), &role).await
                }
            };
            self.idle();
            if ok {
                backoff = Duration::from_secs(2);
            } else {
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(120));
            }
        }
    }

    fn idle(&self) {
        *self.inner.live.lock() = Live::default();
    }

    /// The owner's side, until the sign-in changes, the people are read
    /// again, or the relay drops (false).
    async fn run_owner(&self, machine: String, role: &Role) -> bool {
        let people = match self.inner.host.people().await {
            Ok(p) => p,
            Err(e) => {
                tracing::info!(kind = e.kind(), "requests: who the server is shared with isn't known now");
                return false;
            }
        };
        let relay = self.inner.relays.relay(&self.relay_base());
        let mut by_topic: HashMap<String, (String, bool)> = HashMap::new();
        {
            let mut live = self.inner.live.lock();
            live.relay = Some(relay.clone());
            live.people.clear();
            for p in people {
                let pair = Pair::new(&p.token, &machine);
                by_topic.insert(pair.up.topic.clone(), (p.user_id.clone(), true));
                by_topic.insert(pair.down.topic.clone(), (p.user_id.clone(), false));
                live.people.insert(p.user_id, (p.name, pair));
            }
        }
        if by_topic.is_empty() {
            // Shared with nobody: nothing to listen for until that changes.
            tokio::select! {
                _ = self.inner.wake.notified() => {}
                _ = tokio::time::sleep(PEOPLE_EVERY) => {}
            }
            return true;
        }
        let topics: Vec<String> = by_topic.keys().cloned().collect();
        if self.inner.away.swap(false, Ordering::SeqCst) {
            self.inner.listening_since.store(self.now(), Ordering::SeqCst);
        }
        let since_key = format!("owner.{machine}");
        let since = self.since(&since_key).unwrap_or_else(|| "12h".into());
        let mut rx = match relay.subscribe(&topics, Some(&since)).await {
            Ok(rx) => rx,
            Err(e) => {
                tracing::info!(?e, "requests: the relay can't be reached");
                return false;
            }
        };
        tracing::info!(people = topics.len() / 2, "requests: listening for the people the server is shared with");
        self.tell_rules_if_changed().await;
        let started = tokio::time::Instant::now();
        let mut follow = tokio::time::interval(FOLLOW_EVERY);
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Some(RelayEvent::Message { id, topic, body }) => {
                        self.set_since(&since_key, &id);
                        let Some((user, up)) = by_topic.get(&topic).cloned() else { continue };
                        let pair = self.inner.live.lock().people.get(&user).cloned();
                        let Some((name, pair)) = pair else { continue };
                        if up {
                            match wire::open_up(&pair, &body) {
                                Ok(m) if self.allow(&user) => self.on_up(&user, &name, m).await,
                                Ok(_) => tracing::info!("requests: one person is sending too much; dropped"),
                                Err(r) => tracing::info!(?r, "requests: dropped a message that didn't open"),
                            }
                        } else if let Ok(m) = wire::open_down(&pair, &body) {
                            self.on_answered_elsewhere(&user, m);
                        }
                    }
                    Some(RelayEvent::Open) => {}
                    None => return false,
                },
                _ = follow.tick() => {
                    self.follow().await;
                    if started.elapsed() >= PEOPLE_EVERY || self.inner.host.role().await != *role {
                        return true;
                    }
                }
                // Sign-in, sharing or settings changed: read the people again
                // and tell them their rules if those changed.
                _ = self.inner.wake.notified() => return true,
            }
        }
    }

    /// The asking side, until the sign-in changes or the relay drops.
    async fn run_guest(&self, machine: String, token: Token, role: &Role) -> bool {
        let pair = Pair::new(&token, &machine);
        let relay = self.inner.relays.relay(&self.relay_base());
        {
            let mut live = self.inner.live.lock();
            live.relay = Some(relay.clone());
            live.mine = Some((machine.clone(), pair.clone()));
        }
        let since_key = format!("guest.{machine}");
        let since = self.since(&since_key).unwrap_or_else(|| "12h".into());
        let mut rx = match relay.subscribe(std::slice::from_ref(&pair.down.topic), Some(&since)).await {
            Ok(rx) => rx,
            Err(e) => {
                tracing::info!(?e, "requests: the relay can't be reached");
                return false;
            }
        };
        self.send_up(&Up::Hello { at: self.now() }).await;
        self.resend(&machine, true).await;
        let mut tick = tokio::time::interval(RESEND_EVERY);
        tick.tick().await;
        let mut ticks = 0u32;
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Some(RelayEvent::Message { id, body, .. }) => {
                        self.set_since(&since_key, &id);
                        match wire::open_down(&pair, &body) {
                            Ok(m) => self.on_down(&machine, m),
                            Err(r) => tracing::info!(?r, "requests: dropped an answer that didn't open"),
                        }
                    }
                    Some(RelayEvent::Open) => {}
                    None => return false,
                },
                _ = tick.tick() => {
                    if self.inner.host.role().await != *role {
                        return true;
                    }
                    ticks += 1;
                    self.resend(&machine, ticks.is_multiple_of(CHECK_EVERY)).await;
                }
                _ = self.inner.wake.notified() => {
                    if self.inner.host.role().await != *role {
                        return true;
                    }
                    self.resend(&machine, false).await;
                }
            }
        }
    }

    // ---- the asking side ------------------------------------------------------------

    /// Send again whatever the owner's Platter hasn't seen, and with `check`
    /// the ones still open too (the owner's Platter answers with where each
    /// stands, so an answer missed while away arrives).
    async fn resend(&self, machine: &str, check: bool) {
        let again: Vec<MyAsk> = self
            .inner
            .mine
            .lock()
            .iter()
            .filter(|a| a.server == machine && ((!a.seen && a.state == AskState::Sending) || (check && a.state.open())))
            .cloned()
            .collect();
        let at = self.now();
        for a in again {
            self.send_up(&Up::Ask { id: a.id.clone(), item: AskItem { thumb: None, ..a.item }, at }).await;
        }
    }

    fn on_down(&self, machine: &str, m: Down) {
        match m {
            Down::Policy { can_ask, straight_away, at } => {
                let mut changed = false;
                {
                    let mut p = self.inner.policy.lock();
                    let old = p.get(machine).copied().unwrap_or_default();
                    if at >= old.at {
                        changed = old.can_ask != can_ask || old.straight_away != straight_away || old.at == 0;
                        p.insert(machine.to_string(), Policy { can_ask, straight_away, at });
                    }
                }
                if changed {
                    self.save_policy();
                }
            }
            Down::Status { id, state, progress, note, at } => {
                let arrived = {
                    let mut mine = self.inner.mine.lock();
                    let Some(a) = mine.iter_mut().find(|a| a.id == id && a.server == machine) else { return };
                    if at < a.status_at || a.state == AskState::Cancelled {
                        return;
                    }
                    let was = a.state;
                    a.seen = true;
                    a.status_at = at;
                    a.state = state;
                    a.progress = progress;
                    a.note = note;
                    a.updated_at = self.now();
                    (was != AskState::Ready && state == AskState::Ready).then(|| a.item.album_title().to_string())
                };
                self.save_mine();
                if let Some(title) = arrived {
                    tracing::info!("requests: something asked for arrived");
                    self.inner.host.notify(&format!("{title} is here."), "It's on the server now. Open Platter to play it.");
                    self.inner.host.refresh_library();
                }
            }
        }
    }

    /// The asking side for the current server.
    pub async fn view(&self) -> AsksView {
        let role = self.inner.host.role().await;
        let Role::Guest { machine_id, .. } = role else {
            return AsksView { guest: false, can_request: false, straight_away: false, mine: Vec::new() };
        };
        let p = self.inner.policy.lock().get(&machine_id).copied().unwrap_or_default();
        let mut mine: Vec<MyAsk> = self.inner.mine.lock().iter().filter(|a| a.server == machine_id && a.state != AskState::Cancelled).cloned().collect();
        mine.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        AsksView { guest: true, can_request: p.can_ask, straight_away: p.can_ask && p.straight_away, mine }
    }

    /// Request something of the server's owner. A second request for the
    /// same album is the first one.
    pub async fn request(&self, item: AskItem) -> AppResult<MyAsk> {
        let Role::Guest { machine_id, .. } = self.inner.host.role().await else {
            return Err(AppError::Invalid("Requests are for music on a server someone shares with you.".into()));
        };
        if !item.sane() {
            return Err(AppError::Invalid("That can't be requested.".into()));
        }
        if self.inner.policy.lock().get(&machine_id).is_some_and(|p| p.at > 0 && !p.can_ask) {
            return Err(AppError::Invalid(OFF_NOTE.into()));
        }
        let rg = item.release_group_id.as_deref().map(|r| r.trim().to_ascii_lowercase());
        let (album, artist) = (fold(item.album_title()), fold(&item.artist));
        let now = self.now();
        let ask = {
            let mut mine = self.inner.mine.lock();
            let same = |a: &MyAsk| {
                a.server == machine_id
                    && (a.state.open() || (a.state == AskState::Ready && now.saturating_sub(a.updated_at) < READY_DEDUPE_MS))
                    && (rg.as_deref().is_some_and(|r| a.item.release_group_id.as_deref().map(str::to_ascii_lowercase).as_deref() == Some(r))
                        || (fold(a.item.album_title()) == album && fold(&a.item.artist) == artist))
            };
            if let Some(a) = mine.iter().find(|a| same(a)) {
                return Ok(a.clone());
            }
            if mine.iter().filter(|a| a.server == machine_id && matches!(a.state, AskState::Sending | AskState::Requested)).count() >= OPEN_MAX {
                return Err(AppError::Invalid(FULL_NOTE.into()));
            }
            let a = MyAsk {
                id: uuid::Uuid::new_v4().to_string(),
                server: machine_id.clone(),
                item: AskItem { release_group_id: rg, ..item },
                state: AskState::Sending,
                progress: None,
                note: None,
                created_at: now,
                updated_at: now,
                seen: false,
                status_at: 0,
            };
            mine.push(a.clone());
            a
        };
        self.prune();
        self.save_mine();
        tracing::info!("requests: asked the owner for something");
        self.send_up(&Up::Ask { id: ask.id.clone(), item: AskItem { thumb: None, ..ask.item.clone() }, at: ask.created_at }).await;
        Ok(ask)
    }

    /// Take a request back before the owner said yes.
    pub async fn cancel(&self, id: &str) -> AppResult<()> {
        let at = self.now();
        {
            let mut mine = self.inner.mine.lock();
            let Some(a) = mine.iter_mut().find(|a| a.id == id) else { return Ok(()) };
            if !matches!(a.state, AskState::Sending | AskState::Requested | AskState::NotFound | AskState::Declined | AskState::Refused | AskState::Failed) {
                return Err(AppError::Invalid("It's already on its way, so it can't be taken back.".into()));
            }
            a.state = AskState::Cancelled;
            a.updated_at = at;
        }
        self.save_mine();
        self.send_up(&Up::Cancel { id: id.to_string(), at }).await;
        Ok(())
    }

    // ---- the owner's side ------------------------------------------------------------

    /// Requests made of me, newest first: the open ones and the last month's.
    pub fn incoming(&self) -> Vec<Incoming> {
        let mut v = self.inner.incoming.lock().clone();
        v.retain(|a| a.state != AskState::Cancelled);
        v.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        v
    }

    fn policy_for(&self, user_id: &str) -> Down {
        let s = self.inner.host.settings();
        let usable = self.inner.host.lidarr_usable();
        let rule = s.rule(user_id);
        Down::Policy { can_ask: usable && rule != Rule::Off, straight_away: usable && rule == Rule::Straight, at: self.now() }
    }

    /// Tell everyone their rule when it changed since they were last told
    /// (and once when this Platter starts, so a person who said hello while
    /// it was off hears).
    async fn tell_rules_if_changed(&self) {
        if !self.inner.host.lidarr_usable() {
            return;
        }
        let s = self.inner.host.settings();
        let now = format!("{}|{:?}|{}", s.on, s.rules, self.inner.host.lidarr_usable());
        {
            let mut told = self.inner.told.lock();
            if told.as_deref() == Some(now.as_str()) {
                return;
            }
            *told = Some(now);
        }
        let people: Vec<String> = self.inner.live.lock().people.keys().cloned().collect();
        for u in people {
            let p = self.policy_for(&u);
            self.tell(&u, &p).await;
        }
    }

    fn status_of(a: &Incoming, at: u64) -> Down {
        Down::Status { id: a.id.clone(), state: a.state, progress: a.progress, note: a.note.clone(), at }
    }

    async fn tell_status(&self, a: &Incoming) {
        let m = Self::status_of(a, self.now());
        self.tell(&a.person_id, &m).await;
    }

    /// One person's messages are answered up to a limit an hour, so nobody
    /// can use up the relay's allowance for everyone else.
    fn allow(&self, user: &str) -> bool {
        let now = self.now();
        let mut heard = self.inner.heard.lock();
        let list = heard.entry(user.to_string()).or_default();
        list.retain(|at| now.saturating_sub(*at) < 3600 * 1000);
        if list.len() >= HEARD_PER_HOUR {
            return false;
        }
        list.push(now);
        true
    }

    async fn on_up(&self, user: &str, name: &str, m: Up) {
        match m {
            // Only a device that can get music answers; another of the
            // owner's devices that can't stays quiet rather than saying no.
            Up::Hello { .. } if self.inner.host.lidarr_usable() => {
                let p = self.policy_for(user);
                self.tell(user, &p).await;
            }
            Up::Hello { .. } => {}
            Up::Cancel { id, .. } => {
                let found = {
                    let mut l = self.inner.incoming.lock();
                    l.iter_mut().find(|a| a.id == id && a.person_id == user && a.state == AskState::Requested).map(|a| {
                        a.state = AskState::Cancelled;
                        a.updated_at = self.now();
                        a.clone()
                    })
                };
                if let Some(a) = found {
                    self.save_incoming();
                    self.tell_status(&a).await;
                }
            }
            Up::Ask { id, item, at } => {
                let since = self.inner.listening_since.load(Ordering::SeqCst);
                let stale = at.saturating_add(self.inner.stale_ms.load(Ordering::SeqCst)) < since;
                self.on_ask(user, name, id, item, stale).await
            }
        }
    }

    /// Someone asked for something. A `stale` one (the relay catching this
    /// device up) or one arriving where Lidarr can't be used is only listed:
    /// the device that can get it answers.
    async fn on_ask(&self, user: &str, name: &str, id: String, item: AskItem, stale: bool) {
        // Seen before (sent again, or from the relay's memory): say where it stands.
        let known = self.inner.incoming.lock().iter().find(|a| a.id == id).cloned();
        if let Some(a) = known {
            if a.person_id == user {
                self.tell_status(&a).await;
            }
            return;
        }
        let settings = self.inner.host.settings();
        let rule = settings.rule(user);
        let now = self.now();
        // Nothing they send is fetched: the cover comes from the release group.
        let item = AskItem { thumb: None, ..item };
        let mut a = Incoming {
            id: id.clone(),
            person_id: user.to_string(),
            person_name: name.to_string(),
            item: item.clone(),
            release_group_id: None,
            state: AskState::Requested,
            progress: None,
            note: None,
            created_at: now,
            updated_at: now,
            by_itself: false,
        };
        let refuse = |a: &mut Incoming, note: &str| {
            a.state = AskState::Refused;
            a.note = Some(note.to_string());
        };
        let quiet = stale || !self.inner.host.lidarr_usable();
        if !item.sane() {
            refuse(&mut a, "That can't be requested.");
        } else if rule == Rule::Off {
            refuse(&mut a, OFF_NOTE);
        } else if self.inner.incoming.lock().iter().filter(|x| x.person_id == user && x.state == AskState::Requested).count() >= OPEN_MAX {
            refuse(&mut a, FULL_NOTE);
        }
        if a.state == AskState::Requested {
            match self.inner.host.pick(&item).await {
                Ok(rg) => {
                    a.item.thumb = crate::lidarr::request::cover_thumb(&rg);
                    a.release_group_id = Some(rg);
                }
                Err(why) => refuse(&mut a, &why),
            }
        }
        // Coming already (asked for by the owner or someone else): it joins.
        if let Some(r) = a.release_group_id.as_deref().and_then(|rg| self.inner.host.record(rg)).filter(|r| r.state != RequestState::Failed) {
            let (state, progress) = from_record(&r);
            a.state = state;
            a.progress = progress;
            a.by_itself = true;
        }
        if a.state == AskState::Refused {
            // Not kept: the person hears why, and the owner's list stays theirs.
            tracing::info!(note = a.note.as_deref().unwrap_or_default(), "requests: a request couldn't be taken");
            if !quiet {
                self.tell_status(&a).await;
            }
            return;
        }
        let waiting = a.state == AskState::Requested;
        self.inner.incoming.lock().push(a.clone());
        self.prune();
        self.save_incoming();
        if quiet {
            return;
        }
        if waiting && rule == Rule::Straight {
            let used = self.inner.sent.lock().iter().filter(|(u, at)| u == user && now.saturating_sub(*at) < WEEK_MS).count();
            if used < settings.weekly_limit as usize {
                match self.approve(&id, true).await {
                    Ok(done) if matches!(done.incoming.state, AskState::Getting | AskState::Ready | AskState::NotFound) => {
                        self.inner.sent.lock().push((user.to_string(), now));
                        self.save_sent();
                        return;
                    }
                    Ok(done) if done.incoming.state != AskState::Requested => return,
                    Ok(_) => {}
                    Err(e) => tracing::info!(kind = e.kind(), "requests: a straight-away request waits for a yes"),
                }
            } else {
                self.set_note(&id, OVER_LIMIT_NOTE);
            }
        }
        let a = self.inner.incoming.lock().iter().find(|x| x.id == id).cloned().unwrap_or(a);
        self.tell_status(&a).await;
        if a.state == AskState::Requested {
            self.notice(user, name, &a.item);
        }
    }

    fn set_note(&self, id: &str, note: &str) {
        if let Some(a) = self.inner.incoming.lock().iter_mut().find(|a| a.id == id) {
            a.note = Some(note.to_string());
        }
        self.save_incoming();
    }

    /// The owner's notification, once per person every few minutes when
    /// grouping is on.
    fn notice(&self, user: &str, name: &str, item: &AskItem) {
        let now = self.now();
        if self.inner.host.settings().group_notices {
            let mut n = self.inner.noticed.lock();
            if n.get(user).is_some_and(|at| now.saturating_sub(*at) < GROUP_MS) {
                return;
            }
            n.insert(user.to_string(), now);
        }
        self.inner.host.notify(&format!("{name} requested {}", item.album_title()), &format!("by {}. Open Platter to get it or say no.", item.artist));
    }

    /// The owner said yes or no.
    pub async fn answer(&self, id: &str, yes: bool) -> AppResult<Answered> {
        if !matches!(self.inner.host.role().await, Role::Owner { .. }) {
            return Err(AppError::Invalid("Only the server's owner answers requests.".into()));
        }
        if yes {
            return self.approve(id, false).await;
        }
        let a = {
            let mut l = self.inner.incoming.lock();
            let a = l.iter_mut().find(|a| a.id == id).ok_or_else(|| AppError::NotFound("That request is gone.".into()))?;
            if a.state == AskState::Requested {
                a.state = AskState::Declined;
                a.note = None;
                a.updated_at = self.now();
            }
            a.clone()
        };
        self.save_incoming();
        tracing::info!("requests: said no to a request");
        self.tell_status(&a).await;
        Ok(Answered { incoming: a, needs: None })
    }

    async fn approve(&self, id: &str, by_itself: bool) -> AppResult<Answered> {
        let a = self.inner.incoming.lock().iter().find(|a| a.id == id).cloned().ok_or_else(|| AppError::NotFound("That request is gone.".into()))?;
        if a.state != AskState::Requested && a.state != AskState::NotFound {
            return Ok(Answered { incoming: a, needs: None });
        }
        let Some(rg) = a.release_group_id.clone() else {
            return Err(AppError::Invalid("Platter doesn't know which album this is.".into()));
        };
        let input = RequestInput {
            release_group_id: rg,
            title: a.item.album_title().to_string(),
            artist: a.item.artist.clone(),
            artist_id: None,
            artist_mbid: a.item.artist_mbid.clone(),
            typed: None,
        };
        let mut needs = None;
        let protected = self.inner.host.protected(&input).await;
        let Some(protected) = protected else {
            // Can't tell whether it's kept by hand: it waits rather than going.
            return Err(AppError::Network("Lidarr can't be reached right now, so it's still waiting. Try again in a moment.".into()));
        };
        let (state, progress, note) = if protected {
            (AskState::Refused, None, Some(PROTECTED_NOTE.to_string()))
        } else {
            tracing::info!(by_itself, "requests: sending a request through Get");
            let out = self.inner.host.get(input).await?;
            match (out.record, out.needs) {
                (Some(r), _) => {
                    let (s, p) = from_record(&r);
                    (s, p, None)
                }
                (None, Some(n)) if n.kind == "firstTime" => {
                    let note = by_itself.then(|| FIRST_YES_NOTE.to_string());
                    needs = Some(n);
                    (AskState::Requested, None, note)
                }
                (None, Some(n)) => {
                    let note = Some(n.reason.clone());
                    needs = Some(n);
                    (AskState::Refused, None, note)
                }
                (None, None) => (AskState::Failed, None, Some(FAILED_NOTE.to_string())),
            }
        };
        let a = {
            let mut l = self.inner.incoming.lock();
            let Some(x) = l.iter_mut().find(|x| x.id == id) else { return Err(AppError::NotFound("That request is gone.".into())) };
            x.state = state;
            x.progress = progress;
            x.note = note;
            x.by_itself = by_itself;
            x.updated_at = self.now();
            x.clone()
        };
        self.save_incoming();
        if a.state != AskState::Requested {
            self.tell_status(&a).await;
        }
        Ok(Answered { incoming: a, needs })
    }

    /// Follow what Lidarr is getting and tell the person when it changes.
    async fn follow(&self) {
        let following: Vec<Incoming> = self.inner.incoming.lock().iter().filter(|a| matches!(a.state, AskState::Getting | AskState::NotFound)).cloned().collect();
        for a in following {
            let Some(r) = a.release_group_id.as_deref().and_then(|rg| self.inner.host.record(rg)) else { continue };
            let (state, progress) = from_record(&r);
            if state == a.state && progress == a.progress {
                continue;
            }
            let note = (state == AskState::Failed).then(|| FAILED_NOTE.to_string());
            let changed = {
                let mut l = self.inner.incoming.lock();
                l.iter_mut().find(|x| x.id == a.id).map(|x| {
                    x.state = state;
                    x.progress = progress;
                    x.note = note;
                    x.updated_at = self.now();
                    x.clone()
                })
            };
            if let Some(x) = changed {
                self.save_incoming();
                self.tell_status(&x).await;
            }
        }
    }

    /// Another of the owner's devices answered (or this one, coming back
    /// from the relay): take the answer over.
    fn on_answered_elsewhere(&self, user: &str, m: Down) {
        let Down::Status { id, state, progress, note, .. } = m else { return };
        let changed = {
            let mut l = self.inner.incoming.lock();
            match l.iter_mut().find(|a| a.id == id && a.person_id == user) {
                Some(a) if a.state == AskState::Requested && state != AskState::Requested => {
                    a.state = state;
                    a.progress = progress;
                    a.note = note;
                    a.updated_at = self.now();
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.save_incoming();
        }
    }
}
