//! Last.fm `artist.getInfo` (the user's own API key): listeners, plays, tags
//! and a bio. Images from Last.fm aren't used (its terms leave them out).

use serde_json::Value;
use url::Url;

use super::http::InfoHttp;
use super::limit::Limiter;
use super::{Fetch, FetchError, LastfmData, clean_text, is_mbid};

pub const API: &str = "https://ws.audioscrobbler.com/2.0/";

/// Error codes that mean the key is no good.
const KEY_ERRORS: &[i64] = &[10, 26];
/// "The artist you supplied could not be found".
const NOT_FOUND: i64 = 6;
/// Rate limit exceeded.
const RATE_LIMITED: i64 = 29;

const MAX_TAGS: usize = 8;

const LICENCE_LINE: &str = "User-contributed text is available under the Creative Commons By-SA License";

pub struct Lastfm<'a> {
    pub http: &'a dyn InfoHttp,
    pub limiter: &'a Limiter,
    pub key: &'a str,
    pub lang: &'a str,
    /// Cache-Control max-age of the last reply (its terms ask apps to follow it).
    pub max_age: parking_lot::Mutex<Option<u64>>,
}

/// Last.fm bios end with a "Read more on Last.fm" link and a licence line;
/// the credits block covers both, so they're cut.
pub fn clean_bio(html: &str) -> Option<String> {
    let mut text = html.to_owned();
    if let Some(i) = text.find("<a href=\"https://www.last.fm") {
        text.truncate(i);
    }
    let mut text = clean_text(&text);
    if let Some(i) = text.find(LICENCE_LINE) {
        text.truncate(i);
    }
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn num(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

/// Parse a getInfo reply.
pub fn parse(v: &Value) -> Option<LastfmData> {
    let a = v.get("artist")?;
    // `tag` is a list, or a single object when there is one tag.
    let tags = match a.pointer("/tags/tag") {
        Some(Value::Array(list)) => list.iter().collect::<Vec<_>>(),
        Some(one @ Value::Object(_)) => vec![one],
        _ => Vec::new(),
    };
    Some(LastfmData {
        url: a.get("url").and_then(Value::as_str).map(str::to_owned),
        listeners: num(a.pointer("/stats/listeners")),
        playcount: num(a.pointer("/stats/playcount")),
        tags: tags
            .into_iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .take(MAX_TAGS)
            .collect(),
        bio: a.pointer("/bio/content").and_then(Value::as_str).and_then(clean_bio),
    })
}

impl Lastfm<'_> {
    fn url(&self, by: (&str, &str)) -> String {
        let mut u = Url::parse(API).expect("static url");
        u.query_pairs_mut()
            .append_pair("method", "artist.getinfo")
            .append_pair(by.0, by.1)
            .append_pair("api_key", self.key)
            .append_pair("lang", self.lang)
            .append_pair("autocorrect", "0")
            .append_pair("format", "json");
        u.into()
    }

    /// One call; `Ok(None)` when Last.fm doesn't know the artist.
    async fn call(&self, by: (&str, &str)) -> Fetch<LastfmData> {
        let resp = self.limiter.get(self.http, &self.url(by), &[]).await?;
        *self.max_age.lock() = resp.max_age;
        // Errors come back as JSON with a code, on 4xx or even 200.
        let v: Value = match resp.json() {
            Ok(v) => v,
            Err(_) if resp.status == 401 || resp.status == 403 => return Err(FetchError::KeyRejected),
            Err(e) => return Err(e.into()),
        };
        if let Some(code) = v.get("error").and_then(Value::as_i64) {
            return match code {
                c if KEY_ERRORS.contains(&c) => Err(FetchError::KeyRejected),
                NOT_FOUND => Ok(None),
                RATE_LIMITED => {
                    self.limiter.throttled(None);
                    Err(FetchError::Unavailable("Last.fm rate limit".into()))
                }
                c => Err(FetchError::Unavailable(format!("Last.fm error {c}"))),
            };
        }
        if !resp.ok() {
            return Err(FetchError::Unavailable(format!("Last.fm answered {}", resp.status)));
        }
        Ok(parse(&v))
    }

    /// By MusicBrainz id first, then by name.
    pub async fn lookup(&self, mbid: Option<&str>, name: &str) -> Fetch<LastfmData> {
        if let Some(m) = mbid.filter(|m| is_mbid(m))
            && let Some(d) = self.call(("mbid", m)).await?
        {
            return Ok(Some(d));
        }
        if name.trim().is_empty() {
            return Ok(None);
        }
        self.call(("artist", name)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stats_tags_and_bio() {
        let v: Value = serde_json::from_str(
            r#"{"artist":{"name":"Band","url":"https://www.last.fm/music/Band",
            "stats":{"listeners":"12345","playcount":"999999"},
            "tags":{"tag":[{"name":"indie"},{"name":"rock"}]},
            "bio":{"content":"A band from &quot;Town&quot;.\n\nSecond.  <a href=\"https://www.last.fm/music/Band\">Read more on Last.fm</a>. User-contributed text is available under the Creative Commons By-SA License; additional terms may apply."}}}"#,
        )
        .unwrap();
        let d = parse(&v).unwrap();
        assert_eq!((d.listeners, d.playcount), (Some(12345), Some(999_999)));
        assert_eq!(d.tags, ["indie", "rock"]);
        assert_eq!(d.bio.as_deref(), Some("A band from \"Town\".\n\nSecond."));
    }

    #[test]
    fn single_tag_and_empty_bio() {
        let v: Value =
            serde_json::from_str(r#"{"artist":{"tags":{"tag":{"name":"jazz"}},"bio":{"content":""}}}"#).unwrap();
        let d = parse(&v).unwrap();
        assert_eq!(d.tags, ["jazz"]);
        assert_eq!(d.bio, None);
    }
}
