//! Which hosts a keychain key was entered for. Addresses can arrive from the
//! user's other devices (synced settings), keys never do, so a key is only
//! ever sent to a host the user had in front of them on this device: where
//! the address was when the key was entered, or an address typed here.
//!
//! The record lives in this device's `kv` table, which never syncs. A key
//! saved before this record existed is taken as entered for the addresses
//! set at the time it's first read.

use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::AppResult;

fn kv_key(account: &str) -> String {
    format!("keyHosts.{account}")
}

/// The host and port of an address, lowercased (`10.20.30.40:8686`). None
/// for a blank or unreadable address.
pub fn host_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url.trim()).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    Some(match u.port_or_known_default() {
        Some(p) => format!("{host}:{p}"),
        None => host,
    })
}

/// The host as the user would read it (no default port).
pub fn shown_host(url: &str) -> String {
    url::Url::parse(url.trim())
        .ok()
        .and_then(|u| {
            let h = u.host_str()?.to_string();
            Some(match u.port() {
                Some(p) => format!("{h}:{p}"),
                None => h,
            })
        })
        .unwrap_or_else(|| url.trim().to_string())
}

fn read(db: &DbHandle, account: &str) -> Option<Vec<String>> {
    db.read(|c| kv_get(c, &kv_key(account))).ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
}

fn write(db: &DbHandle, account: &str, hosts: &[String]) -> AppResult<()> {
    let json = serde_json::to_string(hosts).map_err(|e| crate::error::AppError::Internal(e.to_string()))?;
    db.write(|c| kv_set(c, &kv_key(account), &json))
}

/// The key for `account` was entered with these addresses set: only their
/// hosts get it from now on.
pub fn bind(db: &DbHandle, account: &str, urls: &[&str]) -> AppResult<()> {
    let mut hosts: Vec<String> = urls.iter().filter_map(|u| host_of(u)).collect();
    hosts.sort();
    hosts.dedup();
    write(db, account, &hosts)
}

/// The user typed this address on this device: its host may get the key.
pub fn allow(db: &DbHandle, account: &str, url: &str) -> AppResult<()> {
    let Some(h) = host_of(url) else { return Ok(()) };
    let mut hosts = read(db, account).unwrap_or_default();
    if !hosts.contains(&h) {
        hosts.push(h);
        write(db, account, &hosts)?;
    }
    Ok(())
}

/// Record the hosts for a key saved before this record existed: the
/// addresses set now. Does nothing once a record is there.
pub fn adopt(db: &DbHandle, account: &str, urls: &[&str]) -> AppResult<()> {
    if read(db, account).is_none() {
        bind(db, account, urls)?;
    }
    Ok(())
}

/// Whether the key for `account` may be sent to `url`. `current` are the
/// addresses set now, taken as the key's when nothing is recorded yet.
pub fn trusted(db: &DbHandle, account: &str, url: &str, current: &[&str]) -> bool {
    let Some(h) = host_of(url) else { return false };
    let hosts = match read(db, account) {
        Some(h) => h,
        None => {
            if let Err(e) = bind(db, account, current) {
                tracing::warn!("key hosts not saved: {e}");
            }
            current.iter().filter_map(|u| host_of(u)).collect()
        }
    };
    hosts.contains(&h)
}

/// The words for an address the key wasn't entered for.
pub fn moved_words(service: &str, url: &str) -> String {
    format!("{service} moved to {}. Enter its key on this device to use it.", shown_host(url))
}

/// Keychain accounts whose key is bound to an address in Settings, with
/// the settings holding those addresses.
pub fn startup_adopt(db: &DbHandle) {
    let s = crate::settings::load(db);
    let pairs: [(&str, Vec<&str>); 2] = [
        (crate::lidarr::KEY_ACCOUNT, vec![s.lidarr_url.as_str(), s.lidarr_away_url.as_str()]),
        (crate::playhistory::tautulli::KEY_ACCOUNT, vec![s.tautulli_url.as_str()]),
    ];
    for (account, urls) in pairs {
        if let Err(e) = adopt(db, account, &urls) {
            tracing::warn!("key hosts not saved: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, DbHandle) {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("lib.db")).unwrap();
        (dir, db)
    }

    #[test]
    fn hosts_are_host_and_port() {
        assert_eq!(host_of("http://10.20.30.40:8686/lidarr").as_deref(), Some("10.20.30.40:8686"));
        assert_eq!(host_of("https://NAS.example.com").as_deref(), Some("nas.example.com:443"));
        assert_eq!(host_of(""), None);
        assert_eq!(shown_host("https://nas.example.com/x"), "nas.example.com");
        assert_eq!(shown_host("http://10.20.30.41:8686"), "10.20.30.41:8686");
    }

    #[test]
    fn a_key_goes_only_to_the_hosts_it_was_entered_for() {
        let (_d, db) = db();
        bind(&db, "k", &["http://10.20.30.40:8686", ""]).unwrap();
        assert!(trusted(&db, "k", "http://10.20.30.40:8686/", &[]));
        assert!(!trusted(&db, "k", "http://10.20.30.99:8686", &["http://10.20.30.99:8686"]), "a new host isn't taken");
        allow(&db, "k", "http://10.20.30.99:8686").unwrap();
        assert!(trusted(&db, "k", "http://10.20.30.99:8686", &[]));
    }

    #[test]
    fn a_key_from_before_the_record_keeps_working_once() {
        let (_d, db) = db();
        assert!(trusted(&db, "k", "http://10.20.30.40:8686", &["http://10.20.30.40:8686"]));
        // Recorded now: a later address isn't.
        assert!(!trusted(&db, "k", "http://10.20.30.50:8686", &["http://10.20.30.50:8686"]));
        adopt(&db, "k", &["http://10.20.30.50:8686"]).unwrap();
        assert!(!trusted(&db, "k", "http://10.20.30.50:8686", &[]), "adopt does nothing once recorded");
    }
}
