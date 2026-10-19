//! The Bridge's secrets (the link secret, Lidarr's key, its Plex sign-in)
//! in one file on its data volume that only the Bridge's user can read
//! (0600, put right again if it was ever looser). There is no keychain
//! inside a container. Nothing secret is kept anywhere else on the volume.

use std::collections::BTreeMap;
use std::path::PathBuf;

use parking_lot::Mutex;

use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;

pub struct FileKeys {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileKeys {
    pub fn new(path: PathBuf) -> Self {
        Self { path, lock: Mutex::new(()) }
    }

    fn load(&self) -> BTreeMap<String, String> {
        keep_private(&self.path);
        std::fs::read_to_string(&self.path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save(&self, map: &BTreeMap<String, String>) -> AppResult<()> {
        let json = serde_json::to_string(map).map_err(|e| AppError::Internal(e.to_string()))?;
        let tmp = self.path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp);
        write_private(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, &self.path).map_err(|_| AppError::Internal("couldn't save the keys file".into()))?;
        keep_private(&self.path);
        Ok(())
    }
}

/// Only the owner may read or write the file.
#[cfg(unix)]
pub fn keep_private(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(m) = std::fs::metadata(path)
        && m.permissions().mode() & 0o077 != 0
    {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

#[cfg(not(unix))]
pub fn keep_private(_path: &std::path::Path) {}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> AppResult<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| AppError::Internal("couldn't write the keys file".into()))?;
    keep_private(path);
    f.write_all(bytes).map_err(|_| AppError::Internal("couldn't write the keys file".into()))
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> AppResult<()> {
    std::fs::write(path, bytes).map_err(|_| AppError::Internal("couldn't write the keys file".into()))
}

impl TokenStore for FileKeys {
    fn get(&self, account: &str) -> AppResult<Option<Token>> {
        let _one = self.lock.lock();
        match self.load().get(account) {
            Some(v) => Ok(Some(Token::new(v.clone())?)),
            None => Ok(None),
        }
    }

    fn set(&self, account: &str, token: &Token) -> AppResult<()> {
        let _one = self.lock.lock();
        let mut m = self.load();
        m.insert(account.to_string(), token.expose().to_string());
        self.save(&m)
    }

    fn delete(&self, account: &str) -> AppResult<()> {
        let _one = self.lock.lock();
        let mut m = self.load();
        if m.remove(account).is_some() {
            self.save(&m)?;
        }
        Ok(())
    }
}
