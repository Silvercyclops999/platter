//! The connected account against a faked service (wiremock) and a faked
//! browser that answers the loopback sign-in. No real network.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::api::{Endpoints, REFRESH_ACCOUNT, SCOPES};
use super::callback::{Answer, parse_target};
use super::*;
use crate::db::DbHandle;
use crate::plex::keychain::{MemoryStore, TokenStore};
use crate::plex::token::Token;

const CLIENT: &str = "0123456789abcdef0123456789abcdef";
const A: &str = "aaaaaaaa";

struct Events(Mutex<Vec<Value>>);
impl EventSink for Events {
    fn emit(&self, event: &str, v: Value) {
        assert_eq!(event, EVENT);
        self.0.lock().push(v);
    }
}

/// What the faked browser does with the sign-in page.
#[derive(Clone, Copy, PartialEq)]
enum Browser {
    Allow,
    Deny,
    WrongState,
    /// Never answers.
    Nothing,
}

struct Rig {
    server: MockServer,
    acct: AccountImport,
    keys: Arc<MemoryStore>,
    events: Arc<Events>,
    cfg: Arc<Mutex<Config>>,
    clock: Arc<AtomicI64>,
    opened: Arc<Mutex<Vec<String>>>,
    browser: Arc<Mutex<Browser>>,
    offline: Arc<AtomicBool>,
    db: DbHandle,
    _dir: tempfile::TempDir,
}

fn query(url: &str, key: &str) -> Option<String> {
    url::Url::parse(url).ok()?.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
}

async fn rig() -> Rig {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("a.db")).unwrap();
    let keys = Arc::new(MemoryStore::new());
    let events = Arc::new(Events(Mutex::new(Vec::new())));
    let cfg = Arc::new(Mutex::new(Config { enabled: true, client_id: String::new(), recheck: true }));
    keys.set(CLIENT_ACCOUNT, &Token::new(CLIENT).unwrap()).unwrap();
    let clock = Arc::new(AtomicI64::new(1_700_000_000_000));
    let opened = Arc::new(Mutex::new(Vec::new()));
    let browser = Arc::new(Mutex::new(Browser::Allow));
    let offline = Arc::new(AtomicBool::new(false));
    let web: Arc<dyn Web> = Arc::new(crate::web::Switched { inner: Arc::new(crate::web::ReqwestWeb::new().unwrap()), off: offline.clone() });
    let open = {
        let (opened, browser) = (opened.clone(), browser.clone());
        Arc::new(move |url: &str| -> Result<(), String> {
            opened.lock().push(url.to_string());
            let b = *browser.lock();
            let redirect = query(url, "redirect_uri").unwrap();
            let state = query(url, "state").unwrap();
            tokio::spawn(async move {
                let c = reqwest::Client::new();
                // A browser asks for an icon too; the wait goes on.
                let icon = c.get(redirect.replace("/callback", "/favicon.ico")).send().await.unwrap();
                assert_eq!(icon.status(), 404);
                let target = match b {
                    Browser::Allow => format!("{redirect}?code=the-code&state={state}"),
                    Browser::Deny => format!("{redirect}?error=access_denied&state={state}"),
                    Browser::WrongState => {
                        // Something else pokes the port first; the real answer follows.
                        let poke = c.get(format!("{redirect}?code=not-yours&state=someone-elses")).send().await.unwrap();
                        assert_eq!(poke.status(), 400);
                        format!("{redirect}?code=the-code&state={state}")
                    }
                    Browser::Nothing => return,
                };
                let page = c.get(target).send().await.unwrap();
                assert!(page.text().await.unwrap().contains("Platter"));
            });
            Ok(())
        })
    };
    let acct = AccountImport::new(Deps {
        web,
        keys: keys.clone(),
        db: db.clone(),
        events: events.clone(),
        open,
        config: {
            let cfg = cfg.clone();
            Arc::new(move || cfg.lock().clone())
        },
        clock: {
            let clock = clock.clone();
            Arc::new(move || clock.load(Ordering::SeqCst))
        },
        ends: Endpoints { accounts: server.uri(), api: format!("{}/v1", server.uri()) },
        sign_in_wait: Duration::from_secs(5),
    });
    Rig { server, acct, keys, events, cfg, clock, opened, browser, offline, db, _dir: dir }
}

impl Rig {
    fn api(&self) -> String {
        format!("{}/v1", self.server.uri())
    }

    fn signed_in(&self, refresh: &str) {
        self.keys.set(REFRESH_ACCOUNT, &Token::new(refresh).unwrap()).unwrap();
    }

    fn refresh_token(&self) -> Option<String> {
        self.keys.get(REFRESH_ACCOUNT).unwrap().map(|t| t.expose().to_string())
    }

    async fn token_answers(&self, access: &str, refresh: Option<&str>) {
        let mut body = json!({ "access_token": access, "token_type": "Bearer", "expires_in": 3600, "scope": SCOPES });
        if let Some(r) = refresh {
            body["refresh_token"] = json!(r);
        }
        Mock::given(method("POST")).and(path("/api/token")).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(&self.server).await;
    }

    async fn get(&self, p: &str, body: Value) {
        Mock::given(method("GET")).and(path(format!("/v1{p}"))).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(&self.server).await;
    }

    async fn me(&self) {
        self.get("/me", json!({ "id": "me", "display_name": "Sam" })).await;
    }

    async fn requests_to(&self, p: &str) -> usize {
        self.server.received_requests().await.unwrap_or_default().iter().filter(|r| r.url.path() == p).count()
    }

    fn library(&self) {
        self.db
            .write(|c| {
                c.execute(
                    "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, matched, subformats)
                     VALUES (?1, 'Shoreline', 'Shoreline', ?2, 'Low Tide', 1, ?3, 1, '[]')",
                    rusqlite::params![format!("{A}:al1"), format!("{A}:ar:lt"), A],
                )?;
                for (id, title) in [("1", "Quiet Please (2020 Remaster)"), ("2", "Golden Hour")] {
                    c.execute(
                        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, src)
                         VALUES (?1, ?2, ?3, 'Shoreline', ?4, 'Low Tide', 200000, ?5, ?6)",
                        rusqlite::params![format!("{A}:{id}"), title, format!("{A}:al1"), format!("{A}:ar:lt"), format!("/p/{id}"), A],
                    )?;
                }
                c.execute("INSERT INTO file_tags (part_key, status, isrc, scanned_at, ver) VALUES ('/p/1', 'ok', 'QZABC2100001', 1, 1)", [])?;
                crate::identity::index::rebuild(c)?;
                Ok(())
            })
            .unwrap();
    }
}

fn track(name: &str, artist: &str, isrc: Option<&str>) -> Value {
    let mut t = json!({
        "type": "track",
        "name": name,
        "artists": [{ "name": artist }, { "name": "Guest" }],
        "album": { "name": "Shoreline" },
        "duration_ms": 200_000,
        "uri": format!("x:track:{}", name.len()),
        "external_ids": {}
    });
    if let Some(i) = isrc {
        t["external_ids"]["isrc"] = json!(i);
    }
    t
}

fn item(t: Value) -> Value {
    json!({ "added_at": "2024-01-01T00:00:00Z", "is_local": false, "item": t.clone(), "track": t })
}

fn playlist(id: &str, name: &str, owner: &str, collaborative: bool) -> Value {
    json!({ "id": id, "name": name, "owner": { "id": owner }, "collaborative": collaborative, "items": { "href": "", "total": 0 } })
}

fn imported_lists(db: &DbHandle) -> Vec<(String, i64)> {
    db.read(|c| {
        let mut st = c.prepare(
            "SELECT p.name, (SELECT count(*) FROM import_playlist_items i WHERE i.playlist_id = p.id) FROM import_playlists p WHERE p.source = ?1 ORDER BY p.name",
        )?;
        let rows = st.query_map([SOURCE], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .unwrap()
}

// ------------------------------------------------------------------ PKCE and the callback

#[test]
fn pkce_verifier_and_challenge() {
    let v = pkce::verifier();
    assert_eq!(v.len(), 43);
    assert!(v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    assert_ne!(v, pkce::verifier(), "a fresh one each time");
    assert_eq!(pkce::challenge("platter-test-verifier-0123456789-abcdefghijk"), "mmknEE6LbxMZF8A7t-bA-2ETs2XmZBR3SklapmPAIKo");
    assert!(pkce::state().len() >= 32);
}

#[test]
fn the_callback_checks_the_state() {
    assert_eq!(parse_target("/callback?code=c1&state=s1", "s1"), Answer::Code("c1".into()));
    assert_eq!(parse_target("/callback?code=c1&state=s2", "s1"), Answer::WrongState);
    assert_eq!(parse_target("/callback?code=c1", "s1"), Answer::WrongState);
    assert_eq!(parse_target("/callback?error=access_denied&state=s1", "s1"), Answer::Error("access_denied".into()));
    // An error with someone else's state is still just a wrong answer.
    assert_eq!(parse_target("/callback?error=access_denied&state=x", "s1"), Answer::WrongState);
    assert_eq!(parse_target("/favicon.ico", "s1"), Answer::Other);
    assert_eq!(parse_target("/callback?state=s1", "s1"), Answer::Error("no code".into()));
}

#[test]
fn the_sign_in_link_asks_only_to_read_playlists() {
    let api = api::Api::new(
        Arc::new(crate::web::fake::FakeWeb::new()),
        Arc::new(MemoryStore::new()),
        Endpoints::default(),
        Arc::new(|| 0),
    );
    let url = api.authorize_url(CLIENT, "http://127.0.0.1:5000/callback", "chal", "st");
    assert!(url.starts_with("https://accounts.spotify.com/authorize?"));
    assert_eq!(query(&url, "scope").as_deref(), Some("playlist-read-private playlist-read-collaborative"));
    assert_eq!(query(&url, "code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(query(&url, "response_type").as_deref(), Some("code"));
    assert_eq!(query(&url, "redirect_uri").as_deref(), Some("http://127.0.0.1:5000/callback"));
    assert!(query(&url, "client_secret").is_none());
}

// ------------------------------------------------------------------ signing in

#[tokio::test]
async fn signing_in_through_the_browser_keeps_the_refresh_token_in_the_keychain() {
    let r = rig().await;
    r.token_answers("acc-1", Some("ref-1")).await;
    assert_eq!(r.acct.status().state, State::SignedOut);
    let st = r.acct.sign_in().await;
    assert_eq!(st.state, State::Connected, "{st:?}");
    assert_eq!(r.refresh_token().as_deref(), Some("ref-1"));

    // The code went back with the verifier the link's challenge was made from.
    let url = r.opened.lock()[0].clone();
    let reqs = r.server.received_requests().await.unwrap();
    let token = reqs.iter().find(|q| q.url.path() == "/api/token").unwrap();
    let form: std::collections::HashMap<String, String> = url::form_urlencoded::parse(&token.body).into_owned().collect();
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "the-code");
    assert_eq!(form["client_id"], CLIENT);
    assert_eq!(pkce::challenge(&form["code_verifier"]), query(&url, "code_challenge").unwrap());
    assert_eq!(form["redirect_uri"], query(&url, "redirect_uri").unwrap());
    assert!(form["redirect_uri"].starts_with("http://127.0.0.1:") && form["redirect_uri"].ends_with("/callback"));
    assert!(!form.contains_key("client_secret"));

    // The status the UI sees passed through signing in.
    let states: Vec<String> = r.events.0.lock().iter().map(|v| v["state"].as_str().unwrap().to_string()).collect();
    assert!(states.contains(&"signingIn".to_string()) && states.last().unwrap() == "connected", "{states:?}");
    // No token in anything the UI sees.
    assert!(!r.events.0.lock().iter().any(|v| v.to_string().contains("ref-1") || v.to_string().contains("acc-1")));
}

#[tokio::test]
async fn saying_no_on_the_sign_in_page_is_refused() {
    let r = rig().await;
    *r.browser.lock() = Browser::Deny;
    let st = r.acct.sign_in().await;
    assert_eq!(st.state, State::Refused);
    assert!(st.message.unwrap().contains("didn't allow"));
    assert!(r.refresh_token().is_none());
    assert_eq!(r.requests_to("/api/token").await, 0);
}

#[tokio::test]
async fn an_answer_with_another_state_is_refused_and_the_real_one_still_signs_in() {
    let r = rig().await;
    r.token_answers("acc-1", Some("ref-1")).await;
    *r.browser.lock() = Browser::WrongState;
    let st = r.acct.sign_in().await;
    assert_eq!(st.state, State::Connected, "{st:?}");
    // Only the real code was traded, once.
    assert_eq!(r.requests_to("/api/token").await, 1);
}

#[tokio::test]
async fn a_sign_in_can_be_stopped() {
    let r = rig().await;
    *r.browser.lock() = Browser::Nothing;
    let a = r.acct.clone();
    let t = tokio::spawn(async move { a.sign_in().await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(r.acct.status().state, State::SigningIn);
    r.acct.cancel();
    let st = t.await.unwrap();
    assert_eq!(st.state, State::SignedOut);
}

#[tokio::test]
async fn a_client_id_the_service_doesnt_know_is_refused() {
    let r = rig().await;
    Mock::given(method("POST"))
        .and(path("/api/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_client", "error_description": "Invalid client" })))
        .mount(&r.server)
        .await;
    let st = r.acct.sign_in().await;
    assert_eq!(st.state, State::Refused);
    assert!(st.message.unwrap().contains("Client ID"));
}

#[tokio::test]
async fn not_set_up_without_a_client_id() {
    let r = rig().await;
    r.keys.delete(CLIENT_ACCOUNT).unwrap();
    assert_eq!(r.acct.status().state, State::NotSetUp);
    assert_eq!(r.acct.sign_in().await.state, State::NotSetUp);
    assert!(r.opened.lock().is_empty());
    r.signed_in("ref-1");
    assert_eq!(r.acct.read().await.state, State::NotSetUp);
    assert!(r.server.received_requests().await.unwrap().is_empty());
    // Turned off in Settings: nothing happens either.
    r.keys.set(CLIENT_ACCOUNT, &Token::new(CLIENT).unwrap()).unwrap();
    r.cfg.lock().enabled = false;
    r.acct.read().await;
    assert!(r.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn signing_out_removes_the_token() {
    let r = rig().await;
    r.signed_in("ref-1");
    assert_eq!(r.acct.status().state, State::Connected);
    assert_eq!(r.acct.sign_out().state, State::SignedOut);
    assert!(r.refresh_token().is_none());
}

// ------------------------------------------------------------------ reading

async fn two_pages_of_playlists(r: &Rig) {
    let api = r.api();
    Mock::given(method("GET"))
        .and(path("/v1/me/playlists"))
        .and(query_param("offset", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [playlist("p3", "Their Mix", "someone", false), playlist("p4", "Shared Jams", "someone", true)],
            "next": null, "total": 4
        })))
        .with_priority(1)
        .mount(&r.server)
        .await;
    r.get(
        "/me/playlists",
        json!({
            "items": [playlist("p1", "Road Trip", "me", false), playlist("p2", "Evening", "me", false)],
            "next": format!("{api}/me/playlists?offset=2&limit=50"), "total": 4
        }),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/v1/playlists/p1/items"))
        .and(query_param("offset", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [item(json!({ "type": "episode", "name": "A Podcast", "show": {} })), json!({ "item": null, "track": null })],
            "next": null
        })))
        .with_priority(1)
        .mount(&r.server)
        .await;
    r.get(
        "/playlists/p1/items",
        json!({
            "items": [item(track("Quiet, Please", "Low Tide", Some("QZ-ABC-21-00001"))), item(track("Not Here", "Nobody", None))],
            "next": format!("{api}/playlists/p1/items?offset=2&limit=50")
        }),
    )
    .await;
    r.get("/playlists/p2/items", json!({ "items": [item(track("Golden Hour", "Low Tide", None))], "next": null })).await;
    Mock::given(method("GET"))
        .and(path("/v1/playlists/p4/items"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "error": { "status": 403, "message": "Forbidden" } })))
        .mount(&r.server)
        .await;
}

#[tokio::test]
async fn reading_pages_through_playlists_and_feeds_the_import() {
    let r = rig().await;
    r.library();
    r.signed_in("ref-1");
    r.token_answers("acc-1", None).await;
    r.me().await;
    two_pages_of_playlists(&r).await;

    let st = r.acct.read().await;
    assert_eq!(st.state, State::Connected, "{st:?}");
    assert_eq!(st.name.as_deref(), Some("Sam"));
    assert!(st.last_read.is_some());

    // Every call carried the access token from the refresh.
    let reqs = r.server.received_requests().await.unwrap();
    assert!(reqs.iter().filter(|q| q.url.path().starts_with("/v1/")).all(|q| q.headers.get("authorization").unwrap() == "Bearer acc-1"));
    // Other people's playlist wasn't asked for; the shared one was tried.
    assert_eq!(r.requests_to("/v1/playlists/p3/items").await, 0);
    assert_eq!(r.requests_to("/v1/playlists/p4/items").await, 1);
    assert_eq!(r.requests_to("/v1/playlists/p1/items").await, 2, "both pages");

    assert_eq!(imported_lists(&r.db), [("Evening".to_string(), 1), ("Road Trip".to_string(), 2)]);
    let reports = r.db.read(crate::import::store::reports).unwrap();
    let rep = reports.iter().find(|x| x.source == SOURCE).unwrap();
    assert_eq!(rep.playlists, 2);
    assert_eq!((rep.songs.total, rep.songs.matched, rep.songs.missing), (3, 2, 1));
    assert_eq!(rep.skipped.others_playlists, 2);
    assert_eq!(rep.skipped.podcasts, 1);
    assert_eq!(rep.skipped.incomplete_rows, 1);
    assert_eq!(rep.file_name, None);
    // Matched by ISRC, though the library's title differs.
    let (status, tier, isrc): (String, Option<String>, Option<String>) = r
        .db
        .read(|c| Ok(c.query_row("SELECT status, tier, isrc FROM import_songs WHERE source = ?1 AND title = 'Quiet, Please'", [SOURCE], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?)))?))
        .unwrap();
    assert_eq!((status.as_str(), tier.as_deref(), isrc.as_deref()), ("matched", Some("isrc"), Some("QZABC2100001")));
    // The missing song is on the Missing list.
    let missing = r.db.read(|c| crate::import::store::missing(c, 100)).unwrap();
    assert!(missing.iter().any(|m| m.title == "Not Here"));
    // Progress went out: playlists done of the total.
    let last_reading = r.events.0.lock().iter().rev().find(|v| v["state"] == "reading").cloned().unwrap();
    assert_eq!((last_reading["done"].as_u64(), last_reading["total"].as_u64()), (Some(4), Some(4)));
}

#[tokio::test]
async fn reading_again_adds_and_updates_without_removing() {
    let r = rig().await;
    r.signed_in("ref-1");
    r.token_answers("acc-1", None).await;
    r.me().await;
    two_pages_of_playlists(&r).await;
    r.acct.read().await;
    assert_eq!(imported_lists(&r.db).len(), 2);

    // A day later the account has only Road Trip, with a song added.
    r.server.reset().await;
    r.token_answers("acc-1", None).await;
    r.me().await;
    r.get("/me/playlists", json!({ "items": [playlist("p1", "Road Trip", "me", false)], "next": null })).await;
    r.get(
        "/playlists/p1/items",
        json!({ "items": [item(track("Quiet, Please", "Low Tide", None)), item(track("Not Here", "Nobody", None)), item(track("New One", "Nobody", None))], "next": null }),
    )
    .await;
    r.clock.fetch_add(DAY_MS + 1, Ordering::SeqCst);
    assert!(r.acct.due());
    r.acct.read().await;
    assert_eq!(imported_lists(&r.db), [("Evening".to_string(), 1), ("Road Trip".to_string(), 3)], "Evening stays");
    assert!(!r.acct.due(), "just read");
}

#[tokio::test]
async fn the_daily_check_follows_its_setting() {
    let r = rig().await;
    assert!(!r.acct.due(), "not signed in");
    r.signed_in("ref-1");
    assert!(r.acct.due(), "never read");
    r.cfg.lock().recheck = false;
    assert!(!r.acct.due());
    r.cfg.lock().recheck = true;
    r.cfg.lock().enabled = false;
    assert!(!r.acct.due());
}

#[tokio::test]
async fn a_short_retry_after_is_waited_out() {
    let r = rig().await;
    r.signed_in("ref-1");
    r.token_answers("acc-1", None).await;
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&r.server)
        .await;
    r.me().await;
    r.get("/me/playlists", json!({ "items": [], "next": null })).await;
    let t = std::time::Instant::now();
    let st = r.acct.read().await;
    assert_eq!(st.state, State::Connected, "{st:?}");
    assert!(t.elapsed() >= Duration::from_millis(900));
    assert_eq!(r.requests_to("/v1/me").await, 2);
}

#[tokio::test]
async fn a_long_rate_limit_says_when_to_try_again() {
    let r = rig().await;
    r.signed_in("ref-1");
    r.token_answers("acc-1", None).await;
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "600"))
        .mount(&r.server)
        .await;
    let st = r.acct.read().await;
    assert_eq!(st.state, State::RateLimited);
    let now = r.clock.load(Ordering::SeqCst);
    assert_eq!(st.retry_at, Some(now + 600_000));
    assert!(!r.acct.due(), "the daily check waits too");
    r.clock.fetch_add(601_000, Ordering::SeqCst);
    assert_eq!(r.acct.status().state, State::Connected);
}

#[tokio::test]
async fn stop_ends_a_rate_limit_wait() {
    let r = rig().await;
    r.signed_in("ref-1");
    r.token_answers("acc-1", None).await;
    Mock::given(method("GET")).and(path("/v1/me")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "20")).mount(&r.server).await;
    let a = r.acct.clone();
    let t = tokio::spawn(async move { a.read().await });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(r.acct.status().state, State::Reading);
    r.acct.cancel();
    let st = tokio::time::timeout(Duration::from_secs(3), t).await.unwrap().unwrap();
    assert_eq!(st.state, State::Connected);
    assert!(imported_lists(&r.db).is_empty());
}

#[tokio::test]
async fn an_expired_access_token_is_refreshed_and_a_new_refresh_token_kept() {
    let r = rig().await;
    r.signed_in("ref-1");
    Mock::given(method("POST"))
        .and(path("/api/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains(format!("client_id={CLIENT}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "access_token": "acc-2", "expires_in": 3600, "refresh_token": "ref-2" })))
        .mount(&r.server)
        .await;
    // The service stops taking acc-2 (revoked early): one 401, refresh, try again.
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .and(header("authorization", "Bearer acc-2"))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&r.server)
        .await;
    r.me().await;
    r.get("/me/playlists", json!({ "items": [], "next": null })).await;
    let st = r.acct.read().await;
    assert_eq!(st.state, State::Connected, "{st:?}");
    assert_eq!(r.refresh_token().as_deref(), Some("ref-2"), "the rotated token is kept");
    assert_eq!(r.requests_to("/api/token").await, 2);
    let bodies: Vec<String> = r
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|q| q.url.path() == "/api/token")
        .map(|q| String::from_utf8_lossy(&q.body).into_owned())
        .collect();
    assert!(bodies[0].contains("refresh_token=ref-1") && bodies[1].contains("refresh_token=ref-2"));

    // An hour on, the access token has run out and is refreshed first.
    r.clock.fetch_add(3_600_000, Ordering::SeqCst);
    r.acct.read().await;
    assert_eq!(r.requests_to("/api/token").await, 3);
}

#[tokio::test]
async fn a_refresh_token_that_stopped_working_asks_to_sign_in_again() {
    let r = rig().await;
    r.signed_in("ref-old");
    Mock::given(method("POST"))
        .and(path("/api/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant", "error_description": "Refresh token revoked" })))
        .mount(&r.server)
        .await;
    let st = r.acct.read().await;
    assert_eq!(st.state, State::Expired);
    assert!(r.refresh_token().is_none(), "the dead token is removed");
    assert_eq!(r.requests_to("/v1/me").await, 0);
}

#[tokio::test]
async fn an_account_the_developer_app_doesnt_allow_is_refused() {
    let r = rig().await;
    r.signed_in("ref-1");
    r.token_answers("acc-1", None).await;
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(403).set_body_string("Check settings on developer dashboard, the user may not be registered."))
        .mount(&r.server)
        .await;
    let st = r.acct.read().await;
    assert_eq!(st.state, State::Refused);
    assert!(st.message.unwrap().contains("User Management"));
    assert_eq!(r.refresh_token().as_deref(), Some("ref-1"), "still signed in; fixing the app is enough");
}

#[tokio::test]
async fn offline_says_so_and_asks_nothing() {
    let r = rig().await;
    r.signed_in("ref-1");
    r.offline.store(true, Ordering::SeqCst);
    let st = r.acct.read().await;
    assert_eq!(st.state, State::Offline);
    assert!(st.message.unwrap().contains("offline"));
    assert!(r.server.received_requests().await.unwrap().is_empty());
    r.offline.store(false, Ordering::SeqCst);
    r.token_answers("acc-1", None).await;
    r.me().await;
    r.get("/me/playlists", json!({ "items": [], "next": null })).await;
    assert_eq!(r.acct.read().await.state, State::Connected);
}

#[tokio::test]
async fn a_next_link_elsewhere_is_not_followed() {
    let r = rig().await;
    r.signed_in("ref-1");
    r.token_answers("acc-1", None).await;
    r.me().await;
    r.get("/me/playlists", json!({ "items": [playlist("p1", "Road Trip", "me", false)], "next": "http://10.20.30.40:9/steal" })).await;
    r.get("/playlists/p1/items", json!({ "items": [item(track("Golden Hour", "Low Tide", None))], "next": null })).await;
    let st = r.acct.read().await;
    assert_eq!(st.state, State::Connected);
    assert_eq!(imported_lists(&r.db), [("Road Trip".to_string(), 1)]);
}

// ------------------------------------------------------------------ mapping

#[test]
fn track_objects_become_import_songs() {
    let s = song_of(&track("Quiet, Please", "Low Tide", Some("qz-abc-21-00001"))).unwrap();
    assert_eq!((s.artist.as_str(), s.title.as_str(), s.album.as_deref()), ("Low Tide", "Quiet, Please", Some("Shoreline")));
    assert_eq!((s.duration_ms, s.isrc.as_deref()), (Some(200_000), Some("QZABC2100001")));
    assert!(s.service_id.unwrap().starts_with("x:track:"));
    assert!(song_of(&json!({ "name": "No Artist", "artists": [] })).is_none());
    assert!(song_of(&json!({ "name": "", "artists": [{ "name": "X" }] })).is_none());

    let mut skipped = crate::import::Skipped::default();
    let old_style = json!({ "track": track("Older", "Low Tide", None) });
    let songs = songs_of(&[old_style, item(json!({ "type": "episode", "name": "Ep" })), json!({})], &mut skipped);
    assert_eq!(songs.len(), 1, "the older `track` name still reads");
    assert_eq!((skipped.podcasts, skipped.incomplete_rows), (1, 1));
}

// ------------------------------------------------------------------ settings

#[test]
fn settings_daily_check_and_the_source() {
    use crate::settings::{DEVICE_SETTINGS, SYNCED_SETTINGS, Settings, SettingsPatch};
    let mut s = Settings::default();
    assert!(s.streaming_recheck, "re-check daily is on");
    assert!(SYNCED_SETTINGS.contains(&"streamingRecheck") && !DEVICE_SETTINGS.contains(&"streamingRecheck"));
    s.apply(&SettingsPatch { streaming_recheck: Some(false), ..Default::default() }).unwrap();
    assert!(!s.streaming_recheck);
    // The source can be turned on like the file ones.
    s.apply(&SettingsPatch { import_sources: Some(vec![SOURCE.into(), "nope".into()]), ..Default::default() }).unwrap();
    assert_eq!(s.import_sources, [SOURCE]);
    let cfg = Config::from_settings(&s);
    assert!(cfg.enabled && cfg.client_id.is_empty() && !cfg.recheck, "settings never hold the Client ID");
    let json = serde_json::to_string(&s).unwrap().to_lowercase();
    assert!(!json.contains("clientid"));
}

#[tokio::test]
async fn the_client_id_lives_in_the_keychain_and_a_new_one_signs_out() {
    let r = rig().await;
    r.signed_in("ref-1");
    assert_eq!(r.acct.client_id().as_deref(), Some(CLIENT));
    assert!(r.acct.set_client_id(Some("not a client id!")).is_err());
    assert_eq!(r.acct.client_id().as_deref(), Some(CLIENT));
    // The same one again changes nothing.
    assert_eq!(r.acct.set_client_id(Some(&format!(" {CLIENT} "))).unwrap().state, State::Connected);
    // Another app can't use this sign-in.
    let st = r.acct.set_client_id(Some("abcdef0123456789abcdef0123456789")).unwrap();
    assert_eq!(st.state, State::SignedOut);
    assert!(r.refresh_token().is_none());
    assert_eq!(r.acct.client_id().as_deref(), Some("abcdef0123456789abcdef0123456789"));
    // Removing it: not set up.
    assert_eq!(r.acct.set_client_id(None).unwrap().state, State::NotSetUp);
    assert!(r.keys.get(CLIENT_ACCOUNT).unwrap().is_none());
    // The status never carries it.
    assert!(!serde_json::to_string(&r.acct.status()).unwrap().contains("abcdef0123456789"));
}

#[test]
fn picking_a_file_for_the_account_source_says_to_connect() {
    let src = crate::import::sources::source(SOURCE).unwrap();
    let e = src.parser.parse(&[]).unwrap_err().to_string();
    assert!(e.contains("Connect"), "{e}");
}
