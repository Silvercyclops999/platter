//! Catching up after Platter was closed, and adding protected albums to the
//! library, against a faked Lidarr, a faked Plex that only sees an album
//! after a partial scan, real FLAC files in a temporary share, and two
//! devices sharing one store.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use lofty::config::WriteOptions;
use lofty::ogg::tag::VorbisComments;
use lofty::tag::TagExt;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::check::{PlexFile, PlexFiles};
use super::modes::{ArtistPrefs, ModeSetting};
use super::plexside::fake::FakePlex;
use super::promote::StagedState;
use super::shared::{CLAIM_SETTLE_MS, Shared};
use super::upgrade::Hosts;
use super::{ArtistMbid, Lidarr, LidarrDeps, PlexMusic};
use crate::artistinfo::http::HttpResponse;
use crate::db::DbHandle;
use crate::error::AppResult;
use crate::jellyfin::types::iso;
use crate::musicshare::{FolderPair, ShareMap};
use crate::plex::keychain::MemoryStore;
use crate::sync::SyncStore;
use crate::web::{Body, Method, Web, WebRequest};

const QH: &str = "b0000000-0000-0000-0000-000000000002";
const NL: &str = "a0000000-0000-0000-0000-000000000001";
const RG3: &str = "30000000-0000-0000-0000-000000000003";
const RG_SESSIONS: &str = "30000000-0000-0000-0000-000000000001";
const MUSIC: &str = "/data/media/Music";
const STAGING: &str = "/data/staging/music";
const ARCHIVE: &str = "/data/media/Music/Archive/Quiet Hours";
const T0: u64 = 1_800_000_000_000;

// ---------------------------------------------------------------- files

fn flac_bytes() -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    v.extend([0x80, 0, 0, 34]);
    v.extend([0x10, 0x00, 0x10, 0x00]);
    v.extend([0, 0, 0, 0, 0, 0]);
    let packed: u64 = (44_100u64 << 44) | (1 << 41) | (15 << 36) | (44_100 * 200);
    v.extend(packed.to_be_bytes());
    v.extend([0u8; 16]);
    v.extend(vec![0u8; 4096]);
    v
}

fn write_flac(path: &Path, pairs: &[(&str, &str)]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, flac_bytes()).unwrap();
    let mut vc = VorbisComments::new();
    for (k, v) in pairs {
        vc.push(k.to_string(), v.to_string());
    }
    vc.save_to_path(path, WriteOptions::default()).unwrap();
}

fn tags_of(path: &Path) -> Vec<(String, String)> {
    super::tagcarry::read(path).unwrap().pairs
}

// ---------------------------------------------------------------- Lidarr

#[derive(Default)]
struct State {
    history: Vec<Value>,
    /// The oldest event Lidarr still keeps (none: the first in `history`).
    oldest: Option<Value>,
    artists: Vec<Value>,
    albums: Vec<Value>,
    track_files: Vec<Value>,
    tracks: Vec<Value>,
    rename: bool,
    bin: String,
    /// Library files that existed when each write arrived.
    seen_at_write: Vec<(String, bool)>,
    probe: Option<PathBuf>,
}

struct Fake {
    st: Mutex<State>,
    log: Mutex<Vec<(Method, String, String)>>,
}

impl Fake {
    fn writes(&self) -> Vec<(Method, String, String)> {
        self.log.lock().iter().filter(|(m, _, _)| *m != Method::Get).cloned().collect()
    }

    fn handle(&self, m: Method, u: &url::Url, body: &str) -> (u16, Value) {
        let mut st = self.st.lock();
        let path = u.path().trim_start_matches("/api/v1").to_string();
        let q = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.to_string());
        if m != Method::Get {
            let exists = st.probe.as_ref().is_some_and(|p| p.exists());
            st.seen_at_write.push((path.clone(), exists));
        }
        match (m, path.as_str()) {
            (Method::Get, "/system/status") => (200, json!({"appName": "Lidarr", "version": "2.8.2"})),
            (Method::Get, "/rootfolder") => (200, json!([
                {"id": 3, "path": "/data/media/Music/Lidarr", "accessible": true, "defaultQualityProfileId": 1, "defaultMetadataProfileId": 1},
                {"id": 2, "path": STAGING, "accessible": true, "defaultQualityProfileId": 1, "defaultMetadataProfileId": 1}
            ])),
            (Method::Get, "/artist") => (200, Value::Array(st.artists.clone())),
            (Method::Get, "/config/naming") => (200, json!({"id": 1, "renameTracks": st.rename})),
            (Method::Get, "/config/metadataprovider") => (200, json!({"id": 1, "writeAudioTags": "no"})),
            (Method::Get, "/config/mediamanagement") => (200, json!({"id": 1, "recycleBin": st.bin})),
            (Method::Get, "/qualityprofile") => (200, json!([{"id": 1, "name": "Lossless", "upgradeAllowed": false}])),
            (Method::Get, "/importlist" | "/importlistexclusion" | "/notification") => (200, json!([])),
            (Method::Get, "/trackfile") if q("unmapped").is_some() => (200, json!([])),
            (Method::Get, "/trackfile") => {
                let a: i64 = q("albumId").and_then(|x| x.parse().ok()).unwrap_or(-1);
                (200, Value::Array(st.track_files.iter().filter(|f| f["albumId"] == a).cloned().collect()))
            }
            (Method::Delete, "/trackfile/bulk") => {
                let b: Value = serde_json::from_str(body).unwrap();
                let ids: Vec<i64> = b["trackFileIds"].as_array().unwrap().iter().filter_map(Value::as_i64).collect();
                st.track_files.retain(|f| !ids.contains(&f["id"].as_i64().unwrap()));
                (200, json!({}))
            }
            (Method::Get, "/track") => {
                let a: i64 = q("albumId").and_then(|x| x.parse().ok()).unwrap_or(-1);
                (200, Value::Array(st.tracks.iter().filter(|t| t["albumId"] == a).cloned().collect()))
            }
            (Method::Get, "/album") => {
                if let Some(a) = q("artistId") {
                    let a: i64 = a.parse().unwrap();
                    return (200, Value::Array(st.albums.iter().filter(|x| x["artistId"] == a).cloned().collect()));
                }
                (200, Value::Array(st.albums.clone()))
            }
            (Method::Get, p) if p.starts_with("/album/") => {
                let id: i64 = p.rsplit('/').next().unwrap().parse().unwrap();
                match st.albums.iter().find(|a| a["id"] == id) {
                    Some(a) => (200, a.clone()),
                    None => (404, json!({})),
                }
            }
            (Method::Put, "/album/monitor") => {
                let b: Value = serde_json::from_str(body).unwrap();
                for id in b["albumIds"].as_array().unwrap() {
                    if let Some(a) = st.albums.iter_mut().find(|a| a["id"] == *id) {
                        a["monitored"] = b["monitored"].clone();
                    }
                }
                (202, json!([]))
            }
            (Method::Get, "/history/since") => {
                let since = q("date").unwrap_or_default();
                let since = crate::jellyfin::types::unix_s(&since).unwrap_or(0);
                (200, Value::Array(st.history.iter().filter(|h| crate::jellyfin::types::unix_s(h["date"].as_str().unwrap()).unwrap_or(0) >= since).cloned().collect()))
            }
            (Method::Get, "/history") if q("albumId").is_some() => {
                let a: i64 = q("albumId").unwrap().parse().unwrap();
                (200, json!({"records": st.history.iter().filter(|h| h["albumId"] == a).cloned().collect::<Vec<_>>()}))
            }
            (Method::Get, "/history") => (200, json!({"records": st.oldest.clone().or(st.history.first().cloned()).into_iter().collect::<Vec<_>>()})),
            _ => (404, json!({})),
        }
    }
}

#[async_trait]
impl Web for Fake {
    async fn send(&self, req: WebRequest) -> AppResult<HttpResponse> {
        let body = match &req.body {
            Body::Json(j) => j.clone(),
            _ => String::new(),
        };
        let u = url::Url::parse(&req.url).unwrap();
        self.log.lock().push((req.method, format!("{}{}", u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default()), body.clone()));
        let (status, v) = self.handle(req.method, &u, &body);
        Ok(HttpResponse { status, body: Bytes::from(v.to_string()), ..Default::default() })
    }
}

struct Mbids;

#[async_trait]
impl ArtistMbid for Mbids {
    async fn artist_mbid(&self, id: &str, _name: &str) -> Option<String> {
        match id {
            "ar1" => Some(NL.into()),
            "ar2" => Some(QH.into()),
            _ => None,
        }
    }
}

struct PlexAll;

#[async_trait]
impl PlexMusic for PlexAll {
    async fn music_files(&self) -> AppResult<PlexFiles> {
        Ok(PlexFiles { locations: vec![MUSIC.into()], files: vec![PlexFile { path: format!("{ARCHIVE}/Sessions (2019)/1-01 - Low.flac"), size: Some(1) }] })
    }
}

struct TmpHosts(Arc<dyn super::swap::SwapHost>);

impl Hosts for TmpHosts {
    fn host(&self) -> Option<Arc<dyn super::swap::SwapHost>> {
        Some(self.0.clone())
    }
}

// ---------------------------------------------------------------- the rig

struct Dev {
    lidarr: Lidarr,
    shared: Shared,
    plex: Arc<FakePlex>,
    _db: DbHandle,
}

struct Rig {
    tmp: tempfile::TempDir,
    clock: Arc<AtomicU64>,
    web: Arc<Fake>,
    a: Dev,
}

impl Rig {
    fn share(&self, server: &str) -> PathBuf {
        let rest = server.strip_prefix(MUSIC).unwrap().trim_start_matches('/');
        self.tmp.path().join("share").join(rest)
    }

    fn staging(&self, lidarr: &str) -> PathBuf {
        let rest = lidarr.strip_prefix(STAGING).unwrap().trim_start_matches('/');
        self.tmp.path().join("staging").join(rest)
    }

    fn advance(&self, secs: u64) {
        self.clock.fetch_add(secs * 1000, Ordering::SeqCst);
    }

    fn now_s(&self) -> i64 {
        (self.clock.load(Ordering::SeqCst) / 1000) as i64
    }

    fn event(&self, id: i64, kind: &str, album: i64, artist: i64) {
        let date = iso(self.now_s());
        self.web.st.lock().history.push(json!({"id": id, "albumId": album, "artistId": artist, "eventType": kind, "date": date}));
    }

    /// One catch-up pass, then another after the claim has settled.
    /// Catch-ups until every claim has settled; `handled` adds up.
    async fn pass(&self, d: &Dev) -> super::catchup::CatchUpReport {
        let mut r = d.lidarr.catch_up().await.unwrap();
        let mut handled = r.handled;
        for _ in 0..10 {
            if r.settling == 0 {
                break;
            }
            self.advance(CLAIM_SETTLE_MS / 1000 + 1);
            r = d.lidarr.catch_up().await.unwrap();
            handled += r.handled;
        }
        r.handled = handled;
        r
    }
}

fn library(db: &DbHandle) {
    db.write(|c| {
        c.execute_batch(&format!(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched, year, mbid) VALUES
               ('n1', 'First Light', 'First Light', 'ar1', 'Northlight', 'Album', '[]', 1, 2019, NULL),
               ('q1', 'Sessions', 'Sessions', 'ar2', 'Quiet Hours', 'Album', '[]', 1, 2019, '{RG_SESSIONS}'),
               ('q2', 'Tapes', 'Tapes', 'ar2', 'Quiet Hours', 'Album', '[]', 1, 2021, NULL),
               ('q3', 'Demos', 'Demos', 'ar2', 'Quiet Hours', NULL, '[]', 0, NULL, NULL);
             INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, file, disc_number, track_number) VALUES
               ('t1', 'Dawn', 'n1', 'First Light', 'ar1', 'Northlight', '/p/t1', '/data/media/Music/Lidarr/Northlight/First Light (2019)/01 Dawn.flac', 1, 1),
               ('t2', 'Low', 'q1', 'Sessions', 'ar2', 'Quiet Hours', '/p/t2', '{ARCHIVE}/Sessions (2019)/1-01 - Low.flac', 1, 1),
               ('t3', 'High', 'q1', 'Sessions', 'ar2', 'Quiet Hours', '/p/t3', '{ARCHIVE}/Sessions (2019)/1-02 - High.flac', 1, 2),
               ('t4', 'Hiss', 'q2', 'Tapes', 'ar2', 'Quiet Hours', '/p/t4', '{ARCHIVE}/Tapes (2021)/1-01 - Hiss.flac', 1, 1),
               ('t5', 'Sketch', 'q3', 'Demos', 'ar2', 'Quiet Hours', '/p/t5', '{ARCHIVE}/Demos/01 Sketch.flac', 1, 1);
             INSERT INTO file_tags (part_key, status, version, release, scanned_at) VALUES
               ('/p/t1', 'ok', NULL, 'released', 1), ('/p/t2', 'ok', NULL, 'released', 1), ('/p/t3', 'ok', NULL, 'released', 1),
               ('/p/t4', 'ok', NULL, 'released', 1), ('/p/t5', 'ok', 'leak', 'leaked', 1);"
        ))?;
        Ok(())
    })
    .unwrap();
}

fn device(tmp: &Path, name: &str, clock: Arc<AtomicU64>, web: Arc<Fake>) -> Dev {
    let db = DbHandle::open(tmp.join(format!("{name}.db"))).unwrap();
    crate::settings::save(
        &db,
        &crate::settings::Settings { lidarr_url: "http://10.20.30.40:8686".into(), lidarr_staging_folder: STAGING.into(), lidarr_practice_until: 0, ..Default::default() },
    )
    .unwrap();
    library(&db);
    let c = clock.clone();
    let store = SyncStore::with_clock(db.clone(), name, Arc::new(move || c.load(Ordering::SeqCst)));
    let label = name.to_string();
    let shared = Shared::new(store, Arc::new(move || label.clone()));
    let plex = Arc::new(FakePlex { auto: Some(true), ..Default::default() });
    *plex.empty_trash.lock() = Some(true);
    let c = clock.clone();
    let lidarr = Lidarr::new(LidarrDeps {
        web: web.clone(),
        keys: Arc::new(MemoryStore::new()),
        db: db.clone(),
        plex: Arc::new(PlexAll),
        clock: Arc::new(move || (c.load(Ordering::SeqCst) / 1000) as i64),
        mb: Arc::new(super::request_tests::NoMb),
        artist_mbid: Arc::new(Mbids),
        album_wait: std::time::Duration::ZERO,
        shared: shared.clone(),
        plex_server: plex.clone(),
    });
    lidarr.set_key(Some("abc123")).unwrap();
    let map = ShareMap {
        music: vec![FolderPair { server: MUSIC.into(), local: tmp.join("share").to_string_lossy().into() }],
        staging: tmp.join("staging").to_string_lossy().into(),
        backup: String::new(),
    };
    let host = super::swap::DesktopHost { map, lidarr_staging: STAGING.into(), fs: Box::new(super::swap::LocalFs), tags: Box::new(super::swap::LoftyTags) };
    lidarr.set_hosts(Arc::new(TmpHosts(Arc::new(host))));
    Dev { lidarr, shared, plex, _db: db }
}

fn rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicU64::new(T0));
    let web = Arc::new(Fake { st: Mutex::new(State { bin: "/data/lidarr-recycle".into(), ..Default::default() }), log: Mutex::new(vec![]) });
    {
        let mut st = web.st.lock();
        st.artists = vec![
            json!({"id": 9, "artistName": "Northlight", "foreignArtistId": NL, "path": "/data/media/Music/Lidarr/Northlight", "qualityProfileId": 1}),
            json!({"id": 7, "artistName": "Quiet Hours", "foreignArtistId": QH, "path": format!("{STAGING}/Quiet Hours"), "qualityProfileId": 1}),
        ];
    }
    let a = device(tmp.path(), "aaa", clock.clone(), web.clone());
    let r = Rig { tmp, clock, web, a };
    // The library's files: two official albums kept the same way, one leaked demo.
    let common = [("ALBUMARTIST", "Quiet Hours"), ("ARTIST", "Quiet Hours"), ("GENRE", "Ambient"), ("DISCNUMBER", "1"), ("STATUS", "Released")];
    for (file, title, album) in [("Sessions (2019)/1-01 - Low.flac", "Low", "Sessions"), ("Sessions (2019)/1-02 - High.flac", "High", "Sessions"), ("Tapes (2021)/1-01 - Hiss.flac", "Hiss", "Tapes")] {
        let mut t: Vec<(&str, &str)> = common.to_vec();
        t.extend([("TITLE", title), ("ALBUM", album), ("VERSION", "Official release (CD)")]);
        write_flac(&r.share(&format!("{ARCHIVE}/{file}")), &t);
    }
    r
}

/// A new protected album in staging, imported by Lidarr.
fn stage_album(r: &Rig, album_id: i64, title: &str, rg: &str, status: &str, secondary: &[&str]) {
    let dir = format!("{STAGING}/Quiet Hours/{title} (2026)");
    let mut st = r.web.st.lock();
    st.albums.push(json!({"id": album_id, "title": title, "artistId": 7, "foreignAlbumId": rg, "monitored": true,
        "secondaryTypes": secondary, "releaseDate": "2026-03-01T00:00:00Z",
        "releases": [{"id": album_id * 10, "title": "CD", "status": status, "monitored": true}],
        "statistics": {"trackFileCount": 2, "trackCount": 2}}));
    for (i, (song, file)) in [("Arrive", "01 - Arrive.flac"), ("Leave", "02 - Leave.flac")].iter().enumerate() {
        let id = album_id * 100 + i as i64;
        let path = format!("{dir}/{file}");
        st.track_files.push(json!({"id": id, "albumId": album_id, "path": path, "qualityCutoffNotMet": false, "quality": {"quality": {"id": 6, "name": "FLAC"}}}));
        st.tracks.push(json!({"id": id + 5000, "albumId": album_id, "title": song, "mediumNumber": 1, "absoluteTrackNumber": i + 1, "trackFileId": id}));
        drop(path);
    }
    drop(st);
    for (song, file) in [("Arrive", "01 - Arrive.flac"), ("Leave", "02 - Leave.flac")] {
        write_flac(&r.staging(&format!("{dir}/{file}")), &[("TITLE", song), ("ARTIST", "Quiet Hours"), ("ISRC", "XXA012600001"), ("MUSICBRAINZ_TRACKID", "abc")]);
    }
}

// ---------------------------------------------------------------- Managed

#[tokio::test]
async fn a_managed_import_plex_misses_gets_a_partial_scan_then_is_announced() {
    let r = rig();
    let first = r.a.lidarr.catch_up().await.unwrap();
    assert_eq!(first.handled, 0, "the first run starts from now");
    r.advance(10);
    {
        let mut st = r.web.st.lock();
        st.albums.push(json!({"id": 20, "title": "Second Wind", "artistId": 9, "foreignAlbumId": "x", "monitored": true, "releases": []}));
        st.track_files.push(json!({"id": 1, "albumId": 20, "path": "/data/media/Music/Lidarr/Northlight/Second Wind (2021)/01 Rise.flac"}));
    }
    r.event(1, "grabbed", 20, 9);
    r.event(2, "downloadImported", 20, 9);
    let rep = r.pass(&r.a).await;
    assert_eq!(rep.handled, 2);
    // Plex didn't find it on its own within two minutes: just that folder is scanned.
    let folder = "/data/media/Music/Lidarr/Northlight/Second Wind (2021)";
    let all: Vec<String> = r.a.plex.scans.lock().clone();
    assert_eq!(all, vec![folder.to_string()], "only that folder");
    r.advance(61);
    let rep = r.a.lidarr.catch_up().await.unwrap();
    assert_eq!(rep.announced, vec!["Second Wind"]);
    // Nothing global: no rescan command ever went to Lidarr.
    assert!(!r.web.log.lock().iter().any(|(_, p, b)| p.contains("/command") || b.contains("Rescan")));
    let status = r.a.lidarr.catch_up_status();
    assert_eq!(status.last_device.as_deref(), Some("aaa"));
}

#[tokio::test]
async fn a_managed_import_plex_finds_on_its_own_is_only_announced() {
    let r = rig();
    r.a.lidarr.catch_up().await.unwrap();
    r.advance(10);
    {
        let mut st = r.web.st.lock();
        st.albums.push(json!({"id": 21, "title": "Small Hours", "artistId": 9, "foreignAlbumId": "y", "monitored": true, "releases": []}));
        st.track_files.push(json!({"id": 2, "albumId": 21, "path": "/data/media/Music/Lidarr/Northlight/Small Hours (2022)/01 Late.flac"}));
    }
    r.a.plex.seen.lock().push("/data/media/Music/Lidarr/Northlight/Small Hours (2022)".into());
    r.event(3, "downloadImported", 21, 9);
    r.pass(&r.a).await;
    r.advance(121);
    let rep = r.a.lidarr.catch_up().await.unwrap();
    assert!(r.a.plex.scans.lock().is_empty(), "Plex's own scan found it");
    assert_eq!(rep.announced, vec!["Small Hours"]);
}

#[tokio::test]
async fn two_devices_never_act_on_the_same_event_and_share_the_cursor() {
    let r = rig();
    let b = device(r.tmp.path(), "bbb", r.clock.clone(), r.web.clone());
    r.a.lidarr.catch_up().await.unwrap();
    super::shared::tests::sync(&r.a.shared, &b.shared);
    r.advance(10);
    {
        let mut st = r.web.st.lock();
        st.albums.push(json!({"id": 20, "title": "Second Wind", "artistId": 9, "foreignAlbumId": "x", "monitored": true, "releases": []}));
        st.track_files.push(json!({"id": 1, "albumId": 20, "path": "/data/media/Music/Lidarr/Northlight/Second Wind (2021)/01 Rise.flac"}));
    }
    r.event(5, "downloadImported", 20, 9);
    // Both look before either's claim reached the other.
    assert_eq!(r.a.lidarr.catch_up().await.unwrap().settling, 1);
    r.advance(1);
    assert_eq!(b.lidarr.catch_up().await.unwrap().settling, 1);
    super::shared::tests::sync(&r.a.shared, &b.shared);
    super::shared::tests::sync(&b.shared, &r.a.shared);
    r.advance(CLAIM_SETTLE_MS / 1000 + 130);
    let ra = r.a.lidarr.catch_up().await.unwrap();
    let rb = b.lidarr.catch_up().await.unwrap();
    assert_eq!((ra.elsewhere, rb.elsewhere), (0, 1), "the earlier claim wins");
    assert_eq!(r.a.plex.scans.lock().len() + b.plex.scans.lock().len(), 1, "one scan between them");
    super::shared::tests::sync(&r.a.shared, &b.shared);
    assert_eq!(b.lidarr.catch_up_status().last_device.as_deref(), Some("aaa"), "the shared cursor says who did the work");
}

#[tokio::test]
async fn history_that_no_longer_reaches_back_says_so_and_checks_plex() {
    let r = rig();
    r.a.lidarr.catch_up().await.unwrap();
    r.advance(5);
    r.event(1, "grabbed", 20, 9);
    r.pass(&r.a).await;
    // Platter was closed for a long time; Lidarr's oldest record is newer than the last one handled.
    r.advance(90 * 86_400);
    r.web.st.lock().history.clear();
    r.web.st.lock().oldest = Some(json!({"id": 900, "eventType": "grabbed", "date": iso(r.now_s() - 3600)}));
    let rep = r.a.lidarr.catch_up().await.unwrap();
    assert_eq!(rep.gap.as_deref(), Some(super::catchup::GAP_NOTE));
    assert!(r.a.lidarr.catch_up_status().gap.unwrap().contains("may be missed"));
}

#[tokio::test]
async fn turning_on_an_unsafe_setting_pauses_protected_work() {
    let r = rig();
    r.a.lidarr.catch_up().await.unwrap();
    r.web.st.lock().rename = true;
    let rep = r.a.lidarr.catch_up().await.unwrap();
    assert!(rep.paused.as_deref().unwrap().contains("Rename Tracks was turned on"), "{rep:?}");
    // Changing it back resumes.
    r.web.st.lock().rename = false;
    assert!(r.a.lidarr.catch_up().await.unwrap().paused.is_none());
    r.web.st.lock().bin = String::new();
    assert!(r.a.lidarr.catch_up().await.unwrap().paused.unwrap().contains("recycling bin was cleared"));
    r.a.lidarr.resume_protected().unwrap();
    assert!(r.a.lidarr.catch_up().await.unwrap().paused.is_none(), "the user resumed with the bin cleared");
}

// ---------------------------------------------------------------- Protected

async fn staged(r: &Rig, album: i64, title: &str, rg: &str, status: &str, secondary: &[&str], kind: &str) -> super::promote::Staged {
    stage_album(r, album, title, rg, status, secondary);
    r.event(album, kind, album, 7);
    r.pass(&r.a).await;
    r.a.lidarr.staged().into_iter().find(|s| s.album_id == album).unwrap()
}

#[tokio::test]
async fn the_first_album_asks_with_the_exact_names_and_tags_then_is_added_in_order() {
    let r = rig();
    r.a.lidarr.catch_up().await.unwrap();
    r.advance(5);
    let s = staged(&r, 30, "Third Way", RG3, "Official", &[], "downloadImported").await;
    assert_eq!(s.state, StagedState::Ready, "{:?}", s.reason);
    assert!(s.reason.as_deref().unwrap().contains("first new album"));
    let plan = s.plan.clone().unwrap();
    assert_eq!(s.folder.as_deref(), Some(format!("{ARCHIVE}/Third Way (2026)").as_str()));
    assert_eq!(plan.files[0].name, "1-01 - Arrive.flac");
    assert!(plan.files[0].edits.contains(&("VERSION".into(), Some("Official release (Lidarr, FLAC, 2027-01-15)".into()))), "{:?}", plan.files[0].edits);
    assert!(r.web.writes().is_empty(), "nothing done before the yes");
    // Lidarr's monitor write must arrive after the copy.
    r.web.st.lock().probe = Some(r.share(&format!("{ARCHIVE}/Third Way (2026)/1-02 - Leave.flac")));
    let s = r.a.lidarr.promotion_approve(&s.id, true).await.unwrap();
    assert_eq!(s.state, StagedState::Added);
    let new = r.share(&format!("{ARCHIVE}/Third Way (2026)/1-01 - Arrive.flac"));
    let t = tags_of(&new);
    let get = |k: &str| t.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    assert_eq!((get("ALBUMARTIST"), get("GENRE"), get("STATUS"), get("DISCNUMBER")), (Some("Quiet Hours"), Some("Ambient"), Some("Released"), Some("1")));
    assert_eq!((get("ISRC"), get("MUSICBRAINZ_TRACKID")), (Some("XXA012600001"), Some("abc")), "ids kept");
    // Order: copied, then unmonitored, then (only then) staging freed.
    let w = r.web.st.lock().seen_at_write.clone();
    let paths: Vec<&str> = w.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(paths, vec!["/album/monitor", "/trackfile/bulk"]);
    assert!(w.iter().all(|(_, existed)| *existed), "the library copy was there before every write");
    let writes = r.web.writes();
    assert_eq!(serde_json::from_str::<Value>(&writes[0].2).unwrap(), json!({"albumIds": [30], "monitored": false}));
    assert_eq!(serde_json::from_str::<Value>(&writes[1].2).unwrap(), json!({"trackFileIds": [3000, 3001]}));
    assert!(r.a.plex.scans.lock().contains(&format!("{ARCHIVE}/Third Way (2026)")));
    // Existing files are never touched.
    assert_eq!(tags_of(&r.share(&format!("{ARCHIVE}/Sessions (2019)/1-01 - Low.flac"))).iter().find(|(k, _)| k == "VERSION").unwrap().1, "Official release (CD)");
    // The log has the add (asked), the unmonitor and the freeing (part of it).
    let log = r.a.lidarr.changes();
    assert!(log.iter().any(|e| e.what.starts_with("Added Third Way") && e.asked));
    assert!(log.iter().any(|e| e.what.starts_with("Freed Third Way") && !e.asked && e.after.contains("recycling bin")));

    // "Do this automatically": the next one is added on its own.
    r.advance(5);
    let s2 = staged(&r, 31, "Fourth Wall", "30000000-0000-0000-0000-000000000004", "Official", &[], "downloadImported").await;
    assert_eq!(s2.state, StagedState::Added, "{:?}", s2.reason);
    assert!(s2.auto);
}

#[tokio::test]
async fn unofficial_partial_and_clashing_albums_wait() {
    let r = rig();
    r.a.lidarr.catch_up().await.unwrap();
    r.advance(5);
    let s = staged(&r, 40, "Bootleg Night", "40000000-0000-0000-0000-000000000001", "Bootleg", &[], "downloadImported").await;
    assert_eq!(s.state, StagedState::Waiting);
    assert!(s.reason.unwrap().contains("not Official"));
    let s = staged(&r, 41, "Live Room", "40000000-0000-0000-0000-000000000002", "Official", &["Live"], "downloadImported").await;
    assert!(s.reason.unwrap().contains("Live release"));
    let s = staged(&r, 42, "Half Done", "40000000-0000-0000-0000-000000000003", "Official", &[], "albumImportIncomplete").await;
    assert_eq!(s.state, StagedState::Waiting);
    assert!(s.reason.as_deref().unwrap().contains("never adds a partial album"));
    assert!(r.a.lidarr.promotion_approve(&s.id, false).await.is_err(), "not even by hand");
    let s = staged(&r, 43, "Tapes", "40000000-0000-0000-0000-000000000004", "Official", &[], "downloadImported").await;
    assert!(s.reason.unwrap().contains("same as Tapes"), "a clash with the library");
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn one_official_album_to_learn_from_always_asks() {
    let r = rig();
    std::fs::remove_dir_all(r.share(&format!("{ARCHIVE}/Tapes (2021)"))).unwrap();
    r.a.lidarr.save_prefs(QH, &ArtistPrefs { mode: ModeSetting::Auto, auto_add: Some(true), ..Default::default() }).unwrap();
    r.a.lidarr.catch_up().await.unwrap();
    r.advance(5);
    let s = staged(&r, 30, "Third Way", RG3, "Official", &[], "downloadImported").await;
    assert_eq!(s.state, StagedState::Ready, "low confidence asks even with automatic adds on");
    assert!(s.reason.unwrap().contains("isn't sure"));
}

#[tokio::test]
async fn practice_mode_logs_what_it_would_add_and_adds_nothing() {
    let r = rig();
    let s = crate::settings::Settings { lidarr_practice_until: r.now_s() + 7 * 86_400, ..crate::settings::load(&r.a._db) };
    crate::settings::save(&r.a._db, &s).unwrap();
    r.a.lidarr.save_prefs(QH, &ArtistPrefs { auto_add: Some(true), ..Default::default() }).unwrap();
    r.a.lidarr.catch_up().await.unwrap();
    r.advance(5);
    let s = staged(&r, 30, "Third Way", RG3, "Official", &[], "downloadImported").await;
    assert_eq!(s.state, StagedState::Practice);
    assert!(!r.share(&format!("{ARCHIVE}/Third Way (2026)")).exists());
    assert!(r.web.writes().is_empty());
    let log = r.a.lidarr.changes();
    assert!(log[0].practice && log[0].what.contains("Would add Third Way"));
}

#[tokio::test]
async fn undo_moves_the_album_out_and_keeps_plex_trash_for_the_move() {
    let r = rig();
    r.a.lidarr.catch_up().await.unwrap();
    r.advance(5);
    let s = staged(&r, 30, "Third Way", RG3, "Official", &[], "downloadImported").await;
    let s = r.a.lidarr.promotion_approve(&s.id, false).await.unwrap();
    let e = r.a.lidarr.promotion_undo(&s.id, false).await.unwrap_err().to_string();
    assert!(e.contains("empties its trash"), "{e}");
    assert!(r.a.plex.trash_changes.lock().is_empty(), "asked first");
    let s = r.a.lidarr.promotion_undo(&s.id, true).await.unwrap();
    assert_eq!(s.state, StagedState::Undone);
    assert_eq!(*r.a.plex.trash_changes.lock(), vec![false, true], "off for the move, back on after");
    assert!(!r.share(&format!("{ARCHIVE}/Third Way (2026)")).exists());
    let backup = r.tmp.path().join("share").join("_to_delete");
    assert!(std::fs::read_dir(&backup).unwrap().next().is_some());
    assert!(std::fs::read_to_string(r.tmp.path().join("share/.plexignore")).unwrap().contains("_to_delete/*"));
}

#[tokio::test]
async fn self_healing_unmonitors_protected_albums_lidarr_no_longer_needs() {
    let r = rig();
    {
        let mut st = r.web.st.lock();
        // In the library already (Plex matched its release group), still monitored.
        st.albums.push(json!({"id": 50, "title": "Sessions", "artistId": 7, "foreignAlbumId": RG_SESSIONS, "monitored": true, "statistics": {"trackFileCount": 0}}));
        // Imported once into staging, files gone, still monitored.
        st.albums.push(json!({"id": 51, "title": "Gone", "artistId": 7, "foreignAlbumId": "50000000-0000-0000-0000-000000000001", "monitored": true, "statistics": {"trackFileCount": 0}}));
        st.history.push(json!({"id": 1, "albumId": 51, "artistId": 7, "eventType": "downloadImported", "date": iso(r.now_s() - 86_400)}));
        // Requested and not downloaded yet: stays monitored.
        st.albums.push(json!({"id": 52, "title": "Waiting", "artistId": 7, "foreignAlbumId": "50000000-0000-0000-0000-000000000002", "monitored": true, "statistics": {"trackFileCount": 0}}));
    }
    let rep = r.a.lidarr.catch_up().await.unwrap();
    assert_eq!(rep.healed, vec!["Sessions", "Gone"]);
    let w = r.web.writes();
    assert_eq!(serde_json::from_str::<Value>(&w[0].2).unwrap(), json!({"albumIds": [50, 51], "monitored": false}));
}
