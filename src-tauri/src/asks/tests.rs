//! Requests between in-process Platters through a fake ntfy server on this
//! computer (never the real ntfy.sh). Invented people and albums only.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};

use super::wire::{self, AskItem, AskState, Pair, Up};
use super::*;
use crate::db::DbHandle;
use crate::lidarr::request::{GetNeeds, GetOutcome, RequestInput, RequestRecord};
use crate::lidarr::status::RequestState;
use crate::plex::token::Token;
use crate::remote::peer::auth::now_ms;
use crate::social::listen::fake_ntfy::FakeNtfy;
use crate::social::listen::relay::Ntfy;

const SERVER: &str = "demo-machine-0001";

fn tok(s: &str) -> Token {
    Token::new(s).unwrap()
}

fn sam() -> Person {
    Person { user_id: "101".into(), name: "Sam".into(), token: tok("sam-server-token") }
}

fn kim() -> Person {
    Person { user_id: "102".into(), name: "Kim".into(), token: tok("kim-server-token") }
}

fn item(title: &str) -> AskItem {
    AskItem {
        release_group_id: None,
        title: title.into(),
        artist: "Cold Coast".into(),
        artist_mbid: None,
        album: None,
        song: false,
        isrc: None,
        thumb: None,
    }
}

fn rg(title: &str) -> String {
    format!("rg-{}", title.to_lowercase().replace(' ', "-"))
}

fn record(release_group_id: &str, state: RequestState, progress: Option<u8>) -> RequestRecord {
    let mut r: RequestRecord = serde_json::from_value(serde_json::json!({
        "id": format!("req-{release_group_id}"),
        "releaseGroupId": release_group_id,
        "title": "t",
        "artist": "Cold Coast",
        "state": "searching",
        "createdAt": 1,
        "updatedAt": 1,
    }))
    .unwrap();
    r.state = state;
    r.progress = progress;
    r
}

struct Host {
    role: Mutex<Role>,
    people: Mutex<Vec<Person>>,
    usable: AtomicBool,
    settings: Mutex<AskSettings>,
    no_pick: Mutex<HashSet<String>>,
    protected: Mutex<HashSet<String>>,
    /// Lidarr's requests, by release group (shared by the owner's devices).
    records: Arc<Mutex<HashMap<String, RequestRecord>>>,
    /// Every Get sent, by release group (shared by the owner's devices).
    gets: Arc<Mutex<Vec<String>>>,
    first_time: AtomicBool,
    /// Lidarr can't be reached to check an artist.
    lidarr_down: AtomicBool,
    notices: Mutex<Vec<String>>,
    refreshed: AtomicU32,
}

impl Host {
    fn new(role: Role) -> Self {
        Self {
            role: Mutex::new(role),
            people: Mutex::new(vec![sam(), kim()]),
            usable: AtomicBool::new(true),
            settings: Mutex::new(AskSettings { on: true, rules: Vec::new(), weekly_limit: 20, group_notices: false }),
            no_pick: Mutex::default(),
            protected: Mutex::default(),
            records: Arc::default(),
            gets: Arc::default(),
            first_time: AtomicBool::new(false),
            lidarr_down: AtomicBool::new(false),
            notices: Mutex::default(),
            refreshed: AtomicU32::new(0),
        }
    }

    fn rule(&self, user: &str, rule: Rule) {
        let mut s = self.settings.lock();
        s.rules.retain(|(u, _)| u != user);
        s.rules.push((user.to_string(), rule));
    }

    fn notices(&self) -> Vec<String> {
        self.notices.lock().clone()
    }

    fn gets(&self) -> Vec<String> {
        self.gets.lock().clone()
    }
}

#[async_trait]
impl AsksHost for Host {
    async fn role(&self) -> Role {
        self.role.lock().clone()
    }
    async fn people(&self) -> AppResult<Vec<Person>> {
        Ok(self.people.lock().clone())
    }
    fn lidarr_usable(&self) -> bool {
        self.usable.load(Ordering::SeqCst)
    }
    async fn pick(&self, item: &AskItem) -> Result<String, String> {
        if self.no_pick.lock().contains(&item.title) {
            return Err("This isn't on MusicBrainz as an official release yet.".into());
        }
        Ok(rg(&item.title))
    }
    async fn protected(&self, input: &RequestInput) -> Option<bool> {
        (!self.lidarr_down.load(Ordering::SeqCst)).then(|| self.protected.lock().contains(&input.release_group_id))
    }
    async fn get(&self, input: RequestInput) -> AppResult<GetOutcome> {
        if self.first_time.load(Ordering::SeqCst) {
            return Ok(GetOutcome { record: None, needs: Some(GetNeeds { kind: "firstTime".into(), reason: "Type the album's name".into() }) });
        }
        self.gets.lock().push(input.release_group_id.clone());
        let r = self.records.lock().entry(input.release_group_id.clone()).or_insert_with(|| record(&input.release_group_id, RequestState::Searching, None)).clone();
        Ok(GetOutcome { record: Some(r), needs: None })
    }
    fn record(&self, release_group_id: &str) -> Option<RequestRecord> {
        self.records.lock().get(release_group_id).cloned()
    }
    fn settings(&self) -> AskSettings {
        self.settings.lock().clone()
    }
    fn notify(&self, title: &str, _body: &str) {
        self.notices.lock().push(title.to_string());
    }
    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
    fn refresh_library(&self) {
        self.refreshed.fetch_add(1, Ordering::SeqCst);
    }
    fn now_ms(&self) -> u64 {
        now_ms()
    }
}

struct Node {
    asks: Asks,
    host: Arc<Host>,
    _dir: tempfile::TempDir,
}

fn node(ntfy: &FakeNtfy, host: Host) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    let host = Arc::new(host);
    let asks = Asks::new(host.clone(), db, Arc::new(Ntfy::new().unwrap()), Arc::new(RwLock::new(String::new())), ntfy.url.clone());
    Node { asks, host, _dir: dir }
}

fn owner(ntfy: &FakeNtfy) -> Node {
    node(ntfy, Host::new(Role::Owner { machine_id: SERVER.into() }))
}

/// A second device of the same owner: same Lidarr.
fn owner_too(ntfy: &FakeNtfy, first: &Node) -> Node {
    let mut h = Host::new(Role::Owner { machine_id: SERVER.into() });
    h.records = first.host.records.clone();
    h.gets = first.host.gets.clone();
    node(ntfy, h)
}

fn guest(ntfy: &FakeNtfy, p: &Person) -> Node {
    node(ntfy, Host::new(Role::Guest { machine_id: SERVER.into(), token: p.token.clone() }))
}

fn start(n: &Node) {
    tokio::spawn(n.asks.clone().run());
}

async fn until(what: &str, f: impl Fn() -> bool) {
    for _ in 0..250 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for: {what}");
}

async fn until_async<F, Fut>(what: &str, f: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..250 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for: {what}");
}

async fn can_request(g: &Node) {
    until_async("Request to show", || async { g.asks.view().await.can_request }).await;
}

fn mine_state(g: &Node, id: &str) -> Option<AskState> {
    g.asks.inner.mine.lock().iter().find(|a| a.id == id).map(|a| a.state)
}

fn incoming_state(o: &Node, id: &str) -> Option<AskState> {
    o.asks.inner.incoming.lock().iter().find(|a| a.id == id).map(|a| a.state)
}

/// Both running, and the person allowed to ask.
async fn pair(ntfy: &FakeNtfy, p: &Person) -> (Node, Node) {
    let o = owner(ntfy);
    let g = guest(ntfy, p);
    start(&o);
    start(&g);
    can_request(&g).await;
    (o, g)
}

// ---- the whole way round -------------------------------------------------------------

#[tokio::test]
async fn a_request_reaches_the_owner_and_a_yes_gets_it_and_tells_the_person() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    let a = g.asks.request(AskItem { thumb: Some("/ext/https://example.com/cover.jpg".into()), ..item("First Frost") }).await.unwrap();
    assert_eq!(a.state, AskState::Sending);
    until("the owner to have it", || incoming_state(&o, &a.id) == Some(AskState::Requested) && !o.host.notices().is_empty()).await;
    until("the person to see it's with the owner", || mine_state(&g, &a.id) == Some(AskState::Requested)).await;
    assert_eq!(o.host.notices(), vec!["Sam requested First Frost".to_string()]);
    let inc = o.asks.incoming();
    assert_eq!((inc[0].person_name.as_str(), inc[0].release_group_id.as_deref()), ("Sam", Some(rg("First Frost").as_str())));
    // The cover they had isn't sent; the owner's side never fetches an address someone sent.
    assert_eq!(g.asks.inner.mine.lock()[0].item.thumb.as_deref(), Some("/ext/https://example.com/cover.jpg"));
    assert!(inc[0].item.thumb.is_none());
    // Nothing went to Lidarr before the yes.
    assert!(o.host.gets().is_empty());

    let done = o.asks.answer(&a.id, true).await.unwrap();
    assert_eq!(done.incoming.state, AskState::Getting);
    assert_eq!(o.host.gets(), vec![rg("First Frost")]);
    until("the person to see it coming", || mine_state(&g, &a.id) == Some(AskState::Getting)).await;

    o.host.records.lock().insert(rg("First Frost"), record(&rg("First Frost"), RequestState::Downloading, Some(40)));
    o.asks.follow().await;
    until("progress", || g.asks.inner.mine.lock()[0].progress == Some(40)).await;
    o.host.records.lock().insert(rg("First Frost"), record(&rg("First Frost"), RequestState::Imported, None));
    o.asks.follow().await;
    until("it to be ready", || mine_state(&g, &a.id) == Some(AskState::Ready)).await;
    assert_eq!(g.host.notices(), vec!["First Frost is here.".to_string()]);
    assert_eq!(g.host.refreshed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_request_made_while_the_owner_is_away_goes_when_they_are_back() {
    let ntfy = FakeNtfy::start().await;
    let g = guest(&ntfy, &sam());
    start(&g);
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Never told either way: Request doesn't show yet, but a request still goes.
    assert!(!g.asks.view().await.can_request);
    let a = g.asks.request(item("First Frost")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(mine_state(&g, &a.id), Some(AskState::Sending));

    let o = owner(&ntfy);
    start(&o);
    until("the owner to catch up", || incoming_state(&o, &a.id) == Some(AskState::Requested)).await;
    until("the person to hear", || mine_state(&g, &a.id) == Some(AskState::Requested)).await;
    can_request(&g).await;
    assert!(g.asks.inner.mine.lock()[0].seen);
}

#[tokio::test]
async fn sending_the_same_request_twice_is_one_request() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    let a = g.asks.request(item("First Frost")).await.unwrap();
    let b = g.asks.request(AskItem { artist: "COLD COAST".into(), ..item("first frost") }).await.unwrap();
    assert_eq!(a.id, b.id);
    until("the owner to have it", || incoming_state(&o, &a.id).is_some()).await;
    // The relay delivering it again changes nothing.
    let p = Pair::new(&sam().token, SERVER);
    let again = wire::seal_up(&p, &Up::Ask { id: a.id.clone(), item: a.item.clone(), at: a.created_at }).unwrap();
    ntfy.inject(&p.up.topic, &again);
    ntfy.inject(&p.up.topic, &again);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(o.asks.incoming().len(), 1);
    assert_eq!(o.host.notices().len(), 1);
}

// ---- who may ask ----------------------------------------------------------------------

#[tokio::test]
async fn a_message_sealed_by_anyone_else_is_dropped() {
    let ntfy = FakeNtfy::start().await;
    let (o, _g) = pair(&ntfy, &sam()).await;
    let sams = Pair::new(&sam().token, SERVER);
    // Someone who isn't shared with, writing on Sam's channel.
    let stranger = Pair::new(&tok("not-a-share"), SERVER);
    let forged = wire::seal_up(&stranger, &Up::Ask { id: "x1".into(), item: item("Forged"), at: now_ms() }).unwrap();
    ntfy.inject(&sams.up.topic, &forged);
    // Sam's key for another server.
    let elsewhere = Pair::new(&sam().token, "another-machine");
    let wrong = wire::seal_up(&elsewhere, &Up::Ask { id: "x2".into(), item: item("Elsewhere"), at: now_ms() }).unwrap();
    ntfy.inject(&sams.up.topic, &wrong);
    // Plain text.
    ntfy.inject(&sams.up.topic, r#"{"t":"ask","id":"x3"}"#);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(o.asks.incoming().is_empty());
    assert!(o.host.notices().is_empty());
}

#[tokio::test]
async fn someone_the_server_is_no_longer_shared_with_isnt_heard() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    o.host.people.lock().retain(|p| p.user_id != sam().user_id);
    o.asks.poke();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let a = g.asks.request(item("First Frost")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(o.asks.incoming().is_empty());
    assert_eq!(mine_state(&g, &a.id), Some(AskState::Sending));
}

#[tokio::test]
async fn cant_ask_hides_request_and_the_owner_refuses_anyway() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    o.host.rule(&sam().user_id, Rule::Off);
    o.asks.poke();
    until_async("Request to go", || async { !g.asks.view().await.can_request }).await;
    let e = g.asks.request(item("First Frost")).await.unwrap_err();
    assert!(e.to_string().contains(OFF_NOTE), "{e}");

    // An old Platter that asks anyway is refused, and Lidarr never hears.
    let p = Pair::new(&sam().token, SERVER);
    ntfy.inject(&p.up.topic, &wire::seal_up(&p, &Up::Ask { id: "old1".into(), item: item("First Frost"), at: now_ms() }).unwrap());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(o.asks.incoming().is_empty(), "a refusal isn't kept");
    assert!(o.host.gets().is_empty());
    assert!(o.host.notices().is_empty());
    // Kim still may.
    assert_eq!(o.host.settings().rule(&kim().user_id), Rule::Ask);
}

#[tokio::test]
async fn turning_requests_off_for_everyone_hides_request() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    o.host.settings.lock().on = false;
    o.asks.poke();
    until_async("Request to go", || async { !g.asks.view().await.can_request }).await;
}

#[tokio::test]
async fn an_owner_device_without_lidarr_stays_quiet_and_only_lists_requests() {
    let ntfy = FakeNtfy::start().await;
    let o = owner(&ntfy);
    o.host.usable.store(false, Ordering::SeqCst);
    let g = guest(&ntfy, &sam());
    start(&o);
    start(&g);
    tokio::time::sleep(Duration::from_millis(400)).await;
    // It doesn't answer for the owner, so Request doesn't show yet.
    assert!(!g.asks.view().await.can_request);
    let p = Pair::new(&sam().token, SERVER);
    ntfy.inject(&p.up.topic, &wire::seal_up(&p, &Up::Ask { id: "n1".into(), item: item("First Frost"), at: now_ms() }).unwrap());
    until("it to be listed", || incoming_state(&o, "n1") == Some(AskState::Requested)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(ntfy.published().iter().all(|m| m.topic != p.down.topic), "nothing said for the owner");
    assert!(o.host.notices().is_empty());
}

#[tokio::test]
async fn a_device_catching_up_never_answers_or_gets_what_another_device_handled() {
    let ntfy = FakeNtfy::start().await;
    let desk = owner(&ntfy);
    desk.host.rule(&sam().user_id, Rule::Straight);
    desk.host.settings.lock().weekly_limit = 1;
    let g = guest(&ntfy, &sam());
    start(&desk);
    start(&g);
    until_async("straight away", || async { g.asks.view().await.straight_away }).await;
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("it to go", || mine_state(&g, &a.id) == Some(AskState::Getting)).await;
    let b = g.asks.request(item("Saltmarsh")).await.unwrap();
    until("the desk to have it", || incoming_state(&desk, &b.id).is_some()).await;
    desk.asks.answer(&b.id, false).await.unwrap();
    until("the no", || mine_state(&g, &b.id) == Some(AskState::Declined)).await;
    let published = ntfy.published().len();

    // The laptop comes on later; the relay replays all of it, as if from long ago.
    let p = Pair::new(&sam().token, SERVER);
    let old = now_ms() - 3_600_000;
    let laptop = owner_too(&ntfy, &desk);
    laptop.host.rule(&sam().user_id, Rule::Straight);
    // Everything sent before it started listening counts as catch-up (no margin here).
    laptop.asks.inner.stale_ms.store(0, Ordering::SeqCst);
    ntfy.inject(&p.up.topic, &wire::seal_up(&p, &Up::Ask { id: "late".into(), item: item("Reedbeds"), at: old }).unwrap());
    start(&laptop);
    until("the laptop to catch up", || incoming_state(&laptop, &b.id) == Some(AskState::Declined) && incoming_state(&laptop, &a.id).is_some()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(desk.host.gets().len(), 1, "no second Get");
    assert_eq!(incoming_state(&laptop, "late"), Some(AskState::Requested), "an old one is listed, not got");
    assert!(laptop.host.notices().is_empty());
    assert_eq!(mine_state(&g, &b.id), Some(AskState::Declined), "the no stands");
    // Only who may ask (on start), never an answer about a request.
    let said: Vec<_> = ntfy.published()[published..].iter().filter(|m| m.topic == p.down.topic).filter_map(|m| wire::open_down(&p, &m.body).ok()).collect();
    assert!(said.iter().all(|m| matches!(m, wire::Down::Policy { .. })), "{said:?}");
}

#[tokio::test]
async fn a_request_caught_up_on_is_taken_when_its_sent_again_and_nobody_answered() {
    let ntfy = FakeNtfy::start().await;
    let o = owner(&ntfy);
    o.asks.inner.stale_ms.store(0, Ordering::SeqCst);
    let p = Pair::new(&sam().token, SERVER);
    let old = now_ms() - 3_600_000;
    // Sent while this (the only) owner device was off; the relay replays them.
    ntfy.inject(&p.up.topic, &wire::seal_up(&p, &Up::Ask { id: "waits".into(), item: item("Reedbeds"), at: old }).unwrap());
    ntfy.inject(&p.up.topic, &wire::seal_up(&p, &Up::Ask { id: "goes".into(), item: item("Saltmarsh"), at: old }).unwrap());
    start(&o);
    until("both listed", || incoming_state(&o, "waits").is_some() && incoming_state(&o, "goes").is_some()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    // The ids of requests this device has told Sam about.
    let statuses = || -> Vec<String> {
        let said = ntfy.published().into_iter().filter(|m| m.topic == p.down.topic).filter_map(|m| wire::open_down(&p, &m.body).ok());
        said.filter_map(|m| match m {
            wire::Down::Status { id, .. } => Some(id),
            _ => None,
        })
        .collect()
    };
    assert!(statuses().is_empty() && o.host.notices().is_empty(), "caught up on: only listed");
    // The person's Platter hasn't heard back, so it sends them again: nobody
    // else took them, so this device does. Sam may ask first for one...
    ntfy.inject(&p.up.topic, &wire::seal_up(&p, &Up::Ask { id: "waits".into(), item: item("Reedbeds"), at: now_ms() }).unwrap());
    until("the owner told", || !o.host.notices().is_empty()).await;
    until("sam told", || statuses().contains(&"waits".to_string())).await;
    assert_eq!(incoming_state(&o, "waits"), Some(AskState::Requested));
    // ...and goes straight through for the other.
    o.host.rule(&sam().user_id, Rule::Straight);
    ntfy.inject(&p.up.topic, &wire::seal_up(&p, &Up::Ask { id: "goes".into(), item: item("Saltmarsh"), at: now_ms() }).unwrap());
    until("it to go", || incoming_state(&o, "goes") == Some(AskState::Getting)).await;
    assert_eq!(o.host.gets().len(), 1);
    // Sent once more, it's only told where it stands.
    ntfy.inject(&p.up.topic, &wire::seal_up(&p, &Up::Ask { id: "goes".into(), item: item("Saltmarsh"), at: now_ms() }).unwrap());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(o.host.gets().len(), 1, "no second Get");
}

#[tokio::test]
async fn a_refused_request_isnt_kept_and_people_are_limited_per_hour() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    o.host.no_pick.lock().insert("Demo Tape".into());
    let a = g.asks.request(item("Demo Tape")).await.unwrap();
    until("the refusal", || mine_state(&g, &a.id) == Some(AskState::Refused)).await;
    assert!(o.asks.incoming().is_empty());
    // A flood of hellos gets answered up to the hour's limit.
    let p = Pair::new(&sam().token, SERVER);
    for _ in 0..(HEARD_PER_HOUR + 20) {
        ntfy.inject(&p.up.topic, &wire::seal_up(&p, &Up::Hello { at: now_ms() }).unwrap());
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let answers = ntfy.published().iter().filter(|m| m.topic == p.down.topic).count();
    assert!(answers <= HEARD_PER_HOUR + 2, "{answers}");
}

#[tokio::test]
async fn an_answer_missed_while_away_arrives_when_the_person_checks() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("the owner to have it", || incoming_state(&o, &a.id).is_some()).await;
    until("seen", || mine_state(&g, &a.id) == Some(AskState::Requested)).await;
    // The person's Platter missed the yes (as if offline past the relay's memory).
    {
        let mut l = o.asks.inner.incoming.lock();
        l[0].state = AskState::Getting;
    }
    g.asks.resend(SERVER, true).await;
    until("the answer", || mine_state(&g, &a.id) == Some(AskState::Getting)).await;
}

#[tokio::test]
async fn a_status_from_someone_else_doesnt_change_the_owners_list() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("the owner to have it", || incoming_state(&o, &a.id).is_some()).await;
    // Kim knows her own key, and tries to answer Sam's request on her own channel.
    let k = Pair::new(&kim().token, SERVER);
    ntfy.inject(&k.down.topic, &wire::seal_down(&k, &wire::Down::Status { id: a.id.clone(), state: AskState::Ready, progress: None, note: None, at: now_ms() }).unwrap());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(incoming_state(&o, &a.id), Some(AskState::Requested));
}

#[tokio::test]
async fn a_yes_waits_when_lidarr_cant_say_whether_the_artist_is_kept_by_hand() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    o.host.lidarr_down.store(true, Ordering::SeqCst);
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("the owner to have it", || incoming_state(&o, &a.id).is_some()).await;
    assert!(o.asks.answer(&a.id, true).await.is_err());
    assert_eq!(incoming_state(&o, &a.id), Some(AskState::Requested));
    assert!(o.host.gets().is_empty());
}

#[tokio::test]
async fn the_owner_never_sees_request_and_answers_are_owner_only() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    assert!(!o.asks.view().await.can_request);
    assert!(o.asks.request(item("First Frost")).await.is_err());
    let a = g.asks.request(item("First Frost")).await.unwrap();
    assert!(g.asks.answer(&a.id, true).await.is_err());
}

// ---- the owner's checks ---------------------------------------------------------------

#[tokio::test]
async fn a_protected_artist_is_refused_on_yes_and_nothing_is_sent() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    o.host.protected.lock().insert(rg("First Frost"));
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("the owner to have it", || incoming_state(&o, &a.id).is_some()).await;
    let done = o.asks.answer(&a.id, true).await.unwrap();
    assert_eq!(done.incoming.state, AskState::Refused);
    assert!(o.host.gets().is_empty());
    until("the person to hear why", || g.asks.inner.mine.lock()[0].note.as_deref() == Some(PROTECTED_NOTE)).await;
}

#[tokio::test]
async fn something_that_isnt_on_musicbrainz_is_refused_with_why() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    o.host.no_pick.lock().insert("Demo Tape".into());
    let a = g.asks.request(item("Demo Tape")).await.unwrap();
    until("the refusal", || mine_state(&g, &a.id) == Some(AskState::Refused)).await;
    assert!(g.asks.inner.mine.lock()[0].note.as_deref().unwrap().contains("MusicBrainz"));
    assert!(o.host.notices().is_empty());
}

#[tokio::test]
async fn something_already_coming_joins_it_without_a_yes() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    o.host.records.lock().insert(rg("First Frost"), record(&rg("First Frost"), RequestState::Downloading, Some(60)));
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("it to join", || mine_state(&g, &a.id) == Some(AskState::Getting)).await;
    assert_eq!(g.asks.inner.mine.lock()[0].progress, Some(60));
    assert!(o.asks.incoming()[0].by_itself);
    assert!(o.host.gets().is_empty());
    assert!(o.host.notices().is_empty());
}

#[tokio::test]
async fn two_people_asking_for_one_album_get_it_once() {
    let ntfy = FakeNtfy::start().await;
    let o = owner(&ntfy);
    let s = guest(&ntfy, &sam());
    let k = guest(&ntfy, &kim());
    start(&o);
    start(&s);
    start(&k);
    can_request(&s).await;
    can_request(&k).await;
    let a = s.asks.request(item("First Frost")).await.unwrap();
    until("Sam's request", || incoming_state(&o, &a.id).is_some()).await;
    o.asks.answer(&a.id, true).await.unwrap();
    let b = k.asks.request(item("First Frost")).await.unwrap();
    until("Kim's to join", || mine_state(&k, &b.id) == Some(AskState::Getting)).await;
    assert_eq!(o.host.gets().len(), 1);
    o.host.records.lock().insert(rg("First Frost"), record(&rg("First Frost"), RequestState::Imported, None));
    o.asks.follow().await;
    until("both to hear", || mine_state(&s, &a.id) == Some(AskState::Ready) && mine_state(&k, &b.id) == Some(AskState::Ready)).await;
}

#[tokio::test]
async fn ten_waiting_is_the_most() {
    let ntfy = FakeNtfy::start().await;
    let g = guest(&ntfy, &sam());
    for i in 0..OPEN_MAX {
        g.asks.request(item(&format!("Album {i}"))).await.unwrap();
    }
    let e = g.asks.request(item("One More")).await.unwrap_err();
    assert!(e.to_string().contains(FULL_NOTE), "{e}");
    // Taking one back makes room.
    let first = g.asks.inner.mine.lock()[0].id.clone();
    g.asks.cancel(&first).await.unwrap();
    g.asks.request(item("One More")).await.unwrap();
}

#[tokio::test]
async fn the_owner_also_stops_at_ten_waiting() {
    let ntfy = FakeNtfy::start().await;
    let (o, _g) = pair(&ntfy, &sam()).await;
    let p = Pair::new(&sam().token, SERVER);
    for i in 0..=OPEN_MAX {
        ntfy.inject(&p.up.topic, &wire::seal_up(&p, &Up::Ask { id: format!("f{i}"), item: item(&format!("Album {i}")), at: now_ms() }).unwrap());
    }
    until("ten of them", || o.asks.inner.incoming.lock().len() == OPEN_MAX).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(incoming_state(&o, &format!("f{OPEN_MAX}")), None, "the eleventh is refused, not kept");
    assert_eq!(o.asks.incoming().iter().filter(|a| a.state == AskState::Requested).count(), OPEN_MAX);
}

#[tokio::test]
async fn straight_away_goes_through_up_to_the_weekly_limit() {
    let ntfy = FakeNtfy::start().await;
    let o = owner(&ntfy);
    o.host.rule(&sam().user_id, Rule::Straight);
    o.host.settings.lock().weekly_limit = 2;
    let g = guest(&ntfy, &sam());
    start(&o);
    start(&g);
    until_async("straight away", || async { g.asks.view().await.straight_away }).await;
    let a = g.asks.request(item("One")).await.unwrap();
    let b = g.asks.request(item("Two")).await.unwrap();
    until("both to go through", || mine_state(&g, &a.id) == Some(AskState::Getting) && mine_state(&g, &b.id) == Some(AskState::Getting)).await;
    assert!(o.host.notices().is_empty());
    assert!(o.asks.incoming().iter().all(|x| x.by_itself));

    let c = g.asks.request(item("Three")).await.unwrap();
    until("the third to wait for a yes", || incoming_state(&o, &c.id) == Some(AskState::Requested) && !o.host.notices().is_empty()).await;
    assert_eq!(o.host.gets().len(), 2);
    assert_eq!(o.asks.incoming().iter().find(|x| x.id == c.id).unwrap().note.as_deref(), Some(OVER_LIMIT_NOTE));
    assert_eq!(o.host.notices(), vec!["Sam requested Three".to_string()]);
}

#[tokio::test]
async fn the_owners_first_ever_request_still_needs_their_yes() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    o.host.first_time.store(true, Ordering::SeqCst);
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("the owner to have it", || incoming_state(&o, &a.id).is_some()).await;
    let first = o.asks.answer(&a.id, true).await.unwrap();
    assert_eq!(first.needs.map(|n| n.kind), Some("firstTime".to_string()));
    assert_eq!(first.incoming.state, AskState::Requested);
    // Typed and confirmed: the next yes goes.
    o.host.first_time.store(false, Ordering::SeqCst);
    assert_eq!(o.asks.answer(&a.id, true).await.unwrap().incoming.state, AskState::Getting);
    until("the person to see it", || mine_state(&g, &a.id) == Some(AskState::Getting)).await;
}

// ---- no, take back, and other devices -------------------------------------------------

#[tokio::test]
async fn no_thanks_tells_the_person_and_they_can_ask_again() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("the owner to have it", || incoming_state(&o, &a.id).is_some()).await;
    o.asks.answer(&a.id, false).await.unwrap();
    until("the person to hear", || mine_state(&g, &a.id) == Some(AskState::Declined)).await;
    assert!(o.host.gets().is_empty());
    let b = g.asks.request(item("First Frost")).await.unwrap();
    assert_ne!(a.id, b.id);
    until("the second ask", || incoming_state(&o, &b.id) == Some(AskState::Requested)).await;
}

#[tokio::test]
async fn taking_a_request_back_takes_it_off_the_owners_list() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("the owner to have it", || incoming_state(&o, &a.id).is_some()).await;
    g.asks.cancel(&a.id).await.unwrap();
    until("it to go", || o.asks.incoming().is_empty()).await;
    assert!(g.asks.view().await.mine.is_empty());
    // A yes from a stale screen sends nothing.
    o.asks.answer(&a.id, true).await.unwrap();
    assert!(o.host.gets().is_empty());
}

#[tokio::test]
async fn something_on_its_way_cant_be_taken_back() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("the owner to have it", || incoming_state(&o, &a.id).is_some()).await;
    o.asks.answer(&a.id, true).await.unwrap();
    until("the person to see it coming", || mine_state(&g, &a.id) == Some(AskState::Getting)).await;
    assert!(g.asks.cancel(&a.id).await.is_err());
}

#[tokio::test]
async fn a_yes_on_one_of_the_owners_devices_shows_on_the_other() {
    let ntfy = FakeNtfy::start().await;
    let desk = owner(&ntfy);
    let phone = owner_too(&ntfy, &desk);
    let g = guest(&ntfy, &sam());
    start(&desk);
    start(&phone);
    start(&g);
    can_request(&g).await;
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("both devices to have it", || incoming_state(&desk, &a.id).is_some() && incoming_state(&phone, &a.id).is_some()).await;
    phone.asks.answer(&a.id, true).await.unwrap();
    until("the desk to see the yes", || incoming_state(&desk, &a.id) == Some(AskState::Getting)).await;
    assert_eq!(desk.host.gets().len(), 1);
}

#[tokio::test]
async fn requests_and_rules_survive_a_restart() {
    let ntfy = FakeNtfy::start().await;
    let (o, g) = pair(&ntfy, &sam()).await;
    let a = g.asks.request(item("First Frost")).await.unwrap();
    until("the owner to have it", || incoming_state(&o, &a.id).is_some()).await;
    let db = g.asks.inner.db.clone();
    let again = Asks::new(g.host.clone(), db, Arc::new(Ntfy::new().unwrap()), Arc::new(RwLock::new(String::new())), ntfy.url.clone());
    assert!(again.view().await.can_request);
    assert_eq!(again.view().await.mine.len(), 1);
    let db = o.asks.inner.db.clone();
    let o2 = Asks::new(o.host.clone(), db, Arc::new(Ntfy::new().unwrap()), Arc::new(RwLock::new(String::new())), ntfy.url.clone());
    assert_eq!(o2.incoming().len(), 1);
}

#[test]
fn lidarr_states_read_as_the_person_sees_them() {
    let s = |st, p| from_record(&record("x", st, p));
    assert_eq!(s(RequestState::Queued, None), (AskState::Getting, None));
    assert_eq!(s(RequestState::Downloading, Some(30)), (AskState::Getting, Some(30)));
    assert_eq!(s(RequestState::Imported, None), (AskState::Ready, None));
    assert_eq!(s(RequestState::NotFound, None), (AskState::NotFound, None));
    assert_eq!(s(RequestState::Failed, None), (AskState::Failed, None));
}

#[test]
fn rules_default_to_asking_first() {
    let mut s = AskSettings { on: true, rules: vec![("101".into(), Rule::Straight)], weekly_limit: 20, group_notices: true };
    assert_eq!(s.rule("101"), Rule::Straight);
    assert_eq!(s.rule("999"), Rule::Ask);
    s.on = false;
    assert_eq!(s.rule("101"), Rule::Off);
}
