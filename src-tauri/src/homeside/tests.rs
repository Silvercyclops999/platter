//! Platter Bridge: the pair code, sealed frames, pairing, and a desktop and
//! a Bridge talking through an in-process ntfy (never the real ntfy.sh).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use super::code::PairCode;
use super::desktop::{Desktop, DesktopDeps};
use super::link::{self, Counter, Guard, Link, Reassembler, Side};
use super::pair::{Joining, Offer};
use super::run::{Bridge, Env, Parts};
use super::wire::{MusicAccess, Msg};
use crate::db::DbHandle;
use crate::error::AppResult;
use crate::lidarr::check::PlexFiles;
use crate::lidarr::{ArtistMbid, Lidarr, LidarrDeps, PlexMusic};
use crate::plex::keychain::{ACCOUNT_TOKEN, MemoryStore, TokenStore};
use crate::plex::token::Token;
use crate::social::listen::fake_ntfy::FakeNtfy;
use crate::social::listen::relay::{Ntfy, RelayFactory};
use crate::social::listen::seal::Rejected;
use crate::sync::SyncStore;
use crate::sync::store::LIDARR;
use crate::web::Web;
use crate::web::fake::FakeWeb;

const NOW: u64 = 1_800_000_000_000;
const LIDARR_URL: &str = "http://10.20.30.40:8686";
const LIDARR_KEY: &str = "abc123lidarrkey";
const PLEX_TOKEN: &str = "plextoken-SECRET";

// ---------------------------------------------------------------- the code

#[test]
fn a_pair_code_round_trips_and_says_nothing() {
    let c = PairCode::new(None, 1_700_000_000);
    let t = c.text();
    assert!(t.starts_with("PB1-") && t.len() < 70, "{t}");
    assert_eq!(PairCode::parse(&t), Some(c.clone()));
    // Copied with a line break in it.
    let (a, b) = t.split_at(20);
    assert_eq!(PairCode::parse(&format!("{a}\n {b}")), Some(c.clone()));
    let own = PairCode::new(Some("https://ntfy.example.org".into()), 1_700_000_000);
    assert_eq!(PairCode::parse(&own.text()).unwrap().relay.as_deref(), Some("https://ntfy.example.org"));
    for bad in ["", "PB1-", "PB2-abc", "platter://join/abc", &t[..30]] {
        assert!(PairCode::parse(bad).is_none(), "{bad}");
    }
    assert!(c.expired(1_700_000_000 + i64::from(super::code::VALID_S) + 1));
    assert_eq!(format!("{c:?}"), "PairCode(..)");
}

// ---------------------------------------------------------------- frames

fn pair_links() -> (Link, Link) {
    let secret = [7u8; 32];
    let d = Link { id: "l1".into(), secret, relay: "http://relay".into(), side: Side::Desktop, me: "desk".into() };
    let b = Link { id: "l1".into(), secret, relay: "http://relay".into(), side: Side::Bridge, me: "brdg".into() };
    (d, b)
}

#[test]
fn a_big_message_is_split_sealed_and_put_back_together() {
    let (d, b) = pair_links();
    assert_eq!(d.outbox(), b.inbox());
    assert_ne!(d.outbox().topic, d.inbox().topic);
    // Enough that even deflated it needs several pieces.
    let body: String = (0..40_000u32).map(|_| char::from(b'a' + rand::random::<u8>() % 26)).collect();
    let msg = Msg::Answer { id: 3, status: 200, body: body.clone() };
    let mut c = Counter::default();
    let bodies = link::seal_msg(&d.outbox(), &d.me, &mut c, &msg, NOW).unwrap();
    assert!(bodies.len() > 3, "{}", bodies.len());
    assert!(bodies.iter().all(|x| x.starts_with("1.") && x.len() < 4096));
    let mut g = Guard::default();
    let mut r = Reassembler::default();
    let mut out = None;
    for x in &bodies {
        let f = link::open_frame(&b.inbox(), x, NOW).unwrap();
        g.accept(&f).unwrap();
        out = r.add(f, NOW).or(out);
    }
    match out {
        Some(Msg::Answer { id: 3, status: 200, body: got }) => assert_eq!(got, body),
        _ => panic!("not put back together"),
    }
}

#[test]
fn frames_from_elsewhere_late_or_twice_are_dropped() {
    let (d, b) = pair_links();
    let mut c = Counter::default();
    let x = link::seal_msg(&d.outbox(), &d.me, &mut c, &Msg::Unpair, NOW).unwrap().remove(0);
    // Another link's key, or the other direction.
    let other = Link { secret: [8u8; 32], ..b.clone() };
    assert_eq!(link::open_frame(&other.inbox(), &x, NOW).unwrap_err(), Rejected::BadSeal);
    assert!(link::open_frame(&d.inbox(), &x, NOW).is_err());
    // A changed byte.
    let mut t = x.clone().into_bytes();
    t[10] = if t[10] == b'A' { b'B' } else { b'A' };
    assert!(link::open_frame(&b.inbox(), std::str::from_utf8(&t).unwrap(), NOW).is_err());
    // Too late.
    assert_eq!(link::open_frame(&b.inbox(), &x, NOW + link::TTL_MS + 1).unwrap_err(), Rejected::Expired);
    // Twice.
    let mut g = Guard::default();
    let f = link::open_frame(&b.inbox(), &x, NOW).unwrap();
    g.accept(&f).unwrap();
    assert_eq!(g.accept(&f).unwrap_err(), Rejected::Replayed);
    // Counters keep going up across a restart (they start at the clock).
    let mut c2 = Counter::starting_at(c.last());
    assert!(c2.next(NOW) > c.last());
    assert!(Counter::default().next(NOW + 1) > c.last());
}

// ---------------------------------------------------------------- pairing

#[test]
fn a_code_pairs_one_bridge_once() {
    let code = PairCode::new(Some("http://relay".into()), (NOW / 1000) as i64);
    let mut offer = Offer::new(code.clone(), "Desk");
    let (j, (topic, join)) = Joining::start(code.clone(), "Shed", "1.0", NOW).unwrap();
    assert_eq!(topic, offer.listen_topic());
    let ((wtopic, welcome), paired) = offer.answer(&join, NOW).unwrap();
    assert_eq!(wtopic, j.listen_topic());
    assert_eq!(paired.name, "Shed");
    assert!(j.is_welcome(&welcome, NOW));
    let desk = j.finish(&welcome, NOW).unwrap();
    assert_eq!(desk.name, "Desk");
    assert_eq!(desk.link.id, paired.link.id);
    assert_eq!(desk.link.secret, paired.link.secret);
    assert_eq!((desk.link.side, paired.link.side), (Side::Bridge, Side::Desktop));
    // Anyone else with the code later gets nothing.
    let (_, (_, again)) = Joining::start(code.clone(), "Other", "1.0", NOW).unwrap();
    assert!(offer.answer(&again, NOW).is_none(), "the code is good once");
    // A join for another code doesn't open.
    let other = PairCode::new(None, (NOW / 1000) as i64);
    let mut o2 = Offer::new(other, "Desk");
    let (_, (_, wrong)) = Joining::start(code, "Shed", "1.0", NOW).unwrap();
    assert!(o2.answer(&wrong, NOW).is_none());
}

#[test]
fn an_expired_code_does_nothing() {
    let code = PairCode::new(None, 1_000);
    assert!(Joining::start(code, "Shed", "1.0", 1_000 * 1000 + 2 * 3_600_000).is_err());
}

// ---------------------------------------------------------------- end to end

struct NoPlex;

#[async_trait]
impl PlexMusic for NoPlex {
    async fn music_files(&self) -> AppResult<PlexFiles> {
        Ok(PlexFiles::default())
    }
}

struct NoMbid;

#[async_trait]
impl ArtistMbid for NoMbid {
    async fn artist_mbid(&self, _: &str, _: &str) -> Option<String> {
        None
    }
}

struct NoMb;

#[async_trait]
impl crate::lidarr::mbpick::MbGet for NoMb {
    async fn get(&self, _: &str) -> AppResult<Option<serde_json::Value>> {
        Ok(None)
    }
}

struct End {
    db: DbHandle,
    keys: Arc<MemoryStore>,
    store: SyncStore,
    lidarr: Lidarr,
    web: Arc<FakeWeb>,
}

fn side(dir: &std::path::Path, name: &str) -> End {
    let db = DbHandle::open(dir.join(format!("{name}.db"))).unwrap();
    let keys = Arc::new(MemoryStore::new());
    let store = SyncStore::new(db.clone(), name);
    let web = Arc::new(FakeWeb::new());
    let label = name.to_string();
    let lidarr = Lidarr::new(LidarrDeps {
        web: web.clone() as Arc<dyn Web>,
        keys: keys.clone(),
        db: db.clone(),
        plex: Arc::new(NoPlex),
        clock: Arc::new(crate::db::queries::now_s),
        mb: Arc::new(NoMb),
        artist_mbid: Arc::new(NoMbid),
        album_wait: Duration::ZERO,
        shared: crate::lidarr::shared::Shared::new(store.clone(), Arc::new(move || label.clone())),
        plex_server: Arc::new(crate::lidarr::plexside::fake::FakePlex::default()),
    });
    End { db, keys, store, lidarr, web }
}

async fn until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_desktop_and_a_bridge_pair_and_talk_only_in_sealed_messages() {
    let ntfy = FakeNtfy::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let relays: Arc<dyn RelayFactory> = Arc::new(Ntfy::new().unwrap());

    // The desktop: Lidarr and Plex set up.
    let d = side(tmp.path(), "desk");
    let mut s = crate::settings::load(&d.db);
    s.lidarr_url = LIDARR_URL.into();
    s.lidarr_staging_folder = "/data/staging/music".into();
    crate::settings::save(&d.db, &s).unwrap();
    d.lidarr.set_key(Some(LIDARR_KEY)).unwrap();
    d.keys.set(ACCOUNT_TOKEN, &Token::new(PLEX_TOKEN).unwrap()).unwrap();
    let desk = Desktop::new(DesktopDeps {
        db: d.db.clone(),
        keys: d.keys.clone(),
        relays: relays.clone(),
        clock: Arc::new(super::peer::system_ms),
        store: d.store.clone(),
        lidarr: d.lidarr.clone(),
        plex: None,
        name: Arc::new(|| "Desk".to_string()),
    });
    tokio::spawn(desk.clone().run());
    let start = desk.pair_start(Some(ntfy.url.clone())).unwrap();
    assert_eq!(desk.overview().pairing.as_ref().map(|p| p.code.as_str()), Some(start.code.as_str()));

    // The Bridge, started with the code.
    let b = side(tmp.path(), "bridge");
    b.web.route("/api/v1/system/status", None, 200, r#"{"appName": "Lidarr", "version": "2.8.2"}"#);
    let env = Env { data: tmp.path().join("bridge-data"), name: "Shed Bridge".into(), pair_code: Some(start.code.clone()), ..Default::default() };
    let bridge = Bridge::new(Parts {
        env,
        db: b.db.clone(),
        keys: b.keys.clone(),
        relays: relays.clone(),
        clock: Arc::new(super::peer::system_ms),
        store: b.store.clone(),
        lidarr: b.lidarr.clone(),
        web: b.web.clone(),
        plex: None,
        music: MusicAccess::ReadOnly,
    });
    assert!(tokio::time::timeout(Duration::from_secs(20), bridge.pair_from_env()).await.unwrap().unwrap());
    let (stop_tx, stop) = tokio::sync::watch::channel(false);
    tokio::spawn(bridge.clone().run(stop));

    // The desktop hears the Bridge and sends how to reach Lidarr, sealed.
    until("the Bridge's settings", || crate::settings::load(&b.db).lidarr_url == LIDARR_URL).await;
    until("the Lidarr key on the Bridge", || b.lidarr.status().has_key).await;
    assert_eq!(b.keys.get(ACCOUNT_TOKEN).unwrap().unwrap().expose(), PLEX_TOKEN);
    assert_eq!(crate::settings::load(&b.db).lidarr_staging_folder, "/data/staging/music");
    until("the Bridge in Settings", || desk.overview().bridges.first().is_some_and(|x| x.name == "Shed Bridge" && x.online)).await;
    let ov = desk.overview();
    assert!(ov.pairing.is_none(), "the code is used up");
    assert_eq!(ov.bridges[0].music, MusicAccess::ReadOnly);
    // The Bridge already used this code: starting again with it does nothing.
    assert!(!bridge.pair_from_env().await.unwrap());

    // The home Platter route: the desktop reads Lidarr through the Bridge.
    let route = desk.route().expect("a route through the Bridge");
    let r = crate::lidarr::api::LidarrGet::get(route.as_ref(), "/system/status", &[]).await.unwrap();
    assert_eq!(r.status, 200);
    assert!(String::from_utf8_lossy(&r.body).contains("Lidarr"));
    let asked = b.web.log.lock().iter().map(|r| r.url.clone()).collect::<Vec<_>>();
    assert!(asked.iter().any(|u| u == &format!("{LIDARR_URL}/api/v1/system/status")), "{asked:?}");
    // Odd paths are refused on the Bridge.
    assert!(crate::lidarr::api::LidarrGet::get(route.as_ref(), "/../../etc", &[]).await.is_err());

    // Shared Lidarr state goes both ways.
    let ds = crate::lidarr::shared::Shared::new(d.store.clone(), Arc::new(|| "Desk".into()));
    ds.put("artist:k1", &json!({"mode": "protected", "name": "Quiet Hours"})).unwrap();
    until("the desktop's entry on the Bridge", || b.store.has_entry(&LIDARR, "artist:k1").unwrap_or(false)).await;
    let bs = crate::lidarr::shared::Shared::new(b.store.clone(), Arc::new(|| "Shed Bridge".into()));
    bs.put("cursor:x", &json!({"lastId": 4, "lastDate": "", "device": "Shed Bridge", "at": 1})).unwrap();
    until("the Bridge's entry on the desktop", || d.store.has_entry(&LIDARR, "cursor:x").unwrap_or(false)).await;

    // Everything on the relay was sealed: no address, key, token, name or key name.
    let all = ntfy.published();
    assert!(all.len() > 4);
    for m in &all {
        assert!(m.body.starts_with("1."), "{}", m.body);
        assert!(m.topic.len() == 40 && m.topic.bytes().all(|c| c.is_ascii_hexdigit()), "{}", m.topic);
        for secret in ["10.20.30.40", LIDARR_KEY, PLEX_TOKEN, "Shed Bridge", "Desk", "artist:k1", "Quiet Hours", &start.code[4..]] {
            assert!(!m.body.contains(secret) && !m.topic.contains(secret), "{secret} travelled in the clear");
        }
    }

    // Unpairing reaches the Bridge.
    let id = desk.overview().bridges[0].id.clone();
    desk.unpair(&id).await.unwrap();
    assert!(desk.overview().bridges.is_empty());
    until("the Bridge to forget the desktop", || bridge.wire.links.all().is_empty()).await;
    let _ = stop_tx.send(true);
}

#[tokio::test]
async fn the_bridge_passes_on_only_lidarr_reads_and_platters_fixed_writes() {
    let web = Arc::new(FakeWeb::new());
    web.route("/api/v1/", None, 200, "{}");
    let home = crate::lidarr::api::DirectRoute::new(web.clone(), LIDARR_URL, Token::new(LIDARR_KEY).unwrap(), vec![], crate::lidarr::api::Route::Home).unwrap();
    web.route("/api/v1/notification", None, 200, r#"[{"id": 1, "name": "Plex", "fields": [{"name": "authToken", "value": "plex-secret"}]}]"#);
    let ask = |m: Msg| {
        let home = &home;
        async move { super::route::serve(Some(home), "/data/staging/music", &m).await }
    };
    let get = |p: &str| Msg::Get { id: 1, path: p.into(), query: vec![] };
    let write = |m: &str, p: &str, b: &str| Msg::Write { id: 2, method: m.into(), path: p.into(), body: b.into() };
    assert!(matches!(ask(get("/artist")).await, Some(Msg::Answer { status: 200, .. })));
    for p in ["/../config", "artist", "/artist?x=1", "//evil", "/config/host", "/system/backup", "/log/file"] {
        assert!(matches!(ask(get(p)).await, Some(Msg::Failed { .. })), "{p}");
    }
    // Keys never travel back.
    let Some(Msg::Answer { body, .. }) = ask(get("/notification")).await else { panic!("no answer") };
    assert!(!String::from_utf8_lossy(&super::route::response(200, &body).body).contains("plex-secret"));
    assert!(matches!(ask(write("PUT", "/album/monitor", r#"{"albumIds": [1], "monitored": false, "extra": 1}"#)).await, Some(Msg::Answer { .. })));
    assert!(matches!(ask(write("DELETE", "/artist/3", "")).await, Some(Msg::Failed { .. })));
    assert!(matches!(ask(write("POST", "/command", r#"{"name": "RescanFolders"}"#)).await, Some(Msg::Failed { .. })));
    assert!(matches!(ask(write("POST", "/rootfolder", r#"{"path": "/", "defaultQualityProfileId": 1, "defaultMetadataProfileId": 1}"#)).await, Some(Msg::Failed { .. })));
    let sent: Vec<String> = web.log.lock().iter().map(|r| r.url.clone()).collect();
    assert_eq!(sent, [format!("{LIDARR_URL}/api/v1/artist"), format!("{LIDARR_URL}/api/v1/notification"), format!("{LIDARR_URL}/api/v1/album/monitor")]);
    assert_eq!(web.bodies("/album/monitor"), [r#"{"albumIds":[1],"monitored":false}"#], "rebuilt from only what Platter sends");
    assert!(super::route::serve(None, "", &get("/artist")).await.is_some_and(|m| matches!(m, Msg::Failed { .. })));
    assert!(super::route::serve(Some(&home), "", &Msg::Unpair).await.is_none());
}
