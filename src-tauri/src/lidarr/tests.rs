//! The setup check against faked Plex libraries and a faked Lidarr.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::api::{
    Artist, FsEntry, Listing, MediaManagementConfig, MetadataProviderConfig, NamingConfig, QualityProfile, RootFolder,
    UnmappedFolder,
};
use super::check::{CheckContext, CheckReport, ExclusionTarget, FixAction, Level, LidarrSnapshot, PlexFiles, PlexIndex, ProtectedRef, evaluate};
use super::*;
use crate::artistinfo::http::HttpResponse;
use crate::db::DbHandle;
use crate::plex::keychain::MemoryStore;
use crate::web::{Method, WebRequest};

// ------------------------------------------------------------------ fixtures

/// A Plex library: `files` are paths relative to `location`, with sizes.
fn plex(location: &str, files: &[(&str, i64)]) -> PlexFiles {
    let sep = if location.contains('\\') { "\\" } else { "/" };
    PlexFiles {
        locations: vec![location.to_string()],
        files: files
            .iter()
            .map(|(rel, size)| PlexFile { path: format!("{}{sep}{}", location.trim_end_matches(['/', '\\']), rel.replace('/', sep)), size: Some(*size) })
            .collect(),
    }
}

/// Adam-like: one location, a few artists with albums.
fn library(location: &str) -> PlexFiles {
    plex(
        location,
        &[
            ("Northlight/First Light/01 Dawn.flac", 31_000_111),
            ("Northlight/First Light/02 Noon.flac", 28_400_222),
            ("Harbor Sound/Tides (Deluxe)/01 Ebb.mp3", 9_100_333),
            ("Quiet Hours/Unreleased Demos/01 Sketch.flac", 22_000_444),
            ("Glasshouse/Panes/01 Pane.m4a", 7_700_555),
        ],
    )
}

fn root(path: &str, unmapped: &[&str]) -> RootFolder {
    let sep = if path.contains('\\') { "\\" } else { "/" };
    RootFolder {
        id: 1,
        name: None,
        path: path.to_string(),
        accessible: true,
        unmapped_folders: unmapped
            .iter()
            .map(|n| UnmappedFolder { name: n.to_string(), path: format!("{}{sep}{n}", path.trim_end_matches(['/', '\\'])) })
            .collect(),
        ..Default::default()
    }
}

/// A Lidarr with safe settings and the given root folders.
fn lidarr(roots: Vec<RootFolder>) -> LidarrSnapshot {
    LidarrSnapshot {
        version: "2.8.2.4493".into(),
        roots,
        artists: vec![],
        naming: NamingConfig { rename_tracks: false },
        metadata: MetadataProviderConfig { write_audio_tags: "no".into(), scrub_audio_tags: false },
        media: MediaManagementConfig { delete_empty_folders: false, recycle_bin: "/recycle".into() },
        profiles: vec![QualityProfile { id: 1, name: "Lossless".into(), upgrade_allowed: false, ..Default::default() }],
        lists: vec![],
        listings: HashMap::new(),
        exclusions: vec![],
        unmapped: vec![],
        notifications: vec![],
    }
}

fn entry(name: &str, size: Option<i64>) -> FsEntry {
    FsEntry { name: name.into(), path: String::new(), size }
}

fn listing(dirs: &[&str], files: &[(&str, i64)]) -> Option<Listing> {
    Some(Listing {
        directories: dirs.iter().map(|d| entry(d, None)).collect(),
        files: files.iter().map(|(n, s)| entry(n, Some(*s))).collect(),
    })
}

fn ctx() -> CheckContext {
    CheckContext { confirmed_roots: vec![], unencrypted: vec![], route: Some(api::Route::Home), now: 1_800_000_000, staging: None, library_has_unreleased: false, ..Default::default() }
}

/// Run the check like the service does: keep answering the folders it
/// asks for from `disk` (Lidarr's view, keyed by folder path with a
/// trailing separator) until it asks for nothing new.
fn run(p: &PlexFiles, mut l: LidarrSnapshot, c: &CheckContext, disk: &HashMap<String, Option<Listing>>) -> CheckReport {
    let ix = PlexIndex::new(p);
    for _ in 0..8 {
        let (_, wanted) = evaluate(&ix, &l, c);
        let wanted = check::dedup_wanted(wanted, &l.listings);
        if wanted.is_empty() {
            break;
        }
        for w in wanted {
            let got = disk.get(&w).cloned().unwrap_or(None);
            l.listings.insert(w, got);
        }
    }
    evaluate(&ix, &l, c).0
}

fn ids(r: &CheckReport, level: Level) -> Vec<String> {
    r.findings.iter().filter(|f| f.level == level).map(|f| f.id.clone()).collect()
}

fn no_disk() -> HashMap<String, Option<Listing>> {
    HashMap::new()
}

/// What Lidarr sees when its folder `root` is the library folder.
fn same_disk(root: &str) -> HashMap<String, Option<Listing>> {
    let sep = if root.contains('\\') { "\\" } else { "/" };
    let r = root.trim_end_matches(['/', '\\']);
    let d = |rel: &str| format!("{r}{sep}{}{sep}", rel.replace('/', sep));
    HashMap::from([
        (d("Northlight"), listing(&["First Light"], &[])),
        (d("Northlight/First Light"), listing(&[], &[("01 Dawn.flac", 31_000_111), ("02 Noon.flac", 28_400_222)])),
        (d("Harbor Sound"), listing(&["Tides (Deluxe)"], &[])),
        (d("Harbor Sound/Tides (Deluxe)"), listing(&[], &[("01 Ebb.mp3", 9_100_333)])),
        (d("Quiet Hours"), listing(&["Unreleased Demos"], &[])),
        (d("Quiet Hours/Unreleased Demos"), listing(&[], &[("01 Sketch.flac", 22_000_444)])),
        (d("Glasshouse"), listing(&["Panes"], &[])),
        (d("Glasshouse/Panes"), listing(&[], &[("01 Pane.m4a", 7_700_555)])),
    ])
}

const ARTISTS: [&str; 4] = ["Northlight", "Harbor Sound", "Quiet Hours", "Glasshouse"];

// ------------------------------------------------------------------ root folders

const QH_MBID: &str = "b0000000-0000-0000-0000-000000000002";

/// A protected artist: its library folder and, when Lidarr has it, Lidarr's path.
fn prot(name: &str, folder: &str, lidarr_path: Option<&str>) -> ProtectedRef {
    ProtectedRef {
        name: name.into(),
        mbid: Some(QH_MBID.into()),
        folder: Some(folder.into()),
        lidarr_id: lidarr_path.map(|_| 7),
        lidarr_path: lidarr_path.map(str::to_string),
    }
}

fn with_protected(staging: Option<&str>, p: Vec<ProtectedRef>) -> CheckContext {
    CheckContext { staging: staging.map(str::to_string), protected: p, plex_auto_scan: Some(true), ..ctx() }
}

fn artist(name: &str, path: &str) -> Artist {
    Artist { id: 1, artist_name: name.into(), path: path.into(), ..Default::default() }
}

fn excluded(l: &mut LidarrSnapshot) {
    l.exclusions = vec![super::api::Exclusion { foreign_id: QH_MBID.into(), artist_name: "Quiet Hours".into() }];
}

/// The measured layout: Plex reads /data/media/Music, which holds Lidarr's
/// root /data/media/Music/Lidarr (Managed artists) and a protected folder
/// /data/media/Music/Archive beside it; staging /data/staging/music is
/// outside every Plex folder.
fn green() -> (PlexFiles, LidarrSnapshot, CheckContext) {
    let p = plex(
        "/data/media/Music",
        &[
            ("Lidarr/Northlight/First Light (2019)/1-01 - Dawn.flac", 31_000_111),
            ("Archive/Quiet Hours/Sessions/01 Low.flac", 22_000_444),
        ],
    );
    let mut l = lidarr(vec![root("/data/media/Music/Lidarr", &[]), RootFolder { id: 2, ..root("/data/staging/music", &[]) }]);
    l.artists = vec![
        Artist { id: 1, quality_profile_id: Some(1), ..artist("Northlight", "/data/media/Music/Lidarr/Northlight") },
        Artist { id: 7, ..artist("Quiet Hours", "/data/staging/music/Quiet Hours") },
    ];
    excluded(&mut l);
    let c = with_protected(Some("/data/staging/music"), vec![prot("Quiet Hours", "/data/media/Music/Archive/Quiet Hours", Some("/data/staging/music/Quiet Hours"))]);
    (p, l, c)
}

#[test]
fn the_measured_layout_is_green() {
    let (p, l, c) = green();
    let r = run(&p, l, &c, &no_disk());
    assert!(r.passed && r.managed_ok, "{r:?}");
    assert!(ids(&r, Level::Red).is_empty() && ids(&r, Level::Amber).is_empty(), "{r:?}");
    assert_eq!(r.protected_count, 1);
    assert!(ids(&r, Level::Ok).contains(&"plexAutoScan".to_string()));
}

#[test]
fn a_protected_artist_in_lidarr_with_a_library_path_is_red_for_protected_work_only() {
    let (p, mut l, mut c) = green();
    c.protected[0].lidarr_path = Some("/data/media/Music/Lidarr/Quiet Hours".into());
    l.artists[1].path = "/data/media/Music/Lidarr/Quiet Hours".into();
    let r = run(&p, l, &c, &no_disk());
    assert_eq!(ids(&r, Level::Red), vec!["protectedInPlex"]);
    let f = &r.findings[0];
    assert!(f.protected_only && f.detail.contains("Quiet Hours"), "{f:?}");
    assert!(!r.passed, "protected work stops");
    assert!(r.managed_ok, "Managed artists keep working");
}

#[test]
fn a_protected_folder_under_a_lidarr_root_is_red_only_while_lidarr_has_the_artist() {
    let (p, mut l, mut c) = green();
    c.protected[0].folder = Some("/data/media/Music/Lidarr/Quiet Hours".into());
    let r = run(&p, l.clone(), &c, &no_disk());
    assert_eq!(ids(&r, Level::Red), vec!["protectedUnderRoot"]);
    let f = &r.findings[0];
    assert!(f.fix.as_deref().unwrap().contains("Platter never moves it"), "{f:?}");
    assert!(f.fix.as_deref().unwrap().contains("Plex music library still scans"));
    // Lidarr doesn't have the artist: nothing for Lidarr to match.
    c.protected[0].lidarr_path = None;
    c.protected[0].lidarr_id = None;
    l.artists.truncate(1);
    let r = run(&p, l, &c, &no_disk());
    assert!(r.passed, "{r:?}");
}

#[test]
fn only_whole_folder_names_are_inside() {
    let (mut p, mut l, mut c) = green();
    p.locations = vec!["/data/media/Music".into()];
    l.roots[0].path = "/data/media/Music".into();
    // "Music-archive" is not inside "Music".
    c.protected[0].folder = Some("/data/media/Music-archive/Quiet Hours".into());
    let r = run(&p, l, &c, &no_disk());
    assert!(!ids(&r, Level::Red).contains(&"protectedUnderRoot".to_string()), "{r:?}");
}

#[test]
fn either_separator_and_any_case_still_count_as_inside() {
    let (p, l, mut c) = green();
    c.protected[0].lidarr_path = Some(r"\DATA\media\music\Lidarr\Quiet Hours".into());
    let r = run(&p, l, &c, &no_disk());
    assert_eq!(ids(&r, Level::Red), vec!["protectedInPlex"]);
}

#[test]
fn writing_tags_is_red_only_with_protected_artists_under_a_root() {
    let (p, mut l, mut c) = green();
    l.metadata.write_audio_tags = "newFiles".into();
    let r = run(&p, l.clone(), &c, &no_disk());
    assert!(r.passed, "no protected files under a Lidarr root: {r:?}");
    let f = r.findings.iter().find(|f| f.id == "writeAudioTags").unwrap();
    assert_eq!(f.level, Level::Ok);
    assert_eq!(f.action, Some(FixAction::WriteTagsOff));
    // A protected folder under the Lidarr root (artist not in Lidarr).
    c.protected[0].folder = Some("/data/media/Music/Lidarr/Quiet Hours".into());
    c.protected[0].lidarr_path = None;
    c.protected[0].lidarr_id = None;
    let r = run(&p, l, &c, &no_disk());
    assert_eq!(ids(&r, Level::Red), vec!["writeAudioTags"]);
}

#[test]
fn the_staging_folder_must_exist_and_stay_out_of_plex_while_artists_are_protected() {
    let (p, l, mut c) = green();
    c.staging = None;
    let r = run(&p, l.clone(), &c, &no_disk());
    assert_eq!(ids(&r, Level::Red), vec!["stagingMissing"]);
    assert!(r.managed_ok);
    // Nobody protected: no staging folder is fine.
    let r = run(&p, l.clone(), &CheckContext { protected: vec![], ..c.clone() }, &no_disk());
    assert!(r.passed, "{r:?}");
    // Inside a Plex folder.
    let mut l2 = l.clone();
    l2.roots.push(RootFolder { id: 3, ..root("/data/media/Music/Staging", &[]) });
    let r = run(&p, l2, &CheckContext { staging: Some("/data/media/Music/Staging".into()), ..c.clone() }, &no_disk());
    assert!(ids(&r, Level::Red).contains(&"stagingScanned".to_string()), "{r:?}");
    // Not a Lidarr root: offered as a fix.
    let r = run(&p, l, &CheckContext { staging: Some("/data/staging/other".into()), ..c }, &no_disk());
    let f = r.findings.iter().find(|f| f.id == "stagingNotRoot").unwrap();
    assert_eq!(f.action, Some(FixAction::AddStagingRoot { path: "/data/staging/other".into() }));
}

#[test]
fn a_staging_folder_typed_as_a_mapped_share_is_tested_as_the_server_path() {
    let (p, mut l, mut c) = green();
    c.share = vec![("/data/media/Music".into(), r"M:\Music".into())];
    c.staging = Some(r"M:\Music\Staging".into());
    l.roots.push(RootFolder { id: 3, ..root("/data/media/Music/Staging", &[]) });
    let r = run(&p, l, &c, &no_disk());
    assert!(ids(&r, Level::Red).contains(&"stagingScanned".to_string()), "{r:?}");
    assert!(!ids(&r, Level::Red).contains(&"stagingNotRoot".to_string()), "mapped to the root's own path");
}

#[test]
fn a_normal_library_stays_calm() {
    let (p, mut l, mut c) = green();
    c.protected.clear();
    l.exclusions.clear();
    l.media.recycle_bin = String::new();
    l.naming.rename_tracks = true;
    c.plex_auto_scan = Some(false);
    l.notifications = vec![super::api::Notification { id: 1, name: "Plex".into(), implementation: "PlexServer".into() }];
    let r = run(&p, l.clone(), &c, &no_disk());
    assert!(ids(&r, Level::Amber).is_empty(), "info lines, not amber: {r:?}");
    assert!(ids(&r, Level::Ok).contains(&"recycleBin".to_string()));
    // Without Lidarr's Plex connection, Plex not scanning on its own is worth knowing.
    l.notifications.clear();
    let r = run(&p, l.clone(), &c, &no_disk());
    assert_eq!(ids(&r, Level::Amber), vec!["plexAutoScan"]);
    // The same settings with a protected artist are amber.
    let (_, _, pc) = green();
    let mut l3 = l;
    excluded(&mut l3);
    let r = run(&p, l3, &CheckContext { plex_auto_scan: Some(true), ..pc }, &no_disk());
    assert_eq!(ids(&r, Level::Amber), vec!["recycleBin", "renameTracks"]);
    assert_eq!(r.findings.iter().find(|f| f.id == "recycleBin").unwrap().action, Some(FixAction::SetRecycleBin));
}

#[test]
fn protected_artists_off_the_exclusion_list_are_offered_as_a_fix() {
    let (p, mut l, c) = green();
    l.exclusions.clear();
    let r = run(&p, l, &c, &no_disk());
    assert_eq!(ids(&r, Level::Amber), vec!["protectedNotExcluded"]);
    let f = r.findings.iter().find(|f| f.id == "protectedNotExcluded").unwrap();
    assert_eq!(f.action, Some(FixAction::AddExclusions { artists: vec![ExclusionTarget { name: "Quiet Hours".into(), mbid: QH_MBID.into() }] }));
    assert!(r.passed, "a warning, never a block");
}

#[test]
fn folders_lidarr_and_plex_name_differently_stop_protected_work() {
    // Plex sees /data/media/music, Lidarr sees the same disk as /music.
    let p = library("/data/media/music");
    let l = lidarr(vec![root("/music", &ARTISTS)]);
    let c = with_protected(None, vec![]);
    let r = run(&p, l.clone(), &c, &same_disk("/music"));
    assert!(r.passed, "nothing protected, nothing to stop: {r:?}");
    let c = with_protected(Some("/staging"), vec![prot("Quiet Hours", "/data/media/music/Quiet Hours", None)]);
    let mut l2 = l;
    l2.roots.push(RootFolder { id: 2, ..root("/staging", &[]) });
    excluded(&mut l2);
    let r = run(&p, l2, &c, &same_disk("/music"));
    assert_eq!(ids(&r, Level::Red), vec!["pathsDiffer"]);
    assert!(r.findings[0].detail.contains("same sizes"), "{}", r.findings[0].detail);
    assert!(r.managed_ok);
}

#[test]
fn unmatched_files_under_managed_artists_are_offered_only_with_renaming_off() {
    let (p, mut l, c) = green();
    l.unmapped = vec![super::api::TrackFile { id: 3, path: "/data/media/Music/Lidarr/Northlight/Second Wind/01 Rise.flac".into(), size: Some(5) }];
    let r = run(&p, l.clone(), &c, &no_disk());
    let f = r.findings.iter().find(|f| f.id == "unmappedManaged").unwrap();
    assert_eq!(f.level, Level::Ok);
    assert_eq!(f.action, Some(FixAction::Rematch { folders: vec!["/data/media/Music/Lidarr/Northlight/Second Wind".into()] }));
    l.naming.rename_tracks = true;
    let r = run(&p, l, &c, &no_disk());
    let f = r.findings.iter().find(|f| f.id == "unmappedManaged").unwrap();
    assert_eq!(f.action, None);
    assert!(f.detail.contains("doesn't offer it"), "{}", f.detail);
}

#[test]
fn managed_profiles_are_explained() {
    let (p, mut l, c) = green();
    l.profiles = vec![QualityProfile { id: 1, name: "Lossless".into(), upgrade_allowed: true, cutoff: Some(99), ..Default::default() }];
    let r = run(&p, l.clone(), &c, &no_disk());
    let f = r.findings.iter().find(|f| f.id == "managedProfile").unwrap();
    assert_eq!(f.level, Level::Amber);
    assert!(f.title.contains("can't reach its cutoff"));
    l.profiles[0].upgrade_allowed = false;
    let r = run(&p, l, &c, &no_disk());
    let f = r.findings.iter().find(|f| f.id == "managedProfile").unwrap();
    assert_eq!(f.level, Level::Ok);
}

#[test]
fn no_root_folder_stops_everything() {
    let (p, mut l, c) = green();
    l.roots.clear();
    let r = run(&p, l, &c, &no_disk());
    assert!(!r.managed_ok);
    assert_eq!(r.findings[0].id, "noRoots");
}

#[test]
fn without_plex_files_protected_work_waits() {
    let (_, l, c) = green();
    let r = run(&PlexFiles::default(), l, &c, &no_disk());
    assert!(!r.passed);
    assert!(r.managed_ok);
    assert!(ids(&r, Level::Red).contains(&"plexUnknown".to_string()));
}

#[test]
fn library_unreleased_comes_from_the_tags() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("x.db")).unwrap();
    assert!(!db.read(library_has_unreleased).unwrap());
    db.write(|c| {
        c.execute_batch("INSERT INTO album_tags (album_id, version) VALUES ('b', 'leak');")?;
        Ok(())
    })
    .unwrap();
    assert!(db.read(library_has_unreleased).unwrap());
}

#[test]
fn only_plain_http_to_a_public_host_is_a_warning() {
    let (p, l, mut c) = green();
    c.unencrypted = vec!["http://203.0.113.9:8686".into()];
    let r = run(&p, l, &c, &no_disk());
    assert_eq!(ids(&r, Level::Amber), vec!["plainHttp"]);
    assert!(r.passed, "never a block");
    assert!(unencrypted("http://203.0.113.9:8686"));
    assert!(unencrypted("http://lidarr.example.com"));
    assert!(!unencrypted("https://lidarr.example.com"), "a public https address is fine");
    assert!(!unencrypted("http://10.20.30.40:8686"));
    assert!(!unencrypted("http://nas:8686"));
}

#[test]
fn red_comes_first() {
    let (p, mut l, mut c) = green();
    l.naming.rename_tracks = true;
    c.staging = None;
    let r = run(&p, l, &c, &no_disk());
    assert_eq!(r.findings[0].level, Level::Red);
    assert_eq!(r.findings[1].level, Level::Amber);
}

// ------------------------------------------------------------------ the service, with a faked Lidarr

/// A faked Lidarr: answers the API from JSON and lists folders from `disk`.
/// Logs every request's method and URL, and whether the key was sent.
struct FakeLidarr {
    status: Mutex<Result<(u16, Value), ()>>,
    api: HashMap<&'static str, Value>,
    disk: HashMap<String, Value>,
    log: Mutex<Vec<(Method, String, bool)>>,
    /// Hosts that don't answer at all.
    down: Mutex<Vec<String>>,
    /// Extra header names seen, per request URL.
    extra: Mutex<Vec<(String, Vec<String>)>>,
}

impl FakeLidarr {
    fn new() -> Self {
        let api = HashMap::from([
            ("/api/v1/rootfolder", json!([{ "id": 1, "path": "/music", "accessible": true, "unmappedFolders": [
                {"name": "Northlight", "path": "/music/Northlight"}, {"name": "Glasshouse", "path": "/music/Glasshouse"}
            ]}])),
            ("/api/v1/artist", json!([])),
            ("/api/v1/config/naming", json!({"renameTracks": true})),
            ("/api/v1/config/metadataprovider", json!({"writeAudioTags": "newFiles", "scrubAudioTags": false})),
            ("/api/v1/config/mediamanagement", json!({"deleteEmptyFolders": true, "recycleBin": ""})),
            ("/api/v1/qualityprofile", json!([{"id": 1, "name": "Lossless", "upgradeAllowed": false}])),
            ("/api/v1/importlist", json!([])),
        ]);
        let disk = HashMap::from([
            ("/music/Northlight/".to_string(), json!({"directories": [{"name": "First Light", "path": "/music/Northlight/First Light/"}], "files": []})),
            (
                "/music/Northlight/First Light/".to_string(),
                json!({"directories": [], "files": [{"name": "01 Dawn.flac", "size": 31_000_111}]}),
            ),
        ]);
        Self {
            status: Mutex::new(Ok((200, json!({"appName": "Lidarr", "version": "2.8.2.4493"})))),
            api,
            disk,
            log: Mutex::new(vec![]),
            down: Mutex::new(vec![]),
            extra: Mutex::new(vec![]),
        }
    }
}

#[async_trait]
impl Web for FakeLidarr {
    async fn send(&self, req: WebRequest) -> AppResult<HttpResponse> {
        let has_key = req.headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("x-api-key") && v == "abc123");
        self.log.lock().push((req.method, req.url.clone(), has_key));
        let names = req.headers.iter().map(|(k, _)| k.to_string()).filter(|k| !k.eq_ignore_ascii_case("x-api-key")).collect();
        self.extra.lock().push((req.url.clone(), names));
        let u = url::Url::parse(&req.url).unwrap();
        if self.down.lock().iter().any(|h| u.host_str() == Some(h.as_str())) {
            return Err(AppError::Network("connection refused".into()));
        }
        let reply = |status: u16, v: &Value| Ok(HttpResponse { status, body: Bytes::from(v.to_string()), ..Default::default() });
        if !has_key {
            return reply(401, &json!({}));
        }
        let path = u.path();
        if path.ends_with("/api/v1/system/status") {
            return match &*self.status.lock() {
                Ok((s, v)) => reply(*s, v),
                Err(()) => Err(AppError::Network("connection refused".into())),
            };
        }
        if path.ends_with("/api/v1/filesystem") {
            let p = u.query_pairs().find(|(k, _)| k == "path").map(|(_, v)| v.to_string()).unwrap_or_default();
            return match self.disk.get(&p) {
                Some(v) => reply(200, v),
                None => reply(500, &json!({"message": "not found"})),
            };
        }
        match self.api.iter().find(|(k, _)| path.ends_with(*k)) {
            Some((_, v)) => reply(200, v),
            None => reply(404, &json!({})),
        }
    }
}

struct FakePlex(PlexFiles);

#[async_trait]
impl PlexMusic for FakePlex {
    async fn music_files(&self) -> AppResult<PlexFiles> {
        Ok(self.0.clone())
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    db: DbHandle,
    web: Arc<FakeLidarr>,
    keys: Arc<MemoryStore>,
    lidarr: Lidarr,
}

fn rig(url: &str) -> Rig {
    rig_with(url, "")
}

fn rig_with(url: &str, away: &str) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("l.db")).unwrap();
    let s = crate::settings::Settings { lidarr_url: url.into(), lidarr_away_url: away.into(), ..Default::default() };
    crate::settings::save(&db, &s).unwrap();
    let web = Arc::new(FakeLidarr::new());
    let keys = Arc::new(MemoryStore::new());
    let lidarr = Lidarr::new(LidarrDeps {
        web: web.clone(),
        keys: keys.clone(),
        db: db.clone(),
        plex: Arc::new(FakePlex(library("/data/media/music"))),
        clock: Arc::new(|| 1_800_000_000),
        mb: Arc::new(super::request_tests::NoMb),
        artist_mbid: Arc::new(super::request_tests::NoMbid),
        album_wait: std::time::Duration::ZERO,
        shared: super::shared::tests::on(&db),
        plex_server: Arc::new(super::plexside::fake::FakePlex::default()),
    });
    Rig { _dir: dir, db, web, keys, lidarr }
}

#[tokio::test]
async fn a_whole_check_sends_nothing_but_gets_with_the_key_in_a_header() {
    let r = rig("http://10.20.30.40:8686");
    r.lidarr.set_key(Some("abc123")).unwrap();
    let report = r.lidarr.check().await.unwrap();
    // No protected artists in this library: nothing stops, and the settings are info lines.
    assert!(report.passed && report.managed_ok, "{report:?}");
    assert!(ids(&report, Level::Amber).is_empty(), "{report:?}");
    assert_eq!(report.version, "2.8.2.4493");
    let log = r.web.log.lock().clone();
    assert!(log.len() >= 9, "{log:?}");
    for (method, url, has_key) in &log {
        assert_eq!(*method, Method::Get, "only GETs: {url}");
        assert!(*has_key, "key header on {url}");
        assert!(!url.contains("abc123"), "the key never goes in a URL");
        assert!(url.starts_with("http://10.20.30.40:8686/api/v1/"), "{url}");
    }
    assert!(log.iter().any(|(_, u, _)| u.contains("/filesystem?") && u.contains("includeFiles=true")));
}

#[tokio::test]
async fn the_key_lives_in_the_keychain_not_in_settings() {
    let r = rig("http://nas:8686");
    assert!(!r.lidarr.status().has_key);
    let st = r.lidarr.set_key(Some(" abc123 ")).unwrap();
    assert!(st.has_key && st.url_set);
    assert!(st.unencrypted.is_empty() && !st.away_set);
    assert_eq!(r.keys.get(KEY_ACCOUNT).unwrap().unwrap().expose(), "abc123");
    let stored = r.db.read(|c| crate::db::store::kv_get(c, "app.settings")).unwrap().unwrap();
    assert!(!stored.contains("abc123"));
    assert!(r.lidarr.set_key(Some("not a key!")).is_err());
    assert!(!r.lidarr.set_key(None).unwrap().has_key);
}

#[tokio::test]
async fn unreachable_lidarr() {
    let r = rig("http://10.20.30.40:8686");
    r.lidarr.set_key(Some("abc123")).unwrap();
    *r.web.status.lock() = Err(());
    let e = r.lidarr.check().await.unwrap_err();
    assert_eq!(e.kind(), "network");
    assert!(e.to_string().contains("Lidarr isn't reachable"), "{e}");
}

#[tokio::test]
async fn a_refused_key() {
    let r = rig("http://10.20.30.40:8686");
    r.lidarr.set_key(Some("wrongkey")).unwrap();
    let e = r.lidarr.check().await.unwrap_err();
    assert_eq!(e.kind(), "auth");
    assert!(e.to_string().contains("Lidarr didn't accept this API key"), "{e}");
    assert_eq!(r.web.log.lock().len(), 1, "stops at the first refusal");
}

#[tokio::test]
async fn too_old_or_not_lidarr() {
    let r = rig("http://10.20.30.40:8686");
    r.lidarr.set_key(Some("abc123")).unwrap();
    *r.web.status.lock() = Ok((200, json!({"appName": "Lidarr", "version": "1.4.5.3639"})));
    let e = r.lidarr.check().await.unwrap_err();
    assert!(e.to_string().contains("version 1.4.5.3639") && e.to_string().contains("2.0 or later"), "{e}");
    assert_eq!(r.web.log.lock().len(), 1, "nothing else is read from an old Lidarr");
    *r.web.status.lock() = Ok((200, json!({"appName": "Readarr", "version": "0.4.0"})));
    assert!(r.lidarr.check().await.unwrap_err().to_string().contains("doesn't answer like Lidarr"));
    *r.web.status.lock() = Ok((200, json!("<html>")));
    assert_eq!(r.lidarr.check().await.unwrap_err().kind(), "decode");
}

#[tokio::test]
async fn nothing_is_checked_until_address_and_key_are_set() {
    let r = rig("");
    assert!(r.lidarr.check().await.unwrap_err().to_string().contains("address"));
    let r = rig("http://nas:8686");
    assert!(r.lidarr.check().await.unwrap_err().to_string().contains("API key"));
    assert!(r.web.log.lock().is_empty());
}

#[tokio::test]
async fn confirming_an_empty_root_is_kept_per_address() {
    let r = rig("http://10.20.30.40:8686");
    r.lidarr.confirm_root("/Lidarr/", true).unwrap();
    assert_eq!(r.lidarr.confirmations().get("http://10.20.30.40:8686").unwrap(), &vec!["lidarr".to_string()]);
    r.lidarr.confirm_root("/lidarr", false).unwrap();
    assert!(r.lidarr.confirmations().get("http://10.20.30.40:8686").unwrap().is_empty());
    assert!(r.lidarr.confirm_root("/", true).is_err());
}

#[tokio::test]
async fn away_from_home_the_away_address_answers_with_its_headers() {
    let r = rig_with("http://10.20.30.40:8686", "https://lidarr.example.com");
    r.lidarr.set_key(Some("abc123")).unwrap();
    r.lidarr.set_headers(Some("CF-Access-Client-Id: id.access\nCF-Access-Client-Secret: s3cret")).unwrap();
    r.web.down.lock().push("10.20.30.40".into());
    let report = r.lidarr.check().await.unwrap();
    assert_eq!(report.route, Some(api::Route::Away));
    assert!(report.passed, "the same check on every route: {report:?}");
    assert!(ids(&report, Level::Amber).iter().all(|i| i != "plainHttp"), "a public https address is fine");
    let log = r.web.log.lock().clone();
    assert!(log.iter().all(|(m, _, _)| *m == Method::Get));
    assert_eq!(log.iter().filter(|(_, u, _)| u.contains("10.20.30.40")).count(), 1, "home tried once, first");
    assert!(log[0].1.contains("10.20.30.40"));
    for (url, names) in r.web.extra.lock().iter() {
        if url.contains("lidarr.example.com") {
            assert_eq!(names, &vec!["CF-Access-Client-Id".to_string(), "CF-Access-Client-Secret".to_string()]);
        } else {
            assert!(names.is_empty(), "proxy headers only go to the away address");
        }
    }
    let st = r.lidarr.status();
    assert!(st.away_set);
    assert_eq!(st.header_names, vec!["CF-Access-Client-Id", "CF-Access-Client-Secret"]);
    let stored = r.db.read(|c| crate::db::store::kv_get(c, "app.settings")).unwrap().unwrap();
    assert!(!stored.contains("s3cret"), "headers live in the keychain");
}

#[tokio::test]
async fn at_home_the_away_address_is_never_tried() {
    let r = rig_with("http://10.20.30.40:8686", "http://203.0.113.9:8686");
    r.lidarr.set_key(Some("abc123")).unwrap();
    let report = r.lidarr.check().await.unwrap();
    assert_eq!(report.route, Some(api::Route::Home));
    assert!(r.web.log.lock().iter().all(|(_, u, _)| u.contains("10.20.30.40")));
    // A plain http away address on a public host warns, whichever route answered.
    assert!(ids(&report, Level::Amber).contains(&"plainHttp".to_string()));
    assert_eq!(r.lidarr.status().unencrypted, vec!["http://203.0.113.9:8686"]);
}

#[tokio::test]
async fn neither_address_answers() {
    let r = rig_with("http://10.20.30.40:8686", "https://lidarr.example.com");
    r.lidarr.set_key(Some("abc123")).unwrap();
    r.web.down.lock().extend(["10.20.30.40".to_string(), "lidarr.example.com".to_string()]);
    let e = r.lidarr.check().await.unwrap_err();
    assert!(e.to_string().contains("Lidarr isn't reachable"), "{e}");
    // Home refuses the key while away is down: the refusal is what's shown.
    r.web.down.lock().retain(|h| h != "10.20.30.40");
    r.lidarr.set_key(Some("wrongkey")).unwrap();
    assert_eq!(r.lidarr.check().await.unwrap_err().kind(), "auth");
}

#[test]
fn proxy_headers_are_checked() {
    assert_eq!(parse_headers("A: 1\n\n  B-2 : two words ").unwrap(), vec![("A".into(), "1".into()), ("B-2".into(), "two words".into())]);
    for bad in ["no colon", ": empty", "X-Api-Key: x", "Host: h", "A: 1\na: 2", "Bad Name: x", "A: \u{7}"] {
        assert!(parse_headers(bad).is_err(), "{bad:?}");
    }
    assert!(parse_headers("A:1\nB:1\nC:1\nD:1\nE:1\nF:1").is_err());
    assert!(parse_headers("").unwrap().is_empty());
}
#[test]
fn a_root_folder_inside_another_is_worth_knowing() {
    let (p, mut l, c) = green();
    l.roots.push(RootFolder { id: 3, ..root("/data/media/Music/Lidarr/Managed", &["Other"]) });
    l.artists.push(Artist { id: 9, ..artist("Other", "/data/media/Music/Lidarr/Managed/Other") });
    let r = run(&p, l, &c, &no_disk());
    assert_eq!(ids(&r, Level::Amber), vec!["nestedRoots"]);
    let f = r.findings.iter().find(|f| f.id == "nestedRoots").unwrap();
    assert!(f.detail.contains("/data/media/Music/Lidarr/Managed is inside /data/media/Music/Lidarr and holds artists"), "{}", f.detail);
    assert!(f.detail.contains("Never remove the outer folder"), "{}", f.detail);
    assert!(f.fix.as_deref().unwrap().contains("Platter never removes root folders"));
    assert!(r.passed, "a warning, never a block");
}
