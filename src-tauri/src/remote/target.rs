//! Platter as a Plex player, so other Plex apps can play on this computer.
//! Off unless the setting is on, because it opens a local port.
//!
//! - HTTP companion endpoints on the first free port from 32500:
//!   `/resources`, `/player/timeline/poll`, `/player/timeline/subscribe`
//!   (accepted; controllers fall back to polling), and
//!   `/player/playback/{playMedia,play,pause,stop,skipNext,skipPrevious,seekTo,setParameters}`.
//! - GDM: answers `M-SEARCH` on UDP 32412 and announces `HELLO` / `BYE` to
//!   239.0.0.250:32413, so apps on the network list it.
//!
//! `playMedia` only plays from the server Platter is signed in to; the token
//! in its query is ignored (Platter uses its own connection).

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::watch;

use super::playqueue;
use super::service::{LocalPlayer, RemoteEnv};
use super::types::{repeat_code, repeat_from_code};
use super::xml::escape;
use crate::audio::events::PlayerStatus;
use crate::error::{AppError, AppResult};

pub const FIRST_PORT: u16 = 32500;
pub const PORT_TRIES: u16 = 10;
pub const GDM_PORT: u16 = 32412;
pub const GDM_HELLO_PORT: u16 = 32413;
pub const GDM_GROUP: Ipv4Addr = Ipv4Addr::new(239, 0, 0, 250);
const CAPABILITIES: &str = "timeline,playback,playqueues";
const MAX_HEAD: usize = 16 * 1024;

/// How this computer presents itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetInfo {
    pub client_id: String,
    pub name: String,
    pub product: String,
    pub version: String,
    pub platform: String,
    pub platform_version: String,
}

/// The music timeline Platter reports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TargetTimeline {
    pub state: &'static str,
    pub time_ms: u64,
    pub duration_ms: u64,
    pub rating_key: Option<String>,
    /// 0..100.
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: u8,
    pub server_machine_id: Option<String>,
}

#[async_trait]
pub trait TargetHandler: Send + Sync {
    async fn timeline(&self) -> TargetTimeline;
    /// A `/player/...` command (path without the `/player` prefix).
    async fn command(&self, path: &str, params: &HashMap<String, String>) -> AppResult<()>;
}

pub fn resources_xml(info: &TargetInfo) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MediaContainer size=\"1\"><Player title=\"{}\" protocol=\"plex\" protocolVersion=\"1\" protocolCapabilities=\"{CAPABILITIES}\" machineIdentifier=\"{}\" product=\"{}\" platform=\"{}\" platformVersion=\"{}\" deviceClass=\"pc\"/></MediaContainer>",
        escape(&info.name),
        escape(&info.client_id),
        escape(&info.product),
        escape(&info.platform),
        escape(&info.platform_version),
    )
}

pub fn timeline_xml(t: &TargetTimeline, command_id: Option<&str>) -> String {
    let mut music = format!(
        "<Timeline type=\"music\" state=\"{}\" time=\"{}\" duration=\"{}\" volume=\"{}\" shuffle=\"{}\" repeat=\"{}\" controllable=\"playPause,stop,skipPrevious,skipNext,seekTo,volume,shuffle,repeat\"",
        t.state,
        t.time_ms,
        t.duration_ms,
        t.volume,
        u8::from(t.shuffle),
        t.repeat
    );
    if let Some(k) = &t.rating_key {
        music.push_str(&format!(" ratingKey=\"{0}\" key=\"/library/metadata/{0}\"", escape(k)));
    }
    if let Some(m) = &t.server_machine_id {
        music.push_str(&format!(" machineIdentifier=\"{}\" providerIdentifier=\"{}\"", escape(m), playqueue::LIBRARY));
    }
    music.push_str("/>");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MediaContainer commandID=\"{}\" location=\"{}\">{music}<Timeline type=\"video\" state=\"stopped\"/><Timeline type=\"photo\" state=\"stopped\"/></MediaContainer>",
        escape(command_id.unwrap_or("0")),
        if t.state == "stopped" { "navigation" } else { "fullScreenMusic" },
    )
}

fn ok_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Response code=\"200\" status=\"OK\"/>".into()
}

fn err_xml(code: u16, status: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Response code=\"{code}\" status=\"{}\"/>", escape(status))
}

fn query_map(q: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()
}

/// Answer one request: (status, body).
pub async fn respond(handler: &dyn TargetHandler, info: &TargetInfo, method: &str, target: &str) -> (u16, String) {
    if method == "OPTIONS" {
        return (200, String::new());
    }
    if method != "GET" && method != "POST" && method != "PUT" {
        return (405, err_xml(405, "Method not allowed"));
    }
    let (path, q) = target.split_once('?').unwrap_or((target, ""));
    let params = query_map(q);
    match path.trim_end_matches('/') {
        "/resources" => (200, resources_xml(info)),
        "/player/timeline/poll" => {
            let t = handler.timeline().await;
            (200, timeline_xml(&t, params.get("commandID").map(String::as_str)))
        }
        p if p.starts_with("/player/") => match handler.command(&p["/player".len()..], &params).await {
            Ok(()) => (200, ok_xml()),
            Err(AppError::NotFound(_)) => (404, err_xml(404, "Not found")),
            Err(e) => (400, err_xml(400, &e.to_string())),
        },
        _ => (404, err_xml(404, "Not found")),
    }
}

async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Some(String::from_utf8_lossy(&buf).into_owned());
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
    }
}

async fn serve_conn(mut stream: TcpStream, handler: Arc<dyn TargetHandler>, info: Arc<TargetInfo>) {
    let Ok(Some(head)) = tokio::time::timeout(Duration::from_secs(5), read_head(&mut stream)).await else {
        return;
    };
    let mut parts = head.lines().next().unwrap_or("").split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let (code, body) = respond(handler.as_ref(), &info, method, target).await;
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    let resp = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: text/xml;charset=utf-8\r\nContent-Length: {}\r\nX-Plex-Client-Identifier: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, PUT, OPTIONS\r\nAccess-Control-Allow-Headers: *\r\nAccess-Control-Expose-Headers: X-Plex-Client-Identifier\r\nConnection: close\r\n\r\n{body}",
        body.len(),
        info.client_id,
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Accept connections until `stop` turns true.
pub async fn serve(listener: TcpListener, handler: Arc<dyn TargetHandler>, info: Arc<TargetInfo>, mut stop: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            r = listener.accept() => match r {
                Ok((s, _)) => {
                    tokio::spawn(serve_conn(s, handler.clone(), info.clone()));
                }
                Err(e) => {
                    tracing::debug!("companion accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            },
            _ = stop.changed() => {
                if *stop.borrow() { return; }
            }
        }
        if *stop.borrow() {
            return;
        }
    }
}

/// Bind the first free port from `first` on `ip`.
pub async fn bind(ip: Ipv4Addr, first: u16, tries: u16) -> AppResult<TcpListener> {
    let mut last = None;
    for p in first..first.saturating_add(tries) {
        match TcpListener::bind(SocketAddr::from((ip, p))).await {
            Ok(l) => return Ok(l),
            Err(e) => last = Some(e),
        }
    }
    Err(last.map(AppError::from).unwrap_or_else(|| AppError::Io("no free port".into())))
}

// ---- GDM ------------------------------------------------------------------

fn gdm_fields(info: &TargetInfo, port: u16) -> String {
    format!(
        "Content-Type: plex/media-player\r\nResource-Identifier: {}\r\nName: {}\r\nPort: {port}\r\nProduct: {}\r\nVersion: {}\r\nProtocol: plex\r\nProtocol-Version: 1\r\nProtocol-Capabilities: {CAPABILITIES}\r\nDevice-Class: pc\r\n",
        info.client_id, info.name, info.product, info.version
    )
}

/// The answer to a GDM `M-SEARCH`.
pub fn gdm_reply(info: &TargetInfo, port: u16) -> String {
    format!("HTTP/1.0 200 OK\r\n{}\r\n", gdm_fields(info, port))
}

/// The announcement when the player starts.
pub fn gdm_hello(info: &TargetInfo, port: u16) -> String {
    format!("HELLO * HTTP/1.0\r\n{}\r\n", gdm_fields(info, port))
}

/// The goodbye when it stops.
pub fn gdm_bye(info: &TargetInfo, port: u16) -> String {
    format!("BYE * HTTP/1.0\r\n{}\r\n", gdm_fields(info, port))
}

pub fn is_search(datagram: &[u8]) -> bool {
    datagram.starts_with(b"M-SEARCH * HTTP/1.")
}

/// Announce, answer searches until `stop`, then say goodbye.
pub async fn gdm(info: Arc<TargetInfo>, port: u16, mut stop: watch::Receiver<bool>) {
    let group = SocketAddr::from((GDM_GROUP, GDM_HELLO_PORT));
    let announce = UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))).await.ok();
    if let Some(s) = &announce {
        let _ = s.send_to(gdm_hello(&info, port).as_bytes(), group).await;
    }
    let listen = match UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, GDM_PORT))).await {
        Ok(s) => {
            if let Err(e) = s.join_multicast_v4(GDM_GROUP, Ipv4Addr::UNSPECIFIED) {
                tracing::debug!("GDM multicast join failed: {e}");
            }
            Some(s)
        }
        Err(e) => {
            // Another Plex player on this computer has the port; the hello
            // and the server's registration still list Platter.
            tracing::info!("GDM search port busy: {e}");
            None
        }
    };
    let mut buf = [0u8; 2048];
    loop {
        let recv = async {
            match &listen {
                Some(s) => s.recv_from(&mut buf).await.ok(),
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            r = recv => {
                if let (Some((n, from)), Some(s)) = (r, &listen)
                    && is_search(&buf[..n]) {
                        let _ = s.send_to(gdm_reply(&info, port).as_bytes(), from).await;
                    }
            }
            _ = stop.changed() => {
                if *stop.borrow() { break; }
            }
        }
    }
    if let Some(s) = &announce {
        let _ = s.send_to(gdm_bye(&info, port).as_bytes(), group).await;
    }
}

// ---- the local player as a target -----------------------------------------

/// Answers companion commands with the local player.
pub struct PlayerTarget {
    pub local: Arc<dyn LocalPlayer>,
    pub env: Arc<dyn RemoteEnv>,
    pub http: reqwest::Client,
}

fn num(params: &HashMap<String, String>, k: &str) -> Option<i64> {
    params.get(k).and_then(|v| v.trim().parse::<f64>().ok()).map(|f| f as i64)
}

#[async_trait]
impl TargetHandler for PlayerTarget {
    async fn timeline(&self) -> TargetTimeline {
        let machine = self.env.server().await.ok().map(|s| s.machine_id);
        let Ok(s) = self.local.state().await else {
            return TargetTimeline { state: "stopped", ..Default::default() };
        };
        let state = match (s.status, &s.track) {
            (_, None) | (PlayerStatus::Idle, _) | (PlayerStatus::Error, _) => "stopped",
            (PlayerStatus::Playing, _) => "playing",
            (PlayerStatus::Loading | PlayerStatus::Buffering, _) => "buffering",
            (PlayerStatus::Paused, _) => "paused",
        };
        TargetTimeline {
            state,
            time_ms: s.position_ms,
            duration_ms: s.duration_ms,
            rating_key: s.track.as_ref().map(|t| t.id.clone()),
            volume: if s.muted { 0 } else { (s.volume.clamp(0.0, 1.0) * 100.0).round() as u8 },
            shuffle: s.shuffle,
            repeat: repeat_code(s.repeat),
            server_machine_id: machine,
        }
    }

    async fn command(&self, path: &str, params: &HashMap<String, String>) -> AppResult<()> {
        let l = &self.local;
        match path {
            "/playback/playMedia" => {
                let server = self.env.server().await?;
                if let Some(m) = params.get("machineIdentifier")
                    && !m.is_empty()
                    && *m != server.machine_id
                {
                    return Err(AppError::Invalid("Platter plays from its own server only.".into()));
                }
                let key = params
                    .get("key")
                    .and_then(|k| k.rsplit('/').next())
                    .filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric()))
                    .map(str::to_string)
                    .ok_or_else(|| AppError::Invalid("missing key".into()))?;
                let offset = num(params, "offset").unwrap_or(0).max(0) as u64;
                let mut ids = match params.get("containerKey").and_then(|c| playqueue::id_from_container_key(c)) {
                    Some(pq) => playqueue::items(&self.http, &server, &pq).await.unwrap_or_default(),
                    None => Vec::new(),
                };
                if !ids.contains(&key) {
                    ids = vec![key.clone()];
                }
                l.play_tracks(ids, Some(key), offset, true).await
            }
            "/playback/play" => l.resume().await,
            "/playback/pause" | "/playback/stop" => l.pause().await,
            "/playback/skipNext" => l.next().await,
            "/playback/skipPrevious" => l.previous().await,
            "/playback/seekTo" => l.seek(num(params, "offset").unwrap_or(0).max(0) as u64).await,
            "/playback/setParameters" => {
                if let Some(v) = num(params, "volume") {
                    l.set_volume(v.clamp(0, 100) as f32 / 100.0).await?;
                }
                if let Some(s) = num(params, "shuffle") {
                    l.set_shuffle(s != 0).await?;
                }
                if let Some(r) = num(params, "repeat") {
                    l.set_repeat(repeat_from_code(r)).await?;
                }
                Ok(())
            }
            "/timeline/subscribe" | "/timeline/unsubscribe" => Ok(()),
            _ => Err(AppError::NotFound(path.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> TargetInfo {
        TargetInfo {
            client_id: "me".into(),
            name: "Desk & Co".into(),
            product: "Platter".into(),
            version: "0.1.0".into(),
            platform: "Windows".into(),
            platform_version: "x86_64".into(),
        }
    }

    #[test]
    fn gdm_messages() {
        let r = gdm_reply(&info(), 32500);
        assert!(r.starts_with("HTTP/1.0 200 OK\r\n"));
        assert!(r.contains("Resource-Identifier: me\r\n"));
        assert!(r.contains("Port: 32500\r\n"));
        assert!(r.contains("Content-Type: plex/media-player\r\n"));
        assert!(r.ends_with("\r\n\r\n"));
        assert!(gdm_hello(&info(), 1).starts_with("HELLO * HTTP/1.0\r\n"));
        assert!(gdm_bye(&info(), 1).starts_with("BYE * HTTP/1.0\r\n"));
        assert!(is_search(b"M-SEARCH * HTTP/1.1\r\n\r\n"));
        assert!(!is_search(b"HELLO * HTTP/1.0\r\n"));
    }

    #[test]
    fn xml_answers_parse_back() {
        let r = crate::remote::xml::elements(&resources_xml(&info()));
        assert_eq!(r[1].get("title"), Some("Desk & Co"));
        assert_eq!(r[1].get("machineIdentifier"), Some("me"));
        let t = TargetTimeline {
            state: "playing",
            time_ms: 1000,
            duration_ms: 2000,
            rating_key: Some("7".into()),
            volume: 50,
            shuffle: true,
            repeat: 2,
            server_machine_id: Some("srv".into()),
        };
        let parsed = crate::remote::companion::parse_timeline(&timeline_xml(&t, Some("5"))).unwrap();
        assert_eq!(parsed.state, "playing");
        assert_eq!(parsed.rating_key.as_deref(), Some("7"));
        assert_eq!((parsed.volume, parsed.shuffle, parsed.repeat), (Some(50), true, 2));
    }
}
