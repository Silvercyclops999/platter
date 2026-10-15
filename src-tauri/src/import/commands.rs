//! Library import commands. Nothing here writes to Plex: saving an imported
//! playlist or liking its songs there uses the usual playlist and like
//! commands, from an explicit click.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use super::store::{self, Decision, ImportedList, ImportedListPage, MissingSong, ReviewItem, SourceReport};
use super::{files, sources};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::settings::SettingsState;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Everything the import screens show.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOverview {
    pub reports: Vec<SourceReport>,
    pub lists: Vec<ImportedList>,
    pub review_count: u64,
    pub missing_count: u64,
}

#[tauri::command]
pub async fn import_overview(db: State<'_, DbHandle>) -> AppResult<ImportOverview> {
    db.read_async(|c| {
        let (review_count, missing_count) = store::queue_counts(c)?;
        Ok(ImportOverview { reports: store::reports(c)?, lists: store::lists(c)?, review_count, missing_count })
    })
    .await
}

/// Opens a file or folder picker for an import. None when cancelled.
#[tauri::command]
pub async fn import_choose(app: AppHandle, folder: bool) -> AppResult<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let d = app.dialog().file().set_title(if folder { "Choose the folder to import" } else { "Choose the file to import" });
    if folder {
        d.pick_folder(move |p| {
            let _ = tx.send(p);
        });
    } else {
        d.add_filter("Import files", &["zip", "json", "csv", "tsv", "txt", "m3u", "m3u8"]).pick_file(move |p| {
            let _ = tx.send(p);
        });
    }
    let Some(picked) = rx.await.ok().flatten() else { return Ok(None) };
    let path = picked.into_path().map_err(|e| AppError::Io(format!("pick: {e}")))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Imports what's at `path` for a source the user turned on, and returns
/// that source's report.
#[tauri::command]
pub async fn import_run(db: State<'_, DbHandle>, settings: State<'_, SettingsState>, source: String, path: String) -> AppResult<SourceReport> {
    let src = sources::source(&source).ok_or_else(|| AppError::Invalid(format!("unknown import source {source}")))?;
    if !settings.get().await?.import_sources.iter().any(|s| s == &source) {
        return Err(AppError::Invalid("Turn this source on in Settings first.".into()));
    }
    let db = db.inner().clone();
    tokio::task::spawn_blocking(move || run(&db, src, &PathBuf::from(path), now_ms()))
        .await
        .map_err(|e| AppError::Internal(format!("import task: {e}")))?
}

/// Reads, matches and stores one import (blocking).
pub fn run(db: &DbHandle, src: &sources::Source, path: &std::path::Path, now: i64) -> AppResult<SourceReport> {
    let input = files::read_input(path)?;
    if input.is_empty() {
        return Err(AppError::Invalid("Nothing Platter can import was found there.".into()));
    }
    let parsed = src.parser.parse(&input)?;
    if parsed.is_empty() {
        return Err(AppError::Invalid("Those files had no songs in them.".into()));
    }
    let ranks = db.source_ranks();
    let id = src.id;
    let prepared = db.read(|c| store::prepare(c, id, parsed, ranks))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    db.write(|c| store::write(c, id, name.as_deref(), prepared, id == "streaming", now))?;
    db.read(store::reports)?
        .into_iter()
        .find(|r| r.source == id)
        .ok_or_else(|| AppError::Internal("import report missing".into()))
}

#[tauri::command]
pub async fn import_review_queue(db: State<'_, DbHandle>) -> AppResult<Vec<ReviewItem>> {
    db.read_async(store::review_queue).await
}

#[tauri::command]
pub async fn import_review(db: State<'_, DbHandle>, song_id: i64, decision: Decision) -> AppResult<()> {
    db.write_async(move |c| store::review(c, song_id, &decision, now_ms())).await
}

#[tauri::command]
pub async fn import_list_page(db: State<'_, DbHandle>, id: String) -> AppResult<ImportedListPage> {
    db.read_async(move |c| store::list_page(c, &id)).await
}

#[tauri::command]
pub async fn import_missing(db: State<'_, DbHandle>) -> AppResult<Vec<MissingSong>> {
    db.read_async(|c| store::missing(c, store::MISSING_MAX)).await
}

/// Opens a store search page from the Missing list.
#[tauri::command]
pub async fn import_open_link(app: AppHandle, url: String) -> AppResult<()> {
    if !store::link_allowed(&url) {
        return Err(AppError::Invalid("link not allowed".into()));
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| AppError::Internal(format!("open browser: {e}")))
}

/// Forgets everything imported from a source.
#[tauri::command]
pub async fn import_forget(db: State<'_, DbHandle>, source: String) -> AppResult<()> {
    db.write_async(move |c| store::forget(c, &source)).await
}
