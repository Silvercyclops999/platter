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

    pub fn header(mut self, k: &'static str, v: impl Into<String>) -> Self {
        self.headers.push((k, v.into()));
        self
    }

    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }
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
            .and_then(|v| v.trim().parse().ok());
        let body = resp.bytes().await.map_err(|e| AppError::Network(e.without_url().to_string()))?;
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
    fn debug_never_shows_keys_or_bodies() {
        let r = WebRequest::post_form("https://ws.example.org/2.0/?k=secret", vec![("sk".into(), "secret".into())]).header("x-api-key", "secret");
        let d = format!("{r:?}");
        assert!(!d.contains("secret"), "{d}");
        assert!(d.contains("ws.example.org"));
    }
}
