//! LRCLIB (lrclib.net): a free, keyless lyrics database. Two requests are
//! used: `/api/get` with the artist, title, album and duration (an exact
//! lookup), and `/api/search` with the artist and title only, for songs the
//! exact lookup misses (unreleased songs, other editions) or when the exact
//! answer doesn't settle it. Every search hit with the same artist and title
//! is a candidate (see lyrics::candidates). Only those words and the
//! duration are sent.

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::share::matching::exact_key;

pub const BASE: &str = "https://lrclib.net/api";

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
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(default)]
    pub track_name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub album_name: Option<String>,
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
    pub fn synced(&self) -> bool {
        has_text(&self.synced_lyrics)
    }

    pub fn usable(&self) -> bool {
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

/// The exact lookup's reply: the record, or None when it has no words.
/// An instrumental record is kept (it has no words to show, but says the
/// song is an instrumental).
pub fn parse_get(body: &[u8]) -> AppResult<Option<Record>> {
    let r: Record = decode(body)?;
    Ok((r.usable() || r.instrumental).then_some(r))
}

/// Every search hit for the query with the same artist and title once
/// folded and some words, closest length first (synced first on a tie).
pub fn search_hits(q: &Query, body: &[u8]) -> AppResult<Vec<Record>> {
    let hits: Vec<Record> = decode(body)?;
    let (artist, title) = (exact_key(&q.artist), exact_key(&q.title));
    let secs = q.duration_ms as f64 / 1000.0;
    let diff = |r: &Record| r.duration.map(|d| (d - secs).abs()).unwrap_or(f64::INFINITY);
    let mut out: Vec<Record> = hits
        .into_iter()
        .filter(|r| r.usable() && exact_key(&r.artist_name) == artist && exact_key(&r.track_name) == title)
        .collect();
    out.sort_by(|a, b| diff(a).total_cmp(&diff(b)).then(b.synced().cmp(&a.synced())));
    Ok(out)
}
