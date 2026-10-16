//! The only things Platter ever asks Lidarr to do, kept apart from the
//! read-only client. There are exactly three writes, each a fixed method
//! and path:
//!
//! - `POST /api/v1/artist`: add an artist with nothing monitored
//! - `PUT /api/v1/album/monitor`: turn monitoring on or off for albums
//! - `POST /api/v1/command`: an `AlbumSearch` for albums
//!
//! (The fourth call a request makes, the album lookup, is a read.) No other
//! method or path can be built here, so nothing built on this can rename,
//! retag, rescan or delete anything.

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

/// One write. The method and path come from the variant.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteCall {
    AddArtist(AddArtist),
    Monitor { album_ids: Vec<i64>, monitored: bool },
    AlbumSearch { album_ids: Vec<i64> },
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
}

impl WriteCall {
    pub fn method(&self) -> WriteMethod {
        match self {
            WriteCall::AddArtist(_) | WriteCall::AlbumSearch { .. } => WriteMethod::Post,
            WriteCall::Monitor { .. } => WriteMethod::Put,
        }
    }

    /// The path under `/api/v1`.
    pub fn path(&self) -> &'static str {
        match self {
            WriteCall::AddArtist(_) => "/artist",
            WriteCall::Monitor { .. } => "/album/monitor",
            WriteCall::AlbumSearch { .. } => "/command",
        }
    }

    pub fn body(&self) -> String {
        let v = match self {
            WriteCall::AddArtist(a) => serde_json::to_string(a),
            WriteCall::Monitor { album_ids, monitored } => serde_json::to_string(&MonitorBody { album_ids, monitored: *monitored }),
            WriteCall::AlbumSearch { album_ids } => serde_json::to_string(&CommandBody { name: "AlbumSearch", album_ids }),
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

/// The three writes with their answers checked.
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
        assert_eq!((m.method(), m.path()), (WriteMethod::Put, "/album/monitor"));
        assert_eq!(serde_json::from_str::<Value>(&m.body()).unwrap(), json!({"albumIds": [7], "monitored": true}));
        let s = WriteCall::AlbumSearch { album_ids: vec![7] };
        assert_eq!((s.method(), s.path()), (WriteMethod::Post, "/command"));
        assert_eq!(serde_json::from_str::<Value>(&s.body()).unwrap(), json!({"name": "AlbumSearch", "albumIds": [7]}));
    }
}
