//! Identify by sound: songs whose files have no MusicBrainz recording id
//! (or every song, as Settings says) are fingerprinted on this computer and
//! looked up on AcoustID, which names the MusicBrainz recording they sound
//! like.
//!
//! - [`print`]    the fingerprint, from the first two minutes of the file
//! - [`acoustid`] the lookup (only the fingerprint and the file's length
//!   are sent), paced under the service's limit
//! - [`store`]    results per song, cached until the file changes
//! - [`commands`] the Tauri commands
//!
//! What it finds feeds the matcher's fingerprint tier (`identity::`), the
//! library doctor's "sounds like" finding and the hint on unreleased songs.
//! An unreleased song is never matched to the recording it sounds like:
//! the identity rules refuse any match for it, so it only ever gets the hint.
//!
//! Work is light: off until the user turns it on, one file at a time,
//! reads at the lowest priority on the player's transfer gate (waiting
//! while playback downloads), a pause between files, and each file done
//! once. Lookups need an AcoustID application key: the one the build was
//! made with (`PLATTER_ACOUSTID_KEY`), or the user's own from Settings,
//! kept in the keychain. Without either nothing runs.

pub mod acoustid;
pub mod commands;
pub mod print;
pub mod store;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::db::DbHandle;
use crate::error::AppResult;
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;
use crate::tags::fetch::{FetchError, Reader};
use crate::tags::scan::PartSource;
use acoustid::{LookupError, SoundLookup};
use store::Todo;

/// Fired when results or progress change.
pub const EV_CHANGED: &str = "soundid://changed";
/// Keychain account for the user's own AcoustID application key.
pub const KEY_ACCOUNT: &str = "acoustid-key";

/// Which songs to identify (Settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    /// Files with no recording id or ISRC in their tags.
    #[default]
    Untagged,
    All,
}

/// What Settings says (pushed by `settings::publish`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    /// Identify songs by sound, and not offline.
    pub on: bool,
    pub scope: Scope,
}

static OPTIONS: LazyLock<RwLock<Options>> = LazyLock::new(|| RwLock::new(Options::default()));
static WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);

pub fn options() -> Options {
    *OPTIONS.read()
}

/// Set from the settings; turning it on or changing the songs wakes it.
pub fn set_options(o: Options) {
    let changed = {
        let mut w = OPTIONS.write();
        let changed = *w != o;
        *w = o;
        changed
    };
    if changed {
        WAKE.notify_one();
    }
}

/// The key the build was made with, if any.
pub fn app_key() -> Option<&'static str> {
    option_env!("PLATTER_ACOUSTID_KEY").map(str::trim).filter(|k| !k.is_empty())
}

fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Bytes per read: small enough that playback taking the gate costs little.
const CHUNK: u64 = 1024 * 1024;
/// Startup work (the first sync, Home) goes first.
const START_DELAY: Duration = Duration::from_secs(180);
const LOOK_EVERY: Duration = Duration::from_secs(120);
/// Songs per round, and the pause between them.
const PER_ROUND: usize = 20;
const GAP: Duration = Duration::from_secs(2);
/// After network or service trouble.
const BACKOFF: Duration = Duration::from_secs(300);

/// How one song went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Identified (true) or no match (false).
    Done(bool),
    /// Couldn't be read or fingerprinted; not tried again until it changes.
    Failed,
    /// Fingerprinted; the lookup waits (key refused, service busy).
    Waiting(LookupError),
    /// Network or server trouble reading the file: stop for now.
    Later,
}

/// The service: settings, the key, and the background work.
#[derive(Clone)]
pub struct SoundId(Arc<Inner>);

struct Inner {
    db: DbHandle,
    parts: Arc<dyn PartSource>,
    reader: Reader,
    lookup: Arc<dyn SoundLookup>,
    keys: Arc<dyn TokenStore>,
    /// Read from the keychain once (None: not read yet).
    user_key: Mutex<Option<Option<Token>>>,
    built_in: Option<String>,
    refused: AtomicBool,
    running: AtomicBool,
    paused: AtomicBool,
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

/// Key state for Settings (IPC `SoundIdStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Identify songs by sound is on (and not offline).
    pub on: bool,
    /// This build came with an AcoustID key.
    pub app_key: bool,
    /// The user saved their own key on this device.
    pub user_key: bool,
    /// AcoustID refused the key in use.
    pub key_refused: bool,
    /// There's a key to use.
    pub configured: bool,
    pub running: bool,
    /// Waiting after network or service trouble.
    pub paused: bool,
    #[serde(flatten)]
    pub counts: store::Counts,
}

impl SoundId {
    pub fn new(
        db: DbHandle,
        parts: Arc<dyn PartSource>,
        reader: Reader,
        lookup: Arc<dyn SoundLookup>,
        keys: Arc<dyn TokenStore>,
        built_in: Option<&str>,
    ) -> Self {
        Self(Arc::new(Inner {
            db,
            parts,
            reader,
            lookup,
            keys,
            user_key: Mutex::new(None),
            built_in: built_in.map(str::to_string),
            refused: AtomicBool::new(false),
            running: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            on_change: Mutex::new(None),
        }))
    }

    /// Call `f` when results or progress change (fires `soundid://changed`).
    pub fn set_on_change(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *self.0.on_change.lock() = Some(f);
    }

    pub fn db(&self) -> &DbHandle {
        &self.0.db
    }

    fn changed(&self) {
        let f = self.0.on_change.lock().clone();
        if let Some(f) = f {
            f();
        }
    }

    fn user_key(&self) -> Option<Token> {
        let mut k = self.0.user_key.lock();
        if k.is_none() {
            *k = Some(self.0.keys.get(KEY_ACCOUNT).unwrap_or_else(|e| {
                tracing::warn!("reading the AcoustID key: {e}");
                None
            }));
        }
        k.clone().flatten()
    }

    /// The key lookups use: the user's own, else the build's.
    pub fn key(&self) -> Option<String> {
        self.user_key().map(|t| t.expose().to_string()).or_else(|| self.0.built_in.clone())
    }

    /// Save the user's own key (None removes it, going back to the build's).
    pub fn set_key(&self, key: Option<String>) -> AppResult<()> {
        let token = match key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => {
                if k.len() > 100 || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return Err(crate::error::AppError::Invalid("That doesn't look like an AcoustID application key.".into()));
                }
                Some(Token::new(k)?)
            }
            None => None,
        };
        match &token {
            Some(t) => self.0.keys.set(KEY_ACCOUNT, t)?,
            None => self.0.keys.delete(KEY_ACCOUNT)?,
        }
        *self.0.user_key.lock() = Some(token);
        self.0.refused.store(false, Ordering::Relaxed);
        WAKE.notify_one();
        Ok(())
    }

    pub async fn status(&self) -> AppResult<Status> {
        let me = self.clone();
        let user_key = tokio::task::spawn_blocking(move || me.user_key().is_some())
            .await
            .map_err(|e| crate::error::AppError::Internal(e.to_string()))?;
        let scope = options().scope;
        let counts = self.0.db.read_async(move |c| store::counts(c, scope)).await?;
        Ok(Status {
            on: options().on,
            app_key: self.0.built_in.is_some(),
            user_key,
            key_refused: self.0.refused.load(Ordering::Relaxed),
            configured: user_key || self.0.built_in.is_some(),
            running: self.0.running.load(Ordering::Relaxed),
            paused: self.0.paused.load(Ordering::Relaxed),
            counts,
        })
    }

    /// Start the background work for the life of the app.
    pub fn spawn(&self) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.watch().await });
    }

    async fn watch(self) {
        tokio::time::sleep(START_DELAY).await;
        loop {
            let _ = tokio::time::timeout(LOOK_EVERY, WAKE.notified()).await;
            crate::platform::foreground::settled().await;
            if !options().on || self.0.refused.load(Ordering::Relaxed) || self.key().is_none() {
                continue;
            }
            self.0.running.store(true, Ordering::Relaxed);
            self.changed();
            let r = self.run_round(options().scope, PER_ROUND, GAP, &|| options().on).await;
            self.0.running.store(false, Ordering::Relaxed);
            let trouble = matches!(r, Ok(Some(_)));
            self.0.paused.store(trouble, Ordering::Relaxed);
            if let Err(e) = &r {
                tracing::warn!("identify by sound: {e}");
            }
            self.changed();
            if trouble {
                tokio::time::sleep(BACKOFF).await;
            } else if r.is_ok() && self.0.db.read_async(|c| store::pending(c, options().scope, 1)).await.is_ok_and(|v| !v.is_empty()) {
                // More to do: carry on soon.
                WAKE.notify_one();
            }
        }
    }

    /// One round: waiting lookups, then up to `limit` new songs, `gap`
    /// apart. Some(outcome) when it stopped early for trouble. Does nothing
    /// without a key or after the key was refused.
    pub async fn run_round(&self, scope: Scope, limit: usize, gap: Duration, keep_going: &(dyn Fn() -> bool + Sync)) -> AppResult<Option<Outcome>> {
        let Some(key) = self.key() else { return Ok(None) };
        if self.0.refused.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let mut identified = Vec::new();
        let waiting = self.0.db.read_async(move |c| store::printed(c, limit)).await?;
        for (track, fp, secs) in waiting {
            if !keep_going() {
                return Ok(None);
            }
            match self.look_up(&key, &track, &fp, secs).await? {
                Outcome::Done(true) => identified.push(track),
                Outcome::Done(false) | Outcome::Failed => {}
                stop => {
                    self.finish(identified).await?;
                    return Ok(Some(stop));
                }
            }
        }
        let todo = self.0.db.read_async(move |c| store::pending(c, scope, limit)).await?;
        for t in todo {
            if !keep_going() {
                break;
            }
            let id = t.track_id.clone();
            match self.identify(&key, &t).await? {
                Outcome::Done(true) => identified.push(id),
                Outcome::Done(false) | Outcome::Failed => {}
                stop => {
                    self.finish(identified).await?;
                    return Ok(Some(stop));
                }
            }
            self.changed();
            tokio::time::sleep(gap).await;
        }
        self.0.db.write_async(|c| store::prune(c)).await?;
        self.finish(identified).await?;
        Ok(None)
    }

    /// The matcher and the library doctor pick up new results.
    async fn finish(&self, identified: Vec<String>) -> AppResult<()> {
        if identified.is_empty() {
            return Ok(());
        }
        self.0.db.write_async(move |c| crate::identity::index::refresh_for_tracks(c, &identified)).await?;
        crate::doctor::poke();
        Ok(())
    }

    /// Fingerprint one song and look it up.
    pub async fn identify(&self, key: &str, t: &Todo) -> AppResult<Outcome> {
        let src = match self.0.parts.part(&t.track_id, &t.part_key).await {
            Ok(s) => s,
            Err(_) => return Ok(Outcome::Later),
        };
        let want = print::bytes_wanted(t.bitrate, t.duration_ms, src.size);
        let bytes = match read_head(&self.0.reader, &src, want).await {
            Ok(b) => b,
            Err(FetchError::Transient(_)) => return Ok(Outcome::Later),
            Err(_) => {
                let t = t.clone();
                self.0.db.write_async(move |c| store::save_failed(c, &t, now_s())).await?;
                return Ok(Outcome::Failed);
            }
        };
        let hint = crate::audio::decode::extension_hint(t.codec.as_deref(), t.container.as_deref());
        // Decoding is CPU work: off the async threads.
        let fp = tokio::task::spawn_blocking(move || print::fingerprint(bytes, hint.as_deref()))
            .await
            .map_err(|e| crate::error::AppError::Internal(e.to_string()))?;
        let fp = match fp {
            Ok(fp) => fp,
            Err(e) => {
                tracing::debug!("identify by sound: {e}");
                let t = t.clone();
                self.0.db.write_async(move |c| store::save_failed(c, &t, now_s())).await?;
                return Ok(Outcome::Failed);
            }
        };
        // AcoustID wants the whole file's length.
        let secs = ((t.duration_ms + 500) / 1000) as u32;
        {
            let (t, fp) = (t.clone(), fp.clone());
            self.0.db.write_async(move |c| store::save_print(c, &t, &fp, secs, now_s())).await?;
        }
        self.look_up(key, &t.track_id, &fp, secs).await
    }

    async fn look_up(&self, key: &str, track_id: &str, fp: &str, secs: u32) -> AppResult<Outcome> {
        let track = track_id.to_string();
        match self.0.lookup.lookup(key, fp, secs).await {
            Ok(hits) => {
                let found = self.0.db.write_async(move |c| store::save_result(c, &track, &hits, now_s())).await?;
                Ok(Outcome::Done(found))
            }
            Err(LookupError::Rejected(m)) => {
                tracing::debug!("identify by sound: {m}");
                self.0.db.write_async(move |c| store::save_rejected(c, &track, now_s())).await?;
                Ok(Outcome::Done(false))
            }
            Err(LookupError::KeyRefused) => {
                self.0.refused.store(true, Ordering::Relaxed);
                Ok(Outcome::Waiting(LookupError::KeyRefused))
            }
            Err(e) => Ok(Outcome::Waiting(e)),
        }
    }
}

/// The first `want` bytes of a file (all of it when shorter), a chunk at a
/// time through the tag scan's reader.
pub async fn read_head(reader: &Reader, src: &crate::audio::StreamSource, want: u64) -> Result<Vec<u8>, FetchError> {
    let mut out: Vec<u8> = Vec::new();
    while (out.len() as u64) < want {
        let len = CHUNK.min(want - out.len() as u64) as usize;
        let got = match reader.range(src, out.len() as u64, len).await {
            Ok(g) => g,
            // Asked past the end of a file exactly a chunk long.
            Err(FetchError::File(_)) if !out.is_empty() => break,
            Err(e) => return Err(e),
        };
        let n = got.bytes.len();
        out.extend(got.bytes);
        if got.whole || n < len || got.total.is_some_and(|t| out.len() as u64 >= t) {
            break;
        }
    }
    Ok(out)
}
