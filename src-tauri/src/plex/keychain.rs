//! Token storage. The real store is the OS keychain (Windows Credential
//! Manager / macOS Keychain) via `keyring`, behind a per-run cache (see
//! [`super::keystore`]); tests use [`MemoryStore`].
//! Tokens never touch disk in plain text and are never logged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;

use super::keystore::{Backend, CachedStore, Layout, Options};
use super::token::Token;
use crate::error::{AppError, AppResult};

pub const KEYRING_SERVICE: &str = "app.platter.player";
/// The service debug builds use, so a test build never reads or replaces the installed app's items (on a Mac every
/// rebuild is a new identity, and reading the app's item asks for the
/// login password).
pub const DEV_KEYRING_SERVICE: &str = "app.platter.player.dev";
/// Account (plex.tv) token.
pub const ACCOUNT_TOKEN: &str = "plex-token";
/// Access token for the selected server, stored only when it differs from the
/// account token (shared servers).
pub const SERVER_TOKEN: &str = "plex-server-token";

pub trait TokenStore: Send + Sync {
    fn get(&self, account: &str) -> AppResult<Option<Token>>;
    fn set(&self, account: &str, token: &Token) -> AppResult<()>;
    /// Remove; succeeds when nothing was stored.
    fn delete(&self, account: &str) -> AppResult<()>;
}

/// The Plex Home user whose keychain entries this run uses (see
/// `plex::home`). Set once at startup, before any store is made; None is
/// the user who signed in, whose entries have no prefix.
static PROFILE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Use `profile`'s keychain entries for the rest of this run. Only the first
/// call counts.
pub fn set_profile(profile: Option<String>) {
    let _ = PROFILE.set(profile);
}

/// The keychain account name for `account` in a profile: unchanged for the
/// user who signed in, `u.<profile>.<account>` for another Home user.
pub fn profile_account(profile: Option<&str>, account: &str) -> String {
    match profile {
        Some(p) => format!("u.{p}.{account}"),
        None => account.to_string(),
    }
}

/// Where the keychain owner marker goes (the app data folder). Set once at
/// startup with [`init`]; without it, items are never re-saved.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// One cache per service, shared by every [`KeyringStore`] in the process,
/// so an entry is read from the OS at most once a run.
static SHARED: OnceLock<Mutex<HashMap<String, Arc<CachedStore>>>> = OnceLock::new();

/// Call before the first keychain use.
pub fn init(data_dir: &Path) {
    let _ = DATA_DIR.set(data_dir.to_path_buf());
}

/// The owner marker file.
pub fn marker_path() -> Option<PathBuf> {
    DATA_DIR.get().map(|d| d.join("keychain-owner.json"))
}

/// The build's signing identity on macOS: the team id the release build was
/// signed with. Unsigned and ad-hoc builds have none.
pub fn signing_owner() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    option_env!("APPLE_TEAM_ID").map(str::trim).filter(|t| !t.is_empty()).map(|t| format!("developer-id:{t}"))
}

/// The keychain service this build uses: [`KEYRING_SERVICE`] for every
/// release build, signed or not, so an update never loses the sign-in,
/// else [`DEV_KEYRING_SERVICE`].
pub fn keyring_service() -> &'static str {
    service_for(cfg!(debug_assertions))
}

fn service_for(debug: bool) -> &'static str {
    if debug { DEV_KEYRING_SERVICE } else { KEYRING_SERVICE }
}

/// Accounts older builds saved as items of their own. Others (AirPlay
/// passwords, ones added later) move into the macOS vault when first read.
pub fn known_accounts() -> Vec<String> {
    let mut out: Vec<String> = vec![
        ACCOUNT_TOKEN.into(),
        SERVER_TOKEN.into(),
        crate::smart::prompt::ANTHROPIC_KEY_ACCOUNT.into(),
        crate::platform::discord::art::IMGUR_ACCOUNT.into(),
        crate::scrobble::lastfm::KEY_ACCOUNT.into(),
        crate::scrobble::lastfm::SECRET_ACCOUNT.into(),
        crate::scrobble::lastfm::SESSION_ACCOUNT.into(),
        crate::scrobble::listenbrainz::TOKEN_ACCOUNT.into(),
    ];
    for s in crate::artistinfo::InfoSource::ALL {
        if s.takes_key() {
            out.push(s.key_account().into());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The layout for this OS: one item on macOS, one entry per account elsewhere.
pub fn os_layout() -> Layout {
    if cfg!(target_os = "macos") { Layout::Vault } else { Layout::PerAccount }
}

/// OS keychain store. Every store for the same service shares one cache.
#[derive(Clone)]
pub struct KeyringStore {
    inner: Arc<CachedStore>,
    /// The Home user whose entries this store reads (None: the user who
    /// signed in).
    profile: Option<String>,
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyringStore {
    /// The app's store, namespaced to this run's Home user.
    pub fn new() -> Self {
        Self::for_profile(PROFILE.get().cloned().flatten().as_deref())
    }

    /// A store for one Home user's entries (switching writes the new user's
    /// token there).
    pub fn for_profile(profile: Option<&str>) -> Self {
        KeyringStore { profile: profile.map(str::to_string), ..Self::with_service(keyring_service()) }
    }

    pub fn with_service(service: &str) -> Self {
        let mut all = SHARED.get_or_init(Mutex::default).lock();
        let inner = all
            .entry(service.to_string())
            .or_insert_with(|| {
                let backend: Arc<dyn Backend> = match dev_secrets_path() {
                    Some(path) => Arc::new(DevFile::new(path)),
                    None => Arc::new(OsKeychain { service: service.to_string() }),
                };
                Arc::new(CachedStore::new(
                    backend,
                    Options {
                        layout: os_layout(),
                        legacy: known_accounts(),
                        owner: signing_owner(),
                        marker: Arc::new(marker_path),
                    },
                ))
            })
            .clone();
        KeyringStore { inner, profile: None }
    }

    fn account(&self, account: &str) -> String {
        profile_account(self.profile.as_deref(), account)
    }
}

impl TokenStore for KeyringStore {
    fn get(&self, account: &str) -> AppResult<Option<Token>> {
        self.inner.get(&self.account(account))
    }

    fn set(&self, account: &str, token: &Token) -> AppResult<()> {
        self.inner.set(&self.account(account), token)
    }

    fn delete(&self, account: &str) -> AppResult<()> {
        self.inner.delete(&self.account(account))
    }
}

/// The OS keychain itself, one item per account, through `keyring`.
struct OsKeychain {
    service: String,
}

impl OsKeychain {
    fn entry(&self, account: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(&self.service, account).map_err(map_keyring)
    }
}

/// Debug builds on a Mac keep their secrets in this file in the app data
/// folder instead of the keychain: every rebuild is a new app to macOS, and
/// each would ask for the login password. None everywhere else.
fn dev_secrets_path() -> Option<PathBuf> {
    if !(cfg!(debug_assertions) && cfg!(target_os = "macos")) {
        return None;
    }
    DATA_DIR.get().map(|d| d.join("dev-secrets.json"))
}

/// Secrets in one JSON file only this user can read (debug builds on a Mac,
/// see [`dev_secrets_path`]).
struct DevFile {
    path: PathBuf,
    lock: Mutex<()>,
}

impl DevFile {
    fn new(path: PathBuf) -> Self {
        DevFile { path, lock: Mutex::new(()) }
    }

    fn load(&self) -> AppResult<HashMap<String, String>> {
        match std::fs::read(&self.path) {
            Ok(b) => serde_json::from_slice(&b).map_err(|_| AppError::Internal("dev secrets file is not valid".into())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(_) => Err(AppError::Internal("dev secrets file can't be read".into())),
        }
    }

    fn save(&self, map: &HashMap<String, String>) -> AppResult<()> {
        let failed = |_| AppError::Internal("dev secrets file can't be saved".into());
        let body = serde_json::to_vec(map).map_err(|_| AppError::Internal("dev secrets file can't be saved".into()))?;
        let tmp = self.path.with_extension("json.new");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        use std::io::Write;
        opts.open(&tmp).and_then(|mut f| f.write_all(&body).and_then(|_| f.sync_all())).map_err(failed)?;
        std::fs::rename(&tmp, &self.path).map_err(failed)
    }
}

impl Backend for DevFile {
    fn read(&self, account: &str) -> AppResult<Option<String>> {
        let _g = self.lock.lock();
        Ok(self.load()?.remove(account))
    }

    fn write(&self, account: &str, value: &str) -> AppResult<()> {
        let _g = self.lock.lock();
        let mut map = self.load()?;
        map.insert(account.to_string(), value.to_string());
        self.save(&map)
    }

    fn remove(&self, account: &str) -> AppResult<()> {
        let _g = self.lock.lock();
        let mut map = self.load()?;
        if map.remove(account).is_some() {
            self.save(&map)?;
        }
        Ok(())
    }
}

/// keyring errors never contain the secret, but keep the message generic.
fn map_keyring(e: keyring::Error) -> AppError {
    let what = match e {
        keyring::Error::NoStorageAccess(_) => "keychain is locked or unavailable",
        keyring::Error::PlatformFailure(_) => "keychain error",
        keyring::Error::BadEncoding(_) => "keychain entry is not valid text",
        keyring::Error::TooLong(..) | keyring::Error::Invalid(..) => "invalid keychain entry",
        keyring::Error::Ambiguous(_) => "several keychain entries match",
        _ => "keychain error",
    };
    AppError::Internal(what.into())
}

impl Backend for OsKeychain {
    fn read(&self, account: &str) -> AppResult<Option<String>> {
        match self.entry(account)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_keyring(e)),
        }
    }

    fn write(&self, account: &str, value: &str) -> AppResult<()> {
        self.entry(account)?.set_password(value).map_err(map_keyring)
    }

    fn remove(&self, account: &str) -> AppResult<()> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_keyring(e)),
        }
    }
}

/// In-memory store for tests and the browser-mock dev mode.
#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<HashMap<String, Token>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenStore for MemoryStore {
    fn get(&self, account: &str) -> AppResult<Option<Token>> {
        Ok(self.map.lock().get(account).cloned())
    }

    fn set(&self, account: &str, token: &Token) -> AppResult<()> {
        self.map.lock().insert(account.to_string(), token.clone());
        Ok(())
    }

    fn delete(&self, account: &str) -> AppResult<()> {
        self.map.lock().remove(account);
        Ok(())
    }
}

/// A store that counts reads per account, for tests that check what's
/// read and when.
#[cfg(test)]
#[derive(Default)]
pub struct CountingStore {
    pub inner: MemoryStore,
    pub reads: Mutex<HashMap<String, usize>>,
}

#[cfg(test)]
impl CountingStore {
    pub fn reads_of(&self, account: &str) -> usize {
        self.reads.lock().get(account).copied().unwrap_or(0)
    }

    pub fn total_reads(&self) -> usize {
        self.reads.lock().values().sum()
    }
}

#[cfg(test)]
impl TokenStore for CountingStore {
    fn get(&self, account: &str) -> AppResult<Option<Token>> {
        *self.reads.lock().entry(account.to_string()).or_default() += 1;
        self.inner.get(account)
    }

    fn set(&self, account: &str, token: &Token) -> AppResult<()> {
        self.inner.set(account, token)
    }

    fn delete(&self, account: &str) -> AppResult<()> {
        self.inner.delete(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_file_keeps_secrets_for_this_user_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dev-secrets.json");
        let f = DevFile::new(path.clone());
        assert_eq!(f.read("vault").unwrap(), None);
        f.write("vault", "one").unwrap();
        f.write("other", "two").unwrap();
        assert_eq!(f.read("vault").unwrap().as_deref(), Some("one"));
        f.remove("vault").unwrap();
        f.remove("vault").unwrap();
        assert_eq!(f.read("vault").unwrap(), None);
        // A new run reads what the last one saved.
        assert_eq!(DevFile::new(path.clone()).read("other").unwrap().as_deref(), Some("two"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A broken file is an error, never an empty store that overwrites it.
        std::fs::write(&path, b"{not json").unwrap();
        assert!(f.read("other").is_err());
        assert!(f.write("other", "x").is_err());
    }

    #[test]
    fn test_builds_keep_their_own_keychain_items() {
        // Release builds keep the app's items, signed or not; debug builds
        // never touch them.
        assert_eq!(service_for(false), KEYRING_SERVICE);
        assert_eq!(service_for(true), DEV_KEYRING_SERVICE);
        assert_ne!(DEV_KEYRING_SERVICE, KEYRING_SERVICE);
        assert_eq!(keyring_service(), service_for(cfg!(debug_assertions)));
    }

    fn round_trip(store: &dyn TokenStore, account: &str) {
        store.delete(account).unwrap();
        assert_eq!(store.get(account).unwrap(), None);
        let t = Token::new("round-trip-value").unwrap();
        store.set(account, &t).unwrap();
        assert_eq!(store.get(account).unwrap(), Some(t));
        let t2 = Token::new("second-value").unwrap();
        store.set(account, &t2).unwrap();
        assert_eq!(store.get(account).unwrap(), Some(t2));
        store.delete(account).unwrap();
        assert_eq!(store.get(account).unwrap(), None);
        // deleting twice is fine
        store.delete(account).unwrap();
    }

    #[test]
    fn every_older_secret_is_known() {
        let k = known_accounts();
        for a in [
            "plex-token",
            "plex-server-token",
            "anthropic-api-key",
            "imgur-client-id",
            "lastfm-api-key",
            "lastfm-api-secret",
            "lastfm-session",
            "listenbrainz-token",
            "fanart-api-key",
            "audiodb-api-key",
        ] {
            assert!(k.iter().any(|x| x == a), "{a} missing");
        }
        assert!(!k.iter().any(|x| x.ends_with("-unused")));
    }

    #[test]
    fn stores_for_one_service_share_a_cache() {
        let a = KeyringStore::with_service("app.platter.player.share-test");
        let b = KeyringStore::with_service("app.platter.player.share-test");
        assert!(Arc::ptr_eq(&a.inner, &b.inner));
    }

    #[test]
    fn memory_round_trip() {
        round_trip(&MemoryStore::new(), ACCOUNT_TOKEN);
    }

    /// Touches the real OS keychain under a test-only service name.
    #[test]
    #[ignore]
    fn keychain_round_trip_real() {
        let store = KeyringStore::with_service("app.platter.player.test");
        round_trip(&store, "plex-token-test");
    }
}
