//! What Jellyfin sends, as far as Platter reads it. Every field is optional
//! or defaulted: servers differ between versions and leave fields out.

use serde::Deserialize;

/// `/System/Info/Public`: answers without signing in.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PublicInfo {
    pub id: Option<String>,
    pub server_name: Option<String>,
    pub version: Option<String>,
    pub product_name: Option<String>,
    pub local_address: Option<String>,
}

/// `/Users/AuthenticateByName` and `/Users/AuthenticateWithQuickConnect`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AuthResult {
    pub user: Option<User>,
    pub access_token: Option<String>,
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct User {
    pub id: String,
    pub name: Option<String>,
    pub server_id: Option<String>,
    pub policy: Option<Policy>,
}

/// What the account may do. Missing fields count as allowed, as on a server
/// that predates them.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Policy {
    pub is_administrator: bool,
    pub enable_audio_playback_transcoding: bool,
    pub enable_content_downloading: bool,
    pub enable_playback_remuxing: bool,
    pub enable_media_playback: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            is_administrator: false,
            enable_audio_playback_transcoding: true,
            enable_content_downloading: true,
            enable_playback_remuxing: true,
            enable_media_playback: true,
        }
    }
}

/// `/QuickConnect/Initiate` and `/QuickConnect/Connect`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct QuickConnectResult {
    pub authenticated: bool,
    pub secret: String,
    pub code: String,
}

/// A page of items.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct QueryResult {
    pub items: Vec<Item>,
    pub total_record_count: Option<u64>,
}

/// An id and a name (artists, studios, genres on items).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct NameId {
    pub name: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserData {
    pub is_favorite: bool,
    pub play_count: u32,
    pub last_played_date: Option<String>,
    pub played: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MediaStream {
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub codec: Option<String>,
    pub bit_rate: Option<u64>,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MediaSource {
    pub id: Option<String>,
    pub container: Option<String>,
    pub size: Option<u64>,
    pub bitrate: Option<u64>,
    pub path: Option<String>,
    pub media_streams: Vec<MediaStream>,
}

/// One library item (`BaseItemDto`), the fields Platter maps.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Item {
    pub id: String,
    pub name: Option<String>,
    pub sort_name: Option<String>,
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub collection_type: Option<String>,
    pub media_type: Option<String>,
    pub overview: Option<String>,
    pub production_year: Option<i32>,
    pub premiere_date: Option<String>,
    pub date_created: Option<String>,
    pub run_time_ticks: Option<u64>,
    pub index_number: Option<u32>,
    pub parent_index_number: Option<u32>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub album_artist: Option<String>,
    pub album_artists: Vec<NameId>,
    pub artist_items: Vec<NameId>,
    pub artists: Vec<String>,
    pub album_primary_image_tag: Option<String>,
    pub image_tags: std::collections::HashMap<String, String>,
    pub backdrop_image_tags: Vec<String>,
    pub genres: Vec<String>,
    pub studios: Vec<NameId>,
    pub provider_ids: std::collections::HashMap<String, String>,
    pub user_data: Option<UserData>,
    pub media_sources: Vec<MediaSource>,
    pub container: Option<String>,
    pub path: Option<String>,
    pub child_count: Option<u32>,
    pub song_count: Option<u32>,
    pub cumulative_run_time_ticks: Option<u64>,
    pub normalization_gain: Option<f64>,
    pub playlist_item_id: Option<String>,
    pub date_last_saved: Option<String>,
}

/// `/Audio/{id}/Lyrics`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LyricDto {
    pub metadata: LyricMetadata,
    pub lyrics: Vec<LyricLineDto>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LyricMetadata {
    pub is_synced: Option<bool>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LyricLineDto {
    pub text: String,
    /// Ticks (100 ns).
    pub start: Option<u64>,
}

/// One entry of `/Sessions`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct SessionInfo {
    pub user_id: Option<String>,
    pub user_name: Option<String>,
    pub device_id: Option<String>,
    pub now_playing_item: Option<Item>,
    pub play_state: Option<PlayState>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlayState {
    pub is_paused: bool,
}

/// `/Playlists` (create).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlaylistCreated {
    pub id: String,
}

/// Ticks (100 ns) to milliseconds.
pub fn ticks_ms(t: u64) -> u64 {
    t / 10_000
}

/// Milliseconds to ticks.
pub fn ms_ticks(ms: u64) -> u64 {
    ms.saturating_mul(10_000)
}

/// A Jellyfin date (ISO 8601, UTC, up to 7 fraction digits) as unix seconds.
pub fn unix_s(date: &str) -> Option<i64> {
    let d = date.trim();
    if d.len() < 19 {
        return None;
    }
    let num = |a: usize, b: usize| d.get(a..b)?.parse::<i64>().ok();
    let (y, mo, day, h, mi, s) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

/// Unix seconds as the date form Jellyfin takes in queries.
pub fn iso(s: i64) -> String {
    let days = s.div_euclid(86_400);
    let secs = s.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.0000000Z", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// "10.9.11" → (10, 9).
pub fn version(v: &str) -> Option<(u32, u32)> {
    let mut parts = v.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        assert_eq!(unix_s("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_s("2024-02-29T12:34:56.1234567Z"), Some(1_709_210_096));
        assert_eq!(iso(1_709_210_096), "2024-02-29T12:34:56.0000000Z");
        for s in [0, 86_399, 951_782_400, 1_760_000_000] {
            assert_eq!(unix_s(&iso(s)), Some(s));
        }
        assert_eq!(unix_s("nope"), None);
        assert_eq!(unix_s("0001-13-01T00:00:00Z"), None);
    }

    #[test]
    fn versions_and_ticks() {
        assert_eq!(version("10.9.11"), Some((10, 9)));
        assert_eq!(version("10.10"), Some((10, 10)));
        assert_eq!(version("x"), None);
        assert_eq!(ticks_ms(2_000_000_000), 200_000);
        assert_eq!(ms_ticks(1), 10_000);
    }

    #[test]
    fn missing_policy_fields_allow() {
        let p: Policy = serde_json::from_str(r#"{"IsAdministrator":true}"#).unwrap();
        assert!(p.is_administrator && p.enable_audio_playback_transcoding && p.enable_content_downloading);
        let p: Policy = serde_json::from_str(r#"{"EnableContentDownloading":false}"#).unwrap();
        assert!(!p.enable_content_downloading && !p.is_administrator);
    }
}
