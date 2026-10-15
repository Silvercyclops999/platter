//! Listen on against fakes: a wiremock Plex server, wiremock players, a fake
//! local player. Nothing here touches a real device or the network.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use url::Url;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::service::{LocalPlayer, RemoteConfig, RemoteEnv, RemoteService, ServerCtx};
use super::target::{self, PlayerTarget, TargetInfo};
use super::types::{RemoteCommand, RemoteStatus};
use super::RemoteSwitches;
use crate::audio::events::{PlayerState, PlayerStatus};
use crate::audio::queue::{ItemSource, QueueItem, QueueView, Repeat};
use crate::audio::source::{ContextKind, PlayContext};
use crate::audio::testutil::track;
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::plex::Token;
use crate::plex::identity::ClientIdentity;
use crate::plex::state::EventSink;

// ---- fakes ----------------------------------------------------------------

struct Env {
    base: Url,
}

#[async_trait]
impl RemoteEnv for Env {
    async fn server(&self) -> AppResult<ServerCtx> {
        Ok(ServerCtx {
            base: self.base.clone(),
            token: Token::new("server-token").unwrap(),
            machine_id: "srv".into(),
        })
    }
    async fn account_token(&self) -> Option<Token> {
        Some(Token::new("account-token").unwrap())
    }
    async fn account_id(&self) -> Option<String> {
        Some("acct-1".into())
    }
}

fn item(id: &str) -> QueueItem {
    QueueItem {
        item_id: format!("q{id}"),
        track: track(id, "A", &format!("/p/{id}")),
        source: ItemSource::Context,
    }
}

/// A local player that records what it's asked to do.
struct Local {
    state: Mutex<PlayerState>,
    log: Mutex<Vec<String>>,
    /// The chosen output, and the network route an engine would open.
    output: Mutex<Option<String>>,
    route: Mutex<Option<crate::audio::netsink::NetRoute>>,
    net: Mutex<Option<Box<dyn crate::audio::netsink::NetStream>>>,
}

impl Local {
    fn playing(current: &str, next: &[&str], position_ms: u64) -> Arc<Self> {
        let cur = item(current);
        Arc::new(Self {
            state: Mutex::new(PlayerState {
                status: PlayerStatus::Playing,
                track: Some(cur.track.clone()),
                position_ms,
                duration_ms: cur.track.duration_ms,
                volume: 0.5,
                muted: false,
                shuffle: false,
                shuffled_contexts: vec![],
                repeat: Repeat::Off,
                queue: QueueView {
                    current: Some(cur),
                    user_queue: vec![],
                    next_up: next.iter().map(|i| item(i)).collect(),
                    history: vec![],
                    context: None,
                },
                ab_loop: None,
                smart_shuffle: false,
                smart_contexts: vec![],
                smooth_transitions: false,
            }),
            log: Mutex::default(),
            output: Mutex::default(),
            route: Mutex::default(),
            net: Mutex::default(),
        })
    }
    fn log(&self) -> Vec<String> {
        self.log.lock().clone()
    }
    fn push(&self, s: String) {
        self.log.lock().push(s);
    }
}

#[async_trait]
impl LocalPlayer for Local {
    async fn state(&self) -> AppResult<PlayerState> {
        Ok(self.state.lock().clone())
    }
    async fn pause(&self) -> AppResult<()> {
        self.state.lock().status = PlayerStatus::Paused;
        self.push("pause".into());
        Ok(())
    }
    async fn resume(&self) -> AppResult<()> {
        self.state.lock().status = PlayerStatus::Playing;
        self.push("resume".into());
        Ok(())
    }
    async fn seek(&self, ms: u64) -> AppResult<()> {
        self.state.lock().position_ms = ms;
        self.push(format!("seek {ms}"));
        Ok(())
    }
    async fn next(&self) -> AppResult<()> {
        self.push("next".into());
        Ok(())
    }
    async fn previous(&self) -> AppResult<()> {
        self.push("previous".into());
        Ok(())
    }
    async fn set_volume(&self, v: f32) -> AppResult<()> {
        self.push(format!("volume {v}"));
        Ok(())
    }
    async fn set_shuffle(&self, on: bool) -> AppResult<()> {
        self.push(format!("shuffle {on}"));
        Ok(())
    }
    async fn set_repeat(&self, r: Repeat) -> AppResult<()> {
        self.push(format!("repeat {r:?}"));
        Ok(())
    }
    /// Like the queue: a queued entry drops the ones ahead of it, a
    /// context entry skips the context entries before it.
    async fn jump_at(&self, item_id: String, ms: u64, play: bool) -> AppResult<()> {
        {
            let mut st = self.state.lock();
            let q = &mut st.queue;
            let to = if let Some(i) = q.user_queue.iter().position(|x| x.item_id == item_id) {
                q.user_queue.drain(..=i).next_back()
            } else if let Some(j) = q.next_up.iter().position(|x| x.item_id == item_id) {
                q.next_up.drain(..=j).next_back()
            } else {
                None
            };
            if let Some(it) = to {
                if let Some(old) = q.current.replace(it.clone()) {
                    q.history.insert(0, old);
                }
                st.track = Some(it.track);
                st.position_ms = ms;
                st.status = if play { PlayerStatus::Playing } else { PlayerStatus::Paused };
            }
        }
        self.push(format!("jump {item_id} {ms} {play}"));
        Ok(())
    }
    async fn play_tracks(&self, ids: Vec<String>, start: Option<String>, ms: u64, play: bool) -> AppResult<()> {
        let first = start.clone().unwrap_or_else(|| ids[0].clone());
        let from = ids.iter().position(|i| *i == first).unwrap_or(0);
        let rest: Vec<&str> = ids[from + 1..].iter().map(String::as_str).collect();
        let mut st = Local::playing(&first, &rest, ms).state.lock().clone();
        st.status = if play { PlayerStatus::Playing } else { PlayerStatus::Paused };
        *self.state.lock() = st;
        self.push(format!("play {} from {} at {ms} {play}", ids.join(","), start.unwrap_or_default()));
        Ok(())
    }
    async fn tracks(&self, ids: &[String]) -> AppResult<Vec<Track>> {
        Ok(ids.iter().map(|i| track(i, "A", "/p")).collect())
    }
    async fn load_context(&self, context: PlayContext, start: Option<String>) -> AppResult<()> {
        let ids = context.track_ids.unwrap_or_default();
        let first = start.unwrap_or_else(|| ids[0].clone());
        let rest: Vec<&str> = ids.iter().filter(|i| **i != first).map(String::as_str).collect();
        let fresh = Local::playing(&first, &rest, 0);
        let mut st = fresh.state.lock().clone();
        st.status = PlayerStatus::Paused;
        *self.state.lock() = st;
        self.push(format!("load {}", ids.join(",")));
        Ok(())
    }
    async fn output_selected(&self) -> AppResult<Option<String>> {
        Ok(self.output.lock().clone())
    }
    async fn adopt(
        &self,
        _context: Option<crate::audio::queue::ContextInfo>,
        ids: Vec<String>,
        user: Vec<String>,
        shuffle: bool,
        repeat: Repeat,
        ms: u64,
        play: bool,
    ) -> AppResult<()> {
        let rest: Vec<&str> = ids[1..].iter().map(String::as_str).collect();
        let fresh = Local::playing(&ids[0], &rest, ms);
        let mut st = fresh.state.lock().clone();
        st.queue.user_queue = user.iter().map(|u| QueueItem { source: ItemSource::User, ..item(u) }).collect();
        st.status = if play { PlayerStatus::Playing } else { PlayerStatus::Paused };
        st.shuffle = shuffle;
        st.repeat = repeat;
        *self.state.lock() = st;
        self.push(format!("adopt {} | {} | shuffle {shuffle} {repeat:?} at {ms} {play}", ids.join(","), user.join(",")));
        Ok(())
    }
    /// Like the engine with its output open: a network id connects the
    /// route's target, anything else closes it.
    async fn select_output(&self, id: Option<String>) -> AppResult<()> {
        if let Some(mut old) = self.net.lock().take() {
            old.close();
        }
        if let Some(net) = id.as_deref().filter(|i| i.starts_with(crate::audio::netsink::NET_PREFIX)) {
            let target = self
                .route
                .lock()
                .as_ref()
                .and_then(|r| r.get())
                .filter(|t| t.device().id == net)
                .ok_or_else(|| AppError::Invalid("no such output".into()))?;
            let stream = tokio::task::spawn_blocking(move || target.connect()).await.unwrap()?;
            *self.net.lock() = Some(stream);
        }
        self.push(format!("output {}", id.clone().unwrap_or_else(|| "default".into())));
        *self.output.lock() = id;
        Ok(())
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<(String, serde_json::Value)>>);

impl EventSink for Events {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().push((event.to_string(), payload));
    }
}

impl Events {
    fn toasts(&self) -> Vec<String> {
        self.0
            .lock()
            .iter()
            .filter(|(e, _)| e == "toast")
            .map(|(_, p)| p["message"].as_str().unwrap_or_default().to_string())
            .collect()
    }
}

/// A fake player's timeline answer, changeable mid-test.
#[derive(Clone)]
struct Timeline(Arc<Mutex<(u16, String)>>);

impl Timeline {
    fn new() -> Self {
        Self(Arc::new(Mutex::new((200, timeline("stopped", None, 0)))))
    }
    fn set(&self, status: u16, body: String) {
        *self.0.lock() = (status, body);
    }
}

impl Respond for Timeline {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let (s, b) = self.0.lock().clone();
        ResponseTemplate::new(s).set_body_string(b)
    }
}

fn timeline(state: &str, key: Option<&str>, time: u64) -> String {
    let k = key.map(|k| format!(" ratingKey=\"{k}\" key=\"/library/metadata/{k}\"")).unwrap_or_default();
    format!(
        "<MediaContainer><Timeline type=\"music\" state=\"{state}\" time=\"{time}\" duration=\"180000\" volume=\"60\" shuffle=\"0\" repeat=\"0\" playQueueID=\"55\"{k}/></MediaContainer>"
    )
}

struct Rig {
    server: MockServer,
    player: MockServer,
    tl: Timeline,
    local: Arc<Local>,
    events: Arc<Events>,
    remote: RemoteService,
    switches: RemoteSwitches,
}

fn cfg(plex_tv: &MockServer) -> RemoteConfig {
    RemoteConfig {
        plex_tv: Url::parse(&plex_tv.uri()).unwrap(),
        relay: Url::parse(&plex_tv.uri()).unwrap(),
        poll: Duration::from_millis(40),
        refresh: Duration::from_secs(60),
        // Longer than any test: a player that never reports playing keeps
        // its session for the whole test, however slowly the machine runs.
        // Tests of a stop rely on `seen_playing`, not on this running out.
        grace: Duration::from_secs(60),
        // Failures in these tests are answered at once (an error status, a
        // refused connection), so the limit only matters when the machine
        // is too slow to answer in time, which must not count as a miss.
        misses: 2,
        timeout: Duration::from_secs(10),
    }
}

/// A server listing one fake player ("Desk", id "amp") in `/clients`.
async fn rig() -> Rig {
    let entries = json!([
        {"ratingKey": "a", "playQueueItemID": 1}, {"ratingKey": "b", "playQueueItemID": 2}, {"ratingKey": "c", "playQueueItemID": 3}
    ]);
    rig_with(Local::playing("a", &["b", "c"], 61_000), entries).await
}

/// [`rig`] with this computer playing `local`, and play queue 55 holding
/// `entries` on the server.
async fn rig_with(local: Arc<Local>, entries: serde_json::Value) -> Rig {
    let server = MockServer::start().await;
    let player = MockServer::start().await;
    let pu = Url::parse(&player.uri()).unwrap();
    Mock::given(method("GET"))
        .and(path("/clients"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"size": 1, "Server": [
            {"name": "Desk", "address": pu.host_str().unwrap(), "port": pu.port().unwrap(), "machineIdentifier": "amp",
             "product": "Plexamp", "protocolCapabilities": "timeline,playback,playqueues"}
        ]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/status/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"size": 0}})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/playQueues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"playQueueID": 55, "playQueueSelectedItemID": 1}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/playQueues/55"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"Metadata": entries, "playQueueSelectedItemID": 1}})))
        .mount(&server)
        .await;
    let tl = Timeline::new();
    Mock::given(method("GET"))
        .and(path("/player/timeline/poll"))
        .respond_with(tl.clone())
        .mount(&player)
        .await;
    Mock::given(method("GET"))
        .and(path_regex_playback())
        .respond_with(ResponseTemplate::new(200).set_body_string("<Response code=\"200\" status=\"OK\"/>"))
        .mount(&player)
        .await;
    let events = Arc::new(Events::default());
    let switches = RemoteSwitches::default();
    let remote = RemoteService::new(
        Arc::new(Env { base: Url::parse(&server.uri()).unwrap() }),
        local.clone(),
        events.clone(),
        &ClientIdentity::with_id("me"),
        switches.clone(),
        cfg(&server),
    )
    .unwrap();
    Rig { server, player, tl, local, events, remote, switches }
}

fn path_regex_playback() -> wiremock::matchers::PathRegexMatcher {
    wiremock::matchers::path_regex("^/player/playback/.*")
}

async fn requests(s: &MockServer, p: &str) -> Vec<Request> {
    s.received_requests().await.unwrap_or_default().into_iter().filter(|r| r.url.path() == p).collect()
}

fn q(r: &Request) -> HashMap<String, String> {
    r.url.query_pairs().into_owned().collect()
}

async fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    // A hang guard: it returns as soon as `ok` holds.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

// ---- discovery --------------------------------------------------------------

#[tokio::test]
async fn players_merge_sources_and_hide_when_the_setting_is_off() {
    let r = rig().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/resources"))
        .and(header("x-plex-token", "account-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"name": "Desk", "product": "Plexamp", "provides": "client,player", "clientIdentifier": "amp", "presence": false, "connections": []},
            {"name": "Phone", "product": "Plexamp", "platform": "iOS", "provides": "client,player", "clientIdentifier": "phone", "presence": false, "connections": []},
            {"name": "Me", "product": "Platter", "provides": "player", "clientIdentifier": "me"}
        ])))
        .mount(&r.server)
        .await;
    let list = r.remote.players().await;
    let ids: Vec<_> = list.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["amp", "phone"], "online first, this app left out");
    assert!(list[0].online && !list[1].online);
    assert!(r.events.0.lock().iter().any(|(e, _)| e == "remote://players"));

    r.switches.set(false, true, false);
    assert!(r.remote.players().await.is_empty());
}

// ---- move ---------------------------------------------------------------------

#[tokio::test]
async fn move_creates_a_queue_plays_it_at_the_offset_and_pauses_here() {
    let r = rig().await;
    r.remote.players().await;
    let st = r.remote.move_to("amp").await.unwrap();
    let s = st.session.unwrap();
    assert_eq!((s.player_name.as_str(), s.status, s.position_ms), ("Desk", RemoteStatus::Connecting, 61_000));

    let pq = &requests(&r.server, "/playQueues").await[0];
    let pq = q(pq);
    assert_eq!(pq["type"], "audio");
    assert_eq!(pq["uri"], "server://srv/com.plexapp.plugins.library/library/metadata/a,b,c");
    assert_eq!(pq["key"], "/library/metadata/a");
    assert_eq!(pq["shuffle"], "0");

    let pm = requests(&r.player, "/player/playback/playMedia").await;
    assert_eq!(pm.len(), 1);
    let p = q(&pm[0]);
    assert_eq!(p["offset"], "61000");
    assert_eq!(p["key"], "/library/metadata/a");
    assert_eq!(p["containerKey"], "/playQueues/55?window=100&own=1");
    assert_eq!(p["machineIdentifier"], "srv");
    assert_eq!(p["type"], "music");
    assert!(p.contains_key("commandID"));
    assert_eq!(pm[0].headers.get("x-plex-target-client-identifier").unwrap(), "amp");
    assert_eq!(r.local.log(), ["pause"]);

    // The player reports in: playing, position follows.
    r.tl.set(200, timeline("playing", Some("a"), 62_000));
    wait_for("playing", || r.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Playing)).await;
    let s = r.remote.state().session.unwrap();
    assert_eq!(s.volume, Some(0.6));
    assert!(s.position_ms >= 62_000);
    // Timeline follows a song change.
    r.tl.set(200, timeline("playing", Some("b"), 1_000));
    wait_for("song b", || r.remote.state().session.and_then(|s| s.track).is_some_and(|t| t.id == "b")).await;
}

/// Ids as the cache keeps them, with the fake server's tag.
fn tagged(k: &str) -> String {
    format!("{}:{k}", crate::sid::tag("srv"))
}

#[tokio::test]
async fn tagged_ids_go_to_the_player_as_the_servers_own_and_come_back_tagged() {
    let (a, b, c) = (tagged("1"), tagged("2"), tagged("3"));
    let entries = json!([
        {"ratingKey": "1", "playQueueItemID": 1}, {"ratingKey": "2", "playQueueItemID": 2}, {"ratingKey": "3", "playQueueItemID": 3}
    ]);
    let r = rig_with(Local::playing(&a, &[&b, &c], 61_000), entries).await;
    r.remote.players().await;
    r.remote.move_to("amp").await.unwrap();

    let pq = q(&requests(&r.server, "/playQueues").await[0]);
    assert_eq!(pq["uri"], "server://srv/com.plexapp.plugins.library/library/metadata/1,2,3");
    assert_eq!(pq["key"], "/library/metadata/1");
    let pm = q(&requests(&r.player, "/player/playback/playMedia").await[0]);
    assert_eq!(pm["key"], "/library/metadata/1");

    // The player names songs by the server's ids; here they're the cache's.
    r.tl.set(200, timeline("playing", Some("2"), 1_000));
    wait_for("song 2", || r.remote.state().session.and_then(|s| s.track).is_some_and(|t| t.id == b)).await;
    // The queue here moves along to it rather than being replaced.
    let want = format!("jump q{b} 1000 false");
    wait_for("queue follows", || r.local.log().contains(&want)).await;
    assert!(!r.local.log().iter().any(|l| l.starts_with("adopt")), "{:?}", r.local.log());
}

#[tokio::test]
async fn return_takes_the_play_queue_with_tagged_ids() {
    let entries = json!([
        {"ratingKey": "1", "playQueueItemID": 1}, {"ratingKey": "2", "playQueueItemID": 2}, {"ratingKey": "9", "playQueueItemID": 3}
    ]);
    let r = rig_with(Local::playing(&tagged("1"), &[&tagged("2")], 0), entries).await;
    r.switches.set(true, false, true);
    r.remote.players().await;
    r.remote.move_to("amp").await.unwrap();
    r.tl.set(200, timeline("paused", Some("9"), 5_000));
    // The queue here stays its own while the player plays (old device kept
    // going), so coming back takes the player's play queue whole.
    r.remote.return_here().await.unwrap();
    let (one, two, nine) = (tagged("1"), tagged("2"), tagged("9"));
    let want = format!("play {one},{two},{nine} from {nine} at 5000 false");
    assert!(r.local.log().contains(&want), "{:?}", r.local.log());
}

#[tokio::test]
async fn keeping_the_old_device_playing_skips_the_pause() {
    let r = rig().await;
    r.switches.set(true, false, false);
    r.remote.move_to("amp").await.unwrap();
    assert!(r.local.log().is_empty());
}

#[tokio::test]
async fn a_refused_move_says_couldnt_reach_and_keeps_playing_here() {
    let r = rig().await;
    r.player.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&r.player)
        .await;
    r.remote.players().await;
    let e = r.remote.move_to("amp").await.unwrap_err();
    assert!(matches!(&e, AppError::Network(m) if m == "Couldn't reach Desk"), "{e:?}");
    assert!(r.local.log().is_empty());
    assert!(r.remote.state().session.is_none());
}

// ---- commands ------------------------------------------------------------------

#[tokio::test]
async fn controls_map_to_companion_commands() {
    let r = rig().await;
    r.remote.move_to("amp").await.unwrap();
    for c in [
        RemoteCommand::Pause,
        RemoteCommand::Play,
        RemoteCommand::Next,
        RemoteCommand::Previous,
        RemoteCommand::Seek { position_ms: 90_000 },
        RemoteCommand::Volume { volume: 0.4 },
        RemoteCommand::Shuffle { on: true },
        RemoteCommand::Repeat { mode: Repeat::All },
    ] {
        r.remote.command(c).await.unwrap();
    }
    let got: Vec<(String, HashMap<String, String>)> = r
        .player
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|x| x.url.path().starts_with("/player/playback/") && x.url.path() != "/player/playback/playMedia")
        .map(|x| (x.url.path().to_string(), q(&x)))
        .collect();
    let paths: Vec<_> = got.iter().map(|(p, _)| p.trim_start_matches("/player/playback/")).collect();
    assert_eq!(paths, ["pause", "play", "skipNext", "skipPrevious", "seekTo", "setParameters", "setParameters", "setParameters"]);
    assert_eq!(got[4].1["offset"], "90000");
    assert_eq!(got[5].1["volume"], "40");
    assert_eq!(got[6].1["shuffle"], "1");
    assert_eq!(got[7].1["repeat"], "2");
    assert!(got.iter().all(|(_, p)| p["type"] == "music" && p.contains_key("commandID")));
    // Command ids count up.
    let ids: Vec<u64> = got.iter().map(|(_, p)| p["commandID"].parse().unwrap()).collect();
    assert!(ids.windows(2).all(|w| w[1] > w[0]));
    let s = r.remote.state().session.unwrap();
    assert_eq!((s.volume, s.shuffle, s.repeat), (Some(0.4), true, Repeat::All));
}

#[tokio::test]
async fn commands_without_a_remote_session_are_refused() {
    let r = rig().await;
    assert!(r.remote.command(RemoteCommand::Play).await.is_err());
}

// ---- return -------------------------------------------------------------------

#[tokio::test]
async fn return_resumes_here_at_the_players_song_and_position() {
    let r = rig().await;
    r.remote.move_to("amp").await.unwrap();
    r.tl.set(200, timeline("playing", Some("c"), 42_000));
    wait_for("song c", || r.remote.state().session.and_then(|s| s.track).is_some_and(|t| t.id == "c")).await;
    let st = r.remote.return_here().await.unwrap();
    assert!(st.session.is_none());
    // The queue here already followed the player to c (paused); it plays on from its spot.
    assert!(r.local.log().iter().any(|l| l.starts_with("jump qc ") && l.ends_with("false")), "{:?}", r.local.log());
    let st = r.local.state.lock().clone();
    assert_eq!((st.track.unwrap().id.as_str(), st.position_ms, st.status), ("c", 42_000, PlayerStatus::Playing));
    assert_eq!(requests(&r.player, "/player/playback/stop").await.len(), 1);
}

#[tokio::test]
async fn return_after_the_player_moved_past_the_queue_takes_its_play_queue() {
    let r = rig().await;
    Mock::given(method("GET"))
        .and(path("/playQueues/55"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"Metadata": [
            {"ratingKey": "a", "playQueueItemID": 1}, {"ratingKey": "b", "playQueueItemID": 2}, {"ratingKey": "z", "playQueueItemID": 3}
        ]}})))
        .with_priority(1)
        .mount(&r.server)
        .await;
    r.remote.move_to("amp").await.unwrap();
    r.tl.set(200, timeline("paused", Some("z"), 5_000));
    r.remote.return_here().await.unwrap();
    // Either the queue here followed it already, or it's taken on the way back.
    let log = r.local.log();
    assert!(log.iter().any(|l| l == "play a,b,z from z at 5000 false" || l.starts_with("adopt z | ")), "{log:?}");
    let st = r.local.state.lock().clone();
    assert_eq!((st.track.unwrap().id.as_str(), st.position_ms, st.status), ("z", 5_000, PlayerStatus::Paused));
}

#[tokio::test]
async fn return_on_the_same_song_seeks_and_resumes() {
    let r = rig().await;
    r.remote.move_to("amp").await.unwrap();
    r.tl.set(200, timeline("playing", Some("a"), 70_000));
    r.remote.return_here().await.unwrap();
    assert_eq!(r.local.log(), ["pause", "seek 70000", "resume"]);
}

// ---- failure states ---------------------------------------------------------

#[tokio::test]
async fn a_player_that_goes_away_hands_back_paused_with_a_toast() {
    let r = rig().await;
    r.remote.move_to("amp").await.unwrap();
    r.tl.set(200, timeline("playing", Some("b"), 10_000));
    wait_for("song b", || r.remote.state().session.and_then(|s| s.track).is_some_and(|t| t.id == "b")).await;
    r.tl.set(500, String::new());
    wait_for("fallback", || r.remote.state().session.is_none()).await;
    assert!(r.events.toasts().contains(&"Couldn't reach Desk".to_string()));
    wait_for("local jump", || r.local.log().iter().any(|l| l.starts_with("jump qb ") && l.ends_with("false"))).await;
    assert_eq!(r.remote.state().stopped_on, None);
}

#[tokio::test]
async fn stopped_by_someone_else_says_so_and_controls_come_back() {
    let r = rig().await;
    r.remote.move_to("amp").await.unwrap();
    r.tl.set(200, timeline("playing", Some("a"), 65_000));
    wait_for("playing", || r.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Playing)).await;
    r.tl.set(200, timeline("stopped", None, 0));
    wait_for("stopped", || r.remote.state().stopped_on.is_some()).await;
    let st = r.remote.state();
    assert_eq!(st.stopped_on.as_deref(), Some("Desk"));
    assert!(st.session.is_none());
    assert!(r.events.toasts().is_empty());
    // Back here on the same song, paused.
    assert!(r.local.log().contains(&"pause".to_string()));
    // A later return or move clears the note.
    r.remote.return_here().await.unwrap();
    assert_eq!(r.remote.state().stopped_on, None);
}

#[tokio::test]
async fn a_player_still_starting_is_not_counted_as_stopped() {
    let r = rig().await;
    r.remote.move_to("amp").await.unwrap();
    // "stopped" right after playMedia, inside the grace window, for
    // several polls in a row.
    let before = requests(&r.player, "/player/timeline/poll").await.len();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while requests(&r.player, "/player/timeline/poll").await.len() < before + 3 {
        assert!(std::time::Instant::now() < deadline, "the player was never polled");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(r.remote.state().session.is_some());
}

// ---- Platter as a player ---------------------------------------------------------

async fn target_rig() -> (std::net::SocketAddr, Arc<Local>, MockServer, tokio::sync::watch::Sender<bool>) {
    let r = rig().await;
    let local = Local::playing("a", &["b"], 1_000);
    let handler = Arc::new(PlayerTarget {
        local: local.clone(),
        env: Arc::new(Env { base: Url::parse(&r.server.uri()).unwrap() }),
        http: reqwest::Client::new(),
    });
    let info = Arc::new(TargetInfo {
        client_id: "me".into(),
        name: "Desk".into(),
        product: "Platter".into(),
        version: "0.1.0".into(),
        platform: "Windows".into(),
        platform_version: "x".into(),
    });
    // Loopback only: tests never listen on the real network.
    let listener = target::bind(std::net::Ipv4Addr::LOCALHOST, 0, 1).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(target::serve(listener, handler, info, rx));
    (addr, local, r.server, stop)
}

#[tokio::test]
async fn platter_as_a_player_answers_play_media_and_timeline_poll() {
    let (addr, local, _server, stop) = target_rig().await;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{base}/player/playback/playMedia"))
        .query(&[
            ("key", "/library/metadata/b"),
            ("offset", "5000"),
            ("containerKey", "/playQueues/55?window=100&own=1"),
            ("machineIdentifier", "srv"),
            ("commandID", "1"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["access-control-allow-origin"], "*");
    assert_eq!(local.log(), ["play a,b,c from b at 5000 true"]);

    let body = http.get(format!("{base}/player/timeline/poll?wait=0&commandID=2")).send().await.unwrap().text().await.unwrap();
    let t = crate::remote::companion::parse_timeline(&body).unwrap();
    assert_eq!((t.state.as_str(), t.time_ms, t.rating_key.as_deref()), ("playing", 5_000, Some("b")));
    assert_eq!(t.volume, Some(50));

    for (p, want) in [
        ("/player/playback/pause", "pause"),
        ("/player/playback/seekTo?offset=3000", "seek 3000"),
        ("/player/playback/skipNext", "next"),
        ("/player/playback/setParameters?volume=20", "volume 0.2"),
        ("/player/playback/setParameters?repeat=1", "repeat One"),
    ] {
        let resp = http.get(format!("{base}{p}")).send().await.unwrap();
        assert_eq!(resp.status(), 200, "{p}");
        assert_eq!(local.log().last().map(String::as_str), Some(want));
    }

    // Another server's media is refused; unknown paths are 404.
    let resp = http
        .get(format!("{base}/player/playback/playMedia?key=/library/metadata/b&machineIdentifier=other"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(http.get(format!("{base}/nope")).send().await.unwrap().status(), 404);
    let res = http.get(format!("{base}/resources")).send().await.unwrap().text().await.unwrap();
    assert!(res.contains("machineIdentifier=\"me\""));

    stop.send(true).unwrap();
}

#[tokio::test]
async fn playing_something_new_while_remote_sends_it_to_the_same_player() {
    let r = rig().await;
    r.remote.move_to("amp").await.unwrap();
    let ctx = PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(vec!["x".into(), "y".into()]) };
    let st = r.remote.play_context(ctx, None).await.unwrap();
    assert_eq!(st.session.unwrap().track.unwrap().id, "x");
    assert!(r.local.log().contains(&"load x,y".to_string()));
    let pqs = requests(&r.server, "/playQueues").await;
    assert_eq!(q(&pqs[1])["uri"], "server://srv/com.plexapp.plugins.library/library/metadata/x,y");
    let pm = requests(&r.player, "/player/playback/playMedia").await;
    assert_eq!(pm.len(), 2);
    assert_eq!(q(&pm[1])["offset"], "0");
    // Not remote: refused.
    r.remote.return_here().await.unwrap();
    let ctx = PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(vec!["x".into()]) };
    assert!(r.remote.play_context(ctx, None).await.is_err());
}

#[tokio::test]
async fn a_queue_jump_while_remote_moves_the_queue_here_paused_and_resends() {
    let r = rig().await;
    r.remote.move_to("amp").await.unwrap();
    r.remote.queue_jump("qc".into()).await.unwrap();
    assert!(r.local.log().contains(&"jump qc 0 false".to_string()));
    assert_eq!(requests(&r.player, "/player/playback/playMedia").await.len(), 2);
    assert!(r.remote.state().session.is_some());
}

// ---- the queue while a Plex player plays ------------------------------------------

/// A server play queue (id 55) that takes edits the way Plex does: adds go
/// to the end, every change bumps the version.
#[derive(Clone)]
struct FakeQueue(Arc<Mutex<FakeQueueState>>);

struct FakeQueueState {
    version: u32,
    selected: u32,
    entries: Vec<(u32, String)>,
    next_id: u32,
    fail: bool,
}

impl FakeQueue {
    fn new(keys: &[&str]) -> Self {
        let entries: Vec<(u32, String)> = keys.iter().enumerate().map(|(i, k)| (i as u32 + 1, k.to_string())).collect();
        let next_id = entries.len() as u32 + 1;
        Self(Arc::new(Mutex::new(FakeQueueState { version: 1, selected: 1, entries, next_id, fail: false })))
    }
    async fn mount(&self, server: &MockServer) {
        Mock::given(wiremock::matchers::path_regex("^/playQueues/55"))
            .respond_with(self.clone())
            .with_priority(1)
            .mount(server)
            .await;
    }
    fn keys(&self) -> Vec<String> {
        self.0.lock().entries.iter().map(|(_, k)| k.clone()).collect()
    }
    fn version(&self) -> u32 {
        self.0.lock().version
    }
    /// Changed on the server by someone else (another app, a shuffle there).
    fn replace(&self, keys: &[&str]) {
        let mut st = self.0.lock();
        let mut old = std::mem::take(&mut st.entries);
        for k in keys {
            let id = match old.iter().position(|(_, x)| x == k) {
                Some(p) => old.remove(p).0,
                None => {
                    st.next_id += 1;
                    st.next_id - 1
                }
            };
            st.entries.push((id, k.to_string()));
        }
        st.version += 1;
    }
    fn body(st: &FakeQueueState) -> serde_json::Value {
        json!({"MediaContainer": {
            "playQueueID": 55,
            "playQueueVersion": st.version,
            "playQueueSelectedItemID": st.selected,
            "playQueueTotalCount": st.entries.len(),
            "Metadata": st.entries.iter().map(|(id, k)| json!({"playQueueItemID": id, "ratingKey": k})).collect::<Vec<_>>(),
        }})
    }
}

impl Respond for FakeQueue {
    fn respond(&self, r: &Request) -> ResponseTemplate {
        let mut st = self.0.lock();
        if st.fail {
            return ResponseTemplate::new(500);
        }
        let parts: Vec<String> = r.url.path().trim_start_matches('/').split('/').map(str::to_string).collect();
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        let qs = q(r);
        match (r.method.as_str(), parts.as_slice()) {
            ("GET", ["playQueues", "55"]) => {}
            ("PUT", ["playQueues", "55"]) => {
                let keys = qs["uri"].rsplit('/').next().unwrap().to_string();
                for k in keys.split(',') {
                    let id = st.next_id;
                    st.next_id += 1;
                    st.entries.push((id, k.to_string()));
                }
                st.version += 1;
            }
            ("PUT", ["playQueues", "55", "items", item, "move"]) => {
                let item: u32 = item.parse().unwrap();
                let after: u32 = qs["after"].parse().unwrap();
                let from = st.entries.iter().position(|e| e.0 == item).unwrap();
                let e = st.entries.remove(from);
                let to = st.entries.iter().position(|e| e.0 == after).unwrap() + 1;
                st.entries.insert(to, e);
                st.version += 1;
            }
            ("DELETE", ["playQueues", "55", "items", item]) => {
                let item: u32 = item.parse().unwrap();
                st.entries.retain(|e| e.0 != item);
                st.version += 1;
            }
            _ => return ResponseTemplate::new(404),
        }
        ResponseTemplate::new(200).set_body_json(Self::body(&st))
    }
}

/// A timeline that names the play queue entry and version, like Plexamp's.
fn timeline_at(state: &str, key: &str, item: u32, version: u32, time: u64) -> String {
    format!(
        "<MediaContainer><Timeline type=\"music\" state=\"{state}\" time=\"{time}\" duration=\"180000\" volume=\"60\" shuffle=\"0\" repeat=\"0\" playQueueID=\"55\" playQueueItemID=\"{item}\" playQueueVersion=\"{version}\" ratingKey=\"{key}\" key=\"/library/metadata/{key}\"/></MediaContainer>"
    )
}

impl Local {
    fn now(&self) -> (String, Vec<String>, Vec<String>) {
        let st = self.state.lock();
        let ids = |v: &[QueueItem]| v.iter().map(|i| i.track.id.clone()).collect::<Vec<_>>();
        (st.track.as_ref().map(|t| t.id.clone()).unwrap_or_default(), ids(&st.queue.user_queue), ids(&st.queue.next_up))
    }
    /// What `queue_add` does to the queue here.
    fn queue(&self, ids: &[&str], next: bool) {
        let mut st = self.state.lock();
        let items: Vec<QueueItem> = ids.iter().map(|i| QueueItem { source: ItemSource::User, ..item(i) }).collect();
        if next {
            st.queue.user_queue.splice(0..0, items);
        } else {
            st.queue.user_queue.extend(items);
        }
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// On the fake player ("amp") with the play queue a, b, c, playing a.
async fn on_player() -> (Rig, FakeQueue) {
    let r = rig().await;
    let fq = FakeQueue::new(&["a", "b", "c"]);
    fq.mount(&r.server).await;
    r.remote.move_to("amp").await.unwrap();
    r.tl.set(200, timeline_at("playing", "a", 1, 1, 62_000));
    wait_for("playing", || r.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Playing)).await;
    (r, fq)
}

#[tokio::test]
async fn the_queue_here_follows_the_player_along_its_play_queue() {
    let (r, fq) = on_player().await;
    assert_eq!(r.local.now(), ("a".into(), vec![], strs(&["b", "c"])));
    // The player goes on to b by itself: the queue panel shows c next.
    fq.0.lock().selected = 2;
    r.tl.set(200, timeline_at("playing", "b", 2, 1, 1_000));
    wait_for("on b", || r.local.now().0 == "b").await;
    assert_eq!(r.local.now(), ("b".into(), vec![], strs(&["c"])));
    assert!(r.local.log().iter().any(|l| l.starts_with("jump qb ") && l.ends_with("false")), "{:?}", r.local.log());
    assert_eq!(r.local.state.lock().status, PlayerStatus::Paused, "nothing sounds here");
    // Someone adds to the play queue on the player itself: a new version.
    fq.replace(&["a", "b", "x", "c"]);
    r.tl.set(200, timeline_at("playing", "b", 2, fq.version(), 2_000));
    wait_for("x shows", || r.local.now().2 == strs(&["x", "c"])).await;
    assert_eq!(r.local.now().0, "b");
    assert_eq!(r.local.state.lock().status, PlayerStatus::Paused);
}

#[tokio::test]
async fn add_to_queue_on_a_plex_player_goes_into_its_play_queue() {
    let (r, fq) = on_player().await;
    // Add to queue: the song lands after the playing one, before the rest.
    edit(&r.remote, || { r.local.queue(&["x"], false); }).await.unwrap();
    assert_eq!(fq.keys(), strs(&["a", "x", "b", "c"]));
    let adds: Vec<_> = r.server.received_requests().await.unwrap().into_iter().filter(|x| x.method.as_str() == "PUT" && x.url.path() == "/playQueues/55").collect();
    assert_eq!(adds.len(), 1);
    assert_eq!(q(&adds[0])["uri"], "server://srv/com.plexapp.plugins.library/library/metadata/x");
    // The player is told to pick up the new version, without starting over.
    let refresh = requests(&r.player, "/player/playback/refreshPlayQueue").await;
    assert_eq!(refresh.len(), 1);
    assert_eq!((q(&refresh[0])["playQueueID"].as_str(), q(&refresh[0])["type"].as_str()), ("55", "music"));
    assert_eq!(requests(&r.player, "/player/playback/playMedia").await.len(), 1, "no new playMedia");
    // Play next goes ahead of it; adding several keeps their order.
    edit(&r.remote, || { r.local.queue(&["y"], true); }).await.unwrap();
    assert_eq!(fq.keys(), strs(&["a", "y", "x", "b", "c"]));
    edit(&r.remote, || { r.local.queue(&["p", "q"], false); }).await.unwrap();
    assert_eq!(fq.keys(), strs(&["a", "y", "x", "p", "q", "b", "c"]));
    // The player reports the new version: the panel keeps showing them as queued.
    r.tl.set(200, timeline_at("playing", "a", 1, fq.version(), 63_000));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(r.local.now(), ("a".into(), strs(&["y", "x", "p", "q"]), strs(&["b", "c"])));
    assert!(!r.local.log().iter().any(|l| l.starts_with("adopt")), "{:?}", r.local.log());
    // When the player goes on to the queued song, the queue here follows.
    fq.0.lock().selected = 5;
    let y = fq.0.lock().entries[1].0;
    r.tl.set(200, timeline_at("playing", "y", y, fq.version(), 0));
    wait_for("on y", || r.local.now().0 == "y").await;
    assert_eq!(r.local.now(), ("y".into(), strs(&["x", "p", "q"]), strs(&["b", "c"])));
}

#[tokio::test]
async fn queue_edits_with_tagged_ids_reach_the_play_queue_as_the_servers_own() {
    let (one, two) = (tagged("1"), tagged("2"));
    let r = rig_with(Local::playing(&one, &[&two], 0), json!([])).await;
    let fq = FakeQueue::new(&["1", "2"]);
    fq.mount(&r.server).await;
    r.remote.players().await;
    r.remote.move_to("amp").await.unwrap();
    r.tl.set(200, timeline_at("playing", "1", 1, 1, 1_000));
    wait_for("playing", || r.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Playing)).await;
    // A song from this server and one from another server are queued here.
    let elsewhere = format!("{}:7", crate::sid::tag("another-server"));
    let five = tagged("5");
    edit(&r.remote, || { r.local.queue(&[&five, &elsewhere], false); }).await.unwrap();
    // Only this server's song goes in, by its own id; nothing already there moves.
    assert_eq!(fq.keys(), strs(&["1", "5", "2"]));
    let gone = r.server.received_requests().await.unwrap().into_iter().filter(|x| x.method.as_str() == "DELETE").count();
    assert_eq!(gone, 0);
}

#[tokio::test]
async fn remove_reorder_and_clear_reach_the_play_queue() {
    let (r, fq) = on_player().await;
    edit(&r.remote, || { r.local.queue(&["x", "y"], false); }).await.unwrap();
    assert_eq!(fq.keys(), strs(&["a", "x", "y", "b", "c"]));
    // Remove an upcoming context song.
    edit(&r.remote, || { r.local.state.lock().queue.next_up.retain(|i| i.track.id != "c"); }).await.unwrap();
    assert_eq!(fq.keys(), strs(&["a", "x", "y", "b"]));
    let dels: Vec<_> = r.server.received_requests().await.unwrap().into_iter().filter(|x| x.method.as_str() == "DELETE").collect();
    assert_eq!(dels.len(), 1);
    assert_eq!(dels[0].url.path(), "/playQueues/55/items/3");
    // Drag y above x: one move.
    edit(&r.remote, || { r.local.state.lock().queue.user_queue.reverse(); }).await.unwrap();
    assert_eq!(fq.keys(), strs(&["a", "y", "x", "b"]));
    let moves = r.server.received_requests().await.unwrap().into_iter().filter(|x| x.url.path().ends_with("/move")).count();
    // Two to put the first adds after a, one for this drag.
    assert_eq!(moves, 3);
    // Clear queue: only the queued songs go.
    edit(&r.remote, || { r.local.state.lock().queue.user_queue.clear(); }).await.unwrap();
    assert_eq!(fq.keys(), strs(&["a", "b"]));
    assert_eq!(requests(&r.player, "/player/playback/refreshPlayQueue").await.len(), 4);
}

#[tokio::test]
async fn an_edit_while_the_player_is_a_song_ahead_goes_after_its_song() {
    let r = rig().await;
    // Keep playing here: the queue here stays where it was.
    r.switches.set(true, false, false);
    let fq = FakeQueue::new(&["a", "b", "c"]);
    fq.mount(&r.server).await;
    r.remote.move_to("amp").await.unwrap();
    fq.0.lock().selected = 2;
    r.tl.set(200, timeline_at("playing", "b", 2, 1, 1_000));
    wait_for("on b", || r.remote.state().session.and_then(|s| s.track).is_some_and(|t| t.id == "b")).await;
    edit(&r.remote, || { r.local.queue(&["x"], false); }).await.unwrap();
    assert_eq!(fq.keys(), strs(&["a", "b", "x", "c"]));
}

#[tokio::test]
async fn a_shuffle_on_the_player_shows_in_the_queue_here() {
    let (r, fq) = on_player().await;
    r.remote.command(RemoteCommand::Shuffle { on: true }).await.unwrap();
    // The server reorders the play queue; this player doesn't report versions.
    fq.0.lock().entries.swap(1, 2);
    r.tl.set(200, timeline("playing", Some("a"), 63_000));
    wait_for("reshuffled", || r.local.now().2 == strs(&["c", "b"])).await;
}

#[tokio::test]
async fn a_play_queue_that_doesnt_answer_says_the_queue_wasnt_updated() {
    let (r, fq) = on_player().await;
    fq.0.lock().fail = true;
    let e = edit(&r.remote, || { r.local.queue(&["x"], false); }).await.unwrap_err();
    assert!(matches!(&e, AppError::Network(m) if m == "Couldn't update the queue on Desk"), "{e:?}");
    assert!(requests(&r.player, "/player/playback/refreshPlayQueue").await.is_empty());
    // Still playing there.
    assert!(r.remote.state().session.is_some());
}

/// A queue edit here, sent on to the player the way the app does it.
async fn edit(remote: &RemoteService, f: impl FnOnce()) -> AppResult<()> {
    remote
        .edit_queue(|| async {
            f();
            Ok(())
        })
        .await
}

#[tokio::test]
async fn a_poll_between_an_edit_and_sending_it_does_not_undo_it() {
    let (r, fq) = on_player().await;
    // The worst moment: the edit is made here, and the player is polled,
    // with its play queue due a re-read, before the edit has been sent.
    edit(&r.remote, || r.local.queue(&["x"], false)).await.unwrap();
    let r2 = &r;
    r.remote
        .edit_queue(|| async move {
            r2.local.queue(&["y"], false);
            r2.remote.recheck_play_queue();
            let polls = requests(&r2.player, "/player/timeline/poll").await.len();
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while requests(&r2.player, "/player/timeline/poll").await.len() < polls + 1 {
                assert!(std::time::Instant::now() < deadline, "the player was never polled");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            // Room for that poll to take the player's queue over this one,
            // which it must not do while the edit is on its way.
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(fq.keys(), strs(&["a", "x", "y", "b", "c"]));
    assert_eq!(r.local.now(), ("a".into(), strs(&["x", "y"]), strs(&["b", "c"])));
}

#[tokio::test]
async fn edits_when_nothing_plays_elsewhere_do_nothing() {
    let r = rig().await;
    r.remote.queue_edited().await.unwrap();
    assert!(r.server.received_requests().await.unwrap().is_empty());
}

// ---- Cast and AirPlay ---------------------------------------------------------

mod lan_devices {
    use super::*;
    use crate::audio::netsink::NetRoute;
    use crate::plex::keychain::{MemoryStore, TokenStore};
    use crate::remote::airplay::fake::{FakeOptions, FakeRaop};
    use crate::remote::airplay::{password_key, rtsp};
    use crate::remote::cast::fake::FakeCast;
    use crate::remote::cast::proto::{NS_PLEX, NS_RECEIVER};
    use crate::remote::lan::{CAST_SERVICE, MdnsAnswer, RAOP_SERVICE, StaticMdns};
    use crate::remote::types::PlayerVia;
    use crate::remote::LanEnv;
    use std::sync::atomic::Ordering;

    struct Lan {
        r: Rig,
        mdns: Arc<StaticMdns>,
        passwords: Arc<MemoryStore>,
    }

    fn at(service: &str, instance: &str, port: u16, txt: &[(&str, &str)]) -> MdnsAnswer {
        crate::remote::lan::answer(service, instance, "127.0.0.1", port, txt)
    }

    async fn lan(answers: Vec<MdnsAnswer>) -> Lan {
        let r = rig().await;
        Mock::given(method("GET"))
            .and(path("/security/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"token": "transient-1"}})))
            .mount(&r.server)
            .await;
        let mdns = Arc::new(StaticMdns::default());
        *mdns.answers.lock() = answers;
        let passwords = Arc::new(MemoryStore::new());
        let route = NetRoute::default();
        *r.local.route.lock() = Some(route.clone());
        r.remote.set_lan(LanEnv {
            mdns: mdns.clone(),
            route,
            passwords: passwords.clone(),
            tls: crate::remote::cast::conn::tls_config().unwrap(),
        });
        // These tests cover the password prompt, so locked speakers are listed.
        r.switches.show_airplay_locked.store(true, Ordering::SeqCst);
        Lan { r, mdns, passwords }
    }

    fn cast_answer(fake: &FakeCast) -> MdnsAnswer {
        at(CAST_SERVICE, "Chromecast-c1", fake.addr.port(), &[("id", "c1"), ("fn", "Den TV"), ("md", "Chromecast"), ("ca", "201221")])
    }

    #[tokio::test]
    async fn cast_devices_list_with_plex_players_and_hide_with_their_setting() {
        let l = lan(vec![
            at(CAST_SERVICE, "g", 9, &[("id", "g1"), ("fn", "Everywhere"), ("md", "Google Cast Group")]),
            at(RAOP_SERVICE, "AABB@Kitchen", 9, &[("et", "0"), ("pw", "true")]),
            at(RAOP_SERVICE, "CCDD@Old", 9, &[("et", "1")]),
        ])
        .await;
        let list = l.r.remote.players().await;
        let ids: Vec<_> = list.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["amp", "cast:g1", "airplay:AABB", "airplay:CCDD"]);
        assert_eq!(list[1].via, PlayerVia::Cast);
        assert!(list[2].needs_password);
        assert!(list[3].unsupported.is_some());
        // Off: no devices, and mDNS stops.
        l.r.switches.set_show_cast(false);
        let list = l.r.remote.players().await;
        assert_eq!(list.len(), 1);
        assert!(l.mdns.stopped.load(Ordering::SeqCst));
        let scans = l.mdns.scans.load(Ordering::SeqCst);
        l.r.remote.players().await;
        assert_eq!(l.mdns.scans.load(Ordering::SeqCst), scans, "no scanning while off");
        // Both kinds off: nothing at all.
        l.r.switches.set(false, true, false);
        assert!(l.r.remote.players().await.is_empty());
    }

    #[tokio::test]
    async fn moving_to_a_cast_device_follows_it_and_comes_back() {
        let fake = FakeCast::start().await;
        let l = lan(vec![cast_answer(&fake)]).await;
        l.r.remote.players().await;
        let st = l.r.remote.move_to("cast:c1").await.unwrap();
        let s = st.session.unwrap();
        assert_eq!((s.player_name.as_str(), s.status), ("Den TV", RemoteStatus::Connecting));
        let load = fake.state.lock().plex_load.clone().unwrap();
        assert_eq!(load["media"]["customData"]["containerKey"], "/playQueues/55?own=1&window=200");
        assert_eq!(load["media"]["customData"]["server"]["accessToken"], "transient-1", "the transient token, not the server's");
        assert_eq!(load["currentTime"], 61.0);
        assert!(l.r.local.log().contains(&"pause".to_string()));

        wait_for("playing", || l.r.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Playing)).await;
        let s = l.r.remote.state().session.unwrap();
        assert_eq!((s.position_ms, s.volume), (61_000, Some(0.4)));

        l.r.remote.command(RemoteCommand::Pause).await.unwrap();
        l.r.remote.command(RemoteCommand::Volume { volume: 0.25 }).await.unwrap();
        wait_for("pause sent", || fake.types(NS_PLEX) == ["PAUSE"]).await;
        wait_for("paused", || l.r.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Paused && s.volume == Some(0.25))).await;

        let st = l.r.remote.return_here().await.unwrap();
        assert!(st.session.is_none());
        assert_eq!(fake.types(NS_RECEIVER).last().map(String::as_str), Some("STOP"));
        let log = l.r.local.log();
        assert!(log.contains(&"seek 61000".to_string()), "{log:?}");
    }

    #[tokio::test]
    async fn a_cast_device_stopped_elsewhere_hands_back_control() {
        let fake = FakeCast::start().await;
        let l = lan(vec![cast_answer(&fake)]).await;
        l.r.remote.players().await;
        l.r.remote.move_to("cast:c1").await.unwrap();
        wait_for("playing", || l.r.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Playing)).await;
        fake.stop_elsewhere();
        wait_for("stopped", || l.r.remote.state().stopped_on.as_deref() == Some("Den TV")).await;
        assert!(l.r.remote.state().session.is_none());
    }

    #[tokio::test]
    async fn a_cast_device_that_goes_away_is_reported() {
        let fake = FakeCast::start().await;
        let l = lan(vec![cast_answer(&fake)]).await;
        l.r.remote.players().await;
        l.r.remote.move_to("cast:c1").await.unwrap();
        fake.vanish();
        wait_for("gone", || l.r.remote.state().session.is_none()).await;
        assert_eq!(l.r.events.toasts(), vec!["Couldn't reach Den TV".to_string()]);
    }

    #[tokio::test]
    async fn a_cast_device_that_refuses_leaves_playback_here() {
        let fake = FakeCast::start().await;
        fake.state.lock().refuse_load = true;
        let l = lan(vec![cast_answer(&fake)]).await;
        l.r.remote.players().await;
        let e = l.r.remote.move_to("cast:c1").await.unwrap_err();
        assert_eq!(e.to_string(), "network: Couldn't reach Den TV");
        assert!(!l.r.local.log().contains(&"pause".to_string()));
        assert!(l.r.remote.state().session.is_none());
    }

    #[tokio::test]
    async fn airplay_moves_the_sound_and_controls_flush_the_receiver() {
        let fake = FakeRaop::start(FakeOptions::default());
        let l = lan(vec![at(RAOP_SERVICE, "AABB@Kitchen", fake.addr.port(), &[("et", "0,1"), ("cn", "0,1")])]).await;
        l.r.remote.players().await;
        l.r.remote.move_to("airplay:AABB").await.unwrap();
        assert_eq!(l.r.local.output.lock().as_deref(), Some("net:airplay:AABB"));
        assert!(fake.methods().contains(&"RECORD".to_string()));
        let log = l.r.local.log();
        assert!(log.contains(&"volume 1".to_string()), "full volume here; the receiver sets the level: {log:?}");
        assert!(!log.contains(&"pause".to_string()), "the sound keeps coming from here");
        wait_for("playing", || l.r.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Playing)).await;
        // Nothing is heard for the delay yet: the position holds.
        assert_eq!(l.r.remote.state().session.unwrap().position_ms, 61_000);

        l.r.remote.command(RemoteCommand::Seek { position_ms: 90_000 }).await.unwrap();
        assert!(fake.methods().contains(&"FLUSH".to_string()));
        l.r.remote.command(RemoteCommand::Volume { volume: 0.5 }).await.unwrap();
        assert_eq!(fake.last("SET_PARAMETER").unwrap().body, "volume: -15.000000\r\n");
        l.r.remote.command(RemoteCommand::Pause).await.unwrap();
        // Paused at what was heard: 90 s minus the receiver's delay.
        assert!(l.r.local.log().iter().any(|e| e == "seek 87750"), "{:?}", l.r.local.log());

        l.r.remote.return_here().await.unwrap();
        assert_eq!(*l.r.local.output.lock(), None, "back on this computer's output");
        assert!(l.r.local.log().contains(&"volume 0.5".to_string()));
        assert_eq!(fake.methods().last().map(String::as_str), Some("TEARDOWN"));
    }

    #[tokio::test]
    async fn airplay_asks_for_a_password_and_keeps_it() {
        let fake = FakeRaop::start(FakeOptions { password: Some("hunter2".into()), ..Default::default() });
        let l = lan(vec![at(RAOP_SERVICE, "AABB@Kitchen", fake.addr.port(), &[("pw", "true")])]).await;
        assert!(l.r.remote.players().await.iter().any(|p| p.needs_password));
        let e = l.r.remote.move_to("airplay:AABB").await.unwrap_err();
        assert!(matches!(&e, AppError::Auth(m) if m == rtsp::NEEDS_PASSWORD), "{e}");
        let e = l.r.remote.move_with_password("airplay:AABB", "nope".into()).await.unwrap_err();
        assert!(matches!(&e, AppError::Auth(m) if m == rtsp::WRONG_PASSWORD), "{e}");
        assert!(l.passwords.get(&password_key("airplay:AABB")).unwrap().is_none(), "a wrong password isn't kept");
        l.r.remote.move_with_password("airplay:AABB", "hunter2".into()).await.unwrap();
        assert_eq!(l.passwords.get(&password_key("airplay:AABB")).unwrap().unwrap().expose(), "hunter2");
        l.r.remote.return_here().await.unwrap();
        // Next time the stored password is used without asking.
        assert!(!l.r.remote.players().await.iter().any(|p| p.needs_password));
        l.r.remote.move_to("airplay:AABB").await.unwrap();
        assert!(l.r.remote.state().session.is_some());
    }

    #[tokio::test]
    async fn an_airplay_receiver_that_goes_away_hands_back_to_this_computer() {
        let fake = FakeRaop::start(FakeOptions::default());
        let l = lan(vec![at(RAOP_SERVICE, "AABB@Kitchen", fake.addr.port(), &[])]).await;
        l.r.remote.players().await;
        l.r.remote.move_to("airplay:AABB").await.unwrap();
        wait_for("following", || l.r.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Playing)).await;
        // The engine's output broke and fell back to this computer.
        *l.r.local.output.lock() = None;
        wait_for("handed back", || l.r.remote.state().stopped_on.as_deref() == Some("Kitchen")).await;
        assert!(l.r.local.log().contains(&"volume 0.5".to_string()));
        assert!(!l.r.local.log().iter().any(|e| e.starts_with("jump") || e.starts_with("play ")), "playback here is already right");
    }

    #[tokio::test]
    async fn unsupported_and_unreachable_receivers_fail_cleanly() {
        let l = lan(vec![
            at(RAOP_SERVICE, "CCDD@Old", 9, &[("et", "1")]),
            at(RAOP_SERVICE, "EEFF@Gone", 1, &[]),
        ])
        .await;
        l.r.remote.players().await;
        let e = l.r.remote.move_to("airplay:CCDD").await.unwrap_err();
        assert!(e.to_string().contains("Can't play on Old"));
        let e = l.r.remote.move_to("airplay:EEFF").await.unwrap_err();
        assert_eq!(e.to_string(), "network: Couldn't reach Gone");
        assert_eq!(*l.r.local.output.lock(), None);
    }
}

// ---- Platter to Platter ----------------------------------------------------------

/// Two (or more) Platter instances in this process, on a pretend network,
/// listening on loopback only. Nothing reaches a real device.
mod peers {
    use super::*;
    use std::net::Ipv4Addr;
    use tokio::sync::watch;

    use crate::remote::peer::auth::{PeerKey, account_tag, nonce, now_ms, sign};
    use crate::remote::peer::client::{CANT_CHECK, NOT_SAME_ACCOUNT, PeerClient, PeerError, refusal};
    use crate::remote::peer::directory::{MemoryDirectory, MemoryLan};
    use crate::remote::peer::runtime::{self, RuntimeConfig};
    use crate::remote::peer::{self, PATH_HELLO, SERVICE};
    use crate::remote::types::{PlayerKind, PlayerVia};

    /// A Plex account as one instance sees it.
    struct Acct {
        base: Url,
        token: &'static str,
        id: &'static str,
        machine: &'static str,
    }

    #[async_trait]
    impl RemoteEnv for Acct {
        async fn server(&self) -> AppResult<ServerCtx> {
            Ok(ServerCtx {
                base: self.base.clone(),
                token: Token::new("server-token").unwrap(),
                machine_id: self.machine.into(),
            })
        }
        async fn account_token(&self) -> Option<Token> {
            Some(Token::new(self.token).unwrap())
        }
        async fn account_id(&self) -> Option<String> {
            Some(self.id.into())
        }
    }

    struct Peer {
        remote: RemoteService,
        local: Arc<Local>,
        events: Arc<Events>,
        switches: RemoteSwitches,
        port: watch::Receiver<Option<u16>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for Peer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    struct Opts {
        id: &'static str,
        name: &'static str,
        token: &'static str,
        account: &'static str,
        machine: &'static str,
        publish_to: Option<Url>,
    }

    impl Opts {
        fn new(id: &'static str, name: &'static str) -> Self {
            Self { id, name, token: "account-token", account: "acct-1", machine: "srv", publish_to: None }
        }
    }

    async fn start(server: &MockServer, lan: &Arc<MemoryLan>, local: Arc<Local>, o: Opts) -> Peer {
        let events = Arc::new(Events::default());
        let switches = RemoteSwitches::default();
        switches.set_peer(true, o.name, false);
        let env = Arc::new(Acct { base: Url::parse(&server.uri()).unwrap(), token: o.token, id: o.account, machine: o.machine });
        let remote = RemoteService::new(env, local.clone(), events.clone(), &ClientIdentity::with_id(o.id), switches.clone(), cfg(server)).unwrap();
        let dir = Arc::new(MemoryDirectory::new(lan.clone()));
        remote.set_peers(dir.clone());
        let (tx, mut port) = watch::channel(None);
        let rc = RuntimeConfig {
            first_port: 0,
            tries: 1,
            recheck: Duration::from_millis(50),
            publish_to: o.publish_to,
            platform: "Windows".into(),
            version: "0.3.0".into(),
        };
        let task = tokio::spawn(runtime::run(remote.clone(), dir, Arc::new(|| vec![Ipv4Addr::LOCALHOST]), rc, tx));
        tokio::time::timeout(Duration::from_secs(5), port.wait_for(Option::is_some)).await.unwrap().unwrap();
        Peer { remote, local, events, switches, port, task }
    }

    impl Peer {
        fn port(&self) -> u16 {
            self.port.borrow().unwrap()
        }
        fn base(&self) -> Url {
            Url::parse(&format!("http://127.0.0.1:{}", self.port())).unwrap()
        }
    }

    /// A Plex server and a phone-like Plex player, like the other tests.
    async fn net() -> (Rig, Arc<MemoryLan>) {
        (rig().await, MemoryLan::new())
    }

    fn idle() -> Arc<Local> {
        let l = Local::playing("z", &[], 0);
        l.state.lock().status = PlayerStatus::Paused;
        l
    }

    #[tokio::test]
    async fn platter_devices_of_the_same_account_list_first() {
        let (r, lan) = net().await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let b = start(&r.server, &lan, idle(), Opts::new("mac", "Studio Mac")).await;
        // Another account's Platter on the same network.
        lan.insert(crate::remote::lan::answer(
            SERVICE,
            "other",
            "127.0.0.1",
            b.port(),
            &[("id", "other"), ("name", "Neighbour"), ("acct", &account_tag("acct-2"))],
        ));
        let list = a.remote.players().await;
        let ids: Vec<_> = list.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["mac", "amp"], "own device first, foreign and self left out");
        let mac = &list[0];
        assert_eq!((mac.name.as_str(), mac.kind, mac.via), ("Studio Mac", PlayerKind::Computer, PlayerVia::Platter));
        assert_eq!(mac.product, "Platter");
        assert_eq!(mac.playing.as_deref(), Some("Paused: Track z • A"));
        assert!(mac.online && !mac.unreachable && mac.unsupported.is_none());
        // And the other way round.
        let list = b.remote.players().await;
        assert_eq!(list[0].name, "Studio PC");
        assert!(list[0].playing.as_deref().is_some_and(|p| p.starts_with("Track a •")));
    }

    #[tokio::test]
    async fn the_name_setting_and_the_switch_reach_the_announcement() {
        let (r, lan) = net().await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let b = start(&r.server, &lan, idle(), Opts::new("mac", "Studio Mac")).await;
        b.switches.set_peer(true, "Couch Mac", false);
        wait_for("renamed", || {
            lan.answers()
                .iter()
                .any(|x| x.txt.get("name").map(String::as_str) == Some("Couch Mac"))
        })
        .await;
        assert_eq!(a.remote.players().await[0].name, "Couch Mac");
        // Off: it leaves the network and stops answering.
        let port = b.port();
        b.switches.set_peer(false, "Couch Mac", false);
        let mut rx = b.port.clone();
        tokio::time::timeout(Duration::from_secs(5), rx.wait_for(Option::is_none)).await.unwrap().unwrap();
        assert_eq!(lan.len(), 1, "only the PC is announced");
        assert!(a.remote.players().await.iter().all(|p| p.id != "mac"));
        let gone = reqwest::Client::new().get(format!("http://127.0.0.1:{port}{PATH_HELLO}")).send().await;
        assert!(gone.is_err(), "the control port is closed");
    }

    #[tokio::test]
    async fn another_platter_takes_tagged_ids_and_tags_plain_ones() {
        let (one, two) = (tagged("1"), tagged("2"));
        let r = rig_with(Local::playing(&one, &[&two], 3_000), json!([])).await;
        let lan = MemoryLan::new();
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let b = start(&r.server, &lan, idle(), Opts::new("mac", "Studio Mac")).await;
        a.remote.players().await;
        a.remote.move_to("mac").await.unwrap();
        assert_eq!(b.local.log(), [format!("adopt {one},{two} |  | shuffle false Off at 3000 true")]);

        // A sender from before ids carried their server sends plain ids.
        let http = reqwest::Client::new();
        let key = PeerKey::from_token(&Token::new("account-token").unwrap());
        let req = peer::PlayRequest {
            server_id: "srv".into(),
            current: "4".into(),
            user_queue: vec![],
            next_up: vec!["5".into()],
            context: None,
            position_ms: 0,
            shuffle: false,
            repeat: Repeat::Off,
            play: true,
        };
        let body = serde_json::to_vec(&req).unwrap();
        let auth = sign(&key, "POST", "/platter/v1/play", "pc", "mac", now_ms(), &nonce(), &body);
        let resp = http.post(format!("{}platter/v1/play", b.base())).header("x-platter-auth", auth).body(body).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let want = format!("adopt {},{} |  | shuffle false Off at 0 true", tagged("4"), tagged("5"));
        assert_eq!(b.local.log().last(), Some(&want));
    }

    #[tokio::test]
    async fn moving_to_another_platter_and_back() {
        let (r, lan) = net().await;
        {
            let mut st = r.local.state.lock();
            st.queue.user_queue = vec![item("u")];
            st.repeat = Repeat::All;
        }
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let b = start(&r.server, &lan, idle(), Opts::new("mac", "Studio Mac")).await;
        a.remote.players().await;
        let st = a.remote.move_to("mac").await.unwrap();
        let s = st.session.unwrap();
        assert_eq!((s.player_name.as_str(), s.kind, s.status), ("Studio Mac", PlayerKind::Computer, RemoteStatus::Connecting));
        // The Mac took the same queue at the same spot, and the PC paused.
        assert_eq!(b.local.log(), ["adopt a,b,c | u | shuffle false All at 61000 true"]);
        assert_eq!(a.local.log(), ["pause"]);
        assert!(requests(&r.server, "/playQueues").await.is_empty(), "no server play queue between Platters");

        wait_for("following", || a.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Playing)).await;
        let s = a.remote.state().session.unwrap();
        assert_eq!((s.position_ms, s.repeat, s.volume), (61_000, Repeat::All, Some(0.5)));

        // Controls go to the Mac.
        a.remote.command(RemoteCommand::Seek { position_ms: 90_000 }).await.unwrap();
        a.remote.command(RemoteCommand::Pause).await.unwrap();
        assert_eq!(&b.local.log()[1..], ["seek 90000", "pause"]);
        wait_for("paused there", || a.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Paused && s.position_ms == 90_000)).await;

        // Bring it back: the Mac stops, the PC carries on from its spot, paused like the Mac.
        let st = a.remote.return_here().await.unwrap();
        assert!(st.session.is_none());
        assert_eq!(b.local.log().last().map(String::as_str), Some("pause"));
        let log = a.local.log();
        assert!(log.contains(&"seek 90000".to_string()), "{log:?}");
        assert_eq!(log.last().map(String::as_str), Some("pause"));
        assert!(a.events.toasts().is_empty());
    }

    #[tokio::test]
    async fn the_queue_follows_the_other_platter_and_edits_reach_it() {
        let (r, lan) = net().await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let b = start(&r.server, &lan, idle(), Opts::new("mac", "Studio Mac")).await;
        a.remote.players().await;
        a.remote.move_to("mac").await.unwrap();
        // The Mac goes on to b by itself: the queue panel here shows c next.
        b.local.jump_at("qb".into(), 0, true).await.unwrap();
        wait_for("on b here", || a.local.now().0 == "b").await;
        assert_eq!(a.local.now(), ("b".into(), vec![], strs(&["c"])));
        assert_eq!(a.local.state.lock().status, PlayerStatus::Paused);
        // Add to queue here: the Mac gets the queue with it, on the same song.
        edit(&a.remote, || { a.local.queue(&["x"], false); }).await.unwrap();
        let last = b.local.log().last().cloned().unwrap_or_default();
        assert!(last.starts_with("adopt b,c | x | "), "{last}");
        assert_eq!(b.local.now(), ("b".into(), strs(&["x"]), strs(&["c"])));
        assert!(a.remote.state().session.is_some());
    }

    #[tokio::test]
    async fn the_other_platter_stopping_or_vanishing_hands_back() {
        let (r, lan) = net().await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let b = start(&r.server, &lan, idle(), Opts::new("mac", "Studio Mac")).await;
        a.remote.players().await;
        a.remote.move_to("mac").await.unwrap();
        wait_for("following", || a.remote.state().session.is_some_and(|s| s.status == RemoteStatus::Playing)).await;
        // The Mac quits: the PC gets control back, with a toast, paused.
        drop(b);
        wait_for("handed back", || a.remote.state().session.is_none()).await;
        assert_eq!(a.events.toasts(), ["Couldn't reach Studio Mac"]);
        assert_eq!(a.local.state.lock().status, PlayerStatus::Paused);
    }

    #[tokio::test]
    async fn requests_must_be_signed_by_the_same_account() {
        let (r, lan) = net().await;
        device_list(&r.server, "account-token", &[("pc", "pc-token"), ("mac", "account-token")]).await;
        let b = start(&r.server, &lan, idle(), Opts::new("mac", "Studio Mac")).await;
        let http = reqwest::Client::new();
        let url = format!("{}platter/v1/play", b.base());
        // Unsigned.
        let resp = http.post(&url).body("{}").send().await.unwrap();
        assert_eq!(resp.status(), 401);
        // Another account's key.
        let theirs = PeerClient::new(http.clone(), "x".into(), PeerKey::from_token(&Token::new("not-yours").unwrap()));
        assert_eq!(
            theirs.hello(&b.base(), "mac", Duration::from_secs(2)).await,
            Err(PeerError::Refused(NOT_SAME_ACCOUNT.into()))
        );
        let req = peer::PlayRequest {
            server_id: "srv".into(),
            current: "a".into(),
            user_queue: vec![],
            next_up: vec![],
            context: None,
            position_ms: 0,
            shuffle: false,
            repeat: Repeat::Off,
            play: true,
        };
        assert!(theirs.play(&b.base(), "mac", &req, Duration::from_secs(2)).await.is_err());
        // A good request replayed.
        let key = PeerKey::from_token(&Token::new("account-token").unwrap());
        let body = serde_json::to_vec(&req).unwrap();
        let auth = sign(&key, "POST", "/platter/v1/play", "pc", "mac", now_ms(), &nonce(), &body);
        let send = || http.post(&url).header("x-platter-auth", auth.clone()).body(body.clone()).send();
        assert_eq!(send().await.unwrap().status(), 200);
        assert_eq!(send().await.unwrap().status(), 401, "replayed");
        // Signed for another device.
        let other = sign(&key, "GET", PATH_HELLO, "pc", "someone-else", now_ms(), &nonce(), b"");
        let resp = http.get(format!("{}platter/v1/hello", b.base())).header("x-platter-auth", other).send().await.unwrap();
        assert_eq!(resp.status(), 401);
        assert_eq!(b.local.log().len(), 1, "only the good request did anything");
    }

    /// plex.tv's device list as the holder of `token` sees it: `(device id,
    /// that device's token)`.
    async fn device_list(server: &MockServer, token: &str, devices: &[(&str, &str)]) {
        let body: Vec<serde_json::Value> = devices
            .iter()
            .map(|(id, t)| json!({"clientIdentifier": id, "product": "Platter", "provides": "player", "token": t}))
            .collect();
        Mock::given(method("GET"))
            .and(path("/api/v2/devices"))
            .and(header("x-plex-token", token))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn two_devices_with_their_own_tokens_trust_each_other() {
        let (r, lan) = net().await;
        // Every device gets its own token when it signs in.
        let both = [("pc", "account-token"), ("mac", "mac-token")];
        device_list(&r.server, "account-token", &both).await;
        device_list(&r.server, "mac-token", &both).await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let b = start(&r.server, &lan, idle(), Opts { token: "mac-token", ..Opts::new("mac", "Studio Mac") }).await;
        let list = a.remote.players().await;
        assert_eq!(list[0].id, "mac");
        assert_eq!(list[0].unsupported, None);
        a.remote.move_to("mac").await.unwrap();
        wait_for("playing there", || b.local.log().iter().any(|l| l.starts_with("adopt") || l.starts_with("play"))).await;
        // And the other way: the Mac's requests to the PC check out too.
        let mac = b.remote.peer_client().await.unwrap();
        let hello = mac.hello(&a.base(), "pc", Duration::from_secs(2)).await.unwrap();
        assert_eq!(hello.name, "Studio PC");
        // Nothing but keys and signatures crossed: no request carried a token
        // other than to plex.tv.
        for req in r.server.received_requests().await.unwrap() {
            if let Some(t) = req.headers.get("x-plex-token") {
                assert!(["account-token", "mac-token", "server-token"].contains(&t.to_str().unwrap()));
            }
        }
    }

    #[tokio::test]
    async fn another_account_or_home_user_is_listed_as_refused() {
        let (r, lan) = net().await;
        // The PC's account lists the Mac with the token the account holder
        // signed in with there, but the Mac now runs as another Home user
        // (same account tag at worst) with its own token.
        device_list(&r.server, "account-token", &[("pc", "account-token"), ("mac", "mac-admin-token")]).await;
        device_list(&r.server, "home-user-token", &[("mac", "home-user-token")]).await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let b = start(&r.server, &lan, idle(), Opts { token: "home-user-token", ..Opts::new("mac", "Studio Mac") }).await;
        let list = a.remote.players().await;
        assert_eq!(list[0].unsupported.as_deref(), Some(NOT_SAME_ACCOUNT));
        let e = a.remote.move_to("mac").await.unwrap_err();
        assert!(matches!(e, AppError::Auth(_)), "{e}");
        assert_eq!(e.to_string().split_once(": ").map(|x| x.1), Some(format!("Couldn't play on Studio Mac. {NOT_SAME_ACCOUNT}.").as_str()));
        assert!(a.local.log().is_empty(), "nothing paused here");
        assert!(b.local.log().is_empty(), "nothing played there");
        // The other way: the Home user's Mac doesn't list the PC at all.
        let mac = b.remote.peer_client().await.unwrap();
        assert_eq!(mac.hello(&a.base(), "pc", Duration::from_secs(2)).await, Err(PeerError::Refused(NOT_SAME_ACCOUNT.into())));
        // A device of another account entirely.
        let stranger = PeerClient::new(reqwest::Client::new(), "stranger".into(), PeerKey::from_token(&Token::new("their-token").unwrap()));
        assert_eq!(stranger.hello(&b.base(), "mac", Duration::from_secs(2)).await, Err(PeerError::Refused(NOT_SAME_ACCOUNT.into())));
    }

    #[tokio::test]
    async fn without_the_device_list_another_token_cant_be_checked() {
        let (r, lan) = net().await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let _b = start(&r.server, &lan, idle(), Opts { token: "mac-token", ..Opts::new("mac", "Studio Mac") }).await;
        // plex.tv has no device list for the Mac here (it's down).
        let list = a.remote.players().await;
        assert_eq!(list[0].unsupported.as_deref(), Some(CANT_CHECK));
    }

    #[tokio::test]
    async fn a_clock_far_off_is_corrected_once_or_named() {
        let (r, lan) = net().await;
        let b = start(&r.server, &lan, idle(), Opts::new("mac", "Studio Mac")).await;
        let key = PeerKey::from_token(&Token::new("account-token").unwrap());
        // This computer's clock is 7 minutes slow: the Mac says so, and the
        // request goes again with the Mac's time.
        let slow = PeerClient::new(reqwest::Client::new(), "pc".into(), key.clone()).with_clock(Arc::new(|| now_ms() - 7 * 60_000));
        let hello = slow.hello(&b.base(), "mac", Duration::from_secs(2)).await.unwrap();
        assert_eq!(hello.id, "mac");
        // A refusal for the time names it.
        let refused = format!(r#"{{"error":"clock","message":"x","auth":2,"now":{}}}"#, 1_000_000 + 7 * 60_000);
        assert_eq!(refusal(&refused, 1_000_000), "Its clock is 7 minutes ahead of this computer's. Set both to the right time");
    }

    #[tokio::test]
    async fn a_platter_on_another_server_says_so() {
        let (r, lan) = net().await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let b = start(&r.server, &lan, idle(), Opts { machine: "srv-2", ..Opts::new("mac", "Studio Mac") }).await;
        a.remote.players().await;
        let e = a.remote.move_to("mac").await.unwrap_err();
        assert_eq!(e.to_string(), "invalid: That computer plays from a different Plex server.");
        assert!(b.local.log().is_empty() && a.local.log().is_empty());
    }

    /// Seen on the network but its port doesn't answer: what a firewall does.
    #[tokio::test]
    async fn an_unreachable_platter_is_marked_and_a_move_stays_here() {
        let (r, lan) = net().await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        lan.insert(crate::remote::lan::answer(
            SERVICE,
            "mac",
            "127.0.0.1",
            1,
            &[("id", "mac"), ("name", "Studio Mac"), ("acct", &account_tag("acct-1"))],
        ));
        let list = a.remote.players().await;
        let mac = list.iter().find(|p| p.id == "mac").unwrap();
        assert!(mac.unreachable, "listed, but can't be reached");
        assert_eq!(list.last().map(|p| p.id.as_str()), Some("amp"));
        let e = a.remote.move_to("mac").await.unwrap_err();
        assert_eq!(e.to_string(), "network: Couldn't reach Studio Mac");
        assert!(a.local.log().is_empty(), "playback here carries on");
        assert!(a.remote.state().session.is_none());
    }

    /// Plex knows the other Platter (it registered with the server as a
    /// player) but it can't be reached directly: the move goes through Plex.
    #[tokio::test]
    async fn not_on_this_network_falls_back_to_the_plex_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/clients"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"size": 1, "Server": [
                {"name": "Studio Mac", "address": "127.0.0.1", "port": 1, "machineIdentifier": "mac",
                 "product": "Platter", "protocolCapabilities": "timeline,playback,playqueues"}
            ]}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/playQueues"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"playQueueID": 55}})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex_playback())
            .and(header("x-plex-target-client-identifier", "mac"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<Response code=\"200\" status=\"OK\"/>"))
            .mount(&server)
            .await;
        let lan = MemoryLan::new();
        let local = Local::playing("a", &["b"], 5_000);
        let a = start(&server, &lan, local.clone(), Opts::new("pc", "Studio PC")).await;
        let list = a.remote.players().await;
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].via, list[0].unreachable), (PlayerVia::Platter, true));
        let st = a.remote.move_to("mac").await.unwrap();
        assert_eq!(st.session.unwrap().player_name, "Studio Mac");
        let pm = requests(&server, "/player/playback/playMedia").await;
        assert_eq!(pm.len(), 1, "sent through the server");
        assert_eq!(q(&pm[0])["offset"], "5000");
        assert_eq!(local.log(), ["pause"]);
    }

    #[tokio::test]
    async fn platters_plex_lists_count_only_when_recently_seen() {
        let r = rig().await;
        let lan = MemoryLan::new();
        let now = (now_ms() / 1000) as i64;
        Mock::given(method("GET"))
            .and(path("/api/v2/resources"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"name": "Old Laptop", "product": "Platter", "provides": "player", "clientIdentifier": "old",
                 "presence": false, "lastSeenAt": now - 3 * 86_400,
                 "connections": [{"protocol": "http", "address": "127.0.0.1", "port": 1, "uri": "http://127.0.0.1:1", "local": true}]},
                {"name": "Studio Mac", "product": "Platter", "provides": "player", "clientIdentifier": "mac",
                 "presence": false, "lastSeenAt": now - 60, "connections": []}
            ])))
            .mount(&r.server)
            .await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let list = a.remote.players().await;
        let ids: Vec<_> = list.iter().map(|p| (p.id.as_str(), p.unreachable)).collect();
        assert_eq!(ids, [("mac", true), ("amp", false)], "the laptop that's been off for days isn't listed");
    }

    #[tokio::test]
    async fn the_addresses_are_published_to_plex() {
        let r = rig().await;
        Mock::given(method("PUT"))
            .and(path("/devices/pc"))
            .and(header("x-plex-token", "account-token"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&r.server)
            .await;
        let lan = MemoryLan::new();
        let a = start(
            &r.server,
            &lan,
            r.local.clone(),
            Opts { publish_to: Some(Url::parse(&r.server.uri()).unwrap()), ..Opts::new("pc", "Studio PC") },
        )
        .await;
        let port = a.port();
        let mut puts = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            puts = r.server.received_requests().await.unwrap_or_default().into_iter().filter(|x| x.method.as_str() == "PUT").collect();
            if !puts.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(puts.len(), 1);
        assert_eq!(q(&puts[0])["Connection[][uri]"], format!("http://127.0.0.1:{port}"));
    }

    #[tokio::test]
    async fn a_platter_hides_its_own_airplay_receiver() {
        use crate::plex::keychain::MemoryStore;
        use crate::remote::LanEnv;
        use crate::remote::lan::{RAOP_SERVICE, StaticMdns};
        let (r, lan) = net().await;
        let a = start(&r.server, &lan, r.local.clone(), Opts::new("pc", "Studio PC")).await;
        let _b = start(&r.server, &lan, idle(), Opts::new("mac", "Studio Mac")).await;
        let mdns = Arc::new(StaticMdns::default());
        // A receiver at the Mac's address that doesn't say it's a Mac.
        *mdns.answers.lock() = vec![
            crate::remote::lan::answer(RAOP_SERVICE, "AA@Studio Mac", "127.0.0.1", 7000, &[]),
            crate::remote::lan::answer(RAOP_SERVICE, "BB@Kitchen", "192.0.2.40", 7000, &[]),
        ];
        a.remote.set_lan(LanEnv {
            mdns,
            route: Default::default(),
            passwords: Arc::new(MemoryStore::new()),
            tls: crate::remote::cast::conn::tls_config().unwrap(),
        });
        let ids: Vec<String> = a.remote.players().await.into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["mac", "amp", "airplay:BB"]);
    }
}
