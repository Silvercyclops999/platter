//! Listen along: one person hosts, people on the same Plex server follow.
//!
//! The host's computer answers four routes on its Platter control port (the
//! one "Show this computer to my other devices" opens on the local network):
//!
//! - `POST /platter/v1/session/join`  ← [`JoinRequest`] → [`HostState`]
//! - `GET  /platter/v1/session/state` → [`HostState`]
//! - `POST /platter/v1/session/add`   ← [`AddRequest`] (when guests may add)
//! - `POST /platter/v1/session/leave`
//!
//! Guests sign every request with a key made from the secret in the invite
//! (`platter://join/<session>?k=…&h=…`), the same way Platter devices of one
//! account sign theirs, so only people holding the invite get in, each
//! request counts once, and the account token never leaves either computer.
//!
//! A guest asks for the host's state about once a second and follows it:
//! the same song and spot (half the request's round trip is added while
//! playing), pauses and skips. A host that stops answering for
//! [`ListenConfig::host_gone`] is reported gone; a host that ends the
//! session answers 410 and guests stop following.
//!
//! [`Transport`] carries the requests: HTTP on the local network in the app,
//! and an in-process link in tests. Another transport (a Plex play queue the
//! guests poll) can stand in when the two computers can't reach each other.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::error::{AppError, AppResult};
use crate::plex::state::EventSink;
use crate::remote::peer::auth::{self, PeerKey, Refused, Verifier, hex, now_ms};
use crate::remote::peer::server::ExtraRoutes;

pub const PATH_JOIN: &str = "/platter/v1/session/join";
pub const PATH_STATE: &str = "/platter/v1/session/state";
pub const PATH_ADD: &str = "/platter/v1/session/add";
pub const PATH_LEAVE: &str = "/platter/v1/session/leave";
const PREFIX: &str = "/platter/v1/session/";

pub const EVENT: &str = "listen://state";

/// Guests one session takes.
pub const MAX_GUESTS: usize = 10;
/// A guest this far from the host's spot seeks to it.
pub const DRIFT_MS: u64 = 1500;
/// Songs after the current one a guest is sent.
pub const UP_NEXT: usize = 50;
/// Songs a guest may add in one go.
pub const MAX_ADD: usize = 100;
const NAME_MAX: usize = 60;
const MAX_HOSTS: usize = 4;

/// Timing (shorter in tests).
#[derive(Debug, Clone, Copy)]
pub struct ListenConfig {
    /// How often a guest asks for the host's state.
    pub poll: Duration,
    /// No answer for this long: the host is gone.
    pub host_gone: Duration,
    /// A guest not heard from for this long has left.
    pub guest_timeout: Duration,
    /// How long one request may take.
    pub request_timeout: Duration,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            poll: Duration::from_secs(1),
            host_gone: Duration::from_secs(10),
            guest_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_secs(3),
        }
    }
}

// ---- invites ----------------------------------------------------------------

/// What a `platter://join/...` link carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    /// 16 hex digits.
    pub session: String,
    /// The secret the session key is made from (32 hex digits).
    pub secret: String,
    /// The Plex server's tag (see `share::link::server_tag`).
    pub server: Option<String>,
    /// Where the host answers, first to try first.
    pub hosts: Vec<SocketAddr>,
    /// The host's name, for "Listening along with …".
    pub name: String,
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Addresses an invite may point at: private LAN and link-local IPv4 (and
/// this computer, in tests).
fn host_ok(a: &SocketAddr) -> bool {
    match a.ip() {
        IpAddr::V4(v4) => crate::remote::peer::is_lan(v4) || (cfg!(test) && v4.is_loopback()),
        IpAddr::V6(_) => false,
    }
}

fn clean_name(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(NAME_MAX).collect::<String>().trim().to_string()
}

pub fn build_invite(i: &Invite) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    let hosts: Vec<String> = i.hosts.iter().take(MAX_HOSTS).map(|h| h.to_string()).collect();
    let mut s = format!("platter://join/{}?k={}", i.session, i.secret);
    if let Some(t) = &i.server {
        s.push_str(&format!("&s={t}"));
    }
    s.push_str(&format!("&h={}", enc(&hosts.join(","))));
    if !i.name.is_empty() {
        s.push_str(&format!("&n={}", enc(&i.name)));
    }
    s
}

/// Read an invite. None when it isn't one Platter can join.
pub fn parse_invite(raw: &str) -> Option<Invite> {
    let u = url::Url::parse(raw.trim()).ok()?;
    if !u.scheme().eq_ignore_ascii_case("platter") {
        return None;
    }
    let mut segs: Vec<String> = Vec::new();
    if let Some(h) = u.host_str().filter(|h| !h.is_empty()) {
        segs.push(h.to_ascii_lowercase());
    }
    segs.extend(u.path().split('/').filter(|s| !s.is_empty()).map(str::to_string));
    let [word, session] = segs.as_slice() else { return None };
    if word != "join" || !is_hex(session, 16) {
        return None;
    }
    let (mut secret, mut server, mut hosts, mut name) = (None, None, Vec::new(), String::new());
    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "k" if is_hex(&v, 32) => secret = Some(v.to_ascii_lowercase()),
            "s" if is_hex(&v, 8) => server = Some(v.to_ascii_lowercase()),
            "h" => {
                for part in v.split(',').take(MAX_HOSTS) {
                    let a: SocketAddr = part.trim().parse().ok()?;
                    if !host_ok(&a) || a.port() == 0 {
                        return None;
                    }
                    hosts.push(a);
                }
            }
            "n" => name = clean_name(&v),
            _ => {}
        }
    }
    if hosts.is_empty() {
        return None;
    }
    Some(Invite { session: session.to_ascii_lowercase(), secret: secret?, server, hosts, name })
}

fn new_hex(bytes: usize) -> String {
    let mut out = String::new();
    while out.len() < bytes * 2 {
        out.push_str(&hex(uuid::Uuid::new_v4().as_bytes()));
    }
    out.truncate(bytes * 2);
    out
}

// ---- the wire -------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddRequest {
    pub track_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestInfo {
    pub id: String,
    pub name: String,
}

/// What a guest follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostState {
    pub session_id: String,
    pub host_name: String,
    pub track_id: Option<String>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
    /// The songs after this one, in play order.
    pub up_next: Vec<String>,
    pub guests: Vec<GuestInfo>,
    pub guests_can_add: bool,
}

// ---- what the UI sees -----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    None,
    Host,
    Guest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    /// Joining.
    Connecting,
    Live,
    /// The host stopped answering.
    HostGone,
    /// The host ended the session.
    Ended,
}

/// `listen_state` and the `listen://state` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenView {
    pub role: Role,
    pub session_id: Option<String>,
    pub host_name: Option<String>,
    /// The link to send people (host only).
    pub invite: Option<String>,
    /// Who's listening along (names), host first for guests.
    pub listeners: Vec<String>,
    pub guests_can_add: bool,
    pub status: Option<SessionStatus>,
}

impl ListenView {
    pub fn none() -> Self {
        Self { role: Role::None, session_id: None, host_name: None, invite: None, listeners: Vec::new(), guests_can_add: false, status: None }
    }
}

/// For Discord: the session and how many are in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyInfo {
    pub id: String,
    pub size: u32,
    pub max: u32,
}

// ---- the player -------------------------------------------------------------

/// What playback is doing here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlayerNow {
    pub track_id: Option<String>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
    pub up_next: Vec<String>,
}

/// The local player, as a session needs it.
#[async_trait]
pub trait ListenPlayer: Send + Sync {
    async fn now(&self) -> AppResult<PlayerNow>;
    /// Play these songs from the first, at a spot, playing or paused.
    async fn play(&self, ids: Vec<String>, position_ms: u64, play: bool) -> AppResult<()>;
    async fn seek(&self, position_ms: u64) -> AppResult<()>;
    async fn pause(&self) -> AppResult<()>;
    async fn resume(&self) -> AppResult<()>;
    /// Add to the end of the queue.
    async fn queue_add(&self, ids: Vec<String>) -> AppResult<()>;
}

/// One step a guest takes to match the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Load { ids: Vec<String>, position_ms: u64, play: bool },
    Seek(u64),
    Pause,
    Resume,
}

/// Where the host is by the time its answer arrives: half the round trip
/// later while playing, never past the end.
pub fn target_ms(host: &HostState, rtt_ms: u64) -> u64 {
    let at = if host.playing { host.position_ms + rtt_ms / 2 } else { host.position_ms };
    if host.duration_ms > 0 { at.min(host.duration_ms) } else { at }
}

/// What a guest does to match the host.
pub fn plan(local: &PlayerNow, host: &HostState, rtt_ms: u64) -> Vec<Step> {
    let Some(track) = host.track_id.clone() else {
        return if local.playing { vec![Step::Pause] } else { Vec::new() };
    };
    let target = target_ms(host, rtt_ms);
    if local.track_id.as_deref() != Some(track.as_str()) {
        // The guest moved on to the next song a moment before the host did.
        let ahead = local.track_id.is_some()
            && host.up_next.first() == local.track_id.as_ref()
            && host.duration_ms.saturating_sub(target) <= 2 * DRIFT_MS;
        if ahead {
            return Vec::new();
        }
        let mut ids = vec![track];
        ids.extend(host.up_next.iter().take(UP_NEXT).cloned());
        return vec![Step::Load { ids, position_ms: target, play: host.playing }];
    }
    let mut steps = Vec::new();
    if local.position_ms.abs_diff(target) > DRIFT_MS {
        steps.push(Step::Seek(target));
    }
    match (host.playing, local.playing) {
        (true, false) => steps.push(Step::Resume),
        (false, true) => steps.push(Step::Pause),
        _ => {}
    }
    steps
}

async fn apply(player: &dyn ListenPlayer, steps: Vec<Step>) -> AppResult<()> {
    for s in steps {
        match s {
            Step::Load { ids, position_ms, play } => player.play(ids, position_ms, play).await?,
            Step::Seek(ms) => player.seek(ms).await?,
            Step::Pause => player.pause().await?,
            Step::Resume => player.resume().await?,
        }
    }
    Ok(())
}

// ---- transport --------------------------------------------------------------

/// Sends one signed request to a host: (status, body).
#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, host: &SocketAddr, method: &str, path: &str, auth: &str, body: Vec<u8>, timeout: Duration) -> AppResult<(u16, Vec<u8>)>;
}

/// HTTP on the local network.
pub struct HttpTransport(pub reqwest::Client);

impl HttpTransport {
    pub fn new() -> AppResult<Self> {
        reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .build()
            .map(Self)
            .map_err(|e| AppError::Internal(format!("http client: {e}")))
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn send(&self, host: &SocketAddr, method: &str, path: &str, auth: &str, body: Vec<u8>, timeout: Duration) -> AppResult<(u16, Vec<u8>)> {
        let url = format!("http://{host}{path}");
        let req = if method == "POST" {
            self.0.post(url).header("content-type", "application/json").body(body)
        } else {
            self.0.get(url)
        };
        let resp = req
            .header(auth::AUTH_HEADER, auth)
            .timeout(timeout)
            .send()
            .await
            .map_err(|_| AppError::Network("unreachable".into()))?;
        let status = resp.status().as_u16();
        let bytes = resp.bytes().await.map_err(|_| AppError::Network("unreachable".into()))?;
        Ok((status, bytes.to_vec()))
    }
}

// ---- the service ----------------------------------------------------------

struct HostGuest {
    id: String,
    name: String,
    seen: Instant,
}

struct Hosting {
    id: String,
    key: PeerKey,
    invite: String,
    guests: Vec<HostGuest>,
}

struct Following {
    id: String,
    key: PeerKey,
    host_name: String,
    hosts: Vec<SocketAddr>,
    status: SessionStatus,
    last: Option<HostState>,
}

enum Mode {
    Idle,
    Host(Hosting),
    Guest(Following),
}

/// Who this computer is, and the shared parts.
pub struct ListenDeps {
    pub player: Arc<dyn ListenPlayer>,
    pub transport: Arc<dyn Transport>,
    pub events: Arc<dyn EventSink>,
    /// This Platter's id (its Plex client identifier).
    pub own_id: String,
    /// The name others see (the signed-in Plex user's).
    pub name: Arc<dyn Fn() -> String + Send + Sync>,
    /// Let guests add to the queue (setting).
    pub guests_can_add: Arc<AtomicBool>,
    pub cfg: ListenConfig,
}

struct Inner {
    deps: ListenDeps,
    mode: Mutex<Mode>,
    verifier: Verifier,
    /// Bumped whenever following starts or stops; an older loop quits.
    generation: AtomicU64,
    party: tokio::sync::watch::Sender<Option<PartyInfo>>,
}

/// Listen along on this computer. Cheap to clone.
#[derive(Clone)]
pub struct ListenAlong {
    inner: Arc<Inner>,
}

fn json_err(code: &str, message: &str) -> String {
    serde_json::json!({ "error": code, "message": message }).to_string()
}

fn ok_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl ListenAlong {
    pub fn new(deps: ListenDeps) -> Self {
        Self {
            inner: Arc::new(Inner {
                deps,
                mode: Mutex::new(Mode::Idle),
                verifier: Verifier::new(),
                generation: AtomicU64::new(0),
                party: tokio::sync::watch::channel(None).0,
            }),
        }
    }

    /// The session and its size whenever it changes (Discord shows it).
    pub fn subscribe_party(&self) -> tokio::sync::watch::Receiver<Option<PartyInfo>> {
        self.inner.party.subscribe()
    }

    fn cfg(&self) -> ListenConfig {
        self.inner.deps.cfg
    }

    fn prune(&self, h: &mut Hosting) {
        let timeout = self.cfg().guest_timeout;
        h.guests.retain(|g| g.seen.elapsed() < timeout);
    }

    /// What the UI shows now.
    pub fn view(&self) -> ListenView {
        let can_add = self.inner.deps.guests_can_add.load(Ordering::Relaxed);
        let mut mode = self.inner.mode.lock();
        match &mut *mode {
            Mode::Idle => ListenView::none(),
            Mode::Host(h) => {
                self.prune(h);
                ListenView {
                    role: Role::Host,
                    session_id: Some(h.id.clone()),
                    host_name: Some((self.inner.deps.name)()),
                    invite: Some(h.invite.clone()),
                    listeners: h.guests.iter().map(|g| g.name.clone()).collect(),
                    guests_can_add: can_add,
                    status: Some(SessionStatus::Live),
                }
            }
            Mode::Guest(g) => {
                let mut listeners = vec![g.host_name.clone()];
                let me = &self.inner.deps.own_id;
                if let Some(last) = &g.last {
                    listeners.extend(last.guests.iter().filter(|x| &x.id != me).map(|x| x.name.clone()));
                }
                ListenView {
                    role: Role::Guest,
                    session_id: Some(g.id.clone()),
                    host_name: Some(g.host_name.clone()),
                    invite: None,
                    listeners,
                    guests_can_add: g.last.as_ref().is_some_and(|l| l.guests_can_add),
                    status: Some(g.status),
                }
            }
        }
    }

    fn changed(&self) {
        let v = self.view();
        let party = match v.role {
            Role::Host => v.session_id.clone().map(|id| PartyInfo {
                id,
                size: v.listeners.len() as u32 + 1,
                max: MAX_GUESTS as u32 + 1,
            }),
            _ => None,
        };
        self.inner.party.send_if_modified(|p| std::mem::replace(p, party.clone()) != party);
        self.inner.deps.events.emit(EVENT, serde_json::to_value(&v).unwrap_or_default());
    }

    // ---- hosting ----------------------------------------------------------

    /// Start hosting. `hosts` are this computer's control API addresses;
    /// `server` is the Plex server's tag.
    pub fn start(&self, server: Option<String>, hosts: Vec<SocketAddr>) -> AppResult<ListenView> {
        if hosts.is_empty() {
            return Err(AppError::Invalid(
                "Listening sessions need this computer on the local network with Show this computer to my other devices turned on in Settings.".into(),
            ));
        }
        {
            let mut mode = self.inner.mode.lock();
            match &*mode {
                Mode::Host(_) => {}
                Mode::Guest(_) => return Err(AppError::Invalid("Leave the listening session you're in first.".into())),
                Mode::Idle => {
                    let session = new_hex(8);
                    let secret = new_hex(16);
                    let invite = build_invite(&Invite {
                        session: session.clone(),
                        secret: secret.clone(),
                        server,
                        hosts,
                        name: clean_name(&(self.inner.deps.name)()),
                    });
                    *mode = Mode::Host(Hosting { id: session, key: PeerKey::from_secret(secret.as_bytes()), invite, guests: Vec::new() });
                }
            }
        }
        tracing::info!("listening session started");
        self.changed();
        Ok(self.view())
    }

    /// End the session (host) or leave it (guest).
    pub async fn stop(&self) -> AppResult<()> {
        let old = std::mem::replace(&mut *self.inner.mode.lock(), Mode::Idle);
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        match old {
            Mode::Idle => return Ok(()),
            Mode::Host(_) => tracing::info!("listening session ended"),
            Mode::Guest(g) => {
                if matches!(g.status, SessionStatus::Live | SessionStatus::Connecting) {
                    let _ = self.call(&g.key, &g.id, &g.hosts, "POST", PATH_LEAVE, b"{}".to_vec()).await;
                }
                tracing::info!("left the listening session");
            }
        }
        self.changed();
        Ok(())
    }

    async fn host_state(&self, id: &str, guests: Vec<GuestInfo>) -> HostState {
        let now = self.inner.deps.player.now().await.unwrap_or_default();
        HostState {
            session_id: id.to_string(),
            host_name: clean_name(&(self.inner.deps.name)()),
            track_id: now.track_id,
            position_ms: now.position_ms,
            duration_ms: now.duration_ms,
            playing: now.playing,
            up_next: now.up_next.into_iter().take(UP_NEXT).collect(),
            guests,
            guests_can_add: self.inner.deps.guests_can_add.load(Ordering::Relaxed),
        }
    }

    /// Answer a guest's request: (status, JSON body).
    pub async fn answer(&self, method: &str, target: &str, auth_header: Option<&str>, body: &[u8]) -> (u16, String) {
        let path = target.split('?').next().unwrap_or(target);
        let (id, key) = match &*self.inner.mode.lock() {
            Mode::Host(h) => (h.id.clone(), h.key.clone()),
            _ => return (410, json_err("ended", "That listening session has ended.")),
        };
        let from = match self.inner.verifier.verify(Some(&key), auth_header, method, path, &id, body, now_ms()) {
            Ok(from) => from,
            Err(r) => {
                tracing::info!(reason = ?r, "refused a listening session request");
                let msg = if r == Refused::Stale { "The two computers' clocks are too far apart." } else { "That invite isn't for this session." };
                return (401, json_err("refused", msg));
            }
        };
        let mut changed = false;
        let result = match (method, path) {
            ("POST", PATH_JOIN) => {
                let Ok(req) = serde_json::from_slice::<JoinRequest>(body) else {
                    return (400, json_err("invalid", "Bad request"));
                };
                let mut mode = self.inner.mode.lock();
                let Mode::Host(h) = &mut *mode else { return (410, json_err("ended", "That listening session has ended.")) };
                self.prune(h);
                if let Some(g) = h.guests.iter_mut().find(|g| g.id == from) {
                    g.seen = Instant::now();
                } else if h.guests.len() >= MAX_GUESTS {
                    return (409, json_err("full", "That listening session is full."));
                } else {
                    let name = clean_name(&req.name);
                    h.guests.push(HostGuest { id: from.clone(), name: if name.is_empty() { "Guest".into() } else { name }, seen: Instant::now() });
                    changed = true;
                }
                Ok(())
            }
            ("GET", PATH_STATE) => {
                let mut mode = self.inner.mode.lock();
                let Mode::Host(h) = &mut *mode else { return (410, json_err("ended", "That listening session has ended.")) };
                let before = h.guests.len();
                self.prune(h);
                changed = h.guests.len() != before;
                match h.guests.iter_mut().find(|g| g.id == from) {
                    Some(g) => {
                        g.seen = Instant::now();
                        Ok(())
                    }
                    None => Err((403, json_err("notJoined", "Join the session first."))),
                }
            }
            ("POST", PATH_ADD) => {
                if !self.inner.deps.guests_can_add.load(Ordering::Relaxed) {
                    return (403, json_err("notAllowed", "The host doesn't let guests add songs."));
                }
                let Ok(req) = serde_json::from_slice::<AddRequest>(body) else {
                    return (400, json_err("invalid", "Bad request"));
                };
                if req.track_ids.is_empty() || req.track_ids.len() > MAX_ADD || !req.track_ids.iter().all(|t| ok_id(t)) {
                    return (400, json_err("invalid", "Bad song ids."));
                }
                let joined = matches!(&*self.inner.mode.lock(), Mode::Host(h) if h.guests.iter().any(|g| g.id == from));
                if !joined {
                    return (403, json_err("notJoined", "Join the session first."));
                }
                match self.inner.deps.player.queue_add(req.track_ids).await {
                    Ok(()) => Ok(()),
                    Err(e) => Err((409, json_err(e.kind(), "Couldn't add those songs."))),
                }
            }
            ("POST", PATH_LEAVE) => {
                if let Mode::Host(h) = &mut *self.inner.mode.lock() {
                    let before = h.guests.len();
                    h.guests.retain(|g| g.id != from);
                    changed = h.guests.len() != before;
                }
                Ok(())
            }
            _ => return (404, json_err("notFound", "Not found")),
        };
        if changed {
            self.changed();
        }
        if let Err(e) = result {
            return e;
        }
        if path == PATH_LEAVE || path == PATH_ADD {
            return (200, "{}".into());
        }
        let guests = match &*self.inner.mode.lock() {
            Mode::Host(h) => h.guests.iter().map(|g| GuestInfo { id: g.id.clone(), name: g.name.clone() }).collect(),
            _ => Vec::new(),
        };
        (200, serde_json::to_string(&self.host_state(&id, guests).await).unwrap_or_default())
    }

    // ---- following --------------------------------------------------------

    /// Sign and send to the first host address that answers.
    async fn call(&self, key: &PeerKey, session: &str, hosts: &[SocketAddr], method: &str, path: &str, body: Vec<u8>) -> AppResult<(u16, Vec<u8>, u64)> {
        let mut last = AppError::Network("unreachable".into());
        for h in hosts {
            let header = auth::sign(key, method, path, &self.inner.deps.own_id, session, now_ms(), &auth::nonce(), &body);
            let sent = Instant::now();
            match self.inner.deps.transport.send(h, method, path, &header, body.clone(), self.cfg().request_timeout).await {
                Ok((status, bytes)) => return Ok((status, bytes, sent.elapsed().as_millis() as u64)),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn message_of(bytes: &[u8]) -> Option<String> {
        serde_json::from_slice::<serde_json::Value>(bytes).ok()?.get("message")?.as_str().map(str::to_string)
    }

    /// Join a session from its invite link. `server` is this computer's Plex
    /// server tag: a session on another server can't play here.
    pub async fn join(&self, link: &str, server: Option<String>) -> AppResult<ListenView> {
        let inv = parse_invite(link).ok_or_else(|| AppError::Invalid("That isn't a listening session link.".into()))?;
        if let (Some(theirs), Some(mine)) = (&inv.server, &server)
            && theirs != mine
        {
            return Err(AppError::Invalid("That session plays from a different Plex server.".into()));
        }
        let key = PeerKey::from_secret(inv.secret.as_bytes());
        {
            let mut mode = self.inner.mode.lock();
            match &*mode {
                Mode::Host(_) => return Err(AppError::Invalid("End your own listening session first.".into())),
                Mode::Guest(g) if g.id == inv.session && g.status == SessionStatus::Live => {
                    drop(mode);
                    return Ok(self.view());
                }
                _ => {}
            }
            *mode = Mode::Guest(Following {
                id: inv.session.clone(),
                key: key.clone(),
                host_name: if inv.name.is_empty() { "the host".into() } else { inv.name.clone() },
                hosts: inv.hosts.clone(),
                status: SessionStatus::Connecting,
                last: None,
            });
        }
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.changed();
        let body = serde_json::to_vec(&JoinRequest { name: clean_name(&(self.inner.deps.name)()) }).unwrap_or_default();
        let answer = self.call(&key, &inv.session, &inv.hosts, "POST", PATH_JOIN, body).await;
        let fail = |this: &Self, e: AppError| {
            if this.inner.generation.load(Ordering::SeqCst) == generation {
                *this.inner.mode.lock() = Mode::Idle;
                this.changed();
            }
            Err(e)
        };
        let (status, bytes, rtt) = match answer {
            Ok(a) => a,
            Err(_) => return fail(self, AppError::Network("Couldn't reach the host. You need to be on the same network.".into())),
        };
        if status == 410 {
            return fail(self, AppError::NotFound("That listening session has ended.".into()));
        }
        if status != 200 {
            let msg = Self::message_of(&bytes).unwrap_or_else(|| "The host didn't let you join.".into());
            return fail(self, AppError::Invalid(msg));
        }
        let Ok(state) = serde_json::from_slice::<HostState>(&bytes) else {
            return fail(self, AppError::Decode("unreadable answer".into()));
        };
        self.take_state(generation, state, rtt).await;
        let me = self.clone();
        tokio::spawn(async move { me.follow(generation).await });
        tracing::info!("joined a listening session");
        Ok(self.view())
    }

    /// A fresh host state: follow it and update the view.
    async fn take_state(&self, generation: u64, state: HostState, rtt: u64) {
        let mut changed = false;
        {
            let mut mode = self.inner.mode.lock();
            if self.inner.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            if let Mode::Guest(g) = &mut *mode {
                let names = |s: &Option<HostState>| s.as_ref().map(|s| (s.guests.clone(), s.guests_can_add, s.host_name.clone()));
                let next = Some(state.clone());
                changed = g.status != SessionStatus::Live || names(&g.last) != names(&next);
                g.status = SessionStatus::Live;
                if !state.host_name.is_empty() {
                    g.host_name = state.host_name.clone();
                }
                g.last = next;
            }
        }
        if changed {
            self.changed();
        }
        let player = &self.inner.deps.player;
        if let Ok(local) = player.now().await {
            let steps = plan(&local, &state, rtt);
            if !steps.is_empty()
                && let Err(e) = apply(player.as_ref(), steps).await
            {
                tracing::debug!(kind = e.kind(), "couldn't follow the host");
            }
        }
    }

    fn end_following(&self, generation: u64, status: SessionStatus) {
        {
            let mut mode = self.inner.mode.lock();
            if self.inner.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            if let Mode::Guest(g) = &mut *mode {
                g.status = status;
            }
        }
        tracing::info!(?status, "stopped following the listening session");
        self.changed();
    }

    async fn follow(self, generation: u64) {
        let mut last_ok = Instant::now();
        loop {
            tokio::time::sleep(self.cfg().poll).await;
            if self.inner.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            let Some((key, id, hosts)) = (match &*self.inner.mode.lock() {
                Mode::Guest(g) => Some((g.key.clone(), g.id.clone(), g.hosts.clone())),
                _ => None,
            }) else {
                return;
            };
            match self.call(&key, &id, &hosts, "GET", PATH_STATE, Vec::new()).await {
                Ok((200, bytes, rtt)) => match serde_json::from_slice::<HostState>(&bytes) {
                    Ok(state) => {
                        last_ok = Instant::now();
                        self.take_state(generation, state, rtt).await;
                    }
                    Err(_) => tracing::debug!("unreadable host state"),
                },
                Ok((410, _, _)) => return self.end_following(generation, SessionStatus::Ended),
                Ok((403, _, _)) => {
                    // Dropped after a long gap: join again.
                    last_ok = Instant::now();
                    let body = serde_json::to_vec(&JoinRequest { name: clean_name(&(self.inner.deps.name)()) }).unwrap_or_default();
                    let _ = self.call(&key, &id, &hosts, "POST", PATH_JOIN, body).await;
                }
                Ok((status, _, _)) => tracing::debug!(status, "host refused the state request"),
                Err(_) => {}
            }
            if last_ok.elapsed() >= self.cfg().host_gone {
                return self.end_following(generation, SessionStatus::HostGone);
            }
        }
    }

    /// Add songs to the shared queue (a guest's go through the host).
    pub async fn add(&self, track_ids: Vec<String>) -> AppResult<()> {
        if track_ids.is_empty() || track_ids.len() > MAX_ADD || !track_ids.iter().all(|t| ok_id(t)) {
            return Err(AppError::Invalid("Bad song ids.".into()));
        }
        let target = match &*self.inner.mode.lock() {
            Mode::Host(_) => None,
            Mode::Guest(g) if g.status == SessionStatus::Live => Some((g.key.clone(), g.id.clone(), g.hosts.clone())),
            _ => return Err(AppError::Invalid("You're not in a listening session.".into())),
        };
        let Some((key, id, hosts)) = target else {
            return self.inner.deps.player.queue_add(track_ids).await;
        };
        let body = serde_json::to_vec(&AddRequest { track_ids }).unwrap_or_default();
        let (status, bytes, _) = self
            .call(&key, &id, &hosts, "POST", PATH_ADD, body)
            .await
            .map_err(|_| AppError::Network("Couldn't reach the host.".into()))?;
        match status {
            200 => Ok(()),
            _ => Err(AppError::Invalid(Self::message_of(&bytes).unwrap_or_else(|| "The host didn't take those songs.".into()))),
        }
    }
}

#[async_trait]
impl ExtraRoutes for ListenAlong {
    fn handles(&self, path: &str) -> bool {
        path.starts_with(PREFIX)
    }

    async fn respond(&self, method: &str, target: &str, auth: Option<&str>, body: &[u8]) -> (u16, String) {
        self.answer(method, target, auth, body).await
    }
}
