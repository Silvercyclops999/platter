//! Device links: two or three of the user's devices, each with its own
//! database, store and clock, talking through an in-memory exchange or the
//! in-process ntfy (never the real ntfy.sh).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use tempfile::TempDir;

use super::protocol::{Paused, SyncDeps, SyncPeer};
use super::run::{DeviceSync, Timing};
use crate::db::DbHandle;
use crate::homeside::link::{self, Counter, Link, Side};
use crate::homeside::links::{Links, Outbox};
use crate::homeside::peer::Wire;
use crate::homeside::wire::Msg;
use crate::plex::keychain::{ACCOUNT_TOKEN, MemoryStore, TokenStore};
use crate::plex::token::Token;
use crate::social::listen::fake_ntfy::FakeNtfy;
use crate::social::listen::relay::{Ntfy, RelayFactory};
use crate::sync::SyncStore;
use crate::sync::engine::{AccountTag, Groups};
use crate::sync::store::{self, Entry, EntryMap, FOLLOWS, HIDDEN, Hidden, NAMESPACES, PINS, SETTINGS, looks_secret};

const ACCT: &str = "acct-tag-1";
const PLEX_TOKEN: &str = "plex-token-SECRET-1";
const LIDARR_KEY: &str = "lidarr-key-SECRET-2";
const LASTFM_KEY: &str = "lastfm-key-SECRET-3";

struct Tag(Mutex<Option<String>>);

#[async_trait]
impl AccountTag for Tag {
    async fn tag(&self) -> Option<String> {
        self.0.lock().clone()
    }
}

struct Dev {
    id: String,
    store: SyncStore,
    db: DbHandle,
    keys: Arc<MemoryStore>,
    clock: Arc<AtomicU64>,
    links: Links,
    peer: Arc<SyncPeer>,
    acct: Arc<Tag>,
    groups: Arc<Mutex<Groups>>,
    _dir: TempDir,
}

fn dev_with(id: &str, acct: Option<&str>, clock: Option<Arc<AtomicU64>>) -> Dev {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("lib.db")).unwrap();
    let clock = clock.unwrap_or_else(|| Arc::new(AtomicU64::new(1_000_000)));
    let c = clock.clone();
    let store = SyncStore::with_clock(db.clone(), id, Arc::new(move || c.load(Ordering::SeqCst)));
    let keys = Arc::new(MemoryStore::new());
    let links = Links::devices(db.clone(), keys.clone());
    let acct = Arc::new(Tag(Mutex::new(acct.map(str::to_string))));
    let groups = Arc::new(Mutex::new(Groups::default()));
    let g = groups.clone();
    let name = format!("Device {id}");
    let peer = Arc::new(SyncPeer::new(SyncDeps {
        store: store.clone(),
        db: db.clone(),
        links: links.clone(),
        account: acct.clone(),
        groups: Arc::new(move || *g.lock()),
        name: Arc::new(move || name.clone()),
        platform: "Windows".into(),
        version: "0.3.0".into(),
    }));
    Dev { id: id.into(), store, db, keys, clock, links, peer, acct, groups, _dir: dir }
}

fn dev(id: &str) -> Dev {
    dev_with(id, Some(ACCT), None)
}

/// Link two devices (as pairing would): one secret, an end each.
fn pair(a: &Dev, b: &Dev, relay: &str) -> (Link, Link) {
    let secret = crate::social::listen::seal::random32();
    let id = format!("{}-{}", a.id, b.id);
    let la = Link { id: id.clone(), secret, relay: relay.into(), side: Side::Device, me: format!("f{}", a.id), peer: format!("f{}", b.id) };
    let lb = Link { id, secret, relay: relay.into(), side: Side::Device, me: format!("f{}", b.id), peer: format!("f{}", a.id) };
    a.links.add(&la, &format!("Device {}", b.id), "", 1).unwrap();
    b.links.add(&lb, &format!("Device {}", a.id), "", 1).unwrap();
    (la, lb)
}

/// How messages travel in the in-memory exchange.
#[derive(Default, Clone, Copy)]
struct Net {
    /// Deliver the newest message first (out of order).
    lifo: bool,
    /// Lose every other message.
    drop_odd: bool,
}

/// Run messages between two devices until nobody has anything to say.
/// Every message is recorded as it would travel (before sealing).
async fn talk(a: &Dev, la: &Link, b: &Dev, lb: &Link, start: Vec<Msg>, net: Net, tap: &mut Vec<String>) -> usize {
    // (to b?, message)
    let mut q: VecDeque<(bool, Msg)> = start.into_iter().map(|m| (true, m)).collect();
    let mut n = 0;
    while let Some((to_b, m)) = if net.lifo { q.pop_back() } else { q.pop_front() } {
        n += 1;
        assert!(n < 500, "the devices keep talking");
        tap.push(serde_json::to_string(&m).unwrap());
        let (from, from_link, to, to_link) = if to_b { (a, la, b, lb) } else { (b, lb, a, la) };
        from.peer.sent(from_link, &m);
        if net.drop_odd && n % 2 == 1 {
            continue;
        }
        for r in to.peer.handle(to_link, m).await {
            q.push_back((!to_b, r));
        }
    }
    n
}

/// A round started by `a`.
async fn round(a: &Dev, la: &Link, b: &Dev, lb: &Link) -> Vec<String> {
    let mut tap = Vec::new();
    let hello = a.peer.hello(false).await.into_iter().collect();
    talk(a, la, b, lb, hello, Net::default(), &mut tap).await;
    tap
}

fn all_ns(d: &Dev) -> Vec<String> {
    let mut ns: Vec<String> = NAMESPACES.iter().map(|s| s.to_string()).collect();
    ns.extend(d.store.unknown_namespaces().unwrap());
    ns
}

fn same(a: &Dev, b: &Dev) -> bool {
    let mut ns = all_ns(a);
    ns.extend(all_ns(b));
    let refs: Vec<&str> = ns.iter().map(String::as_str).collect();
    a.store.digest(&refs).unwrap() == b.store.digest(&refs).unwrap()
}

fn hidden(title: &str) -> Hidden {
    Hidden { kind: "track".into(), title: title.into(), ..Default::default() }
}

fn tick(d: &Dev, ms: u64) {
    d.clock.fetch_add(ms, Ordering::SeqCst);
}

// ---------------------------------------------------------------- the protocol

#[tokio::test]
async fn two_devices_meet_then_only_changes_move() {
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, "http://relay");
    a.store.put(&SETTINGS, "accent", &json!("#c90")).unwrap();
    a.store.put(&HIDDEN, "track:1", &hidden("One")).unwrap();
    b.store.put(&HIDDEN, "track:2", &hidden("Two")).unwrap();
    round(&a, &la, &b, &lb).await;
    assert!(same(&a, &b));
    assert_eq!(b.store.get(&SETTINGS, "accent").unwrap(), Some(json!("#c90")));
    assert_eq!(a.store.get(&HIDDEN, "track:2").unwrap().unwrap().title, "Two");
    assert!(a.links.get(&la.id).unwrap().last_synced_at.is_some());
    assert_eq!(a.links.get(&la.id).unwrap().device.as_deref(), Some("b"), "learned the other device's id");
    // Nothing more to send either way.
    assert!(a.peer.push(&la).await.is_empty());
    assert!(b.peer.push(&lb).await.is_empty());
    // A change goes out once.
    tick(&b, 10);
    b.store.put(&SETTINGS, "accent", &json!("#09c")).unwrap();
    let out = b.peer.push(&lb).await;
    assert_eq!(out.len(), 1);
    let mut tap = Vec::new();
    talk(&b, &lb, &a, &la, out, Net::default(), &mut tap).await;
    assert_eq!(a.store.get(&SETTINGS, "accent").unwrap(), Some(json!("#09c")));
    assert!(b.peer.push(&lb).await.is_empty(), "not sent twice");
    assert!(a.peer.push(&la).await.is_empty(), "not sent back");
    // An equal round says hello and nothing else.
    let tap = round(&a, &la, &b, &lb).await;
    assert_eq!(tap.len(), 2, "{tap:?}");
}

#[tokio::test]
async fn one_entry_edited_on_both_ends_ends_the_same_either_way() {
    for (lifo, b_starts) in [(false, false), (true, false), (false, true), (true, true)] {
        let a = dev("a");
        let b = dev("b");
        let (la, lb) = pair(&a, &b, "http://relay");
        a.store.put(&SETTINGS, "theme", &json!("dark")).unwrap();
        tick(&b, 5);
        b.store.put(&SETTINGS, "theme", &json!("light")).unwrap();
        let mut tap = Vec::new();
        let net = Net { lifo, ..Default::default() };
        if b_starts {
            let h = b.peer.hello(false).await.into_iter().collect();
            talk(&b, &lb, &a, &la, h, net, &mut tap).await;
        } else {
            let h = a.peer.hello(false).await.into_iter().collect();
            talk(&a, &la, &b, &lb, h, net, &mut tap).await;
        }
        assert!(same(&a, &b), "lifo {lifo}, b starts {b_starts}");
        assert_eq!(a.store.get(&SETTINGS, "theme").unwrap(), Some(json!("light")), "the later edit wins");
    }
}

#[tokio::test]
async fn a_delete_on_one_end_and_an_edit_on_the_other_go_by_time() {
    for delete_later in [false, true] {
        let a = dev("a");
        let b = dev("b");
        let (la, lb) = pair(&a, &b, "http://relay");
        a.store.put(&HIDDEN, "track:1", &hidden("One")).unwrap();
        round(&a, &la, &b, &lb).await;
        if delete_later {
            tick(&b, 5);
            b.store.put(&HIDDEN, "track:1", &hidden("Edited")).unwrap();
            tick(&a, 50);
            assert!(a.store.remove(&HIDDEN, "track:1").unwrap());
        } else {
            tick(&a, 5);
            assert!(a.store.remove(&HIDDEN, "track:1").unwrap());
            tick(&b, 50);
            b.store.put(&HIDDEN, "track:1", &hidden("Edited")).unwrap();
        }
        round(&b, &lb, &a, &la).await;
        assert!(same(&a, &b));
        let got = a.store.get(&HIDDEN, "track:1").unwrap();
        if delete_later {
            assert!(got.is_none(), "the later delete wins");
        } else {
            assert_eq!(got.unwrap().title, "Edited", "the later edit wins");
        }
    }
}

#[tokio::test]
async fn offline_edits_on_both_sides_meet_when_they_reconnect() {
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, "http://relay");
    round(&a, &la, &b, &lb).await;
    // Both paths down: three edits each, nothing sent.
    for i in 0..3 {
        tick(&a, 7);
        a.store.put(&PINS, &format!("album:a{i}"), &serde_json::from_value(json!({"kind": "album", "id": format!("a{i}"), "title": "A"})).unwrap()).unwrap();
        tick(&b, 7);
        b.store.put(&FOLLOWS, &format!("b{i}"), &serde_json::from_value(json!({"id": format!("b{i}"), "title": "B", "followedAt": 1})).unwrap()).unwrap();
    }
    assert!(!same(&a, &b));
    round(&b, &lb, &a, &la).await;
    assert!(same(&a, &b));
    assert_eq!(a.store.list(&FOLLOWS).unwrap().len(), 3);
    assert_eq!(b.store.list(&PINS).unwrap().len(), 3);
}

#[tokio::test]
async fn lost_messages_and_a_lost_outbox_are_repaired_by_the_next_round() {
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, "http://relay");
    round(&a, &la, &b, &lb).await;
    for i in 0..30 {
        tick(&a, 3);
        a.store.put(&HIDDEN, &format!("track:{i}"), &hidden("x")).unwrap();
    }
    // The change goes out and is lost on the way (the relay dropped it, or
    // half its pieces never came).
    let out = a.peer.push(&la).await;
    let mut tap = Vec::new();
    talk(&a, &la, &b, &lb, out, Net { drop_odd: true, ..Default::default() }, &mut tap).await;
    assert!(!same(&a, &b));
    assert!(a.peer.push(&la).await.is_empty(), "the sender thinks it went");
    // A round with half its messages lost still makes progress; the next one finishes.
    let h = b.peer.hello(false).await.into_iter().collect();
    talk(&b, &lb, &a, &la, h, Net { drop_odd: true, ..Default::default() }, &mut tap).await;
    round(&b, &lb, &a, &la).await;
    assert!(same(&a, &b));
    // A reinstall loses the outbox: everything goes again once, and it still converges.
    Outbox::new(&a.db, &la.id).clear().unwrap();
    tick(&a, 3);
    a.store.put(&HIDDEN, "track:new", &hidden("new")).unwrap();
    let out = a.peer.push(&la).await;
    assert!(!out.is_empty());
    talk(&a, &la, &b, &lb, out, Net::default(), &mut tap).await;
    assert!(same(&a, &b));
    assert!(a.peer.push(&la).await.is_empty());
    // Half the pieces of a sealed message never come: nothing opens.
    let big = Msg::SyncEntries { entries: b.store.entries(&[HIDDEN.name()]).unwrap() };
    let bodies = link::seal_msg(&la.outbox(), &la.me, &mut Counter::default(), &noisy(big), 1_000).unwrap();
    assert!(bodies.len() > 2);
    let mut r = link::Reassembler::default();
    let got: Vec<Msg> = bodies.iter().step_by(2).filter_map(|x| r.add(link::open_frame(&lb.inbox(), x, 1_000).unwrap(), 1_000)).collect();
    assert!(got.is_empty());
}

/// A message too big for one relay piece.
fn noisy(m: Msg) -> Msg {
    let Msg::SyncEntries { mut entries } = m else { return m };
    let text: String = (0..20_000u32).map(|_| char::from(b'a' + rand::random::<u8>() % 26)).collect();
    entries.entry("lyrics".into()).or_default().insert("t1".into(), Entry { value: Some(json!({"text": text})), ts: 1, device: "a".into() });
    Msg::SyncEntries { entries }
}

#[tokio::test]
async fn a_newer_apps_data_and_messages_pass_through_to_a_third_device() {
    let a = dev("a"); // stands in for the newer app
    let b = dev("b");
    let c = dev("c");
    let (la, lb) = pair(&a, &b, "http://relay");
    let (lbc, lcb) = pair(&b, &c, "http://relay");
    // A namespace and a key shape only the newer app knows.
    let mut future = EntryMap::new();
    future.entry("savedLooks".into()).or_default().insert("look:1".into(), Entry { value: Some(json!({"name": "Night"})), ts: 900_000, device: "a".into() });
    future.entry("pins".into()).or_default().insert("crate:1".into(), Entry { value: Some(json!({"shape": 2})), ts: 900_000, device: "a".into() });
    a.store.merge(&future, &[PINS.name()]).unwrap();
    // It also says things this build doesn't understand.
    assert!(b.peer.handle(&lb, Msg::Unknown).await.is_empty());
    let Some(Msg::SyncHello(mut h)) = a.peer.hello(false).await else { panic!("no hello") };
    h.proto = 9;
    let mut tap = Vec::new();
    talk(&a, &la, &b, &lb, vec![Msg::SyncHello(h)], Net::default(), &mut tap).await;
    assert!(same(&a, &b), "nothing lost");
    assert_eq!(b.store.unknown_namespaces().unwrap(), ["savedLooks"]);
    // The third device gets it from the middle one.
    round(&b, &lbc, &c, &lcb).await;
    assert!(same(&b, &c));
    assert_eq!(c.store.entries(&["savedLooks"]).unwrap()["savedLooks"]["look:1"].value, Some(json!({"name": "Night"})));
    assert!(c.store.has_entry(&PINS, "crate:1").unwrap());
    // The other device is newer: the status says so.
    let wire = Arc::new(Wire::new(b.links.clone(), Arc::new(Ntfy::new().unwrap()), Arc::new(|| 1)));
    let sync = DeviceSync::new(b.peer.clone(), wire, Timing::default());
    let st = sync.status();
    let to_a = st.iter().find(|s| s.id == lb.id).unwrap();
    assert_eq!(to_a.update_other.as_deref(), Some("Update Platter on Device a to sync everything"));
}

#[tokio::test]
async fn a_device_of_another_account_is_refused_and_gets_nothing() {
    let a = dev("a");
    let b = dev_with("b", Some("someone-else"), None);
    let (la, lb) = pair(&a, &b, "http://relay");
    a.store.put(&SETTINGS, "accent", &json!("#c90")).unwrap();
    b.store.put(&HIDDEN, "track:1", &hidden("theirs")).unwrap();
    let tap = round(&a, &la, &b, &lb).await;
    assert_eq!(tap.len(), 1, "only the hello; no answer, no entries");
    let tap = round(&b, &lb, &a, &la).await;
    assert_eq!(tap.len(), 1);
    assert!(b.store.get(&SETTINGS, "accent").unwrap().is_none());
    assert!(a.store.get(&HIDDEN, "track:1").unwrap().is_none());
    assert_eq!(a.peer.seen(&la.id).unwrap().paused, Some(Paused::OtherAccount));
    // Entries it sends anyway aren't taken, and nothing is pushed to it.
    let forged = Msg::SyncEntries { entries: b.store.entries(&[HIDDEN.name()]).unwrap() };
    assert!(a.peer.handle(&la, forged).await.is_empty());
    assert!(a.store.get(&HIDDEN, "track:1").unwrap().is_none());
    assert!(a.peer.push(&la).await.iter().all(|m| matches!(m, Msg::SyncHello(_))));
    let wire = Arc::new(Wire::new(a.links.clone(), Arc::new(Ntfy::new().unwrap()), Arc::new(|| 1)));
    let st = DeviceSync::new(a.peer.clone(), wire, Timing::default()).status();
    assert_eq!(st[0].words, "This device is signed in to a different account. Sync paused");
    // Signed out: nothing goes at all.
    *a.acct.0.lock() = None;
    assert!(a.peer.hello(false).await.is_none());
}

#[tokio::test]
async fn a_group_switched_off_neither_sends_nor_takes_and_catches_up_later() {
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, "http://relay");
    b.groups.lock().settings = false;
    a.store.put(&SETTINGS, "accent", &json!("#c90")).unwrap();
    b.store.put(&SETTINGS, "theme", &json!("light")).unwrap();
    a.store.put(&HIDDEN, "track:1", &hidden("x")).unwrap();
    round(&a, &la, &b, &lb).await;
    round(&b, &lb, &a, &la).await;
    assert!(b.store.get(&SETTINGS, "accent").unwrap().is_none());
    assert!(a.store.get(&SETTINGS, "theme").unwrap().is_none());
    assert!(b.store.has_entry(&HIDDEN, "track:1").unwrap());
    // Pushed changes skip it too.
    tick(&a, 5);
    a.store.put(&SETTINGS, "accent", &json!("#fff")).unwrap();
    assert!(a.peer.push(&la).await.is_empty());
    // Back on: it catches up.
    b.groups.lock().settings = true;
    round(&b, &lb, &a, &la).await;
    assert_eq!(b.store.get(&SETTINGS, "accent").unwrap(), Some(json!("#fff")));
    assert_eq!(a.store.get(&SETTINGS, "theme").unwrap(), Some(json!("light")));
}

#[tokio::test]
async fn a_device_whose_clock_is_days_ahead_has_its_entries_held() {
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, "http://relay");
    a.clock.store(1_000 * 86_400_000, Ordering::SeqCst);
    b.clock.store(1_003 * 86_400_000, Ordering::SeqCst);
    b.store.put(&SETTINGS, "accent", &json!("#f00")).unwrap();
    round(&b, &lb, &a, &la).await;
    assert!(a.store.get(&SETTINGS, "accent").unwrap().is_none(), "nothing merged");
    assert_eq!(a.store.quarantined().unwrap().len(), 1);
    let wire = Arc::new(Wire::new(a.links.clone(), Arc::new(Ntfy::new().unwrap()), Arc::new(|| 1)));
    let st = DeviceSync::new(a.peer.clone(), wire, Timing::default()).status();
    assert_eq!(st[0].clock.as_deref(), Some("Device b's clock is about 3 days fast"));
    assert_eq!(st[0].held, 1);
    a.store.take_quarantined().unwrap();
    assert_eq!(a.store.get(&SETTINGS, "accent").unwrap(), Some(json!("#f00")));
}

#[tokio::test]
async fn nothing_secret_leaves_in_a_full_sync() {
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, "http://relay");
    // The vault: every key this device holds.
    a.keys.set(ACCOUNT_TOKEN, &Token::new(PLEX_TOKEN).unwrap()).unwrap();
    a.keys.set("lidarr-api-key", &Token::new(LIDARR_KEY).unwrap()).unwrap();
    a.keys.set("lastfm-api-key", &Token::new(LASTFM_KEY).unwrap()).unwrap();
    // Every synced setting, with addresses of the user's own servers.
    let mut s = crate::settings::Settings::default();
    s.lidarr_url = "http://10.20.30.40:8686".into();
    s.tautulli_url = "http://10.20.30.40:8181".into();
    crate::sync::settings::push(&a.store, &s).unwrap();
    a.store.put(&HIDDEN, "track:1", &hidden("x")).unwrap();
    a.store.put(&store::LIDARR, "artist:1", &json!({"mode": "protected"})).unwrap();
    let mut tap = round(&a, &la, &b, &lb).await;
    tap.extend(round(&b, &lb, &a, &la).await);
    tick(&a, 9);
    a.store.put(&HIDDEN, "track:2", &hidden("y")).unwrap();
    let out = a.peer.push(&la).await;
    talk(&a, &la, &b, &lb, out, Net::default(), &mut tap).await;
    assert!(same(&a, &b));
    assert!(tap.iter().any(|m| m.contains("10.20.30.40")), "server addresses do sync");
    let vault = [PLEX_TOKEN, LIDARR_KEY, LASTFM_KEY];
    for m in &tap {
        for v in vault {
            assert!(!m.contains(v), "a key travelled: {m}");
        }
        let msg: Msg = serde_json::from_str(m).unwrap();
        if let Msg::SyncEntries { entries } = msg {
            for (ns, es) in &entries {
                for (k, e) in es {
                    assert!(!e.value.as_ref().is_some_and(looks_secret), "{ns}/{k} looks secret");
                    let lower = k.to_ascii_lowercase();
                    for w in ["token", "password", "secret", "apikey", "api_key", "clientid"] {
                        assert!(!lower.contains(w), "{ns}/{k}");
                    }
                }
            }
        }
    }
    // A secret slipped into the table is never sent, and never taken.
    a.db
        .write(|c| {
            c.execute("INSERT INTO sync_entries (ns, key, value, ts, device) VALUES ('settings', 'odd', '{\"accessToken\":\"x\"}', 99999999, 'a')", [])?;
            Ok(())
        })
        .unwrap();
    let mut tap = round(&b, &lb, &a, &la).await;
    tap.extend(a.peer.push(&la).await.iter().map(|m| serde_json::to_string(m).unwrap()));
    assert!(tap.iter().all(|m| !m.contains("accessToken")));
    let mut evil = EntryMap::new();
    evil.entry("settings".into()).or_default().insert("odd2".into(), Entry { value: Some(json!({"apiKey": "x"})), ts: 2_000_000, device: "a".into() });
    b.peer.handle(&lb, Msg::SyncEntries { entries: evil }).await;
    assert!(!b.store.has_entry(&SETTINGS, "odd2").unwrap());
}

// ---------------------------------------------------------------- the relay

async fn until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..400 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

struct Running {
    dev: Dev,
    sync: Arc<DeviceSync>,
    stop: tokio::sync::watch::Sender<bool>,
}

fn running(d: Dev) -> Running {
    let wire = Arc::new(Wire::new(d.links.clone(), Arc::new(Ntfy::new().unwrap()) as Arc<dyn RelayFactory>, Arc::new(crate::sync::store::system_ms)));
    let sync = Arc::new(DeviceSync::new(d.peer.clone(), wire, Timing { debounce: Duration::from_millis(200), every: Duration::from_secs(1) }));
    let (stop, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(sync.clone().run(rx));
    Running { dev: d, sync, stop }
}

fn real_clock(id: &str, acct: &str) -> Dev {
    let c = Arc::new(AtomicU64::new(crate::sync::store::system_ms()));
    dev_with(id, Some(acct), Some(c))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_devices_keep_in_step_through_the_relay_sealed() {
    let ntfy = FakeNtfy::start().await;
    let a = real_clock("a", ACCT);
    let b = real_clock("b", ACCT);
    pair(&a, &b, &ntfy.url);
    a.store.put(&HIDDEN, "track:1", &hidden("Quiet Hours")).unwrap();
    let a = running(a);
    let b = running(b);
    until("the first round", || b.dev.store.has_entry(&HIDDEN, "track:1").unwrap()).await;
    tick(&b.dev, 10);
    b.dev.store.put(&SETTINGS, "accent", &json!("#0a7c5f")).unwrap();
    until("the change on the other device", || a.dev.store.get(&SETTINGS, "accent").unwrap() == Some(json!("#0a7c5f"))).await;
    until("in sync", || a.sync.status().first().is_some_and(|s| s.words == "In sync")).await;
    for m in ntfy.published() {
        assert!(m.body.starts_with("1."), "sealed");
        for plain in ["Quiet Hours", "#0a7c5f", "track:1", "accent", "Device a", "Device b", ACCT] {
            assert!(!m.body.contains(plain) && !m.topic.contains(plain), "{plain} travelled in the clear");
        }
    }
    let _ = a.stop.send(true);
    let _ = b.stop.send(true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_made_while_the_relay_is_down_or_busy_arrive_after_it_recovers() {
    let ntfy = FakeNtfy::start().await;
    let a = real_clock("a", ACCT);
    let b = real_clock("b", ACCT);
    pair(&a, &b, &ntfy.url);
    let a = running(a);
    let b = running(b);
    until("the first round", || a.sync.status().first().is_some_and(|s| s.words == "In sync")).await;
    ntfy.set_down(true);
    a.dev.store.put(&HIDDEN, "track:down", &hidden("while down")).unwrap();
    until("the relay to be missed", || a.sync.status()[0].words.contains("is away")).await;
    ntfy.set_down(false);
    ntfy.set_limited(true);
    tick(&a.dev, 5);
    a.dev.store.put(&HIDDEN, "track:busy", &hidden("while busy")).unwrap();
    until("the relay to say it's busy", || a.sync.status()[0].words == "The relay is busy, trying again later").await;
    // Local use isn't held up.
    assert!(a.dev.store.get(&HIDDEN, "track:busy").unwrap().is_some());
    ntfy.set_limited(false);
    until("both edits on the other device", || {
        b.dev.store.has_entry(&HIDDEN, "track:down").unwrap() && b.dev.store.has_entry(&HIDDEN, "track:busy").unwrap()
    })
    .await;
    let _ = a.stop.send(true);
    let _ = b.stop.send(true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_of_another_account_gets_nothing_through_the_relay() {
    let ntfy = FakeNtfy::start().await;
    let a = real_clock("a", ACCT);
    let b = real_clock("b", "someone-else");
    pair(&a, &b, &ntfy.url);
    a.store.put(&HIDDEN, "track:1", &hidden("mine")).unwrap();
    let a = running(a);
    let b = running(b);
    until("the refusal", || a.dev.peer.seen(&a.dev.links.all()[0].id).is_some_and(|s| s.paused == Some(Paused::OtherAccount))).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!b.dev.store.has_entry(&HIDDEN, "track:1").unwrap());
    let _ = a.stop.send(true);
    let _ = b.stop.send(true);
}

#[tokio::test]
async fn big_entries_wait_for_the_local_network_instead_of_the_relay() {
    let ntfy = FakeNtfy::start().await;
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, &ntfy.url);
    round(&a, &la, &b, &lb).await;
    let wire = Arc::new(Wire::new(a.links.clone(), Arc::new(Ntfy::new().unwrap()), Arc::new(crate::sync::store::system_ms)));
    let sync = DeviceSync::new(a.peer.clone(), wire, Timing::default());
    let noise = |n: u32| -> String { (0..n).map(|_| char::from(b'a' + rand::random::<u8>() % 26)).collect() };
    let mut msgs = Vec::new();
    for i in 0..3 {
        let mut m = EntryMap::new();
        m.entry("lyrics".into()).or_default().insert(format!("t{i}"), Entry { value: Some(json!({"text": noise(300_000)})), ts: 1, device: "a".into() });
        msgs.push(Msg::SyncEntries { entries: m });
    }
    sync.deliver(&la, msgs).await;
    let sent = ntfy.published().len();
    assert!(sent > 0);
    assert_eq!(sync.status()[0].words, "Waiting for Device b");
    // About 300 KB went; the rest waits.
    assert!(sent * link::PIECE < 2 * super::run::RELAY_MAX, "{sent} pieces");
}

#[test]
fn replays_and_tampered_frames_on_a_device_link_are_dropped() {
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, "http://relay");
    assert_eq!(la.outbox(), lb.inbox());
    assert_eq!(lb.outbox(), la.inbox());
    assert_ne!(la.outbox().topic, lb.outbox().topic, "a topic each");
    let wire = Wire::new(b.links.clone(), Arc::new(Ntfy::new().unwrap()), Arc::new(|| 5_000));
    let mut c = Counter::default();
    let x = link::seal_msg(&la.outbox(), &la.me, &mut c, &Msg::SyncBye, 5_000).unwrap().remove(0);
    let mut t = x.clone().into_bytes();
    t[12] = if t[12] == b'A' { b'B' } else { b'A' };
    assert!(wire.receive(&lb, std::str::from_utf8(&t).unwrap()).is_none(), "tampered");
    assert!(matches!(wire.receive(&lb, &x), Some(Msg::SyncBye)));
    assert!(wire.receive(&lb, &x).is_none(), "replayed");
    // Its own messages don't open as the other end's.
    let mine = link::seal_msg(&lb.outbox(), &lb.me, &mut Counter::default(), &Msg::SyncBye, 5_000).unwrap().remove(0);
    assert!(wire.receive(&lb, &mine).is_none());
    // A Bridge link's channels are not a device link's.
    let bridge = Link { side: Side::Desktop, ..la.clone() };
    assert_ne!(bridge.outbox(), la.outbox());
}

#[test]
fn device_links_are_kept_apart_from_bridge_links() {
    let a = dev("a");
    let b = dev("b");
    pair(&a, &b, "http://relay");
    let bridge = Links::new(a.db.clone(), a.keys.clone());
    assert!(bridge.all().is_empty(), "a Bridge never sees a device link");
    assert_eq!(a.links.links().len(), 1);
}
