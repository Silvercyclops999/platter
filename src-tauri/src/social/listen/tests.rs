//! Listen along: sealing, replay, invites, following, and whole sessions
//! between in-process instances through a fake ntfy server on this
//! computer. Nothing here reaches the real ntfy.sh.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;

use super::fake_ntfy::FakeNtfy;
use super::invite::{self, Token};
use super::relay::{self, Ntfy, RelayEvent, RelayHealth};
use super::seal::{self, Channel, Envelope, Rejected, ReplayGuard, Sender};
use super::matching::{Cand, Found};
use super::wire::{Msg, Op, Person, Queued, Song, State};
use super::*;
use crate::error::AppResult;
use crate::plex::state::EventSink;

// ---- fakes ----------------------------------------------------------------------

#[derive(Default)]
struct Events(Mutex<Vec<(String, serde_json::Value)>>);

impl EventSink for Events {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().push((event.to_string(), payload));
    }
}

impl Events {
    fn last(&self, name: &str) -> Option<serde_json::Value> {
        self.0.lock().iter().rev().find(|(e, _)| e == name).map(|(_, v)| v.clone())
    }
}

/// A player: the song, the rest of what's playing, and the user queue.
#[derive(Default)]
struct P {
    track: Option<String>,
    pos: u64,
    playing: bool,
    rest: Vec<String>,
    user: Vec<String>,
}

#[derive(Default)]
struct FakePlayer {
    p: Mutex<P>,
    log: Mutex<Vec<String>>,
}

impl FakePlayer {
    fn set(&self, track: &str, pos: u64, playing: bool, rest: &[&str]) {
        let mut p = self.p.lock();
        p.track = Some(track.into());
        p.pos = pos;
        p.playing = playing;
        p.rest = rest.iter().map(|s| s.to_string()).collect();
    }
    fn track(&self) -> Option<String> {
        self.p.lock().track.clone()
    }
    fn playing(&self) -> bool {
        self.p.lock().playing
    }
    fn user(&self) -> Vec<String> {
        self.p.lock().user.clone()
    }
}

#[async_trait]
impl ListenPlayer for FakePlayer {
    async fn now(&self) -> AppResult<PlayerNow> {
        let p = self.p.lock();
        let up_next: Vec<String> = p.user.iter().chain(&p.rest).cloned().collect();
        Ok(PlayerNow {
            track_id: p.track.clone(),
            position_ms: p.pos,
            duration_ms: 200_000,
            playing: p.playing,
            user_queue: p.user.clone(),
            up_next_items: (0..up_next.len()).map(|i| format!("i{i}")).collect(),
            up_next,
        })
    }
    async fn play(&self, ids: Vec<String>, position_ms: u64, play: bool) -> AppResult<()> {
        let mut p = self.p.lock();
        p.track = ids.first().cloned();
        p.rest = ids[1..].to_vec();
        p.pos = position_ms;
        p.playing = play;
        self.log.lock().push(format!("play {} at {position_ms}", ids[0]));
        Ok(())
    }
    async fn seek(&self, position_ms: u64) -> AppResult<()> {
        self.p.lock().pos = position_ms;
        self.log.lock().push(format!("seek {position_ms}"));
        Ok(())
    }
    async fn pause(&self) -> AppResult<()> {
        self.p.lock().playing = false;
        self.log.lock().push("pause".into());
        Ok(())
    }
    async fn resume(&self) -> AppResult<()> {
        self.p.lock().playing = true;
        self.log.lock().push("resume".into());
        Ok(())
    }
    async fn next(&self) -> AppResult<()> {
        let mut p = self.p.lock();
        let next = if p.user.is_empty() { (!p.rest.is_empty()).then(|| p.rest.remove(0)) } else { Some(p.user.remove(0)) };
        p.track = next;
        p.pos = 0;
        self.log.lock().push("next".into());
        Ok(())
    }
    async fn previous(&self) -> AppResult<()> {
        self.p.lock().pos = 0;
        self.log.lock().push("previous".into());
        Ok(())
    }
    async fn queue_add(&self, ids: Vec<String>) -> AppResult<()> {
        self.p.lock().user.extend(ids.iter().cloned());
        self.log.lock().push(format!("add {}", ids.join(",")));
        Ok(())
    }
    async fn queue_clear(&self) -> AppResult<()> {
        self.p.lock().user.clear();
        Ok(())
    }
    async fn queue_move(&self, item_id: String, to_index: usize) -> AppResult<()> {
        let mut p = self.p.lock();
        let i: usize = item_id.trim_start_matches('i').parse().unwrap();
        let k = if i < p.user.len() {
            p.user.remove(i)
        } else {
            let j = i - p.user.len();
            p.rest.remove(j)
        };
        let at = to_index.min(p.user.len());
        p.user.insert(at, k);
        self.log.lock().push(format!("move {i} {to_index}"));
        Ok(())
    }
}

/// A library: the ids it has (None: every id the host plays), who the
/// host's songs are, and songs to match against.
#[derive(Default)]
struct FakeLib {
    have: Mutex<Option<Vec<String>>>,
    songs: Mutex<std::collections::HashMap<String, Song>>,
    here: Mutex<Vec<Cand>>,
}

impl FakeLib {
    fn has_only(&self, ids: &[&str]) {
        *self.have.lock() = Some(ids.iter().map(|s| s.to_string()).collect());
    }
    fn song(&self, id: &str, s: Song) {
        self.songs.lock().insert(id.into(), s);
    }
    fn copy(&self, id: &str, title: &str, artist: &str, ms: u64, mbid: Option<&str>) {
        self.here.lock().push(Cand { id: id.into(), title: title.into(), artist: artist.into(), track_artist: None, duration_ms: ms, mbid: mbid.map(str::to_string) });
    }
}

#[async_trait]
impl ListenLibrary for FakeLib {
    async fn songs(&self, ids: Vec<String>) -> std::collections::HashMap<String, Song> {
        let all = self.songs.lock();
        ids.into_iter().filter_map(|k| all.get(&k).cloned().map(|s| (k, s))).collect()
    }
    async fn has(&self, id: &str) -> bool {
        self.have.lock().as_ref().is_none_or(|h| h.iter().any(|x| x == id))
    }
    async fn find(&self, song: &Song) -> Option<Found> {
        matching::best(song, &self.here.lock())
    }
}

/// Plex sharing: what the host's invites did, and the guest's answers.
struct FakeOffer {
    on: std::sync::atomic::AtomicBool,
    me: Mutex<Option<String>>,
    outcome: Mutex<OfferOutcome>,
    invited: Mutex<Vec<String>>,
    shared: Mutex<Vec<String>>,
}

impl Default for FakeOffer {
    fn default() -> Self {
        Self {
            on: std::sync::atomic::AtomicBool::new(true),
            me: Mutex::new(None),
            outcome: Mutex::new(OfferOutcome::Invited),
            invited: Mutex::default(),
            shared: Mutex::default(),
        }
    }
}

#[async_trait]
impl ServerOffer for FakeOffer {
    async fn offered(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }
    async fn invite(&self, plex_user: &str) -> OfferOutcome {
        self.invited.lock().push(plex_user.to_string());
        self.outcome.lock().clone()
    }
    async fn me(&self) -> Option<String> {
        self.me.lock().clone()
    }
    async fn shared(&self, host: &str) {
        self.shared.lock().push(host.to_string());
    }
}

struct FakeServer(Mutex<Option<String>>);

#[async_trait]
impl ServerId for FakeServer {
    async fn machine_id(&self) -> Option<String> {
        self.0.lock().clone()
    }
}

/// What the leak check looks for. None of it may reach the relay or an invite.
const MACHINE_ID: &str = "5e1f0a9bc77d4e21a8f3b6c2d9e0f1a2b3c4d5e6"; // gitleaks:allow (made-up test value)
const PLEX_TOKEN: &str = "plex-account-token-for-tests"; // gitleaks:allow (made-up test value)
const SERVER_URL: &str = "https://192-0-2-10.abcdef.plex.direct:32400";

struct Peer {
    listen: ListenAlong,
    player: Arc<FakePlayer>,
    events: Arc<Events>,
    settings: ListenSettings,
    server: Arc<FakeServer>,
    lib: Arc<FakeLib>,
    offer: Arc<FakeOffer>,
    plain: Arc<Mutex<Vec<String>>>,
}

fn cfg() -> ListenConfig {
    ListenConfig {
        watch: Duration::from_millis(20),
        heartbeat: Duration::from_millis(400),
        grace: Duration::from_millis(200),
        guest_ping: Duration::from_millis(300),
        guest_timeout: Duration::from_secs(5),
        answer_timeout: Duration::from_millis(1500),
        retry: Duration::from_millis(50),
        retry_max: Duration::from_millis(200),
        hello_every: Duration::ZERO,
    }
}

fn peer(name: &str, relay: &FakeNtfy) -> Peer {
    let player = Arc::new(FakePlayer::default());
    let events = Arc::new(Events::default());
    let settings = ListenSettings::default();
    let server = Arc::new(FakeServer(Mutex::new(Some(MACHINE_ID.into()))));
    let lib = Arc::new(FakeLib::default());
    let offer = Arc::new(FakeOffer::default());
    let plain: Arc<Mutex<Vec<String>>> = Arc::default();
    let name = name.to_string();
    let tap = plain.clone();
    let listen = ListenAlong::new(ListenDeps {
        player: player.clone(),
        relays: Arc::new(Ntfy::new().unwrap()),
        events: events.clone(),
        server: server.clone(),
        library: lib.clone(),
        offer: offer.clone(),
        name: Arc::new(move || name.clone()),
        settings: settings.clone(),
        default_relay: relay.url.clone(),
        cfg: cfg(),
        tap: Some(Arc::new(move |s: &str| tap.lock().push(s.to_string()))),
    });
    Peer { listen, player, events, settings, server, lib, offer, plain }
}

async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    // A hang guard: it returns as soon as `f` holds.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}

fn status(p: &Peer) -> Option<SessionStatus> {
    p.listen.view().status
}

/// A host playing "a" at 30 s with "b" next, and its first invite.
async fn hosting(relay: &FakeNtfy) -> (Peer, String) {
    let h = peer("Sam", relay);
    h.player.set("a", 30_000, true, &["b"]);
    let v = h.listen.start().await.unwrap();
    (h, v.invite.unwrap().link)
}

async fn joined(relay: &FakeNtfy, name: &str, link: &str) -> Peer {
    let g = peer(name, relay);
    g.listen.join(link).await.unwrap();
    eventually("guest live", || status(&g) == Some(SessionStatus::Live)).await;
    g
}

// ---- sealing ------------------------------------------------------------------

fn env(m: Msg) -> Envelope {
    Envelope { r: Sender::Host, f: "host".into(), c: 1, x: now_ms() + 60_000, m }
}

#[test]
fn sealed_messages_round_trip_and_changes_are_refused() {
    let ch = Channel::new(b"a session secret", "broadcast");
    assert_eq!(ch.topic.len(), 40);
    assert!(ch.topic.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(format!("{ch:?}"), "Channel(..)", "topics and keys never reach a log");
    let e = env(Msg::Ping);
    let text = seal::seal(&ch, &e, &[]).unwrap();
    assert!(text.starts_with("1.") && !text.contains("Ping"));
    assert_eq!(seal::open(&ch, &text, now_ms(), None).unwrap(), e);
    // Each seal is different (fresh nonce).
    assert_ne!(seal::seal(&ch, &e, &[]).unwrap(), text);

    // A changed byte.
    let mut b = text.clone().into_bytes();
    let i = b.len() / 2;
    b[i] = if b[i] == b'A' { b'B' } else { b'A' };
    assert!(seal::open(&ch, std::str::from_utf8(&b).unwrap(), now_ms(), None).is_err());
    // Another channel of the same secret, another secret, garbage.
    assert_eq!(seal::open(&Channel::new(b"a session secret", "guest up"), &text, now_ms(), None), Err(Rejected::BadSeal));
    assert_eq!(seal::open(&Channel::new(b"another secret", "broadcast"), &text, now_ms(), None), Err(Rejected::BadSeal));
    assert_eq!(seal::open(&ch, "2.abc", now_ms(), None), Err(Rejected::Malformed));
    assert_eq!(seal::open(&ch, "nonsense", now_ms(), None), Err(Rejected::Malformed));
}

#[test]
fn broadcasts_only_open_for_guests_with_a_tag() {
    let ch = Channel::new(b"bcast", "broadcast");
    let (k1, k2, k3) = ([1u8; 32], [2u8; 32], [3u8; 32]);
    let text = seal::seal(&ch, &env(Msg::Ended), &[k1, k2]).unwrap();
    assert!(seal::open(&ch, &text, now_ms(), Some(&k1)).is_ok());
    assert!(seal::open(&ch, &text, now_ms(), Some(&k2)).is_ok());
    // A removed guest (or one forging a host message with the shared key).
    assert_eq!(seal::open(&ch, &text, now_ms(), Some(&k3)), Err(Rejected::Forged));
    let untagged = text.rsplit_once('.').unwrap().0;
    assert_eq!(seal::open(&ch, untagged, now_ms(), Some(&k1)), Err(Rejected::Forged));
    let forged = seal::seal(&ch, &env(Msg::Ended), &[k3]).unwrap();
    assert_eq!(seal::open(&ch, &forged, now_ms(), Some(&k1)), Err(Rejected::Forged));
}

#[test]
fn replays_and_expired_messages_are_refused() {
    let mut g = ReplayGuard::default();
    assert!(g.accept("x", 1).is_ok());
    assert!(g.accept("x", 5).is_ok());
    assert_eq!(g.accept("x", 5), Err(Rejected::Replayed));
    assert_eq!(g.accept("x", 2), Err(Rejected::Replayed));
    assert!(g.accept("y", 2).is_ok(), "counters are per sender");
    g.forget("x");
    assert!(g.accept("x", 1).is_ok());

    let ch = Channel::new(b"s", "c");
    let now = now_ms();
    let old = Envelope { x: now - 1, ..env(Msg::Ping) };
    assert_eq!(seal::open(&ch, &seal::seal(&ch, &old, &[]).unwrap(), now, None), Err(Rejected::Expired));
    let far = Envelope { x: now + 60 * 60_000, ..env(Msg::Ping) };
    assert_eq!(seal::open(&ch, &seal::seal(&ch, &far, &[]).unwrap(), now, None), Err(Rejected::Expired));
}

#[test]
fn only_the_two_sides_of_a_join_agree_a_key() {
    let secret = b"session-and-invite-secret";
    let (guest, msg_a) = seal::pake_guest_start(secret);
    let (msg_b, host_key) = seal::pake_host(secret, &msg_a).unwrap();
    assert_eq!(seal::pake_guest_finish(guest, &msg_b).unwrap(), host_key);
    // Someone else holding the invite, answering the same host message.
    let (other, _) = seal::pake_guest_start(secret);
    assert_ne!(seal::pake_guest_finish(other, &msg_b).unwrap(), host_key);
    // A wrong secret doesn't agree either.
    let (wrong, msg_a) = seal::pake_guest_start(b"a guess");
    let (msg_b, host_key) = seal::pake_host(secret, &msg_a).unwrap();
    assert_ne!(seal::pake_guest_finish(wrong, &msg_b).unwrap(), host_key);
    // The sealed welcome only opens with the agreed key.
    let sealed = seal::seal_with(&host_key, "aad", b"hello").unwrap();
    assert_eq!(seal::open_with(&host_key, "aad", &sealed).unwrap(), b"hello");
    assert!(seal::open_with(&[9; 32], "aad", &sealed).is_err());
    assert!(seal::open_with(&host_key, "other", &sealed).is_err());
}

#[test]
fn a_full_state_still_fits_the_relay_limit() {
    let mut st = State {
        srv: "0".repeat(24),
        t: Some("1234567890".into()),
        ts: None,
        tby: None,
        p: 123_456,
        d: 456_789,
        pl: true,
        q: (0..UP_NEXT).map(|i| Queued { k: format!("{i:064}"), by: Some("f".repeat(16)), s: None }).collect(),
        ctl: true,
        ppl: (0..=MAX_GUESTS).map(|i| Person { id: format!("{i:016}"), name: "N".repeat(NAME_MAX), av: None }).collect(),
    };
    fit(&mut st, MAX_GUESTS);
    assert!(!st.q.is_empty() && st.q.len() < UP_NEXT);
    let tags = vec![[7u8; 32]; MAX_GUESTS];
    let e = Envelope { r: Sender::Host, f: "host".into(), c: u64::MAX, x: u64::MAX / 2, m: Msg::State(st) };
    let text = seal::seal(&Channel::new(b"s", "broadcast"), &e, &tags).unwrap();
    assert!(text.len() <= seal::MAX_BODY, "{} bytes", text.len());
}

#[test]
fn ntfy_stream_lines_are_read() {
    assert_eq!(relay::parse_line(r#"{"id":"x","time":1,"event":"open","topic":"a"}"#), Some(RelayEvent::Open));
    assert_eq!(relay::parse_line(r#"{"id":"x","time":1,"event":"keepalive","topic":"a"}"#), None);
    assert_eq!(
        relay::parse_line(r#"{"id":"m1","time":1,"event":"message","topic":"t","message":"1.abc"}"#),
        Some(RelayEvent::Message { id: "m1".into(), topic: "t".into(), body: "1.abc".into() })
    );
    assert_eq!(relay::parse_line("not json"), None);
}

// ---- invites ------------------------------------------------------------------

#[test]
fn invite_links_carry_only_random_tokens() {
    let t = Token { session: [7; 16], secret: [9; 16], expires: 1_900_000_000, relay: None };
    let link = t.link();
    assert!(link.starts_with("platter://join/"));
    assert_eq!(link.matches(':').count(), 1);
    assert!(!link.contains('?') && !link.contains('&'));
    assert_eq!(invite::parse(&link), Some(t.clone()));
    assert_eq!(format!("{t:?}"), "Token(..)");
    // A self-hosted relay goes in the token, and comes back out.
    let own = Token { relay: Some("https://ntfy.example.org/base".into()), ..t.clone() };
    assert_eq!(invite::parse(&own.link()), Some(own));
    for bad in [
        // The old kind, with an address in it.
        "platter://join/0123456789abcdef?k=00112233445566778899aabbccddeeff&h=10.0.0.5%3A32520",
        "platter://join/",
        "platter://join/AAAA",
        "https://join/AQ",
        "platter://album/AQ",
        &format!("{link}?h=1"),
        &link.replace("join/A", "join/B"),
    ] {
        assert_eq!(invite::parse(bad), None, "{bad}");
    }
    assert_eq!(invite::relay_url(" https://ntfy.example.org/ ").as_deref(), Some("https://ntfy.example.org"));
    for bad in ["ntfy.example.org", "ftp://x.org", "https://a:b@x.org", "https://x.org/?a=1", "https://x.org/#f", ""] {
        assert_eq!(invite::relay_url(bad), None, "{bad}");
    }
}

// ---- following ------------------------------------------------------------------

fn st(track: Option<&str>, pos: u64, playing: bool) -> State {
    State {
        srv: "s".into(),
        t: track.map(str::to_string),
        ts: None,
        tby: None,
        p: pos,
        d: 200_000,
        pl: playing,
        q: vec![Queued { k: "b".into(), by: None, s: None }, Queued { k: "c".into(), by: Some("g1".into()), s: None }],
        ctl: true,
        ppl: Vec::new(),
    }
}

fn local(track: Option<&str>, pos: u64, playing: bool, user: &[&str]) -> PlayerNow {
    let user: Vec<String> = user.iter().map(|s| s.to_string()).collect();
    PlayerNow { track_id: track.map(str::to_string), position_ms: pos, duration_ms: 200_000, playing, up_next: user.clone(), up_next_items: Vec::new(), user_queue: user }
}

#[test]
fn guests_follow_the_host_with_latency_compensation() {
    let q = vec!["b".to_string(), "c".to_string()];
    // Another song: load the host's at the spot, then its up next as the queue.
    assert_eq!(
        plan(&local(Some("x"), 0, true, &[]), &st(Some("a"), 10_000, true), 400),
        vec![Step::Load { id: "a".into(), position_ms: 10_200, play: true }, Step::SetQueue(q.clone())]
    );
    // Paused: no compensation.
    assert_eq!(target_ms(&st(Some("a"), 10_000, false), 400), 10_000);
    // Close enough, same queue: nothing.
    assert!(plan(&local(Some("a"), 10_900, true, &["b", "c"]), &st(Some("a"), 10_000, true), 400).is_empty());
    // The queue changed: just the queue.
    assert_eq!(plan(&local(Some("a"), 10_000, true, &["b"]), &st(Some("a"), 10_000, true), 0), vec![Step::SetQueue(q)]);
    // Drifted: seek.
    assert_eq!(plan(&local(Some("a"), 20_000, true, &["b", "c"]), &st(Some("a"), 10_000, true), 0), vec![Step::Seek(10_000)]);
    // Pauses and plays follow.
    assert_eq!(plan(&local(Some("a"), 10_000, true, &["b", "c"]), &st(Some("a"), 10_000, false), 0), vec![Step::Pause]);
    assert_eq!(plan(&local(Some("a"), 10_000, false, &["b", "c"]), &st(Some("a"), 10_000, true), 0), vec![Step::Resume]);
    // Nothing playing at the host.
    assert_eq!(plan(&local(Some("a"), 0, true, &[]), &st(None, 0, false), 0), vec![Step::Pause]);
    // The guest went on to the next song just before the host: wait.
    assert!(plan(&local(Some("b"), 100, true, &["c"]), &st(Some("a"), 199_000, true), 200).is_empty());
    // Never past the end.
    assert_eq!(target_ms(&st(Some("a"), 199_900, true), 1000), 200_000);
}

// ---- sessions through the relay ---------------------------------------------------

#[tokio::test]
async fn a_session_from_start_to_end_through_the_relay() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    assert_eq!(h.listen.subscribe_party().borrow().as_ref().map(|p| (p.size, p.max)), Some((1, 11)));
    let g = joined(&relay, "Alex", &link).await;

    // The guest plays the host's song at about its spot, with what's next queued.
    assert_eq!(g.player.track().as_deref(), Some("a"));
    assert!(g.player.log.lock()[0].starts_with("play a at 30"), "{:?}", g.player.log.lock());
    assert_eq!(g.player.user(), vec!["b".to_string()]);
    let v = g.listen.view();
    assert_eq!((v.role, v.host_name.as_deref(), v.relay), (Role::Guest, Some("Sam"), RelayHealth::Ok));
    assert_eq!(v.people.iter().map(|p| (p.name.as_str(), p.host, p.me)).collect::<Vec<_>>(), [("Sam", true, false), ("Alex", false, true)]);
    // The host sees who's listening.
    eventually("host sees Alex", || h.listen.view().listeners == ["Alex"]).await;
    assert_eq!(h.events.last(EVENT).unwrap()["people"][1]["name"], "Alex");
    assert_eq!(h.listen.subscribe_party().borrow().as_ref().unwrap().size, 2);

    // The host pauses, then skips: the guest follows.
    h.player.p.lock().playing = false;
    eventually("guest pauses", || !g.player.playing()).await;
    h.player.set("b", 0, true, &[]);
    eventually("guest skips", || g.player.track().as_deref() == Some("b")).await;
    assert!(g.player.playing());

    // The guest changes what's playing for everyone.
    g.listen.control(Op::Pause).await.unwrap();
    eventually("host paused by the guest", || !h.player.playing()).await;
    g.listen.control(Op::Play).await.unwrap();
    eventually("host playing again", || h.player.playing()).await;
    g.listen.control(Op::Seek { ms: 90_000 }).await.unwrap();
    eventually("host seeked", || h.player.p.lock().pos == 90_000).await;

    // The guest adds songs: they're queued at the host with the guest's name on them.
    g.listen.add(vec!["z1".into(), "z2".into()]).await.unwrap();
    eventually("host queued them", || h.player.user() == ["z1", "z2"]).await;
    let gid = g.listen.view().people.iter().find(|p| p.me).unwrap().id.clone();
    eventually("added by shows on the host", || h.listen.view().queue.first().and_then(|q| q.by.clone()) == Some(gid.clone())).await;
    eventually("and on the guest", || g.listen.view().queue.first().and_then(|q| q.by.clone()) == Some(gid.clone())).await;
    eventually("guest queue follows", || g.player.user() == ["z1", "z2"]).await;
    // And reorders them.
    g.listen.control(Op::Move { from: 1, to: 0 }).await.unwrap();
    eventually("host reordered", || h.player.user() == ["z2", "z1"]).await;
    eventually("guest reordered", || g.player.user() == ["z2", "z1"]).await;
    // Next plays the guest's song for everyone.
    g.listen.control(Op::Next).await.unwrap();
    eventually("host on z2", || h.player.track().as_deref() == Some("z2")).await;
    eventually("guest on z2", || g.player.track().as_deref() == Some("z2")).await;
    eventually("the song playing says who added it", || g.listen.view().now_by == Some(gid.clone())).await;

    // "Let others change what's playing" off: guests may only add.
    h.settings.guests_control.store(false, Ordering::SeqCst);
    eventually("guest sees add-only", || !g.listen.view().guests_can_control).await;
    let e = g.listen.control(Op::Pause).await.unwrap_err();
    assert!(e.to_string().contains("Sam only lets others add songs"), "{e}");
    g.listen.add(vec!["z3".into()]).await.unwrap();
    eventually("adding still works", || h.player.user().contains(&"z3".to_string())).await;
    assert!(g.listen.add(vec!["../x".into()]).await.is_err());
    assert!(h.player.playing(), "nothing paused the host");

    // The host ends it: the guest stops following.
    h.listen.stop().await.unwrap();
    assert_eq!(h.listen.view().role, Role::None);
    assert_eq!(*h.listen.subscribe_party().borrow(), None);
    eventually("guest sees the end", || status(&g) == Some(SessionStatus::Ended)).await;
    assert_eq!(g.events.last(EVENT).unwrap()["status"], "ended");
    let before = g.player.log.lock().len();
    h.player.set("q", 0, true, &[]);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(g.player.log.lock().len(), before, "no more following after the end");
    g.listen.stop().await.unwrap();
    assert_eq!(g.listen.view().role, Role::None);
}

#[tokio::test]
async fn a_guest_can_be_refused_control_requests_by_the_host_too() {
    // The host turns control off between the guest's view and its request.
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    let g = joined(&relay, "Alex", &link).await;
    // A request that skips the guest's own check: the host says no.
    h.settings.guests_control.store(false, Ordering::SeqCst);
    let (relay_of_g, up, me) = match &*g.listen.inner.mode.lock() {
        Mode::Guest(f) => (f.relay.clone(), f.up.clone(), f.me.clone()),
        _ => unreachable!(),
    };
    g.listen.send(&relay_of_g, &up, Sender::Guest, &me, Msg::Cmd(Op::Next), &[]).await.unwrap();
    eventually("the guest is told", || g.events.last(EVENT_NOTICE).is_some()).await;
    assert_eq!(g.events.last(EVENT_NOTICE).unwrap()["message"], "Sam only lets others add songs.");
    assert_eq!(h.player.track().as_deref(), Some("a"));
}

#[tokio::test]
async fn an_invite_works_once_and_not_after_it_runs_out() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    let _a = joined(&relay, "Alex", &link).await;
    // The same link again: used.
    let b = peer("Bo", &relay);
    let e = b.listen.join(&link).await.unwrap_err();
    assert!(e.to_string().contains("already used"), "{e}");
    assert_eq!(b.listen.view().role, Role::None);
    // The host offers a fresh one; it works.
    let v = h.listen.invite(false).unwrap();
    let fresh = v.invite.unwrap();
    assert_ne!(fresh.link, link);
    assert!(fresh.qr.is_some());
    let hours = fresh.expires_at.saturating_sub(now_ms()) / 3_600_000;
    assert!((23..=24).contains(&hours), "24 hours by default, got {hours}");
    b.listen.join(&fresh.link).await.unwrap();
    eventually("Bo in", || h.listen.view().people.len() == 3).await;

    // A token past its time is refused before anything is sent.
    let mut t = invite::parse(&h.listen.invite(true).unwrap().invite.unwrap().link).unwrap();
    t.expires = (now_ms() / 1000) as u32 - 10;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let sent = relay.requests().iter().filter(|r| r.starts_with("POST")).count();
    let c = peer("Cy", &relay);
    let e = c.listen.join(&t.link()).await.unwrap_err();
    assert!(e.to_string().contains("expired"), "{e}");
    assert_eq!(c.listen.view().role, Role::None);
    let (up, down) = invite_channels(&t);
    assert!(!relay.published().iter().any(|m| m.topic == up.topic), "nothing sent for it");
    assert!(!relay.requests().iter().any(|r| r.contains(&down.topic) && r.starts_with("GET") && !r.contains(&up.topic)));
    let _ = sent;
    // One the host has let run out gets no welcome either.
    let link = h.listen.invite(true).unwrap().invite.unwrap().link;
    if let Mode::Host(x) = &mut *h.listen.inner.mode.lock() {
        x.invites.last_mut().unwrap().expires_ms = now_ms() - 1;
    }
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(c.listen.join(&link).await.is_err());
    assert_eq!(c.listen.view().role, Role::None);
    // Shorter invites when the setting says so.
    h.settings.invite_hours.store(1, Ordering::SeqCst);
    let v = h.listen.invite(true).unwrap().invite.unwrap();
    assert!(v.expires_at <= now_ms() + 3_600_000);
}

#[tokio::test]
async fn removing_a_guest_revokes_their_keys() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    let a = joined(&relay, "Alex", &link).await;
    let b = joined(&relay, "Bo", &h.listen.invite(false).unwrap().invite.unwrap().link).await;
    eventually("both in", || h.listen.view().people.len() == 3).await;
    let a_id = a.listen.view().people.iter().find(|p| p.me).unwrap().id.clone();
    // What Alex holds before being removed.
    let (a_up, a_bcast, a_tag) = match &*a.listen.inner.mode.lock() {
        Mode::Guest(f) => (f.up.clone(), f.bcast.clone(), f.tag),
        _ => unreachable!(),
    };

    let v = h.listen.remove(&a_id).await.unwrap();
    assert_eq!(v.people.len(), 2);
    eventually("Alex is told", || status(&a) == Some(SessionStatus::Removed)).await;
    assert!(a.listen.control(Op::Pause).await.is_err());

    // Bo still follows, on a new broadcast key; Alex doesn't.
    h.player.set("n", 0, true, &[]);
    eventually("Bo follows", || b.player.track().as_deref() == Some("n")).await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_ne!(a.player.track().as_deref(), Some("n"));
    let after = relay.published().into_iter().filter(|m| m.topic == a_bcast.topic).count();
    h.player.set("o", 0, true, &[]);
    eventually("Bo follows again", || b.player.track().as_deref() == Some("o")).await;
    assert_eq!(relay.published().into_iter().filter(|m| m.topic == a_bcast.topic).count(), after, "nothing more on the old broadcast topic");
    // Any broadcast now can't be opened with Alex's old keys.
    let latest = relay.published().into_iter().rev().find(|m| m.body.matches('.').count() == 2).unwrap();
    assert!(seal::open(&a_bcast, &latest.body, now_ms(), Some(&a_tag)).is_err());

    // Alex's old key can't ask the host for anything.
    let forged = Envelope { r: Sender::Guest, f: a_id.clone(), c: u64::MAX - 1, x: now_ms() + 60_000, m: Msg::Cmd(Op::Pause) };
    relay.inject(&a_up.topic, &seal::seal(&a_up, &forged, &[]).unwrap());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(h.player.playing());
}

#[tokio::test]
async fn an_ended_session_refuses_everyone() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    let spare = h.listen.invite(true).unwrap().invite.unwrap().link;
    let g = joined(&relay, "Alex", &link).await;
    h.listen.stop().await.unwrap();
    eventually("guest sees the end", || status(&g) == Some(SessionStatus::Ended)).await;
    assert!(g.listen.control(Op::Next).await.is_err());
    // An invite from the ended session gets nowhere.
    let late = peer("Bo", &relay);
    let e = late.listen.join(&spare).await.unwrap_err();
    assert!(e.to_string().contains("No answer from the host"), "{e}");
    // Replaying an old message from the session changes nothing.
    let old = relay.published();
    for m in &old {
        relay.inject(&m.topic, &m.body);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.listen.view().role, Role::None);
    assert_eq!(status(&g), Some(SessionStatus::Ended));
    assert!(h.listen.invite(false).is_err());
}

#[tokio::test]
async fn replayed_relay_messages_are_ignored_mid_session() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    let g = joined(&relay, "Alex", &link).await;
    g.listen.control(Op::Pause).await.unwrap();
    eventually("paused", || !h.player.playing()).await;
    // Find the guest's pause on the wire and send the same bytes again later.
    let up = match &*g.listen.inner.mode.lock() {
        Mode::Guest(f) => f.up.clone(),
        _ => unreachable!(),
    };
    let pause = relay
        .published()
        .into_iter()
        .filter(|m| m.topic == up.topic)
        .find(|m| seal::open(&up, &m.body, now_ms(), None).is_ok_and(|e| e.m == Msg::Cmd(Op::Pause)))
        .unwrap();
    h.player.p.lock().playing = true;
    relay.inject(&pause.topic, &pause.body);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(h.player.playing(), "the copy was dropped");
}

const MB: &str = "0f6a1b2c-3d4e-4f50-8a9b-0c1d2e3f4a5b";

fn song(mb: Option<&str>, n: &str, a: &str) -> Song {
    Song { mb: mb.map(str::to_string), n: Some(n.into()), a: Some(a.into()), d: 200_000, u: false }
}

/// A guest on another server, with none of the host's songs.
async fn elsewhere(relay: &FakeNtfy, name: &str, link: &str, setup: impl FnOnce(&FakeLib)) -> Peer {
    let g = peer(name, relay);
    *g.server.0.lock() = Some("another-server-entirely".into());
    g.lib.has_only(&[]);
    setup(&g.lib);
    g.listen.join(link).await.unwrap();
    eventually("guest live", || status(&g) == Some(SessionStatus::Live)).await;
    g
}

#[tokio::test]
async fn a_guest_without_the_hosts_copy_plays_its_own_by_musicbrainz_id_or_by_text() {
    let relay = FakeNtfy::start().await;
    let h = peer("Sam", &relay);
    h.lib.song("a", song(Some(MB), "Night Drive", "The Band"));
    h.lib.song("b", song(None, "Second Song", "The Band"));
    h.player.set("a", 30_000, true, &["b"]);
    let link = h.listen.start().await.unwrap().invite.unwrap().link;
    let g = elsewhere(&relay, "Alex", &link, |lib| {
        lib.copy("x1", "Something Else", "Other", 201_000, Some(&MB.to_uppercase()));
        lib.copy("x2", "second song!", "THE BAND", 202_500, None);
    })
    .await;
    // "a" by its MusicBrainz id, at the host's spot; "b" by artist, title and length.
    eventually("guest plays its copy", || g.player.track().as_deref() == Some("x1")).await;
    assert!(g.player.log.lock()[0].starts_with("play x1 at 30"), "{:?}", g.player.log.lock());
    eventually("its copy of what's next is queued", || g.player.user() == ["x2"]).await;
    let v = g.listen.view();
    assert_eq!(v.queue.first().map(|q| q.track_id.as_str()), Some("x2"), "the queue shows the copy here");
    assert!(v.missing.is_none() && !v.another_version);
    h.player.set("b", 0, true, &[]);
    eventually("guest on its copy of b", || g.player.track().as_deref() == Some("x2")).await;
    assert!(!g.listen.view().another_version, "within 3 s is the same version");
    assert!(h.listen.view().cant_hear.is_empty());
    assert!(g.events.last(EVENT_NOTICE).is_none(), "no toasts");
}

#[tokio::test]
async fn another_version_plays_from_the_same_share_of_the_song() {
    let relay = FakeNtfy::start().await;
    let h = peer("Sam", &relay);
    h.lib.song("a", song(Some(MB), "Night Drive", "The Band"));
    h.player.set("a", 100_000, true, &[]);
    let link = h.listen.start().await.unwrap().invite.unwrap().link;
    // The host's runs 200 s; the copy here 300 s: halfway is 150 s.
    let g = elsewhere(&relay, "Alex", &link, |lib| lib.copy("x1", "Night Drive (Extended)", "The Band", 300_000, Some(MB))).await;
    eventually("guest plays its copy", || g.player.track().as_deref() == Some("x1")).await;
    assert!(g.player.log.lock()[0].starts_with("play x1 at 150"), "{:?}", g.player.log.lock());
    eventually("marked another version", || g.listen.view().another_version).await;
    assert_eq!(g.events.last(EVENT).unwrap()["anotherVersion"], true);
}

#[tokio::test]
async fn nothing_found_waits_quietly_tells_the_host_and_rejoins_at_the_next_song() {
    let relay = FakeNtfy::start().await;
    let h = peer("Sam", &relay);
    h.lib.song("a", song(None, "Gone Song", "The Band"));
    h.lib.song("b", song(None, "Next Song", "The Band"));
    h.player.set("a", 30_000, true, &["b"]);
    let link = h.listen.start().await.unwrap().invite.unwrap().link;
    // From another server, with only the host's next song.
    let g = elsewhere(&relay, "Alex", &link, |lib| lib.has_only(&["b"])).await;
    eventually("the song isn't here", || g.listen.view().missing.is_some()).await;
    let m = g.listen.view().missing.unwrap();
    assert_eq!((m.title.as_deref(), m.artist.as_deref(), m.unreleased), (Some("Gone Song"), Some("The Band"), false));
    assert_eq!(g.events.last(EVENT).unwrap()["missing"]["title"], "Gone Song");
    assert!(g.player.track().is_none() && !g.player.playing(), "nothing plays");
    assert_eq!(status(&g), Some(SessionStatus::Live), "still in the session");
    // The host sees who can't hear it, in the people list and by now playing.
    eventually("the host is told", || h.listen.view().cant_hear == ["Alex"]).await;
    assert!(h.listen.view().people.iter().any(|p| p.name == "Alex" && p.cant_hear));
    // The next song is here: the guest plays it.
    h.player.set("b", 0, true, &["a"]);
    eventually("guest rejoins at the next song", || g.player.track().as_deref() == Some("b") && g.player.playing()).await;
    eventually("the mark clears", || h.listen.view().cant_hear.is_empty()).await;
    assert!(g.listen.view().missing.is_none());
    // Playing, then a song that isn't here: a quiet pause.
    h.player.set("a", 0, true, &[]);
    eventually("guest waits again", || !g.player.playing() && g.listen.view().missing.is_some()).await;
    eventually("and the host sees it again", || h.listen.view().cant_hear == ["Alex"]).await;
    assert!(g.events.last(EVENT_NOTICE).is_none(), "never an error toast");
}

#[tokio::test]
async fn unreleased_songs_are_never_matched_and_their_titles_can_be_hidden() {
    let relay = FakeNtfy::start().await;
    let h = peer("Sam", &relay);
    h.lib.song("a", Song { u: true, ..song(Some(MB), "From The Vault", "The Band") });
    h.player.set("a", 30_000, true, &[]);
    let link = h.listen.start().await.unwrap().invite.unwrap().link;
    // A guest with the very same song elsewhere still doesn't hear it.
    let g = elsewhere(&relay, "Alex", &link, |lib| lib.copy("x1", "From The Vault", "The Band", 200_000, Some(MB))).await;
    // A guest with the host's own copy does.
    let other = joined(&relay, "Bo", &h.listen.invite(true).unwrap().invite.unwrap().link).await;
    assert_eq!(other.player.track().as_deref(), Some("a"));
    eventually("not matched", || g.listen.view().missing.is_some()).await;
    let m = g.listen.view().missing.unwrap();
    assert_eq!((m.title.as_deref(), m.unreleased), (Some("From The Vault"), true), "the title shows by default");
    assert!(g.player.track().is_none());
    assert!(!h.plain.lock().iter().any(|p| p.contains(MB)), "an unreleased song's MusicBrainz id isn't sent");
    // "Hide unreleased titles from guests".
    h.settings.hide_unreleased.store(true, Ordering::SeqCst);
    eventually("the title is hidden", || g.listen.view().missing.is_some_and(|m| m.title.is_none() && m.artist.is_none() && m.unreleased)).await;
    let n = h.plain.lock().len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!h.plain.lock()[n.saturating_sub(1)..].iter().any(|p| p.contains("From The Vault")));
}

#[test]
fn song_identity_is_optional_both_ways() {
    // A state from a host without it reads; a guest without it ignores it.
    let old = r#"{"srv":"s","t":"a","p":1,"d":2,"pl":true,"q":[{"k":"b"}],"ctl":true,"ppl":[]}"#;
    let st: State = serde_json::from_str(old).unwrap();
    assert_eq!((st.ts.as_ref(), st.q[0].s.as_ref()), (None, None));
    assert!(!serde_json::to_string(&st).unwrap().contains("\"ts\""), "nothing extra when unknown");
    let new = serde_json::to_value(State { ts: Some(song(Some(MB), "T", "A")), ..st }).unwrap();
    assert_eq!(new["ts"], serde_json::json!({ "mb": MB, "n": "T", "a": "A", "d": 200_000 }));
    let m: Msg = serde_json::from_str(r#"{"k":"cantHear","t":"a"}"#).unwrap();
    assert_eq!(m, Msg::CantHear { t: Some("a".into()) });
    assert_eq!(serde_json::from_str::<Msg>(r#"{"k":"cantHear"}"#).unwrap(), Msg::CantHear { t: None });
    // Song ids with their server tag can be added.
    assert!(ok_song("0a1b2c3d:12345") && ok_song("12345") && !ok_song("../x") && !ok_song("0a1b2c3d:") && !ok_song("a:b:c"));
}

#[test]
fn a_long_queue_drops_who_the_songs_are_before_the_songs() {
    let mut st = State {
        srv: "0".repeat(24),
        t: Some("0a1b2c3d:1".into()),
        ts: Some(song(Some(MB), &"T".repeat(80), &"A".repeat(80))),
        tby: None,
        p: 1,
        d: 2,
        pl: true,
        q: (0..UP_NEXT).map(|i| Queued { k: format!("0a1b2c3d:{i}"), by: None, s: Some(song(Some(MB), &"T".repeat(60), &"A".repeat(40))) }).collect(),
        ctl: true,
        ppl: Vec::new(),
    };
    fit(&mut st, 2);
    assert_eq!(st.q.len(), UP_NEXT, "every song still goes");
    assert!(st.ts.is_some() && st.q[0].s.is_some(), "the songs playing and next keep who they are");
    assert!(st.q.last().unwrap().s.is_none());
}

#[tokio::test]
async fn a_guest_on_another_server_can_join_but_not_without_one() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    let g = elsewhere(&relay, "Alex", &link, |_| {}).await;
    eventually("the host sees Alex", || h.listen.view().listeners == ["Alex"]).await;
    assert!(g.player.log.lock().is_empty(), "nothing played");
    g.listen.stop().await.unwrap();
    // Not signed in to any server: can't join or host.
    *g.server.0.lock() = None;
    assert!(g.listen.join(&h.listen.invite(true).unwrap().invite.unwrap().link).await.is_err());
    assert!(g.listen.start().await.is_err());
}

#[tokio::test]
async fn a_host_that_goes_quiet_is_shown_gone_and_can_come_back() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    let g = joined(&relay, "Alex", &link).await;
    relay.set_down(true);
    eventually("relay trouble shows", || g.listen.view().relay == RelayHealth::Unreachable).await;
    eventually("host gone after the heartbeat and grace", || status(&g) == Some(SessionStatus::HostGone)).await;
    assert_eq!(g.events.last(EVENT).unwrap()["status"], "hostGone");
    eventually("the host sees it too", || h.listen.view().relay == RelayHealth::Unreachable).await;
    relay.set_down(false);
    eventually("back", || status(&g) == Some(SessionStatus::Live)).await;
    assert_eq!(g.listen.view().relay, RelayHealth::Ok);
}

#[tokio::test]
async fn an_unreachable_or_busy_relay_is_said_so() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    relay.set_down(true);
    let g = peer("Alex", &relay);
    let e = g.listen.join(&link).await.unwrap_err();
    assert!(e.to_string().contains("Couldn't reach the listen along relay"), "{e}");
    relay.set_down(false);
    relay.set_limited(true);
    let e = g.listen.join(&link).await.unwrap_err();
    assert!(e.to_string().contains("relay is busy"), "{e}");
    // The host reconnects (a new invite changes what it listens on) and is refused too.
    let spare = h.listen.invite(true).unwrap();
    assert!(spare.invite.is_some());
    eventually("the host shows the limit", || h.listen.view().relay == RelayHealth::Limited).await;
    relay.set_limited(false);
    eventually("and recovers", || h.listen.view().relay == RelayHealth::Ok).await;
    g.listen.join(&link).await.unwrap();
}

#[tokio::test]
async fn a_self_hosted_relay_goes_in_the_invite_and_the_default_sees_nothing() {
    let default = FakeNtfy::start().await;
    let own = FakeNtfy::start().await;
    let h = peer("Sam", &default);
    h.player.set("a", 0, true, &[]);
    *h.settings.relay.write() = own.url.clone();
    let link = h.listen.start().await.unwrap().invite.unwrap().link;
    assert_eq!(invite::parse(&link).unwrap().relay.as_deref(), Some(own.url.as_str()));
    let g = joined(&default, "Alex", &link).await;
    assert_eq!(g.player.track().as_deref(), Some("a"));
    assert!(default.requests().is_empty(), "{:?}", default.requests());
    assert!(!own.published().is_empty());
}

/// Every place an address, a port, a host name, the machine id or a token
/// could show up.
fn leaks(s: &str) -> Vec<String> {
    let mut found = Vec::new();
    for needle in [MACHINE_ID, PLEX_TOKEN, SERVER_URL, "plex.direct", "://", "localhost", "127.0.0.1", "192.0.2.", "X-Plex", "32400", "token", "address", "url"] {
        if s.to_ascii_lowercase().contains(&needle.to_ascii_lowercase()) {
            found.push(needle.to_string());
        }
    }
    // Dotted quads (IPv4) and host:port pairs.
    let b = s.as_bytes();
    for i in 0..b.len() {
        let rest = &s[i..];
        let quad = rest.split(|c: char| !(c.is_ascii_digit() || c == '.')).next().unwrap_or("");
        if (i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b'.')) && quad.split('.').filter(|p| !p.is_empty() && p.len() <= 3).count() == 4 {
            found.push(format!("ip {quad}"));
        }
        if b[i] == b':' && i > 0 && (b[i - 1].is_ascii_alphanumeric()) && rest[1..].starts_with(|c: char| c.is_ascii_digit()) {
            let digits = rest[1..].chars().take_while(char::is_ascii_digit).count();
            if (2..=5).contains(&digits) {
                found.push(format!("port near {}", &s[i.saturating_sub(8)..(i + 6).min(s.len())]));
            }
        }
    }
    found
}

#[tokio::test]
async fn nothing_identifying_reaches_the_relay_or_the_invite() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    let a = joined(&relay, "Alex", &link).await;
    let b = joined(&relay, "Bo", &h.listen.invite(false).unwrap().invite.unwrap().link).await;
    a.listen.add(vec!["501".into()]).await.unwrap();
    a.listen.control(Op::Pause).await.unwrap();
    eventually("paused", || !h.player.playing()).await;
    let bid = b.listen.view().people.iter().find(|p| p.me).unwrap().id.clone();
    h.listen.remove(&bid).await.unwrap();
    a.listen.stop().await.unwrap();
    let late = h.listen.invite(true).unwrap().invite.unwrap();
    h.listen.stop().await.unwrap();

    // The invite: only the random token (the scheme aside).
    for l in [&link, &late.link] {
        let token = l.strip_prefix("platter://join/").unwrap();
        assert!(token.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'), "{l}");
        assert!(token.len() < 64);
        // Decoded: version, 32 random bytes and the expiry; nothing more.
        let t = invite::parse(l).unwrap();
        assert_eq!(t.relay, None);
        assert_eq!(Token { relay: None, ..t.clone() }.link(), *l);
    }
    // What the relay was asked for: topics (random hex) and nothing else.
    let reqs = relay.requests();
    assert!(!reqs.is_empty());
    for r in &reqs {
        let (_, path) = r.split_once(' ').unwrap();
        let path = path.split('?').next().unwrap().trim_start_matches('/').trim_end_matches("/json");
        for t in path.split(',') {
            assert_eq!(t.len(), 40, "{r}");
            assert!(t.bytes().all(|c| c.is_ascii_hexdigit()), "{r}");
        }
    }
    // What was published: sealed text only, under the size limit, and no
    // Firebase forwarding.
    let published = relay.published();
    assert!(published.len() > 10);
    for m in &published {
        assert!(m.body.starts_with("1.") && m.body.len() <= seal::MAX_BODY, "{}", m.body);
        // Base64url and the dots between parts: no room for an address,
        // a port or a URL (words can turn up by chance in base64, so only
        // the exact secrets are looked for).
        assert!(m.body.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.')), "{}", m.body);
        assert!(m.body.matches('.').count() <= 2);
        for secret in [MACHINE_ID, PLEX_TOKEN, SERVER_URL] {
            assert!(!m.body.contains(secret));
        }
        assert!(m.headers.iter().any(|(k, v)| k == "x-firebase" && v == "no"));
    }
    // And inside the seals, before sealing: names, rating keys, random ids.
    let mut plain = h.plain.lock().clone();
    plain.extend(a.plain.lock().iter().cloned());
    plain.extend(b.plain.lock().iter().cloned());
    assert!(plain.iter().any(|p| p.contains("\"State\"") || p.contains("\"k\":\"state\"")));
    for p in &plain {
        // Keys and sealed parts are random base64: checked for the exact
        // secrets only, the rest for anything that looks like an address.
        let v: serde_json::Value = serde_json::from_str(p).unwrap();
        let rest = scrub(v).to_string();
        assert!(leaks(&rest).is_empty(), "{p}: {:?}", leaks(&rest));
        for secret in [MACHINE_ID, PLEX_TOKEN, SERVER_URL] {
            assert!(!p.contains(secret));
        }
    }
    // The views the UI gets don't carry them either (the invite links aside).
    let links: Vec<String> = h.events.0.lock().iter().filter_map(|(_, v)| v["invite"]["link"].as_str().map(str::to_string)).collect();
    let views: Vec<String> = h.events.0.lock().iter().chain(a.events.0.lock().iter()).map(|e| e.1.to_string()).collect();
    for s in views {
        let rest = links.iter().fold(s.clone(), |acc, l| acc.replace(l, ""));
        assert!(leaks(&rest).is_empty(), "{s}: {:?}", leaks(&rest));
    }
}

/// Long base64 strings (keys, sealed parts) blanked out.
fn scrub(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::String(s) if s.len() >= 24 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') => Value::String(String::new()),
        Value::Array(a) => Value::Array(a.into_iter().map(scrub).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, scrub(v))).collect()),
        v => v,
    }
}

#[test]
fn the_leak_check_finds_what_it_should() {
    assert!(!leaks("host at 192.0.2.10").is_empty());
    assert!(!leaks("10.0.0.5").is_empty());
    assert!(!leaks("box:32520").is_empty());
    assert!(!leaks(MACHINE_ID).is_empty());
    assert!(!leaks(PLEX_TOKEN).is_empty());
    assert!(!leaks("https://ntfy.sh").is_empty());
    assert!(leaks(r#"{"c":12,"x":1800000000000,"m":{"k":"ping"},"p":30000}"#).is_empty());
}

#[tokio::test]
async fn one_session_at_a_time() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    let g = joined(&relay, "Alex", &link).await;
    assert!(g.listen.start().await.is_err(), "a guest can't host");
    assert!(h.listen.join(&link).await.is_err(), "a host can't join");
    assert_eq!(h.listen.start().await.unwrap().role, Role::Host, "starting again shows the same session");
    assert!(g.listen.join("platter://join/nope").await.is_err());
    // Leaving tells the host.
    g.listen.stop().await.unwrap();
    eventually("host sees them go", || h.listen.view().people.len() == 1).await;
}

// ---- "Let them listen from my server" ---------------------------------------

fn offer_of(h: &Peer, name: &str) -> Option<OfferState> {
    h.listen.view().people.into_iter().find(|p| p.name == name).and_then(|p| p.offer)
}

#[tokio::test]
async fn the_host_invites_a_guest_who_cant_hear_and_they_hear_the_hosts_copy_after_accepting() {
    let relay = FakeNtfy::start().await;
    let h = peer("Sam", &relay);
    h.lib.song("a", song(None, "Only On Sam's Server", "The Band"));
    h.player.set("a", 30_000, true, &[]);
    let link = h.listen.start().await.unwrap().invite.unwrap().link;
    assert!(h.listen.view().offer_server, "offered while the setting is on");
    let g = elsewhere(&relay, "Alex", &link, |_| {}).await;
    *g.offer.me.lock() = Some("alex_on_plex".into());
    eventually("the host sees Alex can't hear it", || h.listen.view().cant_hear == ["Alex"]).await;
    let alex = h.listen.view().people.into_iter().find(|p| p.name == "Alex").unwrap();
    assert_eq!(alex.offer, None);
    // One click: the guest's Platter answers with its Plex name, sealed; the
    // host shares through Plex and tells the guest to look for the invite.
    h.listen.offer_server(&alex.id).await.unwrap();
    eventually("invited", || offer_of(&h, "Alex") == Some(OfferState::Invited)).await;
    assert_eq!(*h.offer.invited.lock(), ["alex_on_plex"]);
    eventually("the guest looks for the invite", || *g.offer.shared.lock() == ["Sam"]).await;
    let plain = g.plain.lock().join("\n");
    assert!(plain.contains("alex_on_plex"), "the name went only inside sealed messages");
    assert!(!relay.published().iter().any(|m| m.body.contains("alex_on_plex")), "never in the clear");
    // Accepted: the host's server syncs here and the host's own copy plays.
    assert!(g.player.track().is_none());
    g.lib.has_only(&["a"]);
    g.listen.library_changed().await;
    eventually("the guest plays the host's copy", || g.player.track().as_deref() == Some("a") && g.player.playing()).await;
    assert!(g.listen.view().missing.is_none());
    eventually("the host sees everyone hears it", || h.listen.view().cant_hear.is_empty()).await;
    // Asking again for someone already invited asks again (they may have declined).
    h.listen.offer_server(&alex.id).await.unwrap();
    eventually("asked once more", || h.offer.invited.lock().len() == 2).await;
}

#[tokio::test]
async fn a_guest_who_doesnt_take_invites_in_platter_says_so_and_failures_show() {
    let relay = FakeNtfy::start().await;
    let (h, link) = hosting(&relay).await;
    let g = joined(&relay, "Alex", &link).await;
    let id = h.listen.view().people[1].id.clone();
    // "Accept invites inside Platter" off: no name goes to the host.
    h.listen.offer_server(&id).await.unwrap();
    eventually("no invites", || offer_of(&h, "Alex") == Some(OfferState::NoInvites)).await;
    assert!(h.offer.invited.lock().is_empty());
    assert!(!g.plain.lock().iter().any(|m| m.contains("plexUser") && m.contains("\"u\":\"")));
    // Plex says they already have access: the guest is still told to look.
    *g.offer.me.lock() = Some("alex_on_plex".into());
    *h.offer.outcome.lock() = OfferOutcome::AlreadyShared;
    h.listen.offer_server(&id).await.unwrap();
    eventually("already shared", || offer_of(&h, "Alex") == Some(OfferState::AlreadyShared)).await;
    eventually("told", || g.offer.shared.lock().len() == 1).await;
    // Plex down: the reason shows and the guest isn't sent looking.
    *h.offer.outcome.lock() = OfferOutcome::Failed("Plex isn't answering right now. Try again in a while.".into());
    h.listen.offer_server(&id).await.unwrap();
    eventually("failed", || matches!(offer_of(&h, "Alex"), Some(OfferState::Failed(_)))).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(g.offer.shared.lock().len(), 1);
    // Turned off in Settings: not offered, and refused.
    h.offer.on.store(false, Ordering::Relaxed);
    h.listen.settings_changed().await;
    assert!(!h.listen.view().offer_server);
    assert_eq!(h.listen.offer_server(&id).await.unwrap_err().kind(), "invalid");
    // Only the host offers.
    assert_eq!(g.listen.offer_server("host").await.unwrap_err().kind(), "invalid");
}
