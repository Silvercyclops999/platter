//! Editing against a faked Plex server: who may edit, the unreleased guard,
//! art cache invalidation, undo, and failures.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::plex::identity::ClientIdentity;
use crate::plex::token::Token;
use crate::plex::types::ConnectionKind;

/// One `changed` call: kind, ids, art dropped, resync.
type Changed = (EditKind, Vec<String>, bool, bool);

const MACHINE: &str = "owned-machine";

struct Servers {
    uri: String,
    owned: Vec<String>,
    connected: bool,
}

#[async_trait]
impl EditServers for Servers {
    async fn client_for(&self, _id: &str) -> AppResult<PlexClient> {
        Ok(PlexClient::new(&self.uri, Token::new("tok")?, Arc::new(ClientIdentity::with_id("cid")), Some(ConnectionKind::Local))?
            .with_server(MACHINE))
    }
    async fn owned(&self) -> (Vec<String>, bool) {
        (self.owned.clone(), self.connected)
    }
}

#[derive(Default)]
struct Hooks {
    unreleased: bool,
    settings: Mutex<EditSettings>,
    changed: Mutex<Vec<Changed>>,
}

#[async_trait]
impl EditHooks for Hooks {
    async fn unreleased(&self, _album_id: &str) -> bool {
        self.unreleased
    }
    async fn changed(&self, kind: EditKind, ids: &[String], art: bool, resync: bool) {
        self.changed.lock().push((kind, ids.to_vec(), art, resync));
    }
    fn settings(&self) -> EditSettings {
        *self.settings.lock()
    }
}

fn tag() -> String {
    crate::sid::tag(MACHINE)
}

fn id(k: &str) -> String {
    format!("{}:{k}", tag())
}

struct Rig {
    server: MockServer,
    hooks: Arc<Hooks>,
    editor: PlexEditor,
    _dir: tempfile::TempDir,
}

async fn rig_with(owned: bool, unreleased: bool) -> Rig {
    let server = MockServer::start().await;
    let hooks = Arc::new(Hooks { unreleased, ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let servers = Servers { uri: server.uri(), owned: if owned { vec![tag()] } else { vec![] }, connected: owned };
    let editor = PlexEditor::new(dir.path().to_path_buf(), Arc::new(servers), hooks.clone());
    Rig { server, hooks, editor, _dir: dir }
}

async fn rig() -> Rig {
    rig_with(true, false).await
}

fn ok() -> ResponseTemplate {
    ResponseTemplate::new(200)
}

/// `GET /library/metadata/{k}` for an album.
async fn album(s: &MockServer, k: &str, guid: &str, extra: serde_json::Value) {
    let mut m = serde_json::json!({
        "ratingKey": k, "type": "album", "title": "Old title", "year": 2019, "summary": "old", "guid": guid,
        "thumb": format!("/library/metadata/{k}/thumb/1"), "librarySectionID": "7",
        "Genre": [{"tag": "Pop"}], "Field": [{"name": "summary", "locked": true}]
    });
    if let (Some(o), Some(e)) = (m.as_object_mut(), extra.as_object()) {
        for (a, b) in e {
            o.insert(a.clone(), b.clone());
        }
    }
    Mock::given(method("GET"))
        .and(path(format!("/library/metadata/{k}")))
        .respond_with(ok().set_body_json(serde_json::json!({"MediaContainer": {"Metadata": [m]}})))
        .mount(s)
        .await;
}

async fn posters(s: &MockServer, k: &str, selected: &str, extra: &[&str]) {
    let mut list = vec![serde_json::json!({"key": format!("/library/metadata/{k}/file?url=x"), "ratingKey": selected, "selected": true, "provider": "embedded"})];
    for e in extra {
        list.push(serde_json::json!({"key": e, "ratingKey": e, "thumb": "https://images.plex.tv/photo?u=1", "selected": false, "provider": "caa"}));
    }
    Mock::given(method("GET"))
        .and(path(format!("/library/metadata/{k}/posters")))
        .respond_with(ok().set_body_json(serde_json::json!({"MediaContainer": {"Metadata": list}})))
        .mount(s)
        .await;
}

async fn photo(s: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/photo/:/transcode"))
        .respond_with(ok().set_body_bytes(vec![0xFF, 0xD8, 0xFF, 0xE0, 9, 9]))
        .mount(s)
        .await;
}

#[tokio::test]
async fn access_lists_owned_servers_and_settings() {
    let r = rig().await;
    let a = r.editor.access().await;
    assert!(a.show && a.connected && a.lock_fields && a.match_unreleased);
    assert_eq!(a.servers, vec![tag()]);
    r.hooks.settings.lock().show = false;
    assert!(!r.editor.access().await.show);
}

#[tokio::test]
async fn shared_servers_cant_edit_and_nothing_is_sent() {
    let r = rig_with(false, false).await;
    assert!(r.editor.access().await.servers.is_empty());
    for res in [r.editor.refresh(&id("5"), EditKind::Album).await, r.editor.unmatch(&id("5"), EditKind::Album).await] {
        assert!(matches!(res, Err(AppError::Auth(m)) if m == NO_PERMISSION));
    }
    assert!(r.server.received_requests().await.unwrap().is_empty(), "no request reached the server");
}

#[tokio::test]
async fn editing_turned_off_refuses_writes() {
    let r = rig().await;
    r.hooks.settings.lock().show = false;
    assert!(matches!(r.editor.refresh(&id("5"), EditKind::Album).await, Err(AppError::Invalid(_))));
    assert!(r.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_server_that_refuses_is_hidden_from_then_on() {
    let r = rig().await;
    Mock::given(method("PUT")).and(path("/library/metadata/5/refresh")).respond_with(ResponseTemplate::new(403)).mount(&r.server).await;
    assert!(matches!(r.editor.refresh(&id("5"), EditKind::Album).await, Err(AppError::Auth(_))));
    assert!(r.editor.access().await.servers.is_empty());
}

#[tokio::test]
async fn cover_change_selects_it_drops_cached_art_and_undoes() {
    let r = rig().await;
    album(&r.server, "5", "plex://album/1", serde_json::json!({})).await;
    posters(&r.server, "5", "metadata://posters/old", &["https://caa.example/new.jpg"]).await;
    photo(&r.server).await;
    Mock::given(method("PUT"))
        .and(path("/library/metadata/5/poster"))
        .and(query_param("url", "https://caa.example/new.jpg"))
        .respond_with(ok())
        .expect(1)
        .mount(&r.server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/library/metadata/5/poster"))
        .and(query_param("url", "metadata://posters/old"))
        .respond_with(ok())
        .expect(1)
        .mount(&r.server)
        .await;
    let ch = r.editor.select_image(&id("5"), EditKind::Album, ImageKind::Poster, "https://caa.example/new.jpg".into()).await.unwrap();
    assert_eq!(ch.label, "Changed the cover");
    assert!(ch.undoable);
    assert_eq!(r.hooks.changed.lock().last().unwrap(), &(EditKind::Album, vec![id("5")], true, false), "art dropped for the item");
    let list = r.editor.images(&id("5"), ImageKind::Poster).await.unwrap();
    assert!(list[0].preview.as_deref().unwrap().starts_with(&format!("{}:/library/metadata/5/", tag())), "previews carry the server");
    let undone = r.editor.undo(&ch.id).await.unwrap();
    assert!(undone.undone);
    assert!(r.editor.undo(&ch.id).await.is_err(), "only once");
    assert_eq!(r.editor.history(&id("5")).await.len(), 1);
}

#[tokio::test]
async fn undo_uploads_the_kept_cover_when_plex_no_longer_lists_it() {
    let r = rig().await;
    album(&r.server, "5", "plex://album/1", serde_json::json!({})).await;
    photo(&r.server).await;
    // Before the change the old cover is listed; afterwards it's gone.
    Mock::given(method("GET"))
        .and(path("/library/metadata/5/posters"))
        .respond_with(ok().set_body_json(serde_json::json!({"MediaContainer": {"Metadata": [
            {"key": "/x", "ratingKey": "metadata://posters/old", "selected": true}]}})))
        .up_to_n_times(1)
        .mount(&r.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/library/metadata/5/posters"))
        .respond_with(ok().set_body_json(serde_json::json!({"MediaContainer": {"Metadata": [
            {"key": "/y", "ratingKey": "upload://posters/new", "selected": true}]}})))
        .mount(&r.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/library/metadata/5/posters"))
        .and(wiremock::matchers::header("content-type", "image/png"))
        .respond_with(ok())
        .expect(1)
        .mount(&r.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/library/metadata/5/posters"))
        .and(wiremock::matchers::header("content-type", "image/jpeg"))
        .and(wiremock::matchers::body_bytes(vec![0xFF, 0xD8, 0xFF, 0xE0, 9, 9]))
        .respond_with(ok())
        .expect(1)
        .mount(&r.server)
        .await;
    let ch = r.editor.upload_image(&id("5"), EditKind::Album, ImageKind::Poster, vec![0x89, b'P', b'N', b'G'], "image/png".into()).await.unwrap();
    r.editor.undo(&ch.id).await.unwrap();
}

#[tokio::test]
async fn image_uploads_and_links_are_checked_first() {
    let r = rig().await;
    let big = vec![0u8; IMAGE_MAX_BYTES + 1];
    assert!(matches!(r.editor.upload_image(&id("5"), EditKind::Album, ImageKind::Poster, big, "image/jpeg".into()).await, Err(AppError::Invalid(m)) if m.contains("10 MB")));
    assert!(r.editor.upload_image(&id("5"), EditKind::Album, ImageKind::Poster, vec![1], "image/gif".into()).await.is_err());
    assert!(r.editor.image_link(&id("5"), EditKind::Album, ImageKind::Poster, "file:///c:/x.jpg".into()).await.is_err());
    assert!(r.server.received_requests().await.unwrap().is_empty());
    // The server saying it's too large is passed on.
    album(&r.server, "5", "plex://album/1", serde_json::json!({})).await;
    Mock::given(method("POST")).and(path("/library/metadata/5/posters")).respond_with(ResponseTemplate::new(413)).mount(&r.server).await;
    let e = r.editor.upload_image(&id("5"), EditKind::Album, ImageKind::Poster, vec![1, 2], "image/jpeg".into()).await;
    assert!(matches!(e, Err(AppError::Invalid(m)) if m.contains("too large")));
    assert!(r.editor.history(&id("5")).await.is_empty(), "failed changes aren't logged");
}

#[tokio::test]
async fn artist_background_goes_to_arts() {
    let r = rig().await;
    Mock::given(method("POST"))
        .and(path("/library/metadata/9/arts"))
        .and(query_param("url", "https://example.com/bg.jpg"))
        .respond_with(ok())
        .expect(1)
        .mount(&r.server)
        .await;
    let ch = r.editor.image_link(&id("9"), EditKind::Artist, ImageKind::Art, " https://example.com/bg.jpg ".into()).await.unwrap();
    assert_eq!(ch.label, "Set the background from a link");
}

#[tokio::test]
async fn unreleased_albums_show_the_flag_and_match_can_be_turned_off() {
    let r = rig_with(true, true).await;
    album(&r.server, "5", "local://5", serde_json::json!({})).await;
    let info = r.editor.item(&id("5")).await.unwrap();
    assert!(info.unreleased && !info.matched);
    r.hooks.settings.lock().match_unreleased = false;
    let e = r.editor.set_match(&id("5"), EditKind::Album, "mbid://x".into(), "X".into(), None).await;
    assert!(matches!(e, Err(AppError::Invalid(_))));
    // Unmatch is always there.
    Mock::given(method("PUT")).and(path("/library/metadata/5/unmatch")).respond_with(ok()).expect(1).mount(&r.server).await;
    let ch = r.editor.unmatch(&id("5"), EditKind::Album).await.unwrap();
    assert!(!ch.undoable, "it wasn't matched before");
}

#[tokio::test]
async fn match_then_undo_goes_back_to_the_old_match() {
    let r = rig().await;
    album(&r.server, "5", "plex://album/old", serde_json::json!({})).await;
    Mock::given(method("PUT"))
        .and(path("/library/metadata/5/match"))
        .and(query_param("guid", "mbid://new"))
        .and(query_param("name", "New"))
        .and(query_param("year", "2020"))
        .respond_with(ok())
        .expect(1)
        .mount(&r.server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/library/metadata/5/match"))
        .and(query_param("guid", "plex://album/old"))
        .and(query_param("name", "Old title"))
        .and(query_param("year", "2019"))
        .respond_with(ok())
        .expect(1)
        .mount(&r.server)
        .await;
    let ch = r.editor.set_match(&id("5"), EditKind::Album, "mbid://new".into(), "New".into(), Some(2020)).await.unwrap();
    assert_eq!(ch.label, "Matched to New");
    r.editor.undo(&ch.id).await.unwrap();
}

#[tokio::test]
async fn match_on_an_unmatched_album_undoes_with_unmatch() {
    let r = rig().await;
    album(&r.server, "5", "local://5", serde_json::json!({})).await;
    Mock::given(method("PUT")).and(path("/library/metadata/5/match")).respond_with(ok()).expect(1).mount(&r.server).await;
    Mock::given(method("PUT")).and(path("/library/metadata/5/unmatch")).respond_with(ok()).expect(1).mount(&r.server).await;
    let ch = r.editor.set_match(&id("5"), EditKind::Album, "mbid://x".into(), "X".into(), None).await.unwrap();
    r.editor.undo(&ch.id).await.unwrap();
}

#[tokio::test]
async fn details_are_locked_and_undo_restores_values_and_locks() {
    let r = rig().await;
    album(&r.server, "5", "plex://album/1", serde_json::json!({})).await;
    Mock::given(method("PUT"))
        .and(path("/library/sections/7/all"))
        .and(query_param("type", "9"))
        .and(query_param("id", "5"))
        .and(query_param("title.value", "New title"))
        .and(query_param("title.locked", "1"))
        .and(query_param("genre[0].tag.tag", "Rock"))
        .and(query_param("genre[].tag.tag-", "Pop"))
        .respond_with(ok())
        .expect(1)
        .mount(&r.server)
        .await;
    // Undo: the old title, unlocked again (it wasn't locked); Pop back.
    Mock::given(method("PUT"))
        .and(path("/library/sections/7/all"))
        .and(query_param("title.value", "Old title"))
        .and(query_param("title.locked", "0"))
        .and(query_param("genre[0].tag.tag", "Pop"))
        .and(query_param("genre[].tag.tag-", "Rock"))
        .respond_with(ok())
        .expect(1)
        .mount(&r.server)
        .await;
    let before = r.editor.item(&id("5")).await.unwrap().item.fields;
    let to = Snapshot { title: "New title".into(), genres: vec!["Rock".into()], ..before.clone() };
    let ch = r.editor.edit_details(&id("5"), to).await.unwrap();
    assert_eq!(ch.label, "Edited 2 details");
    assert!(matches!(r.editor.edit_details(&id("5"), before).await, Err(AppError::Invalid(m)) if m == "Nothing changed."));
    r.editor.undo(&ch.id).await.unwrap();
}

#[tokio::test]
async fn details_without_locks_when_turned_off() {
    let r = rig().await;
    r.hooks.settings.lock().lock = false;
    album(&r.server, "5", "plex://album/1", serde_json::json!({})).await;
    Mock::given(method("PUT"))
        .and(path("/library/sections/7/all"))
        .and(query_param("summary.value", "new"))
        .and(query_param("summary.locked", "0"))
        .respond_with(ok())
        .expect(1)
        .mount(&r.server)
        .await;
    let before = r.editor.item(&id("5")).await.unwrap().item.fields;
    let ch = r.editor.edit_details(&id("5"), Snapshot { summary: "new".into(), ..before }).await.unwrap();
    assert_eq!(ch.label, "Edited the summary");
}

#[tokio::test]
async fn merge_moves_the_photo_and_undo_splits() {
    let r = rig().await;
    Mock::given(method("GET"))
        .and(path("/library/metadata/7"))
        .respond_with(ok().set_body_json(serde_json::json!({"MediaContainer": {"Metadata": [{"ratingKey": "7", "type": "artist", "title": "Keeper", "thumb": "/library/metadata/7/thumb/1"}]}})))
        .mount(&r.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/library/metadata/8"))
        .respond_with(ok().set_body_json(serde_json::json!({"MediaContainer": {"Metadata": [{"ratingKey": "8", "type": "artist", "title": "Other", "thumb": "/library/metadata/8/thumb/1"}]}})))
        .mount(&r.server)
        .await;
    photo(&r.server).await;
    Mock::given(method("PUT")).and(path("/library/metadata/7/merge")).and(query_param("ids", "8")).respond_with(ok()).expect(1).mount(&r.server).await;
    Mock::given(method("POST")).and(path("/library/metadata/7/posters")).respond_with(ok()).expect(1).mount(&r.server).await;
    Mock::given(method("PUT")).and(path("/library/metadata/7/split")).respond_with(ok()).expect(1).mount(&r.server).await;
    let ch = r.editor.merge(&id("7"), vec![id("8")], Some(id("8"))).await.unwrap();
    assert_eq!(ch.label, "Merged another artist into Keeper");
    let last = r.hooks.changed.lock().last().cloned().unwrap();
    assert_eq!(last, (EditKind::Artist, vec![id("7"), id("8")], true, true), "both read again, then a sync");
    r.editor.undo(&ch.id).await.unwrap();
    // Not with an artist on another server.
    assert!(r.editor.merge(&id("7"), vec!["99999999:8".into()], None).await.is_err());
}

#[tokio::test]
async fn split_and_refresh_are_logged_but_not_undoable() {
    let r = rig().await;
    Mock::given(method("PUT")).and(path("/library/metadata/7/split")).respond_with(ok()).expect(1).mount(&r.server).await;
    Mock::given(method("PUT")).and(path("/library/metadata/5/refresh")).respond_with(ok()).expect(1).mount(&r.server).await;
    let s = r.editor.split(&id("7")).await.unwrap();
    let f = r.editor.refresh(&id("5"), EditKind::Album).await.unwrap();
    assert!(!s.undoable && !f.undoable);
    assert!(r.editor.undo(&s.id).await.is_err());
}

#[tokio::test]
async fn server_offline_is_a_network_error_and_nothing_is_logged() {
    let dir = tempfile::tempdir().unwrap();
    let servers = Servers { uri: "http://127.0.0.1:1".into(), owned: vec![tag()], connected: true };
    let editor = PlexEditor::new(dir.path().to_path_buf(), Arc::new(servers), Arc::new(Hooks::default()));
    let e = editor.refresh(&id("5"), EditKind::Album).await;
    assert!(matches!(e, Err(AppError::Network(_))), "{e:?}");
    assert!(editor.history(&id("5")).await.is_empty());
}

#[tokio::test]
async fn the_log_survives_a_restart() {
    let server = MockServer::start().await;
    Mock::given(method("PUT")).and(path("/library/metadata/5/refresh")).respond_with(ok()).mount(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let make = || {
        PlexEditor::new(
            dir.path().to_path_buf(),
            Arc::new(Servers { uri: server.uri(), owned: vec![tag()], connected: true }),
            Arc::new(Hooks::default()),
        )
    };
    make().refresh(&id("5"), EditKind::Album).await.unwrap();
    assert_eq!(make().history(&id("5")).await.len(), 1);
}
