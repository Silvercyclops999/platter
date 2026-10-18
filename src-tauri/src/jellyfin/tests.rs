//! A faked Jellyfin server (wiremock) for every call the app makes and
//! every way signing in or playing can fail. The request lists are the
//! exact requests the server sees; tokens only ever travel in the header.

use std::sync::Arc;

use serde_json::json;
use wiremock::matchers::{any, body_partial_json, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::auth;
use super::client::{Identity, JellyfinClient, NOT_JELLYFIN};
use super::likes::LikeDates;
use super::manager::{Deps, Jellyfin, QuickPoll, WRONG_URL};
use super::server::{Core, JellyfinLibrary, JellyfinServer, NO_DOWNLOAD, NO_TRANSCODE, ServerState};
use super::types::{Policy, User};
use crate::audio::source::TrackResolver;
use crate::backend::server::MusicServer;
use crate::backend::{BackendKind, Backends, Caps};
use crate::bridge::StreamPrefs;
use crate::db::source::{ArtFetcher, ItemKind, LibrarySource, RemoteSearch};
use crate::downloads::DownloadSource;
use crate::error::AppError;
use crate::likes::LikeRemote;
use crate::playlists::PlaylistRemote;
use crate::plex::keychain::{MemoryStore, TokenStore};
use crate::plex::token::Token;
use crate::tags::scan::PartSource;
use crate::tracking::{PlayReport, PlayReporter, PlaybackState};

const SERVER_ID: &str = "4a5b6c7d8e9f40a1b2c3d4e5f6a7b8c9";
const TOKEN: &str = "jf-secret-token-abc";
const DEVICE: &str = "device-1";

fn tag() -> String {
    BackendKind::Jellyfin.tag(SERVER_ID)
}

fn ok(v: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(v)
}

fn policy(admin: bool, transcode: bool, download: bool) -> serde_json::Value {
    json!({ "IsAdministrator": admin, "EnableAudioPlaybackTranscoding": transcode, "EnableContentDownloading": download })
}

fn user_json(admin: bool, transcode: bool, download: bool) -> serde_json::Value {
    json!({ "Id": "u1", "Name": "me", "Policy": policy(admin, transcode, download) })
}

fn artist() -> serde_json::Value {
    json!({ "Id": "ar1", "Name": "Band", "Type": "MusicArtist", "ImageTags": {"Primary": "ai"} })
}

fn album() -> serde_json::Value {
    json!({ "Id": "al1", "Name": "Record", "Type": "MusicAlbum", "AlbumArtist": "Band",
        "AlbumArtists": [{"Name": "Band", "Id": "ar1"}], "ChildCount": 2, "ImageTags": {"Primary": "img"},
        "ProviderIds": {"MusicBrainzAlbum": "mb-release"} })
}

fn song(id: &str, n: u32) -> serde_json::Value {
    json!({ "Id": id, "Name": format!("Song {n}"), "Type": "Audio", "AlbumId": "al1", "Album": "Record",
        "AlbumArtist": "Band", "AlbumArtists": [{"Name": "Band", "Id": "ar1"}], "Artists": ["Band"],
        "AlbumPrimaryImageTag": "img", "IndexNumber": n, "ParentIndexNumber": 1, "RunTimeTicks": 2_000_000_000u64,
        "UserData": {"IsFavorite": n == 1, "PlayCount": 2},
        "MediaSources": [{"Container": "flac", "Bitrate": 900_000, "Size": 1000,
            "Path": format!("/music/Band/Record/0{n}.flac"),
            "MediaStreams": [{"Type": "Audio", "Codec": "flac", "BitDepth": 16, "SampleRate": 44100}]}] })
}

fn page(items: Vec<serde_json::Value>) -> serde_json::Value {
    let n = items.len();
    json!({ "Items": items, "TotalRecordCount": n, "StartIndex": 0 })
}

async fn mount(s: &MockServer, m: &str, p: &str, body: serde_json::Value) {
    Mock::given(method(m)).and(path(p)).respond_with(ok(body)).mount(s).await;
}

/// A Jellyfin server with one music library holding one album of two songs.
async fn jellyfin(admin: bool, transcode: bool, download: bool) -> MockServer {
    let s = MockServer::start().await;
    mount(&s, "GET", "/System/Info/Public", json!({
        "Id": SERVER_ID, "ServerName": "Den", "Version": "10.10.3", "ProductName": "Jellyfin Server"
    }))
    .await;
    mount(&s, "GET", "/QuickConnect/Enabled", json!(true)).await;
    mount(&s, "POST", "/QuickConnect/Initiate", json!({"Secret": "qc-secret", "Code": "123456", "Authenticated": false})).await;
    // Approved on the second ask.
    Mock::given(method("GET"))
        .and(path("/QuickConnect/Connect"))
        .and(query_param("Secret", "qc-secret"))
        .respond_with(ok(json!({"Authenticated": false, "Secret": "qc-secret", "Code": "123456"})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/QuickConnect/Connect"))
        .and(query_param("Secret", "qc-secret"))
        .respond_with(ok(json!({"Authenticated": true, "Secret": "qc-secret", "Code": "123456"})))
        .mount(&s)
        .await;
    let auth = json!({ "User": user_json(admin, transcode, download), "AccessToken": TOKEN, "ServerId": SERVER_ID });
    Mock::given(method("POST"))
        .and(path("/Users/AuthenticateWithQuickConnect"))
        .and(body_partial_json(json!({"Secret": "qc-secret"})))
        .respond_with(ok(auth.clone()))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/Users/AuthenticateByName"))
        .and(body_partial_json(json!({"Username": "me", "Pw": "right"})))
        .respond_with(ok(auth))
        .mount(&s)
        .await;
    Mock::given(method("POST")).and(path("/Users/AuthenticateByName")).respond_with(ResponseTemplate::new(401)).with_priority(10).mount(&s).await;
    mount(&s, "GET", "/Users/Me", user_json(admin, transcode, download)).await;
    mount(&s, "GET", "/UserViews", json!({"Items": [
        {"Id": "v1", "Name": "Music", "CollectionType": "music"},
        {"Id": "v2", "Name": "Films", "CollectionType": "movies"},
        {"Id": "v3", "Name": "Live tapes", "CollectionType": "music"}
    ]}))
    .await;
    // Lists answer on the first page only.
    let list = |types: &'static str, items: serde_json::Value| {
        Mock::given(method("GET"))
            .and(path("/Items"))
            .and(query_param("IncludeItemTypes", types))
            .and(query_param("StartIndex", "0"))
            .respond_with(ok(items))
    };
    list("MusicAlbum", page(vec![album()])).mount(&s).await;
    list("Audio", page(vec![song("t1", 1), song("t2", 2)])).mount(&s).await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("Filters", "IsFavorite"))
        .and(query_param("StartIndex", "0"))
        .respond_with(ok(page(vec![song("t1", 1)])))
        .with_priority(1)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("IncludeItemTypes", "Playlist"))
        .respond_with(ok(page(vec![
            json!({"Id": "p1", "Name": "Mine", "Type": "Playlist", "MediaType": "Audio", "ChildCount": 2}),
            json!({"Id": "p2", "Name": "Films", "Type": "Playlist", "MediaType": "Video"}),
        ])))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("Ids", "t1,al1"))
        .respond_with(ok(page(vec![song("t1", 1), album()])))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("Ids", "ar1"))
        .respond_with(ok(page(vec![artist()])))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/Artists/AlbumArtists"))
        .and(query_param("StartIndex", "0"))
        .respond_with(ok(page(vec![artist()])))
        .mount(&s)
        .await;
    mount(&s, "GET", "/Artists/ar1/Similar", page(vec![json!({"Id": "ar9", "Name": "Others"})])).await;
    mount(&s, "GET", "/Playlists/p1/Items", page(vec![
        json!({"Id": "t1", "Type": "Audio", "PlaylistItemId": "e1"}),
        json!({"Id": "t2", "Type": "Audio", "PlaylistItemId": "e2"}),
    ]))
    .await;
    mount(&s, "GET", "/Items/p1", json!({"Id": "p1", "Name": "Mine", "Type": "Playlist", "MediaType": "Audio", "ChildCount": 2})).await;
    mount(&s, "GET", "/Items/t1", json!({"Id": "t1", "Type": "Audio", "NormalizationGain": -7.5})).await;
    mount(&s, "GET", "/Items/ar1", json!({"Id": "ar1", "Type": "MusicArtist", "ProviderIds": {"MusicBrainzArtist": "mb-artist"}})).await;
    mount(&s, "GET", "/Items/al1", album()).await;
    mount(&s, "POST", "/Playlists", json!({"Id": "p7"})).await;
    mount(&s, "GET", "/Audio/t1/Lyrics", json!({"Metadata": {}, "Lyrics": [{"Text": "La", "Start": 10_000_000u64}]})).await;
    mount(&s, "GET", "/Items/t1/InstantMix", page(vec![song("t2", 2), song("t9", 9)])).await;
    mount(&s, "GET", "/Sessions", json!([
        {"UserId": "u2", "UserName": "Sam", "DeviceId": "other", "NowPlayingItem": song("t2", 2), "PlayState": {"IsPaused": false}},
        {"UserId": "u1", "UserName": "me", "DeviceId": DEVICE, "NowPlayingItem": song("t1", 1)}
    ]))
    .await;
    mount(&s, "GET", "/Users", json!([{"Id": "u1", "Name": "me"}, {"Id": "u2", "Name": "Sam"}])).await;
    mount(&s, "GET", "/Library/VirtualFolders", json!([
        {"Name": "Music", "ItemId": "v1", "Locations": ["/music"]},
        {"Name": "Films", "ItemId": "v2", "Locations": ["/films"]}
    ]))
    .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/Items/al1/Images/Primary$"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"\x89PNG\r\n\x1a\nrest".to_vec()).insert_header("content-type", "image/png"))
        .mount(&s)
        .await;
    // Everything else: fine, nothing to say.
    Mock::given(any()).respond_with(ok(page(vec![]))).with_priority(100).mount(&s).await;
    s
}

fn identity() -> Arc<Identity> {
    Arc::new(Identity { device_id: DEVICE.into(), device_name: "Test".into(), version: "1.0".into() })
}

fn signed_core(s: &MockServer, admin: bool, transcode: bool, download: bool) -> Arc<Core> {
    let http = super::client::http();
    let unsigned = JellyfinClient::new(http.clone(), &s.uri(), identity()).unwrap();
    let core = Arc::new(Core::new(&tag(), "Den", unsigned, Arc::new(StreamPrefs::new(None, None)), None, LikeDates::open(None)));
    let client = JellyfinClient::new(http, &s.uri(), identity()).unwrap().signed_in(Token::new(TOKEN).unwrap(), "u1");
    let user = User {
        id: "u1".into(),
        name: Some("me".into()),
        server_id: None,
        policy: Some(Policy {
            is_administrator: admin,
            enable_audio_playback_transcoding: transcode,
            enable_content_downloading: download,
            ..Default::default()
        }),
    };
    core.sign_in(client, user);
    core
}

/// `METHOD /path?k=v&...` with the query sorted.
async fn log(s: &MockServer) -> Vec<String> {
    s.received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            let mut q: Vec<String> = r.url.query_pairs().map(|(k, v)| format!("{k}={v}")).collect();
            q.sort();
            let q = if q.is_empty() { String::new() } else { format!("?{}", q.join("&")) };
            format!("{} {}{q}", r.method, r.url.path())
        })
        .collect()
}

fn req(m: &str, p: &str, q: &[(&str, &str)]) -> String {
    let mut q: Vec<String> = q.iter().map(|(k, v)| format!("{k}={v}")).collect();
    q.sort();
    let q = if q.is_empty() { String::new() } else { format!("?{}", q.join("&")) };
    format!("{m} {p}{q}")
}

/// No request put the token in its address; every signed-in request
/// carried it in the header.
async fn token_only_in_headers(s: &MockServer) {
    for r in s.received_requests().await.unwrap() {
        assert!(!r.url.as_str().contains(TOKEN), "token in {}", r.url.path());
        assert!(!r.url.as_str().to_ascii_lowercase().contains("api_key"), "api_key in {}", r.url.path());
        let h = r.headers.get("authorization").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
        assert!(h.starts_with("MediaBrowser Client=\"Platter\""), "{} {}", r.method, r.url.path());
        assert!(h.contains("DeviceId=\""), "{h}");
    }
}

async fn clear(s: &MockServer) {
    s.reset().await;
}

fn manager(dir: &std::path::Path, store: Arc<MemoryStore>) -> (Jellyfin, Backends) {
    let backends = Backends::empty();
    backends.keep_records_in(dir.to_path_buf());
    let jf = Jellyfin::new(Deps {
        backends: backends.clone(),
        db: None,
        dir: dir.to_path_buf(),
        store,
        http: super::client::http(),
        device_id: DEVICE.into(),
        prefs: Arc::new(StreamPrefs::new(None, None)),
        offline: Arc::default(),
        live: false,
    });
    (jf, backends)
}

// ---- signing in ------------------------------------------------------------------

#[tokio::test]
async fn quick_connect_signs_in_and_only_the_token_is_kept_in_the_keychain() {
    let s = jellyfin(true, true, true).await;
    let d = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let (jf, backends) = manager(d.path(), store.clone());

    let check = jf.check(&format!("{}/web/index.html#!/home", s.uri())).await.unwrap();
    assert_eq!((check.url.as_str(), check.name.as_str(), check.quick_connect), (s.uri().as_str(), "Den", true));
    let code = jf.quick_start(&s.uri()).await.unwrap();
    assert_eq!((code.code.as_str(), code.server_name.as_str()), ("123456", "Den"));
    assert_eq!(jf.quick_poll().await.unwrap(), QuickPoll::Waiting);
    let QuickPoll::Done { server } = jf.quick_poll().await.unwrap() else { panic!("not signed in") };
    assert_eq!(server.tag, tag());
    assert_eq!((server.kind, server.name.as_str(), server.user.as_deref()), (BackendKind::Jellyfin, "Den", Some("me")));
    assert!(server.primary, "the first server added is primary");
    assert!(jf.quick_poll().await.is_err(), "the code is used up");

    // The token is in the keychain under jf.<tag>; the saved list has no secret.
    assert_eq!(store.get(&format!("jf.{}", tag())).unwrap().unwrap().expose(), TOKEN);
    let saved = std::fs::read_to_string(d.path().join(crate::backend::registry::SERVERS_FILE)).unwrap();
    assert!(!saved.contains(TOKEN) && !saved.contains("qc-secret"), "{saved}");
    assert!(saved.contains("\"userId\": \"u1\""), "{saved}");
    let srv = backends.server(&tag()).expect("in use");
    assert_eq!(srv.kind(), BackendKind::Jellyfin);
    assert_eq!(backends.primary().unwrap().tag().await, Some(tag()));
    assert_eq!(backends.main_server().unwrap().tag().await, Some(tag()), "no Plex: Jellyfin has the main library");

    // The sign-in requests carried no token; the secret only went where
    // Jellyfin asks for it.
    let reqs = s.received_requests().await.unwrap();
    let initiate = reqs.iter().find(|r| r.url.path() == "/QuickConnect/Initiate").unwrap();
    assert!(!initiate.headers.get("authorization").unwrap().to_str().unwrap().contains("Token="));
    let finish = reqs.iter().find(|r| r.url.path() == "/Users/AuthenticateWithQuickConnect").unwrap();
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&finish.body).unwrap(), json!({"Secret": "qc-secret"}));
    token_only_in_headers(&s).await;

    // Restored from the saved list on the next start, with its token.
    let (again, b2) = manager(d.path(), store.clone());
    again.restore();
    let core = again.core(&tag()).unwrap();
    assert!(core.signed_in());
    assert_eq!(b2.server(&tag()).unwrap().tag().await, Some(tag()));
}

#[tokio::test]
async fn quick_connect_turned_off_says_so() {
    let s = MockServer::start().await;
    mount(&s, "GET", "/System/Info/Public", json!({"Id": SERVER_ID, "ServerName": "Den", "Version": "10.9.11", "ProductName": "Jellyfin Server"})).await;
    mount(&s, "GET", "/QuickConnect/Enabled", json!(false)).await;
    let d = tempfile::tempdir().unwrap();
    let (jf, _) = manager(d.path(), Arc::new(MemoryStore::new()));
    assert!(!jf.check(&s.uri()).await.unwrap().quick_connect);
    let e = jf.quick_start(&s.uri()).await.unwrap_err();
    assert_eq!(e.to_string(), format!("invalid: {}", auth::QUICK_CONNECT_OFF));

    // A server that says it's on but refuses to start one.
    let s2 = MockServer::start().await;
    mount(&s2, "GET", "/System/Info/Public", json!({"Id": SERVER_ID, "Version": "10.9.11"})).await;
    mount(&s2, "GET", "/QuickConnect/Enabled", json!(true)).await;
    Mock::given(method("POST")).and(path("/QuickConnect/Initiate")).respond_with(ResponseTemplate::new(401)).mount(&s2).await;
    assert_eq!(jf.quick_start(&s2.uri()).await.unwrap_err().to_string(), format!("invalid: {}", auth::QUICK_CONNECT_OFF));
}

#[tokio::test]
async fn password_sign_in_uses_the_password_once() {
    let s = jellyfin(false, true, true).await;
    let d = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let (jf, _) = manager(d.path(), store.clone());
    let e = jf.password(&s.uri(), "me", "wrong".into()).await.unwrap_err();
    assert_eq!(e.to_string(), format!("auth: {}", auth::WRONG_PASSWORD));
    assert!(store.get(&format!("jf.{}", tag())).unwrap().is_none());
    assert!(jf.password(&s.uri(), " ", "x".into()).await.is_err(), "a user name is needed");

    let saved = jf.password(&s.uri(), "me", "right".into()).await.unwrap();
    assert_eq!(saved.tag, tag());
    assert_eq!(store.get(&format!("jf.{}", tag())).unwrap().unwrap().expose(), TOKEN);
    let file = std::fs::read_to_string(d.path().join(crate::backend::registry::SERVERS_FILE)).unwrap();
    assert!(!file.contains("right") && !file.contains(TOKEN), "{file}");
    let likes_dir = d.path().join("jellyfin");
    if likes_dir.exists() {
        for f in std::fs::read_dir(likes_dir).unwrap() {
            assert!(!std::fs::read_to_string(f.unwrap().path()).unwrap().contains("right"));
        }
    }
    token_only_in_headers(&s).await;
}

#[tokio::test]
async fn wrong_address_not_jellyfin_and_too_old() {
    let d = tempfile::tempdir().unwrap();
    let (jf, _) = manager(d.path(), Arc::new(MemoryStore::new()));
    // Nothing listening there.
    let url = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://127.0.0.1:{}", l.local_addr().unwrap().port())
    };
    assert_eq!(jf.check(&url).await.unwrap_err().to_string(), format!("invalid: {WRONG_URL}"));
    assert!(jf.check("").await.is_err());

    // A web server that isn't Jellyfin.
    let web = MockServer::start().await;
    Mock::given(any()).respond_with(ResponseTemplate::new(200).set_body_string("<html>hello</html>")).mount(&web).await;
    assert_eq!(jf.check(&web.uri()).await.unwrap_err().to_string(), format!("invalid: {NOT_JELLYFIN}"));
    let missing = MockServer::start().await;
    assert_eq!(jf.check(&missing.uri()).await.unwrap_err().to_string(), format!("invalid: {NOT_JELLYFIN}"));

    // Jellyfin before 10.9.
    let old = MockServer::start().await;
    mount(&old, "GET", "/System/Info/Public", json!({"Id": SERVER_ID, "ServerName": "Old", "Version": "10.8.13", "ProductName": "Jellyfin Server"})).await;
    let e = jf.check(&old.uri()).await.unwrap_err().to_string();
    assert!(e.contains("10.8.13") && e.contains("10.9 or later"), "{e}");
    assert!(jf.password(&old.uri(), "me", "right".into()).await.is_err());
}

#[tokio::test]
async fn a_revoked_token_asks_to_sign_in_again() {
    let s = MockServer::start().await;
    Mock::given(any()).respond_with(ResponseTemplate::new(401)).mount(&s).await;
    let core = signed_core(&s, false, true, true);
    let lib = JellyfinLibrary::all(core.clone());
    let e = lib.count("v1", ItemKind::Track).await.unwrap_err();
    assert!(matches!(e, AppError::Auth(_)), "{e:?}");
    assert_eq!(core.state(), ServerState::SignInAgain);
    assert!(!LikeRemote::online(core.as_ref()).await);
    // Settings shows it.
    let d = tempfile::tempdir().unwrap();
    let (jf, b) = manager(d.path(), Arc::new(MemoryStore::new()));
    b.edit_records(|f| {
        f.upsert(crate::backend::ServerRecord {
            url: Some(s.uri()),
            user_id: Some("u1".into()),
            ..crate::backend::ServerRecord::new(&tag(), BackendKind::Jellyfin, "Den")
        })
    })
    .unwrap();
    // No token saved for it: it waits for "Sign in again".
    jf.restore();
    assert_eq!(jf.saved()[0].state, Some(ServerState::SignInAgain));
}

#[tokio::test]
async fn without_conversion_or_download_rights_those_say_why() {
    let s = jellyfin(false, false, false).await;
    let core = signed_core(&s, false, false, false);
    let server = JellyfinServer::new(core.clone());
    assert!(!server.caps().has(Caps::TRANSCODE));
    let t = super::map::track(&tag(), &serde_json::from_value(song("t1", 1)).unwrap(), |_| None);
    assert_eq!(server.streams().transcode(&t).await.unwrap_err().to_string(), format!("auth: {NO_TRANSCODE}"));
    // A capped stream plays the original instead.
    let prefs_core = core.clone();
    let src = prefs_core.stream(&t).await.unwrap();
    assert!(src.url.ends_with("/Audio/t1/stream.flac?static=true"), "{}", src.url);
    assert_eq!(server.downloads().fetch(&t, None).await.unwrap_err().to_string(), format!("auth: {NO_DOWNLOAD}"));
    assert!(server.activity().is_none() && server.file_paths().is_none(), "not an administrator");
}

// ---- requests the server sees ----------------------------------------------------------

#[tokio::test]
async fn library_sync_requests() {
    let s = jellyfin(false, true, true).await;
    let core = signed_core(&s, false, true, true);
    let lib = JellyfinLibrary::all(core.clone());
    let sections = lib.music_sections().await.unwrap();
    assert_eq!(sections.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["v1", "v3"], "music libraries only");
    assert_eq!(lib.count("v1", ItemKind::Artist).await.unwrap(), 1);
    assert_eq!(lib.count("v1", ItemKind::Album).await.unwrap(), 1);
    assert_eq!(lib.count("v1", ItemKind::Track).await.unwrap(), 2);
    let artists = lib.artists("v1", 0, 500).await.unwrap();
    let albums = lib.albums("v1", 0, 500).await.unwrap();
    let tracks = lib.tracks("v1", 0, 500).await.unwrap();
    let t = tag();
    assert_eq!(artists[0].id, format!("{t}:ar1"));
    assert_eq!(albums[0].id, format!("{t}:al1"));
    assert_eq!(albums[0].mbid.as_deref(), Some("mb-release"));
    assert_eq!(tracks.iter().map(|x| x.id.clone()).collect::<Vec<_>>(), [format!("{t}:t1"), format!("{t}:t2")]);
    assert_eq!(tracks[0].user_rating, Some(10.0), "a favourite is a like");
    assert!(tracks[0].rated_at.is_some(), "with the date it was first seen");
    let liked = lib.liked_tracks("v1").await.unwrap().unwrap();
    assert_eq!(liked.len(), 1);
    assert_eq!((liked[0].track_id.as_str(), liked[0].rating), (format!("{t}:t1").as_str(), 10.0));
    assert_eq!(liked[0].rated_at, tracks[0].rated_at, "the same first-seen date");
    let playlists = lib.playlists().await.unwrap();
    assert_eq!(playlists.iter().map(|p| p.id.clone()).collect::<Vec<_>>(), [format!("{t}:p1")], "audio playlists only");
    let rows = lib.playlist_entries(&format!("{t}:p1")).await.unwrap();
    assert_eq!(rows.iter().map(|r| (r.track_id.clone(), r.item_id.clone())).collect::<Vec<_>>(), [
        (format!("{t}:t1"), Some("e1".into())),
        (format!("{t}:t2"), Some("e2".into()))
    ]);
    let changed = lib.changed_since("v1", 86_400).await.unwrap();
    // (The fake doesn't filter by date.) Each song once, and the album
    // artist of what changed.
    assert_eq!((changed.tracks.len(), changed.albums.len(), changed.artists.len()), (2, 1, 1));
    assert_eq!(changed.artists[0].id, format!("{t}:ar1"));
    let items = lib.items("v1", &[format!("{t}:t1"), format!("{t}:al1")]).await.unwrap().unwrap();
    assert_eq!((items.tracks.len(), items.albums.len()), (1, 1));
    assert_eq!(lib.all_ids("v1", ItemKind::Track).await.unwrap(), [format!("{t}:t1"), format!("{t}:t2")]);
    assert!(lib.history("v1", 0, 100).await.unwrap().is_empty());
    let similar = lib.artist_similar(&format!("{t}:ar1")).await.unwrap();
    assert_eq!((similar[0].id.as_deref(), similar[0].name.as_str()), (Some(format!("{t}:ar9").as_str()), "Others"));

    let item_q = |types: &'static str, fields: &'static str, start: &'static str, limit: &'static str| {
        vec![
            ("userId", "u1"), ("ParentId", "v1"), ("Recursive", "true"), ("IncludeItemTypes", types), ("StartIndex", start),
            ("Limit", limit), ("Fields", fields), ("SortBy", "SortName"), ("EnableUserData", "true"),
        ]
    };
    let with = |mut q: Vec<(&'static str, &'static str)>, extra: &[(&'static str, &'static str)]| {
        q.extend_from_slice(extra);
        q
    };
    let since = "1970-01-02T00:00:00.0000000Z";
    let artist_q = |fields: &'static str, limit: &'static str| {
        vec![("userId", "u1"), ("ParentId", "v1"), ("StartIndex", "0"), ("Limit", limit), ("Fields", fields), ("SortBy", "SortName"), ("EnableUserData", "true")]
    };
    let expected = vec![
        req("GET", "/UserViews", &[("userId", "u1")]),
        req("GET", "/Artists/AlbumArtists", &artist_q("", "0")),
        req("GET", "/Items", &item_q("MusicAlbum", "", "0", "0")),
        req("GET", "/Items", &item_q("Audio", "", "0", "0")),
        req("GET", "/Artists/AlbumArtists", &artist_q(super::server::ARTIST_FIELDS, "500")),
        req("GET", "/Items", &item_q("MusicAlbum", super::server::ALBUM_FIELDS, "0", "500")),
        req("GET", "/Items", &item_q("Audio", super::server::TRACK_FIELDS, "0", "500")),
        req("GET", "/Items", &with(item_q("Audio", "", "0", "500"), &[("Filters", "IsFavorite"), ("EnableImages", "false")])),
        req("GET", "/Items", &[("userId", "u1"), ("IncludeItemTypes", "Playlist"), ("Recursive", "true"), ("Fields", super::server::PLAYLIST_FIELDS), ("SortBy", "SortName")]),
        req("GET", "/Playlists/p1/Items", &[("userId", "u1"), ("EnableImages", "false")]),
        req("GET", "/Items", &with(item_q("MusicAlbum", super::server::ALBUM_FIELDS, "0", "500"), &[("MinDateLastSaved", since)])),
        req("GET", "/Items", &with(item_q("Audio", super::server::TRACK_FIELDS, "0", "500"), &[("MinDateLastSaved", since)])),
        req("GET", "/Items", &with(item_q("Audio", super::server::TRACK_FIELDS, "0", "500"), &[("MinDateLastSavedForUser", since)])),
        req("GET", "/Items", &with(item_q("MusicAlbum", super::server::ALBUM_FIELDS, "0", "500"), &[("MinDateLastSavedForUser", since)])),
        req("GET", "/Items", &[("userId", "u1"), ("Ids", "ar1"), ("Fields", super::server::ARTIST_FIELDS), ("EnableUserData", "true")]),
        req("GET", "/Items", &[("userId", "u1"), ("Ids", "t1,al1"), ("Fields", "MediaSources,ProviderIds,Genres,DateCreated,Path,SortName,ChildCount,Studios,Overview"), ("EnableUserData", "true")]),
        req("GET", "/Items", &with(item_q("Audio", "", "0", "5000"), &[("EnableImages", "false")])),
        req("GET", "/Artists/ar1/Similar", &[("userId", "u1"), ("Limit", "20")]),
    ];
    assert_eq!(log(&s).await, expected);
    token_only_in_headers(&s).await;
}

#[tokio::test]
async fn likes_playlists_and_play_reports() {
    let s = jellyfin(false, true, true).await;
    let core = signed_core(&s, false, true, true);
    let t = tag();
    let id = |x: &str| format!("{t}:{x}");
    core.set_liked(&id("t2"), true).await.unwrap();
    core.set_liked(&id("t2"), false).await.unwrap();
    let new = core.create("Road", &[id("t1"), id("t2")]).await.unwrap();
    assert_eq!(new, id("p7"));
    core.add(&id("p1"), &[id("t2")]).await.unwrap();
    core.remove(&id("p1"), "e1").await.unwrap();
    // e2 to the top: index 0 in the list without it.
    core.move_item(&id("p1"), "e2", None).await.unwrap();
    core.move_item(&id("p1"), "e1", Some("e2")).await.unwrap();
    core.update(&id("p1"), Some("Renamed"), None).await.unwrap();
    assert!(core.update(&id("p1"), None, Some("words")).await.is_err(), "no descriptions on Jellyfin");
    assert!(core.poster(&id("p1"), vec![1], "image/png").await.is_err());
    core.delete(&id("p1")).await.unwrap();
    let (pl, rows) = PlaylistRemote::fetch(core.as_ref(), &id("p1")).await.unwrap();
    assert_eq!((pl.title.as_str(), rows.len()), ("Mine", 2));
    let report = |state, ms| PlayReport { track_id: id("t1"), state, time_ms: ms, duration_ms: 200_000 };
    core.timeline(&report(PlaybackState::Playing, 0)).await.unwrap();
    core.timeline(&report(PlaybackState::Playing, 10_000)).await.unwrap();
    core.timeline(&report(PlaybackState::Paused, 12_000)).await.unwrap();
    core.timeline(&report(PlaybackState::Stopped, 200_000)).await.unwrap();
    // A play counts once: marked played with when it happened.
    core.scrobble(&id("t1"), Some(1_709_210_096)).await.unwrap();

    let reqs = s.received_requests().await.unwrap();
    let lines = log(&s).await;
    let entries = req("GET", "/Playlists/p1/Items", &[("userId", "u1"), ("EnableImages", "false")]);
    assert_eq!(lines, vec![
        req("POST", "/UserFavoriteItems/t2", &[("userId", "u1")]),
        req("DELETE", "/UserFavoriteItems/t2", &[("userId", "u1")]),
        req("POST", "/Playlists", &[]),
        req("POST", "/Playlists/p1/Items", &[("ids", "t2"), ("userId", "u1")]),
        req("DELETE", "/Playlists/p1/Items", &[("entryIds", "e1")]),
        entries.clone(),
        req("POST", "/Playlists/p1/Items/e2/Move/0", &[]),
        entries.clone(),
        req("POST", "/Playlists/p1/Items/e1/Move/1", &[]),
        req("POST", "/Playlists/p1", &[]),
        req("DELETE", "/Items/p1", &[]),
        req("GET", "/Items/p1", &[("userId", "u1"), ("Fields", super::server::PLAYLIST_FIELDS)]),
        entries,
        req("POST", "/Sessions/Playing", &[]),
        req("POST", "/Sessions/Playing/Progress", &[]),
        req("POST", "/Sessions/Playing/Progress", &[]),
        req("POST", "/Sessions/Playing/Stopped", &[]),
        req("POST", "/UserPlayedItems/t1", &[("userId", "u1"), ("datePlayed", "2024-02-29T12:34:56.0000000Z")]),
    ]);
    let body = |i: usize| serde_json::from_slice::<serde_json::Value>(&reqs[i].body).unwrap();
    assert_eq!(body(2), json!({"Name": "Road", "Ids": ["t1", "t2"], "UserId": "u1", "MediaType": "Audio", "IsPublic": false}));
    assert_eq!(body(9), json!({"Name": "Renamed"}));
    let start = body(13);
    assert_eq!((start["ItemId"].as_str(), start["PositionTicks"].as_u64(), start["IsPaused"].as_bool()), (Some("t1"), Some(0), Some(false)));
    let session = start["PlaySessionId"].as_str().unwrap().to_string();
    assert_eq!(body(14)["PositionTicks"], 100_000_000u64);
    assert_eq!(body(15)["IsPaused"], true);
    assert_eq!(body(15)["EventName"], "Pause");
    let stop = body(16);
    assert_eq!((stop["PlaySessionId"].as_str(), stop["Failed"].as_bool()), (Some(session.as_str()), Some(true)), "the session doesn't count the play");
    token_only_in_headers(&s).await;

    // A stop with no start before it sends nothing.
    clear(&s).await;
    core.timeline(&report(PlaybackState::Stopped, 5)).await.unwrap();
    assert!(log(&s).await.is_empty());
}

#[tokio::test]
async fn streams_downloads_file_reads_and_art() {
    let s = jellyfin(true, true, true).await;
    let core = signed_core(&s, true, true, true);
    let server = JellyfinServer::new(core.clone());
    let t = super::map::track(&tag(), &serde_json::from_value(song("t1", 1)).unwrap(), |_| None);
    let base = s.uri();

    let src = server.streams().stream(&t).await.unwrap();
    assert_eq!(src.url, format!("{base}/Audio/t1/stream.flac?static=true"));
    assert_eq!(src.codec_hint.as_deref(), Some("flac"));
    assert!(src.headers.iter().any(|(k, v)| k == "Authorization" && v.contains(&format!("Token=\"{TOKEN}\""))));
    assert!(!format!("{src:?}").contains(TOKEN));

    let tc = server.streams().transcode(&t).await.unwrap();
    let u = url::Url::parse(&tc.url).unwrap();
    assert_eq!(u.path(), "/Audio/t1/universal");
    let q: std::collections::HashMap<String, String> = u.query_pairs().into_owned().collect();
    assert_eq!(q["MaxStreamingBitrate"], "320000");
    assert_eq!((q["UserId"].as_str(), q["DeviceId"].as_str(), q["AudioCodec"].as_str()), ("u1", DEVICE, "mp3"));
    assert!(!tc.url.contains(TOKEN));

    // A low cap away from home converts.
    let capped = Arc::new(Core::new(&tag(), "Den", JellyfinClient::new(super::client::http(), "https://music.example", identity()).unwrap(), Arc::new(StreamPrefs::new(None, Some(128))), None, LikeDates::open(None)));
    capped.sign_in(
        JellyfinClient::new(super::client::http(), "https://music.example", identity()).unwrap().signed_in(Token::new(TOKEN).unwrap(), "u1"),
        User { id: "u1".into(), ..Default::default() },
    );
    assert!(capped.stream(&t).await.unwrap().url.contains("MaxStreamingBitrate=128000"));

    let original = server.downloads().fetch(&t, None).await.unwrap();
    assert_eq!(original.src.url, format!("{base}/Items/t1/Download"));
    assert!(original.resumable && original.session.is_none());
    let small = server.downloads().fetch(&t, Some(192)).await.unwrap();
    assert!(!small.resumable);
    let session = small.session.clone().unwrap();
    assert_eq!(crate::sid::tag_of(&session), Some(tag().as_str()), "the session names its server");
    // Routed by the session's tag, it reaches this server.
    let b = Backends::empty();
    b.add(&tag(), Arc::new(JellyfinServer::new(core.clone())));
    crate::backend::routed::RoutedDownloads(b.clone()).finished(&small).await;
    let part = server.parts().part(&t.id, &t.part_key).await.unwrap();
    assert_eq!(part.url, format!("{base}/Audio/t1/stream.flac?static=true"));

    let art = server.art().fetch(t.thumb.as_deref().unwrap(), 300, 200).await.unwrap();
    assert_eq!(art.content_type, "image/png");

    let lines = log(&s).await;
    assert_eq!(lines, vec![
        req("DELETE", "/Videos/ActiveEncodings", &[("deviceId", DEVICE), ("playSessionId", crate::sid::key(&session))]),
        req("GET", "/Items/al1/Images/Primary", &[("tag", "img"), ("fillWidth", "300"), ("fillHeight", "200"), ("quality", "90")]),
    ]);
    token_only_in_headers(&s).await;
}

#[tokio::test]
async fn lyrics_mix_details_activity_and_files() {
    let s = jellyfin(true, true, true).await;
    let core = signed_core(&s, true, true, true);
    let server = JellyfinServer::new(core.clone());
    let t = tag();
    let id = |x: &str| format!("{t}:{x}");

    let l = server.lyrics().unwrap().lyrics(&id("t1")).await.unwrap().unwrap();
    assert!(l.synced);
    assert_eq!(l.timed[0].start_ms, 1000);
    assert!(server.lyrics().unwrap().lyrics(&id("t2")).await.unwrap().is_none(), "none for that song");
    assert_eq!(server.radio().unwrap().mix(&id("t1"), 25).await.unwrap(), [id("t2"), id("t9")]);
    assert_eq!(server.mbids().artist_mbid(&id("ar1")).await.unwrap().as_deref(), Some("mb-artist"));
    assert_eq!(server.mbids().album_mbid(&id("al1")).await.unwrap().as_deref(), Some("mb-release"));
    let info = server.streams().audio_info(&super::map::track(&t, &serde_json::from_value(song("t1", 1)).unwrap(), |_| None)).await.unwrap();
    assert_eq!(info.gain_db, Some(-7.5));

    // Friend activity: someone else's session, never this device's.
    let panel = crate::social::activity::read(server.activity().unwrap().as_ref(), 1_000).await;
    assert_eq!(panel.state, crate::social::activity::ActivityState::Ok);
    assert_eq!(panel.friends.len(), 1);
    assert_eq!((panel.friends[0].name.as_str(), panel.friends[0].track_id.as_deref(), panel.friends[0].now), ("Sam", Some(id("t2").as_str()), true));

    // The Lidarr check: the included music folders and every file.
    core.set_include(Some(vec!["v1".into()]));
    let files = server.file_paths().unwrap().music_files().await.unwrap();
    assert_eq!(files.locations, ["/music"]);
    assert_eq!(files.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["/music/Band/Record/01.flac", "/music/Band/Record/02.flac"]);
    assert_eq!(files.files[0].size, Some(1000));

    let hits = server.search().search("rec", None, 10).await.unwrap();
    assert!(hits.albums.is_empty(), "the fake answers nothing for search");
    let lines = log(&s).await;
    assert!(lines.contains(&req("GET", "/Audio/t1/Lyrics", &[])));
    assert!(lines.contains(&req("GET", "/Items/t1/InstantMix", &[("userId", "u1"), ("Limit", "25"), ("EnableImages", "false")])));
    assert!(lines.contains(&req("GET", "/Sessions", &[("ActiveWithinSeconds", "960")])));
    // Each kind on its own, in the included library; playlists anywhere.
    let fields = "MediaSources,ProviderIds,Genres,DateCreated,Path,SortName,ChildCount";
    for kind in ["MusicArtist", "MusicAlbum", "Audio"] {
        assert!(lines.contains(&req("GET", "/Items", &[
            ("userId", "u1"), ("searchTerm", "rec"), ("IncludeItemTypes", kind), ("Recursive", "true"),
            ("Limit", "10"), ("Fields", fields), ("EnableUserData", "true"), ("ParentId", "v1")
        ])), "{kind}: {lines:?}");
    }
    assert!(lines.contains(&req("GET", "/Items", &[
        ("userId", "u1"), ("searchTerm", "rec"), ("IncludeItemTypes", "Playlist"), ("Recursive", "true"),
        ("Limit", "10"), ("Fields", fields), ("EnableUserData", "true")
    ])));
    token_only_in_headers(&s).await;
}

#[tokio::test]
async fn long_playlist_adds_go_out_in_parts() {
    let s = jellyfin(false, true, true).await;
    let core = signed_core(&s, false, true, true);
    let id = |x: &str| format!("{}:{x}", tag());
    let ids: Vec<String> = (0..250).map(|i| id(&format!("{i:032x}"))).collect();
    PlaylistRemote::add(core.as_ref(), &id("p1"), &ids).await.unwrap();
    let posts: Vec<_> = s.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == "/Playlists/p1/Items").collect();
    assert_eq!(posts.len(), 3);
    assert!(posts.iter().all(|r| r.url.as_str().len() < 4_000));
}

// ---- ids and the library cache --------------------------------------------------------

#[tokio::test]
async fn guids_carry_the_tag_and_route_to_their_server() {
    let s = jellyfin(false, true, true).await;
    let core = signed_core(&s, false, true, true);
    let log_ = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let fallback = Arc::new(Fallback(log_.clone()));
    let b = Backends::with_fallback(fallback);
    b.add(&tag(), Arc::new(JellyfinServer::new(core)));
    let guid = "6f1b2c3d4e5f60718293a4b5c6d7e8f9";
    let id = crate::sid::qualify_any(&tag(), guid);
    assert_eq!(crate::sid::split(&id), (Some(tag().as_str()), guid));
    assert_eq!(crate::sid::qualify_any(&tag(), &id), id, "tagged once");
    assert_eq!(b.for_id(&id).unwrap().kind(), BackendKind::Jellyfin);
    assert_eq!(b.for_id("123").unwrap().kind(), BackendKind::Plex, "Plex ids are as they were");
    assert_eq!(b.caps_for(&id), super::server::jellyfin_caps(false, true));
    crate::backend::routed::RoutedLikes(b.clone()).set_liked(&format!("{}:t1", tag()), true).await.unwrap();
    assert!(log(&s).await.contains(&req("POST", "/UserFavoriteItems/t1", &[("userId", "u1")])));
    assert!(log_.lock().is_empty(), "nothing went to the other server");
}

/// Stands in for the Plex account in routing tests.
struct Fallback(Arc<parking_lot::Mutex<Vec<String>>>);

#[async_trait::async_trait]
impl MusicServer for Fallback {
    fn kind(&self) -> BackendKind {
        BackendKind::Plex
    }
    fn caps(&self) -> Caps {
        Caps::NONE
    }
    async fn tag(&self) -> Option<String> {
        None
    }
    async fn name(&self) -> Option<String> {
        None
    }
    async fn reach(&self) -> Option<crate::plex::types::ConnectionKind> {
        None
    }
    async fn server_id(&self) -> Option<String> {
        None
    }
    fn library(&self) -> Arc<dyn LibrarySource> {
        unreachable!()
    }
    fn search(&self) -> Arc<dyn RemoteSearch> {
        unreachable!()
    }
    fn art(&self) -> Arc<dyn ArtFetcher> {
        unreachable!()
    }
    fn streams(&self) -> Arc<dyn TrackResolver> {
        unreachable!()
    }
    fn downloads(&self) -> Arc<dyn DownloadSource> {
        unreachable!()
    }
    fn parts(&self) -> Arc<dyn PartSource> {
        unreachable!()
    }
    fn likes(&self) -> Arc<dyn LikeRemote> {
        self.0.lock().push("likes".into());
        unreachable!()
    }
    fn playlists(&self) -> Arc<dyn PlaylistRemote> {
        unreachable!()
    }
    fn plays(&self) -> Arc<dyn PlayReporter> {
        unreachable!()
    }
    fn mbids(&self) -> Arc<dyn crate::artistinfo::service::MbidLookup> {
        unreachable!()
    }
}

#[tokio::test]
async fn the_main_library_stays_with_plex_and_moves_when_plex_goes() {
    let s = jellyfin(false, true, true).await;
    let core = signed_core(&s, false, true, true);
    core.load_views().await.unwrap();
    let d = tempfile::tempdir().unwrap();
    let b = Backends::with_fallback(Arc::new(Fallback(Arc::default())));
    b.keep_records_in(d.path().to_path_buf());
    assert_eq!(b.main_server().unwrap().kind(), BackendKind::Plex, "nothing saved: as before");
    let plex_tag = BackendKind::Plex.tag("m1");
    b.edit_records(|f| f.upsert(crate::backend::ServerRecord::new(&plex_tag, BackendKind::Plex, "Home"))).unwrap();
    b.add(&tag(), Arc::new(JellyfinServer::new(core)));
    b.edit_records(|f| f.upsert(crate::backend::ServerRecord::new(&tag(), BackendKind::Jellyfin, "Den"))).unwrap();
    assert_eq!(b.primary().unwrap().kind(), BackendKind::Plex, "the first added stays primary");
    assert_eq!(b.main_server().unwrap().kind(), BackendKind::Plex);
    // Jellyfin's libraries sync besides Plex's main one.
    let extras = b.extra_libraries();
    assert_eq!(extras.iter().map(|e| e.src.clone()).collect::<Vec<_>>(), [format!("{}/v1", tag()), format!("{}/v3", tag())]);
    b.edit_records(|f| f.set_primary(&tag())).unwrap();
    assert_eq!(b.primary().unwrap().kind(), BackendKind::Jellyfin, "made primary");
    assert_eq!(b.main_server().unwrap().kind(), BackendKind::Plex, "Plex keeps the main library while it's set up");
    // Signed out of Plex: Jellyfin carries the library on its own.
    assert!(b.forget_plex());
    assert_eq!(b.main_server().unwrap().kind(), BackendKind::Jellyfin);
    assert!(!b.records().servers.iter().any(|r| r.kind == BackendKind::Plex));
}

#[tokio::test]
async fn the_same_album_on_plex_and_jellyfin_is_one_card() {
    use crate::db::fake;
    const PLEX: &str = "aaaaaaaa";
    let s = jellyfin(false, true, true).await;
    let core = signed_core(&s, false, true, true);
    core.load_views().await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let db = crate::db::DbHandle::open(dir.path().join("library.db")).unwrap();

    // The Plex library: the same album, two songs of the same length.
    let ar = fake::artist("1", "Band");
    let mut al = fake::album("2", "Record", &ar, 2001, None, &[]);
    al.track_count = 2;
    let tracks = vec![fake::track("3", "Song 1", &al, 1, 1), fake::track("4", "Song 2", &al, 1, 2)];
    let lib = fake::FakeLibrary { sections: vec![fake::section("1", "uuid-a")], artists: vec![ar], albums: vec![al], tracks, ..Default::default() };
    let plex = Arc::new(fake::FakeSource::new(fake::tagged(lib, PLEX)));
    *plex.tag.lock() = Some(PLEX.to_string());

    let server = JellyfinServer::new(core);
    let extras: Vec<crate::db::sync::ExtraLibrary> = server
        .libraries()
        .into_iter()
        .map(|(view, source)| crate::db::sync::ExtraLibrary { src: crate::db::store::src_of(&tag(), &view), tag: tag(), source })
        .collect();
    assert_eq!(extras.len(), 2, "both music libraries are included by default");
    db.set_extra_libraries(extras.into_iter().take(1).collect());
    db.sync_once(plex, true).await.unwrap();

    let n = |sql: &str| -> i64 { db.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap() };
    assert_eq!(n("SELECT count(*) FROM albums"), 2, "a row from each server");
    assert_eq!(n(&format!("SELECT count(*) FROM albums WHERE id = '{}:al1'", tag())), 1);
    assert_eq!(n(&format!("SELECT count(*) FROM tracks WHERE src = '{}/v1'", tag())), 2);
    assert_eq!(n("SELECT count(*) FROM dupes WHERE kind = 'album'"), 1, "the same album");
    let cards = db
        .read(|c| {
            crate::db::queries::albums_view_on(c, crate::db::AlbumSort::Name, 0, 500, None, crate::tags::TagView::default(), &crate::db::merge::ServerView::All)
                .map(|p| p.total as i64)
        })
        .unwrap();
    assert_eq!(cards, 1, "one card");
    // Jellyfin's favourite arrived as a like on its own copy.
    assert_eq!(n(&format!("SELECT count(*) FROM tracks WHERE id = '{}:t1' AND user_rating >= 10", tag())), 1);
}
