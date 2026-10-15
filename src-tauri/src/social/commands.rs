//! Tauri commands, and the glue to Plex, the player and notifications.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tauri::{AppHandle, Runtime, State};

use super::activity::{ActivityPanel, ActivitySource};
use super::follows::{FollowedArtist, WhatsNew};
use super::listen::wire::Op;
use super::listen::matching::{self, Found};
use super::listen::wire::Song;
use super::listen::{ListenAlong, ListenLibrary, ListenPlayer, ListenView, PlayerNow, ServerId};
use super::{Notifier, Social};
use crate::audio::events::PlayerStatus;
use crate::audio::player::PlayerHandle;
use crate::audio::queue::AddAt;
use crate::audio::source::{ContextKind, PlayContext};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::plex::PlexState;

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
        let c = self.plex.client().await?;
        Ok(c.qualify_json(&c.get_bytes("/status/sessions", &[]).await?))
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
        let c = self.plex.client().await?;
        Ok(c.qualify_json(&c.get_bytes("/status/sessions/history/all", &q).await?))
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
        let after = || s.queue.user_queue.iter().chain(&s.queue.next_up);
        Ok(PlayerNow {
            track_id: s.track.as_ref().map(|t| t.id.clone()),
            position_ms: s.position_ms,
            duration_ms: s.duration_ms,
            playing: matches!(s.status, PlayerStatus::Playing | PlayerStatus::Loading | PlayerStatus::Buffering),
            user_queue: s.queue.user_queue.iter().map(|i| i.track.id.clone()).collect(),
            up_next: after().map(|i| i.track.id.clone()).collect(),
            up_next_items: after().map(|i| i.item_id.clone()).collect(),
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

    async fn next(&self) -> AppResult<()> {
        PlayerHandle::next(self).await
    }

    async fn previous(&self) -> AppResult<()> {
        PlayerHandle::previous(self).await
    }

    async fn queue_add(&self, ids: Vec<String>) -> AppResult<()> {
        PlayerHandle::queue_add(self, ids, AddAt::End).await
    }

    async fn queue_clear(&self) -> AppResult<()> {
        PlayerHandle::queue_clear(self).await
    }

    async fn queue_move(&self, item_id: String, to_index: usize) -> AppResult<()> {
        PlayerHandle::queue_move(self, item_id, to_index).await
    }
}

/// The connected server's machine id (it never leaves this computer; a
/// session hashes it).
pub struct PlexServerId(pub PlexState);

#[async_trait]
impl ServerId for PlexServerId {
    async fn machine_id(&self) -> Option<String> {
        self.0.machine_id().await
    }
}

/// Songs from the library cache: who they are (the host's side) and their
/// copies here (a guest's).
pub struct DbListenLibrary(pub DbHandle);

#[async_trait]
impl ListenLibrary for DbListenLibrary {
    async fn songs(&self, ids: Vec<String>) -> std::collections::HashMap<String, Song> {
        match self.0.read_async(move |c| matching::describe(c, &ids)).await {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(kind = e.kind(), "listen along couldn't describe songs");
                Default::default()
            }
        }
    }

    async fn has(&self, id: &str) -> bool {
        let id = id.to_string();
        self.0.read_async(move |c| matching::has(c, &id)).await.unwrap_or(false)
    }

    async fn find(&self, song: &Song) -> Option<Found> {
        let song = song.clone();
        self.0.read_async(move |c| matching::find(c, &song)).await.ok().flatten()
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

/// The signed-in user's Plex display name, for others in a session (unless
/// the user set another in Settings).
#[derive(Clone, Default)]
pub struct ListenName(pub Arc<Mutex<String>>);

impl ListenName {
    async fn refresh(&self, plex: &PlexState) {
        if let Some(n) = plex.auth_status().await.username.filter(|n| !n.trim().is_empty()) {
            *self.0.lock() = n;
        }
    }
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
pub async fn listen_start(listen: State<'_, ListenAlong>, name: State<'_, ListenName>, plex: State<'_, PlexState>) -> AppResult<ListenView> {
    name.refresh(&plex).await;
    listen.start().await
}

/// The invite to send; `fresh` makes a new link.
#[tauri::command]
pub async fn listen_invite(listen: State<'_, ListenAlong>, fresh: Option<bool>) -> AppResult<ListenView> {
    listen.invite(fresh.unwrap_or(false))
}

#[tauri::command]
pub async fn listen_stop(listen: State<'_, ListenAlong>) -> AppResult<()> {
    listen.stop().await
}

#[tauri::command]
pub async fn listen_join(listen: State<'_, ListenAlong>, name: State<'_, ListenName>, plex: State<'_, PlexState>, link: String) -> AppResult<ListenView> {
    name.refresh(&plex).await;
    listen.join(&link).await
}

#[tauri::command]
pub async fn listen_add(listen: State<'_, ListenAlong>, track_ids: Vec<String>) -> AppResult<()> {
    listen.add(track_ids).await
}

/// Play, pause, skip, seek or move a song for everyone (a guest's go to the host).
#[tauri::command]
pub async fn listen_control(listen: State<'_, ListenAlong>, op: Op) -> AppResult<()> {
    listen.control(op).await
}

/// Remove someone from the session you host.
#[tauri::command]
pub async fn listen_remove(listen: State<'_, ListenAlong>, person_id: String) -> AppResult<ListenView> {
    listen.remove(&person_id).await
}
