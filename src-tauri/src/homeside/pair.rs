//! Pairing a Bridge with a desktop, once, with the code the desktop shows.
//!
//! 1. The desktop makes a [`PairCode`] and listens on the pairing topic
//!    derived from it.
//! 2. The Bridge, given the code, starts a SPAKE2 exchange over the code's
//!    secret and publishes its half, sealed for the pairing channel ([`Join`]).
//! 3. The desktop answers the first join only with its half and a
//!    confirmation sealed with the agreed key ([`Offer::answer`]); then it
//!    forgets the code.
//! 4. The Bridge finishes the exchange and opens the confirmation
//!    ([`Joining::finish`]). Both now hold the link secret, which neither
//!    the relay nor anyone who later sees the code can work out.

use serde::{Deserialize, Serialize};

use super::code::PairCode;
use super::link::{Link, Side};
use crate::error::{AppError, AppResult};
use crate::remote::peer::auth::hex;
use crate::social::listen::seal::{self, Channel, PakeGuest};

/// How long a pairing message counts.
const TTL_MS: u64 = 15 * 60_000;
const CONFIRM_AAD: &str = "platter-bridge welcome";

/// What goes over the pairing channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum PairMsg {
    Join { n: String, a: String, name: String, version: String },
    Welcome { n: String, b: String, c: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Sealed {
    x: u64,
    m: PairMsg,
}

/// Opened with the agreed key: who's who on the link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Confirm {
    desktop: String,
    desktop_name: String,
    bridge: String,
}

fn join_channel(code: &PairCode) -> Channel {
    Channel::new(&code.seed(), "bridge pair join")
}

fn welcome_channel(code: &PairCode) -> Channel {
    Channel::new(&code.seed(), "bridge pair welcome")
}

fn link_secret(key: &[u8; 32]) -> [u8; 32] {
    seal::derive(key, "bridge link")
}

fn link_id(secret: &[u8; 32]) -> String {
    hex(&seal::derive(secret, "bridge link id")[..8])
}

fn open(ch: &Channel, body: &str, now_ms: u64) -> Option<PairMsg> {
    let s: Sealed = seal::open_value(ch, body).ok()?;
    (s.x >= now_ms && s.x <= now_ms + 2 * TTL_MS).then_some(s.m)
}

/// A paired Bridge, as the desktop learns it.
#[derive(Debug, Clone)]
pub struct PairedBridge {
    pub link: Link,
    pub name: String,
    pub version: String,
}

/// A paired desktop, as the Bridge learns it.
#[derive(Debug, Clone)]
pub struct PairedDesktop {
    pub link: Link,
    pub name: String,
}

// ---------------------------------------------------------------- desktop

/// The desktop's side: a code, waiting for its Bridge.
pub struct Offer {
    pub code: PairCode,
    pub desktop_name: String,
    used: bool,
}

impl Offer {
    pub fn new(code: PairCode, desktop_name: &str) -> Self {
        Self { code, desktop_name: desktop_name.to_string(), used: false }
    }

    /// Where the Bridge's join arrives.
    pub fn listen_topic(&self) -> String {
        join_channel(&self.code).topic
    }

    /// The first good join: the welcome to publish (topic, body) and the
    /// new link. Anything else (a second join, a bad seal, an expired code)
    /// is None.
    pub fn answer(&mut self, body: &str, now_ms: u64) -> Option<((String, String), PairedBridge)> {
        if self.used || self.code.expired((now_ms / 1000) as i64) {
            return None;
        }
        let PairMsg::Join { n, a, name, version } = open(&join_channel(&self.code), body, now_ms)? else { return None };
        let msg_a = seal::unb64(&a)?;
        let (msg_b, key) = seal::pake_host(&self.code.secret, &msg_a).ok()?;
        let secret = link_secret(&key);
        let desktop = hex(&seal::random16()[..8]);
        let bridge = hex(&seal::random16()[..8]);
        let confirm = Confirm { desktop: desktop.clone(), desktop_name: self.desktop_name.clone(), bridge };
        let c = seal::seal_with(&key, CONFIRM_AAD, &serde_json::to_vec(&confirm).ok()?).ok()?;
        let ch = welcome_channel(&self.code);
        let body = seal::seal_value(&ch, &Sealed { x: now_ms + TTL_MS, m: PairMsg::Welcome { n, b: seal::b64(&msg_b), c } }).ok()?;
        self.used = true;
        let relay = self.code.relay.clone().unwrap_or_else(|| super::DEFAULT_RELAY.to_string());
        let link = Link { id: link_id(&secret), secret, relay, side: Side::Desktop, me: desktop };
        let name = if name.trim().is_empty() { "Platter Bridge".to_string() } else { name.chars().take(60).collect() };
        Some(((ch.topic, body), PairedBridge { link, name, version: version.chars().take(20).collect() }))
    }
}

// ---------------------------------------------------------------- bridge

/// The Bridge's side, after publishing its join.
pub struct Joining {
    code: PairCode,
    pake: PakeGuest,
    nonce: String,
}

impl Joining {
    /// Start: the join to publish (topic, body).
    pub fn start(code: PairCode, name: &str, version: &str, now_ms: u64) -> AppResult<(Self, (String, String))> {
        if code.expired((now_ms / 1000) as i64) {
            return Err(AppError::Invalid("This pair code has expired. Make a new one in Platter (Settings > Bridge > Pair a Bridge).".into()));
        }
        let (pake, msg_a) = seal::pake_guest_start(&code.secret);
        let nonce = hex(&seal::random16()[..8]);
        let ch = join_channel(&code);
        let m = PairMsg::Join { n: nonce.clone(), a: seal::b64(&msg_a), name: name.to_string(), version: version.to_string() };
        let body = seal::seal_value(&ch, &Sealed { x: now_ms + TTL_MS, m })?;
        Ok((Self { code, pake, nonce }, (ch.topic, body)))
    }

    /// Where the desktop's welcome arrives.
    pub fn listen_topic(&self) -> String {
        welcome_channel(&self.code).topic
    }

    /// Whether `body` is the welcome to this join (without using it up).
    pub fn is_welcome(&self, body: &str, now_ms: u64) -> bool {
        matches!(open(&welcome_channel(&self.code), body, now_ms), Some(PairMsg::Welcome { n, .. }) if n == self.nonce)
    }

    /// Finish with the desktop's welcome.
    pub fn finish(self, body: &str, now_ms: u64) -> AppResult<PairedDesktop> {
        let bad = || AppError::Invalid("The answer from Platter couldn't be opened. Make a new pair code and try again.".into());
        let Some(PairMsg::Welcome { n, b, c }) = open(&welcome_channel(&self.code), body, now_ms) else { return Err(bad()) };
        if n != self.nonce {
            return Err(bad());
        }
        let key = seal::pake_guest_finish(self.pake, &seal::unb64(&b).ok_or_else(bad)?)?;
        let confirm: Confirm = serde_json::from_slice(&seal::open_with(&key, CONFIRM_AAD, &c).map_err(|_| bad())?).map_err(|_| bad())?;
        let secret = link_secret(&key);
        let relay = self.code.relay.clone().unwrap_or_else(|| super::DEFAULT_RELAY.to_string());
        let link = Link { id: link_id(&secret), secret, relay, side: Side::Bridge, me: confirm.bridge };
        let name = if confirm.desktop_name.trim().is_empty() { "a desktop".to_string() } else { confirm.desktop_name.chars().take(60).collect() };
        Ok(PairedDesktop { link, name })
    }
}
