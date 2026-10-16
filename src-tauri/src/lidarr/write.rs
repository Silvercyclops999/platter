//! The only things Platter ever asks Lidarr to do, kept apart from the
//! read-only client. Each write is a fixed method and path ([`WriteCall`]);
//! nothing else can be built here:
//!
//! - add an artist (only with a fresh exclusion check, [`Cleared`]), with
//!   nothing monitored; protected artists unmonitored, into staging
//! - monitor or unmonitor albums, and search for them
//! - add or remove import list exclusions
//! - write back one settings page with one value changed
//! - add the staging root folder; add the "Platter Hi-Res" profile (and
//!   take either away again when nothing uses it)
//! - free imported files inside the staging folder ([`StagingFiles`])
//! - a manual import onto an album's current release
//!
//! Writes to Lidarr's settings, profiles, roots and exclusions always show
//! the exact change and wait for a yes (`changes`). Nothing here renames,
//! retags or rescans, and no root folder holding artists is ever removed.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;

use super::api::{BAD_KEY, DirectRoute, METADATA_DOWN, Route, UNREACHABLE};
use crate::artistinfo::http::HttpResponse;
use crate::error::{AppError, AppResult};

/// What `addOptions` and `monitorNewItems` must say: nothing but the
/// requested album is ever monitored.
pub const MONITOR_NONE: &str = "none";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddOptions {
    pub monitor: &'static str,
    pub search_for_missing_albums: bool,
}

/// Proof that an artist's MusicBrainz id was checked against Lidarr's
/// import list exclusions just before adding it. Only [`clear_exclusion`]
/// makes one, and [`AddArtist`] can't be built without one, so no add path
/// can skip the check. (Lidarr itself doesn't stop a manual add of an
/// excluded artist.) Artists are matched by id, never by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleared {
    mbid: String,
}

impl Cleared {
    pub fn mbid(&self) -> &str {
        &self.mbid
    }
}

/// Check `mbid` against the exclusions Lidarr has right now.
pub fn clear_exclusion(exclusions: &[super::api::Exclusion], mbid: &str, name: &str) -> AppResult<Cleared> {
    let m = mbid.trim().to_ascii_lowercase();
    if !crate::artistinfo::is_mbid(&m) {
        return Err(AppError::Invalid(format!("Platter doesn't know {name}'s MusicBrainz id, so it can't check Lidarr's exclusions. Nothing was sent.")));
    }
    if exclusions.iter().any(|e| e.foreign_id.trim().eq_ignore_ascii_case(&m)) {
        return Err(AppError::Invalid(format!(
            "{name} is on Lidarr's import list exclusions, so Platter won't add it to Lidarr. Nothing was sent. If you want albums for it, add it in Lidarr yourself (for a protected artist, with a path inside the staging folder) or take it off the list."
        )));
    }
    Ok(Cleared { mbid: m })
}

/// The artist to add. Built only by [`AddArtist::managed`] or
/// [`AddArtist::protected`], so monitoring is always limited.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddArtist {
    pub artist_name: String,
    pub foreign_artist_id: String,
    pub quality_profile_id: i64,
    pub metadata_profile_id: i64,
    pub root_folder_path: String,
    pub monitored: bool,
    pub monitor_new_items: &'static str,
    pub add_options: AddOptions,
}

impl AddArtist {
    /// A Managed artist into one of Lidarr's library roots: the artist is
    /// monitored, but no album is until Platter asks for one.
    pub fn managed(cleared: Cleared, name: &str, quality_profile_id: i64, metadata_profile_id: i64, root: &str) -> Self {
        Self {
            artist_name: name.to_string(),
            foreign_artist_id: cleared.mbid,
            quality_profile_id,
            metadata_profile_id,
            root_folder_path: root.to_string(),
            monitored: true,
            monitor_new_items: MONITOR_NONE,
            add_options: AddOptions { monitor: MONITOR_NONE, search_for_missing_albums: false },
        }
    }

    /// A protected artist, only ever into the staging root and unmonitored
    /// (measured: Lidarr then only refreshes the artist and rescans its own
    /// folder; nothing is searched or queued). Platter monitors just the
    /// albums it requests.
    pub fn protected(cleared: Cleared, name: &str, quality_profile_id: i64, metadata_profile_id: i64, staging_root: &str) -> Self {
        Self { monitored: false, ..Self::managed(cleared, name, quality_profile_id, metadata_profile_id, staging_root) }
    }
}

/// One of Lidarr's settings pages, written back whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigKind {
    Naming,
    MetadataProvider,
    MediaManagement,
}

impl ConfigKind {
    pub fn path(self) -> &'static str {
        match self {
            ConfigKind::Naming => "/config/naming",
            ConfigKind::MetadataProvider => "/config/metadataprovider",
            ConfigKind::MediaManagement => "/config/mediamanagement",
        }
    }
}

/// A new root folder (the staging folder): nothing in it is ever monitored
/// by default.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewRoot {
    pub name: String,
    pub path: String,
    pub default_quality_profile_id: i64,
    pub default_metadata_profile_id: i64,
    pub default_monitor_option: &'static str,
    pub default_new_item_monitor_option: &'static str,
}

impl NewRoot {
    pub fn staging(path: &str, quality: i64, metadata: i64) -> Self {
        Self {
            name: "Staging".into(),
            path: path.to_string(),
            default_quality_profile_id: quality,
            default_metadata_profile_id: metadata,
            default_monitor_option: MONITOR_NONE,
            default_new_item_monitor_option: MONITOR_NONE,
        }
    }
}

/// Imported files Platter may free: every one is inside the staging folder
/// (checked when made, by whole folder names). Only [`StagingFiles::check`]
/// makes one, so nothing outside staging can be deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingFiles {
    ids: Vec<i64>,
}

impl StagingFiles {
    pub fn check(files: &[super::api::TrackFile], staging: &str) -> AppResult<StagingFiles> {
        let root = super::paths::NormPath::parse(staging);
        if root.is_empty() {
            return Err(AppError::Invalid("No staging folder is set, so Platter won't free any files.".into()));
        }
        if let Some(f) = files.iter().find(|f| !super::paths::NormPath::parse(&f.path).starts_with(&root) || f.id <= 0) {
            return Err(AppError::Invalid(format!("{} isn't in the staging folder, so Platter won't delete it. Nothing was freed.", f.path)));
        }
        Ok(StagingFiles { ids: files.iter().map(|f| f.id).collect() })
    }

    pub fn ids(&self) -> &[i64] {
        &self.ids
    }
}

/// One file for Lidarr's manual import, built by the re-match guard only.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFile {
    pub path: String,
    pub artist_id: i64,
    pub album_id: i64,
    pub album_release_id: i64,
    pub track_ids: Vec<i64>,
    pub quality: serde_json::Value,
    pub indexer_flags: i64,
    pub download_id: String,
    pub disable_release_switching: bool,
}

/// One write. The method and path come from the variant.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteCall {
    AddArtist(AddArtist),
    Monitor { album_ids: Vec<i64>, monitored: bool },
    AlbumSearch { album_ids: Vec<i64> },
    /// Put an artist on the import list exclusions.
    AddExclusion { foreign_id: String, artist_name: String },
    RemoveExclusion { id: i64 },
    /// A settings page written back whole, with one value changed.
    PutConfig { kind: ConfigKind, id: i64, body: serde_json::Value },
    AddRootFolder(NewRoot),
    RemoveRootFolder { id: i64 },
    AddQualityProfile(serde_json::Value),
    RemoveQualityProfile { id: i64 },
    /// Lidarr deletes these imported files (into its recycling bin when one is set).
    FreeStaging(StagingFiles),
    /// Lidarr's manual import, moving files in place onto the album's
    /// current release, never switching it.
    ManualImport(Vec<ImportFile>),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitorBody<'a> {
    album_ids: &'a [i64],
    monitored: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandBody<'a> {
    name: &'static str,
    album_ids: &'a [i64],
}

/// HTTP method of a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMethod {
    Post,
    Put,
    Delete,
}

impl WriteCall {
    pub fn method(&self) -> WriteMethod {
        match self {
            WriteCall::AddArtist(_)
            | WriteCall::AlbumSearch { .. }
            | WriteCall::AddExclusion { .. }
            | WriteCall::AddRootFolder(_)
            | WriteCall::AddQualityProfile(_)
            | WriteCall::ManualImport(_) => WriteMethod::Post,
            WriteCall::Monitor { .. } | WriteCall::PutConfig { .. } => WriteMethod::Put,
            WriteCall::RemoveExclusion { .. } | WriteCall::RemoveRootFolder { .. } | WriteCall::RemoveQualityProfile { .. } | WriteCall::FreeStaging(_) => {
                WriteMethod::Delete
            }
        }
    }

    /// The path under `/api/v1`.
    pub fn path(&self) -> String {
        match self {
            WriteCall::AddArtist(_) => "/artist".into(),
            WriteCall::Monitor { .. } => "/album/monitor".into(),
            WriteCall::AlbumSearch { .. } | WriteCall::ManualImport(_) => "/command".into(),
            WriteCall::AddExclusion { .. } => "/importlistexclusion".into(),
            WriteCall::RemoveExclusion { id } => format!("/importlistexclusion/{id}"),
            WriteCall::PutConfig { kind, id, .. } => format!("{}/{id}", kind.path()),
            WriteCall::AddRootFolder(_) => "/rootfolder".into(),
            WriteCall::RemoveRootFolder { id } => format!("/rootfolder/{id}"),
            WriteCall::AddQualityProfile(_) => "/qualityprofile".into(),
            WriteCall::RemoveQualityProfile { id } => format!("/qualityprofile/{id}"),
            WriteCall::FreeStaging(_) => "/trackfile/bulk".into(),
        }
    }

    pub fn body(&self) -> String {
        use serde_json::json;
        let v = match self {
            WriteCall::AddArtist(a) => serde_json::to_string(a),
            WriteCall::Monitor { album_ids, monitored } => serde_json::to_string(&MonitorBody { album_ids, monitored: *monitored }),
            WriteCall::AlbumSearch { album_ids } => serde_json::to_string(&CommandBody { name: "AlbumSearch", album_ids }),
            WriteCall::AddExclusion { foreign_id, artist_name } => serde_json::to_string(&json!({"foreignId": foreign_id, "artistName": artist_name})),
            WriteCall::PutConfig { body, .. } | WriteCall::AddQualityProfile(body) => serde_json::to_string(body),
            WriteCall::AddRootFolder(r) => serde_json::to_string(r),
            WriteCall::FreeStaging(f) => serde_json::to_string(&json!({"trackFileIds": f.ids})),
            WriteCall::ManualImport(files) => {
                serde_json::to_string(&json!({"name": "ManualImport", "importMode": "move", "replaceExistingFiles": false, "files": files}))
            }
            WriteCall::RemoveExclusion { .. } | WriteCall::RemoveRootFolder { .. } | WriteCall::RemoveQualityProfile { .. } => Ok(String::new()),
        };
        v.unwrap_or_default()
    }
}

/// How writes reach Lidarr: one call at a time, each a [`WriteCall`].
#[async_trait]
pub trait LidarrWrite: Send + Sync {
    async fn write(&self, call: &WriteCall) -> AppResult<HttpResponse>;
    fn route(&self) -> Route;
}

#[async_trait]
impl LidarrWrite for DirectRoute {
    async fn write(&self, call: &WriteCall) -> AppResult<HttpResponse> {
        self.send_write(call).await
    }

    fn route(&self) -> Route {
        super::api::LidarrGet::route(self)
    }
}

/// The writes with their answers checked.
#[derive(Clone)]
pub struct LidarrWriter {
    transport: Arc<dyn LidarrWrite>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Added {
    #[serde(default)]
    pub id: i64,
}

impl LidarrWriter {
    pub fn new(transport: Arc<dyn LidarrWrite>) -> Self {
        Self { transport }
    }

    async fn send(&self, call: WriteCall) -> AppResult<HttpResponse> {
        let resp = self.transport.write(&call).await.map_err(|_| AppError::Network(UNREACHABLE.into()))?;
        match resp.status {
            200..=299 => Ok(resp),
            401 | 403 => Err(AppError::Auth(BAD_KEY.into())),
            // Adding an artist makes Lidarr ask its metadata service.
            s if s >= 500 && matches!(call, WriteCall::AddArtist(_)) => Err(AppError::Server(METADATA_DOWN.into())),
            s => Err(AppError::Server(format!("Lidarr answered with an error ({s}). Nothing more was sent."))),
        }
    }

    /// Add an artist with monitoring limited; returns Lidarr's id for it.
    pub async fn add_artist(&self, a: AddArtist) -> AppResult<i64> {
        let resp = self.send(WriteCall::AddArtist(a)).await?;
        let added: Added = serde_json::from_slice(&resp.body).unwrap_or_default();
        if added.id <= 0 {
            return Err(AppError::Server("Lidarr didn't say which artist it added. Nothing more was sent.".into()));
        }
        Ok(added.id)
    }

    pub async fn monitor(&self, album_ids: Vec<i64>, monitored: bool) -> AppResult<()> {
        if album_ids.is_empty() {
            return Ok(());
        }
        self.send(WriteCall::Monitor { album_ids, monitored }).await.map(|_| ())
    }

    pub async fn search(&self, album_ids: Vec<i64>) -> AppResult<()> {
        self.send(WriteCall::AlbumSearch { album_ids }).await.map(|_| ())
    }

    /// Any of the other writes; returns the id Lidarr gave a new item (0
    /// when it doesn't say).
    pub async fn send_call(&self, call: WriteCall) -> AppResult<i64> {
        let resp = self.send(call).await?;
        Ok(serde_json::from_slice::<Added>(&resp.body).map(|a| a.id).unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn payloads_limit_monitoring() {
        let ex = vec![super::super::api::Exclusion { foreign_id: "B0000000-0000-0000-0000-000000000002".into(), artist_name: "Quiet Hours".into() }];
        assert!(clear_exclusion(&ex, "b0000000-0000-0000-0000-000000000002", "Quiet Hours").is_err(), "matched by id, any case");
        assert!(clear_exclusion(&ex, "not an id", "Someone").is_err(), "no id, no add");
        let ok = clear_exclusion(&ex, "a0000000-0000-0000-0000-000000000001", "Northlight").unwrap();
        let prot = AddArtist::protected(ok.clone(), "Northlight", 2, 3, "/data/staging/music");
        let v: Value = serde_json::from_str(&WriteCall::AddArtist(prot).body()).unwrap();
        assert_eq!(v["monitored"], false);
        assert_eq!(v["rootFolderPath"], "/data/staging/music");
        assert_eq!(v["addOptions"], json!({"monitor": "none", "searchForMissingAlbums": false}));
        let call = WriteCall::AddArtist(AddArtist::managed(ok, "Northlight", 2, 3, "/downloads/music"));
        assert_eq!(call.method(), WriteMethod::Post);
        assert_eq!(call.path(), "/artist");
        let v: Value = serde_json::from_str(&call.body()).unwrap();
        assert_eq!(
            v,
            json!({
                "artistName": "Northlight", "foreignArtistId": "a0000000-0000-0000-0000-000000000001", "qualityProfileId": 2, "metadataProfileId": 3,
                "rootFolderPath": "/downloads/music", "monitored": true, "monitorNewItems": "none",
                "addOptions": {"monitor": "none", "searchForMissingAlbums": false}
            })
        );
        let m = WriteCall::Monitor { album_ids: vec![7], monitored: true };
        assert_eq!((m.method(), m.path().as_str()), (WriteMethod::Put, "/album/monitor"));
        assert_eq!(serde_json::from_str::<Value>(&m.body()).unwrap(), json!({"albumIds": [7], "monitored": true}));
        let s = WriteCall::AlbumSearch { album_ids: vec![7] };
        assert_eq!((s.method(), s.path().as_str()), (WriteMethod::Post, "/command"));
        assert_eq!(serde_json::from_str::<Value>(&s.body()).unwrap(), json!({"name": "AlbumSearch", "albumIds": [7]}));
    }

    #[test]
    fn only_files_inside_the_staging_folder_can_be_freed() {
        use super::super::api::TrackFile;
        let f = |id, p: &str| TrackFile { id, path: p.into(), ..Default::default() };
        let ok = StagingFiles::check(&[f(1, "/data/staging/music/A/B/01.flac"), f(2, r"\DATA\staging\music\A\B\02.flac")], "/data/staging/music").unwrap();
        let call = WriteCall::FreeStaging(ok);
        assert_eq!((call.method(), call.path().as_str()), (WriteMethod::Delete, "/trackfile/bulk"));
        assert_eq!(serde_json::from_str::<Value>(&call.body()).unwrap(), json!({"trackFileIds": [1, 2]}));
        for bad in ["/data/media/Music/A/01.flac", "/data/staging/music-old/A/01.flac"] {
            assert!(StagingFiles::check(&[f(1, "/data/staging/music/A/01.flac"), f(3, bad)], "/data/staging/music").is_err(), "{bad}");
        }
        assert!(StagingFiles::check(&[f(1, "/x/01.flac")], " ").is_err());
    }

    #[test]
    fn the_other_writes_have_fixed_shapes() {
        let c = WriteCall::PutConfig { kind: ConfigKind::Naming, id: 1, body: json!({"id": 1, "renameTracks": false}) };
        assert_eq!((c.method(), c.path().as_str()), (WriteMethod::Put, "/config/naming/1"));
        let r = WriteCall::AddRootFolder(NewRoot::staging("/data/staging/music", 2, 3));
        let v: Value = serde_json::from_str(&r.body()).unwrap();
        assert_eq!((v["defaultMonitorOption"].as_str(), v["defaultNewItemMonitorOption"].as_str()), (Some("none"), Some("none")));
        let e = WriteCall::AddExclusion { foreign_id: "x".into(), artist_name: "Quiet Hours".into() };
        assert_eq!((e.method(), e.path().as_str()), (WriteMethod::Post, "/importlistexclusion"));
        assert_eq!(WriteCall::RemoveExclusion { id: 4 }.path(), "/importlistexclusion/4");
        assert_eq!(WriteCall::RemoveExclusion { id: 4 }.body(), "");
        let m = WriteCall::ManualImport(vec![]);
        let v: Value = serde_json::from_str(&m.body()).unwrap();
        assert_eq!((v["name"].as_str(), v["importMode"].as_str(), v["replaceExistingFiles"].as_bool()), (Some("ManualImport"), Some("move"), Some(false)));
    }
}
