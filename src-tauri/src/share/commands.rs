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
    crate::platform::show_window(app);
}

impl<R: Runtime> ShareEvents for AppHandle<R> {
    fn link_waiting(&self) {
        focus_main(self);
        let _ = self.emit("deeplink://open", ());
    }
}

/// Wire the deep-link plugin to the share service: a link that started the
/// app waits for the UI; later ones arrive as events. Command links
/// (`platter://next`) go to the control API's automation side instead; one
/// that started the app is dropped (there's nothing to control yet).
pub fn install<R: Runtime>(app: &AppHandle<R>, share: &ShareService) {
    use crate::feed::deck::automation::parse_link;
    let at_start = app.deep_link().get_current().ok().flatten().unwrap_or_default();
    let s = share.clone();
    let handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            if let Some(cmd) = parse_link(url.as_str()) {
                if let Some(deck) = handle.try_state::<crate::feed::deck::server::DeckServer>() {
                    let deck = deck.inner().clone();
                    tauri::async_runtime::spawn(async move { deck.run_link(cmd).await });
                }
                continue;
            }
            let s = s.clone();
            tauri::async_runtime::spawn(async move { s.open(url.as_str()).await });
        }
    });
    for url in at_start {
        if parse_link(url.as_str()).is_some() {
            tracing::info!("command link at start ignored");
            continue;
        }
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

// ---------------------------------------------------------------- share cards

fn header_text(request: &tauri::ipc::Request<'_>, name: &str) -> Option<String> {
    request.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

fn raw_body(request: &tauri::ipc::Request<'_>) -> AppResult<Vec<u8>> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(b) => Ok(b.clone()),
        _ => Err(crate::error::AppError::Invalid("The card image didn't arrive.".into())),
    }
}

/// The QR code for an item's Platter link (`share_card_qr`).
#[tauri::command]
pub async fn share_card_qr(
    share: State<'_, ShareService>,
    kind: ShareKind,
    id: String,
    at_seconds: Option<u32>,
) -> AppResult<super::card::CardQr> {
    share.card_qr(kind, &id, at_seconds).await
}

/// Put a card on the clipboard. The body is the card's RGBA pixels; the
/// `x-card-width` and `x-card-height` headers give its size.
#[tauri::command]
pub async fn share_card_copy(app: AppHandle, request: tauri::ipc::Request<'_>) -> AppResult<()> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let num = |n: &str| header_text(&request, n).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let (w, h) = (num("x-card-width"), num("x-card-height"));
    let rgba = raw_body(&request)?;
    super::card::check_rgba(rgba.len(), w, h)?;
    let image = tauri::image::Image::new_owned(rgba, w, h);
    app.clipboard().write_image(&image).map_err(|e| {
        tracing::warn!("card to clipboard: {e}");
        crate::error::AppError::Internal("Couldn't copy the image.".into())
    })
}

/// Save a card as a PNG through the save dialog. The body is the PNG; the
/// `x-card-name` header (percent-encoded) suggests a name. Answers the saved
/// file's name, or None when the dialog was cancelled.
#[tauri::command]
pub async fn share_card_save(app: AppHandle, request: tauri::ipc::Request<'_>) -> AppResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let png = raw_body(&request)?;
    super::card::check_png(&png)?;
    let title = header_text(&request, "x-card-name")
        .map(|n| percent_encoding::percent_decode_str(&n).decode_utf8_lossy().into_owned())
        .unwrap_or_default();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Save share card")
        .set_file_name(super::card::file_name(&title))
        .add_filter("PNG image", &["png"])
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| crate::error::AppError::Io(format!("save path: {e}")))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    tokio::task::spawn_blocking(move || std::fs::write(&path, png))
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("save: {e}")))??;
    Ok(Some(name))
}
