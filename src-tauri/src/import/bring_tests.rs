//! Bringing music over: each step, and each step's failure faked.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, params};

use super::bring::{self, AlbumState, Bring, BringDeps, Getter, NOT_FOUND, Picked, Server, Summary};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::identity::index;
use crate::lidarr::waves::QueuedGet;

const A: &str = "aaaaaaaa";

fn tid(n: &str) -> String {
    format!("{A}:{n}")
}

fn album(c: &Connection, id: &str, artist: &str, title: &str) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, matched, subformats)
         VALUES (?1, ?2, ?2, ?3, ?4, 1, ?5, 1, '[]')",
        params![tid(id), title, format!("{A}:ar:{artist}"), artist, A],
    )
    .unwrap();
}

fn song(c: &Connection, id: &str, album_id: &str, album: &str, artist: &str, title: &str) {
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, src)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 200000, ?7, ?8)",
        params![tid(id), title, tid(album_id), album, format!("{A}:ar:{artist}"), artist, format!("/p/{id}"), A],
    )
    .unwrap();
}

/// The library has Lights Out (both songs); Shoreline arrives later.
fn library(db: &DbHandle) {
    db.write(|c| {
        album(c, "al1", "The Paper Kites", "Lights Out");
        song(c, "1", "al1", "Lights Out", "The Paper Kites", "Northern Lights");
        song(c, "2", "al1", "Lights Out", "The Paper Kites", "Bloom");
        index::rebuild(c)?;
        Ok(())
    })
    .unwrap();
}

/// Shoreline by Low Tide lands in the library.
fn shoreline_arrives(db: &DbHandle) {
    db.write(|c| {
        album(c, "al2", "Low Tide", "Shoreline");
        song(c, "3", "al2", "Shoreline", "Low Tide", "Golden Hour");
        song(c, "4", "al2", "Shoreline", "Low Tide", "Undertow");
        index::rebuild(c)?;
        Ok(())
    })
    .unwrap();
}

/// A transfer-tool CSV: two liked songs found, two liked songs missing from
/// Shoreline, one from Static (one missing song in a playlist), and a
/// playlist that mixes found and missing.
const EXPORT: &str = "Track name,Artist name,Album,Playlist name\n\
Northern Lights,The Paper Kites,Lights Out,Liked Songs\n\
Bloom,The Paper Kites,Lights Out,Liked Songs\n\
Golden Hour,Low Tide,Shoreline,Liked Songs\n\
Undertow,Low Tide,Shoreline,Liked Songs\n\
Ghost Town,Echo Field,Static,Road Trip\n\
Northern Lights,The Paper Kites,Lights Out,Road Trip\n\
Golden Hour,Low Tide,Shoreline,Road Trip\n";

#[derive(Default)]
struct FakeServer {
    liked: Mutex<Vec<String>>,
    made: Mutex<Vec<(String, Vec<String>)>>,
    added: Mutex<Vec<(String, Vec<String>)>>,
    /// Playlists the user deleted.
    deleted: Mutex<Vec<String>>,
}

#[async_trait]
impl Server for FakeServer {
    async fn like(&self, ids: Vec<String>) -> AppResult<()> {
        self.liked.lock().extend(ids);
        Ok(())
    }
    async fn create(&self, name: &str, ids: Vec<String>) -> AppResult<String> {
        let mut m = self.made.lock();
        m.push((name.to_string(), ids));
        Ok(format!("pl{}", m.len()))
    }
    async fn add(&self, id: &str, ids: Vec<String>) -> AppResult<()> {
        if self.deleted.lock().iter().any(|d| d == id) {
            return Err(AppError::NotFound(format!("playlist {id}")));
        }
        self.added.lock().push((id.to_string(), ids));
        Ok(())
    }
}

struct FakeGetter {
    ready: Mutex<bool>,
    /// What MusicBrainz says, by album title; missing means the network failed.
    picks: Mutex<HashMap<String, Picked>>,
    queued: Mutex<Vec<QueuedGet>>,
    refused: Mutex<HashMap<String, String>>,
}

impl Default for FakeGetter {
    fn default() -> Self {
        Self { ready: Mutex::new(true), picks: Default::default(), queued: Default::default(), refused: Default::default() }
    }
}

#[async_trait]
impl Getter for FakeGetter {
    fn ready(&self) -> bool {
        *self.ready.lock()
    }
    fn practice_until(&self) -> Option<i64> {
        None
    }
    async fn pick(&self, _artist: &str, album: &str) -> AppResult<Picked> {
        self.picks.lock().get(album).cloned().ok_or_else(|| AppError::Network("MusicBrainz can't be reached.".into()))
    }
    fn enqueue(&self, albums: Vec<QueuedGet>) -> AppResult<()> {
        self.queued.lock().extend(albums);
        Ok(())
    }
    fn refused(&self) -> HashMap<String, String> {
        self.refused.lock().clone()
    }
}

struct World {
    dir: tempfile::TempDir,
    db: DbHandle,
    server: Arc<FakeServer>,
    getter: Arc<FakeGetter>,
    bring: Bring,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    library(&db);
    let server = Arc::new(FakeServer::default());
    let getter = Arc::new(FakeGetter::default());
    let bring = Bring::new(BringDeps { db: db.clone(), server: server.clone(), getter: getter.clone(), clock_ms: Arc::new(|| 1_000_000) });
    World { dir, db, server, getter, bring }
}

impl World {
    fn file(&self, name: &str, body: &[u8]) -> PathBuf {
        let p = self.dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    fn summary(&self) -> Summary {
        self.db.read(bring::summary).unwrap()
    }
}

// ------------------------------------------------------------------ step 1

#[tokio::test]
async fn a_file_platter_doesnt_know_is_refused_in_plain_words() {
    let w = world();
    let p = w.file("notes.pdf", b"%PDF-1.4");
    let e = w.bring.read(p).await.unwrap_err();
    assert_eq!(e.message(), bring_unknown());
    let p = w.file("photo.json", br#"{"width": 10}"#);
    assert!(w.bring.read(p).await.is_err());
    assert!(w.server.liked.lock().is_empty() && w.server.made.lock().is_empty());
    assert!(w.db.read(bring::brought).unwrap().is_empty(), "nothing is remembered as brought over");
}

fn bring_unknown() -> String {
    super::sources::UNKNOWN_FILE.to_string()
}

#[tokio::test]
async fn a_file_with_nothing_usable_says_what_was_left_out() {
    let w = world();
    let p = w.file("empty.csv", b"Track name,Artist name\n,Nobody\nNo artist,\n");
    let e = w.bring.read(p).await.unwrap_err();
    assert!(e.message().contains("no songs Platter could use") || e.message().contains("no songs, playlists"), "{}", e.message());
}

#[test]
fn files_are_told_apart() {
    use super::files::InputFile;
    let csv = InputFile::new("x.csv", EXPORT);
    assert_eq!(super::sources::detect(&[csv]).unwrap().0.id, "lists");
    let plist = InputFile::new(
        "Library.xml",
        "<?xml version=\"1.0\"?><!DOCTYPE plist><plist version=\"1.0\"><dict><key>Tracks</key><dict><key>1</key><dict>\
         <key>Name</key><string>Bloom</string><key>Artist</key><string>The Paper Kites</string><key>Loved</key><true/></dict></dict>\
         <key>Playlists</key><array></array></dict></plist>",
    );
    let (src, parsed) = super::sources::detect(&[plist]).unwrap();
    assert_eq!(src.id, "apple");
    assert_eq!(parsed.liked.unwrap().len(), 1);
    let lib = InputFile::new("YourLibrary.json", include_str!("fixtures/YourLibrary.json"));
    assert_eq!(super::sources::detect(&[lib]).unwrap().0.id, "streaming");
}

// ------------------------------------------------------------------ step 2

#[test]
fn clean_and_explicit_copies_follow_the_explicit_setting() {
    let w = world();
    w.db.write(|c| {
        album(c, "al9", "The Paper Kites", "Lights Out (Clean)");
        song(c, "9", "al9", "Lights Out (Clean)", "The Paper Kites", "Bloom (Clean)");
        c.execute("INSERT INTO file_tags (part_key, status, explicit, scanned_at) VALUES ('/p/2', 'ok', 1, 0)", [])?;
        index::rebuild(c)?;
        Ok(())
    })
    .unwrap();
    // Allowed: the explicit copy, whichever one matched.
    assert_eq!(w.db.read(|c| bring::rated_copy(c, &tid("9"), true)).unwrap(), tid("2"));
    assert_eq!(w.db.read(|c| bring::rated_copy(c, &tid("2"), true)).unwrap(), tid("2"));
    // Filtered: the clean one.
    assert_eq!(w.db.read(|c| bring::rated_copy(c, &tid("2"), false)).unwrap(), tid("9"));
    // A song with one copy stays as it is.
    assert_eq!(w.db.read(|c| bring::rated_copy(c, &tid("1"), false)).unwrap(), tid("1"));
}

#[tokio::test]
async fn liked_songs_found_are_liked_and_playlists_made_with_what_is_found() {
    let w = world();
    let st = w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    assert_eq!(st.summary.sentence, "2 of 4 liked songs are already in your library. 3 songs from 2 albums aren't.");
    assert_eq!((st.summary.liked.found, st.summary.liked.total), (2, 4));
    assert_eq!(st.summary.missing_albums, 2);
    assert_eq!(*w.server.liked.lock(), [tid("1"), tid("2")]);
    assert_eq!(*w.server.made.lock(), [("Road Trip".to_string(), vec![tid("1")])]);
    assert_eq!((st.sent.liked, st.sent.playlists_made, st.sent.songs_added), (2, 1, 1));
    let road = st.lists.iter().find(|l| l.name == "Road Trip").unwrap();
    assert_eq!((road.found, road.total, road.playlist_id.as_deref()), (1, 3, Some("pl1")));
    // The playlist's greyed rows: the songs not in the library, in order.
    let waiting = w.db.read(|c| bring::waiting_rows(c, "pl1")).unwrap();
    assert_eq!(waiting.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["Ghost Town", "Golden Hour"]);
    // Most loved album first: Shoreline has two liked songs.
    assert_eq!(st.albums.iter().map(|a| (a.album.as_str(), a.liked, a.songs)).collect::<Vec<_>>(), [("Shoreline", 2, 2), ("Static", 0, 1)]);
}

#[tokio::test]
async fn zero_matches_like_nothing_and_make_no_empty_playlists() {
    let w = world();
    let csv = "Track name,Artist name,Album,Playlist name\nGhost Town,Echo Field,Static,Liked Songs\nUndertow,Low Tide,Shoreline,Night\n";
    let st = w.bring.read(w.file("x.csv", csv.as_bytes())).await.unwrap();
    assert_eq!(st.summary.sentence, "Your one liked song isn't in your library yet. 2 songs from 2 albums aren't.");
    assert!(w.server.liked.lock().is_empty() && w.server.made.lock().is_empty());
    let night = st.lists.iter().find(|l| l.name == "Night").unwrap();
    assert_eq!(night.playlist_id, None, "made once its first song is found");
}

#[tokio::test]
async fn everything_already_there_leaves_nothing_to_get() {
    let w = world();
    let csv = "Track name,Artist name,Album,Playlist name\nNorthern Lights,The Paper Kites,Lights Out,Liked Songs\nBloom,The Paper Kites,Lights Out,Liked Songs\n";
    let st = w.bring.read(w.file("x.csv", csv.as_bytes())).await.unwrap();
    assert_eq!(st.summary.sentence, "All 2 liked songs are already in your library.");
    assert!(st.albums.is_empty());
    assert_eq!(st.progress, bring::Progress::default());
}

#[tokio::test]
async fn a_newer_file_updates_the_same_playlist_without_duplicates() {
    let w = world();
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    // The same file again: nothing is sent twice.
    let st = w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    assert_eq!(st.sent.liked, 2, "the last send stays on the page");
    assert_eq!(w.server.liked.lock().len(), 2);
    assert_eq!(w.server.made.lock().len(), 1);
    assert!(w.server.added.lock().is_empty());
    // A newer file adds a song to Road Trip: it goes into the same playlist.
    let newer = format!("{EXPORT}Bloom,The Paper Kites,Lights Out,Road Trip\n");
    w.bring.read(w.file("export.csv", newer.as_bytes())).await.unwrap();
    assert_eq!(w.server.made.lock().len(), 1);
    assert_eq!(*w.server.added.lock(), [("pl1".to_string(), vec![tid("2")])]);
}

#[tokio::test]
async fn a_song_the_user_unliked_or_took_out_is_never_sent_again() {
    let w = world();
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    // The user unlikes Bloom and takes Northern Lights out of Road Trip (on
    // the server). Bringing the file over again doesn't put them back.
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    assert_eq!(w.server.liked.lock().iter().filter(|i| **i == tid("2")).count(), 1);
    assert!(w.server.added.lock().is_empty());
}

// ------------------------------------------------------------------ arrivals

#[tokio::test]
async fn arriving_albums_are_liked_and_slotted_into_their_playlists() {
    let w = world();
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    shoreline_arrives(&w.db);
    assert_eq!(w.bring.catch_up().await.unwrap(), 2);
    assert_eq!(*w.server.liked.lock(), [tid("1"), tid("2"), tid("3"), tid("4")]);
    assert_eq!(*w.server.added.lock(), [("pl1".to_string(), vec![tid("3")])]);
    let s = w.summary();
    assert_eq!(s.sentence, "All 4 liked songs are already in your library. 1 song from 1 album isn't.");
    // Nothing new: nothing sent.
    assert_eq!(w.bring.catch_up().await.unwrap(), 0);
    assert_eq!(w.server.liked.lock().len(), 4);
}

#[tokio::test]
async fn arrivals_follow_the_settings() {
    let w = world();
    let mut s = crate::settings::load(&w.db);
    s.bring_like_arrivals = false;
    s.bring_fill_playlists = false;
    crate::settings::save(&w.db, &s).unwrap();
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    assert_eq!(w.server.liked.lock().len(), 2, "what's found at once is sent either way");
    shoreline_arrives(&w.db);
    assert_eq!(w.bring.catch_up().await.unwrap(), 2);
    assert_eq!(w.server.liked.lock().len(), 2);
    assert!(w.server.added.lock().is_empty());
}

#[tokio::test]
async fn a_playlist_the_user_deleted_is_left_be() {
    let w = world();
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    w.server.deleted.lock().push("pl1".into());
    shoreline_arrives(&w.db);
    w.bring.catch_up().await.unwrap();
    assert_eq!(w.server.made.lock().len(), 1, "not made again");
    let st = w.bring.status().await.unwrap();
    assert!(st.lists.iter().find(|l| l.name == "Road Trip").unwrap().gone);
}

// ------------------------------------------------------------------ step 3

#[tokio::test]
async fn getting_the_albums_queues_them_most_loved_first() {
    let w = world();
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    w.getter.picks.lock().insert("Shoreline".into(), Picked::Group("11111111-1111-1111-1111-111111111111".into(), "Shoreline".into()));
    w.getter.picks.lock().insert("Static".into(), Picked::Group("22222222-2222-2222-2222-222222222222".into(), "Static".into()));
    let st = w.bring.get(None).await.unwrap();
    assert_eq!(st.progress.on_the_way, 2);
    assert!(st.albums.iter().all(|a| a.state == AlbumState::Picking));
    assert_eq!(w.bring.pick_pending().await.unwrap(), 2);
    let queued: Vec<String> = w.getter.queued.lock().iter().map(|q| q.title.clone()).collect();
    assert_eq!(queued, ["Shoreline", "Static"]);
    let st = w.bring.status().await.unwrap();
    assert_eq!((st.progress.on_the_way, st.progress.here), (2, 0));
    // Shoreline arrives: "2 albums on the way, 1 here so far".
    shoreline_arrives(&w.db);
    w.bring.catch_up().await.unwrap();
    let st = w.bring.status().await.unwrap();
    assert_eq!((st.progress.on_the_way, st.progress.here), (2, 1));
    assert_eq!(st.albums.iter().find(|a| a.album == "Shoreline").unwrap().state, AlbumState::Here);
    // Asking again doesn't queue anything twice.
    w.bring.get(None).await.unwrap();
    assert_eq!(w.bring.pick_pending().await.unwrap(), 0);
    assert_eq!(w.getter.queued.lock().len(), 2);
}

#[tokio::test]
async fn without_lidarr_ready_nothing_is_asked_for() {
    // Not set up, key refused or a red safety check: the check hasn't passed.
    let w = world();
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    *w.getter.ready.lock() = false;
    let e = w.bring.get(None).await.unwrap_err();
    assert_eq!(e.message(), "Set up Lidarr first.");
    let st = w.bring.status().await.unwrap();
    assert!(!st.lidarr.ready);
    assert_eq!(st.progress.to_get, 2);
}

#[tokio::test]
async fn lidarr_or_musicbrainz_out_of_reach_picks_up_where_it_stopped() {
    let w = world();
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    w.bring.get(None).await.unwrap();
    // No answers at all: both wait.
    assert_eq!(w.bring.pick_pending().await.unwrap(), 0);
    let st = w.bring.status().await.unwrap();
    assert_eq!(st.albums.iter().filter(|a| a.state == AlbumState::Picking).count(), 2);
    w.getter.picks.lock().insert("Shoreline".into(), Picked::Group("11111111-1111-1111-1111-111111111111".into(), "Shoreline".into()));
    w.getter.picks.lock().insert("Static".into(), Picked::Group("22222222-2222-2222-2222-222222222222".into(), "Static".into()));
    assert_eq!(w.bring.pick_pending().await.unwrap(), 2);
}

#[tokio::test]
async fn an_album_lidarr_cant_find_stays_on_the_list_and_can_be_tried_again() {
    let w = world();
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    w.getter.picks.lock().insert("Shoreline".into(), Picked::None("MusicBrainz has no official release of it.".into()));
    w.getter.picks.lock().insert("Static".into(), Picked::Group("22222222-2222-2222-2222-222222222222".into(), "Static".into()));
    w.bring.get(None).await.unwrap();
    w.bring.pick_pending().await.unwrap();
    // Lidarr refuses Static once asked (the Get albums queue's Needs you).
    w.getter.refused.lock().insert("22222222-2222-2222-2222-222222222222".into(), "Lidarr doesn't know this album.".into());
    let st = w.bring.status().await.unwrap();
    for name in ["Shoreline", "Static"] {
        let a = st.albums.iter().find(|a| a.album == name).unwrap();
        assert_eq!(a.state, AlbumState::NotFound, "{name}");
        assert!(a.reason.as_deref().unwrap().starts_with(NOT_FOUND), "{name}");
    }
    assert_eq!(st.progress.not_found, 2);
    // Try Shoreline again.
    let key = st.albums.iter().find(|a| a.album == "Shoreline").unwrap().key.clone();
    w.getter.picks.lock().insert("Shoreline".into(), Picked::Group("11111111-1111-1111-1111-111111111111".into(), "Shoreline".into()));
    w.bring.get(Some(vec![key])).await.unwrap();
    assert_eq!(w.bring.pick_pending().await.unwrap(), 1);
}

#[tokio::test]
async fn a_connected_account_is_brought_over_once_it_has_read() {
    let w = world();
    assert!(w.bring.adopt("streaming-account").await.is_err(), "nothing read yet");
    let p = w.file("x.csv", EXPORT.as_bytes());
    w.db.write(|c| {
        let parsed = super::sources::source("lists").unwrap().parser.parse(&super::files::read_input(&p)?)?;
        let prepared = super::store::prepare(c, "streaming-account", parsed, HashMap::new())?;
        super::store::write(c, "streaming-account", None, prepared, true, 5)?;
        Ok(())
    })
    .unwrap();
    let st = w.bring.adopt("streaming-account").await.unwrap();
    assert_eq!(st.summary.sources, ["streaming-account"]);
    assert_eq!(w.server.liked.lock().len(), 2);
}

#[tokio::test]
async fn forgetting_a_source_forgets_what_bringing_it_over_kept() {
    let w = world();
    w.bring.read(w.file("export.csv", EXPORT.as_bytes())).await.unwrap();
    w.db.write(|c| super::store::forget(c, "lists")).unwrap();
    let left: i64 = w
        .db
        .read(|c| Ok(c.query_row("SELECT (SELECT count(*) FROM import_bring_sources) + (SELECT count(*) FROM import_bring_lists) + (SELECT count(*) FROM import_bring_applied)", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(left, 0);
}

// ------------------------------------------------------------------ words

#[test]
fn the_sentence() {
    let s = |liked: (u64, u64), songs: (u64, u64), missing: u64, albums: u64| {
        bring::sentence(&Summary {
            liked: bring::Counts { found: liked.0, total: liked.1 },
            songs: bring::Counts { found: songs.0, total: songs.1 },
            missing_songs: missing,
            missing_albums: albums,
            ..Default::default()
        })
    };
    assert_eq!(s((812, 900), (900, 1000), 88, 31), "812 of 900 liked songs are already in your library. 88 songs from 31 albums aren't.");
    assert_eq!(s((0, 0), (40, 50), 10, 0), "40 of 50 songs are already in your library. 10 songs aren't.");
    assert_eq!(s((0, 0), (0, 0), 0, 3), "3 albums you saved aren't.");
    assert_eq!(s((0, 0), (0, 0), 0, 0), "Those files had no liked songs or playlists.");
    assert_eq!(s((1200, 1200), (1200, 1200), 0, 0), "All 1,200 liked songs are already in your library.");
    assert_eq!(bring::n(1_234_567), "1,234,567");
    assert_eq!(bring::n(999), "999");
}

#[test]
fn where_lidarr_probably_is() {
    assert_eq!(bring::lidarr_guesses(Some("http://10.20.30.40:32400")), ["http://10.20.30.40:8686", "http://localhost:8686"]);
    assert_eq!(
        bring::lidarr_guesses(Some("https://10-20-30-40.0123456789abcdef.plex.direct:32400")),
        ["http://10.20.30.40:8686", "http://localhost:8686"]
    );
    // Remote access or the relay: never guessed (the key would leave home).
    assert_eq!(bring::lidarr_guesses(Some("https://81-2-3-4.0123456789abcdef.plex.direct:32400")), ["http://localhost:8686"]);
    assert_eq!(bring::lidarr_guesses(None), ["http://localhost:8686"]);
}

#[test]
fn dropped_file_names_are_made_safe() {
    assert_eq!(bring::safe_file_name("../../etc/Library.xml"), "Library.xml");
    assert_eq!(bring::safe_file_name("C:\\Users\\me\\my_data.zip"), "my_data.zip");
    assert_eq!(bring::safe_file_name("a:b*c?.csv"), "a_b_c_.csv");
    assert_eq!(bring::safe_file_name(""), "dropped");
    let long = format!("{}.zip", "x".repeat(300));
    assert!(bring::safe_file_name(&long).ends_with(".zip") && bring::safe_file_name(&long).len() < 120);
}

#[test]
fn the_request_pages_open_and_nothing_else_new_does() {
    use super::store::link_allowed;
    for url in bring::REQUEST_PAGES {
        assert!(link_allowed(url), "{url}");
    }
    assert!(!link_allowed("https://takeout.google.com.evil.example/"));
    assert!(!link_allowed("http://takeout.google.com/"));
}
