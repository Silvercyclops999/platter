//! Sealed links between two people, kept for as long as they share a
//! playlist or a Blend, and the packets that travel on them.
//!
//! A link is a random secret only the two hold. Each direction is a listen
//! along [`Channel`] (a topic and a key derived from the secret), so the
//! relay sees random topic names and ciphertext.
//!
//! A packet is `{f: sender, c: counter, fl: floor, m: message}`, as JSON,
//! deflated and cut into pieces small enough for the relay
//! ([`seal_packet`]); each piece is sealed with [`seal::seal_value`].
//!
//! The relay keeps messages for hours, and people are offline for longer,
//! so every packet but an ack stays in the sender's outbox until the other
//! side acknowledges it, and is sent again until then ([`Link::due`]).
//! The receiver applies each counter once, whatever order they come in
//! ([`Link::accept`]); `fl` is the lowest counter still in the sender's
//! outbox, so anything below it that never came is given up on.

use std::collections::HashMap;
use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::{Deserialize, Serialize};

use super::wire::Msg;
use crate::error::{AppError, AppResult};
use crate::social::listen::seal::{self, Channel, Rejected};

/// Raw bytes of a deflated packet per piece (sealed, it stays under the
/// relay's 4 KB).
pub const CHUNK: usize = 2000;
/// Pieces one packet may have.
pub const MAX_PIECES: u16 = 64;
/// Largest packet, inflated.
const MAX_PLAIN: u64 = 1024 * 1024;
/// An outbox longer than this is trimmed from the front.
pub const OUTBOX_MAX: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Packet {
    pub f: String,
    /// 0 for an ack (never queued, never acknowledged).
    pub c: u64,
    #[serde(default)]
    pub fl: u64,
    pub m: Msg,
}

/// One piece of a packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Piece {
    pub id: String,
    pub i: u16,
    pub n: u16,
    pub z: String,
}

/// The sealed texts to publish for a packet, in order.
pub fn seal_packet(ch: &Channel, p: &Packet) -> AppResult<Vec<String>> {
    let plain = serde_json::to_vec(p).map_err(|e| AppError::Internal(format!("packet: {e}")))?;
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&plain).map_err(|e| AppError::Internal(format!("deflate: {e}")))?;
    let z = z.finish().map_err(|e| AppError::Internal(format!("deflate: {e}")))?;
    let n = z.len().div_ceil(CHUNK).max(1);
    if n > MAX_PIECES as usize {
        return Err(AppError::Invalid("That's too much to send at once.".into()));
    }
    let id = crate::remote::peer::auth::hex(&seal::random16()[..6]);
    // Deflate always writes something, so there is at least one piece.
    z.chunks(CHUNK)
        .enumerate()
        .map(|(i, part)| seal::seal_value(ch, &Piece { id: id.clone(), i: i as u16, n: n as u16, z: B64.encode(part) }))
        .collect()
}

fn inflate(z: &[u8]) -> Option<Packet> {
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(z).take(MAX_PLAIN).read_to_end(&mut out).ok()?;
    serde_json::from_slice(&out).ok()
}

/// (topic, packet id) → when the first piece came, and the pieces so far.
type Open = HashMap<(String, String), (u64, Vec<Option<Vec<u8>>>)>;

/// Pieces waiting for the rest of their packet, per topic.
#[derive(Default)]
pub struct Pieces {
    open: Open,
}

impl Pieces {
    /// Open a message; Some when it completes a packet.
    pub fn take(&mut self, ch: &Channel, text: &str, now_ms: u64) -> Result<Option<Packet>, Rejected> {
        let piece: Piece = seal::open_value(ch, text)?;
        if piece.n == 0 || piece.n > MAX_PIECES || piece.i >= piece.n || piece.id.len() > 32 {
            return Err(Rejected::Malformed);
        }
        let bytes = B64.decode(&piece.z).map_err(|_| Rejected::Malformed)?;
        if piece.n == 1 {
            return inflate(&bytes).map(Some).ok_or(Rejected::Malformed);
        }
        // Half-made packets are given up on after an hour.
        self.open.retain(|_, (at, _)| now_ms.saturating_sub(*at) < 3_600_000);
        if self.open.len() > 256 {
            self.open.clear();
        }
        let key = (ch.topic.clone(), piece.id);
        let (_, parts) = self.open.entry(key.clone()).or_insert_with(|| (now_ms, vec![None; piece.n as usize]));
        if parts.len() != piece.n as usize {
            return Err(Rejected::Malformed);
        }
        parts[piece.i as usize] = Some(bytes);
        if parts.iter().any(Option::is_none) {
            return Ok(None);
        }
        let (_, parts) = self.open.remove(&key).unwrap_or_default();
        let all: Vec<u8> = parts.into_iter().flatten().flatten().collect();
        inflate(&all).map(Some).ok_or(Rejected::Malformed)
    }
}

/// A packet waiting for its ack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Out {
    pub c: u64,
    pub m: Msg,
}

/// One side of a link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    /// The shared secret (base64).
    pub secret: String,
    /// The next counter to give out.
    pub next: u64,
    pub outbox: Vec<Out>,
    /// Every counter up to this one came; `extra` came above it.
    pub seen: u64,
    #[serde(default)]
    pub extra: Vec<u64>,
    /// Resends so far without an answer, and when the next is due (unix ms).
    #[serde(default)]
    pub tries: u32,
    #[serde(default)]
    pub due: u64,
    /// Last heard from the other side (unix ms; 0: never).
    #[serde(default)]
    pub heard: u64,
}

/// Which end of a link this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// The owner (a playlist's, or whoever answered a Blend invite).
    Owner,
    Member,
}

impl Link {
    pub fn new(secret: &[u8; 32]) -> Self {
        Self { secret: seal::b64(secret), next: 1, outbox: Vec::new(), seen: 0, extra: Vec::new(), tries: 0, due: 0, heard: 0 }
    }

    fn key(&self) -> [u8; 32] {
        seal::key32(&self.secret).unwrap_or([0; 32])
    }

    /// (where this end sends, where it listens).
    pub fn channels(&self, end: End) -> (Channel, Channel) {
        let k = self.key();
        let down = Channel::new(&k, "circle down");
        let up = Channel::new(&k, "circle up");
        match end {
            End::Owner => (down, up),
            End::Member => (up, down),
        }
    }

    /// Queue a message; returns its counter.
    pub fn push(&mut self, m: Msg, now_ms: u64) -> u64 {
        let c = self.next;
        self.next += 1;
        self.outbox.push(Out { c, m });
        if self.outbox.len() > OUTBOX_MAX {
            self.outbox.drain(..self.outbox.len() - OUTBOX_MAX);
        }
        // Something new goes out at once.
        self.tries = 0;
        self.due = now_ms;
        c
    }

    /// The lowest counter still waiting (or the next one).
    pub fn floor(&self) -> u64 {
        self.outbox.first().map_or(self.next, |o| o.c)
    }

    /// A packet came: true the first time its counter is seen.
    pub fn accept(&mut self, c: u64, floor: u64, now_ms: u64) -> bool {
        self.heard = now_ms;
        if floor > 0 && self.seen < floor - 1 {
            self.seen = floor - 1;
        }
        self.extra.retain(|x| *x > self.seen);
        if c == 0 || c <= self.seen || self.extra.contains(&c) {
            return false;
        }
        self.extra.push(c);
        self.extra.sort_unstable();
        while self.extra.first() == Some(&(self.seen + 1)) {
            self.seen += 1;
            self.extra.remove(0);
        }
        if self.extra.len() > 512 {
            self.extra.drain(..self.extra.len() - 512);
        }
        true
    }

    /// The ack for what came.
    pub fn ack(&self) -> Msg {
        Msg::Ack { upto: self.seen, also: self.extra.clone() }
    }

    /// The other side has these: out of the outbox.
    pub fn acked(&mut self, upto: u64, also: &[u64], now_ms: u64) {
        self.heard = now_ms;
        let before = self.outbox.len();
        self.outbox.retain(|o| o.c > upto && !also.contains(&o.c));
        if self.outbox.len() != before {
            self.tries = 0;
        }
    }

    /// Time to send the outbox (again)?
    pub fn due(&self, now_ms: u64) -> bool {
        !self.outbox.is_empty() && now_ms >= self.due
    }

    /// The outbox went out: wait longer each time nobody answers (from
    /// `first` up to `max`).
    pub fn sent(&mut self, now_ms: u64, first: u64, max: u64) {
        let wait = first.saturating_mul(1u64 << self.tries.min(16)).min(max);
        self.tries = self.tries.saturating_add(1);
        self.due = now_ms + wait;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_big_packet_goes_in_pieces_and_comes_back_whole() {
        let ch = Channel::new(&[7; 32], "t");
        // Random text doesn't deflate much.
        let text: String = (0..300).map(|_| crate::remote::peer::auth::hex(&seal::random16())).collect();
        let p = Packet { f: "me".into(), c: 3, fl: 1, m: Msg::Hello { name: text } };
        let bodies = seal_packet(&ch, &p).unwrap();
        assert!(bodies.len() > 1);
        assert!(bodies.iter().all(|b| b.len() < seal::MAX_BODY));
        let mut pieces = Pieces::default();
        // Out of order is fine.
        let mut got = None;
        for b in bodies.iter().rev() {
            if let Some(x) = pieces.take(&ch, b, 0).unwrap() {
                got = Some(x);
            }
        }
        assert_eq!(got, Some(p));
        // Another channel's key doesn't open it.
        assert!(pieces.take(&Channel::new(&[8; 32], "t"), &bodies[0], 0).is_err());
    }

    #[test]
    fn every_counter_is_taken_once_in_any_order() {
        let mut l = Link::new(&[1; 32]);
        assert!(l.accept(2, 1, 0));
        assert!(!l.accept(2, 1, 0));
        assert!(l.accept(1, 1, 0));
        assert_eq!((l.seen, l.extra.len()), (2, 0));
        assert!(!l.accept(1, 1, 0));
        // The sender gave up on 3 and 4.
        assert!(l.accept(6, 5, 0));
        assert_eq!(l.seen, 4);
        assert!(!l.accept(3, 5, 0));
        assert_eq!(l.ack(), Msg::Ack { upto: 4, also: vec![6] });
    }

    #[test]
    fn the_outbox_waits_longer_each_time_and_empties_on_ack() {
        let mut l = Link::new(&[1; 32]);
        let a = l.push(Msg::Leave, 100);
        let b = l.push(Msg::Hello { name: "x".into() }, 100);
        assert!(l.due(100));
        l.sent(100, 1000, 8000);
        assert!(!l.due(1099) && l.due(1100));
        l.sent(1100, 1000, 8000);
        assert_eq!(l.due, 3100);
        l.acked(a, &[], 3000);
        assert_eq!(l.outbox.iter().map(|o| o.c).collect::<Vec<_>>(), vec![b]);
        assert_eq!(l.floor(), b);
        l.acked(0, &[b], 3000);
        assert!(l.outbox.is_empty() && !l.due(u64::MAX));
    }
}
