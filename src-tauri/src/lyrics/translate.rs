//! Lyrics translation through a service the user sets in Settings (a
//! LibreTranslate-compatible address: `POST /translate`). There's no
//! default service: with none set, nothing is sent anywhere.

use serde::Deserialize;
use serde_json::json;

use crate::error::{AppError, AppResult};

pub const NOT_SET: &str = "Set a translation service in Settings first.";
pub const UNREACHABLE: &str = "Couldn't reach the translation service.";
/// Most lines sent in one go.
pub const MAX_LINES: usize = 400;

/// A translation service address: http(s), no query, trailing slash dropped.
pub fn service_url(s: &str) -> Option<String> {
    let u = url::Url::parse(s.trim()).ok()?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() || u.query().is_some() {
        return None;
    }
    Some(u.as_str().trim_end_matches('/').to_string())
}

/// A language code like `en`, `pt` or `zh-Hant`.
pub fn language(s: &str) -> Option<String> {
    let s = s.trim();
    let mut parts = s.split('-');
    let main = parts.next()?;
    let ok_main = (2..=3).contains(&main.len()) && main.chars().all(|c| c.is_ascii_lowercase());
    let ok_rest = parts.all(|p| (2..=4).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()));
    (ok_main && ok_rest).then(|| s.to_string())
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Translated {
    Many(Vec<String>),
    One(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Answer {
    translated_text: Translated,
}

/// The lines in `target`; blank lines stay blank and aren't sent.
pub async fn translate(http: &reqwest::Client, service: &str, target: &str, lines: &[String]) -> AppResult<Vec<String>> {
    let base = service_url(service).ok_or_else(|| AppError::Invalid(NOT_SET.into()))?;
    if lines.len() > MAX_LINES {
        return Err(AppError::Invalid("Those lyrics are too long to translate.".into()));
    }
    let wanted: Vec<(usize, &str)> = lines.iter().enumerate().filter(|(_, l)| !l.trim().is_empty()).map(|(i, l)| (i, l.as_str())).collect();
    let mut out = vec![String::new(); lines.len()];
    if wanted.is_empty() {
        return Ok(out);
    }
    let q: Vec<&str> = wanted.iter().map(|(_, l)| *l).collect();
    let body = json!({ "q": q, "source": "auto", "target": target, "format": "text" });
    let res = http
        .post(format!("{base}/translate"))
        .json(&body)
        .send()
        .await
        .map_err(|_| AppError::Network(UNREACHABLE.into()))?;
    let status = res.status();
    if !status.is_success() {
        return Err(AppError::Server(format!("The translation service said no (HTTP {}).", status.as_u16())));
    }
    let answer: Answer = res
        .json()
        .await
        .map_err(|_| AppError::Decode("The translation service sent something that isn't a translation.".into()))?;
    let got = match answer.translated_text {
        Translated::Many(v) => v,
        Translated::One(s) => vec![s],
    };
    if got.len() != wanted.len() {
        return Err(AppError::Decode("The translation service sent back the wrong number of lines.".into()));
    }
    for ((i, _), t) in wanted.into_iter().zip(got) {
        out[i] = t;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn addresses_and_languages_are_checked() {
        assert_eq!(service_url(" https://tr.example.com/ ").as_deref(), Some("https://tr.example.com"));
        assert_eq!(service_url("http://localhost:5000").as_deref(), Some("http://localhost:5000"));
        for bad in ["", "ftp://x", "tr.example.com", "https://x/?k=1"] {
            assert!(service_url(bad).is_none(), "{bad}");
        }
        assert_eq!(language("en").as_deref(), Some("en"));
        assert_eq!(language("zh-Hant").as_deref(), Some("zh-Hant"));
        assert!(language("English").is_none() && language("").is_none() && language("e n").is_none());
    }

    #[tokio::test]
    async fn sends_the_lines_and_keeps_blanks_in_place() {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/translate"))
            .and(body_partial_json(json!({ "q": ["愛してる", "さよなら"], "source": "auto", "target": "en" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "translatedText": ["I love you", "Goodbye"] })))
            .expect(1)
            .mount(&s)
            .await;
        let got = translate(&reqwest::Client::new(), &s.uri(), "en", &lines(&["愛してる", "", "さよなら"])).await.unwrap();
        assert_eq!(got, lines(&["I love you", "", "Goodbye"]));
    }

    #[tokio::test]
    async fn not_set_down_refused_and_garbled_say_so() {
        let http = reqwest::Client::new();
        let e = translate(&http, "", "en", &lines(&["x"])).await.unwrap_err();
        assert_eq!(e.to_string(), format!("invalid: {NOT_SET}"));
        let gone = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://127.0.0.1:{}", l.local_addr().unwrap().port())
        };
        assert!(matches!(translate(&http, &gone, "en", &lines(&["x"])).await, Err(AppError::Network(_))));
        let s = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(403)).mount(&s).await;
        let e = translate(&http, &s.uri(), "en", &lines(&["x"])).await.unwrap_err();
        assert!(e.to_string().contains("HTTP 403"), "{e}");
        let s = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "translatedText": ["a", "b"] }))).mount(&s).await;
        assert!(matches!(translate(&http, &s.uri(), "en", &lines(&["x"])).await, Err(AppError::Decode(_))));
        // Nothing to translate: nothing sent.
        assert_eq!(translate(&http, &gone, "en", &lines(&["", " "])).await.unwrap(), lines(&["", ""]));
    }
}
