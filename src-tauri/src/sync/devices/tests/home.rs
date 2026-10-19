//! The home side on device links: a computer with Lidarr set up answers
//! Lidarr requests for the user's other paired devices, the Bridge's way.

use std::sync::atomic::AtomicBool;

use super::app::paired;
use super::*;
use crate::homeside::computer::{ComputerServes, NOT_CHECKED, SETTING_OFF, scrub};
use crate::homeside::wire::SyncHello;
use crate::lidarr::check::PlexFiles;
use crate::lidarr::{ArtistMbid, Lidarr, LidarrDeps, PlexMusic};
use crate::web::Web;
use crate::web::fake::FakeWeb;

const LIDARR_URL: &str = "http://10.20.30.40:8686";
/// The computer's Lidarr key (letters and numbers, as Lidarr makes them).
const KEY: &str = "lidarrkeySECRET42abc";
const STAGING: &str = "/data/staging/music";

struct NoPlex;

#[async_trait]
impl PlexMusic for NoPlex {
    async fn music_files(&self) -> crate::error::AppResult<PlexFiles> {
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
    async fn get(&self, _: &str) -> crate::error::AppResult<Option<serde_json::Value>> {
        Ok(None)
    }
}

fn lidarr_for(d: &Dev, web: Arc<FakeWeb>) -> Lidarr {
    let label = d.id.clone();
    Lidarr::new(LidarrDeps {
        web: web as Arc<dyn Web>,
        keys: d.keys.clone(),
        db: d.db.clone(),
        plex: Arc::new(NoPlex),
        clock: Arc::new(crate::db::queries::now_s),
        mb: Arc::new(NoMb),
        artist_mbid: Arc::new(NoMbid),
        album_wait: Duration::ZERO,
        shared: crate::lidarr::shared::Shared::new(d.store.clone(), Arc::new(move || label.clone())),
        plex_server: Arc::new(crate::lidarr::plexside::fake::FakePlex::default()),
    })
}

/// A computer with Lidarr set up, answering for its other devices.
struct Computer {
    lidarr: Lidarr,
    web: Arc<FakeWeb>,
    on: Arc<AtomicBool>,
}

fn computer(d: &Dev) -> Computer {
    let web = Arc::new(FakeWeb::new());
    web.route("/api/v1/system/status", None, 200, r#"{"appName": "Lidarr", "version": "2.8.2"}"#);
    let mut s = crate::settings::load(&d.db);
    s.lidarr_url = LIDARR_URL.into();
    s.lidarr_staging_folder = STAGING.into();
    crate::settings::save(&d.db, &s).unwrap();
    let lidarr = lidarr_for(d, web.clone());
    lidarr.set_key(Some(KEY)).unwrap();
    let on = Arc::new(AtomicBool::new(true));
    let flag = on.clone();
    d.peer.set_home(Some(Arc::new(ComputerServes { lidarr: lidarr.clone(), db: d.db.clone(), on: Arc::new(move || flag.load(Ordering::SeqCst)) })));
    Computer { lidarr, web, on }
}

fn get(id: u64, path: &str) -> Msg {
    Msg::Get { id, path: path.into(), query: vec![] }
}

fn write(id: u64, method: &str, path: &str, body: &str) -> Msg {
    Msg::Write { id, method: method.into(), path: path.into(), body: body.into() }
}

/// A round so the two have checked each other's hello.
async fn met(a: &Dev, la: &Link, b: &Dev, lb: &Link) {
    round(a, la, b, lb).await;
    assert!(a.peer.verified(&la.id) && b.peer.verified(&lb.id));
}

/// Ask the computer one thing from the phone and run the talk to its end:
/// the answer, and every message that travelled.
async fn ask(phone: &Dev, lph: &Link, pc: &Dev, lpc: &Link, make: impl FnOnce(u64) -> Msg) -> (Option<Result<crate::artistinfo::http::HttpResponse, String>>, Vec<String>) {
    let (id, mut rx) = phone.peer.answers.add(&lph.id);
    let mut tap = Vec::new();
    talk(phone, lph, pc, lpc, vec![make(id)], Net::default(), &mut tap).await;
    (rx.try_recv().ok(), tap)
}

#[tokio::test]
async fn a_computer_answers_lidarr_only_for_a_checked_device_of_the_same_account() {
    let pc = dev("pc");
    let phone = dev("phone");
    let (lpc, lph) = pair(&pc, &phone, "http://relay");
    let c = computer(&pc);

    // Before any hello the request waits, and the computer says hello.
    let (id, mut rx) = phone.peer.answers.add(&lph.id);
    let out = pc.peer.handle(&lpc, get(id, "/system/status")).await;
    assert!(!out.is_empty() && out.iter().all(|m| matches!(m, Msg::SyncHello(_))), "only a hello: {out:?}", out = out.iter().map(Msg::kind).collect::<Vec<_>>());
    assert!(c.web.log.lock().is_empty(), "nothing asked of Lidarr before the hello");
    // The hello round runs, and then the request is answered.
    let mut tap = Vec::new();
    talk(&pc, &lpc, &phone, &lph, out, Net::default(), &mut tap).await;
    let r = rx.try_recv().expect("answered").expect("not refused");
    assert_eq!(r.status, 200);
    assert!(String::from_utf8_lossy(&r.body).contains("Lidarr"));
    assert_eq!(c.web.log.lock().len(), 1);
    assert!(phone.peer.seen(&lph.id).unwrap().lidarr, "its hello said it answers");
    assert!(phone.links.get(&lph.id).unwrap().offers_lidarr, "kept for Settings");

    // A device of another account is never answered.
    let other = dev_with("x", Some("another-account"), None);
    let (lpx, lx) = pair(&pc, &other, "http://relay");
    let (r, _) = ask(&other, &lx, &pc, &lpx, |id| get(id, "/system/status")).await;
    assert!(r.is_none(), "{r:?}");
    let mut tap = Vec::new();
    let hello = other.peer.hello(false).await.into_iter().collect();
    talk(&other, &lx, &pc, &lpx, hello, Net::default(), &mut tap).await;
    let (r, _) = ask(&other, &lx, &pc, &lpx, |id| get(id, "/system/status")).await;
    assert!(matches!(r, Some(Err(ref m)) if m.contains("different account")), "{r:?}");
    assert_eq!(c.web.log.lock().len(), 1, "Lidarr was asked nothing for it");
    // Too many waiting from a link not checked yet: refused plainly.
    let quiet = dev("q");
    let (lpq, _) = pair(&pc, &quiet, "http://relay");
    for i in 0..40 {
        let out = pc.peer.handle(&lpq, get(1000 + i, "/system/status")).await;
        if let Some(Msg::Failed { message, .. }) = out.first() {
            assert_eq!(message, NOT_CHECKED);
        }
    }
    assert_eq!(c.web.log.lock().len(), 1);
}

#[tokio::test]
async fn the_lidarr_key_never_leaves_the_computer() {
    let pc = dev("pc");
    let phone = dev("phone");
    let (lpc, lph) = pair(&pc, &phone, "http://relay");
    let c = computer(&pc);
    met(&phone, &lph, &pc, &lpc).await;
    // Lidarr's answers with keys in every place one could be.
    c.web.route("/api/v1/notification", None, 200, &format!(r#"[{{"id": 1, "name": "Plex", "fields": [{{"name": "apiKey", "value": "{KEY}"}}, {{"name": "authToken", "value": "plex-secret"}}]}}]"#));
    c.web.route("/api/v1/artist", None, 200, r#"[{"id": 1, "artistName": "Northlight", "path": "/music/Northlight"}]"#);
    c.web.route("/api/v1/rootfolder", None, 200, &format!(r#"[{{"id": 1, "path": "/music", "note": "{KEY}"}}]"#));
    c.web.route("/api/v1/album/monitor", None, 202, &format!(r#"[{{"id": 1, "apiKey": "{KEY}"}}]"#));
    let mut all = Vec::new();
    let mut answers = Vec::new();
    for p in ["/system/status", "/notification", "/artist", "/rootfolder", "/config/host", "/../config"] {
        let (r, tap) = ask(&phone, &lph, &pc, &lpc, |id| get(id, p)).await;
        all.extend(tap);
        answers.push(r);
    }
    let (r, tap) = ask(&phone, &lph, &pc, &lpc, |id| write(id, "PUT", "/album/monitor", r#"{"albumIds": [1], "monitored": false}"#)).await;
    all.extend(tap);
    answers.push(r);
    // Lidarr was asked with the key, here.
    assert!(c.web.log.lock().iter().any(|r| r.url.ends_with("/api/v1/notification")));
    // Not one message, nor any answer once unpacked, has it.
    for m in &all {
        assert!(!m.contains(KEY) && !m.contains("plex-secret"), "{m}");
        let msg: Msg = serde_json::from_str(m).unwrap();
        if let Msg::Answer { body, .. } = msg {
            let b = String::from_utf8_lossy(&crate::homeside::route::response(200, &body).body).to_string();
            assert!(!b.contains(KEY) && !b.contains("plex-secret"), "{b}");
        }
    }
    // A read with the key still in a plain field is held back, not sent.
    assert!(matches!(&answers[3], Some(Err(m)) if m.contains("held back")), "{:?}", answers[3]);
    assert!(matches!(&answers[0], Some(Ok(r)) if r.status == 200));
    assert!(matches!(&answers[4], Some(Err(_))) && matches!(&answers[5], Some(Err(_))), "odd reads refused");
}

#[tokio::test]
async fn writes_from_a_device_get_the_bridges_vetting() {
    let pc = dev("pc");
    let phone = dev("phone");
    let (lpc, lph) = pair(&pc, &phone, "http://relay");
    let c = computer(&pc);
    met(&phone, &lph, &pc, &lpc).await;
    c.web.route("/api/v1/", None, 200, "{}");
    c.web.route("/api/v1/system/status", None, 200, r#"{"appName": "Lidarr", "version": "2.8.2"}"#);
    let (home, secrets) = c.lidarr.home_side_route().expect("set up");
    let cases = [
        write(0, "PUT", "/album/monitor", r#"{"albumIds": [1], "monitored": false, "extra": 1}"#),
        write(0, "DELETE", "/artist/3", ""),
        write(0, "POST", "/command", r#"{"name": "RescanFolders"}"#),
        write(0, "POST", "/rootfolder", r#"{"path": "/", "defaultQualityProfileId": 1, "defaultMetadataProfileId": 1}"#),
        write(0, "POST", "/command", r#"{"name": "AlbumSearch", "albumIds": [5], "x": 2}"#),
    ];
    for m in cases {
        // What the Bridge's own code answers, said as a computer.
        let bridge = scrub(crate::homeside::route::serve(Some(&home), STAGING, &m).await.unwrap(), &secrets);
        let Msg::Write { method, path, body, .. } = m else { unreachable!() };
        let (r, _) = ask(&phone, &lph, &pc, &lpc, |id| write(id, &method, &path, &body)).await;
        match (bridge, r) {
            (Msg::Answer { status, .. }, Some(Ok(resp))) => assert_eq!(status, resp.status, "{path}"),
            (Msg::Failed { message, .. }, Some(Err(got))) => {
                assert_eq!(message, got, "{path}");
                assert!(!got.contains("Bridge"), "said as a computer: {got}");
            }
            (b, r) => panic!("{method} {path}: the Bridge said {}, the computer {r:?}", b.kind()),
        }
    }
    // Rebuilt from only what Platter sends, both times.
    assert_eq!(c.web.bodies("/album/monitor"), [r#"{"albumIds":[1],"monitored":false}"#, r#"{"albumIds":[1],"monitored":false}"#]);
    assert_eq!(c.web.bodies("/command"), [r#"{"albumIds":[5],"name":"AlbumSearch"}"#, r#"{"albumIds":[5],"name":"AlbumSearch"}"#]);
    assert!(!c.web.log.lock().iter().any(|r| r.url.contains("/artist/3")), "a refused write never reaches Lidarr");
}

#[tokio::test]
async fn a_request_that_comes_twice_is_made_once() {
    let pc = dev("pc");
    let phone = dev("phone");
    let (lpc, lph) = pair(&pc, &phone, "http://relay");
    let c = computer(&pc);
    met(&phone, &lph, &pc, &lpc).await;
    c.web.route("/api/v1/album/monitor", None, 202, "[]");
    let m = write(77, "PUT", "/album/monitor", r#"{"albumIds": [1], "monitored": false}"#);
    let first = pc.peer.handle(&lpc, m.clone()).await;
    let again = pc.peer.handle(&lpc, m).await;
    assert_eq!(first.len(), 1);
    assert!(matches!(first[0], Msg::Answer { id: 77, status: 202, .. }));
    assert_eq!(serde_json::to_string(&first).unwrap(), serde_json::to_string(&again).unwrap(), "the same answer");
    assert_eq!(c.web.bodies("/album/monitor").len(), 1, "Lidarr was asked once");
}

#[tokio::test]
async fn the_setting_off_refuses_plainly_and_asks_lidarr_nothing() {
    let pc = dev("pc");
    let phone = dev("phone");
    let (lpc, lph) = pair(&pc, &phone, "http://relay");
    let c = computer(&pc);
    c.on.store(false, Ordering::SeqCst);
    met(&phone, &lph, &pc, &lpc).await;
    assert!(!phone.peer.seen(&lph.id).unwrap().lidarr, "its hello says it doesn't answer");
    let (r, _) = ask(&phone, &lph, &pc, &lpc, |id| get(id, "/system/status")).await;
    assert_eq!(r.map(|r| r.map(|_| ())), Some(Err(SETTING_OFF.to_string())));
    assert!(c.web.log.lock().is_empty());
    // On again: the next hello says so, and it answers.
    c.on.store(true, Ordering::SeqCst);
    met(&phone, &lph, &pc, &lpc).await;
    assert!(phone.peer.seen(&lph.id).unwrap().lidarr);
    let (r, _) = ask(&phone, &lph, &pc, &lpc, |id| get(id, "/system/status")).await;
    assert!(matches!(r, Some(Ok(_))), "{r:?}");
}

#[tokio::test]
async fn an_older_device_that_doesnt_answer_lidarr_is_unaffected() {
    // An older computer: it never says `lidarr` and has no home side.
    let old = dev("old");
    let phone = dev("phone");
    let (lold, lph) = pair(&old, &phone, "http://relay");
    let tap = round(&phone, &lph, &old, &lold).await;
    for m in &tap {
        let v: serde_json::Value = serde_json::from_str(m).unwrap();
        assert!(v.get("lidarr").is_none(), "a hello without it reads as before: {m}");
    }
    assert!(same(&old, &phone), "it syncs as before");
    assert!(!phone.peer.seen(&lph.id).unwrap().lidarr);
    assert!(!phone.links.get(&lph.id).unwrap().offers_lidarr);
    // So nothing is ever sent its way for Lidarr.
    let wire = Arc::new(Wire::new(phone.links.clone(), Arc::new(Ntfy::new().unwrap()) as Arc<dyn RelayFactory>, Arc::new(crate::sync::store::system_ms)));
    let sync = Arc::new(DeviceSync::new(phone.peer.clone(), wire, Timing::default()));
    assert!(sync.home_computers().is_empty());
    assert!(sync.computer_route().is_none());
    // A newer hello with the flag and fields this build doesn't know reads fine.
    let mut h: serde_json::Value = serde_json::from_str(&tap[0]).unwrap();
    h["lidarr"] = json!(true);
    h["fromTheFuture"] = json!({"x": 1});
    let Msg::SyncHello(h) = serde_json::from_value::<Msg>(h).unwrap() else { panic!("not a hello") };
    assert!(h.lidarr);
    let plain: SyncHello = serde_json::from_value(json!({"proto": 1, "device": "d", "name": "n", "clockMs": 1, "digests": {}})).unwrap();
    assert!(!plain.lidarr, "an older hello means no");
    // And a kind from a newer build is still ignored.
    let m: Msg = serde_json::from_str(r#"{"t": "lidarrStream", "id": 1}"#).unwrap();
    assert!(old.peer.handle(&lold, m).await.is_empty());
}

#[test]
fn settings_lists_home_sides_in_the_order_they_answer() {
    use crate::homeside::commands::home_sides;
    use crate::homeside::desktop::BridgeInfo;
    use crate::sync::devices::home::ComputerSide;
    let pc = |on| ComputerSide { id: "l1".into(), name: "Studio PC".into(), on };
    let bridge = |online| BridgeInfo { id: "b1".into(), name: "Shed Bridge".into(), version: "1".into(), paired_at: 1, last_seen: None, online, status: None, music: Default::default() };
    let h = home_sides(vec![pc(true)], &[bridge(true)], false, false);
    assert_eq!(h.sides.iter().map(|s| (s.words.as_str(), s.answers)).collect::<Vec<_>>(), [("Your computer at home, on now", true), ("Platter Bridge, on all the time", false)]);
    assert!(h.problem.is_none());
    let h = home_sides(vec![pc(false)], &[bridge(true)], false, false);
    assert_eq!(h.sides.iter().map(|s| s.answers).collect::<Vec<_>>(), [false, true], "the Bridge answers while the computer is off");
    let h = home_sides(vec![pc(false)], &[bridge(false)], false, false);
    assert_eq!(h.problem.as_deref(), Some("Lidarr isn't reachable away from home. Turn on a computer at home or add the Bridge."));
    assert!(home_sides(vec![], &[], false, false).problem.is_some());
    assert!(home_sides(vec![], &[], true, false).problem.is_none(), "this computer is the home side");
    assert!(home_sides(vec![], &[], false, true).problem.is_none(), "an away address of its own");
}

// ---------------------------------------------------------------- as the app runs it

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_reaches_lidarr_through_the_computer_on_the_local_network() {
    let (pc, phone) = paired().await;
    let c = computer(&pc.dev);
    pc.sync.sync_now().await;
    let side = phone.sync.home_computers();
    assert_eq!(side.len(), 1);
    assert!(side[0].on, "{side:?}");
    assert_eq!(side[0].name, "Device pc");
    let route = phone.sync.computer_route().expect("a route through the computer");
    assert_eq!(crate::lidarr::api::LidarrGet::route(route.as_ref()), crate::lidarr::api::Route::Computer);
    let r = crate::lidarr::api::LidarrGet::get(route.as_ref(), "/system/status", &[]).await.unwrap();
    assert_eq!(r.status, 200);
    assert!(c.web.log.lock().iter().any(|r| r.url == format!("{LIDARR_URL}/api/v1/system/status")));

    // The phone's own Lidarr has no address and no key, and goes through it.
    let web = Arc::new(FakeWeb::new());
    let mine = lidarr_for(&phone.dev, web.clone());
    let ds = Arc::downgrade(&phone.sync);
    mine.set_computer_route(Some(Arc::new(move || ds.upgrade().and_then(|d| d.computer_route()))));
    let st = mine.status();
    assert!(!st.has_key && !st.url_set && st.home_side && st.usable() && !st.ready, "{st:?}");
    let (client, status, _) = Lidarr::connect(vec![phone.sync.computer_route().unwrap()]).await.unwrap();
    assert_eq!(status.version, "2.8.2");
    assert_eq!(client.route(), crate::lidarr::api::Route::Computer);
    assert!(web.log.lock().is_empty(), "the phone itself asked nobody");

    // Turned off on the computer: a plain refusal.
    c.on.store(false, Ordering::SeqCst);
    let e = crate::lidarr::api::LidarrGet::get(route.as_ref(), "/system/status", &[]).await.unwrap_err();
    assert!(e.to_string().contains("Let my other devices use Lidarr"), "{e}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn away_from_home_the_request_goes_through_the_relay_sealed() {
    let ntfy = FakeNtfy::start().await;
    let pc = real_clock("pc", ACCT);
    let phone = real_clock("phone", ACCT);
    pair(&pc, &phone, &ntfy.url);
    let c = computer(&pc);
    c.web.route("/api/v1/artist", None, 200, r#"[{"id": 1, "artistName": "Northlight"}]"#);
    let pc = running(pc);
    let phone = running(phone);
    until("the first round", || phone.sync.home_computers().first().is_some_and(|s| s.on)).await;
    let route = phone.sync.computer_route().unwrap();
    let r = crate::lidarr::api::LidarrGet::get(route.as_ref(), "/artist", &[]).await.unwrap();
    assert!(String::from_utf8_lossy(&r.body).contains("Northlight"));
    for m in ntfy.published() {
        assert!(m.body.starts_with("1."), "sealed");
        for plain in [KEY, "Northlight", "10.20.30.40", "/artist"] {
            assert!(!m.body.contains(plain) && !m.topic.contains(plain), "{plain} travelled in the clear");
        }
    }
    // Sync when I'm away from home off here: said plainly, nothing sent.
    phone.sync.set_rules(Arc::new(|| false), Arc::new(|| true));
    let before = ntfy.published().len();
    let e = crate::lidarr::api::LidarrGet::get(route.as_ref(), "/artist", &[]).await.unwrap_err();
    assert!(e.to_string().contains("Sync when I'm away from home is off"), "{e}");
    assert_eq!(ntfy.published().len(), before);
    let _ = pc.stop.send(true);
    let _ = phone.stop.send(true);
}
