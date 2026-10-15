//! Who may control this computer: only Platter devices signed in to the
//! same Plex account.
//!
//! Both sides derive the same key from the account token
//! (`SHA-256("platter-peer-key-v1\n" + token)`); the token itself never
//! leaves the computer. Every request carries
//! `X-Platter-Auth: v1 from=<id>;ts=<unix ms>;nonce=<hex>;sig=<hex>`, where
//! `sig` is HMAC-SHA256 over the method, path, sender, receiver, time, nonce
//! and a hash of the body. The receiver accepts it only within
//! [`MAX_SKEW_MS`] of its own clock and only once per nonce.
//!
//! The account tag in the network announcement is a different hash (of the
//! account's id), so it says nothing about the key.

use std::collections::HashMap;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::plex::Token;

pub const AUTH_HEADER: &str = "x-platter-auth";
/// How far a request's time may be from this computer's clock.
pub const MAX_SKEW_MS: u64 = 30_000;
/// Nonces remembered at most (oldest dropped first).
const MAX_NONCES: usize = 4096;

const BLOCK: usize = 64;

/// HMAC-SHA256 (RFC 2104).
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new().chain_update(ipad).chain_update(msg).finalize();
    Sha256::new().chain_update(opad).chain_update(inner).finalize().into()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The shared key for one Plex account.
#[derive(Clone, PartialEq, Eq)]
pub struct PeerKey([u8; 32]);

impl std::fmt::Debug for PeerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PeerKey(..)")
    }
}

impl PeerKey {
    pub fn from_token(token: &Token) -> Self {
        let mut h = Sha256::new();
        h.update(b"platter-peer-key-v1\n");
        h.update(token.expose().as_bytes());
        Self(h.finalize().into())
    }
}

/// The account tag in the network announcement: 16 hex digits of
/// `SHA-256("platter-account-v1\n" + account id)`.
pub fn account_tag(account_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"platter-account-v1\n");
    h.update(account_id.as_bytes());
    hex(&h.finalize()[..8])
}

fn body_hash(body: &[u8]) -> String {
    hex(&Sha256::digest(body))
}

fn canonical(method: &str, path: &str, from: &str, to: &str, ts: u64, nonce: &str, body: &[u8]) -> String {
    format!("v1\n{method}\n{path}\n{from}\n{to}\n{ts}\n{nonce}\n{}", body_hash(body))
}

/// The header value for a request from `from` to `to`.
#[allow(clippy::too_many_arguments)]
pub fn sign(key: &PeerKey, method: &str, path: &str, from: &str, to: &str, ts: u64, nonce: &str, body: &[u8]) -> String {
    let sig = hmac_sha256(&key.0, canonical(method, path, from, to, ts, nonce, body).as_bytes());
    format!("v1 from={from};ts={ts};nonce={nonce};sig={}", hex(&sig))
}

/// A fresh random nonce.
pub fn nonce() -> String {
    hex(uuid::Uuid::new_v4().as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// No header, or one that doesn't parse.
    Missing,
    /// Too old or from the future.
    Stale,
    /// The same nonce again.
    Replayed,
    /// Signed with another key (another account) or for something else.
    BadSignature,
    /// This computer isn't signed in, so it has no key.
    NoKey,
}

struct Parsed<'a> {
    from: &'a str,
    ts: u64,
    nonce: &'a str,
    sig: &'a str,
}

fn parse(header: &str) -> Option<Parsed<'_>> {
    let rest = header.trim().strip_prefix("v1 ")?;
    let mut from = None;
    let mut ts = None;
    let mut nonce = None;
    let mut sig = None;
    for part in rest.split(';') {
        let (k, v) = part.trim().split_once('=')?;
        match k {
            "from" => from = Some(v),
            "ts" => ts = v.parse().ok(),
            "nonce" => nonce = Some(v),
            "sig" => sig = Some(v),
            _ => {}
        }
    }
    let p = Parsed { from: from?, ts: ts?, nonce: nonce?, sig: sig? };
    let ok_nonce = (16..=64).contains(&p.nonce.len()) && p.nonce.bytes().all(|b| b.is_ascii_hexdigit());
    (ok_nonce && !p.from.is_empty() && p.from.len() <= 128).then_some(p)
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Checks requests and remembers nonces.
#[derive(Default)]
pub struct Verifier {
    seen: Mutex<HashMap<String, u64>>,
}

impl Verifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// The sender's id when the request is good.
    #[allow(clippy::too_many_arguments)]
    pub fn verify(
        &self,
        key: Option<&PeerKey>,
        header: Option<&str>,
        method: &str,
        path: &str,
        own_id: &str,
        body: &[u8],
        now_ms: u64,
    ) -> Result<String, Refused> {
        let p = header.and_then(parse).ok_or(Refused::Missing)?;
        let key = key.ok_or(Refused::NoKey)?;
        if p.ts.abs_diff(now_ms) > MAX_SKEW_MS {
            return Err(Refused::Stale);
        }
        let want = hmac_sha256(&key.0, canonical(method, path, p.from, own_id, p.ts, p.nonce, body).as_bytes());
        if !same(hex(&want).as_bytes(), p.sig.to_ascii_lowercase().as_bytes()) {
            return Err(Refused::BadSignature);
        }
        let mut seen = self.seen.lock();
        seen.retain(|_, t| t.abs_diff(now_ms) <= 2 * MAX_SKEW_MS);
        if seen.contains_key(p.nonce) {
            return Err(Refused::Replayed);
        }
        if seen.len() >= MAX_NONCES
            && let Some(oldest) = seen.iter().min_by_key(|(_, t)| **t).map(|(k, _)| k.clone())
        {
            seen.remove(&oldest);
        }
        seen.insert(p.nonce.to_string(), now_ms);
        Ok(p.from.to_string())
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex(&mac), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        // Test case 6: a key longer than the block.
        let mac = hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First");
        assert_eq!(hex(&mac), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    fn key(t: &str) -> PeerKey {
        PeerKey::from_token(&Token::new(t).unwrap())
    }

    #[test]
    fn same_account_is_accepted_once() {
        let k = key("account-token");
        let v = Verifier::new();
        let now = 1_000_000;
        let h = sign(&k, "POST", "/platter/v1/play", "mac", "pc", now, "00112233445566778899aabbccddeeff", b"{}");
        assert_eq!(v.verify(Some(&k), Some(&h), "POST", "/platter/v1/play", "pc", b"{}", now + 500), Ok("mac".into()));
        assert_eq!(v.verify(Some(&k), Some(&h), "POST", "/platter/v1/play", "pc", b"{}", now + 600), Err(Refused::Replayed));
    }

    #[test]
    fn foreign_stale_and_altered_requests_are_refused() {
        let mine = key("account-token");
        let theirs = key("someone-else");
        let v = Verifier::new();
        let now = 5_000_000;
        let n = || nonce();
        let h = sign(&theirs, "POST", "/p", "x", "pc", now, &n(), b"");
        assert_eq!(v.verify(Some(&mine), Some(&h), "POST", "/p", "pc", b"", now), Err(Refused::BadSignature));
        let h = sign(&mine, "POST", "/p", "x", "pc", now - MAX_SKEW_MS - 1, &n(), b"");
        assert_eq!(v.verify(Some(&mine), Some(&h), "POST", "/p", "pc", b"", now), Err(Refused::Stale));
        // Another body, path or receiver than the one signed.
        let h = sign(&mine, "POST", "/p", "x", "pc", now, &n(), b"a");
        assert_eq!(v.verify(Some(&mine), Some(&h), "POST", "/p", "pc", b"b", now), Err(Refused::BadSignature));
        let h = sign(&mine, "POST", "/p", "x", "other", now, &n(), b"");
        assert_eq!(v.verify(Some(&mine), Some(&h), "POST", "/p", "pc", b"", now), Err(Refused::BadSignature));
        assert_eq!(v.verify(Some(&mine), None, "POST", "/p", "pc", b"", now), Err(Refused::Missing));
        assert_eq!(v.verify(Some(&mine), Some("v1 from=x;ts=1;nonce=zz;sig=00"), "POST", "/p", "pc", b"", now), Err(Refused::Missing));
        let h = sign(&mine, "POST", "/p", "x", "pc", now, &n(), b"");
        assert_eq!(v.verify(None, Some(&h), "POST", "/p", "pc", b"", now), Err(Refused::NoKey));
    }

    #[test]
    fn keys_and_tags_never_show_the_token() {
        assert_ne!(key("a"), key("b"));
        assert_eq!(format!("{:?}", key("secret")), "PeerKey(..)");
        let tag = account_tag("1234");
        assert_eq!(tag.len(), 16);
        assert_eq!(tag, account_tag("1234"));
        assert_ne!(tag, account_tag("1235"));
    }
}
