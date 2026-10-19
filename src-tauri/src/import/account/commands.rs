//! Tauri commands for the connected account. Each returns where the
//! connection stands; progress and changes also go out as
//! `streaming-account://status`.

use tauri::State;

use super::{AccountImport, AccountStatus};
use crate::error::AppResult;

#[tauri::command]
pub async fn streaming_account_status(account: State<'_, AccountImport>) -> AppResult<AccountStatus> {
    Ok(account.status())
}

/// Opens the sign-in page in the browser and waits for its answer, then
/// reads the playlists in the background.
#[tauri::command]
pub async fn streaming_account_sign_in(account: State<'_, AccountImport>) -> AppResult<AccountStatus> {
    let st = account.sign_in().await;
    if st.state == super::State::Connected {
        let a = account.inner().clone();
        tauri::async_runtime::spawn(async move {
            a.read().await;
        });
    }
    Ok(st)
}

/// Reads the playlists now and imports them.
#[tauri::command]
pub async fn streaming_account_read(account: State<'_, AccountImport>) -> AppResult<AccountStatus> {
    Ok(account.read().await)
}

/// Stops a sign-in or a read.
#[tauri::command]
pub async fn streaming_account_cancel(account: State<'_, AccountImport>) -> AppResult<AccountStatus> {
    account.cancel();
    Ok(account.status())
}

#[tauri::command]
pub async fn streaming_account_sign_out(account: State<'_, AccountImport>) -> AppResult<AccountStatus> {
    Ok(account.sign_out())
}
