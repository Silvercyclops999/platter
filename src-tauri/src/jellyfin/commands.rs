//! Commands for adding and managing servers (docs/IPC.md "Servers"). After
//! each change `backends://changed` tells the UI to ask again.

use tauri::State;

use super::manager::{Jellyfin, QuickCode, QuickPoll, SavedServer, ServerCheck, ServerLibrary};
use crate::error::AppResult;
use crate::subsonic::Subsonic;

#[tauri::command]
pub async fn servers_saved(jf: State<'_, Jellyfin>, os: State<'_, Subsonic>) -> AppResult<Vec<SavedServer>> {
    let mut list = jf.saved();
    os.patch(&mut list);
    Ok(list)
}

#[tauri::command]
pub async fn server_set_primary(jf: State<'_, Jellyfin>, tag: String) -> AppResult<()> {
    jf.set_primary(&tag)
}

#[tauri::command]
pub async fn server_remove(jf: State<'_, Jellyfin>, os: State<'_, Subsonic>, tag: String) -> AppResult<()> {
    if os.owns(&tag) {
        return os.remove(&tag).await;
    }
    jf.remove(&tag).await
}

#[tauri::command]
pub async fn server_libraries(jf: State<'_, Jellyfin>, os: State<'_, Subsonic>, tag: String) -> AppResult<Vec<ServerLibrary>> {
    if os.owns(&tag) {
        return os.libraries(&tag).await;
    }
    jf.libraries(&tag).await
}

/// `ids` null includes every library.
#[tauri::command]
pub async fn server_libraries_set(jf: State<'_, Jellyfin>, os: State<'_, Subsonic>, tag: String, ids: Option<Vec<String>>) -> AppResult<()> {
    if os.owns(&tag) {
        return os.set_libraries(&tag, ids);
    }
    jf.set_libraries(&tag, ids)
}

#[tauri::command]
pub async fn jellyfin_check(jf: State<'_, Jellyfin>, url: String) -> AppResult<ServerCheck> {
    jf.check(&url).await
}

#[tauri::command]
pub async fn jellyfin_quick_start(jf: State<'_, Jellyfin>, url: String) -> AppResult<QuickCode> {
    jf.quick_start(&url).await
}

#[tauri::command]
pub async fn jellyfin_quick_poll(jf: State<'_, Jellyfin>) -> AppResult<QuickPoll> {
    jf.quick_poll().await
}

#[tauri::command]
pub async fn jellyfin_quick_cancel(jf: State<'_, Jellyfin>) -> AppResult<()> {
    jf.quick_cancel();
    Ok(())
}

/// The password goes to the server once and is dropped; it's never stored
/// or logged.
#[tauri::command]
pub async fn jellyfin_sign_in_password(jf: State<'_, Jellyfin>, url: String, user: String, password: String) -> AppResult<SavedServer> {
    jf.password(&url, &user, password).await
}
