//! Sending and receiving on links, the same at both ends: seal and publish
//! a message's pieces in order, and keep a subscription to each link's
//! inbox that opens, checks and reassembles what arrives.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::{mpsc, watch};

use super::link::{self, Counter, Link, Reassembler};
use super::links::Links;
use super::wire::Msg;
use crate::error::{AppError, AppResult};
use crate::social::listen::relay::{RelayError, RelayEvent, RelayFactory};

pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Wall clock in ms.
pub fn system_ms() -> u64 {
    crate::sync::store::system_ms()
}

pub struct Wire {
    pub links: Links,
    relays: Arc<dyn RelayFactory>,
    clock: Clock,
    counters: Mutex<HashMap<String, Counter>>,
    parts: Mutex<HashMap<String, Reassembler>>,
    /// One publish sequence per link at a time, so pieces stay in order.
    sending: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Wire {
    pub fn new(links: Links, relays: Arc<dyn RelayFactory>, clock: Clock) -> Self {
        Self { links, relays, clock, counters: Mutex::default(), parts: Mutex::default(), sending: Mutex::default() }
    }

    pub fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    pub fn relays(&self) -> &Arc<dyn RelayFactory> {
        &self.relays
    }

    /// Seal and publish a message.
    pub async fn send(&self, link: &Link, msg: &Msg) -> AppResult<()> {
        let lock = self.sending.lock().entry(link.id.clone()).or_default().clone();
        let _one = lock.lock().await;
        let bodies = {
            let mut counters = self.counters.lock();
            let c = counters.entry(link.id.clone()).or_insert_with(|| self.links.counter(&link.id));
            let bodies = link::seal_msg(&link.outbox(), &link.me, c, msg, self.now_ms())?;
            let last = c.last();
            drop(counters);
            let _ = self.links.update(&link.id, |p| p.counter = p.counter.max(last));
            bodies
        };
        let relay = self.relays.relay(&link.relay);
        let topic = link.outbox().topic;
        let kind = msg.kind();
        if background(msg) && !relay.room_for(bodies.len()) {
            // The rest of this hour's allowance is spent: it goes later.
            crate::social::listen::relay::note(kind, bodies.len(), crate::social::listen::relay::Counted::Held);
            tracing::debug!(kind, pieces = bodies.len(), "relay: held back to stay under the limit");
            return Err(AppError::Network(relay_error(RelayError::Limited)));
        }
        crate::social::listen::relay::as_kind(kind, async {
            for b in &bodies {
                let mut tries = 0;
                loop {
                    match relay.publish(&topic, b).await {
                        Ok(()) => break,
                        // Turned away: trying again soon only counts against the limit.
                        Err(e) if tries < 3 && !matches!(e, RelayError::Refused(400) | RelayError::Limited) => {
                            tries += 1;
                            tokio::time::sleep(Duration::from_millis(500 * (1 << tries))).await;
                        }
                        Err(e) => return Err(AppError::Network(relay_error(e))),
                    }
                }
            }
            Ok(())
        })
        .await?;
        tracing::debug!(kind, pieces = bodies.len(), "bridge link: sent");
        Ok(())
    }

    /// The publish lock for one link: hold it to keep a run of messages in
    /// order on that link.
    pub fn sending(&self, link: &Link) -> Arc<tokio::sync::Mutex<()>> {
        self.sending.lock().entry(link.id.clone()).or_default().clone()
    }

    /// Seal a message for the local network on a device link (the same
    /// frames as the relay, under the link's network keys). `reply`: an
    /// answer to the other end's request, counted apart from requests so
    /// the two never trip each other's replay guard.
    pub fn seal_lan(&self, link: &Link, msg: &Msg, reply: bool) -> AppResult<Vec<String>> {
        let (_, out) = link.lan_channels();
        let mut counters = self.counters.lock();
        let c = counters.entry(link.id.clone()).or_insert_with(|| self.links.counter(&link.id));
        let bodies = link::seal_msg(&out, &lan_sender(&link.me, reply), c, msg, self.now_ms())?;
        let last = c.last();
        drop(counters);
        let _ = self.links.update(&link.id, |p| p.counter = p.counter.max(last));
        Ok(bodies)
    }

    /// Open one message that came over the local network on a device link.
    /// Anything sealed wrong, from the wrong end, expired or replayed is
    /// None.
    pub fn open_lan(&self, link: &Link, bodies: &[String], reply: bool) -> Option<Msg> {
        let (inbox, _) = link.lan_channels();
        let now = self.now_ms();
        let want = lan_sender(&link.peer, reply);
        let mut frames = Vec::with_capacity(bodies.len());
        for b in bodies {
            let frame = link::open_frame(&inbox, b, now).ok()?;
            if frame.f != want {
                return None;
            }
            frames.push(frame);
        }
        // Every piece checked and recorded at once.
        if !self.links.accept(&link.id, &frames.iter().collect::<Vec<_>>()) {
            tracing::debug!("device link: dropped a replay");
            return None;
        }
        let mut parts = Reassembler::default();
        let mut out = None;
        for frame in frames {
            out = parts.add(frame, now);
        }
        out
    }

    /// One relay message from a link's inbox: the whole message once every
    /// piece is in. Anything sealed wrong, expired or replayed is dropped.
    pub fn receive(&self, link: &Link, body: &str) -> Option<Msg> {
        let now = self.now_ms();
        let frame = match link::open_frame(&link.inbox(), body, now) {
            Ok(f) => f,
            Err(why) => {
                tracing::debug!(?why, "bridge link: dropped a message");
                return None;
            }
        };
        if !self.links.accept(&link.id, &[&frame]) {
            tracing::debug!("bridge link: dropped a replay");
            return None;
        }
        self.parts.lock().entry(link.id.clone()).or_default().add(frame, now)
    }
}

/// Work that can wait (synced changes, shared Lidarr state, lyrics,
/// Bring in my listening's plan, the daily look's), so it takes from the relay's allowance.
/// Requests, answers, hellos and status don't.
pub fn background(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::Entries { .. } | Msg::LyricsHave { .. } | Msg::SyncEntries { .. } | Msg::SyncDiff { .. } | Msg::WavesPlan(_) | Msg::WavesProgress(_)
            | Msg::RadarPlan(_) | Msg::RadarMade { .. }
    )
}

/// The error says the relay is busy (turned away, or this device's
/// allowance is spent for now).
pub fn is_busy(e: &AppError) -> bool {
    e.to_string().contains(&relay_error(RelayError::Limited))
}

/// A device's frame id on the local network: requests and answers apart.
fn lan_sender(id: &str, reply: bool) -> String {
    format!("{id}.{}", if reply { "lanr" } else { "lan" })
}

pub fn relay_error(e: RelayError) -> String {
    match e {
        RelayError::Unreachable => "The relay can't be reached.".into(),
        RelayError::Limited => "The relay is turning messages away for a while (too many today). It will try again later.".into(),
        RelayError::Refused(c) => format!("The relay refused the message ({c})."),
    }
}

/// What a link's listener hands on.
pub enum Incoming {
    /// Subscribed (again): anything published from now on arrives.
    Opened(String),
    Msg(String, Msg),
}

/// Listen on a link's inbox until `stop`, handing whole messages to `tx`
/// with the link's id. Reconnects with a growing wait, and picks up after
/// the last message read.
pub async fn listen(wire: Arc<Wire>, link: Link, tx: mpsc::Sender<Incoming>, mut stop: watch::Receiver<bool>) {
    let relay = wire.relays().relay(&link.relay);
    let topic = link.inbox().topic;
    let mut wait = Duration::from_secs(2);
    loop {
        if *stop.borrow() {
            return;
        }
        let since = wire.links.get(&link.id).and_then(|p| p.since);
        match relay.subscribe(std::slice::from_ref(&topic), since.as_deref()).await {
            Ok(mut rx) => loop {
                tokio::select! {
                    ev = rx.recv() => match ev {
                        Some(RelayEvent::Open) => {
                            wait = Duration::from_secs(2);
                            if tx.send(Incoming::Opened(link.id.clone())).await.is_err() {
                                return;
                            }
                        }
                        Some(RelayEvent::Message { id, body, .. }) => {
                            let _ = wire.links.update(&link.id, |p| p.since = Some(id));
                            if let Some(m) = wire.receive(&link, &body)
                                && tx.send(Incoming::Msg(link.id.clone(), m)).await.is_err()
                            {
                                return;
                            }
                        }
                        None => break,
                    },
                    _ = stop.changed() => return,
                }
            },
            Err(e) => {
                tracing::info!("Bridge link: {}", relay_error(e));
                if e == RelayError::Limited {
                    wait = wait.max(Duration::from_secs(60));
                }
            }
        }
        if wire.links.get(&link.id).is_none() {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = stop.changed() => return,
        }
        wait = (wait * 2).min(Duration::from_secs(300));
    }
}
