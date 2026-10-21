//! "Bring in my listening" against the faked Lidarr and MusicBrainz of the
//! request tests, with made-up listening history. Nothing real is reached.

use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::json;

use super::mbpick::tests::{RG_EP, RG_SECOND};
use super::request_tests::{Rig, assert_only_allowed_writes, rig};
use super::status::RequestState;
use super::waves::{self, Gauges, Waves, WavesDeps};
use crate::db::listens::{ImportSource, ImportedPlay, insert_imported};
use crate::social::Notifier;
use crate::web::Method;

#[derive(Default)]
pub(super) struct Told(pub(super) Mutex<Vec<(String, String)>>);

impl Notifier for Told {
    fn notify(&self, title: &str, body: &str) {
        self.0.lock().push((title.into(), body.into()));
    }
}

/// The rig's clock is fixed at 1,800,000,000; days are moved by hand.
pub(super) struct Setup {
    pub(super) r: Rig,
    pub(super) w: Waves,
    pub(super) told: Arc<Told>,
    pub(super) day: Arc<Mutex<i64>>,
}

/// Plays of Northlight albums: Second Wind 120, Basement Tapes (a
/// bootleg) 110, Nowhere Songs (not on MusicBrainz) 105, Small Hours 60,
/// and a few of an album nobody asked for.
fn history(r: &Rig) {
    let mut v = Vec::new();
    let mut n = 0;
    for (album, title, plays) in [("Second Wind", "Breathe In", 120), ("Basement Tapes", "Cellar", 110), ("Nowhere Songs", "Gone", 105), ("Small Hours", "Night Bus", 60), ("Tiny", "Blip", 3)] {
        for _ in 0..plays {
            n += 1;
            v.push(ImportedPlay {
                source: ImportSource::File,
                source_id: format!("streaming:{n}"),
                played_at: 1_700_000_000_000 + n * 1000,
                ms_played: Some(200_000),
                title: title.into(),
                artist: Some("Northlight".into()),
                album: Some(album.into()),
                rating_key: None,
            });
        }
    }
    r.db.write(|c| insert_imported(c, &v, 1)).unwrap();
}

pub(super) async fn setup(practice: bool, patch: serde_json::Value) -> Setup {
    let r = rig();
    let mut s = crate::settings::load(&r.db);
    s.bring_in = true;
    s.lidarr_practice_until = if practice { 1_900_000_000 } else { 0 };
    let p: crate::settings::SettingsPatch = serde_json::from_value(patch).unwrap();
    s.apply(&p).unwrap();
    crate::settings::save(&r.db, &s).unwrap();
    r.mb.route(
        "/release-group?query=releasegroup%3A%22small+hours%22",
        Some(json!({"release-groups": [{"id": RG_EP, "title": "Small Hours", "primary-type": "EP"}]})),
    );
    r.web.st.lock().free_space = Some(400_000_000_000);
    history(&r);
    // A passed check, and the first request already typed by hand.
    r.lidarr.check().await.unwrap();
    r.db.write(|c| crate::db::store::kv_set(c, "lidarr.typedConfirmed", "1")).unwrap();
    let told = Arc::new(Told::default());
    let day = Arc::new(Mutex::new(100));
    let d = day.clone();
    let w = Waves::new(WavesDeps {
        lidarr: r.lidarr.clone(),
        notifier: told.clone(),
        day_of: Arc::new(move |_| *d.lock()),
        hidden: Arc::new(Vec::new),
        utc_offset: None,
    });
    Setup { r, w, told, day }
}

#[test]
fn waves_setting_is_kept_tidy() {
    assert_eq!(waves::clean_waves(&[20, 100, 50]).unwrap(), vec![100, 50, 20]);
    assert!(waves::clean_waves(&[]).is_err());
    assert!(waves::clean_waves(&[50, 50]).is_err(), "each once");
    assert!(waves::clean_waves(&[0]).is_err());
    assert!(waves::clean_waves(&[600, 500, 400, 300, 200, 100]).is_err(), "at most five");
    let s = crate::settings::Settings::default();
    assert!(!s.bring_in);
    assert_eq!((s.bring_in_waves.as_slice(), s.bring_in_per_day, s.bring_in_min_free_gb, s.bring_in_max_queue), (&[100, 50, 20][..], 30, 50, 20));
    for k in ["bringIn", "bringInWaves", "bringInPerDay", "bringInMinFreeGb", "bringInMaxQueue"] {
        assert!(crate::settings::SYNCED_SETTINGS.contains(&k), "{k} is synced");
    }
}

#[test]
fn guards_pause_on_a_full_queue_or_little_space() {
    let ok = Gauges { queue: 20, free: Some(51_000_000_000) };
    assert_eq!(waves::guard(&ok, 20, 50), None);
    assert!(waves::guard(&Gauges { queue: 21, ..ok }, 20, 50).unwrap().contains("queue has 21 items"));
    assert!(waves::guard(&Gauges { free: Some(49_000_000_000), ..ok }, 20, 50).unwrap().contains("Only 49 GB"));
    assert_eq!(waves::guard(&Gauges { free: None, ..ok }, 20, 50), None, "no reading, no pause");
}

#[tokio::test]
async fn a_wave_goes_most_played_first_and_sends_what_needs_you_aside() {
    let t = setup(false, json!({"bringInWaves": [100, 50]})).await;
    let out = t.w.pass().await.unwrap();
    assert_eq!(out.paused, None);
    // Wave 1 (100+): Second Wind requested; the bootleg and the unknown
    // album need the user; then wave 2 (50+): Small Hours.
    assert_eq!(out.requested, vec!["Second Wind", "Small Hours"]);
    assert_eq!(out.needs_you, vec!["Basement Tapes", "Nowhere Songs"]);
    let st = t.w.status().unwrap();
    assert!(st.finished && st.threshold.is_none());
    assert_eq!((st.today, st.requested_total, st.done), (2, 2, 2));
    assert_eq!(st.needs_you.len(), 2);
    assert!(st.needs_you.iter().any(|n| n.album == "Basement Tapes" && n.plays == 110 && !n.reason.is_empty()));
    let recs = t.r.lidarr.requests();
    assert_eq!(recs.len(), 2);
    assert!(recs.iter().all(|r| r.auto && r.state == RequestState::Searching));
    assert!(recs.iter().any(|r| r.release_group_id == RG_SECOND));
    assert_only_allowed_writes(&t.r.web);
    // In the Changes log.
    let log: Vec<String> = t.r.lidarr.changes().into_iter().map(|c| c.what).collect();
    assert!(log.iter().any(|w| w == "Requested Second Wind by Northlight (Bring in my listening)"));
    // Nothing is asked for twice.
    let writes = t.r.web.writes().len();
    t.w.needs_you(&waves::key_of("Northlight", "Nowhere Songs"), false).unwrap();
    let again = t.w.pass().await.unwrap();
    assert!(again.requested.is_empty());
    assert_eq!(t.r.web.writes().len(), writes);
}

#[tokio::test]
async fn the_daily_pace_stops_it_until_local_midnight_with_a_summary() {
    let t = setup(false, json!({"bringInPerDay": 1})).await;
    let out = t.w.pass().await.unwrap();
    assert_eq!(out.requested, vec!["Second Wind"]);
    assert_eq!(t.w.status().unwrap().today, 1);
    assert_eq!(t.told.0.lock().clone(), vec![("Bring in my listening".to_string(), "1 album requested.".to_string())]);
    // Same day: nothing more, and no second summary.
    assert!(t.w.pass().await.unwrap().requested.is_empty());
    assert_eq!(t.told.0.lock().len(), 1);
    // The next day carries on where it left off.
    *t.day.lock() += 1;
    let out = t.w.pass().await.unwrap();
    assert_eq!(out.requested, vec!["Small Hours"]);
    assert_eq!(t.w.status().unwrap().today, 1);
}

#[tokio::test]
async fn practice_mode_only_lists() {
    let t = setup(true, json!({})).await;
    assert!(t.r.lidarr.practice_active());
    let out = t.w.pass().await.unwrap();
    assert_eq!(out.practiced, vec!["Second Wind", "Small Hours"]);
    assert!(out.requested.is_empty());
    assert!(t.r.web.writes().is_empty(), "nothing sent in practice mode");
    assert!(t.r.lidarr.requests().is_empty());
    let st = t.w.status().unwrap();
    assert!(st.practice);
    assert_eq!(st.practiced.iter().map(|p| p.album.as_str()).collect::<Vec<_>>(), ["Small Hours", "Second Wind"]);
    let practice: Vec<String> = t.r.lidarr.changes().into_iter().filter(|c| c.practice).map(|c| c.what).collect();
    assert!(practice.iter().any(|w| w.starts_with("Would request Second Wind")));
}

#[tokio::test]
async fn guards_pause_and_it_carries_on_once_clear() {
    let t = setup(false, json!({})).await;
    t.r.web.st.lock().queue_total = 25;
    let out = t.w.pass().await.unwrap();
    assert!(out.paused.as_deref().unwrap().contains("queue has 25 items"));
    assert!(t.r.web.writes().is_empty());
    t.r.web.st.lock().queue_total = 3;
    t.r.web.st.lock().free_space = Some(10_000_000_000);
    let out = t.w.pass().await.unwrap();
    assert!(out.paused.as_deref().unwrap().contains("Only 10 GB"));
    t.r.web.st.lock().down = true;
    let out = t.w.pass().await.unwrap();
    assert!(out.paused.as_deref().unwrap().contains("can't be reached"));
    // Clear again: it resumes on its own.
    t.r.web.st.lock().down = false;
    t.r.web.st.lock().free_space = Some(400_000_000_000);
    let out = t.w.pass().await.unwrap();
    assert_eq!(out.paused, None);
    assert_eq!(out.requested, vec!["Second Wind", "Small Hours"]);
    let log: Vec<String> = t.r.lidarr.changes().into_iter().map(|c| c.what).collect();
    assert!(log.iter().any(|w| w == "Paused Bring in my listening"));
    assert!(log.iter().any(|w| w == "Bring in my listening carried on"));
}

#[tokio::test]
async fn three_failures_in_a_row_pause_it() {
    let t = setup(false, json!({})).await;
    // Lidarr's metadata service is down: each try fails, the album stays.
    t.r.web.st.lock().lookup_down = true;
    for _ in 0..2 {
        let out = t.w.pass().await.unwrap();
        assert_eq!(out.paused, None);
        assert!(out.requested.is_empty());
    }
    let out = t.w.pass().await.unwrap();
    assert!(out.paused.as_deref().unwrap().contains("3 requests in a row"), "{:?}", out.paused);
    t.r.web.st.lock().lookup_down = false;
    let out = t.w.pass().await.unwrap();
    assert_eq!(out.requested, vec!["Second Wind", "Small Hours"]);
}

#[tokio::test]
async fn the_user_can_pause_and_progress_survives_a_restart() {
    let t = setup(false, json!({"bringInPerDay": 1})).await;
    t.w.pass().await.unwrap();
    t.w.set_paused(true).unwrap();
    *t.day.lock() += 1;
    assert!(t.w.pass().await.unwrap().requested.is_empty(), "paused by the user");
    // A new Waves on the same database (Platter restarted) picks up there.
    let d = t.day.clone();
    let w2 = Waves::new(WavesDeps { lidarr: t.r.lidarr.clone(), notifier: t.told.clone(), day_of: Arc::new(move |_| *d.lock()), hidden: Arc::new(Vec::new), utc_offset: None });
    assert!(w2.status().unwrap().user_paused);
    w2.set_paused(false).unwrap();
    let out = w2.pass().await.unwrap();
    assert_eq!(out.requested, vec!["Small Hours"], "Second Wind was done before the restart");
    assert_eq!(t.r.lidarr.requests().len(), 2);
}

#[tokio::test]
async fn off_or_not_ready_does_nothing() {
    let t = setup(false, json!({"bringIn": false})).await;
    assert_eq!(t.w.pass().await.unwrap(), waves::PassOutcome::default());
    assert!(t.r.web.log.lock().iter().all(|(m, _, _)| *m == Method::Get));
    assert!(!t.w.status().unwrap().on);
}

#[tokio::test]
async fn albums_asked_for_by_hand_or_hidden_are_skipped() {
    let t = setup(false, json!({})).await;
    t.r.lidarr.request(super::request_tests::input(RG_SECOND, "Second Wind", None)).await.unwrap();
    let d = t.day.clone();
    let hidden = vec![crate::import::played::hide_key_for("album", "Northlight", "Small Hours").unwrap()];
    let w = Waves::new(WavesDeps { lidarr: t.r.lidarr.clone(), notifier: t.told.clone(), day_of: Arc::new(move |_| *d.lock()), hidden: Arc::new(move || hidden.clone()), utc_offset: None });
    let out = w.pass().await.unwrap();
    assert!(out.requested.is_empty(), "{out:?}");
    assert_eq!(t.r.lidarr.requests().len(), 1);
    // Retrying a "Needs you" album looks at it again.
    let st = w.needs_you(&waves::key_of("Northlight", "Basement Tapes"), true).unwrap();
    assert!(!st.needs_you.iter().any(|n| n.album == "Basement Tapes"));
}

#[tokio::test]
async fn without_lidarr_here_it_waits_for_the_bridge_instead_of_pausing() {
    let t = setup(false, json!({})).await;
    let mut s = crate::settings::load(&t.r.db);
    s.lidarr_url = String::new();
    crate::settings::save(&t.r.db, &s).unwrap();
    assert!(!t.r.lidarr.status().url_set);
    // A Bridge is linked but hasn't answered yet (the app just started).
    t.r.lidarr.set_bridge_linked(Arc::new(|| true));
    let out = t.w.pass().await.unwrap();
    assert_eq!(out.paused, None);
    assert!(t.r.lidarr.changes().iter().all(|c| c.what != "Paused Bring in my listening"));
    assert!(t.r.web.writes().is_empty());
    // With no Bridge either, it says why it can't go on.
    t.r.lidarr.set_bridge_linked(Arc::new(|| false));
    let out = t.w.pass().await.unwrap();
    assert!(out.paused.as_deref().unwrap().contains("Lidarr isn't set up"));
}
