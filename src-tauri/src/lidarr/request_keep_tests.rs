//! Requests kept and followed over time: searches again after a long time
//! away, one album Lidarr can't answer for, albums Lidarr already had, the
//! list's size limit, waiting requests that keep going wrong, and cancel
//! racing a status look. Against the faked Lidarr of the request tests.

use serde_json::json;

use super::mbpick::tests::{RG_EP, RG_SECOND};
use super::request::{MAX_ATTEMPTS, MAX_KEPT, MAX_RESEARCH_PER_LOOK, RESEARCH_AFTER, RequestRecord};
use super::request_tests::{FakeLidarr, Rig, input, rig};
use super::status::RequestState;
use crate::web::Method;

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

fn searches(web: &FakeLidarr) -> usize {
    web.writes().iter().filter(|(m, p, b)| *m == Method::Post && p == "/api/v1/command" && b.contains("AlbumSearch")).count()
}

fn rgid(n: usize) -> String {
    format!("20000000-0000-0000-0000-{n:012}")
}

/// `n` requests Lidarr is searching for, first searched at T0, each with
/// its monitored album in the faked Lidarr. The first is the newest.
fn searching(r: &Rig, n: usize) -> Vec<RequestRecord> {
    let mut list = Vec::new();
    for i in 0..n {
        let id = 500 + i as i64;
        r.web.st.lock().albums.push(json!({"id": id, "title": format!("Album {i}"), "foreignAlbumId": rgid(i), "artistId": 7, "monitored": true,
            "statistics": {"trackFileCount": 0, "trackCount": 10, "percentOfTracks": 0}}));
        let mut rec = r.lidarr.new_record(&input(&rgid(i), &format!("Album {i}"), None), rgid(i), false);
        rec.state = RequestState::Searching;
        rec.message = None;
        rec.lidarr_album_id = Some(id);
        rec.created_at = T0 - i as i64;
        rec.searched_at = Some(T0);
        rec.searches = 1;
        rec.next_search_at = Some(T0 + RESEARCH_AFTER[0]);
        list.push(rec);
    }
    r.lidarr.save_records(&list).unwrap();
    list
}

#[tokio::test]
async fn after_five_days_away_only_a_few_searches_go_out_and_the_next_comes_after_now() {
    let r = rig();
    r.lidarr.check().await.unwrap();
    searching(&r, 7);
    *r.now.lock() = T0 + 5 * DAY;
    let now = *r.now.lock();
    let l = r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(searches(&r.web), MAX_RESEARCH_PER_LOOK, "capped, not one per missed time");
    let sent: Vec<&RequestRecord> = l.iter().filter(|x| x.searches == 2).collect();
    assert_eq!(sent.len(), MAX_RESEARCH_PER_LOOK);
    for x in &sent {
        let next = x.next_search_at.unwrap();
        assert!(next > now, "the missed times are skipped: {next} after {now}");
        assert_eq!(next, T0 + 6 * DAY, "the first time on the schedule after now");
    }
    // The next looks take the rest, a few at a time, and none goes twice.
    r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(searches(&r.web), 2 * MAX_RESEARCH_PER_LOOK);
    r.lidarr.refresh_statuses().await.unwrap();
    r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(searches(&r.web), 7, "each album searched once");
    assert!(r.lidarr.requests().iter().all(|x| x.searches == 2 && x.next_search_at.is_some_and(|t| t > now)));
}

#[tokio::test]
async fn a_setup_check_that_doesnt_pass_isnt_run_again_on_every_look() {
    let r = rig();
    r.lidarr.check().await.unwrap();
    searching(&r, 1);
    // Plex out of reach: the check can't compare folders.
    r.web.st.lock().plex_down = true;
    *r.now.lock() = T0 + DAY;
    let checks = |web: &FakeLidarr| web.log.lock().iter().filter(|(m, p, _)| *m == Method::Get && p.ends_with("/rootfolder")).count();
    let before = checks(&r.web);
    r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(checks(&r.web), before + 1, "checked before searching again");
    assert_eq!(searches(&r.web), 0, "a check that didn't run sends no search");
    let next = r.lidarr.requests()[0].next_search_at.unwrap();
    assert_eq!(next, T0 + DAY + super::request::RETRY_EVERY.as_secs() as i64);
    // The next quick looks don't read the whole setup again.
    r.lidarr.refresh_statuses().await.unwrap();
    r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(checks(&r.web), before + 1);
    // Plex is back, and the slow look comes round: it goes.
    r.web.st.lock().plex_down = false;
    *r.now.lock() = next;
    r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(searches(&r.web), 1);
}

#[tokio::test]
async fn an_album_lidarr_cant_answer_for_doesnt_resend_the_others() {
    let r = rig();
    r.lidarr.check().await.unwrap();
    let list = searching(&r, 2);
    // The second (older) one is read after the first is searched.
    r.web.st.lock().fail_album = Some(list[1].release_group_id.clone());
    *r.now.lock() = T0 + RESEARCH_AFTER[0];
    r.lidarr.refresh_statuses().await.expect("one album failing doesn't stop the look");
    assert_eq!(searches(&r.web), 1);
    r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(searches(&r.web), 1, "the first album's search was kept, not sent again");
    let kept = r.lidarr.requests();
    assert_eq!(kept.iter().find(|x| x.id == list[0].id).unwrap().searches, 2);
    // Back again: the other one goes out once.
    r.web.st.lock().fail_album = None;
    r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(searches(&r.web), 2);
}

#[tokio::test]
async fn an_album_lidarr_already_had_is_never_searched() {
    let r = rig();
    r.lidarr.check().await.unwrap();
    let mut list = searching(&r, 2);
    for x in list.iter_mut() {
        x.already_in_lidarr = true;
        x.searched_at = None;
        x.searches = 0;
        x.next_search_at = None;
    }
    // One with the schedule set by an older version, too.
    list[1].searched_at = Some(T0);
    list[1].searches = 1;
    list[1].next_search_at = Some(T0 + RESEARCH_AFTER[0]);
    r.lidarr.save_records(&list).unwrap();
    for at in [RESEARCH_AFTER[0], 2 * DAY, 12 * DAY] {
        *r.now.lock() = T0 + at;
        let l = r.lidarr.refresh_statuses().await.unwrap();
        assert!(l.iter().all(|x| x.state == RequestState::Searching), "followed, never Not found yet: {l:?}");
    }
    assert_eq!(searches(&r.web), 0, "Lidarr was never asked to search");
}

#[tokio::test]
async fn a_full_list_keeps_every_request_still_going_and_remembers_what_left() {
    let r = rig();
    let mut list = Vec::new();
    // 60 old requests Lidarr is still working on, and 190 newer imported ones.
    for i in 0..250 {
        let mut rec = r.lidarr.new_record(&input(&rgid(i), &format!("Album {i}"), None), rgid(i), false);
        rec.lidarr_album_id = Some(1000 + i as i64);
        if i < 60 {
            rec.state = RequestState::Searching;
            rec.created_at = T0 - 10_000 + i as i64;
        } else {
            rec.state = RequestState::Imported;
            rec.created_at = T0 + i as i64;
        }
        list.push(rec);
    }
    r.lidarr.save_records(&list).unwrap();
    let kept = r.lidarr.requests();
    assert_eq!(kept.len(), MAX_KEPT);
    assert_eq!(kept.iter().filter(|x| x.state == RequestState::Searching).count(), 60, "none still going was dropped");
    // The oldest imported ones left, and are remembered as asked for.
    let gone = rgid(60);
    assert!(!kept.iter().any(|x| x.release_group_id == gone));
    assert!(r.lidarr.asked_before(&gone));
    assert!(r.lidarr.asked_groups().contains(&gone));
    // Kept across a restart, and an automatic request never asks again.
    let again = r.restart();
    assert!(again.asked_before(&gone));
    let rec = again.request_auto(input(&gone, "Album 60", None)).await.unwrap();
    assert_eq!(rec.state, RequestState::Imported);
    assert!(r.web.writes().is_empty(), "nothing sent");
}

#[test]
fn requests_saved_by_an_older_version_still_read() {
    let r = rig();
    let old = json!([{"id": "x1", "releaseGroupId": RG_SECOND, "title": "Second Wind", "artist": "Northlight", "state": "searching",
        "lidarrAlbumId": 101, "createdAt": T0, "updatedAt": T0, "searchedAt": T0, "searches": 1, "nextSearchAt": T0 + 3600}]);
    r.db.write(|c| crate::db::store::kv_set(c, "lidarr.requests", &old.to_string())).unwrap();
    let l = r.lidarr.requests();
    assert_eq!(l.len(), 1);
    assert_eq!((l[0].attempts, l[0].state), (0, RequestState::Searching));
}

#[tokio::test]
async fn a_passing_server_error_keeps_an_automatic_request_waiting_then_gives_up() {
    let r = rig();
    // Lidarr's metadata answers with a server error.
    r.web.st.lock().lookup_down = true;
    let rec = r.lidarr.request_auto(input(RG_SECOND, "Second Wind", None)).await.unwrap();
    assert_eq!(rec.state, RequestState::Queued, "{:?}", rec.message);
    assert_eq!(rec.attempts, 1);
    assert!(rec.message.as_deref().unwrap().contains("tries again"));
    for _ in 1..MAX_ATTEMPTS - 1 {
        r.lidarr.retry_queued().await.unwrap();
        assert_eq!(r.lidarr.requests()[0].state, RequestState::Queued);
    }
    r.lidarr.retry_queued().await.unwrap();
    let l = r.lidarr.requests();
    assert_eq!((l[0].state, l[0].attempts), (RequestState::Failed, MAX_ATTEMPTS));
    assert!(l[0].message.as_deref().unwrap().contains("gave up"));
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn a_server_error_that_clears_sends_the_waiting_request() {
    let r = rig();
    r.web.st.lock().lookup_down = true;
    r.lidarr.request_auto(input(RG_EP, "Small Hours", None)).await.unwrap();
    r.web.st.lock().lookup_down = false;
    assert_eq!(r.lidarr.retry_queued().await.unwrap(), 1);
    assert_eq!(r.lidarr.requests()[0].state, RequestState::Searching);
}

#[tokio::test]
async fn a_new_artist_still_being_read_is_tried_a_limited_number_of_times() {
    let r = rig();
    r.web.st.lock().albums_delay = 1_000_000;
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert_eq!(rec.state, RequestState::Queued);
    for _ in 0..MAX_ATTEMPTS {
        r.lidarr.retry_queued().await.unwrap();
    }
    let l = r.lidarr.requests();
    assert_eq!(l[0].state, RequestState::Failed, "{:?}", l[0].message);
    // Given up: no more tries.
    let reads = r.web.log.lock().len();
    r.lidarr.retry_queued().await.unwrap();
    assert_eq!(r.web.log.lock().len(), reads);
}

#[tokio::test]
async fn a_search_that_doesnt_go_out_keeps_the_request_and_is_sent_by_the_retry() {
    let r = rig();
    // Lidarr takes the monitoring, then answers the search with a server error.
    r.web.st.lock().fail_search = true;
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.expect("kept, not lost");
    assert_eq!(rec.state, RequestState::Queued, "{:?}", rec.message);
    assert!(rec.lidarr_album_id.is_some());
    assert!(rec.message.as_deref().unwrap().contains("marked for download"));
    assert_eq!(r.lidarr.requests().len(), 1, "saved");
    assert_eq!(searches(&r.web), 1, "the one Lidarr answered with an error");
    r.web.st.lock().fail_search = false;
    assert_eq!(r.lidarr.retry_queued().await.unwrap(), 1);
    let l = r.lidarr.requests();
    assert_eq!(l[0].state, RequestState::Searching);
    assert!(!l[0].already_in_lidarr, "still this request's album");
    assert_eq!(searches(&r.web), 2);
    assert_eq!(r.web.writes().iter().filter(|(m, p, _)| *m == Method::Post && p == "/api/v1/artist").count(), 1, "the artist is added once");
}

#[tokio::test]
async fn cancel_waits_for_a_status_look_in_progress() {
    let r = rig();
    r.web.st.lock().down = true;
    let waiting = r.lidarr.request(input(RG_EP, "Small Hours", Some("Small Hours"))).await.unwrap();
    assert_eq!(waiting.state, RequestState::Queued);
    // Something else holds the request lock (a status look, a retry).
    let guard = r.lidarr.lock.clone().lock_owned().await;
    let l = r.lidarr.clone();
    let id = waiting.id.clone();
    let h = tokio::spawn(async move { l.cancel(&id).await });
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(!h.is_finished(), "cancel waits its turn");
    assert_eq!(r.lidarr.requests().len(), 1);
    drop(guard);
    let left = h.await.unwrap().unwrap();
    assert!(left.is_empty());
}
