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

/// Images of one kind kept for the gallery.
pub const GALLERY_PER_KIND: usize = 6;

/// Every https image of a kind, most liked first (ties keep the service's order).
fn ranked(v: &Value, kind: &str) -> Vec<String> {
    let Some(list) = v.get(kind).and_then(Value::as_array) else { return Vec::new() };
    let likes = |i: &Value| -> i64 {
        match i.get("likes") {
            Some(Value::String(s)) => s.parse().unwrap_or(0),
            Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
            _ => 0,
        }
    };
    let mut imgs: Vec<(i64, String)> = list
        .iter()
        .filter_map(|i| {
            let u = i.get("url").and_then(Value::as_str)?;
            u.starts_with("https://").then(|| (likes(i), u.to_owned()))
        })
        .collect();
    imgs.sort_by_key(|(l, _)| std::cmp::Reverse(*l));
    imgs.into_iter().map(|(_, u)| u).collect()
}

/// The most liked image of a kind.
fn best(v: &Value, kind: &str) -> Option<String> {
    ranked(v, kind).into_iter().next()
}

pub fn parse(mbid: &str, v: &Value) -> Option<FanartData> {
    let backgrounds: Vec<String> = ranked(v, "artistbackground").into_iter().take(GALLERY_PER_KIND).collect();
    let thumbs: Vec<String> = ranked(v, "artistthumb").into_iter().take(GALLERY_PER_KIND).collect();
    let d = FanartData {
        mbid: mbid.to_owned(),
        background: backgrounds.first().cloned(),
        thumb: thumbs.first().cloned(),
        logo: best(v, "hdmusiclogo").or_else(|| best(v, "musiclogo")),
        thumbs,
        backgrounds,
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
        assert_eq!(d.backgrounds.len(), 2);
        assert!(d.backgrounds[1].ends_with("/a.jpg"));
        assert!(d.background.unwrap().ends_with("/b.jpg"));
        assert!(d.logo.unwrap().ends_with("/l.png"));
        assert_eq!(d.thumb, None);
        assert_eq!(parse("m", &serde_json::json!({"name": "x"})), None);
    }
}
