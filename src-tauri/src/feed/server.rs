//! The feed's listener and HTTP handling: GET only, one request per
//! connection, loopback only.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::page::{self, PageOptions};
use super::{FeedConfig, FeedJson, FeedSource, FeedStatus, artist_line};
use crate::plex::state::EventSink;

const MAX_HEAD: usize = 8 * 1024;
/// Cover size served (square, in pixels).
pub const ART_SIZE: u32 = 300;
/// Largest cover size a request may ask for.
pub const ART_SIZE_MAX: u32 = 1200;

/// One HTTP answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// Extra headers (the overlay page's content security policy).
    pub headers: Vec<(&'static str, String)>,
}

impl Reply {
    fn text(status: u16, body: &str) -> Self {
        Self { status, content_type: "text/plain; charset=utf-8", body: body.as_bytes().to_vec(), headers: vec![] }
    }
}

struct Inner {
    source: Arc<dyn FeedSource>,
    events: Arc<dyn EventSink>,
    status: Mutex<FeedStatus>,
    config: Mutex<FeedConfig>,
}

/// The feed server. Cheap to clone.
#[derive(Clone)]
pub struct FeedServer {
    inner: Arc<Inner>,
}

/// Split a request target into its path and query pairs.
fn split_target(target: &str) -> (String, Vec<(String, String)>) {
    let Ok(u) = url::Url::parse(&format!("http://feed{target}")) else { return (target.to_owned(), vec![]) };
    (u.path().to_owned(), u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
}

/// Whether the Host header names this server on loopback.
pub fn host_allowed(host: Option<&str>, port: u16) -> bool {
    let Some(h) = host.map(|h| h.trim().to_ascii_lowercase()) else { return false };
    h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}")
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    }
}

impl FeedServer {
    pub fn new(source: Arc<dyn FeedSource>, events: Arc<dyn EventSink>) -> Self {
        Self {
            inner: Arc::new(Inner {
                source,
                events,
                status: Mutex::new(FeedStatus::Off),
                config: Mutex::new(FeedConfig::default()),
            }),
        }
    }

    pub fn status(&self) -> FeedStatus {
        self.inner.status.lock().clone()
    }

    fn set_status(&self, s: FeedStatus) {
        let changed = {
            let mut cur = self.inner.status.lock();
            let changed = *cur != s;
            *cur = s.clone();
            changed
        };
        if changed {
            self.inner.events.emit("feed://status", serde_json::to_value(&s).unwrap_or_default());
        }
    }

    fn private(&self) -> bool {
        self.inner.config.lock().private_session
    }

    /// The JSON body for `/now-playing.json`.
    pub async fn json(&self, port: u16, data_art: bool) -> FeedJson {
        if self.private() {
            return FeedJson { private: true, ..Default::default() };
        }
        let Some(now) = self.inner.source.now().await else { return FeedJson::default() };
        let t = &now.track;
        let line = artist_line(t);
        let artists = crate::db::credits::split_credits(&line, |_| false).into_iter().map(str::to_owned).collect();
        let art = match t.thumb.as_deref() {
            None => None,
            Some(path) if data_art => self.inner.source.art(path, ART_SIZE).await.ok().map(|a| {
                format!("data:{};base64,{}", a.content_type, base64::engine::general_purpose::STANDARD.encode(&a.bytes))
            }),
            Some(_) => Some(format!("http://127.0.0.1:{port}/art?t={}", urlencode(&t.id))),
        };
        FeedJson {
            playing: now.playing,
            private: false,
            title: Some(t.title.clone()),
            artist: Some(line),
            artists,
            album: Some(t.album_title.clone()).filter(|a| !a.is_empty()),
            art,
            position_ms: now.position_ms.min(now.duration_ms.max(now.position_ms)),
            duration_ms: now.duration_ms,
            track_id: Some(t.id.clone()),
        }
    }

    /// Answer one request.
    pub async fn respond(&self, method: &str, target: &str, host: Option<&str>, port: u16) -> Reply {
        if !host_allowed(host, port) {
            return Reply::text(403, "Only this computer can use the feed, at 127.0.0.1.");
        }
        if method != "GET" && method != "HEAD" {
            return Reply::text(405, "GET only");
        }
        let (path, query) = split_target(target);
        let q = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        match path.as_str() {
            "/now-playing.json" => {
                let body = serde_json::to_vec(&self.json(port, q("art") == Some("data")).await).unwrap_or_default();
                Reply { status: 200, content_type: "application/json", body, headers: vec![] }
            }
            "/now-playing.html" => {
                let opts = PageOptions::from_query(&query);
                Reply {
                    status: 200,
                    content_type: "text/html; charset=utf-8",
                    body: page::render(&opts).into_bytes(),
                    headers: vec![("Content-Security-Policy", page::CSP.to_owned())],
                }
            }
            "/art" => self.art(q("t"), q("s")).await,
            _ => Reply::text(404, "Not found. Try /now-playing.json or /now-playing.html."),
        }
    }

    /// The playing song's cover, only when `t` is the playing song.
    async fn art(&self, track: Option<&str>, size: Option<&str>) -> Reply {
        if self.private() {
            return Reply::text(404, "Not found");
        }
        let Some(now) = self.inner.source.now().await else { return Reply::text(404, "Not found") };
        let (Some(path), Some(t)) = (now.track.thumb.as_deref(), track) else { return Reply::text(404, "Not found") };
        if t != now.track.id {
            return Reply::text(404, "Not found");
        }
        let size = size.and_then(|s| s.parse::<u32>().ok()).unwrap_or(ART_SIZE).clamp(64, ART_SIZE_MAX);
        match self.inner.source.art(path, size).await {
            Ok(a) => Reply {
                status: 200,
                content_type: match a.content_type.as_str() {
                    "image/png" => "image/png",
                    "image/webp" => "image/webp",
                    _ => "image/jpeg",
                },
                body: a.bytes.to_vec(),
                headers: vec![],
            },
            Err(_) => Reply::text(404, "Not found"),
        }
    }

    async fn serve_conn(&self, mut stream: TcpStream, port: u16) {
        let Ok(Some((method, target, host))) = tokio::time::timeout(Duration::from_secs(5), read_head(&mut stream)).await else {
            return;
        };
        let reply = self.respond(&method, &target, host.as_deref(), port).await;
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n",
            reply.status,
            reason(reply.status),
            reply.content_type,
            reply.body.len()
        );
        for (k, v) in &reply.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes()).await;
        if method != "HEAD" {
            let _ = stream.write_all(&reply.body).await;
        }
        let _ = stream.shutdown().await;
    }

    /// Bind 127.0.0.1:`port`. Never any other address.
    pub async fn bind(port: u16) -> std::io::Result<TcpListener> {
        TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await
    }

    /// Follow the settings: open, move or close the port as they change.
    pub async fn follow(self, mut cfg: watch::Receiver<FeedConfig>) {
        loop {
            let c = *cfg.borrow_and_update();
            *self.inner.config.lock() = c;
            if !c.enabled {
                self.set_status(FeedStatus::Off);
                if cfg.changed().await.is_err() {
                    return;
                }
                continue;
            }
            let listener = match Self::bind(c.port).await {
                Ok(l) => l,
                Err(e) => {
                    let status = if e.kind() == std::io::ErrorKind::AddrInUse || e.kind() == std::io::ErrorKind::PermissionDenied {
                        FeedStatus::PortInUse { port: c.port }
                    } else {
                        FeedStatus::Failed { port: c.port, message: e.to_string() }
                    };
                    tracing::info!(port = c.port, "now-playing feed not started: {e}");
                    self.set_status(status);
                    if cfg.changed().await.is_err() {
                        return;
                    }
                    continue;
                }
            };
            let port = listener.local_addr().map(|a| a.port()).unwrap_or(c.port);
            self.set_status(FeedStatus::On { port, url: format!("http://127.0.0.1:{port}/now-playing.json") });
            loop {
                tokio::select! {
                    r = listener.accept() => match r {
                        Ok((s, _)) => {
                            let me = self.clone();
                            tokio::spawn(async move { me.serve_conn(s, port).await });
                        }
                        Err(e) => {
                            tracing::debug!("feed accept failed: {e}");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    },
                    r = cfg.changed() => {
                        if r.is_err() {
                            return;
                        }
                        let next = *cfg.borrow();
                        // A private session only changes what's served.
                        if next.enabled == c.enabled && next.port == c.port {
                            *self.inner.config.lock() = next;
                            continue;
                        }
                        break;
                    }
                }
            }
            // Dropping the listener closes the port before the next round.
            drop(listener);
        }
    }
}

/// Read the request line and the Host header.
async fn read_head(stream: &mut TcpStream) -> Option<(String, String, Option<String>)> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
    }
    parse_head(&String::from_utf8_lossy(&buf))
}

/// (method, target, Host) from a request head.
pub fn parse_head(head: &str) -> Option<(String, String, Option<String>)> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    if !target.starts_with('/') {
        return None;
    }
    let host = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.trim().to_owned());
    Some((method, target, host))
}

fn urlencode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}
