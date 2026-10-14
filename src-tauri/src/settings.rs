//! App settings (docs/IPC.md `settings_get` / `settings_set`), plus the few
//! commands the Settings page needs (art cache size and clear, app info).
//!
//! Stored as one JSON value in the db `kv` table. Unknown or missing fields
//! fall back to defaults, so older and newer builds can share a database.
//! Values are pushed to the parts of the core that use them ([`Live`]) at
//! startup and on every change; player settings go through the player.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::audio::player::PlayerHandle;
use crate::audio::queue::ShuffleStyle;
use crate::bridge::StreamPrefs;
use crate::db::store::{kv_get, kv_set};
use crate::db::{ArtCache, DbHandle, PopularSource};
use crate::error::{AppError, AppResult};
use crate::tracking::TrackingSettings;

const KEY: &str = "app.settings";

/// The project page opened from About.
pub const REPO_URL: &str = "https://github.com/Silvercyclops999/platter";
pub const LICENCE: &str = "GPL-3.0-or-later";

pub const PRECACHE_MIN: u32 = 1;
pub const PRECACHE_MAX: u32 = 50;
pub const ART_CACHE_MIN_MB: u32 = 128;
pub const ART_CACHE_MAX_MB: u32 = 51_200;
const MB: u64 = 1024 * 1024;

/// Streaming quality: the original file, or a transcode cap in kbps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StreamQuality {
    #[default]
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "320")]
    Kbps320,
    #[serde(rename = "256")]
    Kbps256,
    #[serde(rename = "192")]
    Kbps192,
    #[serde(rename = "128")]
    Kbps128,
}

impl StreamQuality {
    /// The cap in kbps, or None for the original file.
    pub fn kbps(self) -> Option<u32> {
        match self {
            Self::Original => None,
            Self::Kbps320 => Some(320),
            Self::Kbps256 => Some(256),
            Self::Kbps192 => Some(192),
            Self::Kbps128 => Some(128),
        }
    }
}

/// Reduce motion: follow the OS, or force it on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReduceMotion {
    #[default]
    System,
    On,
    Off,
}

/// Missing fields (settings saved by an older build) take their defaults.
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
    /// How shuffle orders tracks (spread keeps an artist's songs apart).
    pub shuffle_style: ShuffleStyle,
    /// Streaming quality on the local connection.
    pub quality_home: StreamQuality,
    /// Streaming quality over remote access or the relay.
    pub quality_away: StreamQuality,
    /// Show the format badge (codec, bit depth, sample rate).
    pub show_format_badge: bool,
    /// Albums and playlists resume where they were left.
    pub resume_contexts: bool,
    /// Upcoming tracks kept on disk, 1 to 50.
    pub precache_count: u32,
    /// What ranks the artist page's Popular list.
    pub popular_source: PopularSource,
    /// Artwork cache size in MB.
    pub art_cache_mb: u32,
    /// Album and artist headers tint from artwork.
    pub artwork_colours: bool,
    pub reduce_motion: ReduceMotion,
    /// Chosen output device id; None follows the system default. Changed
    /// with `output_select`, not `settings_set`.
    pub output_device: Option<String>,
    /// Home shows the grid of recent and most played shortcuts.
    pub home_shortcuts: bool,
    /// Home's top takes a colour wash from the shortcuts' artwork (only
    /// while Artwork colours is on).
    pub home_colour_wash: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            private_session: false,
            report_to_plex: true,
            volume: 0.7,
            shuffle_style: ShuffleStyle::Spread,
            quality_home: StreamQuality::Original,
            quality_away: StreamQuality::Kbps320,
            show_format_badge: true,
            resume_contexts: true,
            precache_count: 5,
            popular_source: PopularSource::Global,
            art_cache_mb: 2048,
            artwork_colours: true,
            reduce_motion: ReduceMotion::System,
            output_device: None,
            home_shortcuts: true,
            home_colour_wash: true,
        }
    }
}

/// A partial update; absent fields are left alone.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub private_session: Option<bool>,
    pub report_to_plex: Option<bool>,
    pub volume: Option<f32>,
    pub shuffle_style: Option<ShuffleStyle>,
    pub quality_home: Option<StreamQuality>,
    pub quality_away: Option<StreamQuality>,
    pub show_format_badge: Option<bool>,
    pub resume_contexts: Option<bool>,
    pub precache_count: Option<u32>,
    pub popular_source: Option<PopularSource>,
    pub art_cache_mb: Option<u32>,
    pub artwork_colours: Option<bool>,
    pub reduce_motion: Option<ReduceMotion>,
    pub home_shortcuts: Option<bool>,
    pub home_colour_wash: Option<bool>,
}

fn set<T: Copy>(dst: &mut T, v: Option<T>) {
    if let Some(v) = v {
        *dst = v;
    }
}

impl Settings {
    pub fn apply(&mut self, p: &SettingsPatch) -> AppResult<()> {
        if let Some(v) = p.volume {
            if !v.is_finite() {
                return Err(AppError::Invalid("volume must be a number".into()));
            }
            self.volume = v.clamp(0.0, 1.0);
        }
        set(&mut self.private_session, p.private_session);
        set(&mut self.report_to_plex, p.report_to_plex);
        set(&mut self.shuffle_style, p.shuffle_style);
        set(&mut self.quality_home, p.quality_home);
        set(&mut self.quality_away, p.quality_away);
        set(&mut self.show_format_badge, p.show_format_badge);
        set(&mut self.resume_contexts, p.resume_contexts);
        set(&mut self.precache_count, p.precache_count);
        set(&mut self.popular_source, p.popular_source);
        set(&mut self.art_cache_mb, p.art_cache_mb);
        set(&mut self.artwork_colours, p.artwork_colours);
        set(&mut self.reduce_motion, p.reduce_motion);
        set(&mut self.home_shortcuts, p.home_shortcuts);
        set(&mut self.home_colour_wash, p.home_colour_wash);
        self.clamp();
        Ok(())
    }

    /// Keep ranged values in range (also fixes hand-edited stored values).
    fn clamp(&mut self) {
        self.precache_count = self.precache_count.clamp(PRECACHE_MIN, PRECACHE_MAX);
        self.art_cache_mb = self.art_cache_mb.clamp(ART_CACHE_MIN_MB, ART_CACHE_MAX_MB);
    }

    pub fn art_cache_bytes(&self) -> u64 {
        self.art_cache_mb as u64 * MB
    }
}

/// Read settings; a missing or unreadable value gives the defaults.
pub fn load(db: &DbHandle) -> Settings {
    let mut s: Settings = db
        .read(|c| kv_get(c, KEY))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    s.clamp();
    s
}

pub fn save(db: &DbHandle, s: &Settings) -> AppResult<()> {
    let json = serde_json::to_string(s).map_err(|e| AppError::Internal(e.to_string()))?;
    db.write(|c| kv_set(c, KEY, &json))
}

/// The parts of the core that read settings directly (no player round trip).
#[derive(Clone, Default)]
pub struct Live {
    pub tracking: TrackingSettings,
    pub streams: Arc<StreamPrefs>,
    /// The art cache size the disk monitor returns to after trimming.
    pub art_cap: Arc<AtomicU64>,
    pub art: Option<ArtCache>,
}

/// Push settings to the tracker, stream requests, Popular and the art cache.
pub fn publish(s: &Settings, live: &Live, db: &DbHandle) {
    live.tracking.private_session.store(s.private_session, Ordering::Relaxed);
    live.tracking.report_to_plex.store(s.report_to_plex, Ordering::Relaxed);
    live.streams.set(s.quality_home.kbps(), s.quality_away.kbps());
    db.set_popular_source(s.popular_source);
    let cap = s.art_cache_bytes();
    let old = live.art_cap.swap(cap, Ordering::Relaxed);
    if let Some(art) = &live.art
        && (old != cap || art.cap() > cap)
        && let Err(e) = art.set_cap(cap)
    {
        tracing::warn!("art cache resize failed: {e}");
    }
}

/// Managed state for the settings commands.
#[derive(Clone)]
pub struct SettingsState {
    db: DbHandle,
    live: Live,
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl SettingsState {
    /// Load stored settings and publish them.
    pub fn new(db: DbHandle, live: Live) -> Self {
        publish(&load(&db), &live, &db);
        Self { db, live, lock: Default::default() }
    }

    pub async fn get(&self) -> AppResult<Settings> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || load(&db))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))
    }

    /// Apply a patch, persist it and publish it.
    pub async fn set(&self, patch: SettingsPatch) -> AppResult<Settings> {
        let _guard = self.lock.lock().await;
        let mut s = self.get().await?;
        s.apply(&patch)?;
        let (db, saved) = (self.db.clone(), s.clone());
        tokio::task::spawn_blocking(move || save(&db, &saved))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))??;
        let (live, db) = (self.live.clone(), self.db.clone());
        let published = s.clone();
        // Shrinking the art cache deletes files.
        tokio::task::spawn_blocking(move || publish(&published, &live, &db))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(s)
    }

    /// Store the chosen output device (None: the system default).
    pub async fn set_output_device(&self, id: Option<String>) -> AppResult<()> {
        let _guard = self.lock.lock().await;
        let mut s = self.get().await?;
        if s.output_device == id {
            return Ok(());
        }
        s.output_device = id;
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || save(&db, &s))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
    }
}

/// Push the player's settings (the ones the patch touched).
async fn apply_to_player(player: &PlayerHandle, s: &Settings, patch: &SettingsPatch) -> AppResult<()> {
    if patch.volume.is_some() {
        player.set_volume(s.volume).await?;
    }
    if patch.shuffle_style.is_some() {
        player.set_shuffle_style(s.shuffle_style).await?;
    }
    if patch.resume_contexts.is_some() {
        player.set_resume_contexts(s.resume_contexts).await?;
    }
    if patch.precache_count.is_some() {
        player.set_precache_count(s.precache_count as usize).await?;
    }
    Ok(())
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
    apply_to_player(&player, &s, &patch).await
}

/// Art cache usage for the Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtCacheInfo {
    pub used_bytes: u64,
    pub cap_bytes: u64,
}

/// Delete every cached image. Returns the bytes freed.
pub fn clear_art(art: &ArtCache) -> AppResult<u64> {
    let before = art.size();
    art.clear()?;
    Ok(before.saturating_sub(art.size()))
}

#[tauri::command]
pub async fn art_cache_info(art: State<'_, ArtCache>) -> AppResult<ArtCacheInfo> {
    Ok(ArtCacheInfo { used_bytes: art.size(), cap_bytes: art.cap() })
}

#[tauri::command]
pub async fn art_cache_clear(art: State<'_, ArtCache>) -> AppResult<u64> {
    let art = art.inner().clone();
    tokio::task::spawn_blocking(move || clear_art(&art))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
}

/// Version and licence for the App and About sections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub licence: String,
    pub repo_url: String,
}

#[tauri::command]
pub async fn app_info(app: AppHandle) -> AppResult<AppInfo> {
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        licence: LICENCE.into(),
        repo_url: REPO_URL.into(),
    })
}

/// Open the project page in the default browser.
#[tauri::command]
pub async fn open_project_page(app: AppHandle) -> AppResult<()> {
    app.opener()
        .open_url(REPO_URL, None::<&str>)
        .map_err(|e| AppError::Internal(format!("open browser: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::source::ArtBytes;
    use crate::db::ArtFetcher;

    fn db() -> (DbHandle, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (DbHandle::open(dir.path().join("s.db")).unwrap(), dir)
    }

    struct NoArt;
    #[async_trait::async_trait]
    impl ArtFetcher for NoArt {
        async fn fetch(&self, _: &str, _: u32, _: u32) -> AppResult<ArtBytes> {
            Err(AppError::NotFound("art".into()))
        }
    }

    fn art(dir: &std::path::Path) -> ArtCache {
        ArtCache::open(dir.join("art"), 2048 * MB, Arc::new(NoArt)).unwrap()
    }

    fn image(n: usize) -> ArtBytes {
        ArtBytes { bytes: vec![7u8; n].into(), content_type: "image/jpeg".into() }
    }

    #[test]
    fn defaults_when_nothing_stored() {
        let (db, _d) = db();
        let s = load(&db);
        assert_eq!(s, Settings::default());
        assert!(s.report_to_plex);
        assert!(!s.private_session);
        assert_eq!(s.quality_home, StreamQuality::Original);
        assert_eq!(s.quality_away, StreamQuality::Kbps320);
        assert!(s.show_format_badge && s.resume_contexts && s.artwork_colours);
        assert_eq!(s.precache_count, 5);
        assert_eq!(s.popular_source, PopularSource::Global);
        assert_eq!(s.art_cache_mb, 2048);
        assert_eq!(s.reduce_motion, ReduceMotion::System);
    }

    #[test]
    fn settings_saved_by_an_older_build_load_with_new_defaults() {
        let (db, _d) = db();
        db.write(|c| {
            kv_set(c, KEY, r#"{"privateSession":true,"reportToPlex":false,"volume":0.3,"shuffleStyle":"standard"}"#)
        })
        .unwrap();
        let s = load(&db);
        assert!(s.private_session);
        assert!(!s.report_to_plex);
        assert_eq!(s.shuffle_style, ShuffleStyle::Standard);
        let d = Settings::default();
        assert_eq!(
            (s.quality_home, s.quality_away, s.show_format_badge, s.resume_contexts),
            (d.quality_home, d.quality_away, d.show_format_badge, d.resume_contexts)
        );
        assert_eq!((s.precache_count, s.art_cache_mb), (5, 2048));
        assert_eq!((s.popular_source, s.artwork_colours, s.reduce_motion), (PopularSource::Global, true, ReduceMotion::System));
    }

    #[tokio::test]
    async fn set_persists_across_reopen_and_publishes() {
        let (db, dir) = db();
        let live = Live::default();
        let st = SettingsState::new(db.clone(), live.clone());
        st.set(SettingsPatch {
            private_session: Some(true),
            report_to_plex: Some(false),
            volume: Some(0.25),
            shuffle_style: Some(ShuffleStyle::Standard),
            quality_home: Some(StreamQuality::Kbps256),
            quality_away: Some(StreamQuality::Original),
            show_format_badge: Some(false),
            resume_contexts: Some(false),
            precache_count: Some(12),
            popular_source: Some(PopularSource::MyPlays),
            art_cache_mb: Some(512),
            artwork_colours: Some(false),
            reduce_motion: Some(ReduceMotion::On),
            home_shortcuts: Some(false),
            home_colour_wash: Some(false),
        })
        .await
        .unwrap();
        assert!(live.tracking.private_session.load(Ordering::Relaxed));
        assert!(!live.tracking.report_to_plex.load(Ordering::Relaxed));
        assert_eq!((live.streams.home(), live.streams.away()), (Some(256), None));
        assert_eq!(db.popular_source(), PopularSource::MyPlays);
        assert_eq!(live.art_cap.load(Ordering::Relaxed), 512 * MB);

        // Partial patch leaves other fields alone.
        st.set(SettingsPatch { report_to_plex: Some(true), ..Default::default() }).await.unwrap();
        drop(st);
        drop(db);

        let db2 = DbHandle::open(dir.path().join("s.db")).unwrap();
        let fresh = Live::default();
        let st2 = SettingsState::new(db2.clone(), fresh.clone());
        let s = st2.get().await.unwrap();
        assert!(s.private_session);
        assert!(s.report_to_plex);
        assert_eq!(s.volume, 0.25);
        assert_eq!(s.shuffle_style, ShuffleStyle::Standard);
        assert_eq!((s.quality_home, s.quality_away), (StreamQuality::Kbps256, StreamQuality::Original));
        assert!(!s.show_format_badge && !s.resume_contexts && !s.artwork_colours);
        assert_eq!((s.precache_count, s.art_cache_mb), (12, 512));
        assert_eq!((s.popular_source, s.reduce_motion), (PopularSource::MyPlays, ReduceMotion::On));
        // Loaded at startup into the live switches.
        assert!(fresh.tracking.private_session.load(Ordering::Relaxed));
        assert!(fresh.tracking.report_to_plex.load(Ordering::Relaxed));
        assert_eq!(fresh.streams.home(), Some(256));
        assert_eq!(db2.popular_source(), PopularSource::MyPlays);
    }

    #[test]
    fn ranges_are_clamped_and_volume_must_be_finite() {
        let mut s = Settings::default();
        s.apply(&SettingsPatch { volume: Some(3.0), ..Default::default() }).unwrap();
        assert_eq!(s.volume, 1.0);
        s.apply(&SettingsPatch { volume: Some(-1.0), ..Default::default() }).unwrap();
        assert_eq!(s.volume, 0.0);
        assert!(s.apply(&SettingsPatch { volume: Some(f32::NAN), ..Default::default() }).is_err());
        s.apply(&SettingsPatch { precache_count: Some(0), art_cache_mb: Some(1), ..Default::default() }).unwrap();
        assert_eq!((s.precache_count, s.art_cache_mb), (1, ART_CACHE_MIN_MB));
        s.apply(&SettingsPatch { precache_count: Some(500), ..Default::default() }).unwrap();
        assert_eq!(s.precache_count, 50);
    }

    #[test]
    fn unknown_and_missing_fields_fall_back_to_defaults() {
        let (db, _d) = db();
        db.write(|c| kv_set(c, KEY, r#"{"privateSession":true,"somethingNew":1,"precacheCount":999}"#)).unwrap();
        let s = load(&db);
        assert!(s.private_session);
        assert!(s.report_to_plex);
        assert_eq!(s.precache_count, PRECACHE_MAX);
        db.write(|c| kv_set(c, KEY, "not json")).unwrap();
        assert_eq!(load(&db), Settings::default());
    }

    #[test]
    fn wire_format_is_camel_case() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(v["privateSession"], false);
        assert_eq!(v["reportToPlex"], true);
        assert_eq!(v["shuffleStyle"], "spread");
        assert_eq!(v["qualityHome"], "original");
        assert_eq!(v["qualityAway"], "320");
        assert_eq!(v["showFormatBadge"], true);
        assert_eq!(v["resumeContexts"], true);
        assert_eq!(v["precacheCount"], 5);
        assert_eq!(v["popularSource"], "global");
        assert_eq!(v["artCacheMb"], 2048);
        assert_eq!(v["artworkColours"], true);
        assert_eq!(v["reduceMotion"], "system");
        let p: SettingsPatch = serde_json::from_str(r#"{"shuffleStyle":"standard"}"#).unwrap();
        assert_eq!(p.shuffle_style, Some(ShuffleStyle::Standard));
        let p: SettingsPatch =
            serde_json::from_str(r#"{"qualityAway":"128","popularSource":"myPlays","reduceMotion":"off"}"#).unwrap();
        assert_eq!(p.quality_away, Some(StreamQuality::Kbps128));
        assert_eq!(p.popular_source, Some(PopularSource::MyPlays));
        assert_eq!(p.reduce_motion, Some(ReduceMotion::Off));
        assert_eq!(p.report_to_plex, None);
    }

    #[tokio::test]
    async fn output_device_is_stored_per_device_and_survives_other_changes() {
        let (db, dir) = db();
        let st = SettingsState::new(db.clone(), Live::default());
        assert_eq!(st.get().await.unwrap().output_device, None);
        st.set_output_device(Some("wasapi:usb".into())).await.unwrap();
        // A normal patch keeps it; a patch can't set it.
        let p: SettingsPatch = serde_json::from_str(r#"{"reportToPlex":false,"outputDevice":"x"}"#).unwrap();
        st.set(p).await.unwrap();
        drop(st);
        drop(db);
        let db2 = DbHandle::open(dir.path().join("s.db")).unwrap();
        let s = load(&db2);
        assert_eq!(s.output_device.as_deref(), Some("wasapi:usb"));
        assert!(!s.report_to_plex);
        assert!(serde_json::to_value(Settings::default()).unwrap()["outputDevice"].is_null());
        let st2 = SettingsState::new(db2, Live::default());
        st2.set_output_device(None).await.unwrap();
        assert_eq!(st2.get().await.unwrap().output_device, None);
    }

    #[test]
    fn quality_maps_to_kbps() {
        assert_eq!(StreamQuality::Original.kbps(), None);
        assert_eq!(StreamQuality::Kbps192.kbps(), Some(192));
    }

    #[tokio::test]
    async fn art_cache_size_setting_resizes_the_cache() {
        let (db, dir) = db();
        let cache = art(dir.path());
        cache.store("/a", 64, 64, &image(400_000)).unwrap();
        cache.store("/b", 64, 64, &image(400_000)).unwrap();
        let live = Live { art: Some(cache.clone()), ..Default::default() };
        let st = SettingsState::new(db, live.clone());
        assert_eq!(cache.cap(), 2048 * MB, "default cap published at startup");
        st.set(SettingsPatch { art_cache_mb: Some(ART_CACHE_MIN_MB), ..Default::default() }).await.unwrap();
        assert_eq!(cache.cap(), ART_CACHE_MIN_MB as u64 * MB);
        assert_eq!(live.art_cap.load(Ordering::Relaxed), ART_CACHE_MIN_MB as u64 * MB);
    }

    #[test]
    fn clear_cache_removes_every_image_and_reports_the_space() {
        let dir = tempfile::tempdir().unwrap();
        let cache = art(dir.path());
        cache.store("/a", 64, 64, &image(1000)).unwrap();
        cache.store("/b", 64, 64, &image(3000)).unwrap();
        assert_eq!(cache.size(), 4000);
        assert_eq!(clear_art(&cache).unwrap(), 4000);
        assert_eq!(cache.size(), 0);
        assert!(cache.lookup("/a", 64, 64).unwrap().is_none());
        assert_eq!(clear_art(&cache).unwrap(), 0);
    }
}
