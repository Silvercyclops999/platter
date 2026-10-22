//! Sending and receiving on links, the same at both ends: seal and publish
//! a message's pieces in order, and keep a subscription to each link's
//! inbox that opens, checks and reassembles what arrives.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::StreamExt;
use futures::stream::FuturesUnordered;

use parking_lot::Mutex;
use tokio::sync::{mpsc, watch};

use super::link::{self, Counter, Link, Reassembler};
use super::links::Links;
use super::wire::Msg;
use crate::error::{AppError, AppResult};
use crate::social::listen::relay::{self, Relay, RelayError, RelayEvent, RelayFactory};

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
    /// Relays used side by side with a link's own, by that relay.
    backups: Vec<(String, Vec<String>)>,
    /// Relays that couldn't be reached lately, by address.
    down: DownMap,
    /// Each relay's queue of messages, by link and relay.
    queues: Mutex<HashMap<(String, String), mpsc::UnboundedSender<Job>>>,
}

impl Wire {
    pub fn new(links: Links, relays: Arc<dyn RelayFactory>, clock: Clock) -> Self {
        let backups = vec![(
            crate::social::listen::DEFAULT_RELAY.to_string(),
            crate::social::listen::BACKUP_RELAYS.iter().map(|b| b.to_string()).collect(),
        )];
        Self { links, relays, clock, counters: Mutex::default(), parts: Mutex::default(), sending: Mutex::default(), backups, down: Arc::default(), queues: Mutex::default() }
    }

    pub fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    pub fn relays(&self) -> &Arc<dyn RelayFactory> {
        &self.relays
    }

    /// Whether the relay is being left alone after it couldn't be reached.
    pub fn is_down(&self, base: &str) -> bool {
        self.down.lock().get(base).is_some_and(|c| c.waiting(std::time::Instant::now()))
    }

    /// The relays a link uses: its own, then the backups when it's on
    /// ntfy.sh. A server of the user's own is used alone.
    pub fn relays_for(&self, relay: &str) -> Vec<String> {
        let mut all = vec![relay.to_string()];
        if let Some((_, more)) = self.backups.iter().find(|(p, _)| p == relay) {
            all.extend(more.iter().cloned());
        }
        all
    }

    /// Use these backups for links on `relay` (tests; the app uses
    /// [`crate::social::listen::BACKUP_RELAYS`] for ntfy.sh).
    pub fn with_backups(mut self, relay: &str, backups: Vec<String>) -> Self {
        self.backups.retain(|(p, _)| p != relay);
        self.backups.push((relay.to_string(), backups));
        self
    }

    /// Seal and publish a message on every relay of the link that isn't
    /// known to be down (all of them if every one is). The same frames go
    /// to each, so the other end takes the first copy and its replay guard
    /// drops the rest. Sent once one relay took every piece; the slower
    /// ones carry on by themselves.
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
        let topic = link.outbox().topic;
        let kind = msg.kind();
        let all = self.relays_for(&link.relay);
        let up: Vec<String> = all.iter().filter(|b| !self.is_down(b)).cloned().collect();
        // Every relay marked down: try them all anyway.
        let force = up.is_empty();
        let tries = if force { all } else { up };
        let bodies = Arc::new(bodies);
        let mut held = false;
        let mut going = FuturesUnordered::new();
        for base in tries {
            let relay = self.relays.relay(&base);
            if background(msg) && !relay.room_for(bodies.len()) {
                // The rest of this hour's allowance there is spent: it goes later.
                relay::note(kind, bodies.len(), relay::Counted::Held);
                held = true;
                continue;
            }
            let (done, result) = tokio::sync::oneshot::channel();
            let job = Job { topic: topic.clone(), bodies: bodies.clone(), kind, force, done };
            if self.queue(&link.id, &base, relay).send(job).is_ok() {
                going.push(result);
            }
        }
        let mut worst = held.then_some(RelayError::Limited);
        while let Some(done) = going.next().await {
            match done {
                Ok(Ok(())) => {
                    tracing::debug!(kind, pieces = bodies.len(), "bridge link: sent");
                    return Ok(());
                }
                Ok(Err(e)) => worst = Some(worse(worst, e)),
                Err(_) => {}
            }
        }
        if held && worst == Some(RelayError::Limited) {
            tracing::debug!(kind, pieces = bodies.len(), "relay: held back to stay under the limit");
        }
        Err(AppError::Network(relay_error(worst.unwrap_or(RelayError::Unreachable))))
    }

    /// The queue of messages for one relay of a link. Each relay takes a
    /// link's messages one at a time and in order, however far behind the
    /// others it is, so a slow relay never lets a later message overtake an
    /// earlier one.
    fn queue(&self, link: &str, base: &str, relay: Arc<dyn Relay>) -> mpsc::UnboundedSender<Job> {
        let mut queues = self.queues.lock();
        let key = (link.to_string(), base.to_string());
        if let Some(q) = queues.get(&key).filter(|q| !q.is_closed()) {
            return q.clone();
        }
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(publisher(relay, base.to_string(), self.down.clone(), rx));
        queues.insert(key, tx.clone());
        tx
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

type DownMap = Arc<Mutex<HashMap<String, relay::Cool>>>;

/// One message for one relay.
struct Job {
    topic: String,
    bodies: Arc<Vec<String>>,
    kind: &'static str,
    /// Every relay was marked down, so it's tried anyway.
    force: bool,
    done: tokio::sync::oneshot::Sender<Result<(), RelayError>>,
}

/// Publishes one link's messages on one relay, in order. While the relay is
/// marked down, waiting messages fail at once instead of each waiting out
/// its tries.
async fn publisher(relay: Arc<dyn Relay>, base: String, down: DownMap, mut jobs: mpsc::UnboundedReceiver<Job>) {
    while let Some(job) = jobs.recv().await {
        let skip = !job.force && down.lock().get(&base).is_some_and(|c| c.waiting(std::time::Instant::now()));
        let res = if skip {
            Err(RelayError::Unreachable)
        } else {
            let res = relay::as_kind(job.kind, publish_all(&*relay, &job.topic, &job.bodies)).await;
            match res {
                Ok(()) => mark(&down, &base, true),
                Err(RelayError::Refused(400) | RelayError::Limited) => {}
                Err(_) => mark(&down, &base, false),
            }
            res
        };
        let _ = job.done.send(res);
    }
}

/// A relay answered (`up`), or couldn't be reached or refused: then it's
/// left alone for a while, longer each time in a row.
fn mark(down: &DownMap, base: &str, up: bool) {
    let mut d = down.lock();
    if up {
        d.remove(base);
    } else {
        d.entry(base.to_string()).or_default().wait(std::time::Instant::now(), relay::DOWN_FIRST, relay::DOWN_MAX);
    }
}

/// Every piece of a message on one relay, in order.
async fn publish_all(relay: &dyn Relay, topic: &str, bodies: &[String]) -> Result<(), RelayError> {
    for b in bodies {
        let mut tries = 0;
        loop {
            match relay.publish(topic, b).await {
                Ok(()) => break,
                // Turned away: trying again soon only counts against the limit.
                Err(e) if tries < 3 && !matches!(e, RelayError::Refused(400) | RelayError::Limited) => {
                    tries += 1;
                    tokio::time::sleep(Duration::from_millis(500 * (1 << tries))).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

/// Which error to report when no relay took a message: busy over
/// unreachable over refused, so callers that wait out a busy relay still do.
fn worse(a: Option<RelayError>, b: RelayError) -> RelayError {
    let rank = |e: &RelayError| match e {
        RelayError::Limited => 2,
        RelayError::Unreachable => 1,
        RelayError::Refused(_) => 0,
    };
    match a {
        Some(a) if rank(&a) >= rank(&b) => a,
        _ => b,
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

/// Listen on a link's inbox, on every relay it uses, until `stop`, handing
/// whole messages to `tx` with the link's id. [`Incoming::Opened`] goes
/// once the first relay is listening, not once per relay.
pub async fn listen(wire: Arc<Wire>, link: Link, tx: mpsc::Sender<Incoming>, stop: watch::Receiver<bool>) {
    let open = Arc::new(AtomicUsize::new(0));
    let all = wire.relays_for(&link.relay);
    futures::future::join_all(all.into_iter().map(|base| listen_on(wire.clone(), link.clone(), base, open.clone(), tx.clone(), stop.clone()))).await;
}

/// Where to pick up on a relay: the last message read there, or for a
/// backup never read yet, the last while (the replay guard drops what came
/// in on another relay already).
fn since_on(wire: &Wire, link: &Link, base: &str) -> Option<String> {
    let p = wire.links.get(&link.id)?;
    if base == link.relay {
        return p.since;
    }
    p.since_on.get(base).cloned().or_else(|| Some(((wire.now_ms() / 1000).saturating_sub(link::TTL_MS / 1000)).to_string()))
}

/// One relay of a link: reconnects with a growing wait, and picks up after
/// the last message read there.
async fn listen_on(wire: Arc<Wire>, link: Link, base: String, open: Arc<AtomicUsize>, tx: mpsc::Sender<Incoming>, mut stop: watch::Receiver<bool>) {
    let relay = wire.relays().relay(&base);
    let topic = link.inbox().topic;
    let main = base == link.relay;
    let mut wait = Duration::from_secs(2);
    loop {
        if *stop.borrow() {
            return;
        }
        let since = since_on(&wire, &link, &base);
        match relay.subscribe(std::slice::from_ref(&topic), since.as_deref()).await {
            Ok(mut rx) => {
                let mut opened = false;
                loop {
                    tokio::select! {
                        ev = rx.recv() => match ev {
                            Some(RelayEvent::Open) => {
                                wait = Duration::from_secs(2);
                                // A backup never read: from now on, not the last while again.
                                if !main {
                                    let now = (wire.now_ms() / 1000).to_string();
                                    let _ = wire.links.update(&link.id, |p| {
                                        p.since_on.entry(base.clone()).or_insert(now);
                                    });
                                }
                                if !opened {
                                    opened = true;
                                    if open.fetch_add(1, Ordering::SeqCst) == 0 && tx.send(Incoming::Opened(link.id.clone())).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            Some(RelayEvent::Message { id, body, .. }) => {
                                let _ = wire.links.update(&link.id, |p| {
                                    if main {
                                        p.since = Some(id);
                                    } else {
                                        p.since_on.insert(base.clone(), id);
                                    }
                                });
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
                }
                if opened {
                    open.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(e) => {
                tracing::info!("Bridge link ({}): {}", host(&base), relay_error(e));
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

/// A relay's host name for the log (never the topic).
fn host(base: &str) -> &str {
    base.split("://").nth(1).unwrap_or(base).split('/').next().unwrap_or(base)
}
