//! What an OpenSubsonic server sends (JSON, `f=json`), as far as Platter
//! reads it. Servers differ: fields are optional or defaulted, ids may come
//! as numbers or strings, flags as booleans or "true".

use serde::{Deserialize, Deserializer};

/// A string or a number, as a string.
pub fn string_or_number<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    })
}

/// [`string_or_number`] for an optional field.
pub fn opt_string_or_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) if !s.is_empty() => Some(s),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// A boolean, or "true"/"false" as older servers send.
pub fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Bool(b) => Some(b),
        serde_json::Value::String(s) => Some(s.eq_ignore_ascii_case("true")),
        _ => None,
    })
}

/// A number, or a number in a string.
pub fn lenient_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => n.as_u64().or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

/// The answer's envelope.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Status {
    pub status: String,
    pub version: Option<String>,
    /// The server's product (`navidrome`, `gonic`, ...).
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub server_version: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub open_subsonic: Option<bool>,
    pub error: Option<ApiError>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApiError {
    #[serde(deserialize_with = "lenient_u64")]
    pub code: Option<u64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Extension {
    pub name: String,
    pub versions: Vec<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Named {
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct IdName {
    #[serde(deserialize_with = "string_or_number")]
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ReplayGain {
    pub track_gain: Option<f64>,
    pub album_gain: Option<f64>,
    pub track_peak: Option<f64>,
    pub album_peak: Option<f64>,
    pub base_gain: Option<f64>,
}

/// A song (`Child`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Song {
    #[serde(deserialize_with = "string_or_number")]
    pub id: String,
    pub title: Option<String>,
    pub album: Option<String>,
    pub artist: Option<String>,
    #[serde(deserialize_with = "lenient_u64")]
    pub track: Option<u64>,
    #[serde(deserialize_with = "lenient_u64")]
    pub disc_number: Option<u64>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub genres: Vec<Named>,
    #[serde(deserialize_with = "opt_string_or_number")]
    pub cover_art: Option<String>,
    #[serde(deserialize_with = "lenient_u64")]
    pub size: Option<u64>,
    pub content_type: Option<String>,
    pub suffix: Option<String>,
    /// Seconds.
    #[serde(deserialize_with = "lenient_u64")]
    pub duration: Option<u64>,
    /// kbps.
    #[serde(deserialize_with = "lenient_u64")]
    pub bit_rate: Option<u64>,
    #[serde(deserialize_with = "lenient_u64")]
    pub bit_depth: Option<u64>,
    #[serde(deserialize_with = "lenient_u64")]
    pub sampling_rate: Option<u64>,
    pub path: Option<String>,
    #[serde(deserialize_with = "lenient_u64")]
    pub play_count: Option<u64>,
    pub played: Option<String>,
    pub created: Option<String>,
    #[serde(deserialize_with = "opt_string_or_number")]
    pub album_id: Option<String>,
    #[serde(deserialize_with = "opt_string_or_number")]
    pub artist_id: Option<String>,
    /// When it was starred (liked).
    pub starred: Option<String>,
    #[serde(deserialize_with = "lenient_u64")]
    pub user_rating: Option<u64>,
    pub music_brainz_id: Option<String>,
    pub artists: Vec<IdName>,
    pub display_artist: Option<String>,
    pub album_artists: Vec<IdName>,
    pub display_album_artist: Option<String>,
    pub replay_gain: Option<ReplayGain>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub is_video: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ItemDate {
    pub year: Option<i32>,
    pub month: Option<u32>,
    pub day: Option<u32>,
}

/// An album (`AlbumID3`), with its songs from `getAlbum`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Album {
    #[serde(deserialize_with = "string_or_number")]
    pub id: String,
    pub name: Option<String>,
    pub sort_name: Option<String>,
    pub artist: Option<String>,
    #[serde(deserialize_with = "opt_string_or_number")]
    pub artist_id: Option<String>,
    pub artists: Vec<IdName>,
    pub display_artist: Option<String>,
    #[serde(deserialize_with = "opt_string_or_number")]
    pub cover_art: Option<String>,
    #[serde(deserialize_with = "lenient_u64")]
    pub song_count: Option<u64>,
    #[serde(deserialize_with = "lenient_u64")]
    pub duration: Option<u64>,
    #[serde(deserialize_with = "lenient_u64")]
    pub play_count: Option<u64>,
    pub played: Option<String>,
    pub created: Option<String>,
    pub starred: Option<String>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub genres: Vec<Named>,
    pub record_labels: Vec<Named>,
    pub music_brainz_id: Option<String>,
    pub release_types: Vec<String>,
    pub moods: Vec<String>,
    pub release_date: Option<ItemDate>,
    pub original_release_date: Option<ItemDate>,
    #[serde(deserialize_with = "lenient_bool")]
    pub is_compilation: Option<bool>,
    pub song: Vec<Song>,
}

/// An artist (`ArtistID3`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Artist {
    #[serde(deserialize_with = "string_or_number")]
    pub id: String,
    pub name: Option<String>,
    pub sort_name: Option<String>,
    #[serde(deserialize_with = "opt_string_or_number")]
    pub cover_art: Option<String>,
    #[serde(deserialize_with = "lenient_u64")]
    pub album_count: Option<u64>,
    pub music_brainz_id: Option<String>,
    pub album: Vec<Album>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ArtistIndex {
    pub artist: Vec<Artist>,
}

/// `getArtists`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Artists {
    pub index: Vec<ArtistIndex>,
}

/// `getIndexes`: only its time of last change is read.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Indexes {
    #[serde(deserialize_with = "lenient_u64")]
    pub last_modified: Option<u64>,
}

/// `getScanStatus`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScanStatus {
    #[serde(deserialize_with = "lenient_bool")]
    pub scanning: Option<bool>,
    #[serde(deserialize_with = "lenient_u64")]
    pub count: Option<u64>,
    pub last_scan: Option<String>,
}

/// `getArtistInfo2`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtistInfo {
    pub biography: Option<String>,
    pub music_brainz_id: Option<String>,
    pub similar_artist: Vec<Artist>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AlbumList {
    pub album: Vec<Album>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SongList {
    pub song: Vec<Song>,
}

/// `search3` and `getStarred2`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Found {
    pub artist: Vec<Artist>,
    pub album: Vec<Album>,
    pub song: Vec<Song>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Playlist {
    #[serde(deserialize_with = "string_or_number")]
    pub id: String,
    pub name: Option<String>,
    pub comment: Option<String>,
    pub owner: Option<String>,
    #[serde(deserialize_with = "lenient_u64")]
    pub song_count: Option<u64>,
    #[serde(deserialize_with = "lenient_u64")]
    pub duration: Option<u64>,
    pub created: Option<String>,
    pub changed: Option<String>,
    #[serde(deserialize_with = "opt_string_or_number")]
    pub cover_art: Option<String>,
    pub entry: Vec<Song>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Playlists {
    pub playlist: Vec<Playlist>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MusicFolder {
    #[serde(deserialize_with = "string_or_number")]
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MusicFolders {
    pub music_folder: Vec<MusicFolder>,
}

/// `getUser`: what the account may do. A missing role counts as allowed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct User {
    pub username: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub admin_role: Option<bool>,
    #[serde(deserialize_with = "lenient_bool")]
    pub download_role: Option<bool>,
    #[serde(deserialize_with = "lenient_bool")]
    pub stream_role: Option<bool>,
    #[serde(deserialize_with = "lenient_bool")]
    pub playlist_role: Option<bool>,
    #[serde(deserialize_with = "lenient_bool")]
    pub scrobbling_enabled: Option<bool>,
}

/// `tokenInfo`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TokenInfo {
    pub username: Option<String>,
}

/// One row of `getNowPlaying`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NowPlaying {
    #[serde(flatten)]
    pub song: Song,
    pub username: Option<String>,
    #[serde(deserialize_with = "lenient_u64")]
    pub minutes_ago: Option<u64>,
    pub player_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct NowPlayingList {
    pub entry: Vec<NowPlaying>,
}

/// `getPlayQueue` and `getPlayQueueByIndex`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayQueue {
    #[serde(deserialize_with = "opt_string_or_number")]
    pub current: Option<String>,
    #[serde(deserialize_with = "lenient_u64")]
    pub current_index: Option<u64>,
    #[serde(deserialize_with = "lenient_u64")]
    pub position: Option<u64>,
    pub changed: Option<String>,
    pub changed_by: Option<String>,
    pub entry: Vec<Song>,
}

/// `getLyricsBySongId`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LyricsList {
    pub structured_lyrics: Vec<StructuredLyrics>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StructuredLyrics {
    pub kind: Option<String>,
    pub lang: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub synced: Option<bool>,
    /// Milliseconds added to every line.
    pub offset: Option<i64>,
    pub display_artist: Option<String>,
    pub line: Vec<LyricLine>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LyricLine {
    pub start: Option<i64>,
    pub value: String,
}

/// `getSonicSimilarTracks`: one match.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SonicMatch {
    pub entry: Song,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_flags_come_in_any_shape() {
        let s: Song = serde_json::from_value(serde_json::json!({
            "id": 42, "albumId": "al-1", "track": "3", "duration": 201.0, "isVideo": "false", "coverArt": 7
        }))
        .unwrap();
        assert_eq!((s.id.as_str(), s.album_id.as_deref(), s.track, s.duration), ("42", Some("al-1"), Some(3), Some(201)));
        assert_eq!((s.is_video, s.cover_art.as_deref()), (Some(false), Some("7")));
        let u: User = serde_json::from_value(serde_json::json!({"adminRole": "true", "downloadRole": false})).unwrap();
        assert_eq!((u.admin_role, u.download_role, u.stream_role), (Some(true), Some(false), None));
        let f: MusicFolders = serde_json::from_value(serde_json::json!({"musicFolder": [{"id": 1, "name": "Music"}]})).unwrap();
        assert_eq!(f.music_folder[0].id, "1");
    }

    #[test]
    fn now_playing_rows_are_songs_with_who() {
        let n: NowPlaying = serde_json::from_value(serde_json::json!({
            "id": "s1", "title": "Song", "username": "sam", "minutesAgo": 2, "playerName": "Phone"
        }))
        .unwrap();
        assert_eq!((n.song.id.as_str(), n.username.as_deref(), n.minutes_ago), ("s1", Some("sam"), Some(2)));
    }
}
