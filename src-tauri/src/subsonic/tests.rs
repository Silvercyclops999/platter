//! A faked OpenSubsonic server (wiremock) for every call the app makes and
//! every way signing in or using one can fail. The fake checks the sign-in
//! the way a server does (the API key, or md5(password + salt)); the
//! request lists are what the server sees, without the sign-in, which
//! changes with every salt.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use serde_json::json;
use wiremock::matchers::{any, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

use super::client::{
    self, KEY_REFUSED, NO_API_KEYS, NOT_SUBSONIC, Secret, SubsonicClient, WRONG_PASSWORD, token_for,
};
use super::manager::{Confirm, Deps, Subsonic, WRONG_URL, address_allowed, key_account};
use super::server::{Core, Features, NO_DOWNLOAD, SubsonicLibrary, SubsonicServer, ext};
use super::types::User;
use crate::audio::source::TrackResolver;
use crate::backend::server::{MusicServer, ResumeQueue};
use crate::backend::{BackendKind, Backends, Caps};
use crate::bridge::StreamPrefs;
use crate::db::source::{ArtFetcher, ItemKind, LibrarySource, RemoteSearch};
use crate::downloads::DownloadSource;
use crate::error::AppError;
use crate::jellyfin::server::ServerState;
use crate::likes::LikeRemote;
use crate::playlists::PlaylistRemote;
use crate::plex::keychain::{MemoryStore, TokenStore};
use crate::plex::token::Token;
use crate::tags::scan::PartSource;
use crate::tracking::{PlayReport, PlayReporter, PlaybackState};
use crate::backend::server::ServerRadio;

const KEY: &str = "os-test-key";
const USER: &str = "me";
const PW: &str = "open-sesame";

/// All the extensions Platter uses.
fn all_ext() -> Vec<&'static str> {
    vec![ext::API_KEY, ext::LYRICS, ext::SONIC, ext::PLAYBACK_REPORT, ext::INDEX_QUEUE, ext::TRANSCODE_OFFSET]
}

/// A request's parameters: the query and, for a form post, the body.
fn params(r: &Request) -> BTreeMap<String, Vec<String>> {
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, v) in r.url.query_pairs() {
        m.entry(k.into_owned()).or_default().push(v.into_owned());
    }
    if r.method.as_str() == "POST" {
        for (k, v) in url::form_urlencoded::parse(&r.body) {
            m.entry(k.into_owned()).or_default().push(v.into_owned());
        }
    }
    m
}

fn one<'a>(p: &'a BTreeMap<String, Vec<String>>, k: &str) -> Option<&'a str> {
    p.get(k).and_then(|v| v.first()).map(String::as_str)
}

/// Signed in the way the server accepts: the API key alone, or the user
/// with md5(password + salt).
struct Authed;

impl Match for Authed {
    fn matches(&self, r: &Request) -> bool {
        let p = params(r);
        if let Some(k) = one(&p, "apiKey") {
            return k == KEY && !p.contains_key("u") && !p.contains_key("t") && !p.contains_key("p");
        }
        match (one(&p, "u"), one(&p, "t"), one(&p, "s")) {
            (Some(u), Some(t), Some(s)) => u == USER && t == token_for(PW, s) && !p.contains_key("p"),
            _ => false,
        }
    }
}

fn envelope(open: bool, extra: serde_json::Value) -> serde_json::Value {
    let mut body = json!({ "status": "ok", "version": "1.16.1" });
    if open {
        body["type"] = json!("navidrome");
        body["serverVersion"] = json!("0.58.0");
        body["openSubsonic"] = json!(true);
    }
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    json!({ "subsonic-response": body })
}

fn ok(extra: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(envelope(true, extra))
}

fn failed(code: u64, message: &str) -> ResponseTemplate {
    failed_as(true, code, message)
}

fn failed_as(open: bool, code: u64, message: &str) -> ResponseTemplate {
    let mut body = envelope(open, json!({ "error": { "code": code, "message": message } }));
    body["subsonic-response"]["status"] = json!("failed");
    ResponseTemplate::new(200).set_body_json(body)
}

async fn on(s: &MockServer, m: &str, body: serde_json::Value) {
    Mock::given(path(format!("/rest/{m}"))).and(Authed).respond_with(ok(body)).mount(s).await;
}

fn song(id: &str, n: u32) -> serde_json::Value {
    json!({ "id": id, "title": format!("Song {n}"), "album": "Record", "albumId": "al1", "artist": "Band", "artistId": "ar1",
        "albumArtists": [{"id": "ar1", "name": "Band"}], "track": n, "discNumber": 1, "duration": 200, "coverArt": "al-al1",
        "suffix": "flac", "bitRate": 900, "bitDepth": 16, "samplingRate": 44100, "playCount": 2, "type": "music",
        "path": format!("Band/Record/0{n}.flac"), "starred": if n == 1 { json!("2024-01-01T00:00:00Z") } else { json!(null) },
        "replayGain": {"trackGain": -7.5, "albumGain": -6.0, "trackPeak": 0.98} })
}

fn album_json(created: &str, played: Option<&str>) -> serde_json::Value {
    json!({ "id": "al1", "name": "Record", "artist": "Band", "artistId": "ar1", "coverArt": "al-al1", "songCount": 2,
        "created": created, "played": played, "year": 2001, "releaseTypes": ["Album"], "musicBrainzId": "mb-release" })
}

/// A server with one folder holding one album of two songs. `extensions`
/// None: a plain Subsonic server (no OpenSubsonic at all).
async fn subsonic(extensions: Option<Vec<&str>>, download: bool) -> MockServer {
    let s = MockServer::start().await;
    let open = extensions.is_some();
    // Public: ping answers without a sign-in (refusing it), and the
    // extension list.
    Mock::given(path("/rest/ping"))
        .and(Authed)
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(open, json!({}))))
        .mount(&s)
        .await;
    if let Some(exts) = &extensions {
        let list: Vec<serde_json::Value> = exts.iter().map(|e| json!({"name": e, "versions": [1]})).collect();
        Mock::given(path("/rest/getOpenSubsonicExtensions"))
            .respond_with(ok(json!({ "openSubsonicExtensions": list })))
            .mount(&s)
            .await;
    } else {
        Mock::given(path("/rest/getOpenSubsonicExtensions")).respond_with(ResponseTemplate::new(404)).mount(&s).await;
    }
    on(&s, "tokenInfo", json!({"tokenInfo": {"username": USER}})).await;
    on(&s, "getUser", json!({"user": {"username": USER, "adminRole": false, "downloadRole": download, "streamRole": true}})).await;
    on(&s, "getMusicFolders", json!({"musicFolders": {"musicFolder": [{"id": 1, "name": "Music"}, {"id": 2, "name": "Live tapes"}]}})).await;
    on(&s, "getArtists", json!({"artists": {"index": [{"name": "B", "artist": [{"id": "ar1", "name": "Band", "coverArt": "ar-ar1", "albumCount": 1}]}]}})).await;
    let newest = album_json("2024-03-01T00:00:00Z", None);
    let older = json!({ "id": "al0", "name": "Old", "artist": "Band", "artistId": "ar1", "created": "1970-01-01T00:00:00Z" });
    Mock::given(path("/rest/getAlbumList2"))
        .and(Authed)
        .and(TypeIs("newest"))
        .respond_with(ok(json!({"albumList2": {"album": [newest, older]}})))
        .mount(&s)
        .await;
    Mock::given(path("/rest/getAlbumList2"))
        .and(Authed)
        .and(TypeIs("recent"))
        .respond_with(ok(json!({"albumList2": {"album": [album_json("2024-03-01T00:00:00Z", Some("2024-03-02T00:00:00Z"))]}})))
        .mount(&s)
        .await;
    Mock::given(path("/rest/getAlbumList2"))
        .and(Authed)
        .respond_with(ok(json!({"albumList2": {"album": [album_json("2024-03-01T00:00:00Z", None)]}})))
        .with_priority(10)
        .mount(&s)
        .await;
    let mut full = album_json("2024-03-01T00:00:00Z", None);
    full["song"] = json!([song("s1", 1), song("s2", 2)]);
    on(&s, "getAlbum", json!({ "album": full })).await;
    on(&s, "search3", json!({"searchResult3": {"artist": [], "album": [], "song": [song("s1", 1), song("s2", 2)]}})).await;
    on(&s, "getStarred2", json!({"starred2": {"song": [song("s1", 1)]}})).await;
    on(&s, "getPlaylists", json!({"playlists": {"playlist": [{"id": "p1", "name": "Mine", "songCount": 3, "duration": 600, "comment": "Car"}]}})).await;
    on(&s, "getPlaylist", json!({"playlist": {"id": "p1", "name": "Mine", "songCount": 3,
        "entry": [song("s1", 1), song("s2", 2), song("s1", 1)]}}))
    .await;
    on(&s, "createPlaylist", json!({"playlist": {"id": "p7", "name": "New"}})).await;
    on(&s, "getArtistInfo2", json!({"artistInfo2": {"similarArtist": [{"id": "ar9", "name": "Others"}, {"name": "Not here"}]}})).await;
    on(&s, "getArtist", json!({"artist": {"id": "ar1", "name": "Band", "musicBrainzId": "mb-artist"}})).await;
    on(&s, "getSong", json!({"song": song("s1", 1)})).await;
    Mock::given(path("/rest/getSong")).and(Authed).and(IdIs("s2")).respond_with(ok(json!({"song": song("s2", 2)}))).with_priority(2).mount(&s).await;
    on(&s, "getLyricsBySongId", json!({"lyricsList": {"structuredLyrics": [{"lang": "en", "synced": true, "line": [{"start": 1000, "value": "La"}]}]}})).await;
    on(&s, "getSonicSimilarTracks", json!({"sonicMatch": [{"entry": song("s2", 2), "similarity": 0.9}]})).await;
    on(&s, "getSimilarSongs", json!({"similarSongs": {"song": [song("s2", 2), song("s9", 9)]}})).await;
    on(&s, "getTopSongs", json!({"topSongs": {"song": [song("s2", 2)]}})).await;
    on(&s, "getNowPlaying", json!({"nowPlaying": {"entry": [
        { "id": "s2", "title": "Song 2", "album": "Record", "albumId": "al1", "artist": "Band", "type": "music", "username": "sam", "minutesAgo": 1, "playerName": "Phone" },
        { "id": "s1", "title": "Song 1", "type": "music", "username": USER, "minutesAgo": 0, "playerName": "Platter" }
    ]}}))
    .await;
    on(&s, "getPlayQueueByIndex", json!({"playQueueByIndex": {"currentIndex": 1, "position": 4000, "changed": "2024-03-02T00:00:00Z",
        "changedBy": "Phone app", "entry": [song("s1", 1), song("s2", 2)]}}))
    .await;
    on(&s, "getPlayQueue", json!({"playQueue": {"current": "s2", "position": 4000, "entry": [song("s1", 1), song("s2", 2)]}})).await;
    on(&s, "getScanStatus", json!({"scanStatus": {"scanning": false, "count": 2, "lastScan": "2024-03-01T00:00:00Z"}})).await;
    on(&s, "getIndexes", json!({"indexes": {"lastModified": 1000}})).await;
    Mock::given(path("/rest/getCoverArt"))
        .and(Authed)
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"\x89PNG\r\n\x1a\nrest".to_vec()).insert_header("content-type", "image/png"))
        .mount(&s)
        .await;
    // Calls with nothing to answer: fine.
    Mock::given(path_prefix_rest()).and(Authed).respond_with(ok(json!({}))).with_priority(50).mount(&s).await;
    // Anything not signed in right: refused the way Navidrome refuses.
    Mock::given(any()).respond_with(failed_as(open, 40, "Wrong username or password")).with_priority(100).mount(&s).await;
    s
}

fn path_prefix_rest() -> wiremock::matchers::PathRegexMatcher {
    wiremock::matchers::path_regex(r"^/rest/")
}

/// `getAlbumList2` of one type.
struct TypeIs(&'static str);

impl Match for TypeIs {
    fn matches(&self, r: &Request) -> bool {
        one(&params(r), "type") == Some(self.0)
    }
}

/// A call for one id.
struct IdIs(&'static str);

impl Match for IdIs {
    fn matches(&self, r: &Request) -> bool {
        one(&params(r), "id") == Some(self.0)
    }
}

fn tag_of(s: &MockServer) -> String {
    BackendKind::Subsonic.tag(&s.uri())
}

fn manager(dir: &std::path::Path, store: Arc<MemoryStore>) -> (Subsonic, Backends) {
    let backends = Backends::empty();
    backends.keep_records_in(dir.to_path_buf());
    let os = Subsonic::new(Deps {
        backends: backends.clone(),
        db: None,
        store,
        http: client::http(),
        prefs: Arc::new(StreamPrefs::new(None, None)),
        offline: Arc::default(),
        poll_minutes: Arc::new(AtomicU32::new(5)),
        live: false,
    });
    (os, backends)
}

fn signed_core(s: &MockServer, features: Features, user: User, prefs: StreamPrefs) -> Arc<Core> {
    let http = client::http();
    let unsigned = SubsonicClient::new(http.clone(), &s.uri()).unwrap();
    let core = Arc::new(Core::new(&tag_of(s), "Navidrome", unsigned, Arc::new(prefs), None));
    core.set_features(features);
    let c = SubsonicClient::new(http, &s.uri()).unwrap().signed_in(Secret::ApiKey(Token::new(KEY).unwrap()));
    core.sign_in(c, USER, user);
    core
}

fn open_core(s: &MockServer) -> Arc<Core> {
    signed_core(s, Features::new(true, all_ext().into_iter().map(str::to_string)), User::default(), StreamPrefs::new(None, None))
}

/// `method k=v&...` without the sign-in and common parameters, sorted.
async fn log(s: &MockServer) -> Vec<String> {
    s.received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            let p = params(r);
            let mut q: Vec<String> = p
                .iter()
                .filter(|(k, _)| !matches!(k.as_str(), "f" | "v" | "c" | "u" | "t" | "s" | "apiKey"))
                .flat_map(|(k, vs)| vs.iter().map(move |v| format!("{k}={v}")))
                .collect();
            q.sort();
            let m = r.url.path().trim_start_matches("/rest/").to_string();
            if q.is_empty() { m } else { format!("{m} {}", q.join("&")) }
        })
        .collect()
}

/// The password itself never left: not in an address, not in a body.
async fn password_never_sent(s: &MockServer) {
    for r in s.received_requests().await.unwrap() {
        let all = format!("{} {}", r.url, String::from_utf8_lossy(&r.body));
        assert!(!all.contains(PW), "password sent to {}", r.url.path());
        assert!(!params(&r).contains_key("p"), "p= sent to {}", r.url.path());
    }
}

fn id(s: &MockServer, raw: &str) -> String {
    format!("{}:{raw}", tag_of(s))
}

// ---- adding a server ------------------------------------------------------------------

#[tokio::test]
async fn an_api_key_signs_in_and_only_the_key_is_kept_in_the_keychain() {
    let s = subsonic(Some(all_ext()), true).await;
    let d = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let (os, backends) = manager(d.path(), store.clone());

    let check = os.check(&format!("{}/app/#/album/recentlyAdded", s.uri())).await.unwrap();
    assert_eq!(check.url, s.uri());
    assert!(check.open_subsonic && check.api_keys);
    assert!(check.name.starts_with("Navidrome on "), "{}", check.name);
    assert_eq!(check.version.as_deref(), Some("0.58.0"));

    let saved = os.sign_in(&s.uri(), None, Some(KEY.into()), None).await.unwrap();
    let tag = tag_of(&s);
    assert_eq!(saved.tag, tag);
    assert_eq!((saved.kind, saved.user.as_deref(), saved.primary), (BackendKind::Subsonic, Some(USER), true));
    assert_eq!(saved.state, Some(ServerState::Online), "its folders were read");
    assert!(saved.can_download && saved.can_transcode);

    // The key is in the keychain under os.<tag>; the saved list has no secret.
    assert_eq!(store.get(&key_account(&tag)).unwrap().unwrap().expose(), format!("apikey:{KEY}"));
    let file = std::fs::read_to_string(d.path().join(crate::backend::registry::SERVERS_FILE)).unwrap();
    assert!(!file.contains(KEY) && file.contains("\"kind\": \"subsonic\""), "{file}");
    let srv = backends.server(&tag).expect("in use");
    assert_eq!(srv.kind(), BackendKind::Subsonic);
    assert!(srv.caps().has(Caps::LYRICS | Caps::SONIC | Caps::RESUME_QUEUE));
    assert!(!srv.caps().has(Caps::STAR_RATINGS), "a star is a like, not a rating");
    assert_eq!(backends.main_server().unwrap().tag().await, Some(tag.clone()), "no Plex: it has the main library");
    // An API key went alone, never with a user name.
    for r in s.received_requests().await.unwrap() {
        let p = params(&r);
        if p.contains_key("apiKey") {
            assert!(!p.contains_key("u") && !p.contains_key("t"), "{}", r.url.path());
        }
    }
    assert!(log(&s).await.contains(&"tokenInfo".to_string()), "who the key belongs to");

    // Restored on the next start with its key.
    let (again, b2) = manager(d.path(), store.clone());
    again.restore();
    let core = again.core(&tag).unwrap();
    assert!(core.signed_in());
    assert_eq!(core.username().as_deref(), Some(USER));
    assert_eq!(b2.server(&tag).unwrap().tag().await, Some(tag.clone()));

    // Removing it forgets the key.
    again.remove(&tag).await.unwrap();
    assert!(store.get(&key_account(&tag)).unwrap().is_none());
    assert!(b2.server(&tag).is_none());
}

#[tokio::test]
async fn a_password_signs_every_request_with_a_salted_token() {
    let s = subsonic(Some(all_ext()), true).await;
    let d = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let (os, _) = manager(d.path(), store.clone());
    let e = os.sign_in(&s.uri(), Some(USER.into()), None, Some("wrong".into())).await.unwrap_err();
    assert_eq!(e.to_string(), format!("auth: {WRONG_PASSWORD}"));
    assert!(store.get(&key_account(&tag_of(&s))).unwrap().is_none(), "nothing kept");
    assert!(os.sign_in(&s.uri(), None, None, Some(PW.into())).await.is_err(), "a user name is needed");
    assert!(os.sign_in(&s.uri(), Some(USER.into()), None, None).await.is_err(), "and a password or key");

    let saved = os.sign_in(&s.uri(), Some(USER.into()), None, Some(PW.into())).await.unwrap();
    assert_eq!(saved.user.as_deref(), Some(USER));
    // Kept in the keychain (Subsonic signs every request with it).
    assert_eq!(store.get(&key_account(&saved.tag)).unwrap().unwrap().expose(), format!("password:{PW}"));
    let file = std::fs::read_to_string(d.path().join(crate::backend::registry::SERVERS_FILE)).unwrap();
    assert!(!file.contains(PW), "{file}");
    // Use it, then check every request: a token and salt, never the password.
    let core = os.core(&saved.tag).unwrap();
    SubsonicLibrary::all(core.clone()).albums("1", 0, 500).await.unwrap();
    let salts: std::collections::HashSet<String> = s
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter_map(|r| one(&params(r), "s").map(str::to_string))
        .collect();
    assert!(salts.len() >= 3, "a fresh salt each time");
    password_never_sent(&s).await;
}

#[tokio::test]
async fn a_refused_api_key_says_so() {
    let s = MockServer::start().await;
    Mock::given(path("/rest/getOpenSubsonicExtensions"))
        .respond_with(ok(json!({"openSubsonicExtensions": [{"name": ext::API_KEY, "versions": [1]}]})))
        .mount(&s)
        .await;
    Mock::given(any()).respond_with(failed(44, "Invalid API key")).mount(&s).await;
    let d = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let (os, _) = manager(d.path(), store.clone());
    let e = os.sign_in(&s.uri(), None, Some("old-key".into()), None).await.unwrap_err();
    assert_eq!(e.to_string(), format!("auth: {KEY_REFUSED}"));
    assert!(store.get(&key_account(&tag_of(&s))).unwrap().is_none());
}

#[tokio::test]
async fn wrong_address_and_not_a_subsonic_server() {
    let d = tempfile::tempdir().unwrap();
    let (os, _) = manager(d.path(), Arc::new(MemoryStore::new()));
    let url = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://127.0.0.1:{}", l.local_addr().unwrap().port())
    };
    assert_eq!(os.check(&url).await.unwrap_err().to_string(), format!("invalid: {WRONG_URL}"));
    assert!(os.check("").await.is_err());

    let web = MockServer::start().await;
    Mock::given(any()).respond_with(ResponseTemplate::new(200).set_body_string("<html>hello</html>")).mount(&web).await;
    assert_eq!(os.check(&web.uri()).await.unwrap_err().to_string(), format!("invalid: {NOT_SUBSONIC}"));
    let missing = MockServer::start().await;
    assert_eq!(os.check(&missing.uri()).await.unwrap_err().to_string(), format!("invalid: {NOT_SUBSONIC}"));
    let json_but_not = MockServer::start().await;
    Mock::given(any()).respond_with(ResponseTemplate::new(200).set_body_json(json!({"hello": "world"}))).mount(&json_but_not).await;
    assert_eq!(os.check(&json_but_not.uri()).await.unwrap_err().to_string(), format!("invalid: {NOT_SUBSONIC}"));
    assert!(os.sign_in(&web.uri(), None, Some(KEY.into()), None).await.is_err());
    // A server that moved to another host says where.
    let moved = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(301).insert_header("location", "https://music.example.org/rest/ping"))
        .mount(&moved)
        .await;
    let e = os.check(&moved.uri()).await.unwrap_err().to_string();
    assert!(e.contains("https://music.example.org"), "{e}");
}

#[tokio::test]
async fn a_plain_subsonic_server_works_without_the_extensions() {
    let s = subsonic(None, true).await;
    let d = tempfile::tempdir().unwrap();
    let (os, backends) = manager(d.path(), Arc::new(MemoryStore::new()));
    let check = os.check(&s.uri()).await.unwrap();
    assert!(!check.open_subsonic && !check.api_keys);
    assert!(!check.name.starts_with("Navidrome"), "named by where it is: {}", check.name);
    let e = os.sign_in(&s.uri(), None, Some(KEY.into()), None).await.unwrap_err();
    assert_eq!(e.to_string(), format!("invalid: {NO_API_KEYS}"));

    let saved = os.sign_in(&s.uri(), Some(USER.into()), None, Some(PW.into())).await.unwrap();
    let srv = backends.server(&saved.tag).unwrap();
    let caps = srv.caps();
    assert!(!caps.has(Caps::LYRICS) && !caps.has(Caps::SONIC) && !caps.has(Caps::LOUDNESS));
    assert!(caps.has(Caps::LIKE_DATES | Caps::SERVER_RADIO | Caps::RESUME_QUEUE | Caps::TRANSCODE));
    assert!(srv.lyrics().is_none(), "lyrics hide without songLyrics");

    // Plays: a "now playing" scrobble instead of playbackReport; the queue
    // by song id instead of by index.
    let s2 = subsonic(None, true).await;
    let core2 = signed_core(&s2, Features::default(), User::default(), StreamPrefs::new(None, None));
    let report = |state| PlayReport { track_id: id(&s2, "s1"), state, time_ms: 1000, duration_ms: 200_000 };
    core2.timeline(&report(PlaybackState::Playing)).await.unwrap();
    core2.timeline(&report(PlaybackState::Paused)).await.unwrap();
    core2.timeline(&report(PlaybackState::Stopped)).await.unwrap();
    ResumeQueue::save(core2.as_ref(), &[id(&s2, "s1"), id(&s2, "s2")], 1, 4000).await.unwrap();
    let q = ResumeQueue::load(core2.as_ref()).await.unwrap().unwrap();
    assert_eq!((q.current, q.position_ms), (1, 4000));
    assert_eq!(
        log(&s2).await,
        [
            "scrobble id=s1&submission=false",
            "savePlayQueue current=s2&id=s1&id=s2&position=4000",
            "getPlayQueue",
        ]
    );
}

#[tokio::test]
async fn without_download_rights_downloads_say_why() {
    let s = subsonic(Some(all_ext()), false).await;
    let d = tempfile::tempdir().unwrap();
    let (os, _) = manager(d.path(), Arc::new(MemoryStore::new()));
    let saved = os.sign_in(&s.uri(), None, Some(KEY.into()), None).await.unwrap();
    assert!(!saved.can_download);
    let core = os.core(&saved.tag).unwrap();
    let t = super::map::track(&saved.tag, &serde_json::from_value(song("s1", 1)).unwrap());
    assert_eq!(DownloadSource::fetch(core.as_ref(), &t, None).await.unwrap_err().to_string(), format!("auth: {NO_DOWNLOAD}"));
    assert_eq!(DownloadSource::fetch(core.as_ref(), &t, Some(192)).await.unwrap_err().to_string(), format!("auth: {NO_DOWNLOAD}"));
    // Playing still works.
    assert!(core.stream(&t).await.is_ok());
}

/// Like [`manager`], with a cache that has songs s1 and s2 (as the fake
/// server has them) from the server at `tag`.
fn manager_with_songs(dir: &std::path::Path, store: Arc<MemoryStore>, tag: &str) -> (Subsonic, Backends) {
    manager_with(dir, store, tag, &["s1", "s2"])
}

fn manager_with(dir: &std::path::Path, store: Arc<MemoryStore>, tag: &str, songs: &[&str]) -> (Subsonic, Backends) {
    let db = crate::db::DbHandle::open(dir.join("t.db")).unwrap();
    let rows: Vec<(String, String)> = songs.iter().map(|s| (format!("{tag}:{s}"), format!("Song {}", &s[1..]))).collect();
    db.write(move |c| {
        for (id, title) in &rows {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number,
                   disc_number, duration_ms, part_key) VALUES (?1, ?2, 'a', 'Record', 'r', 'Band', 1, 1, 200000, '/p')",
                [id, title],
            )?;
        }
        Ok(())
    })
    .unwrap();
    let backends = Backends::empty();
    backends.keep_records_in(dir.to_path_buf());
    let os = Subsonic::new(Deps {
        backends: backends.clone(),
        db: Some(db),
        store,
        http: client::http(),
        prefs: Arc::new(StreamPrefs::new(None, None)),
        offline: Arc::default(),
        poll_minutes: Arc::new(AtomicU32::new(5)),
        live: false,
    });
    (os, backends)
}

#[tokio::test]
async fn a_new_address_for_the_same_server_keeps_everything() {
    let home = subsonic(Some(all_ext()), true).await;
    let away = subsonic(Some(all_ext()), true).await;
    let d = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let tag = tag_of(&home);
    let (os, backends) = manager_with_songs(d.path(), store.clone(), &tag);
    os.sign_in(&home.uri(), None, Some(KEY.into()), None).await.unwrap();
    assert_eq!(backends.records().get(&tag).unwrap().product.as_deref(), Some("navidrome"));

    let saved = os.change_address(&tag, &away.uri(), None, Some(KEY.into()), None, Confirm::default()).await.unwrap();
    assert_eq!(saved.tag, tag, "same tag, so the same songs, downloads and history");
    assert_eq!(saved.url.as_deref(), Some(away.uri().as_str()));
    assert_eq!(backends.records().servers.len(), 1, "no second server");
    assert_eq!(os.core(&tag).unwrap().base(), away.uri());
    assert!(store.get(&key_account(&tag)).unwrap().is_some(), "the key stays");
    let asked = log(&away).await;
    assert!(asked.iter().any(|l| l == "getSong id=s1") && asked.iter().any(|l| l == "getSong id=s2"), "it compared two songs: {asked:?}");

    // Signing in again at another address keeps the tag too.
    let saved = os.sign_in_to(Some(&tag), &home.uri(), None, Some(KEY.into()), None, Confirm::default()).await.unwrap();
    assert_eq!(saved.tag, tag);
    assert_eq!(backends.records().servers.len(), 1);
}

fn is(e: &AppError, msg: &str) -> bool {
    matches!(e, AppError::Invalid(m) if m == msg)
}

/// Every request a server saw, as text: address and body.
async fn seen(s: &MockServer) -> Vec<String> {
    s.received_requests().await.unwrap().iter().map(|r| format!("{} {}", r.url, String::from_utf8_lossy(&r.body))).collect()
}

#[tokio::test]
async fn the_saved_sign_in_never_goes_to_a_new_address() {
    let home = subsonic(Some(all_ext()), true).await;
    let d = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let tag = tag_of(&home);
    let (os, backends) = manager_with_songs(d.path(), store.clone(), &tag);
    os.sign_in(&home.uri(), None, Some(KEY.into()), None).await.unwrap();

    // Any address, asked to sign in with what's saved: refused before a
    // single request goes out.
    let catcher = MockServer::start().await;
    Mock::given(any()).respond_with(ok(json!({}))).mount(&catcher).await;
    let e = os.change_address(&tag, &catcher.uri(), None, None, None, Confirm { same_server: true, allow_insecure: true }).await.unwrap_err();
    assert!(is(&e, super::manager::TYPE_SECRET), "{e:?}");
    assert!(catcher.received_requests().await.unwrap().is_empty(), "nothing sent");

    // A typed key goes there; the saved one never does.
    let _ = os.change_address(&tag, &catcher.uri(), None, Some("typed-key".into()), None, Confirm { same_server: true, allow_insecure: true }).await;
    for r in seen(&catcher).await {
        assert!(!r.contains(KEY), "the saved key went to the new address: {r}");
    }
    assert_eq!(backends.records().get(&tag).unwrap().url.as_deref(), Some(home.uri().as_str()));
    assert_eq!(store.get(&key_account(&tag)).unwrap().unwrap().expose(), format!("apikey:{KEY}"), "the saved key stays");

    // A typed password is checked by the server it was typed for, with a
    // salted token (it isn't the saved key).
    let away = subsonic(Some(all_ext()), true).await;
    let saved = os.change_address(&tag, &away.uri(), Some(USER.into()), None, Some(PW.into()), Confirm::default()).await.unwrap();
    assert_eq!(saved.url.as_deref(), Some(away.uri().as_str()));
    assert_eq!(store.get(&key_account(&tag)).unwrap().unwrap().expose(), format!("password:{PW}"), "the typed sign-in is kept");
    password_never_sent(&away).await;
    for r in seen(&away).await {
        assert!(!r.contains(KEY), "{r}");
    }
}

#[tokio::test]
async fn the_sign_in_waits_until_the_address_names_itself_the_same_software() {
    let home = subsonic(Some(all_ext()), true).await;
    // Another product at the new address.
    let other = MockServer::start().await;
    let mut body = envelope(true, json!({}));
    body["subsonic-response"]["type"] = json!("gonic");
    Mock::given(path("/rest/ping")).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(&other).await;
    Mock::given(any()).respond_with(ok(json!({}))).with_priority(10).mount(&other).await;
    let d = tempfile::tempdir().unwrap();
    let tag = tag_of(&home);
    let (os, _) = manager_with_songs(d.path(), Arc::new(MemoryStore::new()), &tag);
    os.sign_in(&home.uri(), None, Some(KEY.into()), None).await.unwrap();
    let e = os.change_address(&tag, &other.uri(), None, Some(KEY.into()), None, Confirm { same_server: true, allow_insecure: false }).await.unwrap_err();
    assert!(is(&e, super::manager::NOT_THIS_SERVER), "{e:?}");
    for r in seen(&other).await {
        assert!(!r.contains(KEY), "a typed key waits for the product check: {r}");
    }
}

#[tokio::test]
async fn a_new_address_that_is_another_server_is_refused() {
    let home = subsonic(Some(all_ext()), true).await;
    let other = subsonic(Some(all_ext()), true).await;
    Mock::given(path("/rest/getSong")).respond_with(failed(70, "Song not found")).with_priority(1).mount(&other).await;
    let d = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let tag = tag_of(&home);
    let (os, backends) = manager_with_songs(d.path(), store, &tag);
    os.sign_in(&home.uri(), None, Some(KEY.into()), None).await.unwrap();
    let e = os.change_address(&tag, &other.uri(), None, Some(KEY.into()), None, Confirm::default()).await.unwrap_err();
    assert!(is(&e, super::manager::NOT_THIS_SERVER), "{e:?}");
    assert_eq!(backends.records().get(&tag).unwrap().url.as_deref(), Some(home.uri().as_str()), "nothing changed");
    // Nor can signing in again move it there, even with "I'm sure".
    let e = os.sign_in_to(Some(&tag), &other.uri(), None, Some(KEY.into()), None, Confirm { same_server: true, allow_insecure: false }).await.unwrap_err();
    assert!(is(&e, super::manager::NOT_THIS_SERVER), "{e:?}");
    assert_eq!(backends.records().get(&tag).unwrap().url.as_deref(), Some(home.uri().as_str()));
}

#[tokio::test]
async fn one_song_that_answers_is_not_enough_and_ids_are_checked() {
    let home = subsonic(Some(all_ext()), true).await;
    let d = tempfile::tempdir().unwrap();
    let tag = tag_of(&home);
    let (os, backends) = manager_with_songs(d.path(), Arc::new(MemoryStore::new()), &tag);
    os.sign_in(&home.uri(), None, Some(KEY.into()), None).await.unwrap();

    // Only s1 is the same song; s2 answers as something else.
    let half = subsonic(Some(all_ext()), true).await;
    let mut wrong = song("s2", 2);
    wrong["title"] = json!("Not this one");
    Mock::given(path("/rest/getSong")).and(IdIs("s2")).respond_with(ok(json!({"song": wrong}))).with_priority(1).mount(&half).await;
    let e = os.change_address(&tag, &half.uri(), None, Some(KEY.into()), None, Confirm::default()).await.unwrap_err();
    assert!(is(&e, super::manager::NOT_THIS_SERVER), "{e:?}");

    // Every id answers with one song of another id.
    let any_id = subsonic(Some(all_ext()), true).await;
    Mock::given(path("/rest/getSong")).respond_with(ok(json!({"song": song("x9", 1)}))).with_priority(1).mount(&any_id).await;
    let e = os.change_address(&tag, &any_id.uri(), None, Some(KEY.into()), None, Confirm::default()).await.unwrap_err();
    assert!(is(&e, super::manager::NOT_THIS_SERVER), "{e:?}");

    // Same title and artist, another length.
    let longer = subsonic(Some(all_ext()), true).await;
    let mut s1 = song("s1", 1);
    s1["duration"] = json!(320);
    Mock::given(path("/rest/getSong")).and(IdIs("s1")).respond_with(ok(json!({"song": s1}))).with_priority(1).mount(&longer).await;
    let e = os.change_address(&tag, &longer.uri(), None, Some(KEY.into()), None, Confirm::default()).await.unwrap_err();
    assert!(is(&e, super::manager::NOT_THIS_SERVER), "{e:?}");
    assert_eq!(backends.records().get(&tag).unwrap().url.as_deref(), Some(home.uri().as_str()));
}

#[tokio::test]
async fn with_nothing_to_compare_only_the_user_can_say_it_is_the_same_server() {
    let home = subsonic(Some(all_ext()), true).await;
    let away = subsonic(Some(all_ext()), true).await;
    let d = tempfile::tempdir().unwrap();
    let tag = tag_of(&home);
    // No songs synced yet.
    let (os, backends) = manager(d.path(), Arc::new(MemoryStore::new()));
    os.sign_in(&home.uri(), None, Some(KEY.into()), None).await.unwrap();
    let e = os.change_address(&tag, &away.uri(), None, Some(KEY.into()), None, Confirm::default()).await.unwrap_err();
    assert!(is(&e, super::manager::CANT_TELL), "{e:?}");
    // Signing in again elsewhere can't take the server over either.
    let e = os.sign_in_to(Some(&tag), &away.uri(), None, Some(KEY.into()), None, Confirm::default()).await.unwrap_err();
    assert!(is(&e, super::manager::CANT_TELL), "{e:?}");
    assert_eq!(backends.records().get(&tag).unwrap().url.as_deref(), Some(home.uri().as_str()));
    let sure = Confirm { same_server: true, allow_insecure: false };
    let saved = os.change_address(&tag, &away.uri(), None, Some(KEY.into()), None, sure).await.unwrap();
    assert_eq!(saved.url.as_deref(), Some(away.uri().as_str()));

    // A server that doesn't name its software can't be told apart, even
    // with songs to compare.
    let plain = subsonic(None, true).await;
    let plain_tag = tag_of(&plain);
    let d2 = tempfile::tempdir().unwrap();
    let (os2, _) = manager_with_songs(d2.path(), Arc::new(MemoryStore::new()), &plain_tag);
    os2.sign_in(&plain.uri(), Some(USER.into()), None, Some(PW.into())).await.unwrap();
    let plain_away = subsonic(None, true).await;
    let e = os2.change_address(&plain_tag, &plain_away.uri(), Some(USER.into()), None, Some(PW.into()), Confirm::default()).await.unwrap_err();
    assert!(is(&e, super::manager::CANT_TELL), "{e:?}");
    assert!(os2.change_address(&plain_tag, &plain_away.uri(), Some(USER.into()), None, Some(PW.into()), sure).await.is_ok());
}

#[tokio::test]
async fn plain_http_on_the_internet_needs_the_user_s_say_so() {
    let home = subsonic(Some(all_ext()), true).await;
    let d = tempfile::tempdir().unwrap();
    let tag = tag_of(&home);
    let (os, _) = manager_with_songs(d.path(), Arc::new(MemoryStore::new()), &tag);
    os.sign_in(&home.uri(), None, Some(KEY.into()), None).await.unwrap();
    // Refused before anything is sent (this address isn't even reachable).
    let e = os.change_address(&tag, "http://music.example:4533", None, Some(KEY.into()), None, Confirm::default()).await.unwrap_err();
    assert!(is(&e, super::manager::INSECURE_ADDRESS), "{e:?}");
    let e = os.sign_in("http://music.example:4533", None, Some(KEY.into()), None).await.unwrap_err();
    assert!(is(&e, super::manager::INSECURE_ADDRESS), "{e:?}");
    // Saved at https: http at home is a downgrade, so it needs it too.
    assert!(address_allowed(Some("https://music.example"), "http://10.20.30.40:4533", false).is_err());
    assert!(address_allowed(Some("https://music.example"), "http://10.20.30.40:4533", true).is_ok());
    assert!(address_allowed(Some("http://10.20.30.40:4533"), "http://10.20.30.41:4533", false).is_ok());
    assert!(address_allowed(None, "https://music.example", false).is_ok());
    assert!(address_allowed(None, "http://music.example", false).is_err());
}

#[tokio::test]
async fn other_software_at_a_saved_address_is_not_this_server() {
    let s = subsonic(Some(all_ext()), true).await;
    let d = tempfile::tempdir().unwrap();
    let (os, backends) = manager(d.path(), Arc::new(MemoryStore::new()));
    let saved = os.sign_in(&s.uri(), None, Some(KEY.into()), None).await.unwrap();
    // The address now answers as other software.
    s.reset().await;
    let mut body = envelope(true, json!({}));
    body["subsonic-response"]["type"] = json!("gonic");
    Mock::given(path("/rest/ping")).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(&s).await;
    let e = os.sign_in(&s.uri(), None, Some(KEY.into()), None).await.unwrap_err();
    assert!(is(&e, super::manager::NOT_THIS_SERVER), "{e:?}");
    let core = os.core(&saved.tag).unwrap();
    assert!(!os.read_features(&core).await, "what it says it can do isn't kept");
    assert_eq!(backends.records().get(&saved.tag).unwrap().product.as_deref(), Some("navidrome"));
}

#[tokio::test]
async fn a_server_out_of_reach_at_start_is_asked_what_it_can_do_once_it_answers() {
    let s = subsonic(Some(all_ext()), true).await;
    let d = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let tag = tag_of(&s);
    {
        let (os, backends) = manager(d.path(), store.clone());
        os.sign_in(&s.uri(), None, Some(KEY.into()), None).await.unwrap();
        let r = backends.records().get(&tag).cloned().unwrap();
        assert_eq!(r.open_subsonic, Some(true));
        assert!(r.extensions.iter().any(|e| e == ext::LYRICS), "kept for the next start");
    }
    // The next start remembers them before asking.
    let (os, _) = manager(d.path(), store.clone());
    os.restore();
    assert!(os.core(&tag).unwrap().caps().has(Caps::LYRICS));

    // A start while it's out of reach: the first ping fails, the next works.
    let d2 = tempfile::tempdir().unwrap();
    let (os, backends) = manager(d2.path(), store.clone());
    backends
        .edit_records(|f| {
            f.upsert(crate::backend::ServerRecord {
                url: Some(s.uri()),
                ..crate::backend::ServerRecord::new(&tag, BackendKind::Subsonic, "Navidrome")
            })
        })
        .unwrap();
    Mock::given(path("/rest/ping")).respond_with(ResponseTemplate::new(503)).up_to_n_times(1).with_priority(1).mount(&s).await;
    os.restore();
    let core = os.core(&tag).unwrap();
    for _ in 0..200 {
        if core.features_read() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(core.features_read(), "asked again once it answered");
    assert!(core.caps().has(Caps::LYRICS));
    assert_eq!(backends.records().get(&tag).unwrap().open_subsonic, Some(true));
}

#[tokio::test]
async fn a_revoked_sign_in_asks_to_sign_in_again() {
    let s = MockServer::start().await;
    Mock::given(any()).respond_with(failed(44, "Invalid API key")).mount(&s).await;
    let core = signed_core(&s, Features::default(), User::default(), StreamPrefs::new(None, None));
    let lib = SubsonicLibrary::all(core.clone());
    let e = lib.albums("1", 0, 10).await.unwrap_err();
    assert!(matches!(e, AppError::Auth(_)), "{e:?}");
    assert_eq!(core.state(), ServerState::SignInAgain);
    assert!(!LikeRemote::online(core.as_ref()).await);

    // Settings shows it for a server whose key is gone from the keychain.
    let d = tempfile::tempdir().unwrap();
    let (os, b) = manager(d.path(), Arc::new(MemoryStore::new()));
    b.edit_records(|f| {
        f.upsert(crate::backend::ServerRecord {
            url: Some(s.uri()),
            user: Some(USER.into()),
            ..crate::backend::ServerRecord::new(&tag_of(&s), BackendKind::Subsonic, "Navidrome")
        })
    })
    .unwrap();
    os.restore();
    let mut list = vec![crate::jellyfin::manager::SavedServer {
        tag: tag_of(&s),
        kind: BackendKind::Subsonic,
        name: "Navidrome".into(),
        url: Some(s.uri()),
        user: Some(USER.into()),
        primary: true,
        state: None,
        can_transcode: true,
        can_download: true,
    }];
    os.patch(&mut list);
    assert_eq!(list[0].state, Some(ServerState::SignInAgain));
}

#[tokio::test]
async fn form_posts_keep_the_sign_in_out_of_addresses() {
    let mut exts = all_ext();
    exts.push(ext::FORM_POST);
    let s = subsonic(Some(exts.clone()), true).await;
    let core = signed_core(&s, Features::new(true, exts.into_iter().map(str::to_string)), User::default(), StreamPrefs::new(None, None));
    let lib = SubsonicLibrary::all(core.clone());
    assert_eq!(lib.albums("1", 0, 500).await.unwrap().len(), 1);
    core.set_liked(&id(&s, "s1"), true).await.unwrap();
    for r in s.received_requests().await.unwrap() {
        assert_eq!(r.method.as_str(), "POST", "{}", r.url.path());
        assert!(r.url.query().is_none_or(|q| !q.contains(KEY)), "{}", r.url);
        assert!(String::from_utf8_lossy(&r.body).contains("apiKey="));
    }
    assert_eq!(log(&s).await, ["getAlbumList2 musicFolderId=1&offset=0&size=500&type=alphabeticalByName", "star id=s1"]);
}

// ---- requests the server sees ----------------------------------------------------------

#[tokio::test]
async fn library_sync_requests() {
    let s = subsonic(Some(all_ext()), true).await;
    let core = open_core(&s);
    let lib = SubsonicLibrary::all(core.clone());
    let t = tag_of(&s);
    let sections = lib.music_sections().await.unwrap();
    assert_eq!(sections.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["1", "2"]);
    assert_eq!(sections[0].uuid, format!("{t}/1"));
    assert_eq!(lib.count("1", ItemKind::Artist).await.unwrap(), 1);
    assert_eq!(lib.count("1", ItemKind::Album).await.unwrap(), 1, "each artist's album count");
    assert_eq!(lib.count("1", ItemKind::Track).await.unwrap(), 0, "two folders: the scan's count covers both");
    let artists = lib.artists("1", 0, 500).await.unwrap();
    assert_eq!((artists[0].id.clone(), artists[0].thumb.clone()), (format!("{t}:ar1"), Some(format!("{t}:/cover/ar-ar1"))));
    let albums = lib.albums("1", 0, 500).await.unwrap();
    assert_eq!((albums[0].id.clone(), albums[0].mbid.as_deref()), (format!("{t}:al1"), Some("mb-release")));
    let tracks = lib.tracks("1", 0, 500).await.unwrap();
    assert_eq!(tracks.iter().map(|x| x.id.clone()).collect::<Vec<_>>(), [format!("{t}:s1"), format!("{t}:s2")]);
    assert_eq!((tracks[0].user_rating, tracks[0].rated_at), (Some(10.0), Some(1_704_067_200)), "a star is a like, with its date");
    let liked = lib.liked_tracks("1").await.unwrap().unwrap();
    assert_eq!((liked[0].track_id.clone(), liked[0].rated_at), (format!("{t}:s1"), Some(1_704_067_200)));
    let playlists = lib.playlists().await.unwrap();
    assert_eq!((playlists[0].id.clone(), playlists[0].summary.as_deref()), (format!("{t}:p1"), Some("Car")));
    let rows = lib.playlist_entries(&format!("{t}:p1")).await.unwrap();
    assert_eq!(
        rows.iter().map(|r| (r.track_id.clone(), r.item_id.clone().unwrap())).collect::<Vec<_>>(),
        [(format!("{t}:s1"), "0@s1".into()), (format!("{t}:s2"), "1@s2".into()), (format!("{t}:s1"), "2@s1".into())]
    );
    // Changed since 2024-02-01: the new album (the old one ends the walk)
    // and the one played since, once, with its songs and artist.
    let changed = lib.changed_since("1", 1_706_745_600).await.unwrap();
    assert_eq!((changed.albums.len(), changed.tracks.len(), changed.artists.len()), (1, 2, 1));
    assert_eq!(lib.all_ids("1", ItemKind::Track).await.unwrap(), [format!("{t}:s1"), format!("{t}:s2")]);
    assert_eq!(lib.all_ids("1", ItemKind::Album).await.unwrap(), [format!("{t}:al1")]);
    assert!(lib.history("1", 0, 100).await.unwrap().is_empty());
    assert!(lib.items("1", &[format!("{t}:s1")]).await.unwrap().is_none(), "a sync instead");
    let similar = lib.artist_similar(&format!("{t}:ar1")).await.unwrap();
    assert_eq!((similar[0].id.clone(), similar[1].id.clone()), (Some(format!("{t}:ar9")), None));

    assert_eq!(
        log(&s).await,
        [
            "getMusicFolders",
            "getArtists musicFolderId=1",
            "getScanStatus",
            "getAlbumList2 musicFolderId=1&offset=0&size=500&type=alphabeticalByName",
            "search3 albumCount=0&artistCount=0&musicFolderId=1&query=&songCount=500&songOffset=0",
            "getStarred2 musicFolderId=1",
            "getPlaylists",
            "getPlaylist id=p1",
            "getAlbumList2 musicFolderId=1&offset=0&size=500&type=newest",
            "getAlbumList2 musicFolderId=1&offset=0&size=500&type=recent",
            "getAlbum id=al1",
            "search3 albumCount=0&artistCount=0&musicFolderId=1&query=&songCount=500&songOffset=0",
            "getAlbumList2 musicFolderId=1&offset=0&size=500&type=alphabeticalByName",
            "getArtistInfo2 count=20&id=ar1",
        ]
    );
}

#[tokio::test]
async fn a_sync_knows_how_many_songs_and_albums_to_expect() {
    let s = subsonic(Some(all_ext()), true).await;
    // One folder: the scan's count is this library's.
    Mock::given(path("/rest/getMusicFolders"))
        .and(Authed)
        .respond_with(ok(json!({"musicFolders": {"musicFolder": [{"id": 1, "name": "Music"}]}})))
        .with_priority(1)
        .mount(&s)
        .await;
    let core = open_core(&s);
    let lib = SubsonicLibrary::all(core.clone());
    lib.music_sections().await.unwrap();
    assert_eq!(lib.count("1", ItemKind::Track).await.unwrap(), 2);
    assert_eq!(lib.count("1", ItemKind::Album).await.unwrap(), 1);
    // Mid-scan, more than it has so far: a short list can't delete anything.
    Mock::given(path("/rest/getScanStatus"))
        .and(Authed)
        .respond_with(ok(json!({"scanStatus": {"scanning": true, "count": 2}})))
        .with_priority(1)
        .mount(&s)
        .await;
    assert_eq!(lib.count("1", ItemKind::Track).await.unwrap(), 3);
    assert_eq!(lib.count("1", ItemKind::Album).await.unwrap(), 2);
}

#[tokio::test]
async fn server_search_stays_in_the_included_folders() {
    let s = subsonic(Some(all_ext()), true).await;
    let core = open_core(&s);
    core.load_views().await.unwrap();
    core.set_include(Some(vec!["2".into()]));
    let hits = RemoteSearch::search(core.as_ref(), "song", None, 5).await.unwrap();
    assert_eq!(hits.tracks.len(), 2);
    let l = log(&s).await;
    assert!(l.contains(&"search3 albumCount=5&artistCount=5&musicFolderId=2&query=song&songCount=5".to_string()), "{l:?}");
}

#[tokio::test]
async fn a_song_added_to_an_album_already_here_is_seen_by_the_change_check() {
    let s = subsonic(Some(all_ext()), true).await;
    let d = tempfile::tempdir().unwrap();
    let db = crate::db::DbHandle::open(d.path().join("t.db")).unwrap();
    let t = tag_of(&s);
    // The cache has one of the album's two songs.
    db.write(|c| {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number,
               disc_number, duration_ms, part_key) VALUES (?1, 'Song 1', ?2, 'Record', 'r', 'R', 1, 1, 1000, '/p')",
            [format!("{t}:s1"), format!("{t}:al1")],
        )?;
        Ok(())
    })
    .unwrap();
    let http = client::http();
    let unsigned = SubsonicClient::new(http.clone(), &s.uri()).unwrap();
    let core = Arc::new(Core::new(&t, "Navidrome", unsigned, Arc::new(StreamPrefs::new(None, None)), Some(db)));
    core.set_features(Features::new(true, all_ext().into_iter().map(str::to_string)));
    core.sign_in(SubsonicClient::new(http, &s.uri()).unwrap().signed_in(Secret::ApiKey(Token::new(KEY).unwrap())), USER, User::default());
    let lib = SubsonicLibrary::all(core);
    // Long after the album was added and played: only its song count tells.
    let changed = lib.changed_since("1", 1_900_000_000).await.unwrap();
    assert_eq!(changed.albums.len(), 1);
    assert_eq!(changed.tracks.len(), 2);
}

#[tokio::test]
async fn long_id_lists_go_out_in_parts_when_ids_are_in_the_address() {
    let s = subsonic(Some(all_ext()), true).await;
    let core = open_core(&s);
    let ids: Vec<String> = (0..400).map(|i| id(&s, &format!("s{i}"))).collect();
    PlaylistRemote::create(core.as_ref(), "Long", &ids).await.unwrap();
    PlaylistRemote::add(core.as_ref(), &id(&s, "p1"), &ids).await.unwrap();
    ResumeQueue::save(core.as_ref(), &ids, 300, 0).await.unwrap();
    let reqs = s.received_requests().await.unwrap();
    let count = |m: &str| reqs.iter().filter(|r| r.url.path() == format!("/rest/{m}")).count();
    assert_eq!(count("createPlaylist"), 1);
    assert_eq!(count("updatePlaylist"), 2 + 3, "two parts after the create, three for the add");
    let longest = reqs.iter().map(|r| r.url.as_str().len()).max().unwrap();
    assert!(longest < 6_000, "{longest}");
    let saved = reqs.iter().find(|r| r.url.path().ends_with("savePlayQueueByIndex")).unwrap();
    let p = params(saved);
    assert_eq!(p["id"].len(), super::server::URL_IDS);
    assert_eq!(p["id"][one(&p, "currentIndex").unwrap().parse::<usize>().unwrap()], "s300", "still on the playing song");
}

/// `search3` over a long list where every tenth entry is a video.
struct LongSearch;

impl wiremock::Respond for LongSearch {
    fn respond(&self, r: &Request) -> ResponseTemplate {
        let p = params(r);
        let num = |k: &str| one(&p, k).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        let (from, count) = (num("songOffset"), num("songCount"));
        let entries: Vec<serde_json::Value> = (from..1200.min(from + count))
            .map(|i| {
                let mut s = song(&format!("s{i}"), 1);
                if i % 10 == 0 {
                    s["type"] = json!("video");
                }
                s
            })
            .collect();
        ok(json!({"searchResult3": {"song": entries}}))
    }
}

#[tokio::test]
async fn videos_in_a_full_page_of_songs_do_not_end_the_list_early() {
    let s = subsonic(Some(all_ext()), true).await;
    Mock::given(path("/rest/search3")).and(Authed).respond_with(LongSearch).with_priority(1).mount(&s).await;
    let core = open_core(&s);
    let lib = SubsonicLibrary::all(core.clone());
    let t = tag_of(&s);
    let ids = lib.all_ids("1", ItemKind::Track).await.unwrap();
    assert_eq!(ids.len(), 1080, "every song, none of the videos");
    assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), 1080, "no song twice");
    assert_eq!(ids.last(), Some(&format!("{t}:s1199")));
    // Paged the way a sync pages: full pages until the real end.
    let mut start = 0u64;
    let mut sizes = Vec::new();
    loop {
        let page = lib.tracks("1", start, 500).await.unwrap();
        sizes.push(page.len());
        start += page.len() as u64;
        if page.len() < 500 {
            break;
        }
    }
    assert_eq!(sizes, [500, 500, 80]);
}

#[tokio::test]
async fn a_server_whose_search_lists_nothing_gives_songs_album_by_album() {
    let s = subsonic(Some(all_ext()), true).await;
    Mock::given(path("/rest/search3"))
        .and(Authed)
        .respond_with(ok(json!({"searchResult3": {}})))
        .with_priority(1)
        .mount(&s)
        .await;
    let core = open_core(&s);
    let lib = SubsonicLibrary::all(core.clone());
    let tracks = lib.tracks("1", 0, 500).await.unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(
        log(&s).await,
        [
            "search3 albumCount=0&artistCount=0&musicFolderId=1&query=&songCount=500&songOffset=0",
            "getAlbumList2 musicFolderId=1&offset=0&size=1&type=alphabeticalByName",
            "getAlbumList2 musicFolderId=1&offset=0&size=500&type=alphabeticalByName",
            "getAlbum id=al1",
        ]
    );
    // Asked once: the next sync goes album by album straight away.
    let before = log(&s).await.len();
    assert_eq!(lib.tracks("1", 0, 500).await.unwrap().len(), 2);
    let after = log(&s).await;
    assert!(after[before..].iter().all(|l| !l.starts_with("search3")), "{after:?}");
}

#[tokio::test]
async fn a_server_whose_search_lists_nothing_is_checked_for_deletions_by_album_counts() {
    let s = subsonic(Some(all_ext()), true).await;
    Mock::given(path("/rest/search3")).and(Authed).respond_with(ok(json!({"searchResult3": {}}))).with_priority(1).mount(&s).await;
    let d = tempfile::tempdir().unwrap();
    let db = crate::db::DbHandle::open(d.path().join("t.db")).unwrap();
    let t = tag_of(&s);
    db.write(|c| {
        for (id, n) in [("s1", 1), ("s2", 2)] {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number,
                   disc_number, duration_ms, part_key) VALUES (?1, 'Song', ?2, 'Record', 'r', 'R', ?3, 1, 1000, '/p')",
                rusqlite::params![format!("{t}:{id}"), format!("{t}:al1"), n],
            )?;
        }
        Ok(())
    })
    .unwrap();
    let http = client::http();
    let unsigned = SubsonicClient::new(http.clone(), &s.uri()).unwrap();
    let core = Arc::new(Core::new(&t, "Navidrome", unsigned, Arc::new(StreamPrefs::new(None, None)), Some(db)));
    core.set_features(Features::new(true, all_ext().into_iter().map(str::to_string)));
    core.sign_in(SubsonicClient::new(http, &s.uri()).unwrap().signed_in(Secret::ApiKey(Token::new(KEY).unwrap())), USER, User::default());
    let lib = SubsonicLibrary::all(core);
    // The first time finds out search can't list songs.
    assert_eq!(lib.all_ids("1", ItemKind::Track).await.unwrap().len(), 2);
    let before = log(&s).await.len();
    // After that: the album list, and no album read when its count matches.
    assert_eq!(lib.all_ids("1", ItemKind::Track).await.unwrap(), [format!("{t}:s1"), format!("{t}:s2")]);
    let after = log(&s).await;
    assert!(after[before..].iter().all(|l| !l.starts_with("getAlbum ")), "{after:?}");
}

#[tokio::test]
async fn a_server_that_ignores_the_offset_does_not_page_forever() {
    let s = subsonic(Some(all_ext()), true).await;
    let page: Vec<serde_json::Value> = (0..500).map(|i| json!({ "id": format!("al{i}"), "name": format!("A{i}") })).collect();
    Mock::given(path("/rest/getAlbumList2"))
        .and(Authed)
        .and(TypeIs("alphabeticalByName"))
        .respond_with(ok(json!({"albumList2": {"album": page}})))
        .with_priority(1)
        .mount(&s)
        .await;
    let lib = SubsonicLibrary::all(open_core(&s));
    let ids = tokio::time::timeout(std::time::Duration::from_secs(10), lib.all_ids("1", ItemKind::Album)).await.expect("stops").unwrap();
    assert_eq!(ids.len(), 500);
}

#[test]
fn finished_plays_are_not_shown_as_playing_now() {
    let s = tokio::runtime::Runtime::new().unwrap().block_on(MockServer::start());
    let core = open_core(&s);
    let list: super::types::NowPlayingList = serde_json::from_value(json!({"entry": [
        { "id": "s2", "title": "Song 2", "type": "music", "username": "sam", "minutesAgo": 1, "playerName": "Phone" },
        { "id": "s3", "title": "Song 3", "type": "music", "username": "kim", "minutesAgo": 45, "playerName": "Phone" }
    ]}))
    .unwrap();
    let v = core.sessions_json(&list);
    let rows = v["MediaContainer"]["Metadata"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["User"]["title"], "sam");
}

#[tokio::test]
async fn change_checks_wait_for_the_scan_and_spot_a_new_index() {
    let s = subsonic(Some(all_ext()), true).await;
    let core = open_core(&s);
    assert!(!core.poll_changes().await.unwrap(), "the first ask only remembers");
    assert!(!core.poll_changes().await.unwrap(), "nothing changed");
    Mock::given(path("/rest/getIndexes")).and(Authed).respond_with(ok(json!({"indexes": {"lastModified": 2000}}))).with_priority(1).mount(&s).await;
    assert!(core.poll_changes().await.unwrap(), "the index changed");
    assert!(!core.poll_changes().await.unwrap());
    Mock::given(path("/rest/getScanStatus")).and(Authed).respond_with(ok(json!({"scanStatus": {"scanning": true}}))).with_priority(1).mount(&s).await;
    assert!(!core.poll_changes().await.unwrap(), "a scan still running waits");
    let l = log(&s).await;
    assert_eq!(&l[..2], ["getScanStatus", "getIndexes ifModifiedSince=0"]);
    assert_eq!(l[3], "getIndexes ifModifiedSince=1000");
    assert_eq!(l[5], "getIndexes ifModifiedSince=1000");
    assert_eq!(l[7], "getIndexes ifModifiedSince=2000");
    assert_eq!(l.last().unwrap(), "getScanStatus");
}

#[tokio::test]
async fn streams_conversions_downloads_and_file_reads() {
    let s = subsonic(Some(all_ext()), true).await;
    // Songs over 128 kbps are converted on this network.
    let core = signed_core(&s, Features::new(true, all_ext().into_iter().map(str::to_string)), User::default(), StreamPrefs::new(Some(128), None));
    let t = super::map::track(&tag_of(&s), &serde_json::from_value(song("s1", 1)).unwrap());
    let q = |u: &str| -> BTreeMap<String, String> {
        url::Url::parse(u).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
    };
    let capped = core.stream(&t).await.unwrap();
    assert!(capped.url.contains("/rest/stream?"), "{capped:?}");
    let cq = q(&capped.url);
    assert_eq!((cq["id"].as_str(), cq["format"].as_str(), cq["maxBitRate"].as_str()), ("s1", "mp3", "128"));
    assert_eq!(cq["apiKey"], KEY);
    assert!(!format!("{capped:?}").contains(KEY), "a stream's Debug never shows its address");
    let full = open_core(&s).stream(&t).await.unwrap();
    assert_eq!(q(&full.url)["format"], "raw");
    assert_eq!(full.codec_hint.as_deref(), Some("flac"));
    let conv = core.transcode(&t).await.unwrap();
    assert_eq!((q(&conv.url)["format"].as_str(), q(&conv.url)["maxBitRate"].as_str()), ("mp3", "320"));
    let dl = DownloadSource::fetch(core.as_ref(), &t, None).await.unwrap();
    assert!(dl.resumable && dl.src.url.contains("/rest/download?"));
    let small = DownloadSource::fetch(core.as_ref(), &t, Some(192)).await.unwrap();
    assert!(!small.resumable && q(&small.src.url)["maxBitRate"] == "192");
    let part = core.part(&t.id, &t.part_key).await.unwrap();
    assert_eq!((q(&part.url)["id"].as_str(), q(&part.url)["format"].as_str()), ("s1", "raw"));
    // ReplayGain from the song's details.
    let a = core.audio_info(&t).await.unwrap();
    assert_eq!((a.gain_db, a.album_gain_db, a.peak), (Some(-7.5), Some(-6.0), Some(0.98)));
    // Cover art, resized by the server.
    let art = ArtFetcher::fetch(core.as_ref(), &format!("{}:/cover/al-al1", tag_of(&s)), 300, 200).await.unwrap();
    assert_eq!(art.content_type, "image/png");
    assert_eq!(log(&s).await, ["getSong id=s1", "getCoverArt id=al-al1&size=300"]);
}

#[tokio::test]
async fn likes_ratings_and_playlist_edits() {
    let s = subsonic(Some(all_ext()), true).await;
    let core = open_core(&s);
    let t = tag_of(&s);
    let pid = format!("{t}:p1");
    core.set_liked(&id(&s, "s1"), true).await.unwrap();
    core.set_liked(&id(&s, "s1"), false).await.unwrap();
    core.rate(&id(&s, "s2"), Some(8.0)).await.unwrap();
    core.rate(&id(&s, "s2"), None).await.unwrap();
    let made = PlaylistRemote::create(core.as_ref(), "New", &[id(&s, "s1"), id(&s, "s2")]).await.unwrap();
    assert_eq!(made, format!("{t}:p7"));
    core.add(&pid, &[id(&s, "s2")]).await.unwrap();
    // The row at index 2 (s1) is still s1 there: removed by that index.
    core.remove(&pid, "2@s1").await.unwrap();
    // The row once at 1 holding s1 moved: the nearest s1 now is index 0.
    core.remove(&pid, "1@s1").await.unwrap();
    assert!(core.remove(&pid, "0@gone").await.is_err(), "a song no longer there");
    // Move the first row after the second: written again in the new order.
    core.move_item(&pid, "0@s1", Some("1@s2")).await.unwrap();
    core.update(&pid, Some("Renamed"), Some("New words")).await.unwrap();
    assert!(core.poster(&pid, vec![1], "image/png").await.is_err());
    PlaylistRemote::delete(core.as_ref(), &pid).await.unwrap();
    let (pl, rows) = PlaylistRemote::fetch(core.as_ref(), &pid).await.unwrap();
    assert_eq!((pl.title.as_str(), rows.len()), ("Mine", 3));
    assert_eq!(
        log(&s).await,
        [
            "star id=s1",
            "unstar id=s1",
            "setRating id=s2&rating=4",
            "setRating id=s2&rating=0",
            "createPlaylist name=New&songId=s1&songId=s2",
            "updatePlaylist playlistId=p1&songIdToAdd=s2",
            "getPlaylist id=p1",
            "updatePlaylist playlistId=p1&songIndexToRemove=2",
            "getPlaylist id=p1",
            "updatePlaylist playlistId=p1&songIndexToRemove=0",
            "getPlaylist id=p1",
            "getPlaylist id=p1",
            "createPlaylist playlistId=p1&songId=s1&songId=s1&songId=s2",
            "updatePlaylist comment=New words&name=Renamed&playlistId=p1",
            "deletePlaylist id=p1",
            "getPlaylist id=p1",
        ]
    );
}

#[tokio::test]
async fn plays_are_reported_and_counted_once() {
    let s = subsonic(Some(all_ext()), true).await;
    let core = open_core(&s);
    let report = |state, ms| PlayReport { track_id: id(&s, "s1"), state, time_ms: ms, duration_ms: 200_000 };
    core.timeline(&report(PlaybackState::Playing, 0)).await.unwrap();
    core.timeline(&report(PlaybackState::Playing, 10_000)).await.unwrap();
    core.timeline(&report(PlaybackState::Paused, 12_000)).await.unwrap();
    core.timeline(&report(PlaybackState::Stopped, 12_000)).await.unwrap();
    // A stop for a song never reported playing sends nothing.
    core.timeline(&PlayReport { track_id: id(&s, "s2"), state: PlaybackState::Stopped, time_ms: 0, duration_ms: 1 }).await.unwrap();
    core.scrobble(&id(&s, "s1"), Some(1_700_000_000)).await.unwrap();
    let r = |state: &str, ms: &str| format!("reportPlayback ignoreScrobble=true&mediaId=s1&mediaType=song&positionMs={ms}&state={state}");
    assert_eq!(
        log(&s).await,
        [
            r("starting", "0"),
            r("playing", "0"),
            r("playing", "10000"),
            r("paused", "12000"),
            r("stopped", "12000"),
            "scrobble id=s1&submission=true&time=1700000000000".to_string(),
        ]
    );
}

#[tokio::test]
async fn lyrics_art_radio_activity_and_the_saved_queue() {
    let s = subsonic(Some(all_ext()), true).await;
    let core = open_core(&s);
    let server = SubsonicServer::new(core.clone());
    let t = tag_of(&s);
    let l = server.lyrics().unwrap().lyrics(&id(&s, "s1")).await.unwrap().unwrap();
    assert!(l.synced && l.timed[0].start_ms == 1000);
    // Radio: sonically similar first.
    assert_eq!(server.radio().unwrap().mix(&id(&s, "s1"), 10).await.unwrap(), [id(&s, "s2")]);
    // Search, while the first sync runs.
    let hits = core.search("song", None, 5).await.unwrap();
    assert_eq!(hits.tracks.len(), 2);
    // What others play: not this app's own report.
    let raw = crate::social::activity::ActivitySource::sessions(core.as_ref()).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let rows = v["MediaContainer"]["Metadata"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0]["ratingKey"].as_str(), rows[0]["User"]["title"].as_str()), (Some(id(&s, "s2").as_str()), Some("sam")));
    assert_eq!(crate::social::activity::ActivitySource::owned(core.as_ref()).await, Some(true));
    let acc: serde_json::Value = serde_json::from_slice(&crate::social::activity::ActivitySource::accounts(core.as_ref()).await.unwrap()).unwrap();
    assert_eq!(acc["MediaContainer"]["Account"].as_array().unwrap().len(), 2);
    // MusicBrainz ids.
    use crate::artistinfo::service::MbidLookup;
    assert_eq!(core.artist_mbid(&format!("{t}:ar1")).await.unwrap().as_deref(), Some("mb-artist"));
    assert_eq!(core.album_mbid(&format!("{t}:al1")).await.unwrap().as_deref(), Some("mb-release"));
    // The saved queue, by index.
    let rq = server.resume().unwrap();
    rq.save(&[id(&s, "s1"), "0badf00d:other".into(), id(&s, "s2")], 1, 4000).await.unwrap();
    let q = rq.load().await.unwrap().unwrap();
    assert_eq!(q.track_ids, [id(&s, "s1"), id(&s, "s2")]);
    assert_eq!((q.current, q.position_ms, q.changed_by.as_deref()), (1, 4000, Some("Phone app")));
    assert_eq!(
        log(&s).await,
        [
            "getLyricsBySongId id=s1",
            "getSonicSimilarTracks count=10&id=s1",
            "search3 albumCount=5&artistCount=5&query=song&songCount=5",
            "getNowPlaying",
            "getNowPlaying",
            "getArtist id=ar1",
            "getAlbum id=al1",
            "savePlayQueueByIndex currentIndex=1&id=s1&id=s2&position=4000",
            "getPlayQueueByIndex",
        ]
    );
}

#[tokio::test]
async fn radio_falls_back_to_similar_then_top_songs() {
    let s = subsonic(Some(vec![ext::API_KEY]), true).await;
    let core = signed_core(&s, Features::new(true, [ext::API_KEY.to_string()]), User::default(), StreamPrefs::new(None, None));
    assert_eq!(core.mix(&id(&s, "s1"), 10).await.unwrap(), [id(&s, "s2"), id(&s, "s9")]);
    Mock::given(path("/rest/getSimilarSongs")).and(Authed).respond_with(ok(json!({"similarSongs": {}}))).with_priority(1).mount(&s).await;
    assert_eq!(core.mix(&id(&s, "ar1"), 10).await.unwrap(), [id(&s, "s2")]);
    let l = log(&s).await;
    assert_eq!(l, ["getSimilarSongs count=10&id=s1", "getSimilarSongs count=10&id=ar1", "getArtist id=ar1", "getTopSongs artist=Band&count=10"]);
}

#[tokio::test]
async fn routed_through_the_servers_by_tag() {
    let s = subsonic(Some(all_ext()), true).await;
    let core = open_core(&s);
    let backends = Backends::empty();
    backends.add(&tag_of(&s), Arc::new(SubsonicServer::new(core.clone())));
    let srv = backends.for_id(&id(&s, "s1")).unwrap();
    assert_eq!(srv.kind(), BackendKind::Subsonic);
    assert!(backends.caps_for(&id(&s, "s1")).has(Caps::RESUME_QUEUE));
    // The queue saver keeps this server's songs.
    assert!(crate::backend::resume::save(&backends, &[id(&s, "s1"), id(&s, "s2")], 500).await.unwrap());
    let offers = crate::backend::resume::offers(&backends).await;
    assert_eq!(offers.len(), 1);
    assert_eq!((offers[0].track_count, offers[0].current_id.clone()), (2, id(&s, "s2")));
    assert!(log(&s).await.contains(&"savePlayQueueByIndex currentIndex=0&id=s1&id=s2&position=500".to_string()));
}

#[tokio::test]
async fn art_paths_that_are_not_covers_are_refused() {
    let s = subsonic(Some(all_ext()), true).await;
    let core = open_core(&s);
    assert!(ArtFetcher::fetch(core.as_ref(), &format!("{}:/elsewhere/1", tag_of(&s)), 10, 10).await.is_err());
    // An error answer instead of a picture.
    Mock::given(path("/rest/getCoverArt")).and(Authed).respond_with(failed(70, "not found")).with_priority(1).mount(&s).await;
    assert!(matches!(ArtFetcher::fetch(core.as_ref(), &format!("{}:/cover/x", tag_of(&s)), 10, 10).await, Err(AppError::NotFound(_))));
}

#[tokio::test]
async fn go_offline_sends_nothing() {
    let s = subsonic(Some(all_ext()), true).await;
    let core = open_core(&s);
    let off = Arc::new(std::sync::atomic::AtomicBool::new(true));
    core.set_offline_switch(off);
    assert!(matches!(SubsonicLibrary::all(core.clone()).albums("1", 0, 1).await, Err(AppError::Network(_))));
    assert!(s.received_requests().await.unwrap().is_empty());
}

/// Against a throwaway Navidrome (never anyone's real server), with
/// CC-licensed music only. Set PLATTER_TEST_SUBSONIC_URL and either
/// PLATTER_TEST_SUBSONIC_KEY, or PLATTER_TEST_SUBSONIC_USER and
/// PLATTER_TEST_SUBSONIC_PASSWORD, then run with `--ignored`.
#[tokio::test]
#[ignore]
async fn live_throwaway_navidrome() {
    let Ok(url) = std::env::var("PLATTER_TEST_SUBSONIC_URL") else {
        eprintln!("PLATTER_TEST_SUBSONIC_URL not set");
        return;
    };
    let key = std::env::var("PLATTER_TEST_SUBSONIC_KEY").ok();
    let user = std::env::var("PLATTER_TEST_SUBSONIC_USER").ok();
    let pw = std::env::var("PLATTER_TEST_SUBSONIC_PASSWORD").ok();
    let d = tempfile::tempdir().unwrap();
    let (os, backends) = manager(d.path(), Arc::new(MemoryStore::new()));
    let check = os.check(&url).await.unwrap();
    eprintln!("server: {} {:?} open={} keys={}", check.name, check.version, check.open_subsonic, check.api_keys);
    let saved = os.sign_in(&url, user, key, pw).await.unwrap();
    let core = os.core(&saved.tag).unwrap();
    // Let the background start read the extensions.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    eprintln!("caps: {}", serde_json::to_string(&core.caps()).unwrap());
    let lib = SubsonicLibrary::all(core.clone());
    let sections = lib.music_sections().await.unwrap();
    let sec = &sections[0].id;
    let artists = lib.artists(sec, 0, 500).await.unwrap();
    let albums = lib.albums(sec, 0, 500).await.unwrap();
    let tracks = lib.tracks(sec, 0, 500).await.unwrap();
    eprintln!("{} artists, {} albums, {} songs", artists.len(), albums.len(), tracks.len());
    assert!(!tracks.is_empty(), "put some CC-licensed music in the test server");
    let t = &tracks[0];
    let src = core.stream(t).await.unwrap();
    let resp = reqwest::Client::new().get(&src.url).header("Range", "bytes=0-1023").send().await.unwrap();
    eprintln!("ranged read: {}", resp.status());
    assert!(resp.status().is_success());
    core.set_liked(&t.id, true).await.unwrap();
    let liked = lib.liked_tracks(sec).await.unwrap().unwrap();
    assert!(liked.iter().any(|r| r.track_id == t.id && r.rated_at.is_some()));
    core.set_liked(&t.id, false).await.unwrap();
    let pl = PlaylistRemote::create(core.as_ref(), "Platter test", std::slice::from_ref(&t.id)).await.unwrap();
    PlaylistRemote::delete(core.as_ref(), &pl).await.unwrap();
    core.scrobble(&t.id, None).await.unwrap();
    eprintln!("change check: {:?}", core.poll_changes().await);
    eprintln!("lyrics: {:?}", crate::lyrics::ServerLyrics::lyrics(core.as_ref(), &t.id).await.map(|l| l.is_some()));
    os.remove(&saved.tag).await.unwrap();
    drop(backends);
}

/// A song tagged with a broken character (half of an emoji) loads with a
/// replacement mark; the server isn't taken for something else.
#[test]
fn a_tag_with_a_broken_character_does_not_stop_the_answer() {
    let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","song":{"id":"s1","title":"Fire \ud83c"}}}"#;
    let (status, v) = super::client::envelope(body).unwrap();
    assert_eq!(status.status, "ok");
    assert_eq!(v["song"]["title"], "Fire \u{fffd}");
}
