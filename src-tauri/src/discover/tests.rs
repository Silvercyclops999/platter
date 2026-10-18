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
    lookup: Arc<FakeLookup>,
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
    let prefs = Arc::new(FakePrefs(Mutex::new(Prefs { search: true, fans: true, new_to_you: true, dismissed: HashSet::from([NOPE.to_string()]), lidarr_source: true, get_kinds: every_kind() })));
    let requests = Arc::new(Requests::default());
    let offline = Arc::new(AtomicBool::new(false));
    let lookup = Arc::new(FakeLookup::default());
    let d = Discover::new(DiscoverDeps {
        db: db.clone(),
        mb: mb.clone(),
        similar: similar.clone(),
        mbids: Arc::new(Mbids),
        top,
        info: Arc::new(Info),
        requests: requests.clone(),
        prefs: prefs.clone(),
        lookup: lookup.clone(),
        offline: offline.clone(),
        clock: Arc::new(|| NOW),
    });
    Rig { _dir: dir, db, mb, similar, prefs, requests, offline, lookup, d }
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
        prefs: Arc::new(FakePrefs(Mutex::new(Prefs { search: true, fans: true, new_to_you: true, dismissed: HashSet::new(), lidarr_source: true, get_kinds: every_kind() }))),
        lookup: Arc::new(FakeLookup::default()),
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
            ("albums".to_string(), vec!["Quiet Sketches".to_string(), "Second Wind".into(), "Tides".into(), "First Light".into()]),
            ("singlesEps".to_string(), vec!["Second Wind".to_string(), "Small Hours".into()]),
            ("live".to_string(), vec!["Live at the Harbor".to_string()]),
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

fn every_kind() -> Vec<String> {
    super::GET_KINDS.iter().map(|s| s.to_string()).collect()
}

#[tokio::test]
async fn catalogue_sections_follow_the_get_albums_kinds() {
    let r = rig();
    r.prefs.0.lock().get_kinds = vec!["studio".into(), "ep".into()];
    let p = r.d.artist_page(ARTIST).await.unwrap();
    let kinds: Vec<&str> = p.sections.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(kinds, ["albums", "singlesEps"], "the live album is left out");
    assert!(!p.popular.is_empty(), "Popular stays");
}

#[tokio::test]
async fn a_featured_guest_gets_a_photo_and_their_releases_by_section() {
    let r = rig();
    with_lidarr(&r);
    let f = r.d.featured_artist("credit:northlight", "Northlight").await.unwrap();
    assert!(f.shown);
    assert_eq!(f.mbid.as_deref(), Some(ARTIST));
    assert_eq!(f.photo.as_deref(), Some("/ext/https://assets.fanart.tv/fanart/music/n.jpg"), "a real photo, not a blank icon");
    let by: Vec<(&str, Vec<&str>)> = f.sections.iter().map(|s| (s.kind.as_str(), s.albums.iter().map(|a| a.title.as_str()).collect())).collect();
    assert_eq!(
        by,
        vec![
            ("albums", vec!["Quiet Sketches", "Second Wind", "Tides", "First Light"]),
            ("singlesEps", vec!["Second Wind", "Small Hours"]),
            ("live", vec!["Live at the Harbor"]),
        ]
    );
    assert!(f.sections.iter().flat_map(|s| &s.albums).all(|a| a.artist == "Northlight" && a.artist_mbid.as_deref() == Some(ARTIST)));
    // Asked again: the photo comes from the cache, not from Lidarr again.
    let lookups = r.lookup.calls.lock().len();
    let again = r.d.featured_artist("credit:northlight", "Northlight").await.unwrap();
    assert_eq!(again, f);
    assert_eq!(r.lookup.calls.lock().len(), lookups, "the photo is cached");

    // A name MusicBrainz doesn't know: nothing to show, no catalogue asked.
    let before = r.mb.calls.lock().len();
    let none = r.d.featured_artist("credit:nobody", "Nobody At All").await.unwrap();
    assert!(!none.shown && none.sections.is_empty() && none.mbid.is_none());
    assert_eq!(r.mb.calls.lock().len(), before);

    // Offline: nothing at all.
    r.offline.store(true, Ordering::SeqCst);
    let off = r.d.featured_artist("credit:northlight", "Northlight").await.unwrap();
    assert_eq!(off, FeaturedArtist::default());
    assert_eq!(r.mb.calls.lock().len(), before, "nothing reached MusicBrainz");
}

#[tokio::test]
async fn a_featured_guests_photo_alone_skips_their_releases() {
    let r = rig();
    with_lidarr(&r);
    let p = r.d.featured_photo("credit:northlight", "Northlight").await.unwrap();
    assert_eq!(p.photo.as_deref(), Some("/ext/https://assets.fanart.tv/fanart/music/n.jpg"));
    assert_eq!(r.mb.count("/release?artist="), 0, "no discography browse for a photo");
    assert_eq!(r.d.featured_photo("credit:x", "Nobody At All").await.unwrap(), CataloguePhoto::default());
}

#[tokio::test]
async fn a_featured_guest_marks_what_the_library_has_and_whats_asked_for() {
    let r = rig();
    r.db.write(|c| {
        c.execute_batch(
            "INSERT INTO artists (id, title, sort_title) VALUES ('ar-n', 'Northlight', 'Northlight');
             INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('al-n', 'First Light', 'First Light', 'ar-n', 'Northlight', 'Album', '[]', 1);",
        )?;
        Ok(())
    })
    .unwrap();
    r.requests.0.lock().insert(RG_TIDES.into(), RequestState::Searching);
    let f = r.d.featured_artist("credit:northlight", "Northlight").await.unwrap();
    let albums = &f.sections[0].albums;
    let first = albums.iter().find(|a| a.title == "First Light").unwrap();
    assert_eq!(first.library_album_id.as_deref(), Some("al-n"), "owned shows In your library");
    let tides = albums.iter().find(|a| a.title == "Tides").unwrap();
    assert_eq!(tides.request, Some(RequestState::Searching));
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

// ---------------------------------------------------------------- Lidarr's search, pictures, Get albums

/// A faked Lidarr search: set up or not, answers by kind, counts calls,
/// can fail, can take a while.
#[derive(Default)]
struct FakeLookup {
    ready: AtomicBool,
    down: AtomicBool,
    artists: Mutex<Vec<LArtist>>,
    albums: Mutex<Vec<LAlbum>>,
    calls: Mutex<Vec<String>>,
    delay_ms: std::sync::atomic::AtomicU64,
}

impl FakeLookup {
    async fn answer(&self, what: String) -> AppResult<()> {
        self.calls.lock().push(what);
        let d = self.delay_ms.load(Ordering::SeqCst);
        if d > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(d)).await;
        }
        if self.down.load(Ordering::SeqCst) {
            return Err(AppError::Server(crate::lidarr::api::METADATA_DOWN.into()));
        }
        Ok(())
    }
}

#[async_trait]
impl CatalogueLookup for FakeLookup {
    fn ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
    async fn artists(&self, term: &str) -> AppResult<Vec<LArtist>> {
        self.answer(format!("artists:{term}")).await?;
        Ok(self.artists.lock().clone())
    }
    async fn albums(&self, term: &str) -> AppResult<Vec<LAlbum>> {
        self.answer(format!("albums:{term}")).await?;
        Ok(self.albums.lock().clone())
    }
    async fn artist(&self, mbid: &str) -> AppResult<Option<LArtist>> {
        self.answer(format!("artist:{mbid}")).await?;
        Ok(self.artists.lock().iter().find(|a| a.foreign_artist_id == mbid).cloned())
    }
}

fn limg(kind: &str, url: &str) -> crate::lidarr::api::LookupImage {
    crate::lidarr::api::LookupImage { cover_type: kind.into(), remote_url: Some(url.into()) }
}

fn lartist(mbid: &str, name: &str, photo: Option<&str>) -> LArtist {
    LArtist {
        foreign_artist_id: mbid.into(),
        artist_name: name.into(),
        artist_type: Some("Group".into()),
        images: photo.map(|p| vec![limg("poster", p)]).unwrap_or_default(),
        ..Default::default()
    }
}

fn lalbum(rgid: &str, title: &str, artist: LArtist, kind: &str, date: &str) -> LAlbum {
    LAlbum {
        foreign_album_id: rgid.into(),
        title: title.into(),
        album_type: Some(kind.into()),
        release_date: Some(date.into()),
        images: vec![limg("cover", &format!("https://coverartarchive.org/release/{rgid}/front.jpg"))],
        artist: Some(artist),
        ..Default::default()
    }
}

/// Lidarr set up with Northlight, Harbor Sound (in the library) and their albums.
fn with_lidarr(r: &Rig) {
    r.lookup.ready.store(true, Ordering::SeqCst);
    let north = lartist(ARTIST, "Northlight", Some("https://assets.fanart.tv/fanart/music/n.jpg"));
    let harbor = lartist(HARBOR, "Harbor Sound", None);
    *r.lookup.artists.lock() = vec![north.clone(), harbor.clone(), lartist(VARIOUS_ARTISTS, "Various Artists", None)];
    *r.lookup.albums.lock() = vec![
        lalbum(RG_TIDES, "Tides", north.clone(), "Album", "2017-02-01T00:00:00Z"),
        lalbum("40000000-0000-0000-0000-000000000001", "Harbor Lights", harbor, "Album", "2019-01-01T00:00:00Z"),
        LAlbum { secondary_types: vec!["Live".into()], ..lalbum(RG_LIVE_X, "Live at Leeds", north, "Album", "2020-01-01T00:00:00Z") },
    ];
}

const RG_LIVE_X: &str = "41000000-0000-0000-0000-000000000001";

#[tokio::test]
async fn search_uses_lidarrs_search_when_its_set_up() {
    let r = rig();
    with_lidarr(&r);
    let a = r.d.search_part("north", "artists").await.unwrap();
    assert_eq!(a.source, "lidarr");
    let names: Vec<&str> = a.artists.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["Northlight"], "the library artist folds in, Various Artists is left out");
    assert_eq!(a.artists[0].photo.as_deref(), Some("/ext/https://assets.fanart.tv/fanart/music/n.jpg"), "Lidarr's picture comes along");
    assert_eq!(a.in_library, ["ar-h"], "the library artist gets the In your library cue");
    let al = r.d.search_part("north", "albums").await.unwrap();
    let titles: Vec<(&str, &str)> = al.albums.iter().map(|x| (x.title.as_str(), x.kind.as_str())).collect();
    assert_eq!(titles, [("Tides", "album"), ("Live at Leeds", "album")]);
    assert_eq!(al.albums[1].secondary, ["live"]);
    assert_eq!(al.albums[0].year, Some(2017));
    assert_eq!(al.albums[0].thumb.as_deref(), Some(format!("/ext/https://coverartarchive.org/release/{RG_TIDES}/front.jpg").as_str()));
    assert_eq!(al.in_library, ["al-h"], "the library's album folds in");
    assert!(r.mb.calls.lock().is_empty(), "artists and albums didn't touch MusicBrainz");
    // Songs always come from MusicBrainz.
    let s = r.d.search_part("north", "songs").await.unwrap();
    assert_eq!(s.source, "musicBrainz");
    assert_eq!(r.mb.count("/recording?query="), 1);
    assert!(r.d.search_part("north", "playlists").await.is_err());
}

#[tokio::test]
async fn search_falls_back_to_musicbrainz_or_follows_the_setting() {
    let r = rig();
    with_lidarr(&r);
    r.lookup.down.store(true, Ordering::SeqCst);
    let a = r.d.search_part("north", "artists").await.unwrap();
    assert_eq!(a.source, "musicBrainz", "Lidarr's metadata service is down: MusicBrainz answers");
    assert_eq!(a.artists[0].name, "Northlight");
    r.lookup.down.store(false, Ordering::SeqCst);
    r.prefs.0.lock().lidarr_source = false;
    let al = r.d.search_part("tides", "albums").await.unwrap();
    assert_eq!(al.source, "musicBrainz", "the setting says MusicBrainz");
    assert!(!r.lookup.calls.lock().iter().any(|c| c.starts_with("albums:")));
    r.prefs.0.lock().lidarr_source = true;
    r.lookup.ready.store(false, Ordering::SeqCst);
    assert_eq!(r.d.search_part("cold", "artists").await.unwrap().source, "musicBrainz", "Lidarr isn't set up");
}

#[tokio::test]
async fn search_parts_are_cached_per_query_with_live_request_states() {
    let r = rig();
    with_lidarr(&r);
    let first = r.d.search_part("North", "albums").await.unwrap();
    assert!(first.albums[0].request.is_none());
    r.requests.0.lock().insert(RG_TIDES.into(), RequestState::Downloading);
    let again = r.d.search_part(" north ", "albums").await.unwrap();
    assert_eq!(r.lookup.calls.lock().len(), 1, "the same query again comes from the cache");
    assert_eq!(again.albums[0].request, Some(RequestState::Downloading), "the request state is current");
    r.d.search_part("northern", "albums").await.unwrap();
    assert_eq!(r.lookup.calls.lock().len(), 2, "another query asks again");
    // Off or offline: nothing.
    r.prefs.0.lock().search = false;
    assert!(!r.d.search_part("north", "artists").await.unwrap().shown);
}

#[tokio::test(start_paused = true)]
async fn first_catalogue_results_arrive_sooner_through_lidarr() {
    // Before: MusicBrainz only, all three kinds in one answer, paced one a
    // second. After: each kind on its own, artists and albums from Lidarr.
    // Every faked server takes 150 ms to answer.
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    struct Slow(Arc<FakeHttp>);
    #[async_trait]
    impl crate::artistinfo::http::InfoHttp for Slow {
        async fn get(&self, url: &str, headers: &[(&'static str, String)]) -> AppResult<crate::artistinfo::http::HttpResponse> {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            self.0.get(url, headers).await
        }
    }
    let http = Arc::new(FakeHttp::new());
    http.route("musicbrainz.org/ws/2/artist?", 200, &json!({"artists": [{"id": ARTIST, "name": "Northlight", "score": 100}]}).to_string());
    http.route("musicbrainz.org/ws/2/release-group?", 200, r#"{"release-groups": []}"#);
    http.route("musicbrainz.org/ws/2/recording?", 200, r#"{"recordings": []}"#);
    let lookup = Arc::new(FakeLookup::default());
    lookup.delay_ms.store(150, Ordering::SeqCst);
    *lookup.artists.lock() = vec![lartist(ARTIST, "Northlight", None)];
    let prefs = Arc::new(FakePrefs(Mutex::new(Prefs { search: true, fans: true, new_to_you: true, dismissed: HashSet::new(), lidarr_source: true, get_kinds: every_kind() })));
    let d = Discover::new(DiscoverDeps {
        db: db.clone(),
        mb: Arc::new(MbCached { http: Arc::new(Slow(http.clone())), limiter: Arc::new(Limiter::per_second(1.0)), db: db.clone(), clock: Arc::new(|| NOW), ttl: 3600 }),
        similar: Arc::new(FakeSimilar::default()),
        mbids: Arc::new(Mbids),
        top: Arc::new(Top::default()),
        info: Arc::new(Info),
        requests: Arc::new(Requests::default()),
        prefs: prefs.clone(),
        lookup: lookup.clone(),
        offline: Arc::new(AtomicBool::new(false)),
        clock: Arc::new(|| NOW),
    });
    let t0 = tokio::time::Instant::now();
    let before = d.search("northlight").await.unwrap();
    let old_ms = (tokio::time::Instant::now() - t0).as_millis();
    assert_eq!(before.artists.len(), 1);

    lookup.ready.store(true, Ordering::SeqCst);
    let t1 = tokio::time::Instant::now();
    let first = Arc::new(Mutex::new(None::<u128>));
    let parts = ["artists", "albums", "songs"].map(|p| {
        let (d, first) = (d.clone(), first.clone());
        async move {
            let part = d.search_part("northlight lidarr", p).await.unwrap();
            let ms = (tokio::time::Instant::now() - t1).as_millis();
            let mut f = first.lock();
            if f.is_none() && (!part.artists.is_empty() || !part.albums.is_empty() || !part.songs.is_empty()) {
                *f = Some(ms);
            }
            ms
        }
    });
    let all = futures::future::join_all(parts).await;
    let new_ms = first.lock().expect("a first result");
    println!("first catalogue result: before {old_ms} ms (MusicBrainz, one answer), after {new_ms} ms (Lidarr, streamed); all parts after {:?} ms", all);
    assert!(old_ms >= 2000, "three MusicBrainz searches at one a second: {old_ms} ms");
    assert!(new_ms < 500, "Lidarr's artists arrive in one round trip: {new_ms} ms");
}

#[tokio::test]
async fn artist_photos_come_from_lidarr_then_the_free_sources() {
    struct Pics;
    #[async_trait]
    impl CatalogueInfo for Pics {
        async fn bio(&self, _: &str, _: &str) -> Option<CatalogueBio> {
            None
        }
        async fn photo(&self, mbid: &str, _: &str) -> Option<CataloguePhoto> {
            (mbid == COLD).then(|| CataloguePhoto { photo: Some("/ext/https://commons.wikimedia.org/wiki/Special:FilePath/C.jpg?width=600".into()), credit: Some("Wikimedia Commons".into()) })
        }
    }
    let r = rig();
    with_lidarr(&r);
    let d = Discover::new(DiscoverDeps {
        db: r.db.clone(),
        mb: r.mb.clone(),
        similar: r.similar.clone(),
        mbids: Arc::new(Mbids),
        top: Arc::new(Top::default()),
        info: Arc::new(Pics),
        requests: r.requests.clone(),
        prefs: r.prefs.clone(),
        lookup: r.lookup.clone(),
        offline: r.offline.clone(),
        clock: Arc::new(|| NOW),
    });
    let p = d.artist_photo(ARTIST, "Northlight").await.unwrap();
    assert_eq!(p.photo.as_deref(), Some("/ext/https://assets.fanart.tv/fanart/music/n.jpg"), "Lidarr's lookup first");
    let p = d.artist_photo(COLD, "Cold Coast").await.unwrap();
    assert_eq!(p.credit.as_deref(), Some("Wikimedia Commons"), "Lidarr has none: Commons, with its credit");
    assert!(p.photo.unwrap().contains("commons.wikimedia.org"));
    let p = d.artist_photo(FEN, "Fen").await.unwrap();
    assert_eq!(p, CataloguePhoto::default(), "no source has one: the card shows a monogram");
    // Kept: asked once each, also offline.
    let asked = r.lookup.calls.lock().len();
    r.offline.store(true, Ordering::SeqCst);
    assert!(d.artist_photo(ARTIST, "Northlight").await.unwrap().photo.is_some());
    assert_eq!(r.lookup.calls.lock().len(), asked);
    assert_eq!(d.artist_photo("bad", "X").await.unwrap(), CataloguePhoto::default());
}

#[tokio::test]
async fn catalogue_artist_pages_have_a_photo_and_popular_albums() {
    let r = rig();
    with_lidarr(&r);
    let p = r.d.artist_page(ARTIST).await.unwrap();
    assert_eq!(p.artist.photo.as_deref(), Some("/ext/https://assets.fanart.tv/fanart/music/n.jpg"), "no fanart.tv key: Lidarr's picture");
    let pop: Vec<&str> = p.popular.iter().map(|a| a.title.as_str()).collect();
    assert!(!pop.is_empty() && pop.len() <= POPULAR_SHOWN);
    assert!(p.popular.iter().all(|a| matches!(a.kind.as_str(), "album" | "ep") && a.secondary.is_empty()), "studio albums and EPs only");
    assert!(!pop.contains(&"Live at the Harbor"));
}

#[tokio::test]
async fn get_albums_takes_the_kinds_asked_for_most_popular_first() {
    let r = rig();
    // Tides has three editions, so it goes first; the rest oldest first.
    let n = || ac(ARTIST, "Northlight");
    let rel = |rg: &str, title: &str, primary: &str, sec: &[&str], date: &str, i: usize| {
        json!({"id": format!("rel-{rg}-{i}"), "status": "Official", "date": date, "artist-credit": n(),
            "release-group": {"id": rg, "title": title, "primary-type": primary, "secondary-types": sec, "first-release-date": date}})
    };
    let mut releases = vec![
        rel(crate::lidarr::mbpick::tests::RG_FIRST, "First Light", "Album", &[], "2015-03-01", 0),
        rel(RG_SKETCH, "Quiet Sketches", "Album", &[], "2023-01-01", 0),
        rel(crate::lidarr::mbpick::tests::RG_EP, "Small Hours", "EP", &[], "2016-01-01", 0),
        rel(crate::lidarr::mbpick::tests::RG_LIVE, "Live at the Harbor", "Album", &["Live"], "2019-01-01", 0),
        rel(crate::lidarr::mbpick::tests::RG_SINGLE, "Second Wind", "Single", &[], "2020-11-01", 0),
    ];
    for i in 0..3 {
        releases.push(rel(RG_TIDES, "Tides", "Album", &[], "2017-02-01", i));
    }
    r.mb.route("/release?artist=", Some(json!({"release-count": releases.len(), "releases": releases})));
    let titles = |p: &GetAlbumsPlan| p.albums.iter().map(|a| a.title.clone()).collect::<Vec<_>>();
    let p = r.d.get_albums_plan(ARTIST, "Northlight", &[]).await.unwrap();
    assert_eq!(p.kinds, ["studio"], "studio albums unless asked otherwise");
    assert_eq!(titles(&p), ["Tides", "First Light", "Quiet Sketches"]);
    let p = r.d.get_albums_plan(ARTIST, "Northlight", &["studio".into(), "ep".into(), "live".into()]).await.unwrap();
    assert_eq!(titles(&p), ["Tides", "First Light", "Small Hours", "Live at the Harbor", "Quiet Sketches"], "never singles");
    // Asked for already, or in the library: left out.
    r.requests.0.lock().insert(RG_TIDES.into(), RequestState::Searching);
    r.db.write(|c| {
        c.execute_batch(
            "INSERT INTO artists (id, title, sort_title) VALUES ('ar1', 'Northlight', 'Northlight');
             INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('a1', 'First Light', 'First Light', 'ar1', 'Northlight', 'Album', '[]', 1);",
        )?;
        Ok(())
    })
    .unwrap();
    let p = r.d.get_albums_plan(ARTIST, "Northlight", &[]).await.unwrap();
    assert_eq!(titles(&p), ["Quiet Sketches"]);
    assert!(r.d.get_albums_plan("nope", "X", &[]).await.is_err());
}

#[test]
fn get_album_kinds() {
    let g = |primary: &str, sec: &[&str]| Group {
        id: String::new(),
        title: String::new(),
        primary: Some(primary.into()),
        secondary: sec.iter().map(|s| s.to_string()).collect(),
        first_release: String::new(),
        statuses: vec![],
        credit: vec![],
    };
    let k = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(kind_wanted(&g("album", &[]), &k(&["studio"])));
    assert!(!kind_wanted(&g("album", &["live"]), &k(&["studio"])));
    assert!(kind_wanted(&g("album", &["live"]), &k(&["live"])));
    assert!(kind_wanted(&g("album", &["compilation"]), &k(&["compilation"])));
    assert!(!kind_wanted(&g("album", &["soundtrack"]), &k(&["studio", "ep", "live", "compilation"])));
    assert!(!kind_wanted(&g("single", &[]), &k(&["studio", "ep", "live", "compilation"])));
    assert!(kind_wanted(&g("ep", &[]), &k(&["ep"])));
    assert_eq!(clean_kinds(&k(&["LIVE", "x"])), ["live"]);
    assert_eq!(clean_kinds(&[]), ["studio"]);
}
