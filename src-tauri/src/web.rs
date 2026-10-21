//! Requests to outside web services that take a body (scrobbling, the AI
//! provider): one small trait so tests fake every answer, and the real
//! client. Secrets go in headers flagged sensitive, or in form bodies, and
//! never into error messages or logs.

use std::time::Duration;

use async_trait::async_trait;

use crate::artistinfo::http::{HttpResponse, user_agent};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Body {
    None,
    /// `application/x-www-form-urlencoded`.
    Form(Vec<(String, String)>),
    Json(String),
}

/// One request. `Debug` leaves out headers and bodies (they carry keys).
#[derive(Clone)]
pub struct WebRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Body,
    pub timeout: Duration,
}

impl std::fmt::Debug for WebRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let host = url::Url::parse(&self.url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
        write!(f, "WebRequest({:?} {host})", self.method)
    }
}

impl WebRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self { method: Method::Get, url: url.into(), headers: Vec::new(), body: Body::None, timeout: Duration::from_secs(15) }
    }

    pub fn post_form(url: impl Into<String>, form: Vec<(String, String)>) -> Self {
        Self { method: Method::Post, body: Body::Form(form), ..Self::get(url) }
    }

    pub fn post_json(url: impl Into<String>, json: String) -> Self {
        Self { method: Method::Post, body: Body::Json(json), ..Self::get(url) }
    }

    pub fn put_json(url: impl Into<String>, json: String) -> Self {
        Self { method: Method::Put, body: Body::Json(json), ..Self::get(url) }
    }

    /// DELETE, with a JSON body when `json` isn't empty.
    pub fn delete_json(url: impl Into<String>, json: String) -> Self {
        let body = if json.is_empty() { Body::None } else { Body::Json(json) };
        Self { method: Method::Delete, body, ..Self::get(url) }
    }

    pub fn header(mut self, k: &'static str, v: impl Into<String>) -> Self {
        self.headers.push((k, v.into()));
        self
    }

    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }
}

/// Redirects for clients that send a key, token or password with every
/// request: followed only to the same host and port (and from http to https
/// there), so a server can't send them on to somewhere else.
pub fn same_origin_redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.stop();
        }
        let Some(first) = attempt.previous().first() else { return attempt.stop() };
        let next = attempt.url();
        let scheme_ok = next.scheme() == first.scheme() || (first.scheme() == "http" && next.scheme() == "https");
        let same_host = next.host_str().map(str::to_ascii_lowercase) == first.host_str().map(str::to_ascii_lowercase);
        let same_port = next.port_or_known_default() == first.port_or_known_default() || (first.scheme() == "http" && next.scheme() == "https" && next.port().is_none());
        if scheme_ok && same_host && same_port { attempt.follow() } else { attempt.stop() }
    })
}

/// A warning when `url` is plain http to an address outside this computer
/// and the home network: keys and passwords sent to it can be read on the
/// way. None for https or a home address.
pub fn plain_http_warning(url: &str) -> Option<&'static str> {
    let u = url::Url::parse(url.trim()).ok()?;
    let host = u.host_str()?;
    (u.scheme() == "http" && !crate::lidarr::paths::is_private_host(host))
        .then_some("A service is set up with a plain http address on the internet, so its key or password can be read on the way. Use its https address.")
}

/// What a server address typed without http or https is tried as: https
/// first for an address on the internet (http after it, for a server that
/// only has that), plain as typed for one at home.
pub fn address_candidates(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if raw.contains("://") {
        return vec![raw.to_string()];
    }
    let host = url::Url::parse(&format!("http://{raw}")).ok().and_then(|u| u.host_str().map(str::to_string));
    match host {
        Some(h) if !crate::lidarr::paths::is_private_host(&h) => vec![format!("https://{raw}"), raw.to_string()],
        _ => vec![raw.to_string()],
    }
}

/// A server whose certificate this computer doesn't trust.
pub const CERT_REFUSED: &str =
    "The server's certificate isn't trusted by this computer (a self-signed one, or for another name). Give the server a trusted certificate, or use its http address at home.";

/// Whether a request failed on the server's certificate.
pub fn certificate_problem(e: &reqwest::Error) -> bool {
    let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(s) = src {
        let t = s.to_string().to_ascii_lowercase();
        if t.contains("certificate") || t.contains("unknownissuer") || t.contains("self signed") || t.contains("self-signed") {
            return true;
        }
        src = s.source();
    }
    false
}

/// What to say when a server sent Platter to another address (redirects
/// are only followed on the same host), naming only where it pointed.
pub fn redirect_note(resp: &reqwest::Response) -> Option<String> {
    if !resp.status().is_redirection() {
        return None;
    }
    let to = resp.headers().get(reqwest::header::LOCATION)?.to_str().ok()?;
    let to = url::Url::parse(to).ok().or_else(|| resp.url().join(to).ok())?;
    let host = to.host_str()?;
    let port = to.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("The server sends Platter on to {}://{host}{port}. Use that address instead.", to.scheme()))
}

/// What an answer too big to read says.
pub const TOO_BIG: &str = "The answer was larger than Platter reads, so it was left unread.";

/// Read a body as it streams in, stopping past `max` bytes (after
/// decompression), so a small compressed answer can't fill memory.
pub async fn body_capped(mut resp: reqwest::Response, max: usize, net: impl Fn(reqwest::Error) -> AppError) -> AppResult<bytes::Bytes> {
    if resp.content_length().is_some_and(|n| n > max as u64) {
        return Err(AppError::Decode(TOO_BIG.into()));
    }
    let mut out = bytes::BytesMut::new();
    while let Some(chunk) = resp.chunk().await.map_err(&net)? {
        if out.len() + chunk.len() > max {
            return Err(AppError::Decode(TOO_BIG.into()));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out.freeze())
}

/// What went wrong reading a reply's body, in words. The HTTP library
/// reports any failure mid-body (the connection closing early, a reset) as
/// "error decoding response body", which reads like a bad reply when the
/// server did answer and the transfer was cut short.
pub fn body_error_text(e: reqwest::Error) -> String {
    let e = e.without_url();
    if e.is_timeout() {
        "the read timed out".into()
    } else if e.is_decode() {
        "the transfer was cut short".into()
    } else {
        e.to_string()
    }
}

/// Most one answer from a music server (Jellyfin, OpenSubsonic) may be.
pub const SERVER_MAX: usize = 256 * 1024 * 1024;

/// Most a web service's answer may be.
pub const WEB_MAX: usize = 64 * 1024 * 1024;

/// Decodes a music server's JSON reply. A tag with a broken character comes
/// back as bytes that aren't UTF-8, as a lone UTF-16 half (`\ud83d` with no
/// partner) or as a control character that wasn't escaped, and JSON rejects
/// all three. Such a reply is read again with each broken character swapped
/// for U+FFFD (a control character is escaped and kept), so one bad tag
/// doesn't stop a whole library from loading.
pub fn decode_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> serde_json::Result<T> {
    serde_json::from_slice(bytes).or_else(|e| {
        // A reply that reads fine but has the wrong shape has nothing to repair.
        if e.is_data() || e.is_io() {
            return Err(e);
        }
        let fixed = repair_strings(String::from_utf8_lossy(bytes).as_bytes());
        if fixed == bytes { Err(e) } else { serde_json::from_slice(&fixed) }
    })
}

/// Swaps lone UTF-16 halves inside JSON strings for `�` and escapes raw
/// control characters there. Everything outside strings is left alone.
fn repair_strings(bytes: &[u8]) -> Vec<u8> {
    fn half(b: &[u8], i: usize) -> Option<u16> {
        let h = b.get(i..i + 6)?;
        if h[0] != b'\\' || h[1] != b'u' || !h[2..].iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        u16::from_str_radix(std::str::from_utf8(&h[2..]).ok()?, 16).ok()
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if !in_string {
            in_string = b == b'"';
            out.push(b);
            i += 1;
            continue;
        }
        match b {
            b'"' => {
                in_string = false;
                out.push(b);
                i += 1;
            }
            // An escape: a UTF-16 half or pair, any other \u, or a two-byte one.
            b'\\' => {
                if let Some(u) = half(bytes, i) {
                    let paired = |n: Option<u16>| n.is_some_and(|n| (0xDC00..0xE000).contains(&n));
                    if (0xD800..0xDC00).contains(&u) && paired(half(bytes, i + 6)) {
                        out.extend_from_slice(&bytes[i..i + 12]);
                        i += 12;
                    } else if (0xD800..0xE000).contains(&u) {
                        out.extend_from_slice(b"\\ufffd");
                        i += 6;
                    } else {
                        out.extend_from_slice(&bytes[i..i + 6]);
                        i += 6;
                    }
                } else {
                    out.extend_from_slice(&bytes[i..(i + 2).min(bytes.len())]);
                    i += 2;
                }
            }
            0x00..=0x1f => {
                out.extend_from_slice(format!("\\u{b:04x}").as_bytes());
                i += 1;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

#[async_trait]
pub trait Web: Send + Sync {
    async fn send(&self, req: WebRequest) -> AppResult<HttpResponse>;
}

/// The real client (rustls, gzip).
pub struct ReqwestWeb {
    client: reqwest::Client,
}

impl ReqwestWeb {
    pub fn new() -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(user_agent())
            .connect_timeout(Duration::from_secs(8))
            // Lidarr's key and proxy headers ride on these requests.
            .redirect(same_origin_redirects())
            .build()
            .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl Web for ReqwestWeb {
    async fn send(&self, req: WebRequest) -> AppResult<HttpResponse> {
        let mut rb = match req.method {
            Method::Get => self.client.get(&req.url),
            Method::Post => self.client.post(&req.url),
            Method::Put => self.client.put(&req.url),
            Method::Delete => self.client.delete(&req.url),
        }
        .timeout(req.timeout);
        for (k, v) in &req.headers {
            let mut hv = reqwest::header::HeaderValue::from_str(v)
                .map_err(|_| AppError::Invalid("header has invalid characters".into()))?;
            hv.set_sensitive(true);
            rb = rb.header(*k, hv);
        }
        rb = match req.body {
            Body::None => rb,
            Body::Form(f) => {
                let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(f.iter()).finish();
                rb.header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded").body(body)
            }
            Body::Json(j) => rb.header(reqwest::header::CONTENT_TYPE, "application/json").body(j),
        };
        let resp = rb.send().await.map_err(|e| AppError::Network(e.without_url().to_string()))?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            // A day at most: a server can't park a caller for years, or
            // overflow the time it computes from this.
            .map(|s| s.min(24 * 3600));
        let body = body_capped(resp, WEB_MAX, |e| AppError::Network(e.without_url().to_string())).await?;
        Ok(HttpResponse { status, body, retry_after, ..Default::default() })
    }
}

/// Refuses every request while Go offline is on.
pub struct Switched {
    pub inner: std::sync::Arc<dyn Web>,
    pub off: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl Web for Switched {
    async fn send(&self, req: WebRequest) -> AppResult<HttpResponse> {
        if self.off.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::Network("You're offline.".into()));
        }
        self.inner.send(req).await
    }
}

#[cfg(test)]
pub mod fake {
    //! Canned answers by URL substring (and an optional body substring), and
    //! a log of every request.

    use super::*;
    use bytes::Bytes;
    use parking_lot::Mutex;

    type Route = (String, Option<String>, Result<HttpResponse, String>);

    #[derive(Default)]
    pub struct FakeWeb {
        routes: Mutex<Vec<Route>>,
        pub log: Mutex<Vec<WebRequest>>,
    }

    pub fn body_text(r: &WebRequest) -> String {
        match &r.body {
            Body::None => String::new(),
            Body::Form(f) => f.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"),
            Body::Json(j) => j.clone(),
        }
    }

    impl FakeWeb {
        pub fn new() -> Self {
            Self::default()
        }

        /// Answer requests whose URL contains `url` and (when given) whose
        /// body contains `body`. Later routes win.
        pub fn route(&self, url: &str, body: Option<&str>, status: u16, reply: &str) {
            self.routes.lock().push((
                url.into(),
                body.map(str::to_owned),
                Ok(HttpResponse { status, body: Bytes::from(reply.to_owned()), ..Default::default() }),
            ));
        }

        pub fn fail(&self, url: &str) {
            self.routes.lock().push((url.into(), None, Err("connection refused".into())));
        }

        pub fn clear(&self) {
            self.routes.lock().clear();
        }

        pub fn bodies(&self, url: &str) -> Vec<String> {
            self.log.lock().iter().filter(|r| r.url.contains(url)).map(body_text).collect()
        }
    }

    #[async_trait]
    impl Web for FakeWeb {
        async fn send(&self, req: WebRequest) -> AppResult<HttpResponse> {
            let body = body_text(&req);
            let url = req.url.clone();
            self.log.lock().push(req);
            let routes = self.routes.lock();
            let hit = routes
                .iter()
                .rev()
                .find(|(u, b, _)| url.contains(u.as_str()) && b.as_deref().is_none_or(|b| body.contains(b)));
            match hit {
                Some((_, _, Ok(r))) => Ok(r.clone()),
                Some((_, _, Err(e))) => Err(AppError::Network(e.clone())),
                None => Ok(HttpResponse { status: 404, body: Bytes::from_static(b"{}"), ..Default::default() }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_characters_become_replacement_characters() {
        let body = br#"{"a":"x\ud83dy","b":"\ud83d\ude00","c":"\\ud83d","d":"\udc00","e":"\ud83d\u0041"}"#;
        assert!(serde_json::from_slice::<serde_json::Value>(body).is_err());
        let v: serde_json::Value = decode_json(body).unwrap();
        assert_eq!(v["a"], "x\u{fffd}y");
        assert_eq!(v["b"], "\u{1f600}");
        assert_eq!(v["c"], "\\ud83d");
        assert_eq!(v["d"], "\u{fffd}");
        assert_eq!(v["e"], "\u{fffd}A");
        let latin1 = b"{\"a\":\"Beyonc\xe9\"}";
        assert!(serde_json::from_slice::<serde_json::Value>(latin1).is_err());
        let v: serde_json::Value = decode_json(latin1).unwrap();
        assert_eq!(v["a"], "Beyonc\u{fffd}");
    }

    #[test]
    fn a_raw_control_character_in_a_tag_is_kept() {
        let body = b"{\"a\":\"one\ttwo\",\"b\":[1,\n2]}";
        assert!(serde_json::from_slice::<serde_json::Value>(body).is_err());
        let v: serde_json::Value = decode_json(body).unwrap();
        assert_eq!(v["a"], "one\ttwo");
        assert_eq!(v["b"], serde_json::json!([1, 2]));
    }

    #[test]
    fn escaped_quotes_and_text_outside_strings_are_left_alone() {
        let body = b"{\"a\":\"say \\\"hi\\\" \\ud83d\",\r\n\"b\":\"x\x00y\"}";
        let v: serde_json::Value = decode_json(body).unwrap();
        assert_eq!(v["a"], "say \"hi\" \u{fffd}");
        assert_eq!(v["b"], "x\u{0}y");
    }

    #[test]
    fn a_reply_with_the_wrong_shape_keeps_its_own_error() {
        #[derive(Debug, serde::Deserialize)]
        struct Needs {
            #[allow(dead_code)]
            id: u32,
        }
        let e = decode_json::<Needs>(b"{\"name\":\"Caf\xe9\"}").unwrap_err();
        assert!(e.is_data(), "repaired, then missing its id: {e}");
        let e = decode_json::<Needs>(b"{\"id\":\"x\"}").unwrap_err();
        assert!(e.is_data(), "{e}");
    }

    #[test]
    fn a_reply_that_is_not_json_still_fails() {
        assert!(decode_json::<serde_json::Value>(b"<html>").is_err());
        assert!(decode_json::<serde_json::Value>(b"{\"a\":\"\xe9").is_err());
        assert!(decode_json::<serde_json::Value>(b"{\"a\":\"\\").is_err());
        assert!(decode_json::<serde_json::Value>(b"{\"a\":\"\\u12").is_err());
    }

    #[tokio::test]
    async fn keys_never_follow_a_redirect_to_another_host() {
        use wiremock::matchers::{header_exists, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let elsewhere = MockServer::start().await;
        Mock::given(path("/grab")).respond_with(ResponseTemplate::new(200).set_body_string("got it")).mount(&elsewhere).await;
        let home = MockServer::start().await;
        Mock::given(path("/away"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", format!("{}/grab", elsewhere.uri())))
            .mount(&home)
            .await;
        Mock::given(path("/move")).respond_with(ResponseTemplate::new(302).insert_header("location", "/here")).mount(&home).await;
        Mock::given(path("/here")).and(header_exists("x-api-key")).respond_with(ResponseTemplate::new(200).set_body_string("ok")).mount(&home).await;
        let web = ReqwestWeb::new().unwrap();
        let r = web.send(WebRequest::get(format!("{}/away", home.uri())).header("x-api-key", "secret")).await.unwrap();
        assert_eq!(r.status, 302, "not followed");
        assert!(elsewhere.received_requests().await.unwrap().is_empty());
        let r = web.send(WebRequest::get(format!("{}/move", home.uri())).header("x-api-key", "secret")).await.unwrap();
        assert_eq!((r.status, &r.body[..]), (200, &b"ok"[..]), "the same host is fine");
    }

    #[test]
    fn plain_http_is_flagged_only_off_the_home_network() {
        assert_eq!(address_candidates("music.example.com"), ["https://music.example.com", "music.example.com"]);
        assert_eq!(address_candidates("nas.local:4533"), ["nas.local:4533"]);
        assert_eq!(address_candidates("10.20.30.40:8096"), ["10.20.30.40:8096"]);
        assert_eq!(address_candidates("http://music.example.com"), ["http://music.example.com"]);
        assert!(plain_http_warning("http://tautulli.example.org:8181").is_some());
        assert!(plain_http_warning("http://203.0.113.9:4533").is_some());
        for ok in ["https://tautulli.example.org", "http://10.0.0.4:8181", "http://nas.local:4533", "http://localhost:8096", "nonsense"] {
            assert!(plain_http_warning(ok).is_none(), "{ok}");
        }
    }

    #[tokio::test]
    async fn an_answer_past_the_cap_is_left_unread() {
        use wiremock::matchers::path;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let s = MockServer::start().await;
        Mock::given(path("/big")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 4096])).mount(&s).await;
        let get = || async { reqwest::get(format!("{}/big", s.uri())).await.unwrap() };
        let net = |e: reqwest::Error| AppError::Network(e.to_string());
        assert!(matches!(body_capped(get().await, 1024, net).await, Err(AppError::Decode(m)) if m == TOO_BIG));
        assert_eq!(body_capped(get().await, 8192, net).await.unwrap().len(), 4096);
    }

    #[test]
    fn debug_never_shows_keys_or_bodies() {
        let r = WebRequest::post_form("https://ws.example.org/2.0/?k=secret", vec![("sk".into(), "secret".into())]).header("x-api-key", "secret");
        let d = format!("{r:?}");
        assert!(!d.contains("secret"), "{d}");
        assert!(d.contains("ws.example.org"));
    }
}
