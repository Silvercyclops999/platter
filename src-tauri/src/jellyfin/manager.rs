//! The Jellyfin servers the app uses: adding one (Quick Connect or a
//! password), signing in again, choosing its libraries, making it primary
//! and removing it. Records go in `servers.json` (no secrets), tokens in
//! the keychain.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::watch;

use super::auth::{self, SignedIn};
use super::client::{Identity, JellyfinClient, OFFLINE};
use super::likes::LikeDates;
use super::server::{Core, JellyfinServer, ServerState, View};
use super::types::{PublicInfo, User};
use crate::backend::{BackendKind, Backends, ServerRecord};
use crate::bridge::StreamPrefs;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;

/// A Quick Connect code lasts this long on the server.
const CODE_LIFETIME: Duration = Duration::from_secs(10 * 60);
pub const WRONG_URL: &str = "Platter can't reach a server at that address. Check it and try again.";

/// What an address turned out to be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCheck {
    /// The address, tidied.
    pub url: String,
    pub name: String,
    pub version: String,
    pub quick_connect: bool,
}

/// A Quick Connect code to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickCode {
    pub code: String,
    pub server_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum QuickPoll {
    Waiting,
    Done { server: SavedServer },
}

/// A server in Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedServer {
    pub tag: String,
    pub kind: BackendKind,
    pub name: String,
    pub url: Option<String>,
    pub user: Option<String>,
    pub primary: bool,
    /// None for Plex, whose state the rest of Settings shows.
    pub state: Option<ServerState>,
    /// The account may convert songs and download them.
    pub can_transcode: bool,
    pub can_download: bool,
}

/// A music library on a server, and whether it's included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerLibrary {
    pub id: String,
    pub title: String,
    pub included: bool,
}

struct Pending {
    url: String,
    info: PublicInfo,
    secret: String,
    started: Instant,
}

/// Tauri-managed.
#[derive(Clone)]
pub struct Jellyfin {
    inner: Arc<Inner>,
}

struct Inner {
    backends: Backends,
    db: Option<DbHandle>,
    dir: PathBuf,
    store: Arc<dyn TokenStore>,
    http: reqwest::Client,
    id: Arc<Identity>,
    prefs: Arc<StreamPrefs>,
    offline: Arc<std::sync::atomic::AtomicBool>,
    pending: Mutex<Option<Pending>>,
    cores: Mutex<HashMap<String, Arc<Core>>>,
    stops: Mutex<HashMap<String, watch::Sender<bool>>>,
    /// Follow servers' websockets (off in tests).
    live: bool,
}

pub struct Deps {
    pub backends: Backends,
    pub db: Option<DbHandle>,
    pub dir: PathBuf,
    pub store: Arc<dyn TokenStore>,
    pub http: reqwest::Client,
    pub device_id: String,
    pub prefs: Arc<StreamPrefs>,
    pub offline: Arc<std::sync::atomic::AtomicBool>,
    pub live: bool,
}

impl Jellyfin {
    pub fn new(d: Deps) -> Self {
        Self {
            inner: Arc::new(Inner {
                backends: d.backends,
                db: d.db,
                dir: d.dir,
                store: d.store,
                http: d.http,
                id: Arc::new(Identity::new(&d.device_id)),
                prefs: d.prefs,
                offline: d.offline,
                pending: Mutex::new(None),
                cores: Mutex::new(HashMap::new()),
                stops: Mutex::new(HashMap::new()),
                live: d.live,
            }),
        }
    }

    fn unsigned(&self, url: &str) -> AppResult<JellyfinClient> {
        if let Some(why) = crate::web::plain_http_warning(url) {
            tracing::warn!("{why}");
        }
        JellyfinClient::new(self.inner.http.clone(), url, self.inner.id.clone())
    }

    pub fn core(&self, tag: &str) -> Option<Arc<Core>> {
        self.inner.cores.lock().get(tag).cloned()
    }

    /// The Core for a record: made and registered the first time.
    fn core_for(&self, tag: &str, name: &str, url: &str) -> AppResult<Arc<Core>> {
        if let Some(c) = self.core(tag) {
            c.set_name(name);
            return Ok(c);
        }
        let likes = LikeDates::open(Some(self.inner.dir.join("jellyfin").join(format!("{tag}.likes.json"))));
        let core = Arc::new(Core::new(tag, name, self.unsigned(url)?, self.inner.prefs.clone(), self.inner.db.clone(), likes));
        core.set_offline_switch(self.inner.offline.clone());
        let b = self.inner.backends.clone();
        core.on_change(Arc::new(move || b.changed()));
        self.inner.cores.lock().insert(tag.to_string(), core.clone());
        self.inner.backends.add(tag, Arc::new(JellyfinServer::new(core.clone())));
        Ok(core)
    }

    /// Bring back the saved Jellyfin servers (at startup). Each signs in with
    /// its saved token; one whose token is gone waits for "Sign in again".
    pub fn restore(&self) {
        for r in self.inner.backends.records().servers.into_iter().filter(|r| r.kind == BackendKind::Jellyfin) {
            let Some(url) = r.url.clone() else { continue };
            let core = match self.core_for(&r.tag, &r.name, &url) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("jellyfin server not restored: {}", e.kind());
                    continue;
                }
            };
            core.set_include(r.libraries.clone());
            let token = auth::load_token(self.inner.store.as_ref(), &r.tag).ok().flatten();
            if let (Some(token), Some(uid)) = (token, r.user_id.clone()) {
                let client = core_client(&core, &self.inner.http, &self.inner.id, token, &uid);
                if let Ok(client) = client {
                    core.sign_in(client, User { id: uid, name: r.user.clone(), ..Default::default() });
                }
            }
            self.start(core);
        }
    }

    /// Check the account, read the libraries, follow the socket and sync.
    fn start(&self, core: Arc<Core>) {
        let me = self.clone();
        let spawn = |f: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>| {
            if tokio::runtime::Handle::try_current().is_ok() {
                tokio::spawn(f);
            } else {
                tauri::async_runtime::spawn(f);
            }
        };
        let c2 = core.clone();
        spawn(Box::pin(async move {
            c2.check().await;
            if c2.load_views().await.is_ok() {
                me.refresh_libraries();
            }
        }));
        if self.inner.live
            && let Some(db) = self.inner.db.clone()
        {
            let (tx, rx) = watch::channel(false);
            if let Some(old) = self.inner.stops.lock().insert(core.tag().to_string(), tx) {
                let _ = old.send(true);
            }
            super::live::spawn(core, db, rx);
        }
    }

    /// Sync the libraries again with the current list.
    pub fn refresh_libraries(&self) {
        if let Some(db) = &self.inner.db {
            db.set_extra_libraries(self.inner.backends.extra_libraries());
            if let Err(e) = db.start_sync(false) {
                tracing::debug!("sync not started: {e}");
            }
        }
        self.inner.backends.changed();
    }

    // ---- adding a server ---------------------------------------------------------

    /// What's at an address: a Jellyfin server new enough, and whether
    /// Quick Connect is on.
    pub async fn check(&self, url: &str) -> AppResult<ServerCheck> {
        let c = self.unsigned(url)?;
        let info = c.public_info().await.map_err(wrong_url)?;
        let quick = auth::quick_connect_enabled(&c).await.unwrap_or(false);
        Ok(ServerCheck {
            url: c.base().to_string(),
            name: info.server_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Jellyfin".into()),
            version: info.version.clone().unwrap_or_default(),
            quick_connect: quick,
        })
    }

    /// Start Quick Connect: a code for the user to approve in a Jellyfin
    /// app they're signed in to. The secret stays here.
    pub async fn quick_start(&self, url: &str) -> AppResult<QuickCode> {
        let c = self.unsigned(url)?;
        let info = c.public_info().await.map_err(wrong_url)?;
        if !auth::quick_connect_enabled(&c).await? {
            return Err(AppError::Invalid(auth::QUICK_CONNECT_OFF.into()));
        }
        let r = auth::quick_connect_start(&c).await?;
        let server_name = info.server_name.clone().unwrap_or_else(|| "Jellyfin".into());
        *self.inner.pending.lock() = Some(Pending { url: c.base().to_string(), info, secret: r.secret, started: Instant::now() });
        Ok(QuickCode { code: r.code, server_name })
    }

    /// Ask whether the code was approved; once it is, sign in and add the
    /// server.
    pub async fn quick_poll(&self) -> AppResult<QuickPoll> {
        let (url, info, secret) = {
            let g = self.inner.pending.lock();
            let p = g.as_ref().ok_or_else(|| AppError::Invalid(auth::QUICK_CONNECT_EXPIRED.into()))?;
            if p.started.elapsed() > CODE_LIFETIME {
                drop(g);
                *self.inner.pending.lock() = None;
                return Err(AppError::Invalid(auth::QUICK_CONNECT_EXPIRED.into()));
            }
            (p.url.clone(), p.info.clone(), p.secret.clone())
        };
        let c = self.unsigned(&url)?;
        if !auth::quick_connect_approved(&c, &secret).await? {
            return Ok(QuickPoll::Waiting);
        }
        let signed = auth::quick_connect_finish(&c, info, &secret).await?;
        *self.inner.pending.lock() = None;
        Ok(QuickPoll::Done { server: self.add(signed, &url).await? })
    }

    pub fn quick_cancel(&self) {
        *self.inner.pending.lock() = None;
    }

    /// Sign in with a user name and password (used once, never kept).
    pub async fn password(&self, url: &str, user: &str, password: String) -> AppResult<SavedServer> {
        if user.trim().is_empty() {
            return Err(AppError::Invalid("Type your Jellyfin user name.".into()));
        }
        let c = self.unsigned(url)?;
        let info = c.public_info().await.map_err(wrong_url)?;
        let signed = auth::password(&c, info, user, password).await?;
        self.add(signed, c.base()).await
    }

    /// Keep a sign-in: the token in the keychain, the record, the server in
    /// use. Signing in again to a known server keeps its libraries.
    pub async fn add(&self, s: SignedIn, url: &str) -> AppResult<SavedServer> {
        auth::save_token(self.inner.store.as_ref(), &s.tag, &s.token)?;
        let name = s.info.server_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Jellyfin".into());
        let existing = self.inner.backends.records().get(&s.tag).cloned();
        let record = ServerRecord {
            url: Some(url.to_string()),
            user: s.user.name.clone(),
            user_id: Some(s.user.id.clone()),
            libraries: existing.and_then(|r| r.libraries),
            ..ServerRecord::new(&s.tag, BackendKind::Jellyfin, &name)
        };
        self.inner.backends.edit_records(|f| f.upsert(record.clone()))?;
        let core = self.core_for(&s.tag, &name, url)?;
        core.set_include(record.libraries.clone());
        let client = core_client(&core, &self.inner.http, &self.inner.id, s.token, &s.user.id)?;
        core.sign_in(client, s.user);
        let _ = core.load_views().await;
        self.start(core);
        self.refresh_libraries();
        self.saved_one(&s.tag).ok_or_else(|| AppError::Internal("server not saved".into()))
    }

    // ---- the saved servers -------------------------------------------------------------

    fn saved_one(&self, tag: &str) -> Option<SavedServer> {
        self.saved().into_iter().find(|s| s.tag == tag)
    }

    /// Every saved server, the primary marked.
    pub fn saved(&self) -> Vec<SavedServer> {
        let file = self.inner.backends.records();
        file.servers
            .iter()
            .map(|r| {
                let core = (r.kind == BackendKind::Jellyfin).then(|| self.core(&r.tag)).flatten();
                let policy = core.as_ref().and_then(|c| c.user()).and_then(|u| u.policy).unwrap_or_default();
                SavedServer {
                    tag: r.tag.clone(),
                    kind: r.kind,
                    name: r.name.clone(),
                    url: r.url.clone(),
                    user: r.user.clone(),
                    primary: file.primary.as_deref() == Some(r.tag.as_str()),
                    state: match r.kind {
                        BackendKind::Plex => None,
                        _ => Some(core.as_ref().map_or(ServerState::SignInAgain, |c| c.state())),
                    },
                    can_transcode: policy.enable_audio_playback_transcoding,
                    can_download: policy.enable_content_downloading,
                }
            })
            .collect()
    }

    pub fn set_primary(&self, tag: &str) -> AppResult<()> {
        let ok = self.inner.backends.edit_records(|f| f.get(tag).is_some() && { f.set_primary(tag); true })?;
        if !ok {
            return Err(AppError::NotFound("That server isn't in Platter any more.".into()));
        }
        self.refresh_libraries();
        Ok(())
    }

    /// Remove a server: its token, its record, and (after the next sync) its
    /// music from the library.
    pub async fn remove(&self, tag: &str) -> AppResult<()> {
        let record = self.inner.backends.records().get(tag).cloned();
        match record.as_ref().map(|r| r.kind) {
            Some(BackendKind::Jellyfin) => {}
            Some(_) => return Err(AppError::Invalid("Sign out of Plex in Settings, Account.".into())),
            None => return Err(AppError::NotFound("That server isn't in Platter any more.".into())),
        }
        if let Some(stop) = self.inner.stops.lock().remove(tag) {
            let _ = stop.send(true);
        }
        let core = self.inner.cores.lock().remove(tag);
        let client = core.as_ref().and_then(|c| c.client().ok());
        auth::sign_out(self.inner.store.as_ref(), tag, client.as_ref()).await?;
        self.inner.backends.remove(tag);
        self.inner.backends.edit_records(|f| f.remove(tag))?;
        self.refresh_libraries();
        Ok(())
    }

    /// A server's music libraries and which are included.
    pub async fn libraries(&self, tag: &str) -> AppResult<Vec<ServerLibrary>> {
        let core = self.core(tag).ok_or_else(|| AppError::NotFound("That server isn't in Platter any more.".into()))?;
        let views = match core.load_views().await {
            Ok(v) => v,
            Err(_) if !core.views().is_empty() => core.views(),
            Err(e) => return Err(e),
        };
        let inc = core.include();
        Ok(views
            .into_iter()
            .map(|View { id, title }| ServerLibrary { included: inc.as_ref().is_none_or(|i| i.contains(&id)), id, title })
            .collect())
    }

    /// Include these libraries (None: all of them, now and later).
    pub fn set_libraries(&self, tag: &str, ids: Option<Vec<String>>) -> AppResult<()> {
        let core = self.core(tag).ok_or_else(|| AppError::NotFound("That server isn't in Platter any more.".into()))?;
        if ids.as_ref().is_some_and(|i| i.is_empty()) {
            return Err(AppError::Invalid("Include at least one library, or remove the server.".into()));
        }
        let ids = ids.map(|mut v| {
            v.sort();
            v.dedup();
            v
        });
        self.inner.backends.edit_records(|f| {
            if let Some(mut r) = f.get(tag).cloned() {
                r.libraries = ids.clone();
                f.upsert(r);
            }
        })?;
        core.set_include(ids);
        self.refresh_libraries();
        Ok(())
    }
}

fn core_client(core: &Core, http: &reqwest::Client, id: &Arc<Identity>, token: crate::plex::token::Token, uid: &str) -> AppResult<JellyfinClient> {
    Ok(JellyfinClient::new(http.clone(), core.base(), id.clone())?.signed_in(token, uid))
}

/// An address that answers nothing is a wrong address.
fn wrong_url(e: AppError) -> AppError {
    match e {
        AppError::Network(m) if m == OFFLINE => AppError::Invalid(WRONG_URL.into()),
        other => other,
    }
}
