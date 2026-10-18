//! Requests against a faked Lidarr (with state) and a faked MusicBrainz.
//! Nothing here reaches a real Lidarr, Plex or MusicBrainz.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::check::{PlexFile, PlexFiles};
use super::mbpick::MbGet;
use super::mbpick::tests::{ARTIST, FakeMb, OTHER, RG_BOOT, RG_EP, RG_SECOND, RG_SKETCH};
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

pub struct Mbid;

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
const STAGING: &str = "/data/staging/music";

#[derive(Default)]
pub(super) struct State {
    pub(super) artists: Vec<Value>,
    pub(super) albums: Vec<Value>,
    pub(super) next_id: i64,
    /// Lidarr's metadata service is down.
    pub(super) lookup_down: bool,
    /// Lidarr's metadata has no such album.
    pub(super) lookup_empty: bool,
    /// A Lidarr that monitors more than it was told to on add.
    pub(super) monitor_extra_on_add: bool,
    pub(super) new_items_all: bool,
    pub(super) queue: Vec<Value>,
    pub(super) history: Vec<Value>,
    /// The root folder is the Plex music folder.
    pub(super) red: bool,
    pub(super) down: bool,
    /// Album list reads that come back empty after an add.
    pub(super) albums_delay: usize,
    /// Command list reads that still show the new artist's refresh running.
    /// While it runs the new albums have no length; when it ends Lidarr
    /// applies "monitor none" to them, as a real Lidarr does.
    pub(super) refresh_polls: usize,
    /// Lidarr also has the staging root.
    pub(super) staging: bool,
    /// Artists on the import list exclusions (MusicBrainz ids).
    pub(super) exclusions: Vec<String>,
    /// Items in the whole download queue.
    pub(super) queue_total: u64,
    /// Free bytes on the root folder, when Lidarr says.
    pub(super) free_space: Option<i64>,
    /// Lidarr moves an album to its other edition after this many reads of it.
    pub(super) switch_after: Option<usize>,
    pub(super) album_reads: usize,
    /// Reads of this release group's album answer with a server error.
    pub(super) fail_album: Option<String>,
    /// Searches answer with a server error.
    pub(super) fail_search: bool,
}

/// A faked Lidarr that keeps artists and albums. Logs method, path and body.
pub(super) struct FakeLidarr {
    pub(super) st: Mutex<State>,
    pub(super) log: Mutex<Vec<(Method, String, String)>>,
}

impl FakeLidarr {
    fn new() -> Arc<Self> {
        Arc::new(Self { st: Mutex::new(State { next_id: 100, ..Default::default() }), log: Mutex::new(vec![]) })
    }

    pub(super) fn writes(&self) -> Vec<(Method, String, String)> {
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
                // Red for everyone: Lidarr has no root folder to download into.
                if st.red {
                    return (200, json!([]));
                }
                let p = ROOT;
                let mut roots = vec![json!({"id": 1, "path": p, "accessible": true, "unmappedFolders": [{"name": "Elsewhere", "path": format!("{p}/Elsewhere")}],
                              "defaultQualityProfileId": 2, "defaultMetadataProfileId": 3, "freeSpace": st.free_space})];
                if st.staging {
                    roots.push(json!({"id": 2, "path": STAGING, "accessible": true, "defaultQualityProfileId": 2, "defaultMetadataProfileId": 3}));
                }
                (200, Value::Array(roots))
            }
            (Method::Get, "/artist") => (200, Value::Array(st.artists.clone())),
            (Method::Get, "/config/naming") => (200, json!({"renameTracks": false})),
            (Method::Get, "/config/metadataprovider") => (200, json!({"writeAudioTags": "no"})),
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
                    if st.fail_album.as_deref() == Some(f.as_str()) {
                        return (500, json!({"message": "Internal error"}));
                    }
                    return (200, Value::Array(st.albums.iter().filter(|a| a["foreignAlbumId"] == f.as_str()).cloned().collect()));
                }
                let aid: i64 = q("artistId").and_then(|a| a.parse().ok()).unwrap_or(-1);
                if st.albums_delay > 0 {
                    st.albums_delay -= 1;
                    return (200, json!([]));
                }
                (200, Value::Array(st.albums.iter().filter(|a| a["artistId"] == aid).cloned().collect()))
            }
            (Method::Get, p) if p.starts_with("/album/") && p[7..].bytes().all(|b| b.is_ascii_digit()) => {
                let id: i64 = p[7..].parse().unwrap_or(-1);
                st.album_reads += 1;
                let switched = st.switch_after.is_some_and(|n| st.album_reads > n);
                match st.albums.iter().find(|a| a["id"] == id) {
                    Some(a) => {
                        let mut a = a.clone();
                        a["releases"] = json!([{"id": id * 10, "title": a["title"], "monitored": !switched, "trackCount": 10},
                                               {"id": id * 10 + 1, "title": a["title"], "monitored": switched, "trackCount": 14}]);
                        (200, a)
                    }
                    None => (404, json!({})),
                }
            }
            (Method::Get, p) if p.starts_with("/artist/") => {
                let id: i64 = p.trim_start_matches("/artist/").parse().unwrap_or(-1);
                match st.artists.iter().find(|a| a["id"] == id) {
                    Some(a) => (200, a.clone()),
                    None => (404, json!({})),
                }
            }
            (Method::Get, "/queue/details") => (200, Value::Array(st.queue.clone())),
            (Method::Get, "/queue") => (200, json!({"page": 1, "pageSize": 1, "totalRecords": st.queue_total, "records": []})),
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
                if st.refresh_polls > 0 {
                    let last = st.albums.len() - 1;
                    st.albums[last]["duration"] = json!(0);
                }
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
            (Method::Post, "/command") if st.fail_search => (500, json!({"message": "Internal error"})),
            (Method::Post, "/command") => (201, json!({"id": 1, "name": "AlbumSearch"})),
            (Method::Get, "/command") => {
                let Some(artist) = st.artists.last().and_then(|a| a["id"].as_i64()) else {
                    return (200, json!([]));
                };
                if st.refresh_polls == 0 {
                    return (200, json!([{"name": "RefreshArtist", "status": "completed", "body": {"artistId": artist}}]));
                }
                st.refresh_polls -= 1;
                if st.refresh_polls == 0 {
                    for a in st.albums.iter_mut().filter(|a| a["artistId"] == artist) {
                        a["duration"] = json!(3_153_000);
                        a["monitored"] = json!(false);
                    }
                }
                (200, json!([{"name": "RefreshArtist", "status": "started", "body": {"artistId": artist}}]))
            }
            (Method::Get, "/importlistexclusion") => {
                (200, Value::Array(st.exclusions.iter().map(|id| json!({"foreignId": id, "artistName": "Northlight"})).collect()))
            }
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

pub(super) struct Rig {
    pub(super) _dir: tempfile::TempDir,
    pub(super) db: DbHandle,
    pub(super) web: Arc<FakeLidarr>,
    pub(super) mb: Arc<FakeMb>,
    pub(super) lidarr: Lidarr,
    /// The clock every Lidarr of this rig reads (starts at 1,800,000,000).
    pub(super) now: Arc<Mutex<i64>>,
    keys: Arc<MemoryStore>,
}

impl Rig {
    /// Platter started again: a new Lidarr on the same database, keychain and server.
    pub(super) fn restart(&self) -> Lidarr {
        lidarr_on(&self.db, &self.web, &self.mb, &self.keys, &self.now)
    }
}

impl Rig {
    /// A Platter Bridge beside this rig's desktop: its own database (with
    /// the same library) and device, the same Lidarr, MusicBrainz and
    /// clock, in bridge mode.
    pub(super) fn bridge(&self) -> (tempfile::TempDir, DbHandle, Lidarr) {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("bridge.db")).unwrap();
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
        let keys = Arc::new(MemoryStore::new());
        let n = self.now.clone();
        let lidarr = Lidarr::new(LidarrDeps {
            web: self.web.clone(),
            keys: keys.clone(),
            db: db.clone(),
            plex: Arc::new(Plex),
            clock: Arc::new(move || *n.lock()),
            mb: self.mb.clone(),
            artist_mbid: Arc::new(Mbid),
            album_wait: std::time::Duration::ZERO,
            shared: super::shared::Shared::new(crate::sync::SyncStore::new(db.clone(), "bridge-device"), Arc::new(|| "Shed Bridge".to_string())),
            plex_server: Arc::new(super::plexside::fake::FakePlex::default()),
        });
        lidarr.set_bridge_mode(true);
        lidarr.set_key(Some("abc123")).unwrap();
        (dir, db, lidarr)
    }
}

fn lidarr_on(db: &DbHandle, web: &Arc<FakeLidarr>, mb: &Arc<FakeMb>, keys: &Arc<MemoryStore>, now: &Arc<Mutex<i64>>) -> Lidarr {
    let n = now.clone();
    Lidarr::new(LidarrDeps {
        web: web.clone(),
        keys: keys.clone(),
        db: db.clone(),
        plex: Arc::new(Plex),
        clock: Arc::new(move || *n.lock()),
        mb: mb.clone(),
        artist_mbid: Arc::new(Mbid),
        album_wait: std::time::Duration::ZERO,
        shared: super::shared::tests::on(db),
        plex_server: Arc::new(super::plexside::fake::FakePlex::default()),
    })
}

pub(super) fn rig() -> Rig {
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
    let keys = Arc::new(MemoryStore::new());
    let now = Arc::new(Mutex::new(1_800_000_000));
    let lidarr = lidarr_on(&db, &web, &mb, &keys, &now);
    lidarr.set_key(Some("abc123")).unwrap();
    Rig { _dir: dir, db, web, mb, lidarr, now, keys }
}

pub(super) fn input(rgid: &str, title: &str, typed: Option<&str>) -> RequestInput {
    RequestInput {
        release_group_id: rgid.into(),
        title: title.into(),
        artist: "Northlight".into(),
        artist_id: Some("ar1".into()),
        artist_mbid: None,
        typed: typed.map(str::to_string),
    }
}

/// Every write that reached Lidarr is one of the three allowed, by method and path.
pub(super) fn assert_only_allowed_writes(web: &FakeLidarr) {
    for (m, p, _) in web.log.lock().iter() {
        let ok = match m {
            Method::Get => p.starts_with("/api/v1/"),
            Method::Post => p == "/api/v1/artist" || p == "/api/v1/command",
            Method::Put => p == "/api/v1/album/monitor",
            Method::Delete => false,
        };
        assert!(ok, "Lidarr was sent {m:?} {p}");
    }
}

/// Measured on a real Lidarr: searching a new artist's album before its
/// refresh ended rejected every result ("duration is 0"), and the refresh
/// then turned the album's monitoring back off. Platter waits it out.
#[tokio::test]
async fn a_new_artists_album_is_monitored_and_searched_only_after_lidarr_finishes_reading_it() {
    let r = rig();
    r.web.st.lock().refresh_polls = 3;
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert_eq!(rec.state, RequestState::Searching, "{:?}", rec.message);
    let log = r.web.log.lock().clone();
    let monitor = log.iter().position(|(m, p, _)| *m == Method::Put && p == "/api/v1/album/monitor").unwrap();
    let search = log.iter().position(|(m, p, _)| *m == Method::Post && p == "/api/v1/command").unwrap();
    let last_refresh_read = log.iter().rposition(|(m, p, _)| *m == Method::Get && p == "/api/v1/command").unwrap();
    assert!(last_refresh_read < monitor && monitor < search, "monitor and search only after the refresh ended");
    let st = r.web.st.lock();
    let album = st.albums.iter().find(|a| a["foreignAlbumId"] == RG_SECOND).unwrap();
    assert_eq!(album["monitored"], true);
    assert!(album["duration"].as_i64().unwrap() > 0);
}

#[tokio::test]
async fn a_refresh_that_never_ends_queues_the_request_instead_of_searching() {
    let r = rig();
    r.web.st.lock().refresh_polls = 10_000;
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert_eq!(rec.state, RequestState::Queued);
    assert!(!r.web.writes().iter().any(|(m, p, _)| *m == Method::Post && p == "/api/v1/command"), "no search");
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
async fn a_catalogue_request_uses_the_artist_id_it_names_and_every_guard() {
    let r = rig();
    // From a page for music that isn't in the library: no library id, the
    // MusicBrainz id given, so no name search.
    let named = |rgid: &str, title: &str| RequestInput {
        release_group_id: rgid.into(),
        title: title.into(),
        artist: "Northlight".into(),
        artist_id: None,
        artist_mbid: Some(ARTIST.to_uppercase()),
        typed: None,
    };
    let before = r.mb.count("/artist/?query=");
    let p = r.lidarr.preview(&named(RG_SECOND, "Second Wind")).await.unwrap();
    assert!(p.reachable && p.adds_artist);
    assert_eq!(r.mb.count("/artist/?query="), before, "the named id is used");
    // The guards still run: a bootleg and a title clash with an unreleased build are refused.
    assert!(r.lidarr.preview(&named(RG_BOOT, "Basement Tapes")).await.is_err());
    assert!(r.lidarr.preview(&named(RG_SKETCH, "Quiet Sketches")).await.is_err());
    // Someone else's id: the release isn't theirs alone.
    let mut other = named(RG_SECOND, "Second Wind");
    other.artist_mbid = Some(OTHER.into());
    assert!(r.lidarr.preview(&other).await.is_err());
    assert!(r.web.writes().is_empty());
    // The id goes with a request that waits, for the retry.
    r.web.st.lock().down = true;
    let rec = r.lidarr.request(RequestInput { typed: Some("Second Wind".into()), ..named(RG_SECOND, "Second Wind") }).await.unwrap();
    assert_eq!(rec.state, RequestState::Queued);
    assert_eq!(rec.artist_mbid.as_deref(), Some(ARTIST.to_uppercase().as_str()));
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
    assert!(r.lidarr.cancel(&sent.id).await.is_err());
    r.web.st.lock().down = true;
    let waiting = r.lidarr.request(input(RG_EP, "Small Hours", None)).await.unwrap();
    assert_eq!(waiting.state, RequestState::Queued);
    let left = r.lidarr.cancel(&waiting.id).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, sent.id);
}

#[tokio::test]
async fn missing_releases_need_the_setting_and_a_green_check() {
    let r = rig();
    let every = ["studio", "ep", "live", "compilation"].map(String::from).to_vec();
    crate::settings::save(&r.db, &crate::settings::Settings { get_albums_kinds: every, ..crate::settings::load(&r.db) }).unwrap();
    // No check yet: hidden.
    assert!(!r.lidarr.missing_releases("ar1", "Northlight").await.unwrap().shown);
    assert!(!r.lidarr.status().ready, "Request stays off before a green check");
    r.lidarr.check().await.unwrap();
    assert!(r.lidarr.status().ready);
    let m = r.lidarr.missing_releases("ar1", "Northlight").await.unwrap();
    assert!(m.shown);
    let titles: Vec<&str> = m.items.iter().map(|i| i.title.as_str()).collect();
    // Have First Light; Quiet Sketches clashes with an unreleased build.
    // Singles and live albums show too, each in its section; the live album
    // says why Get can't ask for it.
    assert_eq!(titles, vec!["Tides", "Small Hours", "Second Wind", "Second Wind", "Live at the Harbor"]);
    let sections: Vec<&str> = m.items.iter().map(|i| i.section.as_str()).collect();
    assert_eq!(sections, vec!["albums", "singlesEps", "singlesEps", "albums", "live"]);
    assert_eq!(m.items[3].year, Some(2021));
    assert_eq!(m.items[1].kind, "ep");
    assert_eq!(m.items[2].kind, "single");
    assert!(m.items[..4].iter().all(|i| i.refusal.is_none()));
    assert!(m.items[4].refusal.as_deref().is_some_and(|r| r.contains("live")), "{:?}", m.items[4].refusal);
    assert_eq!(m.items[4].secondary, ["live"]);
    // Each has its small cover from the Cover Art Archive, fetched through the art cache.
    assert_eq!(
        m.items[3].thumb.as_deref(),
        Some(format!("/ext/https://coverartarchive.org/release-group/{RG_SECOND}/front-250").as_str())
    );
    assert!(m.items.iter().all(|i| crate::artistinfo::art::ext_url(i.thumb.as_deref().unwrap()).is_some()));
    assert_eq!(super::request::cover_thumb("not an id"), None);
    r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    let m = r.lidarr.missing_releases("ar1", "Northlight").await.unwrap();
    assert_eq!(m.items[3].request, Some(RequestState::Searching));
    // The Get albums kinds narrow them.
    let s = crate::settings::Settings { get_albums_kinds: vec!["studio".into()], ..crate::settings::load(&r.db) };
    crate::settings::save(&r.db, &s).unwrap();
    let m = r.lidarr.missing_releases("ar1", "Northlight").await.unwrap();
    assert_eq!(m.items.iter().map(|i| i.section.as_str()).collect::<Vec<_>>(), ["albums", "albums"]);
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
    assert!(!r.lidarr.status().ready, "a red check turns Request off again");
    assert!(r.mb.count("/release?artist=") >= 1);
}

#[tokio::test]
async fn auto_requests_run_every_guard_and_the_full_check_without_a_typed_name() {
    let r = rig();
    let rec = r.lidarr.request_auto(input(RG_SECOND, "Second Wind", None)).await.unwrap();
    assert!(rec.auto);
    assert_eq!(rec.state, RequestState::Searching);
    let log = r.web.log.lock().clone();
    let first_write = log.iter().position(|(m, _, _)| *m != Method::Get).unwrap();
    assert!(log[..first_write].iter().any(|(_, p, _)| p == "/api/v1/rootfolder"), "the full check ran first");
    assert_only_allowed_writes(&r.web);
    assert!(!r.lidarr.typed_confirmed(), "an automatic request doesn't stand in for the first typed one");
    // Never asked for twice.
    let before = r.web.writes().len();
    let again = r.lidarr.request_auto(input(RG_SECOND, "Second Wind", None)).await.unwrap();
    assert_eq!(again.id, rec.id);
    assert_eq!(r.web.writes().len(), before);
}

#[tokio::test]
async fn refused_auto_requests_are_kept_with_why_and_lidarr_is_not_asked() {
    let r = rig();
    let clash = r.lidarr.request_auto(input(RG_SKETCH, "Quiet Sketches", None)).await.unwrap();
    assert_eq!(clash.state, RequestState::Failed);
    assert!(clash.message.as_deref().unwrap().contains("unreleased album already in your library"));
    let boot = r.lidarr.request_auto(input(RG_BOOT, "Basement Tapes", None)).await.unwrap();
    assert_eq!(boot.state, RequestState::Failed);
    assert!(boot.message.as_deref().unwrap().contains("bootleg"));
    assert!(r.web.log.lock().is_empty(), "Lidarr wasn't contacted");
    let list = r.lidarr.requests();
    assert_eq!(list.len(), 2);
    assert!(list.iter().all(|x| x.auto && x.state == RequestState::Failed));
    // A refused one isn't tried again.
    assert_eq!(r.lidarr.request_auto(input(RG_BOOT, "Basement Tapes", None)).await.unwrap().id, boot.id);
}

#[tokio::test]
async fn a_red_check_or_no_lidarr_keeps_an_auto_request_waiting() {
    let r = rig();
    r.web.st.lock().red = true;
    let rec = r.lidarr.request_auto(input(RG_SECOND, "Second Wind", None)).await.unwrap();
    assert_eq!(rec.state, RequestState::Queued);
    assert!(rec.message.as_deref().unwrap().contains("red items"));
    assert!(r.web.writes().is_empty());
    r.web.st.lock().down = true;
    let rec = r.lidarr.request_auto(input(RG_EP, "Small Hours", None)).await.unwrap();
    assert_eq!(rec.state, RequestState::Queued);
    {
        let mut st = r.web.st.lock();
        st.down = false;
        st.red = false;
    }
    r.lidarr.retry_queued().await.unwrap();
    assert!(r.lidarr.requests().iter().all(|x| x.state == RequestState::Searching));
    assert_only_allowed_writes(&r.web);
}

#[tokio::test]
async fn picking_for_a_song_uses_musicbrainz_only() {
    let r = rig();
    let p = r.lidarr.pick("Northlight", Some("ar1"), "Breathe In", true, None).await.unwrap();
    assert_eq!(p.group.unwrap().id, RG_SECOND);
    assert!(r.web.log.lock().is_empty());
}

// ------------------------------------------------------------------ modes

fn protect(r: &Rig) {
    use super::modes::{ArtistPrefs, ModeSetting};
    r.lidarr.save_prefs(ARTIST, &ArtistPrefs { mode: ModeSetting::Protected, ..Default::default() }).unwrap();
    let s = crate::settings::Settings { lidarr_staging_folder: STAGING.into(), ..crate::settings::load(&r.db) };
    crate::settings::save(&r.db, &s).unwrap();
}

#[tokio::test]
async fn a_protected_artist_is_added_unmonitored_into_staging_only() {
    let r = rig();
    r.web.st.lock().staging = true;
    protect(&r);
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert_eq!(rec.state, RequestState::Searching, "{:?}", rec.message);
    assert!(rec.protected);
    assert_eq!(rec.destination.as_deref(), Some("/data/staging/music/Northlight"));
    let writes = r.web.writes();
    let add: Value = serde_json::from_str(&writes[0].2).unwrap();
    assert_eq!(add["monitored"], false);
    assert_eq!(add["rootFolderPath"], STAGING);
    assert_eq!(add["addOptions"], json!({"monitor": "none", "searchForMissingAlbums": false}));
    // Then only the requested album is monitored, and only it is searched.
    let shape: Vec<(Method, &str)> = writes.iter().map(|(m, p, _)| (*m, p.as_str())).collect();
    assert_eq!(shape, vec![(Method::Post, "/api/v1/artist"), (Method::Put, "/api/v1/album/monitor"), (Method::Post, "/api/v1/command")]);
    // Lidarr's path was read back after the add.
    assert!(r.web.log.lock().iter().any(|(m, p, _)| *m == Method::Get && p.starts_with("/api/v1/artist/")));
    assert_only_allowed_writes(&r.web);
}

#[tokio::test]
async fn protected_requests_wait_for_a_staging_folder_while_managed_ones_go_ahead() {
    let r = rig();
    use super::modes::{ArtistPrefs, ModeSetting};
    r.lidarr.save_prefs(ARTIST, &ArtistPrefs { mode: ModeSetting::Protected, ..Default::default() }).unwrap();
    let e = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap_err().to_string();
    assert!(e.contains("No staging folder for protected artists"), "{e}");
    assert!(r.web.writes().is_empty());
    // The same Lidarr, the artist Managed: sent to the library root.
    r.lidarr.save_prefs(ARTIST, &ArtistPrefs { mode: ModeSetting::Managed, ..Default::default() }).unwrap();
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert_eq!(rec.state, RequestState::Searching);
    assert!(!rec.protected);
    assert_eq!(rec.destination.as_deref(), Some("/downloads/lidarr/Northlight"));
}

#[tokio::test]
async fn the_exclusion_check_blocks_every_add() {
    for protected in [false, true] {
        let r = rig();
        r.web.st.lock().staging = true;
        protect(&r);
        if !protected {
            use super::modes::{ArtistPrefs, ModeSetting};
            r.lidarr.save_prefs(ARTIST, &ArtistPrefs { mode: ModeSetting::Managed, ..Default::default() }).unwrap();
        }
        r.web.st.lock().exclusions = vec![ARTIST.to_uppercase()];
        let e = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap_err().to_string();
        assert!(e.contains("import list exclusions"), "{e}");
        assert!(r.web.writes().is_empty(), "nothing sent (protected: {protected})");
        let auto = r.lidarr.request_auto(input(RG_EP, "Small Hours", None)).await.unwrap();
        assert_eq!(auto.state, RequestState::Failed);
        assert!(r.web.writes().is_empty());
    }
}

#[tokio::test]
async fn an_album_plex_matched_to_the_release_group_is_never_requested() {
    let r = rig();
    r.db.write(|c| {
        c.execute("UPDATE albums SET mbid = ?1 WHERE id = 'a1'", [RG_SECOND])?;
        Ok(())
    })
    .unwrap();
    let e = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap_err().to_string();
    assert!(e.contains("already in your library"), "{e}");
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn request_these_takes_at_most_ten_and_runs_each_request_on_its_own() {
    let r = rig();
    // Eleven albums: refused before Lidarr or MusicBrainz is asked anything.
    let many: Vec<RequestInput> = (0..11).map(|i| input(&format!("20000000-0000-0000-0000-0000000000{i:02}"), "X", None)).collect();
    let e = r.lidarr.request_batch(many).await.unwrap_err();
    assert!(e.message().contains("up to 10"), "{e}");
    assert!(r.web.log.lock().is_empty());
    assert!(r.lidarr.request_batch(vec![]).await.is_err());
    // The same album twice counts once; ten different ones are fine.
    let ten: Vec<RequestInput> = (0..10).map(|i| input(&format!("20000000-0000-0000-0000-0000000000{i:02}"), "X", None)).collect();
    assert_eq!(super::request::batch_inputs(ten).unwrap().len(), 10);
    let twice = vec![input(RG_SECOND, "Second Wind", None), input(&RG_SECOND.to_uppercase(), "Second Wind", None)];
    assert_eq!(super::request::batch_inputs(twice).unwrap().len(), 1);
    // Still one request at a time: the first ever needs the name typed, so
    // without it nothing is written and each says why.
    let out = r.lidarr.request_batch(vec![input(RG_SECOND, "Second Wind", None), input(RG_EP, "Small Hours", None)]).await.unwrap();
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|i| i.record.is_none() && i.error.as_deref().is_some_and(|m| m.contains("Type the album's name"))));
    assert!(r.web.writes().is_empty());
    // After one typed request, a batch sends each, with every guard.
    r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    let out = r.lidarr.request_batch(vec![input(RG_EP, "Small Hours", None), input(RG_BOOT, "Bootleg", None)]).await.unwrap();
    assert_eq!(out[0].record.as_ref().map(|x| x.state), Some(RequestState::Searching));
    assert!(out[1].record.is_none() && out[1].error.is_some(), "the bootleg is refused, the rest still went");
    assert_only_allowed_writes(&r.web);
    let saved = r.db.read(|c| Ok(super::request::saved_requests(c))).unwrap();
    assert_eq!(saved.len(), 2);
}

/// Lidarr can move an album to another edition at any moment (its daily
/// refresh with "Automatically Switch Release" on). The request reads the
/// edition before monitoring and again before searching, and stops when it
/// changed in between.
#[tokio::test]
async fn a_request_stops_before_searching_when_lidarr_switches_the_edition() {
    let r = rig();
    // The first read (before monitoring) sees the 10-track edition; later ones the 14-track one.
    r.web.st.lock().switch_after = Some(1);
    let rec = r.lidarr.request(input(RG_SECOND, "Second Wind", Some("Second Wind"))).await.unwrap();
    assert_eq!(rec.state, RequestState::Failed, "{:?}", rec.message);
    let m = rec.message.unwrap();
    assert!(m.contains("Lidarr moved Second Wind to the 14-track edition") && m.contains("didn't search"), "{m}");
    assert!(!r.web.writes().iter().any(|(m, p, _)| *m == Method::Post && p == "/api/v1/command"), "no search was sent");
    let seen = r.lidarr.changes().into_iter().find(|c| c.by_lidarr).expect("logged");
    assert_eq!(seen.what, "Lidarr switched Second Wind to the 14-track edition");
    assert_only_allowed_writes(&r.web);
}
