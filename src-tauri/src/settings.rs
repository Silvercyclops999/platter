//! App settings (docs/IPC.md `settings_get` / `settings_set`).
//!
//! Stored as one JSON value in the db `kv` table. Unknown or missing fields
//! fall back to defaults, so older and newer builds can share a database.
//! The tracking flags are mirrored into [`TrackingSettings`] so the tracker
//! sees changes immediately.

use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::audio::player::PlayerHandle;
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::tracking::TrackingSettings;

const KEY: &str = "app.settings";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Skip the local listen log (Plex still gets reports unless
    /// `report_to_plex` is off).
    pub private_session: bool,
    /// Send timeline and scrobble reports to Plex.
    pub report_to_plex: bool,
    /// Linear slider value, 0..1.
    pub volume: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { private_session: false, report_to_plex: true, volume: 0.7 }
    }
}

/// A partial update; absent fields are left alone.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub private_session: Option<bool>,
    pub report_to_plex: Option<bool>,
    pub volume: Option<f32>,
}

impl Settings {
    pub fn apply(&mut self, p: &SettingsPatch) -> AppResult<()> {
        if let Some(v) = p.volume {
            if !v.is_finite() {
                return Err(AppError::Invalid("volume must be a number".into()));
            }
            self.volume = v.clamp(0.0, 1.0);
        }
        if let Some(v) = p.private_session {
            self.private_session = v;
        }
        if let Some(v) = p.report_to_plex {
            self.report_to_plex = v;
        }
        Ok(())
    }
}

/// Read settings; a missing or unreadable value gives the defaults.
pub fn load(db: &DbHandle) -> Settings {
    db.read(|c| kv_get(c, KEY))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(db: &DbHandle, s: &Settings) -> AppResult<()> {
    let json = serde_json::to_string(s).map_err(|e| AppError::Internal(e.to_string()))?;
    db.write(|c| kv_set(c, KEY, &json))
}

/// Copy the tracking flags into the shared atomics.
pub fn publish(s: &Settings, tracking: &TrackingSettings) {
    tracking.private_session.store(s.private_session, Ordering::Relaxed);
    tracking.report_to_plex.store(s.report_to_plex, Ordering::Relaxed);
}

/// Managed state for the settings commands.
#[derive(Clone)]
pub struct SettingsState {
    db: DbHandle,
    tracking: TrackingSettings,
    lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl SettingsState {
    /// Load stored settings and publish them to the tracker.
    pub fn new(db: DbHandle, tracking: TrackingSettings) -> Self {
        publish(&load(&db), &tracking);
        Self { db, tracking, lock: Default::default() }
    }

    pub async fn get(&self) -> AppResult<Settings> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || load(&db))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))
    }

    /// Apply a patch, persist it and publish the tracking flags.
    pub async fn set(&self, patch: SettingsPatch) -> AppResult<Settings> {
        let _guard = self.lock.lock().await;
        let mut s = self.get().await?;
        s.apply(&patch)?;
        let (db, saved) = (self.db.clone(), s.clone());
        tokio::task::spawn_blocking(move || save(&db, &saved))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))??;
        publish(&s, &self.tracking);
        Ok(s)
    }
}

#[tauri::command]
pub async fn settings_get(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
) -> AppResult<Settings> {
    let mut s = settings.get().await?;
    // The player owns the live volume.
    if let Ok(p) = player.state().await {
        s.volume = p.volume;
    }
    Ok(s)
}

#[tauri::command]
pub async fn settings_set(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
    patch: SettingsPatch,
) -> AppResult<()> {
    let s = settings.set(patch.clone()).await?;
    if patch.volume.is_some() {
        player.set_volume(s.volume).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (DbHandle, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (DbHandle::open(dir.path().join("s.db")).unwrap(), dir)
    }

    #[test]
    fn defaults_when_nothing_stored() {
        let (db, _d) = db();
        let s = load(&db);
        assert_eq!(s, Settings::default());
        assert!(s.report_to_plex);
        assert!(!s.private_session);
    }

    #[tokio::test]
    async fn set_persists_across_reopen_and_publishes_flags() {
        let (db, dir) = db();
        let tracking = TrackingSettings::default();
        let st = SettingsState::new(db.clone(), tracking.clone());
        st.set(SettingsPatch { private_session: Some(true), report_to_plex: Some(false), volume: Some(0.25) })
            .await
            .unwrap();
        assert!(tracking.private_session.load(Ordering::Relaxed));
        assert!(!tracking.report_to_plex.load(Ordering::Relaxed));

        // Partial patch leaves other fields alone.
        st.set(SettingsPatch { report_to_plex: Some(true), ..Default::default() }).await.unwrap();
        drop(st);
        drop(db);

        let db2 = DbHandle::open(dir.path().join("s.db")).unwrap();
        let fresh = TrackingSettings::default();
        let st2 = SettingsState::new(db2, fresh.clone());
        let s = st2.get().await.unwrap();
        assert!(s.private_session);
        assert!(s.report_to_plex);
        assert_eq!(s.volume, 0.25);
        // Loaded at startup into the tracker's switches.
        assert!(fresh.private_session.load(Ordering::Relaxed));
        assert!(fresh.report_to_plex.load(Ordering::Relaxed));
    }

    #[test]
    fn volume_is_clamped_and_must_be_finite() {
        let mut s = Settings::default();
        s.apply(&SettingsPatch { volume: Some(3.0), ..Default::default() }).unwrap();
        assert_eq!(s.volume, 1.0);
        s.apply(&SettingsPatch { volume: Some(-1.0), ..Default::default() }).unwrap();
        assert_eq!(s.volume, 0.0);
        assert!(s.apply(&SettingsPatch { volume: Some(f32::NAN), ..Default::default() }).is_err());
    }

    #[test]
    fn unknown_and_missing_fields_fall_back_to_defaults() {
        let (db, _d) = db();
        db.write(|c| kv_set(c, KEY, r#"{"privateSession":true,"somethingNew":1}"#)).unwrap();
        let s = load(&db);
        assert!(s.private_session);
        assert!(s.report_to_plex);
        db.write(|c| kv_set(c, KEY, "not json")).unwrap();
        assert_eq!(load(&db), Settings::default());
    }

    #[test]
    fn wire_format_is_camel_case() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(v["privateSession"], false);
        assert_eq!(v["reportToPlex"], true);
        let p: SettingsPatch = serde_json::from_str(r#"{"privateSession":true}"#).unwrap();
        assert_eq!(p.private_session, Some(true));
        assert_eq!(p.report_to_plex, None);
    }
}
