//! Lidarr's API, read only. Every call here is a GET: this client has no
//! way to send anything else, and a test checks that a whole setup check
//! sends nothing but GETs. The API key goes in a header flagged sensitive
//! and never into an error, a log line or a URL.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::artistinfo::http::HttpResponse;
use crate::error::{AppError, AppResult};
use crate::plex::token::Token;
use crate::web::{Web, WebRequest};

/// Lidarr's API, version 1 (Lidarr 1.0 and later).
const API: &str = "/api/v1";

pub const UNREACHABLE: &str = "Lidarr isn't reachable. Check the address, and that Lidarr is running.";
pub const BAD_KEY: &str = "Lidarr didn't accept this API key. Copy it again from Lidarr's Settings > General.";
pub const METADATA_DOWN: &str = "Lidarr can't look up albums right now. Its metadata service isn't answering; try again later.";
pub const NOT_LIDARR: &str = "That address doesn't answer like Lidarr. Check the address and port.";

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub app_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmappedFolder {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootFolder {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub path: String,
    #[serde(default = "yes")]
    pub accessible: bool,
    /// Folders under the root that no Lidarr artist uses.
    #[serde(default)]
    pub unmapped_folders: Vec<UnmappedFolder>,
    #[serde(default)]
    pub default_quality_profile_id: Option<i64>,
    #[serde(default)]
    pub default_metadata_profile_id: Option<i64>,
    /// What a new artist in this folder monitors by default (`all`, `none`...).
    #[serde(default)]
    pub default_monitor_option: Option<String>,
    /// Whether new albums get monitored by default (`all`, `none`, `new`).
    #[serde(default)]
    pub default_new_item_monitor_option: Option<String>,
}

/// An artist Lidarr's import lists must never add (`GET /importlistexclusion`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exclusion {
    /// The artist's MusicBrainz id.
    #[serde(default)]
    pub foreign_id: String,
    #[serde(default)]
    pub artist_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub path: String,
    /// The artist's MusicBrainz id.
    #[serde(default)]
    pub foreign_artist_id: String,
    #[serde(default)]
    pub monitored: bool,
    /// `all`, `none` or `new`.
    #[serde(default)]
    pub monitor_new_items: Option<String>,
    /// The folder name Lidarr would use (lookups).
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub quality_profile_id: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamingConfig {
    #[serde(default)]
    pub rename_tracks: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataProviderConfig {
    /// `no`, `newFiles`, `allFiles` or `sync`.
    #[serde(default)]
    pub write_audio_tags: String,
    #[serde(default)]
    pub scrub_audio_tags: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaManagementConfig {
    #[serde(default)]
    pub delete_empty_folders: bool,
    /// Empty when Lidarr deletes files for good.
    #[serde(default)]
    pub recycle_bin: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityProfile {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub upgrade_allowed: bool,
    /// The quality (or group) id Lidarr stops at.
    #[serde(default)]
    pub cutoff: Option<i64>,
    /// Qualities and groups, lowest first.
    #[serde(default)]
    pub items: Vec<ProfileItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityRef {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
}

/// One quality in a profile, or a group of them (`items`, with its own id).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileItem {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub quality: Option<QualityRef>,
    #[serde(default)]
    pub items: Vec<ProfileItem>,
    #[serde(default)]
    pub allowed: bool,
}

/// One track of an album as Lidarr has it (`GET /track`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LidarrTrack {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    /// The MusicBrainz recording id.
    #[serde(default)]
    pub foreign_recording_id: String,
    #[serde(default)]
    pub medium_number: i64,
    #[serde(default)]
    pub absolute_track_number: i64,
    /// Milliseconds.
    #[serde(default)]
    pub duration: i64,
    #[serde(default)]
    pub track_file_id: i64,
}

/// A file Lidarr imported (`GET /trackfile`), with its path as Lidarr sees it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackFile {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportList {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    /// Adds what the list finds without asking.
    #[serde(default)]
    pub enable_automatic_add: bool,
    /// Older field name for the same switch.
    #[serde(default)]
    pub enable: Option<bool>,
}

impl ImportList {
    pub fn adds_artists(&self) -> bool {
        self.enable_automatic_add || self.enable == Some(true)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size: Option<i64>,
}

/// `GET /filesystem`: one folder's subfolders and files, as Lidarr sees it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    #[serde(default)]
    pub directories: Vec<FsEntry>,
    #[serde(default)]
    pub files: Vec<FsEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataProfile {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumStatistics {
    #[serde(default)]
    pub track_file_count: i64,
    #[serde(default)]
    pub track_count: i64,
    #[serde(default)]
    pub percent_of_tracks: f64,
}

/// An album in Lidarr, or one a lookup found (`id` 0: not in Lidarr).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    /// The MusicBrainz release group id.
    #[serde(default)]
    pub foreign_album_id: String,
    #[serde(default)]
    pub artist_id: i64,
    #[serde(default)]
    pub monitored: bool,
    #[serde(default)]
    pub statistics: Option<AlbumStatistics>,
    #[serde(default)]
    pub artist: Option<Artist>,
}

/// One download in Lidarr's queue.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    #[serde(default)]
    pub album_id: Option<i64>,
    /// `queued`, `downloading`, `paused`, `completed`, `failed`, `warning`...
    #[serde(default)]
    pub status: String,
    /// `downloading`, `importPending`, `importing`, `imported`, `failedPending`, `failed`...
    #[serde(default)]
    pub tracked_download_state: Option<String>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub sizeleft: Option<f64>,
}

/// One event in Lidarr's history.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    #[serde(default)]
    pub album_id: Option<i64>,
    /// `grabbed`, `downloadFailed`, `trackFileImported`, `downloadImported`,
    /// `albumImportIncomplete`, `downloadIgnored`...
    #[serde(default)]
    pub event_type: String,
    #[serde(default)]
    pub date: String,
}

/// One of Lidarr's Connect entries (`GET /notification`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    /// `PlexServer` for Plex Media Server.
    #[serde(default)]
    pub implementation: String,
}

/// A list Lidarr pages (`records`) or returns whole.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Paged<T> {
    Page {
        #[serde(default = "Vec::new")]
        records: Vec<T>,
    },
    All(Vec<T>),
}

impl<T> Paged<T> {
    fn into_vec(self) -> Vec<T> {
        match self {
            Paged::Page { records } | Paged::All(records) => records,
        }
    }
}

/// Which way a request reached Lidarr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Route {
    /// The home (LAN) address.
    Home,
    /// The away address (a reverse proxy, tunnel or VPN the user set up).
    Away,
}

/// How GETs reach Lidarr. Only GETs exist at this layer, so nothing built
/// on it can change Lidarr. The direct route is below; another route (a
/// paired home device) can be added behind the same trait.
#[async_trait]
pub trait LidarrGet: Send + Sync {
    /// GET `/api/v1{path}` with `query`. A network failure is an error; any
    /// HTTP answer is returned as is.
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> AppResult<HttpResponse>;
    fn route(&self) -> Route;
}

/// Header names the user adds for a proxy, kept for the life of the app
/// (`WebRequest` takes static names; there are only ever a handful).
fn header_name(name: &str) -> &'static str {
    static NAMES: std::sync::OnceLock<parking_lot::Mutex<std::collections::HashSet<&'static str>>> = std::sync::OnceLock::new();
    let mut set = NAMES.get_or_init(Default::default).lock();
    if let Some(n) = set.get(name) {
        return n;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    set.insert(leaked);
    leaked
}

/// Straight to an address, with the key and any extra proxy headers in
/// headers flagged sensitive.
pub struct DirectRoute {
    web: Arc<dyn Web>,
    base: String,
    key: Token,
    headers: Vec<(String, Token)>,
    route: Route,
    timeout: Duration,
}

impl DirectRoute {
    /// `base` is an address from Settings (already checked by the settings
    /// patch), with any URL base Lidarr runs under.
    pub fn new(web: Arc<dyn Web>, base: &str, key: Token, headers: Vec<(String, Token)>, route: Route) -> AppResult<Self> {
        let u = url::Url::parse(base.trim()).map_err(|_| AppError::Invalid("Lidarr's address isn't a web address.".into()))?;
        if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
            return Err(AppError::Invalid("Lidarr's address starts with http:// or https://.".into()));
        }
        Ok(Self { web, base: base.trim().trim_end_matches('/').to_string(), key, headers, route, timeout: Duration::from_secs(20) })
    }

    /// Give up sooner (the home address when away from home).
    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    /// One of the fixed writes (see `write`), with the same key and headers.
    pub(super) async fn send_write(&self, call: &super::write::WriteCall) -> AppResult<HttpResponse> {
        use super::write::WriteMethod;
        let url = format!("{}{API}{}", self.base, call.path());
        let mut req = match call.method() {
            WriteMethod::Post => WebRequest::post_json(url, call.body()),
            WriteMethod::Put => WebRequest::put_json(url, call.body()),
        }
        .timeout(self.timeout.max(Duration::from_secs(20)));
        for (k, v) in &self.headers {
            req = req.header(header_name(k), v.expose());
        }
        req = req.header("X-Api-Key", self.key.expose());
        self.web.send(req).await
    }
}

#[async_trait]
impl LidarrGet for DirectRoute {
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> AppResult<HttpResponse> {
        let mut url = url::Url::parse(&format!("{}{API}{path}", self.base))
            .map_err(|_| AppError::Invalid("Lidarr's address isn't a web address.".into()))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter());
        }
        let mut req = WebRequest::get(url.to_string()).timeout(self.timeout);
        for (k, v) in &self.headers {
            req = req.header(header_name(k), v.expose());
        }
        req = req.header("X-Api-Key", self.key.expose());
        self.web.send(req).await
    }

    fn route(&self) -> Route {
        self.route
    }
}

/// A client for one Lidarr over one route, GET only.
#[derive(Clone)]
pub struct LidarrClient {
    transport: Arc<dyn LidarrGet>,
}

impl std::fmt::Debug for LidarrClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LidarrClient({:?})", self.transport.route())
    }
}

impl LidarrClient {
    pub fn new(transport: Arc<dyn LidarrGet>) -> Self {
        Self { transport }
    }

    pub fn route(&self) -> Route {
        self.transport.route()
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> AppResult<T> {
        let resp = self.transport.get(path, query).await.map_err(|_| AppError::Network(UNREACHABLE.into()))?;
        match resp.status {
            200..=299 => {}
            401 | 403 => return Err(AppError::Auth(BAD_KEY.into())),
            404 => return Err(AppError::NotFound(NOT_LIDARR.into())),
            s => return Err(AppError::Server(format!("Lidarr answered with an error ({s}). Try again in a moment."))),
        }
        serde_json::from_slice(&resp.body).map_err(|_| AppError::Decode(NOT_LIDARR.into()))
    }

    pub async fn system_status(&self) -> AppResult<SystemStatus> {
        self.get("/system/status", &[]).await
    }

    pub async fn root_folders(&self) -> AppResult<Vec<RootFolder>> {
        self.get("/rootfolder", &[]).await
    }

    pub async fn artists(&self) -> AppResult<Vec<Artist>> {
        self.get("/artist", &[]).await
    }

    pub async fn naming(&self) -> AppResult<NamingConfig> {
        self.get("/config/naming", &[]).await
    }

    pub async fn metadata_provider(&self) -> AppResult<MetadataProviderConfig> {
        self.get("/config/metadataprovider", &[]).await
    }

    pub async fn media_management(&self) -> AppResult<MediaManagementConfig> {
        self.get("/config/mediamanagement", &[]).await
    }

    pub async fn quality_profiles(&self) -> AppResult<Vec<QualityProfile>> {
        self.get("/qualityprofile", &[]).await
    }

    pub async fn import_lists(&self) -> AppResult<Vec<ImportList>> {
        self.get("/importlist", &[]).await
    }

    pub async fn artist(&self, id: i64) -> AppResult<Artist> {
        self.get(&format!("/artist/{id}"), &[]).await
    }

    pub async fn metadata_profiles(&self) -> AppResult<Vec<MetadataProfile>> {
        self.get("/metadataprofile", &[]).await
    }

    /// An artist's albums as Lidarr has them.
    pub async fn albums_of(&self, artist_id: i64) -> AppResult<Vec<Album>> {
        self.get("/album", &[("artistId", &artist_id.to_string())]).await
    }

    /// Every album Lidarr has, for every artist.
    pub async fn all_albums(&self) -> AppResult<Vec<Album>> {
        self.get("/album", &[]).await
    }

    /// The album with this MusicBrainz release group id, when Lidarr has it.
    pub async fn album_by_release_group(&self, rgid: &str) -> AppResult<Option<Album>> {
        let all: Vec<Album> = self.get("/album", &[("foreignAlbumId", rgid)]).await?;
        Ok(all.into_iter().find(|a| a.id > 0 && a.foreign_album_id.eq_ignore_ascii_case(rgid)))
    }

    /// Lidarr's metadata lookup for a release group. Needs Lidarr's own
    /// metadata server, so its failures get their own message.
    pub async fn album_lookup(&self, rgid: &str) -> AppResult<Vec<Album>> {
        let term = format!("lidarr:{rgid}");
        let resp = self.transport.get("/album/lookup", &[("term", term.as_str())]).await.map_err(|_| AppError::Network(UNREACHABLE.into()))?;
        match resp.status {
            200..=299 => serde_json::from_slice(&resp.body).map_err(|_| AppError::Decode(NOT_LIDARR.into())),
            401 | 403 => Err(AppError::Auth(BAD_KEY.into())),
            404 => Ok(Vec::new()),
            _ => Err(AppError::Server(METADATA_DOWN.into())),
        }
    }

    /// Downloads in Lidarr's queue for these albums.
    pub async fn queue(&self, album_ids: &[i64]) -> AppResult<Vec<QueueItem>> {
        let ids = album_ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let p: Paged<QueueItem> = self.get("/queue/details", &[("albumIds", ids.as_str())]).await?;
        Ok(p.into_vec())
    }

    /// The newest history events for one album.
    pub async fn history(&self, album_id: i64) -> AppResult<Vec<HistoryItem>> {
        let id = album_id.to_string();
        let p: Paged<HistoryItem> = self
            .get("/history", &[("albumId", id.as_str()), ("page", "1"), ("pageSize", "20"), ("sortKey", "date"), ("sortDirection", "descending")])
            .await?;
        Ok(p.into_vec())
    }

    /// An album's tracks, with the files Lidarr matched to them.
    pub async fn tracks(&self, album_id: i64) -> AppResult<Vec<LidarrTrack>> {
        self.get("/track", &[("albumId", album_id.to_string().as_str())]).await
    }

    /// An album's imported files.
    pub async fn track_files(&self, album_id: i64) -> AppResult<Vec<TrackFile>> {
        self.get("/trackfile", &[("albumId", album_id.to_string().as_str())]).await
    }

    /// Artists Lidarr's import lists exclude.
    pub async fn exclusions(&self) -> AppResult<Vec<Exclusion>> {
        self.get("/importlistexclusion", &[]).await
    }

    /// Files Lidarr found in its root folders and couldn't match to an album.
    pub async fn unmapped_files(&self) -> AppResult<Vec<TrackFile>> {
        self.get("/trackfile", &[("unmapped", "true")]).await
    }

    /// Lidarr's Connect entries.
    pub async fn notifications(&self) -> AppResult<Vec<Notification>> {
        self.get("/notification", &[]).await
    }

    /// One folder's contents, with file sizes.
    pub async fn list_folder(&self, path: &str) -> AppResult<Listing> {
        self.get("/filesystem", &[("path", path), ("includeFiles", "true"), ("allowFoldersWithoutTrailingSlashes", "true")])
            .await
    }
}

/// `"2.8.2.4493"` → (2, 8, 2). Missing parts count as 0.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split(['.', '-', '+']).map(|p| p.parse::<u64>());
    let major = it.next()?.ok()?;
    let minor = it.next().and_then(Result::ok).unwrap_or(0);
    let patch = it.next().and_then(Result::ok).unwrap_or(0);
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("2.8.2.4493"), Some((2, 8, 2)));
        assert_eq!(parse_version("1.0"), Some((1, 0, 0)));
        assert_eq!(parse_version("v3"), Some((3, 0, 0)));
        assert_eq!(parse_version("x"), None);
    }

    #[test]
    fn shapes_parse_leniently() {
        let r: Vec<RootFolder> = serde_json::from_value(serde_json::json!([
            {"id": 1, "path": "/music", "unmappedFolders": [{"name": "Band", "path": "/music/Band"}], "freeSpace": 1}
        ]))
        .unwrap();
        assert!(r[0].accessible);
        assert_eq!(r[0].unmapped_folders[0].name, "Band");
        let l: ImportList = serde_json::from_value(serde_json::json!({"name": "x", "enableAutomaticAdd": true})).unwrap();
        assert!(l.adds_artists());
        let fs: Listing = serde_json::from_value(serde_json::json!({
            "parent": "/", "directories": [{"type": "folder", "name": "A", "path": "/music/A/"}],
            "files": [{"type": "file", "name": "01.flac", "path": "/music/A/01.flac", "size": 12}]
        }))
        .unwrap();
        assert_eq!(fs.files[0].size, Some(12));
    }

    #[test]
    fn debug_hides_the_key() {
        let web: Arc<dyn Web> = Arc::new(crate::web::fake::FakeWeb::new());
        let r = DirectRoute::new(web, "http://nas:8686", Token::new("secret-key").unwrap(), vec![], Route::Home).unwrap();
        let c = LidarrClient::new(Arc::new(r));
        assert!(!format!("{c:?}").contains("secret-key"));
    }
}
