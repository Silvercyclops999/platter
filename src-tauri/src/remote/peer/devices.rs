//! The keys of this account's other devices, from its device list on
//! plex.tv (`GET /api/v2/devices`, fetched with this computer's own token).
//!
//! plex.tv lists each device the account (or Home user) is signed in on
//! with that device's own token. Only the derived keys are kept, never the
//! tokens. The list is cached for [`TTL`] and fetched again when a sender
//! isn't on it or its key doesn't match, at most once per [`REFETCH`], so a
//! device that just signed in is found without letting anyone on the
//! network make this computer hammer plex.tv.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::Instant;
use url::Url;

use super::auth::{PeerKey, Refused};
use super::server::KeySource;
use crate::error::{AppError, AppResult};
use crate::plex::Token;
use crate::plex::client::{check_status, map_send_error};
use crate::remote::service::RemoteEnv;

/// How long a fetched device list is used.
pub const TTL: Duration = Duration::from_secs(5 * 60);
/// The list is fetched again for a sender it doesn't vouch for at most
/// this often.
pub const REFETCH: Duration = Duration::from_secs(10);

/// Device id to the keys plex.tv lists for it (a device can be listed more
/// than once).
pub type DeviceKeys = HashMap<String, Vec<PeerKey>>;

/// Read `/api/v2/devices` (JSON): each entry's `clientIdentifier` and the
/// key of its `token`. Entries without either are left out.
pub fn parse_devices(body: &[u8]) -> AppResult<DeviceKeys> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| AppError::Invalid("unreadable device list".into()))?;
    let list = v.as_array().ok_or_else(|| AppError::Invalid("unreadable device list".into()))?;
    let mut out = DeviceKeys::new();
    for d in list {
        let (Some(id), Some(token)) = (d["clientIdentifier"].as_str(), d["token"].as_str()) else {
            continue;
        };
        let Ok(token) = Token::new(token) else { continue };
        if id.is_empty() {
            continue;
        }
        let key = PeerKey::from_token(&token);
        let keys = out.entry(id.to_string()).or_default();
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Ok(out)
}

#[derive(Default)]
struct Cache {
    /// The key of the token the list was fetched with (a new sign-in or
    /// another Home user starts over).
    owner: Option<PeerKey>,
    fetched: Option<Instant>,
    tried: Option<Instant>,
    devices: DeviceKeys,
}

/// This computer's key and its account's device keys.
pub struct AccountKeys {
    env: Arc<dyn RemoteEnv>,
    http: reqwest::Client,
    plex_tv: Url,
    ttl: Duration,
    refetch: Duration,
    cache: tokio::sync::Mutex<Cache>,
}

impl AccountKeys {
    /// `http` carries Platter's identity headers.
    pub fn new(env: Arc<dyn RemoteEnv>, http: reqwest::Client, plex_tv: Url) -> Self {
        Self::with_times(env, http, plex_tv, TTL, REFETCH)
    }

    pub fn with_times(env: Arc<dyn RemoteEnv>, http: reqwest::Client, plex_tv: Url, ttl: Duration, refetch: Duration) -> Self {
        Self { env, http, plex_tv, ttl, refetch, cache: Default::default() }
    }

    async fn fetch(&self, token: &Token) -> AppResult<DeviceKeys> {
        let url = self.plex_tv.join("/api/v2/devices").map_err(|e| AppError::Internal(e.to_string()))?;
        let resp = self
            .http
            .get(url)
            .header("accept", "application/json")
            .header("x-plex-token", token.header_value()?)
            .send()
            .await
            .map_err(map_send_error)?;
        let body = check_status(resp).await?.bytes().await.map_err(map_send_error)?;
        parse_devices(&body)
    }

    /// Fetch the list now if it's old (keeps it warm so the first request
    /// from another device doesn't wait on plex.tv).
    pub async fn warm(&self) {
        let _ = self.device_keys("", false).await;
    }
}

#[async_trait]
impl KeySource for AccountKeys {
    async fn key(&self) -> Option<PeerKey> {
        self.env.account_token().await.map(|t| PeerKey::from_token(&t))
    }

    async fn device_keys(&self, from: &str, fresh: bool) -> Result<Vec<PeerKey>, Refused> {
        let token = self.env.account_token().await.ok_or(Refused::NoKey)?;
        let owner = PeerKey::from_token(&token);
        let mut c = self.cache.lock().await;
        if c.owner.as_ref() != Some(&owner) {
            *c = Cache { owner: Some(owner), ..Default::default() };
        }
        let old = c.fetched.is_none_or(|t| t.elapsed() >= self.ttl);
        let may_retry = c.tried.is_none_or(|t| t.elapsed() >= self.refetch);
        if (old || fresh) && may_retry {
            c.tried = Some(Instant::now());
            match self.fetch(&token).await {
                Ok(devices) => {
                    c.devices = devices;
                    c.fetched = Some(Instant::now());
                }
                Err(e) => {
                    tracing::info!(kind = e.kind(), "couldn't fetch the account's device list");
                    if c.fetched.is_none() {
                        return Err(Refused::CantCheck);
                    }
                }
            }
        } else if c.fetched.is_none() {
            return Err(Refused::CantCheck);
        }
        Ok(c.devices.get(from).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(t: &str) -> PeerKey {
        PeerKey::from_token(&Token::new(t).unwrap())
    }

    #[test]
    fn the_list_gives_keys_not_tokens() {
        let body = serde_json::json!([
            {"clientIdentifier": "mac", "product": "Platter", "token": "mac-token"},
            {"clientIdentifier": "tv", "token": "tv-token-1"},
            {"clientIdentifier": "tv", "token": "tv-token-2"},
            {"clientIdentifier": "tv", "token": "tv-token-2"},
            {"clientIdentifier": "no-token"},
            {"clientIdentifier": "", "token": "x"},
            {"token": "orphan"}
        ]);
        let d = parse_devices(body.to_string().as_bytes()).unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d["mac"], vec![key("mac-token")]);
        assert_eq!(d["tv"], vec![key("tv-token-1"), key("tv-token-2")]);
        assert!(parse_devices(b"<MediaContainer/>").is_err());
        assert!(parse_devices(b"{}").is_err());
    }

    use crate::remote::service::ServerCtx;
    use parking_lot::Mutex;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct Env(Mutex<Option<&'static str>>);

    #[async_trait]
    impl RemoteEnv for Env {
        async fn server(&self) -> AppResult<ServerCtx> {
            Err(AppError::Invalid("none".into()))
        }
        async fn account_token(&self) -> Option<Token> {
            self.0.lock().map(|t| Token::new(t).unwrap())
        }
    }

    async fn list(server: &MockServer, token: &str, devices: &[(&str, &str)]) {
        let body: Vec<_> = devices.iter().map(|(id, t)| serde_json::json!({"clientIdentifier": id, "token": t})).collect();
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/devices"))
            .and(header("x-plex-token", token))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    async fn fetches(server: &MockServer) -> usize {
        server.received_requests().await.unwrap().len()
    }

    fn keys(server: &MockServer, env: Arc<Env>, refetch: Duration) -> AccountKeys {
        AccountKeys::with_times(env, reqwest::Client::new(), Url::parse(&server.uri()).unwrap(), Duration::from_secs(300), refetch)
    }

    #[tokio::test]
    async fn the_list_is_cached_and_fetched_again_for_a_new_device() {
        let server = MockServer::start().await;
        list(&server, "pc-token", &[("pc", "pc-token")]).await;
        let env = Arc::new(Env(Mutex::new(Some("pc-token"))));
        let k = keys(&server, env.clone(), Duration::ZERO);
        assert_eq!(k.key().await, Some(key("pc-token")));
        k.warm().await;
        assert_eq!(fetches(&server).await, 1);
        // Within the cache time nothing is fetched for a known device.
        assert_eq!(k.device_keys("pc", false).await, Ok(vec![key("pc-token")]));
        assert_eq!(fetches(&server).await, 1);
        // The Mac signs in: it isn't in the cached list, so the next check
        // fetches it again and finds it.
        list(&server, "pc-token", &[("pc", "pc-token"), ("mac", "mac-token")]).await;
        assert_eq!(k.device_keys("mac", false).await, Ok(vec![]));
        assert_eq!(k.device_keys("mac", true).await, Ok(vec![key("mac-token")]));
        assert_eq!(fetches(&server).await, 1, "one fetch since the reset");
    }

    #[tokio::test]
    async fn refetches_are_limited_and_a_new_sign_in_starts_over() {
        let server = MockServer::start().await;
        list(&server, "pc-token", &[("pc", "pc-token")]).await;
        let env = Arc::new(Env(Mutex::new(Some("pc-token"))));
        let k = keys(&server, env.clone(), Duration::from_secs(60));
        k.warm().await;
        // Someone keeps sending from an unknown id: only the first fetch.
        for _ in 0..5 {
            assert_eq!(k.device_keys("stranger", true).await, Ok(vec![]));
        }
        assert_eq!(fetches(&server).await, 1);
        // Signed in again (or as another Home user): that token's list.
        *env.0.lock() = Some("home-token");
        list(&server, "home-token", &[("mac", "home-mac-token")]).await;
        assert_eq!(k.device_keys("mac", false).await, Ok(vec![key("home-mac-token")]));
        assert_eq!(k.device_keys("pc", false).await, Ok(vec![]));
        // Signed out.
        *env.0.lock() = None;
        assert_eq!(k.device_keys("mac", false).await, Err(Refused::NoKey));
    }

    #[tokio::test]
    async fn plex_tv_down_means_it_cant_check_until_it_has_a_list() {
        let server = MockServer::start().await;
        let env = Arc::new(Env(Mutex::new(Some("pc-token"))));
        let k = keys(&server, env.clone(), Duration::ZERO);
        assert_eq!(k.device_keys("mac", true).await, Err(Refused::CantCheck));
        // Once it has one, a failed refetch keeps the old list.
        list(&server, "pc-token", &[("mac", "mac-token")]).await;
        assert_eq!(k.device_keys("mac", true).await, Ok(vec![key("mac-token")]));
        server.reset().await;
        assert_eq!(k.device_keys("mac", true).await, Ok(vec![key("mac-token")]));
    }
}
