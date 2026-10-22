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
const COLD: &str = "eeeeeeee-1111-2222-3333-444444444444";

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
            "Cold Coast" => Some(COLD.into()),
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
    /// Claims that aren't simply ours, by event.
    claims: Mutex<HashMap<String, ClaimState>>,
    /// The shared day counts, by device ("me" is this one).
    days: Mutex<HashMap<String, (i64, usize)>>,
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
            attempts: 0,
            quick: false,
            cleared: Vec::new(),
            practiced_clears: Vec::new(),
            clear_fails: 0,
            clear_after: None,
        };
        self.list.lock().push(r.clone());
        Ok(r)
    }
    fn claim(&self, event: &str) -> AppResult<ClaimState> {
        Ok(self.claims.lock().get(event).cloned().unwrap_or(ClaimState::Ours))
    }
    fn asked_on(&self, day: i64) -> usize {
        self.days.lock().values().filter(|d| d.0 == day).map(|d| d.1).sum()
    }
    fn note_asked(&self, day: i64) {
        let mut days = self.days.lock();
        let n = days.get("me").filter(|d| d.0 == day).map_or(0, |d| d.1);
        days.insert("me".into(), (day, n + 1));
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
    follows.save(&[FollowedArtist { id: "ar1".into(), title: "Northlight".into(), thumb: None, followed_at: 1, just_show: false }]).unwrap();
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
    r
}

fn album(n: u32, title: &str, ago: i64) -> Value {
    rg(&id(n), title, "Album", &[], &days_ago(ago), &["Official"], ac(ARTIST, "Northlight"))
}

#[tokio::test]
async fn following_with_lidarr_ready_turns_the_radar_on() {
    let r = rig();
    r.settings(|s| s.release_radar = false);
    let st = crate::settings::SettingsState::new(r.db.clone(), Default::default());
    // No Lidarr: following stays a follow, nothing is sent to MusicBrainz.
    r.radar.followed(&st).await.unwrap();
    assert!(!crate::settings::load(&r.db).release_radar);
    // Just show: the radar is the user's own choice then.
    r.lidarr.ready.store(true, Ordering::Relaxed);
    r.settings(|s| s.follow_get = false);
    r.radar.followed(&st).await.unwrap();
    assert!(!crate::settings::load(&r.db).release_radar);
    r.settings(|s| s.follow_get = true);
    r.radar.followed(&st).await.unwrap();
    assert!(crate::settings::load(&r.db).release_radar, "the radar is what finds their new albums");
    assert!(r.events.0.lock().iter().any(|e| e == "settings://changed"));
}

#[tokio::test]
async fn old_auto_request_artists_become_followed_once() {
    let r = rig();
    r.db.write(|c| {
        crate::db::store::kv_set(
            c,
            LEGACY_AUTO_KEY,
            r#"[{"id":"ar1","name":"Northlight","enabledAt":5},{"id":"ar9","name":"Cold Coast","enabledAt":40},{"id":"","name":"x","enabledAt":1}]"#,
        )
    })
    .unwrap();
    assert_eq!(r.radar.migrate_auto().unwrap(), 1, "Northlight was followed already; the empty id is dropped");
    let all = r.follows.list().unwrap();
    let cc = all.iter().find(|f| f.id == "ar9").unwrap();
    assert_eq!((cc.title.as_str(), cc.followed_at, cc.just_show), ("Cold Coast", 40, false), "followed when auto-request was turned on");
    assert_eq!(all.iter().find(|f| f.id == "ar1").unwrap().followed_at, 1, "an earlier follow keeps its date");
    assert_eq!(r.radar.migrate_auto().unwrap(), 0, "only once");
}

#[tokio::test]
async fn every_followed_artist_is_looked_at_and_played_most_only_fill_up() {
    let r = rig();
    let many: Vec<FollowedArtist> =
        (0..70).map(|n| FollowedArtist { id: format!("f{n}"), title: format!("Followed {n}"), thumb: None, followed_at: 1, just_show: false }).collect();
    r.follows.save(&many).unwrap();
    let a = r.radar.artists();
    assert_eq!(a.len(), 70, "past the 60 the played-most list fills to, and Harbor Sound has no room");
    assert!(a.iter().all(|x| x.name.starts_with("Followed")));
    r.follows.save(&many[..5]).unwrap();
    assert_eq!(r.radar.artists().len(), 6, "five followed and one played most");
}

#[tokio::test]
async fn following_gets_new_albums_and_eps_found_after_the_follow() {
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
            rg(&id(24), "Moon EP", "EP", &[], &days_ago(2), &["Official"], ac(ARTIST, "Northlight")),
            rg(&id(23), "Harbor Again", "Album", &[], &days_ago(1), &["Official"], ac(HARBOR, "Harbor Sound")),
        ])),
    );
    let o = r.radar.check().await.unwrap();
    let asked: Vec<String> = o.auto.iter().map(|x| x.title.clone()).collect();
    assert_eq!(asked, vec!["Moon EP", "Paper Moon"], "albums and EPs, not singles, and only for followed artists");
    let summary = r.notes.0.lock().iter().find(|(t, _)| t == "New from artists you follow").cloned().unwrap();
    assert_eq!(summary.1, "Getting 2 new albums: Moon EP, Paper Moon.");
    let item = r.radar.page().unwrap().items.into_iter().find(|i| i.title == "Paper Moon").unwrap();
    assert_eq!(item.request, Some(RequestState::Searching));
    assert!(item.auto);
    // Never asked twice.
    r.later(CHECK_EVERY);
    let o = r.radar.check().await.unwrap();
    assert!(o.auto.is_empty());
    assert_eq!(r.lidarr.list.lock().len(), 2);
}

#[tokio::test]
async fn only_one_device_asks_and_a_new_claim_waits_for_the_next_pass() {
    let r = auto_rig();
    r.radar.check().await.unwrap();
    r.later(CHECK_EVERY);
    r.mb.route("/release-group?query=", Some(answer(vec![album(20, "Paper Moon", 1), album(21, "Second Wind", 2)])));
    // Another device claimed Second Wind first; Paper Moon's claim is new.
    r.lidarr.claims.lock().insert(format!("follow:{}", id(20)), ClaimState::Settling);
    r.lidarr.claims.lock().insert(
        format!("follow:{}", id(21)),
        ClaimState::Theirs(crate::lidarr::shared::Claim { device: "phone".into(), label: "iPhone".into(), at: 1 }),
    );
    let o = r.radar.check().await.unwrap();
    assert!(o.auto.is_empty(), "nothing asked: one is theirs, one is settling");
    assert!(r.lidarr.list.lock().is_empty());
    // Half an hour on, the claim has settled: the waiting one is asked for, the other never.
    r.lidarr.claims.lock().remove(&format!("follow:{}", id(20)));
    r.lidarr.claims.lock().remove(&format!("follow:{}", id(21)));
    r.later(1800);
    let made = r.radar.get_waiting().await.unwrap();
    assert_eq!(made.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["Paper Moon"]);
    assert!(r.notes.0.lock().iter().any(|(t, b)| t == "New from artists you follow" && b == "Getting Paper Moon by Northlight."));
    // Nothing left: no more asks and no more notifications.
    let n = r.notes.0.lock().len();
    assert!(r.radar.get_waiting().await.unwrap().is_empty());
    assert_eq!(r.notes.0.lock().len(), n);
}

#[tokio::test]
async fn singles_too_just_show_and_releases_found_before_the_follow() {
    let r = auto_rig();
    r.top.0.lock().push(RadarArtist { id: Some("ar3".into()), name: "Cold Coast".into() });
    r.radar.check().await.unwrap();
    r.later(CHECK_EVERY);
    // Cold Coast is only played a lot: its new album is shown, never got.
    r.mb.route("/release-group?query=", Some(answer(vec![rg(&id(30), "Tidewater", "Album", &[], &days_ago(1), &["Official"], ac(COLD, "Cold Coast"))])));
    assert!(r.radar.check().await.unwrap().auto.is_empty(), "not followed");
    // Following now doesn't reach back to what was found before.
    let now = *r.now.lock();
    let mut all = r.follows.list().unwrap();
    all.push(FollowedArtist { id: "ar3".into(), title: "Cold Coast".into(), thumb: None, followed_at: now, just_show: false });
    r.follows.save(&all).unwrap();
    r.later(CHECK_EVERY);
    assert!(r.radar.check().await.unwrap().auto.is_empty(), "found before the follow");
    // Singles once Singles too is on; nothing for an artist set to just show.
    r.settings(|s| s.follow_get_singles = true);
    crate::social::follows::set_just_show(r.follows.as_ref(), "ar1", true).unwrap();
    r.later(CHECK_EVERY);
    r.mb.route(
        "/release-group?query=",
        Some(answer(vec![
            rg(&id(31), "Low Tide", "Single", &[], &days_ago(1), &["Official"], ac(COLD, "Cold Coast")),
            album(32, "Hidden Away", 1),
        ])),
    );
    let o = r.radar.check().await.unwrap();
    assert_eq!(o.auto.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["Low Tide"]);
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
    let summary = r.notes.0.lock().iter().find(|(t, _)| t == "New from artists you follow").cloned().unwrap().1;
    assert_eq!(
        summary,
        "Getting 2 new albums: A Side, B Side. 1 couldn't be asked for. Requests in Settings say why. 1 more waits for tomorrow (up to 2 a day)."
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
async fn new_albums_wait_for_the_switch_and_a_ready_lidarr() {
    let r = auto_rig();
    r.radar.check().await.unwrap();
    r.later(CHECK_EVERY);
    r.mb.route("/release-group?query=", Some(answer(vec![album(20, "Paper Moon", 1)])));
    r.settings(|s| s.follow_get = false);
    assert!(r.radar.check().await.unwrap().auto.is_empty(), "just show them, for everyone");
    r.settings(|s| s.follow_get = true);
    r.later(CHECK_EVERY);
    assert!(r.radar.check().await.unwrap().auto.is_empty(), "turned back on, it doesn't reach back for what only showed");
    r.lidarr.ready.store(false, Ordering::Relaxed);
    r.later(CHECK_EVERY);
    r.mb.route("/release-group?query=", Some(answer(vec![album(20, "Paper Moon", 3), album(21, "Second Wind", 1)])));
    assert!(r.radar.check().await.unwrap().auto.is_empty(), "Lidarr not set up or its check not green");
    r.lidarr.ready.store(true, Ordering::Relaxed);
    r.later(CHECK_EVERY);
    let o = r.radar.check().await.unwrap();
    assert_eq!(o.auto.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["Second Wind"], "then the new one goes");
}

#[tokio::test]
async fn after_an_update_and_when_set_back_to_get_nothing_older_is_asked_for() {
    let r = auto_rig();
    r.radar.check().await.unwrap();
    r.later(CHECK_EVERY);
    r.mb.route("/release-group?query=", Some(answer(vec![album(20, "Paper Moon", 1)])));
    // An older build found Paper Moon and only showed it: no pass then.
    r.lidarr.ready.store(false, Ordering::Relaxed);
    r.radar.check().await.unwrap();
    r.db.write(|c| {
        let mut st: serde_json::Value = serde_json::from_str(&crate::db::store::kv_get(c, STORE_KEY)?.unwrap()).unwrap();
        st.as_object_mut().unwrap().remove("getting");
        crate::db::store::kv_set(c, STORE_KEY, &st.to_string())
    })
    .unwrap();
    r.lidarr.ready.store(true, Ordering::Relaxed);
    r.later(CHECK_EVERY);
    assert!(r.radar.check().await.unwrap().auto.is_empty(), "what an older build showed stays shown only");
    // Just show for a while, then get again: only what comes out after.
    crate::social::follows::set_just_show(r.follows.as_ref(), "ar1", true).unwrap();
    r.later(CHECK_EVERY);
    r.mb.route("/release-group?query=", Some(answer(vec![album(20, "Paper Moon", 3), album(21, "Second Wind", 1)])));
    assert!(r.radar.check().await.unwrap().auto.is_empty());
    crate::social::follows::set_just_show(r.follows.as_ref(), "ar1", false).unwrap();
    r.later(CHECK_EVERY);
    r.mb.route(
        "/release-group?query=",
        Some(answer(vec![album(20, "Paper Moon", 4), album(21, "Second Wind", 2), album(22, "Third Light", 1)])),
    );
    let o = r.radar.check().await.unwrap();
    assert_eq!(o.auto.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["Third Light"]);
}

#[tokio::test]
async fn old_auto_request_artists_just_show_when_the_old_switch_was_off() {
    let r = rig();
    r.settings(|s| s.legacy_auto_request = Some(false));
    r.db.write(|c| crate::db::store::kv_set(c, LEGACY_AUTO_KEY, r#"[{"id":"ar9","name":"Cold Coast","enabledAt":40}]"#)).unwrap();
    assert_eq!(r.radar.migrate_auto().unwrap(), 1);
    assert!(r.follows.list().unwrap().iter().find(|f| f.id == "ar9").unwrap().just_show, "followed, never got");
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
        attempts: 0,
        quick: false,
        cleared: Vec::new(),
        practiced_clears: Vec::new(),
        clear_fails: 0,
        clear_after: None,
    };
    assert_eq!(summary(&[], 0, 3), None);
    assert_eq!(summary(&[], 2, 3).unwrap().1, "2 more wait for tomorrow (up to 3 a day).");
    assert_eq!(
        summary(&[rec("X", RequestState::Failed), rec("Y", RequestState::Failed)], 0, 3).unwrap().1,
        "2 couldn't be asked for. Requests in Settings say why."
    );
    assert_eq!(summary(&[rec("Q", RequestState::Queued)], 0, 3).unwrap().1, "Getting Q by Northlight.");
}

#[tokio::test]
async fn every_devices_asks_count_toward_the_daily_cap() {
    let r = auto_rig();
    r.settings(|s| s.auto_request_cap = 3);
    r.radar.check().await.unwrap();
    r.later(CHECK_EVERY);
    r.mb.route("/release-group?query=", Some(answer(vec![album(20, "A Side", 5), album(21, "B Side", 4), album(22, "C Side", 3)])));
    // A Bridge already asked for two today.
    let today = (*r.now.lock()).div_euclid(DAY);
    r.lidarr.days.lock().insert("bridge".into(), (today, 2));
    let o = r.radar.check().await.unwrap();
    assert_eq!(o.auto.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["A Side"]);
    assert_eq!(o.waiting, 2);
    assert_eq!(r.lidarr.days.lock().get("me"), Some(&(today, 1)), "this one counts only its own ask");
    assert_eq!(r.lidarr.asked_on(today), 3);
    // Yesterday's count doesn't hold today back.
    r.later(CHECK_EVERY);
    assert!((*r.now.lock()).div_euclid(DAY) > today);
    let o = r.radar.check().await.unwrap();
    assert_eq!(o.auto.len(), 2);
}

#[tokio::test]
async fn the_bridge_plan_has_the_follows_that_get_new_albums_with_their_ids() {
    let r = auto_rig();
    r.follows
        .save(&[
            FollowedArtist { id: "ar1".into(), title: "Northlight".into(), thumb: None, followed_at: 10, just_show: false },
            FollowedArtist { id: "ar2".into(), title: "Harbor Sound".into(), thumb: Some("/t".into()), followed_at: 20, just_show: true },
            FollowedArtist { id: "ar9".into(), title: "Nobody Known".into(), thumb: None, followed_at: 30, just_show: false },
        ])
        .unwrap();
    r.settings(|s| {
        s.follow_get_singles = true;
        s.auto_request_cap = 5;
    });
    // An artist followed since the last look has their id looked up for it.
    let p = r.radar.bridge_plan().await;
    assert!(p.on && p.singles);
    assert_eq!(p.cap, 5);
    assert_eq!(
        p.follows,
        vec![
            bridge::PlanFollow { id: "ar1".into(), title: "Northlight".into(), mbid: Some(ARTIST.into()), followed_at: 10 },
            bridge::PlanFollow { id: "ar9".into(), title: "Nobody Known".into(), mbid: None, followed_at: 30 },
        ],
        "just-show follows stay on the device"
    );
    // Off here: off there, and nobody's follows travel.
    r.settings(|s| s.follow_get = false);
    let p = r.radar.bridge_plan().await;
    assert!(!p.on);
    assert!(p.follows.is_empty());
    r.settings(|s| {
        s.follow_get = true;
        s.release_radar = false;
    });
    assert!(!r.radar.bridge_plan().await.on);
}

struct BridgeRig {
    _dir: tempfile::TempDir,
    db: DbHandle,
    mb: Arc<FakeMb>,
    lidarr: Arc<FakeLidarr>,
    now: Arc<Mutex<i64>>,
    radar: Radar,
}

/// The radar as the Bridge builds it, on a database of its own.
fn bridge_rig() -> BridgeRig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("bridge.db")).unwrap();
    let mb = Arc::new(FakeMb::default());
    mb.route("/release-group?query=", Some(answer(vec![album(20, "Paper Moon", 3)])));
    let lidarr = Arc::new(FakeLidarr::default());
    lidarr.ready.store(true, Ordering::Relaxed);
    *lidarr.clock.lock() = NOW;
    let now = Arc::new(Mutex::new(NOW));
    let n = now.clone();
    let radar = Radar::new(bridge::deps(db.clone(), mb.clone(), lidarr.clone(), Arc::new(Events::default()), Arc::new(Notes::default()), Arc::new(move || *n.lock())));
    BridgeRig { _dir: dir, db, mb, lidarr, now, radar }
}

fn plan_for_northlight() -> bridge::RadarPlan {
    bridge::RadarPlan {
        on: true,
        singles: false,
        cap: 2,
        follows: vec![bridge::PlanFollow { id: "ar1".into(), title: "Northlight".into(), mbid: Some(ARTIST.into()), followed_at: 1 }],
    }
}

#[test]
fn the_bridge_takes_a_plan_into_its_settings() {
    let BridgeRig { _dir, db, radar, .. } = bridge_rig();
    assert!(!radar.due(), "nothing to look for before a device hands a plan over");
    assert!(bridge::take_plan(&db, "desk", Some(&plan_for_northlight())).unwrap());
    assert!(!bridge::take_plan(&db, "desk", Some(&plan_for_northlight())).unwrap(), "the same plan again changes nothing");
    let s = crate::settings::load(&db);
    assert!(s.release_radar && s.follow_get && !s.follow_get_singles && !s.radar_notify);
    assert_eq!(s.auto_request_cap, 2);
    assert!(radar.due());
    assert_eq!(radar.artists(), vec![RadarArtist { id: Some("ar1".into()), name: "Northlight".into() }], "followed artists only, no listening of its own");
    // A cap no device offers falls back to the usual one; off stops it and drops the follows.
    let mut p = plan_for_northlight();
    p.cap = 99;
    bridge::take_plan(&db, "desk", Some(&p)).unwrap();
    assert_eq!(crate::settings::load(&db).auto_request_cap, DEFAULT_CAP);
    p.on = false;
    bridge::take_plan(&db, "desk", Some(&p)).unwrap();
    assert!(!crate::settings::load(&db).release_radar);
    assert!(!radar.due());
    assert!(bridge::plan(&db).follows.is_empty());
    assert_eq!(bridge::plan_links(&db), vec!["desk".to_string()], "it still reads what the Bridge asked for");
}

#[tokio::test]
async fn the_bridge_looks_and_asks_like_a_device_and_says_what_it_asked_for() {
    let BridgeRig { _dir, db, mb, lidarr, now, radar, .. } = bridge_rig();
    bridge::take_plan(&db, "desk", Some(&plan_for_northlight())).unwrap();
    // Its first look only sets the scene.
    let o = radar.check().await.unwrap();
    assert!(o.auto.is_empty());
    *now.lock() += CHECK_EVERY;
    *lidarr.clock.lock() += CHECK_EVERY;
    mb.route("/release-group?query=", Some(answer(vec![album(20, "Paper Moon", 3), album(21, "Second Wind", 1), album(22, "Refused Record", 1)])));
    lidarr.claims.lock().insert(
        format!("follow:{}", id(21)),
        ClaimState::Theirs(crate::lidarr::shared::Claim { device: "phone".into(), label: "iPhone".into(), at: 1 }),
    );
    let o = radar.check().await.unwrap();
    assert_eq!(
        o.auto.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(),
        vec!["Refused Record"],
        "the phone has Second Wind, and Paper Moon was there at the first look"
    );
    *now.lock() += CHECK_EVERY;
    *lidarr.clock.lock() += CHECK_EVERY;
    mb.route("/release-group?query=", Some(answer(vec![album(23, "Paper Sun", 1)])));
    let o = radar.check().await.unwrap();
    assert_eq!(o.auto.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["Paper Sun"]);
    // What goes back to the devices: what Lidarr has, not refusals.
    let t = *now.lock();
    bridge::note_made(&db, &[lidarr.list.lock()[0].clone()], t).unwrap();
    bridge::note_made(&db, &o.auto, t).unwrap();
    let back = bridge::made_records(&db, &lidarr.list.lock(), t);
    assert_eq!(back.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["Paper Sun"]);
    // One still waiting to be sent stays on the Bridge, so no device sends it too.
    lidarr.list.lock().iter_mut().for_each(|r| r.state = RequestState::Queued);
    assert!(bridge::made_records(&db, &lidarr.list.lock(), t).is_empty());
    // And after a fortnight it's no longer sent.
    lidarr.list.lock().iter_mut().for_each(|r| r.state = RequestState::Imported);
    assert_eq!(bridge::made_records(&db, &lidarr.list.lock(), t).len(), 1);
    assert!(bridge::made_records(&db, &lidarr.list.lock(), t + bridge::MADE_FOR_S).is_empty());
}

#[tokio::test]
async fn the_bridge_knows_artists_ids_from_the_plan() {
    use crate::lidarr::ArtistMbid;
    let BridgeRig { _dir, db, .. } = bridge_rig();
    bridge::take_plan(&db, "desk", Some(&plan_for_northlight())).unwrap();
    let ids = bridge::PlanMbids(db.clone());
    assert_eq!(ids.artist_mbid("ar1", "Someone Else").await.as_deref(), Some(ARTIST), "by id");
    assert_eq!(ids.artist_mbid("", "NORTHLIGHT").await.as_deref(), Some(ARTIST), "by name without an id");
    assert_eq!(ids.artist_mbid("other-id", "Northlight").await, None, "a same-name artist with another id isn't this one");
    assert_eq!(ids.artist_mbid("other-id", "Harbor Sound").await, None);
}

#[test]
fn the_newest_plan_wins_and_an_unpaired_device_is_forgotten() {
    let BridgeRig { _dir, db, .. } = bridge_rig();
    let mut phone = plan_for_northlight();
    phone.singles = true;
    phone.cap = 5;
    phone.follows = vec![bridge::PlanFollow { id: "ar3".into(), title: "Cold Coast".into(), mbid: Some(COLD.into()), followed_at: 60 }];
    bridge::take_plan(&db, "desk", Some(&plan_for_northlight())).unwrap();
    assert!(bridge::take_plan(&db, "phone", Some(&phone)).unwrap());
    // Northlight was unfollowed on the phone: the computer's older list doesn't keep it.
    assert_eq!(bridge::plan(&db), phone);
    let s = crate::settings::load(&db);
    assert!(s.follow_get_singles);
    assert_eq!(s.auto_request_cap, 5);
    assert_eq!(bridge::plan_links(&db), vec!["desk".to_string(), "phone".to_string()]);
    // Turned off on the phone: off.
    bridge::take_plan(&db, "phone", Some(&bridge::RadarPlan::default())).unwrap();
    assert!(!crate::settings::load(&db).release_radar);
    bridge::take_plan(&db, "phone", None).unwrap();
    assert_eq!(bridge::plan_links(&db), vec!["desk".to_string()]);
}

#[tokio::test]
async fn a_release_already_on_the_requests_list_isnt_asked_for_or_claimed() {
    let r = auto_rig();
    r.radar.check().await.unwrap();
    r.later(CHECK_EVERY);
    r.mb.route("/release-group?query=", Some(answer(vec![album(20, "Paper Moon", 1)])));
    // A Bridge's request this device took in, under the release's id in capitals.
    let theirs = r.lidarr.request_auto(RequestInput {
        release_group_id: id(20).to_ascii_uppercase(),
        title: "Paper Moon".into(),
        artist: "Northlight".into(),
        artist_id: Some("ar1".into()),
        artist_mbid: None,
        typed: None,
    });
    theirs.await.unwrap();
    r.lidarr.claims.lock().insert(format!("follow:{}", id(20)), ClaimState::Settling);
    let o = r.radar.check().await.unwrap();
    assert!(o.auto.is_empty());
    assert_eq!(r.lidarr.list.lock().len(), 1);
    r.lidarr.claims.lock().clear();
    r.later(1800);
    assert!(r.radar.get_waiting().await.unwrap().is_empty(), "passed over for good");
    assert_eq!(r.lidarr.asked_on((*r.now.lock()).div_euclid(DAY)), 0);
}
