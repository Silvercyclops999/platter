//! Connect your account: the streaming service's playlists read through its
//! official Web API, with the user's own developer app (their Client ID,
//! PKCE, no secret).
//!
//! Only playlists come this way: the service's developer terms allow moving
//! a user's playlist metadata to another app, not their saved songs,
//! albums or listening history, so those come from the account data
//! download. And only playlists the user owns or collaborates on can be
//! read (the API returns other people's playlists without their songs);
//! those are counted in the report so the user knows to use the download.
//!
//! What's read goes through the same pipeline as a file import (source
//! `streaming-account`): matched to the library (ISRC first, the API has
//! it), into the import tables, the Missing list and playlist saving.
//! Reading again updates the playlists it finds and never removes one, so
//! the user's imported lists stay until they remove them.
//!
//! Re-check daily: while the app runs and the account is connected, the
//! playlists are read again once a day (on by default).
//!
//! Work: the reads are async on the shared runtime, one at a time; the
//! matching and the write run on one blocking thread.

pub mod api;
pub mod callback;
pub mod commands;
pub mod pkce;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

use super::{Parsed, PlaylistIn, SongIn};
use crate::db::DbHandle;
use crate::error::AppError;
use crate::plex::keychain::TokenStore;
use crate::plex::state::EventSink;
use crate::web::Web;
use api::{Api, Endpoints, PAGE};

/// The import source id.
pub const SOURCE: &str = "streaming-account";
/// The keychain account holding the developer app's Client ID.
pub const CLIENT_ACCOUNT: &str = "import-account-client-id";
/// The status event.
pub const EVENT: &str = "streaming-account://status";
/// What to register in the developer app. Loopback addresses may take any
/// port, so the URI has none and each sign-in picks a free one.
pub const REDIRECT_TO_REGISTER: &str = "http://127.0.0.1/callback";
/// How often the background check looks whether a daily read is due.
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
/// A daily read.
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Why something stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fail {
    /// No network, or Go offline is on.
    Offline(String),
    /// The sign-in or the service said no.
    Refused(String),
    /// The kept sign-in stopped working; sign in again.
    Expired,
    /// Too many requests; try again after this many seconds.
    RateLimited(u64),
    /// This one thing can't be read (another person's playlist).
    Forbidden(String),
    /// The user pressed Stop.
    Stopped,
    Other(String),
}

impl From<Fail> for AppError {
    fn from(f: Fail) -> Self {
        match f {
            Fail::Offline(m) => AppError::Network(m),
            Fail::Refused(m) | Fail::Forbidden(m) => AppError::Auth(m),
            Fail::Expired => AppError::Auth("Sign in again.".into()),
            Fail::RateLimited(s) => AppError::Server(format!("Too many requests. Try again in {s} seconds.")),
            Fail::Stopped => AppError::Invalid("Stopped.".into()),
            Fail::Other(m) => AppError::Internal(m),
        }
    }
}

/// The settings this reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// The source is on in Settings > Library import.
    pub enabled: bool,
    /// The developer app's Client ID. Kept in the keychain (this device),
    /// filled in by the service; settings never hold it.
    pub client_id: String,
    /// Re-check daily.
    pub recheck: bool,
}

impl Config {
    pub fn from_settings(s: &crate::settings::Settings) -> Self {
        Config {
            enabled: s.import_sources.iter().any(|x| x == SOURCE),
            client_id: String::new(),
            recheck: s.streaming_recheck,
        }
    }
}

/// Where the connection stands, for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// No Client ID yet.
    NotSetUp,
    /// A Client ID, not signed in.
    SignedOut,
    SigningIn,
    Connected,
    Reading,
    /// The sign-in or the service said no (`message` says why).
    Refused,
    /// The kept sign-in stopped working.
    Expired,
    /// Too many requests (`retryAt`).
    RateLimited,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub state: State,
    /// What to register in the developer app.
    pub redirect_uri: &'static str,
    /// The account's display name, once read this run.
    pub name: Option<String>,
    /// Why it stopped, or what the last read left out.
    pub message: Option<String>,
    /// Unix ms when a rate limit ends.
    pub retry_at: Option<i64>,
    /// Playlists read so far and to read, while reading.
    pub done: u64,
    pub total: u64,
    /// Unix ms of the last read that finished.
    pub last_read: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Idle,
    SigningIn,
    Reading { done: u64, total: u64 },
    /// Rate limited until this unix ms.
    Limited(i64),
    Failed(Fail),
}

/// Opens a link in the system browser.
pub type OpenLink = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub struct Deps {
    pub web: Arc<dyn Web>,
    pub keys: Arc<dyn TokenStore>,
    pub db: DbHandle,
    pub events: Arc<dyn EventSink>,
    pub open: OpenLink,
    /// The settings now.
    pub config: Arc<dyn Fn() -> Config + Send + Sync>,
    /// Unix ms.
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub ends: Endpoints,
    /// How long a sign-in waits for the browser.
    pub sign_in_wait: Duration,
}

struct Inner {
    deps: Deps,
    api: Api,
    phase: Mutex<Phase>,
    name: Mutex<Option<String>>,
    /// A sign-in or read is under way.
    busy: AtomicBool,
}

#[derive(Clone)]
pub struct AccountImport(Arc<Inner>);

/// Clears `busy` when a run ends, however it ends.
struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl AccountImport {
    pub fn new(deps: Deps) -> Self {
        let api = Api::new(deps.web.clone(), deps.keys.clone(), deps.ends.clone(), deps.clock.clone());
        AccountImport(Arc::new(Inner { deps, api, phase: Mutex::new(Phase::Idle), name: Mutex::new(None), busy: AtomicBool::new(false) }))
    }

    /// The settings, with the Client ID from the keychain.
    fn cfg(&self) -> Config {
        let mut c = (self.0.deps.config)();
        c.client_id = self.client_id().unwrap_or_default();
        c
    }

    /// The Client ID kept in the keychain.
    pub fn client_id(&self) -> Option<String> {
        self.0.deps.keys.get(CLIENT_ACCOUNT).ok().flatten().map(|t| t.expose().to_string())
    }

    /// Keeps a new Client ID (letters and digits), or removes it with None
    /// or an empty one. A different app can't use the old sign-in, so a
    /// change signs out.
    pub fn set_client_id(&self, id: Option<&str>) -> Result<AccountStatus, AppError> {
        let id = id.map(str::trim).unwrap_or_default();
        if !id.is_empty() && !(id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())) {
            return Err(AppError::Invalid("A Client ID is the letters and digits on your developer app's page.".into()));
        }
        if self.client_id().unwrap_or_default() == id {
            return Ok(self.status());
        }
        self.sign_out();
        if id.is_empty() {
            self.0.deps.keys.delete(CLIENT_ACCOUNT)?;
        } else {
            self.0.deps.keys.set(CLIENT_ACCOUNT, &crate::plex::token::Token::new(id)?)?;
        }
        self.emit();
        Ok(self.status())
    }

    fn now(&self) -> i64 {
        (self.0.deps.clock)()
    }

    fn last_read(&self) -> Option<i64> {
        self.0
            .deps
            .db
            .read(|c| {
                use rusqlite::OptionalExtension;
                Ok(c.query_row("SELECT imported_at FROM import_sources WHERE source = ?1", [SOURCE], |r| r.get::<_, i64>(0)).optional()?)
            })
            .ok()
            .flatten()
    }

    pub fn status(&self) -> AccountStatus {
        let cfg = self.cfg();
        let signed_in = self.0.api.signed_in();
        let phase = self.0.phase.lock().clone();
        let mut st = AccountStatus {
            state: State::SignedOut,
            redirect_uri: REDIRECT_TO_REGISTER,
            name: self.0.name.lock().clone(),
            message: None,
            retry_at: None,
            done: 0,
            total: 0,
            last_read: self.last_read(),
        };
        if cfg.client_id.is_empty() {
            st.state = State::NotSetUp;
            st.name = None;
            return st;
        }
        st.state = match phase {
            Phase::SigningIn => State::SigningIn,
            Phase::Reading { done, total } => {
                st.done = done;
                st.total = total;
                State::Reading
            }
            Phase::Failed(Fail::Expired) => State::Expired,
            Phase::Failed(Fail::Refused(m)) => {
                st.message = Some(m);
                State::Refused
            }
            Phase::Failed(Fail::Offline(_)) => {
                st.message = Some("You're offline. Platter reads your playlists again once you're back.".into());
                State::Offline
            }
            Phase::Limited(at) => {
                if self.now() < at {
                    st.retry_at = Some(at);
                    State::RateLimited
                } else if signed_in {
                    State::Connected
                } else {
                    State::SignedOut
                }
            }
            Phase::Failed(Fail::Other(m)) | Phase::Failed(Fail::Forbidden(m)) => {
                st.message = Some(m);
                if signed_in { State::Connected } else { State::SignedOut }
            }
            Phase::Failed(Fail::Stopped) | Phase::Failed(Fail::RateLimited(_)) | Phase::Idle => {
                if signed_in {
                    State::Connected
                } else {
                    State::SignedOut
                }
            }
        };
        if !signed_in {
            st.name = None;
        }
        st
    }

    fn set_phase(&self, p: Phase) {
        let p = match p {
            Phase::Failed(Fail::RateLimited(secs)) => Phase::Limited(self.now() + secs as i64 * 1000),
            p => p,
        };
        *self.0.phase.lock() = p;
        self.emit();
    }

    fn emit(&self) {
        let st = self.status();
        self.0.deps.events.emit(EVENT, serde_json::to_value(&st).unwrap_or(Value::Null));
    }

    /// Stops a sign-in or a read.
    pub fn cancel(&self) {
        self.0.api.cancel.store(true, Ordering::SeqCst);
    }

    /// Signs out: the refresh token leaves the keychain. What was imported
    /// stays (Settings has "Remove imported data" for that).
    pub fn sign_out(&self) -> AccountStatus {
        self.cancel();
        let r = self.0.api.forget();
        *self.0.name.lock() = None;
        self.set_phase(match r {
            Ok(()) => Phase::Idle,
            Err(f) => Phase::Failed(f),
        });
        self.status()
    }

    fn take_busy(&self) -> Option<Busy<'_>> {
        if self.0.busy.swap(true, Ordering::SeqCst) {
            return None;
        }
        self.0.api.cancel.store(false, Ordering::SeqCst);
        Some(Busy(&self.0.busy))
    }

    /// Signs in through the browser: a one-off listener on 127.0.0.1, the
    /// sign-in page in the system browser, then the code for tokens.
    pub async fn sign_in(&self) -> AccountStatus {
        let cfg = self.cfg();
        if cfg.client_id.is_empty() || !cfg.enabled {
            return self.status();
        }
        let Some(_busy) = self.take_busy() else { return self.status() };
        self.set_phase(Phase::SigningIn);
        let r = self.sign_in_inner(&cfg.client_id).await;
        self.set_phase(match r {
            Ok(()) => Phase::Idle,
            Err(f) => Phase::Failed(f),
        });
        self.status()
    }

    async fn sign_in_inner(&self, client_id: &str) -> Result<(), Fail> {
        let listener = callback::bind().await?;
        let port = listener.local_addr().map_err(|e| Fail::Other(e.to_string()))?.port();
        let redirect = callback::redirect_uri(port);
        let verifier = pkce::verifier();
        let state = pkce::state();
        let url = self.0.api.authorize_url(client_id, &redirect, &pkce::challenge(&verifier), &state);
        (self.0.deps.open)(&url).map_err(|e| Fail::Other(format!("Couldn't open the browser: {e}")))?;
        let code = callback::wait(listener, &state, self.0.deps.sign_in_wait, self.0.api.cancel.clone()).await?;
        self.0.api.exchange(client_id, &code, &redirect, &verifier).await
    }

    /// Reads the playlists and imports them. Returns where it stands after.
    pub async fn read(&self) -> AccountStatus {
        let cfg = self.cfg();
        if cfg.client_id.is_empty() || !cfg.enabled || !self.0.api.signed_in() {
            return self.status();
        }
        let Some(_busy) = self.take_busy() else { return self.status() };
        self.set_phase(Phase::Reading { done: 0, total: 0 });
        let r = match self.fetch(&cfg.client_id).await {
            Ok(parsed) => self.store(parsed).await,
            Err(f) => Err(f),
        };
        self.set_phase(match r {
            Ok(()) => Phase::Idle,
            Err(f) => Phase::Failed(f),
        });
        self.status()
    }

    /// Everything readable, as an import.
    async fn fetch(&self, client_id: &str) -> Result<Parsed, Fail> {
        let api = &self.0.api;
        let base = api.ends.api.clone();
        let me = match api.get(client_id, &format!("{base}/me")).await {
            Err(Fail::Forbidden(_)) => {
                return Err(Fail::Refused(
                    "The service didn't let your account use this developer app. In the app's settings on the developer dashboard, add your account under User Management, then try again.".into(),
                ));
            }
            other => other?,
        };
        let my_id = me.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        *self.0.name.lock() = me.get("display_name").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).or(Some(my_id.clone()));
        let lists = api.all_items(client_id, &format!("{base}/me/playlists?limit={PAGE}"), |_| {}).await?;
        let total = lists.len() as u64;
        self.set_phase(Phase::Reading { done: 0, total });
        let mut out = Parsed::default();
        for (i, pl) in lists.iter().enumerate() {
            let name = pl.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
            let id = pl.get("id").and_then(Value::as_str).filter(|s| !s.is_empty());
            let owner = pl.pointer("/owner/id").and_then(Value::as_str).unwrap_or_default();
            let collaborative = pl.get("collaborative").and_then(Value::as_bool).unwrap_or(false);
            match (name, id) {
                (Some(name), Some(id)) if owner == my_id || collaborative => {
                    let url = format!("{base}/playlists/{}/items?limit={PAGE}&additional_types=track", url_part(id));
                    match api.all_items(client_id, &url, |_| {}).await {
                        Ok(items) => {
                            let songs = songs_of(&items, &mut out.skipped);
                            out.playlists.push(PlaylistIn { name: name.to_string(), songs });
                        }
                        Err(Fail::Forbidden(_)) => out.skipped.others_playlists += 1,
                        Err(f) => return Err(f),
                    }
                }
                (Some(_), Some(_)) => out.skipped.others_playlists += 1,
                _ => out.skipped.incomplete_rows += 1,
            }
            self.set_phase(Phase::Reading { done: i as u64 + 1, total });
        }
        Ok(out)
    }

    /// Matches and writes on one blocking thread. Playlists that are gone
    /// from the account stay; ones it still has are brought up to date.
    async fn store(&self, parsed: Parsed) -> Result<(), Fail> {
        let db = self.0.deps.db.clone();
        let now = self.now();
        tokio::task::spawn_blocking(move || -> crate::error::AppResult<()> {
            let ranks = db.source_ranks();
            let prepared = db.read(|c| super::store::prepare(c, SOURCE, parsed, ranks))?;
            db.write(|c| super::store::write(c, SOURCE, None, prepared, false, now))?;
            let ranks = db.source_ranks();
            db.write(|c| {
                super::played::keep_up(c, ranks, now);
                Ok(())
            })
        })
        .await
        .map_err(|e| Fail::Other(format!("import task: {e}")))?
        .map_err(|e| Fail::Other(e.to_string()))
    }

    /// Whether a daily read is due now.
    pub fn due(&self) -> bool {
        let cfg = self.cfg();
        if !cfg.enabled || !cfg.recheck || cfg.client_id.is_empty() || !self.0.api.signed_in() || self.0.busy.load(Ordering::SeqCst) {
            return false;
        }
        let st = self.status();
        if matches!(st.state, State::RateLimited | State::Expired | State::Refused) {
            return false;
        }
        st.last_read.is_none_or(|t| self.now() - t >= DAY_MS)
    }

    /// The daily re-check while the app runs.
    pub fn spawn_recheck(&self) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            // Not while the app is starting up.
            tokio::time::sleep(Duration::from_secs(120)).await;
            loop {
                if me.due() {
                    me.read().await;
                }
                tokio::time::sleep(CHECK_EVERY).await;
            }
        });
    }
}

/// An id as one path segment.
fn url_part(id: &str) -> String {
    url::form_urlencoded::byte_serialize(id.as_bytes()).collect()
}

/// A song from a track object.
pub fn song_of(t: &Value) -> Option<SongIn> {
    let title = t.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
    let artist = t
        .get("artists")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find_map(|x| x.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())))?;
    Some(SongIn {
        artist: artist.to_string(),
        title: title.to_string(),
        album: t.pointer("/album/name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
        duration_ms: t.get("duration_ms").and_then(Value::as_u64).filter(|d| *d > 0),
        isrc: t.pointer("/external_ids/isrc").and_then(Value::as_str).and_then(crate::tags::clean_isrc),
        service_id: t.get("uri").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
    })
}

/// The songs of a playlist's items. Episodes are counted as podcasts and
/// left out; items with no song are counted as incomplete.
pub fn songs_of(items: &[Value], skipped: &mut super::Skipped) -> Vec<SongIn> {
    let mut out = Vec::new();
    for it in items {
        // `item` is the current name; `track` the older one.
        let t = it.get("item").filter(|v| !v.is_null()).or_else(|| it.get("track").filter(|v| !v.is_null()));
        let Some(t) = t else {
            skipped.incomplete_rows += 1;
            continue;
        };
        if t.get("type").and_then(Value::as_str) == Some("episode") {
            skipped.podcasts += 1;
            continue;
        }
        match song_of(t) {
            Some(s) => out.push(s),
            None => skipped.incomplete_rows += 1,
        }
    }
    out
}
