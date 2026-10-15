//! Plex Home users: the list on plex.tv, switching to another user (with
//! their PIN when they have one), and the per-user data folders.
//!
//! Each Home user gets their own folder under the app data folder
//! (`users/<key>/`: library cache, settings, queue, downloads, sync store)
//! and their own keychain entries (`u.<key>.` names, see
//! [`super::keychain::profile_account`]). The user who signed in keeps the
//! top folder and the plain keychain names, so nothing moves for an account
//! that never switches.
//!
//! Switching asks plex.tv for the other user's token
//! (`POST /api/v2/home/users/{uuid}/switch`), stores it in that user's
//! keychain entries, remembers who is active in `home-user.json`, and the app
//! restarts into that user's folder. It never writes to a Plex server. The
//! PIN is sent once and never stored.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::auth::{PlexTv, PlexUser};
use super::client::{check_status, map_send_error};
use super::keychain::{ACCOUNT_TOKEN, SERVER_TOKEN, TokenStore};
use super::lenient;
use super::token::Token;
use crate::error::{AppError, AppResult};

/// Who is active on this device, next to the per-user folders.
pub const PROFILE_FILE: &str = "home-user.json";
/// Per-user folders live here, under the app data folder.
pub const USERS_DIR: &str = "users";
pub const EVENT_HOME_CHANGED: &str = "home://changed";

/// One user from `GET /api/v2/home/users`. Fields we don't use (email, the
/// admin's own PIN) are left out on purpose.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawHomeUser {
    #[serde(default, deserialize_with = "lenient::string")]
    pub id: String,
    #[serde(default, deserialize_with = "lenient::string")]
    pub uuid: String,
    #[serde(default, deserialize_with = "lenient::string")]
    pub title: String,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    pub username: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    pub friendly_name: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub thumb: String,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub admin: bool,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub guest: bool,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub restricted: bool,
    /// Switching to this user needs their PIN.
    #[serde(default, deserialize_with = "lenient::bool")]
    pub protected: bool,
}

impl RawHomeUser {
    pub fn name(&self) -> String {
        [self.friendly_name.as_deref(), Some(self.title.as_str()), self.username.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|s| !s.is_empty())
            .unwrap_or("Plex user")
            .to_string()
    }

    fn avatar(&self) -> Option<String> {
        PlexUser { uuid: self.uuid.clone(), username: String::new(), title: String::new(), thumb: self.thumb.clone() }
            .avatar_art_path()
    }
}

#[derive(Debug, Deserialize)]
struct RawHome {
    #[serde(default)]
    users: Vec<RawHomeUser>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSwitched {
    #[serde(default)]
    auth_token: Option<String>,
}

impl PlexTv {
    /// `GET /api/v2/home/users`. An account that isn't a Plex Home answers
    /// with an error or just itself; both come back as no users.
    pub async fn home_users(&self, token: &Token) -> AppResult<Vec<RawHomeUser>> {
        let resp = self
            .http()
            .get(self.url_for("/api/v2/home/users")?)
            .header("x-plex-token", token.header_value()?)
            .send()
            .await
            .map_err(map_send_error)?;
        if matches!(resp.status().as_u16(), 400 | 403 | 404) {
            return Ok(Vec::new());
        }
        let home: RawHome = check_status(resp).await?.json().await.map_err(map_send_error)?;
        Ok(home.users)
    }

    /// `POST /api/v2/home/users/{uuid}/switch`: the other user's token. The
    /// PIN goes in the form body (never the URL). A wrong PIN is
    /// [`wrong_pin`].
    pub async fn switch_home_user(&self, token: &Token, uuid: &str, pin: Option<&str>) -> AppResult<Token> {
        if uuid.is_empty() || !uuid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(AppError::Invalid("Unknown Home user.".into()));
        }
        let mut form: Vec<(&str, &str)> = Vec::new();
        if let Some(p) = pin {
            form.push(("pin", p));
        }
        let resp = self
            .http()
            .post(self.url_for(&format!("/api/v2/home/users/{uuid}/switch"))?)
            .header("x-plex-token", token.header_value()?)
            .form(&form)
            .send()
            .await
            .map_err(map_send_error)?;
        match resp.status().as_u16() {
            401 | 403 | 422 => return Err(wrong_pin(pin.is_some())),
            404 => return Err(AppError::NotFound("That Home user isn't in your Plex Home anymore.".into())),
            _ => {}
        }
        let raw: RawSwitched = check_status(resp).await?.json().await.map_err(map_send_error)?;
        raw.auth_token
            .and_then(|t| Token::new(t).ok())
            .ok_or_else(|| AppError::Decode("plex.tv sent no token for that user".into()))
    }
}

/// plex.tv refused the switch: the PIN was wrong, or one was needed.
pub fn wrong_pin(had_pin: bool) -> AppError {
    AppError::Invalid(if had_pin { "That PIN isn't right." } else { "Enter this user's PIN." }.into())
}

/// A PIN as typed: 4 digits.
pub fn valid_pin(pin: &str) -> bool {
    pin.len() == 4 && pin.bytes().all(|b| b.is_ascii_digit())
}

/// Short folder and keychain key for a user: 16 hex digits of a hash of
/// their uuid, so names never show on disk.
pub fn profile_key(uuid: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in uuid.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// `home-user.json`: who is active on this device. Device-wide, outside the
/// per-user folders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profiles {
    /// The uuid of the Home user to start as; None = the user who signed in.
    #[serde(default)]
    pub active: Option<String>,
    /// The uuid of the user who signed in (owns the top folder), learned on
    /// the first switch.
    #[serde(default)]
    pub owner: Option<String>,
    /// Settings: remember the last user on this device. Off starts as the
    /// user who signed in.
    #[serde(default = "yes")]
    pub remember: bool,
}

fn yes() -> bool {
    true
}

impl Default for Profiles {
    fn default() -> Self {
        Profiles { active: None, owner: None, remember: true }
    }
}

impl Profiles {
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join(PROFILE_FILE))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> AppResult<()> {
        std::fs::create_dir_all(root)?;
        let tmp = root.join(format!("{PROFILE_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(|e| AppError::Internal(e.to_string()))?)?;
        std::fs::rename(&tmp, root.join(PROFILE_FILE))?;
        Ok(())
    }

    /// The active user's key (None = the user who signed in).
    pub fn key(&self) -> Option<String> {
        self.active.as_deref().filter(|a| self.owner.as_deref() != Some(*a)).map(profile_key)
    }
}

/// The data folder for a profile key.
pub fn data_dir(root: &Path, key: Option<&str>) -> PathBuf {
    match key {
        Some(k) => root.join(USERS_DIR).join(k),
        None => root.to_path_buf(),
    }
}

/// At startup: which user to run as and their data folder. With "remember
/// last user" off, the user who signed in. A user's folder starts with the
/// device's Plex client id, so Plex sees one device whoever is active.
pub fn startup(root: &Path) -> (Option<String>, PathBuf) {
    let mut p = Profiles::load(root);
    if !p.remember && p.active.is_some() {
        p.active = None;
        if let Err(e) = p.save(root) {
            tracing::warn!("home user reset not saved: {e}");
        }
    }
    let key = p.key();
    let dir = data_dir(root, key.as_deref());
    if key.is_some() {
        let id = super::identity::ID_FILE;
        if std::fs::create_dir_all(&dir).is_ok() && !dir.join(id).exists() {
            let _ = std::fs::copy(root.join(id), dir.join(id));
        }
    }
    (key, dir)
}

/// IPC: one Home user (docs/IPC.md "Home users").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeUser {
    pub uuid: String,
    pub name: String,
    /// Profile picture as an `/ext/` art path (no token).
    pub avatar: Option<String>,
    pub admin: bool,
    pub restricted: bool,
    /// Switching needs their PIN.
    pub protected: bool,
    /// The user Platter is running as.
    pub current: bool,
}

/// IPC: `home_users`. Empty `users` when the account isn't a Plex Home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeUsers {
    pub users: Vec<HomeUser>,
    /// Settings: remember the last user on this device.
    pub remember: bool,
}

/// Keychain stores per profile key (the OS keychain in the app, memory in
/// tests).
pub trait ProfileStores: Send + Sync {
    fn store(&self, key: Option<&str>) -> Arc<dyn TokenStore>;
}

pub struct KeyringProfiles;

impl ProfileStores for KeyringProfiles {
    fn store(&self, key: Option<&str>) -> Arc<dyn TokenStore> {
        Arc::new(super::keychain::KeyringStore::for_profile(key))
    }
}

/// The outcome of a switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switched {
    /// The new user's profile key (None = the user who signed in).
    pub key: Option<String>,
    /// The app must restart into another folder.
    pub restart: bool,
}

/// Home user list and switching for this run.
#[derive(Clone)]
pub struct Home {
    root: PathBuf,
    /// This run's profile key.
    current: Option<String>,
    tv: PlexTv,
    stores: Arc<dyn ProfileStores>,
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl Home {
    pub fn new(root: PathBuf, current: Option<String>, tv: PlexTv, stores: Arc<dyn ProfileStores>) -> Self {
        Home { root, current, tv, stores, lock: Arc::default() }
    }

    fn token(&self) -> AppResult<Token> {
        self.stores
            .store(self.current.as_deref())
            .get(ACCOUNT_TOKEN)?
            .ok_or_else(|| AppError::Auth("Not signed in.".into()))
    }

    /// The uuid of the user who signed in, learned (and saved) from plex.tv
    /// while running as them.
    async fn owner(&self, p: &mut Profiles, token: &Token) -> AppResult<Option<String>> {
        if p.owner.is_none() && self.current.is_none() {
            let me = self.tv.user(token).await?;
            if !me.uuid.is_empty() {
                p.owner = Some(me.uuid);
                p.save(&self.root)?;
            }
        }
        Ok(p.owner.clone())
    }

    /// The account's Home users, with the one Platter runs as marked. Not
    /// a Home (or only one user): no users.
    pub async fn users(&self) -> AppResult<HomeUsers> {
        let mut p = Profiles::load(&self.root);
        let remember = p.remember;
        let Ok(token) = self.token() else {
            return Ok(HomeUsers { users: Vec::new(), remember });
        };
        let raw = self.tv.home_users(&token).await?;
        if raw.len() < 2 {
            return Ok(HomeUsers { users: Vec::new(), remember });
        }
        let owner = self.owner(&mut p, &token).await.unwrap_or(None);
        let current_uuid = match &self.current {
            None => owner.clone(),
            Some(_) => p.active.clone(),
        };
        let users = raw
            .iter()
            .filter(|u| !u.uuid.is_empty())
            .map(|u| HomeUser {
                uuid: u.uuid.clone(),
                name: u.name(),
                avatar: u.avatar(),
                admin: u.admin,
                restricted: u.restricted,
                protected: u.protected,
                current: current_uuid.as_deref() == Some(u.uuid.as_str()),
            })
            .collect();
        Ok(HomeUsers { users, remember })
    }

    /// Switch to another Home user. Gets their token from plex.tv (with
    /// their PIN when given), stores it in their keychain entries and makes
    /// them the active user. Nothing on any Plex server changes; the current
    /// user's folder and keychain entries are left as they are.
    pub async fn switch(&self, uuid: &str, pin: Option<&str>) -> AppResult<Switched> {
        let _one = self.lock.lock().await;
        if let Some(p) = pin
            && !valid_pin(p)
        {
            return Err(AppError::Invalid("A PIN is 4 digits.".into()));
        }
        let token = self.token()?;
        let mut p = Profiles::load(&self.root);
        let owner = self.owner(&mut p, &token).await?;
        let new_token = self.tv.switch_home_user(&token, uuid, pin).await?;
        let key = if owner.as_deref() == Some(uuid) { None } else { Some(profile_key(uuid)) };
        let store = self.stores.store(key.as_deref());
        store.set(ACCOUNT_TOKEN, &new_token)?;
        // A shared server's token from another session would be stale.
        let _ = store.delete(SERVER_TOKEN);
        p.active = key.as_ref().map(|_| uuid.to_string());
        p.save(&self.root)?;
        if let Some(k) = &key {
            let dir = data_dir(&self.root, Some(k));
            std::fs::create_dir_all(&dir)?;
            let id = super::identity::ID_FILE;
            if !dir.join(id).exists() {
                let _ = std::fs::copy(self.root.join(id), dir.join(id));
            }
        }
        Ok(Switched { restart: key != self.current, key })
    }

    /// Signed out while running as another Home user: the next start is the
    /// user who signed in again. True when that needs a restart.
    pub fn signed_out(&self) -> AppResult<bool> {
        if self.current.is_none() {
            return Ok(false);
        }
        let mut p = Profiles::load(&self.root);
        p.active = None;
        p.save(&self.root)?;
        Ok(true)
    }

    /// Settings: remember the last user on this device.
    pub fn set_remember(&self, on: bool) -> AppResult<()> {
        let mut p = Profiles::load(&self.root);
        p.remember = on;
        p.save(&self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::identity::ClientIdentity;
    use crate::plex::keychain::MemoryStore;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    #[derive(Default)]
    struct Stores(Mutex<HashMap<Option<String>, Arc<MemoryStore>>>);

    impl ProfileStores for Stores {
        fn store(&self, key: Option<&str>) -> Arc<dyn TokenStore> {
            self.0.lock().entry(key.map(str::to_string)).or_default().clone()
        }
    }

    impl Stores {
        fn token(&self, key: Option<&str>) -> Option<String> {
            self.store(key).get(ACCOUNT_TOKEN).unwrap().map(|t| t.expose().to_string())
        }
    }

    fn users_json() -> serde_json::Value {
        serde_json::json!({
            "id": 1, "name": "Home", "guestEnabled": false,
            "users": [
                { "id": 11, "uuid": "owner-uuid", "title": "Owner", "username": "owner", "thumb": "https://plex.tv/users/a/avatar?c=1",
                  "admin": true, "guest": false, "restricted": false, "protected": true, "pin": "9999" },
                { "id": 12, "uuid": "kid-uuid", "title": "Kid", "username": null, "friendlyName": null,
                  "thumb": "https://plex.tv/users/b/avatar?c=2", "admin": false, "restricted": true, "protected": false },
                { "id": 13, "uuid": "partner-uuid", "title": "", "friendlyName": "Partner", "thumb": "",
                  "admin": false, "restricted": false, "protected": true }
            ]
        })
    }

    async fn plex_tv() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/home/users"))
            .respond_with(ResponseTemplate::new(200).set_body_json(users_json()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/user"))
            .and(header("x-plex-token", "owner-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "uuid": "owner-uuid", "username": "owner" })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v2/home/users/kid-uuid/switch"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "uuid": "kid-uuid", "authToken": "kid-token" })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v2/home/users/partner-uuid/switch"))
            .and(body_string("pin=1234"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "authToken": "partner-token" })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v2/home/users/partner-uuid/switch"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({ "errors": [{ "code": 1041 }] })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v2/home/users/owner-uuid/switch"))
            .and(body_string("pin=9999"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "authToken": "owner-token-2" })))
            .mount(&server)
            .await;
        server
    }

    fn home(server: &MockServer, root: &Path, current: Option<&str>, stores: Arc<Stores>) -> Home {
        let tv = PlexTv::with_base(Arc::new(ClientIdentity::with_id("cid")), &server.uri()).unwrap();
        Home::new(root.to_path_buf(), current.map(str::to_string), tv, stores)
    }

    fn signed_in_owner() -> (tempfile::TempDir, Arc<Stores>) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(crate::plex::identity::ID_FILE), "4f1c2a6e-7d1b-4c1a-9f7a-2b3c4d5e6f70").unwrap();
        let stores = Arc::new(Stores::default());
        stores.store(None).set(ACCOUNT_TOKEN, &Token::new("owner-token").unwrap()).unwrap();
        (root, stores)
    }

    #[tokio::test]
    async fn lists_home_users_with_the_current_one_marked() {
        let server = plex_tv().await;
        let (root, stores) = signed_in_owner();
        let h = home(&server, root.path(), None, stores);
        let list = h.users().await.unwrap();
        assert!(list.remember);
        let names: Vec<(&str, bool, bool)> = list.users.iter().map(|u| (u.name.as_str(), u.current, u.protected)).collect();
        assert_eq!(names, vec![("Owner", true, true), ("Kid", false, false), ("Partner", false, true)]);
        assert_eq!(list.users[1].avatar.as_deref(), Some("/ext/https://plex.tv/users/b/avatar?c=2"));
        assert_eq!(list.users[2].avatar, None);
        // The owner is learned once and kept on this device.
        assert_eq!(Profiles::load(root.path()).owner.as_deref(), Some("owner-uuid"));
    }

    #[tokio::test]
    async fn an_account_without_a_home_lists_nobody() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/home/users"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let (root, stores) = signed_in_owner();
        let list = home(&server, root.path(), None, stores).users().await.unwrap();
        assert!(list.users.is_empty());
    }

    #[tokio::test]
    async fn switching_without_a_pin_stores_the_token_in_the_new_users_entries() {
        let server = plex_tv().await;
        let (root, stores) = signed_in_owner();
        let before = std::fs::read_dir(root.path()).unwrap().count();
        let h = home(&server, root.path(), None, stores.clone());
        let s = h.switch("kid-uuid", None).await.unwrap();
        let key = profile_key("kid-uuid");
        assert_eq!(s, Switched { key: Some(key.clone()), restart: true });
        assert_eq!(stores.token(Some(&key)).as_deref(), Some("kid-token"));
        assert_eq!(stores.token(None).as_deref(), Some("owner-token"), "the signed-in user's entry is untouched");
        let p = Profiles::load(root.path());
        assert_eq!(p.active.as_deref(), Some("kid-uuid"));
        assert_eq!(p.key(), Some(key.clone()));
        // The new user's folder exists with the same Plex client id.
        let dir = data_dir(root.path(), Some(&key));
        assert_eq!(
            std::fs::read_to_string(dir.join(crate::plex::identity::ID_FILE)).unwrap(),
            std::fs::read_to_string(root.path().join(crate::plex::identity::ID_FILE)).unwrap()
        );
        // Only the profile file and the users folder were added at the top.
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), before + 2);
        // Only plex.tv was asked, and nothing but the switch was written.
        let posts: Vec<Request> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method.as_str() == "POST")
            .collect();
        assert_eq!(posts.len(), 1);
        assert!(posts[0].url.path().ends_with("/kid-uuid/switch"));
        assert!(posts[0].url.query().is_none(), "nothing in the URL");
    }

    #[tokio::test]
    async fn switching_with_a_pin_sends_it_once_and_never_keeps_it() {
        let server = plex_tv().await;
        let (root, stores) = signed_in_owner();
        let h = home(&server, root.path(), None, stores.clone());
        h.switch("partner-uuid", Some("1234")).await.unwrap();
        assert_eq!(stores.token(Some(&profile_key("partner-uuid"))).as_deref(), Some("partner-token"));
        let saved = std::fs::read_to_string(root.path().join(PROFILE_FILE)).unwrap();
        assert!(!saved.contains("1234"));
    }

    #[tokio::test]
    async fn a_wrong_pin_changes_nothing() {
        let server = plex_tv().await;
        let (root, stores) = signed_in_owner();
        let h = home(&server, root.path(), None, stores.clone());
        let err = h.switch("partner-uuid", Some("0000")).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(ref m) if m == "That PIN isn't right."), "{err:?}");
        let err = h.switch("partner-uuid", None).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(ref m) if m == "Enter this user's PIN."), "{err:?}");
        assert_eq!(stores.token(Some(&profile_key("partner-uuid"))), None);
        assert_eq!(Profiles::load(root.path()).active, None);
        let err = h.switch("partner-uuid", Some("12a4")).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn switching_back_to_the_owner_uses_the_top_folder() {
        let server = plex_tv().await;
        let (root, stores) = signed_in_owner();
        home(&server, root.path(), None, stores.clone()).switch("kid-uuid", None).await.unwrap();
        // Now running as the kid.
        let kid = profile_key("kid-uuid");
        let h = home(&server, root.path(), Some(&kid), stores.clone());
        let list = h.users().await.unwrap();
        assert_eq!(list.users.iter().find(|u| u.current).map(|u| u.uuid.as_str()), Some("kid-uuid"));
        let s = h.switch("owner-uuid", Some("9999")).await.unwrap();
        assert_eq!(s, Switched { key: None, restart: true });
        assert_eq!(stores.token(None).as_deref(), Some("owner-token-2"));
        assert_eq!(stores.token(Some(&kid)).as_deref(), Some("kid-token"), "the kid's entry stays for next time");
        assert_eq!(Profiles::load(root.path()).active, None);
    }

    #[test]
    fn each_user_gets_their_own_folder_and_forgetting_the_last_user_starts_as_the_owner() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(crate::plex::identity::ID_FILE), "id").unwrap();
        assert_eq!(startup(root.path()), (None, root.path().to_path_buf()));
        let mut p = Profiles { active: Some("kid-uuid".into()), owner: Some("owner-uuid".into()), remember: true };
        p.save(root.path()).unwrap();
        let (key, dir) = startup(root.path());
        assert_eq!(key, Some(profile_key("kid-uuid")));
        assert_eq!(dir, root.path().join("users").join(profile_key("kid-uuid")));
        assert!(dir.join(crate::plex::identity::ID_FILE).exists());
        assert_ne!(profile_key("kid-uuid"), profile_key("partner-uuid"));
        // Separate library caches.
        let a = crate::db::DbHandle::open(root.path().join("library.sqlite")).unwrap();
        let b = crate::db::DbHandle::open(dir.join("library.sqlite")).unwrap();
        a.write(|c| crate::db::store::kv_set(c, "who", "owner")).unwrap();
        assert_eq!(b.read(|c| crate::db::store::kv_get(c, "who")).unwrap(), None);
        // Remember last user off: back to the owner at the next start.
        p.remember = false;
        p.save(root.path()).unwrap();
        assert_eq!(startup(root.path()), (None, root.path().to_path_buf()));
        assert_eq!(Profiles::load(root.path()).active, None);
    }

    #[test]
    fn home_users_parse_leniently_and_leave_secrets_out() {
        let raw: RawHome = serde_json::from_value(users_json()).unwrap();
        assert_eq!(raw.users.len(), 3);
        assert_eq!(raw.users[2].name(), "Partner");
        let shown = serde_json::to_string(&HomeUser {
            uuid: raw.users[0].uuid.clone(),
            name: raw.users[0].name(),
            avatar: None,
            admin: true,
            restricted: false,
            protected: true,
            current: true,
        })
        .unwrap();
        assert!(!shown.contains("9999"));
        assert!(valid_pin("0042") && !valid_pin("42") && !valid_pin("abcd"));
    }
}
