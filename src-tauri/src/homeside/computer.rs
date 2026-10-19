//! A computer as the home side: when Platter runs on a computer that has
//! Lidarr set up, the user's other paired devices (a phone, a laptop away
//! from home) reach Lidarr through it over their device link, the local
//! network first and the sealed relay away. Nothing new on the server.
//!
//! The answering half is [`ComputerServes`]: the Bridge's own
//! [`route::serve`] (the same read allowlist, the same write vetting, the
//! same secret stripping), fed this computer's own address and key. The
//! key stays here: it only ever goes to a host it was entered for, and a
//! last scan holds back any answer that still has it in.
//!
//! The asking half is [`ComputerRoute`], a Lidarr route like the home and
//! away addresses, tried after them and before a Bridge.
//!
//! Only on a device link (never a Bridge link), only after the other
//! device's hello was checked (same Plex account) or its request was
//! signed on the local network, and only while Let my other devices use
//! Lidarr through this computer is on (`crate::sync::devices::protocol`).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::{oneshot, watch};

use super::link::Link;
use super::route::{self, ANSWER_WAIT};
use super::wire::Msg;
use crate::artistinfo::http::HttpResponse;
use crate::error::{AppError, AppResult};
use crate::lidarr::Lidarr;
use crate::lidarr::api::{LidarrGet, Route};
use crate::lidarr::relayed;
use crate::lidarr::write::{LidarrWrite, WriteCall};
use crate::plex::token::Token;
use crate::social::listen::seal;
use crate::sync::devices::DeviceSync;

/// The answer when the setting is off on the computer asked.
pub const SETTING_OFF: &str =
    "That computer doesn't let your other devices use Lidarr. Turn on Let my other devices use Lidarr through this computer there.";
/// The answer when the computer asked has no Lidarr set up.
pub const NOT_SET_UP: &str = "Lidarr isn't set up on that computer any more.";
/// The answer when the device asking isn't checked yet.
pub const NOT_CHECKED: &str = "That computer couldn't check this device yet. Try again in a moment.";
/// The answer when Lidarr's answer is too big for the relay.
pub const TOO_BIG: &str = "Lidarr's answer is too big to send through the relay. It goes through when this device is on the same network as your computer.";
/// The answer when an answer still had a secret in it.
const HELD_BACK: &str = "That computer held back Lidarr's answer because it had a key in it.";

/// Answers Lidarr requests from the user's other devices on this device.
#[async_trait]
pub trait HomeServe: Send + Sync {
    /// Whether this device offers it now (said in its hello).
    fn offers(&self) -> bool;
    /// The answer to one `Get` or `Write` (None for anything else).
    async fn answer(&self, msg: &Msg) -> Option<Msg>;
}

/// This computer's Lidarr, for the user's other devices.
pub struct ComputerServes {
    pub lidarr: Lidarr,
    pub db: crate::db::DbHandle,
    /// Let my other devices use Lidarr through this computer.
    pub on: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// The id a `Get` or `Write` carries.
pub fn request_id(msg: &Msg) -> Option<u64> {
    match msg {
        Msg::Get { id, .. } | Msg::Write { id, .. } => Some(*id),
        _ => None,
    }
}

/// The Bridge's words, as a computer says them.
fn as_computer(message: &str) -> String {
    message
        .replace("Your Bridge", "Your computer at home")
        .replace("The Bridge", "Your computer at home")
        .replace("the Bridge", "your computer at home")
}

/// The last look at an answer before it leaves: keys and passwords taken
/// out of a JSON body (reads already are, writes too now), and anything
/// that still has one of `secrets` in it is held back.
pub fn scrub(msg: Msg, secrets: &[Token]) -> Msg {
    match msg {
        Msg::Answer { id, status, body } => {
            let bytes = seal::unb64(&body).unwrap_or_default();
            // Anything but JSON (a proxy's sign-in page, say) goes as its
            // status only, as reads already do.
            let clean = if serde_json::from_slice::<serde_json::Value>(&bytes).is_ok() { relayed::strip_secrets(&bytes).to_vec() } else { Vec::new() };
            if has_secret(&String::from_utf8_lossy(&clean), secrets) {
                tracing::warn!("home side: an answer had a key in it and was held back");
                return Msg::Failed { id, message: HELD_BACK.into() };
            }
            Msg::Answer { id, status, body: seal::b64(&clean) }
        }
        Msg::Failed { id, message } => {
            let message = as_computer(&message);
            if has_secret(&message, secrets) {
                return Msg::Failed { id, message: HELD_BACK.into() };
            }
            Msg::Failed { id, message }
        }
        m => m,
    }
}

/// Whether `text` has one of `secrets` in it, as it is or in base64.
fn has_secret(text: &str, secrets: &[Token]) -> bool {
    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
    secrets.iter().map(Token::expose).filter(|s| !s.is_empty()).any(|s| {
        // The base64 of a key, cut where it doesn't depend on what comes
        // before or after it, so it's found wherever it starts.
        let whole = [STANDARD_NO_PAD.encode(s), URL_SAFE_NO_PAD.encode(s)];
        text.contains(s) || whole.iter().any(|w| text.contains(w.as_str()))
    })
}

/// Most of Lidarr's answer one device gets at once (8 MB).
pub const ANSWER_MAX: usize = 8 * 1024 * 1024;
/// The answer when it's bigger than that.
pub const ANSWER_TOO_BIG: &str = "Lidarr's answer is too big to send to another device.";
/// Most of a write's answer kept to answer it again (it came twice).
const KEEP_MAX: usize = 64 * 1024;

/// An answer past [`ANSWER_MAX`] becomes a plain refusal.
pub fn cap(msg: Msg) -> Msg {
    match msg {
        Msg::Answer { id, body, .. } if body.len() / 4 * 3 > ANSWER_MAX => Msg::Failed { id, message: ANSWER_TOO_BIG.into() },
        m => m,
    }
}

/// What's kept of a write's answer for answering it again: a big body is
/// left out (the status still says how it went).
pub fn kept(msg: &Msg) -> Msg {
    match msg {
        Msg::Answer { id, status, body } if body.len() > KEEP_MAX => Msg::Answer { id: *id, status: *status, body: String::new() },
        m => m.clone(),
    }
}

// ---------------------------------------------------------------- how often

/// Requests one device may have in flight here at once.
pub const IN_FLIGHT_MAX: usize = 4;
/// Requests one device may make a minute, and in one burst.
pub const PER_MINUTE: f64 = 30.0;
pub const BURST: f64 = 10.0;

#[derive(Debug, Clone, Copy)]
struct Budget {
    in_flight: usize,
    tokens: f64,
    at_ms: u64,
}

/// How often each linked device may ask: a few at once, and about one
/// every two seconds after a short burst.
#[derive(Default)]
pub struct Limits {
    links: Mutex<HashMap<String, Budget>>,
}

/// One request being answered; its place frees when dropped.
pub struct Permit<'a> {
    limits: &'a Limits,
    link: String,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if let Some(b) = self.limits.links.lock().get_mut(&self.link) {
            b.in_flight = b.in_flight.saturating_sub(1);
        }
    }
}

impl Limits {
    /// A place for one more request from `link` now, or None when it's
    /// asking too much.
    pub fn take(&self, link: &str, now_ms: u64) -> Option<Permit<'_>> {
        let mut all = self.links.lock();
        let b = all.entry(link.to_string()).or_insert(Budget { in_flight: 0, tokens: BURST, at_ms: now_ms });
        let gained = now_ms.saturating_sub(b.at_ms) as f64 * PER_MINUTE / 60_000.0;
        b.tokens = (b.tokens + gained).min(BURST);
        b.at_ms = b.at_ms.max(now_ms);
        if b.in_flight >= IN_FLIGHT_MAX || b.tokens < 1.0 {
            return None;
        }
        b.tokens -= 1.0;
        b.in_flight += 1;
        Some(Permit { limits: self, link: link.to_string() })
    }

    /// Forget a link (it unpaired).
    pub fn forget(&self, link: &str) {
        self.links.lock().remove(link);
    }
}

/// The answer when a device asks too often.
pub fn too_many(name: &str) -> String {
    format!("Too many requests from {name} right now. Try again in a moment.")
}

#[async_trait]
impl HomeServe for ComputerServes {
    fn offers(&self) -> bool {
        (self.on)() && self.lidarr.home_side_route().is_some()
    }

    async fn answer(&self, msg: &Msg) -> Option<Msg> {
        let id = request_id(msg)?;
        if !(self.on)() {
            return Some(Msg::Failed { id, message: SETTING_OFF.into() });
        }
        let Some((home, secrets)) = self.lidarr.home_side_route() else {
            return Some(Msg::Failed { id, message: NOT_SET_UP.into() });
        };
        let staging = crate::settings::load(&self.db).lidarr_staging_folder;
        // The Bridge's own checks, one code path.
        let out = route::serve(Some(&home), &staging, msg).await?;
        tracing::debug!(kind = msg.kind(), answer = out.kind(), "home side: answered another device");
        Some(cap(scrub(out, &secrets)))
    }
}

// ---------------------------------------------------------------- answered once

/// Writes already answered (or being answered), per link, so one that
/// comes twice (the local network gave up waiting, then the relay brought
/// it) gets the same answer and is never made twice. Reads aren't kept:
/// asking again is harmless. What's kept of an answer is small ([`kept`]).
#[derive(Default)]
pub struct Served {
    list: Mutex<VecDeque<(ServedKey, watch::Receiver<Option<Msg>>)>>,
}

/// A link's id and a request's id.
type ServedKey = (String, u64);

/// Requests remembered at most.
const SERVED_MAX: usize = 64;

pub enum Turn {
    /// New: answer it, and send what's [`kept`] of the answer.
    New(watch::Sender<Option<Msg>>),
    /// Answered before, or being answered: wait on this.
    Again(watch::Receiver<Option<Msg>>),
}

impl Served {
    pub fn start(&self, link: &str, id: u64) -> Turn {
        let mut l = self.list.lock();
        let key = (link.to_string(), id);
        if let Some((_, rx)) = l.iter().find(|(k, _)| *k == key) {
            return Turn::Again(rx.clone());
        }
        let (tx, rx) = watch::channel(None);
        if l.len() >= SERVED_MAX {
            l.pop_front();
        }
        l.push_back((key, rx));
        Turn::New(tx)
    }

    /// Forget a link's requests (it unpaired).
    pub fn forget(&self, link: &str) {
        self.list.lock().retain(|((l, _), _)| l != link);
    }

    /// Forget one request that wasn't made (refused before it started), so
    /// asking again later is a new request.
    pub fn forget_one(&self, link: &str, id: u64) {
        self.list.lock().retain(|((l, i), _)| !(l == link && *i == id));
    }

    /// How many answers are kept (for tests).
    pub fn len(&self) -> usize {
        self.list.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------- asking

type Waiting = Mutex<HashMap<u64, (String, oneshot::Sender<Result<HttpResponse, String>>)>>;

/// Requests this device sent to a computer, waiting for its answer.
pub struct Answers {
    next: AtomicU64,
    waiting: Waiting,
}

impl Default for Answers {
    fn default() -> Self {
        // A fresh start each run, so an id never matches one the computer
        // answered for an earlier run.
        let r = seal::random16();
        let start = u64::from_le_bytes([r[0], r[1], r[2], r[3], r[4], r[5], 0, 0]);
        Self { next: AtomicU64::new(start), waiting: Mutex::default() }
    }
}

impl Answers {
    pub(crate) fn add(&self, link: &str) -> (u64, oneshot::Receiver<Result<HttpResponse, String>>) {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = oneshot::channel();
        self.waiting.lock().insert(id, (link.to_string(), tx));
        (id, rx)
    }

    fn drop_id(&self, id: u64) {
        self.waiting.lock().remove(&id);
    }

    /// Give up on every request that went to `link` (it unpaired).
    pub fn forget(&self, link: &str) {
        self.waiting.lock().retain(|_, (l, _)| l != link);
    }

    /// An answer (or a refusal) came on `link`: taken only for a request
    /// that went to that link.
    pub fn resolve(&self, link: &str, id: u64, r: Result<HttpResponse, String>) {
        let mut w = self.waiting.lock();
        if w.get(&id).is_some_and(|(l, _)| l == link)
            && let Some((_, tx)) = w.remove(&id)
        {
            let _ = tx.send(r);
        }
    }
}

/// Lidarr through one of the user's computers, over their device link.
pub struct ComputerRoute {
    pub sync: Arc<DeviceSync>,
    pub link: Link,
    /// The computer's name, for messages.
    pub name: String,
}

impl ComputerRoute {
    async fn ask(&self, make: impl FnOnce(u64) -> Msg) -> AppResult<HttpResponse> {
        let answers = &self.sync.peer.answers;
        let (id, rx) = answers.add(&self.link.id);
        if let Err(e) = self.sync.ask_home(&self.link, &self.name, make(id)).await {
            answers.drop_id(id);
            return Err(e);
        }
        match tokio::time::timeout(ANSWER_WAIT, rx).await {
            Ok(Ok(Ok(r))) => Ok(r),
            Ok(Ok(Err(why))) => Err(AppError::Network(why)),
            _ => {
                answers.drop_id(id);
                Err(AppError::Network(format!("{} didn't answer in time.", self.name)))
            }
        }
    }
}

#[async_trait]
impl LidarrGet for ComputerRoute {
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> AppResult<HttpResponse> {
        let path = path.to_string();
        let query: Vec<(String, String)> = query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.ask(|id| Msg::Get { id, path, query }).await
    }

    fn route(&self) -> Route {
        Route::Computer
    }
}

#[async_trait]
impl LidarrWrite for ComputerRoute {
    async fn write(&self, call: &WriteCall) -> AppResult<HttpResponse> {
        let (method, path, body) = (call.method().as_str().to_string(), call.path(), call.body());
        self.ask(|id| Msg::Write { id, method, path, body }).await
    }

    fn route(&self) -> Route {
        Route::Computer
    }
}
