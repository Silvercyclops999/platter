//! Discover with MusicBrainz, ListenBrainz and Lidarr faked. Nothing here
//! reaches a real server.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::lb::{LbCached, SimilarArtist, SimilarArtists};
use super::*;
use crate::artistinfo::http::fake::FakeHttp;
use crate::artistinfo::limit::Limiter;
use crate::lidarr::mbpick::tests::{ARTIST, FakeMb, OTHER, RG_BOOT, RG_SKETCH, RG_TIDES, rg};
use crate::lidarr::mbpick::{MbCached, VARIOUS_ARTISTS};
use crate::radar::RadarArtist;

const HARBOR: &str = "cccccccc-1111-2222-3333-444444444444";
const COLD: &str = "dddddddd-1111-2222-3333-444444444444";
const FEN: &str = "eeeeeeee-1111-2222-3333-444444444444";
const NOPE: &str = "ffffffff-1111-2222-3333-444444444444";
const REL_A: &str = "30000000-0000-0000-0000-000000000001";
const REL_B: &str = "30000000-0000-0000-0000-000000000002";
const NOW: i64 = 1_800_000_000;

fn ac(id: &str, name: &str) -> Value {
    json!([{"name": name, "artist": {"id": id, "name": name}}])
}

#[derive(Default)]
struct FakeSimilar {
    map: Mutex<HashMap<String, Vec<SimilarArtist>>>,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl SimilarArtists for FakeSimilar {
    async fn similar(&self, mbid: &str) -> AppResult<Vec<SimilarArtist>> {
        self.calls.lock().push(mbid.to_string());
        Ok(self.map.lock().get(mbid).cloned().unwrap_or_default())
    }
}

fn sim(mbid: &str, name: &str, score: u64) -> SimilarArtist {
    SimilarArtist { mbid: mbid.into(), name: name.into(), score }
}

struct Mbids;

#[async_trait]
impl ArtistMbid for Mbids {
    async fn artist_mbid(&self, _id: &str, name: &str) -> Option<String> {
        match name {
            "Harbor Sound" => Some(HARBOR.into()),
            "Northlight" => Some(ARTIST.into()),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Top(Mutex<Vec<RadarArtist>>);

impl TopPlayed for Top {
    fn top(&self, limit: usize) -> Vec<RadarArtist> {
        self.0.lock().iter().take(limit).cloned().collect()
    }
}

struct Info;

#[async_trait]
impl CatalogueInfo for Info {
    async fn bio(&self, mbid: &str, _name: &str) -> Option<CatalogueBio> {
        (mbid == ARTIST).then(|| CatalogueBio {
            bio: Some("Northlight are a band from Leeds.".into()),
            article_url: Some("https://en.wikipedia.org/wiki/Northlight".into()),
            photo: None,
            banner: None,
        })
    }
}

#[derive(Default)]
struct Requests(Mutex<HashMap<String, RequestState>>);

impl RequestStates for Requests {
    fn states(&self) -> HashMap<String, RequestState> {
        self.0.lock().clone()
    }
}

struct FakePrefs(Mutex<Prefs>);

impl PrefsSource for FakePrefs {
    fn prefs(&self) -> Prefs {
        self.0.lock().clone()
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    db: DbHandle,
    mb: Arc<FakeMb>,
    similar: Arc<FakeSimilar>,
    prefs: Arc<FakePrefs>,
    requests: Arc<Requests>,
    offline: Arc<AtomicBool>,
    d: Discover,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("d.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            "INSERT INTO artists (id, title, sort_title) VALUES ('ar-h', 'Harbor Sound', 'Harbor Sound');
             INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('al-h', 'Harbor Lights', 'Harbor Lights', 'ar-h', 'Harbor Sound', 'Album', '[]', 1);
             INSERT INTO song_index (track_id, artist_fold, title_fold, artist_id) VALUES ('t-h', 'harbor sound', 'pier song', 'ar-h');",
        )?;
        crate::artistinfo::cache::put(c, "ar-h", crate::artistinfo::cache::MBID_ROW, Some(&HARBOR.to_string()), NOW, 1_000_000)?;
        Ok(())
    })
    .unwrap();

    let mb = FakeMb::northlight();
    mb.route(
        "/artist?query=",
        Some(json!({"artists": [
            {"id": ARTIST, "name": "Northlight", "type": "Group", "disambiguation": "Leeds band", "score": 100},
            {"id": HARBOR, "name": "Harbor Sound", "score": 96},
            {"id": NOPE, "name": "Northern Lights Orchestra", "score": 40},
            {"id": VARIOUS_ARTISTS, "name": "Various Artists", "score": 90}
        ]})),
    );
    let mut tides = rg(RG_TIDES, "Tides", "Album", &[], "2017", &["Official"], ac(ARTIST, "Northlight"));
    tides["score"] = json!(100);
    let mut boot = rg(RG_BOOT, "Basement Tapes", "Album", &[], "2018", &["Bootleg"], ac(ARTIST, "Northlight"));
    boot["score"] = json!(100);
    let mut lights = rg("40000000-0000-0000-0000-000000000001", "Harbor Lights", "Album", &[], "2019", &["Official"], ac(HARBOR, "Harbor Sound"));
    lights["score"] = json!(100);
    let mut va = rg("40000000-0000-0000-0000-000000000002", "Summer Hits", "Album", &["Compilation"], "2016", &["Official"], ac(VARIOUS_ARTISTS, "Various Artists"));
    va["score"] = json!(100);
    mb.route("/release-group?query=", Some(json!({"release-groups": [tides, boot, lights, va]})));
    let rel = |rgid: &str, status: &str| json!({"id": "50000000-0000-0000-0000-000000000001", "status": status, "release-group": {"id": rgid, "title": "Tides", "primary-type": "Album"}});
    mb.route(
        "/recording?query=",
        Some(json!({"recordings": [
            {"id": "60000000-0000-0000-0000-000000000001", "score": 100, "title": "Tides", "length": 201000, "artist-credit": ac(ARTIST, "Northlight"), "releases": [rel(RG_TIDES, "Official")]},
            {"id": "60000000-0000-0000-0000-000000000002", "score": 100, "title": "Pier Song", "artist-credit": ac(HARBOR, "Harbor Sound"), "releases": [rel("40000000-0000-0000-0000-000000000001", "Official")]},
            {"id": "60000000-0000-0000-0000-000000000003", "score": 100, "title": "Demo", "artist-credit": ac(ARTIST, "Northlight"), "releases": [rel(RG_BOOT, "Bootleg")]}
        ]})),
    );
    mb.route(
        &format!("/artist/{ARTIST}?"),
        Some(json!({"id": ARTIST, "name": "Northlight", "type": "Group", "area": {"name": "Leeds"}, "life-span": {"begin": "2010"}, "relations": []})),
    );
    mb.route(
        &format!("/artist/{HARBOR}?"),
        Some(json!({"id": HARBOR, "name": "Harbor Sound", "relations": [
            {"type": "collaboration", "direction": "forward", "target-type": "artist", "artist": {"id": OTHER, "name": "Guest"}}
        ]})),
    );
    // Tides with two official releases and its tracklist.
    let mut tides_lookup = rg(RG_TIDES, "Tides", "Album", &[], "2017", &["Official"], ac(ARTIST, "Northlight"));
    tides_lookup["releases"] = json!([
        {"id": REL_B, "status": "Official", "date": "2018-05-01"},
        {"id": REL_A, "status": "Official", "date": "2017-02-01"},
        {"id": "30000000-0000-0000-0000-000000000003", "status": "Bootleg", "date": "2016"}
    ]);
    mb.route(&format!("release-group/{RG_TIDES}?"), Some(tides_lookup));
    mb.route(
        &format!("/release/{REL_A}?"),
        Some(json!({"id": REL_A, "date": "2017-02-01", "artist-credit": ac(ARTIST, "Northlight"),
            "label-info": [{"label": {"name": "Pier Records"}}],
            "media": [{"position": 1, "tracks": [
                {"position": 1, "title": "Undertow", "length": 200000, "artist-credit": ac(ARTIST, "Northlight")},
                {"position": 2, "title": "Tides", "length": 250000, "artist-credit": json!([{"name": "Northlight", "joinphrase": " feat. ", "artist": {"id": ARTIST, "name": "Northlight"}}, {"name": "Guest", "artist": {"id": OTHER, "name": "Guest"}}])}
            ]}]})),
    );

    let similar = Arc::new(FakeSimilar::default());
    similar.map.lock().insert(ARTIST.into(), vec![sim(HARBOR, "Harbor Sound", 900), sim(COLD, "Cold Coast", 800), sim(NOPE, "Dismissed Band", 700)]);
    similar.map.lock().insert(HARBOR.into(), vec![sim(ARTIST, "Northlight", 900), sim(COLD, "Cold Coast", 500), sim(FEN, "Fen", 400)]);
    let top = Arc::new(Top::default());
    top.0.lock().push(RadarArtist { id: Some("ar-h".into()), name: "Harbor Sound".into() });
    let prefs = Arc::new(FakePrefs(Mutex::new(Prefs { search: true, fans: true, new_to_you: true, dismissed: HashSet::from([NOPE.to_string()]) })));
    let requests = Arc::new(Requests::default());
    let offline = Arc::new(AtomicBool::new(false));
    let d = Discover::new(DiscoverDeps {
        db: db.clone(),
        mb: mb.clone(),
        similar: similar.clone(),
        mbids: Arc::new(Mbids),
        top,
        info: Arc::new(Info),
        requests: requests.clone(),
        prefs: prefs.clone(),
        offline: offline.clone(),
        clock: Arc::new(|| NOW),
    });
    Rig { _dir: dir, db, mb, similar, prefs, requests, offline, d }
}

#[tokio::test]
async fn search_shows_only_what_the_library_lacks() {
    let r = rig();
    let s = r.d.search("north").await.unwrap();
    assert!(s.shown);
    let names: Vec<&str> = s.artists.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Northlight"], "the library artist folds into the library, low scores and Various Artists are left out");
    assert_eq!(s.artists[0].disambiguation.as_deref(), Some("Leeds band"));
    let albums: Vec<&str> = s.albums.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(albums, ["Tides"], "bootlegs, Various Artists and albums the library has are left out");
    assert_eq!(s.albums[0].thumb.as_deref(), Some(format!("/ext/https://coverartarchive.org/release-group/{RG_TIDES}/front-250").as_str()));
    let songs: Vec<&str> = s.songs.iter().map(|x| x.title.as_str()).collect();
    assert_eq!(songs, ["Tides"], "songs the library has, and songs only on bootlegs, are left out");
    assert_eq!(s.songs[0].release_group_id.as_deref(), Some(RG_TIDES));
    // A library artist matched by name alone folds too.
    let ix = r.db.read(LibraryIndex::load).unwrap();
    assert_eq!(ix.artist(None, "  HARBOR sound ").as_deref(), Some("ar-h"));
    assert_eq!(ix.artist(Some(HARBOR), "Another Name").as_deref(), Some("ar-h"));
}

#[tokio::test]
async fn search_asks_nothing_while_off_or_offline() {
    let r = rig();
    r.prefs.0.lock().search = false;
    assert!(!r.d.search("north").await.unwrap().shown);
    r.prefs.0.lock().search = true;
    r.offline.store(true, Ordering::SeqCst);
    let s = r.d.search("north").await.unwrap();
    assert!(!s.shown && s.artists.is_empty());
    assert!(r.mb.calls.lock().is_empty(), "nothing reached MusicBrainz");
    r.offline.store(false, Ordering::SeqCst);
    assert!(r.d.search("n").await.unwrap().artists.is_empty(), "one letter isn't searched");
    assert!(r.mb.calls.lock().is_empty());
}

#[tokio::test(start_paused = true)]
async fn searches_are_paced_and_cached() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("c.db")).unwrap();
    let http = Arc::new(FakeHttp::new());
    http.route("musicbrainz.org/ws/2/artist?", 200, &json!({"artists": [{"id": ARTIST, "name": "Northlight", "score": 100}]}).to_string());
    http.route("musicbrainz.org/ws/2/release-group?", 200, r#"{"release-groups": []}"#);
    http.route("musicbrainz.org/ws/2/recording?", 200, r#"{"recordings": []}"#);
    let mb = Arc::new(MbCached { http: http.clone(), limiter: Arc::new(Limiter::per_second(1.0)), db: db.clone(), clock: Arc::new(|| NOW), ttl: 3600 });
    let d = Discover::new(DiscoverDeps {
        db: db.clone(),
        mb,
        similar: Arc::new(FakeSimilar::default()),
        mbids: Arc::new(Mbids),
        top: Arc::new(Top::default()),
        info: Arc::new(Info),
        requests: Arc::new(Requests::default()),
        prefs: Arc::new(FakePrefs(Mutex::new(Prefs { search: true, fans: true, new_to_you: true, dismissed: HashSet::new() }))),
        offline: Arc::new(AtomicBool::new(false)),
        clock: Arc::new(|| NOW),
    });
    let t0 = tokio::time::Instant::now();
    assert_eq!(d.search("Northlight").await.unwrap().artists.len(), 1);
    assert!(tokio::time::Instant::now() - t0 >= std::time::Duration::from_secs(2), "three requests at one a second");
    assert_eq!(http.calls("musicbrainz.org"), 3);
    d.search("Northlight").await.unwrap();
    assert_eq!(http.calls("musicbrainz.org"), 3, "the same search again comes from the cache");
    for (_, headers) in http.log.lock().iter() {
        assert!(headers.is_empty(), "nothing but the URL is sent");
    }
}

#[tokio::test]
async fn catalogue_artist_page_lists_official_releases_by_type() {
    let r = rig();
    let p = r.d.artist_page(ARTIST).await.unwrap();
    assert_eq!(p.artist.name, "Northlight");
    assert_eq!(p.artist.library_id, None);
    assert_eq!(p.area.as_deref(), Some("Leeds"));
    assert_eq!(p.bio.as_deref(), Some("Northlight are a band from Leeds."));
    assert_eq!(p.bio_credit.as_deref(), Some("Wikipedia"));
    let by: Vec<(String, Vec<String>)> = p.sections.iter().map(|s| (s.kind.clone(), s.albums.iter().map(|a| a.title.clone()).collect())).collect();
    assert_eq!(
        by,
        vec![
            ("album".to_string(), vec!["Quiet Sketches".to_string(), "Second Wind".into(), "Tides".into(), "First Light".into()]),
            ("ep".to_string(), vec!["Small Hours".to_string()]),
            ("single".to_string(), vec!["Second Wind".to_string()]),
            ("other".to_string(), vec!["Live at the Harbor".to_string()]),
        ],
        "bootlegs, promos, Various Artists and duets are left out; newest first"
    );
    // Similar artists: the library one keeps its library id, the dismissed one is gone.
    let sim: Vec<(&str, Option<&str>)> = p.similar.iter().map(|a| (a.name.as_str(), a.library_id.as_deref())).collect();
    assert_eq!(sim, [("Harbor Sound", Some("ar-h")), ("Cold Coast", None)]);
    assert!(!p.dismissed);
    r.prefs.0.lock().dismissed.insert(ARTIST.into());
    assert!(r.d.artist_page(ARTIST).await.unwrap().dismissed);
}

#[tokio::test]
async fn a_library_artist_points_at_their_own_page() {
    let r = rig();
    r.mb.route(&format!("/artist/{HARBOR}?"), Some(json!({"id": HARBOR, "name": "Harbor Sound", "relations": []})));
    r.mb.route("/release?artist=", Some(json!({"releases": []})));
    let p = r.d.artist_page(HARBOR).await.unwrap();
    assert_eq!(p.artist.library_id.as_deref(), Some("ar-h"));
    assert!(r.d.artist_page("not-an-id").await.is_err());
    r.offline.store(true, Ordering::SeqCst);
    assert!(r.d.artist_page(ARTIST).await.is_err(), "offline: no page");
}

#[tokio::test]
async fn catalogue_album_page_has_tracks_store_links_and_request_state() {
    let r = rig();
    let p = r.d.album_page(RG_TIDES).await.unwrap();
    assert_eq!(p.album.title, "Tides");
    assert_eq!(p.album.refusal, None, "an official album by the artist alone can be requested");
    let tracks: Vec<(u32, &str, Option<&str>)> = p.tracks.iter().map(|t| (t.number, t.title.as_str(), t.artist.as_deref())).collect();
    assert_eq!(tracks, [(1, "Undertow", None), (2, "Tides", Some("Northlight feat. Guest"))], "the earliest official release's tracks");
    assert_eq!(p.label.as_deref(), Some("Pier Records"));
    assert_eq!(p.released.as_deref(), Some("2017-02-01"));
    assert_eq!(p.links.bandcamp, "https://bandcamp.com/search?q=Northlight%20Tides&item_type=a");
    assert_eq!(p.links.qobuz, "https://www.qobuz.com/search?q=Northlight%20Tides");
    assert_eq!(p.links.itunes, "https://music.apple.com/search?term=Northlight%20Tides");
    assert!(crate::import::store::link_allowed(&p.links.bandcamp) && crate::import::store::link_allowed(&p.links.itunes));
    assert!(p.album.request.is_none() && !p.album.in_missing);

    r.requests.0.lock().insert(RG_TIDES.into(), RequestState::Queued);
    r.db.write(|c| {
        crate::import::store::add_wanted(
            c,
            crate::import::store::WantedAlbum { release_group_id: RG_TIDES.to_uppercase(), title: "Tides".into(), artist: "Northlight".into(), artist_mbid: Some(ARTIST.into()), added_at: 0 },
            NOW,
        )
    })
    .unwrap();
    let p = r.d.album_page(RG_TIDES).await.unwrap();
    assert_eq!(p.album.request, Some(RequestState::Queued));
    assert!(p.album.in_missing, "Add to Missing is remembered");
}

#[tokio::test]
async fn album_pages_say_why_something_cant_be_requested() {
    let r = rig();
    let p = r.d.album_page(RG_BOOT).await.unwrap();
    assert!(p.album.refusal.unwrap().contains("bootleg"));
    // An unreleased build with the same title in the library blocks it.
    r.db.write(|c| {
        c.execute_batch(
            "INSERT INTO artists (id, title, sort_title) VALUES ('ar1', 'Northlight', 'Northlight');
             INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('a2', 'Quiet Sketches (Build 2)', 'Quiet Sketches', 'ar1', 'Northlight', NULL, '[]', 0);",
        )?;
        Ok(())
    })
    .unwrap();
    let p = r.d.album_page(RG_SKETCH).await.unwrap();
    assert!(p.album.refusal.unwrap().contains("unreleased"));
    assert_eq!(p.artist_library_id.as_deref(), Some("ar1"));
}

#[tokio::test]
async fn fans_also_like_adds_artists_outside_the_library() {
    let r = rig();
    let got = r.d.fans_also_like("ar1", "Northlight").await.unwrap();
    let names: Vec<&str> = got.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Cold Coast"], "library artists and dismissed ones are left out");
    r.prefs.0.lock().fans = false;
    assert!(r.d.fans_also_like("ar1", "Northlight").await.unwrap().is_empty());
    r.prefs.0.lock().fans = true;
    assert!(r.d.fans_also_like("ar9", "Unknown").await.unwrap().is_empty(), "no MusicBrainz id: nothing");
}

#[tokio::test]
async fn new_to_you_comes_from_what_is_played_most() {
    let r = rig();
    let n = r.d.new_to_you().await.unwrap();
    assert!(n.shown);
    let names: Vec<&str> = n.artists.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Northlight", "Cold Coast", "Fen", "Guest"], "ListenBrainz first by closeness, then MusicBrainz relationships");
    assert_eq!(n.artists[0].because, ["Harbor Sound"]);
    assert!(!names.contains(&"Harbor Sound"), "the seed itself is in the library");
    // Worked out once, then reused; dismissing takes effect at once.
    let asked = r.similar.calls.lock().len();
    r.prefs.0.lock().dismissed.insert(COLD.into());
    let n = r.d.new_to_you().await.unwrap();
    assert_eq!(r.similar.calls.lock().len(), asked, "cached");
    assert!(!n.artists.iter().any(|a| a.mbid == COLD), "Not interested hides them");
    // An artist that arrives in the library leaves the list.
    r.db.write(|c| {
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('ar1', 'Northlight', 'Northlight')", [])?;
        Ok(())
    })
    .unwrap();
    assert!(!r.d.new_to_you().await.unwrap().artists.iter().any(|a| a.name == "Northlight"));
    r.prefs.0.lock().new_to_you = false;
    assert!(!r.d.new_to_you().await.unwrap().shown);
}

#[tokio::test]
async fn listenbrainz_similar_artists_are_parsed_cached_and_kept_when_down() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("lb.db")).unwrap();
    let http = Arc::new(FakeHttp::new());
    let body = json!([
        {"artist_mbid": COLD, "name": "Cold Coast", "score": 50, "reference_mbid": ARTIST},
        {"artist_mbid": FEN, "name": "Fen", "score": 90, "reference_mbid": ARTIST},
        {"artist_mbid": ARTIST, "name": "Northlight", "score": 999},
        {"artist_mbid": "bad", "name": "Bad"}
    ]);
    http.route("similar-artists/json", 200, &body.to_string());
    let now = Arc::new(Mutex::new(NOW));
    let n = now.clone();
    let lb = LbCached { http: http.clone(), limiter: Arc::new(Limiter::per_second(100.0)), db, clock: Arc::new(move || *n.lock()) };
    let got = lb.similar(ARTIST).await.unwrap();
    let names: Vec<&str> = got.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Fen", "Cold Coast"], "closest first, the artist itself and bad ids left out");
    assert!(http.urls()[0].contains(&format!("artist_mbids={ARTIST}")));
    lb.similar(ARTIST).await.unwrap();
    assert_eq!(http.calls("similar-artists"), 1, "cached");
    *now.lock() += lb::CACHE_TTL + 1;
    http.fail("similar-artists");
    assert_eq!(lb.similar(ARTIST).await.unwrap().len(), 2, "down: the old answer");
    assert!(lb.similar("nope").await.unwrap().is_empty());
}

#[test]
fn dismissed_lists_are_tidy() {
    let list = tidy_dismissed(&[
        Dismissed { mbid: COLD.to_uppercase(), name: " Cold Coast ".into() },
        Dismissed { mbid: COLD.into(), name: "again".into() },
        Dismissed { mbid: "x".into(), name: "X".into() },
    ]);
    assert_eq!(list, [Dismissed { mbid: COLD.into(), name: "Cold Coast".into() }]);
}

#[test]
fn add_to_missing_remembers_albums_until_they_arrive() {
    use crate::import::store::{WantedAlbum, add_wanted, missing_albums, remove_wanted, wanted_albums};
    let r = rig();
    let w = |rgid: &str, title: &str, artist: &str| WantedAlbum {
        release_group_id: rgid.into(),
        title: title.into(),
        artist: artist.into(),
        artist_mbid: None,
        added_at: 0,
    };
    r.db.write(|c| {
        add_wanted(c, w(RG_TIDES, "Tides", "Northlight"), NOW)?;
        add_wanted(c, w(RG_TIDES, "Tides", "Northlight"), NOW)?;
        add_wanted(c, w("40000000-0000-0000-0000-000000000001", "Harbor Lights", "Harbor Sound"), NOW)?;
        assert!(add_wanted(c, w("bad", "X", "Y"), NOW).is_err());
        Ok(())
    })
    .unwrap();
    let list = r.db.read(missing_albums).unwrap();
    assert_eq!(list.len(), 2, "once each, newest first");
    assert_eq!(list[0].album.title, "Harbor Lights");
    assert_eq!(list[0].library_album_id.as_deref(), Some("al-h"), "it's in the library now");
    assert_eq!(list[1].library_album_id, None);
    assert!(list[1].links.bandcamp.contains("item_type=a"));
    r.db.write(|c| remove_wanted(c, RG_TIDES)).unwrap();
    assert_eq!(r.db.read(wanted_albums).unwrap().len(), 1);
}

#[test]
fn queries_are_escaped_words() {
    assert_eq!(mb::escape("AC/DC: Live!"), r"AC\/DC\: Live\!");
    let u = url::Url::parse(&mb::artist_search_url("a+b")).unwrap();
    let q = u.query_pairs().find(|(k, _)| k == "query").unwrap().1.into_owned();
    assert_eq!(q, r"a\+b");
}
