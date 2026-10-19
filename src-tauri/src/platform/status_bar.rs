//! The phone's status bar text colour, set from the app's look (the phone
//! would otherwise pick it from whatever art scrolls under it).

use tauri::{AppHandle, Runtime};

use crate::error::AppResult;

/// `dark`: dark text, for a light look. Nothing off the phone.
#[tauri::command]
pub async fn set_status_bar_style<R: Runtime>(app: AppHandle<R>, dark: bool) -> AppResult<()> {
    #[cfg(target_os = "ios")]
    super::ios_webview::set_status_bar_style(&app, dark);
    #[cfg(not(target_os = "ios"))]
    let _ = (app, dark);
    Ok(())
}
