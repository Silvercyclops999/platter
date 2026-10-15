//! Tauri commands for the library doctor (docs/IPC.md "Library doctor").
//! Read-only toward Plex and files; Ignore is app state in the sync store.

use std::collections::HashMap;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::{EV_CHANGED, Finding, Ignored, Kind, options, store};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::sync::SyncStore;
use crate::sync::store::DOCTOR_IGNORED;

/// What the Library doctor page shows (IPC `DoctorReport`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Run library doctor is on.
    pub on: bool,
    /// Findings to fix, for the checks that are on.
    pub findings: Vec<Finding>,
    /// Findings the user ignored that still show up.
    pub ignored: Vec<Finding>,
    /// `findings.len()`, the badge.
    pub count: usize,
    /// A check is going now.
    pub running: bool,
    /// When the last check finished (unix seconds).
    pub checked_at: Option<i64>,
    /// Files whose album tags are read, of all files (some checks wait for them).
    pub tags_read: u64,
    pub tags_total: u64,
}

fn sync_store(app: &AppHandle) -> Option<SyncStore> {
    app.try_state::<crate::sync::commands::SyncService>().map(|s| s.store.clone())
}

/// Ignored findings by id (none when the sync store isn't set up).
pub async fn ignored(app: &AppHandle) -> HashMap<String, Ignored> {
    let Some(s) = sync_store(app) else { return HashMap::new() };
    s.run(|s| s.list(&DOCTOR_IGNORED)).await.map(|v| v.into_iter().collect()).unwrap_or_default()
}

/// Findings to fix and ignored findings.
pub fn split(all: Vec<Finding>, ignored: &HashMap<String, Ignored>) -> (Vec<Finding>, Vec<Finding>) {
    all.into_iter().partition(|f| !ignored.contains_key(&f.id))
}

/// The report, from the cache and the ignore list.
pub fn report(c: &rusqlite::Connection, ignored: &HashMap<String, Ignored>) -> AppResult<Report> {
    let o = options();
    let all = if o.on { store::all(c, &o)? } else { Vec::new() };
    let (findings, ignored) = split(all, ignored);
    let tags = crate::tags::store::counts(c)?;
    Ok(Report {
        on: o.on,
        count: findings.len(),
        findings,
        ignored,
        running: super::running(),
        checked_at: super::checked_at(),
        tags_read: tags.done,
        tags_total: tags.total,
    })
}

/// How many findings to fix (the badge); 0 while the doctor is off.
pub async fn count(app: &AppHandle, db: &DbHandle) -> AppResult<usize> {
    let ig = ignored(app).await;
    db.read_async(move |c| report(c, &ig).map(|r| r.count)).await
}

/// Tell the UI the findings or the count changed.
pub async fn emit_changed(app: &AppHandle) {
    let Some(db) = app.try_state::<DbHandle>().map(|d| d.inner().clone()) else { return };
    let n = count(app, &db).await.unwrap_or(0);
    let _ = app.emit(EV_CHANGED, serde_json::json!({ "count": n, "running": super::running() }));
}

/// Every finding, with what's ignored.
#[tauri::command]
pub async fn doctor_report(app: AppHandle, db: State<'_, DbHandle>) -> AppResult<Report> {
    let ig = ignored(&app).await;
    db.read_async(move |c| report(c, &ig)).await
}

/// The badge count.
#[tauri::command]
pub async fn doctor_count(app: AppHandle, db: State<'_, DbHandle>) -> AppResult<usize> {
    count(&app, &db).await
}

/// Ignore a finding (it follows the user through the sync file), or bring
/// it back with `ignore` false. `doctor://changed` fires.
#[tauri::command]
pub async fn doctor_ignore(app: AppHandle, id: String, kind: Kind, title: String, ignore: bool) -> AppResult<()> {
    if id.trim().is_empty() || id.len() > 2000 {
        return Err(AppError::Invalid("No finding to ignore.".into()));
    }
    let s = sync_store(&app).ok_or_else(|| AppError::Internal("the sync store isn't ready".into()))?;
    let title: String = title.chars().take(200).collect();
    s.run(move |s| {
        if ignore {
            s.put(&DOCTOR_IGNORED, &id, &Ignored { kind, title, at: super::now_s() })
        } else {
            s.remove(&DOCTOR_IGNORED, &id).map(|_| ())
        }
    })
    .await?;
    emit_changed(&app).await;
    Ok(())
}

/// Check every artist again now (Check now).
#[tauri::command]
pub async fn doctor_run(db: State<'_, DbHandle>) -> AppResult<()> {
    db.write_async(store::forget).await?;
    super::poke();
    Ok(())
}
