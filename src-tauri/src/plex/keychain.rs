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

/// OS keychain store.
#[derive(Debug, Default)]
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new() -> Self {
        Self::with_service(KEYRING_SERVICE)
    }

    pub fn with_service(service: &str) -> Self {
        KeyringStore {
            service: service.to_string(),
        }
    }

    fn entry(&self, account: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(&self.service, account).map_err(map_keyring)
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
