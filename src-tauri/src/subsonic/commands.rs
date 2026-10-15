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
#[tauri::command]
pub async fn subsonic_sign_in(
    os: State<'_, Subsonic>,
    url: String,
    user: Option<String>,
    api_key: Option<String>,
    password: Option<String>,
) -> AppResult<SavedServer> {
    os.sign_in(&url, user, api_key, password).await
}
