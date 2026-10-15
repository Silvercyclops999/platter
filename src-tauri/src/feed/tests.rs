use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::page::{PageOptions, Size, Theme};
use super::server::{FeedServer, host_allowed, parse_head};
use super::*;
use crate::audio::testutil::track;
use crate::error::AppError;

#[derive(Default)]
struct FakeSource {
    now: Mutex<Option<Now>>,
    art_calls: Mutex<Vec<(String, u32)>>,
}

#[async_trait]
impl FeedSource for FakeSource {
    async fn now(&self) -> Option<Now> {
        self.now.lock().clone()
    }
    async fn art(&self, path: &str, size: u32) -> AppResult<ArtBytes> {
        self.art_calls.lock().push((path.to_owned(), size));
        if path == "/missing" {
            return Err(AppError::NotFound("art".into()));
        }
        Ok(ArtBytes { bytes: Bytes::from_static(b"\xff\xd8\xffJPEG"), content_type: "image/jpeg".into() })
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<(String, serde_json::Value)>>);
impl crate::plex::state::EventSink for Events {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().push((event.to_owned(), payload));
    }
}

fn playing() -> Now {
    let mut t = track("t1", "Rue Ellis", "/p");
    t.title = "Night Drive".into();
    t.album_title = "Late".into();
    t.track_artist = Some("Rue Ellis feat. The Moth".into());
    t.thumb = Some("/library/metadata/9/thumb/1".into());
    t.duration_ms = 200_000;
    Now { track: t, playing: true, position_ms: 61_000, duration_ms: 200_000 }
}

fn server() -> (FeedServer, Arc<FakeSource>, Arc<Events>) {
    let src = Arc::new(FakeSource::default());
    *src.now.lock() = Some(playing());
    let ev = Arc::new(Events::default());
    (FeedServer::new(src.clone(), ev.clone()), src, ev)
}

/// A free loopback port (bound and let go).
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn get(addr: SocketAddr, path: &str, host: &str) -> std::io::Result<(u16, String, Vec<u8>)> {
    let mut s = TcpStream::connect(addr).await?;
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes()).await?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await?;
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    Ok((status, head, buf[split + 4..].to_vec()))
}

/// Start following a switch and wait until the status leaves Off/settles.
async fn wait_status(s: &FeedServer, want: impl Fn(&FeedStatus) -> bool) -> FeedStatus {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        let st = s.status();
        if want(&st) {
            return st;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("status never arrived; last {:?}", s.status());
}

#[tokio::test]
async fn json_has_the_song_and_a_local_art_url() {
    let (s, _, _) = server();
    let j = s.json(32540, false).await;
    assert!(j.playing && !j.private);
    assert_eq!(j.title.as_deref(), Some("Night Drive"));
    assert_eq!(j.artist.as_deref(), Some("Rue Ellis feat. The Moth"));
    assert_eq!(j.artists, ["Rue Ellis", "The Moth"]);
    assert_eq!(j.album.as_deref(), Some("Late"));
    assert_eq!(j.art.as_deref(), Some("http://127.0.0.1:32540/art?t=t1"));
    assert_eq!((j.position_ms, j.duration_ms), (61_000, 200_000));
    let v = serde_json::to_value(&j).unwrap();
    for k in ["playing", "title", "artists", "album", "art", "positionMs", "durationMs"] {
        assert!(v.get(k).is_some(), "{k} missing");
    }
}

#[tokio::test]
async fn art_can_come_as_a_data_url() {
    let (s, src, _) = server();
    let j = s.json(32540, true).await;
    assert!(j.art.unwrap().starts_with("data:image/jpeg;base64,"));
    assert_eq!(src.art_calls.lock()[0], ("/library/metadata/9/thumb/1".to_string(), server::ART_SIZE));
}

#[tokio::test]
async fn nothing_playing_and_a_private_session_are_blank() {
    let (s, src, _) = server();
    *src.now.lock() = None;
    assert_eq!(s.json(1, false).await, FeedJson::default());
    *src.now.lock() = Some(playing());
    let switch = FeedSwitch::default();
    let port = free_port();
    switch.set(FeedConfig { enabled: true, port, private_session: true });
    tokio::spawn(s.clone().follow(switch.subscribe()));
    wait_status(&s, |st| matches!(st, FeedStatus::On { .. })).await;
    let j = s.json(port, false).await;
    assert!(j.private && j.title.is_none() && j.art.is_none() && !j.playing);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let host = format!("127.0.0.1:{port}");
    let (code, _, body) = get(addr, "/now-playing.json", &host).await.unwrap();
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["private"], true);
    assert_eq!(v["title"], serde_json::Value::Null);
    assert_eq!(get(addr, "/art?t=t1", &host).await.unwrap().0, 404, "no cover either");
    assert!(src.art_calls.lock().is_empty());
    // Ending the private session shows the song again, on the same port.
    switch.set(FeedConfig { enabled: true, port, private_session: false });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (_, _, body) = get(addr, "/now-playing.json", &host).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["title"], "Night Drive");
}

#[tokio::test]
async fn binds_loopback_only_and_refuses_other_interfaces() {
    let (s, _, _) = server();
    let switch = FeedSwitch::default();
    let port = free_port();
    switch.set(FeedConfig { enabled: true, port, private_session: false });
    tokio::spawn(s.clone().follow(switch.subscribe()));
    let st = wait_status(&s, |st| matches!(st, FeedStatus::On { .. })).await;
    assert_eq!(st, FeedStatus::On { port, url: format!("http://127.0.0.1:{port}/now-playing.json") });
    // Loopback answers.
    let (code, head, _) = get(SocketAddr::from(([127, 0, 0, 1], port)), "/now-playing.json", &format!("127.0.0.1:{port}")).await.unwrap();
    assert_eq!(code, 200);
    assert!(head.to_ascii_lowercase().contains("content-type: application/json"));
    assert!(!head.to_ascii_lowercase().contains("access-control-allow-origin"), "no CORS: other sites can't read it");
    // Every other address of this computer refuses the connection.
    let others: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|i| i.ip())
        .filter(|ip| !ip.is_loopback() && ip.is_ipv4())
        .collect();
    for ip in others {
        let r = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(SocketAddr::new(ip, port))).await;
        assert!(!matches!(r, Ok(Ok(_))), "the feed answered on {ip}");
    }
}

#[tokio::test]
async fn bind_never_takes_a_non_loopback_address() {
    let l = FeedServer::bind(0).await.unwrap();
    assert!(l.local_addr().unwrap().ip().is_loopback());
}

#[tokio::test]
async fn turning_it_off_closes_the_port_and_a_new_port_moves_it() {
    let (s, _, ev) = server();
    let switch = FeedSwitch::default();
    let port = free_port();
    switch.set(FeedConfig { enabled: true, port, private_session: false });
    tokio::spawn(s.clone().follow(switch.subscribe()));
    wait_status(&s, |st| matches!(st, FeedStatus::On { .. })).await;
    switch.set(FeedConfig { enabled: false, port, private_session: false });
    wait_status(&s, |st| *st == FeedStatus::Off).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err(), "the port is closed");

    let other = free_port();
    switch.set(FeedConfig { enabled: true, port: other, private_session: false });
    wait_status(&s, |st| matches!(st, FeedStatus::On { port: p, .. } if *p == other)).await;
    assert!(get(SocketAddr::from(([127, 0, 0, 1], other)), "/now-playing.json", &format!("localhost:{other}")).await.is_ok());
    let names: Vec<_> = ev.0.lock().iter().map(|(n, v)| (n.clone(), v["state"].as_str().unwrap_or_default().to_owned())).collect();
    assert!(names.iter().all(|(n, _)| n == "feed://status"));
    assert_eq!(names.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), ["on", "off", "on"]);
}

#[tokio::test]
async fn a_port_in_use_says_so() {
    let (s, _, _) = server();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let switch = FeedSwitch::default();
    switch.set(FeedConfig { enabled: true, port, private_session: false });
    tokio::spawn(s.clone().follow(switch.subscribe()));
    let st = wait_status(&s, |st| !matches!(st, FeedStatus::Off)).await;
    assert_eq!(st, FeedStatus::PortInUse { port });
    // Freeing the port and asking again works.
    drop(taken);
    let next = free_port();
    switch.set(FeedConfig { enabled: true, port: next, private_session: false });
    wait_status(&s, |st| matches!(st, FeedStatus::On { .. })).await;
}

#[tokio::test]
async fn requests_for_other_hosts_methods_and_paths_are_refused() {
    let (s, _, _) = server();
    assert_eq!(s.respond("GET", "/now-playing.json", Some("evil.example:32540"), 32540).await.status, 403);
    assert_eq!(s.respond("GET", "/now-playing.json", None, 32540).await.status, 403);
    assert_eq!(s.respond("GET", "/now-playing.json", Some("127.0.0.1:1"), 32540).await.status, 403);
    assert_eq!(s.respond("POST", "/now-playing.json", Some("127.0.0.1:32540"), 32540).await.status, 405);
    assert_eq!(s.respond("GET", "/etc/passwd", Some("127.0.0.1:32540"), 32540).await.status, 404);
    assert!(host_allowed(Some("LOCALHOST:32540"), 32540));
    assert!(!host_allowed(Some("192.0.2.1:32540"), 32540));
}

#[tokio::test]
async fn art_is_only_the_playing_songs() {
    let (s, src, _) = server();
    let h = Some("127.0.0.1:9");
    let r = s.respond("GET", "/art?t=t1", h, 9).await;
    assert_eq!((r.status, r.content_type), (200, "image/jpeg"));
    assert_eq!(s.respond("GET", "/art?t=other", h, 9).await.status, 404);
    assert_eq!(s.respond("GET", "/art", h, 9).await.status, 404);
    // Sizes are kept in range.
    s.respond("GET", "/art?t=t1&s=99999", h, 9).await;
    assert_eq!(src.art_calls.lock().last().unwrap().1, server::ART_SIZE_MAX);
    let mut now = playing();
    now.track.thumb = Some("/missing".into());
    *src.now.lock() = Some(now);
    assert_eq!(s.respond("GET", "/art?t=t1", h, 9).await.status, 404);
}

#[tokio::test]
async fn the_overlay_page_takes_its_options_from_the_query() {
    let (s, _, _) = server();
    let r = s.respond("GET", "/now-playing.html?size=large&theme=clear&art=0&progress=0", Some("127.0.0.1:9"), 9).await;
    assert_eq!(r.status, 200);
    assert!(r.headers.iter().any(|(k, v)| *k == "Content-Security-Policy" && v.contains("connect-src 'self'")));
    let html = String::from_utf8(r.body).unwrap();
    assert!(html.contains(r#"<body class="size-large theme-clear no-art no-progress">"#));
    assert!(html.contains("/now-playing.json"));
    // Unknown values fall back to the defaults; nothing from the query reaches the page.
    let q = |s: &str| url::form_urlencoded::parse(s.as_bytes()).into_owned().collect::<Vec<_>>();
    assert_eq!(PageOptions::from_query(&q("size=<script>&theme=x")), PageOptions::default());
    let o = PageOptions::from_query(&q("size=small&theme=light&art=1"));
    assert_eq!((o.size, o.theme, o.art, o.progress), (Size::Small, Theme::Light, true, true));
    let html = String::from_utf8(s.respond("GET", "/now-playing.html?size=%22%3E%3Cscript%3E", Some("127.0.0.1:9"), 9).await.body).unwrap();
    assert!(!html.contains("\"><script>"));
}

#[test]
fn request_heads_parse() {
    let (m, t, h) = parse_head("GET /now-playing.json?art=data HTTP/1.1\r\nhost: 127.0.0.1:5\r\n\r\n").unwrap();
    assert_eq!((m.as_str(), t.as_str(), h.as_deref()), ("GET", "/now-playing.json?art=data", Some("127.0.0.1:5")));
    assert!(parse_head("GET http://x/ HTTP/1.1\r\n\r\n").is_none(), "absolute targets aren't for us");
}

#[test]
fn the_switch_only_announces_real_changes() {
    let sw = FeedSwitch::default();
    let mut rx = sw.subscribe();
    rx.borrow_and_update();
    sw.set(FeedConfig::default());
    assert!(!rx.has_changed().unwrap());
    sw.set(FeedConfig { enabled: true, ..FeedConfig::default() });
    assert!(rx.has_changed().unwrap());
    assert_eq!(sw.get().port, DEFAULT_PORT);
}
