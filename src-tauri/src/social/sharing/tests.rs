//! Sharing against a faked plex.tv (wiremock) and a fake server for the
//! label writes. Nothing here reaches the real plex.tv or anyone's account.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::params;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::links::{LinkDeps, LinkResult};
use super::*;
use crate::db::store::{SyncState, put_sync_state_in};
use crate::plex::identity::ClientIdentity;
use crate::social::listen::fake_ntfy::FakeNtfy;
use crate::social::listen::relay::Ntfy;

const MACHINE: &str = "0123456789abcdef0123456789abcdef01234567";
const FRIEND_MACHINE: &str = "fedcba9876543210fedcba9876543210fedcba98";

const SERVER_XML: &str = r#"<MediaContainer size="1">
  <Server name="Home" machineIdentifier="0123456789abcdef0123456789abcdef01234567">
    <Section id="901" key="3" type="artist" title="Music"/>
    <Section id="902" key="4" type="movie" title="Films"/>
    <Section id="903" key="7" type="artist" title="Live Tapes"/>
  </Server>
</MediaContainer>"#;

// ---- fakes ----------------------------------------------------------------------

struct FakeHost {
    tv: PlexTv,
    server: Mutex<Option<MyServer>>,
    user: Option<String>,
    labels: Mutex<Vec<(String, String)>>,
    fail_labels: AtomicBool,
    before: Mutex<Vec<ServerEntry>>,
    after: Mutex<Vec<ServerEntry>>,
    sections: Mutex<HashMap<String, Vec<Section>>>,
    included: Mutex<Vec<String>>,
}

impl FakeHost {
    fn new(tv_url: &str, user: &str) -> Self {
        Self {
            tv: PlexTv::with_base(Arc::new(ClientIdentity::with_id("cid")), tv_url).unwrap(),
            server: Mutex::new(Some(MyServer { machine_id: MACHINE.into(), name: "Home".into(), owned: true })),
            user: Some(user.into()),
            labels: Mutex::default(),
            fail_labels: AtomicBool::new(false),
            before: Mutex::default(),
            after: Mutex::default(),
            sections: Mutex::default(),
            included: Mutex::default(),
        }
    }
}

#[async_trait]
impl Host for FakeHost {
    fn tv(&self) -> &PlexTv {
        &self.tv
    }
    async fn token(&self) -> Option<Token> {
        Some(Token::new("fake-account-token").unwrap())
    }
    async fn my_server(&self) -> Option<MyServer> {
        self.server.lock().clone()
    }
    async fn main_section(&self) -> Option<String> {
        Some("3".into())
    }
    async fn username(&self) -> Option<String> {
        self.user.clone()
    }
    async fn add_label(&self, section: &str, album_id: &str, label: &str) -> AppResult<bool> {
        assert_eq!(label, HIDE_LABEL);
        if self.fail_labels.load(Ordering::Relaxed) {
            return Err(AppError::Auth("You can't edit this server.".into()));
        }
        self.labels.lock().push((section.into(), album_id.into()));
        Ok(true)
    }
    async fn servers(&self) -> Vec<ServerEntry> {
        self.before.lock().clone()
    }
    async fn refresh_servers(&self) -> AppResult<Vec<ServerEntry>> {
        Ok(self.after.lock().clone())
    }
    async fn music_sections(&self, machine_id: &str) -> AppResult<Vec<Section>> {
        Ok(self.sections.lock().get(machine_id).cloned().unwrap_or_default())
    }
    async fn include(&self, entries: Vec<String>) -> AppResult<()> {
        self.included.lock().extend(entries);
        Ok(())
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<(String, serde_json::Value)>>);
impl EventSink for Events {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().push((event.into(), payload));
    }
}

#[derive(Default)]
struct Notes(Mutex<Vec<(String, String)>>);
impl Notifier for Notes {
    fn notify(&self, title: &str, body: &str) {
        self.0.lock().push((title.into(), body.into()));
    }
}

struct Rig {
    sharing: Sharing,
    host: Arc<FakeHost>,
    events: Arc<Events>,
    notes: Arc<Notes>,
    settings: ShareSettings,
    _dir: tempfile::TempDir,
}

/// An album in library `src`: unreleased (a demo) or released.
fn album(db: &DbHandle, src: &str, id: &str, unreleased: bool) {
    let (src, id) = (src.to_string(), id.to_string());
    db.write(move |c| {
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, format, matched)
             VALUES (?1, 'Album', 'Album', 'ar', 'The Band', 1, ?2, ?3, 1)",
            params![id, src, if unreleased { "Demo" } else { "Album" }],
        )?;
        Ok(())
    })
    .unwrap();
}

fn read(db: &DbHandle, src: &str, section: &str) {
    let (src, section) = (src.to_string(), section.to_string());
    db.write(move |c| {
        put_sync_state_in(c, &src, &SyncState { section_id: section, section_uuid: "u".into(), section_title: None, last_sync_at: Some(1), last_full_sync_at: Some(1), generation: 1 })
    })
    .unwrap();
}

fn rig_with(tv: &MockServer, user: &str, relay: Option<&FakeNtfy>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("s.db")).unwrap();
    // "Music" (the main library): two unreleased albums and one released.
    // "Live Tapes": read, nothing unreleased.
    read(&db, MAIN_SRC, "3");
    album(&db, MAIN_SRC, "aaaaaaaa:11", true);
    album(&db, MAIN_SRC, "aaaaaaaa:12", true);
    album(&db, MAIN_SRC, "aaaaaaaa:13", false);
    let live = store::src_of(&crate::sid::tag(MACHINE), "7");
    read(&db, &live, "7");
    album(&db, &live, "aaaaaaaa:21", false);
    let host = Arc::new(FakeHost::new(&tv.uri(), user));
    let events = Arc::new(Events::default());
    let notes = Arc::new(Notes::default());
    let settings = ShareSettings::default();
    let sharing = Sharing::new(
        host.clone(),
        db,
        settings.clone(),
        events.clone(),
        notes.clone(),
        LinkDeps {
            relays: Arc::new(Ntfy::new().unwrap()),
            relay: Arc::default(),
            default_relay: relay.map(|r| r.url.clone()).unwrap_or_else(|| "http://127.0.0.1:9".into()),
            answer_timeout: Duration::from_secs(5),
            retry: Duration::from_millis(50),
        },
    );
    Rig { sharing, host, events, notes, settings, _dir: dir }
}

fn rig(tv: &MockServer) -> Rig {
    rig_with(tv, "sam_hosts", None)
}

/// plex.tv: my server's libraries, Plex Pass or not, and shares that work.
async fn plex_tv(pass: bool) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path(format!("/api/servers/{MACHINE}"))).respond_with(ResponseTemplate::new(200).set_body_string(SERVER_XML)).mount(&s).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"username": "sam_hosts", "subscription": {"active": pass}})))
        .mount(&s)
        .await;
    s
}

async fn shares_ok(s: &MockServer) {
    Mock::given(method("POST")).and(path(format!("/api/servers/{MACHINE}/shared_servers"))).respond_with(ResponseTemplate::new(200).set_body_string("<MediaContainer/>")).mount(s).await;
}

async fn share_bodies(s: &MockServer) -> Vec<serde_json::Value> {
    s.received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn opts(libs: &[&str], hide: bool) -> ShareOptions {
    ShareOptions { libraries: libs.iter().map(|s| s.to_string()).collect(), hide_unreleased: hide, allow_downloads: false }
}

// ---- before sharing ---------------------------------------------------------------

#[tokio::test]
async fn the_overview_lists_music_libraries_only_with_their_unreleased_albums() {
    let tv = plex_tv(true).await;
    let r = rig(&tv);
    *r.settings.default_libraries.write() = vec!["7".into()];
    let o = r.sharing.overview().await.unwrap();
    assert!(o.owner && o.plex_pass == Some(true));
    assert_eq!(o.server_name.as_deref(), Some("Home"));
    let got: Vec<(&str, Option<u32>, bool)> = o.libraries.iter().map(|l| (l.title.as_str(), l.unreleased, l.default)).collect();
    assert_eq!(got, [("Music", Some(2), false), ("Live Tapes", Some(0), true)], "the film library is never offered");
    // A server shared with me: nothing to share.
    *r.host.server.lock() = Some(MyServer { machine_id: FRIEND_MACHINE.into(), name: "Alex's".into(), owned: false });
    let o = r.sharing.overview().await.unwrap();
    assert!(!o.owner && o.libraries.is_empty());
    assert_eq!(r.sharing.share("robin", &opts(&["3"], true)).await.unwrap_err().kind(), "invalid");
}

#[tokio::test]
async fn a_library_platter_hasnt_read_counts_as_unknown() {
    let tv = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/servers/{MACHINE}")))
        .respond_with(ResponseTemplate::new(200).set_body_string(SERVER_XML.replace(r#"key="7" type="artist""#, r#"key="8" type="artist""#)))
        .mount(&tv)
        .await;
    Mock::given(method("GET")).and(path("/api/v2/user")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"subscription": {"active": true}}))).mount(&tv).await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&tv).await;
    let r = rig(&tv);
    let o = r.sharing.overview().await.unwrap();
    assert_eq!(o.libraries[1].unreleased, None);
    // Hiding can't be promised for it, so nothing is shared.
    let st = r.sharing.share("robin", &opts(&["8"], true)).await.unwrap();
    assert!(matches!(st, ShareStatus::Refused { ref message } if message.contains("hasn't read Live Tapes")), "{st:?}");
}

// ---- sharing ----------------------------------------------------------------------

#[tokio::test]
async fn hiding_unreleased_labels_those_albums_and_the_share_leaves_the_label_out() {
    let tv = plex_tv(true).await;
    shares_ok(&tv).await;
    let r = rig(&tv);
    let st = r.sharing.share("robin_listens", &opts(&["3", "7"], true)).await.unwrap();
    assert_eq!(st, ShareStatus::Invited { hidden: 2 });
    assert_eq!(*r.host.labels.lock(), [("3".to_string(), "aaaaaaaa:11".to_string()), ("3".into(), "aaaaaaaa:12".into())], "only the unreleased ones, on their library");
    let body = &share_bodies(&tv).await[0];
    assert_eq!(body["shared_server"]["library_section_ids"], serde_json::json!([901, 903]));
    assert_eq!(body["shared_server"]["invited_email"], "robin_listens");
    assert_eq!(body["sharing_settings"]["filterMusic"], "label!=platter-unreleased");
    assert_eq!(body["sharing_settings"]["allowSync"], "0", "downloads off");
    // The next share doesn't label the same albums again.
    let st = r.sharing.share("jules@example.com", &ShareOptions { allow_downloads: true, ..opts(&["3"], true) }).await.unwrap();
    assert_eq!(st, ShareStatus::Invited { hidden: 2 });
    assert_eq!(r.host.labels.lock().len(), 2);
    let body = &share_bodies(&tv).await[1];
    assert_eq!((body["sharing_settings"]["allowSync"].as_str(), body["sharing_settings"]["filterMusic"].as_str()), (Some("1"), Some("label!=platter-unreleased")));
    // Hiding off: no labels, no filter.
    let st = r.sharing.share("casey", &opts(&["3"], false)).await.unwrap();
    assert_eq!(st, ShareStatus::Invited { hidden: 0 });
    assert_eq!(share_bodies(&tv).await[2]["sharing_settings"]["filterMusic"], "");
}

#[tokio::test]
async fn without_plex_pass_nothing_is_shared_and_the_other_libraries_are_offered() {
    let tv = plex_tv(false).await;
    shares_ok(&tv).await;
    let r = rig(&tv);
    let st = r.sharing.share("robin", &opts(&["3", "7"], true)).await.unwrap();
    assert_eq!(st, ShareStatus::NeedsPlexPass { with_unreleased: vec!["Music".into()], others: vec!["7".into()] });
    assert!(share_bodies(&tv).await.is_empty() && r.host.labels.lock().is_empty(), "nothing shared, nothing labelled");
    // "Share without Music": the other library, which holds nothing unreleased.
    let st = r.sharing.share("robin", &opts(&["7"], true)).await.unwrap();
    assert_eq!(st, ShareStatus::Invited { hidden: 0 });
    let body = &share_bodies(&tv).await[0];
    assert_eq!(body["shared_server"]["library_section_ids"], serde_json::json!([903]));
    assert_eq!(body["sharing_settings"]["filterMusic"], "", "no label filter without Plex Pass");
}

#[tokio::test]
async fn a_label_write_that_fails_shares_nothing() {
    let tv = plex_tv(true).await;
    shares_ok(&tv).await;
    let r = rig(&tv);
    r.host.fail_labels.store(true, Ordering::Relaxed);
    let st = r.sharing.share("robin", &opts(&["3"], true)).await.unwrap();
    assert!(matches!(st, ShareStatus::Refused { ref message } if message.contains("nothing was shared")), "{st:?}");
    assert!(share_bodies(&tv).await.is_empty());
}

#[tokio::test]
async fn non_music_libraries_are_never_shared() {
    let tv = plex_tv(true).await;
    shares_ok(&tv).await;
    let r = rig(&tv);
    for libs in [&["4"][..], &["3", "4"], &["99"], &[]] {
        assert_eq!(r.sharing.share("robin", &opts(libs, false)).await.unwrap_err().kind(), "invalid", "{libs:?}");
    }
    assert!(r.sharing.make_link(opts(&[], false), 24).await.is_err());
    assert!(share_bodies(&tv).await.is_empty());
}

#[tokio::test]
async fn invite_refused_already_shared_unknown_user_and_plex_down() {
    for (code, body, want) in [
        (400, "<Response status=\"User is already sharing this server\"/>", "alreadyShared"),
        (404, "", "unknownUser"),
        (422, "<Response status=\"Invalid user\"/>", "unknownUser"),
        (403, "", "error"),
        (400, "<Response status=\"Not allowed\"/>", "refused"),
        (503, "", "plexDown"),
    ] {
        let tv = plex_tv(true).await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(code).set_body_string(body)).mount(&tv).await;
        let r = rig(&tv);
        let got = match r.sharing.share("robin", &opts(&["7"], true)).await {
            Ok(st) => serde_json::to_value(&st).unwrap()["status"].as_str().unwrap().to_string(),
            Err(e) => {
                assert_eq!(e.kind(), "auth");
                "error".into()
            }
        };
        assert_eq!(got, want, "{code} {body}");
    }
    // plex.tv not answering at all (not pooled: dropping it stops it).
    let tv = MockServer::builder().start().await;
    let r = rig(&tv);
    drop(tv);
    assert!(matches!(r.sharing.share("robin", &opts(&["3"], true)).await.unwrap(), ShareStatus::PlexDown { .. }));
}

// ---- people with access -------------------------------------------------------------

#[tokio::test]
async fn people_with_access_show_what_they_see_and_when_they_were_on_and_can_be_removed() {
    let tv = plex_tv(true).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/servers/{MACHINE}/shared_servers")))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<MediaContainer>
  <SharedServer id="5001" username="robin_listens" userID="71" acceptedAt="1760000000" allowSync="0" filterMusic="label!=platter-unreleased"><Section key="3" shared="1"/><Section key="7" shared="1"/></SharedServer>
  <SharedServer id="5002" username="jules" userID="72" acceptedAt="" allowSync="1" filterMusic=""><Section key="7" shared="1"/></SharedServer>
</MediaContainer>"#,
        ))
        .mount(&tv)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/users"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(r#"<MediaContainer><User id="71"><Server machineIdentifier="{MACHINE}" lastSeenAt="1760100000"/></User></MediaContainer>"#)))
        .mount(&tv)
        .await;
    Mock::given(method("DELETE")).and(path(format!("/api/servers/{MACHINE}/shared_servers/5002"))).respond_with(ResponseTemplate::new(200)).expect(1).mount(&tv).await;
    let r = rig(&tv);
    let people = r.sharing.people().await.unwrap();
    assert_eq!(people.len(), 2);
    let robin = &people[0];
    assert_eq!((robin.name.as_str(), robin.libraries.clone(), robin.unreleased_hidden, robin.pending, robin.last_seen_at), ("robin_listens", vec!["Music".to_string(), "Live Tapes".into()], true, false, Some(1_760_100_000)));
    let jules = &people[1];
    assert!(jules.pending && jules.allow_downloads && !jules.unreleased_hidden && jules.last_seen_at.is_none());
    r.sharing.remove("5002").await.unwrap();
}

// ---- invites to me --------------------------------------------------------------------

async fn invites_tv() -> MockServer {
    let tv = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/invites/requests"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<MediaContainer><Invite id="8001" friend="1" home="0" server="1" friendlyName="Sam"><Server name="Sam's Music"/></Invite><Invite id="8002" friend="1" home="0" server="0" friendlyName="Friend only"/></MediaContainer>"#,
        ))
        .mount(&tv)
        .await;
    Mock::given(method("PUT")).and(path("/api/invites/requests/8001")).and(query_param("server", "1")).respond_with(ResponseTemplate::new(200)).mount(&tv).await;
    tv
}

#[tokio::test]
async fn a_server_invite_is_announced_once_and_accepting_adds_its_music_libraries() {
    let tv = invites_tv().await;
    let r = rig_with(&tv, "alex_on_plex", None);
    let list = r.sharing.invites(true).await.unwrap();
    assert_eq!(list.len(), 1, "only invites that share a server");
    assert_eq!(list[0].from, "Sam");
    assert_eq!(*r.notes.0.lock(), [("Sam shared their music with you".to_string(), "Open Platter to start listening from their server.".to_string())]);
    r.sharing.invites(true).await.unwrap();
    assert_eq!(r.notes.0.lock().len(), 1, "once");
    assert!(r.events.0.lock().iter().any(|(e, _)| e == EVENT_INVITES));
    // Accept: plex.tv lists Sam's server now; its music joins the library here.
    *r.host.before.lock() = vec![ServerEntry { machine_id: MACHINE.into(), name: "Home".into(), owned: true }];
    *r.host.after.lock() = vec![
        ServerEntry { machine_id: MACHINE.into(), name: "Home".into(), owned: true },
        ServerEntry { machine_id: FRIEND_MACHINE.into(), name: "Sam's Music".into(), owned: false },
    ];
    r.host.sections.lock().insert(FRIEND_MACHINE.into(), vec![Section { id: "12".into(), title: "Music".into() }]);
    let told: Arc<Mutex<Vec<String>>> = Arc::default();
    let t = told.clone();
    r.sharing.set_on_accepted(Arc::new(move |e| t.lock().extend(e)));
    let a = r.sharing.accept("8001").await.unwrap();
    assert_eq!(a, Accepted { servers: vec!["Sam's Music".into()], libraries: 1 });
    let entry = format!("{FRIEND_MACHINE}/12");
    assert_eq!(*r.host.included.lock(), std::slice::from_ref(&entry));
    assert_eq!(*told.lock(), [entry], "listen along hears once they're in");
    assert!(r.sharing.invites(false).await.unwrap().is_empty());
    assert_eq!(r.sharing.accept("9999").await.unwrap_err().kind(), "notFound");
}

#[tokio::test]
async fn with_invites_kept_to_plex_no_notification_and_no_name_for_hosts() {
    let tv = invites_tv().await;
    let r = rig_with(&tv, "alex_on_plex", None);
    r.settings.accept_in_app.store(false, Ordering::Relaxed);
    r.sharing.invites(true).await.unwrap();
    assert!(r.notes.0.lock().is_empty());
    assert_eq!(ServerOffer::me(&r.sharing).await, None);
    r.settings.accept_in_app.store(true, Ordering::Relaxed);
    assert_eq!(ServerOffer::me(&r.sharing).await.as_deref(), Some("alex_on_plex"));
}

#[tokio::test]
async fn one_click_from_listen_along_uses_the_default_libraries() {
    let tv = plex_tv(true).await;
    shares_ok(&tv).await;
    let r = rig(&tv);
    assert!(ServerOffer::offered(&r.sharing).await);
    // None chosen yet: nothing is shared.
    assert!(matches!(ServerOffer::invite(&r.sharing, "alex_on_plex").await, OfferOutcome::Failed(ref m) if m.contains("Settings, Sharing")));
    assert!(share_bodies(&tv).await.is_empty());
    *r.settings.default_libraries.write() = vec!["3".into()];
    assert_eq!(ServerOffer::invite(&r.sharing, "alex_on_plex").await, OfferOutcome::Invited);
    let body = &share_bodies(&tv).await[0];
    assert_eq!((body["shared_server"]["library_section_ids"].clone(), body["sharing_settings"]["filterMusic"].as_str()), (serde_json::json!([901]), Some("label!=platter-unreleased")));
    r.settings.offer.store(false, Ordering::Relaxed);
    assert!(!ServerOffer::offered(&r.sharing).await);
}

// ---- share links --------------------------------------------------------------------------

#[tokio::test]
async fn a_share_link_carries_the_request_through_the_sealed_relay_once() {
    let relay = FakeNtfy::start().await;
    let host_tv = plex_tv(true).await;
    shares_ok(&host_tv).await;
    let host = rig_with(&host_tv, "sam_hosts", Some(&relay));
    let friend_tv = invites_tv().await;
    let friend = rig_with(&friend_tv, "alex_on_plex", Some(&relay));
    let link = host.sharing.make_link(opts(&["7"], true), 24).await.unwrap();
    assert!(link.link.starts_with("platter://share/") && link.qr.is_some());
    assert!(!link.link.contains(MACHINE) && !link.link.contains("sam_hosts"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let got = friend.sharing.open_link(&link.link, "Alex").await.unwrap();
    assert_eq!(got, LinkResult::Invited { host: "sam_hosts".into() });
    let body = &share_bodies(&host_tv).await[0];
    assert_eq!(body["shared_server"]["invited_email"], "alex_on_plex");
    assert_eq!(body["shared_server"]["library_section_ids"], serde_json::json!([903]));
    assert!(host.events.0.lock().iter().any(|(e, v)| e == EVENT_NOTICE && v["message"].as_str().unwrap().starts_with("Alex opened your share link")));
    // Nothing readable went through the relay.
    for m in relay.published() {
        assert!(!m.body.contains("alex_on_plex") && !m.body.contains("Alex") && !m.body.contains(MACHINE), "{}", m.body);
    }
    // The same answer if asked again by the same attempt; someone else is turned away.
    let other = rig_with(&friend_tv, "casey_on_plex", Some(&relay));
    let again = other.sharing.open_link(&link.link, "Casey").await.unwrap();
    assert!(matches!(again, LinkResult::Failed { ref message } if message.contains("already used")), "{again:?}");
    assert_eq!(share_bodies(&host_tv).await.len(), 1);
    // The friend looks for the invite it was promised.
    tokio::time::timeout(Duration::from_secs(5), async {
        while friend.sharing.invites(false).await.unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // Not a share link, or run out.
    assert_eq!(friend.sharing.open_link("platter://join/AAAA", "Alex").await.unwrap_err().kind(), "invalid");
}
