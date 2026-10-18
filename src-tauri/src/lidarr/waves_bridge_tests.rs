//! "Bring in my listening" handed to a Platter Bridge: a desktop and a
//! Bridge, each with its own database, against the same faked Lidarr and
//! MusicBrainz. Plans and progress go straight between them here; the
//! sealed link they travel over has its own tests.

use std::sync::Arc;

use serde_json::{Value, json};

use super::Lidarr;
use super::mbpick::tests::{RG_EP, RG_SECOND};
use super::waves::{PassOutcome, Waves, WavesDeps};
use super::waves_bridge::{HANDOVER_WAIT, PlanAlbums, RETRY_AFTER, SILENT_S, WavesPlan};
use super::waves_tests::{Setup, Told, setup};
use crate::db::DbHandle;

const LINK: &str = "link-1";
const NAME: &str = "Shed Bridge";

struct Pair {
    t: Setup,
    _dir: tempfile::TempDir,
    bdb: DbHandle,
    bl: Lidarr,
    br: Waves,
    btold: Arc<Told>,
}

fn bridge_waves(t: &Setup, bl: &Lidarr, bdb: &DbHandle, told: Arc<Told>) -> Waves {
    let d = t.day.clone();
    Waves::with_albums(
        WavesDeps { lidarr: bl.clone(), notifier: told, day_of: Arc::new(move |_| *d.lock()), hidden: Arc::new(Vec::new) },
        Arc::new(PlanAlbums { db: bdb.clone() }),
    )
}

async fn pair(practice: bool, patch: Value) -> Pair {
    let t = setup(practice, patch).await;
    let (dir, bdb, bl) = t.r.bridge();
    // The desktop's modes reach the Bridge (shared Lidarr state).
    super::shared::tests::sync(&t.r.lidarr.inner.shared, &bl.inner.shared);
    bl.check().await.unwrap();
    let btold = Arc::new(Told::default());
    let br = bridge_waves(&t, &bl, &bdb, btold.clone());
    Pair { t, _dir: dir, bdb, bl, br, btold }
}

impl Pair {
    /// The desktop sends a plan, the Bridge takes it and answers.
    async fn hand(&self) -> WavesPlan {
        let plan = self.t.w.bridge_plan(LINK, NAME).await.unwrap().expect("a plan");
        self.br.take_plan(&plan).await.unwrap();
        self.t.w.bridge_progress(LINK, self.br.progress()).await.unwrap();
        plan
    }

    /// A pass on the Bridge; its progress reaches the desktop when `online`.
    async fn bridge_pass(&self, online: bool) -> PassOutcome {
        let out = self.br.pass().await.unwrap();
        if online {
            self.t.w.bridge_progress(LINK, self.br.progress()).await.unwrap();
        }
        out
    }

    fn advance(&self, s: i64) {
        *self.t.r.now.lock() += s;
    }

    /// Albums Lidarr has for a release group.
    fn in_lidarr(&self, rgid: &str) -> usize {
        self.t.r.web.st.lock().albums.iter().filter(|a| a["foreignAlbumId"] == rgid).count()
    }
}

#[tokio::test]
async fn handing_over_never_asks_for_an_album_twice() {
    let p = pair(false, json!({"bringInPerDay": 1})).await;
    // The desktop runs the first day itself.
    assert_eq!(p.t.w.pass().await.unwrap().requested, vec!["Second Wind"]);
    assert_eq!(p.in_lidarr(RG_SECOND), 1);
    let writes = p.t.r.web.writes().len();
    let plan = p.hand().await;
    assert!(plan.run && plan.progress.is_some(), "a new handover carries how far it got");
    assert!(plan.requested.contains(&RG_SECOND.to_string()));
    assert!(!plan.albums.iter().any(|a| a.album == "Second Wind"), "done albums stay behind");
    assert_eq!(p.t.w.status().unwrap().bridge.as_deref(), Some(NAME));
    // The next day: the desktop leaves the waves to the Bridge.
    *p.t.day.lock() += 1;
    assert!(p.t.w.pass().await.unwrap().requested.is_empty());
    let out = p.bridge_pass(true).await;
    assert_eq!(out.requested, vec!["Small Hours"]);
    assert!(p.t.r.web.writes().len() > writes);
    assert_eq!((p.in_lidarr(RG_SECOND), p.in_lidarr(RG_EP)), (1, 1));
    // Each album was asked for once, on one side.
    let desk: Vec<String> = p.t.r.lidarr.requests().into_iter().map(|r| r.release_group_id).collect();
    let brdg: Vec<String> = p.bl.requests().into_iter().map(|r| r.release_group_id).collect();
    assert_eq!(desk, vec![RG_SECOND.to_string()]);
    assert_eq!(brdg, vec![RG_EP.to_string()]);
    // Sent again (same handover): no progress, nothing new.
    let again = p.t.w.bridge_plan(LINK, NAME).await.unwrap().unwrap();
    assert_eq!(again.id, plan.id);
    assert!(again.progress.is_none());
}

#[tokio::test]
async fn the_bridge_runs_while_the_desktop_is_off_and_progress_comes_back() {
    let p = pair(false, json!({"bringInPerDay": 1, "bringInWaves": [100, 50]})).await;
    p.hand().await;
    // The desktop is off: two days on the Bridge alone.
    assert_eq!(p.bridge_pass(false).await.requested, vec!["Second Wind"]);
    *p.t.day.lock() += 1;
    p.advance(86_400);
    let second = p.bridge_pass(false).await;
    assert_eq!(second.requested, vec!["Small Hours"]);
    assert_eq!(second.needs_you, vec!["Basement Tapes", "Nowhere Songs"]);
    assert_eq!(p.btold.0.lock().len(), 2, "the Bridge logged each day's summary");
    // The desktop comes back and hears from the Bridge.
    let w2 = Waves::new(WavesDeps {
        lidarr: p.t.r.lidarr.clone(),
        notifier: p.t.told.clone(),
        day_of: { let d = p.t.day.clone(); Arc::new(move |_| *d.lock()) },
        hidden: Arc::new(Vec::new),
    });
    w2.bridge_progress(LINK, p.br.progress()).await.unwrap();
    let st = w2.status().unwrap();
    assert_eq!(st.bridge.as_deref(), Some(NAME));
    assert_eq!((st.requested_total, st.done, st.remaining), (2, 2, 0));
    assert_eq!(st.needs_you.iter().map(|n| n.album.as_str()).collect::<Vec<_>>(), ["Nowhere Songs", "Basement Tapes"], "newest first");
    assert_eq!(p.t.told.0.lock().last().map(|(_, b)| b.as_str()), Some("1 album requested, 2 need you."), "the summary reaches the desktop");
    // The Changes log came over with the shared Lidarr state.
    super::shared::tests::sync(&p.bl.inner.shared, &p.t.r.lidarr.inner.shared);
    let log = p.t.r.lidarr.changes();
    let small = log.iter().find(|c| c.what == "Requested Small Hours by Northlight (Bring in my listening)").expect("in the log");
    assert_eq!(small.device, NAME);
    // A "Needs you" retry on the desktop reaches the Bridge.
    let key = super::waves::key_of("Northlight", "Basement Tapes");
    let st = w2.needs_you(&key, true).unwrap();
    assert!(!st.needs_you.iter().any(|n| n.key == key));
    let plan = w2.bridge_plan(LINK, NAME).await.unwrap().unwrap();
    assert_eq!(plan.edits.len(), 1);
    p.br.take_plan(&plan).await.unwrap();
    assert!(!p.br.state().needs_you.iter().any(|n| n.key == key));
    w2.bridge_progress(LINK, p.br.progress()).await.unwrap();
    assert!(w2.bridge_plan(LINK, NAME).await.unwrap().unwrap().edits.is_empty(), "the Bridge confirmed it");
}

#[tokio::test]
async fn turning_it_off_on_the_desktop_stops_the_bridge() {
    let p = pair(false, json!({"bringInPerDay": 1})).await;
    p.hand().await;
    assert_eq!(p.bridge_pass(true).await.requested, vec!["Second Wind"]);
    let mut s = crate::settings::load(&p.t.r.db);
    s.bring_in = false;
    crate::settings::save(&p.t.r.db, &s).unwrap();
    let stop = p.t.w.bridge_plan(LINK, NAME).await.unwrap().unwrap();
    assert!(!stop.run && stop.albums.is_empty());
    p.br.take_plan(&stop).await.unwrap();
    assert!(!p.br.plan_running());
    assert!(!crate::settings::load(&p.bdb).bring_in);
    // The next day the Bridge asks for nothing.
    *p.t.day.lock() += 1;
    let writes = p.t.r.web.writes().len();
    assert_eq!(p.br.pass().await.unwrap(), PassOutcome::default());
    assert_eq!(p.t.r.web.writes().len(), writes);
    // The desktop has the waves back, with what the Bridge did.
    assert!(p.t.w.lease().is_none());
    let st = p.t.w.status().unwrap();
    assert!(st.bridge.is_none());
    assert_eq!(st.done, 1);
    // On again later: it carries on where the Bridge left off, here.
    let mut s = crate::settings::load(&p.t.r.db);
    s.bring_in = true;
    crate::settings::save(&p.t.r.db, &s).unwrap();
    assert_eq!(p.t.w.pass().await.unwrap().requested, vec!["Small Hours"]);
}

#[tokio::test]
async fn practice_mode_on_the_bridge_only_lists() {
    let p = pair(true, json!({})).await;
    let plan = p.hand().await;
    assert_eq!(plan.practice_until, 1_900_000_000);
    assert!(p.bl.practice_active());
    let out = p.bridge_pass(true).await;
    assert_eq!(out.practiced, vec!["Second Wind", "Small Hours"]);
    assert!(out.requested.is_empty());
    assert!(p.t.r.web.writes().is_empty(), "nothing sent in practice mode");
    assert!(p.bl.requests().is_empty());
    let st = p.t.w.status().unwrap();
    assert!(st.practice);
    assert_eq!(st.practiced.len(), 2);
    let practice: Vec<String> = p.bl.changes().into_iter().filter(|c| c.practice).map(|c| c.what).collect();
    assert!(practice.iter().any(|w| w.starts_with("Would request Second Wind")));
}

#[tokio::test]
async fn only_the_album_list_leaves_the_desktop() {
    let p = pair(false, json!({})).await;
    let plan = p.t.w.bridge_plan(LINK, NAME).await.unwrap().unwrap();
    let v = serde_json::to_value(&plan).unwrap();
    let albums = v["albums"].as_array().unwrap();
    assert_eq!(albums.len(), 4, "albums with fewer plays than the last wave stay behind: {albums:?}");
    for a in albums {
        let mut keys: Vec<&str> = a.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["album", "artist", "plays", "releaseGroupId"]);
    }
    assert_eq!(albums[0], json!({"artist": "Northlight", "album": "Second Wind", "releaseGroupId": null, "plays": 120}));
    let text = v.to_string();
    // No songs, play times, sources or anything else from the history.
    for never in ["Breathe In", "Night Bus", "Cellar", "Tiny", "streaming:", "1700000", "200000", "playedAt", "msPlayed"] {
        assert!(!text.contains(never), "{never} left the desktop");
    }
    // The Bridge's database has no listening history at all.
    let plays: i64 = p.bdb.read(|c| Ok(c.query_row("SELECT count(*) FROM imported_plays", [], |r| r.get(0))?)).unwrap();
    assert_eq!(plays, 0);
}

#[tokio::test]
async fn guards_pause_the_bridge_and_it_carries_on_once_clear() {
    let p = pair(false, json!({})).await;
    p.hand().await;
    p.t.r.web.st.lock().queue_total = 25;
    assert!(p.bridge_pass(true).await.paused.unwrap().contains("queue has 25 items"));
    assert!(p.t.w.status().unwrap().paused.unwrap().contains("queue has 25 items"), "the desktop shows why");
    p.t.r.web.st.lock().queue_total = 3;
    p.t.r.web.st.lock().free_space = Some(10_000_000_000);
    assert!(p.bridge_pass(true).await.paused.unwrap().contains("Only 10 GB"));
    p.t.r.web.st.lock().down = true;
    assert!(p.bridge_pass(true).await.paused.unwrap().contains("can't be reached"));
    assert!(p.t.r.web.writes().is_empty());
    p.t.r.web.st.lock().down = false;
    p.t.r.web.st.lock().free_space = Some(400_000_000_000);
    // Three failed requests in a row.
    p.t.r.web.st.lock().lookup_down = true;
    for _ in 0..2 {
        let out = p.bridge_pass(true).await;
        assert_eq!(out.paused, None);
        assert!(out.requested.is_empty());
    }
    assert!(p.bridge_pass(true).await.paused.unwrap().contains("3 requests in a row"));
    p.t.r.web.st.lock().lookup_down = false;
    let out = p.bridge_pass(true).await;
    assert_eq!(out.requested, vec!["Second Wind", "Small Hours"]);
    assert_eq!(p.t.w.status().unwrap().paused, None);
}

#[tokio::test]
async fn both_sides_keep_the_handover_through_a_restart() {
    let p = pair(false, json!({"bringInPerDay": 1})).await;
    p.hand().await;
    assert_eq!(p.bridge_pass(true).await.requested, vec!["Second Wind"]);
    // The Bridge restarts: same database, same plan.
    let br2 = bridge_waves(&p.t, &p.bl, &p.bdb, p.btold.clone());
    assert!(br2.plan_running());
    *p.t.day.lock() += 1;
    p.advance(3 * 3600);
    // The desktop restarts hours later, before it hears from the Bridge:
    // it waits a little rather than run the waves too.
    let w2 = Waves::new(WavesDeps {
        lidarr: p.t.r.lidarr.clone(),
        notifier: p.t.told.clone(),
        day_of: { let d = p.t.day.clone(); Arc::new(move |_| *d.lock()) },
        hidden: Arc::new(Vec::new),
    });
    assert_eq!(w2.status().unwrap().bridge.as_deref(), Some(NAME));
    assert!(w2.pass().await.unwrap().requested.is_empty());
    let out = br2.pass().await.unwrap();
    assert_eq!(out.requested, vec!["Small Hours"]);
    w2.bridge_progress(LINK, br2.progress()).await.unwrap();
    assert_eq!(w2.status().unwrap().requested_total, 2);
    assert_eq!(p.bl.requests().len(), 2);
    assert_eq!(p.t.r.lidarr.requests().len(), 0, "the desktop asked for nothing");
}

#[tokio::test]
async fn a_bridge_that_stops_answering_gives_the_waves_back() {
    let p = pair(false, json!({"bringInPerDay": 1})).await;
    p.hand().await;
    assert_eq!(p.bridge_pass(true).await.requested, vec!["Second Wind"]);
    // Silent for longer than it ever is when it's running.
    p.advance(SILENT_S + 1);
    *p.t.day.lock() += 1;
    let out = p.t.w.pass().await.unwrap();
    assert_eq!(out.requested, vec!["Small Hours"], "the desktop carries on after what the Bridge did");
    assert!(p.t.w.lease().is_none());
    assert!(p.t.w.status().unwrap().bridge.is_none());
    assert_eq!(p.t.r.lidarr.requests().len(), 1);
    // Back again: it gets a new handover with the desktop's progress.
    let plan = p.t.w.bridge_plan(LINK, NAME).await.unwrap().unwrap();
    let prog = plan.progress.clone().unwrap();
    assert_eq!(prog.done.len(), 2);
    p.br.take_plan(&plan).await.unwrap();
    assert_eq!(p.br.state().done.len(), 2);
}

#[tokio::test]
async fn a_bridge_that_never_answers_a_handover_is_left_be_for_a_while() {
    let p = pair(false, json!({"bringInPerDay": 1})).await;
    // An older Bridge: the plan goes nowhere.
    p.t.w.bridge_plan(LINK, NAME).await.unwrap().unwrap();
    assert!(p.t.w.pass().await.unwrap().requested.is_empty(), "waiting for the Bridge to answer");
    p.advance(HANDOVER_WAIT + 1);
    assert_eq!(p.t.w.pass().await.unwrap().requested, vec!["Second Wind"]);
    assert!(p.t.w.bridge_plan(LINK, NAME).await.unwrap().is_none(), "not handed over again yet");
    assert!(!p.t.w.may_hand_to(LINK));
    p.advance(RETRY_AFTER);
    assert!(p.t.w.may_hand_to(LINK));
}

#[test]
fn progress_merges_without_losing_either_side() {
    use super::waves::{NeedsYou, WaveState};
    let ny = |k: &str, at: i64| NeedsYou { key: k.into(), artist: "A".into(), album: k.into(), plays: 1, reason: "r".into(), at, get: None };
    let mut a = WaveState { wave: 0, day: 10, today: 3, done: vec!["x".into()], needs_you: vec![ny("n1", 5)], requested_total: 4, ..Default::default() };
    let b = WaveState { wave: 1, day: 10, today: 5, done: vec!["y".into(), "x".into()], needs_you: vec![ny("n2", 6), ny("x", 1)], requested_total: 2, ..Default::default() };
    super::waves_bridge::merge_progress(&mut a, &b);
    assert_eq!(a.done, ["x", "y"]);
    assert_eq!(a.needs_you.iter().map(|n| n.key.as_str()).collect::<Vec<_>>(), ["n2", "n1"], "done albums don't need you");
    assert_eq!((a.wave, a.today, a.requested_total), (1, 5, 4));
    let later = WaveState { day: 11, today: 1, ..Default::default() };
    super::waves_bridge::merge_progress(&mut a, &later);
    assert_eq!((a.day, a.today), (11, 1));
}
