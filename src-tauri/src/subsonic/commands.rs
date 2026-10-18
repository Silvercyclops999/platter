//! Commands for adding a Navidrome or other Subsonic server (docs/IPC.md
//! "Servers"). The shared server commands (list, primary, libraries,
//! remove) live with the Jellyfin ones and hand Subsonic servers here.

use tauri::State;

use super::manager::{Subsonic, SubsonicCheck};
use crate::error::AppResult;
use crate::jellyfin::manager::SavedServer;

#[tauri::command]
pub async fn subsonic_check(os: State<'_, Subsonic>, url: String) -> AppResult<SubsonicCheck> {
    os.check(&url).await
}

/// An API key, or a user name and password. The key or password is kept in
/// the keychain (Subsonic signs every request with it), never logged.
/// `tag`: signing in again to that server, which keeps its tag.
#[tauri::command]
pub async fn subsonic_sign_in(
    os: State<'_, Subsonic>,
    url: String,
    user: Option<String>,
    api_key: Option<String>,
    password: Option<String>,
    tag: Option<String>,
) -> AppResult<SavedServer> {
    os.sign_in_to(tag.as_deref(), &url, user, api_key, password).await
}

/// A saved server at a new address, checked to be the same server first.
#[tauri::command]
pub async fn subsonic_change_address(os: State<'_, Subsonic>, tag: String, url: String) -> AppResult<SavedServer> {
    os.change_address(&tag, &url).await
}
