//! Library import commands. Nothing here writes to Plex: saving an imported
//! playlist or liking its songs there uses the usual playlist and like
//! commands, from an explicit click.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use super::store::{self, Decision, ImportedList, ImportedListPage, MissingSong, ReviewItem, SourceReport};
use super::{Skipped, files, played, sources};
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
    // History imported before plays were folded counts its songs once folded.
    let unfolded = db
        .read_async(|c| Ok(c.query_row("SELECT EXISTS (SELECT 1 FROM imported_plays WHERE title_fold IS NULL)", [], |r| r.get::<_, bool>(0))?))
        .await?;
    if unfolded {
        db.write_async(played::fill_folds).await?;
    }
    db.read_async(|c| {
        let (review_count, songs_missing) = store::queue_counts(c)?;
        let missing_count = songs_missing + store::wanted_albums(c)?.len() as u64;
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

/// What to say when files had nothing to import, with what was left out.
pub fn nothing_usable(s: &Skipped) -> String {
    let plural = |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = Vec::new();
    if s.podcasts > 0 {
        parts.push(plural(s.podcasts, "podcast or audiobook record", "podcast or audiobook records"));
    }
    if s.short_plays > 0 {
        parts.push(plural(s.short_plays, "play under 30 seconds", "plays under 30 seconds"));
    }
    if s.private_plays > 0 {
        parts.push(plural(s.private_plays, "private play", "private plays"));
    }
    if s.no_metadata > 0 {
        parts.push(plural(s.no_metadata, "record with no song name", "records with no song name"));
    }
    if s.incomplete_rows > 0 {
        parts.push(plural(s.incomplete_rows, "row without an artist or title", "rows without an artist or title"));
    }
    if parts.is_empty() {
        "Those files had no songs, playlists or plays in them.".into()
    } else {
        format!("Those files had no songs Platter could use. Left out: {}.", parts.join(", "))
    }
}

/// Reads, matches and stores one import (blocking).
pub fn run(db: &DbHandle, src: &sources::Source, path: &std::path::Path, now: i64) -> AppResult<SourceReport> {
    let input = files::read_input(path)?;
    if input.is_empty() {
        return Err(AppError::Invalid("Nothing Platter can import was found there.".into()));
    }
    let parsed = src.parser.parse(&input)?;
    if parsed.is_empty() {
        return Err(AppError::Invalid(nothing_usable(&parsed.skipped)));
    }
    let ranks = db.source_ranks();
    let id = src.id;
    let prepared = db.read(|c| store::prepare(c, id, parsed, ranks))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    db.write(|c| store::write(c, id, name.as_deref(), prepared, id == "streaming", now))?;
    let ranks = db.source_ranks();
    db.write(|c| {
        played::keep_up(c, ranks, now);
        Ok(())
    })?;
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

/// Albums added to the Missing list by hand, newest first.
#[tauri::command]
pub async fn import_missing_albums(db: State<'_, DbHandle>) -> AppResult<Vec<store::MissingAlbum>> {
    db.read_async(store::missing_albums).await
}

/// Remember an album that isn't in the library on the Missing list.
#[tauri::command]
pub async fn import_missing_album_add(db: State<'_, DbHandle>, album: store::WantedAlbum) -> AppResult<Vec<store::WantedAlbum>> {
    let now = now_ms() / 1000;
    db.write_async(move |c| store::add_wanted(c, album, now)).await
}

#[tauri::command]
pub async fn import_missing_album_remove(db: State<'_, DbHandle>, release_group_id: String) -> AppResult<Vec<store::WantedAlbum>> {
    db.write_async(move |c| store::remove_wanted(c, &release_group_id)).await
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

/// Takes a song out of one imported list.
#[tauri::command]
pub async fn import_list_remove(db: State<'_, DbHandle>, list_id: String, song_id: i64) -> AppResult<()> {
    db.write_async(move |c| store::remove_from_list(c, &list_id, song_id)).await
}

// ------------------------------------------------------------------ played a lot

fn sync_store(app: &AppHandle) -> Option<crate::sync::SyncStore> {
    use tauri::Manager;
    app.try_state::<crate::sync::commands::SyncService>().map(|s| s.store.clone())
}

/// The hidden rows of "Played a lot" (synced), by key.
async fn hidden(app: &AppHandle) -> Vec<(String, played::HiddenRow)> {
    let Some(s) = sync_store(app) else { return Vec::new() };
    s.run(|s| s.list(&crate::sync::store::PLAYED_HIDDEN)).await.unwrap_or_default()
}

/// A page of "Played a lot, not in your library": imported history plays
/// with no library match, by song or by album, most played first.
#[tauri::command]
pub async fn import_played(app: AppHandle, db: State<'_, DbHandle>, query: played::Query) -> AppResult<played::PlayedPage> {
    let ranks = db.source_ranks();
    let now = now_ms();
    db.write_async(move |c| {
        played::keep_up(c, ranks, now);
        Ok(())
    })
    .await?;
    let keys: Vec<String> = hidden(&app).await.into_iter().map(|(k, _)| k).collect();
    db.read_async(move |c| played::list(c, &query, &keys, now)).await
}

/// Hide a song, an album or a whole artist from "Played a lot" (synced),
/// or bring it back with `hide` false. Returns what's hidden.
#[tauri::command]
pub async fn import_played_hide(app: AppHandle, kind: String, artist: String, name: Option<String>, hide: bool) -> AppResult<Vec<played::HiddenRow>> {
    let name = name.unwrap_or_default();
    let key = played::hide_key_for(&kind, &artist, &name)?;
    let s = sync_store(&app).ok_or_else(|| AppError::Internal("the sync store isn't ready".into()))?;
    let row = played::HiddenRow {
        kind: kind.clone(),
        artist: artist.chars().take(200).collect(),
        name: if kind == "artist" { String::new() } else { name.chars().take(200).collect() },
        hidden_at: now_ms() / 1000,
    };
    s.run(move |s| {
        if hide {
            s.put(&crate::sync::store::PLAYED_HIDDEN, &key, &row)
        } else {
            s.remove(&crate::sync::store::PLAYED_HIDDEN, &key).map(|_| ())
        }
    })
    .await?;
    import_played_hidden(app).await
}

/// What's hidden from "Played a lot", newest first.
#[tauri::command]
pub async fn import_played_hidden(app: AppHandle) -> AppResult<Vec<played::HiddenRow>> {
    let mut all: Vec<played::HiddenRow> = hidden(&app).await.into_iter().map(|(_, h)| h).collect();
    all.sort_by_key(|h| std::cmp::Reverse(h.hidden_at));
    Ok(all)
}
