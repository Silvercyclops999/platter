//! Last.fm `artist.getInfo` (the user's own API key): listeners, plays, tags
//! and a bio. Images from Last.fm aren't used (its terms leave them out).
//! `artist.getTopTracks` is the Popular list when ListenBrainz has none.

use serde_json::Value;
use url::Url;

use super::http::InfoHttp;
use super::limit::Limiter;
use super::{Fetch, FetchError, LastfmData, clean_text, is_mbid};
use crate::db::popular::GlobalTop;

pub const API: &str = "https://ws.audioscrobbler.com/2.0/";

/// Error codes that mean the key is no good.
const KEY_ERRORS: &[i64] = &[10, 26];
/// "The artist you supplied could not be found".
const NOT_FOUND: i64 = 6;
/// Rate limit exceeded.
const RATE_LIMITED: i64 = 29;

const MAX_TAGS: usize = 8;
/// Top tracks asked for.
const TOP_N: &str = "100";

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
        mbid: a.get("mbid").and_then(Value::as_str).filter(|m| is_mbid(m)).map(str::to_owned),
    })
}

/// Parse a getTopTracks reply. `track` is a list, or one object.
pub fn parse_top(v: &Value) -> Option<GlobalTop> {
    let tracks = match v.pointer("/toptracks/track")? {
        Value::Array(list) => list.iter().collect::<Vec<_>>(),
        one @ Value::Object(_) => vec![one],
        _ => return None,
    };
    let pairs: Vec<(String, u64)> = tracks
        .into_iter()
        .filter_map(|t| {
            let name = t.get("name").and_then(Value::as_str)?.trim();
            (!name.is_empty()).then(|| (name.to_owned(), num(t.get("playcount")).unwrap_or(0)))
        })
        .collect();
    (!pairs.is_empty()).then(|| GlobalTop::from_pairs("lastfm", pairs))
}

impl Lastfm<'_> {
    fn url(&self, method: &str, by: (&str, &str)) -> String {
        let mut u = Url::parse(API).expect("static url");
        u.query_pairs_mut()
            .append_pair("method", method)
            .append_pair(by.0, by.1)
            .append_pair("api_key", self.key)
            .append_pair("lang", self.lang)
            .append_pair("autocorrect", "0")
            .append_pair("format", "json");
        if method == "artist.gettoptracks" {
            u.query_pairs_mut().append_pair("limit", TOP_N);
        }
        u.into()
    }

    /// One call; `Ok(None)` when Last.fm doesn't know the artist.
    async fn call(&self, method: &str, by: (&str, &str)) -> Fetch<Value> {
        let resp = self.limiter.get(self.http, &self.url(method, by), &[]).await?;
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
        Ok(Some(v))
    }

    /// By MusicBrainz id first, then by name.
    async fn by_mbid_or_name<T>(&self, method: &str, mbid: Option<&str>, name: &str, parse: impl Fn(&Value) -> Option<T>) -> Fetch<T> {
        if let Some(m) = mbid.filter(|m| is_mbid(m))
            && let Some(d) = self.call(method, ("mbid", m)).await?.as_ref().and_then(&parse)
        {
            return Ok(Some(d));
        }
        if name.trim().is_empty() {
            return Ok(None);
        }
        Ok(self.call(method, ("artist", name)).await?.as_ref().and_then(parse))
    }

    /// Artist info, by MusicBrainz id first, then by name.
    pub async fn lookup(&self, mbid: Option<&str>, name: &str) -> Fetch<LastfmData> {
        self.by_mbid_or_name("artist.getinfo", mbid, name, parse).await
    }

    /// The artist's top tracks, by MusicBrainz id first, then by name.
    pub async fn top_tracks(&self, mbid: Option<&str>, name: &str) -> Fetch<GlobalTop> {
        self.by_mbid_or_name("artist.gettoptracks", mbid, name, parse_top).await
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
    fn parses_top_tracks() {
        let v: Value = serde_json::from_str(
            r#"{"toptracks":{"track":[{"name":"Hit","playcount":"900"},{"name":"Hit - Remastered","playcount":"100"},{"name":"B","playcount":50}]}}"#,
        )
        .unwrap();
        let g = parse_top(&v).unwrap();
        assert_eq!(g.from, "lastfm");
        let got: Vec<_> = g.songs.iter().map(|s| (s.title.as_str(), s.listens)).collect();
        assert_eq!(got, [("Hit", 1000), ("B", 50)]);
        let one: Value = serde_json::from_str(r#"{"toptracks":{"track":{"name":"Only","playcount":"1"}}}"#).unwrap();
        assert_eq!(parse_top(&one).unwrap().songs.len(), 1);
        let none: Value = serde_json::from_str(r#"{"toptracks":{"track":[]}}"#).unwrap();
        assert_eq!(parse_top(&none), None);
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
