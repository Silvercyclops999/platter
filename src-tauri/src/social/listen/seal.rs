//! Sealing listen along messages so the relay only ever sees ciphertext.
//!
//! Every conversation direction is a [`Channel`]: a topic name and a key,
//! both derived (HMAC-SHA256 with a label) from a random secret only the
//! participants hold. The topic is 160 bits of that derivation as hex, so it
//! can't be guessed and says nothing about the secret.
//!
//! On the wire a message is `1.<base64url(nonce ‖ ciphertext)>`, sealed with
//! ChaCha20-Poly1305 and the topic bound in as associated data, so a message
//! copied to another topic doesn't open. Inside is an [`Envelope`]: who sent
//! it (role and id), a counter that only goes up, and an expiry. Receivers
//! drop anything expired, and anything whose counter isn't above the last
//! one seen from that sender on that channel ([`ReplayGuard`]).
//!
//! Host broadcasts go to every guest under one key, so a guest could seal a
//! message that looks like the host's. To stop that, broadcasts carry a
//! third part, `.<base64url(tags)>`: one 128-bit HMAC tag per guest, made with
//! a key only the host and that guest share. A guest accepts a broadcast only
//! when one of the tags is the one it computes itself.
//!
//! Joining uses SPAKE2 over the invite's secret ([`pake_guest_start`],
//! [`pake_host`]): the first person to answer the invite and the host end up
//! with a key nobody else who saw (or later gets) the invite can work out.

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use spake2::{Ed25519Group, Identity, Password, Spake2};

use super::wire::Msg;
use crate::error::{AppError, AppResult};
use crate::remote::peer::auth::{hex, hmac_sha256};

const LABEL: &str = "platter-listen-v1";
/// ntfy.sh takes message bodies up to 4096 bytes; stay under it.
pub const MAX_BODY: usize = 4000;
/// How long a sealed message counts.
pub const TTL_MS: u64 = 5 * 60_000;
/// A message may not claim to count for longer than this (a wrong clock).
const MAX_AHEAD_MS: u64 = 15 * 60_000;
const TAG_LEN: usize = 16;

/// 32 random bytes from the system's generator.
pub fn random32() -> [u8; 32] {
    rand::random()
}

pub fn random16() -> [u8; 16] {
    rand::random()
}

/// A labelled key from a secret.
pub fn derive(secret: &[u8], label: &str) -> [u8; 32] {
    hmac_sha256(secret, format!("{LABEL} {label}").as_bytes())
}

/// One direction of one conversation: where it's published, and its key.
#[derive(Clone, PartialEq, Eq)]
pub struct Channel {
    pub topic: String,
    key: [u8; 32],
}

impl std::fmt::Debug for Channel {
    // Neither the topic nor the key ever goes in a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Channel(..)")
    }
}

impl Channel {
    pub fn new(secret: &[u8], name: &str) -> Self {
        let t = derive(secret, &format!("{name} topic"));
        Self { topic: hex(&t[..20]), key: derive(secret, &format!("{name} key")) }
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        let k: Key = self.key.into();
        ChaCha20Poly1305::new(&k)
    }

    fn aad(&self) -> Vec<u8> {
        format!("{LABEL}\n{}", self.topic).into_bytes()
    }
}

/// Who sent a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sender {
    #[serde(rename = "h")]
    Host,
    #[serde(rename = "g")]
    Guest,
}

/// What's inside a sealed message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// Host or guest.
    pub r: Sender,
    /// The sender's id in the session ("host", a guest's random id, or a
    /// join attempt's nonce).
    pub f: String,
    /// Counts up with every message the sender seals.
    pub c: u64,
    /// Unix ms after which it no longer counts.
    pub x: u64,
    pub m: Msg,
}

/// Why a message was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    Malformed,
    /// Wrong key, wrong topic, or a changed byte.
    BadSeal,
    /// A broadcast without this guest's tag.
    Forged,
    Expired,
    Replayed,
}

/// Seal for a channel. `tag_keys`: one per guest for a broadcast, else empty.
pub fn seal(ch: &Channel, env: &Envelope, tag_keys: &[[u8; 32]]) -> AppResult<String> {
    let plain = serde_json::to_vec(env).map_err(|e| AppError::Internal(format!("seal: {e}")))?;
    let nonce: [u8; 12] = rand::random();
    let ct = ch
        .cipher()
        .encrypt(&Nonce::from(nonce), Payload { msg: &plain, aad: &ch.aad() })
        .map_err(|_| AppError::Internal("couldn't seal".into()))?;
    let mut body = Vec::with_capacity(12 + ct.len());
    body.extend_from_slice(&nonce);
    body.extend_from_slice(&ct);
    let mut out = format!("1.{}", B64.encode(body));
    if !tag_keys.is_empty() {
        let mut tags = Vec::with_capacity(tag_keys.len() * TAG_LEN);
        for k in tag_keys {
            tags.extend_from_slice(&hmac_sha256(k, out.as_bytes())[..TAG_LEN]);
        }
        out.push('.');
        out.push_str(&B64.encode(tags));
    }
    Ok(out)
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Open what [`seal`] made. `tag_key`: this guest's, for a broadcast.
pub fn open(ch: &Channel, text: &str, now_ms: u64, tag_key: Option<&[u8; 32]>) -> Result<Envelope, Rejected> {
    if text.len() > 64 * 1024 {
        return Err(Rejected::Malformed);
    }
    let mut parts = text.trim().split('.');
    let (Some("1"), Some(sealed)) = (parts.next(), parts.next()) else { return Err(Rejected::Malformed) };
    let tags = parts.next();
    if parts.next().is_some() {
        return Err(Rejected::Malformed);
    }
    if let Some(k) = tag_key {
        let tags = B64.decode(tags.ok_or(Rejected::Forged)?).map_err(|_| Rejected::Forged)?;
        let want = &hmac_sha256(k, format!("1.{sealed}").as_bytes())[..TAG_LEN];
        if !tags.chunks(TAG_LEN).fold(false, |found, t| found | same(t, want)) {
            return Err(Rejected::Forged);
        }
    }
    let raw = B64.decode(sealed).map_err(|_| Rejected::Malformed)?;
    if raw.len() < 12 + 16 {
        return Err(Rejected::Malformed);
    }
    let (nonce, ct) = raw.split_at(12);
    let nonce: [u8; 12] = nonce.try_into().map_err(|_| Rejected::Malformed)?;
    let plain = ch
        .cipher()
        .decrypt(&Nonce::from(nonce), Payload { msg: ct, aad: &ch.aad() })
        .map_err(|_| Rejected::BadSeal)?;
    let env: Envelope = serde_json::from_slice(&plain).map_err(|_| Rejected::Malformed)?;
    if env.x < now_ms || env.x > now_ms + MAX_AHEAD_MS {
        return Err(Rejected::Expired);
    }
    Ok(env)
}

/// The last counter seen per sender and channel; anything not above it is a
/// replay.
#[derive(Default)]
pub struct ReplayGuard {
    last: HashMap<String, u64>,
}

impl ReplayGuard {
    pub fn accept(&mut self, who: &str, counter: u64) -> Result<(), Rejected> {
        let last = self.last.entry(who.to_string()).or_insert(0);
        if counter <= *last {
            return Err(Rejected::Replayed);
        }
        *last = counter;
        Ok(())
    }

    pub fn forget(&mut self, who: &str) {
        self.last.remove(who);
    }
}

// ---- sealing with a plain key (the join answer) -------------------------------

pub fn seal_with(key: &[u8; 32], aad: &str, plain: &[u8]) -> AppResult<String> {
    let k: Key = (*key).into();
    let nonce: [u8; 12] = rand::random();
    let ct = ChaCha20Poly1305::new(&k)
        .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad: aad.as_bytes() })
        .map_err(|_| AppError::Internal("couldn't seal".into()))?;
    let mut body = nonce.to_vec();
    body.extend_from_slice(&ct);
    Ok(B64.encode(body))
}

pub fn open_with(key: &[u8; 32], aad: &str, text: &str) -> AppResult<Vec<u8>> {
    let bad = || AppError::Invalid("The host's answer couldn't be opened.".into());
    let raw = B64.decode(text).map_err(|_| bad())?;
    if raw.len() < 12 + 16 {
        return Err(bad());
    }
    let (nonce, ct) = raw.split_at(12);
    let nonce: [u8; 12] = nonce.try_into().map_err(|_| bad())?;
    let k: Key = (*key).into();
    ChaCha20Poly1305::new(&k).decrypt(&Nonce::from(nonce), Payload { msg: ct, aad: aad.as_bytes() }).map_err(|_| bad())
}

pub fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn unb64(s: &str) -> Option<Vec<u8>> {
    if s.len() > 8 * 1024 {
        return None;
    }
    B64.decode(s).ok()
}

pub fn key32(s: &str) -> Option<[u8; 32]> {
    unb64(s)?.try_into().ok()
}

// ---- joining ----------------------------------------------------------------

fn pake_ids() -> (Identity, Identity) {
    (Identity::new(b"platter-listen-guest"), Identity::new(b"platter-listen-host"))
}

/// The guest's half of the key agreement over the invite secret.
pub struct PakeGuest(Spake2<Ed25519Group>);

pub fn pake_guest_start(invite_secret: &[u8]) -> (PakeGuest, Vec<u8>) {
    let (a, b) = pake_ids();
    let (s, msg) = Spake2::<Ed25519Group>::start_a(&Password::new(invite_secret), &a, &b);
    (PakeGuest(s), msg)
}

pub fn pake_guest_finish(state: PakeGuest, msg_b: &[u8]) -> AppResult<[u8; 32]> {
    let k = state.0.finish(msg_b).map_err(|_| AppError::Invalid("The host's answer didn't match the invite.".into()))?;
    Ok(derive(&k, "join"))
}

/// The host's answer to a guest's first message, and the key both now share.
pub fn pake_host(invite_secret: &[u8], msg_a: &[u8]) -> AppResult<(Vec<u8>, [u8; 32])> {
    let (a, b) = pake_ids();
    let (s, msg_b) = Spake2::<Ed25519Group>::start_b(&Password::new(invite_secret), &a, &b);
    let k = s.finish(msg_a).map_err(|_| AppError::Invalid("bad join message".into()))?;
    Ok((msg_b, derive(&k, "join")))
}

/// What a session's server is called inside messages: a hash of the Plex
/// server's machine id keyed with the session id, so it's different in
/// every session and the id itself never travels.
pub fn server_hash(session: &[u8], machine_id: &str) -> String {
    hex(&hmac_sha256(session, format!("{LABEL} server\n{machine_id}").as_bytes())[..12])
}
