//! The pair code: what Settings > Bridge > "Pair a Bridge" shows, and what
//! the Bridge takes once from `BRIDGE_PAIR_CODE`.
//!
//! `PB1-` then base64url of
//!
//! ```text
//! session id (16) ‖ secret (16) ‖ expiry (u32 BE, unix s) ‖ [relay URL]
//! ```
//!
//! Everything is random except the expiry, and the relay URL, which is
//! there only when the user runs their own. No address, key or token is
//! ever in it. The secret is never sent anywhere: both sides prove they
//! know it with a key agreement over the relay (see `pair`).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

use crate::remote::peer::auth::{hex, hmac_sha256};
use crate::social::listen::invite::relay_url;

pub const PREFIX: &str = "PB1-";
const FIXED: usize = 16 + 16 + 4;
/// How long a code can be used.
pub const VALID_S: u32 = 60 * 60;

#[derive(Clone, PartialEq, Eq)]
pub struct PairCode {
    pub session: [u8; 16],
    pub secret: [u8; 16],
    /// Unix seconds.
    pub expires: u32,
    /// The relay, when it isn't the default one.
    pub relay: Option<String>,
}

impl std::fmt::Debug for PairCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairCode(..)")
    }
}

impl PairCode {
    /// A new code good for an hour from `now_s`.
    pub fn new(relay: Option<String>, now_s: i64) -> Self {
        Self { session: rand::random(), secret: rand::random(), expires: (now_s.max(0) as u32).saturating_add(VALID_S), relay }
    }

    /// What the pairing channels are derived from.
    pub fn seed(&self) -> Vec<u8> {
        [self.session.as_slice(), self.secret.as_slice()].concat()
    }

    /// A stand-in to remember a used code by, without keeping the code.
    pub fn fingerprint(&self) -> String {
        hex(&hmac_sha256(&self.seed(), b"platter-bridge used code")[..12])
    }

    pub fn expired(&self, now_s: i64) -> bool {
        now_s > i64::from(self.expires)
    }

    pub fn text(&self) -> String {
        let mut b = Vec::with_capacity(FIXED + 64);
        b.extend_from_slice(&self.session);
        b.extend_from_slice(&self.secret);
        b.extend_from_slice(&self.expires.to_be_bytes());
        if let Some(r) = &self.relay {
            b.extend_from_slice(r.as_bytes());
        }
        format!("{PREFIX}{}", B64.encode(b))
    }

    /// Read a code (spaces and line breaks from copying are ignored).
    pub fn parse(raw: &str) -> Option<Self> {
        let raw: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
        let body = raw.strip_prefix(PREFIX)?;
        if body.len() > 400 {
            return None;
        }
        let b = B64.decode(body).ok()?;
        if b.len() < FIXED {
            return None;
        }
        let session: [u8; 16] = b[0..16].try_into().ok()?;
        let secret: [u8; 16] = b[16..32].try_into().ok()?;
        let expires = u32::from_be_bytes(b[32..36].try_into().ok()?);
        let relay = if b.len() > FIXED {
            let s = std::str::from_utf8(&b[FIXED..]).ok()?;
            Some(relay_url(s).filter(|r| r == s)?)
        } else {
            None
        };
        Some(Self { session, secret, expires, relay })
    }
}
