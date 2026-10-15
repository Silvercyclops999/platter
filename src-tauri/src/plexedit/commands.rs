//! Tauri commands for editing in Plex (docs/IPC.md "Edit in Plex").

use tauri::State;

use super::{EditAccess, EditChange, EditInfo, PlexEditor};
use crate::error::AppResult;
use crate::plex::edit::{EditKind, ImageChoice, ImageKind, MatchCandidate, Snapshot};

#[tauri::command]
pub async fn plex_edit_access(editor: State<'_, PlexEditor>) -> AppResult<EditAccess> {
    Ok(editor.access().await)
}

#[tauri::command]
pub async fn plex_edit_item(editor: State<'_, PlexEditor>, id: String) -> AppResult<EditInfo> {
    editor.item(&id).await
}

#[tauri::command]
pub async fn plex_edit_images(editor: State<'_, PlexEditor>, id: String, which: ImageKind) -> AppResult<Vec<ImageChoice>> {
    editor.images(&id, which).await
}

#[tauri::command]
pub async fn plex_edit_select_image(
    editor: State<'_, PlexEditor>,
    id: String,
    kind: EditKind,
    which: ImageKind,
    key: String,
) -> AppResult<EditChange> {
    editor.select_image(&id, kind, which, key).await
}

#[tauri::command]
pub async fn plex_edit_upload_image(
    editor: State<'_, PlexEditor>,
    id: String,
    kind: EditKind,
    which: ImageKind,
    bytes: Vec<u8>,
    content_type: String,
) -> AppResult<EditChange> {
    editor.upload_image(&id, kind, which, bytes, content_type).await
}

#[tauri::command]
pub async fn plex_edit_image_link(
    editor: State<'_, PlexEditor>,
    id: String,
    kind: EditKind,
    which: ImageKind,
    url: String,
) -> AppResult<EditChange> {
    editor.image_link(&id, kind, which, url).await
}

#[tauri::command]
pub async fn plex_edit_matches(
    editor: State<'_, PlexEditor>,
    id: String,
    title: Option<String>,
    year: Option<i64>,
) -> AppResult<Vec<MatchCandidate>> {
    editor.matches(&id, title, year).await
}

#[tauri::command]
pub async fn plex_edit_match(
    editor: State<'_, PlexEditor>,
    id: String,
    kind: EditKind,
    guid: String,
    name: String,
    year: Option<i64>,
) -> AppResult<EditChange> {
    editor.set_match(&id, kind, guid, name, year).await
}

#[tauri::command]
pub async fn plex_edit_unmatch(editor: State<'_, PlexEditor>, id: String, kind: EditKind) -> AppResult<EditChange> {
    editor.unmatch(&id, kind).await
}

#[tauri::command]
pub async fn plex_edit_details(editor: State<'_, PlexEditor>, id: String, fields: Snapshot) -> AppResult<EditChange> {
    editor.edit_details(&id, fields).await
}

#[tauri::command]
pub async fn plex_edit_merge(
    editor: State<'_, PlexEditor>,
    target: String,
    others: Vec<String>,
    photo_from: Option<String>,
) -> AppResult<EditChange> {
    editor.merge(&target, others, photo_from).await
}

#[tauri::command]
pub async fn plex_edit_split(editor: State<'_, PlexEditor>, id: String) -> AppResult<EditChange> {
    editor.split(&id).await
}

#[tauri::command]
pub async fn plex_edit_refresh(editor: State<'_, PlexEditor>, id: String, kind: EditKind) -> AppResult<EditChange> {
    editor.refresh(&id, kind).await
}

#[tauri::command]
pub async fn plex_edit_undo(editor: State<'_, PlexEditor>, change_id: String) -> AppResult<EditChange> {
    editor.undo(&change_id).await
}

#[tauri::command]
pub async fn plex_edit_history(editor: State<'_, PlexEditor>, id: String) -> AppResult<Vec<EditChange>> {
    Ok(editor.history(&id).await)
}
