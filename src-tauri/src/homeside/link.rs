//! A paired desktop and Bridge talking through the relay.
//!
//! Both ends hold the link secret (from the key agreement at pairing). Each
//! direction is its own [`Channel`]: a topic and a key derived from that
//! secret, so the relay sees two random topic names and ciphertext.
//!
//! A message ([`Msg`]) is JSON, deflated, then cut into pieces small enough
//! for one relay message each. Every piece is sealed in a [`Frame`]: who sent
//! it, a counter that only goes up (kept across restarts), an expiry, and
//! which message and piece it is. Receivers drop anything expired and
//! anything whose counter isn't above the last one seen from that sender
//! ([`Guard`]), then put the pieces back together ([`Reassembler`]).

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use super::wire::Msg;
use crate::error::{AppError, AppResult};
use crate::social::listen::seal::{self, Channel, Rejected};

/// Raw bytes per piece: sealed and in base64 twice, a piece stays under the
/// relay's 4096-byte message limit.
pub const PIECE: usize = 1800;
/// Most pieces in one message (deflated, about 900 KB).
pub const MAX_PIECES: u16 = 512;
/// Largest message once inflated.
const MAX_PLAIN: u64 = 16 * 1024 * 1024;
/// How long a frame counts: long enough to wait on the relay while the
/// other side is off (ntfy.sh keeps messages for 12 hours).
pub const TTL_MS: u64 = 12 * 3_600_000;
/// A frame may not claim to count for longer than this (a wrong clock).
const MAX_AHEAD_MS: u64 = TTL_MS + 3_600_000;
/// Half-built messages waiting for their other pieces.
const MAX_PENDING: usize = 32;

/// Which end this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Desktop,
    Bridge,
    /// One of the user's own devices, linked to another one. The two ends
    /// are equals: each publishes on a topic of its own.
    Device,
}

/// One pairing, as both ends keep it.
#[derive(Clone, PartialEq, Eq)]
pub struct Link {
    /// Random, the same at both ends.
    pub id: String,
    pub secret: [u8; 32],
    /// The relay's base URL.
    pub relay: String,
    pub side: Side,
    /// This end's id inside frames.
    pub me: String,
    /// The other end's id inside frames (device links; empty otherwise).
    pub peer: String,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link").field("id", &self.id).field("side", &self.side).finish_non_exhaustive()
    }
}

impl Link {
    pub fn to_bridge(&self) -> Channel {
        Channel::new(&self.secret, "bridge to-bridge")
    }

    pub fn to_desktop(&self) -> Channel {
        Channel::new(&self.secret, "bridge to-desktop")
    }

    /// A device's own channel on a device link, derived from its frame id.
    fn device_channel(&self, id: &str) -> Channel {
        Channel::new(&self.secret, &format!("device from-{id}"))
    }

    /// Where this end listens.
    pub fn inbox(&self) -> Channel {
        match self.side {
            Side::Desktop => self.to_desktop(),
            Side::Bridge => self.to_bridge(),
            Side::Device => self.device_channel(&self.peer),
        }
    }

    /// Where this end publishes.
    pub fn outbox(&self) -> Channel {
        match self.side {
            Side::Desktop => self.to_bridge(),
            Side::Bridge => self.to_desktop(),
            Side::Device => self.device_channel(&self.me),
        }
    }

    /// On a device link, the key for messages sent over the local network
    /// instead of the relay: (what this end opens, what it seals with).
    /// Their own keys, so a frame can't be moved from one path to the other.
    pub fn lan_channels(&self) -> (Channel, Channel) {
        let ch = |id: &str| Channel::new(&self.secret, &format!("device lan from-{id}"));
        (ch(&self.peer), ch(&self.me))
    }
}

/// How big a message is once deflated, before sealing.
pub fn packed_len(msg: &Msg) -> usize {
    serde_json::to_vec(msg).ok().and_then(|p| deflate(&p).ok()).map_or(usize::MAX, |p| p.len())
}

/// One sealed piece of a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// The sender's id.
    pub f: String,
    /// Counts up with every frame the sender seals.
    pub c: u64,
    /// Unix ms after which it no longer counts.
    pub x: u64,
    /// The message id, the piece and how many there are.
    pub i: String,
    pub n: u16,
    pub t: u16,
    /// The piece, base64.
    pub d: String,
}

/// Counters for frames this end sends: above the last one, and never below
/// the clock (in microseconds), so they keep going up across restarts.
#[derive(Debug, Default, Clone)]
pub struct Counter {
    last: u64,
}

impl Counter {
    /// Carry on from the last counter used.
    pub fn starting_at(last: u64) -> Self {
        Self { last }
    }

    pub fn last(&self) -> u64 {
        self.last
    }

    pub fn next(&mut self, now_ms: u64) -> u64 {
        self.last = (self.last + 1).max(now_ms.saturating_mul(1000));
        self.last
    }
}

fn deflate(bytes: &[u8]) -> AppResult<Vec<u8>> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(bytes).map_err(|e| AppError::Internal(format!("deflate: {e}")))?;
    e.finish().map_err(|e| AppError::Internal(format!("deflate: {e}")))
}

fn inflate(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(bytes).take(MAX_PLAIN + 1).read_to_end(&mut out).ok()?;
    (out.len() as u64 <= MAX_PLAIN).then_some(out)
}

/// Seal `msg` into relay bodies for `ch`, in order.
pub fn seal_msg(ch: &Channel, me: &str, counter: &mut Counter, msg: &Msg, now_ms: u64) -> AppResult<Vec<String>> {
    let plain = serde_json::to_vec(msg).map_err(|e| AppError::Internal(format!("bridge message: {e}")))?;
    let packed = deflate(&plain)?;
    let pieces: Vec<&[u8]> = packed.chunks(PIECE).collect();
    if pieces.len() > MAX_PIECES as usize {
        return Err(AppError::Invalid("That's too much to send through the relay at once.".into()));
    }
    let id = crate::remote::peer::auth::hex(&seal::random16()[..8]);
    let t = pieces.len().max(1) as u16;
    let mut out = Vec::with_capacity(pieces.len());
    for (n, p) in pieces.iter().enumerate() {
        let f = Frame { f: me.to_string(), c: counter.next(now_ms), x: now_ms + TTL_MS, i: id.clone(), n: n as u16, t, d: seal::b64(p) };
        out.push(seal::seal_value(ch, &f)?);
    }
    Ok(out)
}

/// Open one relay body into a frame (seal and expiry checked; replays are
/// the [`Guard`]'s job).
pub fn open_frame(ch: &Channel, body: &str, now_ms: u64) -> Result<Frame, Rejected> {
    let f: Frame = seal::open_value(ch, body)?;
    if f.x < now_ms || f.x > now_ms + MAX_AHEAD_MS {
        return Err(Rejected::Expired);
    }
    if f.t == 0 || f.t > MAX_PIECES || f.n >= f.t || f.d.len() > 8 * 1024 || f.i.len() > 32 || f.f.len() > 64 {
        return Err(Rejected::Malformed);
    }
    Ok(f)
}

/// The last counter seen per sender, and the last few taken. A frame above
/// the last counts; so does one below it that was never taken and is newer
/// than the oldest one kept (it came in late on another relay). Anything
/// else is a replay. Kept with the link so a restart doesn't accept old
/// frames again.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guard {
    pub last: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub recent: BTreeMap<String, Vec<u64>>,
}

/// Counters kept per sender for frames that come in late.
pub const GUARD_WINDOW: usize = 64;

impl Guard {
    pub fn accept(&mut self, f: &Frame) -> Result<(), Rejected> {
        let last = self.last.entry(f.f.clone()).or_insert(0);
        let recent = self.recent.entry(f.f.clone()).or_default();
        if f.c > *last {
            *last = f.c;
        } else if !recent.first().is_some_and(|oldest| f.c > *oldest) {
            return Err(Rejected::Replayed);
        }
        // Kept in order: anything taken and dropped from here is older than
        // the oldest one kept.
        match recent.binary_search(&f.c) {
            Ok(_) => return Err(Rejected::Replayed),
            Err(i) => recent.insert(i, f.c),
        }
        if recent.len() > GUARD_WINDOW {
            recent.remove(0);
        }
        Ok(())
    }
}

/// One half-built message: how many pieces, those in so far, when it started.
type Partial = (u16, BTreeMap<u16, Vec<u8>>, u64);

/// Pieces waiting for the rest of their message.
#[derive(Default)]
pub struct Reassembler {
    pending: HashMap<(String, String), Partial>,
}

impl Reassembler {
    /// Add a piece; the whole message once every piece is in.
    pub fn add(&mut self, f: Frame, now_ms: u64) -> Option<Msg> {
        let bytes = seal::unb64(&f.d)?;
        if f.t == 1 {
            return decode(&bytes);
        }
        if self.pending.len() >= MAX_PENDING {
            // Forget the oldest half-built message.
            if let Some(k) = self.pending.iter().min_by_key(|(_, v)| v.2).map(|(k, _)| k.clone()) {
                self.pending.remove(&k);
            }
        }
        let key = (f.f.clone(), f.i.clone());
        let entry = self.pending.entry(key.clone()).or_insert_with(|| (f.t, BTreeMap::new(), now_ms));
        if entry.0 != f.t {
            self.pending.remove(&key);
            return None;
        }
        entry.1.insert(f.n, bytes);
        if entry.1.len() < f.t as usize {
            return None;
        }
        let (_, parts, _) = self.pending.remove(&key)?;
        let all: Vec<u8> = parts.into_values().flatten().collect();
        decode(&all)
    }
}

fn decode(packed: &[u8]) -> Option<Msg> {
    serde_json::from_slice(&inflate(packed)?).ok()
}
