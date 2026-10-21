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
    /// Whether background work may publish this many messages now. Always
    /// yes, except on ntfy.sh (see [`Allowance`]).
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
    /// Only ntfy.sh is paced by an allowance: a server of the user's own
    /// has whatever limits they gave it.
    fn paced(&self) -> bool {
        self.base == super::DEFAULT_RELAY
    }
}

fn status_err(code: u16) -> RelayError {
    if code == 429 { RelayError::Limited } else { RelayError::Refused(code) }
}

/// The first wait after the relay said no, doubled each time it says no
/// again, up to [`COOL_MAX`].
pub const COOL_FIRST: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(15 * 60) };
pub const COOL_MAX: Duration = if cfg!(test) { Duration::from_secs(1) } else { Duration::from_secs(2 * 3600) };

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
        let wait = if self.next.is_zero() { COOL_FIRST } else { self.next };
        self.until = Some(now + wait);
        self.next = (wait * 2).min(COOL_MAX);
    }

    /// A message went: back to normal.
    pub fn went(&mut self) {
        *self = Self::default();
    }
}

/// Background messages one device may publish to ntfy.sh at once, and how
/// many come back an hour. Four devices at one home together stay near
/// the address's ten an hour, with room left for requests and answers,
/// which never take from it.
pub const ALLOWANCE_MOST: f64 = 30.0;
pub const ALLOWANCE_PER_HOUR: f64 = 4.0;

/// What background work (synced changes, shared Lidarr state, lyrics) may
/// still publish to ntfy.sh from this device. What doesn't fit waits for
/// the home network or a later round.
#[derive(Debug)]
pub struct Allowance {
    left: f64,
    at: Option<Instant>,
}

impl Default for Allowance {
    fn default() -> Self {
        Self { left: ALLOWANCE_MOST, at: None }
    }
}

impl Allowance {
    /// Take `n` messages if there's room. A message too big for the whole
    /// allowance goes once it's full, and the rest of it is owed.
    pub fn take(&mut self, n: usize, now: Instant) -> bool {
        if let Some(at) = self.at {
            let hours = now.saturating_duration_since(at).as_secs_f64() / 3600.0;
            self.left = (self.left + hours * ALLOWANCE_PER_HOUR).min(ALLOWANCE_MOST);
        }
        self.at = Some(now);
        let n = n as f64;
        if self.left >= n.min(ALLOWANCE_MOST) {
            self.left -= n;
            true
        } else {
            false
        }
    }
}

/// Per relay address: its wait and this device's allowance there.
static PACE: LazyLock<parking_lot::Mutex<HashMap<String, (Cool, Allowance)>>> = LazyLock::new(Default::default);

/// What this device published in the hour so far, by kind, for the log.
struct Meter {
    since: Instant,
    sent: HashMap<&'static str, u32>,
    held: u32,
    turned_away: u32,
}

static METER: LazyLock<parking_lot::Mutex<Meter>> =
    LazyLock::new(|| parking_lot::Mutex::new(Meter { since: Instant::now(), sent: HashMap::new(), held: 0, turned_away: 0 }));

tokio::task_local! {
    /// What's being published, for the hourly count (set with [`as_kind`]).
    static KIND: &'static str;
}

/// Publish under a kind name, so the hourly count can say what went.
pub async fn as_kind<F: std::future::Future>(kind: &'static str, f: F) -> F::Output {
    KIND.scope(kind, f).await
}

fn kind_now() -> &'static str {
    KIND.try_with(|k| *k).unwrap_or("other")
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
    let mut m = METER.lock();
    let n = n as u32;
    match what {
        Counted::Sent => *m.sent.entry(kind).or_default() += n,
        Counted::Held => m.held += n,
        Counted::TurnedAway => m.turned_away += n,
    }
    if m.since.elapsed() >= Duration::from_secs(3600) {
        let line = meter_line(&m.sent, m.held, m.turned_away);
        tracing::info!("relay: {line}");
        *m = Meter { since: Instant::now(), sent: HashMap::new(), held: 0, turned_away: 0 };
    }
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
        if PACE.lock().get(&self.base).is_some_and(|(c, _)| c.waiting(Instant::now())) {
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
        let (cool, _) = pace.entry(self.base.clone()).or_default();
        match res {
            Ok(()) => {
                cool.went();
                drop(pace);
                note(kind_now(), 1, Counted::Sent);
            }
            Err(RelayError::Limited) => {
                cool.turned_away(Instant::now());
                drop(pace);
                tracing::info!("relay: it's turning messages away; nothing more goes to it for a while");
                note(kind_now(), 1, Counted::TurnedAway);
            }
            Err(_) => {}
        }
        res
    }

    fn room_for(&self, messages: usize) -> bool {
        if !self.paced() {
            return true;
        }
        PACE.lock().entry(self.base.clone()).or_default().1.take(messages, Instant::now())
    }

    async fn subscribe(&self, topics: &[String], since: Option<&str>) -> Result<mpsc::Receiver<RelayEvent>, RelayError> {
        if topics.is_empty() || !topics.iter().all(|t| ok_topic(t)) {
            return Err(RelayError::Refused(400));
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
