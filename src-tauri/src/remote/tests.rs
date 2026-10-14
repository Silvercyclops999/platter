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
            }),
            log: Mutex::default(),
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
    async fn jump_at(&self, item_id: String, ms: u64, play: bool) -> AppResult<()> {
        self.push(format!("jump {item_id} {ms} {play}"));
        Ok(())
    }
    async fn play_tracks(&self, ids: Vec<String>, start: Option<String>, ms: u64, play: bool) -> AppResult<()> {
        self.push(format!("play {} from {} at {ms} {play}", ids.join(","), start.unwrap_or_default()));
        Ok(())
    }
    async fn tracks(&self, ids: &[String]) -> AppResult<Vec<Track>> {
        Ok(ids.iter().map(|i| track(i, "A", "/p")).collect())
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
        grace: Duration::from_millis(400),
        misses: 2,
        timeout: Duration::from_millis(800),
    }
}

/// A server listing one fake player ("Desk", id "amp") in `/clients`.
async fn rig() -> Rig {
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
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {"Metadata": [
            {"ratingKey": "a", "playQueueItemID": 1}, {"ratingKey": "b", "playQueueItemID": 2}, {"ratingKey": "z", "playQueueItemID": 3}
        ]}})))
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
    let local = Local::playing("a", &["b", "c"], 61_000);
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
    for _ in 0..200 {
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
    assert!(r.local.log().iter().any(|l| l.starts_with("jump qc 42") && l.ends_with("true")), "{:?}", r.local.log());
    assert_eq!(requests(&r.player, "/player/playback/stop").await.len(), 1);
}

#[tokio::test]
async fn return_after_the_player_moved_past_the_queue_takes_its_play_queue() {
    let r = rig().await;
    r.remote.move_to("amp").await.unwrap();
    r.tl.set(200, timeline("paused", Some("z"), 5_000));
    r.remote.return_here().await.unwrap();
    assert!(
        r.local.log().iter().any(|l| l == "play a,b,z from z at 5000 false"),
        "{:?}",
        r.local.log()
    );
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
    // "stopped" right after playMedia, inside the grace window.
    tokio::time::sleep(Duration::from_millis(150)).await;
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
    assert_eq!(local.log(), ["play a,b,z from b at 5000 true"]);

    let body = http.get(format!("{base}/player/timeline/poll?wait=0&commandID=2")).send().await.unwrap().text().await.unwrap();
    let t = crate::remote::companion::parse_timeline(&body).unwrap();
    assert_eq!((t.state.as_str(), t.time_ms, t.rating_key.as_deref()), ("playing", 1_000, Some("a")));
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
