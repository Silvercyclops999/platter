//! Commands for adding a Navidrome or other Subsonic server (docs/IPC.md
//! "Servers"). The shared server commands (list, primary, libraries,
//! remove) live with the Jellyfin ones and hand Subsonic servers here.

use tauri::State;

use super::manager::{Confirm, Subsonic, SubsonicCheck};
use crate::error::AppResult;
use crate::jellyfin::manager::SavedServer;

#[tauri::command]
pub async fn subsonic_check(os: State<'_, Subsonic>, url: String) -> AppResult<SubsonicCheck> {
    os.check(&url).await
}

/// An API key, or a user name and password. The key or password is kept in
/// the keychain (Subsonic signs every request with it), never logged.
/// `tag`: signing in again to that server, which keeps its tag.
/// `same_server`: the user said a new address is the same server, for when
/// Platter can't tell. `allow_insecure`: the user said plain http is fine.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn subsonic_sign_in(
    os: State<'_, Subsonic>,
    url: String,
    user: Option<String>,
    api_key: Option<String>,
    password: Option<String>,
    tag: Option<String>,
    same_server: Option<bool>,
    allow_insecure: Option<bool>,
) -> AppResult<SavedServer> {
    let confirm = Confirm { same_server: same_server.unwrap_or(false), allow_insecure: allow_insecure.unwrap_or(false) };
    os.sign_in_to(tag.as_deref(), &url, user, api_key, password, confirm).await
}

/// A saved server at a new address, checked to be the same server first.
/// The key or password is typed again for the new address; the saved one
/// is never sent there.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn subsonic_change_address(
    os: State<'_, Subsonic>,
    tag: String,
    url: String,
    user: Option<String>,
    api_key: Option<String>,
    password: Option<String>,
    same_server: Option<bool>,
    allow_insecure: Option<bool>,
) -> AppResult<SavedServer> {
    let confirm = Confirm { same_server: same_server.unwrap_or(false), allow_insecure: allow_insecure.unwrap_or(false) };
    os.change_address(&tag, &url, user, api_key, password, confirm).await
}
