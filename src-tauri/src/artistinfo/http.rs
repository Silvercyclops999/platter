//! The HTTP side of the info sources: one small trait so tests can fake every
//! response, and the real client with a polite user agent.

use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;

use crate::error::{AppError, AppResult};

/// Names the app and where to reach the project, as the Wikimedia, Last.fm
/// and MusicBrainz guidelines ask.
pub fn user_agent() -> String {
    format!("Platter/{} (+{})", env!("CARGO_PKG_VERSION"), crate::settings::REPO_URL)
}

const TIMEOUT: Duration = Duration::from_secs(15);

/// What the sources need from a response.
#[derive(Debug, Clone, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Bytes,
    /// `Retry-After` in seconds, when sent.
    pub retry_after: Option<u64>,
    /// `Cache-Control: max-age` in seconds, when sent.
    pub max_age: Option<u64>,
    pub content_type: Option<String>,
}

impl HttpResponse {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> AppResult<T> {
        serde_json::from_slice(&self.body).map_err(|e| AppError::Decode(format!("info source reply: {e}")))
    }
}

/// A GET with extra headers. Secrets go in headers flagged sensitive by the
/// real client, never in error messages.
#[async_trait]
pub trait InfoHttp: Send + Sync {
    async fn get(&self, url: &str, headers: &[(&'static str, String)]) -> AppResult<HttpResponse>;
}

/// The real client (rustls, gzip, 15 s timeout).
pub struct ReqwestHttp {
    client: reqwest::Client,
}

impl ReqwestHttp {
    pub fn new() -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(user_agent())
            .timeout(TIMEOUT)
            .connect_timeout(Duration::from_secs(8))
            .build()
            .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
        Ok(Self { client })
    }
}

fn header_secs(h: &reqwest::header::HeaderMap, name: reqwest::header::HeaderName) -> Option<u64> {
    h.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// `max-age=N` from a Cache-Control value.
pub fn parse_max_age(v: &str) -> Option<u64> {
    v.split(',').find_map(|part| {
        let (k, n) = part.trim().split_once('=')?;
        if k.trim().eq_ignore_ascii_case("max-age") { n.trim().parse().ok() } else { None }
    })
}

#[async_trait]
impl InfoHttp for ReqwestHttp {
    async fn get(&self, url: &str, headers: &[(&'static str, String)]) -> AppResult<HttpResponse> {
        let mut req = self.client.get(url);
        for (k, v) in headers {
            let mut hv = reqwest::header::HeaderValue::from_str(v)
                .map_err(|_| AppError::Invalid("header has invalid characters".into()))?;
            hv.set_sensitive(true);
            req = req.header(*k, hv);
        }
        // Without the URL: some sources carry the key in it.
        let resp = req.send().await.map_err(|e| AppError::Network(e.without_url().to_string()))?;
        let status = resp.status().as_u16();
        let h = resp.headers();
        let retry_after = header_secs(h, reqwest::header::RETRY_AFTER);
        let max_age = h
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_max_age);
        let content_type = h
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = resp.bytes().await.map_err(|e| AppError::Network(e.without_url().to_string()))?;
        Ok(HttpResponse { status, body, retry_after, max_age, content_type })
    }
}

#[cfg(test)]
pub mod fake {
    //! Canned responses by URL substring, and a log of every request.

    use super::*;
    use parking_lot::Mutex;

    /// One logged request: URL and headers.
    pub type Request = (String, Vec<(&'static str, String)>);

    #[derive(Default)]
    pub struct FakeHttp {
        routes: Mutex<Vec<(String, Result<HttpResponse, String>)>>,
        pub log: Mutex<Vec<Request>>,
    }

    impl FakeHttp {
        pub fn new() -> Self {
            Self::default()
        }

        /// Answer requests whose URL contains `needle` (first match wins;
        /// later calls with the same needle replace it).
        pub fn route(&self, needle: &str, status: u16, body: &str) {
            self.set(needle, Ok(HttpResponse { status, body: Bytes::from(body.to_owned()), ..Default::default() }));
        }

        pub fn route_resp(&self, needle: &str, resp: HttpResponse) {
            self.set(needle, Ok(resp));
        }

        /// Requests matching `needle` fail like a dropped connection.
        pub fn fail(&self, needle: &str) {
            self.set(needle, Err("connection refused".into()));
        }

        fn set(&self, needle: &str, r: Result<HttpResponse, String>) {
            let mut routes = self.routes.lock();
            routes.retain(|(n, _)| n != needle);
            routes.push((needle.to_owned(), r));
        }

        pub fn calls(&self, needle: &str) -> usize {
            self.log.lock().iter().filter(|(u, _)| u.contains(needle)).count()
        }

        pub fn urls(&self) -> Vec<String> {
            self.log.lock().iter().map(|(u, _)| u.clone()).collect()
        }
    }

    #[async_trait]
    impl InfoHttp for FakeHttp {
        async fn get(&self, url: &str, headers: &[(&'static str, String)]) -> AppResult<HttpResponse> {
            self.log.lock().push((url.to_owned(), headers.to_vec()));
            let routes = self.routes.lock();
            match routes.iter().find(|(n, _)| url.contains(n.as_str())) {
                Some((_, Ok(r))) => Ok(r.clone()),
                Some((_, Err(e))) => Err(AppError::Network(e.clone())),
                None => Ok(HttpResponse { status: 404, body: Bytes::from_static(b"{}"), ..Default::default() }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_names_the_app_and_project() {
        let ua = user_agent();
        assert!(ua.starts_with("Platter/"));
        assert!(ua.contains("github.com/Silvercyclops999/platter"));
    }

    #[test]
    fn reads_max_age() {
        assert_eq!(parse_max_age("public, max-age=86400"), Some(86400));
        assert_eq!(parse_max_age("no-store"), None);
    }
}
