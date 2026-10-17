//! The release radar with MusicBrainz, Lidarr and notifications faked.
//! Nothing here reaches a real MusicBrainz or Lidarr.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::lidarr::mbpick::VARIOUS_ARTISTS;
use crate::lidarr::mbpick::tests::{ARTIST, FakeMb, OTHER, rg};
use crate::social::follows::FollowedArtist;

const NOW: i64 = 1_800_000_000;
const HARBOR: &str = "cccccccc-1111-2222-3333-444444444444";

fn id(n: u32) -> String {
    format!("20000000-0000-0000-0000-{n:012}")
}

fn days_ago(d: i64) -> String {
    ymd(NOW - d * DAY)
}

fn ac(mbid: &str, name: &str) -> Value {
    json!([{"name": name, "artist": {"id": mbid, "name": name}}])
}

/// What a release group search answers for Northlight and Harbor Sound.
fn answer(extra: Vec<Value>) -> Value {
    let n = || ac(ARTIST, "Northlight");
    let mut groups = vec![
        rg(&id(1), "Lanterns", "Album", &[], &days_ago(20), &["Official"], n()),
        rg(&id(2), "Low Water", "Single", &[], &days_ago(40), &["Official"], n()),
        rg(&id(3), "Cellar Takes", "Album", &[], &days_ago(10), &["Bootleg"], n()),
        rg(&id(4), "Radio Edits", "Album", &[], &days_ago(10), &["Promotion"], n()),
        rg(&id(5), "Live at the Pier", "Album", &["Live"], &days_ago(10), &["Official"], n()),
        rg(&id(6), "Summer Hits 9", "Album", &["Compilation"], &days_ago(10), &["Official"], ac(VARIOUS_ARTISTS, "Various Artists")),
        rg(
            &id(7),
            "Together Again",
            "Album",
            &[],
            &days_ago(10),
            &["Official"],
            json!([{"name": "Northlight", "joinphrase": " & ", "artist": {"id": ARTIST, "name": "Northlight"}}, {"name": "Guest", "artist": {"id": OTHER, "name": "Guest"}}]),
        ),
        rg(&id(8), "Old News", "Album", &[], &days_ago(400), &["Official"], n()),
        rg(&id(9), "Harbor Lights", "EP", &[], &days_ago(5), &["Official"], ac(HARBOR, "Harbor Sound")),
        rg(&id(10), "Quiet Sketches", "Album", &[], &days_ago(30), &["Official"], n()),
        rg(&id(11), "First Light", "Album", &[], &days_ago(60), &["Official"], n()),
    ];
    groups.extend(extra);
    json!({"count": groups.len(), "release-groups": groups})
}

struct Mbids;

#[async_trait]
impl ArtistMbid for Mbids {
    async fn artist_mbid(&self, _artist_id: &str, name: &str) -> Option<String> {
        match name {
            "Northlight" => Some(ARTIST.into()),
            "Harbor Sound" => Some(HARBOR.into()),
            n if n.starts_with("Artist ") => Some(format!("dddddddd-0000-0000-0000-{:012}", n[7..].parse::<u32>().unwrap())),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Follows(Mutex<Vec<FollowedArtist>>);

impl FollowStore for Follows {
    fn list(&self) -> AppResult<Vec<FollowedArtist>> {
        Ok(self.0.lock().clone())
    }
    fn save(&self, all: &[FollowedArtist]) -> AppResult<()> {
        *self.0.lock() = all.to_vec();
        Ok(())
    }
}

#[derive(Default)]
struct Top(Mutex<Vec<RadarArtist>>);

impl TopPlayed for Top {
    fn top(&self, limit: usize) -> Vec<RadarArtist> {
        self.0.lock().iter().take(limit).cloned().collect()
    }
}

/// A Lidarr that keeps requests; titles starting "Refuse" are refused like
/// the MusicBrainz guards would.
#[derive(Default)]
struct FakeLidarr {
    ready: AtomicBool,
    list: Mutex<Vec<RequestRecord>>,
    clock: Mutex<i64>,
}

#[async_trait]
impl RadarLidarr for FakeLidarr {
    fn ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
    fn requests(&self) -> Vec<RequestRecord> {
        self.list.lock().clone()
    }
    async fn request_auto(&self, input: RequestInput) -> AppResult<RequestRecord> {
        let now = *self.clock.lock();
        let refused = input.title.starts_with("Refuse");
        let r = RequestRecord {
            id: format!("req-{}", self.list.lock().len() + 1),
            release_group_id: input.release_group_id,
            title: input.title,
            artist: input.artist,
            artist_id: input.artist_id,
            artist_mbid: input.artist_mbid,
            state: if refused { RequestState::Failed } else { RequestState::Searching },
            destination: None,
            lidarr_album_id: None,
            added_artist: false,
            already_in_lidarr: false,
            created_at: now,
            updated_at: now,
            message: refused.then(|| "MusicBrainz lists this only as a bootleg or promo.".to_string()),
            notes: vec![],
            auto: true,
            protected: false,
            progress: None,
            searched_at: None,
            searches: 0,
            next_search_at: None,
        };
        self.list.lock().push(r.clone());
        Ok(r)
    }
}

#[derive(Default)]
struct Notes(Mutex<Vec<(String, String)>>);

impl Notifier for Notes {
    fn notify(&self, title: &str, body: &str) {
        self.0.lock().push((title.into(), body.into()));
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<String>>);

impl EventSink for Events {
    fn emit(&self, event: &str, _payload: Value) {
        self.0.lock().push(event.into());
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    db: DbHandle,
    mb: Arc<FakeMb>,
    follows: Arc<Follows>,
    top: Arc<Top>,
    lidarr: Arc<FakeLidarr>,
    notes: Arc<Notes>,
    events: Arc<Events>,
    online: Arc<AtomicBool>,
    private: Arc<AtomicBool>,
    now: Arc<Mutex<i64>>,
    radar: Radar,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("radar.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('a1', 'First Light', 'First Light', 'ar1', 'Northlight', 'Album', '[]', 1),
               ('a2', 'Quiet Sketches (Build 2)', 'Quiet Sketches', 'ar1', 'Northlight', NULL, '[]', 0);",
        )?;
        Ok(())
    })
    .unwrap();
    let mb = Arc::new(FakeMb::default());
    mb.route("/release-group?query=", Some(answer(vec![])));
    let follows = Arc::new(Follows::default());
    follows.save(&[FollowedArtist { id: "ar1".into(), title: "Northlight".into(), thumb: None, followed_at: 1 }]).unwrap();
    let top = Arc::new(Top::default());
    top.0.lock().push(RadarArtist { id: Some("ar2".into()), name: "Harbor Sound".into() });
    let lidarr = Arc::new(FakeLidarr::default());
    *lidarr.clock.lock() = NOW;
    let notes = Arc::new(Notes::default());
    let events = Arc::new(Events::default());
    let online = Arc::new(AtomicBool::new(true));
    let private = Arc::new(AtomicBool::new(false));
    let now = Arc::new(Mutex::new(NOW));
    let (n, on) = (now.clone(), online.clone());
    let radar = Radar::new(RadarDeps {
        db: db.clone(),
        mb: mb.clone(),
        mbids: Arc::new(Mbids),
        follows: follows.clone(),
        top: top.clone(),
        lidarr: lidarr.clone(),
        notifier: notes.clone(),
        events: events.clone(),
        usable: Arc::new(move || on.load(Ordering::Relaxed)),
        private_session: private.clone(),
        clock: Arc::new(move || *n.lock()),
    });
    let rig = Rig { _dir: dir, db, mb, follows, top, lidarr, notes, events, online, private, now, radar };
    // The radar asks MusicBrainz, so a new install has it off; these tests turn it on.
    rig.settings(|s| s.release_radar = true);
    rig
}

impl Rig {
    fn later(&self, secs: i64) {
        *self.now.lock() += secs;
        *self.lidarr.clock.lock() += secs;
    }
    fn settings(&self, f: impl FnOnce(&mut crate::settings::Settings)) {
        let mut s = crate::settings::load(&self.db);
        f(&mut s);
        crate::settings::save(&self.db, &s).unwrap();
    }
    fn titles(&self) -> Vec<String> {
        self.radar.page().unwrap().items.into_iter().map(|i| i.title).collect()
    }
}

#[test]
fn one_search_asks_for_several_artists_since_the_window_opened() {
    assert_eq!(ymd(0), "1970-01-01");
    assert_eq!(ymd(NOW), "2027-01-15");
    let u = Url::parse(&search_url(&[ARTIST.into(), HARBOR.into()], "2026-10-17", 0)).unwrap();
    let q = u.query_pairs().find(|(k, _)| k == "query").unwrap().1.into_owned();
    assert_eq!(q, format!("(arid:{ARTIST} OR arid:{HARBOR}) AND firstreleasedate:[2026-10-17 TO 9999]"));
    assert!(u.as_str().starts_with("https://musicbrainz.org/ws/2/release-group?"));
    assert!(u.as_str().contains("fmt=json") && u.as_str().contains("limit=100"));
}

#[tokio::test]
async fn only_official_albums_eps_and_singles_by_the_artist_alone_in_the_window() {
    let r = rig();
    r.radar.check().await.unwrap();
    let mut t = r.titles();
    t.sort();
    assert_eq!(t, vec!["First Light", "Harbor Lights", "Lanterns", "Low Water", "Quiet Sketches"]);
    // Newest first.
    assert_eq!(r.titles()[0], "Harbor Lights");
    let page = r.radar.page().unwrap();
    let ep = page.items.iter().find(|i| i.title == "Harbor Lights").unwrap();
    assert_eq!((ep.kind.as_str(), ep.artist.as_str()), ("ep", "Harbor Sound"));
    assert_eq!(ep.thumb.as_deref(), Some(format!("/ext/https://coverartarchive.org/release-group/{}/front-250", id(9)).as_str()));
    assert_eq!(page.artists, 2);
    // Two artists: one search.
    assert_eq!(r.mb.count("/release-group?query="), 1);
}

#[tokio::test]
async fn the_library_decides_in_library_or_why_it_cant_be_requested() {
    let r = rig();
    r.radar.check().await.unwrap();
    let page = r.radar.page().unwrap();
    let get = |t: &str| page.items.iter().find(|i| i.title == t).unwrap().clone();
    assert_eq!(get("First Light").library_album_id.as_deref(), Some("a1"));
    assert!(get("First Light").refusal.is_none());
    let sketch = get("Quiet Sketches");
    assert_eq!(sketch.library_album_id, None, "an unreleased build of the same name isn't the release");
    assert!(sketch.refusal.unwrap().contains("unreleased album already in your library"));
    let lanterns = get("Lanterns");
    assert!(lanterns.library_album_id.is_none() && lanterns.refusal.is_none() && lanterns.request.is_none());
    // The user's mark counts: marked released, it's the release, in the library.
    crate::db::release::set_marks([(crate::db::release::mark_key("album", "a2"), crate::db::release::Mark::Released)].into_iter().collect());
    let page = r.radar.page().unwrap();
    let sketch = page.items.iter().find(|i| i.title == "Quiet Sketches").unwrap();
    assert_eq!(sketch.library_album_id.as_deref(), Some("a2"));
    assert!(sketch.refusal.is_none());
    crate::db::release::set_marks(Default::default());
}

#[tokio::test]
async fn a_first_look_is_quiet_and_later_finds_are_announced() {
    let r = rig();
    r.radar.check().await.unwrap();
    assert!(r.notes.0.lock().is_empty(), "nothing announced on an artist's first look");
    assert!(r.radar.page().unwrap().items.iter().all(|i| !i.new));
    assert!(!r.radar.due(), "just looked");
    r.later(CHECK_EVERY);
    assert!(r.radar.due());
    r.mb.route("/release-group?query=", Some(answer(vec![rg(&id(20), "Paper Moon", "Album", &[], &days_ago(1), &["Official"], ac(ARTIST, "Northlight"))])));
    let o = r.radar.check().await.unwrap();
    assert_eq!(o.new.len(), 1);
    assert_eq!(r.notes.0.lock().clone(), vec![("New album from Northlight".to_string(), "Paper Moon".to_string())]);
    let page = r.radar.page().unwrap();
    assert!(page.items.iter().find(|i| i.title == "Paper Moon").unwrap().new);
    assert!(!page.items.iter().find(|i| i.title == "Lanterns").unwrap().new);
    assert!(r.events.0.lock().iter().any(|e| e == EVENT_CHANGED));
}

#[tokio::test]
async fn notifications_follow_the_setting_and_private_sessions() {
    let r = rig();
    r.radar.check().await.unwrap();
    let paper = || answer(vec![rg(&id(20), "Paper Moon", "Album", &[], &days_ago(1), &["Official"], ac(ARTIST, "Northlight"))]);
    r.settings(|s| s.radar_notify = false);
    r.mb.route("/release-group?query=", Some(paper()));
    r.radar.check().await.unwrap();
    assert!(r.notes.0.lock().is_empty());
    // A private session stays quiet too.
    r.settings(|s| s.radar_notify = true);
    r.private.store(true, Ordering::Relaxed);
    r.mb.route(
        "/release-group?query=",
        Some(answer(vec![rg(&id(21), "Night Ferry", "Album", &[], &days_ago(1), &["Official"], ac(ARTIST, "Northlight"))])),
    );
    r.radar.check().await.unwrap();
    assert!(r.notes.0.lock().is_empty());
}

#[test]
fn notification_texts() {
    let f = |t: &str, a: &str, k: &str| Found {
        release_group_id: id(1),
        title: t.into(),
        artist: a.into(),
        artist_id: None,
        artist_mbid: ARTIST.into(),
        kind: k.into(),
        date: "2027-01-01".into(),
        found_at: 0,
        baseline: false,
    };
    assert_eq!(notice(&[]), None);
    assert_eq!(notice(&[f("Low Water", "Northlight", "single")]).unwrap().0, "New single from Northlight");
    assert_eq!(notice(&[f("A", "Northlight", "album"), f("B", "Northlight", "ep")]).unwrap(), ("New from Northlight".into(), "A and 1 more".into()));
    assert_eq!(notice(&[f("A", "Northlight", "album"), f("B", "Harbor Sound", "ep")]).unwrap().0, "Release radar");
}

#[tokio::test]
async fn the_setting_off_hides_the_radar_and_stops_looking() {
    let r = rig();
    r.radar.check().await.unwrap();
    r.settings(|s| s.release_radar = false);
    let page = r.radar.page().unwrap();
    assert!(!page.shown && page.items.is_empty());
    assert!(!r.radar.due());
    let before = r.mb.count("musicbrainz");
    r.later(CHECK_EVERY * 2);
    r.radar.check().await.unwrap();
    assert_eq!(r.mb.count("musicbrainz"), before, "nothing asked while off");
}

#[tokio::test]
async fn musicbrainz_down_or_offline_keeps_what_was_found() {
    let r = rig();
    r.radar.check().await.unwrap();
    r.later(CHECK_EVERY);
    *r.mb.down.lock() = true;
    assert!(r.radar.check().await.is_err());
    assert_eq!(r.titles().len(), 5, "the last answers stay");
    assert!(!r.radar.due(), "a failed look waits an hour before the next try");
    r.later(RETRY_AFTER);
    assert!(r.radar.due());
    r.online.store(false, Ordering::Relaxed);
    assert!(!r.radar.due(), "never while MusicBrainz can't be asked");
    assert!(r.radar.check().await.is_err());
}

#[tokio::test]
async fn many_artists_go_a_few_to_a_search_and_unfollowed_ones_drop_off() {
    let r = rig();
    for n in 0..9 {
        r.top.0.lock().push(RadarArtist { id: None, name: format!("Artist {n}") });
    }
    // Ten from the top list at most: Harbor Sound and nine more is ten.
    r.radar.check().await.unwrap();
    assert_eq!(r.mb.count("/release-group?query="), 2, "eleven artists, eight to a search");
    assert_eq!(r.radar.artists().len(), 11);
    // Unfollow Northlight: its releases leave at the next look.
    r.follows.save(&[]).unwrap();
    r.later(CHECK_EVERY);
    r.radar.check().await.unwrap();
    assert_eq!(r.titles(), vec!["Harbor Lights"]);
}

fn auto_rig() -> Rig {
    let r = rig();
    r.lidarr.ready.store(true, Ordering::Relaxed);
    r.radar.set_auto("ar1", "Northlight", true).unwrap();
    r
}

fn album(n: u32, title: &str, ago: i64) -> Value {
    rg(&id(n), title, "Album", &[], &days_ago(ago), &["Official"], ac(ARTIST, "Northlight"))
}

#[tokio::test]
async fn turning_auto_request_on_for_an_artist_turns_it_on_and_is_remembered() {
    let r = rig();
    assert!(!crate::settings::load(&r.db).auto_request);
    let all = r.radar.set_auto("ar1", "Northlight", true).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].enabled_at, NOW);
    assert!(crate::settings::load(&r.db).auto_request, "asking for it per artist turns auto-request on");
    assert!(r.events.0.lock().iter().any(|e| e == "settings://changed"));
    assert!(r.radar.set_auto("ar1", "Northlight", false).unwrap().is_empty());
    assert!(crate::settings::load(&r.db).auto_request, "turning one artist off leaves the switch alone");
    assert!(r.radar.set_auto("", "x", true).is_err());
}

#[tokio::test]
async fn auto_request_only_new_albums_found_after_it_was_turned_on() {
    let r = auto_rig();
    // The first look only sets the scene, and what was there before isn't new.
    let o = r.radar.check().await.unwrap();
    assert!(o.auto.is_empty());
    assert!(r.lidarr.list.lock().is_empty());
    r.later(CHECK_EVERY);
    r.mb.route(
        "/release-group?query=",
        Some(answer(vec![
            album(20, "Paper Moon", 1),
            rg(&id(22), "Moon Single", "Single", &[], &days_ago(1), &["Official"], ac(ARTIST, "Northlight")),
            rg(&id(23), "Harbor Again", "Album", &[], &days_ago(1), &["Official"], ac(HARBOR, "Harbor Sound")),
        ])),
    );
    let o = r.radar.check().await.unwrap();
    let asked: Vec<String> = o.auto.iter().map(|x| x.title.clone()).collect();
    assert_eq!(asked, vec!["Paper Moon"], "albums only, and only for artists with auto-request on");
    let summary = r.notes.0.lock().iter().find(|(t, _)| t == "Auto-request").cloned().unwrap();
    assert_eq!(summary.1, "Requested Paper Moon by Northlight.");
    let item = r.radar.page().unwrap().items.into_iter().find(|i| i.title == "Paper Moon").unwrap();
    assert_eq!(item.request, Some(RequestState::Searching));
    assert!(item.auto);
    // Never asked twice.
    r.later(CHECK_EVERY);
    let o = r.radar.check().await.unwrap();
    assert!(o.auto.is_empty());
    assert_eq!(r.lidarr.list.lock().len(), 1);
}

#[tokio::test]
async fn the_daily_cap_holds_refusals_dont_count_and_the_rest_wait_for_tomorrow() {
    let r = auto_rig();
    r.settings(|s| s.auto_request_cap = 2);
    r.radar.check().await.unwrap();
    r.later(CHECK_EVERY);
    r.mb.route(
        "/release-group?query=",
        Some(answer(vec![album(20, "A Side", 5), album(21, "Refused Record", 4), album(22, "B Side", 3), album(23, "C Side", 2)])),
    );
    let o = r.radar.check().await.unwrap();
    let asked: Vec<(String, RequestState)> = o.auto.iter().map(|x| (x.title.clone(), x.state)).collect();
    assert_eq!(
        asked,
        vec![("A Side".into(), RequestState::Searching), ("Refused Record".into(), RequestState::Failed), ("B Side".into(), RequestState::Searching)]
    );
    assert_eq!(o.waiting, 1);
    let summary = r.notes.0.lock().iter().find(|(t, _)| t == "Auto-request").cloned().unwrap().1;
    assert_eq!(
        summary,
        "Requested 2 new albums: A Side, B Side. 1 couldn't be requested. Requests in Settings say why. 1 more waits for tomorrow (up to 2 a day)."
    );
    // Later the same day: the cap is used up.
    let day = (*r.now.lock()).div_euclid(DAY);
    r.later(3600);
    assert_eq!((*r.now.lock()).div_euclid(DAY), day);
    let o = r.radar.check().await.unwrap();
    assert!(o.auto.is_empty());
    assert_eq!(o.waiting, 1);
    // The next day.
    r.later(CHECK_EVERY);
    assert!((*r.now.lock()).div_euclid(DAY) > day);
    let o = r.radar.check().await.unwrap();
    assert_eq!(o.auto.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["C Side"], "tomorrow, the one that waited");
}

#[tokio::test]
async fn auto_request_waits_for_the_switch_and_a_ready_lidarr() {
    let r = auto_rig();
    r.radar.check().await.unwrap();
    r.later(CHECK_EVERY);
    r.mb.route("/release-group?query=", Some(answer(vec![album(20, "Paper Moon", 1)])));
    r.settings(|s| s.auto_request = false);
    assert!(r.radar.check().await.unwrap().auto.is_empty(), "switched off for everyone");
    r.settings(|s| s.auto_request = true);
    r.lidarr.ready.store(false, Ordering::Relaxed);
    r.later(CHECK_EVERY);
    assert!(r.radar.check().await.unwrap().auto.is_empty(), "Lidarr not set up or its check not green");
    r.lidarr.ready.store(true, Ordering::Relaxed);
    r.later(CHECK_EVERY);
    assert_eq!(r.radar.check().await.unwrap().auto.len(), 1, "then it goes");
}

#[test]
fn summary_texts() {
    let rec = |t: &str, st: RequestState| RequestRecord {
        id: t.into(),
        release_group_id: id(1),
        title: t.into(),
        artist: "Northlight".into(),
        artist_id: None,
        artist_mbid: None,
        state: st,
        destination: None,
        lidarr_album_id: None,
        added_artist: false,
        already_in_lidarr: false,
        created_at: 0,
        updated_at: 0,
        message: None,
        notes: vec![],
        auto: true,
        protected: false,
        progress: None,
        searched_at: None,
        searches: 0,
        next_search_at: None,
    };
    assert_eq!(summary(&[], 0, 3), None);
    assert_eq!(summary(&[], 2, 3).unwrap().1, "2 more wait for tomorrow (up to 3 a day).");
    assert_eq!(
        summary(&[rec("X", RequestState::Failed), rec("Y", RequestState::Failed)], 0, 3).unwrap().1,
        "2 couldn't be requested. Requests in Settings say why."
    );
    assert_eq!(summary(&[rec("Q", RequestState::Queued)], 0, 3).unwrap().1, "Requested Q by Northlight.");
}
