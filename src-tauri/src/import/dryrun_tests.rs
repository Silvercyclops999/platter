//! Bringing music over end to end with a made-up account data download the
//! size and shape of a real one: thousands of liked songs, a hundred and
//! more playlists, names in every script, duplicates, local files, podcast
//! episodes, history, the files a download holds that aren't music, and a
//! damaged one. The server and Lidarr are fakes that record what they're
//! sent, and a few of them fail on purpose.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::bring::{Bring, BringDeps, Getter, Picked, Server};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::identity::index;
use crate::lidarr::waves::QueuedGet;

// ------------------------------------------------------------------ the library

/// Artists with names that trip up matching: accents, symbols, commas,
/// other scripts, an "&" and a "+".
const ODD_ARTISTS: &[&str] = &[
    "Beyoncé",
    "AC/DC",
    "Sigur Rós",
    "宇多田ヒカル",
    "Tyler, The Creator",
    "Florence + The Machine",
    "!!!",
    "$uicideboy$",
    "MØ",
    "The The",
    "Earth, Wind & Fire",
    "Guns N' Roses",
    "P!nk",
    "Röyksopp",
    "Мумий Тролль",
    "BTS",
];

const ODD_TITLES: &[&str] = &["Halo", "Let's Go", "Ágætis byrjun", "First Love", "See You Again", "Dog Days Are Over"];

struct Lib {
    /// (id, artist, album, title)
    songs: Vec<(String, String, String, String)>,
}

fn band(n: usize) -> String {
    format!("Band {n:03}")
}

/// About 2,500 songs: 120 bands with two albums of ten, and the odd artists
/// with one album of six.
fn library() -> Lib {
    let mut songs = Vec::new();
    for b in 0..120 {
        for (ai, album) in ["Record A", "Record B"].iter().enumerate() {
            for s in 1..=10 {
                songs.push((format!("{b}-{ai}-{s}"), band(b), album.to_string(), format!("Song {s:02}")));
            }
        }
    }
    for (i, a) in ODD_ARTISTS.iter().enumerate() {
        for (s, t) in ODD_TITLES.iter().enumerate() {
            songs.push((format!("odd{i}-{s}"), a.to_string(), format!("{a} Album"), t.to_string()));
        }
    }
    Lib { songs }
}

const SRC: &str = "aaaaaaaa";

fn tid(n: &str) -> String {
    format!("{SRC}:{n}")
}

fn load_library(db: &DbHandle, lib: &Lib) {
    db.write(|c| {
        let tx = c.transaction()?;
        let mut albums = std::collections::HashSet::new();
        for (id, artist, album, title) in &lib.songs {
            let album_id = format!("al:{artist}:{album}");
            if albums.insert(album_id.clone()) {
                tx.execute(
                    "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, matched, subformats)
                     VALUES (?1, ?2, ?2, ?3, ?4, 10, ?5, 1, '[]')",
                    params![tid(&album_id), album, tid(&format!("ar:{artist}")), artist, SRC],
                )?;
            }
            tx.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, src)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 200000, ?7, ?8)",
                params![tid(id), title, tid(&album_id), album, tid(&format!("ar:{artist}")), artist, format!("/p/{id}"), SRC],
            )?;
        }
        tx.commit()?;
        index::rebuild(c)?;
        Ok(())
    })
    .unwrap();
}

// ------------------------------------------------------------------ the download

fn track(artist: &str, album: &str, title: &str, n: usize) -> Value {
    json!({"trackName": title, "artistName": artist, "albumName": album, "trackUri": format!("svc:track:{n:022}")})
}

fn item(t: Value) -> Value {
    json!({"track": t, "episode": null, "audiobook": null, "localTrack": null, "addedDate": "2023-05-01"})
}

/// Missing albums: 200 bands nobody has, one album each.
fn lost(n: usize) -> String {
    format!("Lost Band {n:03}")
}

/// The whole download, as (path inside the zip, bytes).
fn download() -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut n = 0;
    let mut next = || {
        n += 1;
        n
    };

    // Liked songs: Record A of every band (1,200 here), the odd artists'
    // songs (96 here), and 2,000 songs from 200 albums that aren't.
    let mut liked = Vec::new();
    for b in 0..120 {
        for s in 1..=10 {
            liked.push(json!({"artist": band(b), "album": "Record A", "track": format!("Song {s:02}"), "uri": format!("svc:track:{:022}", next())}));
        }
    }
    for a in ODD_ARTISTS {
        for t in ODD_TITLES {
            liked.push(json!({"artist": a, "album": format!("{a} Album"), "track": t, "uri": format!("svc:track:{:022}", next())}));
        }
    }
    for l in 0..200 {
        for s in 1..=10 {
            liked.push(json!({"artist": lost(l), "album": "Gone", "track": format!("Lost {s:02}"), "uri": format!("svc:track:{:022}", next())}));
        }
    }
    // Saved albums: 20 the library has, 30 it doesn't (10 of them also
    // albums of liked songs).
    let mut saved = Vec::new();
    for b in 0..20 {
        saved.push(json!({"artist": band(b), "album": "Record B", "uri": format!("svc:album:{b}")}));
    }
    for l in 190..220 {
        saved.push(json!({"artist": lost(l), "album": "Gone", "uri": format!("svc:album:l{l}")}));
    }
    let library = json!({
        "tracks": liked,
        "albums": saved,
        "shows": [{"name": "A Show", "publisher": "Someone", "uri": "svc:show:1"}],
        "episodes": [{"name": "Episode 1", "show": "A Show", "uri": "svc:episode:1"}],
        "bannedTracks": [],
        "artists": [{"name": "Band 001", "uri": "svc:artist:1"}],
        "bannedArtists": [],
        "other": []
    });
    files.push(("Account Data/YourLibrary.json".into(), serde_json::to_vec_pretty(&library).unwrap()));

    // Playlists, over two files as large downloads split them.
    let mut lists = Vec::new();
    // A huge one: every library song and a missing album, 2,500 and more.
    let mut everything = Vec::new();
    for b in 0..120 {
        for album in ["Record A", "Record B"] {
            for s in 1..=10 {
                everything.push(item(track(&band(b), album, &format!("Song {s:02}"), next())));
            }
        }
    }
    for s in 1..=10 {
        everything.push(item(track(&lost(0), "Gone", &format!("Lost {s:02}"), next())));
    }
    lists.push(json!({"name": "Everything", "lastModifiedDate": "2024-01-02", "items": everything, "description": null, "numberOfFollowers": 0}));
    // 100 everyday playlists of 30 songs, two of them with the same name.
    for p in 0..100 {
        let name = if p < 2 { "Chill".to_string() } else { format!("Mix {p:03}") };
        let items: Vec<Value> = (0..30).map(|i| item(track(&band((p + i) % 120), "Record B", &format!("Song {:02}", i % 10 + 1), next()))).collect();
        lists.push(json!({"name": name, "lastModifiedDate": "2023-01-01", "items": items, "collaborators": []}));
    }
    // Names in other scripts, emoji, spaces, a 100 character name used twice.
    let long = "L".repeat(100);
    for name in ["🔥 Gym 🔥", "  Spaces around  ", "夜のドライブ", "Ünïcödé / Slashes \\ and: colons?", long.as_str(), long.as_str()] {
        let items: Vec<Value> = ODD_ARTISTS.iter().map(|a| item(track(a, &format!("{a} Album"), ODD_TITLES[0], next()))).collect();
        lists.push(json!({"name": name, "items": items}));
    }
    // One with nothing in it, one of only missing songs.
    lists.push(json!({"name": "Empty", "items": []}));
    lists.push(json!({"name": "Not Yet", "items": (1..=5).map(|s| item(track(&lost(5), "Gone", &format!("Lost {s:02}"), s))).collect::<Vec<_>>()}));
    // Local files, a podcast episode, a record with nothing in it, and the
    // same song twice.
    lists.push(json!({"name": "Odds and Ends", "items": [
        {"track": null, "episode": null, "audiobook": null, "addedDate": "2023-01-01",
         "localTrack": {"uri": "svc:local:Band+000:Record+A:Song+01:200"}},
        {"track": null, "episode": null, "audiobook": null, "addedDate": "2023-01-01",
         "localTrack": {"uri": "svc:local:Sigur+R%C3%B3s:Sigur+R%C3%B3s+Album:%C3%81g%C3%A6tis+byrjun:200"}},
        {"track": null, "episode": null, "audiobook": null, "addedDate": "2023-01-01",
         "localTrack": {"uri": "svc:local:::my+demo:95"}},
        {"track": null, "episode": {"episodeName": "Ep 1", "showName": "A Show", "episodeUri": "svc:episode:1"}, "audiobook": null, "localTrack": null},
        {"track": null, "episode": null, "audiobook": null, "localTrack": null},
        item(track(&band(3), "Record A", "Song 04", 1)),
        item(track(&band(3), "Record A", "Song 04", 1)),
    ]}));
    let (one, two) = lists.split_at(60);
    files.push(("Account Data/Playlist1.json".into(), serde_json::to_vec(&json!({"playlists": one})).unwrap()));
    files.push(("Account Data/Playlist2.json".into(), serde_json::to_vec(&json!({"playlists": two})).unwrap()));

    // Listening history: 20,000 plays, a few podcasts and skips.
    let mut plays = Vec::new();
    for i in 0..20_000u64 {
        let b = (i % 120) as usize;
        let min = i * 4;
        plays.push(json!({
            "endTime": format!("2024-{:02}-{:02} {:02}:{:02}", 1 + (min / 43_200) % 12, 1 + (min / 1440) % 28, (min / 60) % 24, min % 60),
            "artistName": band(b), "trackName": format!("Song {:02}", i % 10 + 1), "msPlayed": if i % 50 == 0 { 2_000 } else { 180_000 }
        }));
    }
    files.push(("Account Data/StreamingHistory_music_0.json".into(), serde_json::to_vec(&plays).unwrap()));
    files.push((
        "Account Data/StreamingHistory_podcast_0.json".into(),
        serde_json::to_vec(&json!([{"endTime": "2024-01-01 10:00", "podcastName": "A Show", "episodeName": "Ep 1", "msPlayed": 900000}])).unwrap(),
    ));

    // Files a download holds that aren't music.
    for (name, body) in [
        ("Userdata.json", json!({"username": "someone", "email": "someone@example.com", "country": "US", "birthdate": "1990-01-01"})),
        ("Identity.json", json!({"displayName": "Someone", "imageUrl": null})),
        ("Payments.json", json!({"payment_method": "card", "country": "US"})),
        (
            "SearchQueries.json",
            json!([{"platform": "IPHONE", "searchTime": "2024-01-01T00:00:00Z", "searchQuery": "band", "searchInteractionURIs": []}]),
        ),
        ("Marquee.json", json!([{"artistName": "Band 001", "segment": "Light listeners"}])),
        ("Inferences.json", json!({"inferences": ["1P_Custom_Something"]})),
        ("Follow.json", json!({"followerCount": 1, "followingUsersCount": 2, "dismissingUsersCount": 0})),
    ] {
        files.push((format!("Account Data/{name}"), serde_json::to_vec(&body).unwrap()));
    }
    files.push(("Account Data/ReadMeFirst_AccountData.pdf".into(), b"%PDF-1.4 not read".to_vec()));
    // A file cut off half way.
    files.push(("Account Data/Playlist3.json".into(), br#"{"playlists": [{"name": "Cut off", "items": [{"track": {"trackName": "#.to_vec()));
    files
}

fn zip_of(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in files {
            z.start_file(name.as_str(), opts).unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

// ------------------------------------------------------------------ fakes

/// The server: records what it's sent. Can be made to fail.
#[derive(Default)]
struct FakeServer {
    liked: Mutex<Vec<String>>,
    made: Mutex<Vec<(String, Vec<String>)>>,
    added: Mutex<Vec<(String, Vec<String>)>>,
    /// Fail the next likes or playlist calls with this.
    fail_likes: Mutex<Option<AppError>>,
    fail_create: Mutex<Option<AppError>>,
}

fn copy(e: &AppError) -> AppError {
    match e {
        AppError::Network(m) => AppError::Network(m.clone()),
        AppError::NotFound(m) => AppError::NotFound(m.clone()),
        AppError::Invalid(m) => AppError::Invalid(m.clone()),
        other => AppError::Internal(other.message()),
    }
}

#[async_trait]
impl Server for FakeServer {
    async fn like(&self, ids: Vec<String>) -> AppResult<()> {
        if let Some(e) = self.fail_likes.lock().as_ref() {
            return Err(copy(e));
        }
        self.liked.lock().extend(ids);
        Ok(())
    }
    async fn create(&self, name: &str, ids: Vec<String>) -> AppResult<String> {
        if let Some(e) = self.fail_create.lock().as_ref() {
            return Err(copy(e));
        }
        // What the real playlist queue refuses.
        if name.trim().is_empty() || name.trim().chars().count() > crate::db::playlists::TITLE_MAX {
            return Err(AppError::Invalid(format!("Playlist names can be up to {} characters.", crate::db::playlists::TITLE_MAX)));
        }
        let mut m = self.made.lock();
        m.push((name.to_string(), ids));
        Ok(format!("pl{}", m.len()))
    }
    async fn add(&self, id: &str, ids: Vec<String>) -> AppResult<()> {
        self.added.lock().push((id.to_string(), ids));
        Ok(())
    }
}

/// Lidarr and MusicBrainz: every album is found, unless named in `unknown`.
#[derive(Default)]
struct FakeGetter {
    queued: Mutex<Vec<QueuedGet>>,
    picks: Mutex<usize>,
    unknown: Mutex<Vec<String>>,
}

#[async_trait]
impl Getter for FakeGetter {
    fn ready(&self) -> bool {
        true
    }
    fn practice_until(&self) -> Option<i64> {
        None
    }
    async fn pick(&self, artist: &str, _album: &str) -> AppResult<Picked> {
        let mut n = self.picks.lock();
        *n += 1;
        if self.unknown.lock().iter().any(|a| a == artist) {
            return Ok(Picked::None("MusicBrainz has no official release of it.".into()));
        }
        Ok(Picked::Group(format!("{:08x}-0000-4000-8000-{:012x}", *n, *n), "Gone".into()))
    }
    fn enqueue(&self, albums: Vec<QueuedGet>) -> AppResult<()> {
        self.queued.lock().extend(albums);
        Ok(())
    }
    fn refused(&self) -> HashMap<String, String> {
        HashMap::new()
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
    load_library(&db, &library());
    let server = Arc::new(FakeServer::default());
    let getter = Arc::new(FakeGetter::default());
    let bring = Bring::new(BringDeps { db: db.clone(), server: server.clone(), getter: getter.clone(), clock_ms: Arc::new(|| 1_000_000) });
    World { dir, db, server, getter, bring }
}

impl World {
    fn zip(&self) -> PathBuf {
        let p = self.dir.path().join("my_data.zip");
        std::fs::write(&p, zip_of(&download())).unwrap();
        p
    }
}

/// What the library tables hold, to show bringing music over leaves them be.
fn library_print(c: &Connection) -> String {
    c.query_row(
        "SELECT (SELECT count(*) FROM tracks) || ':' || (SELECT count(*) FROM albums) || ':' ||
                (SELECT group_concat(id || title || album_title || artist_title || part_key, '|') FROM (SELECT * FROM tracks ORDER BY id))",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

// ------------------------------------------------------------------ the run

#[tokio::test]
async fn a_whole_download_comes_over_and_leaves_the_library_be() {
    let w = world();
    let before = w.db.read(|c| Ok(library_print(c))).unwrap();
    let t = Instant::now();
    let st = w.bring.read(w.zip()).await.unwrap();
    // A few seconds on a debug build; this is the slow step.
    assert!(t.elapsed().as_secs() < 60, "{:?}", t.elapsed());
    assert_eq!(st.summary.sources, ["streaming"]);
    // Every library song liked is found, odd names and other scripts too.
    assert_eq!(st.summary.sentence, "1,296 of 3,296 liked songs are already in your library. 2,000 songs from 220 albums aren't.");
    assert_eq!(st.summary.unsure, 0);
    assert_eq!(st.sent.liked, 1_296);
    assert_eq!(w.server.liked.lock().len(), 1_296);
    let liked: std::collections::HashSet<String> = w.server.liked.lock().iter().cloned().collect();
    assert_eq!(liked.len(), 1_296, "each once");
    // 110 playlists; the empty one and the one of missing songs wait.
    assert_eq!(st.summary.playlists, 110);
    let waiting: Vec<&str> = st.lists.iter().filter(|l| l.playlist_id.is_none()).map(|l| l.name.as_str()).collect();
    assert_eq!(waiting, ["Empty", "Not Yet"]);
    assert_eq!(w.server.made.lock().len(), 108);
    let chill: Vec<&str> = st.lists.iter().filter(|l| l.name.starts_with("Chill")).map(|l| l.name.as_str()).collect();
    assert_eq!(chill, ["Chill", "Chill (2)"], "two lists with one name stay two");
    let big = st.lists.iter().find(|l| l.name == "Everything").unwrap();
    assert_eq!((big.found, big.total), (2_400, 2_410));
    let made_big = w.server.made.lock().iter().find(|(n, _)| n == "Everything").unwrap().1.len();
    assert_eq!(made_big, 2_400);
    // Local files are found by their names; the one with no artist, the
    // episode and the empty record are left out; a song twice goes once.
    let odds = st.lists.iter().find(|l| l.name == "Odds and Ends").unwrap();
    assert_eq!((odds.found, odds.total), (4, 4), "{odds:?}");
    let made_odds = w.server.made.lock().iter().find(|(n, _)| n == "Odds and Ends").unwrap().1.clone();
    assert_eq!(made_odds, [tid("0-0-1"), tid("odd2-2"), tid("3-0-4")]);
    let reports = w.db.read(super::store::reports).unwrap();
    let r = &reports[0];
    assert_eq!((r.skipped.podcasts, r.skipped.incomplete_rows), (2, 2));
    assert_eq!(r.skipped.unknown_files, 8, "seven files that aren't music and the cut-off one");
    assert_eq!(r.skipped.short_plays, 400);
    assert!(r.plays.total > 18_000 && r.plays.matched == r.plays.total, "{:?}", r.plays);
    // Albums to get: the 200 missing liked albums, and saved albums the
    // library lacks (ten of them overlap); the ones it has aren't listed.
    assert_eq!(st.albums.len(), 220);
    assert_eq!(st.progress.to_get, 220);
    assert!(st.albums[0].liked == 10, "most loved first");
    assert!(st.albums.iter().all(|a| a.artist.starts_with("Lost Band")));

    // Step 3: all of them asked for, picked 50 a look, each queued once.
    w.bring.get(None).await.unwrap();
    for _ in 0..5 {
        w.bring.pick_pending().await.unwrap();
    }
    assert_eq!(w.getter.queued.lock().len(), 220);
    let st = w.bring.status().await.unwrap();
    assert_eq!((st.progress.to_get, st.progress.on_the_way, st.progress.here), (0, 220, 0));

    // The library tables are exactly as they were.
    assert_eq!(w.db.read(|c| Ok(library_print(c))).unwrap(), before);
}

// ------------------------------------------------------------------ failures

#[tokio::test]
async fn the_server_failing_keeps_the_file_in_and_the_next_look_sends_it() {
    let w = world();
    *w.server.fail_likes.lock() = Some(AppError::Network("The server can't be reached.".into()));
    *w.server.fail_create.lock() = Some(AppError::Network("The server can't be reached.".into()));
    let st = w.bring.read(w.zip()).await.expect("the file is in even when the server isn't there");
    assert!(st.summary.liked.total > 0);
    assert!(w.server.liked.lock().is_empty() && w.server.made.lock().is_empty());
    // A look with the server still away sends nothing and loses nothing.
    w.bring.pass().await;
    assert!(w.server.liked.lock().is_empty());
    // Back: the next look sends it all, once.
    *w.server.fail_likes.lock() = None;
    *w.server.fail_create.lock() = None;
    w.bring.pass().await;
    let liked = w.server.liked.lock().len();
    let made = w.server.made.lock().len();
    assert_eq!(liked as u64, st.summary.liked.found);
    assert!(made > 100, "{made}");
    w.bring.pass().await;
    assert_eq!((w.server.liked.lock().len(), w.server.made.lock().len()), (liked, made), "nothing twice");
}

#[tokio::test]
async fn one_playlist_the_server_turns_down_doesnt_stop_the_others() {
    let w = world();
    let st = w.bring.read(w.zip()).await.unwrap();
    let made: Vec<String> = w.server.made.lock().iter().map(|(n, _)| n.clone()).collect();
    // Every list with a song found was made, the long ones cut to fit.
    let want = st.lists.iter().filter(|l| l.found > 0).count();
    assert_eq!(made.len(), want, "{made:?}");
    assert!(made.iter().all(|n| n.chars().count() <= crate::db::playlists::TITLE_MAX));
    let long = "L".repeat(100);
    assert!(made.contains(&long));
    assert!(made.contains(&format!("{} (2)", "L".repeat(96))), "the second keeps its number");
}

#[tokio::test]
async fn a_damaged_zip_is_refused_in_plain_words_and_nothing_is_kept() {
    let w = world();
    let mut bytes = zip_of(&download());
    bytes.truncate(bytes.len() / 2);
    let p = w.dir.path().join("my_data.zip");
    std::fs::write(&p, bytes).unwrap();
    let e = w.bring.read(p).await.unwrap_err();
    assert!(e.message().contains("zip couldn't be opened"), "{}", e.message());
    assert!(w.db.read(super::bring::brought).unwrap().is_empty());
    assert!(w.server.liked.lock().is_empty());
}

#[tokio::test]
async fn bringing_the_same_download_again_sends_nothing_twice() {
    let w = world();
    w.bring.read(w.zip()).await.unwrap();
    let (liked, made) = (w.server.liked.lock().len(), w.server.made.lock().len());
    let st = w.bring.read(w.zip()).await.unwrap();
    assert_eq!((w.server.liked.lock().len(), w.server.made.lock().len()), (liked, made));
    assert!(w.server.added.lock().is_empty());
    assert_eq!(
        st.lists.len(),
        w.db.read(|c| Ok(c.query_row("SELECT count(*) FROM import_playlists", [], |r| r.get::<_, i64>(0))?)).unwrap() as usize
    );
}

#[test]
fn long_playlist_names_are_cut_to_fit() {
    use super::bring::fit_title;
    assert_eq!(fit_title("  Road Trip "), "Road Trip");
    assert_eq!(fit_title("   "), "Playlist");
    let max = crate::db::playlists::TITLE_MAX;
    assert_eq!(fit_title(&"a".repeat(max)), "a".repeat(max));
    assert_eq!(fit_title(&"a".repeat(max + 20)), "a".repeat(max));
    assert_eq!(fit_title(&format!("{} (12)", "a".repeat(max))), format!("{} (12)", "a".repeat(max - 5)));
    assert_eq!(fit_title(&format!("{} (x)", "a".repeat(max))).chars().count(), max);
    let wide = "夜".repeat(max + 1);
    assert_eq!(fit_title(&wide).chars().count(), max, "characters, not bytes");
}

#[test]
fn local_files_in_playlists_are_read_from_their_link() {
    use super::sources::Parser;
    use super::sources::streaming_export::StreamingExport;
    let body = json!({"playlists": [{"name": "Mine", "items": [
        {"track": null, "episode": null, "localTrack": {"uri": "svc:local:Sigur+R%C3%B3s:Takk...:Gl%C3%B3s%C3%B3li:375"}},
        {"track": null, "episode": null, "localTrack": {"uri": "svc:local:AC%2FDC::Back+In+Black%3A+Live:"}},
        {"track": null, "episode": null, "localTrack": {"uri": "svc:local:::no+artist:95"}},
        {"track": null, "episode": null, "localTrack": {"uri": "svc:track:abc"}},
        {"track": null, "episode": {"episodeName": "Ep"}, "localTrack": null}
    ]}]});
    let p = StreamingExport.parse(&[super::files::InputFile::new("Playlist1.json", serde_json::to_vec(&body).unwrap())]).unwrap();
    let songs = &p.playlists[0].songs;
    assert_eq!(songs.len(), 2);
    assert_eq!(
        (songs[0].artist.as_str(), songs[0].title.as_str(), songs[0].album.as_deref(), songs[0].duration_ms),
        ("Sigur Rós", "Glósóli", Some("Takk..."), Some(375_000))
    );
    assert_eq!(
        (songs[1].artist.as_str(), songs[1].title.as_str(), songs[1].album.as_deref(), songs[1].duration_ms),
        ("AC/DC", "Back In Black: Live", None, None)
    );
    assert_eq!((p.skipped.incomplete_rows, p.skipped.podcasts), (2, 1));
}
