//! `PlexClient`: typed access to one Plex Media Server over the connection
//! the race picked. Shapes follow docs/PLEX-API.md (2026-10-13 findings).
//!
//! The token is sent only as the `X-Plex-Token` header (marked sensitive);
//! it never appears in a URL this module builds, in an error message, or in
//! `Debug` output.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use reqwest::header::{HeaderMap, HeaderName};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use super::identity::ClientIdentity;
use super::lenient;
use super::token::Token;
use super::types::ConnectionKind;
use crate::error::{AppError, AppResult};
use crate::model::{Album, Artist, Playlist, ReleaseSection, Track};

/// Page size used for full-library listing (docs/PLEX-API.md).
pub const PAGE_SIZE: u64 = 500;
const LIBRARY_IDENTIFIER: &str = "com.plexapp.plugins.library";

// ---------------------------------------------------------------------------
// Error mapping (shared with auth.rs)

/// Map a transport error. Uses `without_url` so no URL ends up in messages.
pub fn map_send_error(e: reqwest::Error) -> AppError {
    let e = e.without_url();
    if e.is_timeout() {
        AppError::Network("The request timed out.".into())
    } else if e.is_connect() {
        AppError::Network("Couldn't connect.".into())
    } else if e.is_decode() {
        AppError::Decode(format!("unexpected response: {e}"))
    } else if e.is_status() {
        status_error(e.status().unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), "")
    } else {
        AppError::Network(e.to_string())
    }
}

/// Map an HTTP status to `AppError`. `what` is a path without query.
pub fn status_error(status: StatusCode, what: &str) -> AppError {
    let code = status.as_u16();
    match code {
        401 | 403 => AppError::Auth("Plex rejected the sign-in. Please sign in again.".into()),
        404 => AppError::NotFound(if what.is_empty() {
            "not found".into()
        } else {
            what.to_string()
        }),
        503 => AppError::Server("Server is starting or unavailable (503).".into()),
        500..=599 => AppError::Server(format!("Server error {code}.")),
        _ => AppError::Invalid(format!("Unexpected response {code}.")),
    }
}

/// Pass a successful response through, map anything else.
pub async fn check_status(resp: Response) -> AppResult<Response> {
    let status = resp.status();
    if status.is_success() {
        Ok(resp)
    } else {
        Err(status_error(status, resp.url().path()))
    }
}

// ---------------------------------------------------------------------------
// Raw Plex shapes

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    #[serde(rename = "MediaContainer")]
    media_container: T,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub size: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub total_size: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub offset: Option<i64>,
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<PlexMetadata>,
    #[serde(default, rename = "Directory")]
    pub directory: Vec<Directory>,
    #[serde(default, rename = "Hub")]
    pub hub: Vec<PlexHub>,
    #[serde(default, rename = "Setting")]
    pub setting: Vec<Setting>,
    // server root `/` and `/identity`
    #[serde(default)]
    pub friendly_name: Option<String>,
    #[serde(default)]
    pub machine_identifier: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    pub version: Option<String>,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub my_plex_subscription: bool,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub music_analysis: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Tag {
    #[serde(default)]
    pub tag: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UltraBlurColors {
    #[serde(default)]
    pub top_left: Option<String>,
    #[serde(default)]
    pub top_right: Option<String>,
    #[serde(default)]
    pub bottom_left: Option<String>,
    #[serde(default)]
    pub bottom_right: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexStream {
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub id: Option<i64>,
    /// 2 = audio.
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub stream_type: Option<i64>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub bit_depth: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub sampling_rate: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub channels: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_f64")]
    pub gain: Option<f64>,
    #[serde(default, deserialize_with = "lenient::opt_f64")]
    pub album_gain: Option<f64>,
    #[serde(default, deserialize_with = "lenient::opt_f64")]
    pub peak: Option<f64>,
    #[serde(default, deserialize_with = "lenient::opt_f64")]
    pub album_peak: Option<f64>,
    #[serde(default, deserialize_with = "lenient::opt_f64")]
    pub loudness: Option<f64>,
    #[serde(default, deserialize_with = "lenient::opt_f64")]
    pub lra: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexPart {
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub id: Option<i64>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub duration: Option<i64>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub size: Option<i64>,
    #[serde(default)]
    pub container: Option<String>,
    /// Only sent with `checkFiles=1`: the file is on disk.
    #[serde(default, deserialize_with = "lenient::opt_bool")]
    pub exists: Option<bool>,
    /// Only sent with `checkFiles=1`: the server can read the file.
    #[serde(default, deserialize_with = "lenient::opt_bool")]
    pub accessible: Option<bool>,
    #[serde(default, rename = "Stream")]
    pub streams: Vec<PlexStream>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexMedia {
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub id: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub duration: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub bitrate: Option<i64>,
    #[serde(default)]
    pub audio_codec: Option<String>,
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub audio_channels: Option<i64>,
    #[serde(default, rename = "Part")]
    pub parts: Vec<PlexPart>,
}

/// One `Metadata` item (artist, album, track, playlist, history row, ...).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexMetadata {
    #[serde(default, deserialize_with = "lenient::string")]
    pub rating_key: String,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub title_sort: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    pub parent_rating_key: Option<String>,
    #[serde(default)]
    pub parent_title: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    pub grandparent_rating_key: Option<String>,
    #[serde(default)]
    pub grandparent_title: Option<String>,
    #[serde(default)]
    pub original_title: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub index: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub parent_index: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub year: Option<i64>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub art: Option<String>,
    #[serde(default)]
    pub parent_thumb: Option<String>,
    #[serde(default)]
    pub grandparent_thumb: Option<String>,
    #[serde(default)]
    pub composite: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub duration: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub added_at: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub updated_at: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub view_count: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub skip_count: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub last_viewed_at: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_f64")]
    pub user_rating: Option<f64>,
    /// Global listener count on tracks (from Last.fm), when Plex has one.
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub rating_count: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub leaf_count: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub child_count: Option<i64>,
    #[serde(default)]
    pub studio: Option<String>,
    #[serde(default)]
    pub originally_available_at: Option<String>,
    #[serde(
        default,
        deserialize_with = "lenient::opt_string",
        rename = "librarySectionID"
    )]
    pub library_section_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub smart: bool,
    #[serde(default)]
    pub playlist_type: Option<String>,
    // history rows
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub viewed_at: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64", rename = "accountID")]
    pub account_id: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64", rename = "deviceID")]
    pub device_id: Option<i64>,
    // playlists items
    #[serde(
        default,
        deserialize_with = "lenient::opt_i64",
        rename = "playlistItemID"
    )]
    pub playlist_item_id: Option<i64>,
    #[serde(default, rename = "Format")]
    pub format: Vec<Tag>,
    #[serde(default, rename = "Subformat")]
    pub subformat: Vec<Tag>,
    #[serde(default, rename = "Genre")]
    pub genre: Vec<Tag>,
    #[serde(default, rename = "Style")]
    pub style: Vec<Tag>,
    #[serde(default, rename = "Mood")]
    pub mood: Vec<Tag>,
    #[serde(default, rename = "UltraBlurColors")]
    pub ultra_blur_colors: Option<UltraBlurColors>,
    #[serde(default, rename = "Media")]
    pub media: Vec<PlexMedia>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Directory {
    #[serde(default, deserialize_with = "lenient::string")]
    pub key: String,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub scanner: Option<String>,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub scanned_at: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub updated_at: Option<i64>,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub refreshing: bool,
    #[serde(default)]
    pub fast_key: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexHub {
    #[serde(default)]
    pub hub_identifier: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub size: Option<i64>,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub more: bool,
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<PlexMetadata>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Setting {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub value: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Typed results

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaType {
    Artist = 8,
    Album = 9,
    Track = 10,
}

impl MetaType {
    pub fn code(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerIdentity {
    pub machine_identifier: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRoot {
    pub friendly_name: Option<String>,
    pub machine_identifier: Option<String>,
    pub version: Option<String>,
    pub my_plex_subscription: bool,
    pub music_analysis: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibrarySection {
    pub id: String,
    pub title: String,
    /// Plex section type; music is "artist".
    pub kind: String,
    pub uuid: Option<String>,
    pub agent: Option<String>,
    pub scanner: Option<String>,
    pub scanned_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub refreshing: bool,
}

impl LibrarySection {
    pub fn is_music(&self) -> bool {
        self.kind == "artist"
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionPrefs {
    /// "Prefer local metadata".
    pub respect_tags: Option<bool>,
    /// Sonic analysis on this library.
    pub music_analysis: Option<bool>,
    /// Every pref id → value, for checks added later.
    pub all: Vec<(String, serde_json::Value)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagValue {
    /// Filter key to pass as `?format=` / `?subformat=`.
    pub key: String,
    pub title: String,
}

/// Bit depth, sample rate and loudness, only present in track detail.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioStreamInfo {
    pub codec: Option<String>,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub gain: Option<f64>,
    pub album_gain: Option<f64>,
    pub peak: Option<f64>,
    pub album_peak: Option<f64>,
    pub loudness: Option<f64>,
    pub lra: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub rating_key: String,
    pub title: String,
    pub artist_title: Option<String>,
    /// Unix seconds.
    pub viewed_at: i64,
    pub account_id: Option<i64>,
    pub device_id: Option<i64>,
}

/// A URL plus the headers needed to fetch it. The token is only in
/// `headers` (marked sensitive), never in `url`.
#[derive(Clone)]
pub struct StreamRequest {
    pub url: Url,
    pub headers: HeaderMap,
}

impl std::fmt::Debug for StreamRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.keys().map(HeaderName::as_str).collect();
        f.debug_struct("StreamRequest")
            .field("url", &self.url.as_str())
            .field("headers", &names)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Buffering,
}

impl PlaybackState {
    fn as_str(self) -> &'static str {
        match self {
            PlaybackState::Playing => "playing",
            PlaybackState::Paused => "paused",
            PlaybackState::Stopped => "stopped",
            PlaybackState::Buffering => "buffering",
        }
    }
}

/// `/:/timeline` report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineReport {
    pub rating_key: String,
    pub state: PlaybackState,
    pub time_ms: u64,
    pub duration_ms: u64,
    pub play_queue_item_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Release type mapping

/// Map Plex `Format` / `Subformat` tags (any case) to an artist-page section
/// (docs/TAGGING.md). One mapping for the whole app lives in `db::release`.
pub fn release_section(format: Option<&str>, subformats: &[String]) -> ReleaseSection {
    crate::db::release_section(format, subformats)
}

// ---------------------------------------------------------------------------
// Mapping to model.rs

fn non_empty(s: &Option<String>) -> Option<String> {
    s.as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn u32_of(v: Option<i64>) -> u32 {
    v.and_then(|n| u32::try_from(n).ok()).unwrap_or(0)
}

fn opt_u32(v: Option<i64>) -> Option<u32> {
    v.and_then(|n| u32::try_from(n).ok())
}

impl PlexMetadata {
    pub fn to_artist(&self) -> Artist {
        Artist {
            id: self.rating_key.clone(),
            title: self.title.clone(),
            sort_title: non_empty(&self.title_sort).unwrap_or_else(|| self.title.clone()),
            thumb: non_empty(&self.thumb),
            art: non_empty(&self.art),
            view_count: u32_of(self.view_count),
        }
    }

    pub fn format_tag(&self) -> Option<String> {
        self.format
            .first()
            .map(|t| t.tag.trim().to_lowercase())
            .filter(|t| !t.is_empty())
    }

    pub fn subformat_tags(&self) -> Vec<String> {
        self.subformat
            .iter()
            .map(|t| t.tag.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    }

    pub fn to_album(&self) -> Album {
        let format = self.format_tag();
        let subformats = self.subformat_tags();
        let section = release_section(format.as_deref(), &subformats);
        Album {
            id: self.rating_key.clone(),
            title: self.title.clone(),
            sort_title: non_empty(&self.title_sort).unwrap_or_else(|| self.title.clone()),
            artist_id: self.parent_rating_key.clone().unwrap_or_default(),
            artist_title: self.parent_title.clone().unwrap_or_default(),
            year: self.year.and_then(|y| i32::try_from(y).ok()),
            thumb: non_empty(&self.thumb),
            format,
            subformats,
            section,
            genres: self.genre.iter().map(|g| g.tag.clone()).collect(),
            track_count: u32_of(self.leaf_count),
            added_at: self.added_at.unwrap_or(0),
            view_count: u32_of(self.view_count),
            last_viewed_at: self.last_viewed_at,
            tint: self
                .ultra_blur_colors
                .as_ref()
                .and_then(|c| non_empty(&c.top_left)),
        }
    }

    pub fn audio_stream(&self) -> Option<AudioStreamInfo> {
        let s = self
            .media
            .first()?
            .parts
            .first()?
            .streams
            .iter()
            .find(|s| s.stream_type == Some(2))?;
        Some(AudioStreamInfo {
            codec: s.codec.clone(),
            bit_depth: opt_u32(s.bit_depth),
            sample_rate: opt_u32(s.sampling_rate),
            channels: opt_u32(s.channels),
            gain: s.gain,
            album_gain: s.album_gain,
            peak: s.peak,
            album_peak: s.album_peak,
            loudness: s.loudness,
            lra: s.lra,
        })
    }

    pub fn to_track(&self) -> Track {
        let media = self.media.first();
        let part = media.and_then(|m| m.parts.first());
        let stream = self.audio_stream();
        Track {
            id: self.rating_key.clone(),
            title: self.title.clone(),
            album_id: self.parent_rating_key.clone().unwrap_or_default(),
            album_title: self.parent_title.clone().unwrap_or_default(),
            artist_id: self.grandparent_rating_key.clone().unwrap_or_default(),
            artist_title: self.grandparent_title.clone().unwrap_or_default(),
            track_artist: non_empty(&self.original_title),
            track_number: u32_of(self.index),
            disc_number: opt_u32(self.parent_index).unwrap_or(1),
            duration_ms: self
                .duration
                .or(media.and_then(|m| m.duration))
                .and_then(|d| u64::try_from(d).ok())
                .unwrap_or(0),
            thumb: non_empty(&self.parent_thumb).or_else(|| non_empty(&self.thumb)),
            part_key: part.and_then(|p| p.key.clone()).unwrap_or_default(),
            codec: media.and_then(|m| m.audio_codec.clone()),
            container: media.and_then(|m| m.container.clone()),
            bitrate: media.and_then(|m| opt_u32(m.bitrate)),
            bit_depth: stream.as_ref().and_then(|s| s.bit_depth),
            sample_rate: stream.as_ref().and_then(|s| s.sample_rate),
            user_rating: self.user_rating.map(|r| r as f32),
            view_count: u32_of(self.view_count),
            last_viewed_at: self.last_viewed_at,
            rating_count: self.rating_count.and_then(|n| u32::try_from(n).ok()),
        }
    }

    pub fn to_playlist(&self) -> Playlist {
        Playlist {
            id: self.rating_key.clone(),
            title: self.title.clone(),
            smart: self.smart,
            track_count: u32_of(self.leaf_count),
            duration_ms: self
                .duration
                .and_then(|d| u64::try_from(d).ok())
                .unwrap_or(0),
            thumb: non_empty(&self.composite).or_else(|| non_empty(&self.thumb)),
        }
    }

    pub fn to_history(&self) -> HistoryEntry {
        HistoryEntry {
            rating_key: self.rating_key.clone(),
            title: self.title.clone(),
            artist_title: non_empty(&self.grandparent_title),
            viewed_at: self.viewed_at.unwrap_or(0),
            account_id: self.account_id,
            device_id: self.device_id,
        }
    }
}

fn total_of(c: &Container, fallback: usize) -> u64 {
    c.total_size
        .or(c.size)
        .and_then(|n| u64::try_from(n).ok())
        .unwrap_or(fallback as u64)
}

/// Keep only non-empty music hubs from a `/hubs/search` response.
pub fn music_hubs(hubs: Vec<PlexHub>) -> Vec<PlexHub> {
    hubs.into_iter()
        .filter(|h| {
            let t = h.kind.as_deref().unwrap_or(h.hub_identifier.as_str());
            matches!(t, "artist" | "album" | "track" | "playlist") && !h.metadata.is_empty()
        })
        .collect()
}

fn prefs_bool(v: &serde_json::Value) -> Option<bool> {
    match v {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::Number(n) => n.as_i64().map(|n| n != 0),
        serde_json::Value::String(s) => match s.trim() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Client

#[derive(Debug, Clone)]
pub struct PlexClient {
    http: reqwest::Client,
    base: Url,
    token: Token,
    identity: Arc<ClientIdentity>,
    connection: Option<ConnectionKind>,
}

impl PlexClient {
    pub fn new(
        base: &str,
        token: Token,
        identity: Arc<ClientIdentity>,
        connection: Option<ConnectionKind>,
    ) -> AppResult<Self> {
        let base = Url::parse(base.trim_end_matches('/'))
            .map_err(|e| AppError::Invalid(format!("server url: {e}")))?;
        let mut headers = identity.headers()?;
        headers.insert("x-plex-token", token.header_value()?);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
        Ok(PlexClient {
            http,
            base,
            token,
            identity,
            connection,
        })
    }

    pub fn base_url(&self) -> &Url {
        &self.base
    }

    pub fn connection(&self) -> Option<ConnectionKind> {
        self.connection
    }

    pub fn identity(&self) -> &Arc<ClientIdentity> {
        &self.identity
    }

    /// Headers for a request made outside this client (audio engine,
    /// websocket): identity + sensitive token.
    pub fn auth_headers(&self) -> AppResult<HeaderMap> {
        let mut h = self.identity.headers()?;
        h.insert("x-plex-token", self.token.header_value()?);
        Ok(h)
    }

    /// Absolute URL for a server path (no token).
    pub fn url(&self, path: &str) -> AppResult<Url> {
        let mut u = self.base.clone();
        let (p, q) = match path.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (path, None),
        };
        let joined = format!("{}{}", self.base.path().trim_end_matches('/'), p);
        u.set_path(&joined);
        u.set_query(q);
        Ok(u)
    }

    fn request(&self, method: Method, path: &str) -> AppResult<RequestBuilder> {
        Ok(self.http.request(method, self.url(path)?))
    }

    async fn send(&self, rb: RequestBuilder) -> AppResult<Response> {
        let resp = rb.send().await.map_err(map_send_error)?;
        check_status(resp).await
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        paging: Option<(u64, u64)>,
    ) -> AppResult<T> {
        let mut rb = self.request(Method::GET, path)?.query(query);
        if let Some((start, size)) = paging {
            rb = rb
                .header("x-plex-container-start", start.to_string())
                .header("x-plex-container-size", size.to_string());
        }
        let resp = self.send(rb).await?;
        let bytes = resp.bytes().await.map_err(map_send_error)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| AppError::Decode(format!("{}: {e}", path.split('?').next().unwrap_or(""))))
    }

    async fn container(
        &self,
        path: &str,
        query: &[(&str, String)],
        paging: Option<(u64, u64)>,
    ) -> AppResult<Container> {
        let env: Envelope<Container> = self.get_json(path, query, paging).await?;
        Ok(env.media_container)
    }

    // ---- server -----------------------------------------------------------

    /// `GET /identity` (no auth needed by Plex, sent anyway).
    pub async fn identity_info(&self) -> AppResult<ServerIdentity> {
        let c = self.container("/identity", &[], None).await?;
        Ok(ServerIdentity {
            machine_identifier: c.machine_identifier.unwrap_or_default(),
            version: c.version,
        })
    }

    /// `GET /` — version, Plex Pass, server-level sonic analysis. Requires a
    /// valid token (401 when revoked).
    pub async fn root(&self) -> AppResult<ServerRoot> {
        let c = self.container("/", &[], None).await?;
        Ok(ServerRoot {
            friendly_name: c.friendly_name,
            machine_identifier: c.machine_identifier,
            version: c.version,
            my_plex_subscription: c.my_plex_subscription,
            music_analysis: c.music_analysis,
        })
    }

    // ---- sections ---------------------------------------------------------

    pub async fn sections(&self) -> AppResult<Vec<LibrarySection>> {
        let c = self.container("/library/sections", &[], None).await?;
        Ok(c.directory
            .into_iter()
            .map(|d| LibrarySection {
                id: d.key,
                title: d.title,
                kind: d.kind.unwrap_or_default(),
                uuid: d.uuid,
                agent: d.agent,
                scanner: d.scanner,
                scanned_at: d.scanned_at,
                updated_at: d.updated_at,
                refreshing: d.refreshing,
            })
            .collect())
    }

    pub async fn music_sections(&self) -> AppResult<Vec<LibrarySection>> {
        Ok(self
            .sections()
            .await?
            .into_iter()
            .filter(LibrarySection::is_music)
            .collect())
    }

    /// `GET /library/sections/{id}/prefs`
    pub async fn section_prefs(&self, section: &str) -> AppResult<SectionPrefs> {
        let c = self
            .container(&format!("/library/sections/{section}/prefs"), &[], None)
            .await?;
        let find = |id: &str| {
            c.setting
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| prefs_bool(&s.value))
        };
        Ok(SectionPrefs {
            respect_tags: find("respectTags"),
            music_analysis: find("musicAnalysis"),
            all: c
                .setting
                .iter()
                .map(|s| (s.id.clone(), s.value.clone()))
                .collect(),
        })
    }

    // ---- listing ----------------------------------------------------------

    /// Total items of a type (start=0, size=0 headers → only totalSize).
    pub async fn count(&self, section: &str, ty: MetaType) -> AppResult<u64> {
        let c = self
            .container(
                &format!("/library/sections/{section}/all"),
                &[("type", ty.code().to_string())],
                Some((0, 0)),
            )
            .await?;
        Ok(total_of(&c, 0))
    }

    /// One page of `/library/sections/{id}/all?type=N&<filters>`, paged with
    /// `X-Plex-Container-Start`/`-Size` headers.
    pub async fn list(
        &self,
        section: &str,
        ty: MetaType,
        start: u64,
        size: u64,
        filters: &[(&str, String)],
    ) -> AppResult<Page<PlexMetadata>> {
        let mut q: Vec<(&str, String)> = vec![("type", ty.code().to_string())];
        q.extend(filters.iter().cloned());
        let c = self
            .container(
                &format!("/library/sections/{section}/all"),
                &q,
                Some((start, size)),
            )
            .await?;
        let total = total_of(&c, c.metadata.len());
        Ok(Page {
            items: c.metadata,
            total,
        })
    }

    /// Every item, `PAGE_SIZE` per request.
    pub async fn list_all(
        &self,
        section: &str,
        ty: MetaType,
        filters: &[(&str, String)],
    ) -> AppResult<Vec<PlexMetadata>> {
        let mut out = Vec::new();
        let mut start = 0;
        loop {
            let page = self.list(section, ty, start, PAGE_SIZE, filters).await?;
            let n = page.items.len() as u64;
            out.extend(page.items);
            start += n;
            if n == 0 || start >= page.total {
                break;
            }
        }
        Ok(out)
    }

    pub async fn artists(&self, section: &str, start: u64, size: u64) -> AppResult<Page<Artist>> {
        let p = self
            .list(section, MetaType::Artist, start, size, &[])
            .await?;
        Ok(Page {
            items: p.items.iter().map(PlexMetadata::to_artist).collect(),
            total: p.total,
        })
    }

    /// Note: list items carry no `Format`, so `section` is `Albums` here;
    /// tag release types with [`Self::tag_values`] + [`Self::list`] filters.
    pub async fn albums(&self, section: &str, start: u64, size: u64) -> AppResult<Page<Album>> {
        let p = self
            .list(section, MetaType::Album, start, size, &[])
            .await?;
        Ok(Page {
            items: p.items.iter().map(PlexMetadata::to_album).collect(),
            total: p.total,
        })
    }

    pub async fn tracks(&self, section: &str, start: u64, size: u64) -> AppResult<Page<Track>> {
        let p = self
            .list(section, MetaType::Track, start, size, &[])
            .await?;
        Ok(Page {
            items: p.items.iter().map(PlexMetadata::to_track).collect(),
            total: p.total,
        })
    }

    /// Items of a type changed at or after `since` (unix seconds):
    /// `/library/sections/{id}/all?type=N&updatedAt>=T`.
    pub async fn updated_since(
        &self,
        section: &str,
        ty: MetaType,
        since: i64,
        start: u64,
        size: u64,
    ) -> AppResult<Page<PlexMetadata>> {
        // Plex filter operator syntax: the key is `updatedAt>` and the value
        // follows `=`; the URL encodes it as `updatedAt%3E=T`.
        self.list(
            section,
            ty,
            start,
            size,
            &[("updatedAt>", since.to_string())],
        )
        .await
    }

    /// Values of a tag field, e.g. `format` or `subformat`, for a type:
    /// `/library/sections/{id}/{field}?type=9`.
    pub async fn tag_values(
        &self,
        section: &str,
        field: &str,
        ty: MetaType,
    ) -> AppResult<Vec<TagValue>> {
        let c = self
            .container(
                &format!("/library/sections/{section}/{field}"),
                &[("type", ty.code().to_string())],
                None,
            )
            .await?;
        Ok(c.directory
            .into_iter()
            .map(|d| TagValue {
                key: d.key,
                title: d.title,
            })
            .collect())
    }

    // ---- metadata ---------------------------------------------------------

    pub async fn metadata(&self, rating_key: &str) -> AppResult<PlexMetadata> {
        let c = self
            .container(&format!("/library/metadata/{rating_key}"), &[], None)
            .await?;
        c.metadata
            .into_iter()
            .next()
            .ok_or_else(|| AppError::NotFound(format!("item {rating_key}")))
    }

    /// Ask the server whether a track's file can be read right now
    /// (`GET /library/metadata/{id}?checkFiles=1`). This is a metadata call,
    /// not a file transfer, so it never interrupts a playing stream.
    /// `Ok(None)` when the server doesn't say.
    pub async fn file_readable(&self, rating_key: &str) -> AppResult<Option<bool>> {
        let c = self
            .container(
                &format!("/library/metadata/{rating_key}"),
                &[("checkFiles", "1".to_string())],
                None,
            )
            .await?;
        let part = c
            .metadata
            .first()
            .and_then(|m| m.media.first())
            .and_then(|m| m.parts.first());
        Ok(part.and_then(|p| match (p.exists, p.accessible) {
            (None, None) => None,
            (e, a) => Some(e.unwrap_or(true) && a.unwrap_or(true)),
        }))
    }

    pub async fn children(&self, rating_key: &str) -> AppResult<Vec<PlexMetadata>> {
        let c = self
            .container(
                &format!("/library/metadata/{rating_key}/children"),
                &[],
                None,
            )
            .await?;
        Ok(c.metadata)
    }

    /// Album detail (has `Format`/`Subformat`, so `section` is real).
    pub async fn album(&self, rating_key: &str) -> AppResult<Album> {
        Ok(self.metadata(rating_key).await?.to_album())
    }

    pub async fn album_tracks(&self, rating_key: &str) -> AppResult<Vec<Track>> {
        Ok(self
            .children(rating_key)
            .await?
            .iter()
            .map(PlexMetadata::to_track)
            .collect())
    }

    /// Track detail incl. bit depth, sample rate and loudness.
    pub async fn track(&self, rating_key: &str) -> AppResult<(Track, Option<AudioStreamInfo>)> {
        let m = self.metadata(rating_key).await?;
        Ok((m.to_track(), m.audio_stream()))
    }

    // ---- hubs, history, playlists, search ---------------------------------

    pub async fn section_hubs(&self, section: &str) -> AppResult<Vec<PlexHub>> {
        Ok(self
            .container(&format!("/hubs/sections/{section}"), &[], None)
            .await?
            .hub)
    }

    /// `/status/sessions/history/all?librarySectionID=…&sort=viewedAt:desc`.
    pub async fn history(
        &self,
        section: &str,
        start: u64,
        size: u64,
    ) -> AppResult<Page<HistoryEntry>> {
        let c = self
            .container(
                "/status/sessions/history/all",
                &[
                    ("librarySectionID", section.to_string()),
                    ("sort", "viewedAt:desc".to_string()),
                ],
                Some((start, size)),
            )
            .await?;
        let total = total_of(&c, c.metadata.len());
        Ok(Page {
            items: c.metadata.iter().map(PlexMetadata::to_history).collect(),
            total,
        })
    }

    pub async fn playlists(&self) -> AppResult<Vec<Playlist>> {
        let c = self
            .container("/playlists", &[("playlistType", "audio".to_string())], None)
            .await?;
        Ok(c.metadata.iter().map(PlexMetadata::to_playlist).collect())
    }

    pub async fn playlist_items(&self, id: &str, start: u64, size: u64) -> AppResult<Page<Track>> {
        let c = self
            .container(&format!("/playlists/{id}/items"), &[], Some((start, size)))
            .await?;
        let total = total_of(&c, c.metadata.len());
        Ok(Page {
            items: c.metadata.iter().map(PlexMetadata::to_track).collect(),
            total,
        })
    }

    /// `/hubs/search?query=…&sectionId=…&limit=…` → one hub per type.
    pub async fn search(
        &self,
        query: &str,
        section: Option<&str>,
        limit: u32,
    ) -> AppResult<Vec<PlexHub>> {
        let mut q = vec![("query", query.to_string()), ("limit", limit.to_string())];
        if let Some(s) = section {
            q.push(("sectionId", s.to_string()));
        }
        Ok(self.container("/hubs/search", &q, None).await?.hub)
    }

    /// [`Self::search`] keeping only non-empty artist/album/track/playlist
    /// hubs. Needed because `sectionId` does NOT restrict results to that
    /// library on PMS 1.43 (movies/episodes come back too).
    pub async fn search_music(
        &self,
        query: &str,
        section: Option<&str>,
        limit: u32,
    ) -> AppResult<Vec<PlexHub>> {
        Ok(music_hubs(self.search(query, section, limit).await?))
    }

    // ---- artwork, streaming -----------------------------------------------

    /// Resized artwork bytes from `/photo/:/transcode`.
    pub async fn photo(&self, image_path: &str, width: u32, height: u32) -> AppResult<Bytes> {
        let rb = self.request(Method::GET, "/photo/:/transcode")?.query(&[
            ("url", image_path.to_string()),
            ("width", width.to_string()),
            ("height", height.to_string()),
            ("minSize", "1".to_string()),
            ("upscale", "1".to_string()),
        ]);
        let resp = self.send(rb.header("accept", "image/*")).await?;
        resp.bytes().await.map_err(map_send_error)
    }

    /// Direct-play request for a part key (`/library/parts/…`). Supports
    /// range requests.
    pub fn stream_request(&self, part_key: &str) -> AppResult<StreamRequest> {
        if !part_key.starts_with('/') {
            return Err(AppError::Invalid("part key must start with '/'".into()));
        }
        Ok(StreamRequest {
            url: self.url(part_key)?,
            headers: self.auth_headers()?,
        })
    }

    /// Universal-transcoder request for a track, capped at `max_kbps`, as
    /// MP3 over HTTP. `session` should be a UUID per playback; stop with
    /// [`Self::transcode_stop`].
    pub fn transcode_request(
        &self,
        rating_key: &str,
        max_kbps: u32,
        session: &str,
    ) -> AppResult<StreamRequest> {
        let mut url = self.url("/music/:/transcode/universal/start.mp3")?;
        url.query_pairs_mut()
            .append_pair("path", &format!("/library/metadata/{rating_key}"))
            .append_pair("mediaIndex", "0")
            .append_pair("partIndex", "0")
            .append_pair("protocol", "http")
            .append_pair("directPlay", "0")
            .append_pair("directStream", "0")
            .append_pair("hasMDE", "1")
            .append_pair("musicBitrate", &max_kbps.to_string())
            .append_pair("maxAudioBitrate", &max_kbps.to_string())
            .append_pair("session", session);
        let mut headers = self.auth_headers()?;
        if let Ok(v) = session.parse() {
            headers.insert("x-plex-session-identifier", v);
        }
        Ok(StreamRequest { url, headers })
    }

    /// Stop a transcode session started with [`Self::transcode_request`].
    pub async fn transcode_stop(&self, session: &str) -> AppResult<()> {
        let rb = self
            .request(Method::GET, "/music/:/transcode/universal/stop")?
            .query(&[("session", session)]);
        self.send(rb).await.map(|_| ())
    }

    // ---- play tracking ------------------------------------------

    /// `GET /:/timeline` progress report.
    pub async fn timeline(&self, r: &TimelineReport) -> AppResult<()> {
        let mut q = vec![
            ("ratingKey", r.rating_key.clone()),
            ("key", format!("/library/metadata/{}", r.rating_key)),
            ("identifier", LIBRARY_IDENTIFIER.to_string()),
            ("state", r.state.as_str().to_string()),
            ("time", r.time_ms.to_string()),
            ("duration", r.duration_ms.to_string()),
        ];
        if let Some(id) = &r.play_queue_item_id {
            q.push(("playQueueItemID", id.clone()));
        }
        let rb = self.request(Method::GET, "/:/timeline")?.query(&q);
        self.send(rb).await.map(|_| ())
    }

    /// `GET /:/scrobble` — mark played.
    pub async fn scrobble(&self, rating_key: &str) -> AppResult<()> {
        let rb = self
            .request(Method::GET, "/:/scrobble")?
            .query(&[("key", rating_key), ("identifier", LIBRARY_IDENTIFIER)]);
        self.send(rb).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TOKEN: &str = "client-secret-token-xyz";

    async fn check_files_reply(server: &MockServer, id: &str, part: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path(format!("/library/metadata/{id}")))
            .and(query_param("checkFiles", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {"Metadata": [{"ratingKey": id, "type": "track", "title": "t",
                    "Media": [{"Part": [part]}]}]}
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn file_readable_reads_check_files_flags() {
        let server = MockServer::start().await;
        check_files_reply(&server, "1", serde_json::json!({"key": "/p/1", "exists": "1", "accessible": "1"})).await;
        check_files_reply(&server, "2", serde_json::json!({"key": "/p/2", "exists": "1", "accessible": "0"})).await;
        check_files_reply(&server, "3", serde_json::json!({"key": "/p/3", "exists": false})).await;
        check_files_reply(&server, "4", serde_json::json!({"key": "/p/4"})).await;
        let c = client(&server);
        assert_eq!(c.file_readable("1").await.unwrap(), Some(true));
        assert_eq!(c.file_readable("2").await.unwrap(), Some(false));
        assert_eq!(c.file_readable("3").await.unwrap(), Some(false));
        assert_eq!(c.file_readable("4").await.unwrap(), None);
    }

    fn client(server: &MockServer) -> PlexClient {
        PlexClient::new(
            &server.uri(),
            Token::new(TOKEN).unwrap(),
            Arc::new(ClientIdentity::with_id("cid")),
            Some(ConnectionKind::Local),
        )
        .unwrap()
    }

    fn mc(v: serde_json::Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({ "MediaContainer": v }))
    }

    #[test]
    fn release_section_mapping() {
        use ReleaseSection::*;
        let s = |f: Option<&str>, subs: &[&str]| {
            release_section(f, &subs.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        assert_eq!(s(None, &[]), Albums);
        assert_eq!(s(Some("Album"), &[]), Albums);
        assert_eq!(s(Some("EP"), &[]), SinglesEps);
        assert_eq!(s(Some("Single"), &[]), SinglesEps);
        assert_eq!(s(Some("Album"), &["Live"]), Live);
        assert_eq!(s(Some("Album"), &["Compilation"]), Compilations);
        assert_eq!(s(Some("Album"), &["Soundtrack"]), Soundtracks);
        assert_eq!(s(Some("EP"), &["Soundtrack"]), Soundtracks);
        assert_eq!(s(Some("Album"), &["Remix"]), Remixes);
        assert_eq!(s(Some("Album"), &["Demo"]), Demos);
        assert_eq!(s(Some("Album"), &["Mixtape/Street"]), Mixtapes);
        assert_eq!(s(Some("album"), &["mixtape/street"]), Mixtapes);
        assert_eq!(s(Some("Something"), &[]), Albums);
    }

    #[test]
    fn album_mapping_from_detail() {
        let m: PlexMetadata = serde_json::from_value(serde_json::json!({
            "ratingKey": "100", "type": "album", "title": "LP", "titleSort": "LP, The",
            "parentRatingKey": "10", "parentTitle": "Artist", "year": 2020, "thumb": "/t",
            "leafCount": 12, "addedAt": 1700000000, "viewCount": 3,
            "Format": [{"tag": "EP"}], "Subformat": [{"tag": "Live"}],
            "Genre": [{"tag": "Rock"}], "UltraBlurColors": {"topLeft": "5b3a2c"}
        }))
        .unwrap();
        let a = m.to_album();
        assert_eq!(a.id, "100");
        assert_eq!(a.sort_title, "LP, The");
        assert_eq!(a.artist_id, "10");
        assert_eq!(a.format.as_deref(), Some("ep"));
        assert_eq!(a.subformats, vec!["live"]);
        assert_eq!(a.section, ReleaseSection::Live);
        assert_eq!(a.track_count, 12);
        assert_eq!(a.tint.as_deref(), Some("5b3a2c"));
        assert_eq!(a.genres, vec!["Rock"]);
    }

    #[test]
    fn track_rating_count_is_optional_and_lenient() {
        let parse = |v: serde_json::Value| serde_json::from_value::<PlexMetadata>(v).unwrap().to_track();
        let none = parse(serde_json::json!({ "ratingKey": "1", "type": "track", "title": "Demo" }));
        assert_eq!(none.rating_count, None);
        let text = parse(serde_json::json!({ "ratingKey": "2", "type": "track", "title": "Hit", "ratingCount": "4521" }));
        assert_eq!(text.rating_count, Some(4521));
        let neg = parse(serde_json::json!({ "ratingKey": "3", "type": "track", "title": "Odd", "ratingCount": -1 }));
        assert_eq!(neg.rating_count, None);
    }

    #[test]
    fn track_mapping_with_streams() {
        let m: PlexMetadata = serde_json::from_value(serde_json::json!({
            "ratingKey": "500", "type": "track", "title": "Song",
            "parentRatingKey": "100", "parentTitle": "LP", "parentIndex": 2, "index": 5,
            "grandparentRatingKey": "10", "grandparentTitle": "Artist",
            "originalTitle": "Artist, Guest", "duration": 200000, "parentThumb": "/pt",
            "userRating": 10.0, "viewCount": 1, "ratingCount": 1834562,
            "Media": [{"bitrate": 900, "audioCodec": "flac", "container": "flac", "duration": 200000,
                "Part": [{"id": 1, "key": "/library/parts/1/2/file.flac", "Stream": [
                    {"streamType": 2, "codec": "flac", "bitDepth": 24, "samplingRate": 96000,
                     "gain": "-7.5", "albumGain": -6.1, "peak": 0.98, "loudness": -9.2, "lra": 5.0}
                ]}]}]
        }))
        .unwrap();
        let t = m.to_track();
        assert_eq!(t.part_key, "/library/parts/1/2/file.flac");
        assert_eq!(t.disc_number, 2);
        assert_eq!(t.track_number, 5);
        assert_eq!(t.track_artist.as_deref(), Some("Artist, Guest"));
        assert_eq!(t.bit_depth, Some(24));
        assert_eq!(t.sample_rate, Some(96000));
        assert_eq!(t.codec.as_deref(), Some("flac"));
        assert_eq!(t.thumb.as_deref(), Some("/pt"));
        assert_eq!(t.user_rating, Some(10.0));
        assert_eq!(t.rating_count, Some(1_834_562));
        let s = m.audio_stream().unwrap();
        assert_eq!(s.gain, Some(-7.5));
        assert_eq!(s.album_gain, Some(-6.1));
    }

    #[tokio::test]
    async fn paging_uses_headers_and_token_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/sections/3/all"))
            .and(query_param("type", "9"))
            .and(header("x-plex-container-start", "500"))
            .and(header("x-plex-container-size", "500"))
            .and(header("x-plex-token", TOKEN))
            .and(header("accept", "application/json"))
            .respond_with(mc(serde_json::json!({
                "size": 1, "totalSize": 501, "offset": 500,
                "Metadata": [{"ratingKey": "9", "title": "A", "parentRatingKey": "1", "parentTitle": "X"}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let p = client(&server).albums("3", 500, 500).await.unwrap();
        assert_eq!(p.total, 501);
        assert_eq!(p.items[0].id, "9");
        let reqs = server.received_requests().await.unwrap();
        assert!(
            !reqs[0].url.as_str().contains(TOKEN),
            "token must not be in URL"
        );
    }

    #[tokio::test]
    async fn list_all_walks_pages() {
        let server = MockServer::start().await;
        let items = |n: usize, off: usize| {
            (0..n)
                .map(|i| serde_json::json!({"ratingKey": (off + i).to_string(), "title": "t"}))
                .collect::<Vec<_>>()
        };
        Mock::given(method("GET"))
            .and(path("/library/sections/3/all"))
            .and(header("x-plex-container-start", "0"))
            .respond_with(mc(
                serde_json::json!({"size": 500, "totalSize": 700, "Metadata": items(500, 0)}),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/3/all"))
            .and(header("x-plex-container-start", "500"))
            .respond_with(mc(
                serde_json::json!({"size": 200, "totalSize": 700, "Metadata": items(200, 500)}),
            ))
            .mount(&server)
            .await;
        let all = client(&server)
            .list_all("3", MetaType::Track, &[])
            .await
            .unwrap();
        assert_eq!(all.len(), 700);
        assert_eq!(all[699].rating_key, "699");
    }

    #[tokio::test]
    async fn count_uses_zero_size() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/sections/3/all"))
            .and(query_param("type", "10"))
            .and(header("x-plex-container-size", "0"))
            .respond_with(mc(serde_json::json!({"size": 0, "totalSize": 1439})))
            .mount(&server)
            .await;
        assert_eq!(
            client(&server).count("3", MetaType::Track).await.unwrap(),
            1439
        );
    }

    #[tokio::test]
    async fn updated_since_sends_filter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/sections/3/all"))
            .and(query_param("updatedAt>", "1700000000"))
            .respond_with(mc(serde_json::json!({"size": 0, "totalSize": 0})))
            .expect(1)
            .mount(&server)
            .await;
        let p = client(&server)
            .updated_since("3", MetaType::Track, 1_700_000_000, 0, 100)
            .await
            .unwrap();
        assert_eq!(p.total, 0);
    }

    #[tokio::test]
    async fn sections_prefs_and_root() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/library/sections"))
            .respond_with(mc(serde_json::json!({"Directory": [
                {"key": "3", "type": "artist", "title": "Music", "uuid": "u-1", "agent": "tv.plex.agents.music"},
                {"key": "1", "type": "movie", "title": "Movies"}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/3/prefs"))
            .respond_with(mc(serde_json::json!({"Setting": [
                {"id": "respectTags", "value": true},
                {"id": "musicAnalysis", "value": "0"}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(mc(serde_json::json!({
                "friendlyName": "Home", "machineIdentifier": "m", "version": "1.43.4",
                "myPlexSubscription": true, "musicAnalysis": 2
            })))
            .mount(&server)
            .await;
        let c = client(&server);
        let music = c.music_sections().await.unwrap();
        assert_eq!(music.len(), 1);
        assert_eq!(music[0].uuid.as_deref(), Some("u-1"));
        let prefs = c.section_prefs("3").await.unwrap();
        assert_eq!(prefs.respect_tags, Some(true));
        assert_eq!(prefs.music_analysis, Some(false));
        let root = c.root().await.unwrap();
        assert!(root.my_plex_subscription);
        assert_eq!(root.music_analysis, Some(2));
        assert_eq!(root.version.as_deref(), Some("1.43.4"));
    }

    #[tokio::test]
    async fn error_mapping() {
        let server = MockServer::start().await;
        for (p, code) in [
            ("/a", 401u16),
            ("/b", 403),
            ("/c", 404),
            ("/d", 503),
            ("/e", 500),
            ("/f", 400),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(code).set_body_string(format!("echo {TOKEN}")))
                .mount(&server)
                .await;
        }
        let c = client(&server);
        let kind = |e: AppError| (e.kind(), e.to_string());
        let get = |p: &'static str| {
            let c = c.clone();
            async move { c.container(p, &[], None).await.unwrap_err() }
        };
        for (p, want) in [
            ("/a", "auth"),
            ("/b", "auth"),
            ("/c", "notFound"),
            ("/d", "server"),
            ("/e", "server"),
            ("/f", "invalid"),
        ] {
            let (k, msg) = kind(get(p).await);
            assert_eq!(k, want, "{p}");
            assert!(!msg.contains(TOKEN));
        }
        // connection refused → network
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        let dead = PlexClient::new(
            &format!("http://{addr}"),
            Token::new(TOKEN).unwrap(),
            Arc::new(ClientIdentity::with_id("c")),
            None,
        )
        .unwrap();
        let e = dead.root().await.unwrap_err();
        assert_eq!(e.kind(), "network");
        assert!(!e.to_string().contains(TOKEN));
        assert!(!format!("{e:?}").contains(TOKEN));
    }

    #[tokio::test]
    async fn timeout_is_network_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/identity"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
            .mount(&server)
            .await;
        let c = client(&server);
        let rb = c
            .request(Method::GET, "/identity")
            .unwrap()
            .timeout(Duration::from_millis(100));
        let e = c.send(rb).await.unwrap_err();
        assert_eq!(e.kind(), "network");
    }

    #[test]
    fn debug_output_never_contains_token() {
        let c = PlexClient::new(
            "http://127.0.0.1:1",
            Token::new(TOKEN).unwrap(),
            Arc::new(ClientIdentity::with_id("c")),
            None,
        )
        .unwrap();
        assert!(!format!("{c:?}").contains(TOKEN));
        let s = c.stream_request("/library/parts/1/2/file.flac").unwrap();
        assert!(!s.url.as_str().contains(TOKEN));
        assert!(!format!("{s:?}").contains(TOKEN));
        assert!(s.headers["x-plex-token"].is_sensitive());
        assert_eq!(s.url.path(), "/library/parts/1/2/file.flac");
        let t = c.transcode_request("500", 320, "sess-1").unwrap();
        assert!(!t.url.as_str().contains(TOKEN));
        assert!(t.url.as_str().contains("maxAudioBitrate=320"));
        assert!(t.url.as_str().contains("path=%2Flibrary%2Fmetadata%2F500"));
        assert!(c.stream_request("relative").is_err());
    }

    #[tokio::test]
    async fn photo_sends_transcode_params() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/photo/:/transcode"))
            .and(query_param("url", "/library/metadata/1/thumb/2"))
            .and(query_param("width", "300"))
            .and(query_param("height", "300"))
            .and(query_param("minSize", "1"))
            .and(query_param("upscale", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8, 2, 3]))
            .mount(&server)
            .await;
        let b = client(&server)
            .photo("/library/metadata/1/thumb/2", 300, 300)
            .await
            .unwrap();
        assert_eq!(&b[..], &[1, 2, 3]);
    }

    #[tokio::test]
    async fn timeline_and_scrobble_requests() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/:/timeline"))
            .and(query_param("ratingKey", "500"))
            .and(query_param("key", "/library/metadata/500"))
            .and(query_param("state", "playing"))
            .and(query_param("time", "1000"))
            .and(query_param("duration", "200000"))
            .and(query_param("playQueueItemID", "77"))
            .and(header("x-plex-token", TOKEN))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/:/scrobble"))
            .and(query_param("key", "500"))
            .and(query_param("identifier", "com.plexapp.plugins.library"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let c = client(&server);
        c.timeline(&TimelineReport {
            rating_key: "500".into(),
            state: PlaybackState::Playing,
            time_ms: 1000,
            duration_ms: 200000,
            play_queue_item_id: Some("77".into()),
        })
        .await
        .unwrap();
        c.scrobble("500").await.unwrap();
    }

    #[tokio::test]
    async fn history_playlists_search_hubs_children() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/status/sessions/history/all"))
            .and(query_param("librarySectionID", "3"))
            .and(query_param("sort", "viewedAt:desc"))
            .respond_with(mc(serde_json::json!({"size": 1, "totalSize": 1232, "Metadata": [
                {"ratingKey": "500", "title": "Song", "grandparentTitle": "Artist", "viewedAt": 1700000000, "accountID": 1, "deviceID": 2}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/playlists"))
            .and(query_param("playlistType", "audio"))
            .respond_with(mc(serde_json::json!({"Metadata": [
                {"ratingKey": "p1", "title": "Mix", "smart": true, "leafCount": 30, "duration": 1000, "composite": "/c"}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/playlists/p1/items"))
            .respond_with(mc(serde_json::json!({"totalSize": 30, "Metadata": [{"ratingKey": "500", "title": "Song"}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/hubs/search"))
            .and(query_param("query", "abc"))
            .and(query_param("sectionId", "3"))
            .respond_with(mc(serde_json::json!({"Hub": [
                {"hubIdentifier": "artist", "type": "artist", "size": 1, "Metadata": [{"ratingKey": "10", "title": "Abc"}]},
                {"hubIdentifier": "album", "type": "album", "size": 0}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/hubs/sections/3"))
            .respond_with(mc(serde_json::json!({"Hub": [{"hubIdentifier": "music.recent.added.3", "size": 0, "more": true}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/metadata/100/children"))
            .respond_with(mc(serde_json::json!({"Metadata": [{"ratingKey": "500", "title": "Song", "index": 1}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/3/format"))
            .and(query_param("type", "9"))
            .respond_with(mc(
                serde_json::json!({"Directory": [{"key": "1234", "title": "EP"}]}),
            ))
            .mount(&server)
            .await;
        let c = client(&server);
        let h = c.history("3", 0, 50).await.unwrap();
        assert_eq!(h.total, 1232);
        assert_eq!(h.items[0].artist_title.as_deref(), Some("Artist"));
        let pl = c.playlists().await.unwrap();
        assert!(pl[0].smart);
        assert_eq!(pl[0].thumb.as_deref(), Some("/c"));
        assert_eq!(c.playlist_items("p1", 0, 100).await.unwrap().total, 30);
        let hubs = c.search("abc", Some("3"), 10).await.unwrap();
        assert_eq!(hubs[0].metadata[0].to_artist().title, "Abc");
        assert!(hubs[1].metadata.is_empty());
        let music = c.search_music("abc", Some("3"), 10).await.unwrap();
        assert_eq!(music.len(), 1);
        let mixed = music_hubs(vec![
            PlexHub {
                hub_identifier: "movie".into(),
                kind: Some("movie".into()),
                metadata: vec![PlexMetadata::default()],
                ..Default::default()
            },
            PlexHub {
                hub_identifier: "track".into(),
                kind: Some("track".into()),
                metadata: vec![PlexMetadata::default()],
                ..Default::default()
            },
        ]);
        assert_eq!(mixed.len(), 1);
        assert_eq!(mixed[0].hub_identifier, "track");
        assert!(c.section_hubs("3").await.unwrap()[0].more);
        assert_eq!(c.album_tracks("100").await.unwrap()[0].track_number, 1);
        let tv = c.tag_values("3", "format", MetaType::Album).await.unwrap();
        assert_eq!(tv[0].title, "EP");
    }

    #[test]
    fn url_joins_paths_with_colons() {
        let c = PlexClient::new(
            "https://a.plex.direct:32400/",
            Token::new("t").unwrap(),
            Arc::new(ClientIdentity::with_id("c")),
            None,
        )
        .unwrap();
        assert_eq!(
            c.url("/:/timeline").unwrap().as_str(),
            "https://a.plex.direct:32400/:/timeline"
        );
        assert_eq!(
            c.url("/library/metadata/1?x=1").unwrap().as_str(),
            "https://a.plex.direct:32400/library/metadata/1?x=1"
        );
    }
}
