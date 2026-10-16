//! Every Lidarr write asks, is logged and can be put back; the re-match
//! guard against a faked Lidarr.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::check::{ExclusionTarget, FixAction, PlexFiles};
use super::{Lidarr, LidarrDeps, PlexMusic};
use crate::artistinfo::http::HttpResponse;
use crate::db::DbHandle;
use crate::error::AppResult;
use crate::plex::keychain::MemoryStore;
use crate::web::{Body, Method, Web, WebRequest};

const QH: &str = "b0000000-0000-0000-0000-000000000002";
const DIR: &str = "/data/media/Music/Lidarr/Northlight/First Light (2019)";

struct State {
    naming: Value,
    media: Value,
    metadata: Value,
    exclusions: Vec<Value>,
    roots: Vec<Value>,
    profiles: Vec<Value>,
    artists: Vec<Value>,
    album_files: Vec<Value>,
    unmapped: Vec<Value>,
    /// The import drops one of the album's files (the edition-switch trap).
    import_drops: bool,
    next: i64,
}

struct Fake {
    st: Mutex<State>,
    log: Mutex<Vec<(Method, String, String)>>,
}

impl Fake {
    fn new() -> Arc<Self> {
        let flac = |id: i64, name: &str| json!({"quality": {"id": id, "name": name}, "items": [], "allowed": true});
        Arc::new(Self {
            st: Mutex::new(State {
                naming: json!({"id": 1, "renameTracks": true, "standardTrackFormat": "{Album Title}/{track:00} {Track Title}"}),
                media: json!({"id": 1, "recycleBin": "", "deleteEmptyFolders": false}),
                metadata: json!({"id": 1, "writeAudioTags": "newFiles", "scrubAudioTags": false}),
                exclusions: vec![],
                roots: vec![json!({"id": 3, "path": "/data/media/Music/Lidarr", "accessible": true, "defaultQualityProfileId": 1, "defaultMetadataProfileId": 1})],
                profiles: vec![json!({"id": 1, "name": "Lossless", "upgradeAllowed": true, "cutoff": 1001, "items": [
                    {"quality": {"id": 1, "name": "MP3-320"}, "items": [], "allowed": true},
                    {"id": 1001, "name": "Lossless", "allowed": true, "items": [flac(6, "FLAC"), flac(21, "FLAC 24bit"), flac(7, "ALAC")]}
                ]})],
                artists: vec![json!({"id": 9, "artistName": "Northlight", "path": "/data/media/Music/Lidarr/Northlight", "qualityProfileId": 1})],
                album_files: vec![json!({"id": 1, "path": format!("{DIR}/1-01 Dawn.flac")})],
                unmapped: vec![json!({"id": 0, "path": format!("{DIR}/1-02 Noon.flac")})],
                import_drops: false,
                next: 50,
            }),
            log: Mutex::new(vec![]),
        })
    }

    fn writes(&self) -> Vec<(Method, String, String)> {
        self.log.lock().iter().filter(|(m, _, _)| *m != Method::Get).cloned().collect()
    }

    fn handle(&self, m: Method, u: &url::Url, body: &str) -> (u16, Value) {
        let mut st = self.st.lock();
        let path = u.path().trim_start_matches("/api/v1").to_string();
        let q = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.to_string());
        let b: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        match (m, path.as_str()) {
            (Method::Get, "/system/status") => (200, json!({"appName": "Lidarr", "version": "2.8.2"})),
            (Method::Get, "/config/naming") => (200, st.naming.clone()),
            (Method::Get, "/config/mediamanagement") => (200, st.media.clone()),
            (Method::Get, "/config/metadataprovider") => (200, st.metadata.clone()),
            (Method::Put, "/config/naming/1") => {
                st.naming = b;
                (202, st.naming.clone())
            }
            (Method::Put, "/config/mediamanagement/1") => {
                st.media = b;
                (202, st.media.clone())
            }
            (Method::Put, "/config/metadataprovider/1") => {
                st.metadata = b;
                (202, st.metadata.clone())
            }
            (Method::Get, "/importlistexclusion") => (200, Value::Array(st.exclusions.clone())),
            (Method::Post, "/importlistexclusion") => {
                st.next += 1;
                let mut e = b;
                e["id"] = json!(st.next);
                st.exclusions.push(e.clone());
                (201, e)
            }
            (Method::Delete, p) if p.starts_with("/importlistexclusion/") => {
                let id: i64 = p.rsplit('/').next().unwrap().parse().unwrap();
                st.exclusions.retain(|e| e["id"] != id);
                (200, json!({}))
            }
            (Method::Get, "/rootfolder") => (200, Value::Array(st.roots.clone())),
            (Method::Post, "/rootfolder") => {
                st.next += 1;
                let mut r = b;
                r["id"] = json!(st.next);
                st.roots.push(r.clone());
                (201, r)
            }
            (Method::Delete, p) if p.starts_with("/rootfolder/") => {
                let id: i64 = p.rsplit('/').next().unwrap().parse().unwrap();
                st.roots.retain(|r| r["id"] != id);
                (200, json!({}))
            }
            (Method::Get, "/qualityprofile") => (200, Value::Array(st.profiles.clone())),
            (Method::Post, "/qualityprofile") => {
                st.next += 1;
                let mut p = b;
                p["id"] = json!(st.next);
                st.profiles.push(p.clone());
                (201, p)
            }
            (Method::Delete, p) if p.starts_with("/qualityprofile/") => {
                let id: i64 = p.rsplit('/').next().unwrap().parse().unwrap();
                st.profiles.retain(|r| r["id"] != id);
                (200, json!({}))
            }
            (Method::Get, "/metadataprofile") => (200, json!([{"id": 1, "name": "Standard"}])),
            (Method::Get, "/artist") => (200, Value::Array(st.artists.clone())),
            (Method::Get, "/trackfile") if q("unmapped").is_some() => (200, Value::Array(st.unmapped.clone())),
            (Method::Get, "/trackfile") => (200, Value::Array(st.album_files.clone())),
            (Method::Get, "/manualimport") => (
                200,
                json!([{"path": format!("{DIR}/1-02 Noon.flac"), "artist": {"id": 9}, "album": {"id": 10, "title": "First Light"},
                    "albumReleaseId": 100, "tracks": [{"id": 77, "mediumNumber": 1, "absoluteTrackNumber": 2}],
                    "quality": {"quality": {"id": 6, "name": "FLAC"}}, "rejections": [{"reason": "Has missing tracks", "type": "permanent"}]}]),
            ),
            (Method::Get, "/album/10") => (
                200,
                json!({"id": 10, "title": "First Light", "artistId": 9, "releases": [{"id": 100, "title": "CD", "monitored": true}, {"id": 101, "title": "Deluxe", "monitored": false}]}),
            ),
            (Method::Get, "/track") => (200, json!([{"id": 501, "mediumNumber": 1, "absoluteTrackNumber": 1, "hasFile": true}, {"id": 502, "mediumNumber": 1, "absoluteTrackNumber": 2, "hasFile": false}])),
            (Method::Post, "/command") if b["name"] == "ManualImport" => {
                for f in b["files"].as_array().unwrap() {
                    st.next += 1;
                    let id = st.next;
                    st.album_files.push(json!({"id": id, "path": f["path"]}));
                }
                st.unmapped.clear();
                if st.import_drops {
                    st.album_files.remove(0);
                }
                (201, json!({"id": 1}))
            }
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
        let p = format!("{}{}", u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default());
        self.log.lock().push((req.method, p, body.clone()));
        let (status, v) = self.handle(req.method, &u, &body);
        Ok(HttpResponse { status, body: Bytes::from(v.to_string()), ..Default::default() })
    }
}

struct Plex;

#[async_trait]
impl PlexMusic for Plex {
    async fn music_files(&self) -> AppResult<PlexFiles> {
        Ok(PlexFiles { locations: vec!["/data/media/Music".into()], files: vec![] })
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    web: Arc<Fake>,
    lidarr: Lidarr,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("c.db")).unwrap();
    crate::settings::save(&db, &crate::settings::Settings { lidarr_url: "http://10.20.30.40:8686".into(), ..Default::default() }).unwrap();
    let web = Fake::new();
    let lidarr = Lidarr::new(LidarrDeps {
        web: web.clone(),
        keys: Arc::new(MemoryStore::new()),
        db: db.clone(),
        plex: Arc::new(Plex),
        clock: Arc::new(|| 1_800_000_000),
        mb: Arc::new(super::request_tests::NoMb),
        artist_mbid: Arc::new(super::request_tests::NoMbid),
        album_wait: std::time::Duration::ZERO,
        shared: super::shared::tests::on(&db),
        plex_server: Arc::new(super::plexside::fake::FakePlex::default()),
    });
    lidarr.set_key(Some("abc123")).unwrap();
    Rig { _dir: dir, web, lidarr }
}

#[tokio::test]
async fn a_setting_change_is_shown_first_then_made_logged_and_put_back() {
    let r = rig();
    let p = r.lidarr.propose(FixAction::RenameOff, None).await.unwrap();
    assert_eq!((p.before.as_str(), p.after.as_str()), ("On", "Off"));
    assert!(r.web.writes().is_empty(), "showing a change writes nothing");
    let e = r.lidarr.apply(&p.id).await.unwrap();
    assert!(e.asked);
    let w = r.web.writes();
    assert_eq!(w.len(), 1);
    assert_eq!((w[0].0, w[0].1.as_str()), (Method::Put, "/api/v1/config/naming/1"));
    let body: Value = serde_json::from_str(&w[0].2).unwrap();
    assert_eq!(body["renameTracks"], false);
    assert_eq!(body["standardTrackFormat"], "{Album Title}/{track:00} {Track Title}", "the rest of the page is written back as it was");
    let log = r.lidarr.changes();
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].what.as_str(), log[0].before.as_str(), log[0].after.as_str()), ("Turn off Rename Tracks", "On", "Off"));
    // A proposal is used once.
    assert!(r.lidarr.apply(&p.id).await.is_err());
    // Put it back.
    r.lidarr.put_back(&e.id).await.unwrap();
    assert_eq!(r.web.st.lock().naming["renameTracks"], true);
    assert!(r.lidarr.changes()[0].put_back_at.is_some());
    assert!(r.lidarr.put_back(&e.id).await.is_err(), "only once");
}

#[tokio::test]
async fn a_setting_changed_after_it_was_shown_is_not_overwritten() {
    let r = rig();
    let p = r.lidarr.propose(FixAction::WriteTagsOff, None).await.unwrap();
    r.web.st.lock().metadata["writeAudioTags"] = json!("allFiles");
    let e = r.lidarr.apply(&p.id).await.unwrap_err().to_string();
    assert!(e.contains("changed since"), "{e}");
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn exclusions_are_added_after_a_yes_and_can_be_taken_off_again() {
    let r = rig();
    let action = FixAction::AddExclusions { artists: vec![ExclusionTarget { name: "Quiet Hours".into(), mbid: QH.into() }] };
    let p = r.lidarr.propose(action.clone(), None).await.unwrap();
    assert!(p.after.contains("Quiet Hours"));
    let e = r.lidarr.apply(&p.id).await.unwrap();
    assert_eq!(r.web.st.lock().exclusions[0]["foreignId"], QH);
    assert!(r.lidarr.propose(action, None).await.is_err(), "already there");
    r.lidarr.put_back(&e.id).await.unwrap();
    assert!(r.web.st.lock().exclusions.is_empty());
}

#[tokio::test]
async fn the_recycle_bin_and_staging_root_stay_out_of_plex_and_other_roots() {
    let r = rig();
    for bad in ["/data/media/Music/recycle", "/data/media/Music/Lidarr/.recycle", "recycle"] {
        assert!(r.lidarr.propose(FixAction::SetRecycleBin, Some(bad.into())).await.is_err(), "{bad}");
    }
    let p = r.lidarr.propose(FixAction::SetRecycleBin, Some("/data/lidarr-recycle/".into())).await.unwrap();
    assert_eq!((p.before.as_str(), p.after.as_str()), ("(none)", "/data/lidarr-recycle"));
    for bad in ["/data/media/Music/Staging", "/data/media/Music/Lidarr/staging"] {
        assert!(r.lidarr.propose(FixAction::AddStagingRoot { path: bad.into() }, None).await.is_err(), "{bad}");
    }
    assert!(r.web.writes().is_empty());
    let p = r.lidarr.propose(FixAction::AddStagingRoot { path: "/data/staging/music".into() }, None).await.unwrap();
    let e = r.lidarr.apply(&p.id).await.unwrap();
    let root = r.web.st.lock().roots.last().cloned().unwrap();
    assert_eq!((root["path"].as_str(), root["defaultMonitorOption"].as_str()), (Some("/data/staging/music"), Some("none")));
    // Once Lidarr keeps an artist there, the root is never removed.
    r.web.st.lock().artists.push(json!({"id": 11, "path": "/data/staging/music/Quiet Hours"}));
    assert!(r.lidarr.put_back(&e.id).await.unwrap_err().to_string().contains("won't remove it"));
    assert_eq!(r.web.st.lock().roots.len(), 2);
}

#[tokio::test]
async fn the_hires_profile_allows_only_24_bit_and_stops_at_flac_24bit() {
    let r = rig();
    let p = r.lidarr.propose_hires_profile().await.unwrap();
    r.lidarr.apply(&p.id).await.unwrap();
    let prof = r.web.st.lock().profiles.last().cloned().unwrap();
    assert_eq!(prof["name"], super::changes::HIRES_PROFILE);
    assert_eq!(prof["cutoff"], 21);
    assert_eq!(prof["upgradeAllowed"], true);
    let group = &prof["items"][1];
    let allowed: Vec<&str> = group["items"].as_array().unwrap().iter().filter(|i| i["allowed"] == true).map(|i| i["quality"]["name"].as_str().unwrap()).collect();
    assert_eq!(allowed, vec!["FLAC 24bit"]);
    assert_eq!(prof["items"][0]["allowed"], false, "no lossy");
    assert!(r.lidarr.propose_hires_profile().await.is_err(), "only one");
}

#[tokio::test]
async fn a_rematch_needs_renaming_off_and_imports_explicit_tracks_on_the_current_edition() {
    let r = rig();
    let e = r.lidarr.rematch_preview(DIR).await.unwrap_err().to_string();
    assert!(e.contains("Rename Tracks is on"), "{e}");
    r.web.st.lock().naming["renameTracks"] = json!(false);
    let plan = r.lidarr.rematch_preview(DIR).await.unwrap();
    assert_eq!(plan.files.len(), 1);
    assert!(r.web.writes().is_empty());
    assert!(r.web.log.lock().iter().any(|(_, p, _)| p.starts_with("/api/v1/manualimport?") && p.contains("filterExistingFiles=false")));
    r.lidarr.rematch_apply(DIR).await.unwrap();
    let w = r.web.writes();
    assert_eq!(w.len(), 1);
    let body: Value = serde_json::from_str(&w[0].2).unwrap();
    let f = &body["files"][0];
    assert_eq!((f["albumReleaseId"].as_i64(), f["disableReleaseSwitching"].as_bool()), (Some(100), Some(true)));
    assert_eq!(f["trackIds"], json!([502]), "the current release's track, not the proposal's id");
    assert!(r.lidarr.changes()[0].what.starts_with("Re-match First Light"));
}

#[tokio::test]
async fn a_rematch_that_unlinks_a_file_stops_and_says_so() {
    let r = rig();
    {
        let mut st = r.web.st.lock();
        st.naming["renameTracks"] = json!(false);
        st.import_drops = true;
    }
    let e = r.lidarr.rematch_apply(DIR).await.unwrap_err().to_string();
    assert!(e.contains("no longer linked"), "{e}");
    assert!(r.lidarr.changes()[0].what.ends_with("stopped"));
}
