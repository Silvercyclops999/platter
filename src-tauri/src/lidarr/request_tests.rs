//! Requests against a faked Lidarr (with state) and a faked MusicBrainz.
//! Nothing here reaches a real Lidarr, Plex or MusicBrainz.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::check::{PlexFile, PlexFiles};
use super::mbpick::MbGet;
use super::mbpick::tests::{ARTIST, FakeMb, RG_BOOT, RG_EP, RG_SECOND, RG_SKETCH};
use super::request::{QUEUED_NOTE, RequestInput};
use super::status::RequestState;
use super::{ArtistMbid, Lidarr, LidarrDeps, PlexMusic};
use crate::artistinfo::http::HttpResponse;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::plex::keychain::MemoryStore;
use crate::web::{Body, Method, Web, WebRequest};

/// A MusicBrainz that knows nothing.
pub struct NoMb;

#[async_trait]
impl MbGet for NoMb {
    async fn get(&self, _url: &str) -> AppResult<Option<Value>> {
        Ok(None)
    }
}

pub struct NoMbid;

#[async_trait]
impl ArtistMbid for NoMbid {
    async fn artist_mbid(&self, _artist_id: &str, _name: &str) -> Option<String> {
        None
    }
}

struct Mbid;

#[async_trait]
impl ArtistMbid for Mbid {
    async fn artist_mbid(&self, artist_id: &str, _name: &str) -> Option<String> {
        (artist_id == "ar1").then(|| ARTIST.to_string())
    }
}

struct Plex;

#[async_trait]
impl PlexMusic for Plex {
    async fn music_files(&self) -> AppResult<PlexFiles> {
        Ok(PlexFiles {
            locations: vec!["/data/media/music".into()],
            files: vec![PlexFile { path: "/data/media/music/Northlight/First Light/01 Dawn.flac".into(), size: Some(31_000_111) }],
        })
    }
}

const ROOT: &str = "/downloads/lidarr";

#[derive(Default)]
struct State {
    artists: Vec<Value>,
    albums: Vec<Value>,
    next_id: i64,
    /// Lidarr's metadata service is down.
    lookup_down: bool,
    /// Lidarr's metadata has no such album.
    lookup_empty: bool,
    /// A Lidarr that monitors more than it was told to on add.
    monitor_extra_on_add: bool,
    new_items_all: bool,
    queue: Vec<Value>,
    history: Vec<Value>,
    /// The root folder is the Plex music folder.
    red: bool,
    down: bool,
    /// Album list reads that come back empty after an add.
    albums_delay: usize,
}

/// A faked Lidarr that keeps artists and albums. Logs method, path and body.
struct FakeLidarr {
    st: Mutex<State>,
    log: Mutex<Vec<(Method, String, String)>>,
}

impl FakeLidarr {
    fn new() -> Arc<Self> {
        Arc::new(Self { st: Mutex::new(State { next_id: 100, ..Default::default() }), log: Mutex::new(vec![]) })
    }

    fn writes(&self) -> Vec<(Method, String, String)> {
        self.log.lock().iter().filter(|(m, _, _)| *m != Method::Get).cloned().collect()
    }

    fn catalog(rgid: &str) -> Option<&'static str> {
        [(RG_SECOND, "Second Wind"), (RG_EP, "Small Hours"), ("10000000-0000-0000-0000-000000000001", "First Light")]
            .into_iter()
            .find(|(id, _)| *id == rgid)
            .map(|(_, t)| t)
    }

    fn handle(&self, method: Method, u: &url::Url, body: &str) -> (u16, Value) {
        let mut st = self.st.lock();
        let path = u.path().trim_start_matches("/api/v1");
        let q = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.to_string());
        match (method, path) {
            (Method::Get, "/system/status") => (200, json!({"appName": "Lidarr", "version": "2.8.2.4493"})),
            (Method::Get, "/rootfolder") => {
                let p = if st.red { "/data/media/music" } else { ROOT };
                (200, json!([{"id": 1, "path": p, "accessible": true, "unmappedFolders": [{"name": "Elsewhere", "path": format!("{p}/Elsewhere")}],
                              "defaultQualityProfileId": 2, "defaultMetadataProfileId": 3}]))
            }
            (Method::Get, "/artist") => (200, Value::Array(st.artists.clone())),
            (Method::Get, "/config/naming") => (200, json!({"renameTracks": false})),
            (Method::Get, "/config/metadataprovider") => (200, json!({"writeAudioTags": "newFiles"})),
            (Method::Get, "/config/mediamanagement") => (200, json!({"deleteEmptyFolders": false, "recycleBin": "/recycle"})),
            (Method::Get, "/qualityprofile") => (200, json!([{"id": 2, "name": "Lossless", "upgradeAllowed": false}])),
            (Method::Get, "/importlist") => (200, json!([])),
            (Method::Get, "/metadataprofile") => (200, json!([{"id": 3, "name": "Standard"}])),
            (Method::Get, "/filesystem") => (200, json!({"directories": [], "files": []})),
            (Method::Get, "/album/lookup") => {
                if st.lookup_down {
                    return (503, json!({"message": "Search failed. Unable to communicate with metadata service."}));
                }
                let rgid = q("term").unwrap_or_default().trim_start_matches("lidarr:").to_string();
                match Self::catalog(&rgid).filter(|_| !st.lookup_empty) {
                    Some(t) => (200, json!([{"id": 0, "title": t, "foreignAlbumId": rgid,
                        "artist": {"artistName": "Northlight", "foreignArtistId": ARTIST, "folder": "Northlight"}}])),
                    None => (200, json!([])),
                }
            }
            (Method::Get, "/album") => {
                if let Some(f) = q("foreignAlbumId") {
                    return (200, Value::Array(st.albums.iter().filter(|a| a["foreignAlbumId"] == f.as_str()).cloned().collect()));
                }
                let aid: i64 = q("artistId").and_then(|a| a.parse().ok()).unwrap_or(-1);
                if st.albums_delay > 0 {
                    st.albums_delay -= 1;
                    return (200, json!([]));
                }
                (200, Value::Array(st.albums.iter().filter(|a| a["artistId"] == aid).cloned().collect()))
            }
            (Method::Get, p) if p.starts_with("/artist/") => {
                let id: i64 = p.trim_start_matches("/artist/").parse().unwrap_or(-1);
                match st.artists.iter().find(|a| a["id"] == id) {
                    Some(a) => (200, a.clone()),
                    None => (404, json!({})),
                }
            }
            (Method::Get, "/queue/details") => (200, Value::Array(st.queue.clone())),
            (Method::Get, "/history") => (200, json!({"records": st.history.clone()})),
            (Method::Post, "/artist") => {
                let b: Value = serde_json::from_str(body).unwrap();
                st.next_id += 1;
                let id = st.next_id;
                let new_items = if st.new_items_all { "all" } else { b["monitorNewItems"].as_str().unwrap_or("all") };
                let artist = json!({"id": id, "artistName": b["artistName"], "foreignArtistId": b["foreignArtistId"],
                    "path": format!("{}/{}", b["rootFolderPath"].as_str().unwrap(), b["artistName"].as_str().unwrap()),
                    "monitored": b["monitored"], "monitorNewItems": new_items});
                st.artists.push(artist.clone());
                let extra = st.monitor_extra_on_add;
                for (rg, title) in [(RG_SECOND, "Second Wind"), (RG_EP, "Small Hours"), ("10000000-0000-0000-0000-000000000001", "First Light")] {
                    st.next_id += 1;
                    let nid = st.next_id;
                    st.albums.push(json!({"id": nid, "title": title, "foreignAlbumId": rg, "artistId": id,
                        "monitored": extra && rg == RG_EP, "statistics": {"trackFileCount": 0, "trackCount": 10, "percentOfTracks": 0}}));
                }
                (201, artist)
            }
            (Method::Put, "/album/monitor") => {
                let b: Value = serde_json::from_str(body).unwrap();
                let on = b["monitored"].as_bool().unwrap();
                for id in b["albumIds"].as_array().unwrap() {
                    if let Some(a) = st.albums.iter_mut().find(|a| a["id"] == *id) {
                        a["monitored"] = json!(on);
                    }
                }
                (202, json!([]))
            }
            (Method::Post, "/command") => (201, json!({"id": 1, "name": "AlbumSearch"})),
            _ => (405, json!({})),
        }
    }
}

#[async_trait]
impl Web for FakeLidarr {
    async fn send(&self, req: WebRequest) -> AppResult<HttpResponse> {
        let body = match &req.body {
            Body::Json(j) => j.clone(),
            _ => String::new(),
        };
        let u = url::Url::parse(&req.url).unwrap();
        let path = format!("{}{}", u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default());
        self.log.lock().push((req.method, path, body.clone()));
        if self.st.lock().down {
            return Err(AppError::Network("connection refused".into()));
        }
        assert!(req.headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("x-api-key") && v == "abc123"));
        assert!(!req.url.contains("abc123"));
        let (status, v) = self.handle(req.method, &u, &body);
        Ok(HttpResponse { status, body: Bytes::from(v.to_string()), ..Default::default() })
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    db: DbHandle,
    web: Arc<FakeLidarr>,
    mb: Arc<FakeMb>,
    lidarr: Lidarr,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("r.db")).unwrap();
    let s = crate::settings::Settings { lidarr_url: "http://10.20.30.40:8686".into(), ..Default::default() };
    crate::settings::save(&db, &s).unwrap();
    db.write(|c| {
        c.execute_batch(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('a1', 'First Light', 'First Light', 'ar1', 'Northlight', 'Album', '[]', 1),
               ('a2', 'Quiet Sketches (Build 2)', 'Quiet Sketches', 'ar1', 'Northlight', NULL, '[]', 0);
             INSERT INTO album_tags (album_id, version) VALUES ('a2', 'build');",
        )?;
        Ok(())
    })
    .unwrap();
    let web = FakeLidarr::new();
    let mb = FakeMb::northlight();
    let lidarr = Lidarr::new(LidarrDeps {
        web: web.clone(),
        keys: Arc::new(MemoryStore::new()),
        db: db.clone(),
        plex: Arc::new(Plex),
        clock: Arc::new(|| 1_800_000_000),
        mb: mb.clone(),
        artist_mbid: Arc::new(Mbid),
        album_wait: std::time::Duration::ZERO,
    });
    lidarr.set_key(Some("abc123")).unwrap();
    Rig { _dir: dir, db, web, mb, lidarr }
}

fn input(rgid: &str, title: &str, typed: Option<&str>) -> RequestInput {
    RequestInput {
        release_group_id: rgid.into(),
        title: title.into(),
        artist: "Northlight".into(),
        artist_id: Some("ar1".into()),
        typed: typed.map(str::to_string),
    }
}

/// Every write that reached Lidarr is one of the three allowed, by method and path.
fn assert_only_allowed_writes(web: &FakeLidarr) {
    for (m, p, _) in web.log.lock().iter() {
        let ok = match m {
            Method::Get => p.starts_with("/api/v1/"),
            Method::Post => p == "/api/v1/artist" || p == "/api/v1/command",
            Method::Put => p == "/api/v1/album/monitor",
        };
        assert!(ok, "Lidarr was sent {m:?} {p}");
    }
}

#[tokio::test]
async fn a_request_adds_the_artist_limited_then_monitors_and_searches_only_the_album() {
    let r = rig();
    // The very first request needs the album's name typed.
    let e = r.lidarr.request(input(RG_SECOND, "Second Wind", None)).await.unwrap_err();
    assert!(e.to_string().contains("Type the album's name"), "{e}");
    assert!(r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second"))).await.is_err());
    assert!(r.web.writes().is_empty());

    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some(" second  wind "))).await.unwrap();
    assert_eq!(rec.state, RequestState::Searching);
    assert_eq!(rec.destination.as_deref(), Some("/downloads/lidarr/Northlight"));
    assert!(rec.added_artist);
    let writes = r.web.writes();
    let shape: Vec<(Method, &str)> = writes.iter().map(|(m, p, _)| (*m, p.as_str())).collect();
    assert_eq!(shape, vec![(Method::Post, "/api/v1/artist"), (Method::Put, "/api/v1/album/monitor"), (Method::Post, "/api/v1/command")]);
    let add: Value = serde_json::from_str(&writes[0].2).unwrap();
    assert_eq!(add["monitored"], true);
    assert_eq!(add["monitorNewItems"], "none");
    assert_eq!(add["addOptions"], json!({"monitor": "none", "searchForMissingAlbums": false}));
    assert_eq!(add["rootFolderPath"], ROOT);
    assert_eq!((add["qualityProfileId"].as_i64(), add["metadataProfileId"].as_i64()), (Some(2), Some(3)));
    let album_id = rec.lidarr_album_id.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&writes[1].2).unwrap(), json!({"albumIds": [album_id], "monitored": true}));
    assert_eq!(serde_json::from_str::<Value>(&writes[2].2).unwrap(), json!({"name": "AlbumSearch", "albumIds": [album_id]}));
    // Only that album is monitored in Lidarr.
    let on: Vec<String> =
        r.web.st.lock().albums.iter().filter(|a| a["monitored"] == true).map(|a| a["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(on, vec!["Second Wind"]);
    assert_only_allowed_writes(&r.web);

    // The full check ran before the first write (status, root folders...).
    let log = r.web.log.lock().clone();
    let first_write = log.iter().position(|(m, _, _)| *m != Method::Get).unwrap();
    assert!(log[..first_write].iter().any(|(_, p, _)| p == "/api/v1/rootfolder"));
    assert!(log[..first_write].iter().any(|(_, p, _)| p.starts_with("/api/v1/album/lookup?term=lidarr")));

    // Typed once: the next request goes straight through, without adding the artist again.
    let rec = r.lidarr.request(input(RG_EP, "Small Hours", None)).await.unwrap();
    assert_eq!(rec.state, RequestState::Searching);
    assert!(!rec.added_artist);
    assert_eq!(r.web.writes().iter().filter(|(_, p, _)| p == "/api/v1/artist").count(), 1);
    // Asking for the same album again returns the request, sends nothing.
    let before = r.web.writes().len();
    r.lidarr.request(input(RG_EP, "Small Hours", None)).await.unwrap();
    assert_eq!(r.web.writes().len(), before);
    assert_only_allowed_writes(&r.web);
}

#[tokio::test]
async fn read_back_turns_off_anything_else_lidarr_monitored_and_says_so() {
    let r = rig();
    {
        let mut st = r.web.st.lock();
        st.monitor_extra_on_add = true;
        st.new_items_all = true;
    }
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert_eq!(rec.state, RequestState::Searching);
    assert!(rec.notes.iter().any(|n| n.contains("Small Hours") && n.contains("turned it off")), "{:?}", rec.notes);
    assert!(rec.notes.iter().any(|n| n.contains("new albums")), "{:?}", rec.notes);
    let writes = r.web.writes();
    let off = writes.iter().position(|(_, p, b)| p == "/api/v1/album/monitor" && b.contains("false")).unwrap();
    let search = writes.iter().position(|(_, p, _)| p == "/api/v1/command").unwrap();
    assert!(off < search, "turned off before searching");
    assert_eq!(r.web.st.lock().albums.iter().filter(|a| a["monitored"] == true).count(), 1);
    assert_only_allowed_writes(&r.web);
}

#[tokio::test]
async fn a_red_check_stops_every_request() {
    let r = rig();
    r.web.st.lock().red = true;
    let e = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap_err();
    assert!(e.to_string().contains("red items"), "{e}");
    assert!(r.web.writes().is_empty());
    assert!(r.lidarr.preview(&input(RG_SECOND, "Second Wind", None)).await.is_err());
    assert!(r.lidarr.requests().is_empty(), "nothing saved");
}

#[tokio::test]
async fn lidarr_metadata_down_or_album_unknown() {
    let r = rig();
    r.web.st.lock().lookup_down = true;
    let e = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap_err();
    assert!(e.to_string().contains("Lidarr can't look up albums right now"), "{e}");
    r.web.st.lock().lookup_down = false;
    r.web.st.lock().lookup_empty = true;
    let e = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap_err();
    assert!(e.to_string().contains("Lidarr doesn't know this album"), "{e}");
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn an_album_already_in_lidarr_shows_its_status_and_is_not_added_again() {
    let r = rig();
    {
        let mut st = r.web.st.lock();
        st.artists.push(json!({"id": 5, "artistName": "Northlight", "foreignArtistId": ARTIST, "path": "/downloads/lidarr/Northlight", "monitored": true}));
        st.albums.push(json!({"id": 9, "title": "Second Wind", "foreignAlbumId": RG_SECOND, "artistId": 5, "monitored": true,
            "statistics": {"trackFileCount": 0, "trackCount": 10, "percentOfTracks": 0}}));
        st.queue.push(json!({"albumId": 9, "status": "downloading", "trackedDownloadState": "downloading"}));
    }
    let p = r.lidarr.preview(&input(RG_SECOND, "Second Wind", None)).await.unwrap();
    assert_eq!(p.existing, Some(RequestState::Downloading));
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert!(rec.already_in_lidarr);
    assert_eq!(rec.state, RequestState::Downloading);
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn unofficial_or_clashing_albums_are_refused_before_lidarr_is_asked() {
    let r = rig();
    let e = r.lidarr.request(input(RG_SKETCH, "Quiet Sketches", Some("Quiet Sketches"))).await.unwrap_err();
    assert!(e.to_string().contains("unreleased album already in your library"), "{e}");
    let e = r.lidarr.request(input(RG_BOOT, "Basement Tapes", Some("Basement Tapes"))).await.unwrap_err();
    assert!(e.to_string().contains("bootleg"), "{e}");
    assert!(r.web.log.lock().is_empty(), "Lidarr wasn't contacted");
    assert!(!r.lidarr.typed_confirmed(), "a refused request doesn't count as the first");
}

#[tokio::test]
async fn unreachable_lidarr_queues_and_the_retry_sends_after_a_full_check() {
    let r = rig();
    r.lidarr.check().await.unwrap();
    r.web.st.lock().down = true;
    let p = r.lidarr.preview(&input(RG_SECOND, "Second Wind", None)).await.unwrap();
    assert!(!p.reachable && p.destination_is_guess);
    assert_eq!(p.destination.as_deref(), Some("/downloads/lidarr/Northlight"), "from the last check");
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert_eq!(rec.state, RequestState::Queued);
    assert_eq!(rec.message.as_deref(), Some(QUEUED_NOTE));
    assert!(r.web.writes().is_empty());
    // Still down: stays waiting.
    assert_eq!(r.lidarr.retry_queued().await.unwrap(), 0);
    assert_eq!(r.lidarr.requests()[0].state, RequestState::Queued);
    // Back, but now red: still waiting, nothing sent.
    {
        let mut st = r.web.st.lock();
        st.down = false;
        st.red = true;
    }
    r.lidarr.retry_queued().await.unwrap();
    assert_eq!(r.lidarr.requests()[0].state, RequestState::Queued);
    assert!(r.lidarr.requests()[0].message.as_deref().unwrap().contains("red items"));
    assert!(r.web.writes().is_empty());
    // Green: sent.
    r.web.log.lock().clear();
    r.web.st.lock().red = false;
    assert_eq!(r.lidarr.retry_queued().await.unwrap(), 1);
    assert_eq!(r.lidarr.requests()[0].state, RequestState::Searching);
    let log = r.web.log.lock().clone();
    let first_write = log.iter().position(|(m, _, _)| *m != Method::Get).unwrap();
    assert!(log[..first_write].iter().any(|(_, p, _)| p == "/api/v1/system/status"));
    assert!(log[..first_write].iter().any(|(_, p, _)| p == "/api/v1/rootfolder"));
    assert_only_allowed_writes(&r.web);
}

#[tokio::test]
async fn a_new_artists_album_that_appears_late_is_finished_by_the_retry() {
    let r = rig();
    r.web.st.lock().albums_delay = 1_000;
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert_eq!(rec.state, RequestState::Queued);
    assert!(rec.message.unwrap().contains("still reading"));
    r.web.st.lock().albums_delay = 0;
    r.lidarr.retry_queued().await.unwrap();
    let rec = &r.lidarr.requests()[0];
    assert_eq!(rec.state, RequestState::Searching);
    assert_eq!(r.web.writes().iter().filter(|(_, p, _)| p == "/api/v1/artist").count(), 1, "the artist is added once");
    assert_only_allowed_writes(&r.web);
}

#[tokio::test]
async fn preview_shows_where_files_land() {
    let r = rig();
    let p = r.lidarr.preview(&input(RG_SECOND, "Second Wind", None)).await.unwrap();
    assert!(p.reachable && p.needs_typed && p.adds_artist && !p.destination_is_guess);
    assert_eq!(p.destination.as_deref(), Some("/downloads/lidarr/Northlight"));
    assert_eq!(p.existing, None);
    assert!(r.web.writes().is_empty(), "a preview only reads");
}

#[tokio::test]
async fn status_follows_lidarrs_queue_and_history() {
    let r = rig();
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    let id = rec.lidarr_album_id.unwrap();
    r.web.st.lock().queue.push(json!({"albumId": id, "status": "downloading", "trackedDownloadState": "downloading"}));
    assert_eq!(r.lidarr.refresh_statuses().await.unwrap()[0].state, RequestState::Downloading);
    {
        let mut st = r.web.st.lock();
        st.queue.clear();
        st.history.push(json!({"albumId": id, "eventType": "downloadFailed"}));
    }
    let l = r.lidarr.refresh_statuses().await.unwrap();
    assert_eq!(l[0].state, RequestState::Failed);
    assert!(l[0].message.is_some());
    assert_only_allowed_writes(&r.web);
}

#[tokio::test]
async fn only_waiting_or_finished_requests_can_be_cancelled() {
    let r = rig();
    let sent = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert!(r.lidarr.cancel(&sent.id).is_err());
    r.web.st.lock().down = true;
    let waiting = r.lidarr.request(input(RG_EP, "Small Hours", None)).await.unwrap();
    assert_eq!(waiting.state, RequestState::Queued);
    let left = r.lidarr.cancel(&waiting.id).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, sent.id);
}

#[tokio::test]
async fn missing_releases_need_the_setting_and_a_green_check() {
    let r = rig();
    // No check yet: hidden.
    assert!(!r.lidarr.missing_releases("ar1", "Northlight").await.unwrap().shown);
    r.lidarr.check().await.unwrap();
    let m = r.lidarr.missing_releases("ar1", "Northlight").await.unwrap();
    assert!(m.shown);
    let titles: Vec<&str> = m.items.iter().map(|i| i.title.as_str()).collect();
    // Have First Light; Quiet Sketches clashes with an unreleased build.
    assert_eq!(titles, vec!["Tides", "Small Hours", "Second Wind"]);
    assert_eq!(m.items[2].year, Some(2021));
    assert_eq!(m.items[1].kind, "ep");
    r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    let m = r.lidarr.missing_releases("ar1", "Northlight").await.unwrap();
    assert_eq!(m.items[2].request, Some(RequestState::Searching));
    // Setting off: hidden.
    let s = crate::settings::Settings { show_missing_releases: false, ..crate::settings::load(&r.db) };
    crate::settings::save(&r.db, &s).unwrap();
    assert!(!r.lidarr.missing_releases("ar1", "Northlight").await.unwrap().shown);
    // A red check later hides them too.
    let s = crate::settings::Settings { show_missing_releases: true, ..crate::settings::load(&r.db) };
    crate::settings::save(&r.db, &s).unwrap();
    r.web.st.lock().red = true;
    r.lidarr.check().await.unwrap();
    assert!(!r.lidarr.missing_releases("ar1", "Northlight").await.unwrap().shown);
    assert!(r.mb.count("/release?artist=") >= 1);
}

#[tokio::test]
async fn picking_for_a_song_uses_musicbrainz_only() {
    let r = rig();
    let p = r.lidarr.pick("Northlight", Some("ar1"), "Breathe In", true, None).await.unwrap();
    assert_eq!(p.group.unwrap().id, RG_SECOND);
    assert!(r.web.log.lock().is_empty());
}
