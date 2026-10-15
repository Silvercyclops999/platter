//! The exact requests a Plex server sees for each backend call the app makes
//! (library sync, likes, play reports, playlist edits, streams, downloads,
//! artwork, search). The expected lists were recorded once against the code
//! as it was before the backend layer existed; any change to the wiring must
//! keep them byte for byte.

use std::sync::Arc;
use std::time::Duration;

use wiremock::matchers::{any, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::audio::source::TrackResolver;
use crate::db::source::{ArtFetcher, ItemKind, LibrarySource, RemoteSearch};
use crate::downloads::DownloadSource;
use crate::likes::RatingRemote;
use crate::model::Track;
use crate::playlists::PlaylistRemote;
use crate::plex::PlexState;
use crate::plex::client::{PlaybackState, PlexMetadata, TimelineReport};
use crate::plex::discovery::RaceConfig;
use crate::plex::keychain::{ACCOUNT_TOKEN, MemoryStore, TokenStore};
use crate::plex::token::Token;
use crate::tracking::PlayReporter;

const ACCT: &str = "account-secret-token";
const MACHINE: &str = "m1";

/// What the app talks to, built the way lib.rs builds it.
struct Wiring {
    library: Arc<dyn LibrarySource>,
    search: Arc<dyn RemoteSearch>,
    art: Arc<dyn ArtFetcher>,
    likes: Arc<dyn RatingRemote>,
    plays: Arc<dyn PlayReporter>,
    playlists: Arc<dyn PlaylistRemote>,
    streams: Arc<dyn TrackResolver>,
    downloads: Arc<dyn DownloadSource>,
    prefs: Arc<crate::bridge::StreamPrefs>,
}

fn wire(plex: &PlexState) -> Wiring {
    use crate::backend::routed::*;
    let prefs = Arc::new(crate::bridge::StreamPrefs::new(None, None));
    let b = crate::backend::Backends::with_plex(crate::backend::plex::PlexServer::new(plex.clone(), prefs.clone(), None));
    Wiring {
        library: b.primary().unwrap().library(),
        search: Arc::new(RoutedSearch(b.clone())),
        art: Arc::new(RoutedArt(b.clone())),
        likes: Arc::new(RoutedLikes(b.clone())),
        plays: Arc::new(RoutedPlays(b.clone())),
        playlists: Arc::new(RoutedPlaylists(b.clone())),
        streams: Arc::new(RoutedStreams(b.clone())),
        downloads: Arc::new(RoutedDownloads(b)),
        prefs,
    }
}

fn json(v: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(v)
}

async fn get(s: &MockServer, p: &str, body: serde_json::Value) {
    Mock::given(method("GET")).and(path(p)).respond_with(json(body)).mount(s).await;
}

async fn pms() -> MockServer {
    let s = MockServer::builder().start().await;
    get(&s, "/identity", serde_json::json!({"MediaContainer": {"machineIdentifier": MACHINE, "version": "1.43.4"}})).await;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(header("x-plex-token", ACCT))
        .respond_with(json(serde_json::json!({
            "MediaContainer": {"machineIdentifier": MACHINE, "version": "1.43.4", "myPlexSubscription": true}
        })))
        .mount(&s)
        .await;
    get(&s, "/library/sections", serde_json::json!({
        "MediaContainer": {"Directory": [{"key": "3", "type": "artist", "title": "Music", "uuid": "u-3"}]}
    }))
    .await;
    get(&s, "/library/sections/3/prefs", serde_json::json!({
        "MediaContainer": {"Setting": [{"id": "respectTags", "value": true}]}
    }))
    .await;
    get(&s, "/library/sections/3/format", serde_json::json!({
        "MediaContainer": {"Directory": [{"key": "ep", "title": "EP"}]}
    }))
    .await;
    get(&s, "/library/metadata/9", serde_json::json!({
        "MediaContainer": {"Metadata": [{"ratingKey": "9", "type": "artist", "title": "A",
            "Similar": [{"id": 4, "tag": "B"}]}]}
    }))
    .await;
    get(&s, "/playlists/5", serde_json::json!({
        "MediaContainer": {"Metadata": [{"ratingKey": "5", "type": "playlist", "playlistType": "audio", "title": "P"}]}
    }))
    .await;
    Mock::given(method("POST"))
        .and(path("/playlists"))
        .respond_with(json(serde_json::json!({
            "MediaContainer": {"Metadata": [{"ratingKey": "77", "type": "playlist", "playlistType": "audio", "title": "New"}]}
        })))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/photo/:/transcode"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"\x89PNG\r\n\x1a\nrest".to_vec()))
        .mount(&s)
        .await;
    // Everything else: an empty container.
    Mock::given(any())
        .respond_with(json(serde_json::json!({"MediaContainer": {"size": 0, "totalSize": 0}})))
        .with_priority(100)
        .mount(&s)
        .await;
    s
}

async fn plex_tv(server: &MockServer) -> MockServer {
    let tv = MockServer::builder().start().await;
    get(&tv, "/api/v2/user", serde_json::json!({"username": "listener"})).await;
    get(&tv, "/api/v2/resources", serde_json::json!([{
        "name": "Home", "product": "Plex Media Server", "productVersion": "1.43.4",
        "clientIdentifier": MACHINE, "provides": "server", "owned": true, "presence": true,
        "accessToken": ACCT,
        "connections": [{"protocol": "http", "uri": server.uri(), "local": true, "relay": false}]
    }]))
    .await;
    tv
}

async fn connected() -> (PlexState, MockServer, MockServer, tempfile::TempDir) {
    let server = pms().await;
    let tv = plex_tv(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    store.set(ACCOUNT_TOKEN, &Token::new(ACCT).unwrap()).unwrap();
    let cfg = RaceConfig { timeout: Duration::from_millis(800), grace: Duration::from_millis(100) };
    let plex = PlexState::with_parts(dir.path().to_path_buf(), store, &tv.uri(), cfg).unwrap();
    plex.restore().await.unwrap();
    assert!(plex.client().await.is_ok(), "connected to the fake server");
    (plex, server, tv, dir)
}

/// `METHOD /path?query`, with per-play session ids blanked.
fn line(method: &str, url: &url::Url) -> String {
    let q: Vec<String> = url
        .query_pairs()
        .map(|(k, v)| if k == "session" { format!("{k}=*") } else { format!("{k}={v}") })
        .collect();
    if q.is_empty() { format!("{method} {}", url.path()) } else { format!("{method} {}?{}", url.path(), q.join("&")) }
}

/// Requests the app makes on its own while connected (health checks, the
/// notification socket); not part of any call.
fn background(url: &url::Url) -> bool {
    matches!(url.path(), "/" | "/identity" | "/:/websockets/notifications" | "/library/sections/3/prefs")
}

/// Requests the server received since `from`, with paging headers.
async fn since(s: &MockServer, from: &mut usize) -> Vec<String> {
    let all = s.received_requests().await.unwrap_or_default();
    let out = all[*from..]
        .iter()
        .filter(|r| !background(&r.url))
        .map(|r| {
            let paging: Vec<String> = ["x-plex-container-start", "x-plex-container-size"]
                .iter()
                .filter_map(|h| r.headers.get(*h).and_then(|v| v.to_str().ok()).map(|v| format!("{h}={v}")))
                .collect();
            let l = line(r.method.as_str(), &r.url);
            if paging.is_empty() { l } else { format!("{l} [{}]", paging.join(",")) }
        })
        .collect();
    *from = all.len();
    out
}

/// Wait until the requests that follow connecting have stopped.
async fn quiet(s: &MockServer) {
    let mut last = usize::MAX;
    for _ in 0..50 {
        let n = s.received_requests().await.unwrap_or_default().len();
        if n == last {
            return;
        }
        last = n;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A stream or download request as the player would send it, with the names
/// of the headers it carries (the token goes in a header, never the URL).
fn source_line(src: &crate::audio::StreamSource) -> String {
    let url = url::Url::parse(&src.url).unwrap();
    let mut names: Vec<String> = src.headers.iter().map(|(k, _)| k.to_lowercase()).collect();
    names.sort();
    names.dedup();
    assert!(!src.url.contains(ACCT), "no token in the URL");
    format!("{} [{}] {:?}", line("GET", &url), names.join(","), src.codec_hint)
}

fn track(tag: &str) -> Track {
    let mut t = PlexMetadata {
        rating_key: format!("{tag}:42"),
        title: "Song".into(),
        kind: Some("track".into()),
        ..Default::default()
    }
    .to_track();
    t.part_key = "/library/parts/7/1/file.flac".into();
    t.codec = Some("flac".into());
    t.bitrate = Some(1411);
    t
}

#[tokio::test]
async fn plex_requests_stay_exactly_the_same() {
    let (plex, server, _tv, _dir) = connected().await;
    let w = wire(&plex);
    let tag = crate::sid::tag(MACHINE);
    let id = |k: &str| format!("{tag}:{k}");
    quiet(&server).await;
    let mut from = server.received_requests().await.unwrap_or_default().len();
    let mut log: Vec<(String, Vec<String>)> = Vec::new();

    // Library sync.
    w.library.music_sections().await.unwrap();
    log.push(("sections".into(), since(&server, &mut from).await));
    w.library.count("3", ItemKind::Track).await.unwrap();
    w.library.artists("3", 0, 500).await.unwrap();
    w.library.albums("3", 0, 500).await.unwrap();
    w.library.tracks("3", 500, 500).await.unwrap();
    log.push(("pages".into(), since(&server, &mut from).await));
    w.library.changed_since("3", 1_700_000_000).await.unwrap();
    log.push(("changed".into(), since(&server, &mut from).await));
    w.library.items("3", &[id("1"), id("2")]).await.unwrap();
    w.library.all_ids("3", ItemKind::Album).await.unwrap();
    w.library.history("3", 0, 100).await.unwrap();
    log.push(("items, ids, history".into(), since(&server, &mut from).await));
    w.library.playlists().await.unwrap();
    w.library.playlist_items(&id("5")).await.unwrap();
    w.library.playlist_entries(&id("5")).await.unwrap();
    log.push(("playlists".into(), since(&server, &mut from).await));
    w.library.rated_tracks("3").await.unwrap();
    w.library.artist_similar(&id("9")).await.unwrap();
    log.push(("ratings, similar".into(), since(&server, &mut from).await));
    w.search.search("abc", Some("3"), 10).await.unwrap();
    log.push(("search".into(), since(&server, &mut from).await));

    // Likes.
    assert!(w.likes.online().await);
    w.likes.rate(&id("42"), Some(crate::db::likes::LIKED_RATING)).await.unwrap();
    w.likes.rate(&id("42"), None).await.unwrap();
    log.push(("likes".into(), since(&server, &mut from).await));

    // Play reports.
    assert!(w.plays.online().await);
    w.plays
        .timeline(&TimelineReport {
            rating_key: id("42"),
            state: PlaybackState::Playing,
            time_ms: 1000,
            duration_ms: 200_000,
            play_queue_item_id: None,
        })
        .await
        .unwrap();
    w.plays.scrobble(&id("42")).await.unwrap();
    log.push(("plays".into(), since(&server, &mut from).await));

    // Playlist edits.
    assert!(w.playlists.online().await);
    let made = w.playlists.create("New", &[id("42"), id("43")]).await.unwrap();
    assert_eq!(made, id("77"));
    w.playlists.add(&id("5"), &[id("44")]).await.unwrap();
    w.playlists.remove(&id("5"), "601").await.unwrap();
    w.playlists.move_item(&id("5"), "601", Some("602")).await.unwrap();
    w.playlists.move_item(&id("5"), "601", None).await.unwrap();
    w.playlists.update(&id("5"), Some("Renamed"), Some("About")).await.unwrap();
    w.playlists.poster(&id("5"), b"\x89PNG".to_vec(), "image/png").await.unwrap();
    w.playlists.fetch(&id("5")).await.unwrap();
    w.playlists.delete(&id("5")).await.unwrap();
    log.push(("playlist edits".into(), since(&server, &mut from).await));

    // Artwork.
    let art = w.art.fetch(&id("/library/metadata/42/thumb/1"), 300, 300).await.unwrap();
    assert_eq!(art.content_type, "image/png");
    log.push(("art".into(), since(&server, &mut from).await));

    // Streams: the original file, a capped stream and a transcode. Built,
    // not sent: the player sends them.
    let t = track(&tag);
    let mut built = vec![source_line(&w.streams.stream(&t).await.unwrap())];
    w.prefs.set(Some(256), None);
    built.push(source_line(&w.streams.stream(&t).await.unwrap()));
    w.prefs.set(None, None);
    built.push(source_line(&w.streams.transcode(&t).await.unwrap()));
    // Downloads: the original, and a transcode that is stopped afterwards.
    assert!(w.downloads.connection().await.is_some());
    let orig = w.downloads.fetch(&t, None).await.unwrap();
    built.push(format!("{} resumable={}", source_line(&orig.src), orig.resumable));
    let mp3 = w.downloads.fetch(&t, Some(192)).await.unwrap();
    built.push(format!("{} resumable={}", source_line(&mp3.src), mp3.resumable));
    w.downloads.finished(&mp3).await;
    log.push(("streams built".into(), built));
    log.push(("download finished".into(), since(&server, &mut from).await));

    let got: String = log.iter().map(|(name, lines)| format!("# {name}\n{}\n", lines.join("\n"))).collect();
    let expected = EXPECTED.replace("{tag}", &tag);
    if got != expected {
        panic!("Plex requests changed.\n--- got\n{got}\n--- expected\n{expected}");
    }
    plex.sign_out().await.unwrap();
}

/// Recorded against the code before the backend layer.
const EXPECTED: &str = r#"# sections
GET /library/sections
# pages
GET /library/sections/3/all?type=10 [x-plex-container-start=0,x-plex-container-size=0]
GET /library/sections/3/all?type=8 [x-plex-container-start=0,x-plex-container-size=500]
GET /library/sections/3/format?type=9
GET /library/sections/3/subformat?type=9
GET /library/sections/3/style?type=9
GET /library/sections/3/mood?type=9
GET /library/sections/3/all?type=9&includeGuids=1&format=ep [x-plex-container-start=0,x-plex-container-size=500]
GET /library/sections/3/all?type=9&includeGuids=1 [x-plex-container-start=0,x-plex-container-size=500]
GET /library/sections/3/all?type=10&includeGuids=1 [x-plex-container-start=500,x-plex-container-size=500]
# changed
GET /library/sections/3/all?type=8&updatedAt>=1700000000 [x-plex-container-start=0,x-plex-container-size=500]
GET /library/sections/3/all?type=9&includeGuids=1&updatedAt>=1700000000 [x-plex-container-start=0,x-plex-container-size=500]
GET /library/sections/3/all?type=10&includeGuids=1&updatedAt>=1700000000 [x-plex-container-start=0,x-plex-container-size=500]
# items, ids, history
GET /library/metadata/1,2
GET /library/sections/3/all?type=9&includeGuids=1 [x-plex-container-start=0,x-plex-container-size=500]
GET /status/sessions/history/all?librarySectionID=3&sort=viewedAt:desc [x-plex-container-start=0,x-plex-container-size=100]
# playlists
GET /playlists?playlistType=audio
GET /playlists/5/items [x-plex-container-start=0,x-plex-container-size=500]
GET /playlists/5/items [x-plex-container-start=0,x-plex-container-size=500]
# ratings, similar
GET /library/sections/3/all?type=10&includeGuids=1&userRating>=1 [x-plex-container-start=0,x-plex-container-size=500]
GET /library/metadata/9
# search
GET /hubs/search?query=abc&limit=10&sectionId=3
# likes
PUT /:/rate?key=42&identifier=com.plexapp.plugins.library&rating=10
PUT /:/rate?key=42&identifier=com.plexapp.plugins.library&rating=-1
# plays
GET /:/timeline?ratingKey=42&key=/library/metadata/42&identifier=com.plexapp.plugins.library&state=playing&time=1000&duration=200000
GET /:/scrobble?key=42&identifier=com.plexapp.plugins.library
# playlist edits
POST /playlists?type=audio&title=New&smart=0&uri=server://m1/com.plexapp.plugins.library/library/metadata/42,43
PUT /playlists/5/items?uri=server://m1/com.plexapp.plugins.library/library/metadata/44
DELETE /playlists/5/items/601
PUT /playlists/5/items/601/move?after=602
PUT /playlists/5/items/601/move
PUT /playlists/5?title=Renamed&summary=About
POST /library/metadata/5/posters
GET /playlists/5
GET /playlists/5/items [x-plex-container-start=0,x-plex-container-size=500]
DELETE /playlists/5
# art
GET /photo/:/transcode?url=/library/metadata/42/thumb/1&width=300&height=300&minSize=1&upscale=1
# streams built
GET /library/parts/7/1/file.flac [accept,x-plex-client-identifier,x-plex-device,x-plex-device-name,x-plex-platform,x-plex-platform-version,x-plex-product,x-plex-provides,x-plex-token,x-plex-version] Some("flac")
GET /music/:/transcode/universal/start.mp3?path=/library/metadata/42&mediaIndex=0&partIndex=0&protocol=http&directPlay=0&directStream=0&hasMDE=1&musicBitrate=256&maxAudioBitrate=256&session=* [accept,x-plex-client-identifier,x-plex-device,x-plex-device-name,x-plex-platform,x-plex-platform-version,x-plex-product,x-plex-provides,x-plex-session-identifier,x-plex-token,x-plex-version] Some("mp3")
GET /music/:/transcode/universal/start.mp3?path=/library/metadata/42&mediaIndex=0&partIndex=0&protocol=http&directPlay=0&directStream=0&hasMDE=1&musicBitrate=320&maxAudioBitrate=320&session=* [accept,x-plex-client-identifier,x-plex-device,x-plex-device-name,x-plex-platform,x-plex-platform-version,x-plex-product,x-plex-provides,x-plex-session-identifier,x-plex-token,x-plex-version] Some("mp3")
GET /library/parts/7/1/file.flac [accept,x-plex-client-identifier,x-plex-device,x-plex-device-name,x-plex-platform,x-plex-platform-version,x-plex-product,x-plex-provides,x-plex-token,x-plex-version] Some("flac") resumable=true
GET /music/:/transcode/universal/start.mp3?path=/library/metadata/42&mediaIndex=0&partIndex=0&protocol=http&directPlay=0&directStream=0&hasMDE=1&musicBitrate=192&maxAudioBitrate=192&session=* [accept,x-plex-client-identifier,x-plex-device,x-plex-device-name,x-plex-platform,x-plex-platform-version,x-plex-product,x-plex-provides,x-plex-session-identifier,x-plex-token,x-plex-version] Some("mp3") resumable=false
# download finished
GET /music/:/transcode/universal/stop?session=*
"#;
