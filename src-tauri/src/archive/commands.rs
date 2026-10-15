//! Tauri commands for the archive (docs/IPC.md "Archive"). Read-only toward
//! Plex and files: marks and sections are app state.

use rusqlite::OptionalExtension;
use tauri::{AppHandle, Emitter, Manager, State};

use super::eras::EraPage;
use super::versions::VersionItem;
use super::{in_tx, marks};
use crate::db::DbHandle;
use crate::db::release::Mark;
use crate::error::{AppError, AppResult};
use crate::sync::SyncStore;

fn store(app: &AppHandle) -> AppResult<SyncStore> {
    app.try_state::<crate::sync::commands::SyncService>()
        .map(|s| s.store.clone())
        .ok_or_else(|| AppError::Internal("the sync store isn't ready".into()))
}

/// The album artist and album of a marked item.
fn owner_of(c: &rusqlite::Connection, kind: &str, id: &str) -> AppResult<Option<(String, String)>> {
    let sql = if kind == "track" {
        "SELECT artist_id, album_id FROM tracks WHERE id = ?1"
    } else {
        "SELECT artist_id, id FROM albums WHERE id = ?1"
    };
    Ok(c.query_row(sql, [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
}

/// Mark an album or a song ("album" or "track") as unreleased or released,
/// or clear the mark (null). `releasestatus://changed` and
/// `artist://changed` fire once statuses are worked out again.
#[tauri::command]
pub async fn release_mark_set(app: AppHandle, db: State<'_, DbHandle>, kind: String, id: String, mark: Option<Mark>) -> AppResult<()> {
    let s = store(&app)?;
    {
        let (kind, id) = (kind.clone(), id.clone());
        s.run(move |s| marks::set(s, &kind, &id, mark)).await?;
    }
    let owner = {
        let (kind, id) = (kind.clone(), id.clone());
        db.write_async(move |c| {
            let owner = owner_of(c, &kind, &id)?;
            if let Some((artist, _)) = &owner {
                let ids = vec![artist.clone()];
                in_tx(c, |c| {
                    crate::identity::index::refresh_artists(c, &ids)?;
                    super::refresh_artists(c, &ids)
                })?;
            }
            Ok(owner)
        })
        .await?
    };
    if let Some((artist, album)) = owner {
        let _ = app.emit("releasestatus://changed", serde_json::json!({ "id": album }));
        let _ = app.emit("artist://changed", serde_json::json!({ "id": artist }));
    }
    Ok(())
}

/// A song and its other versions for the Other versions panel, released
/// ones first.
#[tauri::command]
pub async fn track_versions(db: State<'_, DbHandle>, id: String) -> AppResult<Vec<VersionItem>> {
    let show = db.tag_view().show_edits;
    db.read_async(move |c| super::versions::versions_of(c, &id, show)).await
}

/// Every version of a song on one timeline, oldest first.
#[tauri::command]
pub async fn track_lineage(db: State<'_, DbHandle>, id: String) -> AppResult<super::lineage::Lineage> {
    let show = db.tag_view().show_edits;
    db.read_async(move |c| super::lineage::lineage_of(c, &id, show)).await
}

/// An artist's era: its songs by album or build, newest first.
#[tauri::command]
pub async fn era_page(db: State<'_, DbHandle>, artist_id: String, name: String) -> AppResult<EraPage> {
    let show = db.tag_view().show_edits;
    db.read_async(move |c| super::eras::era_page(c, &artist_id, &name, show)).await
}

/// Move an album to a section of the user's own on its artist's page, or
/// back to its own section (null). `artist://changed` fires.
#[tauri::command]
pub async fn album_section_set(app: AppHandle, album_id: String, artist_id: String, name: Option<String>) -> AppResult<()> {
    let s = store(&app)?;
    let artist = artist_id.clone();
    s.run(move |s| marks::set_section(s, &album_id, &artist_id, name.as_deref())).await?;
    let _ = app.emit("artist://changed", serde_json::json!({ "id": artist }));
    Ok(())
}

/// An artist's own section names, A to Z (for "Move to section").
#[tauri::command]
pub async fn artist_sections(app: AppHandle, artist_id: String) -> AppResult<Vec<String>> {
    let s = store(&app)?;
    s.run(move |s| marks::section_names(s, &artist_id)).await
}
