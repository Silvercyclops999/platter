//! The receiving side: a small HTTP server on the LAN that lets the user's
//! other Platter devices start, follow and control playback here.
//!
//! Every request must be signed (see [`super::auth`]); anything else is
//! answered 401 and does nothing. JSON in and out, one request per
//! connection.
//!
//! - `GET  /platter/v1/hello`   → [`Hello`]
//! - `GET  /platter/v1/state`   → [`PeerTimeline`]
//! - `POST /platter/v1/play`    ← [`PlayRequest`]
//! - `POST /platter/v1/command` ← `RemoteCommand`
//! - `POST /platter/v1/stop`    (pauses here)

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::auth::{AUTH_HEADER, PeerKey, Refused, Verifier, now_ms};
use super::{Hello, MAX_SONGS, PATH_COMMAND, PATH_HELLO, PATH_PLAY, PATH_STATE, PATH_STOP, PeerTimeline, PlayRequest};
use crate::audio::events::PlayerStatus;
use crate::error::{AppError, AppResult};
use crate::remote::service::{LocalPlayer, RemoteEnv};
use crate::remote::types::RemoteCommand;

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 512 * 1024;

/// What the control API does.
#[async_trait]
pub trait PeerHandler: Send + Sync {
    async fn hello(&self) -> Hello;
    async fn timeline(&self) -> PeerTimeline;
    async fn play(&self, req: PlayRequest) -> AppResult<()>;
    async fn command(&self, cmd: RemoteCommand) -> AppResult<()>;
    async fn stop(&self) -> AppResult<()>;
}

/// Where the account key comes from (the signed-in account's token).
#[async_trait]
pub trait KeySource: Send + Sync {
    async fn key(&self) -> Option<PeerKey>;
}

pub struct EnvKey(pub Arc<dyn RemoteEnv>);

#[async_trait]
impl KeySource for EnvKey {
    async fn key(&self) -> Option<PeerKey> {
        self.0.account_token().await.map(|t| PeerKey::from_token(&t))
    }
}

/// More routes on the same port, with their own checks (a listening
/// session's guests sign with the session's key, not the account's).
#[async_trait]
pub trait ExtraRoutes: Send + Sync {
    fn handles(&self, path: &str) -> bool;
    async fn respond(&self, method: &str, target: &str, auth: Option<&str>, body: &[u8]) -> (u16, String);
}

/// Where the extra routes are plugged in (set once the app has them).
pub type ExtraSlot = Arc<parking_lot::RwLock<Option<Arc<dyn ExtraRoutes>>>>;

/// The control API: checks signatures, then hands requests to the handler.
pub struct PeerServer {
    pub own_id: String,
    pub handler: Arc<dyn PeerHandler>,
    pub keys: Arc<dyn KeySource>,
    verifier: Verifier,
    extra: ExtraSlot,
}

pub(crate) fn json_err(code: &str, message: &str) -> String {
    serde_json::json!({ "error": code, "message": message }).to_string()
}

pub(crate) fn refused(r: Refused) -> (u16, String) {
    let (code, msg) = match r {
        Refused::Missing => ("unsigned", "Only your own Platter devices can control this one."),
        Refused::BadSignature => ("notYourAccount", "That device isn't signed in to the same Plex account."),
        Refused::Stale => ("clock", "The two computers' clocks are too far apart."),
        Refused::Replayed => ("replayed", "That request was already used."),
        Refused::NoKey => ("signedOut", "This computer isn't signed in to Plex."),
    };
    (401, json_err(code, msg))
}

pub(crate) fn app_err(e: AppError) -> (u16, String) {
    let status = match &e {
        AppError::NotFound(_) => 404,
        AppError::Invalid(_) => 409,
        _ => 500,
    };
    let text = e.to_string();
    let prefix = format!("{}: ", e.kind());
    (status, json_err(e.kind(), text.strip_prefix(&prefix).unwrap_or(&text)))
}

impl PeerServer {
    pub fn new(own_id: String, handler: Arc<dyn PeerHandler>, keys: Arc<dyn KeySource>) -> Self {
        Self { own_id, handler, keys, verifier: Verifier::new(), extra: ExtraSlot::default() }
    }

    /// Answer the extra routes in `slot` too.
    pub fn with_extra(mut self, slot: ExtraSlot) -> Self {
        self.extra = slot;
        self
    }

    /// Answer one request: (status, JSON body).
    pub async fn respond(&self, method: &str, target: &str, auth: Option<&str>, body: &[u8]) -> (u16, String) {
        let path = target.split('?').next().unwrap_or(target);
        let extra = self.extra.read().clone();
        if let Some(x) = extra.filter(|x| x.handles(path)) {
            return x.respond(method, target, auth, body).await;
        }
        let known = matches!(
            (method, path),
            ("GET", PATH_HELLO) | ("GET", PATH_STATE) | ("POST", PATH_PLAY) | ("POST", PATH_COMMAND) | ("POST", PATH_STOP)
        );
        if !known {
            return (404, json_err("notFound", "Not found"));
        }
        let key = self.keys.key().await;
        if let Err(r) = self.verifier.verify(key.as_ref(), auth, method, target, &self.own_id, body, now_ms()) {
            tracing::info!(reason = ?r, "refused a control request");
            return refused(r);
        }
        let h = &self.handler;
        let done = |r: AppResult<()>| match r {
            Ok(()) => (200, "{}".to_string()),
            Err(e) => app_err(e),
        };
        match path {
            PATH_HELLO => (200, serde_json::to_string(&h.hello().await).unwrap_or_default()),
            PATH_STATE => (200, serde_json::to_string(&h.timeline().await).unwrap_or_default()),
            PATH_PLAY => match serde_json::from_slice::<PlayRequest>(body) {
                Ok(req) => done(h.play(req).await),
                Err(_) => (400, json_err("invalid", "Bad request")),
            },
            PATH_COMMAND => match serde_json::from_slice::<RemoteCommand>(body) {
                Ok(cmd) => done(h.command(cmd).await),
                Err(_) => (400, json_err("invalid", "Bad request")),
            },
            _ => done(h.stop().await),
        }
    }
}

struct RawRequest {
    method: String,
    target: String,
    auth: Option<String>,
    body: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> Option<RawRequest> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let mut auth = None;
    let mut len = 0usize;
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            if k == AUTH_HEADER {
                auth = Some(v.trim().to_string());
            } else if k == "content-length" {
                len = v.trim().parse().ok()?;
            }
        }
    }
    if len > MAX_BODY {
        return None;
    }
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Some(RawRequest { method, target, auth, body })
}

async fn serve_conn(mut stream: TcpStream, server: Arc<PeerServer>) {
    let Ok(Some(req)) = tokio::time::timeout(Duration::from_secs(5), read_request(&mut stream)).await else {
        return;
    };
    let (code, body) = server.respond(&req.method, &req.target, req.auth.as_deref(), &req.body).await;
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        410 => "Gone",
        _ => "Internal Server Error",
    };
    let resp = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Accept connections on every listener until `stop` turns true.
pub async fn serve(listeners: Vec<TcpListener>, server: Arc<PeerServer>, stop: watch::Receiver<bool>) {
    let mut tasks = Vec::new();
    for listener in listeners {
        let server = server.clone();
        let mut stop = stop.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                tokio::select! {
                    r = listener.accept() => match r {
                        Ok((s, _)) => {
                            tokio::spawn(serve_conn(s, server.clone()));
                        }
                        Err(e) => {
                            tracing::debug!("control accept failed: {e}");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    },
                    r = stop.changed() => {
                        if r.is_err() || *stop.borrow() { return; }
                    }
                }
            }
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
}

/// Listen on the same port on every address in `ips`: the first port from
/// `first` free on all of them.
pub async fn bind(ips: &[Ipv4Addr], first: u16, tries: u16) -> AppResult<(Vec<TcpListener>, u16)> {
    if ips.is_empty() {
        return Err(AppError::Network("This computer isn't on a local network.".into()));
    }
    let mut last = None;
    'port: for p in first..first.saturating_add(tries.max(1)) {
        let mut out = Vec::new();
        let mut port = p;
        for ip in ips {
            match TcpListener::bind(SocketAddr::from((*ip, port))).await {
                Ok(l) => {
                    // Port 0 picks one; the other addresses take the same.
                    port = l.local_addr().map(|a| a.port()).unwrap_or(port);
                    out.push(l);
                }
                Err(e) => {
                    last = Some(e);
                    continue 'port;
                }
            }
        }
        return Ok((out, port));
    }
    Err(last.map(AppError::from).unwrap_or_else(|| AppError::Io("no free port".into())))
}

// ---- the local player as a target -----------------------------------------

/// Answers the control API with the local player.
pub struct PlayerPeer {
    pub local: Arc<dyn LocalPlayer>,
    pub env: Arc<dyn RemoteEnv>,
    pub id: String,
    pub platform: String,
    pub version: String,
    /// The name shown to other devices, as it is now.
    pub name: Arc<dyn Fn() -> String + Send + Sync>,
}

#[async_trait]
impl PeerHandler for PlayerPeer {
    async fn hello(&self) -> Hello {
        let playing = self.local.state().await.ok().and_then(|s| {
            let t = s.track?;
            if matches!(s.status, PlayerStatus::Idle | PlayerStatus::Error) {
                return None;
            }
            let artist = t.track_artist.clone().unwrap_or_else(|| t.artist_title.clone());
            let what = if artist.is_empty() { t.title.clone() } else { format!("{} • {artist}", t.title) };
            Some(if s.status == PlayerStatus::Paused { format!("Paused: {what}") } else { what })
        });
        Hello {
            id: self.id.clone(),
            name: (self.name)(),
            platform: self.platform.clone(),
            version: self.version.clone(),
            playing,
        }
    }

    async fn timeline(&self) -> PeerTimeline {
        let Ok(s) = self.local.state().await else {
            return PeerTimeline {
                state: "stopped".into(),
                time_ms: 0,
                duration_ms: 0,
                track_id: None,
                volume: 0.0,
                shuffle: false,
                repeat: Default::default(),
            };
        };
        let state = match (s.status, &s.track) {
            (_, None) | (PlayerStatus::Idle, _) | (PlayerStatus::Error, _) => "stopped",
            (PlayerStatus::Playing, _) => "playing",
            (PlayerStatus::Loading | PlayerStatus::Buffering, _) => "buffering",
            (PlayerStatus::Paused, _) => "paused",
        };
        PeerTimeline {
            state: state.into(),
            time_ms: s.position_ms,
            duration_ms: s.duration_ms,
            track_id: s.track.as_ref().map(|t| t.id.clone()),
            volume: if s.muted { 0.0 } else { s.volume.clamp(0.0, 1.0) },
            shuffle: s.shuffle,
            repeat: s.repeat,
        }
    }

    async fn play(&self, req: PlayRequest) -> AppResult<()> {
        let server = self.env.server().await?;
        if req.server_id != server.machine_id {
            return Err(AppError::Invalid("That computer plays from a different Plex server.".into()));
        }
        let ok_id = |s: &String| !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric());
        if !ok_id(&req.current) || !req.user_queue.iter().chain(&req.next_up).all(ok_id) {
            return Err(AppError::Invalid("Bad song ids.".into()));
        }
        let mut ids = vec![req.current.clone()];
        ids.extend(req.next_up.iter().take(MAX_SONGS).cloned());
        let user: Vec<String> = req.user_queue.into_iter().take(MAX_SONGS).collect();
        self.local
            .adopt(req.context, ids, user, req.shuffle, req.repeat, req.position_ms, req.play)
            .await
    }

    async fn command(&self, cmd: RemoteCommand) -> AppResult<()> {
        let l = &self.local;
        match cmd {
            RemoteCommand::Play => l.resume().await,
            RemoteCommand::Pause => l.pause().await,
            RemoteCommand::Next => l.next().await,
            RemoteCommand::Previous => l.previous().await,
            RemoteCommand::Seek { position_ms } => l.seek(position_ms).await,
            RemoteCommand::Volume { volume } => {
                let v = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 0.0 };
                l.set_volume(v).await
            }
            RemoteCommand::Shuffle { on } => l.set_shuffle(on).await,
            RemoteCommand::Repeat { mode } => l.set_repeat(mode).await,
        }
    }

    async fn stop(&self) -> AppResult<()> {
        self.local.pause().await
    }
}
