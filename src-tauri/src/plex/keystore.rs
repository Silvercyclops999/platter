//! What sits behind [`super::keychain::KeyringStore`]: a cache so each entry
//! is read from the OS at most once a run, and the macOS layout.
//!
//! macOS asks for the login password whenever an app reads a keychain item it
//! doesn't own, and an item belongs to the build that first saved it. So on
//! macOS every secret lives in one item (account [`VAULT_ACCOUNT`], a JSON
//! map): a change of signing identity costs one prompt, not one per secret.
//! After reading an item another identity owns, it's saved again (staged copy,
//! delete, add) so this build owns it and later launches don't ask. A marker
//! file in the app data folder remembers which items this identity re-saved.
//!
//! Windows keeps one Credential Manager entry per account (an entry has a
//! size limit, and there is no per-app prompt to avoid).
//!
//! Values never reach logs; errors are generic.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::token::Token;
use crate::error::{AppError, AppResult};

/// The one macOS item that holds every secret.
pub const VAULT_ACCOUNT: &str = "vault";
/// A copy kept while the vault item is being replaced; read back when the
/// vault is missing, so a failed replace never loses anything.
pub const STAGING_ACCOUNT: &str = "vault.new";
/// After a failed read, don't ask the OS again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// Raw OS items, one per account.
pub trait Backend: Send + Sync {
    fn read(&self, account: &str) -> AppResult<Option<String>>;
    /// Add the item, or update it in place when it exists.
    fn write(&self, account: &str, value: &str) -> AppResult<()>;
    /// Ok when nothing was there.
    fn remove(&self, account: &str) -> AppResult<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One OS item per account (Windows, Linux).
    PerAccount,
    /// Every account in one item (macOS).
    Vault,
}

pub type MarkerPath = Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>;

pub struct Options {
    pub layout: Layout,
    /// Accounts older builds kept as items of their own. They move into the
    /// vault the first time it loads; anything not listed moves when first read.
    pub legacy: Vec<String>,
    /// The signing identity of this build. None (unsigned or ad-hoc builds)
    /// never re-saves items.
    pub owner: Option<String>,
    /// The marker file listing items this owner has re-saved.
    pub marker: MarkerPath,
}

pub struct CachedStore {
    os: Arc<dyn Backend>,
    opts: Options,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// What each account holds; None is "nothing stored".
    cache: HashMap<String, Option<Token>>,
    /// Accounts (or the vault) whose last read failed, and when.
    failed: HashMap<String, Instant>,
    /// The vault once loaded.
    vault: Option<Vault>,
}

/// The vault item's contents. No Debug: it holds the values.
#[derive(Serialize, Deserialize, Default, Clone)]
struct Vault {
    #[serde(default)]
    v: u32,
    #[serde(default)]
    items: BTreeMap<String, String>,
    /// The older single items were all checked and moved in.
    #[serde(default)]
    moved: bool,
}

impl Vault {
    fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Marker {
    owner: String,
    accounts: Vec<String>,
}

fn unavailable() -> AppError {
    AppError::Internal("keychain is locked or unavailable".into())
}

impl CachedStore {
    pub fn new(os: Arc<dyn Backend>, opts: Options) -> Self {
        CachedStore { os, opts, state: Mutex::default() }
    }

    pub fn layout(&self) -> Layout {
        self.opts.layout
    }

    pub fn get(&self, account: &str) -> AppResult<Option<Token>> {
        let mut st = self.state.lock();
        if let Some(v) = st.cache.get(account) {
            return Ok(v.clone());
        }
        match self.opts.layout {
            Layout::PerAccount => {
                recently_failed(&st, account)?;
                match self.os.read(account) {
                    Ok(raw) => {
                        st.failed.remove(account);
                        let t = raw.and_then(|s| Token::new(s).ok());
                        st.cache.insert(account.to_string(), t.clone());
                        Ok(t)
                    }
                    Err(e) => {
                        st.failed.insert(account.to_string(), Instant::now());
                        Err(e)
                    }
                }
            }
            Layout::Vault => {
                self.load_vault(&mut st)?;
                if let Some(v) = st.cache.get(account) {
                    return Ok(v.clone());
                }
                // Not in the vault: an older single item may still hold it.
                // Reading a missing item never prompts.
                recently_failed(&st, account)?;
                let raw = match self.os.read(account) {
                    Ok(raw) => raw,
                    Err(e) => {
                        tracing::warn!("keychain: reading an older entry failed: {e}");
                        st.failed.insert(account.to_string(), Instant::now());
                        return Err(e);
                    }
                };
                let t = raw.and_then(|s| Token::new(s).ok());
                if let Some(t) = &t {
                    self.move_in(&mut st, account, t);
                }
                st.cache.insert(account.to_string(), t.clone());
                Ok(t)
            }
        }
    }

    pub fn set(&self, account: &str, token: &Token) -> AppResult<()> {
        let mut st = self.state.lock();
        match self.opts.layout {
            Layout::PerAccount => self.os.write(account, token.expose())?,
            Layout::Vault => {
                st.failed.remove(VAULT_ACCOUNT);
                self.load_vault(&mut st)?;
                let mut next = st.vault.clone().unwrap_or_default();
                next.items.insert(account.to_string(), token.expose().to_string());
                self.save_vault(&mut st, next)?;
                // An older single item for it is stale now.
                if let Err(e) = self.os.remove(account) {
                    tracing::warn!("keychain: removing an older entry failed: {e}");
                }
            }
        }
        st.failed.remove(account);
        st.cache.insert(account.to_string(), Some(token.clone()));
        Ok(())
    }

    pub fn delete(&self, account: &str) -> AppResult<()> {
        let mut st = self.state.lock();
        match self.opts.layout {
            Layout::PerAccount => self.os.remove(account)?,
            Layout::Vault => {
                st.failed.remove(VAULT_ACCOUNT);
                self.load_vault(&mut st)?;
                let mut next = st.vault.clone().unwrap_or_default();
                if next.items.remove(account).is_some() {
                    self.save_vault(&mut st, next)?;
                }
                self.os.remove(account)?;
            }
        }
        st.failed.remove(account);
        st.cache.insert(account.to_string(), None);
        Ok(())
    }

    // ------------------------------------------------------------ vault

    fn load_vault(&self, st: &mut State) -> AppResult<()> {
        if st.vault.is_some() {
            return Ok(());
        }
        recently_failed(st, VAULT_ACCOUNT)?;
        let raw = match self.os.read(VAULT_ACCOUNT) {
            Ok(raw) => raw,
            Err(e) => {
                tracing::warn!("keychain: reading the vault failed: {e}");
                st.failed.insert(VAULT_ACCOUNT.to_string(), Instant::now());
                return Err(e);
            }
        };
        st.failed.remove(VAULT_ACCOUNT);
        let (mut vault, from_os) = match raw {
            Some(s) => (parse(&s), true),
            // A replace that stopped halfway leaves the staged copy.
            None => match self.os.read(STAGING_ACCOUNT) {
                Ok(Some(s)) => {
                    tracing::warn!("keychain: restoring the vault from its staged copy");
                    (parse(&s), false)
                }
                _ => (Vault::default(), false),
            },
        };
        vault.v = 1;
        let was_moved = vault.moved;
        let moved = if was_moved { Vec::new() } else { self.collect_legacy(&mut vault) };
        let dirty = !from_os || !moved.is_empty() || vault.moved != was_moved;
        for (k, v) in &vault.items {
            st.cache.insert(k.clone(), Token::new(v.clone()).ok());
        }
        if !dirty {
            st.vault = Some(vault);
            self.reown_if_needed(st);
            return Ok(());
        }
        let json = vault.to_json();
        let saved = if !from_os {
            // A new item: adding it makes this build its owner.
            self.write_new(&json).map(|()| {
                self.mark_owned(VAULT_ACCOUNT);
                let _ = self.os.remove(STAGING_ACCOUNT);
            })
        } else if self.needs_reown(VAULT_ACCOUNT) {
            match self.replace(VAULT_ACCOUNT, &json) {
                Ok(()) => {
                    self.mark_owned(VAULT_ACCOUNT);
                    Ok(())
                }
                Err(e) => {
                    tracing::warn!("keychain: re-saving the vault failed: {e}");
                    self.write_in_place(&json)
                }
            }
        } else {
            self.write_in_place(&json)
        };
        match saved {
            Ok(()) => {
                st.vault = Some(vault);
                // Only now that the vault holds them are the old items removed.
                for account in moved {
                    if let Err(e) = self.os.remove(&account) {
                        tracing::warn!("keychain: removing an older entry failed: {e}");
                    }
                }
            }
            Err(e) => {
                // The values stay usable this run, and the old items stay put.
                tracing::warn!("keychain: saving the vault failed: {e}");
                vault.moved = false;
                st.vault = Some(vault);
            }
        }
        Ok(())
    }

    /// Read the older single items into `vault`. Returns the accounts read
    /// (removed once the vault is saved). A read that fails leaves `moved`
    /// off so the next launch tries again.
    fn collect_legacy(&self, vault: &mut Vault) -> Vec<String> {
        let mut moved = Vec::new();
        let mut complete = true;
        for account in &self.opts.legacy {
            if account == VAULT_ACCOUNT || account == STAGING_ACCOUNT {
                continue;
            }
            if vault.items.contains_key(account) {
                // The vault's copy is newer; the old one just goes.
                moved.push(account.clone());
                continue;
            }
            match self.os.read(account) {
                Ok(Some(v)) => {
                    if Token::new(v.clone()).is_ok() {
                        vault.items.insert(account.clone(), v);
                    }
                    moved.push(account.clone());
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("keychain: reading an older entry failed: {e}");
                    complete = false;
                }
            }
        }
        vault.moved = complete;
        moved
    }

    /// An older single item read outside the first move: into the vault, then gone.
    fn move_in(&self, st: &mut State, account: &str, token: &Token) {
        let mut next = st.vault.clone().unwrap_or_default();
        next.items.insert(account.to_string(), token.expose().to_string());
        match self.save_vault(st, next) {
            Ok(()) => {
                if let Err(e) = self.os.remove(account) {
                    tracing::warn!("keychain: removing an older entry failed: {e}");
                }
            }
            Err(e) => tracing::warn!("keychain: moving an entry into the vault failed: {e}"),
        }
    }

    /// Write `next` as the vault and keep it in memory once it's saved.
    fn save_vault(&self, st: &mut State, next: Vault) -> AppResult<()> {
        self.write_in_place(&next.to_json())?;
        st.vault = Some(next);
        Ok(())
    }

    fn write_in_place(&self, json: &str) -> AppResult<()> {
        self.os.write(VAULT_ACCOUNT, json)
    }

    /// Add a vault item that didn't exist, trying twice.
    fn write_new(&self, json: &str) -> AppResult<()> {
        self.os.write(VAULT_ACCOUNT, json).or_else(|_| self.os.write(VAULT_ACCOUNT, json))
    }

    /// After reading a vault another identity may own: save it again so
    /// this build owns it, once per identity.
    fn reown_if_needed(&self, st: &mut State) {
        if !self.needs_reown(VAULT_ACCOUNT) {
            return;
        }
        let Some(json) = st.vault.as_ref().map(Vault::to_json) else { return };
        match self.replace(VAULT_ACCOUNT, &json) {
            Ok(()) => self.mark_owned(VAULT_ACCOUNT),
            Err(e) => tracing::warn!("keychain: re-saving the vault failed: {e}"),
        }
    }

    /// Delete and add `account` again with the same value. A staged copy
    /// is written first, so a failure at any step leaves the value in the
    /// keychain somewhere.
    fn replace(&self, account: &str, value: &str) -> AppResult<()> {
        self.os.write(STAGING_ACCOUNT, value)?;
        if let Err(e) = self.os.remove(account) {
            // The original is still there.
            let _ = self.os.remove(STAGING_ACCOUNT);
            return Err(e);
        }
        let added = self.os.write(account, value).or_else(|e| {
            tracing::warn!("keychain: adding the entry back failed, trying once more: {e}");
            self.os.write(account, value)
        });
        match added {
            Ok(()) => {
                if let Err(e) = self.os.remove(STAGING_ACCOUNT) {
                    tracing::warn!("keychain: removing the staged copy failed: {e}");
                }
                Ok(())
            }
            // The staged copy stays and is read back on the next load.
            Err(e) => Err(e),
        }
    }

    // ------------------------------------------------------------ marker

    fn read_marker(&self) -> Option<Marker> {
        let path = (self.opts.marker)()?;
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Signed builds re-save an item once per identity.
    fn needs_reown(&self, account: &str) -> bool {
        let Some(owner) = self.opts.owner.as_deref() else { return false };
        // Without a marker it can't remember doing it, so it never starts.
        if (self.opts.marker)().is_none() {
            return false;
        }
        !self.read_marker().is_some_and(|m| m.owner == owner && m.accounts.iter().any(|a| a == account))
    }

    fn mark_owned(&self, account: &str) {
        let Some(owner) = self.opts.owner.as_deref() else { return };
        let Some(path) = (self.opts.marker)() else { return };
        let mut m = self.read_marker().filter(|m| m.owner == owner).unwrap_or_else(|| Marker {
            owner: owner.to_string(),
            accounts: Vec::new(),
        });
        if !m.accounts.iter().any(|a| a == account) {
            m.accounts.push(account.to_string());
        }
        if let Err(e) = serde_json::to_string(&m).map_err(std::io::Error::other).and_then(|s| std::fs::write(&path, s)) {
            tracing::warn!("keychain: saving the owner marker failed: {e}");
        }
    }
}

fn recently_failed(st: &State, account: &str) -> AppResult<()> {
    match st.failed.get(account) {
        Some(at) if at.elapsed() < RETRY_AFTER => Err(unavailable()),
        _ => Ok(()),
    }
}

fn parse(s: &str) -> Vault {
    serde_json::from_str(s).unwrap_or_else(|_| {
        tracing::warn!("keychain: the vault isn't readable; starting it again");
        Vault::default()
    })
}

#[cfg(test)]
#[path = "keystore_tests.rs"]
mod tests;
