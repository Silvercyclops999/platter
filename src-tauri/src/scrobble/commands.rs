//! Tauri commands for scrobbling (docs/IPC.md "Scrobbling").

use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::lastfm::{Lastfm, PendingAuth};
use super::listenbrainz::ListenBrainz;
use super::{ScrobbleNotice, Scrobbler, Service};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};

pub const KEY_LASTFM_USER: &str = "scrobble.lastfm.user";
const KEY_LB_USER: &str = "scrobble.listenbrainz.user";

/// Managed state behind the commands.
#[derive(Clone)]
pub struct ScrobbleState {
    pub scrobbler: Scrobbler,
    pub lastfm: Arc<Lastfm>,
    pub listenbrainz: Arc<ListenBrainz>,
    pub db: DbHandle,
    pending: Arc<Mutex<Option<PendingAuth>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastfmStatus {
    pub has_key: bool,
    pub has_secret: bool,
    pub signed_in: bool,
    pub user: Option<String>,
    /// Last.fm refused the session: sign in again.
    pub rejected: bool,
    /// A sign-in waits for the user to approve it on last.fm.
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenBrainzStatus {
    pub has_token: bool,
    pub user: Option<String>,
    pub rejected: bool,
}

/// `scrobble_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrobbleStatus {
    pub lastfm: LastfmStatus,
    pub listenbrainz: ListenBrainzStatus,
    /// Scrobbles waiting to be sent.
    pub queued: u32,
}

impl ScrobbleState {
    pub fn new(scrobbler: Scrobbler, lastfm: Arc<Lastfm>, listenbrainz: Arc<ListenBrainz>, db: DbHandle) -> Self {
        Self { scrobbler, lastfm, listenbrainz, db, pending: Default::default() }
    }

    fn kv(&self, key: &str) -> Option<String> {
        self.db.read(|c| kv_get(c, key)).ok().flatten().filter(|s| !s.is_empty())
    }

    /// Blocking (reads the keychain).
    pub fn status(&self) -> ScrobbleStatus {
        let signed_in = super::ScrobbleService::signed_in(self.lastfm.as_ref());
        let has_token = super::ScrobbleService::signed_in(self.listenbrainz.as_ref());
        ScrobbleStatus {
            lastfm: LastfmStatus {
                has_key: self.lastfm.has_key(),
                has_secret: self.lastfm.has_secret(),
                signed_in,
                user: if signed_in { self.kv(KEY_LASTFM_USER) } else { None },
                rejected: self.scrobbler.is_rejected(Service::Lastfm),
                pending: self.pending.lock().is_some(),
            },
            listenbrainz: ListenBrainzStatus {
                has_token,
                user: if has_token { self.kv(KEY_LB_USER) } else { None },
                rejected: self.scrobbler.is_rejected(Service::Listenbrainz),
            },
            queued: self.scrobbler.queued().map(|q| q.len() as u32).unwrap_or(0),
        }
    }

    /// Start the Last.fm sign-in; resolves to the page to open.
    pub async fn lastfm_start(&self) -> AppResult<String> {
        let p = self.lastfm.auth_start().await?;
        let url = p.url.clone();
        *self.pending.lock() = Some(p);
        Ok(url)
    }

    /// Finish it after the user approved on last.fm.
    pub async fn lastfm_finish(&self) -> AppResult<ScrobbleStatus> {
        let token = self
            .pending
            .lock()
            .as_ref()
            .map(|p| p.token.clone())
            .ok_or_else(|| AppError::Invalid("Start signing in to Last.fm first.".into()))?;
        let user = self.lastfm.auth_finish(&token).await?;
        *self.pending.lock() = None;
        self.db.write(|c| kv_set(c, KEY_LASTFM_USER, &user))?;
        self.scrobbler.signed_in(Service::Lastfm);
        let s = self.scrobbler.clone();
        tokio::spawn(async move { s.replay().await });
        Ok(self.status())
    }

    pub fn lastfm_sign_out(&self) -> AppResult<ScrobbleStatus> {
        self.lastfm.sign_out()?;
        *self.pending.lock() = None;
        self.db.write(|c| kv_set(c, KEY_LASTFM_USER, ""))?;
        Ok(self.status())
    }

    /// Check the ListenBrainz token (after it's saved or changed).
    pub async fn listenbrainz_check(&self) -> AppResult<ScrobbleStatus> {
        match self.listenbrainz.user().await? {
            Some(user) => {
                self.db.write(|c| kv_set(c, KEY_LB_USER, &user))?;
                self.scrobbler.signed_in(Service::Listenbrainz);
                let s = self.scrobbler.clone();
                tokio::spawn(async move { s.replay().await });
            }
            None if super::ScrobbleService::signed_in(self.listenbrainz.as_ref()) => {
                self.scrobbler.reject_now(Service::Listenbrainz);
            }
            None => {}
        }
        Ok(self.status())
    }
}

/// A toast when a service refuses the sign-in.
pub struct ToastNotice(pub crate::plex::PlexState);

impl ScrobbleNotice for ToastNotice {
    fn rejected(&self, service: Service) {
        let what = match service {
            Service::Lastfm => "Last.fm signed Platter out. Sign in again in Settings to keep scrobbling.",
            Service::Listenbrainz => "ListenBrainz didn't accept your token. Check it in Settings to keep scrobbling.",
        };
        self.0.toast("error", what);
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn scrobble_status(state: State<'_, ScrobbleState>) -> AppResult<ScrobbleStatus> {
    let s = state.inner().clone();
    blocking(move || Ok(s.status())).await
}

/// Save (or remove, with null) the Last.fm shared secret.
#[tauri::command]
pub async fn lastfm_secret_set(state: State<'_, ScrobbleState>, secret: Option<String>) -> AppResult<ScrobbleStatus> {
    let s = state.inner().clone();
    blocking(move || {
        s.lastfm.set_secret(secret.as_deref())?;
        Ok(s.status())
    })
    .await
}

/// Ask Last.fm for a sign-in and open its page in the browser.
#[tauri::command]
pub async fn lastfm_sign_in_start(app: AppHandle, state: State<'_, ScrobbleState>) -> AppResult<ScrobbleStatus> {
    let url = state.lastfm_start().await?;
    app.opener().open_url(url, None::<&str>).map_err(|e| AppError::Internal(format!("open browser: {e}")))?;
    let s = state.inner().clone();
    blocking(move || Ok(s.status())).await
}

#[tauri::command]
pub async fn lastfm_sign_in_finish(state: State<'_, ScrobbleState>) -> AppResult<ScrobbleStatus> {
    state.lastfm_finish().await
}

#[tauri::command]
pub async fn lastfm_sign_out(state: State<'_, ScrobbleState>) -> AppResult<ScrobbleStatus> {
    let s = state.inner().clone();
    blocking(move || s.lastfm_sign_out()).await
}

#[tauri::command]
pub async fn listenbrainz_check(state: State<'_, ScrobbleState>) -> AppResult<ScrobbleStatus> {
    state.listenbrainz_check().await
}
