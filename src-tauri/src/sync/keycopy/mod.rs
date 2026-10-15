//! Copying keys between devices. Never through the sync file: two Platters
//! signed in to the same Plex account talk directly on the local network.
//!
//! The device that needs keys shows a 6-digit code (and the code as a QR
//! code) for 5 minutes. On a device that has them, the user types the code
//! and ticks which keys to send. The sender finds the device of this
//! account that's waiting, the two run a SPAKE2 exchange over the code
//! ([`crypto`]), the receiver proves it reached the same key, and only then
//! are the ticked keys sent, sealed. The receiver puts them in its keychain
//! through the features that use them. A wrong code fails the exchange; the
//! receiver allows [`TRIES`] attempts per code.
//!
//! Keys are never logged and never written anywhere but the keychain.

pub mod crypto;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::remote::peer::PeerDevice;
use crate::remote::peer::client::{PeerClient, PeerError};
use crate::remote::peer::keys::{KeysDeliver, KeysHello, KeysHelloReply, PeerKeys};
use crate::sync::store::Clock;

/// A code lasts this long.
pub const CODE_TTL_MS: u64 = 5 * 60 * 1000;
/// Wrong codes allowed before the code stops working.
pub const TRIES: u8 = 5;
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// The keys that can be copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyKind {
    /// ListenBrainz user token.
    Listenbrainz,
    /// Last.fm API key.
    Lastfm,
    /// Fanart.tv API key.
    Fanart,
    /// TheAudioDB API key.
    Audiodb,
    /// Imgur client id (Discord covers).
    Imgur,
    /// The catbox.moe upload choice (not a secret, but it goes with the keys).
    Catbox,
}

impl KeyKind {
    pub const ALL: [KeyKind; 6] = [Self::Listenbrainz, Self::Lastfm, Self::Fanart, Self::Audiodb, Self::Imgur, Self::Catbox];
}

/// A key's value. Never printed.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// Where keys are read from and stored (the keychain, through the features
/// that use them).
#[async_trait]
pub trait KeyVault: Send + Sync {
    /// The keys this device has.
    async fn available(&self) -> Vec<KeyKind>;
    async fn read(&self, kind: KeyKind) -> Option<Secret>;
    async fn store(&self, kind: KeyKind, value: Secret) -> AppResult<()>;
}

/// What the receiving dialog shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceivePhase {
    /// No code showing.
    #[default]
    Idle,
    /// Showing a code.
    Waiting,
    /// A device is sending.
    Receiving,
    Received,
    Expired,
    /// Too many wrong codes; this one stopped working.
    Locked,
}

/// A QR code as rows of modules ('1' dark).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Qr {
    pub size: usize,
    pub rows: Vec<String>,
}

/// The QR code for a code: a `platter://copy-keys/<code>` link.
pub fn qr(code: &str) -> Option<Qr> {
    let q = qrcode::QrCode::with_error_correction_level(format!("platter://copy-keys/{code}"), qrcode::EcLevel::M).ok()?;
    let size = q.width();
    let colors = q.to_colors();
    let rows = colors
        .chunks(size)
        .map(|r| r.iter().map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' }).collect())
        .collect();
    Some(Qr { size, rows })
}

/// For the receiving dialog (`keys_receive_*`, `keys://receive`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiveState {
    pub phase: ReceivePhase,
    pub code: Option<String>,
    pub qr: Option<Qr>,
    pub expires_at: Option<u64>,
    /// The sending device's name.
    pub from: Option<String>,
    /// What arrived.
    pub received: Vec<KeyKind>,
}

pub trait KeyEvents: Send + Sync {
    fn receive_state(&self, s: &ReceiveState);
}

struct Session {
    code: String,
    expires_at: u64,
    tries_left: u8,
    /// The exchange in progress: sender id, its name, and the key reached.
    pending: Option<(String, String, crypto::SharedKey)>,
    phase: ReceivePhase,
    from: Option<String>,
    received: Vec<KeyKind>,
}

/// The receiving side.
#[derive(Clone)]
pub struct KeyReceiver {
    inner: Arc<RInner>,
}

struct RInner {
    own_id: String,
    vault: Arc<dyn KeyVault>,
    clock: Clock,
    session: Mutex<Option<Session>>,
    events: Mutex<Option<Arc<dyn KeyEvents>>>,
}

/// The sealed message.
#[derive(Serialize, Deserialize)]
struct Bundle {
    keys: Vec<(KeyKind, Secret)>,
}

fn new_code() -> String {
    format!("{:06}", rand::random::<u32>() % 1_000_000)
}

impl KeyReceiver {
    pub fn new(own_id: String, vault: Arc<dyn KeyVault>, clock: Clock) -> Self {
        Self {
            inner: Arc::new(RInner { own_id, vault, clock, session: Mutex::new(None), events: Mutex::new(None) }),
        }
    }

    pub fn set_events(&self, events: Arc<dyn KeyEvents>) {
        *self.inner.events.lock() = Some(events);
    }

    fn view(s: Option<&Session>) -> ReceiveState {
        match s {
            None => ReceiveState::default(),
            Some(s) => {
                let showing = matches!(s.phase, ReceivePhase::Waiting | ReceivePhase::Receiving);
                ReceiveState {
                    phase: s.phase,
                    code: showing.then(|| s.code.clone()),
                    qr: if showing { qr(&s.code) } else { None },
                    expires_at: showing.then_some(s.expires_at),
                    from: s.from.clone(),
                    received: s.received.clone(),
                }
            }
        }
    }

    /// Expire a session past its time.
    fn tick(&self, s: &mut Option<Session>) {
        let now = (self.inner.clock)();
        if let Some(x) = s.as_mut()
            && matches!(x.phase, ReceivePhase::Waiting | ReceivePhase::Receiving)
            && now >= x.expires_at
        {
            x.phase = ReceivePhase::Expired;
            x.pending = None;
        }
    }

    fn notify(&self, st: &ReceiveState) {
        if let Some(ev) = self.inner.events.lock().clone() {
            ev.receive_state(st);
        }
    }

    /// Show a new code (any earlier one stops working).
    pub fn start(&self) -> ReceiveState {
        let st = {
            let mut s = self.inner.session.lock();
            *s = Some(Session {
                code: new_code(),
                expires_at: (self.inner.clock)() + CODE_TTL_MS,
                tries_left: TRIES,
                pending: None,
                phase: ReceivePhase::Waiting,
                from: None,
                received: Vec::new(),
            });
            Self::view(s.as_ref())
        };
        self.notify(&st);
        st
    }

    pub fn cancel(&self) {
        *self.inner.session.lock() = None;
        self.notify(&ReceiveState::default());
    }

    pub fn state(&self) -> ReceiveState {
        let mut s = self.inner.session.lock();
        self.tick(&mut s);
        Self::view(s.as_ref())
    }
}

#[async_trait]
impl PeerKeys for KeyReceiver {
    async fn waiting(&self) -> bool {
        self.state().phase == ReceivePhase::Waiting
    }

    async fn hello(&self, from: &str, req: KeysHello) -> AppResult<KeysHelloReply> {
        let msg_a = crypto::unhex(&req.msg)?;
        let mut guard = self.inner.session.lock();
            self.tick(&mut guard);
            let s = guard
                .as_mut()
                .filter(|s| s.phase == ReceivePhase::Waiting)
                .ok_or_else(|| AppError::NotFound("That device isn't waiting for keys.".into()))?;
            if s.tries_left == 0 {
                s.phase = ReceivePhase::Locked;
                return Err(AppError::Invalid("Too many wrong codes.".into()));
            }
            // Every exchange uses up a try until keys arrive.
            s.tries_left -= 1;
            let (msg_b, key) = crypto::receiver_answer(&s.code, from, &self.inner.own_id, &msg_a)?;
            let confirm = key.confirmation();
            let name: String = req.name.chars().take(60).collect();
            s.pending = Some((from.to_string(), name, key));
        Ok(KeysHelloReply { msg: crypto::hex(&msg_b), confirm: crypto::hex(&confirm) })
    }

    async fn deliver(&self, from: &str, req: KeysDeliver) -> AppResult<()> {
        let nonce = crypto::unhex(&req.nonce)?;
        let sealed = crypto::unhex(&req.sealed)?;
        let (keys, name) = {
            let mut guard = self.inner.session.lock();
            self.tick(&mut guard);
            let s = guard
                .as_mut()
                .filter(|s| s.phase == ReceivePhase::Waiting)
                .ok_or_else(|| AppError::NotFound("That device isn't waiting for keys.".into()))?;
            let Some((sender, name, key)) = s.pending.take().filter(|(id, _, _)| id == from) else {
                return Err(AppError::Invalid("Start again with the code.".into()));
            };
            let plain = match crypto::open(&key, &sender, &self.inner.own_id, &nonce, &sealed) {
                Ok(p) => p,
                Err(e) => {
                    if s.tries_left == 0 {
                        s.phase = ReceivePhase::Locked;
                    }
                    return Err(e);
                }
            };
            let bundle: Bundle = serde_json::from_slice(&plain).map_err(|_| AppError::Invalid("Unreadable keys.".into()))?;
            s.phase = ReceivePhase::Receiving;
            s.from = Some(name.clone());
            (bundle.keys, name)
        };
        self.notify(&self.state());
        let mut got = Vec::new();
        let mut failed = None;
        for (kind, value) in keys {
            match self.inner.vault.store(kind, value).await {
                Ok(()) => got.push(kind),
                Err(e) => {
                    tracing::warn!(?kind, "couldn't store a copied key: {}", e.kind());
                    failed = Some(e);
                }
            }
        }
        let st = {
            let mut guard = self.inner.session.lock();
            if let Some(s) = guard.as_mut() {
                s.phase = ReceivePhase::Received;
                s.received = got.clone();
                s.from = Some(name);
            }
            Self::view(guard.as_ref())
        };
        tracing::info!(count = got.len(), "keys copied from another device");
        self.notify(&st);
        match failed {
            Some(e) if got.is_empty() => Err(e),
            _ => Ok(()),
        }
    }
}

/// What a send did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOutcome {
    pub device: String,
    pub sent: Vec<KeyKind>,
}

/// Is the code 6 digits?
pub fn valid_code(code: &str) -> Option<String> {
    let c: String = code.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    (c.len() == 6 && c.bytes().all(|b| b.is_ascii_digit())).then_some(c)
}

fn peer_err(e: PeerError) -> AppError {
    match e {
        PeerError::Unreachable => AppError::Network("Couldn't reach the other device.".into()),
        PeerError::Refused(m) => AppError::Auth(m),
        PeerError::Failed(m) => AppError::Invalid(m),
    }
}

/// Send the ticked keys to the device of this account showing `code`.
pub async fn send(
    client: &PeerClient,
    own_name: &str,
    devices: Vec<PeerDevice>,
    code: &str,
    kinds: &[KeyKind],
    vault: &dyn KeyVault,
) -> AppResult<SendOutcome> {
    let code = valid_code(code).ok_or_else(|| AppError::Invalid("The code is 6 digits.".into()))?;
    let mut keys = Vec::new();
    for k in KeyKind::ALL.iter().filter(|k| kinds.contains(k)) {
        if let Some(v) = vault.read(*k).await {
            keys.push((*k, v));
        }
    }
    if keys.is_empty() {
        return Err(AppError::Invalid("Choose at least one key this computer has.".into()));
    }
    // Which device is showing a code?
    let asks = devices.iter().map(|d| async move {
        for b in &d.bases {
            if let Ok(true) = client.keys_waiting(b, &d.id, STEP_TIMEOUT).await {
                return Some((d.clone(), b.clone()));
            }
        }
        None
    });
    let waiting: Vec<_> = futures::future::join_all(asks).await.into_iter().flatten().collect();
    let (device, base) = match waiting.len() {
        0 => {
            return Err(AppError::NotFound(
                "No device is showing a code. On the other computer, open Settings and choose Copy keys from another device.".into(),
            ));
        }
        1 => waiting.into_iter().next().expect("one"),
        _ => return Err(AppError::Invalid("More than one device is showing a code. Close the others and try again.".into())),
    };
    let me = client.own_id().to_string();
    let (state, msg_a) = crypto::sender_start(&code, &me, &device.id);
    let reply = client
        .keys_hello(&base, &device.id, &KeysHello { msg: crypto::hex(&msg_a), name: own_name.to_string() }, STEP_TIMEOUT)
        .await
        .map_err(peer_err)?;
    let key = crypto::sender_finish(state, &crypto::unhex(&reply.msg)?)?;
    if !crypto::same(&key.confirmation(), &crypto::unhex(&reply.confirm)?) {
        return Err(AppError::Invalid("That code doesn't match. Check the code on the other computer.".into()));
    }
    let plain = serde_json::to_vec(&Bundle { keys: keys.clone() }).map_err(|e| AppError::Internal(e.to_string()))?;
    let (nonce, sealed) = crypto::seal(&key, &me, &device.id, &plain)?;
    client
        .keys_deliver(&base, &device.id, &KeysDeliver { nonce: crypto::hex(&nonce), sealed: crypto::hex(&sealed) }, STEP_TIMEOUT)
        .await
        .map_err(peer_err)?;
    tracing::info!(count = keys.len(), "keys sent to another device");
    Ok(SendOutcome { device: device.name, sent: keys.into_iter().map(|(k, _)| k).collect() })
}
