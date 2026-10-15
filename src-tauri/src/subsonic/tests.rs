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
use super::manager::{Deps, Subsonic, WRONG_URL, key_account};
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
    assert_eq!(lib.count("1", ItemKind::Album).await.unwrap(), 0, "counted as they come");
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
            "getAlbumList2 musicFolderId=1&offset=0&size=500&type=alphabeticalByName",
            "search3 albumCount=0&artistCount=0&musicFolderId=1&query=&songCount=500&songOffset=0",
            "getStarred2 musicFolderId=1",
            "getPlaylists",
            "getPlaylist id=p1",
            "getAlbumList2 musicFolderId=1&offset=0&size=500&type=newest",
            "getAlbumList2 musicFolderId=1&offset=0&size=50&type=recent",
            "getAlbum id=al1",
            "search3 albumCount=0&artistCount=0&musicFolderId=1&query=&songCount=500&songOffset=0",
            "getAlbumList2 musicFolderId=1&offset=0&size=500&type=alphabeticalByName",
            "getArtistInfo2 count=20&id=ar1",
        ]
    );
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
