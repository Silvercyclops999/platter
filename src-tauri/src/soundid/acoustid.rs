//! The AcoustID lookup: a fingerprint and the file's length go out, the
//! recordings it sounds like (MusicBrainz ids) come back. Nothing else is
//! sent. Requests are paced under AcoustID's limit of three a second, and a
//! busy or failing service pauses lookups for a while.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::artistinfo::limit::Limiter;

pub const LOOKUP_URL: &str = "https://api.acoustid.org/v2/lookup";
/// Requests a second, under AcoustID's limit of three.
pub const PER_SECOND: f64 = 2.5;
const TIMEOUT: Duration = Duration::from_secs(20);
/// AcoustID's error code for an application key it doesn't know.
const INVALID_KEY: i64 = 4;

/// A MusicBrainz recording a fingerprint matched.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// 0 to 1: how closely the fingerprint matched.
    pub score: f32,
    /// MusicBrainz recording id, lower case.
    pub recording_mbid: String,
    pub title: String,
    /// The recording's artists, joined as credited.
    pub artist: String,
    /// The first album it's on, when AcoustID says.
    pub album: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    /// The key was refused: stop until a new one is set.
    KeyRefused,
    /// Rate limited or the service is down: try again later.
    Busy,
    /// Network trouble: try again later.
    Transient(String),
    /// AcoustID couldn't use this fingerprint.
    Rejected(String),
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LookupError::KeyRefused => f.write_str("AcoustID refused the key"),
            LookupError::Busy => f.write_str("AcoustID is busy"),
            LookupError::Transient(m) => write!(f, "network: {m}"),
            LookupError::Rejected(m) => write!(f, "AcoustID: {m}"),
        }
    }
}

/// Looks a fingerprint up (a fake in tests).
#[async_trait]
pub trait SoundLookup: Send + Sync {
    /// Best match first. Empty when nothing matched.
    async fn lookup(&self, key: &str, fingerprint: &str, duration_s: u32) -> Result<Vec<Hit>, LookupError>;
}

#[derive(Deserialize)]
struct Reply {
    status: String,
    #[serde(default)]
    error: Option<ReplyError>,
    #[serde(default)]
    results: Vec<ResultJ>,
}

#[derive(Deserialize)]
struct ReplyError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct ResultJ {
    #[serde(default)]
    score: f32,
    #[serde(default)]
    recordings: Vec<RecordingJ>,
}

#[derive(Deserialize)]
struct RecordingJ {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artists: Vec<ArtistJ>,
    #[serde(default)]
    releasegroups: Vec<GroupJ>,
}

#[derive(Deserialize)]
struct ArtistJ {
    name: String,
    #[serde(default)]
    joinphrase: Option<String>,
}

#[derive(Deserialize)]
struct GroupJ {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

fn valid_mbid(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// The hits in an AcoustID reply, best first. Recordings without a title
/// (AcoustID knows the id but not its name) are left out.
pub fn parse(body: &str) -> Result<Vec<Hit>, LookupError> {
    let r: Reply = serde_json::from_str(body).map_err(|e| LookupError::Rejected(format!("unreadable reply: {e}")))?;
    if r.status != "ok" {
        let e = r.error.unwrap_or(ReplyError { code: 0, message: "unknown error".into() });
        return Err(if e.code == INVALID_KEY { LookupError::KeyRefused } else { LookupError::Rejected(e.message) });
    }
    let mut out: Vec<Hit> = Vec::new();
    for res in r.results {
        for rec in res.recordings {
            let id = rec.id.trim().to_ascii_lowercase();
            let Some(title) = rec.title.filter(|t| !t.trim().is_empty()) else { continue };
            if !valid_mbid(&id) || out.iter().any(|h| h.recording_mbid == id) {
                continue;
            }
            let mut artist = String::new();
            for a in &rec.artists {
                artist.push_str(&a.name);
                artist.push_str(a.joinphrase.as_deref().unwrap_or(", "));
            }
            let artist = artist.trim_end_matches([',', ' ']).to_string();
            let album = rec
                .releasegroups
                .iter()
                .find(|g| g.kind.as_deref() == Some("Album"))
                .or(rec.releasegroups.first())
                .and_then(|g| g.title.clone());
            out.push(Hit { score: res.score.clamp(0.0, 1.0), recording_mbid: id, title, artist, album });
        }
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(out)
}

/// The real service.
pub struct AcoustId {
    client: reqwest::Client,
    url: String,
    limiter: Arc<Limiter>,
}

impl AcoustId {
    pub fn new(client: reqwest::Client) -> Self {
        Self::with_url(client, LOOKUP_URL)
    }

    /// Another address (tests).
    pub fn with_url(client: reqwest::Client, url: &str) -> Self {
        Self { client, url: url.to_string(), limiter: Arc::new(Limiter::per_second(PER_SECOND)) }
    }

    pub fn limiter(&self) -> &Limiter {
        &self.limiter
    }
}

#[async_trait]
impl SoundLookup for AcoustId {
    async fn lookup(&self, key: &str, fingerprint: &str, duration_s: u32) -> Result<Vec<Hit>, LookupError> {
        self.limiter.acquire().await.map_err(|_| LookupError::Busy)?;
        let duration = duration_s.to_string();
        let form = [
            ("client", key),
            ("duration", duration.as_str()),
            ("fingerprint", fingerprint),
            ("meta", "recordings releasegroups compress"),
            ("format", "json"),
        ];
        let resp = self
            .client
            .post(&self.url)
            .timeout(TIMEOUT)
            .form(&form)
            .send()
            .await
            .map_err(|e| LookupError::Transient(e.without_url().to_string()))?;
        let status = resp.status();
        if status.as_u16() == 429 || status.is_server_error() {
            let after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
            self.limiter.throttled(after);
            return Err(LookupError::Busy);
        }
        let body = resp.text().await.map_err(|e| LookupError::Transient(e.without_url().to_string()))?;
        let out = parse(&body);
        if out.is_ok() {
            self.limiter.succeeded();
        }
        out
    }
}
