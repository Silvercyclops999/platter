//! Tauri commands for Listen on (docs/IPC.md), plus the app's bindings of
//! the service to Plex and the local player.

use std::net::Ipv4Addr;
use std::sync::Arc;

use async_trait::async_trait;
use tauri::State;
use tokio::sync::watch;

#[cfg(not(target_os = "ios"))]
use super::peer::directory::MdnsPeers;
use super::peer::directory::PeerDirectory;
use super::peer::runtime::{self, RuntimeConfig};
use super::peer::{self};
use super::service::{LocalPlayer, RemoteEnv, RemoteService, ServerCtx};
use super::target::{self, PlayerTarget, TargetInfo};
use super::types::{RemoteCommand, RemotePlayer, RemoteState};
use crate::audio::events::PlayerState;
use crate::audio::player::PlayerHandle;
use crate::audio::queue::{ContextInfo, Repeat};
use crate::audio::source::{ContextKind, PlayContext};
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::plex::{PlexState, Token};

/// Plex players Platter can play on (empty when the setting hides them).
#[tauri::command]
pub async fn remote_players(remote: State<'_, RemoteService>) -> AppResult<Vec<RemotePlayer>> {
    Ok(remote.players().await)
}

/// Playback on another device, if any.
#[tauri::command]
pub async fn remote_state(remote: State<'_, RemoteService>) -> AppResult<RemoteState> {
    Ok(remote.state())
}

/// Move playback to a player at the current position.
#[tauri::command]
pub async fn remote_move(remote: State<'_, RemoteService>, player_id: String) -> AppResult<RemoteState> {
    remote.move_to(&player_id).await
}

/// Bring playback back to this computer at the player's position.
#[tauri::command]
pub async fn remote_return(remote: State<'_, RemoteService>) -> AppResult<RemoteState> {
    remote.return_here().await
}

/// Play an album, playlist or songs on the device playing now (instead of
/// `play_context` while remote).
#[tauri::command]
pub async fn remote_play_context(
    remote: State<'_, RemoteService>,
    context: PlayContext,
    start_track_id: Option<String>,
) -> AppResult<RemoteState> {
    remote.play_context(context, start_track_id).await
}

/// Jump to a queue entry on the device playing now (instead of
/// `queue_jump` while remote).
#[tauri::command]
pub async fn remote_queue_jump(remote: State<'_, RemoteService>, item_id: String) -> AppResult<RemoteState> {
    remote.queue_jump(item_id).await
}

/// Move to an AirPlay receiver with its password (kept in the keychain
/// once it works).
#[tauri::command]
pub async fn remote_move_with_password(
    remote: State<'_, RemoteService>,
    player_id: String,
    password: String,
) -> AppResult<RemoteState> {
    remote.move_with_password(&player_id, password).await
}

/// A player bar control while playing on another device.
#[tauri::command]
pub async fn remote_command(remote: State<'_, RemoteService>, command: RemoteCommand) -> AppResult<RemoteState> {
    remote.command(command).await
}

// ---- bindings ---------------------------------------------------------------

/// Plex for the service: the selected server and the account token.
pub struct PlexRemoteEnv(pub PlexState, pub AccountId);

/// The account's id, looked up once per token. A failed lookup isn't
/// tried again for a minute, and callers at the same time share one lookup.
pub struct AccountId {
    /// Token key, the id (None: the lookup failed), when it was looked up.
    cache: parking_lot::Mutex<Option<(String, Option<String>, std::time::Instant)>>,
    lookup: tokio::sync::Mutex<()>,
    /// The longest one lookup may take (others wait on it).
    wait: std::time::Duration,
}

impl Default for AccountId {
    fn default() -> Self {
        Self { cache: Default::default(), lookup: Default::default(), wait: ACCOUNT_LOOKUP }
    }
}

/// How long a failed account lookup is remembered.
const ACCOUNT_RETRY: std::time::Duration = std::time::Duration::from_secs(60);
/// How long plex.tv gets to say who the account is.
const ACCOUNT_LOOKUP: std::time::Duration = std::time::Duration::from_secs(6);

impl AccountId {
    /// Another limit for one lookup (tests).
    pub fn with_wait(wait: std::time::Duration) -> Self {
        Self { wait, ..Self::default() }
    }

    fn cached(&self, key: &str) -> Option<Option<String>> {
        let g = self.cache.lock();
        let (k, id, at) = g.as_ref()?;
        (k == key && (id.is_some() || at.elapsed() < ACCOUNT_RETRY)).then(|| id.clone())
    }

    /// The id for this token key, fetched only when it isn't known.
    pub async fn get<F, Fut>(&self, key: String, fetch: F) -> Option<String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Option<String>>,
    {
        if let Some(hit) = self.cached(&key) {
            return hit;
        }
        let _one = self.lookup.lock().await;
        if let Some(hit) = self.cached(&key) {
            return hit;
        }
        // A lookup that hangs counts as failed, so nobody waits on it for good.
        let id = tokio::time::timeout(self.wait, fetch()).await.ok().flatten().filter(|id| !id.is_empty());
        if id.is_none() {
            tracing::debug!("the account lookup failed or took too long");
        }
        *self.cache.lock() = Some((key, id.clone(), std::time::Instant::now()));
        id
    }
}

impl PlexRemoteEnv {
    pub fn new(plex: PlexState) -> Self {
        Self(plex, AccountId::default())
    }
}

#[async_trait]
impl RemoteEnv for PlexRemoteEnv {
    async fn server(&self) -> AppResult<ServerCtx> {
        let client = self.0.client().await?;
        let machine_id = self
            .0
            .machine_id()
            .await
            .ok_or_else(|| AppError::Invalid("No server selected.".into()))?;
        let headers = client.auth_headers()?;
        let token = headers
            .get("x-plex-token")
            .and_then(|v| v.to_str().ok())
            .and_then(|t| Token::new(t).ok())
            .ok_or_else(|| AppError::Auth("Not signed in.".into()))?;
        Ok(ServerCtx {
            base: client.base_url().clone(),
            token,
            machine_id,
        })
    }
    async fn server_for(&self, id: &str) -> AppResult<ServerCtx> {
        let Some(tag) = crate::sid::tag_of(id) else { return self.server().await };
        if self.0.tag().await.as_deref() == Some(tag) {
            return self.server().await;
        }
        let client = self.0.client_for_tag(tag).await?;
        let machine_id = self
            .0
            .account_servers()
            .await
            .into_iter()
            .find(|s| crate::sid::tag(&s.machine_id) == tag)
            .map(|s| s.machine_id)
            .ok_or_else(|| AppError::NotFound("That server isn't on your account any more.".into()))?;
        let headers = client.auth_headers()?;
        let token = headers
            .get("x-plex-token")
            .and_then(|v| v.to_str().ok())
            .and_then(|t| Token::new(t).ok())
            .ok_or_else(|| AppError::Auth("Not signed in.".into()))?;
        Ok(ServerCtx { base: client.base_url().clone(), token, machine_id })
    }
    async fn account_token(&self) -> Option<Token> {
        self.0.account_token().await
    }
    async fn account_id(&self) -> Option<String> {
        let token = self.0.account_token().await?;
        let key = crate::remote::peer::auth::hex(&crate::remote::peer::auth::hmac_sha256(b"account-id-cache", token.expose().as_bytes()));
        let plex = &self.0;
        self.1.get(key, || async move { plex.plex_tv().user(&token).await.ok().map(|u| u.uuid) }).await
    }
}

#[async_trait]
impl LocalPlayer for PlayerHandle {
    async fn state(&self) -> AppResult<PlayerState> {
        PlayerHandle::state(self).await
    }
    async fn pause(&self) -> AppResult<()> {
        PlayerHandle::pause(self).await
    }
    async fn resume(&self) -> AppResult<()> {
        PlayerHandle::resume(self).await
    }
    async fn seek(&self, ms: u64) -> AppResult<()> {
        PlayerHandle::seek(self, ms).await
    }
    async fn next(&self) -> AppResult<()> {
        PlayerHandle::next(self).await
    }
    async fn previous(&self) -> AppResult<()> {
        PlayerHandle::previous(self).await
    }
    async fn set_volume(&self, v: f32) -> AppResult<()> {
        PlayerHandle::set_volume(self, v).await
    }
    async fn set_shuffle(&self, on: bool) -> AppResult<()> {
        PlayerHandle::set_shuffle(self, on).await
    }
    async fn set_repeat(&self, r: Repeat) -> AppResult<()> {
        PlayerHandle::set_repeat(self, r).await
    }
    async fn jump_at(&self, item_id: String, ms: u64, play: bool) -> AppResult<()> {
        self.queue_jump_at(item_id, ms, play).await
    }
    async fn play_tracks(&self, ids: Vec<String>, start: Option<String>, ms: u64, play: bool) -> AppResult<()> {
        let ctx = PlayContext {
            kind: ContextKind::Tracks,
            id: None,
            track_ids: Some(ids),
        };
        self.play_context_at(ctx, start, ms, play).await
    }
    async fn tracks(&self, ids: &[String]) -> AppResult<Vec<Track>> {
        self.lookup_tracks(ids).await
    }
    async fn load_context(&self, context: PlayContext, start_track_id: Option<String>) -> AppResult<()> {
        self.play_context_at(context, start_track_id, 0, false).await
    }
    async fn output_selected(&self) -> AppResult<Option<String>> {
        Ok(self.output_state().await?.selected)
    }
    async fn select_output(&self, id: Option<String>) -> AppResult<()> {
        PlayerHandle::select_output(self, id).await.map(|_| ())
    }
    async fn adopt(
        &self,
        context: Option<ContextInfo>,
        ids: Vec<String>,
        user: Vec<String>,
        shuffle: bool,
        repeat: Repeat,
        ms: u64,
        play: bool,
    ) -> AppResult<()> {
        self.adopt_queue(context, ids, user, shuffle, repeat, ms, play).await
    }
}

/// Show this computer to the user's other Platter devices while the
/// setting is on, and find theirs. The answer is the control API's port
/// while it listens.
pub fn spawn_peers(remote: &RemoteService, platform: String, version: String) -> watch::Receiver<Option<u16>> {
    // iOS allows Bonjour but not raw multicast from an app.
    #[cfg(target_os = "ios")]
    let dir: Arc<dyn PeerDirectory> = super::peer::bonjour::BonjourPeers::new();
    #[cfg(not(target_os = "ios"))]
    let dir: Arc<dyn PeerDirectory> = Arc::new(MdnsPeers::new());
    remote.set_peers(dir.clone());
    let (port, rx) = watch::channel(None);
    tauri::async_runtime::spawn(runtime::run(
        remote.clone(),
        dir,
        Arc::new(peer::lan_addrs),
        RuntimeConfig::app(platform, version),
        port,
    ));
    rx
}

/// Start or stop Platter-as-a-player as the setting changes.
pub fn spawn_target(remote: &RemoteService, info: TargetInfo) {
    let mut on = remote.switches().allow_casting.subscribe();
    let handler: Arc<dyn target::TargetHandler> = Arc::new(PlayerTarget {
        local: remote.local(),
        env: remote.env(),
        http: remote.http(),
    });
    let info = Arc::new(info);
    tauri::async_runtime::spawn(async move {
        let mut running: Option<watch::Sender<bool>> = None;
        loop {
            let want = *on.borrow_and_update();
            if want && running.is_none() {
                match target::bind(Ipv4Addr::UNSPECIFIED, target::FIRST_PORT, target::PORT_TRIES).await {
                    Ok(listener) => {
                        let port = listener.local_addr().map(|a| a.port()).unwrap_or(target::FIRST_PORT);
                        tracing::info!(port, "playing from other Plex apps is on");
                        let (stop, rx) = watch::channel(false);
                        tokio::spawn(target::serve(listener, handler.clone(), info.clone(), rx.clone()));
                        tokio::spawn(target::gdm(info.clone(), port, rx));
                        running = Some(stop);
                    }
                    Err(e) => tracing::warn!("couldn't open the player port: {e}"),
                }
            } else if !want && let Some(stop) = running.take() {
                let _ = stop.send(true);
                tracing::info!("playing from other Plex apps is off");
            }
            if on.changed().await.is_err() {
                if let Some(stop) = running.take() {
                    let _ = stop.send(true);
                }
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::AccountId;

    #[tokio::test]
    async fn the_account_id_is_looked_up_once_per_token() {
        let ids = AccountId::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch = |c: Arc<AtomicUsize>, answer: Option<&'static str>| {
            move || async move {
                c.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                answer.map(str::to_string)
            }
        };
        // Two refreshes at once share one lookup.
        let (a, b) = tokio::join!(
            ids.get("t1".into(), fetch(calls.clone(), Some("acct"))),
            ids.get("t1".into(), fetch(calls.clone(), Some("acct")))
        );
        assert_eq!((a.as_deref(), b.as_deref()), (Some("acct"), Some("acct")));
        assert_eq!(ids.get("t1".into(), fetch(calls.clone(), Some("other"))).await.as_deref(), Some("acct"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Another token: looked up again. Offline: not retried at once.
        assert_eq!(ids.get("t2".into(), fetch(calls.clone(), None)).await, None);
        assert_eq!(ids.get("t2".into(), fetch(calls.clone(), Some("late"))).await, None);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_hung_account_lookup_gives_up_and_frees_the_others() {
        let ids = AccountId::with_wait(std::time::Duration::from_millis(100));
        let t = std::time::Instant::now();
        let hung = || std::future::pending::<Option<String>>();
        // Two callers at once: the second waits on the first, which gives up.
        let (a, b) = tokio::join!(ids.get("t1".into(), hung), ids.get("t1".into(), hung));
        assert_eq!((a, b), (None, None));
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
        // Another token isn't held up by it either.
        let ok = || async { Some("acct".to_string()) };
        assert_eq!(ids.get("t2".into(), ok).await.as_deref(), Some("acct"));
    }
}
