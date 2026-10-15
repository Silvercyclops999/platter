//! Calls reach the server an item belongs to, and missing capabilities
//! fail softly. Fake servers record what they're asked.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use super::routed::*;
use super::*;
use crate::audio::source::{StreamSource, TrackResolver};
use crate::db::source::{ArtBytes, ArtFetcher, LibrarySource, PlaylistEntry, RemoteHits, RemoteSearch};
use crate::downloads::{DownloadSource, Fetch};
use crate::error::{AppError, AppResult};
use crate::likes::LikeRemote;
use crate::model::{Playlist, Track};
use crate::playlists::PlaylistRemote;
use crate::plex::types::ConnectionKind;
use crate::tags::scan::PartSource;
use crate::tracking::{PlayReport, PlayReporter};

/// A server that writes `<name> <call> <id>` for everything it's asked.
struct Fake {
    name: &'static str,
    caps: Caps,
    log: Arc<Mutex<Vec<String>>>,
    online: bool,
}

impl Fake {
    fn new(name: &'static str, caps: Caps, log: &Arc<Mutex<Vec<String>>>) -> Arc<Self> {
        Arc::new(Self { name, caps, log: log.clone(), online: true })
    }
    fn note(&self, what: &str, id: &str) {
        self.log.lock().push(format!("{} {what} {id}", self.name));
    }
}

fn src(url: &str) -> StreamSource {
    StreamSource { url: url.into(), headers: Vec::new(), size: None, codec_hint: None }
}

#[async_trait]
impl RemoteSearch for Fake {
    async fn search(&self, query: &str, _: Option<&str>, _: u32) -> AppResult<RemoteHits> {
        self.note("search", query);
        Ok(RemoteHits::default())
    }
}

#[async_trait]
impl ArtFetcher for Fake {
    async fn fetch(&self, path: &str, _: u32, _: u32) -> AppResult<ArtBytes> {
        self.note("art", path);
        Ok(ArtBytes { bytes: bytes::Bytes::from_static(b"x"), content_type: "image/png".into() })
    }
}

#[async_trait]
impl TrackResolver for Fake {
    async fn stream(&self, t: &Track) -> AppResult<StreamSource> {
        self.note("stream", &t.id);
        Ok(src("http://a/s"))
    }
    async fn transcode(&self, t: &Track) -> AppResult<StreamSource> {
        self.note("transcode", &t.id);
        Ok(src("http://a/t"))
    }
    fn save_silence(&self, id: &str, _: u64, _: u64) {
        self.note("silence", id);
    }
}

#[async_trait]
impl DownloadSource for Fake {
    async fn connection(&self) -> Option<ConnectionKind> {
        self.note("connection", "-");
        Some(ConnectionKind::Local)
    }
    async fn fetch(&self, t: &Track, _: Option<u32>) -> AppResult<Fetch> {
        self.note("download", &t.id);
        Ok(Fetch { src: src("http://a/d"), resumable: true, session: None })
    }
}

#[async_trait]
impl PartSource for Fake {
    async fn part(&self, id: &str, _: &str) -> AppResult<StreamSource> {
        self.note("part", id);
        Ok(src("http://a/p"))
    }
}

#[async_trait]
impl LikeRemote for Fake {
    async fn online(&self) -> bool {
        self.note("online", "-");
        self.online
    }
    async fn set_liked(&self, id: &str, liked: bool) -> AppResult<()> {
        self.note(if liked { "like" } else { "unlike" }, id);
        Ok(())
    }
}

#[async_trait]
impl PlaylistRemote for Fake {
    async fn online(&self) -> bool {
        self.online
    }
    async fn create(&self, _: &str, ids: &[String]) -> AppResult<String> {
        self.note("create", &ids.join(","));
        Ok("new".into())
    }
    async fn add(&self, id: &str, _: &[String]) -> AppResult<()> {
        self.note("add", id);
        Ok(())
    }
    async fn remove(&self, id: &str, _: &str) -> AppResult<()> {
        self.note("remove", id);
        Ok(())
    }
    async fn move_item(&self, id: &str, _: &str, _: Option<&str>) -> AppResult<()> {
        self.note("move", id);
        Ok(())
    }
    async fn update(&self, id: &str, _: Option<&str>, _: Option<&str>) -> AppResult<()> {
        self.note("update", id);
        Ok(())
    }
    async fn poster(&self, id: &str, _: Vec<u8>, _: &str) -> AppResult<()> {
        self.note("poster", id);
        Ok(())
    }
    async fn delete(&self, id: &str) -> AppResult<()> {
        self.note("delete", id);
        Ok(())
    }
    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)> {
        self.note("fetch", id);
        Err(AppError::NotFound(id.into()))
    }
}

#[async_trait]
impl PlayReporter for Fake {
    async fn online(&self) -> bool {
        self.online
    }
    async fn timeline(&self, r: &PlayReport) -> AppResult<()> {
        self.note("timeline", &r.track_id);
        Ok(())
    }
    async fn scrobble(&self, id: &str, at: Option<i64>) -> AppResult<()> {
        self.note(&format!("scrobble@{at:?}"), id);
        Ok(())
    }
}

#[async_trait]
impl crate::artistinfo::service::MbidLookup for Fake {
    async fn artist_mbid(&self, id: &str) -> AppResult<Option<String>> {
        self.note("artist mbid", id);
        Ok(None)
    }
}

/// Wraps a [`Fake`] as a whole server (Arc so each part is the same fake).
struct Server(Arc<Fake>);

#[async_trait]
impl MusicServer for Server {
    fn kind(&self) -> BackendKind {
        BackendKind::Jellyfin
    }
    fn caps(&self) -> Caps {
        self.0.caps
    }
    async fn tag(&self) -> Option<String> {
        None
    }
    async fn name(&self) -> Option<String> {
        Some(self.0.name.into())
    }
    async fn reach(&self) -> Option<ConnectionKind> {
        Some(ConnectionKind::Local)
    }
    async fn server_id(&self) -> Option<String> {
        Some(format!("{}-id", self.0.name))
    }
    fn library(&self) -> Arc<dyn LibrarySource> {
        Arc::new(crate::db::fake::FakeSource::default())
    }
    fn search(&self) -> Arc<dyn RemoteSearch> {
        self.0.clone()
    }
    fn art(&self) -> Arc<dyn ArtFetcher> {
        self.0.clone()
    }
    fn streams(&self) -> Arc<dyn TrackResolver> {
        self.0.clone()
    }
    fn downloads(&self) -> Arc<dyn DownloadSource> {
        self.0.clone()
    }
    fn parts(&self) -> Arc<dyn PartSource> {
        self.0.clone()
    }
    fn likes(&self) -> Arc<dyn LikeRemote> {
        self.0.clone()
    }
    fn playlists(&self) -> Arc<dyn PlaylistRemote> {
        self.0.clone()
    }
    fn plays(&self) -> Arc<dyn PlayReporter> {
        self.0.clone()
    }
    fn mbids(&self) -> Arc<dyn crate::artistinfo::service::MbidLookup> {
        self.0.clone()
    }
}

const OTHER: &str = "0badc0de";

/// A main server taking every unclaimed id, and another one under OTHER.
fn rig() -> (Backends, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let main = Fake::new("main", Caps::LYRICS | Caps::TRANSCODE, &log);
    let other = Fake::new("other", Caps::SONIC, &log);
    let b = Backends::with_fallback(Arc::new(Server(main)));
    b.add(OTHER, Arc::new(Server(other)));
    (b, log)
}

fn track(id: &str) -> Track {
    let mut t = crate::plex::client::PlexMetadata { rating_key: id.into(), kind: Some("track".into()), ..Default::default() }
        .to_track();
    t.part_key = "/p".into();
    t
}

fn take(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    std::mem::take(&mut *log.lock())
}

#[tokio::test]
async fn calls_go_to_the_server_the_id_names() {
    let (b, log) = rig();
    let theirs = format!("{OTHER}:9");
    let mine = "1a2b3c4d:9".to_string();

    RoutedLikes(b.clone()).set_liked(&theirs, true).await.unwrap();
    RoutedLikes(b.clone()).set_liked(&mine, false).await.unwrap();
    RoutedLikes(b.clone()).set_liked("42", true).await.unwrap();
    assert_eq!(take(&log), vec![format!("other like {theirs}"), format!("main unlike {mine}"), "main like 42".into()]);

    let plays = RoutedPlays(b.clone());
    plays
        .timeline(&PlayReport { track_id: theirs.clone(), state: crate::tracking::PlaybackState::Playing, time_ms: 0, duration_ms: 1 })
        .await
        .unwrap();
    plays.scrobble(&theirs, Some(5)).await.unwrap();
    assert_eq!(take(&log), vec![format!("other timeline {theirs}"), format!("other scrobble@Some(5) {theirs}")]);

    let art = format!("{OTHER}:/cover/al-1");
    RoutedArt(b.clone()).fetch(&art, 10, 10).await.unwrap();
    RoutedStreams(b.clone()).stream(&track(&theirs)).await.unwrap();
    RoutedStreams(b.clone()).transcode(&track(&mine)).await.unwrap();
    RoutedStreams(b.clone()).save_silence(&theirs, 1, 2);
    RoutedParts(b.clone()).part(&theirs, "/p").await.unwrap();
    RoutedDownloads(b.clone()).fetch(&track(&theirs), None).await.unwrap();
    assert_eq!(
        take(&log),
        vec![
            format!("other art {art}"),
            format!("other stream {theirs}"),
            format!("main transcode {mine}"),
            format!("other silence {theirs}"),
            format!("other part {theirs}"),
            format!("other download {theirs}"),
        ]
    );

    // Playlists live with their songs; a new one on its first song's server.
    let pl = RoutedPlaylists(b.clone());
    pl.create("P", std::slice::from_ref(&theirs)).await.unwrap();
    pl.add(&format!("{OTHER}:p1"), &[]).await.unwrap();
    pl.delete("7").await.unwrap();
    assert_eq!(take(&log), vec![format!("other create {theirs}"), format!("other add {OTHER}:p1"), "main delete 7".into()]);
}

#[tokio::test]
async fn questions_about_no_item_go_to_the_primary_server() {
    let (b, log) = rig();
    assert!(RoutedLikes(b.clone()).online().await);
    RoutedSearch(b.clone()).search("q", None, 5).await.unwrap();
    RoutedDownloads(b.clone()).connection().await;
    assert_eq!(take(&log), vec!["main online -", "main search q", "main connection -"]);
    use crate::share::ServerId;
    assert_eq!(RoutedServerId(b.clone()).machine_id().await.as_deref(), Some("main-id"));
}

#[tokio::test]
async fn missing_capabilities_fail_softly() {
    let (b, _log) = rig();
    // No lyrics on the server: none, so LRCLIB can answer.
    use crate::lyrics::ServerLyrics;
    assert_eq!(RoutedLyrics(b.clone()).lyrics(&format!("{OTHER}:9")).await.unwrap(), None);
    // No smart playlists on the server.
    use crate::smart::SmartRemote;
    let smart = RoutedSmart(b.clone());
    assert!(!smart.online().await);
    assert!(matches!(smart.create("t", "u").await, Err(AppError::Invalid(_))));
    // No friend activity: reads as not connected.
    use crate::social::activity::ActivitySource;
    assert_eq!(RoutedActivity(b.clone()).owned().await, None);
    // Can't check files: unknown, so the last verdict stays.
    use crate::monitor::files::FileProbe;
    assert_eq!(RoutedFiles(b.clone()).readable("1").await, None);
    // No file paths for Lidarr, no editing, no remote players.
    use crate::lidarr::PlexMusic;
    assert!(matches!(RoutedFilePaths(b.clone()).music_files().await, Err(AppError::Invalid(_))));
    use crate::plexedit::EditServers;
    assert_eq!(RoutedEdit(b.clone()).owned().await, (Vec::new(), false));
    assert!(RoutedEdit(b.clone()).client_for("1").await.is_err());
    use crate::remote::RemoteEnv;
    assert!(RoutedRemote(b.clone()).server().await.is_err());
    assert!(RoutedRemote(b.clone()).account_token().await.is_none());
}

#[test]
fn capabilities_combine_across_servers() {
    let (b, _log) = rig();
    assert!(b.caps_all().has(Caps::LYRICS | Caps::TRANSCODE | Caps::SONIC));
    assert!(!b.caps_all().has(Caps::METADATA_EDIT));
    assert!(b.caps_for(&format!("{OTHER}:1")).has(Caps::SONIC));
    assert!(!b.caps_for(&format!("{OTHER}:1")).has(Caps::LYRICS));
    assert!(b.caps_for("1").has(Caps::LYRICS));
    assert_eq!(b.all().len(), 2);
}

#[test]
fn no_server_is_an_offline_error() {
    let b = Backends::with_fallback(Arc::new(Server(Fake::new("x", Caps::NONE, &Arc::default()))));
    assert!(b.primary().is_ok());
    let none = Backends::empty();
    assert!(matches!(none.primary(), Err(AppError::Network(_))));
    assert!(matches!(none.for_id("1"), Err(AppError::Network(_))));
    assert_eq!(none.caps_all(), Caps::NONE);
}

#[derive(Default)]
struct Sink(Mutex<Vec<String>>);
impl crate::plex::state::EventSink for Sink {
    fn emit(&self, event: &str, _: serde_json::Value) {
        self.0.lock().push(event.into());
    }
}

#[test]
fn adding_or_removing_a_server_tells_the_ui() {
    let (b, log) = rig();
    let sink = Arc::new(Sink::default());
    b.set_event_sink(sink.clone());
    b.add("12345678", Arc::new(Server(Fake::new("third", Caps::NONE, &log))));
    b.remove("12345678");
    b.remove("12345678");
    assert_eq!(*sink.0.lock(), vec![backends::EVENT_BACKENDS_CHANGED, backends::EVENT_BACKENDS_CHANGED]);
}

#[test]
fn plex_has_every_capability_platter_uses_from_it() {
    let d = tempfile::tempdir().unwrap();
    let plex = crate::plex::PlexState::with_parts(
        d.path().to_path_buf(),
        Arc::new(crate::plex::keychain::MemoryStore::new()),
        "http://127.0.0.1:9",
        Default::default(),
    )
    .unwrap();
    let db = crate::db::DbHandle::open(d.path().join("t.db")).unwrap();
    let with_db = plex::PlexServer::new(plex.clone(), Arc::default(), Some(db));
    let caps = with_db.caps();
    for (name, c) in Caps::NAMED {
        let expected = !matches!(*name, "resumeQueue" | "backdatedScrobble" | "serverRadio");
        assert_eq!(caps.has(*c), expected, "{name}");
    }
    assert!(with_db.smart().is_some() && with_db.activity().is_some() && with_db.lyrics().is_some());
    assert!(with_db.edit().is_some() && with_db.remote().is_some() && with_db.files().is_some());
    assert!(with_db.file_paths().is_some());
    assert_eq!(with_db.kind(), BackendKind::Plex);
    // Without the library cache there's no music section for these.
    let bare = plex::PlexServer::new(plex, Arc::default(), None);
    assert!(bare.smart().is_none() && bare.activity().is_none());
    assert!(!bare.caps().has(Caps::SERVER_SMART_PLAYLISTS));
    assert!(!bare.caps().has(Caps::FRIEND_ACTIVITY));
}
