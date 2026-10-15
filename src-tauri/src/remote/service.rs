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
use super::airplay::{self, AirPlayLink, AirPlayTarget};
use super::cast::{self, CastLink};
use super::lan::{self, LanDevice, LanLink, MdnsSource};
use super::discovery::{self, Found};
use super::peer::{self, PeerDevice};
use super::peer::auth::{PeerKey, account_tag};
use super::peer::client::{PeerClient, PeerError, PeerLink};
use super::peer::directory::PeerDirectory;
use super::playqueue;
use super::types::{
    EV_REMOTE_PLAYERS, EV_REMOTE_STATE, PlayerKind, PlayerVia, RemoteCommand, RemotePlayer, RemoteSession, RemoteState,
    RemoteStatus, repeat_code, repeat_from_code,
};
use crate::audio::events::{PlayerState, PlayerStatus};
use crate::audio::queue::{ContextInfo, Repeat};
use crate::audio::source::PlayContext;
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::audio::netsink::NetRoute;
use crate::plex::Token;
use crate::plex::keychain::TokenStore;
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
    /// The signed-in Plex account's id (its uuid), for the account tag
    /// other Platter devices match on.
    async fn account_id(&self) -> Option<String> {
        None
    }
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
    /// The chosen output id (None follows the system default).
    async fn output_selected(&self) -> AppResult<Option<String>> {
        Ok(None)
    }
    /// Moves the sound to an output (a network one while on AirPlay).
    async fn select_output(&self, _id: Option<String>) -> AppResult<()> {
        Err(AppError::Invalid("This player has no outputs to choose.".into()))
    }
    /// Take over a queue from another Platter: `ids` is the playing song
    /// then next up (in play order), `user` the user queue.
    #[allow(clippy::too_many_arguments)]
    async fn adopt(
        &self,
        _context: Option<ContextInfo>,
        ids: Vec<String>,
        _user: Vec<String>,
        _shuffle: bool,
        repeat: Repeat,
        ms: u64,
        play: bool,
    ) -> AppResult<()> {
        let first = ids.first().cloned();
        self.play_tracks(ids, first, ms, play).await?;
        self.set_repeat(repeat).await
    }
}

/// What Cast devices and AirPlay receivers need: mDNS, the output route
/// AirPlay plays through, the keychain for receiver passwords, and TLS.
#[derive(Clone)]
pub struct LanEnv {
    pub mdns: Arc<dyn MdnsSource>,
    pub route: NetRoute,
    pub passwords: Arc<dyn TokenStore>,
    pub tls: Arc<rustls::ClientConfig>,
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
    /// A Cast device or AirPlay receiver (Plex players have routes instead).
    lan: Option<Arc<dyn LanLink>>,
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
    lan: Mutex<Option<LanEnv>>,
    /// The user's other Platter devices (None until the app sets it up).
    peers: Mutex<Option<Arc<dyn PeerDirectory>>>,
    peer_http: reqwest::Client,
    /// More routes the control API answers (listening sessions).
    extra: peer::server::ExtraSlot,
}

/// Cheap to clone; managed by Tauri.
#[derive(Clone)]
pub struct RemoteService {
    inner: Arc<Inner>,
}

/// How long a Platter device gets to answer when the list refreshes.
const PEER_PROBE: Duration = Duration::from_millis(1500);
/// A Platter device plex.tv saw this recently counts as on.
const PEER_SEEN_SECS: i64 = 15 * 60;

fn now_secs() -> i64 {
    (peer::auth::now_ms() / 1000) as i64
}

/// A stable 64-bit id for this sender (AirPlay wants 16 hex digits).
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
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
                lan: Mutex::new(None),
                peers: Mutex::new(None),
                extra: Default::default(),
                peer_http: reqwest::Client::builder()
                    .no_proxy()
                    .connect_timeout(PEER_PROBE)
                    .build()
                    .map_err(|e| AppError::Internal(format!("http client: {e}")))?,
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

    /// Where more control API routes plug in.
    pub fn extra_routes(&self) -> peer::server::ExtraSlot {
        self.inner.extra.clone()
    }

    /// Turns on the user's other Platter devices.
    pub fn set_peers(&self, dir: Arc<dyn PeerDirectory>) {
        *self.inner.peers.lock() = Some(dir);
    }

    pub fn own_id(&self) -> &str {
        &self.inner.own_id
    }

    /// The tag other Platter devices of this account announce.
    pub async fn account_tag(&self) -> Option<String> {
        self.inner.env.account_id().await.map(|id| account_tag(&id))
    }

    async fn peer_client(&self) -> Option<PeerClient> {
        let token = self.inner.env.account_token().await?;
        Some(PeerClient::new(self.inner.peer_http.clone(), self.inner.own_id.clone(), PeerKey::from_token(&token)))
    }

    /// Turns on Cast and AirPlay devices.
    pub fn set_lan(&self, lan: LanEnv) {
        *self.inner.lan.lock() = Some(lan);
    }

    fn lan(&self) -> AppResult<LanEnv> {
        self.inner
            .lan
            .lock()
            .clone()
            .ok_or_else(|| AppError::Invalid("Cast and AirPlay devices aren't available.".into()))
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
        let plex = self.inner.switches.show_players();
        let cast = self.inner.switches.show_cast();
        if !cast && let Some(l) = self.inner.lan.lock().as_ref() {
            l.mdns.stop();
        }
        let (found, lan, mdns_peers) = tokio::join!(
            async {
                if plex { self.discover().await } else { Vec::new() }
            },
            async {
                if cast { self.lan_devices().await } else { Vec::new() }
            },
            self.peer_answers()
        );
        let (platter, mut found): (Vec<Found>, Vec<Found>) =
            found.into_iter().partition(|f| f.player.product == peer::PRODUCT);
        let mut rows = self.platter_rows(mdns_peers, platter).await;
        // A computer running Platter also answers AirPlay: that's it, not a speaker.
        let peer_ips: Vec<std::net::IpAddr> = rows.iter().filter_map(|f| f.peer.as_ref()).flat_map(|d| d.ips()).collect();
        let lan: Vec<Found> = lan
            .into_iter()
            .filter(|f| match &f.lan {
                Some(LanDevice::AirPlay(d)) => !peer_ips.contains(&d.addr.ip()),
                _ => true,
            })
            .collect();
        rows.append(&mut found);
        rows.extend(lan);
        let found = rows;
        let list: Vec<RemotePlayer> = found.iter().map(|f| f.player.clone()).collect();
        *self.inner.players.lock() = found;
        self.emit(EV_REMOTE_PLAYERS, &list);
        list
    }

    /// Cast devices and AirPlay receivers on this network.
    async fn lan_devices(&self) -> Vec<Found> {
        let Ok(lan) = self.lan() else { return Vec::new() };
        let answers = lan.mdns.scan().await;
        let has_password = |id: &str| lan.passwords.get(&airplay::password_key(id)).ok().flatten().is_some();
        let mut list = lan::lan_found(&answers, &has_password, self.inner.switches.show_airplay_locked());
        list.sort_by_key(|f| f.player.name.to_lowercase());
        list
    }

    /// Platter devices of this account announcing on this network.
    async fn peer_answers(&self) -> Vec<PeerDevice> {
        let dir = self.inner.peers.lock().clone();
        let Some(dir) = dir else { return Vec::new() };
        let Some(tag) = self.account_tag().await else { return Vec::new() };
        let own = self.inner.own_id.clone();
        let mut out: Vec<PeerDevice> = Vec::new();
        for a in dir.scan().await {
            if let Some(d) = peer::from_mdns(&a, &own, &tag)
                && !out.iter().any(|o| o.id == d.id)
            {
                out.push(d);
            }
        }
        out
    }

    /// The Platter rows: devices announcing here and the ones Plex lists,
    /// each asked whether it answers.
    async fn platter_rows(&self, announced: Vec<PeerDevice>, from_plex: Vec<Found>) -> Vec<Found> {
        let own = self.inner.own_id.as_str();
        let now = now_secs();
        // Everything with an address gets asked.
        let mut candidates: Vec<(PeerDevice, bool)> = announced.into_iter().map(|d| (d, true)).collect();
        for f in &from_plex {
            if f.player.id == own {
                continue;
            }
            match candidates.iter_mut().find(|(d, _)| d.id == f.player.id) {
                Some((d, _)) => {
                    for u in &f.direct {
                        if !d.bases.contains(u) {
                            d.bases.push(u.clone());
                        }
                    }
                }
                None if !f.direct.is_empty() => candidates.push((
                    PeerDevice {
                        id: f.player.id.clone(),
                        name: f.player.name.clone(),
                        platform: String::new(),
                        bases: f.direct.clone(),
                    },
                    false,
                )),
                None => {}
            }
        }
        let client = self.peer_client().await;
        let probes = futures::future::join_all(candidates.iter().map(|(d, _)| {
            let client = client.clone();
            async move {
                match client {
                    Some(c) => c.first_answer(&d.bases, &d.id, PEER_PROBE).await,
                    None => Err(PeerError::Unreachable),
                }
            }
        }))
        .await;
        let recently = |f: &Found| f.player.online || f.last_seen.is_some_and(|t| now - t <= PEER_SEEN_SECS);
        let mut rows: Vec<Found> = Vec::new();
        for ((mut dev, announced), probe) in candidates.into_iter().zip(probes) {
            let plex = from_plex.iter().find(|f| f.player.id == dev.id);
            let mut player = lan::lan_player(dev.id.clone(), dev.name.clone(), peer::PRODUCT.into(), PlayerKind::Computer, PlayerVia::Platter);
            match probe {
                Ok((base, hello)) => {
                    dev.bases.retain(|b| *b != base);
                    dev.bases.insert(0, base);
                    player.name = hello.name.clone();
                    dev.name = hello.name;
                    player.playing = hello.playing;
                }
                Err(PeerError::Refused(why)) => player.unsupported = Some(why),
                Err(_) => {
                    // An old address plex.tv still has, from a computer that's off.
                    if !announced && !plex.is_some_and(recently) {
                        continue;
                    }
                    player.unreachable = true;
                }
            }
            rows.push(Found {
                player,
                direct: Vec::new(),
                token: plex.and_then(|f| f.token.clone()),
                via_server: plex.is_some_and(|f| f.via_server),
                sonos: false,
                lan: None,
                peer: Some(dev),
                last_seen: plex.and_then(|f| f.last_seen),
            });
        }
        // Seen through Plex, with no address to try.
        for f in from_plex {
            if f.player.id == own || rows.iter().any(|r| r.player.id == f.player.id) || !recently(&f) {
                continue;
            }
            let mut player = lan::lan_player(f.player.id.clone(), f.player.name.clone(), peer::PRODUCT.into(), PlayerKind::Computer, PlayerVia::Platter);
            player.playing = f.player.playing.clone();
            player.unreachable = true;
            rows.push(Found { player, direct: Vec::new(), ..f });
        }
        rows.sort_by(|a, b| {
            a.player
                .unreachable
                .cmp(&b.player.unreachable)
                .then_with(|| a.player.name.to_lowercase().cmp(&b.player.name.to_lowercase()))
        });
        rows
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
        if let Some(link) = self.here_link() {
            // AirPlay: the queue is here, so just play it.
            self.inner.local.load_context(context, start_track_id).await?;
            self.inner.local.resume().await?;
            link.jumped().await;
            return Ok(self.state());
        }
        self.inner.local.load_context(context, start_track_id).await?;
        self.resend(found).await
    }

    /// Jump to a queue entry while playing on another device: the queue here
    /// moves (paused) and the player gets it from that song.
    pub async fn queue_jump(&self, item_id: String) -> AppResult<RemoteState> {
        let _op = self.inner.op.lock().await;
        let found = self
            .inner
            .active
            .lock()
            .as_ref()
            .map(|a| a.found.clone())
            .ok_or_else(|| AppError::Invalid("Nothing is playing on another device.".into()))?;
        if let Some(link) = self.here_link() {
            self.inner.local.jump_at(item_id, 0, true).await?;
            link.jumped().await;
            return Ok(self.state());
        }
        self.inner.local.jump_at(item_id, 0, false).await?;
        self.resend(found).await
    }

    /// The active link when its sound comes from this computer (AirPlay).
    fn here_link(&self) -> Option<Arc<dyn LanLink>> {
        self.inner
            .active
            .lock()
            .as_ref()
            .and_then(|a| a.lan.clone())
            .filter(|l| l.plays_here())
    }

    /// Move to an AirPlay receiver with its password; kept in the keychain
    /// once it works.
    pub async fn move_with_password(&self, player_id: &str, password: String) -> AppResult<RemoteState> {
        let _op = self.inner.op.lock().await;
        if self.inner.active.lock().is_some() {
            self.return_inner(Some(false)).await;
        }
        let found = self
            .inner
            .players
            .lock()
            .iter()
            .find(|f| f.player.id == player_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound("That device isn't available any more.".into()))?;
        let state = self.start_lan(found, Some(password.clone())).await?;
        if let Ok(lan) = self.lan()
            && let Ok(t) = Token::new(password)
            && let Err(e) = lan.passwords.set(&airplay::password_key(player_id), &t)
        {
            tracing::warn!("couldn't keep the AirPlay password: {e}");
        }
        if let Some(f) = self.inner.players.lock().iter_mut().find(|f| f.player.id == player_id) {
            f.player.needs_password = false;
        }
        Ok(state)
    }

    /// Start on a Cast device or AirPlay receiver.
    async fn start_lan(&self, found: Found, password: Option<String>) -> AppResult<RemoteState> {
        let lan = self.lan()?;
        let name = found.player.name.clone();
        if let Some(why) = &found.player.unsupported {
            return Err(AppError::Invalid(format!("Can't play on {name}: {why}.")));
        }
        let st = self.inner.local.state().await?;
        let Some(track) = st.track.clone() else {
            return Err(AppError::Invalid("Play something first, then move it.".into()));
        };
        let (link, play_queue_id): (Arc<dyn LanLink>, Option<String>) = match found.lan.clone() {
            Some(LanDevice::Cast(dev)) => {
                let server = self.inner.env.server().await?;
                let mut tracks = vec![track.clone()];
                tracks.extend(st.queue.user_queue.iter().map(|i| i.track.clone()));
                tracks.extend(st.queue.next_up.iter().map(|i| i.track.clone()));
                let ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
                let pq = playqueue::create(&self.inner.http, &server, &ids, &track.id, st.repeat).await?;
                let token = match cast::plan::transient_token(&self.inner.http, &server).await {
                    Some(t) => t,
                    None => {
                        tracing::info!("no transient token from the server; the cast device gets the server token");
                        server.token.clone()
                    }
                };
                let plan = cast::plan::plan(&server, &token, &self.inner.own_id, &pq.id, &tracks, st.position_ms, st.repeat);
                let link = CastLink::start(&dev, &plan, lan.tls.clone(), self.inner.cfg.timeout)
                    .await
                    .map_err(|e| {
                        tracing::info!(kind = e.kind(), "cast device didn't start");
                        AppError::Network(couldnt_reach(&name))
                    })?;
                if self.inner.switches.stop_old()
                    && let Err(e) = self.inner.local.pause().await
                {
                    tracing::warn!("pausing here after a move failed: {e}");
                }
                (Arc::new(link), Some(pq.id))
            }
            Some(LanDevice::AirPlay(dev)) => {
                let password = match password {
                    Some(p) => Some(p),
                    None => lan
                        .passwords
                        .get(&airplay::password_key(&dev.id))
                        .ok()
                        .flatten()
                        .map(|t| t.expose().to_string()),
                };
                let client_id = format!("{:016X}", fnv(&self.inner.own_id));
                let target = Arc::new(AirPlayTarget::new(dev, password, client_id, self.inner.cfg.timeout, st.volume));
                let link = AirPlayLink::start(target, self.inner.local.clone(), lan.route.clone())
                    .await
                    .map_err(|e| match e {
                        AppError::Auth(_) => e,
                        other => {
                            tracing::info!(kind = other.kind(), "AirPlay receiver didn't start");
                            AppError::Network(couldnt_reach(&name))
                        }
                    })?;
                (Arc::new(link), None)
            }
            None => return Err(AppError::Invalid("Not a Cast or AirPlay device.".into())),
        };
        let session = RemoteSession {
            player_id: found.player.id.clone(),
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
            routes: Vec::new(),
            lan: Some(link),
            session,
            play_queue_id,
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

    async fn resend(&self, found: Found) -> AppResult<RemoteState> {
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
        if found.lan.is_some() {
            return self.start_lan(found, None).await;
        }
        if found.player.via == PlayerVia::Platter {
            return self.start_peer(found).await;
        }
        self.start_companion(found).await
    }

    /// Another Platter device: its control API on this network first; when
    /// that doesn't answer, the Plex companion path if it has one.
    async fn start_peer(&self, found: Found) -> AppResult<RemoteState> {
        let name = found.player.name.clone();
        let st = self.inner.local.state().await?;
        let Some(track) = st.track.clone() else {
            return Err(AppError::Invalid("Play something first, then move it.".into()));
        };
        let server = self.inner.env.server().await?;
        let client = self
            .peer_client()
            .await
            .ok_or_else(|| AppError::Auth("Not signed in.".into()))?;
        let req = peer::PlayRequest {
            server_id: server.machine_id.clone(),
            current: track.id.clone(),
            user_queue: st.queue.user_queue.iter().map(|i| i.track.id.clone()).take(peer::MAX_SONGS).collect(),
            next_up: st.queue.next_up.iter().map(|i| i.track.id.clone()).take(peer::MAX_SONGS).collect(),
            context: st.queue.context.clone(),
            position_ms: st.position_ms,
            shuffle: st.shuffle,
            repeat: st.repeat,
            play: true,
        };
        let bases = found.peer.as_ref().map(|d| d.bases.clone()).unwrap_or_default();
        let mut taken = None;
        let mut last = PeerError::Unreachable;
        for base in bases {
            match client.play(&base, &found.player.id, &req, self.inner.cfg.timeout).await {
                Ok(()) => {
                    taken = Some(base);
                    break;
                }
                Err(PeerError::Unreachable) => {}
                Err(e) => {
                    last = e;
                    break;
                }
            }
        }
        let Some(base) = taken else {
            return match last {
                PeerError::Unreachable if found.via_server => {
                    tracing::info!("the other Platter isn't reachable here; trying through Plex");
                    self.start_companion(found).await
                }
                PeerError::Unreachable => Err(AppError::Network(couldnt_reach(&name))),
                PeerError::Refused(why) => Err(AppError::Auth(format!("{}: {why}.", couldnt_reach(&name)))),
                PeerError::Failed(why) => Err(AppError::Invalid(why)),
            };
        };
        if self.inner.switches.stop_old()
            && let Err(e) = self.inner.local.pause().await
        {
            tracing::warn!("pausing here after a move failed: {e}");
        }
        let link = PeerLink { client, base, id: found.player.id.clone(), timeout: self.inner.cfg.timeout };
        let session = RemoteSession {
            player_id: found.player.id.clone(),
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
            routes: Vec::new(),
            lan: Some(Arc::new(link)),
            session,
            play_queue_id: None,
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

    /// A Plex player through the companion protocol.
    async fn start_companion(&self, found: Found) -> AppResult<RemoteState> {
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
            lan: None,
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
        if let Some(t) = self.poll_active(&a).await.ok().flatten()
            && t.state != "stopped"
        {
            self.apply_timeline(&mut a, &t).await;
        }
        if let Some(link) = &a.lan {
            link.leave(self.inner.switches.stop_old()).await;
        } else if self.inner.switches.stop_old() {
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
        let (id, name, routes, lan) = {
            let g = self.inner.active.lock();
            let a = g
                .as_ref()
                .ok_or_else(|| AppError::Invalid("Nothing is playing on another device.".into()))?;
            (a.found.player.id.clone(), a.session.player_name.clone(), a.routes.clone(), a.lan.clone())
        };
        if let Some(link) = lan {
            if let Err(e) = link.command(&cmd).await {
                tracing::info!(kind = e.kind(), "device refused a command");
                return Err(AppError::Network(couldnt_reach(&name)));
            }
            self.after_command(&id, cmd);
            return Ok(self.state());
        }
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
        self.after_command(&id, cmd);
        Ok(self.state())
    }

    /// The session as the command left it, until the next status.
    fn after_command(&self, id: &str, cmd: RemoteCommand) {
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
    }

    /// The active device's timeline: Ok(None) when it stopped, Err when it
    /// didn't answer.
    async fn poll_active(&self, a: &Active) -> AppResult<Option<Timeline>> {
        match &a.lan {
            Some(link) => link.poll().await,
            None => self
                .inner
                .companion
                .send(&a.found.player.id, &a.routes, "/player/timeline/poll", &[("wait", "0".into())])
                .await
                .map(|body| companion::parse_timeline(&body)),
        }
    }

    /// Control comes back here after the device stopped or went away.
    async fn take_back(&self, a: &Active) {
        match &a.lan {
            Some(link) => {
                link.leave(false).await;
                if !link.plays_here() {
                    self.resume_local(a, false).await;
                }
            }
            None => self.resume_local(a, false).await,
        }
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
            let Some(snapshot) = self.inner.active.lock().clone() else {
                return;
            };
            let polled = self.poll_active(&snapshot).await;
            let _op = self.inner.op.lock().await;
            if !self.current(generation) {
                return;
            }
            let Some(mut a) = self.inner.active.lock().clone() else {
                return;
            };
            match polled {
                Ok(tl) => {
                    a.misses = 0;
                    let stopped = tl.as_ref().is_none_or(|t| t.state == "stopped");
                    if stopped {
                        if a.seen_playing || a.started.elapsed() >= self.inner.cfg.grace {
                            // Stopped there by someone else: control comes back here.
                            self.inner.generation.fetch_add(1, Ordering::SeqCst);
                            self.inner.active.lock().take();
                            *self.inner.stopped_on.lock() = Some(a.session.player_name.clone());
                            self.take_back(&a).await;
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
                        self.take_back(&a).await;
                        self.emit_state();
                        return;
                    }
                    *self.inner.active.lock() = Some(a);
                }
            }
        }
    }
}
