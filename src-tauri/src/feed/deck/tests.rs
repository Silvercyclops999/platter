use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::pairing::{MAX_WRONG, PairError, Pairing, TOKEN_ACCOUNT};
use super::server::{DeckServer, Request, cover_key};
use super::*;
use crate::audio::testutil::track;
use crate::error::AppError;
use crate::plex::keychain::{MemoryStore, TokenStore};

#[derive(Default)]
struct Fake {
    now: Mutex<Option<DeckNow>>,
    liked: Mutex<bool>,
    lidarr: Mutex<bool>,
    calls: Mutex<Vec<String>>,
    fail_next: Mutex<bool>,
}

#[async_trait]
impl DeckHost for Fake {
    async fn now(&self) -> Option<DeckNow> {
        self.now.lock().clone()
    }
    async fn liked(&self, _: &str) -> bool {
        *self.liked.lock()
    }
    async fn cover(&self, thumb: &str, size: u32) -> AppResult<ArtBytes> {
        self.calls.lock().push(format!("cover {thumb} {size}"));
        Ok(ArtBytes { bytes: Bytes::from_static(b"\xff\xd8\xff"), content_type: "image/jpeg".into() })
    }
    fn can_request(&self) -> bool {
        *self.lidarr.lock()
    }
    async fn toggle(&self) -> AppResult<()> {
        self.calls.lock().push("toggle".into());
        Ok(())
    }
    async fn next(&self) -> AppResult<()> {
        if std::mem::take(&mut *self.fail_next.lock()) {
            return Err(AppError::Network("down".into()));
        }
        self.calls.lock().push("next".into());
        Ok(())
    }
    async fn previous(&self) -> AppResult<()> {
        self.calls.lock().push("previous".into());
        Ok(())
    }
    async fn set_liked(&self, id: &str, liked: bool) -> AppResult<()> {
        self.calls.lock().push(format!("like {id} {liked}"));
        *self.liked.lock() = liked;
        Ok(())
    }
    async fn set_volume(&self, v: f32) -> AppResult<()> {
        self.calls.lock().push(format!("volume {v:.2}"));
        Ok(())
    }
    async fn request_album(&self, t: &Track) -> AppResult<()> {
        self.calls.lock().push(format!("request {}", t.id));
        Ok(())
    }
    async fn listen_along(&self) -> AppResult<()> {
        self.calls.lock().push("listen".into());
        Ok(())
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<(String, serde_json::Value)>>);
impl crate::plex::state::EventSink for Events {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().push((event.to_owned(), payload));
    }
}

fn playing() -> DeckNow {
    let mut t = track("t1", "Rue Ellis", "/p");
    t.title = "Night Drive".into();
    t.album_title = "Late".into();
    t.thumb = Some("/library/metadata/9/thumb/1".into());
    DeckNow { track: Some(t), playing: true, volume: 0.5, muted: false }
}

struct Rig {
    server: DeckServer,
    host: Arc<Fake>,
    pairing: Arc<Pairing>,
    store: Arc<MemoryStore>,
    events: Arc<Events>,
}

fn rig() -> Rig {
    let host = Arc::new(Fake::default());
    *host.now.lock() = Some(playing());
    let store = Arc::new(MemoryStore::new());
    let pairing = Arc::new(Pairing::new(store.clone()));
    let events = Arc::new(Events::default());
    let server = DeckServer::new(host.clone(), pairing.clone(), events.clone());
    Rig { server, host, pairing, store, events }
}

const LOOP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const PORT: u16 = DECK_PORT;

fn req(method: &str, target: &str, token: Option<&str>, body: &str) -> Request {
    let mut headers = vec![("Host".to_owned(), format!("127.0.0.1:{PORT}"))];
    if let Some(t) = token {
        headers.push(("Authorization".into(), format!("Bearer {t}")));
    }
    Request { method: method.into(), target: target.into(), headers, body: body.as_bytes().to_vec() }
}

impl Rig {
    async fn send(&self, r: &Request) -> server::Reply {
        self.server.respond(r, LOOP, PORT, Instant::now()).await
    }

    async fn paired(&self) -> String {
        let code = self.pairing.code();
        let r = self.send(&req("POST", "/v1/pair", None, &format!(r#"{{"code":"{code}"}}"#))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        r.body["token"].as_str().unwrap().to_owned()
    }

    async fn command(&self, token: &str, body: &str) -> server::Reply {
        self.send(&req("POST", "/v1/command", Some(token), body)).await
    }
}

// ---------------------------------------------------------------- pairing

#[test]
fn the_code_is_six_digits() {
    let p = Pairing::new(Arc::new(MemoryStore::new()));
    let c = p.code();
    assert_eq!(c.len(), 6);
    assert!(c.chars().all(|ch| ch.is_ascii_digit()));
}

#[test]
fn pairing_keeps_only_a_hash_in_the_keychain() {
    let store = Arc::new(MemoryStore::new());
    let p = Pairing::new(store.clone());
    assert!(!p.paired());
    let code = p.code();
    let token = p.pair(&code).unwrap();
    assert_eq!(token.len(), 64);
    let kept = store.get(TOKEN_ACCOUNT).unwrap().unwrap();
    assert_ne!(kept.expose(), token, "the token itself is never stored");
    assert!(p.paired() && p.check(&token));
    assert!(!p.check("nope") && !p.check(""));
    // The code is used up.
    assert_ne!(p.code(), code);
    assert_eq!(p.pair(&code), Err(PairError::WrongCode));
    // A new Pairing (next launch) reads the keychain.
    let again = Pairing::new(store);
    assert!(again.check(&token));
}

#[test]
fn wrong_codes_make_a_new_code() {
    let p = Pairing::new(Arc::new(MemoryStore::new()));
    let code = p.code();
    let wrong = if code == "000000" { "000001" } else { "000000" };
    for _ in 0..MAX_WRONG - 1 {
        assert_eq!(p.pair(wrong), Err(PairError::WrongCode));
        assert_eq!(p.code(), code);
    }
    assert_eq!(p.pair(wrong), Err(PairError::WrongCode));
    assert_ne!(p.code(), code, "the code changes after too many wrong ones");
}

#[test]
fn revoke_forgets_the_token() {
    let store = Arc::new(MemoryStore::new());
    let p = Pairing::new(store.clone());
    let token = p.pair(&p.code()).unwrap();
    let code = p.code();
    p.revoke().unwrap();
    assert!(!p.paired() && !p.check(&token));
    assert_eq!(store.get(TOKEN_ACCOUNT).unwrap(), None);
    assert_ne!(p.code(), code);
}

#[test]
fn pairing_again_replaces_the_old_token() {
    let p = Pairing::new(Arc::new(MemoryStore::new()));
    let a = p.pair(&p.code()).unwrap();
    let b = p.pair(&p.code()).unwrap();
    assert!(!p.check(&a) && p.check(&b));
}

/// A keychain that refuses everything.
struct Locked;
impl TokenStore for Locked {
    fn get(&self, _: &str) -> AppResult<Option<crate::plex::Token>> {
        Err(AppError::Internal("keychain is locked or unavailable".into()))
    }
    fn set(&self, _: &str, _: &crate::plex::Token) -> AppResult<()> {
        Err(AppError::Internal("keychain is locked or unavailable".into()))
    }
    fn delete(&self, _: &str) -> AppResult<()> {
        Err(AppError::Internal("keychain is locked or unavailable".into()))
    }
}

#[tokio::test]
async fn a_locked_keychain_fails_pairing_cleanly() {
    let host = Arc::new(Fake::default());
    let pairing = Arc::new(Pairing::new(Arc::new(Locked)));
    let s = DeckServer::new(host, pairing.clone(), Arc::new(Events::default()));
    assert!(!pairing.paired());
    let r = s.respond(&req("POST", "/v1/pair", None, &format!(r#"{{"code":"{}"}}"#, pairing.code())), LOOP, PORT, Instant::now()).await;
    assert_eq!(r.status, 500);
    assert!(s.revoke().is_err());
}

// ---------------------------------------------------------------- rate limit

#[test]
fn the_rate_limit_refills_over_time() {
    let t0 = Instant::now();
    let mut l = RateLimit::new(2.0, 3.0);
    assert!(l.allow(t0) && l.allow(t0) && l.allow(t0));
    assert!(!l.allow(t0));
    assert!(!l.allow(t0 + Duration::from_millis(400)));
    assert!(l.allow(t0 + Duration::from_millis(1000)));
    // Never more than the burst, however long it waits.
    let later = t0 + Duration::from_secs(60);
    assert!(l.allow(later) && l.allow(later) && l.allow(later));
    assert!(!l.allow(later));
}

#[tokio::test]
async fn too_many_requests_get_429() {
    let r = rig();
    let t = r.paired().await;
    let now = Instant::now();
    let mut codes = vec![];
    for _ in 0..(BURST as usize + 5) {
        codes.push(r.server.respond(&req("GET", "/v1/state?cover=x", Some(&t), ""), LOOP, PORT, now).await.status);
    }
    assert!(codes.iter().filter(|c| **c == 429).count() >= 4, "{codes:?}");
    // A second later some go through again.
    let later = now + Duration::from_secs(1);
    assert_eq!(r.server.respond(&req("GET", "/v1/hello", None, ""), LOOP, PORT, later).await.status, 200);
}

#[tokio::test]
async fn pairing_attempts_are_slowed_down() {
    let r = rig();
    let now = Instant::now();
    let mut codes = vec![];
    for _ in 0..5 {
        codes.push(r.server.respond(&req("POST", "/v1/pair", None, r#"{"code":"x"}"#), LOOP, PORT, now).await.status);
    }
    assert_eq!(codes, vec![401, 401, 401, 429, 429]);
}

// ---------------------------------------------------------------- who may ask

#[tokio::test]
async fn other_computers_are_turned_away() {
    let r = rig();
    let reply = r.server.respond(&req("GET", "/v1/hello", None, ""), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)), PORT, Instant::now()).await;
    assert_eq!(reply.status, 403);
}

#[tokio::test]
async fn other_host_names_and_web_pages_are_turned_away() {
    let r = rig();
    let mut q = req("GET", "/v1/hello", None, "");
    q.headers[0].1 = format!("evil.example:{PORT}");
    assert_eq!(r.send(&q).await.status, 403);
    let mut q = req("GET", "/v1/hello", None, "");
    q.headers.push(("Origin".into(), "https://evil.example".into()));
    assert_eq!(r.send(&q).await.status, 403);
    let mut q = req("POST", "/v1/pair", None, r#"{"code":"1"}"#);
    q.headers.push(("Sec-Fetch-Site".into(), "cross-site".into()));
    assert_eq!(r.send(&q).await.status, 403);
    // localhost by name is fine.
    let mut q = req("GET", "/v1/hello", None, "");
    q.headers[0].1 = format!("localhost:{PORT}");
    assert_eq!(r.send(&q).await.status, 200);
}

#[tokio::test]
async fn unpaired_clients_get_nothing() {
    let r = rig();
    for (m, p) in [("GET", "/v1/state"), ("POST", "/v1/command")] {
        assert_eq!(r.send(&req(m, p, None, r#"{"command":"next"}"#)).await.status, 401);
        assert_eq!(r.send(&req(m, p, Some("made-up"), r#"{"command":"next"}"#)).await.status, 401);
    }
    assert!(r.host.calls.lock().is_empty());
    let hello = r.send(&req("GET", "/v1/hello", None, "")).await;
    assert_eq!(hello.body["paired"], false);
    assert_eq!(hello.body["app"], "Platter");
}

#[tokio::test]
async fn a_revoked_token_stops_working() {
    let r = rig();
    let t = r.paired().await;
    assert_eq!(r.send(&req("GET", "/v1/hello", Some(&t), "")).await.body["paired"], true);
    let st = r.server.revoke().unwrap();
    assert!(!st.paired);
    assert_eq!(r.send(&req("GET", "/v1/state", Some(&t), "")).await.status, 401);
    assert_eq!(r.store.get(TOKEN_ACCOUNT).unwrap(), None);
}

#[tokio::test]
async fn pairing_announces_the_new_status() {
    let r = rig();
    r.paired().await;
    let ev = r.events.0.lock();
    let last = ev.iter().rev().find(|(e, _)| e == "deck://status").unwrap();
    assert_eq!(last.1["paired"], true);
}

// ---------------------------------------------------------------- state

#[tokio::test]
async fn state_has_the_song_cover_and_volume() {
    let r = rig();
    *r.host.lidarr.lock() = true;
    let t = r.paired().await;
    let s = r.send(&req("GET", "/v1/state", Some(&t), "")).await;
    assert_eq!(s.status, 200);
    let b = &s.body;
    assert_eq!(b["title"], "Night Drive");
    assert_eq!(b["artist"], "Rue Ellis");
    assert_eq!(b["album"], "Late");
    assert_eq!(b["playing"], true);
    assert_eq!(b["volume"], 50);
    assert_eq!(b["liked"], false);
    assert_eq!(b["canRequest"], true);
    assert!(b["cover"].as_str().unwrap().starts_with("data:image/jpeg;base64,"));
    assert!(r.host.calls.lock().contains(&format!("cover /library/metadata/9/thumb/1 {COVER_SIZE}")));
    // Sending the key back skips the cover.
    let key = b["coverKey"].as_str().unwrap().to_owned();
    assert_eq!(key, cover_key("/library/metadata/9/thumb/1"));
    let s = r.send(&req("GET", &format!("/v1/state?cover={key}"), Some(&t), "")).await;
    assert!(s.body.get("cover").is_none());
    assert_eq!(s.body["coverKey"], key.as_str());
}

#[tokio::test]
async fn a_private_session_hides_the_song() {
    let r = rig();
    r.server.set_config(DeckConfig { enabled: true, private_session: true });
    let t = r.paired().await;
    let b = r.send(&req("GET", "/v1/state", Some(&t), "")).await.body;
    assert_eq!(b["private"], true);
    assert!(b["title"].is_null() && b["cover"].is_null() && b["coverKey"].is_null());
    assert_eq!(b["playing"], true, "play state still shows on the key");
}

#[tokio::test]
async fn nothing_playing_is_an_empty_state() {
    let r = rig();
    *r.host.now.lock() = None;
    let t = r.paired().await;
    let b = r.send(&req("GET", "/v1/state", Some(&t), "")).await.body;
    assert_eq!(b["playing"], false);
    assert!(b["title"].is_null());
}

// ---------------------------------------------------------------- commands

#[tokio::test]
async fn commands_reach_the_player() {
    let r = rig();
    let t = r.paired().await;
    for c in ["playPause", "next", "previous", "listenAlong"] {
        let reply = r.command(&t, &format!(r#"{{"command":"{c}"}}"#)).await;
        assert_eq!(reply.status, 200, "{c}");
    }
    assert_eq!(*r.host.calls.lock(), vec!["toggle", "next", "previous", "listen"]);
}

#[tokio::test]
async fn like_toggles() {
    let r = rig();
    let t = r.paired().await;
    r.command(&t, r#"{"command":"like"}"#).await;
    r.command(&t, r#"{"command":"like"}"#).await;
    assert_eq!(*r.host.calls.lock(), vec!["like t1 true", "like t1 false"]);
}

#[tokio::test]
async fn volume_sets_and_steps_within_range() {
    let r = rig();
    let t = r.paired().await;
    r.command(&t, r#"{"command":"volumeSet","value":30}"#).await;
    r.command(&t, r#"{"command":"volumeStep","value":-5}"#).await;
    r.command(&t, r#"{"command":"volumeStep","value":80}"#).await;
    r.command(&t, r#"{"command":"volumeSet","value":-20}"#).await;
    assert_eq!(*r.host.calls.lock(), vec!["volume 0.30", "volume 0.45", "volume 1.00", "volume 0.00"]);
    // Muted steps up from zero.
    r.host.now.lock().as_mut().unwrap().muted = true;
    r.host.calls.lock().clear();
    r.command(&t, r#"{"command":"volumeStep","value":5}"#).await;
    assert_eq!(*r.host.calls.lock(), vec!["volume 0.05"]);
}

#[tokio::test]
async fn request_album_needs_lidarr_and_a_song() {
    let r = rig();
    let t = r.paired().await;
    let reply = r.command(&t, r#"{"command":"requestAlbum"}"#).await;
    assert_eq!(reply.status, 409);
    assert!(reply.body["error"].as_str().unwrap().contains("Lidarr"));
    *r.host.lidarr.lock() = true;
    assert_eq!(r.command(&t, r#"{"command":"requestAlbum"}"#).await.status, 200);
    *r.host.now.lock() = None;
    assert_eq!(r.command(&t, r#"{"command":"requestAlbum"}"#).await.status, 409);
    assert_eq!(*r.host.calls.lock(), vec!["request t1"]);
}

#[tokio::test]
async fn a_failed_command_says_why_without_details() {
    let r = rig();
    let t = r.paired().await;
    *r.host.fail_next.lock() = true;
    let reply = r.command(&t, r#"{"command":"next"}"#).await;
    assert_eq!(reply.status, 409);
    assert_eq!(reply.body["error"], "Platter couldn't reach the server.");
}

#[tokio::test]
async fn unknown_commands_and_routes() {
    let r = rig();
    let t = r.paired().await;
    assert_eq!(r.command(&t, r#"{"command":"formatDisk"}"#).await.status, 400);
    assert_eq!(r.command(&t, "not json").await.status, 400);
    assert_eq!(r.send(&req("GET", "/v1/command", Some(&t), "")).await.status, 405);
    assert_eq!(r.send(&req("GET", "/nope", Some(&t), "")).await.status, 404);
}

// ---------------------------------------------------------------- over the wire

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn http(addr: SocketAddr, raw: String) -> (u16, serde_json::Value) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(raw.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_slice(&buf[split + 4..]).unwrap_or_default())
}

async fn wait_for(s: &DeckServer, want: impl Fn(&Listening) -> bool) -> DeckStatus {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        let st = s.status();
        if want(&st.listening) {
            return st;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("status never settled: {:?}", s.status());
}

#[tokio::test]
async fn the_port_opens_when_allowed_and_closes_when_not() {
    let r = rig();
    let port = free_port();
    let switch = DeckSwitch::default();
    tokio::spawn(r.server.clone().follow(switch.subscribe(), port));
    let st = wait_for(&r.server, |l| *l == Listening::Off).await;
    assert_eq!(st.code, None, "no code while it's off");
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());

    switch.set(DeckConfig { enabled: true, private_session: false });
    let st = wait_for(&r.server, |l| *l == Listening::On { port }).await;
    assert_eq!(st.code.as_deref(), Some(r.pairing.code().as_str()));
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let code = r.pairing.code();
    let body = format!(r#"{{"code":"{code}"}}"#);
    let (status, b) = http(
        addr,
        format!("POST /v1/pair HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
    )
    .await;
    assert_eq!(status, 200);
    let token = b["token"].as_str().unwrap().to_owned();
    let cmd = r#"{"command":"next"}"#;
    let (status, _) = http(
        addr,
        format!("POST /v1/command HTTP/1.1\r\nHost: localhost:{port}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{cmd}", cmd.len()),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(*r.host.calls.lock(), vec!["next"]);
    // A body that's too big is refused before it's read.
    let (status, _) = http(addr, format!("POST /v1/command HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 999999\r\n\r\n")).await;
    assert_eq!(status, 413);

    switch.set(DeckConfig { enabled: false, private_session: false });
    wait_for(&r.server, |l| *l == Listening::Off).await;
    let mut closed = false;
    for _ in 0..100 {
        if TcpStream::connect(addr).await.is_err() {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(closed, "turning it off closes the port");
}

#[tokio::test]
async fn a_busy_port_is_reported() {
    let r = rig();
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let switch = DeckSwitch::default();
    switch.set(DeckConfig { enabled: true, private_session: false });
    tokio::spawn(r.server.clone().follow(switch.subscribe(), port));
    let st = wait_for(&r.server, |l| !matches!(l, Listening::Off)).await;
    assert!(matches!(st.listening, Listening::PortInUse { .. } | Listening::Failed { .. }), "{st:?}");
    drop(held);
}

#[test]
fn status_serializes_flat() {
    let st = DeckStatus { listening: Listening::On { port: 1 }, code: Some("123456".into()), paired: false };
    assert_eq!(serde_json::to_value(&st).unwrap(), serde_json::json!({ "state": "on", "port": 1, "code": "123456", "paired": false }));
}
