use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::*;
use crate::audio::gate::Priority;
use crate::db::fake;
use crate::db::store::{upsert_albums, upsert_tracks};
use crate::model::Album;

const BODY: usize = 96 * 1024;

fn body_of(id: &str) -> Vec<u8> {
    let seed = id.bytes().fold(7u8, |a, b| a.wrapping_mul(31).wrapping_add(b));
    (0..BODY).map(|i| seed.wrapping_add((i % 251) as u8)).collect()
}

/// Requests the file server saw: path and Range header.
type Hits = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// Serves `/library/parts/{id}/1/file.flac` with Range support, and
/// `/transcode/{id}` as a plain 200.
struct Files {
    hits: Hits,
}

impl Respond for Files {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let path = req.url.path().to_string();
        let range = req.headers.get("range").and_then(|v| v.to_str().ok()).map(str::to_owned);
        self.hits.lock().push((path.clone(), range.clone()));
        let id = path.split('/').nth(3).unwrap_or("").to_string();
        let id = if path.starts_with("/transcode/") { path.trim_start_matches("/transcode/").to_string() } else { id };
        if id == "gone" {
            return ResponseTemplate::new(404);
        }
        let body = body_of(&id);
        match range.as_deref().and_then(|r| r.strip_prefix("bytes=")).and_then(|r| r.trim_end_matches('-').parse::<usize>().ok()) {
            Some(from) if from < body.len() && !path.starts_with("/transcode/") => ResponseTemplate::new(206)
                .insert_header("content-range", format!("bytes {from}-{}/{}", body.len() - 1, body.len()).as_str())
                .set_body_bytes(body[from..].to_vec()),
            _ => ResponseTemplate::new(200).set_body_bytes(body),
        }
    }
}

struct FakeSource {
    base: String,
    conn: Mutex<Option<ConnectionKind>>,
    /// Songs on another server: its connection.
    conn_of: Mutex<HashMap<String, Option<ConnectionKind>>>,
    fetched: Mutex<Vec<(String, Option<u32>)>>,
    finished: AtomicUsize,
}

#[async_trait]
impl DownloadSource for FakeSource {
    async fn connection(&self) -> Option<ConnectionKind> {
        *self.conn.lock()
    }
    async fn connection_for(&self, track: &Track) -> Option<ConnectionKind> {
        match self.conn_of.lock().get(&track.id) {
            Some(c) => *c,
            None => *self.conn.lock(),
        }
    }
    async fn fetch(&self, track: &Track, kbps: Option<u32>) -> AppResult<Fetch> {
        self.fetched.lock().push((track.id.clone(), kbps));
        let url = match kbps {
            None => format!("{}{}", self.base, track.part_key),
            Some(k) => format!("{}/transcode/{}?musicBitrate={k}", self.base, track.id),
        };
        Ok(Fetch {
            src: StreamSource { url, ..Default::default() },
            resumable: kbps.is_none(),
            session: kbps.map(|_| "s1".to_string()),
        })
    }
    async fn finished(&self, _fetch: &Fetch) {
        self.finished.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Events {
    statuses: Mutex<Vec<DownloadStatus>>,
    changed: AtomicUsize,
}

impl DownloadEvents for Events {
    fn status(&self, s: &DownloadStatus) {
        self.statuses.lock().push(s.clone());
    }
    fn files_changed(&self) {
        self.changed.fetch_add(1, Ordering::SeqCst);
    }
}

struct Rig {
    d: Downloads,
    db: DbHandle,
    src: Arc<FakeSource>,
    events: Arc<Events>,
    hits: Hits,
    gate: Arc<TransferGate>,
    dir: PathBuf,
    _tmp: tempfile::TempDir,
    _server: MockServer,
}

fn albums() -> (Album, Album, Vec<Track>) {
    let ar = fake::artist("ar", "Artist");
    let a = fake::album("a", "First", &ar, 2001, None, &[]);
    let b = fake::album("b", "Second", &ar, 2002, None, &[]);
    let tracks = vec![
        fake::track("a1", "A one", &a, 1, 1),
        fake::track("a2", "A two", &a, 1, 2),
        fake::track("b1", "B one", &b, 1, 1),
        fake::track("b2", "B two", &b, 1, 2),
    ];
    (a, b, tracks)
}

async fn rig_in(tmp: tempfile::TempDir, free: Option<u64>) -> Rig {
    let server = MockServer::start().await;
    let hits = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("GET"))
        .and(path_regex("^/(library|transcode)/.*"))
        .respond_with(Files { hits: hits.clone() })
        .mount(&server)
        .await;
    let db = DbHandle::open(tmp.path().join("l.db")).unwrap();
    let (a, b, mut tracks) = albums();
    let mut gone = fake::track("gone", "Gone", &a, 2, 1);
    gone.part_key = "/library/parts/gone/1/file.flac".into();
    gone.album_id = "x".into();
    tracks.push(gone);
    db.write(|c| {
        let tx = c.transaction()?;
        upsert_albums(&tx, &[a.clone(), b.clone()], 1)?;
        upsert_tracks(&tx, &tracks, 1)?;
        tx.execute("INSERT OR REPLACE INTO playlists (id, title) VALUES ('p', 'Mix')", [])?;
        tx.execute(
            "INSERT OR REPLACE INTO playlist_items (playlist_id, position, track_id) VALUES ('p', 0, 'b1'), ('p', 1, 'a1')",
            [],
        )?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let src = Arc::new(FakeSource {
        base: server.uri(),
        conn: Mutex::new(Some(ConnectionKind::Local)),
        conn_of: Mutex::new(HashMap::new()),
        fetched: Mutex::new(Vec::new()),
        finished: AtomicUsize::new(0),
    });
    let dir = tmp.path().join("downloads");
    let d = Downloads::new(Deps {
        db: db.clone(),
        dir: dir.clone(),
        source: src.clone(),
        free_space: Arc::new(move |_| free),
        art: None,
    });
    let events = Arc::new(Events::default());
    d.set_events(events.clone());
    let gate = TransferGate::new();
    d.attach(gate.clone(), None, crate::audio::http::http_client());
    Rig { d, db, src, events, hits, gate, dir, _tmp: tmp, _server: server }
}

async fn rig() -> Rig {
    rig_in(tempfile::tempdir().unwrap(), Some(1 << 40)).await
}

fn key(kind: ItemKind, id: &str) -> ItemKey {
    ItemKey::new(kind, id)
}

/// Steps the queue until it has nothing left to do.
async fn drain(d: &Downloads) -> Vec<Step> {
    let mut steps = Vec::new();
    for _ in 0..20 {
        let s = d.step().await.unwrap();
        steps.push(s);
        if s != Step::Worked {
            break;
        }
    }
    steps
}

fn item<'a>(s: &'a DownloadStatus, kind: ItemKind, id: &str) -> &'a ItemStatus {
    s.items.iter().find(|i| i.kind == kind && i.id == id).expect("item")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_fetches_in_the_order_items_were_added() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "b"), "Second".into(), None).await.unwrap();
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    let st = r.d.status().unwrap();
    assert_eq!(item(&st, ItemKind::Album, "a").state, ItemState::Queued);
    assert_eq!(st.queued, 4);
    assert_eq!(drain(&r.d).await.last(), Some(&Step::Idle));
    let order: Vec<String> = r.src.fetched.lock().iter().map(|f| f.0.clone()).collect();
    assert_eq!(order, ["b1", "b2", "a1", "a2"]);
    // Files are named by track id, byte for byte what the server sent.
    assert_eq!(std::fs::read(r.dir.join("a1.flac")).unwrap(), body_of("a1"));
    let st = r.d.status().unwrap();
    let a = item(&st, ItemKind::Album, "a");
    assert_eq!((a.state, a.done, a.total, a.progress), (ItemState::Downloaded, 2, 2, 1.0));
    assert_eq!(st.used_bytes, 4 * BODY as u64);
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2", "b1", "b2"]);
    assert!(r.events.changed.load(Ordering::SeqCst) >= 4);
    // The play path finds them.
    let (_, _, tracks) = albums();
    assert_eq!(r.d.path(&tracks[0]), Some(r.dir.join("a1.flac")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloads_wait_behind_playback_on_the_gate_and_resume_by_range() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    // Pre-cache holds the gate: the download (background) must wait.
    let hold = r.gate.acquire(&Priority::Precache.cell()).await;
    let d = r.d.clone();
    let task = tokio::spawn(async move { d.step().await.unwrap() });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!task.is_finished(), "a download never jumps playback's transfers");
    assert!(r.hits.lock().is_empty());
    drop(hold);
    assert_eq!(task.await.unwrap(), Step::Worked);
    assert_eq!(std::fs::read(r.dir.join("a1.flac")).unwrap(), body_of("a1"));

    // A song that starts playing takes the gate from a download in flight.
    let part = r.dir.join(part_name("a2.flac", "/library/parts/a2/1/file.flac"));
    std::fs::write(&part, &body_of("a2")[..1000]).unwrap();
    let d = r.d.clone();
    let task = tokio::spawn(async move { d.step().await.unwrap() });
    let p = r.gate.acquire(&Priority::Current.cell()).await;
    drop(p);
    assert_eq!(task.await.unwrap(), Step::Worked);
    assert_eq!(std::fs::read(r.dir.join("a2.flac")).unwrap(), body_of("a2"));
    assert!(
        r.hits.lock().iter().any(|(p, range)| p.contains("/a2/") && range.as_deref() == Some("bytes=1000-")),
        "carried on from the partial file"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_survives_a_restart_and_resumes_the_partial_file() {
    let tmp = tempfile::tempdir().unwrap();
    let keep = tmp.path().to_path_buf();
    let r = rig_in(tmp, Some(1 << 40)).await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    // Quit with half of a1 fetched.
    std::fs::write(r.dir.join(part_name("a1.flac", "/library/parts/a1/1/file.flac")), &body_of("a1")[..5000]).unwrap();
    let Rig { _tmp, db, .. } = r;
    drop(db);
    let r2 = rig_in(_tmp, Some(1 << 40)).await;
    assert_eq!(r2.dir, keep.join("downloads"));
    assert_eq!(r2.d.status().unwrap().queued, 2, "the queue is still there");
    drain(&r2.d).await;
    assert_eq!(std::fs::read(r2.dir.join("a1.flac")).unwrap(), body_of("a1"));
    assert!(r2.hits.lock().iter().any(|(p, range)| p.contains("/a1/") && range.as_deref() == Some("bytes=5000-")));
    assert_eq!(item(&r2.d.status().unwrap(), ItemKind::Album, "a").state, ItemState::Downloaded);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp3_quality_asks_the_transcoder_and_stops_its_session() {
    let r = rig().await;
    r.d.set_prefs(Prefs { quality: DownloadQuality::Kbps192, ..Prefs::default() });
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    drain(&r.d).await;
    assert_eq!(r.src.fetched.lock()[0], ("a1".to_string(), Some(192)));
    assert_eq!(r.src.finished.load(Ordering::SeqCst), 2);
    assert!(r.dir.join("a1.mp3").is_file());
    let row = r.db.read(|c| store::file(c, "a1")).unwrap().unwrap();
    assert_eq!((row.quality.as_str(), row.file.as_deref()), ("192", Some("a1.mp3")));
}

#[test]
fn transcode_request_for_a_download_is_shaped_like_playback() {
    // The request the Plex source sends for an MP3 download.
    let dir = tempfile::tempdir().unwrap();
    let id = Arc::new(crate::plex::identity::ClientIdentity::load_or_create(dir.path()).unwrap());
    let token = crate::plex::token::Token::new("tok123456789").unwrap();
    let c = crate::plex::client::PlexClient::new("http://192.0.2.10:32400", token, id, Some(ConnectionKind::Local)).unwrap();
    let req = c.transcode_request("777", 320, "sess-1").unwrap();
    let url = req.url.as_str();
    assert!(url.starts_with("http://192.0.2.10:32400/music/:/transcode/universal/start.mp3?"));
    for want in [
        "path=%2Flibrary%2Fmetadata%2F777",
        "protocol=http",
        "directPlay=0",
        "directStream=0",
        "musicBitrate=320",
        "maxAudioBitrate=320",
        "session=sess-1",
    ] {
        assert!(url.contains(want), "{want} in {url}");
    }
    assert!(!url.contains("tok123456789"), "the token never goes in the URL");
    assert_eq!(req.headers.get("x-plex-session-identifier").unwrap(), "sess-1");
    assert!(req.headers.get("x-plex-token").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playlists_follow_the_library_and_shared_songs_stay() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    r.d.add(key(ItemKind::Playlist, "p"), "Mix".into(), None).await.unwrap();
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2", "b1"]);
    let st = r.d.status().unwrap();
    // a1 is shared, so removing the playlist would only free b1.
    assert_eq!(item(&st, ItemKind::Playlist, "p").bytes, BODY as u64);

    // The playlist gains b2 and loses a1 (still on the album).
    r.db.write(|c| {
        c.execute("DELETE FROM playlist_items WHERE playlist_id = 'p'", [])?;
        c.execute("INSERT INTO playlist_items (playlist_id, position, track_id) VALUES ('p', 0, 'b1'), ('p', 1, 'b2')", [])?;
        Ok(())
    })
    .unwrap();
    r.d.resync().await.unwrap();
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2", "b1", "b2"]);
    // The playlist drops b1: no other item has it, so its file goes.
    r.db.write(|c| {
        c.execute("DELETE FROM playlist_items WHERE track_id = 'b1'", [])?;
        Ok(())
    })
    .unwrap();
    r.d.resync().await.unwrap();
    assert!(!r.dir.join("b1.flac").exists());
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2", "b2"]);
    // Removing the album deletes its songs (the playlist has neither now).
    r.d.remove(key(ItemKind::Album, "a")).await.unwrap();
    assert_eq!(r.d.downloaded_ids(), ["b2"]);
    assert!(!r.dir.join("a1.flac").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_library_answer_keeps_downloaded_files() {
    let r = rig().await;
    r.db.write(|c| {
        c.execute("UPDATE tracks SET user_rating = 10, rated_at = 5 WHERE id = 'a2'", [])?;
        Ok(())
    })
    .unwrap();
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    r.d.add(key(ItemKind::Playlist, "p"), "Mix".into(), None).await.unwrap();
    r.d.add(key(ItemKind::Liked, ""), "Liked Songs".into(), None).await.unwrap();
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2", "b1"]);
    // A short read leaves the album, the playlist and the likes empty.
    r.db.write(|c| {
        c.execute("DELETE FROM playlist_items", [])?;
        c.execute("UPDATE tracks SET album_id = 'elsewhere' WHERE album_id = 'a'", [])?;
        c.execute("UPDATE tracks SET user_rating = NULL", [])?;
        Ok(())
    })
    .unwrap();
    r.d.resync().await.unwrap();
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2", "b1"]);
    assert!(r.dir.join("a1.flac").is_file() && r.dir.join("b1.flac").is_file());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn liked_songs_download_new_likes() {
    let r = rig().await;
    r.db.write(|c| {
        c.execute("UPDATE tracks SET user_rating = 10, rated_at = 5 WHERE id = 'a2'", [])?;
        Ok(())
    })
    .unwrap();
    r.d.add(key(ItemKind::Liked, "anything"), "Liked Songs".into(), None).await.unwrap();
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a2"]);
    r.db.write(|c| {
        c.execute("UPDATE tracks SET user_rating = 10, rated_at = 6 WHERE id = 'b1'", [])?;
        Ok(())
    })
    .unwrap();
    r.d.resync().await.unwrap();
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a2", "b1"]);
    assert_eq!(item(&r.d.status().unwrap(), ItemKind::Liked, "").state, ItemState::Downloaded);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mix_downloads_the_songs_it_is_given() {
    let r = rig().await;
    assert!(r.d.add(key(ItemKind::Mix, "m1"), "Mix 1".into(), None).await.is_err());
    r.d.add(key(ItemKind::Mix, "m1"), "Mix 1".into(), Some(vec!["b2".into(), "a1".into()])).await.unwrap();
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a1", "b2"]);
    // A resync leaves a mix's own list alone.
    r.d.resync().await.unwrap();
    assert_eq!(r.d.downloaded_ids(), ["a1", "b2"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_limit_stops_the_queue_with_a_reason() {
    let r = rig().await;
    // Room for the first song only (the estimate is from bitrate and length).
    let first = estimate_bytes(&albums().2[0], DownloadQuality::Original);
    r.d.set_prefs(Prefs { limit_bytes: first + 10, ..Prefs::default() });
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    assert_eq!(r.d.step().await.unwrap(), Step::Worked);
    assert_eq!(r.d.step().await.unwrap(), Step::Paused(PauseReason::Full));
    assert_eq!(r.d.downloaded_ids(), ["a1"]);
    // Raising the limit carries on.
    r.d.set_prefs(Prefs::default());
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_song_bigger_than_its_estimate_is_not_fetched_again_and_again() {
    let r = rig().await;
    // Estimated at about 1 KB, really 96 KB; room for 50 KB.
    r.db.write(|c| Ok(c.execute("UPDATE tracks SET duration_ms = 1000, bitrate = 8", [])?)).unwrap();
    r.d.set_prefs(Prefs { limit_bytes: 50_000, ..Prefs::default() });
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    // Each song is tried once (one too big doesn't hold up the next), then
    // the queue stays paused.
    assert_eq!(r.d.step().await.unwrap(), Step::Paused(PauseReason::Full));
    assert_eq!(r.d.step().await.unwrap(), Step::Paused(PauseReason::Full));
    let fetched = r.hits.lock().len();
    assert_eq!(fetched, 2);
    for _ in 0..3 {
        assert_eq!(r.d.step().await.unwrap(), Step::Paused(PauseReason::Full));
    }
    assert_eq!(r.hits.lock().len(), fetched, "stays paused without fetching them again");
    // More room: it goes.
    r.d.set_prefs(Prefs::default());
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_full_disk_stops_the_queue() {
    let r = rig_in(tempfile::tempdir().unwrap(), Some(1024)).await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    assert_eq!(r.d.step().await.unwrap(), Step::Paused(PauseReason::NoSpace));
    assert!(r.hits.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn away_from_home_waits_when_downloads_are_home_only() {
    let r = rig().await;
    let t = &albums().2[0];
    *r.src.conn.lock() = Some(ConnectionKind::Remote);
    assert_eq!(r.d.unreachable(t).await, Some(PauseReason::NotHome));
    *r.src.conn.lock() = Some(ConnectionKind::Relay);
    assert_eq!(r.d.unreachable(t).await, Some(PauseReason::NotHome));
    r.d.set_prefs(Prefs { home_only: false, ..Prefs::default() });
    assert_eq!(r.d.unreachable(t).await, None);
    *r.src.conn.lock() = None;
    assert_eq!(r.d.unreachable(t).await, Some(PauseReason::Offline));
    *r.src.conn.lock() = Some(ConnectionKind::Local);
    r.d.set_prefs(Prefs { offline: true, ..Prefs::default() });
    assert_eq!(r.d.blocked().await, Some(PauseReason::Offline), "Go offline");
    // The step says why it waits.
    r.d.set_prefs(Prefs::default());
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    *r.src.conn.lock() = Some(ConnectionKind::Remote);
    assert_eq!(r.d.step().await.unwrap(), Step::Paused(PauseReason::NotHome));
    assert!(r.hits.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_song_waits_for_its_own_server() {
    let r = rig().await;
    // The main server is out of reach; album b's songs live on another one
    // that is at home.
    *r.src.conn.lock() = None;
    r.src.conn_of.lock().insert("b1".into(), Some(ConnectionKind::Local));
    r.src.conn_of.lock().insert("b2".into(), Some(ConnectionKind::Local));
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    r.d.add(key(ItemKind::Album, "b"), "Second".into(), None).await.unwrap();
    assert_eq!(r.d.blocked().await, None, "the main server's connection doesn't hold up the queue");
    assert_eq!(drain(&r.d).await.last(), Some(&Step::Paused(PauseReason::Offline)));
    assert_eq!(r.d.downloaded_ids(), ["b1", "b2"]);
    // The other way round: the main server is home, b's is away.
    *r.src.conn.lock() = Some(ConnectionKind::Local);
    r.src.conn_of.lock().insert("a2".into(), Some(ConnectionKind::Remote));
    assert_eq!(drain(&r.d).await.last(), Some(&Step::Paused(PauseReason::NotHome)));
    assert_eq!(r.d.downloaded_ids(), ["a1", "b1", "b2"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_song_too_big_for_the_room_left_lets_smaller_ones_by() {
    let r = rig().await;
    // a1 is estimated far bigger than a2; room for a2 only.
    r.db.write(|c| Ok(c.execute("UPDATE tracks SET duration_ms = 100000000 WHERE id = 'a1'", [])?)).unwrap();
    let small = estimate_bytes(&albums().2[1], DownloadQuality::Original);
    r.d.set_prefs(Prefs { limit_bytes: small + 10, ..Prefs::default() });
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    assert_eq!(r.d.step().await.unwrap(), Step::Worked);
    assert_eq!(r.d.downloaded_ids(), ["a2"]);
    assert_eq!(r.d.step().await.unwrap(), Step::Paused(PauseReason::Full));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn going_offline_mid_download_stops_it_and_keeps_the_part() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    r.d.set_prefs(Prefs { offline: true, ..Prefs::default() });
    assert_eq!(r.d.step().await.unwrap(), Step::Paused(PauseReason::Offline));
    assert!(r.hits.lock().is_empty(), "no request once offline");
    assert!(r.d.downloaded_ids().is_empty());
    assert_eq!(r.d.status().unwrap().queued, 2, "still queued for later");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_download_retries_then_shows_as_an_error() {
    let r = rig().await;
    r.d.add(key(ItemKind::Mix, "m"), "Mix".into(), Some(vec!["a1".into(), "gone".into()])).await.unwrap();
    drain(&r.d).await;
    let row = r.db.read(|c| store::file(c, "gone")).unwrap().unwrap();
    assert_eq!((row.state.as_str(), row.attempts), ("failed", 1));
    assert_eq!(row.error.as_deref(), Some("The file isn't on the server any more."));
    // Waiting for its retry, it isn't an error yet.
    assert_ne!(item(&r.d.status().unwrap(), ItemKind::Mix, "m").state, ItemState::Error);
    r.db.write(|c| {
        c.execute("UPDATE download_files SET attempts = ?1 WHERE track_id = 'gone'", [store::MAX_ATTEMPTS])?;
        Ok(())
    })
    .unwrap();
    let st = r.d.status().unwrap();
    let m = item(&st, ItemKind::Mix, "m");
    assert_eq!((m.state, m.done, m.failed), (ItemState::Error, 1, 1));
    assert_eq!(m.error.as_deref(), Some("The file isn't on the server any more."));
    // Retry puts it back in the queue.
    r.d.retry(Some(key(ItemKind::Mix, "m"))).await.unwrap();
    assert_eq!(r.d.status().unwrap().queued, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_missing_on_disk_is_offered_again() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    drain(&r.d).await;
    std::fs::remove_file(r.dir.join("a1.flac")).unwrap();
    // The play path notices and doesn't use it.
    assert_eq!(r.d.path(&albums().2[0]), None);
    let st = r.d.status().unwrap();
    let a = item(&st, ItemKind::Album, "a");
    assert_eq!((a.state, a.missing), (ItemState::Error, 1));
    assert_eq!(a.error.as_deref(), Some("The file is missing from this computer."));
    assert_eq!(r.d.downloaded_ids(), ["a2"]);
    // The startup check finds the same.
    std::fs::remove_file(r.dir.join("a2.flac")).unwrap();
    assert_eq!(r.d.check_files(), 1);
    // Download again.
    r.d.retry(Some(key(ItemKind::Album, "a"))).await.unwrap();
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_browse_lists_downloaded_albums() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "b"), "Second".into(), None).await.unwrap();
    drain(&r.d).await;
    let ids = r.d.downloaded_ids();
    let lib = r.db.read(|c| crate::monitor::cached::cached_library_of(c, &ids)).unwrap();
    assert_eq!(lib.albums.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["b"]);
    assert_eq!(lib.playlist_ids, ["p"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_all_and_moving_the_folder() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    drain(&r.d).await;
    let to = r.dir.parent().unwrap().join("elsewhere");
    r.d.move_to(to.clone()).await.unwrap();
    assert!(to.join("a1.flac").is_file() && !r.dir.join("a1.flac").exists());
    assert_eq!(r.d.path(&albums().2[0]), Some(to.join("a1.flac")));
    assert_eq!(r.d.remove_all().await.unwrap(), 2 * BODY as u64);
    assert!(r.d.downloaded_ids().is_empty());
    assert!(!to.join("a1.flac").exists());
    assert!(r.d.status().unwrap().items.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_move_that_fails_halfway_puts_the_files_back() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    drain(&r.d).await;
    let to = r.dir.parent().unwrap().join("elsewhere");
    // Something in the way of the second file.
    std::fs::create_dir_all(to.join("a2.flac").join("x")).unwrap();
    assert!(r.d.move_to(to.clone()).await.is_err());
    assert!(r.dir.join("a1.flac").is_file() && r.dir.join("a2.flac").is_file(), "both back where they were");
    assert!(!to.join("a1.flac").exists());
    assert_eq!(r.d.dir(), r.dir);
    assert_eq!(r.d.path(&albums().2[0]), Some(r.dir.join("a1.flac")));
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unplugged_downloads_drive_marks_nothing_missing() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    drain(&r.d).await;
    // The folder can't be reached (and can't be made again).
    let away = r.dir.with_file_name("unplugged");
    std::fs::rename(&r.dir, &away).unwrap();
    std::fs::write(&r.dir, b"not a folder").unwrap();
    assert_eq!(r.d.check_files(), 0);
    assert_eq!(r.d.path(&albums().2[0]), None, "plays from the server meanwhile");
    assert!(r.d.downloaded_ids().is_empty(), "offline, nothing there counts as playable");
    assert_eq!(r.d.blocked().await, Some(PauseReason::FolderMissing));
    let st = r.d.status().unwrap();
    let a = item(&st, ItemKind::Album, "a");
    assert_eq!((a.state, a.missing), (ItemState::Downloaded, 0));
    // Plugged back in: everything is still there.
    std::fs::remove_file(&r.dir).unwrap();
    std::fs::rename(&away, &r.dir).unwrap();
    assert_eq!(r.d.check_files(), 0);
    assert_eq!(r.d.path(&albums().2[0]), Some(r.dir.join("a1.flac")));
    assert_eq!(r.d.blocked().await, None);
}

#[test]
fn quality_and_file_names() {
    let (_, _, tracks) = albums();
    assert_eq!(extension(&tracks[0], DownloadQuality::Original), "flac");
    assert_eq!(extension(&tracks[0], DownloadQuality::Kbps128), "mp3");
    assert!(file_stem("12/../34").starts_with("1234-"));
    assert_eq!(file_stem("123"), "123");
    // Ids that clean up to the same text still get their own files.
    assert_ne!(file_stem("a.1"), file_stem("a1"));
    assert_ne!(file_stem("0a1b2c3d:9"), file_stem("0a1b2c3d9"));
    let long = "x".repeat(70);
    assert_ne!(file_stem(&long), file_stem(&format!("{long}y")));
    assert_eq!(serde_json::to_value(DownloadQuality::Kbps320).unwrap(), "320");
    // 900 kbps for 200 s.
    assert_eq!(estimate_bytes(&tracks[0], DownloadQuality::Original), 22_500_000);
    assert_eq!(estimate_bytes(&tracks[0], DownloadQuality::Kbps128), 3_200_000);
}

/// Waits up to `secs` for `f` to hold.
async fn within(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f()
}

fn add_to_playlist(db: &DbHandle, position: i64, track: &str) {
    let track = track.to_string();
    db.write(move |c| {
        c.execute(
            "INSERT INTO playlist_items (playlist_id, position, track_id) VALUES ('p', ?1, ?2)",
            rusqlite::params![position, track],
        )?;
        Ok(())
    })
    .unwrap();
}

fn drop_from_playlist(db: &DbHandle, track: &str) {
    let track = track.to_string();
    db.write(move |c| {
        c.execute("DELETE FROM playlist_items WHERE playlist_id = 'p' AND track_id = ?1", [track])?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_library_change_reaches_downloaded_items_straight_away() {
    let r = rig().await;
    r.d.add(key(ItemKind::Playlist, "p"), "Mix".into(), None).await.unwrap();
    r.d.add(key(ItemKind::Album, "b"), "Second".into(), None).await.unwrap();
    let run = tokio::spawn(r.d.clone().run());
    assert!(within(5, || r.d.downloaded_ids() == ["a1", "b1", "b2"]).await);

    // A song added to the playlist is fetched long before the 30 s re-read.
    add_to_playlist(&r.db, 2, "a2");
    r.d.poke();
    assert!(within(5, || r.d.downloaded_ids() == ["a1", "a2", "b1", "b2"]).await, "{:?}", r.d.downloaded_ids());
    let st = r.d.status().unwrap();
    assert_eq!(item(&st, ItemKind::Playlist, "p").state, ItemState::Downloaded);
    assert_eq!(item(&st, ItemKind::Playlist, "p").total, 3);
    // On the way it showed as catching up, then as downloaded again.
    let states: Vec<_> = r
        .events
        .statuses
        .lock()
        .iter()
        .filter_map(|s| s.items.iter().find(|i| i.kind == ItemKind::Playlist).map(|i| (i.state, i.total)))
        .collect();
    let back = states.iter().position(|s| *s == (ItemState::Downloading, 3)).expect("in progress with the new song");
    assert!(states[back..].contains(&(ItemState::Downloaded, 3)));

    // A song removed from the playlist and on no other item: its file goes.
    drop_from_playlist(&r.db, "a1");
    r.d.poke();
    assert!(within(5, || r.d.downloaded_ids() == ["a2", "b1", "b2"]).await);
    assert!(!r.dir.join("a1.flac").exists());
    // b1 is on the album too, so its file stays when the playlist drops it.
    drop_from_playlist(&r.db, "b1");
    r.d.poke();
    assert!(within(5, || item(&r.d.status().unwrap(), ItemKind::Playlist, "p").total == 1).await);
    assert!(r.dir.join("b1.flac").exists());
    run.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_like_downloads_straight_away() {
    let r = rig().await;
    r.d.add(key(ItemKind::Liked, ""), "Liked Songs".into(), None).await.unwrap();
    let run = tokio::spawn(r.d.clone().run());
    r.db.write(|c| {
        c.execute("UPDATE tracks SET user_rating = 10, rated_at = 7 WHERE id IN ('a1', 'b2')", [])?;
        Ok(())
    })
    .unwrap();
    r.d.poke();
    assert!(within(5, || r.d.downloaded_ids() == ["a1", "b2"]).await);
    // Unliking removes the file.
    r.db.write(|c| {
        c.execute("UPDATE tracks SET user_rating = NULL WHERE id = 'b2'", [])?;
        Ok(())
    })
    .unwrap();
    r.d.poke();
    assert!(within(5, || r.d.downloaded_ids() == ["a1"]).await);
    assert!(!r.dir.join("b2.flac").exists());
    run.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn go_offline_holds_new_songs_until_it_is_turned_off() {
    let r = rig().await;
    r.d.add(key(ItemKind::Playlist, "p"), "Mix".into(), None).await.unwrap();
    let run = tokio::spawn(r.d.clone().run());
    assert!(within(5, || r.d.downloaded_ids() == ["a1", "b1"]).await);
    r.d.set_prefs(Prefs { offline: true, ..Prefs::default() });
    add_to_playlist(&r.db, 2, "b2");
    r.d.poke();
    assert!(within(5, || r.d.status().unwrap().queued == 1).await);
    assert!(within(5, || r.d.status().unwrap().paused == Some(PauseReason::Offline)).await);
    // Room for a wrong download to start, which the checks below would see.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let st = r.d.status().unwrap();
    assert_eq!(st.paused, Some(PauseReason::Offline));
    assert_eq!(item(&st, ItemKind::Playlist, "p").state, ItemState::Queued);
    assert_eq!(r.d.downloaded_ids(), ["a1", "b1"]);
    assert!(!r.hits.lock().iter().any(|(p, _)| p.contains("/b2/")));
    // Turned off again: it catches up by itself.
    r.d.set_prefs(Prefs::default());
    assert!(within(5, || r.d.downloaded_ids() == ["a1", "b1", "b2"]).await);
    run.abort();
}

#[test]
fn the_app_pokes_downloads_on_library_like_and_mix_changes() {
    assert_eq!(FOLLOW_EVENTS, ["library://changed", "likes://changed", "mixes://changed"]);
    assert_eq!(crate::likes::EVENT_LIKES_CHANGED, FOLLOW_EVENTS[1]);
}

// ---- resuming a partial file -------------------------------------------------

/// A bare HTTP server: `answer` gets each request's Range start and returns
/// the whole response (head and body); the connection closes after it.
async fn raw_server(answer: impl Fn(Option<u64>) -> Vec<u8> + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<Option<u64>>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file.flac", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (answer, log) = (Arc::new(answer), seen.clone());
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
            let range = head
                .lines()
                .find_map(|l| l.strip_prefix("range: bytes="))
                .and_then(|r| r.trim().trim_end_matches('-').parse::<u64>().ok());
            log.lock().push(range);
            let _ = sock.write_all(&answer(range)).await;
            let _ = sock.shutdown().await;
        }
    });
    (url, seen)
}

fn response(status: &str, headers: &[String], body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\nConnection: close\r\n").into_bytes();
    for h in headers {
        out.extend_from_slice(format!("{h}\r\n").as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

/// Serves `body` with Range support; `full` answers a request without a
/// Range.
fn ranged(body: Vec<u8>, full: impl Fn(&[u8]) -> Vec<u8> + Send + Sync + 'static) -> impl Fn(Option<u64>) -> Vec<u8> + Send + Sync + 'static {
    move |range| match range {
        None => full(&body),
        Some(from) if (from as usize) < body.len() => response(
            "206 Partial Content",
            &[format!("Content-Range: bytes {from}-{}/{}", body.len() - 1, body.len()), format!("Content-Length: {}", body.len() - from as usize)],
            &body[from as usize..],
        ),
        Some(_) => response("416 Range Not Satisfiable", &[format!("Content-Range: bytes */{}", body.len()), "Content-Length: 0".into()], b""),
    }
}

async fn fetch_to(url: &str, part: &Path) -> transfer::Outcome {
    let client = crate::audio::http::http_client();
    let gate = TransferGate::new();
    let ctx = transfer::Ctx {
        client: &client,
        gate: &gate,
        host: None,
        room: u64::MAX,
        min_len: None,
        keep_going: &|| true,
        progress: &|_, _| {},
    };
    let fetch = Fetch { src: StreamSource { url: url.into(), ..Default::default() }, resumable: true, session: None };
    transfer::fetch_file(&ctx, &fetch, part).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_partial_file_that_does_not_match_the_server_starts_over() {
    let body = body_of("new");
    let (url, seen) = raw_server(ranged(body.clone(), |b| {
        response("200 OK", &[format!("Content-Length: {}", b.len())], b)
    }))
    .await;
    let tmp = tempfile::tempdir().unwrap();
    let part = tmp.path().join("x.flac.part");
    // Longer than the file now on the server: "nothing past that" isn't done.
    std::fs::write(&part, vec![9u8; BODY + 10]).unwrap();
    assert_eq!(fetch_to(&url, &part).await, transfer::Outcome::Done(BODY as u64));
    assert_eq!(std::fs::read(&part).unwrap(), body);
    assert_eq!(seen.lock().clone(), [Some(BODY as u64 + 10), None]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resume_answered_from_the_wrong_place_starts_over() {
    let body = body_of("shifted");
    let b2 = body.clone();
    let (url, seen) = raw_server(move |range| match range {
        // Says partial, but sends from the start.
        Some(_) => response(
            "206 Partial Content",
            &[format!("Content-Range: bytes 0-{}/{}", b2.len() - 1, b2.len()), format!("Content-Length: {}", b2.len())],
            &b2,
        ),
        None => response("200 OK", &[format!("Content-Length: {}", b2.len())], &b2),
    })
    .await;
    let tmp = tempfile::tempdir().unwrap();
    let part = tmp.path().join("x.flac.part");
    std::fs::write(&part, &body[..1000]).unwrap();
    assert_eq!(fetch_to(&url, &part).await, transfer::Outcome::Done(BODY as u64));
    assert_eq!(std::fs::read(&part).unwrap(), body, "not the start glued on twice");
    assert_eq!(seen.lock().clone(), [Some(1000), None]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_download_without_a_length_is_checked_before_it_counts() {
    let body = body_of("nolength");
    // No length, and the connection drops halfway.
    let (url, seen) = raw_server(ranged(body.clone(), |b| response("200 OK", &[], &b[..b.len() / 2]))).await;
    let tmp = tempfile::tempdir().unwrap();
    let part = tmp.path().join("x.flac.part");
    assert_eq!(fetch_to(&url, &part).await, transfer::Outcome::Done(BODY as u64));
    assert_eq!(std::fs::read(&part).unwrap(), body);
    let half = BODY as u64 / 2;
    assert_eq!(seen.lock().clone(), [None, Some(half), Some(half)], "asked past the end, then fetched the rest");
}

// ---- mixes, replaced songs and leftovers ---------------------------------------

fn store_mix(db: &DbHandle, id: &str, tracks: &[&str]) {
    use crate::mixes::store::{Stored, save_all};
    let m = Stored {
        id: id.into(),
        kind: crate::mixes::MixKind::Daily,
        title: "Daily Mix 1".into(),
        description: String::new(),
        color: "#000".into(),
        artist_ids: vec![],
        tag: None,
        tracks: tracks.iter().map(|t| (t.to_string(), false)).collect(),
    };
    db.write(move |c| save_all(c, std::slice::from_ref(&m), "2026-10-18", 1)).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_downloaded_mix_follows_its_mix_when_it_is_made_again() {
    let r = rig().await;
    store_mix(&r.db, "daily:1", &["a1", "b1"]);
    // The stored mix wins over a page's own list (a steered one).
    r.d.add(key(ItemKind::Mix, "daily:1"), "Daily Mix 1".into(), Some(vec!["b2".into()])).await.unwrap();
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a1", "b1"]);
    // The next day's mix: a1 left, a2 came in.
    store_mix(&r.db, "daily:1", &["a2", "b1"]);
    r.d.resync().await.unwrap();
    assert!(!r.dir.join("a1.flac").exists(), "the song that left is freed");
    drain(&r.d).await;
    assert_eq!(r.d.downloaded_ids(), ["a2", "b1"]);
    let st = r.d.status().unwrap();
    let m = item(&st, ItemKind::Mix, "daily:1");
    assert_eq!((m.state, m.done, m.total), (ItemState::Downloaded, 2, 2));
    assert_eq!(st.used_bytes, 2 * BODY as u64, "only what is listed is counted");
    // A mix no longer made keeps what it has.
    r.db.write(|c| Ok(c.execute("DELETE FROM mixes", [])?)).unwrap();
    r.d.resync().await.unwrap();
    assert_eq!(r.d.downloaded_ids(), ["a2", "b1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remade_mix_downloads_its_new_songs_straight_away() {
    let r = rig().await;
    store_mix(&r.db, "daily:1", &["a1"]);
    r.d.add(key(ItemKind::Mix, "daily:1"), "Daily Mix 1".into(), Some(vec!["a1".into()])).await.unwrap();
    let run = tokio::spawn(r.d.clone().run());
    assert!(within(5, || r.d.downloaded_ids() == ["a1"]).await);
    store_mix(&r.db, "daily:1", &["b2"]);
    // What the app does on mixes://changed.
    r.d.poke();
    assert!(within(5, || r.d.downloaded_ids() == ["b2"]).await, "{:?}", r.d.downloaded_ids());
    assert!(!r.dir.join("a1.flac").exists());
    run.abort();
}

fn new_part_key(db: &DbHandle, id: &str, key: &str) {
    let (id, key) = (id.to_string(), key.to_string());
    db.write(move |c| Ok(c.execute("UPDATE tracks SET part_key = ?2 WHERE id = ?1", rusqlite::params![id, key])?)).unwrap();
}

fn track_now(db: &DbHandle, id: &str) -> Track {
    let id = id.to_string();
    db.read(move |c| queries::tracks_by_ids(c, &[id])).unwrap().remove(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_song_replaced_on_the_server_plays_from_its_old_file_until_the_new_one_lands() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    r.d.add(key(ItemKind::Album, "b"), "Second".into(), None).await.unwrap();
    drain(&r.d).await;
    // Plex scanned a1 again: it has a new part.
    new_part_key(&r.db, "a1", "/library/parts/a1/2/file.flac");
    r.d.resync().await.unwrap();
    let st = r.d.status().unwrap();
    assert_eq!(st.queued, 1, "waiting to be fetched again");
    assert_eq!(st.used_bytes, 4 * BODY as u64, "its old file still counts");
    // Something else changes the set of files: a1 still plays from disk.
    r.d.remove(key(ItemKind::Album, "b")).await.unwrap();
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2"]);
    assert_eq!(r.d.path(&track_now(&r.db, "a1")), Some(r.dir.join("a1.flac")));
    // Away from home, nothing is fetched and it keeps playing.
    *r.src.conn.lock() = Some(ConnectionKind::Remote);
    assert_eq!(r.d.step().await.unwrap(), Step::Paused(PauseReason::NotHome));
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2"]);
    assert_eq!(item(&r.d.status().unwrap(), ItemKind::Album, "a").state, ItemState::Downloaded);
    // Home again: the new file replaces the old one.
    *r.src.conn.lock() = Some(ConnectionKind::Local);
    drain(&r.d).await;
    assert!(r.hits.lock().iter().any(|(p, _)| p == "/library/parts/a1/2/file.flac"));
    let row = r.db.read(|c| store::file(c, "a1")).unwrap().unwrap();
    assert_eq!((row.state.as_str(), row.part_key.as_str()), ("done", "/library/parts/a1/2/file.flac"));
    let st = r.d.status().unwrap();
    assert_eq!((st.queued, st.used_bytes), (0, 2 * BODY as u64));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_refetch_keeps_the_old_file_playing() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    drain(&r.d).await;
    // The new part can't be fetched.
    new_part_key(&r.db, "a1", "/library/parts/gone/2/file.flac");
    assert_eq!(drain(&r.d).await.last(), Some(&Step::Idle), "waits out its retry instead of looping");
    let row = r.db.read(|c| store::file(c, "a1")).unwrap().unwrap();
    assert_eq!((row.state.as_str(), row.attempts), ("done", 1));
    assert_eq!(r.d.downloaded_ids(), ["a1", "a2"]);
    assert!(r.dir.join("a1.flac").is_file());
    let st = r.d.status().unwrap();
    assert_eq!((st.queued, st.used_bytes), (0, 2 * BODY as u64));
    assert_eq!(item(&st, ItemKind::Album, "a").state, ItemState::Downloaded);
    // Retry tries it again now.
    r.d.retry(Some(key(ItemKind::Album, "a"))).await.unwrap();
    assert_eq!(r.d.status().unwrap().queued, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_partial_file_of_an_older_version_is_not_continued() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    std::fs::create_dir_all(&r.dir).unwrap();
    // Half of a1 came before the file changed on the server.
    let old = r.dir.join(part_name("a1.flac", "/library/parts/a1/1/file.flac"));
    std::fs::write(&old, vec![1u8; 1000]).unwrap();
    new_part_key(&r.db, "a1", "/library/parts/a1/2/file.flac");
    assert_eq!(r.d.step().await.unwrap(), Step::Worked);
    assert!(!old.exists());
    assert!(r.hits.lock().iter().all(|(_, range)| range.is_none()), "fetched from the start");
    assert_eq!(std::fs::read(r.dir.join("a1.flac")).unwrap(), body_of("a1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_an_item_deletes_its_partial_files() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    std::fs::create_dir_all(&r.dir).unwrap();
    let part = r.dir.join(part_name("a1.flac", "/library/parts/a1/1/file.flac"));
    let mp3 = r.dir.join(part_name("a1.mp3", "/library/parts/a1/1/file.flac"));
    std::fs::write(&part, b"half").unwrap();
    std::fs::write(&mp3, b"half").unwrap();
    // Another song whose id starts the same way keeps its own.
    let other = r.dir.join(part_name("a10.flac", "/x"));
    std::fs::write(&other, b"half").unwrap();
    r.d.remove(key(ItemKind::Album, "a")).await.unwrap();
    assert!(!part.exists() && !mp3.exists());
    assert!(other.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_that_could_not_be_deleted_is_deleted_later() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    drain(&r.d).await;
    // Something holds a1 so it can't go (a folder in its place stands in
    // for a file in use).
    let a1 = r.dir.join("a1.flac");
    std::fs::remove_file(&a1).unwrap();
    std::fs::create_dir_all(a1.join("held")).unwrap();
    r.d.remove(key(ItemKind::Album, "a")).await.unwrap();
    assert_eq!(r.d.retry_pending_deletes(), 1, "still waiting");
    // Let go of it.
    std::fs::remove_dir_all(&a1).unwrap();
    std::fs::write(&a1, b"x").unwrap();
    assert_eq!(r.d.retry_pending_deletes(), 0);
    assert!(!a1.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_sweeps_leftover_partial_files_and_unused_art() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    std::fs::create_dir_all(r.dir.join(art::ART_DIR)).unwrap();
    let wanted = r.dir.join(part_name("a1.flac", "/library/parts/a1/1/file.flac"));
    let orphan = r.dir.join(part_name("zz.flac", "/library/parts/zz/1/file.flac"));
    let legacy = r.dir.join("qq.flac.part");
    let not_ours = r.dir.join("notes.txt");
    for p in [&wanted, &orphan, &legacy, &not_ours] {
        std::fs::write(p, b"x").unwrap();
    }
    let thumb = albums().2[0].thumb.clone().expect("a cover");
    let used_art = art::path(&r.dir, &thumb);
    let old_art = art::path(&r.dir, "/library/metadata/999/thumb/1");
    std::fs::write(&used_art, b"img").unwrap();
    std::fs::write(&old_art, b"img").unwrap();
    r.d.sweep();
    assert!(wanted.exists() && used_art.exists() && not_ours.exists());
    assert!(!orphan.exists() && !legacy.exists() && !old_art.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_deleted_while_running_stops_counting_within_a_check() {
    let r = rig().await;
    r.d.add(key(ItemKind::Album, "a"), "First".into(), None).await.unwrap();
    drain(&r.d).await;
    r.d.set_check_every(Duration::from_millis(100));
    let run = tokio::spawn(r.d.clone().run());
    std::fs::remove_file(r.dir.join("a1.flac")).unwrap();
    assert!(within(5, || r.d.downloaded_ids() == ["a2"]).await, "{:?}", r.d.downloaded_ids());
    assert_eq!(item(&r.d.status().unwrap(), ItemKind::Album, "a").missing, 1);
    run.abort();
}

#[test]
fn a_transcode_must_come_to_most_of_its_size() {
    let (_, _, tracks) = albums();
    // 900 kbps source, 200 s, at 320: 8 MB, and at least 70 percent of it.
    assert_eq!(shortest_transcode(&tracks[0], DownloadQuality::Kbps320), Some(5_600_000));
    assert_eq!(shortest_transcode(&tracks[0], DownloadQuality::Original), None);
    // A source smaller than the quality may come back as it is.
    let mut small = tracks[0].clone();
    small.bitrate = Some(128);
    assert_eq!(shortest_transcode(&small, DownloadQuality::Kbps320), Some(128 * 125 * 200 * 7 / 10));
    small.bitrate = None;
    assert_eq!(shortest_transcode(&small, DownloadQuality::Kbps320), None, "can't tell");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_transcode_that_ends_early_is_tried_again_not_kept() {
    let body = body_of("cut");
    let (url, seen) = raw_server(move |_| response("200 OK", &[], &body[..body.len() / 2])).await;
    let tmp = tempfile::tempdir().unwrap();
    let part = tmp.path().join("x.mp3.part");
    let client = crate::audio::http::http_client();
    let gate = TransferGate::new();
    let ctx = transfer::Ctx {
        client: &client,
        gate: &gate,
        host: None,
        room: u64::MAX,
        min_len: Some(BODY as u64 * 9 / 10),
        keep_going: &|| true,
        progress: &|_, _| {},
    };
    let fetch = Fetch { src: StreamSource { url, ..Default::default() }, resumable: false, session: None };
    assert_eq!(
        transfer::fetch_file(&ctx, &fetch, &part).await,
        transfer::Outcome::Failed("The server stopped sending the song early.".into())
    );
    assert_eq!(seen.lock().len(), 3, "tried three times");
    // The whole song is kept.
    let body = body_of("whole");
    let (url, _) = raw_server(move |_| response("200 OK", &[], &body)).await;
    let fetch = Fetch { src: StreamSource { url, ..Default::default() }, resumable: false, session: None };
    assert_eq!(transfer::fetch_file(&ctx, &fetch, &part).await, transfer::Outcome::Done(BODY as u64));
}
