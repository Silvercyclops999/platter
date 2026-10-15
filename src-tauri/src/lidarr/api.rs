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
