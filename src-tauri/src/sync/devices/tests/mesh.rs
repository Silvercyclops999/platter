//! Three of the user's devices, each paired with the others (or in a
//! line), a long time apart, and unpairing. Every message is counted.

use std::collections::{BTreeMap, HashMap, VecDeque};

use super::*;
use crate::remote::peer::sync::{LanBody, PeerSync};

/// Devices by index, and each one's end of its link to another.
struct Mesh {
    devs: Vec<Dev>,
    links: HashMap<(usize, usize), Link>,
}

impl Mesh {
    fn new(n: usize, pairs: &[(usize, usize)]) -> Self {
        let devs: Vec<Dev> = (0..n).map(|i| dev(&format!("{}", (b'a' + i as u8) as char))).collect();
        let mut links = HashMap::new();
        for &(i, j) in pairs {
            let (li, lj) = pair(&devs[i], &devs[j], "http://relay");
            links.insert((i, j), li);
            links.insert((j, i), lj);
        }
        Self { devs, links }
    }

    fn link(&self, from: usize, to: usize) -> &Link {
        &self.links[&(from, to)]
    }

    fn all_same(&self) -> bool {
        self.devs.windows(2).all(|w| same(&w[0], &w[1]))
    }
}

/// What crossed one link one way, by message kind.
type Count = BTreeMap<(usize, usize, &'static str), usize>;

/// Deliver messages, then let every device send what it has that its
/// other devices don't (as it would a moment after a change), until no
/// device has anything to say. Every message is counted.
async fn settle(m: &Mesh, start: Vec<(usize, usize, Msg)>, count: &mut Count) -> usize {
    let mut q: VecDeque<(usize, usize, Msg)> = start.into();
    let mut n = 0;
    loop {
        while let Some((from, to, msg)) = q.pop_front() {
            n += 1;
            assert!(n < 2000, "the devices keep talking");
            *count.entry((from, to, msg.kind())).or_default() += 1;
            m.devs[from].peer.sent(m.link(from, to), &msg);
            for r in m.devs[to].peer.handle(m.link(to, from), msg).await {
                q.push_back((to, from, r));
            }
        }
        for &(i, j) in m.links.keys() {
            for msg in m.devs[i].peer.push(m.link(i, j)).await {
                q.push_back((i, j, msg));
            }
        }
        if q.is_empty() {
            return n;
        }
    }
}

/// A round on every link, started by the lower end.
async fn rounds(m: &Mesh, count: &mut Count) -> usize {
    let mut n = 0;
    let mut pairs: Vec<(usize, usize)> = m.links.keys().copied().filter(|(i, j)| i < j).collect();
    pairs.sort();
    for (i, j) in pairs {
        let hello = m.devs[i].peer.hello(false).await.unwrap();
        n += settle(m, vec![(i, j, hello)], count).await;
    }
    n
}

fn entries_sent(c: &Count) -> usize {
    c.iter().filter(|((_, _, k), _)| *k == "syncEntries").map(|(_, n)| n).sum()
}

#[tokio::test]
async fn a_change_on_one_of_three_reaches_the_other_two_once_and_then_all_is_quiet() {
    let m = Mesh::new(3, &[(0, 1), (0, 2), (1, 2)]);
    let mut c = Count::new();
    rounds(&m, &mut c).await;
    assert!(m.all_same());
    // One change on a.
    tick(&m.devs[0], 10);
    m.devs[0].store.put(&HIDDEN, "track:1", &hidden("One")).unwrap();
    let mut c = Count::new();
    settle(&m, Vec::new(), &mut c).await;
    assert!(m.all_same(), "b and c have it");
    assert!(m.devs[1].store.has_entry(&HIDDEN, "track:1").unwrap() && m.devs[2].store.has_entry(&HIDDEN, "track:1").unwrap());
    // It went from a to each other device once; b and c pass it on to each
    // other at most once each way, and nothing goes back to a.
    for to in [1, 2] {
        assert_eq!(c.get(&(0, to, "syncEntries")), Some(&1), "a to {to} once: {c:?}");
    }
    for from in [1, 2] {
        assert_eq!(c.get(&(from, 0, "syncEntries")), None, "nothing back to a: {c:?}");
    }
    assert!(c.get(&(1, 2, "syncEntries")).unwrap_or(&0) <= &1 && c.get(&(2, 1, "syncEntries")).unwrap_or(&0) <= &1, "{c:?}");
    assert!(entries_sent(&c) <= 4, "{c:?}");
    // Nothing more to say: no loop, no resend.
    let mut c = Count::new();
    assert_eq!(settle(&m, Vec::new(), &mut c).await, 0, "{c:?}");
    // A round on every link is two hellos each, and nothing else.
    let mut c = Count::new();
    assert_eq!(rounds(&m, &mut c).await, 6, "{c:?}");
    assert_eq!(entries_sent(&c), 0);
}

#[tokio::test]
async fn a_change_reaches_a_device_two_hops_away_through_the_middle_one() {
    // a and c aren't paired with each other, only with b.
    let m = Mesh::new(3, &[(0, 1), (1, 2)]);
    let mut c = Count::new();
    rounds(&m, &mut c).await;
    tick(&m.devs[2], 10);
    m.devs[2].store.put(&SETTINGS, "accent", &json!("#0a7c5f")).unwrap();
    let mut c = Count::new();
    settle(&m, Vec::new(), &mut c).await;
    assert_eq!(m.devs[0].store.get(&SETTINGS, "accent").unwrap(), Some(json!("#0a7c5f")), "two hops");
    assert_eq!(entries_sent(&c), 2, "c to b, b to a, once each: {c:?}");
    let mut c = Count::new();
    assert_eq!(settle(&m, Vec::new(), &mut c).await, 0, "and nothing after: {c:?}");
}

#[tokio::test]
async fn edits_on_all_three_at_once_end_the_same_everywhere() {
    let m = Mesh::new(3, &[(0, 1), (0, 2), (1, 2)]);
    let mut c = Count::new();
    rounds(&m, &mut c).await;
    // The same key on all three, a few ms apart; something of its own on each.
    for (i, d) in m.devs.iter().enumerate() {
        tick(d, 5 * (i as u64 + 1));
        d.store.put(&SETTINGS, "theme", &json!(format!("look {i}"))).unwrap();
        d.store.put(&HIDDEN, &format!("track:{i}"), &hidden("mine")).unwrap();
    }
    let mut c = Count::new();
    settle(&m, Vec::new(), &mut c).await;
    assert!(m.all_same());
    for d in &m.devs {
        assert_eq!(d.store.get(&SETTINGS, "theme").unwrap(), Some(json!("look 2")), "the latest edit wins everywhere");
        assert_eq!(d.store.list(&HIDDEN).unwrap().len(), 3);
    }
    // Each entry crossed each link at most once each way.
    for ((from, to, kind), n) in &c {
        if *kind == "syncEntries" {
            assert!(*n <= 3, "{from} to {to}: {n} messages: {c:?}");
        }
    }
    let mut c = Count::new();
    assert_eq!(settle(&m, Vec::new(), &mut c).await, 0, "{c:?}");
}

// ---------------------------------------------------------------- a long time apart

const DAY: u64 = 86_400_000;

fn set_clock(d: &Dev, ms: u64) {
    d.clock.store(ms, Ordering::SeqCst);
}

/// a is used every day; b is put away for 100 days. Meanwhile a deletes
/// something they both had and, long enough ago, forgets the deletion.
async fn apart_100_days(lyrics_off_on_b: bool) -> (Dev, Dev, Link, Link) {
    let a = dev("a");
    let b = dev("b");
    let t0 = 1_000 * DAY;
    set_clock(&a, t0);
    set_clock(&b, t0);
    let (la, lb) = pair(&a, &b, "http://relay");
    // Made a while before they were last in step.
    set_clock(&a, t0 - 10 * DAY);
    a.store.put(&HIDDEN, "track:old", &hidden("Deleted later")).unwrap();
    a.store.put(&HIDDEN, "track:keep", &hidden("Kept")).unwrap();
    a.store.put(&store::LYRICS, "song:1", &store::UserLyricsEntry { text: "words".into(), prefer: false }).unwrap();
    set_clock(&a, t0);
    b.groups.lock().lyrics = !lyrics_off_on_b;
    round(&a, &la, &b, &lb).await;
    assert!(b.store.has_entry(&HIDDEN, "track:old").unwrap());
    b.groups.lock().lyrics = true;
    // a deletes it on day 5, adds something on day 20.
    set_clock(&a, t0 + 5 * DAY);
    assert!(a.store.remove(&HIDDEN, "track:old").unwrap());
    set_clock(&a, t0 + 20 * DAY);
    a.store.put(&HIDDEN, "track:new", &hidden("Added while apart")).unwrap();
    // b, away, has something of its own on day 50.
    set_clock(&b, t0 + 50 * DAY);
    b.store.put(&HIDDEN, "track:phone", &hidden("Hidden on the phone")).unwrap();
    // Day 100: a has forgotten the deletion (kept 90 days).
    set_clock(&a, t0 + 100 * DAY);
    set_clock(&b, t0 + 100 * DAY);
    assert_eq!(a.store.prune(t0 + 10 * DAY).unwrap(), 1);
    (a, b, la, lb)
}

#[tokio::test]
async fn back_after_a_long_time_a_deleted_and_forgotten_item_stays_gone() {
    for b_starts in [true, false] {
        let (a, b, la, lb) = apart_100_days(false).await;
        if b_starts {
            round(&b, &lb, &a, &la).await;
        } else {
            round(&a, &la, &b, &lb).await;
        }
        for d in [&a, &b] {
            assert!(!d.store.has_entry(&HIDDEN, "track:old").unwrap(), "{}: not brought back (b starts {b_starts})", d.id);
            for k in ["track:keep", "track:new", "track:phone"] {
                assert!(d.store.has_entry(&HIDDEN, k).unwrap(), "{}: {k} (b starts {b_starts})", d.id);
            }
        }
        assert!(same(&a, &b), "b starts {b_starts}");
        // It says so until the next round finds them fully in step.
        let wire = Arc::new(Wire::new(b.links.clone(), Arc::new(Ntfy::new().unwrap()), Arc::new(|| 1)));
        let sync = DeviceSync::new(b.peer.clone(), wire, Timing::default());
        assert_eq!(sync.status()[0].words, "Catching up after a long time away");
        round(&b, &lb, &a, &la).await;
        assert_eq!(sync.status()[0].words, "In sync");
        assert!(b.links.get(&lb.id).unwrap().catching_up_since.is_none());
    }
}

#[tokio::test]
async fn back_after_a_long_time_what_wasnt_kept_in_step_then_still_comes() {
    // b had lyrics switched off when they were last in step: a's lyrics
    // aren't something b let go of, so they come over.
    let (a, b, la, lb) = apart_100_days(true).await;
    round(&b, &lb, &a, &la).await;
    assert!(b.store.has_entry(&store::LYRICS, "song:1").unwrap());
    assert!(a.store.has_entry(&store::LYRICS, "song:1").unwrap(), "and a keeps them");
    assert!(!b.store.has_entry(&HIDDEN, "track:old").unwrap());
}

#[tokio::test]
async fn a_round_cut_short_after_a_long_time_apart_still_goes_carefully() {
    let (a, b, la, lb) = apart_100_days(false).await;
    // Half the messages of the first round are lost.
    let mut tap = Vec::new();
    let h = b.peer.hello_to(&lb.id, false).await.into_iter().collect();
    talk(&b, &lb, &a, &la, h, Net { drop_odd: true, ..Default::default() }, &mut tap).await;
    assert!(b.links.get(&lb.id).unwrap().catching_up_since.is_some(), "still catching up");
    round(&a, &la, &b, &lb).await;
    round(&b, &lb, &a, &la).await;
    assert!(same(&a, &b));
    assert!(!a.store.has_entry(&HIDDEN, "track:old").unwrap() && !b.store.has_entry(&HIDDEN, "track:old").unwrap());
    assert!(a.store.has_entry(&HIDDEN, "track:phone").unwrap());
}

#[tokio::test]
async fn a_new_pairing_copies_old_entries_as_usual() {
    // Never in step before: nothing is let go of, however old.
    let a = dev("a");
    let b = dev("b");
    set_clock(&a, 1_000 * DAY);
    a.store.put(&HIDDEN, "track:ancient", &hidden("old")).unwrap();
    set_clock(&a, 1_200 * DAY);
    set_clock(&b, 1_200 * DAY);
    let (la, lb) = pair(&a, &b, "http://relay");
    round(&b, &lb, &a, &la).await;
    assert!(b.store.has_entry(&HIDDEN, "track:ancient").unwrap());
    assert!(a.links.get(&la.id).unwrap().catching_up_since.is_none());
}

// ---------------------------------------------------------------- unpair and forget

#[tokio::test]
async fn unpairing_tells_the_other_device_and_both_forget_the_link() {
    let m = Mesh::new(3, &[(0, 1), (0, 2), (1, 2)]);
    m.devs[0].store.put(&HIDDEN, "track:a", &hidden("from a")).unwrap();
    let mut c = Count::new();
    rounds(&m, &mut c).await;
    let (a, b) = (&m.devs[0], &m.devs[1]);
    let (la, lb) = (m.link(0, 1).clone(), m.link(1, 0).clone());
    assert!(a.db.read(|c| crate::db::store::kv_get(c, &format!("bridge.sent.{}", la.id))).unwrap().is_some(), "an outbox to begin with");
    // a unpairs b: b hears the bye, a forgets.
    assert!(b.peer.handle(&lb, Msg::SyncBye).await.is_empty());
    a.peer.forget(&la.id).unwrap();
    for (d, l) in [(a, &la), (b, &lb)] {
        assert!(d.links.get(&l.id).is_none(), "{} forgot the link", d.id);
        assert!(d.db.read(|c| crate::db::store::kv_get(c, &format!("bridge.sent.{}", l.id))).unwrap().is_none(), "{} dropped its outbox", d.id);
        assert!(d.peer.seen(&l.id).is_none());
    }
    // The third device still syncs with both.
    tick(&m.devs[2], 10);
    m.devs[2].store.put(&HIDDEN, "track:c", &hidden("from c")).unwrap();
    let mut c = Count::new();
    for (i, j) in [(2usize, 0usize), (2, 1)] {
        let msgs = m.devs[2].peer.push(m.link(i, j)).await;
        settle_one(&m, i, j, msgs, &mut c).await;
    }
    assert!(a.store.has_entry(&HIDDEN, "track:c").unwrap() && b.store.has_entry(&HIDDEN, "track:c").unwrap());
    // a and b no longer talk: a's push to b has nowhere to go.
    assert!(a.links.links().iter().all(|l| l.id != la.id));
}

/// Messages on one link until it's quiet (not the whole mesh, whose links
/// may be gone).
async fn settle_one(m: &Mesh, from: usize, to: usize, start: Vec<Msg>, count: &mut Count) {
    let mut q: VecDeque<(usize, usize, Msg)> = start.into_iter().map(|x| (from, to, x)).collect();
    while let Some((f, t, msg)) = q.pop_front() {
        *count.entry((f, t, msg.kind())).or_default() += 1;
        m.devs[f].peer.sent(m.link(f, t), &msg);
        for r in m.devs[t].peer.handle(m.link(t, f), msg).await {
            q.push_back((t, f, r));
        }
    }
}

#[tokio::test]
async fn forgetting_a_device_that_is_offline_works_and_it_is_no_longer_answered() {
    let a = dev("a");
    let b = dev("b");
    // Nothing answers on the relay's port.
    let (la, lb) = pair(&a, &b, "http://127.0.0.1:9");
    a.links.update(&la.id, |p| p.device = Some("b".into())).unwrap();
    round(&a, &la, &b, &lb).await;
    // The relay can't be reached: the bye doesn't go, a forgets all the same.
    let wire = Arc::new(Wire::new(a.links.clone(), Arc::new(Ntfy::new().unwrap()), Arc::new(crate::sync::store::system_ms)));
    let sync = DeviceSync::new(a.peer.clone(), wire, Timing::default());
    sync.forget(&la.id).await.unwrap();
    assert!(a.links.all().is_empty());
    assert!(sync.status().is_empty());
    // b, which never heard, still has its end; what it sends isn't opened.
    assert!(b.links.get(&lb.id).is_some());
    let hello = b.peer.hello(false).await.unwrap();
    let wire_b = Wire::new(b.links.clone(), Arc::new(Ntfy::new().unwrap()), Arc::new(crate::sync::store::system_ms));
    let frames = wire_b.seal_lan(&lb, &hello, false).unwrap();
    match sync.exchange("b", LanBody { link: lb.id.clone(), msgs: vec![frames] }).await {
        Err(crate::error::AppError::NotFound(m)) => assert!(m.contains("isn't linked"), "{m}"),
        other => panic!("answered: {other:?}"),
    }
    let relay = link::seal_msg(&lb.outbox(), &lb.me, &mut Counter::default(), &hello, crate::sync::store::system_ms()).unwrap();
    assert!(sync.wire.receive(&la, &relay[0]).is_none(), "the relay frame isn't opened either");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unpairing_through_the_relay_reaches_the_other_device() {
    let ntfy = FakeNtfy::start().await;
    let a = real_clock("a", ACCT);
    let b = real_clock("b", ACCT);
    pair(&a, &b, &ntfy.url);
    let a = running(a);
    let b = running(b);
    until("the first round", || a.sync.status().first().is_some_and(|s| s.words == "In sync")).await;
    let id = a.dev.links.all()[0].id.clone();
    a.sync.forget(&id).await.unwrap();
    assert!(a.dev.links.all().is_empty());
    until("the other device to forget it too", || b.dev.links.all().is_empty()).await;
    let _ = a.stop.send(true);
    let _ = b.stop.send(true);
}
