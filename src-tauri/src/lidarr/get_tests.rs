//! One-click Get, live request states, searches again on a schedule and
//! "Get albums" through the waves, against the faked Lidarr and MusicBrainz
//! of the request tests. Nothing real is reached.

use std::sync::Arc;

use serde_json::{Value, json};

use super::mbpick::tests::{RG_BOOT, RG_EP, RG_SECOND, RG_SKETCH};
use super::request::{GIVE_UP_AFTER, NOT_FOUND_NOTE, RESEARCH_AFTER};
use super::request_tests::{FakeLidarr, assert_only_allowed_writes, input, rig};
use super::status::RequestState;
use super::waves::{QueuedGet, Waves, WavesDeps};
use crate::web::Method;

const T0: i64 = 1_800_000_000;

/// Every command Lidarr was sent, as (name, album ids).
fn commands(web: &FakeLidarr) -> Vec<(String, Vec<i64>)> {
    web.writes()
        .into_iter()
        .filter(|(m, p, _)| *m == Method::Post && p == "/api/v1/command")
        .map(|(_, _, b)| {
            let v: Value = serde_json::from_str(&b).unwrap();
            (v["name"].as_str().unwrap_or_default().to_string(), v["albumIds"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default())
        })
        .collect()
}

#[tokio::test]
async fn get_asks_for_the_first_request_typed_then_sends_in_one_click() {
    let r = rig();
    let out = r.lidarr.get(input(RG_SECOND, "Second Wind", None)).await.unwrap();
    let needs = out.needs.expect("the first request ever needs the user");
    assert_eq!(needs.kind, "firstTime");
    assert!(out.record.is_none());
    assert!(r.web.writes().is_empty(), "nothing was sent");
    // The dialog sends it with the name typed; after that, Get sends at once.
    r.lidarr.get(input(RG_EP, "Small Hours", Some("Small Hours"))).await.unwrap().record.expect("sent");
    let out = r.lidarr.get(input(RG_SECOND, "Second Wind", None)).await.unwrap();
    assert!(out.needs.is_none());
    let rec = out.record.unwrap();
    assert_eq!(rec.state, RequestState::Searching, "{:?}", rec.message);
    assert_eq!((rec.searched_at, rec.searches, rec.next_search_at), (Some(T0), 1, Some(T0 + RESEARCH_AFTER[0])));
    assert_only_allowed_writes(&r.web);
}

#[tokio::test]
async fn get_says_why_when_a_check_says_no_and_sends_nothing() {
    let r = rig();
    r.lidarr.get(input(RG_EP, "Small Hours", Some("Small Hours"))).await.unwrap();
    let before = r.web.writes().len();
    let boot = r.lidarr.get(input(RG_BOOT, "Basement Tapes", None)).await.unwrap();
    let n = boot.needs.unwrap();
    assert_eq!(n.kind, "refused");
    assert!(n.reason.contains("bootleg"), "{}", n.reason);
    let clash = r.lidarr.get(input(RG_SKETCH, "Quiet Sketches", None)).await.unwrap();
    assert!(clash.needs.unwrap().reason.contains("unreleased"));
    assert_eq!(r.web.writes().len(), before, "a refused Get asks Lidarr nothing");
}

#[tokio::test]
async fn states_follow_the_queue_with_progress_then_importing_and_imported() {
    let r = rig();
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    let id = rec.lidarr_album_id.unwrap();
    r.web.st.lock().queue = vec![json!({"albumId": id, "status": "downloading", "trackedDownloadState": "downloading", "size": 500.0, "sizeleft": 300.0})];
    let l = r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!((l[0].state, l[0].progress), (RequestState::Downloading, Some(40)));
    r.web.st.lock().queue = vec![json!({"albumId": id, "status": "completed", "trackedDownloadState": "importPending", "size": 500.0, "sizeleft": 0.0})];
    let l = r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!((l[0].state, l[0].progress), (RequestState::Importing, None));
    {
        let mut st = r.web.st.lock();
        st.queue.clear();
        st.history.push(json!({"albumId": id, "eventType": "downloadImported"}));
    }
    assert_eq!(r.lidarr.refresh_statuses().await.unwrap()[0].state, RequestState::Imported);
    assert!(!RequestState::Imported.in_flight());
}

#[tokio::test]
async fn a_failed_download_says_why() {
    let r = rig();
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    let id = rec.lidarr_album_id.unwrap();
    r.web.st.lock().queue = vec![json!({"albumId": id, "status": "failed", "trackedDownloadState": "failedPending", "errorMessage": "Not enough seeders"})];
    let l = r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(l[0].state, RequestState::Failed);
    assert!(l[0].message.as_deref().unwrap().contains("Not enough seeders"));
}

#[tokio::test]
async fn nothing_grabbed_is_searched_again_on_schedule_across_restarts_then_not_found() {
    let r = rig();
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    let id = rec.lidarr_album_id.unwrap();
    assert_eq!(commands(&r.web), [("AlbumSearch".to_string(), vec![id])]);

    // Not due yet: reads only.
    *r.now.lock() = T0 + RESEARCH_AFTER[0] - 60;
    r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(commands(&r.web).len(), 1);

    // Due: one AlbumSearch for that album, then the next is scheduled.
    *r.now.lock() = T0 + RESEARCH_AFTER[0];
    let l = r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(commands(&r.web).len(), 2);
    assert_eq!((l[0].searches, l[0].next_search_at), (2, Some(T0 + RESEARCH_AFTER[1])));

    // Platter starts again: the schedule was kept.
    let again = r.restart();
    assert_eq!(again.requests()[0].next_search_at, Some(T0 + RESEARCH_AFTER[1]));
    for (n, after) in RESEARCH_AFTER.iter().enumerate().skip(1) {
        *r.now.lock() = T0 + after;
        again.refresh_statuses().await.unwrap();
        assert_eq!(commands(&r.web).len(), n + 2, "search number {} went out at +{after} s", n + 2);
    }
    let l = again.requests();
    assert_eq!((l[0].searches, l[0].next_search_at, l[0].state), (11, None, RequestState::Searching));

    // A day later with nothing: Not found yet.
    *r.now.lock() = T0 + GIVE_UP_AFTER;
    let l = again.refresh_statuses().await.unwrap();
    assert_eq!(l[0].state, RequestState::NotFound);
    assert_eq!(l[0].message.as_deref(), Some(NOT_FOUND_NOTE));
    *r.now.lock() += 10 * 86_400;
    again.refresh_statuses().await.unwrap();
    assert_eq!(commands(&r.web).len(), 11, "no more searches once it's Not found yet");

    // Search again: one more, and the schedule starts over.
    let rec = again.search_again(&l[0].id).await.unwrap();
    assert_eq!((rec.state, rec.searches), (RequestState::Searching, 1));
    assert_eq!(rec.next_search_at, Some(*r.now.lock() + RESEARCH_AFTER[0]));
    let cmds = commands(&r.web);
    assert_eq!(cmds.len(), 12);
    assert!(cmds.iter().all(|(name, ids)| name == "AlbumSearch" && ids == &vec![id]), "only AlbumSearch, only that album: {cmds:?}");
    assert!(!r.web.writes().iter().any(|(_, _, b)| b.contains("MissingAlbumSearch")));
    assert_only_allowed_writes(&r.web);

    // Something turns up later: it's followed again.
    r.web.st.lock().history.push(json!({"albumId": id, "eventType": "grabbed"}));
    assert_eq!(again.refresh_statuses().await.unwrap()[0].state, RequestState::Downloading);
}

#[tokio::test]
async fn a_found_album_is_not_searched_again() {
    let r = rig();
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    let id = rec.lidarr_album_id.unwrap();
    r.web.st.lock().queue = vec![json!({"albumId": id, "status": "downloading", "trackedDownloadState": "downloading"})];
    *r.now.lock() = T0 + RESEARCH_AFTER[2];
    r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(commands(&r.web).len(), 1);
    assert!(r.lidarr.search_again(&rec.id).await.is_err(), "Lidarr is working on it");
}

struct Quiet;

impl crate::social::Notifier for Quiet {
    fn notify(&self, _title: &str, _body: &str) {}
}

/// Waves on the rig, out of practice mode (it only lists what it would ask for).
fn waves(r: &super::request_tests::Rig) -> Waves {
    let mut s = crate::settings::load(&r.db);
    s.lidarr_practice_until = 0;
    crate::settings::save(&r.db, &s).unwrap();
    Waves::new(WavesDeps {
        lidarr: r.lidarr.clone(),
        notifier: Arc::new(Quiet),
        day_of: Arc::new(|at| at / 86_400),
        hidden: Arc::new(Vec::new),
    })
}

fn queued(rg: &str, title: &str) -> QueuedGet {
    QueuedGet { release_group_id: rg.into(), title: title.into(), artist: "Northlight".into(), artist_mbid: Some(super::mbpick::tests::ARTIST.into()), artist_id: None }
}

#[tokio::test]
async fn get_albums_goes_through_the_waves_paced_path_with_bring_in_off() {
    let r = rig();
    let mut s = crate::settings::load(&r.db);
    s.bring_in = false;
    s.bring_in_per_day = 1;
    s.lidarr_practice_until = 0;
    crate::settings::save(&r.db, &s).unwrap();
    r.web.st.lock().free_space = Some(400_000_000_000);
    r.lidarr.check().await.unwrap();
    let w = waves(&r);
    let added = w.enqueue(vec![queued(RG_SECOND, "Second Wind"), queued(RG_EP, "Small Hours"), queued(RG_SECOND, "Second Wind"), queued("bad", "X")], "Northlight").unwrap();
    assert_eq!(added, 2, "once each, real ids only");
    assert!(r.web.writes().is_empty(), "queued, nothing sent yet");

    // One a day here: the first goes out, the second waits for tomorrow.
    let out = w.pass().await.unwrap();
    assert_eq!(out.requested, ["Second Wind"]);
    assert_eq!(w.status().unwrap().queued.len(), 1);
    assert_eq!(r.lidarr.requests()[0].state, RequestState::Searching);
    w.pass().await.unwrap();
    assert_eq!(w.status().unwrap().queued.len(), 1, "today's count is reached");
    *r.now.lock() += 86_400;
    let out = w.pass().await.unwrap();
    assert_eq!(out.requested, ["Small Hours"]);
    assert!(w.status().unwrap().queued.is_empty());
    // Asked for already: not queued again.
    assert_eq!(w.enqueue(vec![queued(RG_SECOND, "Second Wind")], "Northlight").unwrap(), 0);
    assert_only_allowed_writes(&r.web);
}

#[tokio::test]
async fn get_albums_puts_a_refused_album_under_needs_you_and_carries_on() {
    let r = rig();
    r.web.st.lock().free_space = Some(400_000_000_000);
    r.lidarr.check().await.unwrap();
    let w = waves(&r);
    w.enqueue(vec![queued(RG_BOOT, "Basement Tapes"), queued(RG_SECOND, "Second Wind")], "Northlight").unwrap();
    let out = w.pass().await.unwrap();
    assert_eq!(out.needs_you, ["Basement Tapes"]);
    assert_eq!(out.requested, ["Second Wind"]);
    let st = w.status().unwrap();
    assert!(st.needs_you[0].reason.contains("bootleg"));
    // Try again puts it back on the queue; leaving it be drops it.
    let st = w.needs_you(&st.needs_you[0].key, true).unwrap();
    assert_eq!(st.queued.len(), 1);
    let st = w.unqueue(&[]).unwrap();
    assert!(st.queued.is_empty());
}

#[tokio::test]
async fn get_albums_pauses_while_lidarrs_queue_is_full() {
    let r = rig();
    {
        let mut st = r.web.st.lock();
        st.free_space = Some(400_000_000_000);
        st.queue_total = 50;
    }
    r.lidarr.check().await.unwrap();
    let w = waves(&r);
    w.enqueue(vec![queued(RG_SECOND, "Second Wind")], "Northlight").unwrap();
    let out = w.pass().await.unwrap();
    assert!(out.paused.unwrap().contains("queue has 50 items"));
    assert!(out.requested.is_empty());
    assert_eq!(w.status().unwrap().queued.len(), 1, "kept for later");
}
