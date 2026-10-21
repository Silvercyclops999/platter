//! Pairing two of the user's own devices, once, so they keep in sync.
//!
//! The device being added shows two codes for 10 minutes:
//!
//! - A 6-digit code, for a device on the same network. The other device
//!   types it, finds this one over mDNS (same Plex account only, every
//!   request signed), and the two run a SPAKE2 exchange over the code
//!   through the control port ([`crate::remote::peer::pair`]). The joining
//!   device proves it reached the key first, in its finish; only then does
//!   this one answer with anything derived from the key. A wrong code fails
//!   that finish; [`TRIES`] failed finishes per code. A hello alone uses up
//!   nothing, so another device can't spend the tries.
//! - A long `PB1-` code and its QR code, for a device somewhere else. It
//!   goes through the sealed relay ([`crate::homeside::pair`], under device
//!   labels). The relay topic comes from the code's 128 random bits; a
//!   topic from 6 digits could be guessed, so a 6-digit code never touches
//!   the relay. When a device joins with it, this one asks "Pair with
//!   <name>?" and answers only once the user says yes.
//!
//! Either way both ends say which Plex account they're signed in to inside
//! the sealed exchange and refuse another one, and both end up with the
//! same [`Side::Device`] link in `Links::devices`, the other device's id
//! filled in. The link secret comes from the key exchange; neither the
//! code nor anything that travelled lets anyone else work it out. No key
//! or token is in any of it.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, watch};

use crate::error::{AppError, AppResult};
use crate::homeside::code::PairCode;
use crate::homeside::link::{Link, Side};
use crate::homeside::links::Links;
use crate::homeside::pair::{DeviceMe, Joining, Offer};
use crate::homeside::peer::relay_error;
use crate::remote::peer::PeerDevice;
use crate::remote::peer::auth::hex;
use crate::remote::peer::client::{PeerClient, PeerError};
use crate::remote::peer::pair::{PairFinish, PairFinishReply, PairHello, PairHelloReply, PeerPair};
use crate::social::listen::DEFAULT_RELAY;
use crate::social::listen::invite::relay_url;
use crate::social::listen::relay::{RelayEvent, RelayFactory};
use crate::social::listen::seal;
use crate::sync::engine::AccountTag;
use crate::sync::keycopy::crypto::{self, SharedKey};
use crate::sync::keycopy::{Qr, qr_text, valid_code};
use crate::sync::store::Clock;

/// The codes last this long.
pub const CODE_TTL_MS: u64 = 10 * 60 * 1000;
/// Wrong 6-digit codes allowed before the code stops working.
pub const TRIES: u8 = 5;
/// Exchanges started and not finished kept at once (one per device).
const PENDING_MAX: usize = 4;
const STEP_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a device joining with the long code waits for the answer (the
/// other device's user has to say yes).
const RELAY_WAIT: Duration = if cfg!(test) { Duration::from_secs(4) } else { Duration::from_secs(90) };
/// Longest wait between tries at the relay while showing the long code.
const RELAY_RETRY_MAX: Duration = Duration::from_secs(60);

const WELCOME_AAD: &str = "platter-device pair welcome";
const FINISH_AAD: &str = "platter-device pair finish";

/// Shown when the relay can't be reached while the codes are up.
pub const RELAY_DOWN: &str = "The relay can't be reached. The 6-digit code still works on this network.";

/// What pairing needs from the app.
#[derive(Clone)]
pub struct PairDeps {
    /// This device's id (its Plex client id).
    pub own_id: String,
    /// Device links (`Links::devices`).
    pub links: Links,
    pub account: Arc<dyn AccountTag>,
    pub name: Arc<dyn Fn() -> String + Send + Sync>,
    pub version: String,
    /// The relay new links use: the user's own, or the default one.
    pub relay: Arc<dyn Fn() -> String + Send + Sync>,
    pub relays: Arc<dyn RelayFactory>,
    pub clock: Clock,
}

/// What the "Add a device" dialog shows on the device being added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PairPhase {
    #[default]
    Idle,
    /// Showing the codes.
    Waiting,
    /// A device joined with the long code: "Pair with <with>?"
    Confirm,
    Paired,
    Expired,
    /// Too many wrong 6-digit codes; these stopped working.
    Locked,
    /// The user said no to the device that joined.
    Declined,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairState {
    pub phase: PairPhase,
    /// For a device on the same network.
    pub code: Option<String>,
    /// For a device somewhere else, and its QR code (not while the relay
    /// can't be reached).
    pub long_code: Option<String>,
    pub qr: Option<Qr>,
    pub expires_at: Option<u64>,
    /// The device it paired with, or the one asking to.
    pub with: Option<String>,
    /// Something to say beside the codes (the relay can't be reached).
    pub note: Option<String>,
}

/// A pairing that finished.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paired {
    /// The new link's id.
    pub id: String,
    /// The other device's name.
    pub name: String,
}

/// What each end tells the other inside the sealed exchange.
#[derive(Serialize, Deserialize)]
struct End {
    /// Its id inside frames on the link.
    frame: String,
    name: String,
    acct: String,
    #[serde(default)]
    version: String,
    /// The relay the link uses (from the showing device).
    #[serde(default)]
    relay: String,
}

/// A 6-digit exchange started by `from`: the key reached, this end's frame
/// id and the relay offered.
struct Pending {
    from: String,
    key: SharedKey,
    frame: String,
    relay: String,
}

struct Showing {
    code: String,
    long: PairCode,
    expires_at: u64,
    tries_left: u8,
    pending: Vec<Pending>,
    phase: PairPhase,
    with: Option<String>,
    /// The relay can't be reached: the long code is hidden.
    relay_down: bool,
    /// The user's answer to "Pair with <with>?"
    asking: Option<oneshot::Sender<bool>>,
    stop: watch::Sender<bool>,
}

/// Both ends of a pairing: showing the codes, and joining with one.
pub struct DevicePairing {
    d: PairDeps,
    showing: Mutex<Option<Showing>>,
    on_paired: Mutex<Option<OnPaired>>,
    on_state: Mutex<Option<OnState>>,
}

type OnPaired = Arc<dyn Fn(&Paired) + Send + Sync>;
type OnState = Arc<dyn Fn(&PairState) + Send + Sync>;

fn new_code() -> String {
    format!("{:06}", rand::random_range(0..1_000_000u32))
}

/// The 6-digit code as the SPAKE2 password, apart from key copy's.
fn password(code: &str) -> String {
    format!("platter device pair {code}")
}

fn link_secret(key: &SharedKey) -> [u8; 32] {
    key.derive("platter-device-pair-v1 link")
}

fn link_id(secret: &[u8; 32]) -> String {
    hex(&seal::derive(secret, "device link id")[..8])
}

fn confirmation(key: &SharedKey) -> [u8; 32] {
    key.derive("platter-device-pair-v1 confirm")
}

fn seal_side(key: &SharedKey, aad: &str, s: &End) -> AppResult<String> {
    let plain = serde_json::to_vec(s).map_err(|e| AppError::Internal(e.to_string()))?;
    seal::seal_with(&key.derive("platter-device-pair-v1 seal"), aad, &plain)
}

const NO_MATCH: &str = "That code doesn't match. Check the code on the other device.";

fn open_side(key: &SharedKey, aad: &str, text: &str) -> AppResult<End> {
    let bad = || AppError::Invalid(NO_MATCH.into());
    let plain = seal::open_with(&key.derive("platter-device-pair-v1 seal"), aad, text).map_err(|_| bad())?;
    serde_json::from_slice(&plain).map_err(|_| bad())
}

const OTHER_ACCOUNT: &str = "That device is signed in to a different Plex account or Home user.";

fn other_account() -> AppError {
    AppError::Auth(OTHER_ACCOUNT.into())
}

fn peer_err(e: PeerError) -> AppError {
    match e {
        PeerError::Unreachable => AppError::Network("Couldn't reach the other device.".into()),
        PeerError::Refused(m) => AppError::Auth(m),
        PeerError::Failed(m) if m == OTHER_ACCOUNT => other_account(),
        PeerError::Failed(m) => AppError::Invalid(m),
    }
}

/// The QR code for a long code.
pub fn qr(long_code: &str) -> Option<Qr> {
    qr_text(&format!("platter://pair-device/{long_code}"))
}

/// A long code typed, pasted or scanned (`platter://pair-device/PB1-...`).
fn long_code(raw: &str) -> Option<PairCode> {
    let raw = raw.trim();
    PairCode::parse(raw.strip_prefix("platter://pair-device/").unwrap_or(raw))
}

impl DevicePairing {
    pub fn new(d: PairDeps) -> Arc<Self> {
        Arc::new(Self { d, showing: Mutex::new(None), on_paired: Mutex::new(None), on_state: Mutex::new(None) })
    }

    /// Called with each new link (to start syncing with it).
    pub fn set_on_paired(&self, f: OnPaired) {
        *self.on_paired.lock() = Some(f);
    }

    /// Called when what the dialog shows changes.
    pub fn set_on_state(&self, f: OnState) {
        *self.on_state.lock() = Some(f);
    }

    fn now(&self) -> u64 {
        (self.d.clock)()
    }

    fn relay(&self) -> String {
        relay_url(&(self.d.relay)()).unwrap_or_else(|| DEFAULT_RELAY.to_string())
    }

    fn view(s: Option<&Showing>) -> PairState {
        let Some(s) = s else { return PairState::default() };
        let showing = s.phase == PairPhase::Waiting;
        let long = (showing && !s.relay_down).then(|| s.long.text());
        PairState {
            phase: s.phase,
            code: showing.then(|| s.code.clone()),
            qr: long.as_deref().and_then(qr),
            long_code: long,
            expires_at: matches!(s.phase, PairPhase::Waiting | PairPhase::Confirm).then_some(s.expires_at),
            with: s.with.clone(),
            note: (showing && s.relay_down).then(|| RELAY_DOWN.to_string()),
        }
    }

    fn tick(&self, s: &mut Option<Showing>) {
        let now = self.now();
        if let Some(x) = s.as_mut()
            && matches!(x.phase, PairPhase::Waiting | PairPhase::Confirm)
            && now >= x.expires_at
        {
            x.phase = PairPhase::Expired;
            x.pending.clear();
            x.asking = None;
            let _ = x.stop.send(true);
        }
    }

    pub fn state(&self) -> PairState {
        let mut s = self.showing.lock();
        self.tick(&mut s);
        Self::view(s.as_ref())
    }

    fn notify(&self) {
        let st = self.state();
        if let Some(f) = self.on_state.lock().clone() {
            f(&st);
        }
    }

    fn paired(&self, p: &Paired) {
        tracing::info!("Paired with another of your devices.");
        if let Some(f) = self.on_paired.lock().clone() {
            f(p);
        }
    }

    /// Show new codes (any earlier ones stop working). Needs this device
    /// signed in: the other device must be of the same account.
    pub async fn start(self: &Arc<Self>) -> AppResult<PairState> {
        if self.d.account.tag().await.is_none() {
            return Err(AppError::Auth("Sign in to Plex first.".into()));
        }
        let now = self.now();
        let relay = self.relay();
        let mut long = PairCode::new((relay != DEFAULT_RELAY).then(|| relay.clone()), (now / 1000) as i64);
        long.expires = ((now + CODE_TTL_MS) / 1000) as u32;
        let (stop_tx, stop) = watch::channel(false);
        let old = self.showing.lock().replace(Showing {
            code: new_code(),
            long: long.clone(),
            expires_at: now + CODE_TTL_MS,
            tries_left: TRIES,
            pending: Vec::new(),
            phase: PairPhase::Waiting,
            with: None,
            relay_down: false,
            asking: None,
            stop: stop_tx,
        });
        if let Some(old) = old {
            let _ = old.stop.send(true);
        }
        let me = self.clone();
        tokio::spawn(async move {
            if let Err(e) = me.wait_on_relay(long, relay, stop).await {
                tracing::info!("device pairing: {e}");
            }
        });
        self.notify();
        Ok(self.state())
    }

    pub fn cancel(&self) {
        if let Some(s) = self.showing.lock().take() {
            let _ = s.stop.send(true);
        }
        self.notify();
    }

    /// The user's answer to "Pair with <name>?" for a device that joined
    /// with the long code.
    pub fn confirm(&self, yes: bool) -> AppResult<PairState> {
        let tx = self.showing.lock().as_mut().filter(|s| s.phase == PairPhase::Confirm).and_then(|s| s.asking.take());
        let tx = tx.ok_or_else(|| AppError::NotFound("No device is asking to pair.".into()))?;
        let _ = tx.send(yes);
        if !yes {
            self.end(PairPhase::Declined, None);
        }
        Ok(self.state())
    }

    /// The codes are used: stop showing them.
    fn done(&self, with: &str) {
        self.end(PairPhase::Paired, Some(with));
    }

    fn end(&self, phase: PairPhase, with: Option<&str>) {
        if let Some(s) = self.showing.lock().as_mut() {
            s.phase = phase;
            if let Some(w) = with {
                s.with = Some(w.to_string());
            }
            s.pending.clear();
            s.asking = None;
            let _ = s.stop.send(true);
        }
        self.notify();
    }

    fn set_relay_down(&self, down: bool) {
        let changed = self.showing.lock().as_mut().is_some_and(|s| {
            let was = s.relay_down;
            s.relay_down = down;
            was != down
        });
        if changed {
            self.notify();
        }
    }

    fn add(&self, link: &Link, name: &str, version: &str, device: &str) -> AppResult<Paired> {
        let name: String = name.chars().take(60).collect();
        let name = if name.trim().is_empty() { "Your other device".to_string() } else { name };
        self.d.links.add(link, &name, &version.chars().take(20).collect::<String>(), (self.now() / 1000) as i64)?;
        let device: String = device.chars().take(100).collect();
        if let Err(e) = self.d.links.update(&link.id, |p| p.device = Some(device)) {
            // Half a link is no link: take it back out.
            if let Err(e) = self.d.links.remove(&link.id) {
                tracing::warn!("device pairing: couldn't take back a half-made link: {e}");
            }
            return Err(e);
        }
        Ok(Paired { id: link.id.clone(), name })
    }

    // ------------------------------------------------------------ the long code

    /// How long the long code has left (ms).
    fn left(&self, code: &PairCode) -> u64 {
        (i64::from(code.expires) * 1000 - self.now() as i64).max(0) as u64
    }

    /// Wait on the relay for a device joining with the long code. When the
    /// relay can't be reached, the long code is hidden (the 6-digit code
    /// still works) and the relay is tried again, waiting longer each time.
    async fn wait_on_relay(self: &Arc<Self>, code: PairCode, relay_url: String, mut stop: watch::Receiver<bool>) -> AppResult<()> {
        let Some(acct) = self.d.account.tag().await else { return Ok(()) };
        let relay = self.d.relays.relay(&relay_url);
        let mut offer = Offer::for_device(code.clone(), &(self.d.name)(), DeviceMe { device: self.d.own_id.clone(), acct });
        let topic = offer.listen_topic();
        let mut wait = Duration::from_secs(2);
        loop {
            let mut opened = false;
            if self.left(&code) == 0 {
                self.notify();
                return Ok(());
            }
            match relay.subscribe(std::slice::from_ref(&topic), None).await {
                Err(e) => {
                    tracing::info!("device pairing: {}", relay_error(e));
                    self.set_relay_down(true);
                }
                Ok(mut rx) => loop {
                    let left = self.left(&code).max(1);
                    // Closed before it opened: the relay can't be reached.
                    tokio::select! {
                        _ = stop.changed() => return Ok(()),
                        _ = tokio::time::sleep(Duration::from_millis(left)) => {
                            self.notify();
                            return Ok(());
                        }
                        ev = rx.recv() => match ev {
                            Some(RelayEvent::Message { body, .. }) => {
                                if self.state().phase != PairPhase::Waiting {
                                    return Ok(());
                                }
                                if let Some(((wtopic, wbody), joined)) = offer.answer(&body, self.now()) {
                                    return self.ask_then_welcome(&*relay, (wtopic, wbody), joined, &code, &mut stop).await;
                                }
                            }
                            Some(RelayEvent::Open) => {
                                opened = true;
                                self.set_relay_down(false);
                                wait = Duration::from_secs(2);
                            }
                            None => {
                                if !opened {
                                    self.set_relay_down(true);
                                }
                                break;
                            }
                        }
                    }
                },
            }
            tokio::select! {
                _ = stop.changed() => return Ok(()),
                _ = tokio::time::sleep(wait) => {}
            }
            wait = (wait * 2).min(RELAY_RETRY_MAX);
        }
    }

    /// Ask "Pair with <name>?" and send the welcome only on a yes.
    async fn ask_then_welcome(
        &self,
        relay: &dyn crate::social::listen::relay::Relay,
        (wtopic, wbody): (String, String),
        joined: crate::homeside::pair::PairedBridge,
        code: &PairCode,
        stop: &mut watch::Receiver<bool>,
    ) -> AppResult<()> {
        let (tx, rx) = oneshot::channel();
        {
            let mut g = self.showing.lock();
            let Some(s) = g.as_mut().filter(|s| s.phase == PairPhase::Waiting) else { return Ok(()) };
            s.phase = PairPhase::Confirm;
            s.with = Some(joined.name.clone());
            s.pending.clear();
            s.asking = Some(tx);
        }
        self.notify();
        let left = self.left(code).max(1);
        let yes = tokio::select! {
            _ = stop.changed() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_millis(left)) => {
                self.notify();
                return Ok(());
            }
            r = rx => r.unwrap_or(false),
        };
        if !yes {
            tracing::info!("device pairing: the user turned a device away");
            self.end(PairPhase::Declined, None);
            return Ok(());
        }
        // The link is saved before the welcome goes: the other device treats
        // the welcome as paired, so this end must already be.
        let p = match self.add(&joined.link, &joined.name, &joined.version, &joined.device) {
            Ok(p) => p,
            Err(e) => {
                self.end(PairPhase::Expired, None);
                return Err(e);
            }
        };
        if let Err(e) = relay.publish(&wtopic, &wbody).await {
            if let Err(e) = self.d.links.remove(&p.id) {
                tracing::warn!("device pairing: couldn't take back a link the welcome never reached: {e}");
            }
            self.end(PairPhase::Expired, None);
            return Err(AppError::Network(relay_error(e)));
        }
        self.done(&p.name);
        self.paired(&p);
        Ok(())
    }

    /// Join the device showing this long code, through the relay.
    async fn join_relay(&self, code: PairCode) -> AppResult<Paired> {
        let acct = self.d.account.tag().await.ok_or_else(|| AppError::Auth("Sign in to Plex first.".into()))?;
        let relay_url = code.relay.clone().unwrap_or_else(|| DEFAULT_RELAY.to_string());
        let relay = self.d.relays.relay(&relay_url);
        let me = DeviceMe { device: self.d.own_id.clone(), acct };
        let (joining, (topic, body)) = Joining::start_device(code, &(self.d.name)(), &self.d.version, me, self.now())?;
        let unreachable = || AppError::Network("The relay can't be reached. Check the connection and try again.".into());
        let mut rx = relay.subscribe(&[joining.listen_topic()], None).await.map_err(|_| unreachable())?;
        // Listening first, so the answer can't arrive before it.
        loop {
            match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
                Ok(Some(RelayEvent::Open)) => break,
                Ok(Some(_)) => {}
                _ => return Err(unreachable()),
            }
        }
        relay.publish(&topic, &body).await.map_err(|e| AppError::Network(relay_error(e)))?;
        let gave_up = tokio::time::Instant::now() + RELAY_WAIT;
        let mut retry = Duration::from_millis(500);
        loop {
            match tokio::time::timeout_at(gave_up, rx.recv()).await {
                Ok(Some(RelayEvent::Message { body, .. })) if joining.is_welcome(&body, self.now()) => {
                    let d = joining.finish(&body, self.now())?;
                    return self.add(&d.link, &d.name, "", &d.device).inspect(|p| self.paired(p));
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    // The relay closed the stream: listen again, not at once.
                    if tokio::time::timeout_at(gave_up, tokio::time::sleep(retry)).await.is_err() {
                        continue;
                    }
                    retry = (retry * 2).min(Duration::from_secs(8));
                    rx = relay.subscribe(&[joining.listen_topic()], None).await.map_err(|_| unreachable())?;
                }
                Err(_) => {
                    return Err(AppError::Network(
                        "Your other device didn't answer. Check it's online and still showing its code, say yes there when it asks, and try again.".into(),
                    ));
                }
            }
        }
    }

    // ------------------------------------------------------------ the 6-digit code

    /// Join the device on this network showing `code`. `devices`: this
    /// account's other devices found over mDNS.
    async fn join_lan(&self, client: &PeerClient, devices: Vec<PeerDevice>, code: &str) -> AppResult<Paired> {
        let acct = self.d.account.tag().await.ok_or_else(|| AppError::Auth("Sign in to Plex first.".into()))?;
        let asks = devices.iter().map(|d| async move {
            for b in &d.bases {
                if let Ok(true) = client.pair_waiting(b, &d.id, STEP_TIMEOUT).await {
                    return Some((d.clone(), b.clone()));
                }
            }
            None
        });
        let waiting: Vec<_> = futures::future::join_all(asks).await.into_iter().flatten().collect();
        let (device, base) = match waiting.len() {
            0 => {
                return Err(AppError::NotFound(
                    "No device on this network is showing a code. Away from home, use the long code or scan its QR code instead.".into(),
                ));
            }
            1 => waiting.into_iter().next().expect("one"),
            _ => return Err(AppError::Invalid("More than one device is showing a code. Close the others and try again.".into())),
        };
        let me = client.own_id().to_string();
        let (state, msg_a) = crypto::sender_start(&password(code), &me, &device.id);
        let reply: PairHelloReply =
            client.pair_hello(&base, &device.id, &PairHello { msg: crypto::hex(&msg_a) }, STEP_TIMEOUT).await.map_err(peer_err)?;
        let key = crypto::sender_finish(state, &crypto::unhex(&reply.msg)?)?;
        // This end proves it has the code first; the other answers only then.
        let frame = hex(&seal::random16()[..8]);
        let mine = End { frame: frame.clone(), name: (self.d.name)(), acct: acct.clone(), version: self.d.version.clone(), relay: String::new() };
        let sealed = seal_side(&key, FINISH_AAD, &mine)?;
        let done: PairFinishReply = client.pair_finish(&base, &device.id, &PairFinish { sealed }, STEP_TIMEOUT).await.map_err(peer_err)?;
        if !crypto::same(&confirmation(&key), &crypto::unhex(&done.confirm)?) {
            return Err(AppError::Invalid(NO_MATCH.into()));
        }
        let theirs = open_side(&key, WELCOME_AAD, &done.sealed)?;
        if theirs.acct != acct {
            return Err(other_account());
        }
        let secret = link_secret(&key);
        let relay = relay_url(&theirs.relay).unwrap_or_else(|| DEFAULT_RELAY.to_string());
        let link = Link { id: link_id(&secret), secret, relay, side: Side::Device, me: frame, peer: theirs.frame };
        let p = self.add(&link, &theirs.name, &theirs.version, &device.id)?;
        self.paired(&p);
        Ok(p)
    }

    /// Join with what the user typed or scanned: a 6-digit code on this
    /// network, or a long code anywhere. `lan`: a signed client and this
    /// account's devices on this network, when there are any.
    pub async fn join(&self, code: &str, lan: Option<(PeerClient, Vec<PeerDevice>)>) -> AppResult<Paired> {
        if let Some(c) = valid_code(code) {
            let Some((client, devices)) = lan else {
                return Err(AppError::Invalid(
                    "A 6-digit code works only on the same network. Away from home, use the long code or scan its QR code.".into(),
                ));
            };
            return self.join_lan(&client, devices, &c).await;
        }
        let Some(long) = long_code(code) else {
            return Err(AppError::Invalid("That isn't a pairing code. Type the 6 digits, or paste the long code.".into()));
        };
        if long.expired((self.now() / 1000) as i64) {
            return Err(AppError::Invalid("This code has expired. Show a new one on your other device.".into()));
        }
        self.join_relay(long).await
    }
}

#[async_trait::async_trait]
impl PeerPair for DevicePairing {
    async fn waiting(&self) -> bool {
        self.state().phase == PairPhase::Waiting
    }

    /// The first half of the exchange. Answers with this end's half only:
    /// nothing derived from the key, and no try used up.
    async fn hello(&self, from: &str, req: PairHello) -> AppResult<PairHelloReply> {
        if self.d.account.tag().await.is_none() {
            return Err(AppError::Auth("This device isn't signed in to Plex.".into()));
        }
        let msg_a = crypto::unhex(&req.msg)?;
        let frame = hex(&seal::random16()[..8]);
        let relay = self.relay();
        let mut guard = self.showing.lock();
        self.tick(&mut guard);
        let s = guard
            .as_mut()
            .filter(|s| s.phase == PairPhase::Waiting)
            .ok_or_else(|| AppError::NotFound("That device isn't showing a code.".into()))?;
        if s.tries_left == 0 {
            return Err(AppError::Invalid("Too many wrong codes.".into()));
        }
        let (msg_b, key) = crypto::receiver_answer(&password(&s.code), from, &self.d.own_id, &msg_a)?;
        s.pending.retain(|p| p.from != from);
        if s.pending.len() >= PENDING_MAX {
            s.pending.remove(0);
        }
        s.pending.push(Pending { from: from.to_string(), key, frame, relay });
        Ok(PairHelloReply { msg: crypto::hex(&msg_b) })
    }

    /// The joining device's proof. A wrong code uses up a try; the right
    /// one makes the link here and answers with this end's side.
    async fn finish(&self, from: &str, req: PairFinish) -> AppResult<PairFinishReply> {
        let acct = self.d.account.tag().await.ok_or_else(|| AppError::Auth("This device isn't signed in to Plex.".into()))?;
        let (theirs, p) = {
            let mut guard = self.showing.lock();
            self.tick(&mut guard);
            let s = guard
                .as_mut()
                .filter(|s| s.phase == PairPhase::Waiting)
                .ok_or_else(|| AppError::NotFound("That device isn't showing a code.".into()))?;
            let Some(i) = s.pending.iter().position(|p| p.from == from) else {
                return Err(AppError::Invalid("Start again with the code.".into()));
            };
            let p = s.pending.remove(i);
            match open_side(&p.key, FINISH_AAD, &req.sealed) {
                Ok(t) => (t, p),
                Err(e) => {
                    s.tries_left = s.tries_left.saturating_sub(1);
                    if s.tries_left == 0 {
                        s.phase = PairPhase::Locked;
                        s.pending.clear();
                        let _ = s.stop.send(true);
                    }
                    drop(guard);
                    self.notify();
                    return Err(e);
                }
            }
        };
        if theirs.acct != acct {
            return Err(other_account());
        }
        let mine = End { frame: p.frame.clone(), name: (self.d.name)(), acct, version: self.d.version.clone(), relay: p.relay.clone() };
        let reply = PairFinishReply { confirm: crypto::hex(&confirmation(&p.key)), sealed: seal_side(&p.key, WELCOME_AAD, &mine)? };
        let secret = link_secret(&p.key);
        let link = Link { id: link_id(&secret), secret, relay: p.relay, side: Side::Device, me: p.frame, peer: theirs.frame };
        let paired = self.add(&link, &theirs.name, &theirs.version, from)?;
        self.done(&paired.name);
        self.paired(&paired);
        Ok(reply)
    }
}
