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
use crate::backend::registry::{same_product, server_list, server_text};
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
/// A new address Platter has nothing to check against (no songs synced yet,
/// or a server that doesn't name its software). The user can say it's the
/// same server.
pub const CANT_TELL: &str =
    "Platter can't tell whether the server at that address is this one. If you're sure it is, tick \"I'm sure this is the same server\" and try again.";
/// A new address that needs the key or password typed again.
pub const TYPE_SECRET: &str = "Type this server's API key, or your password, for the new address.";
/// Plain http where it isn't safe, without the user saying it's fine.
pub const INSECURE_ADDRESS: &str =
    "That address is plain http, so your sign-in could be read on its way to the server. Use the server's https address, or tick \"Use this http address anyway\".";
/// Songs from the cache asked for at a new address, to tell it's the same server.
const KNOWN_SONGS: usize = 4;
/// Of those, how many must come back the same (or all, when fewer are cached).
const KNOWN_MATCHES: usize = 2;

/// What the user said about a new address.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Confirm {
    /// "I'm sure this is the same server": go on when there's nothing to
    /// compare. A server that is plainly another one is still refused.
    pub same_server: bool,
    /// "Use this http address anyway".
    pub allow_insecure: bool,
}

/// An address the sign-in may go to: plain http on the internet, or http
/// where the server was saved at https, needs the user's say-so.
pub fn address_allowed(saved: Option<&str>, new: &str, allow_insecure: bool) -> AppResult<()> {
    if allow_insecure {
        return Ok(());
    }
    let scheme = |u: &str| url::Url::parse(u).ok().map(|u| u.scheme().to_string());
    let downgrade = saved.and_then(scheme).as_deref() == Some("https") && scheme(new).as_deref() == Some("http");
    if downgrade || crate::web::plain_http_warning(new).is_some() {
        return Err(AppError::Invalid(INSECURE_ADDRESS.into()));
    }
    Ok(())
}

/// A song as the cache has it, to compare with the server's answer.
#[derive(Debug, Clone)]
struct KnownSong {
    id: String,
    title: String,
    artist: String,
    duration_ms: u64,
}

fn fold(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

impl KnownSong {
    /// The server's song is this one: same id, title, artist and length.
    fn matches(&self, tag: &str, s: &super::types::Song) -> bool {
        let t = super::map::track(tag, s);
        t.id == self.id
            && fold(&t.title) == fold(&self.title)
            && fold(&t.artist_title) == fold(&self.artist)
            && t.duration_ms.abs_diff(self.duration_ms) <= 2000
    }
}
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
    /// Plain http to an address on the internet: the sign-in can be read
    /// on the way.
    pub insecure: bool,
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

/// The key or password the user typed. None when nothing was typed.
fn typed_secret(user: Option<String>, api_key: Option<String>, password: Option<String>) -> AppResult<Option<Secret>> {
    match (api_key.filter(|k| !k.trim().is_empty()), password.filter(|p| !p.is_empty())) {
        (Some(k), _) => Ok(Some(Secret::ApiKey(Token::new(k)?))),
        (None, Some(pw)) => {
            let Some(u) = user.filter(|u| !u.trim().is_empty()) else { return Err(AppError::Invalid("Type your user name.".into())) };
            Ok(Some(Secret::Password { user: u, password: Token::new(pw)? }))
        }
        (None, None) => Ok(None),
    }
}

/// An API key goes only to a server that takes them.
fn usable_secret(secret: Secret, features: &Features) -> AppResult<Secret> {
    if secret.is_key() && !features.has(ext::API_KEY) {
        return Err(AppError::Invalid(NO_API_KEYS.into()));
    }
    Ok(secret)
}

/// A new address names itself as the software this server was. Another
/// product is another server; a server (then or now) that doesn't say
/// can't be told apart, so only the user's say-so lets it through.
fn product_check(record: &ServerRecord, now: Option<&str>, confirmed: bool) -> AppResult<()> {
    match (record.product.as_deref(), now) {
        (Some(was), Some(now)) if same_product(was, now) => Ok(()),
        (Some(_), Some(_)) => Err(AppError::Invalid(NOT_THIS_SERVER.into())),
        _ if confirmed => Ok(()),
        _ => Err(AppError::Invalid(CANT_TELL.into())),
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
        let (me, weak) = (self.clone(), Arc::downgrade(&core));
        core.on_back(Arc::new(move || {
            let (me, weak) = (me.clone(), weak.clone());
            spawn(Box::pin(async move {
                if let Some(core) = weak.upgrade() {
                    me.read_features(&core).await;
                }
            }));
        }));
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
            // What it could do last time, until it's asked again.
            if let Some(open) = r.open_subsonic {
                core.set_features(Features::new(open, r.extensions.clone()));
            }
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
            if !c2.features_read() {
                me.read_features(&c2).await;
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

    /// Ask the server what it can do (its OpenSubsonic extensions) and keep
    /// the answer for the next start. False when it didn't answer; it's
    /// asked again once it does.
    pub async fn read_features(&self, core: &Arc<Core>) -> bool {
        let Ok(c) = core.client() else { return false };
        let Ok(status) = c.ping().await else { return false };
        let tag = core.tag().to_string();
        // Other software answering at the address isn't this server: what
        // it says it can do isn't kept.
        if let (Some(was), Some(now)) = (self.inner.backends.records().get(&tag).and_then(|r| r.product.clone()), status.kind.as_deref())
            && !same_product(&was, now)
        {
            tracing::warn!("a music server answered as other software; its features weren't read");
            return false;
        }
        let open = status.open_subsonic.unwrap_or(false);
        let names = server_list(c.extensions().await.into_iter().map(|e| e.name));
        core.mark_features_read();
        core.set_features(Features::new(open, names.clone()));
        // The form posts need the extension list.
        if core.features().has(ext::FORM_POST)
            && let (Some(name), Ok(c)) = (core.username(), core.client())
        {
            core.sign_in(c, &name, core.user().unwrap_or_default());
        }
        let saved = self.inner.backends.edit_records(|f| {
            if let Some(mut r) = f.get(&tag).cloned() {
                r.open_subsonic = Some(open);
                r.extensions = names.clone();
                f.upsert(r);
            }
        });
        if let Err(e) = saved {
            tracing::debug!("server features not kept: {}", e.kind());
        }
        true
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
    /// An address typed without http or https is tried as https first
    /// when it's on the internet.
    pub async fn check(&self, url: &str) -> AppResult<SubsonicCheck> {
        let tries = crate::web::address_candidates(url);
        let mut last = None;
        for (i, url) in tries.iter().enumerate() {
            let c = self.unsigned(url)?;
            let status = match c.probe().await {
                Ok(s) => s,
                Err(AppError::Network(m)) if i + 1 < tries.len() => {
                    last = Some(AppError::Network(m));
                    continue;
                }
                Err(e) => return Err(wrong_url(e)),
            };
            let exts = c.extensions().await;
            return Ok(SubsonicCheck {
                url: c.base().to_string(),
                name: server_name(status.kind.as_deref().and_then(server_text).as_deref(), c.base()),
                version: status.server_version.or(status.version),
                open_subsonic: status.open_subsonic.unwrap_or(false),
                api_keys: exts.iter().any(|e| e.name == ext::API_KEY),
                insecure: crate::web::plain_http_warning(c.base()).is_some(),
            });
        }
        Err(wrong_url(last.unwrap_or_else(|| AppError::Invalid(WRONG_URL.into()))))
    }

    /// Sign in with an API key (preferred) or a user name and password, and
    /// add the server (or sign in to it again).
    pub async fn sign_in(&self, url: &str, user: Option<String>, api_key: Option<String>, password: Option<String>) -> AppResult<SavedServer> {
        self.sign_in_to(None, url, user, api_key, password, Confirm::default()).await
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
        confirm: Confirm,
    ) -> AppResult<SavedServer> {
        let keep = keep.filter(|t| self.owns(t));
        let c = self.unsigned(url)?;
        let saved = keep.and_then(|t| self.inner.backends.records().get(t).cloned());
        address_allowed(saved.as_ref().and_then(|r| r.url.as_deref()), c.base(), confirm.allow_insecure)?;
        let user = user.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        let typed = typed_secret(user.clone(), api_key, password)?
            .ok_or_else(|| AppError::Invalid("Type an API key, or your user name and password.".into()))?;
        let status = c.probe().await.map_err(wrong_url)?;
        let kind = status.kind.as_deref().and_then(server_text);
        // Signing in again at another address: it must name itself as the
        // same software before the sign-in goes there.
        let moving = saved.as_ref().filter(|r| r.url.as_deref() != Some(c.base()));
        if let Some(r) = moving {
            product_check(r, kind.as_deref(), confirm.same_server)?;
        } else if let Some(r) = saved.clone().or_else(|| self.inner.backends.records().get(&BackendKind::Subsonic.tag(c.base())).cloned())
            && let (Some(was), Some(now)) = (r.product.as_deref(), kind.as_deref())
            && !same_product(was, now)
        {
            // The same address, but other software answers there now.
            return Err(AppError::Invalid(NOT_THIS_SERVER.into()));
        }
        let features = self.features_at(&c, &status).await;
        let secret = usable_secret(typed, &features)?;
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
                if let Some(r) = moving {
                    self.same_server_as(r, kind.as_deref(), &signed, confirm.same_server).await?;
                }
                t.to_string()
            }
            None => BackendKind::Subsonic.tag(c.base()),
        };
        self.inner.store.set(&key_account(&tag), &secret.stored()?)?;
        let name = server_name(kind.as_deref(), c.base());
        let existing = self.inner.backends.records().get(&tag).cloned();
        let moved = existing.as_ref().is_some_and(|r| r.url.as_deref() != Some(c.base()));
        let record = ServerRecord {
            url: Some(c.base().to_string()),
            user: (!username.is_empty()).then(|| username.clone()),
            user_id: None,
            open_subsonic: Some(features.open),
            extensions: server_list(features.names.iter().cloned()),
            product: existing.as_ref().and_then(|r| r.product.clone()).or(kind.clone()),
            libraries: existing.and_then(|r| r.libraries),
            ..ServerRecord::new(&tag, BackendKind::Subsonic, &name)
        };
        self.inner.backends.edit_records(|f| f.upsert(record.clone()))?;
        if moved {
            self.forget_core(&tag);
        }
        let core = self.core_for(&tag, &name, c.base())?;
        core.set_include(record.libraries.clone());
        core.mark_features_read();
        core.set_features(features);
        core.sign_in(c.signed_in(secret), &username, account);
        let _ = core.load_views().await;
        self.start(core);
        self.refresh_libraries();
        self.saved_one(&tag).ok_or_else(|| AppError::Internal("server not saved".into()))
    }

    /// The server's extensions at an address, as kept (capped).
    async fn features_at(&self, c: &SubsonicClient, status: &super::types::Status) -> Features {
        let names = server_list(c.extensions().await.into_iter().map(|e| e.name));
        Features::new(status.open_subsonic.unwrap_or(false), names)
    }

    /// Use a saved server at a new address (http or https, another host,
    /// port or path, a proxy in front of it). It keeps its tag, so its
    /// songs, downloads, likes, playlists and history stay as they are.
    ///
    /// The saved key or password never goes to the new address: the user
    /// types it again (`user`, `api_key`, `password`), and it's sent only
    /// after the address answers, unsigned, as the same kind of server.
    /// Then the songs this server had must be there, the same songs; when
    /// there's nothing to compare, only the user's "I'm sure" lets it
    /// through.
    pub async fn change_address(
        &self,
        tag: &str,
        url: &str,
        user: Option<String>,
        api_key: Option<String>,
        password: Option<String>,
        confirm: Confirm,
    ) -> AppResult<SavedServer> {
        let record = self
            .inner
            .backends
            .records()
            .get(tag)
            .filter(|r| r.kind == BackendKind::Subsonic)
            .cloned()
            .ok_or_else(|| AppError::NotFound("That server isn't in Platter any more.".into()))?;
        // Nothing goes anywhere without a typed key or password.
        let user = user.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).or(record.user.clone());
        let typed = typed_secret(user, api_key, password)?.ok_or_else(|| AppError::Invalid(TYPE_SECRET.into()))?;
        let c = self.unsigned(url)?;
        if record.url.as_deref() == Some(c.base()) {
            return self.saved_one(tag).ok_or_else(|| AppError::Internal("server not saved".into()));
        }
        address_allowed(record.url.as_deref(), c.base(), confirm.allow_insecure)?;
        let status = c.probe().await.map_err(wrong_url)?;
        let kind = status.kind.as_deref().and_then(server_text);
        product_check(&record, kind.as_deref(), confirm.same_server)?;
        let features = self.features_at(&c, &status).await;
        let secret = usable_secret(typed, &features)?;
        let signed = c.clone().signed_in(secret.clone()).with_form_post(features.has(ext::FORM_POST));
        signed.ping().await.map_err(wrong_url)?;
        self.same_server_as(&record, kind.as_deref(), &signed, confirm.same_server).await?;
        // The typed sign-in worked here: it's the one kept now.
        self.inner.store.set(&key_account(tag), &secret.stored()?)?;
        let base = c.base().to_string();
        let product = record.product.clone().or(kind.clone());
        let extensions = server_list(features.names.iter().cloned());
        let username = match &secret {
            Secret::Password { user, .. } => Some(user.clone()),
            Secret::ApiKey(_) => record.user.clone(),
        };
        self.inner.backends.edit_records(|f| {
            if let Some(mut r) = f.get(tag).cloned() {
                r.url = Some(base.clone());
                r.product = product.clone();
                r.open_subsonic = Some(features.open);
                r.extensions = extensions.clone();
                r.user = username.clone();
                f.upsert(r);
            }
        })?;
        let username = username.unwrap_or_default();
        let account = self.core(tag).and_then(|c| c.user()).unwrap_or_default();
        self.forget_core(tag);
        let core = self.core_for(tag, &record.name, &base)?;
        core.set_include(record.libraries.clone());
        core.mark_features_read();
        core.set_features(features);
        core.sign_in(c.signed_in(secret), &username, account);
        let _ = core.load_views().await;
        self.start(core);
        self.refresh_libraries();
        self.saved_one(tag).ok_or_else(|| AppError::Internal("server not saved".into()))
    }

    /// The server at a new address is the one `record` names: the same
    /// kind of server, and the songs this one had are there as the same
    /// songs (id, title, artist and length), at least [`KNOWN_MATCHES`] of
    /// them, or all when fewer are cached. With nothing to compare it's
    /// refused unless the user said it's the same server.
    async fn same_server_as(&self, record: &ServerRecord, kind: Option<&str>, signed: &SubsonicClient, confirmed: bool) -> AppResult<()> {
        product_check(record, kind, confirmed)?;
        let known = self.known_songs(&record.tag).await;
        if known.is_empty() {
            return if confirmed { Ok(()) } else { Err(AppError::Invalid(CANT_TELL.into())) };
        }
        let need = KNOWN_MATCHES.min(known.len());
        let mut same = 0;
        for k in &known {
            let r: AppResult<super::types::Song> = signed.get("getSong", &[("id", crate::sid::key(&k.id).to_string())], Some("song")).await;
            match r {
                Ok(s) if k.matches(&record.tag, &s) => same += 1,
                Ok(_) | Err(AppError::NotFound(_)) => {}
                Err(e) => return Err(wrong_url(e)),
            }
            if same >= need {
                return Ok(());
            }
        }
        Err(AppError::Invalid(NOT_THIS_SERVER.into()))
    }

    /// A few songs the cache has from this server.
    async fn known_songs(&self, tag: &str) -> Vec<KnownSong> {
        let Some(db) = self.inner.db.clone() else { return Vec::new() };
        let like = format!("{tag}:%");
        db.read_async(move |c| {
            let mut st = c.prepare("SELECT id, title, artist_title, duration_ms FROM tracks WHERE id LIKE ?1 ORDER BY id LIMIT ?2")?;
            let songs = st
                .query_map(rusqlite::params![like, KNOWN_SONGS as i64], |r| {
                    Ok(KnownSong {
                        id: r.get(0)?,
                        title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        artist: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        duration_ms: r.get::<_, Option<i64>>(3)?.unwrap_or(0).max(0) as u64,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(songs)
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
    fn the_refusals_name_the_boxes_the_settings_page_shows() {
        // The UI shows these boxes and matches CANT_TELL word for word.
        assert!(CANT_TELL.contains("\"I'm sure this is the same server\""));
        assert!(INSECURE_ADDRESS.contains("\"Use this http address anyway\""));
        assert!(!CANT_TELL.contains('\u{2014}') && !INSECURE_ADDRESS.contains('\u{2014}') && !TYPE_SECRET.contains('\u{2014}'));
    }

    #[test]
    fn product_and_typed_sign_in_checks() {
        let mut r = ServerRecord::new("t", BackendKind::Subsonic, "S");
        assert!(product_check(&r, Some("navidrome"), false).is_err(), "nothing kept to compare");
        assert!(product_check(&r, Some("navidrome"), true).is_ok());
        r.product = Some("navidrome".into());
        assert!(product_check(&r, Some("Navidrome"), false).is_ok());
        assert!(product_check(&r, None, false).is_err(), "a server that doesn't say");
        assert!(product_check(&r, Some("gonic"), true).is_err(), "another product even when sure");
        assert!(typed_secret(None, None, None).unwrap().is_none());
        assert!(typed_secret(None, None, Some("pw".into())).is_err(), "a password needs a user");
        assert!(typed_secret(None, Some("  ".into()), None).unwrap().is_none());
    }

    #[test]
    fn the_keychain_entry_is_per_server() {
        assert_eq!(key_account("0badf00d"), "os.0badf00d");
    }
}
