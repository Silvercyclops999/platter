//! The control API's listener and HTTP handling: one request per
//! connection, small JSON bodies, loopback only.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::pairing::{PairError, Pairing, hex};
use super::{
    API_VERSION, BURST, COVER_SIZE, Command, DeckConfig, DeckHost, DeckState, DeckStatus, Listening, PAIR_BURST, PAIR_RATE, RATE,
    READ_TIMEOUT, RateLimit,
};
use crate::error::AppError;
use crate::feed::artist_line;
use crate::feed::server::host_allowed;
use crate::plex::state::EventSink;

const MAX_HEAD: usize = 8 * 1024;
const MAX_BODY: usize = 4 * 1024;

/// One parsed request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn bearer(&self) -> Option<&str> {
        let v = self.header("authorization")?;
        let (scheme, token) = v.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
    }
}

/// One JSON answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: serde_json::Value,
}

impl Reply {
    fn ok(body: serde_json::Value) -> Self {
        Self { status: 200, body }
    }
    fn error(status: u16, message: &str) -> Self {
        Self { status, body: serde_json::json!({ "error": message }) }
    }
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    }
}

struct Inner {
    host: Arc<dyn DeckHost>,
    pairing: Arc<Pairing>,
    automation: Arc<Pairing>,
    events: Arc<dyn EventSink>,
    listening: Mutex<Listening>,
    config: Mutex<DeckConfig>,
    limit: Mutex<RateLimit>,
    pair_limit: Mutex<RateLimit>,
}

/// The control API server. Cheap to clone.
#[derive(Clone)]
pub struct DeckServer {
    inner: Arc<Inner>,
}

/// A short name for a cover, so a client can skip one it has.
pub fn cover_key(thumb: &str) -> String {
    hex(&Sha256::digest(thumb.as_bytes())[..8])
}

impl DeckServer {
    pub fn new(host: Arc<dyn DeckHost>, pairing: Arc<Pairing>, events: Arc<dyn EventSink>) -> Self {
        Self {
            inner: Arc::new(Inner {
                host,
                automation: Arc::new(pairing.sibling(super::pairing::AUTOMATION_ACCOUNT)),
                pairing,
                events,
                listening: Mutex::new(Listening::Off),
                config: Mutex::new(DeckConfig::default()),
                limit: Mutex::new(RateLimit::new(RATE, BURST)),
                pair_limit: Mutex::new(RateLimit::new(PAIR_RATE, PAIR_BURST)),
            }),
        }
    }

    pub fn status(&self) -> DeckStatus {
        let listening = self.inner.listening.lock().clone();
        let on = matches!(listening, Listening::On { .. });
        let c = *self.inner.config.lock();
        DeckStatus {
            code: (on && self.deck_on(&c)).then(|| self.inner.pairing.code()),
            paired: self.inner.pairing.paired(),
            automation_code: (on && c.automation).then(|| self.inner.automation.code()),
            automation_paired: self.inner.automation.paired(),
            listening,
        }
    }

    /// The Stream Deck side is on (it's the only side when neither is set,
    /// as in a server not yet following settings).
    fn deck_on(&self, c: &DeckConfig) -> bool {
        c.enabled || !c.automation
    }

    /// Forget the paired script or command line and show a new code.
    pub fn revoke_automation(&self) -> crate::error::AppResult<DeckStatus> {
        self.inner.automation.revoke()?;
        self.emit_status();
        Ok(self.status())
    }

    /// Run a `platter://` command link (only while automation is on).
    pub async fn run_link(&self, cmd: Command) {
        if !self.inner.config.lock().automation {
            self.inner.host.notice("Turn on Allow automation in Settings to control Platter from links.");
            return;
        }
        let r = self.command(cmd).await;
        if r.status != 200 {
            let m = r.body["error"].as_str().unwrap_or("That didn't work.").to_owned();
            self.inner.host.notice(&m);
        }
    }

    fn emit_status(&self) {
        self.inner.events.emit("deck://status", serde_json::to_value(self.status()).unwrap_or_default());
    }

    fn set_listening(&self, l: Listening) {
        let changed = {
            let mut cur = self.inner.listening.lock();
            let changed = *cur != l;
            *cur = l;
            changed
        };
        if changed {
            self.emit_status();
        }
    }

    /// Forget the paired Stream Deck and show a new code.
    pub fn revoke(&self) -> crate::error::AppResult<DeckStatus> {
        self.inner.pairing.revoke()?;
        self.emit_status();
        Ok(self.status())
    }

    #[cfg(test)]
    pub fn set_config(&self, c: DeckConfig) {
        *self.inner.config.lock() = c;
    }

    #[cfg(test)]
    pub fn automation_code_for_tests(&self) -> String {
        self.inner.automation.code()
    }

    /// `/v1/state`; `have` names the cover the client already shows.
    pub async fn state(&self, have: Option<&str>) -> DeckState {
        let host = &self.inner.host;
        let private = self.inner.config.lock().private_session;
        let mut out = DeckState { api: API_VERSION, private, can_request: host.can_request(), ..Default::default() };
        let Some(now) = host.now().await else { return out };
        out.playing = now.playing;
        out.volume = (now.volume.clamp(0.0, 1.0) * 100.0).round() as u8;
        out.muted = now.muted;
        let Some(t) = now.track else { return out };
        out.liked = host.liked(&t.id).await;
        if private {
            return out;
        }
        out.title = Some(t.title.clone());
        out.artist = Some(artist_line(&t));
        out.album = Some(t.album_title.clone()).filter(|a| !a.is_empty());
        out.track_id = Some(t.id.clone());
        if let Some(thumb) = t.thumb.as_deref() {
            let key = cover_key(thumb);
            if have != Some(key.as_str())
                && have != Some("none")
                && let Ok(a) = host.cover(thumb, COVER_SIZE).await
            {
                out.cover = Some(format!("data:{};base64,{}", a.content_type, base64::engine::general_purpose::STANDARD.encode(&a.bytes)));
            }
            out.cover_key = Some(key);
        }
        out
    }

    async fn command(&self, cmd: Command) -> Reply {
        let host = &self.inner.host;
        let now = host.now().await;
        let track = now.as_ref().and_then(|n| n.track.clone());
        let r = match cmd {
            Command::PlayPause => host.toggle().await,
            Command::Next => host.next().await,
            Command::Previous => host.previous().await,
            Command::Like => match &track {
                Some(t) => {
                    let liked = host.liked(&t.id).await;
                    host.set_liked(&t.id, !liked).await
                }
                None => Err(AppError::Invalid("Nothing is playing.".into())),
            },
            Command::VolumeSet { value } => host.set_volume(percent(value)).await,
            Command::VolumeStep { value } => {
                let cur = now.as_ref().map(|n| if n.muted { 0.0 } else { n.volume }).unwrap_or(0.0) as f64 * 100.0;
                host.set_volume(percent(cur + value)).await
            }
            Command::RequestAlbum => match &track {
                Some(_) if !host.can_request() => Err(AppError::Invalid("Set up Lidarr in Platter's Settings first.".into())),
                Some(t) => host.request_album(t).await,
                None => Err(AppError::Invalid("Nothing is playing.".into())),
            },
            Command::ListenAlong => host.listen_along().await,
            Command::Play => host.resume().await,
            Command::Pause => host.pause().await,
            Command::PlayPlaylist { playlist, shuffle } => match playlist.trim() {
                "" => Err(AppError::Invalid("Name a playlist to play.".into())),
                p => host.play_playlist(p, shuffle).await,
            },
            Command::Search { query } => host.show(serde_json::json!({ "name": "search", "query": query.trim() })).await,
            Command::Open { page } => match super::automation::page_route(&page) {
                Some(route) => host.show(route).await,
                None => Err(AppError::Invalid(format!("Platter has no page called {}.", page.trim()))),
            },
        };
        match r {
            Ok(()) => Reply::ok(serde_json::json!({ "ok": true })),
            Err(e) => Reply::error(409, &message(&e)),
        }
    }

    /// Answer one request from `peer`, at `now`.
    pub async fn respond(&self, req: &Request, peer: IpAddr, port: u16, now: Instant) -> Reply {
        if !peer.is_loopback() {
            return Reply::error(403, "Only this computer can use Platter's control API.");
        }
        if !host_allowed(req.header("host"), port) {
            return Reply::error(403, "Use 127.0.0.1 or localhost.");
        }
        // Web pages always send one of these; the Stream Deck plugin never does.
        if req.header("origin").is_some() || req.header("sec-fetch-site").is_some_and(|v| v != "none") {
            return Reply::error(403, "Web pages can't use Platter's control API.");
        }
        if !self.inner.limit.lock().allow(now) {
            return Reply::error(429, "Too many requests. Slow down.");
        }
        let (path, query) = match req.target.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (req.target.as_str(), None),
        };
        let c = *self.inner.config.lock();
        let authed = req
            .bearer()
            .is_some_and(|t| (self.deck_on(&c) && self.inner.pairing.check(t)) || (c.automation && self.inner.automation.check(t)));
        match (req.method.as_str(), path) {
            ("GET", "/v1/hello") => Reply::ok(serde_json::json!({ "app": "Platter", "api": API_VERSION, "paired": authed })),
            ("POST", "/v1/pair") => {
                if !self.inner.pair_limit.lock().allow(now) {
                    return Reply::error(429, "Too many tries. Wait a moment.");
                }
                let body = serde_json::from_slice::<serde_json::Value>(&req.body).unwrap_or_default();
                let code = body.get("code").and_then(|c| c.as_str()).unwrap_or_default().to_owned();
                let automation = body.get("client").and_then(|c| c.as_str()) == Some("automation");
                let pairing = match automation {
                    true if !c.automation => return Reply::error(403, "Turn on Allow automation in Platter's Settings first."),
                    true => &self.inner.automation,
                    false if !self.deck_on(&c) => return Reply::error(403, "Turn on Allow Stream Deck in Platter's Settings first."),
                    false => &self.inner.pairing,
                };
                let r = pairing.pair(&code);
                self.emit_status();
                match r {
                    Ok(token) => Reply::ok(serde_json::json!({ "token": token })),
                    Err(PairError::WrongCode) => Reply::error(401, "That code doesn't match the one in Platter's Settings."),
                    Err(PairError::Keychain(_)) => Reply::error(500, "Platter couldn't save the pairing in the keychain."),
                }
            }
            (_, "/v1/state" | "/v1/command") if !authed => Reply::error(401, "Pair with Platter first."),
            ("GET", "/v1/state") => {
                let have = query.and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "cover").map(|(_, v)| v.into_owned()));
                Reply::ok(serde_json::to_value(self.state(have.as_deref()).await).unwrap_or_default())
            }
            ("POST", "/v1/command") => match serde_json::from_slice::<Command>(&req.body) {
                Ok(cmd) => self.command(cmd).await,
                Err(_) => Reply::error(400, "Unknown command."),
            },
            (_, "/v1/hello" | "/v1/pair" | "/v1/state" | "/v1/command") => Reply::error(405, "Wrong method."),
            _ => Reply::error(404, "Not found."),
        }
    }

    async fn serve_conn(&self, mut stream: TcpStream, peer: IpAddr, port: u16) {
        let reply = match tokio::time::timeout(READ_TIMEOUT, read_request(&mut stream)).await {
            Ok(Ok(req)) => self.respond(&req, peer, port, Instant::now()).await,
            Ok(Err(status)) => Reply::error(status, "Bad request."),
            Err(_) => return,
        };
        let body = serde_json::to_vec(&reply.body).unwrap_or_default();
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
            reply.status,
            reason(reply.status),
            body.len()
        );
        let _ = stream.write_all(head.as_bytes()).await;
        let _ = stream.write_all(&body).await;
        let _ = stream.shutdown().await;
    }

    /// Bind 127.0.0.1:`port`. Never any other address.
    pub async fn bind(port: u16) -> std::io::Result<TcpListener> {
        TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await
    }

    /// Follow the settings: open or close the port as they change.
    pub async fn follow(self, mut cfg: watch::Receiver<DeckConfig>, port: u16) {
        loop {
            let c = *cfg.borrow_and_update();
            *self.inner.config.lock() = c;
            if !c.listening() {
                self.set_listening(Listening::Off);
                if cfg.changed().await.is_err() {
                    return;
                }
                continue;
            }
            let listener = match Self::bind(port).await {
                Ok(l) => l,
                Err(e) => {
                    let l = if e.kind() == std::io::ErrorKind::AddrInUse || e.kind() == std::io::ErrorKind::PermissionDenied {
                        Listening::PortInUse { port }
                    } else {
                        Listening::Failed { port, message: e.to_string() }
                    };
                    tracing::info!(port, "stream deck control not started: {e}");
                    self.set_listening(l);
                    if cfg.changed().await.is_err() {
                        return;
                    }
                    continue;
                }
            };
            let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
            self.set_listening(Listening::On { port });
            loop {
                tokio::select! {
                    r = listener.accept() => match r {
                        Ok((s, addr)) => {
                            let me = self.clone();
                            tokio::spawn(async move { me.serve_conn(s, addr.ip(), port).await });
                        }
                        Err(e) => {
                            tracing::debug!("stream deck accept failed: {e}");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    },
                    r = cfg.changed() => {
                        if r.is_err() {
                            return;
                        }
                        let next = *cfg.borrow();
                        if next.listening() == c.listening() {
                            *self.inner.config.lock() = next;
                            // The codes shown follow which sides are on.
                            self.emit_status();
                            continue;
                        }
                        break;
                    }
                }
            }
            drop(listener);
        }
    }
}

/// 0..100 percent to the player's 0..1.
fn percent(v: f64) -> f32 {
    if v.is_finite() { (v / 100.0).clamp(0.0, 1.0) as f32 } else { 0.0 }
}

/// The message a key shows (or logs) for a failed command.
fn message(e: &AppError) -> String {
    match e {
        AppError::Invalid(m) | AppError::NotFound(m) => m.clone(),
        AppError::Network(_) | AppError::Server(_) => "Platter couldn't reach the server.".into(),
        _ => "That didn't work.".into(),
    }
}

/// Read one request: head, then a body up to [`MAX_BODY`]. Err is the status to answer.
async fn read_request(stream: &mut TcpStream) -> Result<Request, u16> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let end = loop {
        let n = stream.read(&mut chunk).await.map_err(|_| 400u16)?;
        if n == 0 {
            return Err(400);
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return Err(413);
        }
    };
    let mut req = parse_head(&String::from_utf8_lossy(&buf[..end])).ok_or(400u16)?;
    let len: usize = req.header("content-length").map(|v| v.trim().parse().map_err(|_| 400u16)).transpose()?.unwrap_or(0);
    if len > MAX_BODY {
        return Err(413);
    }
    let mut body = buf[end..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).await.map_err(|_| 400u16)?;
        if n == 0 {
            return Err(400);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    req.body = body;
    Ok(req)
}

/// The request line and headers.
pub fn parse_head(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    if !target.starts_with('/') {
        return None;
    }
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
    Some(Request { method, target, headers, body: vec![] })
}
