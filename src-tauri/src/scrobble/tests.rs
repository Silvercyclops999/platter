//! Scrobbling against faked Last.fm and ListenBrainz answers: thresholds,
//! the offline queue, private sessions, unreleased music, sign-in and a
//! refused session.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use parking_lot::Mutex;

use super::commands::ScrobbleState;
use super::lastfm::{self, Lastfm};
use super::listenbrainz::{self, ListenBrainz};
use super::*;
use crate::audio::events::TrackingEvent;
use crate::model::Track;
use crate::plex::keychain::{MemoryStore, TokenStore};
use crate::plex::token::Token;
use crate::web::fake::FakeWeb;

const NOW: i64 = 1_800_000_000;

struct NoEvents;
impl EventSink for NoEvents {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

#[derive(Default)]
struct Notices(Mutex<Vec<Service>>);
impl ScrobbleNotice for Notices {
    fn rejected(&self, s: Service) {
        self.0.lock().push(s);
    }
}

struct Unrel(Vec<String>);
impl Unreleased for Unrel {
    fn unreleased(&self, t: &Track) -> bool {
        self.0.contains(&t.id)
    }
}

fn track(id: &str, ms: u64) -> Track {
    Track { mbid: None,
        id: id.into(),
        title: format!("Song {id}"),
        album_id: "a".into(),
        album_title: "Record".into(),
        artist_id: "r".into(),
        artist_title: "Band".into(),
        track_artist: Some("Singer".into()),
        track_number: 3,
        disc_number: 1,
        duration_ms: ms,
        thumb: None,
        part_key: "/library/parts/1/2/file.flac".into(),
        codec: None,
        container: None,
        bitrate: None,
        bit_depth: None,
        sample_rate: None,
        user_rating: None,
        view_count: 0,
        last_viewed_at: None,
        rating_count: None,
        file: None,
        other_versions: None,
        song_status: None,
        rated_at: None,
        genres: vec![],
    }
}

struct Rig {
    s: Scrobbler,
    web: Arc<FakeWeb>,
    keys: Arc<MemoryStore>,
    switches: ScrobbleSwitches,
    notices: Arc<Notices>,
    state: ScrobbleState,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("s.db")).unwrap();
    let web = Arc::new(FakeWeb::new());
    let keys = Arc::new(MemoryStore::new());
    for (a, v) in [(lastfm::KEY_ACCOUNT, "key1"), (lastfm::SECRET_ACCOUNT, "sec1"), (lastfm::SESSION_ACCOUNT, "sk1"), (listenbrainz::TOKEN_ACCOUNT, "lbtok")] {
        keys.set(a, &Token::new(v).unwrap()).unwrap();
    }
    let lf = Arc::new(Lastfm::new(web.clone(), keys.clone()));
    let lb = Arc::new(ListenBrainz::new(web.clone(), keys.clone()));
    let switches = ScrobbleSwitches::default();
    switches.lastfm.store(true, Ordering::Relaxed);
    switches.listenbrainz.store(true, Ordering::Relaxed);
    let notices = Arc::new(Notices::default());
    let s = Scrobbler::new(ScrobbleDeps {
        db: db.clone(),
        services: vec![lf.clone(), lb.clone()],
        switches: switches.clone(),
        unreleased: Arc::new(Unrel(vec!["demo".into()])),
        notice: notices.clone(),
        events: Arc::new(NoEvents),
        clock: Arc::new(|| NOW),
    });
    web.route("audioscrobbler", None, 200, r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#);
    web.route("submit-listens", None, 200, r#"{"status":"ok"}"#);
    let state = ScrobbleState::new(s.clone(), lf, lb, db);
    Rig { s, web, keys, switches, notices, state, _dir: dir }
}

async fn play(r: &Rig, t: &Track, item: &str, listened_ms: u64) {
    r.s.handle(&TrackingEvent::TrackStarted { track: t.clone(), item_id: item.into(), at: (NOW - 300) * 1000, context: None }).await;
    r.s.handle(&TrackingEvent::TrackEnded { track: t.clone(), item_id: item.into(), listened_ms, completed: false, skipped_at_ms: None }).await;
}

fn scrobbles(r: &Rig) -> (Vec<String>, Vec<String>) {
    let lf: Vec<String> = r.web.bodies("audioscrobbler").into_iter().filter(|b| b.contains("method=track.scrobble")).collect();
    let lb: Vec<String> = r.web.bodies("submit-listens").into_iter().filter(|b| !b.contains("playing_now")).collect();
    (lf, lb)
}

#[test]
fn the_threshold_is_half_or_four_minutes_and_over_thirty_seconds() {
    assert!(!counts(30_000, 30_000), "30 s songs never count");
    assert!(counts(31_000, 15_500));
    assert!(!counts(200_000, 99_999));
    assert!(counts(200_000, 100_000));
    assert!(counts(20 * 60_000, 240_000), "4 minutes is enough for a long song");
    assert!(!counts(20 * 60_000, 239_999));
}

#[tokio::test]
async fn a_play_past_the_threshold_scrobbles_to_both_with_now_playing_first() {
    let r = rig();
    play(&r, &track("1", 200_000), "q1", 120_000).await;
    let all = r.web.bodies("audioscrobbler");
    assert!(all[0].contains("method=track.updateNowPlaying"), "{all:?}");
    let (lf, lb) = scrobbles(&r);
    assert_eq!((lf.len(), lb.len()), (1, 1));
    // The song's own artist, the album artist, the start time, signed.
    assert!(lf[0].contains("artist[0]=Singer") && lf[0].contains("albumArtist[0]=Band"), "{}", lf[0]);
    assert!(lf[0].contains(&format!("timestamp[0]={}", NOW - 300)) && lf[0].contains("api_sig=") && lf[0].contains("sk=sk1"));
    let v: serde_json::Value = serde_json::from_str(&lb[0]).unwrap();
    assert_eq!(v["listen_type"], "single");
    assert_eq!(v["payload"][0]["listened_at"], NOW - 300);
    assert_eq!(v["payload"][0]["track_metadata"]["artist_name"], "Singer");
    // The token goes in a header, never the body.
    assert!(!lb[0].contains("lbtok"));
    let req = r.web.log.lock().iter().find(|q| q.url.contains("submit-listens")).cloned().unwrap();
    assert!(req.headers.iter().any(|(k, v)| *k == "Authorization" && v == "Token lbtok"));
}

#[tokio::test]
async fn short_or_barely_heard_songs_dont_scrobble() {
    let r = rig();
    play(&r, &track("1", 25_000), "q1", 25_000).await;
    play(&r, &track("2", 200_000), "q2", 60_000).await;
    assert_eq!(scrobbles(&r), (vec![], vec![]));
}

#[tokio::test]
async fn a_private_session_never_scrobbles_or_says_now_playing() {
    let r = rig();
    r.switches.private_session.store(true, Ordering::Relaxed);
    play(&r, &track("1", 200_000), "q1", 200_000).await;
    assert!(r.web.log.lock().is_empty());
    assert!(r.s.queued().unwrap().is_empty(), "nothing waits for later either");
    // Turning it on mid-song keeps that song out too.
    r.switches.private_session.store(false, Ordering::Relaxed);
    let t = track("2", 200_000);
    r.s.handle(&TrackingEvent::TrackStarted { track: t.clone(), item_id: "q2".into(), at: NOW * 1000, context: None }).await;
    r.switches.private_session.store(true, Ordering::Relaxed);
    r.s.handle(&TrackingEvent::TrackEnded { track: t, item_id: "q2".into(), listened_ms: 200_000, completed: true, skipped_at_ms: None }).await;
    assert_eq!(scrobbles(&r), (vec![], vec![]));
}

#[tokio::test]
async fn unreleased_music_only_scrobbles_with_its_setting() {
    let r = rig();
    play(&r, &track("demo", 200_000), "q1", 200_000).await;
    assert!(r.web.log.lock().is_empty());
    r.switches.unreleased.store(true, Ordering::Relaxed);
    play(&r, &track("demo", 200_000), "q2", 200_000).await;
    assert_eq!(scrobbles(&r).0.len(), 1);
}

#[tokio::test]
async fn offline_scrobbles_wait_and_go_out_in_order() {
    let r = rig();
    r.web.fail("audioscrobbler");
    r.web.fail("submit-listens");
    play(&r, &track("1", 200_000), "q1", 200_000).await;
    play(&r, &track("2", 200_000), "q2", 200_000).await;
    assert_eq!(r.s.queued().unwrap().len(), 4, "two songs for two services");
    // Back online: one batch per service, oldest first.
    r.web.clear();
    r.web.route("audioscrobbler", None, 200, "{}");
    r.web.route("submit-listens", None, 200, "{}");
    r.s.replay().await;
    assert!(r.s.queued().unwrap().is_empty());
    let (lf, lb) = scrobbles(&r);
    let lf = lf.last().unwrap();
    assert!(lf.contains("track[0]=Song 1") && lf.contains("track[1]=Song 2"), "{lf}");
    let v: serde_json::Value = serde_json::from_str(lb.last().unwrap()).unwrap();
    assert_eq!(v["listen_type"], "import");
    assert_eq!(v["payload"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn scrobbles_older_than_two_weeks_are_dropped() {
    let r = rig();
    r.web.fail("audioscrobbler");
    r.web.fail("submit-listens");
    let old = Listen::from_track(&track("1", 200_000), NOW - QUEUE_MAX_AGE_S - 10);
    r.s.scrobble(old).await;
    assert_eq!(r.s.queued().unwrap().len(), 2);
    r.s.replay().await;
    assert!(r.s.queued().unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_session_keeps_the_scrobbles_and_tells_the_user_once() {
    let r = rig();
    r.web.route("audioscrobbler", None, 200, r#"{"error":9,"message":"Invalid session key - Please re-authenticate"}"#);
    play(&r, &track("1", 200_000), "q1", 200_000).await;
    play(&r, &track("2", 200_000), "q2", 200_000).await;
    assert_eq!(*r.notices.0.lock(), vec![Service::Lastfm], "one toast");
    assert!(r.s.is_rejected(Service::Lastfm));
    let waiting = r.s.queued().unwrap();
    assert_eq!(waiting.iter().filter(|q| q.service == Service::Lastfm).count(), 2);
    assert!(r.state.status().lastfm.rejected);
    // ListenBrainz carried on.
    assert_eq!(scrobbles(&r).1.len(), 2);

    // Signing in again sends what waited.
    r.web.route("audioscrobbler", Some("method=auth.getToken"), 200, r#"{"token":"tok9"}"#);
    let url = r.state.lastfm_start().await.unwrap();
    assert!(url.starts_with("https://www.last.fm/api/auth/?api_key=key1&token=tok9"), "{url}");
    r.web.route("audioscrobbler", Some("method=auth.getSession"), 200, r#"{"session":{"name":"me","key":"sk2","subscriber":0}}"#);
    r.web.route("audioscrobbler", Some("method=track.scrobble"), 200, "{}");
    let st = r.state.lastfm_finish().await.unwrap();
    assert!(st.lastfm.signed_in && !st.lastfm.rejected);
    assert_eq!(st.lastfm.user.as_deref(), Some("me"));
    assert_eq!(r.keys.get(lastfm::SESSION_ACCOUNT).unwrap().unwrap().expose(), "sk2");
    r.s.replay().await;
    assert!(r.s.queued().unwrap().is_empty());
}

#[tokio::test]
async fn finishing_before_approving_says_so() {
    let r = rig();
    r.web.route("audioscrobbler", Some("method=auth.getToken"), 200, r#"{"token":"t"}"#);
    r.state.lastfm_start().await.unwrap();
    r.web.route("audioscrobbler", Some("method=auth.getSession"), 200, r#"{"error":14,"message":"Unauthorized Token"}"#);
    let e = r.state.lastfm_finish().await.unwrap_err();
    assert!(e.to_string().contains("Allow Platter"), "{e}");
    assert!(r.state.status().lastfm.pending, "still waiting for the approval");
}

#[tokio::test]
async fn signing_in_needs_the_key_and_secret() {
    let r = rig();
    r.keys.delete(lastfm::SECRET_ACCOUNT).unwrap();
    assert!(r.state.lastfm_start().await.is_err());
    assert!(r.web.log.lock().is_empty());
}

#[tokio::test]
async fn a_refused_listenbrainz_token_shows_and_is_cleared_by_a_good_one() {
    let r = rig();
    r.web.route("submit-listens", None, 401, r#"{"code":401}"#);
    play(&r, &track("1", 200_000), "q1", 200_000).await;
    assert!(r.s.is_rejected(Service::Listenbrainz));
    assert!(r.notices.0.lock().contains(&Service::Listenbrainz));
    r.web.route("validate-token", None, 200, r#"{"valid":true,"user_name":"me"}"#);
    r.web.route("submit-listens", None, 200, "{}");
    let st = r.state.listenbrainz_check().await.unwrap();
    assert!(!st.listenbrainz.rejected);
    assert_eq!(st.listenbrainz.user.as_deref(), Some("me"));
}

#[tokio::test]
async fn a_service_thats_off_gets_nothing() {
    let r = rig();
    r.switches.lastfm.store(false, Ordering::Relaxed);
    play(&r, &track("1", 200_000), "q1", 200_000).await;
    assert!(r.web.bodies("audioscrobbler").is_empty());
    assert_eq!(scrobbles(&r).1.len(), 1);
}

#[tokio::test]
async fn nothing_waiting_means_no_keychain_reads() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("s.db")).unwrap();
    let web = Arc::new(FakeWeb::new());
    let keys = Arc::new(crate::plex::keychain::CountingStore::default());
    let lf = Arc::new(Lastfm::new(web.clone(), keys.clone()));
    let lb = Arc::new(ListenBrainz::new(web.clone(), keys.clone()));
    let switches = ScrobbleSwitches::default();
    switches.lastfm.store(true, Ordering::Relaxed);
    switches.listenbrainz.store(true, Ordering::Relaxed);
    let s = Scrobbler::new(ScrobbleDeps {
        db,
        services: vec![lf, lb],
        switches,
        unreleased: Arc::new(Unrel(vec![])),
        notice: Arc::new(Notices::default()),
        events: Arc::new(NoEvents),
        clock: Arc::new(|| NOW),
    });
    // The minute timer with an empty queue.
    for _ in 0..3 {
        s.replay().await;
    }
    assert_eq!(keys.total_reads(), 0);
}
