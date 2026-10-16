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
use crate::audio::queue::Repeat;
use crate::audio::source::{ContextKind, PlayContext};
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
    extras: Mutex<DeckExtras>,
    devices: Mutex<Vec<DeckDevice>>,
}

impl Fake {
    fn call(&self, c: String) -> AppResult<()> {
        self.calls.lock().push(c);
        Ok(())
    }
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
    async fn pause(&self) -> AppResult<()> {
        self.calls.lock().push("pause".into());
        Ok(())
    }
    async fn resume(&self) -> AppResult<()> {
        self.calls.lock().push("resume".into());
        Ok(())
    }
    async fn play_playlist(&self, query: &str, shuffle: bool) -> AppResult<()> {
        if query == "missing" {
            return Err(AppError::NotFound("No playlist called missing.".into()));
        }
        self.calls.lock().push(format!("playlist {query} {shuffle}"));
        Ok(())
    }
    async fn show(&self, route: serde_json::Value) -> AppResult<()> {
        self.calls.lock().push(format!("show {route}"));
        Ok(())
    }
    fn notice(&self, message: &str) {
        self.calls.lock().push(format!("notice {message}"));
    }
    fn raise(&self) {
        self.calls.lock().push("raise".into());
    }
    async fn extras(&self, _: Option<&Track>) -> DeckExtras {
        self.extras.lock().clone()
    }
    async fn seek(&self, ms: u64) -> AppResult<()> {
        self.call(format!("seek {ms}"))
    }
    async fn set_shuffle(&self, on: bool) -> AppResult<()> {
        self.call(format!("shuffle {on}"))
    }
    async fn set_smart_shuffle(&self, on: bool) -> AppResult<()> {
        self.call(format!("smart {on}"))
    }
    async fn set_repeat(&self, r: Repeat) -> AppResult<()> {
        self.call(format!("repeat {r:?}"))
    }
    async fn set_muted(&self, m: bool) -> AppResult<()> {
        self.call(format!("mute {m}"))
    }
    async fn set_settings(&self, patch: serde_json::Value) -> AppResult<()> {
        self.call(format!("settings {patch}"))
    }
    async fn rate(&self, id: &str, stars: Option<u8>) -> AppResult<()> {
        self.call(format!("rate {id} {stars:?}"))
    }
    async fn playlist_add(&self, p: &str, t: &str) -> AppResult<()> {
        self.call(format!("add {p} {t}"))
    }
    async fn playlist_remove(&self, p: &str, t: &str) -> AppResult<()> {
        self.call(format!("remove {p} {t}"))
    }
    async fn play(&self, ctx: PlayContext, start: Option<String>, start_ms: u64, shuffle: bool) -> AppResult<()> {
        self.call(format!("play {:?} {:?} {:?} {start:?} {start_ms} {shuffle}", ctx.kind, ctx.id, ctx.track_ids))
    }
    async fn queue_context(&self, ctx: PlayContext) -> AppResult<()> {
        self.call(format!("queue {:?} {:?}", ctx.kind, ctx.id))
    }
    async fn resolve_link(&self, link: &str) -> Option<DeckLink> {
        match link {
            "platter://album/5" => Some(DeckLink::Album("5".into())),
            "platter://track/7?t=30" => Some(DeckLink::Track { id: "7".into(), album_id: "5".into(), at_seconds: Some(30) }),
            _ => None,
        }
    }
    async fn search_tracks(&self, q: &str, limit: usize) -> AppResult<Vec<String>> {
        assert_eq!(limit, 50);
        Ok(if q == "nothing" { vec![] } else { vec!["s1".into(), "s2".into()] })
    }
    async fn mix_of_kind(&self, kind: &str) -> AppResult<Option<String>> {
        Ok((kind == "daylist").then(|| "mix-day".into()))
    }
    async fn library(&self, kind: LibraryKind) -> AppResult<Vec<DeckItem>> {
        Ok(vec![DeckItem {
            id: "p1".into(),
            title: format!("{kind:?}"),
            subtitle: Some("3 songs".into()),
            thumb: Some("/library/metadata/77/thumb/1".into()),
            cover_key: None,
        }])
    }
    async fn queue(&self) -> Vec<DeckQueued> {
        let mut t = track("q1", "Ana Vale", "/p");
        t.title = "Shoreline".into();
        t.thumb = Some("/library/metadata/8/thumb/1".into());
        vec![DeckQueued { item_id: "i1".into(), track: t }]
    }
    async fn queue_jump(&self, id: &str) -> AppResult<()> {
        self.call(format!("jump {id}"))
    }
    async fn devices(&self) -> Vec<DeckDevice> {
        self.devices.lock().clone()
    }
    async fn listen_on(&self, d: &str) -> AppResult<()> {
        self.call(format!("listen on {d}"))
    }
    async fn set_miniplayer(&self, open: bool) -> AppResult<()> {
        self.call(format!("miniplayer {open}"))
    }
    async fn set_window(&self, v: bool) -> AppResult<()> {
        self.call(format!("window {v}"))
    }
    fn window_action(&self, payload: serde_json::Value, bring_up: bool) {
        self.calls.lock().push(format!("action {payload} {bring_up}"));
    }
    async fn bookmark(&self, t: &Track, at: u64) -> AppResult<()> {
        self.call(format!("bookmark {} {at}", t.id))
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
    let mut next = track("t2", "Ana Vale", "/p");
    next.title = "Shoreline".into();
    next.thumb = Some("/library/metadata/8/thumb/1".into());
    DeckNow {
        track: Some(t),
        playing: true,
        volume: 0.5,
        muted: false,
        position_ms: 61_000,
        duration_ms: 200_000,
        next: Some(next),
        ..Default::default()
    }
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

// ---------------------------------------------------------------- pairing in the window

impl Rig {
    async fn at(&self, r: &Request, now: Instant) -> server::Reply {
        self.server.respond(r, LOOP, PORT, now).await
    }

    async fn ask(&self, now: Instant) -> server::Reply {
        self.at(&req("POST", "/v1/pair/request", None, r#"{"client":"streamdeck"}"#), now).await
    }

    async fn ask_status(&self, id: &str, now: Instant) -> server::Reply {
        self.at(&req("POST", "/v1/pair/status", None, &format!(r#"{{"request":"{id}"}}"#)), now).await
    }

    fn prompts(&self) -> Vec<serde_json::Value> {
        self.events.0.lock().iter().filter(|(e, _)| e == "deck://pair-request").map(|(_, p)| p.clone()).collect()
    }
}

#[tokio::test]
async fn asking_brings_the_window_up_and_allowing_pairs() {
    let r = rig();
    let t0 = Instant::now();
    let a = r.ask(t0).await;
    assert_eq!(a.status, 200, "{:?}", a.body);
    let id = a.body["request"].as_str().unwrap().to_owned();
    assert_eq!(id.len(), 32);
    assert!(r.host.calls.lock().contains(&"raise".to_owned()), "the window comes up");
    assert_eq!(r.prompts(), vec![serde_json::json!({ "id": id })]);
    assert_eq!(r.server.pair_prompt().as_deref(), Some(id.as_str()));
    // Nothing is paired while it waits, however often the plugin checks.
    for _ in 0..3 {
        assert_eq!(r.ask_status(&id, t0).await.body, serde_json::json!({ "state": "waiting" }));
    }
    assert!(!r.pairing.paired());
    assert!(r.server.pair_answer_at(&id, true, t0 + Duration::from_secs(3)).unwrap());
    assert_eq!(r.prompts().last(), Some(&serde_json::json!({ "id": null })), "the prompt closes");
    assert!(r.server.pair_prompt().is_none());
    let s = r.ask_status(&id, t0 + Duration::from_secs(4)).await;
    assert_eq!(s.body["state"], "allowed");
    let token = s.body["token"].as_str().unwrap().to_owned();
    assert!(r.pairing.check(&token));
    assert!(r.server.status().paired);
    assert_eq!(r.command(&token, r#"{"command":"next"}"#).await.status, 200);
    // The token is handed over once.
    assert_eq!(r.ask_status(&id, t0 + Duration::from_secs(5)).await.body["state"], "gone");
    // Only a hash is kept.
    assert_ne!(r.store.get(TOKEN_ACCOUNT).unwrap().unwrap().expose(), token);
}

#[tokio::test]
async fn dont_allow_pairs_nothing() {
    let r = rig();
    let t0 = Instant::now();
    let id = r.ask(t0).await.body["request"].as_str().unwrap().to_owned();
    assert!(r.server.pair_answer_at(&id, false, t0).unwrap());
    assert_eq!(r.ask_status(&id, t0).await.body, serde_json::json!({ "state": "denied" }));
    assert!(!r.pairing.paired());
    // Answering again, or another id, does nothing.
    assert!(!r.server.pair_answer_at(&id, true, t0).unwrap());
    assert!(!r.server.pair_answer_at("nope", true, t0).unwrap());
    assert!(!r.pairing.paired());
    assert_eq!(r.ask_status("someone-else", t0).await.body["state"], "gone");
}

#[tokio::test]
async fn one_prompt_at_a_time_and_a_few_seconds_between() {
    let r = rig();
    let t0 = Instant::now();
    let id = r.ask(t0).await.body["request"].as_str().unwrap().to_owned();
    // Another ask while it shows is turned away, and doesn't raise again.
    let busy = r.ask(t0 + Duration::from_secs(10)).await;
    assert_eq!(busy.status, 409, "{:?}", busy.body);
    assert_eq!(r.host.calls.lock().iter().filter(|c| *c == "raise").count(), 1);
    assert_eq!(r.prompts().len(), 1);
    assert!(r.server.pair_answer_at(&id, false, t0 + Duration::from_secs(10)).unwrap());
    // Right after an answer: too soon.
    assert_eq!(r.ask(t0 + Duration::from_secs(12)).await.status, 429);
    // A few seconds later it may ask again.
    let again = r.ask(t0 + Duration::from_secs(10) + ask::GAP).await;
    assert_eq!(again.status, 200, "{:?}", again.body);
    assert_ne!(again.body["request"].as_str().unwrap(), id);
}

#[tokio::test]
async fn asks_share_the_pairing_rate_limit() {
    let r = rig();
    let t0 = Instant::now();
    let mut codes = vec![];
    for _ in 0..5 {
        codes.push(r.ask(t0).await.status);
    }
    // One prompt, then busy, then slowed down.
    assert_eq!(codes, vec![200, 409, 409, 429, 429]);
}

#[test]
fn an_unanswered_prompt_goes_away() {
    let mut a = ask::Asks::default();
    let t0 = Instant::now();
    let id = a.open(t0).unwrap();
    assert_eq!(a.current(t0 + ask::TTL - Duration::from_secs(1)).as_deref(), Some(id.as_str()));
    assert_eq!(a.current(t0 + ask::TTL), None);
    assert_eq!(a.status(&id, t0 + ask::TTL), ask::AskState::Gone);
    // Too late to allow it.
    let late: Result<bool, ()> = a.answer(&id, true, t0 + ask::TTL, || panic!("no token for a gone prompt"));
    assert_eq!(late, Ok(false));
    assert!(a.open(t0 + ask::TTL).is_ok());
}

#[test]
fn nothing_is_allowed_without_an_answer() {
    let mut a = ask::Asks::default();
    let t0 = Instant::now();
    let id = a.open(t0).unwrap();
    for s in 0..200 {
        assert!(!matches!(a.status(&id, t0 + Duration::from_secs(s)), ask::AskState::Allowed(_)));
    }
}

#[tokio::test]
async fn asking_needs_allow_stream_deck() {
    let r = rig();
    r.server.set_config(DeckConfig { enabled: false, private_session: false, automation: true });
    let a = r.ask(Instant::now()).await;
    assert_eq!(a.status, 403);
    assert!(r.prompts().is_empty() && !r.host.calls.lock().contains(&"raise".to_owned()));
}

#[tokio::test]
async fn the_code_still_works_and_asking_replaces_its_token() {
    let r = rig();
    let by_code = r.paired().await;
    assert!(r.pairing.check(&by_code));
    let t0 = Instant::now();
    let id = r.ask(t0).await.body["request"].as_str().unwrap().to_owned();
    // The code shown in Settings still pairs while a prompt is open.
    let by_code_again = r.paired().await;
    assert!(r.pairing.check(&by_code_again));
    r.server.pair_answer_at(&id, true, t0).unwrap();
    let token = r.ask_status(&id, t0).await.body["token"].as_str().unwrap().to_owned();
    assert!(r.pairing.check(&token) && !r.pairing.check(&by_code_again), "one pairing, the newest");
    // And the code still works after (once the pairing limit refills).
    let code = r.pairing.code();
    let last = r.at(&req("POST", "/v1/pair", None, &format!(r#"{{"code":"{code}"}}"#)), t0 + Duration::from_secs(10)).await;
    assert!(r.pairing.check(last.body["token"].as_str().unwrap()));
}

#[tokio::test]
async fn a_locked_keychain_fails_allowing_cleanly() {
    let host = Arc::new(Fake::default());
    let s = DeckServer::new(host, Arc::new(Pairing::new(Arc::new(Locked))), Arc::new(Events::default()));
    let t0 = Instant::now();
    let a = s.respond(&req("POST", "/v1/pair/request", None, "{}"), LOOP, PORT, t0).await;
    let id = a.body["request"].as_str().unwrap().to_owned();
    assert!(s.pair_answer_at(&id, true, t0).is_err());
    // Still showing, so the person can try again.
    assert_eq!(s.pair_prompt().as_deref(), Some(id.as_str()));
}

#[tokio::test]
async fn the_pair_calls_want_post() {
    let r = rig();
    assert_eq!(r.send(&req("GET", "/v1/pair/request", None, "")).await.status, 405);
    assert_eq!(r.send(&req("GET", "/v1/pair/status", None, "")).await.status, 405);
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
    r.server.set_config(DeckConfig { enabled: true, private_session: true, automation: false });
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

    switch.set(DeckConfig { enabled: true, private_session: false, automation: false });
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

    switch.set(DeckConfig { enabled: false, private_session: false, automation: false });
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
    switch.set(DeckConfig { enabled: true, private_session: false, automation: false });
    tokio::spawn(r.server.clone().follow(switch.subscribe(), port));
    let st = wait_for(&r.server, |l| !matches!(l, Listening::Off)).await;
    assert!(matches!(st.listening, Listening::PortInUse { .. } | Listening::Failed { .. }), "{st:?}");
    drop(held);
}

#[test]
fn status_serializes_flat() {
    let st = DeckStatus { listening: Listening::On { port: 1 }, code: Some("123456".into()), paired: false, automation_code: None, automation_paired: false };
    assert_eq!(serde_json::to_value(&st).unwrap(), serde_json::json!({ "state": "on", "port": 1, "code": "123456", "paired": false, "automationCode": null, "automationPaired": false }));
}

// ---------------------------------------------------------------- automation

fn automation_on(r: &Rig, deck: bool) {
    r.server.set_config(DeckConfig { enabled: deck, private_session: false, automation: true });
}

impl Rig {
    async fn pair_automation(&self, code: &str) -> server::Reply {
        self.send(&req("POST", "/v1/pair", None, &format!(r#"{{"code":"{code}","client":"automation"}}"#))).await
    }
}

#[tokio::test]
async fn automation_pairs_on_its_own_code_only_while_allowed() {
    let r = rig();
    r.server.set_config(DeckConfig { enabled: true, private_session: false, automation: false });
    let code = r.server.automation_code_for_tests();
    assert_eq!(r.pair_automation(&code).await.status, 403, "refused while automation is off");
    let deck = r.paired().await;
    automation_on(&r, true);
    let wrong = r.pair_automation(&r.pairing.code()).await;
    assert_eq!(wrong.status, 401, "the Stream Deck's code doesn't pair automation");
    assert!(!r.server.status().automation_paired);
    assert!(r.store.get(pairing::AUTOMATION_ACCOUNT).unwrap().is_none());
    assert!(r.store.get(TOKEN_ACCOUNT).unwrap().is_some(), "the Stream Deck keeps its pairing");
    assert_eq!(r.command(&deck, r#"{"command":"next"}"#).await.status, 200);
}

#[tokio::test]
async fn automation_and_stream_deck_tokens_follow_their_settings() {
    let r = rig();
    automation_on(&r, true);
    let deck = r.paired().await;
    // Pair automation with the code the server holds.
    let auto_code = r.server.automation_code_for_tests();
    let a = r.pair_automation(&auto_code).await;
    assert_eq!(a.status, 200, "{:?}", a.body);
    let auto = a.body["token"].as_str().unwrap().to_owned();
    assert_ne!(auto, deck);
    assert!(r.server.status().automation_paired && r.server.status().paired);
    assert_eq!(r.command(&auto, r#"{"command":"pause"}"#).await.status, 200);
    // Stream Deck off, automation on: only the automation token works.
    automation_on(&r, false);
    assert_eq!(r.command(&deck, r#"{"command":"next"}"#).await.status, 401);
    assert_eq!(r.command(&auto, r#"{"command":"next"}"#).await.status, 200);
    assert_eq!(r.server.status().code, None, "no Stream Deck code while it's off");
    // Automation off: its token stops working.
    r.server.set_config(DeckConfig { enabled: true, private_session: false, automation: false });
    assert_eq!(r.command(&auto, r#"{"command":"next"}"#).await.status, 401);
    assert_eq!(r.command(&deck, r#"{"command":"next"}"#).await.status, 200);
    // Revoke forgets the automation pairing only.
    automation_on(&r, true);
    r.server.revoke_automation().unwrap();
    assert_eq!(r.command(&auto, r#"{"command":"next"}"#).await.status, 401);
    assert_eq!(r.command(&deck, r#"{"command":"next"}"#).await.status, 200);
}

#[tokio::test]
async fn automation_commands_play_pause_playlists_search_and_pages() {
    let r = rig();
    automation_on(&r, true);
    let t = r.paired().await;
    for body in [
        r#"{"command":"play"}"#,
        r#"{"command":"pause"}"#,
        r#"{"command":"playPlaylist","playlist":" Road Trip ","shuffle":true}"#,
        r#"{"command":"playPlaylist","playlist":"Gym"}"#,
        r#"{"command":"search","query":"night drive"}"#,
        r#"{"command":"open","page":"liked"}"#,
    ] {
        assert_eq!(r.command(&t, body).await.status, 200, "{body}");
    }
    assert_eq!(
        *r.host.calls.lock(),
        vec![
            "resume",
            "pause",
            "playlist Road Trip true",
            "playlist Gym false",
            r#"show {"name":"search","query":"night drive"}"#,
            r#"show {"name":"liked"}"#,
        ]
    );
    let missing = r.command(&t, r#"{"command":"playPlaylist","playlist":"missing"}"#).await;
    assert_eq!((missing.status, missing.body["error"].as_str()), (409, Some("No playlist called missing.")));
    let page = r.command(&t, r#"{"command":"open","page":"nowhere"}"#).await;
    assert_eq!(page.body["error"], "Platter has no page called nowhere.");
    assert_eq!(r.command(&t, r#"{"command":"playPlaylist","playlist":""}"#).await.status, 409);
    // The command line asks for the state without the cover.
    let s = r.send(&req("GET", "/v1/state?cover=none", Some(&t), "")).await;
    assert!(s.body.get("cover").is_none() && s.body["title"] == "Night Drive");
}

#[tokio::test]
async fn command_links_run_only_while_automation_is_on() {
    let r = rig();
    r.server.run_link(automation::parse_link("platter://next").unwrap()).await;
    assert_eq!(*r.host.calls.lock(), vec!["notice Turn on Allow automation in Settings to control Platter from links."]);
    r.host.calls.lock().clear();
    automation_on(&r, false);
    r.server.run_link(automation::parse_link("platter://next").unwrap()).await;
    r.server.run_link(automation::parse_link("platter://play?playlist=missing").unwrap()).await;
    r.server.run_link(automation::parse_link("platter://open/settings").unwrap()).await;
    assert_eq!(
        *r.host.calls.lock(),
        vec!["next", "notice No playlist called missing.", r#"show {"name":"settings"}"#]
    );
}

// ---------------------------------------------------------------- command line

fn args(a: &[&str]) -> Vec<String> {
    std::iter::once("platter").chain(a.iter().copied()).map(String::from).collect()
}

#[test]
fn command_line_flags() {
    use cli::{Action, parse};
    assert_eq!(parse(&args(&[])), None, "no flags: the app starts");
    assert_eq!(parse(&args(&["--autostart"])), None);
    assert_eq!(parse(&args(&["platter://next"])), None, "links go to the app");
    assert_eq!(parse(&args(&["--next"])), Some(Ok(Action::Command(serde_json::json!({ "command": "next" })))));
    assert_eq!(parse(&args(&["--toggle"])), Some(Ok(Action::Command(serde_json::json!({ "command": "playPause" })))));
    assert_eq!(parse(&args(&["--now-playing", "--json"])), Some(Ok(Action::NowPlaying { json: true })));
    assert_eq!(
        parse(&args(&["--playlist", "Road Trip", "--shuffle"])),
        Some(Ok(Action::Command(serde_json::json!({ "command": "playPlaylist", "playlist": "Road Trip", "shuffle": true }))))
    );
    assert_eq!(parse(&args(&["--pair", "123456"])), Some(Ok(Action::Pair("123456".into()))));
    assert!(matches!(parse(&args(&["--pair", "12"])), Some(Err(_))));
    assert!(matches!(parse(&args(&["--playlist"])), Some(Err(_))));
    assert_eq!(parse(&args(&["--help"])), Some(Ok(Action::Help)));
}

#[test]
fn now_playing_line() {
    assert_eq!(cli::now_line(&serde_json::json!({ "playing": true, "title": "Night Drive", "artist": "Rue Ellis" })), "Night Drive - Rue Ellis");
    assert_eq!(cli::now_line(&serde_json::json!({ "playing": false, "title": "Night Drive", "artist": "Rue Ellis" })), "(paused) Night Drive - Rue Ellis");
    assert_eq!(cli::now_line(&serde_json::json!({ "private": true })), "Private session");
    assert_eq!(cli::now_line(&serde_json::json!({})), "Nothing playing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn command_line_pairs_and_controls_the_running_app() {
    use cli::{Action, run_with};
    let r = rig();
    let port = free_port();
    let keychain = MemoryStore::new();
    let run = |a: Action| {
        let k = &keychain;
        tokio::task::block_in_place(|| run_with(a, port, k))
    };
    // Nothing listening.
    let (code, msg) = run(Action::Command(serde_json::json!({ "command": "next" })));
    assert_eq!(code, cli::FAILED, "{msg}");
    assert!(msg.starts_with("Pair first"), "{msg}");
    let (code, _) = run(Action::Pair("123456".into()));
    assert_eq!(code, cli::NOT_RUNNING);

    let switch = DeckSwitch::default();
    switch.set(DeckConfig { enabled: false, private_session: false, automation: true });
    tokio::spawn(r.server.clone().follow(switch.subscribe(), port));
    wait_for(&r.server, |l| *l == Listening::On { port }).await;
    let (code, msg) = run(Action::Pair("000000".into()));
    assert_eq!((code, msg.as_str()), (cli::FAILED, "That code doesn't match the one in Platter's Settings."));
    let good = r.server.status().automation_code.unwrap();
    assert_eq!(run(Action::Pair(good)), (cli::OK, "Paired with Platter.".to_string()));
    assert!(keychain.get(cli::CLI_TOKEN_ACCOUNT).unwrap().is_some(), "the token is kept in the keychain");

    assert_eq!(run(Action::Command(serde_json::json!({ "command": "next" }))).0, cli::OK);
    assert_eq!(*r.host.calls.lock(), vec!["next"]);
    let (code, out) = run(Action::NowPlaying { json: true });
    assert_eq!(code, cli::OK);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["title"], "Night Drive");
    assert!(v.get("cover").is_none() && v.get("coverKey").is_none());
    assert_eq!(run(Action::NowPlaying { json: false }), (cli::OK, "Night Drive - Rue Ellis".to_string()));
    // Turning automation off closes the port: the command line says so.
    switch.set(DeckConfig { enabled: false, private_session: false, automation: false });
    wait_for(&r.server, |l| *l == Listening::Off).await;
    assert_eq!(run(Action::Command(serde_json::json!({ "command": "next" }))).0, cli::NOT_RUNNING);
}

// ---------------------------------------------------------------- more keys

/// Send a command and expect it to work.
async fn ok(r: &Rig, t: &str, body: &str) {
    let reply = r.command(t, body).await;
    assert_eq!(reply.status, 200, "{body}: {:?}", reply.body);
}

#[tokio::test]
async fn state_has_position_modes_next_and_extras() {
    let r = rig();
    {
        let mut n = r.host.now.lock();
        let n = n.as_mut().unwrap();
        n.shuffle = true;
        n.repeat = Repeat::One;
        n.context = Some((ContextKind::Playlist, Some("p1".into())));
    }
    *r.host.extras.lock() = DeckExtras {
        crossfade: true,
        miniplayer: true,
        window: false,
        device: Some("Kitchen".into()),
        stars: 4,
        can_rate: true,
        playlists: vec!["p1".into()],
    };
    r.server.set_ui(UiReport { lyrics: true });
    let t = r.paired().await;
    let b = r.send(&req("GET", "/v1/state", Some(&t), "")).await.body;
    assert_eq!(b["positionMs"], 61_000);
    assert_eq!(b["durationMs"], 200_000);
    assert_eq!(b["shuffle"], true);
    assert_eq!(b["smartShuffle"], false);
    assert_eq!(b["repeat"], "one");
    assert_eq!(b["crossfade"], true);
    assert_eq!(b["lyrics"], true);
    assert_eq!(b["miniplayer"], true);
    assert_eq!(b["window"], false);
    assert_eq!(b["device"], "Kitchen");
    assert_eq!(b["stars"], 4);
    assert_eq!(b["canRate"], true);
    assert_eq!(b["playlists"], serde_json::json!(["p1"]));
    assert_eq!(b["context"], serde_json::json!({ "kind": "playlist", "id": "p1" }));
    assert_eq!(b["next"]["title"], "Shoreline");
    assert_eq!(b["next"]["artist"], "Ana Vale");
    assert_eq!(b["next"]["coverKey"], cover_key("/library/metadata/8/thumb/1").as_str());
    assert!(b["sleep"].is_null());
}

#[tokio::test]
async fn a_private_session_hides_what_comes_next() {
    let r = rig();
    r.server.set_config(DeckConfig { enabled: true, private_session: true, automation: false });
    let t = r.paired().await;
    let b = r.send(&req("GET", "/v1/state", Some(&t), "")).await.body;
    assert!(b["next"].is_null());
    assert_eq!(r.send(&req("GET", "/v1/queue", Some(&t), "")).await.body["items"], serde_json::json!([]));
}

#[tokio::test]
async fn covers_come_by_key_at_the_size_asked() {
    let r = rig();
    let t = r.paired().await;
    // Unknown keys and the size range.
    assert_eq!(r.send(&req("GET", "/v1/cover?key=abc", Some(&t), "")).await.status, 404);
    assert_eq!(r.send(&req("GET", "/v1/cover", Some(&t), "")).await.status, 400);
    let s = r.send(&req("GET", "/v1/state?size=288", Some(&t), "")).await.body;
    assert!(r.host.calls.lock().contains(&"cover /library/metadata/9/thumb/1 288".to_owned()));
    let next = s["next"]["coverKey"].as_str().unwrap().to_owned();
    let c = r.send(&req("GET", &format!("/v1/cover?key={next}&size=9999"), Some(&t), "")).await;
    assert_eq!(c.status, 200);
    assert_eq!(c.body["key"], next.as_str());
    assert!(c.body["cover"].as_str().unwrap().starts_with("data:image/jpeg;base64,"));
    assert!(r.host.calls.lock().contains(&"cover /library/metadata/8/thumb/1 288".to_owned()), "clamped to 288");
    assert_eq!(server::cover_size(Some("size=10")), COVER_SIZE);
    assert_eq!(server::cover_size(None), COVER_SIZE);
    // Needs the token.
    assert_eq!(r.send(&req("GET", &format!("/v1/cover?key={next}"), None, "")).await.status, 401);
}

#[tokio::test]
async fn library_queue_and_devices_list() {
    let r = rig();
    let t = r.paired().await;
    let b = r.send(&req("GET", "/v1/library?kind=playlists", Some(&t), "")).await.body;
    assert_eq!(b["kind"], "playlists");
    assert_eq!(b["items"][0]["id"], "p1");
    assert_eq!(b["items"][0]["title"], "Playlists");
    assert_eq!(b["items"][0]["subtitle"], "3 songs");
    let key = b["items"][0]["coverKey"].as_str().unwrap().to_owned();
    assert!(b["items"][0].get("thumb").is_none(), "art paths stay inside");
    assert_eq!(r.send(&req("GET", &format!("/v1/cover?key={key}"), Some(&t), "")).await.status, 200);
    assert_eq!(r.send(&req("GET", "/v1/library?kind=songs", Some(&t), "")).await.status, 400);
    for k in ["albums", "artists", "mixes"] {
        assert_eq!(r.send(&req("GET", &format!("/v1/library?kind={k}"), Some(&t), "")).await.status, 200, "{k}");
    }
    let q = r.send(&req("GET", "/v1/queue", Some(&t), "")).await.body;
    assert_eq!(q["items"][0], serde_json::json!({ "itemId": "i1", "title": "Shoreline", "artist": "Ana Vale", "coverKey": cover_key("/library/metadata/8/thumb/1") }));
    *r.host.devices.lock() = vec![DeckDevice { id: LOCAL_DEVICE.into(), name: "This computer".into(), active: true, needs_window: false }];
    let d = r.send(&req("GET", "/v1/devices", Some(&t), "")).await.body;
    assert_eq!(d["items"][0], serde_json::json!({ "id": "local", "name": "This computer", "active": true, "needsWindow": false }));
    assert_eq!(r.send(&req("POST", "/v1/queue", Some(&t), "")).await.status, 405);
    assert_eq!(r.send(&req("GET", "/v1/devices", None, "")).await.status, 401);
}

#[tokio::test]
async fn seek_shuffle_repeat_and_mute() {
    let r = rig();
    let t = r.paired().await;
    ok(&r, &t, r#"{"command":"seekBy","value":-10}"#).await;
    ok(&r, &t, r#"{"command":"seekBy","value":-100}"#).await;
    ok(&r, &t, r#"{"command":"seekBy","value":1000}"#).await;
    ok(&r, &t, r#"{"command":"shuffle"}"#).await;
    ok(&r, &t, r#"{"command":"shuffle","smart":true}"#).await;
    ok(&r, &t, r#"{"command":"repeat"}"#).await;
    ok(&r, &t, r#"{"command":"mute"}"#).await;
    r.host.now.lock().as_mut().unwrap().repeat = Repeat::One;
    ok(&r, &t, r#"{"command":"repeat"}"#).await;
    assert_eq!(
        *r.host.calls.lock(),
        vec!["seek 51000", "seek 0", "seek 199500", "shuffle true", "smart true", "repeat All", "mute true", "repeat Off"]
    );
    assert_eq!(server::next_repeat(Repeat::All), Repeat::One);
    *r.host.now.lock() = None;
    assert_eq!(r.command(&t, r#"{"command":"seekBy","value":10}"#).await.status, 409);
}

#[tokio::test]
async fn crossfade_and_private_session_change_settings() {
    let r = rig();
    let t = r.paired().await;
    ok(&r, &t, r#"{"command":"crossfade"}"#).await;
    r.host.extras.lock().crossfade = true;
    ok(&r, &t, r#"{"command":"crossfade"}"#).await;
    ok(&r, &t, r#"{"command":"privateSession"}"#).await;
    assert_eq!(
        *r.host.calls.lock(),
        vec![r#"settings {"crossfade":"smart"}"#, r#"settings {"crossfade":"off"}"#, r#"settings {"privateSession":true}"#]
    );
}

#[tokio::test]
async fn playlists_add_ask_and_remove() {
    let r = rig();
    let t = r.paired().await;
    ok(&r, &t, r#"{"command":"addToPlaylist","playlist":"p2"}"#).await;
    ok(&r, &t, r#"{"command":"addToPlaylist"}"#).await;
    r.host.extras.lock().playlists = vec!["p2".into()];
    let dup = r.command(&t, r#"{"command":"addToPlaylist","playlist":"p2"}"#).await;
    assert_eq!(dup.status, 409);
    assert!(dup.body["error"].as_str().unwrap().contains("already"));
    ok(&r, &t, r#"{"command":"removeFromPlaylist","playlist":"p2"}"#).await;
    assert_eq!(r.command(&t, r#"{"command":"removeFromPlaylist","playlist":"p9"}"#).await.status, 409);
    assert_eq!(
        *r.host.calls.lock(),
        vec!["add p2 t1", r#"action {"kind":"addToPlaylist","trackIds":["t1"]} true"#, "remove p2 t1"]
    );
}

#[tokio::test]
async fn rating_needs_star_ratings_and_the_same_rating_clears() {
    let r = rig();
    let t = r.paired().await;
    let no = r.command(&t, r#"{"command":"rate","stars":3}"#).await;
    assert_eq!(no.status, 409);
    r.host.extras.lock().can_rate = true;
    ok(&r, &t, r#"{"command":"rate","stars":3}"#).await;
    r.host.extras.lock().stars = 3;
    ok(&r, &t, r#"{"command":"rate","stars":3}"#).await;
    assert_eq!(r.command(&t, r#"{"command":"rate","stars":6}"#).await.status, 409);
    assert_eq!(*r.host.calls.lock(), vec!["rate t1 Some(3)", "rate t1 None"]);
}

#[tokio::test]
async fn play_things() {
    let r = rig();
    let t = r.paired().await;
    ok(&r, &t, r#"{"command":"playItem","kind":"album","id":"5","shuffle":true}"#).await;
    ok(&r, &t, r#"{"command":"playItem","kind":"mix","id":"m1"}"#).await;
    ok(&r, &t, r#"{"command":"playItem","kind":"playlist","id":"p1","queue":true}"#).await;
    ok(&r, &t, r#"{"command":"playItem","kind":"album","id":"5","start":12.5}"#).await;
    ok(&r, &t, r#"{"command":"playLink","link":"platter://album/5"}"#).await;
    ok(&r, &t, r#"{"command":"playLink","link":"platter://track/7?t=30"}"#).await;
    ok(&r, &t, r#"{"command":"playSearch","query":" night "}"#).await;
    ok(&r, &t, r#"{"command":"radio"}"#).await;
    ok(&r, &t, r#"{"command":"playShortcut","name":"liked"}"#).await;
    ok(&r, &t, r#"{"command":"playShortcut","name":"daylist"}"#).await;
    assert_eq!(
        *r.host.calls.lock(),
        vec![
            r#"play Album Some("5") None None 0 true"#,
            r#"play Mix Some("m1") None None 0 false"#,
            r#"queue Playlist Some("p1")"#,
            r#"play Album Some("5") None None 12500 false"#,
            r#"play Album Some("5") None None 0 false"#,
            r#"play Album Some("5") None Some("7") 30000 false"#,
            r#"play Search Some("night") Some(["s1", "s2"]) None 0 false"#,
            r#"play Radio Some("track:t1") None None 0 false"#,
            r#"play Liked Some("liked") None None 0 false"#,
            r#"play Mix Some("mix-day") None None 0 false"#,
        ]
    );
    for bad in [
        r#"{"command":"playItem","kind":"album","id":" "}"#,
        r#"{"command":"playLink","link":"platter://album/404"}"#,
        r#"{"command":"playSearch","query":"nothing"}"#,
        r#"{"command":"playShortcut","name":"discover"}"#,
        r#"{"command":"playShortcut","name":"weekly"}"#,
    ] {
        assert_eq!(r.command(&t, bad).await.status, 409, "{bad}");
    }
    assert_eq!(r.command(&t, r#"{"command":"playItem","kind":"song","id":"1"}"#).await.status, 400);
}

#[tokio::test]
async fn queue_devices_and_the_window() {
    let r = rig();
    let t = r.paired().await;
    *r.host.devices.lock() = vec![
        DeckDevice { id: LOCAL_DEVICE.into(), name: "This computer".into(), active: true, needs_window: false },
        DeckDevice { id: "d1".into(), name: "Kitchen".into(), active: false, needs_window: false },
        DeckDevice { id: "d2".into(), name: "Den".into(), active: false, needs_window: true },
    ];
    ok(&r, &t, r#"{"command":"queueJump","itemId":"i1"}"#).await;
    ok(&r, &t, r#"{"command":"listenOn","device":"d1"}"#).await;
    ok(&r, &t, r#"{"command":"listenOn","device":"local"}"#).await;
    ok(&r, &t, r#"{"command":"listenOn","device":"d2"}"#).await;
    ok(&r, &t, r#"{"command":"listenOn"}"#).await;
    assert_eq!(r.command(&t, r#"{"command":"listenOn","device":"gone"}"#).await.status, 409);
    ok(&r, &t, r#"{"command":"lyrics"}"#).await;
    r.server.set_ui(UiReport { lyrics: true });
    ok(&r, &t, r#"{"command":"lyrics"}"#).await;
    ok(&r, &t, r#"{"command":"miniplayer"}"#).await;
    ok(&r, &t, r#"{"command":"window"}"#).await;
    ok(&r, &t, r#"{"command":"nowPlaying"}"#).await;
    ok(&r, &t, r#"{"command":"queue"}"#).await;
    ok(&r, &t, r#"{"command":"bookmark"}"#).await;
    assert_eq!(
        *r.host.calls.lock(),
        vec![
            "jump i1",
            "listen on d1",
            r#"action {"device":"d2","kind":"listenOn"} true"#,
            r#"action {"kind":"listenOn"} true"#,
            r#"action {"kind":"lyrics","open":true} true"#,
            r#"action {"kind":"lyrics","open":false} false"#,
            "miniplayer true",
            "window true",
            r#"action {"kind":"nowPlaying"} true"#,
            r#"action {"kind":"queue"} true"#,
            "bookmark t1 61000",
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn the_sleep_timer_pauses_after_its_minutes() {
    let r = rig();
    let t = r.paired().await;
    ok(&r, &t, r#"{"command":"sleep","minutes":5}"#).await;
    let s = r.send(&req("GET", "/v1/state", Some(&t), "")).await.body;
    assert_eq!(s["sleep"], serde_json::json!({ "remainingMs": 300_000, "endOfSong": false }));
    tokio::time::sleep(Duration::from_secs(60)).await;
    let s = r.send(&req("GET", "/v1/state", Some(&t), "")).await.body;
    assert_eq!(s["sleep"]["remainingMs"], 240_000);
    assert!(!r.host.calls.lock().iter().any(|c| c == "pause"));
    tokio::time::sleep(Duration::from_secs(241)).await;
    assert!(r.host.calls.lock().iter().any(|c| c == "pause"));
    assert!(r.send(&req("GET", "/v1/state", Some(&t), "")).await.body["sleep"].is_null());
    // Bad minutes.
    assert_eq!(r.command(&t, r#"{"command":"sleep","minutes":0}"#).await.status, 409);
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_or_replaced_sleep_timer_does_nothing() {
    let r = rig();
    let t = r.paired().await;
    ok(&r, &t, r#"{"command":"sleep","minutes":1}"#).await;
    ok(&r, &t, r#"{"command":"sleepCancel"}"#).await;
    ok(&r, &t, r#"{"command":"sleep","minutes":10}"#).await;
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(!r.host.calls.lock().iter().any(|c| c == "pause"), "the first timer was cancelled");
    ok(&r, &t, r#"{"command":"sleepCancel"}"#).await;
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert!(!r.host.calls.lock().iter().any(|c| c == "pause"));
}

#[tokio::test(start_paused = true)]
async fn an_end_of_song_timer_stops_the_next_song() {
    let r = rig();
    let t = r.paired().await;
    ok(&r, &t, r#"{"command":"sleep","endOfSong":true}"#).await;
    let s = r.send(&req("GET", "/v1/state", Some(&t), "")).await.body;
    assert_eq!(s["sleep"], serde_json::json!({ "remainingMs": null, "endOfSong": true }));
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(r.host.calls.lock().is_empty() || !r.host.calls.lock().iter().any(|c| c == "pause"));
    // The next song starts.
    {
        let mut n = r.host.now.lock();
        let n = n.as_mut().unwrap();
        n.track = n.next.take();
        n.position_ms = 50;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let calls = r.host.calls.lock().clone();
    assert!(calls.ends_with(&["pause".to_owned(), "seek 0".to_owned()]), "{calls:?}");
    assert!(r.send(&req("GET", "/v1/state", Some(&t), "")).await.body["sleep"].is_null());
    // Nothing playing: nothing to finish.
    *r.host.now.lock() = None;
    assert_eq!(r.command(&t, r#"{"command":"sleep","endOfSong":true}"#).await.status, 409);
}

#[test]
fn library_kinds_read_singular_and_plural() {
    assert_eq!(LibraryKind::parse("album"), Some(LibraryKind::Albums));
    assert_eq!(LibraryKind::parse("mixes"), Some(LibraryKind::Mixes));
    assert_eq!(LibraryKind::parse("songs"), None);
    assert_eq!(LibraryKind::Artists.context(), ContextKind::Artist);
}
