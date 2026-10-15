//! `PlexState`: the Tauri-managed owner of sign-in, the selected server, its
//! current connection, health monitoring and the notification socket.
//!
//! Other modules get a ready client with [`PlexState::client`] and can
//! listen to server push events with [`PlexState::subscribe`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{Notify, RwLock, broadcast, mpsc, watch};

use super::auth::{self, Pin, PlexTv, PlexUser};
use super::client::PlexClient;
use super::discovery::{
    self, DiscoveredServer, DiscoveryCache, Probe, RaceConfig, ServerList, load_servers,
};
use super::health::{HealthAction, HealthMachine, PING_INTERVAL, ProbeOutcome, ScanTracker};
use super::identity::ClientIdentity;
use super::keychain::{ACCOUNT_TOKEN, KeyringStore, SERVER_TOKEN, TokenStore};
use super::notifications::{self, PlexEvent};
use super::token::Token;
use super::types::{
    AuthState, AuthStatus, EVENT_AUTH_CHANGED, EVENT_HEALTH_CHANGED, EVENT_TOAST, Health,
    HealthStatus, PinInfo, ServerInfo, Toast,
};
use crate::error::{AppError, AppResult};

/// Where events go. The app uses [`TauriSink`]; tests record them.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

struct NoopSink;
impl EventSink for NoopSink {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

/// Emits through a Tauri `AppHandle`.
pub struct TauriSink(pub tauri::AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        use tauri::Emitter;
        if let Err(e) = self.0.emit(event, payload) {
            tracing::warn!("emit {event} failed: {e}");
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn spawn<F>(fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tauri::async_runtime::spawn(fut);
}

#[derive(Debug, Clone)]
struct Current {
    server: DiscoveredServer,
    token: Token,
    /// None while no connection answers (offline).
    client: Option<PlexClient>,
}

#[derive(Debug, Default)]
struct Session {
    account: Option<Token>,
    username: Option<String>,
    /// The account's profile picture as an `/ext/` art path (no token).
    avatar: Option<String>,
    servers: Vec<DiscoveredServer>,
    current: Option<Current>,
    cache: DiscoveryCache,
    /// Plex rejected the saved token; waiting for the user to sign in again.
    reauth: bool,
}

struct Inner {
    data_dir: PathBuf,
    identity: Arc<ClientIdentity>,
    store: Arc<dyn TokenStore>,
    tv: PlexTv,
    probe_http: reqwest::Client,
    race_cfg: RaceConfig,
    sink: Mutex<Arc<dyn EventSink>>,
    session: RwLock<Session>,
    pin_cancel: Mutex<Option<watch::Sender<bool>>>,
    health: Mutex<HealthMachine>,
    last_health: Mutex<Option<Health>>,
    scan: Mutex<ScanTracker>,
    /// Stops the health loop + notification socket of the current server.
    background_stop: Mutex<Option<watch::Sender<bool>>>,
    notif_stop: Mutex<Option<watch::Sender<bool>>>,
    recheck: Notify,
    events: broadcast::Sender<PlexEvent>,
    /// False while `start` is restoring the saved session; true otherwise.
    restored: watch::Sender<bool>,
    /// Go offline: no requests to the server or plex.tv.
    offline: Arc<std::sync::atomic::AtomicBool>,
    /// Clients for the account's other servers (more than one library), by
    /// server tag, with when their connection was picked.
    others: Mutex<std::collections::HashMap<String, (PlexClient, std::time::Instant)>>,
}

/// A connection picked for another server is raced again after this.
const OTHER_SERVER_TTL: Duration = Duration::from_secs(10 * 60);

/// Cheap to clone; all clones share state.
#[derive(Clone)]
pub struct PlexState {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PlexState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlexState")
            .field("data_dir", &self.inner.data_dir)
            .finish_non_exhaustive()
    }
}

impl PlexState {
    /// Production constructor: OS keychain, real plex.tv, client id stored in
    /// `app_data_dir`.
    pub fn new(app_data_dir: PathBuf) -> AppResult<Self> {
        Self::with_parts(
            app_data_dir,
            Arc::new(KeyringStore::new()),
            auth::PLEX_TV,
            RaceConfig::default(),
        )
    }

    /// Injectable constructor (tests).
    pub fn with_parts(
        data_dir: PathBuf,
        store: Arc<dyn TokenStore>,
        plex_tv_base: &str,
        race_cfg: RaceConfig,
    ) -> AppResult<Self> {
        let identity = Arc::new(ClientIdentity::load_or_create(&data_dir)?);
        let tv = PlexTv::with_base(identity.clone(), plex_tv_base)?;
        let probe_http = reqwest::Client::builder()
            .default_headers(identity.headers()?)
            .connect_timeout(race_cfg.timeout)
            .build()
            .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
        let cache = DiscoveryCache::load(&data_dir);
        let (events, _) = broadcast::channel(256);
        Ok(PlexState {
            inner: Arc::new(Inner {
                data_dir,
                identity,
                store,
                tv,
                probe_http,
                race_cfg,
                sink: Mutex::new(Arc::new(NoopSink)),
                session: RwLock::new(Session {
                    cache,
                    ..Session::default()
                }),
                pin_cancel: Mutex::new(None),
                health: Mutex::new(HealthMachine::new()),
                last_health: Mutex::new(None),
                scan: Mutex::new(ScanTracker::default()),
                background_stop: Mutex::new(None),
                notif_stop: Mutex::new(None),
                recheck: Notify::new(),
                events,
                restored: watch::channel(true).0,
                offline: Arc::default(),
                others: Mutex::new(std::collections::HashMap::new()),
            }),
        })
    }

    pub fn set_event_sink(&self, sink: Arc<dyn EventSink>) {
        *self.inner.sink.lock() = sink;
    }

    /// Call once from Tauri `setup`: wires events to the app and restores
    /// the previous session in the background.
    pub fn start(&self, app: tauri::AppHandle) {
        self.set_event_sink(Arc::new(TauriSink(app)));
        self.restore_in_background();
    }

    /// Restore the saved session on a background task. Until it finishes,
    /// `client_when_ready` callers wait for it.
    fn restore_in_background(&self) {
        self.inner.restored.send_replace(false);
        let me = self.clone();
        spawn(async move {
            if let Err(e) = me.restore().await {
                tracing::warn!(kind = e.kind(), "restoring Plex session failed");
            }
            me.inner.restored.send_replace(true);
        });
    }

    pub fn identity(&self) -> &Arc<ClientIdentity> {
        &self.inner.identity
    }

    // ---- events -----------------------------------------------------------

    fn emit<T: Serialize>(&self, event: &str, payload: &T) {
        let v = serde_json::to_value(payload).unwrap_or(serde_json::Value::Null);
        let sink = self.inner.sink.lock().clone();
        sink.emit(event, v);
    }

    /// Show a toast in the UI.
    pub fn toast(&self, kind: &str, message: impl Into<String>) {
        self.emit(
            EVENT_TOAST,
            &Toast {
                kind: kind.into(),
                message: message.into(),
            },
        );
    }

    async fn emit_auth(&self) {
        let s = self.auth_status().await;
        self.emit(EVENT_AUTH_CHANGED, &s);
    }

    fn emit_health(&self) {
        let h = self.health();
        let mut last = self.inner.last_health.lock();
        if last.as_ref() != Some(&h) {
            let was_offline = last.as_ref().is_some_and(|p| {
                p.status == HealthStatus::Offline
                    && p.banner
                        .as_ref()
                        .is_some_and(|b| b.kind == "offline" || b.kind == "noInternet")
            });
            let back = was_offline
                && matches!(
                    h.status,
                    HealthStatus::Local | HealthStatus::Remote | HealthStatus::Relay
                );
            *last = Some(h.clone());
            drop(last);
            self.emit(EVENT_HEALTH_CHANGED, &h);
            if back {
                self.toast("info", "Your server is back.");
            }
        }
    }

    /// Server push events (library updates, scans, playback) for sync etc.
    pub fn subscribe(&self) -> broadcast::Receiver<PlexEvent> {
        self.inner.events.subscribe()
    }

    // ---- queries ----------------------------------------------------------

    pub async fn auth_status(&self) -> AuthStatus {
        let pending = self.inner.pin_cancel.lock().is_some();
        let s = self.inner.session.read().await;
        if s.account.is_none() {
            let mut st = AuthStatus::signed_out();
            if pending {
                st.state = AuthState::Pending;
            }
            st.reauth_required = s.reauth;
            st.username = if s.reauth { s.username.clone() } else { None };
            st.avatar = if s.reauth { s.avatar.clone() } else { None };
            return st;
        }
        let server = s.current.as_ref().map(|c| {
            c.server.info(
                c.client.is_some(),
                c.client.as_ref().and_then(PlexClient::connection),
            )
        });
        AuthStatus {
            state: AuthState::SignedIn,
            username: s.username.clone(),
            avatar: s.avatar.clone(),
            needs_server_pick: server.is_none() && !s.servers.is_empty(),
            server,
            reauth_required: false,
        }
    }

    pub async fn servers(&self) -> Vec<ServerInfo> {
        let s = self.inner.session.read().await;
        s.servers
            .iter()
            .map(|srv| match &s.current {
                Some(c) if c.server.machine_id == srv.machine_id => srv.info(
                    c.client.is_some(),
                    c.client.as_ref().and_then(PlexClient::connection),
                ),
                _ => srv.info(srv.presence, None),
            })
            .collect()
    }

    pub fn health(&self) -> Health {
        self.inner.health.lock().snapshot(now_ms())
    }

    /// Go offline is on.
    pub fn offline_by_choice(&self) -> bool {
        self.inner.offline.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The Go offline switch, for other network users to check.
    pub fn offline_switch(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.inner.offline.clone()
    }

    /// Go offline on: drop the connection and stop every request to the
    /// server and plex.tv. Off: reconnect as at startup.
    pub fn set_offline_by_choice(&self, on: bool) {
        if self.inner.offline.swap(on, std::sync::atomic::Ordering::SeqCst) == on {
            return;
        }
        self.inner.health.lock().set_offline_by_choice(on);
        self.emit_health();
        let me = self.clone();
        spawn(async move {
            if on {
                if let Some(tx) = me.inner.notif_stop.lock().take() {
                    let _ = tx.send(true);
                }
                if let Some(cur) = me.inner.session.write().await.current.as_mut() {
                    cur.client = None;
                }
                me.emit_auth().await;
                return;
            }
            let (has_server, signed_in) = {
                let s = me.inner.session.read().await;
                (s.current.is_some(), s.account.is_some())
            };
            if has_server {
                // The health loop re-races the server's addresses.
                me.recheck();
            } else if signed_in {
                // Started offline: pick up the server as at startup.
                me.restore_in_background();
            }
        });
    }

    /// A client for the selected server over the current best connection.
    pub async fn client(&self) -> AppResult<PlexClient> {
        if self.offline_by_choice() {
            return Err(AppError::Network("You're offline.".into()));
        }
        let s = self.inner.session.read().await;
        match &s.current {
            Some(Current {
                client: Some(c), ..
            }) => Ok(c.clone()),
            Some(_) => Err(AppError::Network("Your server is offline.".into())),
            None if s.account.is_none() => Err(AppError::Auth("Not signed in.".into())),
            None => Err(AppError::Invalid("No server selected.".into())),
        }
    }

    /// The connected server's tag (`crate::sid`), which its ids carry.
    pub async fn tag(&self) -> Option<String> {
        self.machine_id().await.map(|m| crate::sid::tag(&m))
    }

    /// A client for the server an item id or image path belongs to
    /// (`crate::sid`): the connected server for its own ids and for ids
    /// without a tag, else another of the account's servers.
    pub async fn client_for(&self, id: &str) -> AppResult<PlexClient> {
        match crate::sid::tag_of(id) {
            None => self.client().await,
            Some(t) => self.client_for_tag(t).await,
        }
    }

    /// A client for the account's server with this tag. Another server's
    /// connection is raced the first time (local address first, as for the
    /// connected server) and kept for a while; it never changes the
    /// connected server or its health.
    pub async fn client_for_tag(&self, tag: &str) -> AppResult<PlexClient> {
        let (server, account) = {
            let s = self.inner.session.read().await;
            if s.current.as_ref().is_some_and(|c| crate::sid::tag(&c.server.machine_id) == tag) {
                drop(s);
                return self.client().await;
            }
            let account = s.account.clone().ok_or_else(|| AppError::Auth("Not signed in.".into()))?;
            let server = s
                .servers
                .iter()
                .find(|x| crate::sid::tag(&x.machine_id) == tag)
                .cloned()
                .ok_or_else(|| AppError::NotFound("That server isn't on your account any more.".into()))?;
            (server, account)
        };
        if self.offline_by_choice() {
            return Err(AppError::Network("You're offline.".into()));
        }
        if let Some((c, at)) = self.inner.others.lock().get(tag)
            && at.elapsed() < OTHER_SERVER_TTL
        {
            return Ok(c.clone());
        }
        let out = discovery::race_detailed(&self.inner.probe_http, &server.connections, Some(&server.machine_id), self.inner.race_cfg).await;
        let Some(p) = out.best else {
            self.inner.others.lock().remove(tag);
            return Err(AppError::Network(format!("{} is offline.", server.name)));
        };
        // A shared server takes its own token from resources; an owned one
        // takes the account's.
        let token = server.access_token.clone().unwrap_or(account);
        let c = PlexClient::new(&p.uri, token, self.inner.identity.clone(), Some(p.kind))?.with_server(&server.machine_id);
        self.inner.others.lock().insert(tag.to_string(), (c.clone(), std::time::Instant::now()));
        Ok(c)
    }

    /// The machine id of the server an item id belongs to (`crate::sid`):
    /// the connected server's for ids without a tag.
    pub async fn machine_id_for(&self, id: &str) -> Option<String> {
        let Some(tag) = crate::sid::tag_of(id) else { return self.machine_id().await };
        let s = self.inner.session.read().await;
        s.current
            .iter()
            .map(|c| &c.server)
            .chain(s.servers.iter())
            .find(|x| crate::sid::tag(&x.machine_id) == tag)
            .map(|x| x.machine_id.clone())
    }

    /// Every server on the account (owned and shared), as plex.tv listed
    /// them, with each one's tag.
    pub async fn account_servers(&self) -> Vec<DiscoveredServer> {
        self.inner.session.read().await.servers.clone()
    }

    /// The selected server's machine id (names library items in playlist
    /// writes).
    pub async fn machine_id(&self) -> Option<String> {
        let s = self.inner.session.read().await;
        s.current.as_ref().map(|c| c.server.machine_id.clone())
    }

    /// The signed-in account's token (plex.tv calls such as the player
    /// list). Never logged or shown.
    pub async fn account_token(&self) -> Option<Token> {
        self.inner.session.read().await.account.clone()
    }

    /// The plex.tv client (same identity headers as the server client).
    pub fn plex_tv(&self) -> &PlexTv {
        &self.inner.tv
    }

    /// Like `client`, but first waits (up to `wait`) for the saved session to
    /// finish restoring at startup, so early requests such as artwork don't
    /// fail just because they arrived before the keychain was read.
    pub async fn client_when_ready(&self, wait: std::time::Duration) -> AppResult<PlexClient> {
        let mut rx = self.inner.restored.subscribe();
        let _ = tokio::time::timeout(wait, rx.wait_for(|done| *done)).await;
        self.client().await
    }

    /// [`Self::client_for`], first waiting like [`Self::client_when_ready`].
    pub async fn client_for_when_ready(&self, id: &str, wait: std::time::Duration) -> AppResult<PlexClient> {
        let mut rx = self.inner.restored.subscribe();
        let _ = tokio::time::timeout(wait, rx.wait_for(|done| *done)).await;
        self.client_for(id).await
    }

    /// Ask the health loop to ping now (wake from sleep, network change).
    pub fn recheck(&self) {
        self.inner.recheck.notify_one();
    }

    /// Check again now if the home network was blocked (the window came back
    /// to the front, maybe from System Settings).
    pub fn recheck_if_local_network_blocked(&self) {
        if self.inner.health.lock().local_network_blocked() {
            self.recheck();
        }
    }

    /// The computer woke from sleep or its network changed: reopen the
    /// notification socket (it is likely dead) and ping now.
    pub async fn reconnect(&self) {
        let client = self
            .inner
            .session
            .read()
            .await
            .current
            .as_ref()
            .and_then(|c| c.client.clone());
        if let Some(c) = client {
            self.start_notifications(c);
        }
        self.recheck();
    }

    /// The computer has no network route at all. Show "You're offline"
    /// right away instead of waiting for the next ping.
    pub fn on_network_lost(&self) {
        self.inner.health.lock().on_no_network();
        self.emit_health();
    }

    /// Result of the music files check (see `crate::monitor`).
    pub fn set_files_available(&self, available: bool) {
        self.inner.health.lock().set_files_available(available);
        self.emit_health();
    }

    pub fn files_available(&self) -> bool {
        self.inner.health.lock().files_available()
    }

    /// The last library sync found no music library on the server.
    pub fn set_no_music_library(&self, none: bool) {
        self.inner.health.lock().set_no_music_library(none);
        self.emit_health();
    }

    pub fn no_music_library(&self) -> bool {
        self.inner.health.lock().no_music_library()
    }

    /// The disk holding the caches is nearly full.
    pub fn set_low_disk(&self, low: bool) {
        self.inner.health.lock().set_low_disk(low);
        self.emit_health();
    }

    /// Only music stored on this computer can play right now.
    pub fn cached_only(&self) -> bool {
        self.inner.health.lock().cached_only()
    }

    /// Connected to the server over any route.
    pub fn connected(&self) -> bool {
        self.inner.health.lock().connected()
    }

    // ---- sign-in ----------------------------------------------------------

    /// Create a PIN, hand the auth URL to `open` (the browser), and poll in
    /// the background. Completion is reported via `auth://changed`.
    pub async fn start_sign_in<F>(&self, open: F) -> AppResult<PinInfo>
    where
        F: FnOnce(&str) -> AppResult<()>,
    {
        self.start_sign_in_with(open, auth::PIN_POLL_INTERVAL, auth::PIN_TIMEOUT)
            .await
    }

    pub async fn start_sign_in_with<F>(
        &self,
        open: F,
        interval: Duration,
        timeout: Duration,
    ) -> AppResult<PinInfo>
    where
        F: FnOnce(&str) -> AppResult<()>,
    {
        self.cancel_sign_in();
        let pin: Pin = self.inner.tv.create_pin().await?;
        let auth_url = self.inner.tv.auth_url(&pin);
        let (tx, rx) = watch::channel(false);
        *self.inner.pin_cancel.lock() = Some(tx);
        if let Err(e) = open(&auth_url) {
            // Not fatal: the UI shows the code + URL as a fallback.
            tracing::warn!(kind = e.kind(), "could not open the browser");
        }
        self.emit_auth().await;
        let me = self.clone();
        let pin_id = pin.id.clone();
        spawn(async move {
            let result = me.inner.tv.poll_pin(&pin_id, interval, timeout, rx).await;
            *me.inner.pin_cancel.lock() = None;
            match result {
                Ok(token) => {
                    if let Err(e) = me.complete_sign_in(token).await {
                        me.toast("error", format!("Sign-in failed: {e}"));
                    }
                }
                Err(e) if auth::is_cancelled(&e) => {}
                Err(e) => {
                    me.toast("error", e.to_string());
                }
            }
            me.emit_auth().await;
        });
        Ok(PinInfo {
            code: pin.code,
            auth_url,
            expires_at: pin.expires_at,
        })
    }

    pub fn cancel_sign_in(&self) {
        if let Some(tx) = self.inner.pin_cancel.lock().take() {
            let _ = tx.send(true);
        }
    }

    /// Store the token, look up the user and servers, auto-select when
    /// possible.
    pub async fn complete_sign_in(&self, token: Token) -> AppResult<()> {
        self.inner.store.set(ACCOUNT_TOKEN, &token)?;
        let user = self.inner.tv.user(&token).await.ok();
        {
            let mut s = self.inner.session.write().await;
            s.account = Some(token.clone());
            s.username = user.as_ref().map(PlexUser::display_name);
            s.avatar = user.as_ref().and_then(PlexUser::avatar_art_path);
            s.reauth = false;
        }
        self.refresh_and_pick(&token).await
    }

    /// Restore a session from the keychain (app start).
    pub async fn restore(&self) -> AppResult<AuthStatus> {
        let Some(token) = self.inner.store.get(ACCOUNT_TOKEN)? else {
            self.emit_auth().await;
            self.emit_health();
            return Ok(self.auth_status().await);
        };
        {
            let mut s = self.inner.session.write().await;
            s.account = Some(token.clone());
        }
        // Username is cosmetic; don't block on it.
        let me = self.clone();
        let t2 = token.clone();
        let offline = self.offline_by_choice();
        spawn(async move {
            if offline {
                return;
            }
            if let Ok(u) = me.inner.tv.user(&t2).await {
                {
                    let mut s = me.inner.session.write().await;
                    s.username = Some(u.display_name());
                    s.avatar = u.avatar_art_path();
                }
                me.emit_auth().await;
            }
        });
        self.refresh_and_pick(&token).await?;
        Ok(self.auth_status().await)
    }

    async fn refresh_and_pick(&self, token: &Token) -> AppResult<()> {
        let cache = self.inner.session.read().await.cache.clone();
        // Go offline: the saved server list, without asking plex.tv.
        let loaded = if self.offline_by_choice() {
            Ok(ServerList {
                servers: cache.servers.iter().map(super::discovery::CachedServer::to_discovered).collect(),
                from_cache: true,
            })
        } else {
            load_servers(&self.inner.tv, token, &cache).await
        };
        let list = match loaded {
            Ok(l) => l,
            Err(AppError::Auth(_)) => {
                self.expire_session().await;
                return Err(AppError::Auth("Please sign in again.".into()));
            }
            Err(e) => {
                self.emit_auth().await;
                return Err(e);
            }
        };
        let ServerList {
            servers,
            from_cache,
        } = list;
        let previous = cache.selected.clone();
        {
            let mut s = self.inner.session.write().await;
            s.servers = servers.clone();
            if !from_cache {
                s.cache.update_servers(&servers);
                let _ = s.cache.save(&self.inner.data_dir);
            }
        }
        let pick = match &previous {
            Some(id) if servers.iter().any(|x| &x.machine_id == id) => Some(id.clone()),
            Some(_) if !from_cache => {
                // The last server is gone from this account.
                let mut h = self.inner.health.lock();
                h.on_server_selected("", None);
                h.on_access_removed();
                drop(h);
                self.emit_health();
                if servers.len() == 1 {
                    // Switching straight to the only other server clears
                    // the banner, so say it once.
                    self.toast("info", "You no longer have access to this server.");
                    Some(servers[0].machine_id.clone())
                } else {
                    None
                }
            }
            _ if servers.len() == 1 => Some(servers[0].machine_id.clone()),
            _ => None,
        };
        match pick {
            Some(id) => self.select_server(&id).await,
            None => {
                self.emit_auth().await;
                Ok(())
            }
        }
    }

    pub async fn sign_out(&self) -> AppResult<()> {
        self.cancel_sign_in();
        self.sign_out_inner("").await;
        Ok(())
    }

    async fn sign_out_inner(&self, reason: &str) {
        self.stop_background();
        let _ = self.inner.store.delete(ACCOUNT_TOKEN);
        let _ = self.inner.store.delete(SERVER_TOKEN);
        {
            let mut s = self.inner.session.write().await;
            s.account = None;
            s.username = None;
            s.avatar = None;
            s.servers.clear();
            s.current = None;
            s.reauth = false;
        }
        *self.inner.scan.lock() = ScanTracker::default();
        self.inner.health.lock().on_signed_out();
        if !reason.is_empty() {
            self.toast("error", reason);
        }
        self.emit_auth().await;
        self.emit_health();
    }

    /// Plex rejected the saved token (expired or revoked). Forget it, stop
    /// talking to the server, and keep the app open so cached music still
    /// plays; the UI shows "Please sign in again" with a sign-in button.
    async fn expire_session(&self) {
        self.stop_background();
        let _ = self.inner.store.delete(ACCOUNT_TOKEN);
        let _ = self.inner.store.delete(SERVER_TOKEN);
        {
            let mut s = self.inner.session.write().await;
            s.account = None;
            s.current = None;
            s.reauth = true;
        }
        *self.inner.scan.lock() = ScanTracker::default();
        self.inner.health.lock().on_token_rejected();
        self.emit_auth().await;
        self.emit_health();
    }

    // ---- server selection + connection ------------------------------------

    fn server_token(&self, server: &DiscoveredServer, account: &Token) -> Token {
        if let Some(t) = &server.access_token {
            return t.clone();
        }
        match self.inner.store.get(SERVER_TOKEN) {
            Ok(Some(t)) => t,
            _ => account.clone(),
        }
    }

    async fn race_server(&self, server: &DiscoveredServer) -> Option<Probe> {
        let connections = match discovery::dev_override() {
            Some(c) => vec![c],
            None => server.connections.clone(),
        };
        let out = discovery::race_detailed(
            &self.inner.probe_http,
            &connections,
            Some(&server.machine_id),
            self.inner.race_cfg,
        )
        .await;
        // Only macOS asks each app for home network access; elsewhere an
        // instant "no route" is a real network problem.
        let blocked = cfg!(target_os = "macos") && out.local_refused;
        if blocked {
            tracing::warn!(
                fallback = ?out.best.as_ref().map(|p| p.kind),
                "the system refused every local address: Local Network access is likely off"
            );
        }
        self.inner.health.lock().set_local_network_blocked(blocked);
        out.best
    }

    /// Pick a server and connect over its best connection.
    pub async fn select_server(&self, machine_id: &str) -> AppResult<()> {
        let (server, account) = {
            let s = self.inner.session.read().await;
            let account = s
                .account
                .clone()
                .ok_or_else(|| AppError::Auth("Not signed in.".into()))?;
            let server = s
                .servers
                .iter()
                .find(|x| x.machine_id == machine_id)
                .cloned()
                .ok_or_else(|| AppError::NotFound(format!("server {machine_id}")))?;
            (server, account)
        };
        self.stop_background();
        let token = self.server_token(&server, &account);
        if token != account {
            let _ = self.inner.store.set(SERVER_TOKEN, &token);
        } else {
            let _ = self.inner.store.delete(SERVER_TOKEN);
        }
        self.inner
            .health
            .lock()
            .on_server_selected(&server.name, server.version.clone());
        let probe = if self.offline_by_choice() { None } else { self.race_server(&server).await };
        let client = match &probe {
            Some(p) => Some(PlexClient::new(
                &p.uri,
                token.clone(),
                self.inner.identity.clone(),
                Some(p.kind),
            )?.with_server(&server.machine_id)),
            None => None,
        };
        {
            let mut s = self.inner.session.write().await;
            s.current = Some(Current {
                server: server.clone(),
                token,
                client: client.clone(),
            });
            s.cache.selected = Some(server.machine_id.clone());
            if let Some(p) = &probe {
                s.cache.record_winner(&server.machine_id, p);
            }
            let _ = s.cache.save(&self.inner.data_dir);
        }
        match &probe {
            Some(p) => {
                self.inner.health.lock().on_probe(
                    ProbeOutcome::Ok {
                        connection: p.kind,
                        latency_ms: p.latency.as_millis().min(u32::MAX as u128) as u32,
                        version: p.version.clone(),
                    },
                    now_ms(),
                );
            }
            None => {
                // Only a guess at this point; the health loop's re-race
                // checks plex.tv too.
                self.inner.health.lock().on_rediscover_failed(now_ms(), true)
            }
        }
        self.emit_auth().await;
        self.emit_health();
        if let Some(c) = client {
            self.start_notifications(c.clone());
            let me = self.clone();
            let mid = server.machine_id.clone();
            spawn(async move { me.check_library_settings(&mid, &c).await });
        }
        self.start_health_loop();
        Ok(())
    }

    /// Spec 02 "Library settings check": warn once per server if local
    /// metadata preference is off, or if there is no music library.
    async fn check_library_settings(&self, machine_id: &str, client: &PlexClient) {
        if self
            .inner
            .session
            .read()
            .await
            .cache
            .settings_checked
            .iter()
            .any(|m| m == machine_id)
        {
            return;
        }
        let Ok(sections) = client.music_sections().await else {
            return;
        };
        // An empty list is reported by the library sync as a banner.
        for sec in &sections {
            let Ok(prefs) = client.section_prefs(&sec.id).await else {
                return;
            };
            if prefs.respect_tags == Some(false) {
                self.toast(
                    "info",
                    format!(
                        "“{}” isn't set to prefer local metadata, so Plex may replace your tags. Turn on “Prefer local metadata” in the library's advanced settings.",
                        sec.title
                    ),
                );
            }
        }
        let mut s = self.inner.session.write().await;
        s.cache.settings_checked.push(machine_id.to_string());
        let _ = s.cache.save(&self.inner.data_dir);
    }

    // ---- background: health loop + notifications ---------------------------

    fn stop_background(&self) {
        self.inner.others.lock().clear();
        if let Some(tx) = self.inner.background_stop.lock().take() {
            let _ = tx.send(true);
        }
        if let Some(tx) = self.inner.notif_stop.lock().take() {
            let _ = tx.send(true);
        }
    }

    fn start_notifications(&self, client: PlexClient) {
        if let Some(tx) = self.inner.notif_stop.lock().take() {
            let _ = tx.send(true);
        }
        let (stop_tx, stop_rx) = watch::channel(false);
        *self.inner.notif_stop.lock() = Some(stop_tx);
        let (tx, mut rx) = mpsc::channel(64);
        let named = client.clone();
        spawn(notifications::run(client, tx, stop_rx));
        let me = self.clone();
        spawn(async move {
            while let Some(mut ev) = rx.recv().await {
                // Item ids carry the server's tag like everything else.
                match &mut ev {
                    PlexEvent::LibraryUpdate { item_id: Some(id), .. }
                    | PlexEvent::Playing { rating_key: Some(id), .. } => *id = named.qualify(id),
                    _ => {}
                }
                me.on_plex_event(&ev);
                let _ = me.inner.events.send(ev);
            }
        });
    }

    fn on_plex_event(&self, ev: &PlexEvent) {
        let now = now_ms();
        let scan = {
            let mut t = self.inner.scan.lock();
            t.on_event(ev).then(|| t.progress())
        };
        if let Some(scan) = scan {
            self.inner.health.lock().on_scan(scan);
            self.emit_health();
        }
        match ev {
            PlexEvent::UpdateState { state } => {
                let installing = matches!(state.as_str(), "installing" | "updating");
                self.inner.health.lock().on_update_state(installing, now);
                self.emit_health();
            }
            PlexEvent::Shutdown => {
                self.inner.health.lock().on_shutdown_notice(now);
                self.recheck();
            }
            PlexEvent::Disconnected => self.recheck(),
            _ => {}
        }
    }

    fn start_health_loop(&self) {
        let (stop_tx, mut stop_rx) = watch::channel(false);
        if let Some(old) = self.inner.background_stop.lock().replace(stop_tx) {
            let _ = old.send(true);
        }
        let me = self.clone();
        spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(PING_INTERVAL) => {}
                    _ = me.inner.recheck.notified() => {}
                    changed = stop_rx.changed() => {
                        if changed.is_err() || *stop_rx.borrow() { return; }
                    }
                }
                if *stop_rx.borrow() {
                    return;
                }
                me.health_tick().await;
            }
        });
    }

    async fn ping(client: &PlexClient) -> ProbeOutcome {
        let started = tokio::time::Instant::now();
        if let Err(e) = client.identity_info().await {
            return ProbeOutcome::from_error(&e);
        }
        let latency_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        // `/identity` ignores the token; `/` checks it.
        match client.root().await {
            Ok(root) => ProbeOutcome::Ok {
                connection: client
                    .connection()
                    .unwrap_or(super::types::ConnectionKind::Remote),
                latency_ms,
                version: root.version,
            },
            Err(e) => ProbeOutcome::from_error(&e),
        }
    }

    /// One health check: ping, then act on the machine's decision.
    pub async fn health_tick(&self) {
        if self.offline_by_choice() {
            return;
        }
        let current = self.inner.session.read().await.current.clone();
        let Some(current) = current else { return };
        let outcome = match &current.client {
            Some(c) => Self::ping(c).await,
            None => ProbeOutcome::Unreachable,
        };
        let action = self.inner.health.lock().on_probe(outcome, now_ms());
        match action {
            // Playing over remote or relay because macOS blocked the home
            // network: race again each tick, so turning access on in System
            // Settings moves back to the local connection by itself.
            HealthAction::None if self.inner.health.lock().retry_local() => self.rediscover(current).await,
            HealthAction::None => {}
            HealthAction::SignInRequired => {
                self.expire_session().await;
                return;
            }
            HealthAction::Rediscover => self.rediscover(current).await,
        }
        self.emit_health();
    }

    /// Re-fetch the server's addresses from plex.tv (falling back to the
    /// cache) and re-race them.
    async fn rediscover(&self, current: Current) {
        let account = self.inner.session.read().await.account.clone();
        let mut server = current.server.clone();
        // Whether plex.tv answered: if neither it nor the server does, this
        // computer is the one that's offline.
        let mut internet = false;
        if let Some(account) = &account {
            let cache = self.inner.session.read().await.cache.clone();
            match load_servers(&self.inner.tv, account, &cache).await {
                Ok(list) => {
                    internet = !list.from_cache;
                    match list
                        .servers
                        .iter()
                        .find(|s| s.machine_id == server.machine_id)
                    {
                        Some(fresh) => {
                            server.connections = fresh.connections.clone();
                            let mut s = self.inner.session.write().await;
                            s.servers = list.servers.clone();
                            if !list.from_cache {
                                s.cache.update_servers(&list.servers);
                            }
                        }
                        None if !list.from_cache => {
                            self.inner.health.lock().on_access_removed();
                            let mut s = self.inner.session.write().await;
                            s.servers = list.servers;
                            s.current = None;
                            drop(s);
                            self.stop_background();
                            self.emit_auth().await;
                            return;
                        }
                        None => {}
                    }
                }
                Err(AppError::Auth(_)) => {
                    self.expire_session().await;
                    return;
                }
                Err(_) => {}
            }
        }
        match self.race_server(&server).await {
            Some(p) => {
                let client = match PlexClient::new(
                    &p.uri,
                    current.token.clone(),
                    self.inner.identity.clone(),
                    Some(p.kind),
                ) {
                    Ok(c) => c.with_server(&server.machine_id),
                    Err(_) => return,
                };
                let changed = current
                    .client
                    .as_ref()
                    .map(|c| c.base_url().as_str().to_string())
                    != Some(client.base_url().as_str().to_string());
                {
                    let mut s = self.inner.session.write().await;
                    if let Some(cur) = &mut s.current {
                        cur.server.connections = server.connections.clone();
                        cur.client = Some(client.clone());
                    }
                    s.cache.record_winner(&server.machine_id, &p);
                    let _ = s.cache.save(&self.inner.data_dir);
                }
                self.inner.health.lock().on_probe(
                    ProbeOutcome::Ok {
                        connection: p.kind,
                        latency_ms: p.latency.as_millis().min(u32::MAX as u128) as u32,
                        version: p.version.clone(),
                    },
                    now_ms(),
                );
                if changed {
                    self.start_notifications(client);
                    self.emit_auth().await;
                }
            }
            None => {
                {
                    let mut s = self.inner.session.write().await;
                    if let Some(cur) = &mut s.current {
                        cur.client = None;
                    }
                }
                self.inner
                    .health
                    .lock()
                    .on_rediscover_failed(now_ms(), internet);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::keychain::MemoryStore;
    use crate::plex::types::{ConnectionKind, HealthStatus};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ACCT: &str = "account-secret-token";

    #[derive(Default)]
    struct Recorder(Mutex<Vec<(String, serde_json::Value)>>);
    impl EventSink for Recorder {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.0.lock().push((event.to_string(), payload));
        }
    }
    impl Recorder {
        fn names(&self) -> Vec<String> {
            self.0.lock().iter().map(|(n, _)| n.clone()).collect()
        }
        fn toasts(&self) -> Vec<String> {
            self.0
                .lock()
                .iter()
                .filter(|(n, _)| n == EVENT_TOAST)
                .map(|(_, v)| v["message"].as_str().unwrap_or("").to_string())
                .collect()
        }
        fn all_text(&self) -> String {
            format!("{:?}", self.0.lock())
        }
    }

    fn cfg() -> RaceConfig {
        RaceConfig {
            timeout: Duration::from_millis(800),
            grace: Duration::from_millis(100),
        }
    }

    async fn pms(machine: &str, respect_tags: bool) -> MockServer {
        // Not pooled: dropping it really stops the listener.
        let s = MockServer::builder().start().await;
        Mock::given(method("GET"))
            .and(path("/identity"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {"machineIdentifier": machine, "version": "1.43.4"}
            })))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/"))
            .and(header("x-plex-token", ACCT))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {"machineIdentifier": machine, "version": "1.43.4", "myPlexSubscription": true}
            })))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {"Directory": [{"key": "3", "type": "artist", "title": "Music"}]}
            })))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/3/prefs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {"Setting": [{"id": "respectTags", "value": respect_tags}]}
            })))
            .mount(&s)
            .await;
        s
    }

    fn resource(machine: &str, uri: &str) -> serde_json::Value {
        serde_json::json!({
            "name": "Home", "product": "Plex Media Server", "productVersion": "1.43.4",
            "clientIdentifier": machine, "provides": "server", "owned": true, "presence": true,
            "accessToken": ACCT,
            "connections": [{"protocol": "http", "uri": uri, "local": true, "relay": false}]
        })
    }

    async fn plex_tv(resources: serde_json::Value) -> MockServer {
        // Not pooled, so a test can take plex.tv down by dropping it.
        let tv = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/pins"))
            .respond_with(ResponseTemplate::new(201).set_body_json(
                serde_json::json!({"id": 1, "code": "CODE", "expiresAt": "2026-10-13T12:00:00Z"}),
            ))
            .mount(&tv)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/pins/1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": 1, "code": "CODE", "authToken": ACCT})),
            )
            .mount(&tv)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"username": "listener", "thumb": "https://plex.tv/users/abc/avatar?c=7"})),
            )
            .mount(&tv)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/resources"))
            .respond_with(ResponseTemplate::new(200).set_body_json(resources))
            .mount(&tv)
            .await;
        tv
    }

    fn state(
        dir: &std::path::Path,
        tv: &str,
        store: Arc<MemoryStore>,
    ) -> (PlexState, Arc<Recorder>) {
        let st = PlexState::with_parts(dir.to_path_buf(), store, tv, cfg()).unwrap();
        let rec = Arc::new(Recorder::default());
        st.set_event_sink(rec.clone());
        (st, rec)
    }

    async fn wait_signed_in(st: &PlexState) -> AuthStatus {
        for _ in 0..100 {
            let a = st.auth_status().await;
            if a.state == AuthState::SignedIn && a.server.is_some() {
                return a;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        panic!("never signed in: {:?}", st.auth_status().await);
    }

    #[tokio::test]
    async fn full_sign_in_connects_and_warns_about_settings() {
        let server = pms("m1", false).await;
        let tv = plex_tv(serde_json::json!([resource("m1", &server.uri())])).await;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let (st, rec) = state(dir.path(), &tv.uri(), store.clone());

        let mut opened = String::new();
        let pin = st
            .start_sign_in_with(
                |u| {
                    opened = u.to_string();
                    Ok(())
                },
                Duration::from_millis(10),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_eq!(pin.code, "CODE");
        assert_eq!(opened, pin.auth_url);
        assert!(!pin.auth_url.contains(ACCT));

        let a = wait_signed_in(&st).await;
        assert_eq!(a.username.as_deref(), Some("listener"));
        assert_eq!(a.avatar.as_deref(), Some("/ext/https://plex.tv/users/abc/avatar?c=7"));
        let srv = a.server.unwrap();
        assert_eq!(srv.machine_id, "m1");
        assert_eq!(srv.connection, Some(ConnectionKind::Local));
        assert!(!a.needs_server_pick);
        assert_eq!(store.get(ACCOUNT_TOKEN).unwrap().unwrap().expose(), ACCT);
        assert_eq!(st.health().status, HealthStatus::Local);
        assert!(st.client().await.is_ok());

        // settings warning toast arrives in the background
        for _ in 0..100 {
            if !rec.toasts().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            rec.toasts()
                .iter()
                .any(|t| t.contains("prefer local metadata"))
        );
        assert!(rec.names().iter().any(|n| n == EVENT_AUTH_CHANGED));
        assert!(rec.names().iter().any(|n| n == EVENT_HEALTH_CHANGED));
        // the token never shows up in any emitted payload
        assert!(!rec.all_text().contains(ACCT));
        // the disk cache has no token
        let raw = std::fs::read_to_string(DiscoveryCache::path(dir.path())).unwrap();
        assert!(!raw.contains(ACCT));
        assert!(!format!("{st:?}").contains(ACCT));

        st.sign_out().await.unwrap();
        assert_eq!(store.get(ACCOUNT_TOKEN).unwrap(), None);
        assert_eq!(st.auth_status().await.state, AuthState::SignedOut);
        assert_eq!(st.health().status, HealthStatus::NoAuth);
        assert_eq!(st.client().await.unwrap_err().kind(), "auth");
    }

    #[tokio::test]
    async fn sign_in_can_be_cancelled() {
        let tv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/pins"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": 1, "code": "C"})),
            )
            .mount(&tv)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/pins/1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 1, "code": "C"})),
            )
            .mount(&tv)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let (st, _rec) = state(dir.path(), &tv.uri(), Arc::new(MemoryStore::new()));
        st.start_sign_in_with(
            |_| Ok(()),
            Duration::from_millis(20),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert_eq!(st.auth_status().await.state, AuthState::Pending);
        st.cancel_sign_in();
        assert_eq!(st.auth_status().await.state, AuthState::SignedOut);
    }

    #[tokio::test]
    async fn early_requests_wait_for_the_saved_session() {
        let server = pms("m1", true).await;
        let tv = plex_tv(serde_json::json!([resource("m1", &server.uri())])).await;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        store
            .set(ACCOUNT_TOKEN, &Token::new(ACCT).unwrap())
            .unwrap();
        let (st, _rec) = state(dir.path(), &tv.uri(), store);
        st.restore_in_background();
        // A plain client() this early fails; the startup-aware one waits.
        let c = st.client_when_ready(Duration::from_secs(10)).await;
        assert!(c.is_ok(), "{:?}", c.err());
    }

    #[tokio::test]
    async fn waiting_without_a_restore_returns_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let (st, _rec) = state(dir.path(), "http://127.0.0.1:9", Arc::new(MemoryStore::new()));
        let t = std::time::Instant::now();
        let c = st.client_when_ready(Duration::from_secs(10)).await;
        assert!(matches!(c, Err(AppError::Auth(_))));
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn restore_with_plex_tv_down_uses_cached_server() {
        let server = pms("m1", true).await;
        let dir = tempfile::tempdir().unwrap();
        // previous run cached the server
        let mut cache = DiscoveryCache::default();
        cache.servers.push(discovery::CachedServer {
            machine_id: "m1".into(),
            name: "Home".into(),
            owned: true,
            version: Some("1.43.4".into()),
            connections: vec![discovery::Connection {
                uri: server.uri(),
                kind: ConnectionKind::Local,
            }],
            last_uri: Some(server.uri()),
            last_kind: Some(ConnectionKind::Local),
        });
        cache.selected = Some("m1".into());
        cache.settings_checked.push("m1".into());
        cache.save(dir.path()).unwrap();
        let store = Arc::new(MemoryStore::new());
        store
            .set(ACCOUNT_TOKEN, &Token::new(ACCT).unwrap())
            .unwrap();
        // plex.tv: nothing listening
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = format!("http://{}", l.local_addr().unwrap());
        drop(l);
        let (st, _rec) = state(dir.path(), &dead, store);
        let a = st.restore().await.unwrap();
        assert_eq!(a.state, AuthState::SignedIn);
        assert_eq!(a.server.unwrap().machine_id, "m1");
        let h = st.health();
        assert_eq!(h.status, HealthStatus::Local);
        assert!(h.banner.is_none());
        st.sign_out().await.unwrap();
    }

    #[tokio::test]
    async fn revoked_token_on_restore_signs_out() {
        let tv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/resources"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&tv)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        store
            .set(ACCOUNT_TOKEN, &Token::new(ACCT).unwrap())
            .unwrap();
        let (st, _rec) = state(dir.path(), &tv.uri(), store.clone());
        let err = st.restore().await.unwrap_err();
        assert_eq!(err.kind(), "auth");
        assert_eq!(store.get(ACCOUNT_TOKEN).unwrap(), None);
        let a = st.auth_status().await;
        assert_eq!(a.state, AuthState::SignedOut);
        // The app stays open with a sign-in banner, not the welcome screen.
        assert!(a.reauth_required);
        let h = st.health();
        assert_eq!(h.banner.unwrap().message, "Please sign in again.");
        assert!(h.cached_only);
        // Signing out for real clears it.
        st.sign_out().await.unwrap();
        assert!(!st.auth_status().await.reauth_required);
        assert!(st.health().banner.is_none());
    }

    #[tokio::test]
    async fn access_removed_when_server_disappears() {
        let other = pms("m2", true).await;
        let tv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/resources"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                resource("m2", &other.uri()),
                resource("m3", &other.uri())
            ])))
            .mount(&tv)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&tv)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = DiscoveryCache {
            selected: Some("gone".into()),
            ..Default::default()
        };
        cache.save(dir.path()).unwrap();
        let store = Arc::new(MemoryStore::new());
        store
            .set(ACCOUNT_TOKEN, &Token::new(ACCT).unwrap())
            .unwrap();
        let (st, _rec) = state(dir.path(), &tv.uri(), store);
        let a = st.restore().await.unwrap();
        assert!(a.needs_server_pick);
        assert!(a.server.is_none());
        let h = st.health();
        assert_eq!(h.banner.unwrap().kind, "accessRemoved");
        assert_eq!(st.servers().await.len(), 2);
    }

    #[tokio::test]
    async fn offline_server_then_health_tick() {
        // resources list a server nobody answers for
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = format!("http://{}", l.local_addr().unwrap());
        drop(l);
        let tv = plex_tv(serde_json::json!([resource("m1", &dead)])).await;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        store
            .set(ACCOUNT_TOKEN, &Token::new(ACCT).unwrap())
            .unwrap();
        let (st, _rec) = state(dir.path(), &tv.uri(), store);
        let a = st.restore().await.unwrap();
        let srv = a.server.unwrap();
        assert!(!srv.online);
        assert_eq!(st.health().status, HealthStatus::Offline);
        assert_eq!(st.client().await.unwrap_err().kind(), "network");
        st.health_tick().await;
        assert_eq!(st.health().status, HealthStatus::Offline);
        st.sign_out().await.unwrap();
    }

    #[tokio::test]
    async fn health_tick_detects_revoked_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/identity"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {"machineIdentifier": "m1"}
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let tv = plex_tv(serde_json::json!([resource("m1", &server.uri())])).await;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        store
            .set(ACCOUNT_TOKEN, &Token::new(ACCT).unwrap())
            .unwrap();
        let (st, _rec) = state(dir.path(), &tv.uri(), store.clone());
        st.restore().await.unwrap();
        st.health_tick().await;
        let a = st.auth_status().await;
        assert_eq!(a.state, AuthState::SignedOut);
        assert!(a.reauth_required);
        assert_eq!(st.health().banner.unwrap().kind, "signIn");
        assert_eq!(store.get(ACCOUNT_TOKEN).unwrap(), None);
        // No more server calls while waiting for sign-in.
        assert!(st.client().await.is_err());
    }

    async fn connected(server: &MockServer) -> (PlexState, Arc<Recorder>, tempfile::TempDir, MockServer) {
        let tv = plex_tv(serde_json::json!([resource("m1", &server.uri())])).await;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        store
            .set(ACCOUNT_TOKEN, &Token::new(ACCT).unwrap())
            .unwrap();
        let (st, rec) = state(dir.path(), &tv.uri(), store);
        st.restore().await.unwrap();
        assert_eq!(st.health().status, HealthStatus::Local);
        (st, rec, dir, tv)
    }

    #[tokio::test]
    async fn server_back_after_offline_is_announced() {
        let server = pms("m1", true).await;
        let (st, rec, _d, _tv) = connected(&server).await;
        st.inner.health.lock().on_rediscover_failed(now_ms(), true);
        st.emit_health();
        assert_eq!(st.health().banner.unwrap().kind, "offline");
        assert!(st.health().cached_only);
        st.health_tick().await;
        assert_eq!(st.health().status, HealthStatus::Local);
        assert!(rec.toasts().iter().any(|t| t == "Your server is back."));
        st.sign_out().await.unwrap();
    }

    #[tokio::test]
    async fn going_offline_stops_server_calls_and_coming_back_reconnects() {
        let server = pms("m1", true).await;
        let (st, _rec, _d, _tv) = connected(&server).await;
        st.set_offline_by_choice(true);
        assert!(st.offline_by_choice());
        assert!(st.client().await.is_err(), "no server calls while offline");
        assert!(st.health().cached_only);
        assert_eq!(st.health().banner.unwrap().kind, "goneOffline");
        // Requests made so far; a health tick sends none.
        let before = server.received_requests().await.unwrap_or_default().len();
        st.health_tick().await;
        assert_eq!(server.received_requests().await.unwrap_or_default().len(), before);
        st.set_offline_by_choice(false);
        assert!(!st.offline_by_choice());
        st.health_tick().await;
        st.health_tick().await;
        assert_eq!(st.health().status, HealthStatus::Local);
        assert!(st.client().await.is_ok());
        st.sign_out().await.unwrap();
    }

    #[tokio::test]
    async fn computer_offline_says_you_are_offline() {
        // Server and plex.tv both unreachable: the problem is this computer.
        let server = pms("m1", true).await;
        let (st, _rec, _d, tv) = connected(&server).await;
        drop(server);
        drop(tv);
        st.health_tick().await;
        let h = st.health();
        assert_eq!(h.status, HealthStatus::Offline);
        assert_eq!(h.banner.unwrap().message, "You're offline.");
        st.sign_out().await.unwrap();
    }

    #[tokio::test]
    async fn server_down_with_internet_says_server_offline() {
        let server = pms("m1", true).await;
        let (st, _rec, _d, _tv) = connected(&server).await;
        drop(server);
        st.health_tick().await;
        let b = st.health().banner.unwrap();
        assert_eq!(b.kind, "offline");
        assert!(b.message.starts_with("Your server is offline. Last seen"));
        st.sign_out().await.unwrap();
    }

    #[tokio::test]
    async fn network_lost_is_immediate_and_reconnect_recovers() {
        let server = pms("m1", true).await;
        let (st, _rec, _d, _tv) = connected(&server).await;
        st.on_network_lost();
        assert_eq!(st.health().banner.unwrap().kind, "noInternet");
        // network back (or woke from sleep): reconnect pings right away
        st.reconnect().await;
        st.health_tick().await;
        assert_eq!(st.health().status, HealthStatus::Local);
        assert!(st.health().banner.is_none());
        st.sign_out().await.unwrap();
    }

    #[tokio::test]
    async fn scan_progress_from_server_events() {
        let server = pms("m1", true).await;
        let (st, _rec, _d, _tv) = connected(&server).await;
        st.on_plex_event(&PlexEvent::Activity {
            event: "started".into(),
            uuid: "u1".into(),
            kind: "library.update.section".into(),
            title: None,
            subtitle: None,
            progress: Some(0),
            section_id: Some("3".into()),
        });
        for i in 0..4 {
            st.on_plex_event(&PlexEvent::LibraryUpdate {
                section_id: Some("3".into()),
                item_id: Some(format!("{i}")),
                item_type: Some(10),
                state: Some(0),
            });
        }
        st.on_plex_event(&PlexEvent::LibraryUpdate {
            section_id: Some("3".into()),
            item_id: Some("0".into()),
            item_type: Some(10),
            state: Some(5),
        });
        let scan = st.health().scan.unwrap();
        assert_eq!((scan.done, scan.total), (1, 4));
        st.on_plex_event(&PlexEvent::Activity {
            event: "ended".into(),
            uuid: "u1".into(),
            kind: "library.update.section".into(),
            title: None,
            subtitle: None,
            progress: Some(100),
            section_id: Some("3".into()),
        });
        assert!(st.health().scan.is_none());
        st.sign_out().await.unwrap();
    }

    #[tokio::test]
    async fn files_unavailable_and_back() {
        let server = pms("m1", true).await;
        let (st, _rec, _d, _tv) = connected(&server).await;
        st.set_files_available(false);
        let h = st.health();
        assert_eq!(h.banner.unwrap().kind, "filesUnavailable");
        assert!(h.cached_only);
        st.set_files_available(true);
        assert!(st.health().banner.is_none());
        st.sign_out().await.unwrap();
    }

    #[tokio::test]
    async fn health_tick_rediscovers_new_address() {
        // The server moves: plex.tv first lists A, A dies, plex.tv lists B.
        let a = pms("m1", true).await;
        let b = pms("m1", true).await;
        let tv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/resources"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([resource("m1", &a.uri())])),
            )
            .up_to_n_times(1)
            .mount(&tv)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/resources"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([resource("m1", &b.uri())])),
            )
            .mount(&tv)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        store
            .set(ACCOUNT_TOKEN, &Token::new(ACCT).unwrap())
            .unwrap();
        let (st, _rec) = state(dir.path(), &tv.uri(), store);
        st.restore().await.unwrap();
        let before = st.client().await.unwrap().base_url().clone();
        drop(a); // address A stops answering
        st.health_tick().await;
        let after = st.client().await.unwrap().base_url().clone();
        assert_ne!(before, after);
        assert_eq!(after.as_str().trim_end_matches('/'), b.uri());
        let h = st.health();
        assert_eq!(h.status, HealthStatus::Local);
        assert!(h.banner.is_none());
        st.sign_out().await.unwrap();
    }
}
