//! Tapping a song the library doesn't have, against the faked Lidarr,
//! MusicBrainz and Plex of the request tests, with a fake download client
//! (the queue) and a host that records what it was asked to play. Nothing
//! real is reached.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::mbpick::tests::RG_SECOND;
use super::quick::NO_COPY;
use super::request_tests::{FakeLidarr, Rig, rig};
use super::tap::{AWAY_NOTE, LibraryTrack, PLEX_LATE_NOTE, PLEX_WAIT, STALLED_AFTER, STALLED_NOTE, Tap, TapHost, TapInput, TapPhase, Taps};
use crate::error::AppResult;
use crate::web::Method;

#[derive(Default)]
struct Host {
    files: Mutex<Vec<PathBuf>>,
    library: Mutex<Vec<(String, bool)>>,
    refreshed: Mutex<usize>,
    flagged: Mutex<Vec<String>>,
    found: Mutex<Vec<String>>,
    seen: Mutex<Vec<(TapPhase, Option<u8>)>>,
}

#[async_trait]
impl TapHost for Host {
    fn changed(&self, tap: &Tap) {
        self.seen.lock().push((tap.phase, tap.progress));
    }
    async fn play_file(&self, _tap: &Tap, file: &Path) -> AppResult<()> {
        self.files.lock().push(file.to_path_buf());
        Ok(())
    }
    async fn play_library(&self, _tap: &Tap, track: &LibraryTrack, from_download: bool) -> AppResult<bool> {
        self.library.lock().push((track.track_id.clone(), from_download));
        Ok(true)
    }
    fn refresh_library(&self) {
        *self.refreshed.lock() += 1;
    }
    fn flag_missing(&self, tap: &Tap) {
        self.flagged.lock().push(tap.title.clone());
    }
    fn found(&self, tap: &Tap) {
        self.found.lock().push(tap.title.clone());
    }
}

struct T {
    r: Rig,
    host: Arc<Host>,
    taps: Taps,
    dl: tempfile::TempDir,
}

fn release(guid: &str, protocol: &str, seeders: Option<i64>) -> Value {
    json!({"guid": guid, "indexerId": 3, "title": format!("Northlight - Second Wind {guid}"), "protocol": protocol, "seeders": seeders,
        "quality": {"quality": {"id": 6, "name": "FLAC"}}, "approved": true, "rejected": false, "rejections": [], "downloadAllowed": true})
}

/// A rig whose first request ever was confirmed already, with two copies
/// on the indexers (a thin torrent first, usenet second).
async fn setup() -> T {
    let r = rig();
    // The first request ever was typed once, some time ago; Lidarr doesn't have the artist yet.
    r.db.write(|c| crate::db::store::kv_set(c, "lidarr.typedConfirmed", "1")).unwrap();
    r.web.st.lock().releases = vec![release("t-1", "torrent", Some(2)), release("nzb-1", "usenet", None)];
    let host = Arc::new(Host::default());
    let taps = Taps::new(r.lidarr.clone(), host.clone());
    T { r, host, taps, dl: tempfile::tempdir().unwrap() }
}

fn tap_in() -> TapInput {
    TapInput {
        title: "Dawn".into(),
        artist: "Northlight".into(),
        artist_id: Some("ar1".into()),
        album: Some("Second Wind".into()),
        release_group_id: Some(RG_SECOND.into()),
        duration_ms: Some(201_000),
        ..Default::default()
    }
}

fn album_id(web: &FakeLidarr) -> i64 {
    web.st.lock().albums.iter().find(|a| a["foreignAlbumId"] == RG_SECOND).and_then(|a| a["id"].as_i64()).unwrap()
}

fn writes(web: &FakeLidarr) -> Vec<(Method, String)> {
    web.writes().into_iter().map(|(m, p, _)| (m, p)).collect()
}

fn count(web: &FakeLidarr, method: Method, path: &str) -> usize {
    web.log.lock().iter().filter(|(m, p, _)| *m == method && p.split('?').next() == Some(path)).count()
}

impl T {
    /// The download is in the client's queue, this far along.
    fn downloading(&self, left: f64) {
        let id = album_id(&self.r.web);
        self.r.web.st.lock().queue = vec![json!({"albumId": id, "status": "downloading", "trackedDownloadState": "downloading", "size": 100.0, "sizeleft": left,
            "protocol": "usenet", "outputPath": self.dl.path().join("Second Wind").to_string_lossy()})];
    }

    /// The download finished: its files are in the client's folder, Lidarr hasn't imported yet.
    fn finished(&self) {
        let dir = self.dl.path().join("Second Wind");
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["01 - Rise.flac", "02 - Dawn.flac"] {
            std::fs::write(dir.join(f), b"fLaC").unwrap();
        }
        let id = album_id(&self.r.web);
        self.r.web.st.lock().queue = vec![json!({"albumId": id, "status": "completed", "trackedDownloadState": "importPending", "size": 100.0, "sizeleft": 0.0,
            "protocol": "usenet", "outputPath": dir.to_string_lossy()})];
    }

    /// Lidarr imported it into the artist's folder.
    fn imported(&self) {
        let id = album_id(&self.r.web);
        let mut st = self.r.web.st.lock();
        st.queue.clear();
        st.history = vec![json!({"albumId": id, "eventType": "downloadImported"})];
        if let Some(a) = st.albums.iter_mut().find(|a| a["id"] == id) {
            a["statistics"] = json!({"trackFileCount": 10, "trackCount": 10, "percentOfTracks": 100.0});
        }
        st.track_files = vec![
            json!({"id": 1, "path": "/downloads/lidarr/Northlight/Second Wind (2024)/01 - Rise.flac", "albumId": id}),
            json!({"id": 2, "path": "/downloads/lidarr/Northlight/Second Wind (2024)/02 - Dawn.flac", "albumId": id}),
        ];
    }

    /// Plex scanned it and the library caught up.
    fn in_library(&self) {
        self.r
            .db
            .write(|c| {
                c.execute_batch(&format!(
                    "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, mbid, added_at) VALUES ('a9', 'Second Wind', 'Second Wind', 'ar1', 'Northlight', '{RG_SECOND}', 2);
                     INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, part_key) VALUES
                       ('t91', 'Rise', 'a9', 'Second Wind', 'ar1', 'Northlight', 1, '/library/parts/91/1/file.flac'),
                       ('t92', 'Dawn', 'a9', 'Second Wind', 'ar1', 'Northlight', 2, '/library/parts/92/1/file.flac');"
                ))?;
                Ok(())
            })
            .unwrap();
    }

    fn tap0(&self) -> Tap {
        self.taps.list().into_iter().next().unwrap()
    }
}

/// The timing test: the fastest copy is grabbed straight away (no waiting
/// for Lidarr to read the artist), the song starts as soon as its file is
/// complete, before the import and the scan, then moves to Plex's copy.
#[tokio::test]
async fn a_tapped_song_plays_from_the_finished_download_before_the_import_and_the_scan() {
    let t = setup().await;
    let before = 0;
    let out = t.taps.tap(tap_in()).await.unwrap();
    let tap = out.tap.expect("sent");
    assert_eq!(tap.phase, TapPhase::Downloading, "{:?}", tap.message);
    assert!(tap.timings.found_ms.is_some());
    // Only the album went in, the usenet copy was grabbed, nothing asked Lidarr to search.
    let new: Vec<(Method, String)> = writes(&t.r.web)[before..].to_vec();
    assert_eq!(
        new,
        vec![(Method::Post, "/api/v1/album".into()), (Method::Put, "/api/v1/album/monitor".into()), (Method::Post, "/api/v1/release".into())]
    );
    assert_eq!(t.r.web.st.lock().grabbed, ["nzb-1"]);
    assert_eq!(count(&t.r.web, Method::Get, "/api/v1/release"), 1, "one search of the indexers");
    let add: Value = serde_json::from_str(&t.r.web.writes()[before].2).unwrap();
    assert_eq!(add["addOptions"]["searchForNewAlbum"], false);
    assert_eq!(add["artist"]["addOptions"], json!({"monitor": "none", "searchForMissingAlbums": false}));

    t.downloading(38.0);
    t.taps.follow().await;
    assert_eq!((t.tap0().phase, t.tap0().progress), (TapPhase::Downloading, Some(62)));
    assert!(t.host.files.lock().is_empty());

    t.finished();
    t.taps.follow().await;
    let tap = t.tap0();
    assert_eq!(tap.phase, TapPhase::Playing);
    assert!(tap.from_download);
    assert_eq!(t.host.files.lock().as_slice(), [t.dl.path().join("Second Wind").join("02 - Dawn.flac")]);
    assert!(t.r.plex.scans.lock().is_empty(), "playing before the import and the scan");
    assert!(tap.timings.downloaded_ms.is_some() && tap.timings.playing_ms.is_some());

    t.imported();
    t.taps.follow().await;
    assert_eq!(t.r.plex.scans.lock().as_slice(), ["/downloads/lidarr/Northlight/Second Wind (2024)"], "just that folder");
    assert!(*t.host.refreshed.lock() >= 1);
    assert_eq!(t.tap0().phase, TapPhase::Playing, "still playing from the download");

    t.in_library();
    t.taps.follow().await;
    assert_eq!(t.host.library.lock().as_slice(), [("t92".to_string(), true)], "moved to Plex's copy (at the same spot, the host's job)");
    assert_eq!(t.tap0().phase, TapPhase::Ready);
    let last = t.r.lidarr.last_get().unwrap();
    assert_eq!(last.title, "Dawn");
    assert!(last.timings.found_ms.is_some() && last.timings.downloaded_ms.is_some() && last.timings.playing_ms.is_some());
    // Done: no more reads of Lidarr for it.
    let reads = t.r.web.log.lock().len();
    t.taps.follow().await;
    assert_eq!(t.r.web.log.lock().len(), reads);
}

#[tokio::test]
async fn without_the_file_in_reach_it_plays_the_moment_the_library_has_it() {
    let t = setup().await;
    t.taps.tap(tap_in()).await.unwrap();
    // The client's folder isn't reachable from this device.
    let id = album_id(&t.r.web);
    t.r.web.st.lock().queue = vec![json!({"albumId": id, "status": "completed", "trackedDownloadState": "importPending", "sizeleft": 0.0, "outputPath": "/nowhere/Second Wind"})];
    t.taps.follow().await;
    assert_eq!(t.tap0().phase, TapPhase::Importing);
    assert!(t.host.files.lock().is_empty());
    t.imported();
    t.in_library();
    t.taps.follow().await;
    assert_eq!(t.host.library.lock().as_slice(), [("t92".to_string(), false)]);
    assert_eq!(t.tap0().phase, TapPhase::Ready);
}

#[tokio::test]
async fn play_while_arriving_off_waits_for_the_library() {
    let t = setup().await;
    let mut s = crate::settings::load(&t.r.db);
    s.lidarr_play_arriving = false;
    crate::settings::save(&t.r.db, &s).unwrap();
    t.taps.tap(tap_in()).await.unwrap();
    t.finished();
    t.taps.follow().await;
    assert!(t.host.files.lock().is_empty());
    assert_eq!(t.tap0().phase, TapPhase::Importing);
}

#[tokio::test]
async fn nothing_on_the_indexers_says_so_once_and_keeps_it_on_the_list() {
    let t = setup().await;
    t.r.web.st.lock().releases.clear();
    let tap = t.taps.tap(tap_in()).await.unwrap().tap.unwrap();
    assert_eq!(tap.phase, TapPhase::Waiting);
    assert_eq!(tap.message.as_deref(), Some(NO_COPY));
    assert!(t.r.web.st.lock().grabbed.is_empty());
    let rec = t.r.lidarr.requests().into_iter().find(|r| r.release_group_id == RG_SECOND).expect("stays on the list");
    assert!(rec.next_search_at.is_some(), "searched again on the usual schedule");
    t.taps.follow().await;
    t.taps.follow().await;
    assert_eq!(count(&t.r.web, Method::Get, "/api/v1/release"), 1, "no searching in the background beyond the schedule");
    assert_eq!(t.tap0().phase, TapPhase::Waiting);
}

#[tokio::test]
async fn copies_that_are_incomplete_or_unofficial_are_turned_down_with_why() {
    let t = setup().await;
    t.r.web.st.lock().releases = vec![
        json!({"guid": "a", "indexerId": 3, "protocol": "usenet", "approved": false, "rejected": true, "rejections": ["Album match is not close enough: 61.2% vs 80%"]}),
        json!({"guid": "b", "indexerId": 3, "protocol": "torrent", "approved": true, "seeders": 0}),
    ];
    let tap = t.taps.tap(tap_in()).await.unwrap().tap.unwrap();
    assert_eq!(tap.phase, TapPhase::Waiting);
    let m = tap.message.unwrap();
    assert!(m.contains("turned down") && m.contains("not close enough") && m.contains(NO_COPY), "{m}");
    assert!(t.r.web.st.lock().grabbed.is_empty());
}

#[tokio::test]
async fn a_download_client_that_refuses_fails_the_tap_with_why() {
    let t = setup().await;
    t.r.web.st.lock().refuse_grab = true;
    let tap = t.taps.tap(tap_in()).await.unwrap().tap.unwrap();
    assert_eq!(tap.phase, TapPhase::Failed);
    assert!(tap.message.unwrap().contains("download client"));
}

#[tokio::test]
async fn a_second_tap_on_the_same_song_while_its_coming_joins_the_first() {
    let t = setup().await;
    let a = t.taps.tap(tap_in()).await.unwrap().tap.unwrap();
    let b = t.taps.tap(tap_in()).await.unwrap().tap.unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(t.r.web.st.lock().grabbed.len(), 1, "no second grab");
    assert_eq!(count(&t.r.web, Method::Get, "/api/v1/release"), 1);
    assert_eq!(t.taps.list().len(), 1);
}

#[tokio::test]
async fn only_the_latest_tap_plays_when_it_lands() {
    let t = setup().await;
    t.taps.tap(tap_in()).await.unwrap();
    // Another song tapped since (already in the library here): the first one
    // still comes in, but doesn't take over.
    t.taps.list();
    {
        let other = TapInput { title: "Rise".into(), ..tap_in() };
        t.taps.tap(other).await.unwrap();
    }
    let first = t.taps.list().into_iter().find(|x| x.title == "Dawn").unwrap();
    assert!(!first.wanted);
    t.finished();
    t.taps.follow().await;
    let files = t.host.files.lock().clone();
    assert_eq!(files, [t.dl.path().join("Second Wind").join("01 - Rise.flac")], "only the newest tap plays");
}

#[tokio::test(start_paused = true)]
async fn a_stalled_download_is_shown_and_followed_on() {
    let t = setup().await;
    t.taps.tap(tap_in()).await.unwrap();
    t.downloading(60.0);
    t.taps.follow().await;
    assert_eq!(t.tap0().message, None);
    tokio::time::advance(STALLED_AFTER + Duration::from_secs(1)).await;
    t.taps.follow().await;
    assert_eq!(t.tap0().message.as_deref(), Some(STALLED_NOTE));
    assert_eq!(t.tap0().phase, TapPhase::Downloading, "keeps trying");
    t.downloading(20.0);
    t.taps.follow().await;
    assert_eq!(t.tap0().message, None, "moving again");
    assert_eq!(t.tap0().progress, Some(80));
}

#[tokio::test(start_paused = true)]
async fn plex_never_finding_it_keeps_the_download_playing_and_flags_it() {
    let t = setup().await;
    t.taps.tap(tap_in()).await.unwrap();
    t.finished();
    t.taps.follow().await;
    t.imported();
    t.taps.follow().await;
    tokio::time::advance(PLEX_WAIT + Duration::from_secs(1)).await;
    t.taps.follow().await;
    let tap = t.tap0();
    assert_eq!(tap.phase, TapPhase::Playing);
    assert!(tap.from_download);
    assert_eq!(tap.message.as_deref(), Some(PLEX_LATE_NOTE));
    assert_eq!(t.host.flagged.lock().as_slice(), ["Dawn"]);
    t.taps.follow().await;
    assert_eq!(t.host.flagged.lock().len(), 1, "flagged once");
    assert!(t.host.found.lock().is_empty());
    // Plex shows it after all: the flag clears.
    t.in_library();
    t.taps.follow().await;
    assert_eq!(t.host.found.lock().as_slice(), ["Dawn"]);
}

#[tokio::test]
async fn lidarr_out_of_reach_keeps_the_tap_waiting_and_says_so() {
    let t = setup().await;
    t.r.web.st.lock().down = true;
    let tap = t.taps.tap(tap_in()).await.unwrap().tap.unwrap();
    assert_eq!(tap.phase, TapPhase::Searching);
    assert_eq!(tap.message.as_deref(), Some(AWAY_NOTE));
    t.taps.follow().await;
    assert_eq!(t.tap0().message.as_deref(), Some(AWAY_NOTE));
    // Back: the waiting request goes out (full check first) and the tap follows it.
    t.r.web.st.lock().down = false;
    t.taps.follow().await;
    assert!(t.r.web.writes().iter().any(|(m, p, _)| *m == Method::Post && (p == "/api/v1/album" || p == "/api/v1/artist")));
}

#[tokio::test]
async fn lidarr_going_down_mid_download_keeps_the_tap() {
    let t = setup().await;
    t.taps.tap(tap_in()).await.unwrap();
    t.downloading(50.0);
    t.taps.follow().await;
    t.r.web.st.lock().down = true;
    t.taps.follow().await;
    assert_eq!(t.tap0().phase, TapPhase::Downloading);
    assert_eq!(t.tap0().message.as_deref(), Some(AWAY_NOTE));
}

#[tokio::test]
async fn the_setting_off_or_no_lidarr_says_where_to_turn_it_on() {
    let t = setup().await;
    let mut s = crate::settings::load(&t.r.db);
    s.lidarr_tap_to_get = false;
    crate::settings::save(&t.r.db, &s).unwrap();
    let e = t.taps.tap(tap_in()).await.unwrap_err();
    assert!(e.message().contains("Settings > Lidarr > More"), "{e}");
    assert!(t.r.web.st.lock().grabbed.is_empty());
}

#[tokio::test]
async fn the_first_request_ever_still_needs_its_name_typed() {
    let r = rig();
    let host = Arc::new(Host::default());
    let taps = Taps::new(r.lidarr.clone(), host);
    let out = taps.tap(tap_in()).await.unwrap();
    assert_eq!(out.needs.unwrap().kind, "firstTime");
    assert!(r.web.writes().is_empty());
}

#[tokio::test]
async fn the_preferred_fastest_setting_off_takes_lidarrs_own_first_choice() {
    let t = setup().await;
    let mut s = crate::settings::load(&t.r.db);
    s.lidarr_prefer_fastest = false;
    crate::settings::save(&t.r.db, &s).unwrap();
    t.taps.tap(tap_in()).await.unwrap();
    assert_eq!(t.r.web.st.lock().grabbed, ["t-1"]);
}

#[tokio::test]
async fn an_album_lidarr_already_lists_is_only_monitored_then_grabbed() {
    let t = setup().await;
    // Lidarr has the artist with all its albums (none monitored).
    t.r.lidarr.request(super::request_tests::input(super::mbpick::tests::RG_EP, "Small Hours", None)).await.unwrap();
    let before = t.r.web.writes().len();
    t.taps.tap(tap_in()).await.unwrap();
    assert_eq!(writes(&t.r.web)[before..].to_vec(), vec![(Method::Put, "/api/v1/album/monitor".into()), (Method::Post, "/api/v1/release".into())]);
    assert_eq!(t.r.web.st.lock().grabbed, ["nzb-1"]);
}

#[tokio::test]
async fn someone_who_doesnt_own_the_server_cant_get_a_song() {
    let t = setup().await;
    t.r.lidarr.set_owner(false);
    let e = t.taps.tap(tap_in()).await.unwrap_err();
    assert_eq!(e.to_string(), format!("invalid: {}", crate::lidarr::NOT_OWNER));
    assert!(t.taps.list().is_empty(), "no tap is followed");
    assert!(t.r.web.writes().is_empty(), "nothing goes to Lidarr");
}
