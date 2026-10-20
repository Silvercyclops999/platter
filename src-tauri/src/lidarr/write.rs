//! The only things Platter ever asks Lidarr to do, kept apart from the
//! read-only client. Each write is a fixed method and path ([`WriteCall`]);
//! nothing else can be built here:
//!
//! - add an artist (only with a fresh exclusion check, [`Cleared`]), with
//!   nothing monitored; protected artists unmonitored, into staging
//! - monitor or unmonitor albums, and search for them
//! - add one album (with its artist, nothing else monitored, no refresh of
//!   the artist's other albums), and grab one release Lidarr approved for it
//! - add or remove import list exclusions
//! - write back one settings page with one value changed
//! - add the staging root folder; add the "Platter Hi-Res" profile (and
//!   take either away again when nothing uses it)
//! - free imported files inside the staging folder ([`StagingFiles`])
//! - a manual import onto an album's current release
//! - turn off one album's "Automatically Switch Release" ([`SwitchOff`]),
//!   with the album otherwise written back exactly as Lidarr sent it
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

/// One album added on its own, with its artist when Lidarr doesn't have
/// them yet: the quick way in for a song someone wants to hear now. The
/// artist is added without reading its whole discography and with nothing
/// else monitored ([`AddArtist`]), and Lidarr doesn't search: Platter
/// picks the release itself ([`GrabRelease`]). Only [`AddAlbum::new`]
/// makes one, from an [`AddArtist`], so the exclusion check can't be skipped.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAlbum {
    pub foreign_album_id: String,
    pub title: String,
    pub monitored: bool,
    pub any_release_ok: bool,
    pub artist: AddArtist,
    pub add_options: AlbumAddOptions,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumAddOptions {
    pub search_for_new_album: bool,
}

impl AddAlbum {
    pub fn new(artist: AddArtist, release_group_id: &str, title: &str) -> Self {
        Self {
            foreign_album_id: release_group_id.trim().to_ascii_lowercase(),
            title: title.to_string(),
            monitored: true,
            any_release_ok: true,
            artist,
            add_options: AlbumAddOptions { search_for_new_album: false },
        }
    }
}

/// One release from Lidarr's own search for an album, sent to the download
/// client. Only [`GrabRelease::of`] makes one, and only from a release
/// Lidarr approved that isn't a whole discography.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabRelease {
    guid: String,
    indexer_id: i64,
}

impl GrabRelease {
    pub fn of(r: &super::api::Release) -> AppResult<Self> {
        if !r.approved || r.rejected || r.discography || r.guid.trim().is_empty() || r.indexer_id <= 0 {
            return Err(AppError::Invalid("That release isn't one Lidarr approved for this album, so Platter didn't grab it.".into()));
        }
        Ok(Self { guid: r.guid.clone(), indexer_id: r.indexer_id })
    }

    pub fn guid(&self) -> &str {
        &self.guid
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

/// One album written back with only "Automatically Switch Release"
/// (`anyReleaseOk`) turned off. Only [`SwitchOff::from_current`] makes one,
/// from the album as Lidarr just sent it, so nothing else about the album
/// (its edition, monitoring, profile) can change through it.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchOff {
    id: i64,
    body: serde_json::Value,
}

impl SwitchOff {
    /// `current` is `GET /album/{id}`'s answer, untouched.
    pub fn from_current(id: i64, current: &serde_json::Value) -> AppResult<Self> {
        if id <= 0 || !current.is_object() || current.get("id").and_then(serde_json::Value::as_i64) != Some(id) {
            return Err(AppError::Invalid("Lidarr's answer wasn't that album, so Platter didn't change anything.".into()));
        }
        if current.get("anyReleaseOk").and_then(serde_json::Value::as_bool) != Some(true) {
            return Err(AppError::Invalid("Automatically Switch Release is already off for this album.".into()));
        }
        let mut body = current.clone();
        body["anyReleaseOk"] = serde_json::Value::Bool(false);
        Ok(Self { id, body })
    }

    pub fn id(&self) -> i64 {
        self.id
    }
}

/// Whether `body` is `current` with only `anyReleaseOk` turned off (for a
/// write that came over the relay, checked against a fresh read).
pub fn only_switch_off(current: &serde_json::Value, body: &serde_json::Value) -> bool {
    let (Some(a), Some(b)) = (current.as_object(), body.as_object()) else { return false };
    if b.get("anyReleaseOk") != Some(&serde_json::Value::Bool(false)) || a.get("anyReleaseOk") != Some(&serde_json::Value::Bool(true)) {
        return false;
    }
    let rest = |m: &serde_json::Map<String, serde_json::Value>| m.iter().filter(|(k, _)| *k != "anyReleaseOk").map(|(k, v)| (k.clone(), v.clone())).collect::<std::collections::BTreeMap<_, _>>();
    rest(a) == rest(b)
}

/// One write. The method and path come from the variant.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteCall {
    AddArtist(AddArtist),
    AddAlbum(AddAlbum),
    GrabRelease(GrabRelease),
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
    /// One album's "Automatically Switch Release" turned off.
    SwitchOff(SwitchOff),
    /// One of the writes above as it arrived through a Bridge (the home
    /// Platter route) or from the Bridge's queue, checked again here.
    Relayed(RelayedWrite),
}

/// A write that came over the relay as method, path and body. Only
/// [`RelayedWrite::check`] makes one, and it takes only the shapes the
/// variants above produce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayedWrite {
    method: String,
    path: String,
    body: String,
}

/// Most bytes a relayed write's body may have.
const RELAYED_MAX: usize = 1024 * 1024;

fn numbered(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.len() <= 12 && n.bytes().all(|b| b.is_ascii_digit()))
}

impl RelayedWrite {
    pub fn check(method: &str, path: &str, body: &str) -> AppResult<Self> {
        let refuse = || AppError::Invalid("That isn't one of the changes Platter makes to Lidarr, so the Bridge didn't send it.".into());
        let m = method.to_ascii_uppercase();
        if body.len() > RELAYED_MAX {
            return Err(refuse());
        }
        let json = || serde_json::from_str::<serde_json::Value>(body).ok().filter(serde_json::Value::is_object);
        let ok = match m.as_str() {
            "POST" => match path {
                "/artist" | "/album" | "/release" | "/importlistexclusion" | "/rootfolder" | "/qualityprofile" => json().is_some(),
                "/command" => json().is_some_and(|v| matches!(v["name"].as_str(), Some("AlbumSearch" | "ManualImport"))),
                _ => false,
            },
            "PUT" => {
                if numbered(path, "/album/") {
                    // Only the release switch turned off, for the album the
                    // path names; the Bridge also checks it against a fresh
                    // read of the album before passing it on.
                    let id = path.trim_start_matches("/album/").parse::<i64>().ok();
                    json().is_some_and(|v| v["anyReleaseOk"] == serde_json::Value::Bool(false) && v["id"].as_i64() == id)
                } else {
                    (path == "/album/monitor"
                        || ["/config/naming/", "/config/metadataprovider/", "/config/mediamanagement/"].iter().any(|p| numbered(path, p)))
                        && json().is_some()
                }
            }
            "DELETE" => {
                (path == "/trackfile/bulk" && json().is_some())
                    || (["/importlistexclusion/", "/rootfolder/", "/qualityprofile/"].iter().any(|p| numbered(path, p)) && body.trim().is_empty())
            }
            _ => false,
        };
        if !ok {
            return Err(refuse());
        }
        Ok(Self { method: m, path: path.to_string(), body: body.to_string() })
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    /// The same write with its body rebuilt (see `relayed::vet`).
    pub(super) fn with_body(&self, body: String) -> Self {
        Self { method: self.method.clone(), path: self.path.clone(), body }
    }

    /// The album a release-switch write is for, if it is one.
    pub fn switch_off_album(&self) -> Option<i64> {
        if self.method == "PUT" && numbered(&self.path, "/album/") { self.path.trim_start_matches("/album/").parse().ok() } else { None }
    }

    /// A release-switch write must be the album as Lidarr has it now with
    /// only that switch turned off. Reads the album (GET) to make sure;
    /// every other write passes as it is.
    pub async fn confirm_with(&self, get: &dyn super::api::LidarrGet) -> AppResult<()> {
        let Some(id) = self.switch_off_album() else { return Ok(()) };
        let refuse = || AppError::Invalid("That album changed in Lidarr, or the change would touch more than its release switch, so it wasn't sent.".into());
        let resp = get.get(&format!("/album/{id}"), &[]).await.map_err(|_| AppError::Network(UNREACHABLE.into()))?;
        if !(200..=299).contains(&resp.status) {
            return Err(refuse());
        }
        let current: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|_| refuse())?;
        self.confirm_against(&current)
    }

    /// [`Self::confirm_with`] against an album already read.
    pub fn confirm_against(&self, current: &serde_json::Value) -> AppResult<()> {
        if self.switch_off_album().is_none() {
            return Ok(());
        }
        let refuse = || AppError::Invalid("That album changed in Lidarr, or the change would touch more than its release switch, so it wasn't sent.".into());
        let body: serde_json::Value = serde_json::from_str(&self.body).map_err(|_| refuse())?;
        if only_switch_off(current, &body) { Ok(()) } else { Err(refuse()) }
    }
}

impl WriteCall {
    /// A change to Lidarr's own settings, profiles, roots or exclusions:
    /// these always show the change and wait for a person's yes, so a
    /// Bridge never makes one on its own.
    pub fn is_settings_write(&self) -> bool {
        match self {
            WriteCall::AddExclusion { .. }
            | WriteCall::RemoveExclusion { .. }
            | WriteCall::PutConfig { .. }
            | WriteCall::AddRootFolder(_)
            | WriteCall::RemoveRootFolder { .. }
            | WriteCall::AddQualityProfile(_)
            | WriteCall::RemoveQualityProfile { .. }
            | WriteCall::SwitchOff(_) => true,
            WriteCall::Relayed(r) => {
                ["/importlistexclusion", "/config/", "/rootfolder", "/qualityprofile"].iter().any(|p| r.path.starts_with(p)) || r.switch_off_album().is_some()
            }
            _ => false,
        }
    }
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

impl WriteMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            WriteMethod::Post => "POST",
            WriteMethod::Put => "PUT",
            WriteMethod::Delete => "DELETE",
        }
    }
}

impl WriteCall {
    pub fn method(&self) -> WriteMethod {
        match self {
            WriteCall::AddArtist(_)
            | WriteCall::AddAlbum(_)
            | WriteCall::GrabRelease(_)
            | WriteCall::AlbumSearch { .. }
            | WriteCall::AddExclusion { .. }
            | WriteCall::AddRootFolder(_)
            | WriteCall::AddQualityProfile(_)
            | WriteCall::ManualImport(_) => WriteMethod::Post,
            WriteCall::Relayed(r) => match r.method.as_str() {
                "PUT" => WriteMethod::Put,
                "DELETE" => WriteMethod::Delete,
                _ => WriteMethod::Post,
            },
            WriteCall::Monitor { .. } | WriteCall::PutConfig { .. } | WriteCall::SwitchOff(_) => WriteMethod::Put,
            WriteCall::RemoveExclusion { .. } | WriteCall::RemoveRootFolder { .. } | WriteCall::RemoveQualityProfile { .. } | WriteCall::FreeStaging(_) => {
                WriteMethod::Delete
            }
        }
    }

    /// The path under `/api/v1`.
    pub fn path(&self) -> String {
        match self {
            WriteCall::AddArtist(_) => "/artist".into(),
            WriteCall::AddAlbum(_) => "/album".into(),
            WriteCall::GrabRelease(_) => "/release".into(),
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
            WriteCall::SwitchOff(s) => format!("/album/{}", s.id),
            WriteCall::Relayed(r) => r.path.clone(),
        }
    }

    pub fn body(&self) -> String {
        use serde_json::json;
        let v = match self {
            WriteCall::AddArtist(a) => serde_json::to_string(a),
            WriteCall::AddAlbum(a) => serde_json::to_string(a),
            WriteCall::GrabRelease(g) => serde_json::to_string(g),
            WriteCall::Monitor { album_ids, monitored } => serde_json::to_string(&MonitorBody { album_ids, monitored: *monitored }),
            WriteCall::AlbumSearch { album_ids } => serde_json::to_string(&CommandBody { name: "AlbumSearch", album_ids }),
            WriteCall::AddExclusion { foreign_id, artist_name } => serde_json::to_string(&json!({"foreignId": foreign_id, "artistName": artist_name})),
            WriteCall::PutConfig { body, .. } | WriteCall::AddQualityProfile(body) => serde_json::to_string(body),
            WriteCall::SwitchOff(s) => serde_json::to_string(&s.body),
            WriteCall::AddRootFolder(r) => serde_json::to_string(r),
            WriteCall::FreeStaging(f) => serde_json::to_string(&json!({"trackFileIds": f.ids})),
            WriteCall::ManualImport(files) => {
                serde_json::to_string(&json!({"name": "ManualImport", "importMode": "move", "replaceExistingFiles": false, "files": files}))
            }
            WriteCall::RemoveExclusion { .. } | WriteCall::RemoveRootFolder { .. } | WriteCall::RemoveQualityProfile { .. } => Ok(String::new()),
            WriteCall::Relayed(r) => Ok(r.body.clone()),
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
            s if s >= 500 && matches!(call, WriteCall::AddArtist(_) | WriteCall::AddAlbum(_)) => Err(AppError::Server(METADATA_DOWN.into())),
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

    /// Add one album (and its artist when new); returns Lidarr's id for the album.
    pub async fn add_album(&self, a: AddAlbum) -> AppResult<i64> {
        let resp = self.send(WriteCall::AddAlbum(a)).await?;
        let added: Added = serde_json::from_slice(&resp.body).unwrap_or_default();
        if added.id <= 0 {
            return Err(AppError::Server("Lidarr didn't say which album it added. Nothing more was sent.".into()));
        }
        Ok(added.id)
    }

    /// Send one approved release to the download client.
    pub async fn grab(&self, g: GrabRelease) -> AppResult<()> {
        self.send(WriteCall::GrabRelease(g)).await.map(|_| ())
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
    fn an_album_comes_in_on_its_own_and_only_an_approved_release_is_grabbed() {
        let ok = clear_exclusion(&[], "a0000000-0000-0000-0000-000000000001", "Northlight").unwrap();
        let artist = AddArtist::managed(ok, "Northlight", 2, 3, "/downloads/music");
        let call = WriteCall::AddAlbum(AddAlbum::new(artist, " B0000000-0000-0000-0000-00000000000A ", "Second Wind"));
        assert_eq!((call.method(), call.path().as_str()), (WriteMethod::Post, "/album"));
        assert!(!call.is_settings_write());
        let v: Value = serde_json::from_str(&call.body()).unwrap();
        assert_eq!(v["foreignAlbumId"], "b0000000-0000-0000-0000-00000000000a");
        assert_eq!((v["monitored"].as_bool(), v["addOptions"]["searchForNewAlbum"].as_bool()), (Some(true), Some(false)));
        assert_eq!(v["artist"]["addOptions"], json!({"monitor": "none", "searchForMissingAlbums": false}));
        assert_eq!(v["artist"]["monitorNewItems"], "none");

        use super::super::api::Release;
        let good = Release { guid: "g1".into(), indexer_id: 4, approved: true, download_allowed: true, ..Default::default() };
        let call = WriteCall::GrabRelease(GrabRelease::of(&good).unwrap());
        assert_eq!((call.method(), call.path().as_str()), (WriteMethod::Post, "/release"));
        assert_eq!(serde_json::from_str::<Value>(&call.body()).unwrap(), json!({"guid": "g1", "indexerId": 4}));
        for bad in [
            Release { approved: false, ..good.clone() },
            Release { rejected: true, ..good.clone() },
            Release { discography: true, ..good.clone() },
            Release { guid: " ".into(), ..good.clone() },
            Release { indexer_id: 0, ..good.clone() },
        ] {
            assert!(GrabRelease::of(&bad).is_err(), "{bad:?}");
        }
        assert!(RelayedWrite::check("POST", "/release", r#"{"guid": "g1", "indexerId": 4}"#).is_ok());
        assert!(RelayedWrite::check("POST", "/album", r#"{"foreignAlbumId": "x"}"#).is_ok());
        assert!(RelayedWrite::check("DELETE", "/release", "").is_err());
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

    #[test]
    fn relayed_writes_take_only_the_fixed_shapes() {
        let ok = |m: &str, p: &str, b: &str| RelayedWrite::check(m, p, b).is_ok();
        assert!(ok("PUT", "/album/monitor", r#"{"albumIds": [1], "monitored": false}"#));
        assert!(ok("post", "/command", r#"{"name": "AlbumSearch", "albumIds": [1]}"#));
        assert!(ok("DELETE", "/importlistexclusion/12", ""));
        assert!(ok("PUT", "/config/naming/1", r#"{"id": 1}"#));
        assert!(ok("DELETE", "/trackfile/bulk", r#"{"trackFileIds": [1]}"#));
        assert!(!ok("POST", "/command", r#"{"name": "DeleteArtist"}"#), "only the two commands Platter sends");
        assert!(!ok("DELETE", "/artist/4", ""), "never deletes an artist");
        assert!(!ok("PUT", "/config/host/1", r#"{"id": 1}"#), "not a page Platter writes");
        assert!(!ok("DELETE", "/rootfolder/1/../x", ""));
        assert!(!ok("GET", "/artist", ""));
        assert!(!ok("PUT", "/album/monitor", "not json"));
        let w = WriteCall::Relayed(RelayedWrite::check("PUT", "/config/naming/1", r#"{"id": 1}"#).unwrap());
        assert!(w.is_settings_write());
        assert_eq!((w.method(), w.path().as_str()), (WriteMethod::Put, "/config/naming/1"));
        assert!(!WriteCall::Relayed(RelayedWrite::check("PUT", "/album/monitor", "{}").unwrap()).is_settings_write());
        assert!(WriteCall::PutConfig { kind: ConfigKind::Naming, id: 1, body: json!({}) }.is_settings_write());
        assert!(!WriteCall::Monitor { album_ids: vec![1], monitored: false }.is_settings_write());
    }

    #[test]
    fn the_release_switch_write_changes_that_one_field_and_nothing_else() {
        let album = json!({"id": 9, "title": "Panes", "monitored": true, "anyReleaseOk": true, "qualityProfileId": 2,
            "releases": [{"id": 90, "monitored": true, "trackCount": 13}, {"id": 91, "monitored": false, "trackCount": 19}]});
        let s = SwitchOff::from_current(9, &album).unwrap();
        let call = WriteCall::SwitchOff(s);
        assert_eq!((call.method(), call.path().as_str()), (WriteMethod::Put, "/album/9"));
        assert!(call.is_settings_write(), "asked first, never on a Bridge's own");
        let body: Value = serde_json::from_str(&call.body()).unwrap();
        assert_eq!(body["anyReleaseOk"], false);
        let mut back = body.clone();
        back["anyReleaseOk"] = json!(true);
        assert_eq!(back, album, "everything else is the album as Lidarr sent it");
        assert!(only_switch_off(&album, &body));
        // Not the album asked for, or already off: nothing is built.
        assert!(SwitchOff::from_current(8, &album).is_err());
        assert!(SwitchOff::from_current(9, &json!({"id": 9, "anyReleaseOk": false})).is_err());
        // Anything else changed alongside is refused.
        for (k, v) in [("monitored", json!(false)), ("qualityProfileId", json!(5)), ("releases", json!([{"id": 91, "monitored": true}])), ("extra", json!(1))] {
            let mut bad = body.clone();
            bad[k] = v;
            assert!(!only_switch_off(&album, &bad), "{k}");
        }
        assert!(!only_switch_off(&album, &album), "the switch itself must go off");
    }

    #[tokio::test]
    async fn a_relayed_release_switch_is_checked_against_a_fresh_read() {
        use super::super::api::{LidarrGet, Route};
        struct One(Value);
        #[async_trait]
        impl LidarrGet for One {
            async fn get(&self, path: &str, _q: &[(&str, &str)]) -> AppResult<HttpResponse> {
                assert_eq!(path, "/album/9");
                Ok(HttpResponse { status: 200, body: bytes::Bytes::from(self.0.to_string()), ..Default::default() })
            }
            fn route(&self) -> Route {
                Route::Home
            }
        }
        let album = json!({"id": 9, "monitored": true, "anyReleaseOk": true, "releases": [{"id": 90, "monitored": true}]});
        let good = WriteCall::SwitchOff(SwitchOff::from_current(9, &album).unwrap()).body();
        let r = RelayedWrite::check("PUT", "/album/9", &good).unwrap();
        assert!(WriteCall::Relayed(r.clone()).is_settings_write());
        r.confirm_with(&One(album.clone())).await.unwrap();
        // Lidarr moved the album to another edition since: refused.
        let moved = json!({"id": 9, "monitored": true, "anyReleaseOk": true, "releases": [{"id": 91, "monitored": true}]});
        assert!(r.confirm_with(&One(moved)).await.is_err());
        // A body that changes monitoring too, or names another album, never passes.
        let sneaky = json!({"id": 9, "monitored": false, "anyReleaseOk": false, "releases": [{"id": 90, "monitored": true}]}).to_string();
        assert!(RelayedWrite::check("PUT", "/album/9", &sneaky).unwrap().confirm_with(&One(album.clone())).await.is_err());
        assert!(RelayedWrite::check("PUT", "/album/8", &good).is_err(), "the path and body name the same album");
        assert!(RelayedWrite::check("PUT", "/album/9", r#"{"id": 9, "anyReleaseOk": true}"#).is_err(), "only ever turned off");
        assert!(RelayedWrite::check("DELETE", "/album/9", "").is_err());
    }
}