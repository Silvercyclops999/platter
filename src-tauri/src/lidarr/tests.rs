//! The setup check against faked Plex libraries and a faked Lidarr.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::api::{
    Artist, FsEntry, ImportList, Listing, MediaManagementConfig, MetadataProviderConfig, NamingConfig, QualityProfile, RootFolder,
    UnmappedFolder,
};
use super::check::{CheckContext, CheckReport, Level, LidarrSnapshot, PlexFile, PlexFiles, PlexIndex, evaluate};
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
        metadata: MetadataProviderConfig { write_audio_tags: "newFiles".into(), scrub_audio_tags: false },
        media: MediaManagementConfig { delete_empty_folders: false, recycle_bin: "/recycle".into() },
        profiles: vec![QualityProfile { id: 1, name: "Lossless".into(), upgrade_allowed: false }],
        lists: vec![],
        listings: HashMap::new(),
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
    CheckContext { confirmed_roots: vec![], unencrypted: vec![], route: Some(api::Route::Home), now: 1_800_000_000 }
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

#[test]
fn same_path_is_red_even_when_empty() {
    let r = run(&library("/data/music"), lidarr(vec![root("/data/music/", &[])]), &ctx(), &no_disk());
    assert!(!r.passed);
    let f = r.findings.iter().find(|f| f.id == "rootOverlap").unwrap();
    assert_eq!(f.level, Level::Red);
    assert!(f.title.contains("same folder"), "{}", f.title);
    assert!(f.fix.as_deref().unwrap().contains("Root Folders"));
}

#[test]
fn parent_and_child_paths_are_red() {
    let parent = run(&library("/data/music"), lidarr(vec![root("/data", &["music", "movies"])]), &ctx(), &no_disk());
    assert!(parent.findings.iter().any(|f| f.id == "rootOverlap" && f.title.contains("holds")), "{parent:?}");
    let child = run(&library("/data/music"), lidarr(vec![root("/data/music/Lidarr", &[])]), &ctx(), &no_disk());
    assert!(child.findings.iter().any(|f| f.id == "rootOverlap" && f.title.contains("is inside")), "{child:?}");
    assert!(!parent.passed && !child.passed);
}

#[test]
fn case_and_separators_dont_hide_the_same_path() {
    let r = run(&library(r"M:\Music"), lidarr(vec![root("m:/music", &[])]), &ctx(), &no_disk());
    assert_eq!(ids(&r, Level::Red), vec!["rootOverlap"]);
}

#[test]
fn docker_remap_with_different_strings_is_red_by_content() {
    // Plex sees /data/media/music, Lidarr sees the same disk as /music.
    let p = library("/data/media/music");
    let l = lidarr(vec![root("/music", &ARTISTS)]);
    let r = run(&p, l.clone(), &ctx(), &same_disk("/music"));
    let f = r.findings.iter().find(|f| f.id == "rootOverlap").expect("overlap found");
    assert_eq!(f.level, Level::Red);
    assert!(f.title.contains("is your Plex music folder"), "{}", f.title);
    assert!(f.detail.contains("same files, with the same sizes"), "{}", f.detail);
    assert!(!r.passed);
    // Lidarr can't list the folders: the matching names alone keep it red.
    let blind = run(&p, l, &ctx(), &no_disk());
    let f = blind.findings.iter().find(|f| f.id == "rootOverlap").expect("still red without listings");
    assert!(f.detail.contains("same folder names"), "{}", f.detail);
}

#[test]
fn unc_versus_mapped_drive_is_red_by_content() {
    let p = library(r"M:\");
    let l = lidarr(vec![root(r"\\nas\music", &ARTISTS)]);
    let r = run(&p, l, &ctx(), &same_disk(r"\\nas\music"));
    assert_eq!(ids(&r, Level::Red), vec!["rootOverlap"]);
    assert!(r.findings[0].detail.contains("same sizes"));
}

#[test]
fn lidarr_root_inside_the_library_under_another_name_is_red() {
    // Plex /data/music holds a Lidarr folder; Lidarr calls it /downloads.
    let mut p = library("/data/music");
    p.files.push(PlexFile { path: "/data/music/Lidarr/New Band/Debut/01 Hello.flac".into(), size: Some(5_000) });
    let disk = HashMap::from([
        ("/downloads/New Band/".to_string(), listing(&["Debut"], &[])),
        ("/downloads/New Band/Debut/".to_string(), listing(&[], &[("01 Hello.flac", 5_000)])),
    ]);
    let r = run(&p, lidarr(vec![root("/downloads", &["New Band"])]), &ctx(), &disk);
    let f = r.findings.iter().find(|f| f.id == "rootOverlap").unwrap();
    assert!(f.title.contains("inside your Plex music folder"), "{}", f.title);
}

#[test]
fn a_separate_root_with_other_contents_passes() {
    let p = library("/data/music");
    // Shares one artist name with the library, but its files differ.
    let disk = HashMap::from([
        ("/downloads/Northlight/".to_string(), listing(&["Later Album"], &[])),
        ("/downloads/Other Band/".to_string(), listing(&["Record"], &[])),
    ]);
    let r = run(&p, lidarr(vec![root("/downloads", &["Northlight", "Other Band"])]), &ctx(), &disk);
    assert!(r.passed, "{r:?}");
    assert!(ids(&r, Level::Ok).contains(&"rootSeparate".to_string()));
    // Same name, same album folder, different files: still separate.
    let disk = HashMap::from([
        ("/downloads/Northlight/".to_string(), listing(&["First Light"], &[])),
        ("/downloads/Northlight/First Light/".to_string(), listing(&[], &[("01 Dawn.flac", 12)])),
    ]);
    let r = run(&p, lidarr(vec![root("/downloads", &["Northlight"])]), &ctx(), &disk);
    assert!(r.passed, "{r:?}");
}

#[test]
fn an_empty_root_needs_the_user_to_confirm_it() {
    let p = library("/data/media/music");
    let l = lidarr(vec![root("/lidarr", &[])]);
    let r = run(&p, l.clone(), &ctx(), &no_disk());
    assert!(!r.passed, "an empty folder is never green on its own");
    let f = r.findings.iter().find(|f| f.id == "rootEmpty").unwrap();
    assert_eq!(f.level, Level::Red);
    assert_eq!(f.empty_root.as_ref().map(|e| (e.path.as_str(), e.confirmed)), Some(("/lidarr", false)));
    let mut c = ctx();
    c.confirmed_roots = vec![paths::NormPath::parse("/Lidarr/").key()];
    let r = run(&p, l.clone(), &c, &no_disk());
    assert!(r.passed, "{r:?}");
    assert_eq!(r.findings.iter().find(|f| f.id == "rootEmpty").unwrap().level, Level::Ok);
    // Once it fills with the library's folders, the content check wins over the confirmation.
    let mut filled = l;
    filled.roots = vec![root("/lidarr", &ARTISTS)];
    let r = run(&p, filled, &c, &same_disk("/lidarr"));
    assert!(!r.passed);
    assert!(ids(&r, Level::Red).contains(&"rootOverlap".to_string()));
}

#[test]
fn unreadable_roots_and_no_roots_are_red() {
    let mut r0 = root("/lidarr", &[]);
    r0.accessible = false;
    let r = run(&library("/data/music"), lidarr(vec![r0]), &ctx(), &no_disk());
    assert_eq!(ids(&r, Level::Red), vec!["rootUnreadable"]);
    let r = run(&library("/data/music"), lidarr(vec![]), &ctx(), &no_disk());
    assert_eq!(ids(&r, Level::Red), vec!["noRoots"]);
}

#[test]
fn without_plex_files_nothing_can_be_proven() {
    let r = run(&PlexFiles { locations: vec!["/data/music".into()], files: vec![] }, lidarr(vec![root("/lidarr", &["A"])]), &ctx(), &no_disk());
    assert!(!r.passed);
    assert_eq!(ids(&r, Level::Red), vec!["plexUnknown"]);
}

// ------------------------------------------------------------------ artists

fn artist(name: &str, path: &str) -> Artist {
    Artist { id: 1, artist_name: name.into(), path: path.into(), ..Default::default() }
}

#[test]
fn artists_in_the_plex_folder_are_red() {
    let p = library("/data/media/music");
    // Lidarr's root is the library under another name, and it manages one artist there.
    let mut l = lidarr(vec![root("/music", &["Harbor Sound", "Quiet Hours", "Glasshouse"])]);
    l.artists = vec![artist("Northlight", "/music/Northlight")];
    let r = run(&p, l, &ctx(), &same_disk("/music"));
    let f = r.findings.iter().find(|f| f.id == "artistsInside").unwrap();
    assert_eq!(f.level, Level::Red);
    assert!(f.title.contains("an artist"), "{}", f.title);
    assert!(f.detail.contains("Northlight"));
    assert!(f.fix.as_deref().unwrap().contains("unticked"));
}

#[test]
fn artists_by_path_string_and_by_last_two_names_are_red() {
    let p = library("/data/music");
    let mut l = lidarr(vec![root("/lidarr", &["Fresh"])]);
    l.artists = vec![
        // Under the Plex folder by its path.
        artist("Glasshouse", "/data/music/Glasshouse"),
        // Not under any Lidarr root; same last two names as a Plex folder.
        artist("Harbor Sound", "/mnt/music/Harbor Sound"),
    ];
    let r = run(&p, l, &ctx(), &no_disk());
    let f = r.findings.iter().find(|f| f.id == "artistsInside").unwrap();
    assert!(f.title.contains("2 artists"), "{}", f.title);
}

#[test]
fn artists_in_a_proven_separate_root_are_fine() {
    let p = library("/data/music");
    let mut l = lidarr(vec![root("/downloads", &[])]);
    l.artists = vec![artist("Northlight", "/downloads/Northlight")];
    let disk = HashMap::from([("/downloads/Northlight/".to_string(), listing(&["New Album"], &[]))]);
    let r = run(&p, l, &ctx(), &disk);
    assert!(r.passed, "{r:?}");
    assert_eq!(r.findings.iter().find(|f| f.id == "artistsInside").unwrap().level, Level::Ok);
}

// ------------------------------------------------------------------ amber settings

fn separate() -> (PlexFiles, LidarrSnapshot, HashMap<String, Option<Listing>>) {
    let disk = HashMap::from([("/downloads/Someone/".to_string(), listing(&["Album"], &[]))]);
    (library("/data/music"), lidarr(vec![root("/downloads", &["Someone"])]), disk)
}

#[test]
fn safe_settings_give_no_amber() {
    let (p, l, disk) = separate();
    let r = run(&p, l, &ctx(), &disk);
    assert!(r.passed);
    assert!(ids(&r, Level::Amber).is_empty(), "{r:?}");
    assert!(r.findings.iter().all(|f| f.level == Level::Ok));
}

#[test]
fn each_amber_setting_is_found_and_explained() {
    type Tweak = fn(&mut LidarrSnapshot);
    let cases: [(&str, Tweak); 8] = [
        ("renameTracks", |l| l.naming.rename_tracks = true),
        ("writeAudioTags", |l| l.metadata.write_audio_tags = "allFiles".into()),
        ("writeAudioTags", |l| l.metadata.write_audio_tags = "sync".into()),
        ("scrubAudioTags", |l| l.metadata.scrub_audio_tags = true),
        ("deleteEmptyFolders", |l| l.media.delete_empty_folders = true),
        ("recycleBin", |l| l.media.recycle_bin = " ".into()),
        ("upgradeAllowed", |l| l.profiles.push(QualityProfile { id: 2, name: "Any".into(), upgrade_allowed: true })),
        ("importLists", |l| l.lists.push(ImportList { id: 1, name: "Charts".into(), enable_automatic_add: true, enable: None })),
    ];
    for (id, tweak) in cases {
        let (p, mut l, disk) = separate();
        tweak(&mut l);
        let r = run(&p, l, &ctx(), &disk);
        assert_eq!(ids(&r, Level::Amber), vec![id.to_string()], "{id}");
        assert!(r.passed, "amber never blocks: {id}");
        let f = r.findings.iter().find(|f| f.level == Level::Amber).unwrap();
        assert!(f.fix.as_deref().is_some_and(|x| x.starts_with("In Lidarr")), "{id}");
        assert_eq!(r.findings[0].level, Level::Amber, "amber comes before OK");
    }
    // New-files-only tag writing is fine.
    let (p, mut l, disk) = separate();
    l.metadata.write_audio_tags = "newFiles".into();
    assert!(ids(&run(&p, l, &ctx(), &disk), Level::Amber).is_empty());
}

#[test]
fn only_plain_http_to_a_public_host_is_a_warning() {
    let (p, l, disk) = separate();
    let mut c = ctx();
    c.unencrypted = vec!["http://203.0.113.9:8686".into()];
    let r = run(&p, l, &c, &disk);
    assert_eq!(ids(&r, Level::Amber), vec!["plainHttp"]);
    assert!(r.passed, "never a block");
    assert!(r.findings[0].title.contains("unencrypted"));
    assert!(unencrypted("http://203.0.113.9:8686"));
    assert!(unencrypted("http://lidarr.example.com"));
    assert!(!unencrypted("https://lidarr.example.com"), "a public https address is fine");
    assert!(!unencrypted("http://10.20.30.40:8686"));
    assert!(!unencrypted("http://nas:8686"));
}
#[test]
fn red_comes_first() {
    let (p, mut l, _) = separate();
    l.naming.rename_tracks = true;
    l.roots = vec![root("/data/music", &[])];
    let r = run(&p, l, &ctx(), &no_disk());
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
    });
    Rig { _dir: dir, db, web, keys, lidarr }
}

#[tokio::test]
async fn a_whole_check_sends_nothing_but_gets_with_the_key_in_a_header() {
    let r = rig("http://10.20.30.40:8686");
    r.lidarr.set_key(Some("abc123")).unwrap();
    let report = r.lidarr.check().await.unwrap();
    // Docker remap: same files under /music, so it's red, plus two ambers.
    assert_eq!(ids(&report, Level::Red), vec!["rootOverlap"]);
    assert_eq!(ids(&report, Level::Amber), vec!["renameTracks", "deleteEmptyFolders", "recycleBin"]);
    assert!(report.findings[0].detail.contains("same sizes"), "listings were followed: {:?}", report.findings[0]);
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
    assert_eq!(ids(&report, Level::Red), vec!["rootOverlap"], "the same check on every route");
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