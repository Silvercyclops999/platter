//! plex.tv account API: PIN sign-in, the signed-in user,
//! and the resources (servers) list. The base URL is injectable for tests.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::watch;
use url::Url;

use super::client::{check_status, map_send_error};
use super::discovery::Resource;
use super::identity::ClientIdentity;
use super::lenient;
use super::token::Token;
use crate::error::{AppError, AppResult};

pub const PLEX_TV: &str = "https://plex.tv";
pub const AUTH_APP: &str = "https://app.plex.tv/auth";
pub const PIN_POLL_INTERVAL: Duration = Duration::from_secs(2);
pub const PIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPin {
    #[serde(deserialize_with = "lenient::string")]
    id: String,
    code: String,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    auth_token: Option<String>,
}

/// A created PIN. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub id: String,
    pub code: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexUser {
    #[serde(default, deserialize_with = "lenient::string")]
    pub uuid: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub title: String,
    /// Profile picture on plex.tv (`/users/<id>/avatar?c=<n>`, which
    /// redirects to an image on assets.plex.tv). Public; needs no token.
    #[serde(default, deserialize_with = "lenient::string")]
    pub thumb: String,
}

impl PlexUser {
    /// The profile picture as an art path for the art cache, or None when
    /// it's missing or not on a plex.tv host. Any token in the URL is
    /// dropped so the webview never sees one.
    pub fn avatar_art_path(&self) -> Option<String> {
        let mut u = Url::parse(self.thumb.trim()).ok()?;
        let kept: Vec<(String, String)> = u
            .query_pairs()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("x-plex-token"))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if kept.is_empty() {
            u.set_query(None);
        } else {
            u.query_pairs_mut().clear().extend_pairs(kept);
        }
        u.set_fragment(None);
        let path = format!("{}{}", crate::artistinfo::art::EXT_PREFIX, u);
        let ok = crate::artistinfo::art::ext_url(&path)
            .is_some_and(|e| e.host_str().is_some_and(|h| h == "plex.tv" || h.ends_with(".plex.tv")));
        ok.then_some(path)
    }

    pub fn display_name(&self) -> String {
        if !self.username.is_empty() {
            self.username.clone()
        } else {
            self.title.clone()
        }
    }
}

/// Client for plex.tv.
#[derive(Debug, Clone)]
pub struct PlexTv {
    http: reqwest::Client,
    base: Url,
    identity: Arc<ClientIdentity>,
}

impl PlexTv {
    pub fn new(identity: Arc<ClientIdentity>) -> AppResult<Self> {
        Self::with_base(identity, PLEX_TV)
    }

    pub fn with_base(identity: Arc<ClientIdentity>, base: &str) -> AppResult<Self> {
        let base = Url::parse(base).map_err(|e| AppError::Invalid(format!("plex.tv url: {e}")))?;
        let http = reqwest::Client::builder()
            .default_headers(identity.headers()?)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
        Ok(PlexTv {
            http,
            base,
            identity,
        })
    }

    pub fn identity(&self) -> &ClientIdentity {
        &self.identity
    }

    fn url(&self, path: &str) -> AppResult<Url> {
        self.base
            .join(path)
            .map_err(|e| AppError::Internal(format!("url: {e}")))
    }

    pub(crate) fn url_for(&self, path: &str) -> AppResult<Url> {
        self.url(path)
    }

    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// `POST /api/v2/pins?strong=true`
    pub async fn create_pin(&self) -> AppResult<Pin> {
        let resp = self
            .http
            .post(self.url("/api/v2/pins")?)
            .query(&[("strong", "true")])
            .send()
            .await
            .map_err(map_send_error)?;
        let raw: RawPin = check_status(resp)
            .await?
            .json()
            .await
            .map_err(map_send_error)?;
        Ok(Pin {
            id: raw.id,
            code: raw.code,
            expires_at: raw.expires_at,
        })
    }

    /// The browser URL the user approves the PIN at. Contains only the client
    /// id and the PIN code.
    pub fn auth_url(&self, pin: &Pin) -> String {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("clientID", &self.identity.client_id)
            .append_pair("code", &pin.code)
            .append_pair("context[device][product]", &self.identity.product)
            .finish();
        format!("{AUTH_APP}#?{query}")
    }

    /// `GET /api/v2/pins/{id}` once. `Ok(None)` = not approved yet.
    /// A 404 means the PIN expired or was removed.
    pub async fn check_pin(&self, pin_id: &str) -> AppResult<Option<Token>> {
        let resp = self
            .http
            .get(self.url(&format!("/api/v2/pins/{pin_id}"))?)
            .send()
            .await
            .map_err(map_send_error)?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AppError::Auth(
                "The sign-in code expired. Try again.".into(),
            ));
        }
        let raw: RawPin = check_status(resp)
            .await?
            .json()
            .await
            .map_err(map_send_error)?;
        Ok(raw.auth_token.and_then(|t| Token::new(t).ok()))
    }

    /// Poll until approved, expired, timed out or cancelled (send `true` on
    /// the watch channel, or drop the sender). Transient network errors are
    /// retried; auth errors (expired PIN) end the poll.
    pub async fn poll_pin(
        &self,
        pin_id: &str,
        interval: Duration,
        timeout: Duration,
        mut cancel: watch::Receiver<bool>,
    ) -> AppResult<Token> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if *cancel.borrow() {
                return Err(cancelled());
            }
            match self.check_pin(pin_id).await {
                Ok(Some(token)) => return Ok(token),
                Ok(None) => {}
                Err(AppError::Network(_)) | Err(AppError::Server(_)) => {
                    tracing::debug!("pin poll: transient error, retrying");
                }
                Err(e) => return Err(e),
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(AppError::Auth("Sign-in timed out. Try again.".into()));
            }
            let wait = interval.min(deadline - now);
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() {
                        return Err(cancelled());
                    }
                }
            }
        }
    }

    /// `GET /api/v2/user`
    pub async fn user(&self, token: &Token) -> AppResult<PlexUser> {
        let resp = self
            .http
            .get(self.url("/api/v2/user")?)
            .header("x-plex-token", token.header_value()?)
            .send()
            .await
            .map_err(map_send_error)?;
        check_status(resp)
            .await?
            .json()
            .await
            .map_err(map_send_error)
    }

    /// `GET /api/v2/resources?includeHttps=1&includeRelay=1` (all resources;
    /// filter with [`Resource::is_server`]).
    pub async fn resources(&self, token: &Token) -> AppResult<Vec<Resource>> {
        let resp = self
            .http
            .get(self.url("/api/v2/resources")?)
            .query(&[("includeHttps", "1"), ("includeRelay", "1")])
            .header("x-plex-token", token.header_value()?)
            .send()
            .await
            .map_err(map_send_error)?;
        check_status(resp)
            .await?
            .json()
            .await
            .map_err(map_send_error)
    }
}

pub fn cancelled() -> AppError {
    AppError::Auth("Sign-in cancelled.".into())
}

pub fn is_cancelled(e: &AppError) -> bool {
    matches!(e, AppError::Auth(m) if m == "Sign-in cancelled.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn tv(server: &MockServer) -> PlexTv {
        PlexTv::with_base(Arc::new(ClientIdentity::with_id("cid-1")), &server.uri()).unwrap()
    }

    fn pin_json(token: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "id": 4242, "code": "abcd1234", "expiresIn": 1800,
            "expiresAt": "2026-10-13T12:30:00Z", "authToken": token
        })
    }

    #[tokio::test]
    async fn create_pin_sends_identity_and_strong() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/pins"))
            .and(query_param("strong", "true"))
            .and(header("x-plex-client-identifier", "cid-1"))
            .and(header("x-plex-product", "Platter"))
            .and(header("accept", "application/json"))
            .respond_with(ResponseTemplate::new(201).set_body_json(pin_json(None)))
            .expect(1)
            .mount(&server)
            .await;
        let t = tv(&server);
        let pin = t.create_pin().await.unwrap();
        assert_eq!(pin.id, "4242");
        assert_eq!(pin.code, "abcd1234");
        let url = t.auth_url(&pin);
        assert!(url.starts_with("https://app.plex.tv/auth#?"));
        assert!(url.contains("clientID=cid-1"));
        assert!(url.contains("code=abcd1234"));
        assert!(url.contains("context%5Bdevice%5D%5Bproduct%5D=Platter"));
    }

    #[tokio::test]
    async fn poll_returns_token_once_approved() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/pins/4242"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pin_json(None)))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/pins/4242"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pin_json(Some("tok-approved"))))
            .mount(&server)
            .await;
        let (_tx, rx) = watch::channel(false);
        let token = tv(&server)
            .poll_pin(
                "4242",
                Duration::from_millis(10),
                Duration::from_secs(5),
                rx,
            )
            .await
            .unwrap();
        assert_eq!(token.expose(), "tok-approved");
    }

    #[tokio::test]
    async fn poll_reports_expired_pin() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/pins/4242"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let (_tx, rx) = watch::channel(false);
        let err = tv(&server)
            .poll_pin(
                "4242",
                Duration::from_millis(10),
                Duration::from_secs(5),
                rx,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::Auth(ref m) if m.contains("expired")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn poll_times_out() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/pins/4242"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pin_json(None)))
            .mount(&server)
            .await;
        let (_tx, rx) = watch::channel(false);
        let err = tv(&server)
            .poll_pin(
                "4242",
                Duration::from_millis(10),
                Duration::from_millis(80),
                rx,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::Auth(ref m) if m.contains("timed out")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn poll_can_be_cancelled() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/pins/4242"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pin_json(None)))
            .mount(&server)
            .await;
        let (tx, rx) = watch::channel(false);
        let t = tv(&server);
        let task = tokio::spawn(async move {
            t.poll_pin("4242", Duration::from_secs(2), Duration::from_secs(600), rx)
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send(true).unwrap();
        let err = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("cancel should end the poll promptly")
            .unwrap()
            .unwrap_err();
        assert!(is_cancelled(&err));
    }

    #[tokio::test]
    async fn poll_survives_a_transient_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/pins/4242"))
            .respond_with(ResponseTemplate::new(502))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/pins/4242"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pin_json(Some("tok-2"))))
            .mount(&server)
            .await;
        let (_tx, rx) = watch::channel(false);
        let token = tv(&server)
            .poll_pin(
                "4242",
                Duration::from_millis(10),
                Duration::from_secs(5),
                rx,
            )
            .await
            .unwrap();
        assert_eq!(token.expose(), "tok-2");
    }

    #[tokio::test]
    async fn user_and_resources_send_token_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/user"))
            .and(header("x-plex-token", "acct"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"id": 1, "uuid": "u1", "username": "someone", "title": "Some One"}),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/resources"))
            .and(query_param("includeHttps", "1"))
            .and(query_param("includeRelay", "1"))
            .and(header("x-plex-token", "acct"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;
        let t = tv(&server);
        let tok = Token::new("acct").unwrap();
        assert_eq!(t.user(&tok).await.unwrap().display_name(), "someone");
        assert!(t.resources(&tok).await.unwrap().is_empty());
    }

    fn user_with(thumb: &str) -> PlexUser {
        PlexUser {
            uuid: "u".into(),
            username: "someone".into(),
            title: String::new(),
            thumb: thumb.into(),
        }
    }

    #[test]
    fn avatar_becomes_an_ext_art_path_without_token() {
        assert_eq!(
            user_with("https://plex.tv/users/abc123/avatar?c=1700000000")
                .avatar_art_path()
                .as_deref(),
            Some("/ext/https://plex.tv/users/abc123/avatar?c=1700000000")
        );
        assert_eq!(
            user_with("https://plex.tv/users/abc/avatar?X-Plex-Token=secret&c=5")
                .avatar_art_path()
                .as_deref(),
            Some("/ext/https://plex.tv/users/abc/avatar?c=5")
        );
        assert_eq!(
            user_with("https://assets.plex.tv/avatars/a.png?x-plex-token=secret")
                .avatar_art_path()
                .as_deref(),
            Some("/ext/https://assets.plex.tv/avatars/a.png")
        );
    }

    #[test]
    fn avatar_off_plex_or_missing_is_none() {
        for bad in [
            "",
            "not a url",
            "http://plex.tv/users/a/avatar",
            "https://evil.example/a.png",
            "https://plex.tv.evil.example/a.png",
        ] {
            assert_eq!(user_with(bad).avatar_art_path(), None, "{bad}");
        }
    }

    #[test]
    fn user_thumb_may_be_null_or_missing() {
        let u: PlexUser = serde_json::from_value(serde_json::json!({"username": "a", "thumb": null})).unwrap();
        assert_eq!(u.avatar_art_path(), None);
        let u: PlexUser = serde_json::from_value(serde_json::json!({"username": "a"})).unwrap();
        assert_eq!(u.thumb, "");
    }

    #[tokio::test]
    async fn revoked_token_is_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let err = tv(&server)
            .user(&Token::new("acct").unwrap())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "auth");
    }
}
