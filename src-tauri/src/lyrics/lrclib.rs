//! LRCLIB (lrclib.net): a free, keyless lyrics database. Two requests are
//! used: `/api/get` with the artist, title, album and duration (an exact
//! lookup), and `/api/search` with the artist and title only, for songs the
//! exact lookup misses (unreleased songs, other editions). Only those words
//! and the duration are sent.

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::share::matching::exact_key;

pub const BASE: &str = "https://lrclib.net/api";
/// A search hit's synced lyrics are used only when its length is this close
/// to the track's; further off they would drift, so the plain text is used.
pub const SYNC_TOLERANCE_S: f64 = 10.0;

/// What a lookup sends.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration_ms: i64,
}

impl Query {
    pub fn seconds(&self) -> i64 {
        (self.duration_ms + 500) / 1000
    }
}

/// The exact lookup URL.
pub fn get_url(q: &Query) -> String {
    let mut u = url::Url::parse(&format!("{BASE}/get")).expect("valid base url");
    u.query_pairs_mut()
        .append_pair("artist_name", &q.artist)
        .append_pair("track_name", &q.title)
        .append_pair("album_name", &q.album)
        .append_pair("duration", &q.seconds().to_string());
    u.into()
}

/// The search URL (artist and title only).
pub fn search_url(q: &Query) -> String {
    let mut u = url::Url::parse(&format!("{BASE}/search")).expect("valid base url");
    u.query_pairs_mut()
        .append_pair("artist_name", &q.artist)
        .append_pair("track_name", &q.title);
    u.into()
}

/// One LRCLIB record.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(default)]
    pub track_name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub plain_lyrics: Option<String>,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
}

fn has_text(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|t| !t.trim().is_empty())
}

impl Record {
    fn synced(&self) -> bool {
        has_text(&self.synced_lyrics)
    }

    fn usable(&self) -> bool {
        !self.instrumental && (self.synced() || has_text(&self.plain_lyrics))
    }

    /// The text to keep: synced lyrics when allowed and present, else plain.
    pub fn text(&self, allow_synced: bool) -> Option<String> {
        if !self.usable() {
            return None;
        }
        if allow_synced && self.synced() {
            return self.synced_lyrics.clone();
        }
        if has_text(&self.plain_lyrics) {
            return self.plain_lyrics.clone();
        }
        // Only synced lyrics, but they can't be trusted to line up: keep the
        // words without their times.
        self.synced_lyrics.as_deref().map(|s| {
            crate::plex::details::parse_lyrics_text(s).lines.join("\n")
        })
    }
}

fn decode<T: serde::de::DeserializeOwned>(body: &[u8]) -> AppResult<T> {
    serde_json::from_slice(body).map_err(|e| AppError::Decode(format!("lyrics reply: {e}")))
}

/// The exact lookup's reply: the lyrics text, or None (instrumental or empty).
pub fn parse_get(body: &[u8]) -> AppResult<Option<String>> {
    Ok(decode::<Record>(body)?.text(true))
}

/// The best search hit for the query: artist and title equal once folded;
/// synced lyrics within the tolerance first, then the closest length.
/// Returns the text to keep.
pub fn best_search_hit(q: &Query, body: &[u8]) -> AppResult<Option<String>> {
    let hits: Vec<Record> = decode(body)?;
    let (artist, title) = (exact_key(&q.artist), exact_key(&q.title));
    let secs = q.duration_ms as f64 / 1000.0;
    let diff = |r: &Record| r.duration.map(|d| (d - secs).abs()).unwrap_or(f64::INFINITY);
    let best = hits
        .into_iter()
        .filter(|r| r.usable() && exact_key(&r.artist_name) == artist && exact_key(&r.track_name) == title)
        .map(|r| {
            let in_sync = r.synced() && diff(&r) <= SYNC_TOLERANCE_S;
            (r, in_sync)
        })
        .min_by(|(a, a_sync), (b, b_sync)| b_sync.cmp(a_sync).then(diff(a).total_cmp(&diff(b))));
    Ok(best.and_then(|(r, in_sync)| r.text(in_sync)))
}
