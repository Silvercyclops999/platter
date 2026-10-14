//! The Listen on state machine.
//!
//! Move: create a server play queue from Platter's queue (current song
//! first), tell the player to play it at the current position, pause here,
//! then follow the player's timeline once a second. Commands from the player
//! bar go to the player while remote. Return: read the player's position,
//! stop it, and carry on here from that song and spot.
//!
//! A player that stops answering, or stops by itself (someone stopped it
//! there), hands control back: playback here is put on that song, paused at
//! the last known position.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::time::Instant;
use url::Url;

use super::RemoteSwitches;
use super::companion::{self, Companion, Route, Timeline};
use super::discovery::{self, Found};
use super::playqueue;
use super::types::{
    EV_REMOTE_PLAYERS, EV_REMOTE_STATE, RemoteCommand, RemotePlayer, RemoteSession, RemoteState, RemoteStatus,
    repeat_code, repeat_from_code,
};
use crate::audio::events::{PlayerState, PlayerStatus};
use crate::audio::queue::Repeat;
use crate::audio::source::PlayContext;
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::plex::Token;
use crate::plex::client::{check_status, map_send_error};
use crate::plex::identity::ClientIdentity;
use crate::plex::state::EventSink;
use crate::plex::types::{EVENT_TOAST, Toast};

/// The selected server, for play queues and proxied commands.
#[derive(Debug, Clone)]
pub struct ServerCtx {
    pub base: Url,
    pub token: Token,
    pub machine_id: String,
}

impl ServerCtx {
    /// Absolute URL for a server path (no token).
    pub fn url(&self, path: &str) -> AppResult<Url> {
        let mut u = self.base.clone();
        u.set_path(&format!("{}{}", self.base.path().trim_end_matches('/'), path));
        Ok(u)
    }
}

/// The outside world the service needs from Plex.
#[async_trait]
pub trait RemoteEnv: Send + Sync {
    async fn server(&self) -> AppResult<ServerCtx>;
    async fn account_token(&self) -> Option<Token>;
}

/// The local player, through its public methods.
#[async_trait]
pub trait LocalPlayer: Send + Sync {
    async fn state(&self) -> AppResult<PlayerState>;
    async fn pause(&self) -> AppResult<()>;
    async fn resume(&self) -> AppResult<()>;
    async fn seek(&self, ms: u64) -> AppResult<()>;
    async fn next(&self) -> AppResult<()>;
    async fn previous(&self) -> AppResult<()>;
    async fn set_volume(&self, v: f32) -> AppResult<()>;
    async fn set_shuffle(&self, on: bool) -> AppResult<()>;
    async fn set_repeat(&self, r: Repeat) -> AppResult<()>;
    /// Jump to a queue entry at a position, playing or paused.
    async fn jump_at(&self, item_id: String, ms: u64, play: bool) -> AppResult<()>;
    /// Play these songs from `start` at a position, playing or paused.
    async fn play_tracks(&self, ids: Vec<String>, start: Option<String>, ms: u64, play: bool) -> AppResult<()>;
    async fn tracks(&self, ids: &[String]) -> AppResult<Vec<Track>>;
    /// Load a context paused at its start (nothing sounds here).
    async fn load_context(&self, context: PlayContext, start_track_id: Option<String>) -> AppResult<()>;
}

#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub plex_tv: Url,
    /// The Sonos relay.
    pub relay: Url,
    /// Timeline poll interval while remote.
    pub poll: Duration,
    /// Player list refresh while remote and the picker is closed.
    pub refresh: Duration,
    /// A player that reports "stopped" this soon after a move is still
    /// starting, not stopped.
    pub grace: Duration,
    /// Failed polls in a row before the player counts as gone.
    pub misses: u32,
    /// Per-request timeout for player commands.
    pub timeout: Duration,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            plex_tv: Url::parse("https://plex.tv").expect("static url"),
            relay: Url::parse("https://sonos.plex.tv").expect("static url"),
            poll: Duration::from_secs(1),
            refresh: Duration::from_secs(60),
            grace: Duration::from_secs(8),
            misses: 3,
            timeout: Duration::from_secs(4),
        }
    }
}

#[derive(Clone)]
struct Active {
    found: Found,
    routes: Vec<Route>,
    session: RemoteSession,
    play_queue_id: Option<String>,
    started: Instant,
    seen_playing: bool,
    misses: u32,
}

struct Inner {
    env: Arc<dyn RemoteEnv>,
    local: Arc<dyn LocalPlayer>,
    sink: Arc<dyn EventSink>,
    switches: RemoteSwitches,
    cfg: RemoteConfig,
    own_id: String,
    http: reqwest::Client,
    companion: Companion,
    players: Mutex<Vec<Found>>,
    active: Mutex<Option<Active>>,
    stopped_on: Mutex<Option<String>>,
    /// Bumped on every move and return; a follow loop from an older one stops.
    generation: AtomicU64,
    /// Moves and returns one at a time.
    op: tokio::sync::Mutex<()>,
}

/// Cheap to clone; managed by Tauri.
#[derive(Clone)]
pub struct RemoteService {
    inner: Arc<Inner>,
}

pub fn couldnt_reach(name: &str) -> String {
    format!("Couldn't reach {name}")
}

fn status_of(state: &str) -> Option<RemoteStatus> {
    match state {
        "playing" => Some(RemoteStatus::Playing),
        "paused" => Some(RemoteStatus::Paused),
        "buffering" => Some(RemoteStatus::Buffering),
        _ => None,
    }
}

impl RemoteService {
    pub fn new(
        env: Arc<dyn RemoteEnv>,
        local: Arc<dyn LocalPlayer>,
        sink: Arc<dyn EventSink>,
        identity: &ClientIdentity,
        switches: RemoteSwitches,
        cfg: RemoteConfig,
    ) -> AppResult<Self> {
        let http = companion::http_client(identity, cfg.timeout)?;
        Ok(Self {
            inner: Arc::new(Inner {
                env,
                local,
                sink,
                switches,
                own_id: identity.client_id.clone(),
                companion: Companion::new(http.clone()),
                http,
                cfg,
                players: Mutex::default(),
                active: Mutex::default(),
                stopped_on: Mutex::default(),
                generation: AtomicU64::new(0),
                op: tokio::sync::Mutex::new(()),
            }),
        })
    }

    pub fn local(&self) -> Arc<dyn LocalPlayer> {
        self.inner.local.clone()
    }
    pub fn env(&self) -> Arc<dyn RemoteEnv> {
        self.inner.env.clone()
    }
    pub fn http(&self) -> reqwest::Client {
        self.inner.http.clone()
    }
    pub fn switches(&self) -> &RemoteSwitches {
        &self.inner.switches
    }

    fn emit<T: Serialize>(&self, event: &str, payload: &T) {
        let v = serde_json::to_value(payload).unwrap_or(serde_json::Value::Null);
        self.inner.sink.emit(event, v);
    }

    fn toast_error(&self, message: String) {
        self.emit(EVENT_TOAST, &Toast { kind: "error".into(), message });
    }

    pub fn state(&self) -> RemoteState {
        RemoteState {
            session: self.inner.active.lock().as_ref().map(|a| a.session.clone()),
            stopped_on: self.inner.stopped_on.lock().clone(),
        }
    }

    fn emit_state(&self) {
        let s = self.state();
        self.emit(EV_REMOTE_STATE, &s);
    }

    pub fn is_remote(&self) -> bool {
        self.inner.active.lock().is_some()
    }

    // ---- discovery ------------------------------------------------------

    async fn get_bytes(&self, url: Url, token: &Token) -> AppResult<Vec<u8>> {
        let resp = self
            .inner
            .http
            .get(url)
            .header("x-plex-token", token.header_value()?)
            .send()
            .await
            .map_err(map_send_error)?;
        Ok(check_status(resp).await?.bytes().await.map_err(map_send_error)?.to_vec())
    }

    async fn discover(&self) -> Vec<Found> {
        let server = self.inner.env.server().await.ok();
        let account = self.inner.env.account_token().await;
        let own = self.inner.own_id.as_str();
        let clients = async {
            let Some(s) = &server else { return Vec::new() };
            match s.url("/clients") {
                Ok(u) => match self.get_bytes(u, &s.token).await {
                    Ok(b) => discovery::from_clients(&b, own),
                    Err(e) => {
                        tracing::debug!(kind = e.kind(), "server clients unavailable");
                        Vec::new()
                    }
                },
                Err(_) => Vec::new(),
            }
        };
        let sessions = async {
            let Some(s) = &server else { return Default::default() };
            match s.url("/status/sessions") {
                Ok(u) => self
                    .get_bytes(u, &s.token)
                    .await
                    .map(|b| discovery::now_playing(&b))
                    .unwrap_or_default(),
                Err(_) => Default::default(),
            }
        };
        let resources = async {
            let Some(t) = &account else { return Vec::new() };
            let Ok(mut u) = self.inner.cfg.plex_tv.join("/api/v2/resources") else {
                return Vec::new();
            };
            u.set_query(Some("includeHttps=1&includeRelay=0"));
            match self.get_bytes(u, t).await {
                Ok(b) => discovery::from_resources(&b, own),
                Err(e) => {
                    tracing::debug!(kind = e.kind(), "plex.tv players unavailable");
                    Vec::new()
                }
            }
        };
        let sonos = async {
            let Some(t) = &account else { return Vec::new() };
            let Ok(u) = self.inner.cfg.relay.join("/resources") else {
                return Vec::new();
            };
            match self.get_bytes(u, t).await {
                Ok(b) => discovery::from_sonos(&String::from_utf8_lossy(&b)),
                // Most accounts have no Sonos linked.
                Err(_) => Vec::new(),
            }
        };
        let (clients, playing, resources, sonos) = tokio::join!(clients, sessions, resources, sonos);
        discovery::merge(vec![clients, resources, sonos], &playing)
    }

    /// Find players now (picker open, or the background refresh). Empty when
    /// the setting hides them.
    pub async fn players(&self) -> Vec<RemotePlayer> {
        if !self.inner.switches.show_players() {
            self.inner.players.lock().clear();
            return Vec::new();
        }
        let found = self.discover().await;
        let list: Vec<RemotePlayer> = found.iter().map(|f| f.player.clone()).collect();
        *self.inner.players.lock() = found;
        self.emit(EV_REMOTE_PLAYERS, &list);
        list
    }

    async fn routes_for(&self, f: &Found, server: Option<&ServerCtx>) -> Vec<Route> {
        let relay_token = if f.sonos { self.inner.env.account_token().await } else { None };
        companion::routes(
            f,
            server.map(|s| (&s.base, &s.token)),
            relay_token.as_ref().map(|t| (&self.inner.cfg.relay, t)),
        )
    }

    // ---- move -----------------------------------------------------------

    /// Move playback to a player. Fails with "Couldn't reach <device>" when
    /// it doesn't take the command; playback here then carries on.
    pub async fn move_to(&self, player_id: &str) -> AppResult<RemoteState> {
        let _op = self.inner.op.lock().await;
        if self.inner.active.lock().as_ref().is_some_and(|a| a.found.player.id == player_id) {
            return Ok(self.state());
        }
        // From one player to another: come back here first (paused), then go.
        if self.inner.active.lock().is_some() {
            self.return_inner(Some(false)).await;
        }
        let mut found = self.inner.players.lock().iter().find(|f| f.player.id == player_id).cloned();
        if found.is_none() {
            self.players().await;
            found = self.inner.players.lock().iter().find(|f| f.player.id == player_id).cloned();
        }
        let found = found.ok_or_else(|| AppError::NotFound("That device isn't available any more.".into()))?;
        self.start_on(found).await
    }

    /// Play something new while playing on another device: it's loaded here
    /// (paused, so nothing sounds here) and sent to the same player.
    pub async fn play_context(&self, context: PlayContext, start_track_id: Option<String>) -> AppResult<RemoteState> {
        let _op = self.inner.op.lock().await;
        let found = self
            .inner
            .active
            .lock()
            .as_ref()
            .map(|a| a.found.clone())
            .ok_or_else(|| AppError::Invalid("Nothing is playing on another device.".into()))?;
        self.inner.local.load_context(context, start_track_id).await?;
        // The old follow loop stops; the player gets the new queue.
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.active.lock().take();
        let r = self.start_on(found).await;
        if r.is_err() {
            self.emit_state();
        }
        r
    }

    /// Send Platter's queue to `found` and start following it.
    async fn start_on(&self, found: Found) -> AppResult<RemoteState> {
        let player_id = found.player.id.clone();
        let player_id = player_id.as_str();
        let name = found.player.name.clone();

        let st = self.inner.local.state().await?;
        let Some(track) = st.track.clone() else {
            return Err(AppError::Invalid("Play something first, then move it.".into()));
        };
        let server = self.inner.env.server().await?;
        let mut ids = vec![track.id.clone()];
        ids.extend(st.queue.user_queue.iter().map(|i| i.track.id.clone()));
        ids.extend(st.queue.next_up.iter().map(|i| i.track.id.clone()));
        let pq = playqueue::create(&self.inner.http, &server, &ids, &track.id, st.repeat).await?;
        let routes = self.routes_for(&found, Some(&server)).await;
        let host = server.base.host_str().unwrap_or_default().to_string();
        let port = server.base.port_or_known_default().unwrap_or(32400).to_string();
        let params = [
            ("providerIdentifier", playqueue::LIBRARY.to_string()),
            ("machineIdentifier", server.machine_id.clone()),
            ("protocol", server.base.scheme().to_string()),
            ("address", host),
            ("port", port),
            ("token", server.token.expose().to_string()),
            ("key", format!("/library/metadata/{}", track.id)),
            ("offset", st.position_ms.to_string()),
            ("containerKey", format!("/playQueues/{}?window=100&own=1", pq.id)),
            ("type", "music".to_string()),
        ];
        if let Err(e) = self
            .inner
            .companion
            .send(player_id, &routes, "/player/playback/playMedia", &params)
            .await
        {
            tracing::info!(kind = e.kind(), "player didn't take playMedia");
            return Err(AppError::Network(couldnt_reach(&name)));
        }
        if self.inner.switches.stop_old()
            && let Err(e) = self.inner.local.pause().await
        {
            tracing::warn!("pausing here after a move failed: {e}");
        }
        if st.repeat == Repeat::One {
            let _ = self
                .inner
                .companion
                .send(
                    player_id,
                    &routes,
                    "/player/playback/setParameters",
                    &[("repeat", repeat_code(Repeat::One).to_string()), ("type", "music".into())],
                )
                .await;
        }
        let session = RemoteSession {
            player_id: player_id.to_string(),
            player_name: name,
            kind: found.player.kind,
            status: RemoteStatus::Connecting,
            duration_ms: track.duration_ms,
            track: Some(track),
            position_ms: st.position_ms,
            volume: None,
            shuffle: st.shuffle,
            repeat: st.repeat,
        };
        *self.inner.active.lock() = Some(Active {
            found,
            routes,
            session,
            play_queue_id: Some(pq.id),
            started: Instant::now(),
            seen_playing: false,
            misses: 0,
        });
        *self.inner.stopped_on.lock() = None;
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.spawn_follow(generation);
        self.emit_state();
        Ok(self.state())
    }

    // ---- return ---------------------------------------------------------

    /// Back to this computer: read the player's position, stop it (unless
    /// the setting keeps it playing) and carry on here.
    pub async fn return_here(&self) -> AppResult<RemoteState> {
        let _op = self.inner.op.lock().await;
        *self.inner.stopped_on.lock() = None;
        self.return_inner(None).await;
        self.emit_state();
        Ok(self.state())
    }

    /// `play`: None keeps the player's play state, Some forces it.
    async fn return_inner(&self, play: Option<bool>) {
        let Some(mut a) = self.inner.active.lock().take() else {
            return;
        };
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        let id = a.found.player.id.clone();
        if let Ok(body) = self
            .inner
            .companion
            .send(&id, &a.routes, "/player/timeline/poll", &[("wait", "0".into())])
            .await
            && let Some(t) = companion::parse_timeline(&body)
            && t.state != "stopped"
        {
            self.apply_timeline(&mut a, &t).await;
        }
        if self.inner.switches.stop_old() {
            let _ = self
                .inner
                .companion
                .send(&id, &a.routes, "/player/playback/stop", &[("type", "music".into())])
                .await;
        }
        let playing = matches!(a.session.status, RemoteStatus::Playing | RemoteStatus::Buffering | RemoteStatus::Connecting);
        self.resume_local(&a, play.unwrap_or(playing)).await;
    }

    /// Put playback here on the player's song and position.
    async fn resume_local(&self, a: &Active, play: bool) {
        let local = &self.inner.local;
        let Some(key) = a.session.track.as_ref().map(|t| t.id.clone()) else {
            return;
        };
        let pos = a.session.position_ms;
        let r: AppResult<()> = async {
            let st = local.state().await?;
            if st.track.as_ref().is_some_and(|t| t.id == key) {
                local.seek(pos).await?;
                if play && !matches!(st.status, PlayerStatus::Playing | PlayerStatus::Loading | PlayerStatus::Buffering) {
                    local.resume().await?;
                } else if !play {
                    local.pause().await?;
                }
                return Ok(());
            }
            let q = &st.queue;
            if let Some(item) = q.user_queue.iter().chain(q.next_up.iter()).find(|i| i.track.id == key) {
                return local.jump_at(item.item_id.clone(), pos, play).await;
            }
            // The player moved on past Platter's queue: take its play queue.
            let mut ids = Vec::new();
            if let Some(pq) = &a.play_queue_id
                && let Ok(server) = self.inner.env.server().await
            {
                ids = playqueue::items(&self.inner.http, &server, pq).await.unwrap_or_default();
            }
            if !ids.contains(&key) {
                ids = vec![key.clone()];
            }
            local.play_tracks(ids, Some(key.clone()), pos, play).await
        }
        .await;
        if let Err(e) = r {
            tracing::warn!("resuming here failed: {e}");
        }
    }

    // ---- commands -------------------------------------------------------

    /// A player bar control while remote.
    pub async fn command(&self, cmd: RemoteCommand) -> AppResult<RemoteState> {
        let _op = self.inner.op.lock().await;
        let (id, name, routes) = {
            let g = self.inner.active.lock();
            let a = g
                .as_ref()
                .ok_or_else(|| AppError::Invalid("Nothing is playing on another device.".into()))?;
            (a.found.player.id.clone(), a.session.player_name.clone(), a.routes.clone())
        };
        let music = ("type", "music".to_string());
        let (path, params): (&str, Vec<(&str, String)>) = match &cmd {
            RemoteCommand::Play => ("/player/playback/play", vec![music]),
            RemoteCommand::Pause => ("/player/playback/pause", vec![music]),
            RemoteCommand::Next => ("/player/playback/skipNext", vec![music]),
            RemoteCommand::Previous => ("/player/playback/skipPrevious", vec![music]),
            RemoteCommand::Seek { position_ms } => ("/player/playback/seekTo", vec![("offset", position_ms.to_string()), music]),
            RemoteCommand::Volume { volume } => {
                let v = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 0.0 };
                ("/player/playback/setParameters", vec![("volume", ((v * 100.0).round() as i64).to_string()), music])
            }
            RemoteCommand::Shuffle { on } => ("/player/playback/setParameters", vec![("shuffle", u8::from(*on).to_string()), music]),
            RemoteCommand::Repeat { mode } => ("/player/playback/setParameters", vec![("repeat", repeat_code(*mode).to_string()), music]),
        };
        if let Err(e) = self.inner.companion.send(&id, &routes, path, &params).await {
            tracing::info!(kind = e.kind(), path, "player refused a command");
            return Err(AppError::Network(couldnt_reach(&name)));
        }
        if let Some(a) = self.inner.active.lock().as_mut().filter(|a| a.found.player.id == id) {
            let s = &mut a.session;
            match cmd {
                RemoteCommand::Play => s.status = RemoteStatus::Playing,
                RemoteCommand::Pause => s.status = RemoteStatus::Paused,
                RemoteCommand::Seek { position_ms } => s.position_ms = position_ms,
                RemoteCommand::Volume { volume } => s.volume = Some(volume.clamp(0.0, 1.0)),
                RemoteCommand::Shuffle { on } => s.shuffle = on,
                RemoteCommand::Repeat { mode } => s.repeat = mode,
                RemoteCommand::Next | RemoteCommand::Previous => s.position_ms = 0,
            }
        }
        self.emit_state();
        Ok(self.state())
    }

    // ---- following ------------------------------------------------------

    async fn apply_timeline(&self, a: &mut Active, t: &Timeline) {
        if let Some(st) = status_of(&t.state) {
            a.session.status = st;
            a.seen_playing = true;
        }
        a.session.position_ms = t.time_ms;
        if let Some(v) = t.volume {
            a.session.volume = Some((v.clamp(0, 100) as f32) / 100.0);
        }
        a.session.shuffle = t.shuffle;
        a.session.repeat = repeat_from_code(t.repeat);
        if let Some(pq) = &t.play_queue_id {
            a.play_queue_id = Some(pq.clone());
        }
        if let Some(key) = &t.rating_key
            && a.session.track.as_ref().is_none_or(|tr| &tr.id != key)
        {
            a.session.track = self
                .inner
                .local
                .tracks(std::slice::from_ref(key))
                .await
                .ok()
                .and_then(|v| v.into_iter().next());
        }
        a.session.duration_ms = if t.duration_ms > 0 {
            t.duration_ms
        } else {
            a.session.track.as_ref().map_or(0, |t| t.duration_ms)
        };
    }

    fn current(&self, generation: u64) -> bool {
        self.inner.generation.load(Ordering::SeqCst) == generation
    }

    fn spawn_follow(&self, generation: u64) {
        let me = self.clone();
        tokio::spawn(async move { me.follow(generation).await });
    }

    async fn follow(&self, generation: u64) {
        let mut last_refresh = Instant::now();
        loop {
            tokio::time::sleep(self.inner.cfg.poll).await;
            if !self.current(generation) {
                return;
            }
            if last_refresh.elapsed() >= self.inner.cfg.refresh {
                last_refresh = Instant::now();
                let me = self.clone();
                tokio::spawn(async move {
                    me.players().await;
                });
            }
            let Some((id, routes)) = self
                .inner
                .active
                .lock()
                .as_ref()
                .map(|a| (a.found.player.id.clone(), a.routes.clone()))
            else {
                return;
            };
            let polled = self
                .inner
                .companion
                .send(&id, &routes, "/player/timeline/poll", &[("wait", "0".into())])
                .await;
            let _op = self.inner.op.lock().await;
            if !self.current(generation) {
                return;
            }
            let Some(mut a) = self.inner.active.lock().clone() else {
                return;
            };
            match polled {
                Ok(body) => {
                    a.misses = 0;
                    let tl = companion::parse_timeline(&body);
                    let stopped = tl.as_ref().is_none_or(|t| t.state == "stopped");
                    if stopped {
                        if a.seen_playing || a.started.elapsed() >= self.inner.cfg.grace {
                            // Stopped there by someone else: control comes back here.
                            self.inner.generation.fetch_add(1, Ordering::SeqCst);
                            self.inner.active.lock().take();
                            *self.inner.stopped_on.lock() = Some(a.session.player_name.clone());
                            self.resume_local(&a, false).await;
                            self.emit_state();
                            return;
                        }
                    } else if let Some(t) = tl {
                        self.apply_timeline(&mut a, &t).await;
                    }
                    *self.inner.active.lock() = Some(a);
                    self.emit_state();
                }
                Err(_) => {
                    a.misses += 1;
                    if a.misses >= self.inner.cfg.misses {
                        self.inner.generation.fetch_add(1, Ordering::SeqCst);
                        self.inner.active.lock().take();
                        self.toast_error(couldnt_reach(&a.session.player_name));
                        self.resume_local(&a, false).await;
                        self.emit_state();
                        return;
                    }
                    *self.inner.active.lock() = Some(a);
                }
            }
        }
    }
}
