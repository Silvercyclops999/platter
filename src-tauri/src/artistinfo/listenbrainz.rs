//! ListenBrainz top recordings for an artist (by MusicBrainz id): the global
//! Popular list. No key is needed for answers ListenBrainz has cached; for
//! the rest it asks for a user token (free, from the user's ListenBrainz
//! settings), sent as `Authorization: Token <token>` when one is saved.

use std::borrow::Cow;

use serde::Deserialize;
use serde_json::Value;

use super::http::InfoHttp;
use super::limit::Limiter;
use super::{Fetch, FetchError, is_mbid};
use crate::db::popular::GlobalTop;

pub const API: &str = "https://api.listenbrainz.org/1/";

pub struct ListenBrainz<'a> {
    pub http: &'a dyn InfoHttp,
    pub limiter: &'a Limiter,
    pub token: Option<&'a str>,
}

/// What one lookup gives.
#[derive(Debug, Clone, PartialEq)]
pub struct TopRecordings {
    pub top: GlobalTop,
    /// Artist credit names on the recordings (for names with separators).
    pub credit_names: Vec<String>,
}

/// One recording in a top-recordings reply. Only these fields are read; the
/// rest of the answer (often several MB) is skipped while parsing.
#[derive(Deserialize)]
struct Rec<'a> {
    #[serde(borrow, default)]
    recording_name: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    release_name: Option<Cow<'a, str>>,
    #[serde(default)]
    total_listen_count: Option<u64>,
    #[serde(borrow, default)]
    artists: Option<Vec<Credit<'a>>>,
}

#[derive(Deserialize)]
struct Credit<'a> {
    #[serde(borrow, default)]
    artist_credit_name: Option<Cow<'a, str>>,
}

/// Parse a top-recordings reply: a JSON array of recordings, kept as the
/// top songs with the releases they came out on. Anything else is a bad
/// answer.
pub fn parse(body: &[u8]) -> Result<Option<TopRecordings>, FetchError> {
    let items: Vec<Rec> = serde_json::from_slice(body)
        .map_err(|e| FetchError::Unavailable(format!("ListenBrainz answer is not a list of recordings: {e}")))?;
    let mut entries = Vec::with_capacity(items.len());
    let mut credit_names: Vec<String> = Vec::new();
    for it in items {
        let Some(title) = it.recording_name.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
            continue;
        };
        let release = it.release_name.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned);
        entries.push((title.to_owned(), it.total_listen_count.unwrap_or(0), release));
        for a in it.artists.iter().flatten() {
            if let Some(n) = a.artist_credit_name.as_deref().map(str::trim)
                && !n.is_empty()
                && !credit_names.iter().any(|c| c == n)
            {
                credit_names.push(n.to_owned());
            }
        }
    }
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(TopRecordings { top: GlobalTop::from_entries("listenbrainz", entries), credit_names }))
}

impl ListenBrainz<'_> {
    fn headers(&self) -> Vec<(&'static str, String)> {
        self.token.map(|t| vec![("Authorization", format!("Token {t}"))]).unwrap_or_default()
    }

    /// The artist's top recordings. `Ok(None)` when ListenBrainz has none, or
    /// wants a token and none is saved.
    pub async fn top(&self, mbid: &str) -> Fetch<TopRecordings> {
        if !is_mbid(mbid) {
            return Ok(None);
        }
        let url = format!("{API}popularity/top-recordings-for-artist/{mbid}");
        let resp = self.limiter.get(self.http, &url, &self.headers()).await?;
        match resp.status {
            200 => parse(&resp.body),
            401 | 403 if self.token.is_some() => Err(FetchError::KeyRejected),
            // No token: ListenBrainz only answers what it has cached.
            401 | 403 => Ok(None),
            400 | 404 => Ok(None),
            s => Err(FetchError::Unavailable(format!("ListenBrainz answered {s}"))),
        }
    }

    /// Check a token: `Err(KeyRejected)` when ListenBrainz says it's invalid.
    pub async fn validate(&self) -> Result<(), FetchError> {
        let resp = self.limiter.get(self.http, &format!("{API}validate-token"), &self.headers()).await?;
        if !resp.ok() {
            return Err(FetchError::Unavailable(format!("ListenBrainz answered {}", resp.status)));
        }
        let v: Value = resp.json().map_err(FetchError::from)?;
        match v.get("valid").and_then(Value::as_bool) {
            Some(true) => Ok(()),
            Some(false) => Err(FetchError::KeyRejected),
            None => Err(FetchError::Unavailable("ListenBrainz token check: bad answer".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::http::fake::FakeHttp;
    use super::*;

    const MBID: &str = "11111111-2222-3333-4444-555555555555";

    fn rec(title: &str, n: u64, credit: &str) -> Value {
        serde_json::json!({
            "artist_mbids": [MBID], "artist_name": credit,
            "artists": [{"artist_credit_name": credit, "artist_mbid": MBID, "join_phrase": ""}],
            "recording_mbid": "r", "recording_name": title, "release_name": "LP",
            "total_listen_count": n, "total_user_count": 1, "length": 200000
        })
    }

    fn lb<'a>(http: &'a FakeHttp, limiter: &'a Limiter, token: Option<&'a str>) -> ListenBrainz<'a> {
        ListenBrainz { http, limiter, token }
    }

    #[tokio::test]
    async fn parses_and_merges_recordings() {
        let http = FakeHttp::new();
        let body = serde_json::json!([
            rec("Big Song", 900, "Rue, The Moth"),
            rec("Other", 500, "Rue, The Moth"),
            rec("Big Song - 2011 Remaster", 200, "Rue, The Moth"),
        ]);
        http.route("top-recordings-for-artist", 200, &body.to_string());
        let l = Limiter::per_second(100.0);
        let got = lb(&http, &l, None).top(MBID).await.unwrap().unwrap();
        assert_eq!(got.top.from, "listenbrainz");
        let titles: Vec<_> = got.top.songs.iter().map(|s| (s.title.as_str(), s.listens)).collect();
        assert_eq!(titles, [("Big Song", 1100), ("Other", 500)]);
        // The release each recording came out on is kept (once) to pick the library copy.
        assert_eq!(got.top.songs[0].releases, ["LP"]);
        assert_eq!(got.credit_names, ["Rue, The Moth"]);
        assert!(http.urls()[0].ends_with(&format!("top-recordings-for-artist/{MBID}")));
    }

    #[tokio::test]
    async fn token_goes_in_the_header_only() {
        let http = FakeHttp::new();
        http.route("top-recordings-for-artist", 200, "[]");
        let l = Limiter::per_second(100.0);
        assert_eq!(lb(&http, &l, Some("tok")).top(MBID).await, Ok(None));
        let log = http.log.lock();
        assert_eq!(log[0].1, vec![("Authorization", "Token tok".to_string())]);
        assert!(!log[0].0.contains("tok"));
    }

    #[tokio::test]
    async fn statuses() {
        let l = Limiter::per_second(100.0);
        let http = FakeHttp::new();
        http.route("top-recordings-for-artist", 401, r#"{"code":401,"error":"token"}"#);
        assert_eq!(lb(&http, &l, None).top(MBID).await, Ok(None), "no token: nothing");
        assert_eq!(lb(&http, &l, Some("bad")).top(MBID).await, Err(FetchError::KeyRejected));
        http.route("top-recordings-for-artist", 404, "{}");
        assert_eq!(lb(&http, &l, None).top(MBID).await, Ok(None));
        http.route("top-recordings-for-artist", 200, r#"{"unexpected": true}"#);
        assert!(matches!(lb(&http, &l, None).top(MBID).await, Err(FetchError::Unavailable(_))));
        http.route("top-recordings-for-artist", 200, "not json");
        assert!(matches!(lb(&http, &l, None).top(MBID).await, Err(FetchError::Unavailable(_))));
        http.fail("top-recordings-for-artist");
        assert!(matches!(lb(&http, &l, None).top(MBID).await, Err(FetchError::Unavailable(_))));
        // Not an MBID: no request at all.
        let before = http.calls("api.listenbrainz.org");
        assert_eq!(lb(&http, &l, None).top("nope").await, Ok(None));
        assert_eq!(http.calls("api.listenbrainz.org"), before);
    }

    #[tokio::test]
    async fn validates_tokens() {
        let l = Limiter::per_second(100.0);
        let http = FakeHttp::new();
        http.route("validate-token", 200, r#"{"code":200,"message":"Token valid.","valid":true}"#);
        assert_eq!(lb(&http, &l, Some("t")).validate().await, Ok(()));
        http.route("validate-token", 200, r#"{"code":200,"message":"Token invalid.","valid":false}"#);
        assert_eq!(lb(&http, &l, Some("t")).validate().await, Err(FetchError::KeyRejected));
    }
}
