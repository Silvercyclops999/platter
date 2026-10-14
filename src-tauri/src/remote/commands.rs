//! Tauri commands for Listen on (docs/IPC.md), plus the app's bindings of
//! the service to Plex and the local player.

use std::net::Ipv4Addr;
use std::sync::Arc;

use async_trait::async_trait;
use tauri::State;
use tokio::sync::watch;

use super::service::{LocalPlayer, RemoteEnv, RemoteService, ServerCtx};
use super::target::{self, PlayerTarget, TargetInfo};
use super::types::{RemoteCommand, RemotePlayer, RemoteState};
use crate::audio::events::PlayerState;
use crate::audio::player::PlayerHandle;
use crate::audio::queue::Repeat;
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

/// A player bar control while playing on another device.
#[tauri::command]
pub async fn remote_command(remote: State<'_, RemoteService>, command: RemoteCommand) -> AppResult<RemoteState> {
    remote.command(command).await
}

// ---- bindings ---------------------------------------------------------------

/// Plex for the service: the selected server and the account token.
pub struct PlexRemoteEnv(pub PlexState);

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
    async fn account_token(&self) -> Option<Token> {
        self.0.account_token().await
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
