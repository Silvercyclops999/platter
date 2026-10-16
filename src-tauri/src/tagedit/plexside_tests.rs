//! The folder rescan request and the library settings check.

use std::sync::Arc;

use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::plexside::{assess, library_check, parse_agents, refresh_folder_on};
use crate::plex::PlexClient;
use crate::plex::client::LibrarySection;
use crate::plex::identity::ClientIdentity;
use crate::plex::token::Token;
use crate::plex::types::ConnectionKind;

fn client(s: &MockServer) -> PlexClient {
    PlexClient::new(&s.uri(), Token::new("tok").unwrap(), Arc::new(ClientIdentity::with_id("cid")), Some(ConnectionKind::Local)).unwrap()
}

#[tokio::test]
async fn rescan_asks_for_just_that_folder() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/library/sections/7/refresh"))
        .and(query_param("path", "/srv/music/Demo Artist/Demo Album & More"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    refresh_folder_on(&client(&s), "7", "/srv/music/Demo Artist/Demo Album & More").await.unwrap();
    // A section id never carries anything into the path.
    Mock::given(method("GET")).and(path("/library/sections/7x/refresh")).respond_with(ResponseTemplate::new(200)).expect(1).mount(&s).await;
    refresh_folder_on(&client(&s), "7/../x", "/srv/music").await.unwrap();
}

fn section(agent: &str) -> LibrarySection {
    LibrarySection {
        id: "7".into(),
        title: "Music".into(),
        kind: "artist".into(),
        uuid: None,
        agent: Some(agent.into()),
        scanner: None,
        scanned_at: None,
        updated_at: None,
        refreshing: false,
        locations: vec![],
    }
}

#[test]
fn library_settings_are_explained_when_off() {
    let agents = parse_agents(br#"{"MediaContainer":{"Agent":[{"identifier":"tv.plex.agents.music"},{"identifier":"com.plexapp.agents.none"}]}}"#);
    assert_eq!(agents, ["tv.plex.agents.music", "com.plexapp.agents.none"]);
    let on = assess(&section("tv.plex.agents.music"), &[("respectTags".into(), json!("1"))], &agents);
    assert!(on.ok, "{on:?}");
    assert_eq!(on.agent, "Plex Music");
    let off = assess(&section("tv.plex.agents.music"), &[("respectTags".into(), json!(false))], &[]);
    assert!(!off.ok);
    assert_eq!(off.items[0].ok, Some(false));
    assert!(off.items[0].help.contains("Edit Library, then Advanced, turn on Prefer local metadata"), "{}", off.items[0].help);
    assert_eq!(off.items[2].ok, Some(false), "no Personal Media agent listed");
    // A legacy agent with embedded tags off.
    let legacy = assess(&section("com.plexapp.agents.lastfm"), &[("respectTags".into(), json!(true)), ("useEmbeddedTags".into(), json!(0))], &agents);
    assert_eq!(legacy.items[1].ok, Some(false));
    assert!(legacy.items[1].help.contains("Use embedded tags"));
    // Plex not saying stays unknown, not "off".
    let unknown = assess(&section("tv.plex.agents.music"), &[], &agents);
    assert_eq!(unknown.items[0].ok, None);
    assert!(unknown.ok);
    // A Personal Media library reads tags as they are.
    assert!(assess(&section("com.plexapp.agents.none"), &[], &[]).ok);
    assert!(parse_agents(b"not json").is_empty());
}

#[tokio::test]
async fn library_check_reads_sections_prefs_and_agents() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/library/sections")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"Directory": [
        {"key": "7", "type": "artist", "title": "Music", "agent": "tv.plex.agents.music"},
        {"key": "1", "type": "movie", "title": "Movies"}
    ]}}))).mount(&s).await;
    Mock::given(method("GET")).and(path("/library/sections/7/prefs")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"Setting": [
        {"id": "respectTags", "value": false}
    ]}}))).mount(&s).await;
    Mock::given(method("GET")).and(path("/system/agents")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"Agent": [
        {"identifier": "com.plexapp.agents.none"}
    ]}}))).mount(&s).await;
    let r = library_check(&client(&s)).await.unwrap();
    assert_eq!(r.len(), 1, "music libraries only");
    assert_eq!(r[0].items[0].ok, Some(false));
    assert_eq!(r[0].items[2].ok, Some(true));
}
