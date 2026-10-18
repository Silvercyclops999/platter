//! The OpenSubsonic servers the app uses: adding one with an API key or a
//! user name and password, signing in again, choosing its folders, checking
//! it for changes, and removing it. Records go in `servers.json` (no
//! secrets); the API key or password goes in the keychain under `os.<tag>`.
//! A password has to be kept: Subsonic signs every request with it.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::watch;

use super::client::{NO_API_KEYS, OFFLINE, Secret, SubsonicClient};
use super::server::{Core, Features, SubsonicServer, ext};
use super::types::{TokenInfo, User};
use crate::backend::{BackendKind, Backends, ServerRecord};
use crate::bridge::StreamPrefs;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::jellyfin::manager::{SavedServer, ServerLibrary};
use crate::jellyfin::server::{ServerState, View};
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;

pub const WRONG_URL: &str = "Platter can't reach a server at that address. Check it and try again.";
/// A new address that answers as another server.
pub const NOT_THIS_SERVER: &str =
    "The server at that address isn't this one: it doesn't have this server's songs. Check the address, or add it as a new server.";
/// Songs from the cache asked for at a new address, to tell it's the same server.
const KNOWN_SONGS: usize = 3;
/// How often a server is checked for changes, by default (Settings).
pub const DEFAULT_POLL_MINUTES: u32 = 5;
pub const POLL_MINUTES_MIN: u32 = 1;
pub const POLL_MINUTES_MAX: u32 = 1440;

/// The keychain entry for a server's API key or password.
pub fn key_account(tag: &str) -> String {
    format!("os.{tag}")
}

/// What an address turned out to be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicCheck {
    /// The address, tidied.
    pub url: String,
    pub name: String,
    pub version: Option<String>,
    /// It's an OpenSubsonic server (more than the original API).
    pub open_subsonic: bool,
    /// It takes API keys.
    pub api_keys: bool,
}

/// Tauri-managed.
#[derive(Clone)]
pub struct Subsonic {
    inner: Arc<Inner>,
}

struct Inner {
    backends: Backends,
    db: Option<DbHandle>,
    store: Arc<dyn TokenStore>,
    http: reqwest::Client,
    prefs: Arc<StreamPrefs>,
    offline: Arc<std::sync::atomic::AtomicBool>,
    poll_minutes: Arc<AtomicU32>,
    cores: Mutex<HashMap<String, Arc<Core>>>,
    stops: Mutex<HashMap<String, watch::Sender<bool>>>,
    /// Check servers for changes in the background (off in tests).
    live: bool,
}

pub struct Deps {
    pub backends: Backends,
    pub db: Option<DbHandle>,
    pub store: Arc<dyn TokenStore>,
    pub http: reqwest::Client,
    pub prefs: Arc<StreamPrefs>,
    pub offline: Arc<std::sync::atomic::AtomicBool>,
    /// Settings, "Check a Subsonic server for changes every".
    pub poll_minutes: Arc<AtomicU32>,
    pub live: bool,
}

/// A name for the server: its product and where it is.
fn server_name(product: Option<&str>, base: &str) -> String {
    let host = url::Url::parse(base).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    let product = product.map(str::trim).filter(|p| !p.is_empty()).map(|p| {
        let mut c = p.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    });
    match (product, host.is_empty()) {
        (Some(p), false) => format!("{p} on {host}"),
        (Some(p), true) => p,
        (None, false) => host,
        (None, true) => "Music server".into(),
    }
}

fn spawn(f: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(f);
    } else {
        tauri::async_runtime::spawn(f);
    }
}

/// An address that answers nothing is a wrong address.
fn wrong_url(e: AppError) -> AppError {
    match e {
        AppError::Network(m) if m == OFFLINE => AppError::Invalid(WRONG_URL.into()),
        other => other,
    }
}

impl Subsonic {
    pub fn new(d: Deps) -> Self {
        Self {
            inner: Arc::new(Inner {
                backends: d.backends,
                db: d.db,
                store: d.store,
                http: d.http,
                prefs: d.prefs,
                offline: d.offline,
                poll_minutes: d.poll_minutes,
                cores: Mutex::new(HashMap::new()),
                stops: Mutex::new(HashMap::new()),
                live: d.live,
            }),
        }
    }

    fn unsigned(&self, url: &str) -> AppResult<SubsonicClient> {
        if let Some(why) = crate::web::plain_http_warning(url) {
            tracing::warn!("{why}");
        }
        SubsonicClient::new(self.inner.http.clone(), url)
    }

    pub fn core(&self, tag: &str) -> Option<Arc<Core>> {
        self.inner.cores.lock().get(tag).cloned()
    }

    /// Whether this tag is a Subsonic server's.
    pub fn owns(&self, tag: &str) -> bool {
        self.inner.backends.records().get(tag).is_some_and(|r| r.kind == BackendKind::Subsonic)
    }

    fn core_for(&self, tag: &str, name: &str, url: &str) -> AppResult<Arc<Core>> {
        if let Some(c) = self.core(tag) {
            c.set_name(name);
            return Ok(c);
        }
        let core = Arc::new(Core::new(tag, name, self.unsigned(url)?, self.inner.prefs.clone(), self.inner.db.clone()));
        core.set_offline_switch(self.inner.offline.clone());
        let b = self.inner.backends.clone();
        core.on_change(Arc::new(move || b.changed()));
        self.inner.cores.lock().insert(tag.to_string(), core.clone());
        self.inner.backends.add(tag, Arc::new(SubsonicServer::new(core.clone())));
        Ok(core)
    }

    /// Bring back the saved servers (at startup). Each signs in with its
    /// saved key or password; one whose sign-in is gone waits for "Sign in
    /// again".
    pub fn restore(&self) {
        for r in self.inner.backends.records().servers.into_iter().filter(|r| r.kind == BackendKind::Subsonic) {
            let Some(url) = r.url.clone() else { continue };
            let core = match self.core_for(&r.tag, &r.name, &url) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("music server not restored: {}", e.kind());
                    continue;
                }
            };
            core.set_include(r.libraries.clone());
            let stored = self.inner.store.get(&key_account(&r.tag)).ok().flatten();
            let secret = stored.as_ref().and_then(|t| Secret::from_stored(t, r.user.as_deref()));
            if let (Some(secret), Ok(c)) = (secret, self.unsigned(&url)) {
                core.sign_in(c.signed_in(secret), r.user.as_deref().unwrap_or_default(), User::default());
            }
            self.start(core);
        }
    }

    /// Read its extensions and folders, check the account, sync, and start
    /// checking it for changes.
    fn start(&self, core: Arc<Core>) {
        let me = self.clone();
        let c2 = core.clone();
        spawn(Box::pin(async move {
            if c2.features() == Features::default()
                && let Ok(c) = c2.client()
            {
                let open = c.ping().await.ok().and_then(|s| s.open_subsonic).unwrap_or(false);
                let names = c.extensions().await.into_iter().map(|e| e.name);
                c2.set_features(Features::new(open, names));
                // The form posts need the extension list.
                if c2.features().has(ext::FORM_POST)
                    && let (Some(name), Ok(c)) = (c2.username(), c2.client())
                {
                    c2.sign_in(c, &name, c2.user().unwrap_or_default());
                }
            }
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
            spawn(Box::pin(poll_loop(core, db, self.inner.poll_minutes.clone(), self.inner.offline.clone(), rx)));
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

    /// What's at an address: a Subsonic server, and whether it takes API keys.
    pub async fn check(&self, url: &str) -> AppResult<SubsonicCheck> {
        let c = self.unsigned(url)?;
        let status = c.probe().await.map_err(wrong_url)?;
        let exts = c.extensions().await;
        Ok(SubsonicCheck {
            url: c.base().to_string(),
            name: server_name(status.kind.as_deref(), c.base()),
            version: status.server_version.or(status.version),
            open_subsonic: status.open_subsonic.unwrap_or(false),
            api_keys: exts.iter().any(|e| e.name == ext::API_KEY),
        })
    }

    /// Sign in with an API key (preferred) or a user name and password, and
    /// add the server (or sign in to it again).
    pub async fn sign_in(&self, url: &str, user: Option<String>, api_key: Option<String>, password: Option<String>) -> AppResult<SavedServer> {
        self.sign_in_to(None, url, user, api_key, password).await
    }

    /// Like [`Self::sign_in`]; `keep` is the server being signed in to
    /// again. It keeps its tag (and so its songs, downloads, likes and
    /// history) even when its address was typed differently, once the
    /// address is checked to be the same server.
    pub async fn sign_in_to(
        &self,
        keep: Option<&str>,
        url: &str,
        user: Option<String>,
        api_key: Option<String>,
        password: Option<String>,
    ) -> AppResult<SavedServer> {
        let keep = keep.filter(|t| self.owns(t));
        let c = self.unsigned(url)?;
        let status = c.probe().await.map_err(wrong_url)?;
        let names: Vec<String> = c.extensions().await.into_iter().map(|e| e.name).collect();
        let features = Features::new(status.open_subsonic.unwrap_or(false), names);
        let user = user.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        let secret = match (api_key.filter(|k| !k.trim().is_empty()), password.filter(|p| !p.is_empty())) {
            (Some(k), _) => {
                if !features.has(ext::API_KEY) {
                    return Err(AppError::Invalid(NO_API_KEYS.into()));
                }
                Secret::ApiKey(Token::new(k)?)
            }
            (None, Some(pw)) => {
                let Some(u) = user.clone() else { return Err(AppError::Invalid("Type your user name.".into())) };
                Secret::Password { user: u, password: Token::new(pw)? }
            }
            (None, None) => return Err(AppError::Invalid("Type an API key, or your user name and password.".into())),
        };
        let signed = c.clone().signed_in(secret.clone()).with_form_post(features.has(ext::FORM_POST));
        signed.ping().await.map_err(wrong_url)?;
        // Who the key belongs to.
        let username = match &secret {
            Secret::Password { user, .. } => user.clone(),
            Secret::ApiKey(_) => signed
                .get::<TokenInfo>("tokenInfo", &[], Some("tokenInfo"))
                .await
                .ok()
                .and_then(|t| t.username)
                .or(user.clone())
                .unwrap_or_default(),
        };
        let account: User = if username.is_empty() {
            User::default()
        } else {
            signed.get("getUser", &[("username", username.clone())], Some("user")).await.unwrap_or_default()
        };
        let tag = match keep {
            Some(t) => {
                self.same_server(t, status.kind.as_deref(), &signed).await?;
                t.to_string()
            }
            None => BackendKind::Subsonic.tag(c.base()),
        };
        self.inner.store.set(&key_account(&tag), &secret.stored()?)?;
        let name = server_name(status.kind.as_deref(), c.base());
        let existing = self.inner.backends.records().get(&tag).cloned();
        let moved = existing.as_ref().is_some_and(|r| r.url.as_deref() != Some(c.base()));
        let record = ServerRecord {
            url: Some(c.base().to_string()),
            user: (!username.is_empty()).then(|| username.clone()),
            user_id: None,
            product: status.kind.clone().or(existing.as_ref().and_then(|r| r.product.clone())),
            libraries: existing.and_then(|r| r.libraries),
            ..ServerRecord::new(&tag, BackendKind::Subsonic, &name)
        };
        self.inner.backends.edit_records(|f| f.upsert(record.clone()))?;
        if moved {
            self.forget_core(&tag);
        }
        let core = self.core_for(&tag, &name, c.base())?;
        core.set_include(record.libraries.clone());
        core.set_features(features);
        core.sign_in(c.signed_in(secret), &username, account);
        let _ = core.load_views().await;
        self.start(core);
        self.refresh_libraries();
        self.saved_one(&tag).ok_or_else(|| AppError::Internal("server not saved".into()))
    }

    /// Use a saved server at a new address (http or https, another host,
    /// port or path, a proxy in front of it). It keeps its tag, so its
    /// songs, downloads, likes, playlists and history stay as they are. The
    /// new address must be the same server: it signs in with the saved key
    /// or password, names itself the same kind of server, and has the songs
    /// this server had.
    pub async fn change_address(&self, tag: &str, url: &str) -> AppResult<SavedServer> {
        let record = self
            .inner
            .backends
            .records()
            .get(tag)
            .filter(|r| r.kind == BackendKind::Subsonic)
            .cloned()
            .ok_or_else(|| AppError::NotFound("That server isn't in Platter any more.".into()))?;
        let c = self.unsigned(url)?;
        let status = c.probe().await.map_err(wrong_url)?;
        if record.url.as_deref() == Some(c.base()) {
            return self.saved_one(tag).ok_or_else(|| AppError::Internal("server not saved".into()));
        }
        let names: Vec<String> = c.extensions().await.into_iter().map(|e| e.name).collect();
        let features = Features::new(status.open_subsonic.unwrap_or(false), names);
        let stored = self.inner.store.get(&key_account(tag))?;
        let secret = stored
            .as_ref()
            .and_then(|t| Secret::from_stored(t, record.user.as_deref()))
            .ok_or_else(|| AppError::Auth("Sign in to this server again, then change its address.".into()))?;
        let signed = c.clone().signed_in(secret.clone()).with_form_post(features.has(ext::FORM_POST));
        signed.ping().await.map_err(wrong_url)?;
        self.same_server_as(&record, status.kind.as_deref(), &signed).await?;
        let base = c.base().to_string();
        let product = status.kind.clone().or(record.product.clone());
        self.inner.backends.edit_records(|f| {
            if let Some(mut r) = f.get(tag).cloned() {
                r.url = Some(base.clone());
                r.product = product.clone();
                f.upsert(r);
            }
        })?;
        let username = record.user.clone().unwrap_or_default();
        let account = self.core(tag).and_then(|c| c.user()).unwrap_or_default();
        self.forget_core(tag);
        let core = self.core_for(tag, &record.name, &base)?;
        core.set_include(record.libraries.clone());
        core.set_features(features);
        core.sign_in(c.signed_in(secret), &username, account);
        let _ = core.load_views().await;
        self.start(core);
        self.refresh_libraries();
        self.saved_one(tag).ok_or_else(|| AppError::Internal("server not saved".into()))
    }

    async fn same_server(&self, tag: &str, kind: Option<&str>, signed: &SubsonicClient) -> AppResult<()> {
        let Some(record) = self.inner.backends.records().get(tag).cloned() else { return Ok(()) };
        if record.url.as_deref() == Some(signed.base()) {
            return Ok(());
        }
        self.same_server_as(&record, kind, signed).await
    }

    /// The server at a new address is the one `record` names: the same
    /// kind of server, and songs this one had are there.
    async fn same_server_as(&self, record: &ServerRecord, kind: Option<&str>, signed: &SubsonicClient) -> AppResult<()> {
        if let (Some(was), Some(now)) = (record.product.as_deref(), kind)
            && !was.eq_ignore_ascii_case(now)
        {
            return Err(AppError::Invalid(NOT_THIS_SERVER.into()));
        }
        let known = self.known_songs(&record.tag).await;
        if known.is_empty() {
            return Ok(());
        }
        for id in &known {
            let r: AppResult<super::types::Song> = signed.get("getSong", &[("id", crate::sid::key(id).to_string())], Some("song")).await;
            match r {
                Ok(_) => return Ok(()),
                Err(AppError::NotFound(_)) => {}
                Err(e) => return Err(wrong_url(e)),
            }
        }
        Err(AppError::Invalid(NOT_THIS_SERVER.into()))
    }

    /// A few song ids the cache has from this server.
    async fn known_songs(&self, tag: &str) -> Vec<String> {
        let Some(db) = self.inner.db.clone() else { return Vec::new() };
        let like = format!("{tag}:%");
        db.read_async(move |c| {
            let mut st = c.prepare("SELECT id FROM tracks WHERE id LIKE ?1 LIMIT ?2")?;
            let ids = st
                .query_map(rusqlite::params![like, KNOWN_SONGS as i64], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ids)
        })
        .await
        .unwrap_or_default()
    }

    /// Drop a server's live parts (they hold its address) so they're made
    /// again; its record and keychain entry stay.
    fn forget_core(&self, tag: &str) {
        if let Some(stop) = self.inner.stops.lock().remove(tag) {
            let _ = stop.send(true);
        }
        self.inner.cores.lock().remove(tag);
    }

    // ---- the saved servers -------------------------------------------------------------

    fn saved_one(&self, tag: &str) -> Option<SavedServer> {
        let file = self.inner.backends.records();
        let r = file.get(tag)?;
        let mut list = vec![SavedServer {
            tag: r.tag.clone(),
            kind: r.kind,
            name: r.name.clone(),
            url: r.url.clone(),
            user: r.user.clone(),
            primary: file.primary.as_deref() == Some(tag),
            state: None,
            can_transcode: true,
            can_download: true,
        }];
        self.patch(&mut list);
        list.pop()
    }

    /// Fill in how each Subsonic server in the Settings list is doing.
    pub fn patch(&self, list: &mut [SavedServer]) {
        for s in list.iter_mut().filter(|s| s.kind == BackendKind::Subsonic) {
            let core = self.core(&s.tag);
            s.state = Some(core.as_ref().map_or(ServerState::SignInAgain, |c| c.state()));
            s.can_transcode = true;
            s.can_download = core.as_ref().is_none_or(|c| c.may_download());
        }
    }

    /// Remove a server: its key or password, its record, and (after the
    /// next sync) its music from the library.
    pub async fn remove(&self, tag: &str) -> AppResult<()> {
        if !self.owns(tag) {
            return Err(AppError::NotFound("That server isn't in Platter any more.".into()));
        }
        if let Some(stop) = self.inner.stops.lock().remove(tag) {
            let _ = stop.send(true);
        }
        self.inner.cores.lock().remove(tag);
        self.inner.store.delete(&key_account(tag))?;
        self.inner.backends.remove(tag);
        self.inner.backends.edit_records(|f| f.remove(tag))?;
        self.refresh_libraries();
        Ok(())
    }

    /// A server's music folders and which are included.
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

    /// Include these folders (None: all of them, now and later).
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

/// Every few minutes (Settings), ask the server whether its library
/// changed, and sync when it did. Subsonic can't tell the app as it
/// happens, or list what changed; the weekly full pass of the sync catches
/// edits the check can't see.
async fn poll_loop(core: Arc<Core>, db: DbHandle, minutes: Arc<AtomicU32>, offline: Arc<std::sync::atomic::AtomicBool>, mut stop: watch::Receiver<bool>) {
    let mut first = true;
    loop {
        if !first {
            let wait = minutes.load(Ordering::Relaxed).clamp(POLL_MINUTES_MIN, POLL_MINUTES_MAX) as u64 * 60;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                _ = stop.changed() => {}
            }
        }
        first = false;
        if *stop.borrow() {
            return;
        }
        if offline.load(Ordering::Relaxed) || !core.signed_in() {
            continue;
        }
        match core.poll_changes().await {
            Ok(true) => {
                tracing::info!("music server library changed; syncing");
                if let Err(e) = db.start_sync(false) {
                    tracing::debug!("sync not started: {e}");
                }
            }
            Ok(false) => {}
            Err(e) => tracing::debug!("change check failed: {}", e.kind()),
        }
    }
}

/// The settings value, clamped.
pub fn poll_minutes(v: u32) -> u32 {
    v.clamp(POLL_MINUTES_MIN, POLL_MINUTES_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_say_what_and_where() {
        assert_eq!(server_name(Some("navidrome"), "http://music.example:4533"), "Navidrome on music.example");
        assert_eq!(server_name(None, "https://music.example/nd"), "music.example");
        assert_eq!(server_name(Some(" "), "https://music.example"), "music.example");
        assert_eq!(poll_minutes(0), 1);
        assert_eq!(poll_minutes(5), 5);
    }

    #[test]
    fn the_keychain_entry_is_per_server() {
        assert_eq!(key_account("0badf00d"), "os.0badf00d");
    }
}
