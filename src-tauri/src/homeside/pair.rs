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
//!
//! Two of the user's own devices pair the same way when they aren't on one
//! network ([`Offer::for_device`], [`Joining::start_device`]), under labels
//! of their own so a device code never pairs a Bridge, and the other way
//! round. Each says which Plex account it's signed in to inside the sealed
//! exchange, and a device of another account is turned away.

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
    Join {
        n: String,
        a: String,
        name: String,
        version: String,
        /// A device's id and account tag (device pairing only).
        #[serde(default, skip_serializing_if = "String::is_empty")]
        dev: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        acct: String,
    },
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
    /// The answering device's id and account tag (device pairing only).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    dev: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    acct: String,
}

/// What a pairing's channels, confirmation and link are derived under.
#[derive(Clone, Copy)]
struct Labels {
    join: &'static str,
    welcome: &'static str,
    confirm: &'static str,
    secret: &'static str,
    id: &'static str,
    /// Which end the link is for the side showing the code, and the one entering it.
    offer_side: Side,
    join_side: Side,
    /// A name to show when the other end sent none.
    offer_fallback: &'static str,
    join_fallback: &'static str,
}

const BRIDGE: Labels = Labels {
    join: "bridge pair join",
    welcome: "bridge pair welcome",
    confirm: CONFIRM_AAD,
    secret: "bridge link",
    id: "bridge link id",
    offer_side: Side::Desktop,
    join_side: Side::Bridge,
    offer_fallback: "Platter Bridge",
    join_fallback: "a desktop",
};

/// A code the Bridge shows (in its log), entered on a computer or phone:
/// the same exchange the other way round, under labels of its own so a
/// code of one kind never pairs as the other.
const BRIDGE_SHOWN: Labels = Labels {
    join: "bridge shown pair join",
    welcome: "bridge shown pair welcome",
    confirm: "platter-bridge shown welcome",
    secret: "bridge link",
    id: "bridge link id",
    offer_side: Side::Bridge,
    join_side: Side::Desktop,
    offer_fallback: "Your computer",
    join_fallback: "Platter Bridge",
};

const DEVICE: Labels = Labels {
    join: "device pair join",
    welcome: "device pair welcome",
    confirm: "platter-device welcome",
    secret: "device link",
    id: "device link id",
    offer_side: Side::Device,
    join_side: Side::Device,
    offer_fallback: "Your other device",
    join_fallback: "Your other device",
};

/// A device's own side of a device pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceMe {
    /// Its Plex client id.
    pub device: String,
    /// Its account tag; the other device must have the same.
    pub acct: String,
}

fn join_channel(l: Labels, code: &PairCode) -> Channel {
    Channel::new(&code.seed(), l.join)
}

fn welcome_channel(l: Labels, code: &PairCode) -> Channel {
    Channel::new(&code.seed(), l.welcome)
}

fn link_secret(l: Labels, key: &[u8; 32]) -> [u8; 32] {
    seal::derive(key, l.secret)
}

fn link_id(l: Labels, secret: &[u8; 32]) -> String {
    hex(&seal::derive(secret, l.id)[..8])
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
    /// The joining device's id (device pairing; empty for a Bridge).
    pub device: String,
}

/// A paired desktop, as the Bridge learns it.
#[derive(Debug, Clone)]
pub struct PairedDesktop {
    pub link: Link,
    pub name: String,
    /// The answering device's id (device pairing; empty for a desktop).
    pub device: String,
}

// ---------------------------------------------------------------- desktop

/// The desktop's side: a code, waiting for its Bridge.
pub struct Offer {
    pub code: PairCode,
    pub desktop_name: String,
    used: bool,
    labels: Labels,
    me: Option<DeviceMe>,
}

impl Offer {
    pub fn new(code: PairCode, desktop_name: &str) -> Self {
        Self { code, desktop_name: desktop_name.to_string(), used: false, labels: BRIDGE, me: None }
    }

    /// The Bridge showing a code of its own, for a computer or phone to enter.
    pub fn from_bridge(code: PairCode, bridge_name: &str) -> Self {
        Self { code, desktop_name: bridge_name.to_string(), used: false, labels: BRIDGE_SHOWN, me: None }
    }

    /// A device showing a code for another of the user's devices.
    pub fn for_device(code: PairCode, name: &str, me: DeviceMe) -> Self {
        Self { code, desktop_name: name.to_string(), used: false, labels: DEVICE, me: Some(me) }
    }

    /// Where the Bridge's join arrives.
    pub fn listen_topic(&self) -> String {
        join_channel(self.labels, &self.code).topic
    }

    /// The first good join: the welcome to publish (topic, body) and the
    /// new link. Anything else (a second join, a bad seal, an expired code)
    /// is None.
    pub fn answer(&mut self, body: &str, now_ms: u64) -> Option<((String, String), PairedBridge)> {
        if self.used || self.code.expired((now_ms / 1000) as i64) {
            return None;
        }
        let l = self.labels;
        let PairMsg::Join { n, a, name, version, dev, acct } = open(&join_channel(l, &self.code), body, now_ms)? else { return None };
        if let Some(me) = &self.me
            && (acct.is_empty() || acct != me.acct || dev.is_empty() || dev == me.device)
        {
            tracing::info!("device pairing: turned away a device of another account");
            return None;
        }
        let msg_a = seal::unb64(&a)?;
        let (msg_b, key) = seal::pake_host(&self.code.secret, &msg_a).ok()?;
        let secret = link_secret(l, &key);
        let desktop = hex(&seal::random16()[..8]);
        let bridge = hex(&seal::random16()[..8]);
        let confirm = Confirm {
            desktop: desktop.clone(),
            desktop_name: self.desktop_name.clone(),
            bridge: bridge.clone(),
            dev: self.me.as_ref().map(|m| m.device.clone()).unwrap_or_default(),
            acct: self.me.as_ref().map(|m| m.acct.clone()).unwrap_or_default(),
        };
        let c = seal::seal_with(&key, l.confirm, &serde_json::to_vec(&confirm).ok()?).ok()?;
        let ch = welcome_channel(l, &self.code);
        let body = seal::seal_value(&ch, &Sealed { x: now_ms + TTL_MS, m: PairMsg::Welcome { n, b: seal::b64(&msg_b), c } }).ok()?;
        self.used = true;
        let relay = self.code.relay.clone().unwrap_or_else(|| super::DEFAULT_RELAY.to_string());
        let link = Link { id: link_id(l, &secret), secret, relay, side: l.offer_side, me: desktop, peer: bridge };
        let name = shown_name(&name);
        let name = if name.is_empty() { l.offer_fallback.to_string() } else { name };
        let device = if self.me.is_some() { dev.chars().take(100).collect() } else { String::new() };
        Some(((ch.topic, body), PairedBridge { link, name, version: version.chars().take(20).collect(), device }))
    }
}

/// A name the other end sent, made safe to show: no control or direction
/// characters (which could make it read as another name), at most 60.
fn shown_name(name: &str) -> String {
    let bidi = |c: char| matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{061c}');
    let kept: String = name.chars().filter(|c| !c.is_control() && !bidi(*c)).take(60).collect();
    kept.trim().to_string()
}


// ---------------------------------------------------------------- bridge

/// The Bridge's side, after publishing its join.
pub struct Joining {
    code: PairCode,
    pake: PakeGuest,
    nonce: String,
    labels: Labels,
    me: Option<DeviceMe>,
}

impl Joining {
    /// Start: the join to publish (topic, body).
    pub fn start(code: PairCode, name: &str, version: &str, now_ms: u64) -> AppResult<(Self, (String, String))> {
        if code.expired((now_ms / 1000) as i64) {
            return Err(AppError::Invalid("This pair code has expired. Make a new one in Platter (Settings > Bridge > Pair a Bridge).".into()));
        }
        Self::begin(code, name, version, now_ms, BRIDGE, None)
    }

    /// A computer or phone entering the code a Bridge showed.
    pub fn start_to_bridge(code: PairCode, name: &str, version: &str, now_ms: u64) -> AppResult<(Self, (String, String))> {
        if code.expired((now_ms / 1000) as i64) {
            return Err(AppError::Invalid("This Bridge code has expired. While it isn't paired, the Bridge shows a new one in its log every hour.".into()));
        }
        Self::begin(code, name, version, now_ms, BRIDGE_SHOWN, None)
    }

    /// A device joining another of the user's devices with its long code.
    pub fn start_device(code: PairCode, name: &str, version: &str, me: DeviceMe, now_ms: u64) -> AppResult<(Self, (String, String))> {
        if code.expired((now_ms / 1000) as i64) {
            return Err(AppError::Invalid("This code has expired. Show a new one on your other device.".into()));
        }
        Self::begin(code, name, version, now_ms, DEVICE, Some(me))
    }

    fn begin(code: PairCode, name: &str, version: &str, now_ms: u64, labels: Labels, me: Option<DeviceMe>) -> AppResult<(Self, (String, String))> {
        let (pake, msg_a) = seal::pake_guest_start(&code.secret);
        let nonce = hex(&seal::random16()[..8]);
        let ch = join_channel(labels, &code);
        let m = PairMsg::Join {
            n: nonce.clone(),
            a: seal::b64(&msg_a),
            name: name.to_string(),
            version: version.to_string(),
            dev: me.as_ref().map(|m| m.device.clone()).unwrap_or_default(),
            acct: me.as_ref().map(|m| m.acct.clone()).unwrap_or_default(),
        };
        let body = seal::seal_value(&ch, &Sealed { x: now_ms + TTL_MS, m })?;
        Ok((Self { code, pake, nonce, labels, me }, (ch.topic, body)))
    }

    /// Where the desktop's welcome arrives.
    pub fn listen_topic(&self) -> String {
        welcome_channel(self.labels, &self.code).topic
    }

    /// Whether `body` is the welcome to this join (without using it up).
    pub fn is_welcome(&self, body: &str, now_ms: u64) -> bool {
        matches!(open(&welcome_channel(self.labels, &self.code), body, now_ms), Some(PairMsg::Welcome { n, .. }) if n == self.nonce)
    }

    /// Finish with the desktop's welcome.
    pub fn finish(self, body: &str, now_ms: u64) -> AppResult<PairedDesktop> {
        let bad = || AppError::Invalid("The answer from Platter couldn't be opened. Make a new pair code and try again.".into());
        let l = self.labels;
        let Some(PairMsg::Welcome { n, b, c }) = open(&welcome_channel(l, &self.code), body, now_ms) else { return Err(bad()) };
        if n != self.nonce {
            return Err(bad());
        }
        let key = seal::pake_guest_finish(self.pake, &seal::unb64(&b).ok_or_else(bad)?)?;
        let confirm: Confirm = serde_json::from_slice(&seal::open_with(&key, l.confirm, &c).map_err(|_| bad())?).map_err(|_| bad())?;
        if let Some(me) = &self.me
            && (confirm.acct.is_empty() || confirm.acct != me.acct)
        {
            return Err(AppError::Auth("That device is signed in to a different Plex account or Home user.".into()));
        }
        let secret = link_secret(l, &key);
        let relay = self.code.relay.clone().unwrap_or_else(|| super::DEFAULT_RELAY.to_string());
        let link = Link { id: link_id(l, &secret), secret, relay, side: l.join_side, me: confirm.bridge, peer: confirm.desktop };
        let name = shown_name(&confirm.desktop_name);
        let name = if name.is_empty() { l.join_fallback.to_string() } else { name };
        let device = if self.me.is_some() { confirm.dev.chars().take(100).collect() } else { String::new() };
        Ok(PairedDesktop { link, name, device })
    }
}

#[cfg(test)]
mod shown_name_tests {
    #[test]
    fn names_lose_control_and_direction_characters() {
        assert_eq!(super::shown_name("Adam\u{202e}s PC\n"), "Adams PC");
        assert_eq!(super::shown_name(" \u{200f} "), "");
        assert_eq!(super::shown_name(&"x".repeat(80)).len(), 60);
    }
}
