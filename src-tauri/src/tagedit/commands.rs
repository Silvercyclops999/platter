//! Tauri commands for the tag editor, the tag check and the fixes
//! (docs/IPC.md "Tag editor").

use std::path::Path;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use super::check::Report;
use super::editor::{CheckScope, EditorData, SaveRequest, SaveResult, TagEditor, UndoInfo};
use super::fixes::{FixKind, Preview, Releases};
use super::lookup::Candidate;
use super::paths::{self, FolderCheck};
use super::plexside::{SectionCheck, library_check};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::plex::PlexState;
use crate::settings::SettingsState;

/// Shared so blocking work can hold it.
pub type Editor = Arc<TagEditor>;

/// Files the music folder check looks for.
const SAMPLES: usize = 5;

fn editor(e: &State<'_, Editor>) -> Editor {
    let e = Arc::clone(e);
    e.set_options(super::published());
    e
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::Internal(e.to_string()))?
}

fn changed(app: &AppHandle, r: &SaveResult) {
    if r.saved > 0 {
        let _ = app.emit(crate::tags::commands::EV_CHANGED, ());
    }
}

/// Random tracks with a known file, for the music folder check.
fn samples(db: &DbHandle) -> AppResult<Vec<(String, String)>> {
    db.read(|c| {
        let mut st = c.prepare(&format!(
            "SELECT id, file FROM tracks WHERE file IS NOT NULL AND file != '' ORDER BY random() LIMIT {SAMPLES}"
        ))?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

/// Checks a folder as the music folder: finds where Plex's paths are in
/// it, then looks for 5 random songs with the size Plex has.
async fn check_folder(e: &TagEditor, local: &str) -> AppResult<FolderCheck> {
    let picks = samples(e.db())?;
    if picks.is_empty() {
        return Ok(paths::verify(None, &[], |_| None)).map(|mut c| {
            c.message = "Plex hasn't sent any songs yet. Check again after the library syncs.".into();
            c
        });
    }
    let ids: Vec<String> = picks.iter().map(|p| p.0.clone()).collect();
    let sizes = e.plex().part_sizes(&ids).await.unwrap_or_else(|err| {
        tracing::debug!("part sizes: {err}");
        ids.iter().map(|i| (i.clone(), None)).collect()
    });
    let files: Vec<(String, Option<u64>)> =
        picks.iter().map(|(id, f)| (f.clone(), sizes.iter().find(|s| &s.0 == id).and_then(|s| s.1))).collect();
    let local = local.to_string();
    blocking(move || {
        if !Path::new(&local).is_dir() {
            return Ok(FolderCheck {
                ok: false,
                found: 0,
                checked: files.len(),
                message: "Platter can't open this folder. If it's on a network share, check that the share is connected.".into(),
                mapping: None,
                probes: Vec::new(),
            });
        }
        let servers: Vec<String> = files.iter().map(|f| f.0.clone()).collect();
        let m = paths::detect(&local, &servers, |p| Path::new(p).exists());
        Ok(paths::verify(m.as_ref(), &files, |p| std::fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| m.len())))
    })
    .await
}

/// Sets the music folder when it holds the library's files.
async fn set_folder(e: &TagEditor, settings: &SettingsState, local: String) -> AppResult<FolderCheck> {
    let check = check_folder(e, &local).await?;
    if let Some(m) = check.mapping.clone().filter(|_| check.found > 0) {
        settings
            .modify_and_publish(move |s| {
                s.music_folder = m.local;
                s.music_folder_server = m.server;
                Ok(true)
            })
            .await?;
    }
    Ok(check)
}

/// Settings, Library, Music folder: Choose.
#[tauri::command]
pub async fn tagedit_pick_music_folder(
    app: AppHandle,
    e: State<'_, Editor>,
    settings: State<'_, SettingsState>,
) -> AppResult<Option<FolderCheck>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::platform::pick_folder(app.dialog().file().set_title("Choose the folder that holds your music"), move |p| {
        let _ = tx.send(p);
    });
    let Some(picked) = rx.await.ok().flatten() else { return Ok(None) };
    let dir = picked.into_path().map_err(|e| AppError::Io(format!("folder: {e}")))?;
    let e = editor(&e);
    set_folder(&e, &settings, dir.to_string_lossy().into_owned()).await.map(Some)
}

/// A typed path (a UNC path the folder picker can't browse to).
#[tauri::command]
pub async fn tagedit_set_music_folder(e: State<'_, Editor>, settings: State<'_, SettingsState>, path: String) -> AppResult<FolderCheck> {
    let path = path.trim().to_string();
    if path.is_empty() || path.chars().count() > 1024 || path.chars().any(char::is_control) {
        return Err(AppError::Invalid("Enter the folder's full path.".into()));
    }
    let e = editor(&e);
    set_folder(&e, &settings, path).await
}

#[tauri::command]
pub async fn tagedit_clear_music_folder(settings: State<'_, SettingsState>) -> AppResult<()> {
    settings
        .modify_and_publish(|s| {
            s.music_folder.clear();
            s.music_folder_server.clear();
            Ok(true)
        })
        .await
        .map(|_| ())
}

/// The music folder's state now ("Connected, 5 of 5 files found").
#[tauri::command]
pub async fn tagedit_music_folder_status(e: State<'_, Editor>) -> AppResult<Option<FolderCheck>> {
    let e = editor(&e);
    let Some(m) = e.options().mapping else { return Ok(None) };
    let mut c = check_folder(&e, &m.local).await?;
    // Checked against the mapping in use, not one found anew.
    if c.mapping.as_ref().is_some_and(|x| x.server != m.server) {
        c.message = "The music folder now matches Plex's paths differently. Choose it again to update it.".into();
        c.ok = false;
    }
    Ok(Some(c))
}

#[tauri::command]
pub async fn tagedit_open(e: State<'_, Editor>, album_id: Option<String>, track_ids: Vec<String>) -> AppResult<EditorData> {
    let e = editor(&e);
    blocking(move || e.open(album_id.as_deref(), &track_ids)).await
}

#[tauri::command]
pub async fn tagedit_save(app: AppHandle, e: State<'_, Editor>, request: SaveRequest) -> AppResult<SaveResult> {
    let e = editor(&e);
    let r = e.save(request).await?;
    changed(&app, &r);
    Ok(r)
}

#[tauri::command]
pub async fn tagedit_undo(app: AppHandle, e: State<'_, Editor>, id: String) -> AppResult<SaveResult> {
    let e = editor(&e);
    let r = e.undo(&id).await?;
    changed(&app, &r);
    Ok(r)
}

#[tauri::command]
pub async fn tagedit_history(e: State<'_, Editor>) -> AppResult<Vec<UndoInfo>> {
    Ok(e.history())
}

#[tauri::command]
pub async fn tagedit_check(e: State<'_, Editor>, scope: CheckScope) -> AppResult<Report> {
    let e = editor(&e);
    blocking(move || e.check(&scope)).await
}

/// "This is fine" on a finding, or undo it.
#[tauri::command]
pub async fn tagedit_check_fine(e: State<'_, Editor>, id: String, fine: bool) -> AppResult<()> {
    let e = editor(&e);
    e.db().write(|c| super::editor::set_ignored(c, &id, fine))
}

#[tauri::command]
pub async fn tagedit_fix_preview(e: State<'_, Editor>, kind: FixKind, album_id: String) -> AppResult<Preview> {
    let e = editor(&e);
    blocking(move || e.fix_preview(kind, &album_id)).await
}

#[tauri::command]
pub async fn tagedit_isrc_releases(e: State<'_, Editor>, album_id: String) -> AppResult<Releases> {
    editor(&e).isrc_releases(&album_id).await
}

#[tauri::command]
pub async fn tagedit_isrc_preview(e: State<'_, Editor>, album_id: String, release: Candidate) -> AppResult<Preview> {
    editor(&e).isrc_preview(&album_id, &release).await
}

/// Mark explicit or clean: some albums, or an artist's era.
#[tauri::command]
pub async fn tagedit_explicit_preview(
    e: State<'_, Editor>,
    album_ids: Vec<String>,
    artist_id: Option<String>,
    era: Option<String>,
    explicit: bool,
) -> AppResult<Preview> {
    let e = editor(&e);
    blocking(move || {
        let ids = match (artist_id, era) {
            (Some(a), Some(era)) => e.era_albums(&a, &era)?,
            _ => album_ids,
        };
        if ids.is_empty() {
            return Err(AppError::NotFound("No albums to mark.".into()));
        }
        e.explicit_preview(&ids, explicit)
    })
    .await
}

/// The music library's settings in Plex (read only).
#[tauri::command]
pub async fn tagedit_library_settings(plex: State<'_, PlexState>) -> AppResult<Vec<SectionCheck>> {
    let client = plex.client().await?;
    library_check(&client).await
}
