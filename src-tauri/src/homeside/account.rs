//! Links by Plex account, with no codes. Every Platter signed in to a Plex
//! account (the app on a computer or phone, a Bridge signed in through
//! plex.tv/link) finds the account's other Platters in its device list on
//! plex.tv and links to each of them by itself.
//!
//! plex.tv lists every device with its own token, and only to the account
//! (or Home user) it belongs to (docs/PLEX-API.md). Both ends of a link work
//! out the same secret from the two devices' keys (a hash of each token, as
//! in `crate::remote::peer::auth`) and their ids, so nothing is exchanged to
//! set a link up and no token ever leaves a device. A device of another
//! account, a Home user or a friend the server is shared with isn't in the
//! list and can't work the secret out. A device signed out on plex.tv drops
//! out of the list, and its link goes at the next look.
//!
//! Past that a link is the same as one paired with a code: sealed frames
//! through the relay. A link paired with a code keeps working, and a device
//! it already reaches isn't linked twice.
//!
//! Each Platter also listens on a knock topic of its own (from its key), so
//! one that just linked can ask the other to look at the list now instead
//! of at its next look. Only the account's devices know the key, so nobody
//! else can find the topic.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, mpsc, watch};

use super::link::{Link, Side};
use super::links::Paired;
use super::peer::Clock;
use crate::error::{AppError, AppResult};
use crate::plex::Token;
use crate::remote::peer::auth::{PeerKey, hex, hmac_sha256};
use crate::social::listen::relay::{RelayEvent, RelayFactory};
use crate::social::listen::seal::{self, Channel};

/// How often each Platter looks at the account's device list.
pub const LOOK_EVERY: Duration = Duration::from_secs(10 * 60);
/// How often to check whether this Platter signed in (no network).
pub const SIGN_IN_CHECK: Duration = Duration::from_secs(20);
/// At most one look this often, however many knocks come.
pub const LOOK_GAP: Duration = Duration::from_secs(15);
/// Keys kept per device (plex.tv can list one device twice).
const KEYS_MAX: usize = 3;
/// Devices taken from one list.
const DEVICES_MAX: usize = 200;
/// How long a knock counts.
const KNOCK_TTL_MS: u64 = 10 * 60_000;
/// A new link reads its inbox from this long before it was made, so what
/// the other end sent first (it may have linked a little earlier) arrives.
pub const BACKLOG_S: i64 = 30 * 60;
/// A code-paired Bridge not heard from in this long gives way to a Bridge
/// linked by account that answers (it lost its half of the pairing, say).
pub const QUIET_S: i64 = 150 * 60;

/// What plex.tv calls this app.
pub const PRODUCT: &str = crate::plex::identity::PRODUCT;
/// The device name a Bridge gives plex.tv.
pub const BRIDGE_DEVICE: &str = "Platter Bridge";

/// One device in the account's list.
#[derive(Clone, PartialEq)]
pub struct AccountDevice {
    /// Its Plex client id.
    pub id: String,
    pub name: String,
    pub product: String,
    pub platform: String,
    pub device: String,
    /// Its tokens' keys (never the tokens).
    pub keys: Vec<PeerKey>,
}

impl std::fmt::Debug for AccountDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountDevice").field("product", &self.product).field("platform", &self.platform).finish_non_exhaustive()
    }
}

impl AccountDevice {
    pub fn is_platter(&self) -> bool {
        self.product == PRODUCT
    }

    /// A Bridge: Platter on Linux (the app itself ships for Windows, macOS
    /// and iOS), or one that says so.
    pub fn is_bridge(&self) -> bool {
        self.is_platter() && (self.device == BRIDGE_DEVICE || self.platform.eq_ignore_ascii_case("linux"))
    }
}

/// Read `/api/v2/devices` (JSON). Entries without an id or a token are left
/// out; one device listed twice is one entry with both keys.
pub fn parse_list(body: &[u8]) -> AppResult<Vec<AccountDevice>> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| AppError::Invalid("unreadable device list".into()))?;
    let list = v.as_array().ok_or_else(|| AppError::Invalid("unreadable device list".into()))?;
    let mut out: Vec<AccountDevice> = Vec::new();
    let text = |d: &serde_json::Value, k: &str| d[k].as_str().unwrap_or_default().chars().take(100).collect::<String>();
    for d in list {
        let (Some(id), Some(token)) = (d["clientIdentifier"].as_str(), d["token"].as_str()) else { continue };
        let Ok(token) = Token::new(token) else { continue };
        if id.is_empty() || id.len() > 100 {
            continue;
        }
        let key = PeerKey::from_token(&token);
        if let Some(e) = out.iter_mut().find(|e| e.id == id) {
            if !e.keys.contains(&key) && e.keys.len() < KEYS_MAX {
                e.keys.push(key);
            }
            continue;
        }
        if out.len() >= DEVICES_MAX {
            break;
        }
        out.push(AccountDevice {
            id: id.to_string(),
            name: text(d, "name"),
            product: text(d, "product"),
            platform: text(d, "platform"),
            device: text(d, "device"),
            keys: vec![key],
        });
    }
    Ok(out)
}

/// This Platter, as the account knows it.
#[derive(Clone)]
pub struct Me {
    /// Its Plex client id.
    pub id: String,
    /// The key of the token it's signed in with.
    pub key: PeerKey,
    pub bridge: bool,
}

/// Which list a link goes in: to a Bridge, or between two of the user's
/// own devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bridge,
    Device,
}

/// The link between this Platter and another one on the account, the same
/// at both ends.
pub fn account_link(kind: Kind, me: &Me, peer_id: &str, peer_key: &PeerKey, relay: &str) -> Link {
    let mine = (me.key.bytes(), me.id.as_str());
    let theirs = (peer_key.bytes(), peer_id);
    let (lo, hi) = if mine <= theirs { (mine, theirs) } else { (theirs, mine) };
    let label = match kind {
        Kind::Bridge => "bridge",
        Kind::Device => "device",
    };
    let base = [lo.0.as_slice(), hi.0.as_slice()].concat();
    let secret = hmac_sha256(&base, format!("platter-account-link-v1\n{label}\n{}\n{}", lo.1, hi.1).as_bytes());
    let id = hex(&seal::derive(&secret, "account link id")[..8]);
    let frame_id = |dev: &str| hex(&seal::derive(&secret, &format!("account frame {dev}"))[..8]);
    let side = match kind {
        Kind::Device => Side::Device,
        Kind::Bridge if me.bridge => Side::Bridge,
        Kind::Bridge => Side::Desktop,
    };
    Link { id, secret, relay: relay.to_string(), side, me: frame_id(&me.id), peer: frame_id(peer_id) }
}

/// A link to make.
#[derive(Clone)]
pub struct Wanted {
    pub link: Link,
    /// The other device's Plex client id.
    pub device: String,
    pub name: String,
    /// Its key, to knock with.
    pub key: PeerKey,
}

/// What to change in one list of links.
#[derive(Default)]
pub struct Plan {
    pub add: Vec<Wanted>,
    /// Account links whose device left the list (or signed in again).
    pub drop: Vec<String>,
}

/// Bring one list of links in line with the account's device list. None
/// when the list doesn't have this Platter under the key it's signed in
/// with yet (just signed in, say): then nothing changes, since the others
/// couldn't work out the same links. A device reached by a code-paired link
/// that `works` isn't linked again.
pub fn plan(kind: Kind, me: &Me, list: &[AccountDevice], have: &[Paired], apart: &[String], relay: &str, works: impl Fn(&Paired) -> bool) -> Option<Plan> {
    plan_heard(kind, me, list, have, apart, relay, works, |_| false)
}

/// [`plan`], where `heard` says an account link was ever heard from. A
/// device listed with more than one sign-in answers only on its current
/// one, so once a link to it was heard from, its other links go.
#[allow(clippy::too_many_arguments)]
pub fn plan_heard(
    kind: Kind,
    me: &Me,
    list: &[AccountDevice],
    have: &[Paired],
    apart: &[String],
    relay: &str,
    works: impl Fn(&Paired) -> bool,
    heard: impl Fn(&Paired) -> bool,
) -> Option<Plan> {
    if !list.iter().any(|d| d.id == me.id && d.keys.contains(&me.key)) {
        return None;
    }
    if me.bridge && kind == Kind::Device {
        return Some(Plan::default());
    }
    let reached: HashSet<&str> = have.iter().filter(|p| !p.auto && works(p)).filter_map(|p| p.device.as_deref()).collect();
    let mut wanted: Vec<Wanted> = Vec::new();
    for d in list {
        let fits = match kind {
            _ if !d.is_platter() || d.id == me.id => false,
            // A Bridge links to computers and phones, never another Bridge.
            Kind::Bridge if me.bridge => !d.is_bridge(),
            Kind::Bridge => d.is_bridge(),
            Kind::Device => !d.is_bridge(),
        };
        if !fits || reached.contains(d.id.as_str()) {
            continue;
        }
        let mine: Vec<Wanted> = d
            .keys
            .iter()
            .map(|key| Wanted { link: account_link(kind, me, &d.id, key, relay), device: d.id.clone(), name: d.name.clone(), key: key.clone() })
            .filter(|w| !apart.contains(&w.link.id))
            .collect();
        let answered: HashSet<&str> = have.iter().filter(|p| p.auto && heard(p)).map(|p| p.id.as_str()).collect();
        if mine.iter().any(|w| answered.contains(w.link.id.as_str())) {
            wanted.extend(mine.into_iter().filter(|w| answered.contains(w.link.id.as_str())));
        } else {
            wanted.extend(mine);
        }
    }
    let had: HashSet<&str> = have.iter().map(|p| p.id.as_str()).collect();
    let keep: HashSet<String> = wanted.iter().map(|w| w.link.id.clone()).collect();
    Some(Plan {
        drop: have.iter().filter(|p| p.auto && !keep.contains(&p.id)).map(|p| p.id.clone()).collect(),
        add: wanted.into_iter().filter(|w| !had.contains(w.link.id.as_str())).collect(),
    })
}

/// The account owns the server (`machine`, or any server when it isn't
/// known yet). Only the owner's devices are linked to a Bridge, since
/// through it they reach Lidarr.
pub fn owns_server(servers: &[crate::plex::discovery::DiscoveredServer], machine: Option<&str>) -> bool {
    servers.iter().any(|s| s.owned && machine.is_none_or(|m| s.machine_id == m))
}

/// The list as a Platter that may not link to Bridges sees it: Bridges
/// left out, so account links to them drop and none are made.
pub fn without_bridges(list: &[AccountDevice]) -> Vec<AccountDevice> {
    list.iter().filter(|d| !d.is_bridge()).cloned().collect()
}

/// The list as a Bridge whose account doesn't own the server sees it: only
/// Bridges (itself), so it links to no device and drops what it had.
pub fn only_bridges(list: &[AccountDevice]) -> Vec<AccountDevice> {
    list.iter().filter(|d| d.is_bridge()).cloned().collect()
}

// ---------------------------------------------------------------- knocks

#[derive(Serialize, Deserialize)]
struct Knock {
    /// Unix ms after which it no longer counts.
    x: u64,
}

/// Where a Platter with this key hears knocks.
pub fn knock_channel(key: &PeerKey) -> Channel {
    Channel::new(key.bytes(), "account knock")
}

pub fn knock_body(key: &PeerKey, now_ms: u64) -> AppResult<String> {
    seal::seal_value(&knock_channel(key), &Knock { x: now_ms + KNOCK_TTL_MS })
}

pub fn is_knock(key: &PeerKey, body: &str, now_ms: u64) -> bool {
    seal::open_value::<Knock>(&knock_channel(key), body).is_ok_and(|k| k.x >= now_ms && k.x <= now_ms + 2 * KNOCK_TTL_MS)
}

// ---------------------------------------------------------------- looking

/// One end's side of looking: the app (Bridges and devices) or a Bridge.
#[async_trait]
pub trait AccountSide: Send + Sync {
    /// This Platter on the account and its token, while signed in.
    async fn me(&self) -> Option<(Me, Token)>;
    /// The account's device list.
    async fn list(&self, token: &Token) -> AppResult<Vec<AccountDevice>>;
    /// Bring the links in line with the list: the keys of the devices
    /// newly linked (they're knocked).
    async fn apply(&self, me: &Me, list: &[AccountDevice]) -> Vec<PeerKey>;
}

/// Looks at the account's device list every few minutes, when asked, and
/// when knocked, and keeps the links in line with it.
pub struct Looker {
    side: Arc<dyn AccountSide>,
    relays: Arc<dyn RelayFactory>,
    relay: String,
    clock: Clock,
    now: Notify,
    last: Mutex<Option<tokio::time::Instant>>,
}

impl Looker {
    pub fn new(side: Arc<dyn AccountSide>, relays: Arc<dyn RelayFactory>, relay: &str, clock: Clock) -> Arc<Self> {
        Arc::new(Self { side, relays, relay: relay.to_string(), clock, now: Notify::new(), last: Mutex::default() })
    }

    /// Look soon (after a sign-in, a network change, Settings opening).
    pub fn look_now(&self) {
        self.now.notify_one();
    }

    /// One look: how many links were made, or None when not signed in or
    /// the list couldn't be read.
    pub async fn look(&self) -> Option<usize> {
        *self.last.lock() = Some(tokio::time::Instant::now());
        let (me, token) = self.side.me().await?;
        let list = match self.side.list(&token).await {
            Ok(l) => l,
            Err(e) => {
                tracing::info!(kind = e.kind(), "couldn't read the account's device list");
                return None;
            }
        };
        let fresh = self.side.apply(&me, &list).await;
        let relay = self.relays.relay(&self.relay);
        for key in &fresh {
            if let Ok(body) = knock_body(key, (self.clock)()) {
                let _ = relay.publish(&knock_channel(key).topic, &body).await;
            }
        }
        Some(fresh.len())
    }

    /// Look every [`LOOK_EVERY`], soon after signing in, when asked and
    /// when knocked, until `stop`.
    pub async fn run(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let (tx, mut knocks) = mpsc::channel::<()>(8);
        let mut listening: Option<(PeerKey, watch::Sender<bool>)> = None;
        let mut every = tokio::time::interval(LOOK_EVERY);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        every.tick().await;
        let mut check = tokio::time::interval(SIGN_IN_CHECK);
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut soon: Option<tokio::time::Instant> = None;
        loop {
            tokio::select! {
                _ = stop.changed() => {
                    if let Some((_, s)) = listening.take() {
                        let _ = s.send(true);
                    }
                    return;
                }
                _ = check.tick() => {
                    // Knocks for the key this Platter is signed in with now,
                    // and a look as soon as it signs in (or in again).
                    let key = self.side.me().await.map(|(m, _)| m.key);
                    if listening.as_ref().map(|(k, _)| k) != key.as_ref() {
                        if let Some((_, old)) = listening.take() {
                            let _ = old.send(true);
                        }
                        if let Some(k) = key {
                            let (stop_tx, stop_rx) = watch::channel(false);
                            tokio::spawn(listen_knocks(self.relays.clone(), self.relay.clone(), k.clone(), self.clock.clone(), tx.clone(), stop_rx));
                            listening = Some((k, stop_tx));
                            soon = Some(self.next_allowed());
                        }
                    }
                }
                _ = every.tick() => {
                    self.look().await;
                }
                Some(()) = knocks.recv() => soon = Some(self.next_allowed()),
                _ = self.now.notified() => soon = Some(self.next_allowed()),
                _ = async { if let Some(t) = soon { tokio::time::sleep_until(t).await } else { std::future::pending::<()>().await } } => {
                    soon = None;
                    self.look().await;
                }
            }
        }
    }

    fn next_allowed(&self) -> tokio::time::Instant {
        let now = tokio::time::Instant::now();
        match *self.last.lock() {
            Some(t) if t + LOOK_GAP > now => t + LOOK_GAP,
            _ => now,
        }
    }
}

async fn listen_knocks(relays: Arc<dyn RelayFactory>, relay: String, key: PeerKey, clock: Clock, tx: mpsc::Sender<()>, mut stop: watch::Receiver<bool>) {
    let relay = relays.relay(&relay);
    let topic = knock_channel(&key).topic;
    let mut wait = Duration::from_secs(2);
    loop {
        if *stop.borrow() {
            return;
        }
        if let Ok(mut rx) = relay.subscribe(std::slice::from_ref(&topic), None).await {
            loop {
                tokio::select! {
                    ev = rx.recv() => match ev {
                        Some(RelayEvent::Open) => wait = Duration::from_secs(2),
                        Some(RelayEvent::Message { body, .. }) => {
                            if is_knock(&key, &body, clock()) {
                                let _ = tx.try_send(());
                            }
                        }
                        None => break,
                    },
                    _ = stop.changed() => return,
                }
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = stop.changed() => return,
        }
        wait = (wait * 2).min(Duration::from_secs(300));
    }
}

/// A code-paired Bridge that went quiet, whose place an account-linked
/// Bridge has taken. That's the pairing it lost (its keys file was lost, or
/// only one end saved it). One that was heard from must be the same device;
/// one never heard from (it never said which device it is) goes when it's
/// older than [`QUIET_S`] and has the live one's name. Gives the
/// code-paired links to forget.
pub fn replaced(have: &[Paired], last_seen: impl Fn(&str) -> Option<i64>, now_s: i64) -> Vec<String> {
    let live: Vec<&Paired> = have.iter().filter(|p| p.auto && last_seen(&p.id).is_some_and(|t| now_s - t < QUIET_S)).collect();
    have.iter()
        .filter(|p| !p.auto)
        .filter(|p| match last_seen(&p.id) {
            // Never heard from: the other end never kept its half.
            None => now_s - p.paired_at >= QUIET_S && live.iter().any(|a| a.name == p.name || (p.device.is_some() && a.device == p.device)),
            Some(t) => now_s - t >= QUIET_S && p.device.is_some() && live.iter().any(|a| a.device == p.device),
        })
        .map(|p| p.id.clone())
        .collect()
}

// ---------------------------------------------------------------- the app

/// The app's side: Bridges into Settings > Bridge's links, the user's
/// other computers and phones into device sync's.
pub struct AppAccount {
    pub plex: crate::plex::PlexState,
    pub bridges: Arc<super::desktop::Desktop>,
    pub devices: Arc<crate::sync::devices::DeviceSync>,
}

#[async_trait]
impl AccountSide for AppAccount {
    async fn me(&self) -> Option<(Me, Token)> {
        let token = self.plex.account_token().await?;
        let me = Me { id: self.plex.identity().client_id.clone(), key: PeerKey::from_token(&token), bridge: false };
        Some((me, token))
    }

    async fn list(&self, token: &Token) -> AppResult<Vec<AccountDevice>> {
        parse_list(&self.plex.plex_tv().devices_json(token).await?)
    }

    async fn apply(&self, me: &Me, list: &[AccountDevice]) -> Vec<PeerKey> {
        // Only the server's owner links to Bridges (they reach Lidarr).
        // Until a server is connected (at start), Bridges stay as they are.
        let mut keys = if self.plex.machine_id().await.is_none() {
            Vec::new()
        } else if self.plex.owns_current_server().await {
            self.bridges.account_links(me, list)
        } else {
            self.bridges.account_links(me, &without_bridges(list))
        };
        keys.extend(self.devices.account_links(me, list));
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::homeside::links::Links;

    fn key(t: &str) -> PeerKey {
        PeerKey::from_token(&Token::new(t).unwrap())
    }

    fn dev(id: &str, platform: &str, token: &str) -> AccountDevice {
        AccountDevice { id: id.into(), name: format!("{id} name"), product: PRODUCT.into(), platform: platform.into(), device: platform.into(), keys: vec![key(token)] }
    }

    fn me(id: &str, token: &str, bridge: bool) -> Me {
        Me { id: id.into(), key: key(token), bridge }
    }

    /// The account's list: a Mac, a phone, a PC, the Bridge and a TV app.
    fn list() -> Vec<AccountDevice> {
        vec![
            dev("mac", "macOS", "mac-token"),
            dev("phone", "iOS", "phone-token"),
            dev("pc", "Windows", "pc-token"),
            dev("bridge", "Linux", "bridge-token"),
            AccountDevice { product: "Plex for LG".into(), ..dev("tv", "webOS", "tv-token") },
        ]
    }

    fn paired(id: &str, device: Option<&str>, auto: bool) -> Paired {
        serde_json::from_value(serde_json::json!({
            "id": id, "relay": "r", "side": "desktop", "me": "m", "name": "Shed", "pairedAt": 1, "device": device, "auto": auto
        }))
        .unwrap()
    }

    fn ids(p: &Plan) -> Vec<String> {
        let mut v: Vec<String> = p.add.iter().map(|w| w.device.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn the_list_gives_platters_with_keys_never_tokens() {
        let body = serde_json::json!([
            {"clientIdentifier": "mac", "name": "Studio Mac", "product": "Platter", "platform": "macOS", "token": "mac-token"},
            {"clientIdentifier": "bridge", "name": "3f2a", "product": "Platter", "platform": "Linux", "device": "Linux", "token": "b1"},
            {"clientIdentifier": "bridge", "product": "Platter", "platform": "Linux", "token": "b2"},
            {"clientIdentifier": "tv", "product": "Plex for LG", "token": "tv"},
            {"clientIdentifier": "no-token", "product": "Platter"},
            {"token": "orphan"}
        ]);
        let l = parse_list(body.to_string().as_bytes()).unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].name, "Studio Mac");
        assert!(l[0].is_platter() && !l[0].is_bridge());
        assert!(l[1].is_bridge());
        assert_eq!(l[1].keys, vec![key("b1"), key("b2")], "listed twice: both keys");
        assert!(!l[2].is_platter());
        assert!(!format!("{:?}", l[0]).contains("mac-token"));
        assert!(parse_list(b"{}").is_err());
    }

    #[test]
    fn both_ends_work_out_the_same_link_and_nobody_else_does() {
        let mac = me("mac", "mac-token", false);
        let bridge = me("bridge", "bridge-token", true);
        let a = account_link(Kind::Bridge, &mac, "bridge", &key("bridge-token"), "r");
        let b = account_link(Kind::Bridge, &bridge, "mac", &key("mac-token"), "r");
        assert_eq!((a.id.as_str(), a.secret), (b.id.as_str(), b.secret));
        assert_eq!((a.side, b.side), (Side::Desktop, Side::Bridge));
        assert_eq!(a.outbox(), b.inbox());
        assert_eq!(b.outbox(), a.inbox());
        // Two devices: each listens where the other publishes.
        let m = account_link(Kind::Device, &mac, "phone", &key("phone-token"), "r");
        let p = account_link(Kind::Device, &me("phone", "phone-token", false), "mac", &key("mac-token"), "r");
        assert_eq!(m.id, p.id);
        assert_eq!(m.outbox(), p.inbox());
        assert_eq!(p.outbox(), m.inbox());
        // Another account (another token), a device signed in again, or a
        // Bridge link used as a device link: never the same link.
        assert_ne!(account_link(Kind::Bridge, &me("mac", "other-account-token", false), "bridge", &key("bridge-token"), "r").secret, a.secret);
        assert_ne!(account_link(Kind::Bridge, &mac, "bridge", &key("bridge-token-2"), "r").secret, a.secret);
        assert_ne!(account_link(Kind::Device, &mac, "bridge", &key("bridge-token"), "r").secret, a.secret);
    }

    #[test]
    fn a_computer_links_to_the_bridge_and_the_other_devices_a_bridge_to_every_device() {
        let mac = me("mac", "mac-token", false);
        let p = plan(Kind::Bridge, &mac, &list(), &[], &[], "r", |_| true).unwrap();
        assert_eq!(ids(&p), ["bridge"]);
        let p = plan(Kind::Device, &mac, &list(), &[], &[], "r", |_| true).unwrap();
        assert_eq!(ids(&p), ["pc", "phone"], "not itself, not the Bridge, not the TV app");
        let bridge = me("bridge", "bridge-token", true);
        let p = plan(Kind::Bridge, &bridge, &list(), &[], &[], "r", |_| true).unwrap();
        assert_eq!(ids(&p), ["mac", "pc", "phone"]);
        assert!(plan(Kind::Device, &bridge, &list(), &[], &[], "r", |_| true).unwrap().add.is_empty());
    }

    #[test]
    fn nothing_changes_until_the_list_has_this_device_under_its_own_key() {
        // Another account or a Home user: its own list never has this token.
        assert!(plan(Kind::Bridge, &me("mac", "home-user-token", false), &list(), &[], &[], "r", |_| true).is_none());
        assert!(plan(Kind::Bridge, &me("new", "new-token", false), &list(), &[], &[], "r", |_| true).is_none());
    }

    #[test]
    fn a_device_signed_out_on_plex_tv_drops_and_a_new_one_is_added() {
        let mac = me("mac", "mac-token", false);
        let first = plan(Kind::Device, &mac, &list(), &[], &[], "r", |_| true).unwrap();
        let have: Vec<Paired> = first.add.iter().map(|w| paired(&w.link.id, Some(&w.device), true)).collect();
        // The phone was signed out on plex.tv.
        let l: Vec<AccountDevice> = list().into_iter().filter(|d| d.id != "phone").collect();
        let p = plan(Kind::Device, &mac, &l, &have, &[], "r", |_| true).unwrap();
        assert!(p.add.is_empty());
        let phone = first.add.iter().find(|w| w.device == "phone").unwrap();
        assert_eq!(p.drop, vec![phone.link.id.clone()]);
        // Signed in again: a new token, so a new link and the old one goes.
        let mut l = list();
        l[1].keys = vec![key("phone-token-2")];
        let p = plan(Kind::Device, &mac, &l, &have, &[], "r", |_| true).unwrap();
        assert_eq!(ids(&p), ["phone"]);
        assert_eq!(p.drop, vec![phone.link.id.clone()]);
    }

    fn server(id: &str, owned: bool) -> crate::plex::discovery::DiscoveredServer {
        crate::plex::discovery::DiscoveredServer { machine_id: id.into(), name: id.into(), owned, presence: true, version: None, connections: vec![], access_token: None }
    }

    #[test]
    fn only_the_servers_owner_is_linked_to_a_bridge() {
        // The owner's server, and a friend's shared with them.
        let servers = [server("home", true), server("friend", false)];
        assert!(owns_server(&servers, Some("home")));
        assert!(!owns_server(&servers, Some("friend")), "a shared server isn't theirs");
        assert!(owns_server(&servers, None));
        assert!(!owns_server(&[server("friend", false)], None));
        assert!(!owns_server(&[], None));

        // A computer signed in as someone who doesn't own the server drops
        // the Bridge it had and links to no other.
        let mac = me("mac", "mac-token", false);
        let first = plan(Kind::Bridge, &mac, &list(), &[], &[], "r", |_| true).unwrap();
        let have: Vec<Paired> = first.add.iter().map(|w| paired(&w.link.id, Some(&w.device), true)).collect();
        assert_eq!(have.len(), 1);
        let p = plan(Kind::Bridge, &mac, &without_bridges(&list()), &have, &[], "r", |_| true).unwrap();
        assert!(p.add.is_empty());
        assert_eq!(p.drop, vec![have[0].id.clone()]);
        // Its other devices still keep in sync with each other.
        let p = plan(Kind::Device, &mac, &without_bridges(&list()), &[], &[], "r", |_| true).unwrap();
        assert_eq!(ids(&p), ["pc", "phone"]);

        // A Bridge signed in to an account that doesn't own the server
        // drops every device and links to none.
        let bridge = me("bridge", "bridge-token", true);
        let first = plan(Kind::Bridge, &bridge, &list(), &[], &[], "r", |_| true).unwrap();
        let have: Vec<Paired> = first.add.iter().map(|w| paired(&w.link.id, Some(&w.device), true)).collect();
        assert_eq!(have.len(), 3);
        let p = plan(Kind::Bridge, &bridge, &only_bridges(&list()), &have, &[], "r", |_| true).unwrap();
        assert!(p.add.is_empty());
        assert_eq!(p.drop.len(), 3);
    }

    #[test]
    fn switching_plex_users_on_a_device_drops_its_bridge() {
        let mac = me("mac", "mac-token", false);
        let first = plan(Kind::Bridge, &mac, &list(), &[], &[], "r", |_| true).unwrap();
        let have: Vec<Paired> = first.add.iter().map(|w| paired(&w.link.id, Some(&w.device), true)).collect();
        // Someone else signs in on the Mac: a new token, and their own list,
        // which has no Bridge.
        let them = me("mac", "their-token", false);
        let theirs = vec![dev("mac", "macOS", "their-token"), dev("their-phone", "iOS", "their-phone-token")];
        let p = plan(Kind::Bridge, &them, &theirs, &have, &[], "r", |_| true).unwrap();
        assert!(p.add.is_empty());
        assert_eq!(p.drop, vec![have[0].id.clone()]);
    }

    #[test]
    fn a_code_paired_link_that_works_keeps_working_and_one_that_doesnt_is_relinked() {
        let mac = me("mac", "mac-token", false);
        let old = paired("old", Some("bridge"), false);
        let p = plan(Kind::Bridge, &mac, &list(), std::slice::from_ref(&old), &[], "r", |_| true).unwrap();
        assert!(p.add.is_empty() && p.drop.is_empty(), "not linked twice, and never dropped");
        let p = plan(Kind::Bridge, &mac, &list(), &[old], &[], "r", |_| false).unwrap();
        assert_eq!(ids(&p), ["bridge"], "its other end lost it: linked again by account");
    }

    #[test]
    fn a_device_the_user_unpaired_stays_apart_until_it_signs_in_again() {
        let mac = me("mac", "mac-token", false);
        let first = plan(Kind::Device, &mac, &list(), &[], &[], "r", |_| true).unwrap();
        let phone = first.add.iter().find(|w| w.device == "phone").unwrap().link.id.clone();
        let p = plan(Kind::Device, &mac, &list(), &[], std::slice::from_ref(&phone), "r", |_| true).unwrap();
        assert_eq!(ids(&p), ["pc"]);
        // The phone signs in to Plex again: a new link, which is made.
        let mut l = list();
        l[1].keys = vec![key("phone-token-2")];
        let p = plan(Kind::Device, &mac, &l, &[], std::slice::from_ref(&phone), "r", |_| true).unwrap();
        assert_eq!(ids(&p), ["pc", "phone"]);
    }

    #[test]
    fn a_device_signed_in_twice_keeps_only_the_link_that_answers() {
        let mac = me("mac", "mac-token", false);
        let mut l = list();
        l[1].keys = vec![key("phone-old"), key("phone-new")];
        let first = plan(Kind::Device, &mac, &l, &[], &[], "r", |_| true).unwrap();
        let phone: Vec<&Wanted> = first.add.iter().filter(|w| w.device == "phone").collect();
        assert_eq!(phone.len(), 2, "both, until one answers");
        let have: Vec<Paired> = first.add.iter().map(|w| paired(&w.link.id, Some(&w.device), true)).collect();
        let answered = phone[1].link.id.clone();
        let p = plan_heard(Kind::Device, &mac, &l, &have, &[], "r", |_| true, |p| p.id == answered).unwrap();
        assert!(p.add.is_empty());
        assert_eq!(p.drop, vec![phone[0].link.id.clone()]);
        // Nothing heard yet: both stay.
        let p = plan_heard(Kind::Device, &mac, &l, &have, &[], "r", |_| true, |_| false).unwrap();
        assert!(p.add.is_empty() && p.drop.is_empty());
    }

    #[test]
    fn a_quiet_code_paired_bridge_gives_way_to_the_same_one_linked_by_account() {
        let now = 1_000_000;
        let mut auto = paired("auto", Some("bridge"), true);
        auto.name = "Unraid Bridge".into();
        let mut never = paired("never", None, false);
        never.name = "Unraid Bridge".into();
        let mut quiet = paired("quiet", Some("bridge"), false);
        quiet.name = "Old name".into();
        let mut fine = paired("fine", None, false);
        fine.name = "Unraid Bridge".into();
        let mut other = paired("other", None, false);
        other.name = "Garage Bridge".into();
        // A second Bridge with the same name, quiet for a while: not this one.
        let mut twin = paired("twin", Some("garage"), false);
        twin.name = "Unraid Bridge".into();
        // Paired with a code a moment ago and not heard from yet.
        let mut fresh = paired("fresh", None, false);
        fresh.name = "Unraid Bridge".into();
        fresh.paired_at = now - 60;
        let seen = |id: &str| match id {
            "auto" | "fine" => Some(now - 60),
            "quiet" | "other" | "twin" => Some(now - QUIET_S - 1),
            _ => None,
        };
        let mut gone = replaced(&[auto.clone(), never, quiet, fine, other, twin, fresh], seen, now);
        gone.sort();
        assert_eq!(gone, ["never", "quiet"]);
        // Until the account link answers, nothing goes.
        assert!(replaced(&[auto, paired("never", None, false)], |_| None, now).is_empty());
    }

    #[test]
    fn a_knock_opens_only_for_its_key_and_only_for_a_while() {
        let k = key("bridge-token");
        let body = knock_body(&k, 1_000).unwrap();
        assert!(is_knock(&k, &body, 1_000));
        assert!(!is_knock(&key("other"), &body, 1_000));
        assert!(!is_knock(&k, &body, 1_000 + KNOCK_TTL_MS + 1));
        assert_ne!(knock_channel(&k).topic, knock_channel(&key("other")).topic);
    }

    #[test]
    fn an_account_link_keeps_its_replay_guard_when_it_comes_back_and_reads_a_little_back() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::DbHandle::open(dir.path().join("l.db")).unwrap();
        let links = Links::new(db, Arc::new(crate::plex::keychain::MemoryStore::new()));
        let mac = me("mac", "mac-token", false);
        let w = plan(Kind::Bridge, &mac, &list(), &[], &[], "r", |_| true).unwrap().add.remove(0);
        links.add_auto(&w, 10_000).unwrap();
        let p = links.get(&w.link.id).unwrap();
        assert!(p.auto);
        assert_eq!(p.device.as_deref(), Some("bridge"));
        assert_eq!(p.since.as_deref(), Some((10_000 - BACKLOG_S).to_string().as_str()));
        links.update(&w.link.id, |p| {
            p.guard.last.insert("brdg".into(), 77);
        }).unwrap();
        links.remove(&w.link.id).unwrap();
        links.add_auto(&w, 20_000).unwrap();
        assert_eq!(links.get(&w.link.id).unwrap().guard.last.get("brdg"), Some(&77));
        // Unpaired by the user: kept apart.
        links.unpaired(&w.link.id);
        assert_eq!(links.apart(), vec![w.link.id.clone()]);
    }
}
