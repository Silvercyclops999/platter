//! Pairing two devices: the 6-digit code on one network, the long code
//! through the relay, and every way it should fail.

use std::net::Ipv4Addr;

use super::lan::{Fixed, NoPlayer, key};
use super::*;
use crate::error::{AppError, AppResult};
use crate::homeside::code::PairCode;
use crate::homeside::pair::{DeviceMe, Joining, Offer};
use crate::remote::peer::PeerDevice;
use crate::remote::peer::client::PeerClient;
use crate::remote::peer::keys::RouteList;
use crate::remote::peer::pair::{PairFinish, PairFinishReply, PairHello, PairHelloReply, PairRoutes, PeerPair};
use crate::remote::peer::server::{self, ExtraSlot, PeerServer};
use crate::remote::peer::sync::SyncRoutes;
use crate::sync::devices::pair::{CODE_TTL_MS, DevicePairing, PairDeps, PairPhase, TRIES};

pub(super) const ACCT_TOKEN: &str = "acct-token-SECRET-9";

/// Every pairing request and answer as it crossed the network.
struct Tap {
    inner: Arc<DevicePairing>,
    wire: Mutex<Vec<String>>,
}

#[async_trait]
impl PeerPair for Tap {
    async fn waiting(&self) -> bool {
        self.inner.waiting().await
    }
    async fn hello(&self, from: &str, req: PairHello) -> AppResult<PairHelloReply> {
        self.wire.lock().push(serde_json::to_string(&req).unwrap());
        let r = self.inner.hello(from, req).await;
        if let Ok(r) = &r {
            self.wire.lock().push(serde_json::to_string(r).unwrap());
        }
        r
    }
    async fn finish(&self, from: &str, req: PairFinish) -> AppResult<PairFinishReply> {
        self.wire.lock().push(serde_json::to_string(&req).unwrap());
        let r = self.inner.finish(from, req).await;
        if let Ok(r) = &r {
            self.wire.lock().push(serde_json::to_string(r).unwrap());
        }
        r
    }
}

pub(super) struct Host {
    pub(super) dev: Dev,
    pub(super) pairing: Arc<DevicePairing>,
    pub(super) sync: Arc<DeviceSync>,
    tap: Arc<Tap>,
    pub(super) device: PeerDevice,
    pub(super) client: PeerClient,
    _stop: tokio::sync::watch::Sender<bool>,
}

fn deps(d: &Dev, relay: &str) -> PairDeps {
    let relay = relay.to_string();
    let name = format!("Device {}", d.id);
    PairDeps {
        own_id: d.id.clone(),
        links: d.links.clone(),
        account: d.acct.clone(),
        name: Arc::new(move || name.clone()),
        version: "0.3.0".into(),
        relay: Arc::new(move || relay.clone()),
        relays: Arc::new(Ntfy::new().unwrap()),
        clock: Arc::new(crate::sync::store::system_ms),
    }
}

/// A device with its control port on loopback answering pairing and sync,
/// signing with `token` (devices with one token are one account).
pub(super) async fn host(d: Dev, token: &str, relay: &str) -> Host {
    let pairing = DevicePairing::new(deps(&d, relay));
    let tap = Arc::new(Tap { inner: pairing.clone(), wire: Mutex::default() });
    let wire = Arc::new(Wire::new(d.links.clone(), Arc::new(Ntfy::new().unwrap()) as Arc<dyn RelayFactory>, Arc::new(crate::sync::store::system_ms)));
    let sync = Arc::new(DeviceSync::new(d.peer.clone(), wire, Timing::default()));
    let s = sync.clone();
    pairing.set_on_paired(Arc::new(move |_| s.kick()));
    let slot = ExtraSlot::default();
    let routes: Vec<Arc<dyn crate::remote::peer::server::ExtraRoutes>> = vec![
        Arc::new(PairRoutes::new(d.id.clone(), Arc::new(Fixed(key(token))), tap.clone())),
        Arc::new(SyncRoutes::new(d.id.clone(), Arc::new(Fixed(key(token))), sync.clone())),
    ];
    *slot.write() = Some(Arc::new(RouteList(routes)));
    let srv = Arc::new(PeerServer::new(d.id.clone(), Arc::new(NoPlayer), Arc::new(Fixed(key(token)))).with_extra(slot));
    let (listeners, port) = server::bind(&[Ipv4Addr::LOCALHOST], 0, 1).await.unwrap();
    let (stop, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(server::serve(listeners, srv, rx));
    let device = PeerDevice {
        id: d.id.clone(),
        name: format!("Device {}", d.id),
        platform: "iOS".into(),
        bases: vec![url::Url::parse(&format!("http://127.0.0.1:{port}")).unwrap()],
    };
    let client = PeerClient::new(reqwest::Client::new(), d.id.clone(), key(token));
    Host { dev: d, pairing, sync, tap, device, client, _stop: stop }
}

fn only_link(d: &Dev) -> (Link, crate::homeside::links::Paired) {
    let all = d.links.all();
    assert_eq!(all.len(), 1, "one link");
    (d.links.link(&all[0].id).unwrap(), all[0].clone())
}

/// Both ends hold the same device link, each the other's mirror.
fn linked(a: &Dev, b: &Dev) {
    let (la, pa) = only_link(a);
    let (lb, pb) = only_link(b);
    assert_eq!((la.side, lb.side), (Side::Device, Side::Device));
    assert_eq!(la.id, lb.id);
    assert_eq!(la.secret, lb.secret);
    assert_eq!((la.me.as_str(), la.peer.as_str()), (lb.peer.as_str(), lb.me.as_str()));
    assert!(!la.me.is_empty() && la.me != la.peer);
    assert_eq!(pa.device.as_deref(), Some(b.id.as_str()), "the other device's id is filled in");
    assert_eq!(pb.device.as_deref(), Some(a.id.as_str()));
    assert_eq!(pa.name, format!("Device {}", b.id));
    assert_eq!(pb.name, format!("Device {}", a.id));
    // Never a Bridge link.
    assert!(Links::new(a.db.clone(), a.keys.clone()).all().is_empty());
    assert!(Links::new(b.db.clone(), b.keys.clone()).all().is_empty());
}

fn no_secret_in(text: &str, extra: &[&str]) {
    for v in [PLEX_TOKEN, LIDARR_KEY, LASTFM_KEY, ACCT_TOKEN].iter().chain(extra) {
        assert!(!text.contains(v), "{v} travelled: {text}");
    }
}

// ---------------------------------------------------------------- the 6-digit code

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_6_digit_code_pairs_two_devices_on_one_network_and_they_sync_there() {
    let ntfy = FakeNtfy::start().await;
    let phone = host(real_clock("phone", ACCT), ACCT_TOKEN, &ntfy.url).await;
    let pc = host(real_clock("pc", ACCT), ACCT_TOKEN, &ntfy.url).await;
    pc.dev.keys.set(ACCOUNT_TOKEN, &Token::new(PLEX_TOKEN).unwrap()).unwrap();
    pc.dev.store.put(&HIDDEN, "track:1", &hidden("Quiet Hours")).unwrap();
    let shown = phone.pairing.start().await.unwrap();
    assert_eq!(shown.phase, PairPhase::Waiting);
    let code = shown.code.clone().unwrap();
    assert_eq!(code.len(), 6);
    assert!(shown.long_code.as_deref().is_some_and(|c| c.starts_with("PB1-")));
    assert!(shown.qr.is_some());
    let p = pc.pairing.join(&code, Some((pc.client.clone(), vec![phone.device.clone()]))).await.unwrap();
    assert_eq!(p.name, "Device phone");
    linked(&pc.dev, &phone.dev);
    assert_eq!(phone.pairing.state().phase, PairPhase::Paired);
    assert_eq!(phone.pairing.state().with.as_deref(), Some("Device pc"));
    assert!(phone.pairing.state().code.is_none(), "the code stops showing");
    // The new link syncs over the network straight away.
    struct Direct(PeerClient, url::Url);
    #[async_trait]
    impl crate::sync::devices::lan::LanSend for Direct {
        async fn post(&self, device: &str, path: &str, body: &crate::remote::peer::sync::LanBody) -> Result<crate::remote::peer::sync::LanBody, crate::remote::peer::client::PeerError> {
            self.0.sync_exchange(&self.1, device, path, body, Duration::from_secs(5)).await
        }
    }
    pc.sync.set_lan(Arc::new(Direct(pc.client.clone(), phone.device.bases[0].clone())));
    pc.sync.sync_now().await;
    assert!(phone.dev.store.has_entry(&HIDDEN, "track:1").unwrap());
    assert!(ntfy.published().is_empty(), "nothing went through the relay");
    // Nothing secret crossed the network while pairing.
    let wire = phone.tap.wire.lock().clone();
    assert!(wire.len() >= 3, "{wire:?}");
    for m in &wire {
        no_secret_in(m, &[ACCT, "Device pc", "Device phone"]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_6_digit_code_fails_and_too_many_lock_the_code() {
    let phone = host(real_clock("phone", ACCT), ACCT_TOKEN, "http://relay").await;
    let pc = host(real_clock("pc", ACCT), ACCT_TOKEN, "http://relay").await;
    let code = phone.pairing.start().await.unwrap().code.unwrap();
    let wrong = if code == "000000" { "111111" } else { "000000" };
    let lan = || Some((pc.client.clone(), vec![phone.device.clone()]));
    let e = pc.pairing.join(wrong, lan()).await.unwrap_err();
    assert!(e.to_string().contains("doesn't match"), "{e}");
    assert!(pc.dev.links.all().is_empty() && phone.dev.links.all().is_empty());
    for _ in 1..TRIES {
        assert!(pc.pairing.join(wrong, lan()).await.is_err());
    }
    // Out of tries: even the right code no longer works.
    assert!(pc.pairing.join(&code, lan()).await.is_err());
    assert_eq!(phone.pairing.state().phase, PairPhase::Locked);
    assert!(pc.dev.links.all().is_empty() && phone.dev.links.all().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_hello_answer_holds_nothing_from_the_key_and_spends_no_try() {
    let phone = host(real_clock("phone", ACCT), ACCT_TOKEN, "http://relay").await;
    let pc = host(real_clock("pc", ACCT), ACCT_TOKEN, "http://relay").await;
    let code = phone.pairing.start().await.unwrap().code.unwrap();
    // Another device of the account sends hello after hello and never
    // finishes: the answer is the other half of the exchange only, so it
    // can't test a guess, and no try is used up.
    for _ in 0..(TRIES as usize + 3) {
        let (_, msg_a) = crate::sync::keycopy::crypto::sender_start("platter device pair 123456", "rogue", "phone");
        let reply = phone.pairing.hello("rogue", PairHello { msg: crate::sync::keycopy::crypto::hex(&msg_a) }).await.unwrap();
        let v = serde_json::to_value(&reply).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["msg"], "nothing but the exchange's half: {v}");
    }
    assert_eq!(phone.pairing.state().phase, PairPhase::Waiting);
    // The right code still pairs.
    pc.pairing.join(&code, Some((pc.client.clone(), vec![phone.device.clone()]))).await.unwrap();
    linked(&pc.dev, &phone.dev);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_of_another_account_cant_pair_with_the_6_digit_code() {
    let phone = host(real_clock("phone", ACCT), ACCT_TOKEN, "http://relay").await;
    // Another account: signs with another key, says another tag.
    let other = host(real_clock("other", "someone-else"), "another-token", "http://relay").await;
    let code = phone.pairing.start().await.unwrap().code.unwrap();
    let e = other.pairing.join(&code, Some((other.client.clone(), vec![phone.device.clone()]))).await.unwrap_err();
    assert!(e.to_string().contains("No device") || matches!(e, AppError::Auth(_)), "{e}");
    assert!(phone.dev.links.all().is_empty() && other.dev.links.all().is_empty());
    assert_eq!(phone.pairing.state().phase, PairPhase::Waiting, "still showing for the right device");
    // Same key but signed in as another Home user: the sealed check refuses.
    let home_user = host(real_clock("kid", "home-user-2"), ACCT_TOKEN, "http://relay").await;
    let e = home_user.pairing.join(&code, Some((home_user.client.clone(), vec![phone.device.clone()]))).await.unwrap_err();
    assert!(matches!(e, AppError::Auth(_)), "{e}");
    assert!(phone.dev.links.all().is_empty() && home_user.dev.links.all().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_6_digit_code_never_goes_through_the_relay() {
    let ntfy = FakeNtfy::start().await;
    let phone = host(real_clock("phone", ACCT), ACCT_TOKEN, &ntfy.url).await;
    let pc = host(real_clock("pc", ACCT), ACCT_TOKEN, &ntfy.url).await;
    let code = phone.pairing.start().await.unwrap().code.unwrap();
    // Away from home: no devices on this network, or no network at all.
    let e = pc.pairing.join(&code, Some((pc.client.clone(), Vec::new()))).await.unwrap_err();
    assert!(e.to_string().contains("long code"), "{e}");
    let e = pc.pairing.join(&code, None).await.unwrap_err();
    assert!(e.to_string().contains("same network"), "{e}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let topics: Vec<String> = ntfy.published().into_iter().map(|m| m.topic).collect();
    assert!(topics.is_empty(), "nothing published for a 6-digit code: {topics:?}");
    // The only topic listened on comes from the long code, not the digits.
    for r in ntfy.requests() {
        assert!(!r.contains(&code), "{r}");
    }
    assert!(pc.dev.links.all().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_expired_or_cancelled_code_pairs_nothing() {
    let clock = Arc::new(AtomicU64::new(crate::sync::store::system_ms()));
    let d = dev_with("phone", Some(ACCT), Some(clock.clone()));
    let mut deps = deps(&d, "http://relay");
    let c = clock.clone();
    deps.clock = Arc::new(move || c.load(Ordering::SeqCst));
    let pairing = DevicePairing::new(deps);
    pairing.start().await.unwrap();
    clock.fetch_add(CODE_TTL_MS + 1, Ordering::SeqCst);
    assert_eq!(pairing.state().phase, PairPhase::Expired);
    assert!(pairing.state().code.is_none());
    let e = pairing.hello("pc", PairHello { msg: "00".into() }).await.unwrap_err();
    assert!(matches!(e, AppError::NotFound(_)));
    pairing.start().await.unwrap();
    pairing.cancel();
    assert_eq!(pairing.state().phase, PairPhase::Idle);
    assert!(!pairing.waiting().await);
    // Signed out: no code at all.
    *d.acct.0.lock() = None;
    assert!(matches!(pairing.start().await.unwrap_err(), AppError::Auth(_)));
}

#[tokio::test]
async fn a_finish_from_another_device_than_the_hello_is_refused() {
    let d = dev("phone");
    let pairing = DevicePairing::new(deps(&d, "http://relay"));
    let code = pairing.start().await.unwrap().code.unwrap();
    let (_, msg_a) = crate::sync::keycopy::crypto::sender_start(&format!("platter device pair {code}"), "pc", "phone");
    pairing.hello("pc", PairHello { msg: crate::sync::keycopy::crypto::hex(&msg_a) }).await.unwrap();
    let e = pairing.finish("intruder", PairFinish { sealed: "x".into() }).await.unwrap_err();
    assert!(e.to_string().contains("Start again"), "{e}");
    assert!(d.links.all().is_empty());
}

#[tokio::test]
async fn things_that_arent_codes_are_turned_down() {
    let d = dev("pc");
    let pairing = DevicePairing::new(deps(&d, "http://relay"));
    for bad in ["", "12345", "1234567", "abcdef", "PB1-nope", "platter://copy-keys/123456"] {
        let e = pairing.join(bad, None).await.unwrap_err();
        assert!(matches!(e, AppError::Invalid(_)), "{bad}: {e}");
    }
    assert!(d.links.all().is_empty());
}

// ---------------------------------------------------------------- the long code

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_long_code_pairs_two_devices_through_the_relay_sealed() {
    let ntfy = FakeNtfy::start().await;
    let phone = real_clock("phone", ACCT);
    let pc = real_clock("pc", ACCT);
    let host_side = DevicePairing::new(deps(&phone, &ntfy.url));
    let joiner = DevicePairing::new(deps(&pc, &ntfy.url));
    let long = host_side.start().await.unwrap().long_code.unwrap();
    until("the code's listener on the relay", || ntfy.requests().iter().any(|r| r.starts_with("GET") && r.contains("/json"))).await;
    // Scanned from the QR code: the link form works too.
    let join = {
        let joiner = joiner.clone();
        let long = long.clone();
        tokio::spawn(async move { joiner.join(&format!("platter://pair-device/{long}"), None).await })
    };
    // The phone asks first, naming the device, and links nothing yet.
    until("the phone to ask", || host_side.state().phase == PairPhase::Confirm).await;
    assert_eq!(host_side.state().with.as_deref(), Some("Device pc"));
    assert!(phone.links.all().is_empty() && pc.links.all().is_empty());
    host_side.confirm(true).unwrap();
    let p = join.await.unwrap().unwrap();
    assert_eq!(p.name, "Device phone");
    until("the phone to record the link", || !phone.links.all().is_empty()).await;
    linked(&pc, &phone);
    assert_eq!(only_link(&pc).0.relay, ntfy.url, "the link uses the relay the code named");
    assert_eq!(host_side.state().phase, PairPhase::Paired);
    let published = ntfy.published();
    assert!(published.len() >= 2, "a join and a welcome");
    for m in &published {
        assert!(m.body.starts_with("1."), "sealed");
        no_secret_in(&m.body, &[ACCT, "Device pc", "Device phone"]);
        no_secret_in(&m.topic, &[ACCT, "Device"]);
    }
    // Used: the same code doesn't pair a third device.
    let third = real_clock("tablet", ACCT);
    let e = DevicePairing::new(deps(&third, &ntfy.url)).join(&long, None).await.unwrap_err();
    assert!(matches!(e, AppError::Network(_)), "{e}");
    assert!(third.links.all().is_empty());
    assert_eq!(phone.links.all().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn saying_no_to_a_device_that_joined_with_the_long_code_pairs_nothing() {
    let ntfy = FakeNtfy::start().await;
    let phone = real_clock("phone", ACCT);
    let pc = real_clock("pc", ACCT);
    let host_side = DevicePairing::new(deps(&phone, &ntfy.url));
    let joiner = DevicePairing::new(deps(&pc, &ntfy.url));
    let long = host_side.start().await.unwrap().long_code.unwrap();
    until("the code's listener on the relay", || ntfy.requests().iter().any(|r| r.starts_with("GET") && r.contains("/json"))).await;
    let join = tokio::spawn(async move { joiner.join(&long, None).await });
    until("the phone to ask", || host_side.state().phase == PairPhase::Confirm).await;
    assert!(host_side.state().code.is_none(), "the codes stop while it asks");
    host_side.confirm(false).unwrap();
    assert_eq!(host_side.state().phase, PairPhase::Declined);
    let e = join.await.unwrap().unwrap_err();
    assert!(matches!(e, AppError::Network(_)), "{e}");
    assert!(phone.links.all().is_empty() && pc.links.all().is_empty());
    assert_eq!(ntfy.published().len(), 1, "the join only, no welcome");
    assert!(host_side.confirm(true).is_err(), "nothing is asking now");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_welcome_the_relay_turns_away_leaves_no_half_link() {
    let ntfy = FakeNtfy::start().await;
    let phone = real_clock("phone", ACCT);
    let pc = real_clock("pc", ACCT);
    let host_side = DevicePairing::new(deps(&phone, &ntfy.url));
    let joiner = DevicePairing::new(deps(&pc, &ntfy.url));
    let long = host_side.start().await.unwrap().long_code.unwrap();
    until("the code's listener on the relay", || ntfy.requests().iter().any(|r| r.starts_with("GET") && r.contains("/json"))).await;
    let join = tokio::spawn(async move { joiner.join(&long, None).await });
    until("the phone to ask", || host_side.state().phase == PairPhase::Confirm).await;
    // The welcome can't be sent: the link saved for it is taken back.
    ntfy.set_limited(true);
    host_side.confirm(true).unwrap();
    until("the phone to give up", || host_side.state().phase == PairPhase::Expired).await;
    assert!(phone.links.all().is_empty(), "no link without a welcome sent");
    assert!(join.await.unwrap().is_err());
    assert!(pc.links.all().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_that_cant_be_saved_sends_no_welcome() {
    let ntfy = FakeNtfy::start().await;
    let phone = real_clock("phone", ACCT);
    let pc = real_clock("pc", ACCT);
    let host_side = DevicePairing::new(deps(&phone, &ntfy.url));
    let joiner = DevicePairing::new(deps(&pc, &ntfy.url));
    let long = host_side.start().await.unwrap().long_code.unwrap();
    until("the code's listener on the relay", || ntfy.requests().iter().any(|r| r.starts_with("GET") && r.contains("/json"))).await;
    let join = tokio::spawn(async move { joiner.join(&long, None).await });
    until("the phone to ask", || host_side.state().phase == PairPhase::Confirm).await;
    // The phone can't save the link (its database refuses the write).
    phone.db.write(|c| Ok(c.execute_batch("DROP TABLE kv")?)).unwrap();
    host_side.confirm(true).unwrap();
    until("the phone to give up", || host_side.state().phase == PairPhase::Expired).await;
    assert_eq!(ntfy.published().len(), 1, "the join only: the other device was never told it's paired");
    assert!(join.await.unwrap().is_err());
    assert!(pc.links.all().is_empty());
}

#[tokio::test]
async fn a_relay_that_cant_be_reached_hides_the_long_code_and_says_so() {
    let d = dev("phone");
    // Nothing listens here.
    let pairing = DevicePairing::new(deps(&d, "http://127.0.0.1:9"));
    pairing.start().await.unwrap();
    until("the note", || pairing.state().note.is_some()).await;
    let st = pairing.state();
    assert_eq!(st.note.as_deref(), Some(crate::sync::devices::pair::RELAY_DOWN));
    assert_eq!(st.phase, PairPhase::Waiting, "the 6-digit code still works");
    assert!(st.code.is_some() && st.long_code.is_none() && st.qr.is_none());
    pairing.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_of_another_account_cant_pair_with_the_long_code() {
    let ntfy = FakeNtfy::start().await;
    let phone = real_clock("phone", ACCT);
    let other = real_clock("other", "someone-else");
    let host_side = DevicePairing::new(deps(&phone, &ntfy.url));
    let long = host_side.start().await.unwrap().long_code.unwrap();
    until("the code's listener on the relay", || ntfy.requests().iter().any(|r| r.starts_with("GET") && r.contains("/json"))).await;
    let e = DevicePairing::new(deps(&other, &ntfy.url)).join(&long, None).await.unwrap_err();
    assert!(matches!(e, AppError::Network(_)), "no answer for another account: {e}");
    assert!(phone.links.all().is_empty() && other.links.all().is_empty());
    assert_eq!(host_side.state().phase, PairPhase::Waiting);
}

#[test]
fn the_showing_end_answers_only_a_join_of_its_own_account() {
    let code = PairCode::new(None, 1_000);
    let now = 1_000 * 1000 + 5;
    let phone = || DeviceMe { device: "phone".into(), acct: "mine".into() };
    for (acct, dev) in [("theirs", "pc"), ("", "pc"), ("mine", ""), ("mine", "phone")] {
        let mut offer = Offer::for_device(code.clone(), "Phone", phone());
        let (_, (_, join)) = Joining::start_device(code.clone(), "PC", "1", DeviceMe { device: dev.into(), acct: acct.into() }, now).unwrap();
        assert!(offer.answer(&join, now).is_none(), "{acct:?} {dev:?}");
    }
    let mut offer = Offer::for_device(code.clone(), "Phone", phone());
    let (j, (_, join)) = Joining::start_device(code, "PC", "1", DeviceMe { device: "pc".into(), acct: "mine".into() }, now).unwrap();
    let ((_, welcome), joined) = offer.answer(&join, now).unwrap();
    let d = j.finish(&welcome, now).unwrap();
    assert_eq!((joined.device.as_str(), d.device.as_str()), ("pc", "phone"));
    assert_eq!((joined.link.side, d.link.side), (Side::Device, Side::Device));
    assert_eq!(joined.link.secret, d.link.secret);
}

#[test]
fn bridge_codes_and_device_codes_dont_cross() {
    let code = PairCode::new(None, 1_000);
    let now = 1_000 * 1000 + 5;
    let me = || DeviceMe { device: "pc".into(), acct: "a".into() };
    // A Bridge joining a device's code: never answered.
    let mut device_offer = Offer::for_device(code.clone(), "Phone", DeviceMe { device: "phone".into(), acct: "a".into() });
    let (_, (topic, join)) = Joining::start(code.clone(), "Shed", "1", now).unwrap();
    assert_ne!(topic, device_offer.listen_topic());
    assert!(device_offer.answer(&join, now).is_none());
    // A device joining a Bridge pairing code: never answered either.
    let mut bridge_offer = Offer::new(code.clone(), "Studio");
    let (_, (topic, join)) = Joining::start_device(code, "PC", "1", me(), now).unwrap();
    assert_ne!(topic, bridge_offer.listen_topic());
    assert!(bridge_offer.answer(&join, now).is_none());
}
