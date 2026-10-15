//! Token storage. The real store is the OS keychain (Windows Credential
//! Manager / macOS Keychain) via `keyring`; tests use [`MemoryStore`].
//! Tokens never touch disk in plain text and are never logged.

use std::collections::HashMap;

use parking_lot::Mutex;

use super::token::Token;
use crate::error::{AppError, AppResult};

pub const KEYRING_SERVICE: &str = "app.platter.player";
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

/// OS keychain store.
#[derive(Debug, Default)]
pub struct KeyringStore {
    service: String,
    profile: Option<String>,
}

impl KeyringStore {
    /// The app's store, namespaced to this run's Home user.
    pub fn new() -> Self {
        KeyringStore {
            service: KEYRING_SERVICE.to_string(),
            profile: PROFILE.get().cloned().flatten(),
        }
    }

    /// A store for one Home user's entries (switching writes the new user's
    /// token there).
    pub fn for_profile(profile: Option<&str>) -> Self {
        KeyringStore {
            service: KEYRING_SERVICE.to_string(),
            profile: profile.map(str::to_string),
        }
    }

    pub fn with_service(service: &str) -> Self {
        KeyringStore {
            service: service.to_string(),
            profile: None,
        }
    }

    fn entry(&self, account: &str) -> AppResult<keyring::Entry> {
        let account = profile_account(self.profile.as_deref(), account);
        keyring::Entry::new(&self.service, &account).map_err(map_keyring)
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

impl TokenStore for KeyringStore {
    fn get(&self, account: &str) -> AppResult<Option<Token>> {
        match self.entry(account)?.get_password() {
            Ok(s) => Ok(Token::new(s).ok()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_keyring(e)),
        }
    }

    fn set(&self, account: &str, token: &Token) -> AppResult<()> {
        self.entry(account)?
            .set_password(token.expose())
            .map_err(map_keyring)
    }

    fn delete(&self, account: &str) -> AppResult<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn home_users_get_their_own_keychain_names() {
        assert_eq!(profile_account(None, ACCOUNT_TOKEN), "plex-token");
        assert_eq!(profile_account(Some("0a1b2c3d"), ACCOUNT_TOKEN), "u.0a1b2c3d.plex-token");
        assert_ne!(profile_account(Some("a"), ACCOUNT_TOKEN), profile_account(Some("b"), ACCOUNT_TOKEN));
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
