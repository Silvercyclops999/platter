//! The relay every listen along message goes through, so participants only
//! ever connect to it and never to each other.
//!
//! [`Relay`] is the seam: [`Ntfy`] talks to an ntfy server (ntfy.sh by
//! default, or the user's own), and another relay can stand in later.
//!
//! ntfy, as used here:
//! - publish: `POST <base>/<topic>` with the sealed text as the body, and
//!   `X-Firebase: no` so ntfy.sh doesn't pass it on to Firebase.
//! - subscribe: `GET <base>/<t1>,<t2>/json[?since=<id>]`, one JSON object a
//!   line: `{"event":"open"|"keepalive"|"message", "id", "time", "topic",
//!   "message"}`. Messages carry no publisher identity or address.
//!
//! Errors never include the URL (it has topic names in it).
//!
//! ntfy.sh lets one address publish about 250 messages a day, refilled
//! evenly (about 10 an hour), and a home's computers, a phone on its Wi-Fi
//! and the Bridge all share that address. So once it says no, nothing more
//! is published there for a while ([`Cool`]), background work takes from a
//! small allowance per device ([`Allowance`]), and the log says each hour
//! what went out ([`note`]).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use serde::Serialize;
use tokio::sync::mpsc;

/// What a subscription delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayEvent {
    /// Connected and listening.
    Open,
    Message { id: String, topic: String, body: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayError {
    Unreachable,
    /// The relay is turning requests away for a while (ntfy answers 429).
    Limited,
    /// Any other refusal (status code).
    Refused(u16),
}

/// How the relay connection is doing, for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RelayHealth {
    #[default]
    Connecting,
    Ok,
    Unreachable,
    Limited,
}

impl From<RelayError> for RelayHealth {
    fn from(e: RelayError) -> Self {
        match e {
            RelayError::Limited => RelayHealth::Limited,
            _ => RelayHealth::Unreachable,
        }
    }
}

#[async_trait]
pub trait Relay: Send + Sync {
    async fn publish(&self, topic: &str, body: &str) -> Result<(), RelayError>;
    /// Whether background work may publish this many messages now, taking
    /// the room for them if so. Always yes, except on the public relays
    /// (see [`Allowance`] and [`Burst`]).
    fn room_for(&self, _messages: usize) -> bool {
        true
    }
    /// Listen on these topics. The channel closes when the connection ends.
    async fn subscribe(&self, topics: &[String], since: Option<&str>) -> Result<mpsc::Receiver<RelayEvent>, RelayError>;
}

/// Makes a relay for a base URL.
pub trait RelayFactory: Send + Sync {
    fn relay(&self, base: &str) -> Arc<dyn Relay>;
}

/// ntfy servers.
pub struct Ntfy {
    http: reqwest::Client,
    /// No bytes (not even a keepalive) for this long: the stream is dead.
    idle: Duration,
}

impl Ntfy {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "relay http client".to_string())?;
        // ntfy sends a keepalive every 45 s by default.
        Ok(Self { http, idle: Duration::from_secs(120) })
    }
}

impl RelayFactory for Ntfy {
    fn relay(&self, base: &str) -> Arc<dyn Relay> {
        Arc::new(NtfyRelay { http: self.http.clone(), base: base.trim_end_matches('/').to_string(), idle: self.idle })
    }
}

pub struct NtfyRelay {
    http: reqwest::Client,
    base: String,
    idle: Duration,
}

impl NtfyRelay {
    /// Only the public servers (ntfy.sh and its backups) are paced by an
    /// allowance: a server of the user's own has whatever limits they gave it.
    fn paced(&self) -> bool {
        self.base == super::DEFAULT_RELAY || super::BACKUP_RELAYS.contains(&self.base.as_str())
    }

    /// Wait for room in this relay's run of requests (see [`Burst`]), so a
    /// lot at once (say every link starting up) goes spread out instead of
    /// being turned away. False when it would wait too long.
    async fn turn(&self) -> bool {
        if !self.paced() {
            return true;
        }
        let give_up = Instant::now() + BURST_WAIT_MOST;
        loop {
            let now = Instant::now();
            let wait = match pace_entry(&mut PACE.lock(), &self.base).burst.take(now) {
                Ok(()) => return true,
                Err(wait) => wait,
            };
            if now + wait > give_up {
                return false;
            }
            tokio::time::sleep(wait).await;
        }
    }
}

fn status_err(code: u16) -> RelayError {
    if code == 429 { RelayError::Limited } else { RelayError::Refused(code) }
}

/// The first wait after the relay said no, doubled each time it says no
/// again, up to [`COOL_MAX`].
pub const COOL_FIRST: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(15 * 60) };
pub const COOL_MAX: Duration = if cfg!(test) { Duration::from_secs(1) } else { Duration::from_secs(2 * 3600) };

/// The same for the backup relays. They have messages to spare all day, so
/// a no from one is about too many at once: a short pause is enough.
pub const BACKUP_COOL_FIRST: Duration = if cfg!(test) { Duration::from_millis(100) } else { Duration::from_secs(10) };
pub const BACKUP_COOL_MAX: Duration = if cfg!(test) { Duration::from_millis(400) } else { Duration::from_secs(2 * 60) };

/// After the relay turned a message away (it answers 429), nothing more is
/// published there until the wait is over: every try counts against the
/// limit, so trying again at once only keeps it reached.
#[derive(Debug, Default)]
pub struct Cool {
    until: Option<Instant>,
    next: Duration,
}

impl Cool {
    pub fn waiting(&self, now: Instant) -> bool {
        self.until.is_some_and(|u| now < u)
    }

    /// The relay said no: wait, longer each time in a row.
    pub fn turned_away(&mut self, now: Instant) {
        self.wait(now, COOL_FIRST, COOL_MAX);
    }

    /// [`Cool::turned_away`] for this relay: a backup only pauses briefly.
    pub fn turned_away_by(&mut self, base: &str, now: Instant) {
        if super::BACKUP_RELAYS.contains(&base) {
            self.wait(now, BACKUP_COOL_FIRST, BACKUP_COOL_MAX);
        } else {
            self.turned_away(now);
        }
    }

    /// Wait `first`, then twice as long each time in a row, up to `most`.
    pub fn wait(&mut self, now: Instant, first: Duration, most: Duration) {
        let wait = if self.next.is_zero() { first } else { self.next };
        self.until = Some(now + wait);
        self.next = (wait * 2).min(most);
    }

    /// A message went: back to normal.
    pub fn went(&mut self) {
        *self = Self::default();
    }
}

/// How long a relay that couldn't be reached (or refused) is left alone
/// when there are others to use, doubled each time in a row.
pub const DOWN_FIRST: Duration = if cfg!(test) { Duration::from_secs(2) } else { Duration::from_secs(60) };
pub const DOWN_MAX: Duration = if cfg!(test) { Duration::from_secs(4) } else { Duration::from_secs(30 * 60) };

/// Background messages one device may publish to ntfy.sh at once, and how
/// many come back an hour. Four devices at one home together stay near
/// the address's ten an hour, with room left for requests and answers,
/// which never take from it.
pub const ALLOWANCE_MOST: f64 = 30.0;
pub const ALLOWANCE_PER_HOUR: f64 = 4.0;

/// The same for the backup relays. They allow an address about 17,000
/// messages in 12 hours (1,440 an hour), so four devices at one home at
/// this pace use about a third of it. What may go at once stays small:
/// [`Burst`] is what really limits a run of messages there.
pub const BACKUP_ALLOWANCE_MOST: f64 = 40.0;
pub const BACKUP_ALLOWANCE_PER_HOUR: f64 = 120.0;

/// Requests (publishes and listens) one device may make to a public relay
/// in a row, and how often one more comes back. The relays let an address
/// make about 60 in a row and then one every 5 seconds, and every device
/// at a home shares that, so one device keeps to a third of the run and
/// a slower pace.
pub const BURST_MOST: f64 = 20.0;
pub const BURST_EVERY: Duration = if cfg!(test) { Duration::from_millis(50) } else { Duration::from_secs(6) };
/// What background work leaves for requests and answers.
pub const BURST_KEEP: f64 = 5.0;
/// How long a request waits for its turn before giving up on that relay
/// (any other relays carry on).
pub const BURST_WAIT_MOST: Duration = if cfg!(test) { Duration::from_millis(500) } else { Duration::from_secs(30) };

/// The run of requests this device may still make to one relay right now.
/// Requests and answers wait their turn; background work only goes when
/// it leaves [`BURST_KEEP`] for them, and otherwise waits for a later round.
#[derive(Debug)]
pub struct Burst {
    left: f64,
    at: Option<Instant>,
}

impl Default for Burst {
    fn default() -> Self {
        Self { left: BURST_MOST, at: None }
    }
}

impl Burst {
    fn fill(&mut self, now: Instant) {
        if let Some(at) = self.at {
            let back = now.saturating_duration_since(at).as_secs_f64() / BURST_EVERY.as_secs_f64();
            self.left = (self.left + back).min(BURST_MOST);
        }
        self.at = Some(now);
    }

    /// One request: taken, or how long until there's room for it.
    pub fn take(&mut self, now: Instant) -> Result<(), Duration> {
        self.fill(now);
        if self.left >= 1.0 {
            self.left -= 1.0;
            Ok(())
        } else {
            Err(BURST_EVERY.mul_f64(1.0 - self.left))
        }
    }

    /// Whether background work can make `n` requests now and still leave
    /// [`BURST_KEEP`]. A message too big for that goes once the run is full.
    pub fn fits(&mut self, n: usize, now: Instant) -> bool {
        self.fill(now);
        self.left >= (n as f64 + BURST_KEEP).min(BURST_MOST)
    }

    /// Take `n` requests that [`Burst::fits`] said there was room for.
    pub fn take_n(&mut self, n: usize) {
        self.left -= n as f64;
    }

    /// The relay said too many at once: start again from nothing.
    pub fn emptied(&mut self, now: Instant) {
        self.fill(now);
        self.left = self.left.min(0.0);
    }
}

/// What background work (synced changes, shared Lidarr state, lyrics) may
/// still publish to a public relay from this device. What doesn't fit
/// waits for the home network or a later round.
#[derive(Debug)]
pub struct Allowance {
    left: f64,
    at: Option<Instant>,
    most: f64,
    per_hour: f64,
}

impl Default for Allowance {
    fn default() -> Self {
        Self::new(ALLOWANCE_MOST, ALLOWANCE_PER_HOUR)
    }
}

impl Allowance {
    pub fn new(most: f64, per_hour: f64) -> Self {
        Self { left: most, at: None, most, per_hour }
    }

    /// The allowance for a relay: the backups' is bigger than ntfy.sh's.
    pub fn for_relay(base: &str) -> Self {
        if super::BACKUP_RELAYS.contains(&base) { Self::new(BACKUP_ALLOWANCE_MOST, BACKUP_ALLOWANCE_PER_HOUR) } else { Self::default() }
    }

    /// Take `n` messages if there's room. A message too big for the whole
    /// allowance goes once it's full, and the rest of it is owed.
    pub fn take(&mut self, n: usize, now: Instant) -> bool {
        if let Some(at) = self.at {
            let hours = now.saturating_duration_since(at).as_secs_f64() / 3600.0;
            self.left = (self.left + hours * self.per_hour).min(self.most);
        }
        self.at = Some(now);
        let n = n as f64;
        if self.left >= n.min(self.most) {
            self.left -= n;
            true
        } else {
            false
        }
    }
}

/// One relay's pace: its wait after a no, this device's background
/// allowance and its run of requests there.
struct Pace {
    cool: Cool,
    allowance: Allowance,
    burst: Burst,
}

fn pace_entry<'a>(pace: &'a mut HashMap<String, Pace>, base: &str) -> &'a mut Pace {
    pace.entry(base.to_string()).or_insert_with(|| Pace { cool: Cool::default(), allowance: Allowance::for_relay(base), burst: Burst::default() })
}

/// Per relay address.
static PACE: LazyLock<parking_lot::Mutex<HashMap<String, Pace>>> = LazyLock::new(Default::default);

/// What this device published in the hour so far, by kind, for the log.
#[derive(Default)]
struct Meter {
    since: Option<Instant>,
    sent: HashMap<&'static str, u32>,
    held: u32,
    turned_away: u32,
    /// Per relay host: sent there, turned away there.
    relays: std::collections::BTreeMap<String, (u32, u32)>,
}

static METER: LazyLock<parking_lot::Mutex<Meter>> = LazyLock::new(Default::default);

tokio::task_local! {
    /// What's being published, for the hourly count, and whether it's
    /// background work whose requests [`Relay::room_for`] already took
    /// (set with [`as_kind`]).
    static KIND: (&'static str, bool);
}

/// Publish under a kind name, so the hourly count can say what went.
/// `paid`: background work that [`Relay::room_for`] made room for.
pub async fn as_kind<F: std::future::Future>(kind: &'static str, paid: bool, f: F) -> F::Output {
    KIND.scope((kind, paid), f).await
}

fn kind_now() -> &'static str {
    KIND.try_with(|k| k.0).unwrap_or("other")
}

fn paid_now() -> bool {
    KIND.try_with(|k| k.1).unwrap_or(false)
}

/// What a count is about: `n` messages of a kind went, were held back to
/// stay under the limit, or were turned away by the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counted {
    Sent,
    Held,
    TurnedAway,
}

/// Count messages for the hourly line in the log ("relay: 23 messages in
/// the last hour (syncHello 6, entries 12, other 5)").
pub fn note(kind: &'static str, n: usize, what: Counted) {
    note_at(None, kind, n, what);
}

/// [`note`] for messages that went to (or were turned away by) one relay,
/// so the hourly line also says how each relay is doing.
fn note_at(host: Option<&str>, kind: &'static str, n: usize, what: Counted) {
    let mut m = METER.lock();
    let since = *m.since.get_or_insert_with(Instant::now);
    let n = n as u32;
    match what {
        Counted::Sent => *m.sent.entry(kind).or_default() += n,
        Counted::Held => m.held += n,
        Counted::TurnedAway => m.turned_away += n,
    }
    if let Some(h) = host {
        let r = m.relays.entry(h.to_string()).or_default();
        match what {
            Counted::Sent => r.0 += n,
            Counted::TurnedAway => r.1 += n,
            Counted::Held => {}
        }
    }
    if since.elapsed() >= Duration::from_secs(3600) {
        let line = meter_line(&m.sent, m.held, m.turned_away);
        let by = relays_line(&m.relays);
        tracing::info!("relay: {line}{by}");
        *m = Meter { since: Some(Instant::now()), ..Default::default() };
    }
}

/// How each relay did in the hour: "; by relay: ntfy.envs.net 12,
/// ntfy.sh 0 (3 turned away)". Empty with no relay counted.
pub fn relays_line(relays: &std::collections::BTreeMap<String, (u32, u32)>) -> String {
    if relays.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = relays
        .iter()
        .map(|(h, (sent, away))| if *away > 0 { format!("{h} {sent} ({away} turned away)") } else { format!("{h} {sent}") })
        .collect();
    format!("; by relay: {}", parts.join(", "))
}

/// A relay's host name for the log (never the topic).
pub fn host(base: &str) -> &str {
    base.split("://").nth(1).unwrap_or(base).split('/').next().unwrap_or(base)
}

/// The hourly line: the total, then each kind, most first.
pub fn meter_line(sent: &HashMap<&'static str, u32>, held: u32, turned_away: u32) -> String {
    let total: u32 = sent.values().sum();
    let mut kinds: Vec<(&&str, &u32)> = sent.iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let parts: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
    let mut line = format!("{total} messages in the last hour");
    if !parts.is_empty() {
        line.push_str(&format!(" ({})", parts.join(", ")));
    }
    if held > 0 {
        line.push_str(&format!(", {held} held back to stay under the limit"));
    }
    if turned_away > 0 {
        line.push_str(&format!(", {turned_away} turned away"));
    }
    line
}

fn ok_topic(t: &str) -> bool {
    !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// One line of the JSON stream.
pub fn parse_line(line: &str) -> Option<RelayEvent> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    match v.get("event")?.as_str()? {
        "open" => Some(RelayEvent::Open),
        "message" => Some(RelayEvent::Message {
            id: v.get("id")?.as_str()?.to_string(),
            topic: v.get("topic")?.as_str()?.to_string(),
            body: v.get("message")?.as_str()?.to_string(),
        }),
        _ => None,
    }
}

#[async_trait]
impl Relay for NtfyRelay {
    async fn publish(&self, topic: &str, body: &str) -> Result<(), RelayError> {
        if !ok_topic(topic) {
            return Err(RelayError::Refused(400));
        }
        if PACE.lock().get(&self.base).is_some_and(|p| p.cool.waiting(Instant::now())) {
            note(kind_now(), 1, Counted::Held);
            return Err(RelayError::Limited);
        }
        if !paid_now() && !self.turn().await {
            note(kind_now(), 1, Counted::Held);
            return Err(RelayError::Limited);
        }
        let resp = self
            .http
            .post(format!("{}/{topic}", self.base))
            .header("X-Firebase", "no")
            .header("content-type", "text/plain")
            .body(body.to_string())
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|_| RelayError::Unreachable)?;
        let code = resp.status().as_u16();
        let res = if resp.status().is_success() { Ok(()) } else { Err(status_err(code)) };
        let mut pace = PACE.lock();
        let p = pace_entry(&mut pace, &self.base);
        match res {
            Ok(()) => {
                p.cool.went();
                drop(pace);
                note_at(Some(host(&self.base)), kind_now(), 1, Counted::Sent);
            }
            Err(RelayError::Limited) => {
                let now = Instant::now();
                p.cool.turned_away_by(&self.base, now);
                p.burst.emptied(now);
                drop(pace);
                tracing::info!("relay {}: it's turning messages away; nothing more goes to it for a while (any other relays carry on)", host(&self.base));
                note_at(Some(host(&self.base)), kind_now(), 1, Counted::TurnedAway);
            }
            Err(_) => {}
        }
        res
    }

    fn room_for(&self, messages: usize) -> bool {
        if !self.paced() {
            return true;
        }
        let now = Instant::now();
        let mut pace = PACE.lock();
        let p = pace_entry(&mut pace, &self.base);
        if !p.burst.fits(messages, now) || !p.allowance.take(messages, now) {
            return false;
        }
        p.burst.take_n(messages);
        true
    }

    async fn subscribe(&self, topics: &[String], since: Option<&str>) -> Result<mpsc::Receiver<RelayEvent>, RelayError> {
        if topics.is_empty() || !topics.iter().all(|t| ok_topic(t)) {
            return Err(RelayError::Refused(400));
        }
        if !self.turn().await {
            return Err(RelayError::Limited);
        }
        let mut req = self.http.get(format!("{}/{}/json", self.base, topics.join(",")));
        if let Some(id) = since.filter(|s| ok_topic(s)) {
            req = req.query(&[("since", id)]);
        }
        let resp = req.send().await.map_err(|_| RelayError::Unreachable)?;
        if !resp.status().is_success() {
            return Err(status_err(resp.status().as_u16()));
        }
        let (tx, rx) = mpsc::channel(64);
        let idle = self.idle;
        tokio::spawn(async move {
            let mut stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            loop {
                let chunk = match tokio::time::timeout(idle, stream.next()).await {
                    Ok(Some(Ok(c))) => c,
                    _ => return,
                };
                buf.extend_from_slice(&chunk);
                while let Some(i) = buf.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=i).collect();
                    if let Some(ev) = std::str::from_utf8(&line).ok().and_then(parse_line)
                        && tx.send(ev).await.is_err()
                    {
                        return;
                    }
                }
                if buf.len() > 256 * 1024 {
                    return;
                }
            }
        });
        Ok(rx)
    }
}
