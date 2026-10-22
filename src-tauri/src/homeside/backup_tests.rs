//! Links on more than one relay: a home whose address one relay turns
//! away still gets through on the others (in-process ntfy servers only).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, watch};

use super::link::{Link, Side};
use super::links::Links;
use super::peer::{self, Incoming, Wire};
use super::wire::Msg;
use crate::db::DbHandle;
use crate::plex::keychain::MemoryStore;
use crate::social::listen::fake_ntfy::FakeNtfy;
use crate::social::listen::relay::{self, Ntfy, Relay, RelayError, RelayEvent, RelayFactory};

/// A relay factory that can't reach some addresses (like a home whose
/// address one relay has banned), and reaches the rest normally.
struct Blocked {
    inner: Arc<dyn RelayFactory>,
    blocked: Vec<String>,
    dead: Arc<Dead>,
    /// Reached, but each publish takes a while.
    slow: Vec<String>,
}

struct Slow(Arc<dyn Relay>);

#[async_trait]
impl Relay for Slow {
    async fn publish(&self, topic: &str, body: &str) -> Result<(), RelayError> {
        tokio::time::sleep(Duration::from_millis(150)).await;
        self.0.publish(topic, body).await
    }
    async fn subscribe(&self, topics: &[String], since: Option<&str>) -> Result<mpsc::Receiver<RelayEvent>, RelayError> {
        self.0.subscribe(topics, since).await
    }
}

/// Can't be reached; counts how often it was asked to publish.
#[derive(Default)]
struct Dead(AtomicUsize);

#[async_trait]
impl Relay for Arc<Dead> {
    async fn publish(&self, _: &str, _: &str) -> Result<(), RelayError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(RelayError::Unreachable)
    }
    async fn subscribe(&self, _: &[String], _: Option<&str>) -> Result<mpsc::Receiver<RelayEvent>, RelayError> {
        Err(RelayError::Unreachable)
    }
}

impl RelayFactory for Blocked {
    fn relay(&self, base: &str) -> Arc<dyn Relay> {
        if self.blocked.iter().any(|b| b == base) {
            Arc::new(self.dead.clone())
        } else if self.slow.iter().any(|b| b == base) {
            Arc::new(Slow(self.inner.relay(base)))
        } else {
            self.inner.relay(base)
        }
    }
}

struct End {
    wire: Arc<Wire>,
    link: Link,
    /// Publishes this end tried on relays it can't reach.
    dead: Arc<Dead>,
    _dir: tempfile::TempDir,
}

fn end(side: Side, me: &str, peer: &str, main: &FakeNtfy, backups: &[&FakeNtfy], blocked: &[&FakeNtfy], slow: &[&FakeNtfy]) -> End {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("l.db")).unwrap();
    let links = Links::new(db, Arc::new(MemoryStore::new()));
    let link = Link { id: "l1".into(), secret: [9u8; 32], relay: main.url.clone(), side, me: me.into(), peer: peer.into() };
    links.add(&link, "Other end", "", 1).unwrap();
    let dead = Arc::new(Dead::default());
    let relays: Arc<dyn RelayFactory> =
        Arc::new(Blocked { inner: Arc::new(Ntfy::new().unwrap()), blocked: blocked.iter().map(|n| n.url.clone()).collect(), dead: dead.clone(), slow: slow.iter().map(|n| n.url.clone()).collect() });
    let wire = Wire::new(links, relays, Arc::new(peer::system_ms)).with_backups(&main.url, backups.iter().map(|n| n.url.clone()).collect());
    End { wire: Arc::new(wire), link, dead, _dir: dir }
}

fn pair(main: &FakeNtfy, backups: &[&FakeNtfy], desk_blocked: &[&FakeNtfy], bridge_blocked: &[&FakeNtfy]) -> (End, End) {
    (end(Side::Desktop, "desk", "brdg", main, backups, desk_blocked, &[]), end(Side::Bridge, "brdg", "desk", main, backups, bridge_blocked, &[]))
}

/// Listen on an end's link; the channel gets what arrives.
fn listen(e: &End) -> (mpsc::Receiver<Incoming>, watch::Sender<bool>) {
    let (tx, rx) = mpsc::channel(64);
    let (stop_tx, stop) = watch::channel(false);
    tokio::spawn(peer::listen(e.wire.clone(), e.link.clone(), tx, stop));
    (rx, stop_tx)
}

async fn next(rx: &mut mpsc::Receiver<Incoming>) -> Option<Incoming> {
    tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.ok().flatten()
}

/// The next whole message, skipping "open".
async fn next_msg(rx: &mut mpsc::Receiver<Incoming>) -> Option<Msg> {
    loop {
        match next(rx).await? {
            Incoming::Msg(_, m) => return Some(m),
            Incoming::Opened(_) => {}
        }
    }
}

fn answer(id: u64) -> Msg {
    Msg::Answer { id, status: 200, body: format!("answer {id}") }
}

fn id_of(m: &Msg) -> Option<u64> {
    match m {
        Msg::Answer { id, .. } => Some(*id),
        _ => None,
    }
}

/// Wait for the link to open, and for each of `on` to have had `n`
/// listeners in all (both ends count).
async fn opened(rx: &mut mpsc::Receiver<Incoming>, on: &[&FakeNtfy], n: usize) {
    assert!(matches!(next(rx).await, Some(Incoming::Opened(_))), "the link opened");
    for r in on {
        super::tests::until("a listener on every relay", || r.requests().iter().filter(|q| q.starts_with("GET")).count() >= n).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_on_ntfy_sh_also_uses_the_backups_and_one_of_its_own_doesnt() {
    let links = Links::new(DbHandle::open(tempfile::tempdir().unwrap().path().join("l.db")).unwrap(), Arc::new(MemoryStore::new()));
    let wire = Wire::new(links, Arc::new(Ntfy::new().unwrap()), Arc::new(|| 1));
    let all = wire.relays_for(crate::social::listen::DEFAULT_RELAY);
    assert_eq!(all[0], crate::social::listen::DEFAULT_RELAY);
    assert_eq!(all.len(), 1 + crate::social::listen::BACKUP_RELAYS.len());
    assert!(all.iter().all(|r| r.starts_with("https://")));
    assert_eq!(wire.relays_for("https://ntfy.example.org"), vec!["https://ntfy.example.org".to_string()]);
    // Kept with a slash on the end, it's still ntfy.sh.
    assert_eq!(wire.relays_for("https://ntfy.sh/").len(), all.len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn when_the_main_relay_is_down_messages_go_through_a_backup() {
    let (main, b1, b2) = (FakeNtfy::start().await, FakeNtfy::start().await, FakeNtfy::start().await);
    main.set_down(true);
    let (desk, bridge) = pair(&main, &[&b1, &b2], &[], &[]);
    let (mut rx, _stop) = listen(&bridge);
    opened(&mut rx, &[&b1, &b2], 1).await;
    desk.wire.send(&desk.link, &answer(1)).await.unwrap();
    assert_eq!(next_msg(&mut rx).await.as_ref().and_then(id_of), Some(1));
    // And back the other way.
    let (mut drx, _dstop) = listen(&desk);
    opened(&mut drx, &[&b1, &b2], 2).await;
    bridge.wire.send(&bridge.link, &answer(2)).await.unwrap();
    assert_eq!(next_msg(&mut drx).await.as_ref().and_then(id_of), Some(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_home_the_main_relay_turns_away_still_hears_a_phone_that_reaches_it() {
    // The phone (desk end here) reaches every relay and its sends to the
    // main one go through; the Bridge can't reach the main one at all.
    let (main, b1) = (FakeNtfy::start().await, FakeNtfy::start().await);
    let (phone, bridge) = pair(&main, &[&b1], &[], &[&main]);
    let (mut rx, _stop) = listen(&bridge);
    opened(&mut rx, &[&b1], 1).await;
    phone.wire.send(&phone.link, &answer(7)).await.unwrap();
    assert_eq!(next_msg(&mut rx).await.as_ref().and_then(id_of), Some(7));
    super::tests::until("the copy on each relay", || main.published().len() == 1 && b1.published().len() == 1).await;
    // The Bridge answers on the one it can reach, and the phone hears it.
    let (mut prx, _pstop) = listen(&phone);
    opened(&mut prx, &[&main, &b1], 1).await;
    super::tests::until("the phone on the backup", || b1.requests().iter().filter(|q| q.starts_with("GET")).count() >= 2).await;
    bridge.wire.send(&bridge.link, &answer(8)).await.unwrap();
    assert_eq!(next_msg(&mut prx).await.as_ref().and_then(id_of), Some(8));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn copies_on_every_relay_arrive_once_and_the_link_opens_once() {
    let (main, b1, b2) = (FakeNtfy::start().await, FakeNtfy::start().await, FakeNtfy::start().await);
    let (desk, bridge) = pair(&main, &[&b1, &b2], &[], &[]);
    let (mut rx, _stop) = listen(&bridge);
    opened(&mut rx, &[&main, &b1, &b2], 1).await;
    // A message in several pieces, then a small one.
    let big: String = (0..30_000u32).map(|_| char::from(b'a' + rand::random::<u8>() % 26)).collect();
    desk.wire.send(&desk.link, &Msg::Answer { id: 1, status: 200, body: big.clone() }).await.unwrap();
    desk.wire.send(&desk.link, &answer(2)).await.unwrap();
    match next_msg(&mut rx).await {
        Some(Msg::Answer { id: 1, body, .. }) => assert_eq!(body, big),
        _ => panic!("the big message didn't arrive whole"),
    }
    assert_eq!(next_msg(&mut rx).await.as_ref().and_then(id_of), Some(2));
    let n = main.published().len();
    assert!(n > 2, "{n}");
    super::tests::until("every piece on every relay", || b1.published().len() == n && b2.published().len() == n).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(rx.try_recv().is_err(), "no copies and no second open");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_relay_keeps_the_order_of_a_links_messages() {
    // The Bridge hears only the backup, and from the phone the backup is
    // slow: the phone's sends are done (the main relay took them) long
    // before the backup has them all.
    let (main, b1) = (FakeNtfy::start().await, FakeNtfy::start().await);
    let phone = end(Side::Desktop, "desk", "brdg", &main, &[&b1], &[], &[&b1]);
    let bridge = end(Side::Bridge, "brdg", "desk", &main, &[&b1], &[&main], &[]);
    let (mut rx, _stop) = listen(&bridge);
    opened(&mut rx, &[&b1], 1).await;
    let big: String = (0..20_000u32).map(|_| char::from(b'a' + rand::random::<u8>() % 26)).collect();
    phone.wire.send(&phone.link, &Msg::Answer { id: 1, status: 200, body: big }).await.unwrap();
    for id in 2..5 {
        phone.wire.send(&phone.link, &answer(id)).await.unwrap();
    }
    assert!(b1.published().len() < main.published().len(), "the backup is still behind");
    let mut got = Vec::new();
    while got.len() < 4 {
        got.push(next_msg(&mut rx).await.as_ref().and_then(id_of).expect("every message, none lost"));
    }
    assert_eq!(got, vec![1, 2, 3, 4]);
}

#[test]
fn a_frame_that_comes_in_late_on_another_relay_still_counts_once() {
    use super::link::{Frame, Guard};
    let f = |c: u64| Frame { f: "desk".into(), c, x: 0, i: "m".into(), n: 0, t: 1, d: String::new() };
    let mut g = Guard::default();
    g.accept(&f(10)).unwrap();
    g.accept(&f(30)).unwrap();
    // 20 came in late: never taken, newer than the oldest kept.
    g.accept(&f(20)).unwrap();
    assert!(g.accept(&f(20)).is_err(), "only once");
    assert!(g.accept(&f(30)).is_err() && g.accept(&f(10)).is_err());
    assert!(g.accept(&f(5)).is_err(), "older than anything kept");
    // Once it's out of the window, a late one that old doesn't count.
    for c in 100..100 + super::link::GUARD_WINDOW as u64 {
        g.accept(&f(c)).unwrap();
    }
    assert!(g.accept(&f(25)).is_err());
    // A guard kept by an older build (no window yet) takes only what's newer.
    let mut old: Guard = serde_json::from_value(serde_json::json!({"last": {"desk": 30}})).unwrap();
    assert!(old.accept(&f(20)).is_err());
    old.accept(&f(31)).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_backup_that_is_down_is_left_alone_for_a_while() {
    let (main, b1) = (FakeNtfy::start().await, FakeNtfy::start().await);
    let (desk, bridge) = pair(&main, &[&b1], &[&b1], &[]);
    let (mut rx, _stop) = listen(&bridge);
    opened(&mut rx, &[&main, &b1], 1).await;
    desk.wire.send(&desk.link, &answer(1)).await.unwrap();
    assert_eq!(next_msg(&mut rx).await.as_ref().and_then(id_of), Some(1));
    // Its tries finish by themselves, then it's marked down and not asked
    // again for the next message.
    super::tests::until("the backup marked down", || desk.wire.is_down(&b1.url)).await;
    let tried = desk.dead.0.load(Ordering::SeqCst);
    assert_eq!(tried, 4, "the first try and three more");
    desk.wire.send(&desk.link, &answer(2)).await.unwrap();
    assert_eq!(next_msg(&mut rx).await.as_ref().and_then(id_of), Some(2));
    assert_eq!(desk.dead.0.load(Ordering::SeqCst), tried, "left alone");
    // Once its wait is over it's tried again.
    tokio::time::sleep(relay::DOWN_FIRST + Duration::from_millis(200)).await;
    assert!(!desk.wire.is_down(&b1.url));
    desk.wire.send(&desk.link, &answer(3)).await.unwrap();
    super::tests::until("the backup tried again", || desk.dead.0.load(Ordering::SeqCst) > tried).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_every_relay_down_the_send_says_the_relay_cant_be_reached() {
    let (main, b1) = (FakeNtfy::start().await, FakeNtfy::start().await);
    let (desk, _bridge) = pair(&main, &[&b1], &[&main, &b1], &[]);
    let err = desk.wire.send(&desk.link, &answer(1)).await.unwrap_err();
    assert!(err.to_string().contains(&peer::relay_error(RelayError::Unreachable)), "{err}");
    assert!(desk.wire.is_down(&main.url) && desk.wire.is_down(&b1.url));
    // Tried again even while every one is marked down.
    let before = desk.dead.0.load(Ordering::SeqCst);
    assert!(desk.wire.send(&desk.link, &answer(2)).await.is_err());
    assert!(desk.dead.0.load(Ordering::SeqCst) > before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_busy_relay_with_a_down_backup_still_reads_as_busy() {
    let (main, b1) = (FakeNtfy::start().await, FakeNtfy::start().await);
    main.set_limited(true);
    let (desk, _bridge) = pair(&main, &[&b1], &[&b1], &[]);
    let err = desk.wire.send(&desk.link, &answer(1)).await.unwrap_err();
    assert!(peer::is_busy(&err), "{err}");
}

#[test]
fn only_a_build_that_listens_on_the_backups_counts() {
    for v in ["0.2.38", "0.2.39", "0.3.0", "1.0.0", "v0.2.38", "0.2.38-beta.1"] {
        assert!(peer::hears_backups(v), "{v}");
    }
    for v in ["", "0.2.37", "0.1.99", "junk", "0.2"] {
        assert!(!peer::hears_backups(v), "{v}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_work_leaves_the_main_relay_to_requests_when_the_other_end_hears_the_backups() {
    let (main, b1, b2) = (FakeNtfy::start().await, FakeNtfy::start().await, FakeNtfy::start().await);
    let (desk, bridge) = pair(&main, &[&b1, &b2], &[], &[]);
    let entries = || Msg::Entries { entries: crate::sync::store::EntryMap::new() };
    let set = |v: &str, heard: &[&FakeNtfy]| {
        let heard: Vec<String> = heard.iter().map(|n| n.url.clone()).collect();
        desk.wire
            .links
            .update(&desk.link.id, |p| {
                p.version = v.into();
                p.since_on = heard.into_iter().map(|u| (u, "m00000000001".to_string())).collect();
            })
            .unwrap();
    };
    let counts = || (main.published().len(), b1.published().len(), b2.published().len());
    // An older build at the other end: background still goes everywhere.
    set("0.2.37", &[&b1]);
    desk.wire.send(&desk.link, &entries()).await.unwrap();
    super::tests::until("on all three", || counts() == (1, 1, 1)).await;
    // A new build, but never heard on a backup yet: everywhere too.
    set("0.2.38", &[]);
    desk.wire.send(&desk.link, &entries()).await.unwrap();
    super::tests::until("on all three again", || counts() == (2, 2, 2)).await;
    // Heard on one backup: background only there, requests everywhere.
    set("0.2.38", &[&b1]);
    desk.wire.send(&desk.link, &entries()).await.unwrap();
    super::tests::until("on the backup it was heard on", || counts() == (2, 3, 2)).await;
    desk.wire.send(&desk.link, &answer(1)).await.unwrap();
    super::tests::until("the answer on all three", || counts() == (3, 4, 3)).await;
    // That backup takes nothing: the main relay still carries it.
    b1.set_limited(true);
    desk.wire.send(&desk.link, &entries()).await.unwrap();
    super::tests::until("on the main relay", || main.published().len() == 4).await;
    drop(bridge);
}

#[test]
fn the_backups_allow_more_background_work_than_ntfy_sh() {
    let t0 = std::time::Instant::now();
    let mut main = relay::Allowance::for_relay(crate::social::listen::DEFAULT_RELAY);
    let mut backup = relay::Allowance::for_relay(crate::social::listen::BACKUP_RELAYS[0]);
    assert!(main.take(relay::ALLOWANCE_MOST as usize, t0) && !main.take(1, t0));
    assert!(backup.take(relay::BACKUP_ALLOWANCE_MOST as usize, t0) && !backup.take(1, t0));
    assert!(relay::BACKUP_ALLOWANCE_PER_HOUR > relay::ALLOWANCE_PER_HOUR);
}

#[test]
fn a_link_is_heard_from_when_its_newest_frame_came_in() {
    let mut p: super::links::Paired = serde_json::from_value(serde_json::json!({
        "id": "l", "relay": "r", "side": "bridge", "me": "m", "name": "PC", "pairedAt": 1_000
    }))
    .unwrap();
    assert_eq!(super::run::last_heard_s(&p), 1_000, "never heard: when it was paired");
    // Counters are the sender's clock in microseconds.
    p.guard.last.insert("desk".into(), 1_800_000_000_000_000);
    p.guard.last.insert("desk.lan".into(), 1_700_000_000_000_000);
    assert_eq!(super::run::last_heard_s(&p), 1_800_000_000);
}
