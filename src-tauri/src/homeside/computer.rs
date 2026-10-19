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
            let clean = if serde_json::from_slice::<serde_json::Value>(&bytes).is_ok() { relayed::strip_secrets(&bytes).to_vec() } else { bytes };
            let text = String::from_utf8_lossy(&clean);
            if secrets.iter().any(|s| !s.expose().is_empty() && text.contains(s.expose())) {
                tracing::warn!("home side: an answer had a key in it and was held back");
                return Msg::Failed { id, message: HELD_BACK.into() };
            }
            Msg::Answer { id, status, body: seal::b64(&clean) }
        }
        Msg::Failed { id, message } => {
            let message = as_computer(&message);
            if secrets.iter().any(|s| !s.expose().is_empty() && message.contains(s.expose())) {
                return Msg::Failed { id, message: HELD_BACK.into() };
            }
            Msg::Failed { id, message }
        }
        m => m,
    }
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
        Some(scrub(out, &secrets))
    }
}

// ---------------------------------------------------------------- answered once

/// Requests already answered (or being answered), per link, so one that
/// comes twice (the local network gave up waiting, then the relay brought
/// it) gets the same answer and a write is never made twice.
#[derive(Default)]
pub struct Served {
    list: Mutex<VecDeque<(ServedKey, watch::Receiver<Option<Msg>>)>>,
}

/// A link's id and a request's id.
type ServedKey = (String, u64);

/// Requests remembered at most.
const SERVED_MAX: usize = 128;

pub enum Turn {
    /// New: answer it, then [`Served::done`].
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
