//! Similar artists from ListenBrainz's public similarity data (free, no
//! account): who people listen to alongside an artist, by MusicBrainz id.
//! Paced at one request a second and cached for a week on this device.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::artistinfo::http::InfoHttp;
use crate::artistinfo::is_mbid;
use crate::artistinfo::limit::Limiter;
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};

pub const SIMILAR_API: &str = "https://labs.api.listenbrainz.org/similar-artists/json";
/// The similarity data set asked for (listening sessions over about 20 years).
pub const ALGORITHM: &str = "session_based_days_7500_session_300_contribution_5_threshold_10_limit_100_filter_True_skip_30";
/// How long an answer is kept.
pub const CACHE_TTL: i64 = 7 * 24 * 3600;
pub const LB_DOWN: &str = "ListenBrainz isn't answering right now.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarArtist {
    pub mbid: String,
    pub name: String,
    /// How strongly they're listened to together (higher is closer).
    pub score: u64,
}

/// Artists like one artist, closest first.
#[async_trait]
pub trait SimilarArtists: Send + Sync {
    async fn similar(&self, mbid: &str) -> AppResult<Vec<SimilarArtist>>;
}

pub fn similar_url(mbid: &str) -> String {
    let mut u = Url::parse(SIMILAR_API).expect("static url");
    u.query_pairs_mut().append_pair("artist_mbids", mbid).append_pair("algorithm", ALGORITHM);
    u.into()
}

fn collect(v: &Value, reference: &str, out: &mut Vec<SimilarArtist>) {
    match v {
        Value::Array(items) => items.iter().for_each(|x| collect(x, reference, out)),
        Value::Object(o) => {
            if let Some(m) = o.get("artist_mbid").and_then(Value::as_str) {
                let mbid = m.to_ascii_lowercase();
                let name = o.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
                if is_mbid(&mbid) && !name.is_empty() && mbid != reference && !out.iter().any(|x| x.mbid == mbid) {
                    out.push(SimilarArtist { mbid, name, score: o.get("score").and_then(Value::as_u64).unwrap_or(0) });
                }
            } else if let Some(d) = o.get("data") {
                collect(d, reference, out);
            }
        }
        _ => {}
    }
}

/// Parse an answer: a list of artists (or of data sets holding one).
pub fn parse(body: &[u8], reference: &str) -> AppResult<Vec<SimilarArtist>> {
    let v: Value = serde_json::from_slice(body).map_err(|_| AppError::Network(LB_DOWN.into()))?;
    let mut out = Vec::new();
    collect(&v, &reference.to_ascii_lowercase(), &mut out);
    out.sort_by_key(|a| std::cmp::Reverse(a.score));
    Ok(out)
}

/// The real one: paced and cached in the local store.
pub struct LbCached {
    pub http: Arc<dyn InfoHttp>,
    pub limiter: Arc<Limiter>,
    pub db: DbHandle,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

#[derive(Serialize, Deserialize)]
struct Cached {
    at: i64,
    list: Vec<SimilarArtist>,
}

#[async_trait]
impl SimilarArtists for LbCached {
    async fn similar(&self, mbid: &str) -> AppResult<Vec<SimilarArtist>> {
        if !is_mbid(mbid) {
            return Ok(Vec::new());
        }
        let key = format!("discover.lb:{}", mbid.to_ascii_lowercase());
        let now = (self.clock)();
        let k = key.clone();
        let hit: Option<Cached> = self.db.read_async(move |c| kv_get(c, &k)).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok());
        if let Some(h) = &hit
            && now - h.at < CACHE_TTL
        {
            return Ok(h.list.clone());
        }
        let fresh = match self.limiter.get(self.http.as_ref(), &similar_url(mbid), &[]).await {
            Ok(r) if r.ok() => parse(&r.body, mbid)?,
            Ok(r) if matches!(r.status, 400 | 404) => Vec::new(),
            // Busy or unreachable: an old answer is better than none.
            _ => return hit.map(|h| h.list).ok_or_else(|| AppError::Network(LB_DOWN.into())),
        };
        let json = serde_json::to_string(&Cached { at: now, list: fresh.clone() }).unwrap_or_default();
        let _ = self.db.write_async(move |c| kv_set(c, &key, &json)).await;
        Ok(fresh)
    }
}
