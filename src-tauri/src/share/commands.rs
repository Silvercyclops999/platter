//! Tauri side of sharing (docs/IPC.md "Sharing"): the commands, the server
//! id from Plex, and the `platter://` handler.
//!
//! On Windows a link starts a second copy of the app with the link as its
//! argument; the single-instance plugin hands it to the running copy, which
//! the deep-link plugin turns into an open-url event. On macOS the system
//! sends the event directly. Either way the link lands in [`ShareService::open`].

use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_deep_link::DeepLinkExt;

use super::{DeepLinkOpen, ServerId, ShareEvents, ShareInfo, ShareKind, ShareService};
use crate::error::AppResult;
use crate::plex::PlexState;

/// The server id from the signed-in Plex server.
pub struct PlexServerId(pub PlexState);

#[async_trait::async_trait]
impl ServerId for PlexServerId {
    async fn machine_id(&self) -> Option<String> {
        self.0.machine_id().await
    }
}

/// Bring the main window forward.
pub fn focus_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

impl<R: Runtime> ShareEvents for AppHandle<R> {
    fn link_waiting(&self) {
        focus_main(self);
        let _ = self.emit("deeplink://open", ());
    }
}

/// Wire the deep-link plugin to the share service: a link that started the
/// app waits for the UI; later ones arrive as events.
pub fn install<R: Runtime>(app: &AppHandle<R>, share: &ShareService) {
    let at_start = app.deep_link().get_current().ok().flatten().unwrap_or_default();
    let s = share.clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            let s = s.clone();
            tauri::async_runtime::spawn(async move { s.open(url.as_str()).await });
        }
    });
    for url in at_start {
        let s = share.clone();
        tauri::async_runtime::spawn(async move { s.open(url.as_str()).await });
    }
}

#[tauri::command]
pub async fn share_info(
    share: State<'_, ShareService>,
    kind: ShareKind,
    id: String,
    at_seconds: Option<u32>,
) -> AppResult<ShareInfo> {
    share.info(kind, &id, at_seconds).await
}

#[tauri::command]
pub async fn share_public_link(share: State<'_, ShareService>, kind: ShareKind, id: String) -> AppResult<String> {
    share.public_link(kind, &id).await
}

#[tauri::command]
pub async fn deep_link_take(share: State<'_, ShareService>) -> AppResult<Option<DeepLinkOpen>> {
    Ok(share.take())
}
