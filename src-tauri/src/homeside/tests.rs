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
    let d = Link { id: "l1".into(), secret, relay: "http://relay".into(), side: Side::Desktop, me: "desk".into(), peer: "brdg".into() };
    let b = Link { id: "l1".into(), secret, relay: "http://relay".into(), side: Side::Bridge, me: "brdg".into(), peer: "desk".into() };
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

#[test]
fn a_frame_opened_twice_at_once_is_taken_once() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("l.db")).unwrap();
    let links = super::links::Links::new(db, Arc::new(MemoryStore::new()));
    let (d, b) = pair_links();
    links.add(&b, "Desk", "1", 1).unwrap();
    let mut c = Counter::default();
    let x = link::seal_msg(&d.outbox(), &d.me, &mut c, &Msg::Unpair, NOW).unwrap().remove(0);
    let f = link::open_frame(&b.inbox(), &x, NOW).unwrap();
    // Many at once: only one gets past the guard.
    let taken: usize = std::thread::scope(|s| {
        let hs: Vec<_> = (0..8).map(|_| s.spawn(|| links.accept("l1", &[&f]) as usize)).collect();
        hs.into_iter().map(|h| h.join().unwrap()).sum()
    });
    assert_eq!(taken, 1);
    assert!(!links.accept("l1", &[&f]), "and never again");
    assert!(!links.accept("gone", &[&f]), "a link that isn't there takes nothing");
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
    assert!(b.keys.get(ACCOUNT_TOKEN).unwrap().is_none(), "the Plex account's sign-in never reaches the Bridge");
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
    for p in ["/../config", "artist", "/artist?x=1", "//evil", "/config/host", "/system/backup", "/log/file", "/indexer", "/downloadclient", "/indexer/1", "/downloadclient/2", "/filesystem"] {
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
    // Folder listings only inside Lidarr's folders, named once.
    let fs = |q: Vec<(&str, &str)>| Msg::Get { id: 1, path: "/filesystem".into(), query: q.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() };
    for q in [vec![("path", "/")], vec![("path", "/config")], vec![("path", "/data/staging/music/../../../config")], vec![("path", "/data/staging/music/A"), ("path", "/config")]] {
        assert!(matches!(ask(fs(q.clone())).await, Some(Msg::Failed { .. })), "{q:?}");
    }
    assert!(matches!(ask(fs(vec![("path", "/data/staging/music/A")])).await, Some(Msg::Answer { .. })));
}

#[test]
fn an_older_bridges_hello_means_no_waves() {
    let old: Msg = serde_json::from_value(json!({"t": "hello", "name": "Shed", "version": "0.2.20", "music": "none"})).unwrap();
    assert!(matches!(old, Msg::Hello { waves: false, .. }));
}

struct Quiet;

impl crate::social::Notifier for Quiet {
    fn notify(&self, _: &str, _: &str) {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bring_in_my_listening_goes_to_the_bridge_sealed_and_comes_back() {
    use crate::db::listens::{ImportSource, ImportedPlay, insert_imported};
    use crate::lidarr::waves::{Waves, WavesDeps};
    let ntfy = FakeNtfy::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let relays: Arc<dyn RelayFactory> = Arc::new(Ntfy::new().unwrap());
    let d = side(tmp.path(), "desk");
    let mut s = crate::settings::load(&d.db);
    s.lidarr_url = LIDARR_URL.into();
    s.bring_in = true;
    crate::settings::save(&d.db, &s).unwrap();
    d.lidarr.set_key(Some(LIDARR_KEY)).unwrap();
    // Listening history: one album played a lot.
    let plays: Vec<ImportedPlay> = (0..120)
        .map(|n| ImportedPlay {
            source: ImportSource::File,
            source_id: format!("streaming:{n}"),
            played_at: 1_700_000_000_000 + n * 1000,
            ms_played: Some(200_000),
            title: "Breathe In".into(),
            artist: Some("Northlight".into()),
            album: Some("Second Wind".into()),
            rating_key: None,
        })
        .collect();
    d.db.write(|c| insert_imported(c, &plays, 1)).unwrap();
    let waves = Waves::new(WavesDeps {
        lidarr: d.lidarr.clone(),
        notifier: Arc::new(Quiet),
        day_of: Arc::new(|at| at / 86_400),
        hidden: Arc::new(Vec::new),
        utc_offset: None,
    });
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
    desk.set_waves(waves.clone());
    tokio::spawn(desk.clone().run());
    let start = desk.pair_start(Some(ntfy.url.clone())).unwrap();

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

    // The Bridge says it can run the waves; the desktop hands them over and
    // the Bridge's answer comes back.
    until("the plan on the Bridge", || bridge.waves.plan_running()).await;
    until("the Bridge's answer", || waves.lease().is_some_and(|l| l.acked && l.running)).await;
    assert_eq!(waves.status().unwrap().bridge.as_deref(), Some("Shed Bridge"));
    assert!(crate::settings::load(&b.db).bring_in);
    let on_bridge: i64 = b.db.read(|c| Ok(c.query_row("SELECT count(*) FROM imported_plays", [], |r| r.get(0))?)).unwrap();
    assert_eq!(on_bridge, 0, "no listening history on the Bridge");

    // Off on the desktop: the Bridge stops.
    let mut s = crate::settings::load(&d.db);
    s.bring_in = false;
    crate::settings::save(&d.db, &s).unwrap();
    waves.settings_changed();
    until("the Bridge to stop", || !bridge.waves.plan_running()).await;
    until("the waves back on the desktop", || waves.lease().is_none()).await;

    for m in &ntfy.published() {
        for secret in ["Second Wind", "Northlight", "Breathe In"] {
            assert!(!m.body.contains(secret), "{secret} travelled in the clear");
        }
    }
    let _ = stop_tx.send(true);
}

// ---------------------------------------------------------------- one-command install

const ENV_LIDARR: &str = "http://10.20.30.41:8686";
const ENV_KEY: &str = "envlidarrkey999";

fn bridge_from(b: &End, env: Env, relays: Arc<dyn RelayFactory>) -> Arc<Bridge> {
    Bridge::new(Parts {
        env,
        db: b.db.clone(),
        keys: b.keys.clone(),
        relays,
        clock: Arc::new(super::peer::system_ms),
        store: b.store.clone(),
        lidarr: b.lidarr.clone(),
        web: b.web.clone(),
        plex: None,
        music: MusicAccess::None,
    })
}

fn desktop_for(d: &End, name: &'static str, relays: Arc<dyn RelayFactory>) -> Arc<Desktop> {
    Desktop::new(DesktopDeps {
        db: d.db.clone(),
        keys: d.keys.clone(),
        relays,
        clock: Arc::new(super::peer::system_ms),
        store: d.store.clone(),
        lidarr: d.lidarr.clone(),
        plex: None,
        name: Arc::new(move || name.to_string()),
    })
}

#[tokio::test]
async fn the_bridge_starts_from_its_own_settings_with_no_computer_and_they_win() {
    let tmp = tempfile::tempdir().unwrap();
    let b = side(tmp.path(), "bridge");
    let env = Env {
        name: "Shed Bridge".into(),
        lidarr_url: Some(ENV_LIDARR.into()),
        lidarr_key: Some(ENV_KEY.into()),
        plex_url: Some("http://10.20.30.42:32400".into()),
        ..Default::default()
    };
    let bridge = bridge_from(&b, env, Arc::new(Ntfy::new().unwrap()));
    // Lidarr is set up before any computer said anything.
    assert_eq!(crate::settings::load(&b.db).lidarr_url, ENV_LIDARR);
    let st = b.lidarr.status();
    assert!(st.url_set && st.has_key);
    assert_eq!(b.keys.get(crate::lidarr::KEY_ACCOUNT).unwrap().unwrap().expose(), ENV_KEY);
    assert!(crate::keyhost::trusted(&b.db, crate::lidarr::KEY_ACCOUNT, ENV_LIDARR, &[ENV_LIDARR]), "the key goes only to its own address");
    assert_eq!(bridge.status().from_env, ["lidarrUrl", "lidarrKey", "plexUrl"]);
    // Not paired: it says how, and doesn't wait for a computer's Lidarr settings.
    assert!(bridge.catch_up_once().await.is_none());
    assert!(bridge.status().doing.contains("Add the Bridge"), "{}", bridge.status().doing);
    // A computer's settings come later: the Bridge's own win, the rest is taken.
    let cfg = super::wire::BridgeConfig {
        desktop: "Desk".into(),
        lidarr_url: LIDARR_URL.into(),
        lidarr_key: Some(LIDARR_KEY.into()),
        staging_folder: "/data/staging/music".into(),
        ..Default::default()
    };
    bridge.apply_config(&cfg).await.unwrap();
    let s = crate::settings::load(&b.db);
    assert_eq!(s.lidarr_url, ENV_LIDARR);
    assert_eq!(s.lidarr_staging_folder, "/data/staging/music");
    assert_eq!(b.keys.get(crate::lidarr::KEY_ACCOUNT).unwrap().unwrap().expose(), ENV_KEY);
}

#[test]
fn a_bridge_made_code_pairs_once_and_never_as_the_other_kind() {
    let code = PairCode::new(Some("http://relay".into()), (NOW / 1000) as i64);
    let text = code.bridge_text();
    assert!(text.starts_with("PBB-"), "{text}");
    assert_eq!(PairCode::parse_bridge_made(&text), Some(code.clone()));
    assert!(PairCode::parse(&text).is_none(), "a Bridge's code isn't a BRIDGE_PAIR_CODE");
    assert!(PairCode::parse_bridge_made(&code.text()).is_none());
    // The Bridge shows it; a phone enters it.
    let mut offer = Offer::from_bridge(code.clone(), "Shed Bridge");
    let (j, (topic, join)) = Joining::start_to_bridge(code.clone(), "Adam's phone", "1.0", NOW).unwrap();
    assert_eq!(topic, offer.listen_topic());
    let ((wtopic, welcome), phone) = offer.answer(&join, NOW).unwrap();
    assert_eq!(wtopic, j.listen_topic());
    assert_eq!(phone.name, "Adam's phone");
    let bridge = j.finish(&welcome, NOW).unwrap();
    assert_eq!(bridge.name, "Shed Bridge");
    assert_eq!(bridge.link.secret, phone.link.secret);
    assert_eq!((phone.link.side, bridge.link.side), (Side::Bridge, Side::Desktop), "each end keeps its own role");
    // Good once.
    let (_, (_, again)) = Joining::start_to_bridge(code.clone(), "Someone else", "1.0", NOW).unwrap();
    assert!(offer.answer(&again, NOW).is_none());
    // A join of the other kind with the same code doesn't open here, and the other way round.
    let mut offer2 = Offer::from_bridge(code.clone(), "Shed Bridge");
    let (_, (_, desk_kind)) = Joining::start(code.clone(), "Shed", "1.0", NOW).unwrap();
    assert!(offer2.answer(&desk_kind, NOW).is_none());
    let mut desk_offer = Offer::new(code.clone(), "Desk");
    let (_, (_, shown_kind)) = Joining::start_to_bridge(code, "Phone", "1.0", NOW).unwrap();
    assert!(desk_offer.answer(&shown_kind, NOW).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_pairs_with_the_code_the_bridge_shows() {
    let ntfy = FakeNtfy::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let relays: Arc<dyn RelayFactory> = Arc::new(Ntfy::new().unwrap());
    // The Bridge, started from its own settings, with no code given.
    let b = side(tmp.path(), "bridge");
    let env = Env {
        name: "Shed Bridge".into(),
        relay: Some(ntfy.url.clone()),
        lidarr_url: Some(ENV_LIDARR.into()),
        lidarr_key: Some(ENV_KEY.into()),
        ..Default::default()
    };
    let bridge = bridge_from(&b, env, relays.clone());
    let (stop_tx, stop) = tokio::sync::watch::channel(false);
    tokio::spawn(bridge.clone().run(stop));
    until("the Bridge's own code", || bridge.shown_code().is_some()).await;
    let code = bridge.shown_code().unwrap();
    let page = super::health::status_text(&bridge);
    assert!(page.contains(&code) && page.contains("Add the Bridge"), "{page}");

    // The phone: no Lidarr of its own.
    let p = side(tmp.path(), "phone");
    // The test relay runs on this computer: like anyone running their own
    // relay, the phone has it set, or a pasted code naming it is refused.
    let mut ps = crate::settings::load(&p.db);
    ps.listen_relay = ntfy.url.clone();
    crate::settings::save(&p.db, &ps).unwrap();
    let phone = desktop_for(&p, "Adam's phone", relays.clone());
    tokio::spawn(phone.clone().run());
    // The kind of code a computer makes is turned away at once, with words.
    let wrong = PairCode::new(None, (NOW / 1000) as i64).text();
    assert!(phone.join_bridge(&wrong).await.unwrap_err().to_string().contains("PBB-"));
    let name = tokio::time::timeout(Duration::from_secs(30), phone.join_bridge(&code)).await.unwrap().unwrap();
    assert_eq!(name, "Shed Bridge");
    until("the Bridge to stop showing the code", || bridge.shown_code().is_none()).await;
    assert_eq!(bridge.wire.links.all().len(), 1);
    // The Bridge says hello, so the phone hears it and its music access.
    until("the Bridge answering on the phone", || phone.overview().bridges.first().is_some_and(|x| x.online)).await;
    let ov = phone.overview();
    assert_eq!(ov.bridges[0].name, "Shed Bridge");
    // Its status says which settings are its own.
    until("the Bridge's status", || phone.overview().bridges[0].status.is_some()).await;
    assert_eq!(phone.overview().bridges[0].status.as_ref().unwrap().from_env, ["lidarrUrl", "lidarrKey"]);
    // Lidarr through the Bridge, which uses its own address and key.
    b.web.route("/api/v1/system/status", None, 200, r#"{"appName": "Lidarr", "version": "2.8.2"}"#);
    let route = phone.route().expect("a route through the Bridge");
    let r = crate::lidarr::api::LidarrGet::get(route.as_ref(), "/system/status", &[]).await.unwrap();
    assert_eq!(r.status, 200);
    assert!(b.web.log.lock().iter().any(|r| r.url == format!("{ENV_LIDARR}/api/v1/system/status")));
    // Nothing on the relay in the clear.
    for m in ntfy.published() {
        for secret in [ENV_KEY, ENV_LIDARR, "10.20.30.41", "Shed Bridge", "Adam's phone", &code[4..]] {
            assert!(!m.body.contains(secret) && !m.topic.contains(secret), "{secret} travelled in the clear");
        }
    }
    let _ = stop_tx.send(true);
}

#[tokio::test]
async fn the_bridge_never_keeps_the_plex_account_sign_in() {
    let tmp = tempfile::tempdir().unwrap();
    // A computer sends no account sign-in, whatever it's signed in with.
    let d = side(tmp.path(), "desk");
    d.keys.set(ACCOUNT_TOKEN, &Token::new(PLEX_TOKEN).unwrap()).unwrap();
    let desk = desktop_for(&d, "Desk", Arc::new(Ntfy::new().unwrap()));
    d.keys.set(crate::plex::keychain::SERVER_TOKEN, &Token::new(PLEX_TOKEN).unwrap()).unwrap();
    let cfg = desk.config().await;
    assert!(cfg.plex_token.is_none() && cfg.plex_server_token.is_none());
    assert_eq!(cfg.version, super::VERSION, "the config says which Platter sent it");
    let sent = serde_json::to_value(Msg::Config(Box::new(cfg.clone()))).unwrap();
    assert!(sent["plexToken"].is_null(), "no account sign-in, not even an empty one: {}", sent["plexToken"]);
    assert!(!sent.to_string().contains(PLEX_TOKEN));
    // Read back on a Bridge, it's still no sign-in.
    let back: Msg = serde_json::from_value(sent).unwrap();
    assert!(matches!(back, Msg::Config(c) if c.plex_token.is_none()));

    // An older computer's account sign-in is never kept; a server's own token is, in the keys file.
    let b = side(tmp.path(), "bridge");
    let bridge = bridge_from(&b, Env { name: "Shed".into(), ..Default::default() }, Arc::new(Ntfy::new().unwrap()));
    let old = super::wire::BridgeConfig { desktop: "Old desk".into(), plex_token: Some(PLEX_TOKEN.into()), plex_server: Some("m1".into()), ..Default::default() };
    bridge.apply_config(&old).await.unwrap();
    assert!(b.keys.get(ACCOUNT_TOKEN).unwrap().is_none());
    let shared = super::wire::BridgeConfig {
        plex_server: Some("m1".into()),
        plex_server_token: Some("server-only-token".into()),
        plex_connections: vec!["https://10-0-0-5.example.plex.direct:32400".into()],
        ..Default::default()
    };
    bridge.apply_config(&shared).await.unwrap();
    assert_eq!(b.keys.get(super::run::PLEX_SERVER_TOKEN).unwrap().unwrap().expose(), "server-only-token");
    assert!(b.keys.get(ACCOUNT_TOKEN).unwrap().is_none());
    let kv: Vec<String> = b.db.read(|c| {
        let mut st = c.prepare("SELECT value FROM kv")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(Result::ok).collect())
    }).unwrap();
    assert!(kv.iter().all(|v| !v.contains("server-only-token") && !v.contains(PLEX_TOKEN)), "no token in the database");

    // Only a token Plex gives a server that isn't the account's own counts as narrower.
    use crate::plex::discovery::{Connection, DiscoveredServer};
    let account = Token::new(PLEX_TOKEN).unwrap();
    let server = |id: &str, t: &str| DiscoveredServer {
        machine_id: id.into(),
        name: id.into(),
        owned: false,
        presence: true,
        version: None,
        connections: vec![Connection { uri: "https://10-0-0-5.example.plex.direct:32400".into(), kind: crate::plex::types::ConnectionKind::Local }],
        access_token: Some(Token::new(t).unwrap()),
    };
    let owned = [server("m1", PLEX_TOKEN)];
    assert!(super::run::server_only_token(&owned, &account, Some("m1")).is_none(), "an owned server's token is the account's");
    let both = [server("m1", PLEX_TOKEN), server("m2", "shared-token")];
    let (m, t, uris) = super::run::server_only_token(&both, &account, Some("m2")).unwrap();
    assert_eq!((m.as_str(), t.expose(), uris.len()), ("m2", "shared-token", 1));
    assert!(super::run::server_only_token(&both, &account, None).is_none(), "which server isn't a guess");
}

#[test]
fn the_ignored_sign_in_line_names_the_computer_and_its_version() {
    use super::run::account_sign_in_note;
    let old = account_sign_in_note("Desk", "0.2.29");
    assert!(old.contains("Platter 0.2.29 on Desk") && old.contains("before 0.2.30") && old.contains("Update Platter"), "{old}");
    let unknown = account_sign_in_note("", "");
    assert!(unknown.contains("a computer") && unknown.contains("older than 0.2.30"), "{unknown}");
    // A build that stopped sending it isn't called older.
    for v in ["0.2.30", "0.2.31", "1.0.0"] {
        let line = account_sign_in_note("Desk", v);
        assert!(line.contains(&format!("Platter {v} on Desk")) && !line.contains("older") && !line.contains("before"), "{line}");
    }
}

#[tokio::test]
async fn an_existing_bridges_account_sign_in_is_given_up_once() {
    let tmp = tempfile::tempdir().unwrap();
    let b = side(tmp.path(), "bridge");
    // Left by an older desktop.
    b.keys.set(ACCOUNT_TOKEN, &Token::new(PLEX_TOKEN).unwrap()).unwrap();
    b.keys.set(crate::plex::keychain::SERVER_TOKEN, &Token::new(PLEX_TOKEN).unwrap()).unwrap();
    let bridge = bridge_from(&b, Env { name: "Shed".into(), ..Default::default() }, Arc::new(Ntfy::new().unwrap()));
    bridge.migrate_plex().await;
    assert!(b.keys.get(ACCOUNT_TOKEN).unwrap().is_none());
    assert!(b.keys.get(crate::plex::keychain::SERVER_TOKEN).unwrap().is_none());
    // Once: the Bridge's own sign-in later (plex.tv/link) is kept.
    b.keys.set(ACCOUNT_TOKEN, &Token::new("bridge-own-device-token").unwrap()).unwrap();
    b.db.write(|c| crate::db::store::kv_set(c, super::run::PLEX_OWN_KEY, "1")).unwrap();
    bridge.migrate_plex().await;
    assert_eq!(b.keys.get(ACCOUNT_TOKEN).unwrap().unwrap().expose(), "bridge-own-device-token");
}

#[test]
fn the_keys_file_is_only_for_the_bridge() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("keys.json");
    let keys = super::keys::FileKeys::new(path.clone());
    keys.set(ACCOUNT_TOKEN, &Token::new("t1").unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let _ = keys.get(ACCOUNT_TOKEN).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "put right again");
    }
    assert_eq!(keys.get(ACCOUNT_TOKEN).unwrap().unwrap().expose(), "t1");
    assert!(!path.with_extension("tmp").exists());
}

#[test]
fn the_music_mount_is_matched_against_plexs_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let mount = tmp.path().join("music");
    for f in ["Northlight/First/01.flac", "Northlight/First/02.flac", "Quiet Hours/Low/01.flac"] {
        let p = mount.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"").unwrap();
    }
    let files: Vec<String> = ["/data/media/Music/Northlight/First/01.flac", "/data/media/Music/Northlight/First/02.flac", "/data/media/Music/Quiet Hours/Low/01.flac", "/data/media/Music/Gone/x.flac"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let got = super::run::suggest_plex_path(&files, &mount, |p| p.exists());
    assert_eq!(got.as_deref(), Some("/data/media/Music"));
    // Mounted one folder lower (just one artist): still the right start.
    let low = super::run::suggest_plex_path(&files[..2], &mount.join("Northlight"), |p| p.exists());
    assert_eq!(low.as_deref(), Some("/data/media/Music/Northlight"));
    // Plex on Windows.
    let win = vec![r"D:\Media\Music\Northlight\First\01.flac".to_string()];
    assert_eq!(super::run::suggest_plex_path(&win, &mount, |p| p.exists()).as_deref(), Some(r"D:\Media\Music"));
    // Nothing in common: no guess.
    assert!(super::run::suggest_plex_path(&files, &tmp.path().join("empty"), |p| p.exists()).is_none());
    assert!(super::run::suggest_plex_path(&[], &mount, |p| p.exists()).is_none());
}

#[test]
fn the_status_page_is_plain_and_only_what_it_says() {
    let tmp = tempfile::tempdir().unwrap();
    let b = side(tmp.path(), "bridge");
    let bridge = bridge_from(&b, Env { name: "<Shed>".into(), ..Default::default() }, Arc::new(Ntfy::new().unwrap()));
    let (code, kind, page) = super::health::answer(&bridge, "GET / HTTP/1.1\r\n", 0);
    assert_eq!((code, kind), ("200 OK", "text/html; charset=utf-8"));
    assert!(page.contains("&lt;Shed&gt;") && !page.contains("<Shed>"), "{page}");
    assert_eq!(super::health::answer(&bridge, "GET /status HTTP/1.1\r\n", 0).1, "text/plain; charset=utf-8");
    assert_eq!(super::health::answer(&bridge, "GET /keys.json HTTP/1.1\r\n", 0).0, "404 Not Found");
    assert_eq!(super::health::answer(&bridge, "POST /health HTTP/1.1\r\n", 0).0, "404 Not Found");
    assert!(super::health::answer(&bridge, "GET /health HTTP/1.1\r\n", 0).0.starts_with("200"));
}
