//! Pairing two of the user's own devices, once, so they keep in sync.
//!
//! The device being added shows two codes for 10 minutes:
//!
//! - A 6-digit code, for a device on the same network. The other device
//!   types it, finds this one over mDNS (same Plex account only, every
//!   request signed), and the two run a SPAKE2 exchange over the code
//!   through the control port ([`crate::remote::peer::pair`]). A wrong code
//!   fails the exchange; [`TRIES`] attempts per code.
//! - A long `PB1-` code and its QR code, for a device somewhere else. It
//!   goes through the sealed relay ([`crate::homeside::pair`], under device
//!   labels). The relay topic comes from the code's 128 random bits; a
//!   topic from 6 digits could be guessed, so a 6-digit code never touches
//!   the relay.
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
use tokio::sync::watch;

use crate::error::{AppError, AppResult};
use crate::homeside::code::PairCode;
use crate::homeside::link::{Link, Side};
use crate::homeside::links::Links;
use crate::homeside::pair::{DeviceMe, Joining, Offer};
use crate::homeside::peer::relay_error;
use crate::remote::peer::PeerDevice;
use crate::remote::peer::auth::hex;
use crate::remote::peer::client::{PeerClient, PeerError};
use crate::remote::peer::pair::{PairFinish, PairHello, PairHelloReply, PeerPair};
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
const STEP_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a device joining with the long code waits for the answer.
const RELAY_WAIT: Duration = if cfg!(test) { Duration::from_secs(4) } else { Duration::from_secs(90) };

const WELCOME_AAD: &str = "platter-device pair welcome";
const FINISH_AAD: &str = "platter-device pair finish";

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
    Paired,
    Expired,
    /// Too many wrong 6-digit codes; these stopped working.
    Locked,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairState {
    pub phase: PairPhase,
    /// For a device on the same network.
    pub code: Option<String>,
    /// For a device somewhere else, and its QR code.
    pub long_code: Option<String>,
    pub qr: Option<Qr>,
    pub expires_at: Option<u64>,
    /// The device it paired with.
    pub with: Option<String>,
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

struct Showing {
    code: String,
    long: PairCode,
    expires_at: u64,
    tries_left: u8,
    /// The exchange in progress: the joining device, the key reached, this
    /// end's frame id and the relay offered.
    pending: Option<(String, SharedKey, String, String)>,
    phase: PairPhase,
    with: Option<String>,
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
    format!("{:06}", rand::random::<u32>() % 1_000_000)
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

fn open_side(key: &SharedKey, aad: &str, text: &str) -> AppResult<End> {
    let bad = || AppError::Invalid("That code doesn't match. Check the code on the other device.".into());
    let plain = seal::open_with(&key.derive("platter-device-pair-v1 seal"), aad, text).map_err(|_| bad())?;
    serde_json::from_slice(&plain).map_err(|_| bad())
}

fn other_account() -> AppError {
    AppError::Auth("That device is signed in to a different Plex account or Home user.".into())
}

fn peer_err(e: PeerError) -> AppError {
    match e {
        PeerError::Unreachable => AppError::Network("Couldn't reach the other device.".into()),
        PeerError::Refused(m) => AppError::Auth(m),
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
        let long = showing.then(|| s.long.text());
        PairState {
            phase: s.phase,
            code: showing.then(|| s.code.clone()),
            qr: long.as_deref().and_then(qr),
            long_code: long,
            expires_at: showing.then_some(s.expires_at),
            with: s.with.clone(),
        }
    }

    fn tick(&self, s: &mut Option<Showing>) {
        let now = self.now();
        if let Some(x) = s.as_mut()
            && x.phase == PairPhase::Waiting
            && now >= x.expires_at
        {
            x.phase = PairPhase::Expired;
            x.pending = None;
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
            pending: None,
            phase: PairPhase::Waiting,
            with: None,
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

    /// The codes are used: stop showing them.
    fn done(&self, with: &str) {
        if let Some(s) = self.showing.lock().as_mut() {
            s.phase = PairPhase::Paired;
            s.with = Some(with.to_string());
            s.pending = None;
            let _ = s.stop.send(true);
        }
        self.notify();
    }

    fn add(&self, link: &Link, name: &str, version: &str, device: &str) -> AppResult<Paired> {
        let name: String = name.chars().take(60).collect();
        let name = if name.trim().is_empty() { "Your other device".to_string() } else { name };
        self.d.links.add(link, &name, &version.chars().take(20).collect::<String>(), (self.now() / 1000) as i64)?;
        let device: String = device.chars().take(100).collect();
        self.d.links.update(&link.id, |p| p.device = Some(device))?;
        Ok(Paired { id: link.id.clone(), name })
    }

    // ------------------------------------------------------------ the long code

    /// Wait on the relay for a device joining with the long code.
    async fn wait_on_relay(self: &Arc<Self>, code: PairCode, relay_url: String, mut stop: watch::Receiver<bool>) -> AppResult<()> {
        let Some(acct) = self.d.account.tag().await else { return Ok(()) };
        let relay = self.d.relays.relay(&relay_url);
        let mut offer = Offer::for_device(code.clone(), &(self.d.name)(), DeviceMe { device: self.d.own_id.clone(), acct });
        let topic = offer.listen_topic();
        loop {
            let mut rx = relay.subscribe(std::slice::from_ref(&topic), None).await.map_err(|e| AppError::Network(relay_error(e)))?;
            loop {
                let left = (i64::from(code.expires) * 1000 - self.now() as i64).max(1) as u64;
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
                                relay.publish(&wtopic, &wbody).await.map_err(|e| AppError::Network(relay_error(e)))?;
                                let p = self.add(&joined.link, &joined.name, &joined.version, &joined.device)?;
                                self.done(&p.name);
                                self.paired(&p);
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
        loop {
            match tokio::time::timeout_at(gave_up, rx.recv()).await {
                Ok(Some(RelayEvent::Message { body, .. })) if joining.is_welcome(&body, self.now()) => {
                    let d = joining.finish(&body, self.now())?;
                    return self.add(&d.link, &d.name, "", &d.device).inspect(|p| self.paired(p));
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    rx = relay.subscribe(&[joining.listen_topic()], None).await.map_err(|_| unreachable())?;
                }
                Err(_) => {
                    return Err(AppError::Network(
                        "Your other device didn't answer. Keep its code showing, check it's online, and try again.".into(),
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
        if !crypto::same(&confirmation(&key), &crypto::unhex(&reply.confirm)?) {
            return Err(AppError::Invalid("That code doesn't match. Check the code on the other device.".into()));
        }
        let theirs = open_side(&key, WELCOME_AAD, &reply.sealed)?;
        if theirs.acct != acct {
            return Err(other_account());
        }
        let frame = hex(&seal::random16()[..8]);
        let mine = End { frame: frame.clone(), name: (self.d.name)(), acct, version: self.d.version.clone(), relay: String::new() };
        let sealed = seal_side(&key, FINISH_AAD, &mine)?;
        client.pair_finish(&base, &device.id, &PairFinish { sealed }, STEP_TIMEOUT).await.map_err(peer_err)?;
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

    async fn hello(&self, from: &str, req: PairHello) -> AppResult<PairHelloReply> {
        let acct = self.d.account.tag().await.ok_or_else(|| AppError::Auth("This device isn't signed in to Plex.".into()))?;
        let msg_a = crypto::unhex(&req.msg)?;
        let frame = hex(&seal::random16()[..8]);
        let relay = self.relay();
        let mine = End { frame: frame.clone(), name: (self.d.name)(), acct, version: self.d.version.clone(), relay: relay.clone() };
        let reply = {
            let mut guard = self.showing.lock();
            self.tick(&mut guard);
            let s = guard
                .as_mut()
                .filter(|s| s.phase == PairPhase::Waiting)
                .ok_or_else(|| AppError::NotFound("That device isn't showing a code.".into()))?;
            if s.tries_left == 0 {
                s.phase = PairPhase::Locked;
                let _ = s.stop.send(true);
                return Err(AppError::Invalid("Too many wrong codes.".into()));
            }
            // Every exchange uses up a try until the pairing finishes.
            s.tries_left -= 1;
            let (msg_b, key) = crypto::receiver_answer(&password(&s.code), from, &self.d.own_id, &msg_a)?;
            let sealed = seal_side(&key, WELCOME_AAD, &mine)?;
            let reply = PairHelloReply { msg: crypto::hex(&msg_b), confirm: crypto::hex(&confirmation(&key)), sealed };
            s.pending = Some((from.to_string(), key, frame, relay));
            reply
        };
        Ok(reply)
    }

    async fn finish(&self, from: &str, req: PairFinish) -> AppResult<()> {
        let acct = self.d.account.tag().await.ok_or_else(|| AppError::Auth("This device isn't signed in to Plex.".into()))?;
        let (theirs, key, frame, relay) = {
            let mut guard = self.showing.lock();
            self.tick(&mut guard);
            let s = guard
                .as_mut()
                .filter(|s| s.phase == PairPhase::Waiting)
                .ok_or_else(|| AppError::NotFound("That device isn't showing a code.".into()))?;
            let Some((_, key, frame, relay)) = s.pending.take().filter(|(id, ..)| id == from) else {
                return Err(AppError::Invalid("Start again with the code.".into()));
            };
            match open_side(&key, FINISH_AAD, &req.sealed) {
                Ok(t) => (t, key, frame, relay),
                Err(e) => {
                    if s.tries_left == 0 {
                        s.phase = PairPhase::Locked;
                        let _ = s.stop.send(true);
                    }
                    return Err(e);
                }
            }
        };
        if theirs.acct != acct {
            return Err(other_account());
        }
        let secret = link_secret(&key);
        let link = Link { id: link_id(&secret), secret, relay, side: Side::Device, me: frame, peer: theirs.frame };
        let p = self.add(&link, &theirs.name, &theirs.version, from)?;
        self.done(&p.name);
        self.paired(&p);
        Ok(())
    }
}
