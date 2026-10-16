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
            // Once a minute is plenty: syncs run minutes apart at most.
            tokio::time::sleep(Duration::from_secs(60)).await;
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

/// "Let them listen from my server" for someone in the session you host.
#[tauri::command]
pub async fn listen_offer_server(listen: State<'_, ListenAlong>, person_id: String) -> AppResult<ListenView> {
    listen.offer_server(&person_id).await
}

// ---- sharing ----------------------------------------------------------------------

use super::sharing::links::{LinkResult, LinkView};
use super::sharing::{Accepted, PersonWithAccess, ShareOptions, ShareStatus, Sharing, SharingOverview};
use crate::plex::sharing::ReceivedInvite;

/// Settings, Sharing: my server's music libraries, their unreleased albums,
/// and Plex Pass.
#[tauri::command]
pub async fn sharing_overview(sharing: State<'_, Sharing>) -> AppResult<SharingOverview> {
    sharing.overview().await
}

/// Share my music with a Plex username or email.
#[tauri::command]
pub async fn sharing_share(sharing: State<'_, Sharing>, invitee: String, options: ShareOptions) -> AppResult<ShareStatus> {
    sharing.share(&invitee, &options).await
}

#[tauri::command]
pub async fn sharing_people(sharing: State<'_, Sharing>) -> AppResult<Vec<PersonWithAccess>> {
    sharing.people().await
}

#[tauri::command]
pub async fn sharing_remove(sharing: State<'_, Sharing>, id: String) -> AppResult<()> {
    sharing.remove(&id).await
}

/// A link or QR code that lets someone ask for my music.
#[tauri::command]
pub async fn sharing_link(sharing: State<'_, Sharing>, settings: State<'_, crate::settings::SettingsState>, options: ShareOptions) -> AppResult<LinkView> {
    let hours = settings.get().await?.listen_invite_hours;
    sharing.make_link(options, hours).await
}

/// Open a share link someone gave me.
#[tauri::command]
pub async fn sharing_open_link(sharing: State<'_, Sharing>, listen_name: State<'_, ListenName>, plex: State<'_, PlexState>, link: String) -> AppResult<LinkResult> {
    listen_name.refresh(&plex).await;
    let name = listen_name.0.lock().clone();
    sharing.open_link(&link, &name).await
}

/// Server invites I received (`fresh` asks plex.tv).
#[tauri::command]
pub async fn sharing_invites(sharing: State<'_, Sharing>, fresh: Option<bool>) -> AppResult<Vec<ReceivedInvite>> {
    sharing.invites(fresh.unwrap_or(true)).await
}

#[tauri::command]
pub async fn sharing_accept(sharing: State<'_, Sharing>, id: String) -> AppResult<Accepted> {
    sharing.accept(&id).await
}

/// Look for invites now and then (and tell listen along about shared
/// libraries once they're read).
pub fn spawn_sharing(sharing: Sharing, plex: PlexState, listen: ListenAlong, db: DbHandle) {
    let l = listen.clone();
    sharing.set_on_accepted(Arc::new(move |entries: Vec<String>| {
        let (l, db) = (l.clone(), db.clone());
        tauri::async_runtime::spawn(async move {
            // Wait (up to 20 min) for the new libraries' first sync.
            let srcs: Vec<String> = entries
                .iter()
                .filter_map(|e| crate::libraries::parse_entry(e))
                .map(|(m, s)| crate::db::store::src_of(&crate::sid::tag(m), s))
                .collect();
            for _ in 0..240 {
                let s2 = srcs.clone();
                let read = db.read_async(move |c| Ok(s2.iter().any(|s| crate::db::store::sync_state_in(c, s).ok().flatten().is_some_and(|st| st.last_sync_at.is_some())))).await.unwrap_or(false);
                if read {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            l.library_changed().await;
        });
    }));
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            if plex.account_token().await.is_some() && !plex.offline_by_choice() {
                let _ = sharing.invites(true).await;
            }
            listen.settings_changed().await;
            tokio::time::sleep(Duration::from_secs(15 * 60)).await;
        }
    });
}
