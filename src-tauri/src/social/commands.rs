//! Tauri commands, and the glue to Plex, the player and notifications.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tauri::{AppHandle, Runtime, State};
use tokio::sync::watch;

use super::activity::{ActivityPanel, ActivitySource};
use super::follows::{FollowedArtist, WhatsNew};
use super::listen::{ListenAlong, ListenPlayer, ListenView, PlayerNow};
use super::{Notifier, Social};
use crate::audio::events::PlayerStatus;
use crate::audio::player::PlayerHandle;
use crate::audio::queue::AddAt;
use crate::audio::source::{ContextKind, PlayContext};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::plex::PlexState;
use crate::share::link::server_tag;

// ---- Plex -------------------------------------------------------------------

/// Activity from the connected server.
pub struct PlexActivity {
    pub plex: PlexState,
    pub db: DbHandle,
}

#[async_trait]
impl ActivitySource for PlexActivity {
    async fn owned(&self) -> Option<bool> {
        self.plex.client().await.ok()?;
        self.plex.auth_status().await.server.map(|s| s.owned)
    }

    async fn sessions(&self) -> AppResult<Vec<u8>> {
        Ok(self.plex.client().await?.get_bytes("/status/sessions", &[]).await?.to_vec())
    }

    async fn history(&self, size: u64) -> AppResult<Vec<u8>> {
        let section = self
            .db
            .read_async(crate::db::store::sync_state)
            .await?
            .map(|s| s.section_id)
            .ok_or_else(|| AppError::Invalid("no music library yet".into()))?;
        let q = [
            ("librarySectionID", section),
            ("sort", "viewedAt:desc".to_string()),
            ("X-Plex-Container-Start", "0".to_string()),
            ("X-Plex-Container-Size", size.to_string()),
        ];
        Ok(self.plex.client().await?.get_bytes("/status/sessions/history/all", &q).await?.to_vec())
    }

    async fn accounts(&self) -> AppResult<Vec<u8>> {
        Ok(self.plex.client().await?.get_bytes("/accounts", &[]).await?.to_vec())
    }
}

// ---- the player -------------------------------------------------------------

#[async_trait]
impl ListenPlayer for PlayerHandle {
    async fn now(&self) -> AppResult<PlayerNow> {
        let s = self.state().await?;
        let up_next = s.queue.user_queue.iter().chain(&s.queue.next_up).map(|i| i.track.id.clone()).collect();
        Ok(PlayerNow {
            track_id: s.track.as_ref().map(|t| t.id.clone()),
            position_ms: s.position_ms,
            duration_ms: s.duration_ms,
            playing: matches!(s.status, PlayerStatus::Playing | PlayerStatus::Loading | PlayerStatus::Buffering),
            up_next,
        })
    }

    async fn play(&self, ids: Vec<String>, position_ms: u64, play: bool) -> AppResult<()> {
        let start = ids.first().cloned();
        let ctx = PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(ids) };
        self.play_context_at(ctx, start, position_ms, play).await
    }

    async fn seek(&self, position_ms: u64) -> AppResult<()> {
        PlayerHandle::seek(self, position_ms).await
    }

    async fn pause(&self) -> AppResult<()> {
        PlayerHandle::pause(self).await
    }

    async fn resume(&self) -> AppResult<()> {
        PlayerHandle::resume(self).await
    }

    async fn queue_add(&self, ids: Vec<String>) -> AppResult<()> {
        PlayerHandle::queue_add(self, ids, AddAt::End).await
    }
}

// ---- notifications ------------------------------------------------------------

pub struct OsNotifier<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> Notifier for OsNotifier<R> {
    fn notify(&self, title: &str, body: &str) {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = self.0.notification().builder().title(title).body(body).show() {
            tracing::debug!("new release notification failed: {e}");
        }
    }
}

// ---- background -----------------------------------------------------------------

/// Look for new releases whenever a library sync finishes.
pub fn spawn_watch(social: Social, db: DbHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last: Option<i64> = None;
        // The first look sets the marks and the badge.
        social.library_changed();
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let at = db.read_async(crate::db::store::sync_state).await.ok().flatten().and_then(|s| s.last_sync_at);
            if at.is_some() && at != last {
                if last.is_some() {
                    social.library_changed();
                }
                last = at;
            }
        }
    });
}

/// The control API's addresses while it listens (hosting needs them).
pub struct ListenAddrs {
    pub port: watch::Receiver<Option<u16>>,
}

impl ListenAddrs {
    pub fn now(&self) -> Vec<SocketAddr> {
        let Some(port) = *self.port.borrow() else { return Vec::new() };
        crate::remote::peer::lan_addrs().into_iter().map(|ip| SocketAddr::from((ip, port))).collect()
    }
}

/// The signed-in user's name, for others in a session.
#[derive(Clone, Default)]
pub struct ListenName(pub Arc<Mutex<String>>);

impl ListenName {
    async fn refresh(&self, plex: &PlexState) {
        if let Some(n) = plex.auth_status().await.username.filter(|n| !n.trim().is_empty()) {
            *self.0.lock() = n;
        }
    }
}

async fn tag(plex: &PlexState) -> Option<String> {
    plex.machine_id().await.map(|m| server_tag(&m))
}

// ---- commands -----------------------------------------------------------------

#[tauri::command]
pub async fn friend_activity(social: State<'_, Social>, fresh: Option<bool>) -> AppResult<ActivityPanel> {
    Ok(social.activity(fresh.unwrap_or(false)).await)
}

#[tauri::command]
pub async fn follows_list(social: State<'_, Social>) -> AppResult<Vec<FollowedArtist>> {
    let s = social.inner().clone();
    tokio::task::spawn_blocking(move || s.follows()).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn follow_set(
    social: State<'_, Social>,
    id: String,
    title: String,
    thumb: Option<String>,
    on: bool,
) -> AppResult<Vec<FollowedArtist>> {
    let s = social.inner().clone();
    tokio::task::spawn_blocking(move || s.set_follow(id, title, thumb, on))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn whats_new(social: State<'_, Social>) -> AppResult<WhatsNew> {
    let s = social.inner().clone();
    tokio::task::spawn_blocking(move || s.whats_new()).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn whats_new_seen(social: State<'_, Social>) -> AppResult<()> {
    let s = social.inner().clone();
    tokio::task::spawn_blocking(move || s.whats_new_seen()).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn listen_state(listen: State<'_, ListenAlong>) -> AppResult<ListenView> {
    Ok(listen.view())
}

#[tauri::command]
pub async fn listen_start(
    listen: State<'_, ListenAlong>,
    addrs: State<'_, ListenAddrs>,
    name: State<'_, ListenName>,
    plex: State<'_, PlexState>,
) -> AppResult<ListenView> {
    name.refresh(&plex).await;
    listen.start(tag(&plex).await, addrs.now())
}

#[tauri::command]
pub async fn listen_stop(listen: State<'_, ListenAlong>) -> AppResult<()> {
    listen.stop().await
}

#[tauri::command]
pub async fn listen_join(
    listen: State<'_, ListenAlong>,
    name: State<'_, ListenName>,
    plex: State<'_, PlexState>,
    link: String,
) -> AppResult<ListenView> {
    name.refresh(&plex).await;
    listen.join(&link, tag(&plex).await).await
}

#[tauri::command]
pub async fn listen_add(listen: State<'_, ListenAlong>, track_ids: Vec<String>) -> AppResult<()> {
    listen.add(track_ids).await
}
