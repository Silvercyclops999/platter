//! The iTunes Search API: one search per lookup, no key. Apple asks for
//! about 20 calls a minute at most, so requests are spaced 3 seconds apart
//! and a 429 or 5xx pauses lookups (see `artistinfo::limit`).
//!
//! Only the search words (artist and title, or artist and album) are sent.

use serde::Deserialize;

use super::matching::{AlbumHit, SongHit};
use crate::error::AppResult;

pub const SEARCH_URL: &str = "https://itunes.apple.com/search";
/// Results to look through per search.
pub const LIMIT: u32 = 25;

/// What to search for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entity {
    Song,
    Album,
}

/// The search URL for `term`.
pub fn search_url(term: &str, entity: Entity) -> String {
    let mut u = url::Url::parse(SEARCH_URL).expect("valid base url");
    u.query_pairs_mut()
        .append_pair("term", term)
        .append_pair("media", "music")
        .append_pair("entity", match entity {
            Entity::Song => "song",
            Entity::Album => "album",
        })
        .append_pair("limit", &LIMIT.to_string());
    u.into()
}

#[derive(Debug, Deserialize)]
struct Reply {
    #[serde(default)]
    results: Vec<Item>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    #[serde(default)]
    wrapper_type: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    collection_type: Option<String>,
    track_id: Option<u64>,
    collection_id: Option<u64>,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    collection_name: String,
    track_time_millis: Option<i64>,
    track_count: Option<i64>,
}

/// Song results from a search reply (other kinds are skipped).
pub fn parse_songs(body: &[u8]) -> AppResult<Vec<SongHit>> {
    let r: Reply = serde_json::from_slice(body).map_err(|e| crate::error::AppError::Decode(format!("store reply: {e}")))?;
    Ok(r.results
        .into_iter()
        .filter(|i| i.wrapper_type == "track" && i.kind.as_deref() == Some("song"))
        .filter_map(|i| {
            Some(SongHit {
                id: i.track_id?,
                artist: i.artist_name,
                title: i.track_name,
                album: i.collection_name,
                duration_ms: i.track_time_millis,
            })
        })
        .collect())
}

/// Album results from a search reply.
pub fn parse_albums(body: &[u8]) -> AppResult<Vec<AlbumHit>> {
    let r: Reply = serde_json::from_slice(body).map_err(|e| crate::error::AppError::Decode(format!("store reply: {e}")))?;
    Ok(r.results
        .into_iter()
        .filter(|i| i.wrapper_type == "collection" && i.collection_type.as_deref().is_none_or(|t| t == "Album"))
        .filter_map(|i| {
            Some(AlbumHit {
                id: i.collection_id?,
                artist: i.artist_name,
                title: i.collection_name,
                track_count: i.track_count,
            })
        })
        .collect())
}

/// The public song.link page for a store song.
pub fn song_page(track_id: u64) -> String {
    format!("https://song.link/i/{track_id}")
}

/// The public album.link page for a store album.
pub fn album_page(collection_id: u64) -> String {
    format!("https://album.link/i/{collection_id}")
}
