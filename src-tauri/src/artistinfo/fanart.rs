//! Fanart.tv v3 music (the user's API key, by MusicBrainz id only): background,
//! photo and logo. The key goes in the `api-key` header, never in the URL.

use serde_json::Value;

use super::http::InfoHttp;
use super::limit::Limiter;
use super::{FanartData, Fetch, FetchError, is_mbid};

pub const API: &str = "https://webservice.fanart.tv/v3/music/";

pub struct Fanart<'a> {
    pub http: &'a dyn InfoHttp,
    pub limiter: &'a Limiter,
    pub key: &'a str,
}

/// The most liked image of a kind.
fn best(v: &Value, kind: &str) -> Option<String> {
    let list = v.get(kind)?.as_array()?;
    let likes = |i: &Value| -> i64 {
        match i.get("likes") {
            Some(Value::String(s)) => s.parse().unwrap_or(0),
            Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
            _ => 0,
        }
    };
    list.iter()
        .filter(|i| i.get("url").and_then(Value::as_str).is_some_and(|u| u.starts_with("https://")))
        .max_by_key(|i| likes(i))
        .and_then(|i| i.get("url").and_then(Value::as_str))
        .map(str::to_owned)
}

pub fn parse(mbid: &str, v: &Value) -> Option<FanartData> {
    let d = FanartData {
        mbid: mbid.to_owned(),
        background: best(v, "artistbackground"),
        thumb: best(v, "artistthumb"),
        logo: best(v, "hdmusiclogo").or_else(|| best(v, "musiclogo")),
    };
    (d.background.is_some() || d.thumb.is_some() || d.logo.is_some()).then_some(d)
}

impl Fanart<'_> {
    pub async fn lookup(&self, mbid: Option<&str>) -> Fetch<FanartData> {
        let Some(mbid) = mbid.filter(|m| is_mbid(m)) else { return Ok(None) };
        let url = format!("{API}{mbid}");
        let resp = self.limiter.get(self.http, &url, &[("api-key", self.key.to_owned())]).await?;
        match resp.status {
            401 | 403 => Err(FetchError::KeyRejected),
            404 => Ok(None),
            s if (200..300).contains(&s) => Ok(parse(mbid, &resp.json()?)),
            s => Err(FetchError::Unavailable(format!("Fanart.tv answered {s}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_most_liked_image_of_each_kind() {
        let v: Value = serde_json::from_str(
            r#"{"name":"Band","artistbackground":[
                {"id":"1","url":"https://assets.fanart.tv/fanart/music/x/artistbackground/a.jpg","likes":"2"},
                {"id":"2","url":"https://assets.fanart.tv/fanart/music/x/artistbackground/b.jpg","likes":"9"}],
              "musiclogo":[{"id":"3","url":"https://assets.fanart.tv/fanart/music/x/musiclogo/l.png","likes":"0"}]}"#,
        )
        .unwrap();
        let d = parse("m", &v).unwrap();
        assert!(d.background.unwrap().ends_with("/b.jpg"));
        assert!(d.logo.unwrap().ends_with("/l.png"));
        assert_eq!(d.thumb, None);
        assert_eq!(parse("m", &serde_json::json!({"name": "x"})), None);
    }
}
