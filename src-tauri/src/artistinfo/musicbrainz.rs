//! MusicBrainz artist search, to find the MusicBrainz id of a credited
//! artist (a name in a track's artist line with no library artist, so no
//! Plex id). Only an exact match is taken: score 100 and the same name or
//! alias once case and spacing are folded. MusicBrainz asks for at most one
//! request a second and a user agent that names the app.

use serde_json::Value;
use url::Url;

use super::http::InfoHttp;
use super::limit::Limiter;
use super::{Fetch, FetchError, fold_name, is_mbid};

pub const API: &str = "https://musicbrainz.org/ws/2/artist/";

pub struct MusicBrainz<'a> {
    pub http: &'a dyn InfoHttp,
    pub limiter: &'a Limiter,
}

/// A Lucene phrase: quotes and backslashes escaped.
fn phrase(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push('"');
    for ch in name.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

pub fn search_url(name: &str) -> String {
    let p = phrase(name.trim());
    let mut u = Url::parse(API).expect("static url");
    u.query_pairs_mut()
        .append_pair("query", &format!("artist:{p} OR alias:{p}"))
        .append_pair("limit", "5")
        .append_pair("fmt", "json");
    u.into()
}

/// The id of the first exact match in a search reply.
pub fn pick(v: &Value, name: &str) -> Option<String> {
    let want = fold_name(name);
    v.get("artists")?.as_array()?.iter().find_map(|a| {
        let score = a.get("score").and_then(Value::as_u64)?;
        if score != 100 {
            return None;
        }
        let named = a.get("name").and_then(Value::as_str).is_some_and(|n| fold_name(n) == want);
        let aliased = a
            .get("aliases")
            .and_then(Value::as_array)
            .is_some_and(|al| al.iter().any(|x| x.get("name").and_then(Value::as_str).is_some_and(|n| fold_name(n) == want)));
        let id = a.get("id").and_then(Value::as_str)?;
        ((named || aliased) && is_mbid(id)).then(|| id.to_owned())
    })
}

impl MusicBrainz<'_> {
    /// `Ok(None)` when there is no exact match.
    pub async fn artist_id(&self, name: &str) -> Fetch<String> {
        if name.trim().is_empty() {
            return Ok(None);
        }
        let resp = self.limiter.get(self.http, &search_url(name), &[]).await?;
        if !resp.ok() {
            return Err(FetchError::Unavailable(format!("MusicBrainz answered {}", resp.status)));
        }
        let v: Value = resp.json().map_err(FetchError::from)?;
        Ok(pick(&v, name))
    }
}

#[cfg(test)]
mod tests {
    use super::super::http::fake::FakeHttp;
    use super::*;
    use serde_json::json;

    const A: &str = "11111111-2222-3333-4444-555555555555";
    const B: &str = "66666666-2222-3333-4444-555555555555";

    #[test]
    fn exact_name_or_alias_with_full_score_only() {
        let v = json!({"artists": [
            {"id": B, "name": "Rue Ellis Band", "score": 100},
            {"id": A, "name": "Rue Ellis", "score": 97},
        ]});
        assert_eq!(pick(&v, "Rue Ellis"), None, "close is not enough");
        let v = json!({"artists": [{"id": A, "name": "RUE  ellis", "score": 100}]});
        assert_eq!(pick(&v, "Rue Ellis").as_deref(), Some(A));
        let v = json!({"artists": [{"id": A, "name": "Ruth Ellis", "score": 100, "aliases": [{"name": "Rue Ellis"}]}]});
        assert_eq!(pick(&v, "rue ellis").as_deref(), Some(A));
        let v = json!({"artists": [{"id": "not-an-id", "name": "Rue Ellis", "score": 100}]});
        assert_eq!(pick(&v, "Rue Ellis"), None);
        assert_eq!(pick(&json!({}), "Rue Ellis"), None);
    }

    #[test]
    fn the_query_is_an_escaped_phrase() {
        let u = Url::parse(&search_url("Say \"Hi\" $")).unwrap();
        let q = u.query_pairs().find(|(k, _)| k == "query").unwrap().1.into_owned();
        assert_eq!(q, r#"artist:"Say \"Hi\" $" OR alias:"Say \"Hi\" $""#);
        assert!(u.as_str().contains("fmt=json"));
    }

    #[tokio::test]
    async fn asks_once_and_reads_the_answer() {
        let http = FakeHttp::new();
        http.route("musicbrainz.org/ws/2/artist", 200, &json!({"artists": [{"id": A, "name": "Guest", "score": 100}]}).to_string());
        let l = Limiter::per_second(100.0);
        let mb = MusicBrainz { http: &http, limiter: &l };
        assert_eq!(mb.artist_id("Guest").await, Ok(Some(A.to_string())));
        assert_eq!(mb.artist_id("  ").await, Ok(None));
        assert_eq!(http.calls("musicbrainz.org"), 1);
        http.route("musicbrainz.org/ws/2/artist", 400, "{}");
        assert!(matches!(mb.artist_id("Guest").await, Err(FetchError::Unavailable(_))));
    }
}
