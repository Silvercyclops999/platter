//! Live, read-only checks against the developer's real server. All
//! `#[ignore]`; run with `cargo test live_ -- --ignored --nocapture`.
//!
//! The token is read from the repo's `.env` (PLEX_TOKEN) and never printed.
//! Output is limited to counts, kinds, latencies and versions — no
//! addresses, machine ids or names (the repo will be public).
//!
//! Write endpoints (`/:/timeline`, `/:/scrobble`) have no live test here on
//! purpose.

use std::sync::Arc;
use std::time::Duration;

use super::auth::PlexTv;
use super::client::{MetaType, PlexClient};
use super::discovery::{self, RaceConfig};
use super::identity::ClientIdentity;
use super::token::Token;

const ENV_FILE: &str = "C:/Users/Adam/Desktop/Plexify/.env";

fn dev_token() -> Token {
    let iter = dotenvy::from_path_iter(ENV_FILE).expect(".env not found");
    for item in iter {
        let (k, v) = item.expect(".env parse");
        if k == "PLEX_TOKEN" {
            return Token::new(v).expect("empty PLEX_TOKEN");
        }
    }
    panic!("PLEX_TOKEN missing from .env");
}

fn identity() -> Arc<ClientIdentity> {
    // Fixed test id so the real server sees one "Platter (tests)" device.
    let mut id = ClientIdentity::with_id("platter-live-tests-0001");
    id.device_name = "Platter live tests".into();
    Arc::new(id)
}

async fn connect() -> PlexClient {
    let id = identity();
    let tv = PlexTv::new(id.clone()).unwrap();
    let token = dev_token();
    let servers = discovery::servers_from_resources(tv.resources(&token).await.unwrap());
    let server = servers
        .iter()
        .find(|s| s.owned)
        .or(servers.first())
        .expect("no server");
    let http = reqwest::Client::builder()
        .default_headers(id.headers().unwrap())
        .build()
        .unwrap();
    let p = discovery::race(
        &http,
        &server.connections,
        Some(&server.machine_id),
        RaceConfig::default(),
    )
    .await
    .expect("no connection answered");
    PlexClient::new(
        &p.uri,
        server.access_token.clone().unwrap_or(token),
        id,
        Some(p.kind),
    )
    .unwrap()
}

#[tokio::test]
#[ignore]
async fn live_discovery_and_race() {
    let id = identity();
    let tv = PlexTv::new(id.clone()).unwrap();
    let token = dev_token();
    let user = tv.user(&token).await.unwrap();
    assert!(!user.display_name().is_empty());
    let servers = discovery::servers_from_resources(tv.resources(&token).await.unwrap());
    println!("servers: {}", servers.len());
    let http = reqwest::Client::builder()
        .default_headers(id.headers().unwrap())
        .build()
        .unwrap();
    for s in &servers {
        let kinds: Vec<_> = s.connections.iter().map(|c| c.kind).collect();
        println!(
            "server owned={} presence={} version={:?} has_token={} connections={kinds:?}",
            s.owned,
            s.presence,
            s.version,
            s.access_token.is_some()
        );
        for c in &s.connections {
            let started = std::time::Instant::now();
            let one = discovery::race(
                &http,
                std::slice::from_ref(c),
                Some(&s.machine_id),
                RaceConfig::default(),
            )
            .await;
            println!(
                "  probe {:?}: ok={} in {:?}",
                c.kind,
                one.is_some(),
                started.elapsed()
            );
        }
        let started = std::time::Instant::now();
        let p = discovery::race(
            &http,
            &s.connections,
            Some(&s.machine_id),
            RaceConfig::default(),
        )
        .await;
        println!(
            "  race winner: {:?} latency={:?} total={:?}",
            p.as_ref().map(|p| p.kind),
            p.as_ref().map(|p| p.latency),
            started.elapsed()
        );
    }
}

#[tokio::test]
#[ignore]
async fn live_library_reads() {
    let c = connect().await;
    println!("connection: {:?}", c.connection());
    let ident = c.identity_info().await.unwrap();
    assert!(!ident.machine_identifier.is_empty());
    let root = c.root().await.unwrap();
    println!(
        "root: version={:?} plexPass={} musicAnalysis={:?}",
        root.version, root.my_plex_subscription, root.music_analysis
    );
    let music = c.music_sections().await.unwrap();
    println!("music sections: {}", music.len());
    let sec = &music[0];
    assert!(sec.uuid.is_some());
    let prefs = c.section_prefs(&sec.id).await.unwrap();
    println!(
        "prefs: respectTags={:?} musicAnalysis={:?}",
        prefs.respect_tags, prefs.music_analysis
    );
    let ids: Vec<String> = prefs
        .all
        .iter()
        .map(|(k, v)| match v {
            serde_json::Value::Bool(b) => format!("{k}={b}"),
            serde_json::Value::Number(n) => format!("{k}={n}"),
            _ => k.clone(),
        })
        .collect();
    println!("pref ids: {ids:?}");
    for ty in [MetaType::Artist, MetaType::Album, MetaType::Track] {
        println!("count {ty:?}: {}", c.count(&sec.id, ty).await.unwrap());
    }
    let artists = c.artists(&sec.id, 0, 3).await.unwrap();
    assert!(artists.items.len() <= 3);
    let albums = c.albums(&sec.id, 0, 5).await.unwrap();
    println!("albums page: {} of {}", albums.items.len(), albums.total);
    let tracks = c.tracks(&sec.id, 0, 5).await.unwrap();
    println!(
        "tracks page: {} of {}; first has part key={}, codec={:?}",
        tracks.items.len(),
        tracks.total,
        tracks.items[0].part_key.starts_with("/library/parts/"),
        tracks.items[0].codec
    );
    // paging: page 2 differs from page 1
    let p2 = c.tracks(&sec.id, 5, 5).await.unwrap();
    assert_ne!(p2.items[0].id, tracks.items[0].id);
    let all_albums = c.list_all(&sec.id, MetaType::Album, &[]).await.unwrap();
    assert_eq!(all_albums.len() as u64, albums.total);

    for field in ["format", "subformat"] {
        let vals = c.tag_values(&sec.id, field, MetaType::Album).await.unwrap();
        for v in &vals {
            let page = c
                .list(&sec.id, MetaType::Album, 0, 500, &[(field, v.key.clone())])
                .await
                .unwrap();
            println!(
                "{field} '{}' (key is numeric: {}): {} albums",
                v.title,
                v.key.parse::<i64>().is_ok(),
                page.total
            );
        }
    }
    // album detail has Format; track detail has streams
    let album = c.album(&albums.items[0].id).await.unwrap();
    println!(
        "album detail: format={:?} subformats={:?} section={:?}",
        album.format, album.subformats, album.section
    );
    let kids = c.album_tracks(&album.id).await.unwrap();
    assert!(!kids.is_empty());
    let (t, audio) = c.track(&kids[0].id).await.unwrap();
    println!(
        "track detail: bitDepth={:?} sampleRate={:?} audio={:?}",
        t.bit_depth,
        t.sample_rate,
        audio.as_ref().map(|a| (
            a.gain.is_some(),
            a.album_gain.is_some(),
            a.peak.is_some(),
            a.loudness.is_some()
        ))
    );
    let hist = c.history(&sec.id, 0, 5).await.unwrap();
    println!("history: {} of {}", hist.items.len(), hist.total);
    let pls = c.playlists().await.unwrap();
    println!(
        "playlists: {} (smart {})",
        pls.len(),
        pls.iter().filter(|p| p.smart).count()
    );
    if let Some(p) = pls.first() {
        let items = c.playlist_items(&p.id, 0, 5).await.unwrap();
        println!(
            "playlist items page: {} of {}",
            items.items.len(),
            items.total
        );
    }
    let hubs = c.section_hubs(&sec.id).await.unwrap();
    println!("section hubs: {}", hubs.len());
    let summarize = |hubs: &[super::client::PlexHub]| {
        hubs.iter()
            .filter(|h| !h.metadata.is_empty())
            .map(|h| (h.hub_identifier.clone(), h.metadata.len()))
            .collect::<Vec<_>>()
    };
    let found = c.search("a", Some(&sec.id), 5).await.unwrap();
    println!("search 'a' non-empty hubs: {:?}", summarize(&found));
    let name = artists.items[0].title.clone();
    let prefix: String = name.chars().take(4).collect();
    for q in [name.as_str(), prefix.as_str()] {
        let with = c.search(q, Some(&sec.id), 5).await.unwrap();
        let without = c.search(q, None, 5).await.unwrap();
        println!(
            "search artist name (len {}): with sectionId {:?} / without {:?}",
            q.chars().count(),
            summarize(&with),
            summarize(&without)
        );
    }
    if let Some(thumb) = albums.items.iter().find_map(|a| a.thumb.clone()) {
        let bytes = c.photo(&thumb, 200, 200).await.unwrap();
        println!("photo transcode bytes: {}", bytes.len());
        assert!(bytes.len() > 100);
    }
}

#[tokio::test]
#[ignore]
async fn live_updated_since_filter() {
    let c = connect().await;
    let sec = c.music_sections().await.unwrap().remove(0);
    let total = c.count(&sec.id, MetaType::Track).await.unwrap();
    let from_zero = c
        .updated_since(&sec.id, MetaType::Track, 0, 0, 0)
        .await
        .unwrap()
        .total;
    let future = c
        .updated_since(&sec.id, MetaType::Track, 4_000_000_000, 0, 0)
        .await
        .unwrap()
        .total;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let last_90d = c
        .updated_since(&sec.id, MetaType::Track, now - 90 * 86_400, 0, 0)
        .await
        .unwrap()
        .total;
    println!(
        "tracks total={total} updatedAt>=0: {from_zero} >=future: {future} >=90d ago: {last_90d}"
    );
    assert_eq!(from_zero, total);
    assert_eq!(future, 0, "filter ignored?");
}

#[tokio::test]
#[ignore]
async fn live_stream_and_transcode() {
    let c = connect().await;
    let sec = c.music_sections().await.unwrap().remove(0);
    let t = c.tracks(&sec.id, 0, 1).await.unwrap().items.remove(0);
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    // direct stream with a range
    let req = c.stream_request(&t.part_key).unwrap();
    let resp = http
        .get(req.url.clone())
        .headers(req.headers.clone())
        .header("range", "bytes=0-15")
        .send()
        .await
        .unwrap();
    println!(
        "direct: status={} content-type={:?} accept-ranges={:?}",
        resp.status(),
        resp.headers().get("content-type"),
        resp.headers().get("accept-ranges")
    );
    assert_eq!(resp.status().as_u16(), 206);
    assert_eq!(resp.bytes().await.unwrap().len(), 16);
    // universal transcoder, 128 kbps mp3
    let session = uuid::Uuid::new_v4().to_string();
    let req = c.transcode_request(&t.id, 128, &session).unwrap();
    let resp = http
        .get(req.url.clone())
        .headers(req.headers.clone())
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let ctype = resp.headers().get("content-type").cloned();
    let mut got = 0usize;
    let mut stream = resp.bytes_stream();
    use futures::StreamExt;
    while let Some(Ok(chunk)) = stream.next().await {
        got += chunk.len();
        if got > 32 * 1024 {
            break;
        }
    }
    drop(stream);
    println!("transcode: status={status} content-type={ctype:?} first bytes={got}");
    let stop = c.transcode_stop(&session).await;
    println!("transcode stop: {:?}", stop.as_ref().map_err(|e| e.kind()));
    assert!(status.is_success());
    assert!(got > 1024);
}

#[tokio::test]
#[ignore]
async fn live_notifications_connect() {
    use super::notifications::{self, PlexEvent};
    let c = connect().await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(notifications::run(c, tx, stop_rx));
    let first = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .unwrap();
    println!("first websocket event: {first:?}");
    assert_eq!(first, Some(PlexEvent::Connected));
    // collect whatever arrives for a few seconds
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    while let Ok(Some(ev)) = tokio::time::timeout_at(until, rx.recv()).await {
        println!("event: {ev:?}");
    }
    stop_tx.send(true).unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
}
