//! Friend activity from faked sessions and history, follows and What's New,
//! the release announcement, and listening sessions between two in-process
//! instances (and once over HTTP on this computer).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;

use super::activity::{self, Account, ActivitySource, ActivityState, LiveRow, PlayedRow};
use super::follows::{self, FollowStore, FollowedArtist, KvFollows};
use super::listen::{self, HostState, Invite, ListenAlong, ListenConfig, ListenDeps, ListenPlayer, PlayerNow, Role, SessionStatus, Step, Transport};
use super::{Notifier, Social, SocialDeps, SocialSwitches};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::plex::state::EventSink;

const NOW: i64 = 1_800_000_000;

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

// ---- friend activity --------------------------------------------------------

const SESSIONS: &str = r#"{"MediaContainer":{"size":2,"Metadata":[
 {"type":"track","ratingKey":"501","title":"Song A","grandparentTitle":"Band","parentTitle":"Record","parentRatingKey":"50",
  "parentThumb":"/library/metadata/50/thumb/1","User":{"id":"7","title":"Sam","thumb":"https://plex.tv/users/x/avatar?c=1"},"Player":{"state":"playing"}},
 {"type":"episode","ratingKey":"9","title":"Show","User":{"id":"8","title":"Kim"}}
]}}"#;

const HISTORY: &str = r#"{"MediaContainer":{"size":4,"Metadata":[
 {"type":"track","ratingKey":"601","title":"Old Song","grandparentTitle":"Other","parentTitle":"LP","parentKey":"/library/metadata/60","viewedAt":1799999000,"accountID":9},
 {"type":"track","ratingKey":"602","title":"Older","grandparentTitle":"Other","parentTitle":"LP","parentKey":"/library/metadata/60","viewedAt":1799990000,"accountID":9},
 {"type":"track","ratingKey":"603","title":"Mine","grandparentTitle":"Me","parentTitle":"LP","viewedAt":1799999900,"accountID":1},
 {"type":"track","ratingKey":"604","title":"Ancient","grandparentTitle":"X","parentTitle":"Y","viewedAt":1700000000,"accountID":10},
 {"type":"track","ratingKey":"605","title":"Earlier Sam","grandparentTitle":"X","parentTitle":"Y","viewedAt":1799999990,"accountID":7}
]}}"#;

const ACCOUNTS: &str = r#"{"MediaContainer":{"size":4,"Account":[
 {"id":1,"name":"owner","thumb":""},{"id":7,"name":"Sam","thumb":""},{"id":9,"name":"Alex","thumb":""},{"id":10,"name":"Pat"},{"id":11,"name":""}
]}}"#;

#[test]
fn sessions_history_and_accounts_parse() {
    let live = activity::parse_sessions(SESSIONS.as_bytes()).unwrap();
    assert_eq!(live.len(), 1, "only music sessions");
    let l = &live[0];
    assert_eq!((l.account_id, l.track_id.as_str(), l.album_id.as_deref(), l.playing), (7, "501", Some("50"), true));
    assert_eq!(l.user_thumb.as_deref(), Some("/ext/https://plex.tv/users/x/avatar?c=1"));
    let h = activity::parse_history(HISTORY.as_bytes()).unwrap();
    assert_eq!(h.len(), 5);
    assert_eq!(h[0].album_id.as_deref(), Some("60"), "the album id comes from parentKey");
    let a = activity::parse_accounts(ACCOUNTS.as_bytes()).unwrap();
    assert_eq!(a.iter().map(|a| a.id).collect::<Vec<_>>(), vec![1, 7, 9, 10], "unnamed accounts are left out");
    assert!(a.iter().all(|a| a.thumb.is_none()));
    assert!(activity::parse_sessions(b"not json").is_err());
}

#[test]
fn one_row_per_person_newest_first_live_wins_own_and_old_left_out() {
    let live = activity::parse_sessions(SESSIONS.as_bytes()).unwrap();
    let played = activity::parse_history(HISTORY.as_bytes()).unwrap();
    let accounts = activity::parse_accounts(ACCOUNTS.as_bytes()).unwrap();
    let rows = activity::build_rows(&live, &played, &accounts, Some(1), NOW);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Sam", "Alex"], "own account and plays older than a week are left out");
    assert!(rows[0].now && rows[0].title == "Song A", "a live session beats Sam's history");
    assert_eq!((rows[1].title.as_str(), rows[1].at, rows[1].now), ("Old Song", 1_799_999_000, false), "Alex's latest play");
    // Without the live session, Sam's latest history row shows.
    let rows = activity::build_rows(&[], &played, &accounts, Some(1), NOW);
    assert_eq!(rows[0].name, "Sam");
    assert_eq!(rows[0].title, "Earlier Sam");
}

#[test]
fn a_paused_session_still_counts_but_a_playing_one_wins() {
    let row = |track: &str, playing: bool| LiveRow {
        account_id: 3,
        user_name: Some("Lee".into()),
        user_thumb: None,
        track_id: track.into(),
        title: track.into(),
        artist: String::new(),
        album_id: None,
        album: String::new(),
        cover: None,
        playing,
    };
    let rows = activity::build_rows(&[row("1", true), row("2", false)], &[], &[], None, NOW);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].track_id, "1");
    let only: Vec<PlayedRow> = Vec::new();
    let accounts = vec![Account { id: 3, name: "Lee".into(), thumb: None }];
    let rows = activity::build_rows(&[row("2", false)], &only, &accounts, None, NOW);
    assert!(rows[0].now);
}

struct FakeSource {
    owned: Option<bool>,
    refuse: bool,
}

#[async_trait]
impl ActivitySource for FakeSource {
    async fn owned(&self) -> Option<bool> {
        self.owned
    }
    async fn sessions(&self) -> AppResult<Vec<u8>> {
        if self.refuse {
            return Err(AppError::Auth("no".into()));
        }
        Ok(SESSIONS.as_bytes().to_vec())
    }
    async fn history(&self, _size: u64) -> AppResult<Vec<u8>> {
        Ok(HISTORY.as_bytes().to_vec())
    }
    async fn accounts(&self) -> AppResult<Vec<u8>> {
        Ok(ACCOUNTS.as_bytes().to_vec())
    }
}

#[tokio::test]
async fn admin_sees_activity_others_are_told_it_isnt_available() {
    let p = activity::read(&FakeSource { owned: Some(true), refuse: false }, NOW).await;
    assert_eq!(p.state, ActivityState::Ok);
    assert_eq!(p.friends.len(), 2);
    let p = activity::read(&FakeSource { owned: Some(false), refuse: false }, NOW).await;
    assert_eq!((p.state, p.friends.len()), (ActivityState::NotAdmin, 0));
    let p = activity::read(&FakeSource { owned: Some(true), refuse: true }, NOW).await;
    assert_eq!(p.state, ActivityState::NotAdmin, "the server refusing the read means no admin rights");
    let p = activity::read(&FakeSource { owned: None, refuse: false }, NOW).await;
    assert_eq!(p.state, ActivityState::Offline);
}

// ---- follows and What's New ---------------------------------------------------

#[derive(Default)]
struct Notes(Mutex<Vec<(String, String)>>);

impl Notifier for Notes {
    fn notify(&self, title: &str, body: &str) {
        self.0.lock().push((title.into(), body.into()));
    }
}

fn db() -> (DbHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("s.db")).unwrap();
    (db, dir)
}

fn add_album(db: &DbHandle, id: &str, artist: &str, added: i64) {
    let (id, artist) = (id.to_string(), artist.to_string());
    db.write(move |c| {
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, added_at) VALUES (?1, ?2, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, format!("Album {id}"), artist, format!("Artist {artist}"), added],
        )?;
        Ok(())
    })
    .unwrap();
}

struct Rig {
    social: Social,
    db: DbHandle,
    notes: Arc<Notes>,
    events: Arc<Events>,
    now: Arc<Mutex<i64>>,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let (db, dir) = db();
    let notes = Arc::new(Notes::default());
    let events = Arc::new(Events::default());
    let now = Arc::new(Mutex::new(NOW));
    let clock = now.clone();
    let social = Social::new(SocialDeps {
        db: db.clone(),
        follows: Arc::new(KvFollows(db.clone())),
        activity: Arc::new(FakeSource { owned: Some(true), refuse: false }),
        events: events.clone(),
        notifier: notes.clone(),
        switches: SocialSwitches::default(),
        clock: Arc::new(move || *clock.lock()),
    });
    Rig { social, db, notes, events, now, _dir: dir }
}

#[test]
fn follow_unfollow_and_follow_again_keep_one_entry() {
    let r = rig();
    r.social.set_follow("7".into(), "Band".into(), None, true).unwrap();
    r.social.set_follow("credit:guest".into(), "Guest".into(), None, true).unwrap();
    let all = r.social.set_follow("7".into(), "Band".into(), None, true).unwrap();
    assert_eq!(all.len(), 2, "following twice keeps one");
    let all = r.social.set_follow("7".into(), "Band".into(), None, false).unwrap();
    assert_eq!(all.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["credit:guest"]);
    // Kept in the database, and the UI was told.
    assert_eq!(KvFollows(r.db.clone()).list().unwrap().len(), 1);
    assert_eq!(r.events.last(super::EVENT_FOLLOWS).unwrap().as_array().unwrap().len(), 1);
    assert!(r.social.set_follow(" ".into(), "x".into(), None, true).is_err());
}

#[test]
fn whats_new_splits_new_and_earlier_and_counts_unseen() {
    let r = rig();
    add_album(&r.db, "1", "7", NOW - 10 * 86400);
    add_album(&r.db, "2", "8", NOW - 86400);
    // Nothing followed: everything recently added, and the first look is "seen".
    let w = r.social.whats_new().unwrap();
    assert!(!w.following);
    assert_eq!(w.items.len(), 2);
    assert_eq!(w.unseen, 0);
    r.social.set_follow("7".into(), "Artist 7".into(), None, true).unwrap();
    *r.now.lock() = NOW + 100;
    add_album(&r.db, "3", "7", NOW + 50);
    add_album(&r.db, "4", "7", NOW - 200 * 86400);
    let w = r.social.whats_new().unwrap();
    assert!(w.following);
    let ids: Vec<(&str, bool)> = w.items.iter().map(|i| (i.album.id.as_str(), i.new)).collect();
    assert_eq!(ids, [("3", true), ("1", false)], "followed only, newest first, old ones left out");
    assert_eq!(w.unseen, 1);
    r.social.whats_new_seen().unwrap();
    assert_eq!(r.social.whats_new().unwrap().unseen, 0);
    assert_eq!(r.events.last(super::EVENT_WHATS_NEW).unwrap()["unseen"], 0);
}

#[test]
fn new_releases_are_announced_once_only_when_on_and_not_private() {
    let r = rig();
    r.social.set_follow("7".into(), "Artist 7".into(), None, true).unwrap();
    add_album(&r.db, "1", "7", NOW - 5);
    // The first check only sets the mark.
    r.social.switches().notify_releases.store(true, Ordering::Relaxed);
    r.social.library_changed();
    assert!(r.notes.0.lock().is_empty());
    *r.now.lock() = NOW + 100;
    add_album(&r.db, "2", "7", NOW + 50);
    add_album(&r.db, "3", "8", NOW + 60);
    r.social.library_changed();
    assert_eq!(*r.notes.0.lock(), vec![("New from Artist 7".to_string(), "Album 2".to_string())]);
    // Once only.
    r.social.library_changed();
    assert_eq!(r.notes.0.lock().len(), 1);
    // Off, or a private session: nothing shows (and the mark still moves).
    add_album(&r.db, "5", "7", NOW + 70);
    r.social.switches().private_session.store(true, Ordering::Relaxed);
    r.social.library_changed();
    r.social.switches().private_session.store(false, Ordering::Relaxed);
    r.social.switches().notify_releases.store(false, Ordering::Relaxed);
    add_album(&r.db, "6", "7", NOW + 80);
    r.social.library_changed();
    assert_eq!(r.notes.0.lock().len(), 1);
    // The badge counts what's unseen while What's New announcements are on.
    assert!(r.events.last(super::EVENT_WHATS_NEW).unwrap()["unseen"].as_u64().unwrap() >= 1);
    r.social.switches().whats_new.store(false, Ordering::Relaxed);
    r.social.badge();
    assert_eq!(r.events.last(super::EVENT_WHATS_NEW).unwrap()["unseen"], 0);
}

#[test]
fn an_album_added_before_the_follow_isnt_announced() {
    let (db, _d) = db();
    add_album(&db, "1", "7", NOW + 10);
    let follows = vec![FollowedArtist { id: "7".into(), title: "A".into(), thumb: None, followed_at: NOW + 20 }];
    let found = db.read(|c| follows::to_announce(c, &follows, NOW)).unwrap();
    assert!(found.albums.is_empty());
    add_album(&db, "2", "7", NOW + 30);
    let found = db.read(|c| follows::to_announce(c, &follows, NOW)).unwrap();
    assert_eq!(found.albums.len(), 1);
    assert_eq!(found.mark, NOW + 30);
}

#[test]
fn notices_name_one_artist_or_count_several() {
    let mk = |id: &str, artist: &str| {
        let (d, _x) = db();
        add_album(&d, id, artist, NOW);
        d.read(|c| follows::releases(c, &[], 0, 10)).unwrap().remove(0)
    };
    let a = mk("1", "7");
    let b = mk("2", "7");
    let c = mk("3", "8");
    assert_eq!(follows::notice(&[]), None);
    assert_eq!(follows::notice(std::slice::from_ref(&a)).unwrap().0, "New from Artist 7");
    assert_eq!(follows::notice(&[a.clone(), b]).unwrap().1, "Album 1 and 1 more");
    assert_eq!(follows::notice(&[a, c]).unwrap(), ("New releases".into(), "2 new releases from artists you follow".into()));
}

#[tokio::test]
async fn friend_activity_off_reads_nothing() {
    let r = rig();
    assert_eq!(r.social.activity(false).await.state, ActivityState::Ok);
    r.social.switches().show_activity.store(false, Ordering::Relaxed);
    assert_eq!(r.social.activity(false).await.state, ActivityState::Off);
}

// ---- listening sessions -----------------------------------------------------

#[test]
fn invites_round_trip_and_bad_ones_are_refused() {
    let inv = Invite {
        session: "0123456789abcdef".into(),
        secret: "00112233445566778899aabbccddeeff".into(), // gitleaks:allow (test value, not a secret)
        server: Some("0a1b2c3d".into()),
        hosts: vec!["10.0.0.5:32520".parse().unwrap(), "172.16.0.9:32521".parse().unwrap()],
        name: "Sam & Co".into(),
    };
    let link = listen::build_invite(&inv);
    assert!(link.starts_with("platter://join/0123456789abcdef?k="));
    assert_eq!(listen::parse_invite(&link), Some(inv));
    let ok = "platter://join/0123456789abcdef?k=00112233445566778899aabbccddeeff&h=10.0.0.5:32520";
    assert!(listen::parse_invite(ok).is_some());
    for bad in [
        "platter://join/0123?k=00112233445566778899aabbccddeeff&h=10.0.0.5:32520",
        "platter://join/0123456789abcdef?h=10.0.0.5:32520",
        "platter://join/0123456789abcdef?k=00112233445566778899aabbccddeeff",
        "platter://join/0123456789abcdef?k=00112233445566778899aabbccddeeff&h=8.8.8.8:32520",
        "platter://join/0123456789abcdef?k=00112233445566778899aabbccddeeff&h=10.0.0.5:0",
        "platter://album/0123456789abcdef?k=00112233445566778899aabbccddeeff&h=10.0.0.5:32520",
        "https://join/0123456789abcdef?k=00112233445566778899aabbccddeeff&h=10.0.0.5:32520",
    ] {
        assert_eq!(listen::parse_invite(bad), None, "{bad}");
    }
}

fn host(track: Option<&str>, pos: u64, playing: bool) -> HostState {
    HostState {
        session_id: "s".into(),
        host_name: "Sam".into(),
        track_id: track.map(str::to_string),
        position_ms: pos,
        duration_ms: 200_000,
        playing,
        up_next: vec!["b".into(), "c".into()],
        guests: Vec::new(),
        guests_can_add: true,
    }
}

fn local(track: Option<&str>, pos: u64, playing: bool) -> PlayerNow {
    PlayerNow { track_id: track.map(str::to_string), position_ms: pos, duration_ms: 200_000, playing, up_next: Vec::new() }
}

#[test]
fn guests_follow_the_host_with_latency_compensation() {
    // Another song: load the host's at the spot, plus what's next.
    assert_eq!(
        listen::plan(&local(Some("x"), 0, true), &host(Some("a"), 10_000, true), 400),
        vec![Step::Load { ids: vec!["a".into(), "b".into(), "c".into()], position_ms: 10_200, play: true }]
    );
    // Paused: no compensation.
    assert_eq!(listen::target_ms(&host(Some("a"), 10_000, false), 400), 10_000);
    // Close enough: nothing.
    assert!(listen::plan(&local(Some("a"), 10_900, true), &host(Some("a"), 10_000, true), 400).is_empty());
    // Drifted: seek.
    assert_eq!(listen::plan(&local(Some("a"), 20_000, true), &host(Some("a"), 10_000, true), 0), vec![Step::Seek(10_000)]);
    // Pauses and plays follow.
    assert_eq!(listen::plan(&local(Some("a"), 10_000, true), &host(Some("a"), 10_000, false), 0), vec![Step::Pause]);
    assert_eq!(listen::plan(&local(Some("a"), 10_000, false), &host(Some("a"), 10_000, true), 0), vec![Step::Resume]);
    // Nothing playing at the host.
    assert_eq!(listen::plan(&local(Some("a"), 0, true), &host(None, 0, false), 0), vec![Step::Pause]);
    // The guest went on to the next song just before the host: wait.
    assert!(listen::plan(&local(Some("b"), 100, true), &host(Some("a"), 199_000, true), 200).is_empty());
    // Never past the end.
    assert_eq!(listen::target_ms(&host(Some("a"), 199_900, true), 1000), 200_000);
}

#[derive(Default)]
struct FakePlayer {
    now: Mutex<PlayerNow>,
    log: Mutex<Vec<String>>,
}

#[async_trait]
impl ListenPlayer for FakePlayer {
    async fn now(&self) -> AppResult<PlayerNow> {
        Ok(self.now.lock().clone())
    }
    async fn play(&self, ids: Vec<String>, position_ms: u64, play: bool) -> AppResult<()> {
        let mut n = self.now.lock();
        n.track_id = ids.first().cloned();
        n.up_next = ids[1..].to_vec();
        n.position_ms = position_ms;
        n.duration_ms = 200_000;
        n.playing = play;
        self.log.lock().push(format!("play {} at {position_ms}", ids[0]));
        Ok(())
    }
    async fn seek(&self, position_ms: u64) -> AppResult<()> {
        self.now.lock().position_ms = position_ms;
        self.log.lock().push(format!("seek {position_ms}"));
        Ok(())
    }
    async fn pause(&self) -> AppResult<()> {
        self.now.lock().playing = false;
        self.log.lock().push("pause".into());
        Ok(())
    }
    async fn resume(&self) -> AppResult<()> {
        self.now.lock().playing = true;
        self.log.lock().push("resume".into());
        Ok(())
    }
    async fn queue_add(&self, ids: Vec<String>) -> AppResult<()> {
        self.now.lock().up_next.extend(ids.iter().cloned());
        self.log.lock().push(format!("add {}", ids.join(",")));
        Ok(())
    }
}

/// Requests go straight to the instance at that address.
#[derive(Default)]
struct Mem {
    hosts: Mutex<HashMap<SocketAddr, ListenAlong>>,
    down: AtomicBool,
}

#[async_trait]
impl Transport for Mem {
    async fn send(&self, host: &SocketAddr, method: &str, path: &str, auth: &str, body: Vec<u8>, _t: Duration) -> AppResult<(u16, Vec<u8>)> {
        if self.down.load(Ordering::SeqCst) {
            return Err(AppError::Network("unreachable".into()));
        }
        let Some(h) = self.hosts.lock().get(host).cloned() else {
            return Err(AppError::Network("unreachable".into()));
        };
        let (status, text) = h.answer(method, path, Some(auth), &body).await;
        Ok((status, text.into_bytes()))
    }
}

struct Peer {
    listen: ListenAlong,
    player: Arc<FakePlayer>,
    events: Arc<Events>,
    can_add: Arc<AtomicBool>,
}

fn cfg() -> ListenConfig {
    ListenConfig {
        poll: Duration::from_millis(15),
        host_gone: Duration::from_millis(150),
        guest_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_millis(500),
    }
}

fn peer(id: &str, name: &str, transport: Arc<dyn Transport>) -> Peer {
    let player = Arc::new(FakePlayer::default());
    let events = Arc::new(Events::default());
    let can_add = Arc::new(AtomicBool::new(true));
    let name = name.to_string();
    let listen = ListenAlong::new(ListenDeps {
        player: player.clone(),
        transport,
        events: events.clone(),
        own_id: id.into(),
        name: Arc::new(move || name.clone()),
        guests_can_add: can_add.clone(),
        cfg: cfg(),
    });
    Peer { listen, player, events, can_add }
}

async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}

const HOST_AT: &str = "10.0.0.5:32520";

fn two() -> (Arc<Mem>, Peer, Peer) {
    let mem = Arc::new(Mem::default());
    let h = peer("host-pc", "Sam", mem.clone());
    let g = peer("guest-pc", "Alex", mem.clone());
    mem.hosts.lock().insert(HOST_AT.parse().unwrap(), h.listen.clone());
    *h.player.now.lock() = PlayerNow {
        track_id: Some("a".into()),
        position_ms: 30_000,
        duration_ms: 200_000,
        playing: true,
        up_next: vec!["b".into()],
    };
    (mem, h, g)
}

#[tokio::test]
async fn a_session_from_start_to_end_with_two_instances() {
    let (_mem, h, g) = two();
    let view = h.listen.start(Some("0a1b2c3d".into()), vec![HOST_AT.parse().unwrap()]).unwrap();
    assert_eq!((view.role, view.status), (Role::Host, Some(SessionStatus::Live)));
    let invite = view.invite.unwrap();
    assert!(listen::parse_invite(&invite).is_some());
    assert_eq!(h.listen.subscribe_party().borrow().as_ref().map(|p| (p.size, p.max)), Some((1, 11)));

    // Another server: refused before anything is sent.
    assert!(g.listen.join(&invite, Some("ffffffff".into())).await.is_err());
    let v = g.listen.join(&invite, Some("0a1b2c3d".into())).await.unwrap();
    assert_eq!((v.role, v.status, v.host_name.as_deref()), (Role::Guest, Some(SessionStatus::Live), Some("Sam")));
    // The guest plays the host's song at about its spot, then what's next.
    let log = g.player.log.lock().clone();
    assert!(log[0].starts_with("play a at 30"), "{log:?}");
    assert_eq!(g.player.now.lock().up_next, vec!["b".to_string()]);
    // The host sees who's listening.
    assert_eq!(h.listen.view().listeners, vec!["Alex".to_string()]);
    assert_eq!(h.events.last(listen::EVENT).unwrap()["listeners"][0], "Alex");
    assert_eq!(h.listen.subscribe_party().borrow().as_ref().unwrap().size, 2);

    // The host pauses, then skips: the guest follows.
    h.player.now.lock().playing = false;
    eventually("guest pauses", || !g.player.now.lock().playing).await;
    *h.player.now.lock() = PlayerNow { track_id: Some("b".into()), position_ms: 0, duration_ms: 200_000, playing: true, up_next: vec![] };
    eventually("guest skips", || g.player.now.lock().track_id.as_deref() == Some("b")).await;

    // Guests add to the shared queue, while the host allows it.
    g.listen.add(vec!["z1".into(), "z2".into()]).await.unwrap();
    assert!(h.player.log.lock().contains(&"add z1,z2".to_string()));
    h.can_add.store(false, Ordering::SeqCst);
    let e = g.listen.add(vec!["z3".into()]).await.unwrap_err();
    assert!(e.to_string().contains("doesn't let guests add"));
    assert!(g.listen.add(vec!["../x".into()]).await.is_err());

    // The host ends it: the guest stops following.
    h.listen.stop().await.unwrap();
    assert_eq!(h.listen.view().role, Role::None);
    assert_eq!(*h.listen.subscribe_party().borrow(), None);
    eventually("guest sees the end", || g.listen.view().status == Some(SessionStatus::Ended)).await;
    assert_eq!(g.events.last(listen::EVENT).unwrap()["status"], "ended");
    let before = g.player.log.lock().len();
    *h.player.now.lock() = PlayerNow { track_id: Some("q".into()), ..Default::default() };
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(g.player.log.lock().len(), before, "no more following after the end");
    g.listen.stop().await.unwrap();
    assert_eq!(g.listen.view().role, Role::None);
}

#[tokio::test]
async fn a_host_that_goes_away_is_reported_gone() {
    let (mem, h, g) = two();
    let invite = h.listen.start(None, vec![HOST_AT.parse().unwrap()]).unwrap().invite.unwrap();
    g.listen.join(&invite, None).await.unwrap();
    mem.down.store(true, Ordering::SeqCst);
    eventually("host gone", || g.listen.view().status == Some(SessionStatus::HostGone)).await;
    assert_eq!(g.events.last(listen::EVENT).unwrap()["status"], "hostGone");
}

#[tokio::test]
async fn leaving_tells_the_host_and_a_wrong_secret_gets_nothing() {
    let (_mem, h, g) = two();
    let invite = h.listen.start(None, vec![HOST_AT.parse().unwrap()]).unwrap().invite.unwrap();
    g.listen.join(&invite, None).await.unwrap();
    g.listen.stop().await.unwrap();
    assert!(h.listen.view().listeners.is_empty());
    // Someone with the session id but not its secret.
    let forged = invite.replace(&listen::parse_invite(&invite).unwrap().secret, "ffffffffffffffffffffffffffffffff");
    let e = g.listen.join(&forged, None).await.unwrap_err();
    assert!(e.to_string().contains("isn't for this session"), "{e}");
    assert_eq!(g.listen.view().role, Role::None);
    assert!(h.listen.view().listeners.is_empty());
    // Nobody hosting: a stale invite says the session ended.
    h.listen.stop().await.unwrap();
    assert!(g.listen.join(&invite, None).await.unwrap_err().to_string().contains("ended"));
}

#[tokio::test]
async fn hosting_needs_the_control_port_and_one_session_at_a_time() {
    let (_mem, h, g) = two();
    assert!(h.listen.start(None, vec![]).is_err());
    let invite = h.listen.start(None, vec![HOST_AT.parse().unwrap()]).unwrap().invite.unwrap();
    g.listen.join(&invite, None).await.unwrap();
    assert!(g.listen.start(None, vec![HOST_AT.parse().unwrap()]).is_err(), "a guest can't host");
    assert!(h.listen.join(&invite, None).await.is_err(), "a host can't join");
}

// ---- over HTTP, through the control API -------------------------------------

struct NoHandler;

#[async_trait]
impl crate::remote::peer::server::PeerHandler for NoHandler {
    async fn hello(&self) -> crate::remote::peer::Hello {
        crate::remote::peer::Hello { id: "x".into(), name: "x".into(), platform: "x".into(), version: "x".into(), playing: None }
    }
    async fn timeline(&self) -> crate::remote::peer::PeerTimeline {
        unreachable!()
    }
    async fn play(&self, _req: crate::remote::peer::PlayRequest) -> AppResult<()> {
        unreachable!()
    }
    async fn command(&self, _cmd: crate::remote::types::RemoteCommand) -> AppResult<()> {
        unreachable!()
    }
    async fn stop(&self) -> AppResult<()> {
        unreachable!()
    }
}

struct NoKey;

#[async_trait]
impl crate::remote::peer::server::KeySource for NoKey {
    async fn key(&self) -> Option<crate::remote::peer::auth::PeerKey> {
        None
    }
}

#[tokio::test]
async fn a_guest_joins_over_http_through_the_control_port() {
    use crate::remote::peer::server::{self, ExtraSlot, PeerServer};
    let http: Arc<dyn Transport> = Arc::new(listen::HttpTransport::new().unwrap());
    let h = peer("host-pc", "Sam", http.clone());
    let g = peer("guest-pc", "Alex", http);
    *h.player.now.lock() = PlayerNow { track_id: Some("a".into()), position_ms: 5_000, duration_ms: 200_000, playing: true, up_next: vec![] };
    let slot = ExtraSlot::default();
    *slot.write() = Some(Arc::new(h.listen.clone()));
    let srv = Arc::new(PeerServer::new("host-pc".into(), Arc::new(NoHandler), Arc::new(NoKey)).with_extra(slot));
    let (listeners, port) = server::bind(&["127.0.0.1".parse().unwrap()], 0, 1).await.unwrap();
    let (stop, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(server::serve(listeners, srv.clone(), rx));
    let at: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let invite = h.listen.start(None, vec![at]).unwrap().invite.unwrap();
    g.listen.join(&invite, None).await.unwrap();
    assert_eq!(g.player.now.lock().track_id.as_deref(), Some("a"));
    assert_eq!(h.listen.view().listeners, vec!["Alex".to_string()]);
    // The account routes still refuse the session's guests.
    let (code, _) = srv.respond("GET", crate::remote::peer::PATH_HELLO, None, b"").await;
    assert_eq!(code, 401);
    g.listen.stop().await.unwrap();
    h.listen.stop().await.unwrap();
    let _ = stop.send(true);
}
