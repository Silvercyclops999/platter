//! Free catalogues for ISRCs: MusicBrainz first, then Deezer's public API
//! (no key needed). Only official releases are offered, and the user
//! confirms the one release a folder is matched against.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One release a folder could be matched against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// "musicbrainz" or "deezer".
    pub source: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub date: Option<String>,
    pub country: Option<String>,
    pub tracks: u32,
}

/// One song on a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatTrack {
    pub title: String,
    pub artist: String,
    pub duration_ms: Option<i64>,
    pub isrc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    /// The service couldn't be reached or answered with an error.
    Down(String),
}

#[async_trait]
pub trait Catalog: Send + Sync {
    /// Official releases matching an album. MusicBrainz's first.
    async fn releases(&self, artist: &str, album: &str) -> Result<Vec<Candidate>, LookupError>;
    async fn tracks(&self, c: &Candidate) -> Result<Vec<CatTrack>, LookupError>;
}

// ---------------------------------------------------------------- parsing

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string).filter(|x| !x.trim().is_empty())
}

/// An artist credit as one line ("Demo Artist feat. Other Artist").
fn credit(v: &Value) -> String {
    v.get("artist-credit")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|c| format!("{}{}", s(c, "name").unwrap_or_default(), s(c, "joinphrase").unwrap_or_default())).collect::<String>())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// MusicBrainz release search: official releases only.
pub fn mb_releases(v: &Value) -> Vec<Candidate> {
    v.get("releases")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|r| s(r, "status").is_some_and(|x| x.eq_ignore_ascii_case("official")))
        .filter_map(|r| {
            Some(Candidate {
                source: "musicbrainz".into(),
                id: s(r, "id").filter(|id| crate::tags::clean_mbid(id).is_some())?,
                title: s(r, "title")?,
                artist: credit(r),
                date: s(r, "date"),
                country: s(r, "country"),
                tracks: r.get("track-count").and_then(Value::as_u64).unwrap_or(0) as u32,
            })
        })
        .collect()
}

/// A MusicBrainz release with `inc=recordings+isrcs+artist-credits`.
pub fn mb_tracks(v: &Value) -> Vec<CatTrack> {
    let mut out = Vec::new();
    for m in v.get("media").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        for t in m.get("tracks").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
            let rec = t.get("recording").cloned().unwrap_or_default();
            let isrcs: Vec<String> =
                rec.get("isrcs").and_then(Value::as_array).map(|a| a.iter().filter_map(|i| i.as_str().and_then(crate::tags::clean_isrc)).collect()).unwrap_or_default();
            let artist = if t.get("artist-credit").is_some() { credit(t) } else { credit(&rec) };
            out.push(CatTrack {
                title: s(t, "title").or_else(|| s(&rec, "title")).unwrap_or_default(),
                artist,
                duration_ms: t.get("length").and_then(Value::as_i64).or_else(|| rec.get("length").and_then(Value::as_i64)),
                // A recording with two ISRCs (two masterings) is ambiguous.
                isrc: (isrcs.len() == 1).then(|| isrcs[0].clone()),
            });
        }
    }
    out
}

/// Deezer album search.
pub fn deezer_releases(v: &Value) -> Vec<Candidate> {
    v.get("data")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|a| {
            Some(Candidate {
                source: "deezer".into(),
                id: a.get("id").and_then(Value::as_u64)?.to_string(),
                title: s(a, "title")?,
                artist: a.get("artist").and_then(|x| s(x, "name")).unwrap_or_default(),
                date: s(a, "release_date"),
                country: None,
                tracks: a.get("nb_tracks").and_then(Value::as_u64).unwrap_or(0) as u32,
            })
        })
        .collect()
}

/// Deezer's album track list: (track id, title, artist, seconds).
pub fn deezer_album_tracks(v: &Value) -> Vec<(u64, String, String, Option<i64>)> {
    v.pointer("/tracks/data")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|t| {
            Some((
                t.get("id").and_then(Value::as_u64)?,
                s(t, "title")?,
                t.get("artist").and_then(|x| s(x, "name")).unwrap_or_default(),
                t.get("duration").and_then(Value::as_i64),
            ))
        })
        .collect()
}

/// A Deezer track's ISRC.
pub fn deezer_isrc(v: &Value) -> Option<String> {
    s(v, "isrc").and_then(|i| crate::tags::clean_isrc(&i))
}

// ---------------------------------------------------------------- the real catalogues

pub const MB_API: &str = "https://musicbrainz.org/ws/2/";
pub const DEEZER_API: &str = "https://api.deezer.com/";

pub struct WebCatalog {
    http: reqwest::Client,
    mb: String,
    deezer: String,
    /// MusicBrainz asks for one request a second.
    last_mb: tokio::sync::Mutex<Option<Instant>>,
}

impl WebCatalog {
    pub fn new() -> Self {
        Self::with_bases(MB_API, DEEZER_API)
    }

    pub fn with_bases(mb: &str, deezer: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(crate::artistinfo::http::user_agent())
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { http, mb: mb.into(), deezer: deezer.into(), last_mb: tokio::sync::Mutex::new(None) }
    }

    async fn get(&self, url: &str, mb: bool) -> Result<Value, LookupError> {
        let who = if mb { "MusicBrainz" } else { "Deezer" };
        if mb {
            let mut last = self.last_mb.lock().await;
            if let Some(t) = *last {
                let wait = Duration::from_millis(1100).saturating_sub(t.elapsed());
                tokio::time::sleep(wait).await;
            }
            *last = Some(Instant::now());
        }
        let r = self.http.get(url).send().await.map_err(|e| LookupError::Down(format!("{who} couldn't be reached: {e}")))?;
        if !r.status().is_success() {
            return Err(LookupError::Down(format!("{who} answered {}", r.status().as_u16())));
        }
        let v: Value = r.json().await.map_err(|e| LookupError::Down(format!("{who} sent something unreadable: {e}")))?;
        // Deezer reports errors in a 200.
        if !mb && v.get("error").is_some() {
            return Err(LookupError::Down("Deezer couldn't answer right now".into()));
        }
        Ok(v)
    }
}

impl Default for WebCatalog {
    fn default() -> Self {
        Self::new()
    }
}

fn quoted(s: &str) -> String {
    s.replace(['"', '\\'], " ")
}

#[async_trait]
impl Catalog for WebCatalog {
    async fn releases(&self, artist: &str, album: &str) -> Result<Vec<Candidate>, LookupError> {
        let q = format!("release:\"{}\" AND artist:\"{}\" AND status:official", quoted(album), quoted(artist));
        let mb_url = format!("{}release/?query={}&fmt=json&limit=10", self.mb, urlencode(&q));
        let mb = self.get(&mb_url, true).await.map(|v| mb_releases(&v));
        let dz_q = format!("artist:\"{}\" album:\"{}\"", quoted(artist), quoted(album));
        let dz_url = format!("{}search/album?q={}&limit=10", self.deezer, urlencode(&dz_q));
        let dz = self.get(&dz_url, false).await.map(|v| deezer_releases(&v));
        match (mb, dz) {
            (Err(a), Err(_)) => Err(a),
            (a, b) => Ok(a.unwrap_or_default().into_iter().chain(b.unwrap_or_default()).collect()),
        }
    }

    async fn tracks(&self, c: &Candidate) -> Result<Vec<CatTrack>, LookupError> {
        if c.source == "musicbrainz" {
            let id = crate::tags::clean_mbid(&c.id).ok_or_else(|| LookupError::Down("Not a MusicBrainz release".into()))?;
            let v = self.get(&format!("{}release/{id}?inc=recordings+isrcs+artist-credits&fmt=json", self.mb), true).await?;
            return Ok(mb_tracks(&v));
        }
        let id: u64 = c.id.parse().map_err(|_| LookupError::Down("Not a Deezer album".into()))?;
        let album = self.get(&format!("{}album/{id}", self.deezer), false).await?;
        let mut out = Vec::new();
        for (tid, title, artist, secs) in deezer_album_tracks(&album) {
            let isrc = self.get(&format!("{}track/{tid}", self.deezer), false).await.ok().and_then(|v| deezer_isrc(&v));
            out.push(CatTrack { title, artist, duration_ms: secs.map(|s| s * 1000), isrc });
        }
        Ok(out)
    }
}

fn urlencode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}
