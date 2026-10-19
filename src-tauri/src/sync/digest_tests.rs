//! Digests, key lists and the outbox: what two devices compare to find
//! what to swap.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;
use tempfile::TempDir;

use super::store::{self, Entry, EntryMap, HIDDEN, Hidden, PINS, SETTINGS, Stamp, SyncStore, diff_keys, digest_of};
use crate::db::DbHandle;
use crate::homeside::links::{BATCH, Outbox, batches};
use crate::sync::pins::Pin;

struct Dev {
    store: SyncStore,
    db: DbHandle,
    clock: Arc<AtomicU64>,
    _dir: TempDir,
}

fn dev(id: &str) -> Dev {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("lib.db")).unwrap();
    let clock = Arc::new(AtomicU64::new(1_000_000));
    let c = clock.clone();
    let store = SyncStore::with_clock(db.clone(), id, Arc::new(move || c.load(Ordering::SeqCst)));
    Dev { store, db, clock, _dir: dir }
}

fn pin(id: &str) -> Pin {
    serde_json::from_value(json!({"kind": "album", "id": id, "title": id, "order": 1})).unwrap()
}

fn stamp(k: &str, ts: u64, d: &str) -> Stamp {
    (k.to_string(), ts, d.to_string())
}

#[test]
fn equal_stores_give_equal_digests() {
    let a = dev("a");
    let b = dev("b");
    a.store.put(&PINS, "album:1", &pin("1")).unwrap();
    a.clock.fetch_add(10, Ordering::SeqCst);
    a.store.put(&SETTINGS, "accent", &json!("#c90")).unwrap();
    let ns = [PINS.name(), SETTINGS.name(), HIDDEN.name()];
    assert_ne!(a.store.digest(&ns).unwrap(), b.store.digest(&ns).unwrap());
    b.store.merge(&a.store.entries(&ns).unwrap(), &ns).unwrap();
    let (da, db) = (a.store.digest(&ns).unwrap(), b.store.digest(&ns).unwrap());
    assert_eq!(da, db);
    assert_eq!(da.len(), 3, "an empty namespace has a digest too");
    assert_eq!(da[HIDDEN.name()].n, 0);
    assert_eq!(da[PINS.name()].n, 1);
}

#[test]
fn digests_dont_depend_on_order() {
    let list = vec![stamp("b", 2, "x"), stamp("a", 1, "y"), stamp("c", 3, "x")];
    let mut rev = list.clone();
    rev.reverse();
    assert_eq!(digest_of(&list), digest_of(&rev));
    // Any change to a key, a time or a device changes it.
    for other in [
        vec![stamp("b", 2, "x"), stamp("a", 1, "y"), stamp("d", 3, "x")],
        vec![stamp("b", 2, "x"), stamp("a", 1, "y"), stamp("c", 4, "x")],
        vec![stamp("b", 2, "x"), stamp("a", 1, "z"), stamp("c", 3, "x")],
        vec![stamp("b", 2, "x"), stamp("a", 1, "y")],
    ] {
        assert_ne!(digest_of(&list), digest_of(&other), "{other:?}");
    }
    assert_eq!(digest_of(&[]).n, 0);
}

#[test]
fn deletions_count_in_digests_and_key_lists() {
    let a = dev("a");
    let b = dev("b");
    a.store.put(&HIDDEN, "track:1", &Hidden { kind: "track".into(), ..Default::default() }).unwrap();
    b.store.merge(&a.store.entries(&[HIDDEN.name()]).unwrap(), &[HIDDEN.name()]).unwrap();
    let before = a.store.digest(&[HIDDEN.name()]).unwrap();
    a.clock.fetch_add(5, Ordering::SeqCst);
    assert!(a.store.remove(&HIDDEN, "track:1").unwrap());
    let after = a.store.digest(&[HIDDEN.name()]).unwrap();
    assert_ne!(before, after, "a deletion changes the digest");
    assert_eq!(after[HIDDEN.name()].n, 1, "the tombstone counts");
    let d = diff_keys(&a.store.stamps(HIDDEN.name()).unwrap(), &b.store.stamps(HIDDEN.name()).unwrap());
    assert_eq!(d.send, ["track:1"], "the deletion goes to the other device");
    assert!(d.want.is_empty());
    let sent = a.store.entries_for(HIDDEN.name(), &d.send).unwrap();
    assert!(sent[HIDDEN.name()]["track:1"].is_tombstone());
}

#[test]
fn key_lists_say_what_to_send_and_what_to_ask_for() {
    let mine = vec![stamp("same", 5, "a"), stamp("newer-here", 9, "a"), stamp("only-here", 1, "a"), stamp("older-here", 2, "a")];
    let theirs = vec![stamp("same", 5, "a"), stamp("newer-here", 8, "b"), stamp("only-there", 1, "b"), stamp("older-here", 3, "b")];
    let d = diff_keys(&mine, &theirs);
    assert_eq!(d.send, ["newer-here", "only-here"]);
    assert_eq!(d.want, ["older-here", "only-there"]);
    // Same time: the larger device id wins, the same way on both ends.
    let d = diff_keys(&[stamp("k", 5, "a")], &[stamp("k", 5, "b")]);
    assert_eq!((d.send.len(), d.want.as_slice()), (0, ["k".to_string()].as_slice()));
    let d = diff_keys(&[stamp("k", 5, "b")], &[stamp("k", 5, "a")]);
    assert_eq!((d.send.as_slice(), d.want.len()), (["k".to_string()].as_slice(), 0));
}

#[test]
fn the_outbox_keeps_the_same_key_apart_in_each_namespace() {
    let a = dev("a");
    // One key in two namespaces.
    a.store.put(&PINS, "album:1", &pin("1")).unwrap();
    a.store.put(&HIDDEN, "album:1", &Hidden { kind: "album".into(), ..Default::default() }).unwrap();
    let ns = [PINS.name(), HIDDEN.name()];
    let ob = Outbox::new(&a.db, "link1");
    let first = ob.pending(&a.store, &ns).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].values().map(|m| m.len()).sum::<usize>(), 2);
    // The other end has the pin only: the hidden item still goes.
    let pins_only: EntryMap = first[0].iter().filter(|(n, _)| n.as_str() == PINS.name()).map(|(n, m)| (n.clone(), m.clone())).collect();
    ob.mark(&pins_only).unwrap();
    let left = ob.pending(&a.store, &ns).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].keys().collect::<Vec<_>>(), [HIDDEN.name()]);
    // A key list from the other end marks too.
    ob.mark_stamps(HIDDEN.name(), &a.store.stamps(HIDDEN.name()).unwrap()).unwrap();
    assert!(ob.pending(&a.store, &ns).unwrap().is_empty());
    // A change goes again.
    a.clock.fetch_add(10, Ordering::SeqCst);
    a.store.put(&HIDDEN, "album:1", &Hidden { kind: "album".into(), title: "x".into(), ..Default::default() }).unwrap();
    assert_eq!(ob.pending(&a.store, &ns).unwrap().len(), 1);
    // A lost outbox sends everything again.
    ob.clear().unwrap();
    assert_eq!(ob.pending(&a.store, &ns).unwrap()[0].values().map(|m| m.len()).sum::<usize>(), 2);
}

#[test]
fn an_outbox_from_before_namespaces_still_reads_as_lidarr() {
    let a = dev("a");
    a.store.put(&store::LIDARR, "artist:1", &json!({"mode": "protected"})).unwrap();
    let e = a.store.entries(&[store::LIDARR.name()]).unwrap()[store::LIDARR.name()]["artist:1"].clone();
    let old = json!({"artist:1": [e.ts, e.device]}).to_string();
    a.db.write(|c| crate::db::store::kv_set(c, "bridge.sent.old", &old)).unwrap();
    let ob = Outbox::new(&a.db, "old");
    assert!(ob.pending(&a.store, &[store::LIDARR.name()]).unwrap().is_empty());
}

#[test]
fn secrets_never_leave_through_the_outbox() {
    let a = dev("a");
    // Written straight into the table, as a damaged or very old row could be.
    a.db
        .write(|c| {
            c.execute(
                "INSERT INTO sync_entries (ns, key, value, ts, device) VALUES ('settings', 'odd', '{\"plexToken\":\"x\"}', 5, 'a')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    a.store.put(&SETTINGS, "accent", &json!("#c90")).unwrap();
    let out = Outbox::new(&a.db, "l").pending(&a.store, &[SETTINGS.name()]).unwrap();
    let keys: Vec<&String> = out.iter().flat_map(|b| b.values().flat_map(|m| m.keys())).collect();
    assert_eq!(keys, ["accent"]);
}

#[test]
fn batches_stay_small() {
    let mut all = EntryMap::new();
    for i in 0..(BATCH + 10) {
        all.entry("pins".into()).or_default().insert(format!("k{i:04}"), Entry { value: Some(json!(i)), ts: 1, device: "a".into() });
    }
    let b = batches(all);
    assert_eq!(b.len(), 2);
    assert_eq!(b[0]["pins"].len(), BATCH);
    // Big values cut batches by size too.
    let big = "x".repeat(60 * 1024);
    let mut all = EntryMap::new();
    for i in 0..5 {
        all.entry("lyrics".into()).or_default().insert(format!("t{i}"), Entry { value: Some(json!(big)), ts: 1, device: "a".into() });
    }
    let b = batches(all);
    assert!(b.len() >= 2, "{}", b.len());
    assert_eq!(b.iter().map(|m| m["lyrics"].len()).sum::<usize>(), 5);
}

// ---------------------------------------------------------------- a newer app

fn future_entries() -> EntryMap {
    let mut m = EntryMap::new();
    let e = |v: serde_json::Value, ts| Entry { value: Some(v), ts, device: "newer".into() };
    m.entry("savedLooks".into()).or_default().insert("look:1".into(), e(json!({"name": "Night", "accent": "#123"}), 50));
    m.entry("savedLooks".into()).or_default().insert("look:2".into(), Entry { value: None, ts: 60, device: "newer".into() });
    // A key and a value shape this build doesn't know, in a namespace it does.
    m.entry("pins".into()).or_default().insert("crate:9".into(), e(json!({"shape": "future"}), 70));
    // Names no build would make.
    m.entry("../escape".into()).or_default().insert("k".into(), e(json!(1), 80));
    m.entry("x".repeat(80)).or_default().insert("k".into(), e(json!(1), 80));
    // A secret, in an unknown namespace too.
    m.entry("vault".into()).or_default().insert("k".into(), e(json!({"apiKey": "nope"}), 90));
    m
}

#[test]
fn a_newer_apps_namespaces_are_kept_and_passed_on_but_not_applied() {
    let b = dev("b");
    let mut rx = b.store.subscribe();
    let changed = b.store.merge(&future_entries(), &[PINS.name()]).unwrap();
    assert_eq!(changed, ["pins"], "only a namespace this build knows counts as changed");
    let _ = rx.try_recv();
    assert_eq!(b.store.unknown_namespaces().unwrap(), ["savedLooks"], "bad names and secrets are dropped");
    // The unknown value is kept but left out of the list.
    assert!(b.store.list(&PINS).unwrap().is_empty());
    assert!(b.store.has_entry(&PINS, "crate:9").unwrap());
    // Digests and key lists carry it.
    let d = b.store.digest(&["savedLooks"]).unwrap();
    assert_eq!(d["savedLooks"].n, 2, "the deletion counts");
    assert_eq!(b.store.stamps("savedLooks").unwrap().len(), 2);
    // A third device gets all of it from this one.
    let c = dev("c");
    let ns = ["savedLooks", PINS.name()];
    c.store.merge(&b.store.entries(&ns).unwrap(), &[PINS.name()]).unwrap();
    assert_eq!(c.store.digest(&ns).unwrap(), b.store.digest(&ns).unwrap(), "nothing lost on the way");
    assert_eq!(c.store.entries(&["savedLooks"]).unwrap()["savedLooks"]["look:1"].value, Some(json!({"name": "Night", "accent": "#123"})));
    // An older entry doesn't replace a later one in the kept namespace.
    let mut old = EntryMap::new();
    old.entry("savedLooks".into()).or_default().insert("look:1".into(), Entry { value: Some(json!("old")), ts: 1, device: "a".into() });
    c.store.merge(&old, &[]).unwrap();
    assert_eq!(c.store.entries(&["savedLooks"]).unwrap()["savedLooks"]["look:1"].ts, 50);
    // The unknown ones never land in the known table.
    let n: i64 = c.db.read(|x| Ok(x.query_row("SELECT count(*) FROM sync_entries WHERE ns = 'savedLooks'", [], |r| r.get(0))?)).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn a_namespace_this_build_learns_is_taken_in() {
    let b = dev("b");
    // As an older build kept it, before this build knew "pins".
    b.db
        .write(|c| {
            c.execute(
                "INSERT INTO sync_entries_unknown (ns, key, value, ts, device) VALUES ('pins', 'album:7', ?1, 5, 'newer')",
                [json!({"kind": "album", "id": "7", "title": "Seven"}).to_string()],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(b.store.get(&PINS, "album:7").unwrap().is_none());
    assert_eq!(b.store.adopt_learned().unwrap(), ["pins"]);
    assert_eq!(b.store.get(&PINS, "album:7").unwrap().unwrap().title, "Seven");
    assert!(b.store.unknown_namespaces().unwrap().is_empty());
    assert!(b.store.adopt_learned().unwrap().is_empty());
}

#[test]
fn an_unknown_message_kind_or_field_is_ignored_not_an_error() {
    use std::io::Write as _;

    use crate::homeside::link::{Frame, Reassembler};
    use crate::homeside::wire::Msg;
    let m: Msg = serde_json::from_value(json!({"t": "syncTeleport", "anything": [1, 2]})).unwrap();
    assert!(matches!(m, Msg::Unknown));
    let h: Msg = serde_json::from_value(json!({
        "t": "syncHello", "proto": 9, "device": "d", "name": "Studio PC", "clockMs": 5,
        "digests": {}, "futureField": {"x": 1}
    }))
    .unwrap();
    match h {
        Msg::SyncHello(h) => assert_eq!((h.proto, h.answer, h.acct.as_str()), (9, false, "")),
        _ => panic!("not a hello"),
    }
    // Through a frame, as it would arrive: decoded, not dropped.
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(br#"{"t":"syncTeleport","x":1}"#).unwrap();
    let packed = e.finish().unwrap();
    let f = Frame { f: "peer".into(), c: 1, x: 2, i: "i".into(), n: 0, t: 1, d: crate::social::listen::seal::b64(&packed) };
    assert!(matches!(Reassembler::default().add(f, 1), Some(Msg::Unknown)));
}
