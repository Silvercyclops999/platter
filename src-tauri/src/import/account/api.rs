//! The service's Web API, read-only: the token endpoint (code exchange and
//! refresh, PKCE, no client secret) and paged GETs with the access token.
//!
//! The refresh token lives in the OS keychain and nowhere else; the access
//! token only in memory. Neither goes into an error message or a log, and
//! `next` links are followed only when they point at the API itself.
//!
//! Rate limits: a 429 waits out its Retry-After when that's short (up to
//! [`MAX_WAIT_S`], a few times), else the read stops with
//! [`Fail::RateLimited`] so the UI can say when to try again.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::Value;

use super::Fail;
use crate::artistinfo::http::HttpResponse;
use crate::error::AppError;
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;
use crate::web::{Web, WebRequest};

/// Where sign-in and tokens live.
pub const ACCOUNTS: &str = "https://accounts.spotify.com";
/// The Web API.
pub const API: &str = "https://api.spotify.com/v1";
/// Reading playlists, private and collaborative ones included. Nothing else
/// is asked for.
pub const SCOPES: &str = "playlist-read-private playlist-read-collaborative";
/// The keychain account holding the refresh token.
pub const REFRESH_ACCOUNT: &str = "import-account-refresh";
/// Longest Retry-After waited out inline, in seconds.
pub const MAX_WAIT_S: u64 = 30;
/// Inline waits per request before giving up.
const MAX_WAITS: u32 = 3;
/// Items per page (the most the playlist endpoints give).
pub const PAGE: u32 = 50;
/// Pages read from one list at most (50 000 items), against a loop.
const MAX_PAGES: usize = 1_000;

#[derive(Clone, Debug)]
pub struct Endpoints {
    pub accounts: String,
    pub api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self { accounts: ACCOUNTS.into(), api: API.into() }
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Deserialize, Default)]
struct TokenError {
    #[serde(default)]
    error: String,
}

pub struct Api {
    web: Arc<dyn Web>,
    keys: Arc<dyn TokenStore>,
    pub ends: Endpoints,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub cancel: Arc<AtomicBool>,
    /// The access token and when it stops working (unix ms).
    access: Mutex<Option<(Token, i64)>>,
}

fn net(e: AppError) -> Fail {
    match e {
        AppError::Network(m) => Fail::Offline(m),
        other => Fail::Other(other.to_string()),
    }
}

impl Api {
    pub fn new(web: Arc<dyn Web>, keys: Arc<dyn TokenStore>, ends: Endpoints, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        Self { web, keys, ends, clock, cancel: Arc::new(AtomicBool::new(false)), access: Mutex::new(None) }
    }

    /// The sign-in page's address.
    pub fn authorize_url(&self, client_id: &str, redirect: &str, challenge: &str, state: &str) -> String {
        let q = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", challenge)
            .append_pair("state", state)
            .append_pair("scope", SCOPES)
            .finish();
        format!("{}/authorize?{q}", self.ends.accounts)
    }

    /// Whether a refresh token is kept.
    pub fn signed_in(&self) -> bool {
        self.keys.get(REFRESH_ACCOUNT).ok().flatten().is_some()
    }

    /// Forgets both tokens (sign out, or a refresh token that stopped working).
    pub fn forget(&self) -> Result<(), Fail> {
        *self.access.lock() = None;
        self.keys.delete(REFRESH_ACCOUNT).map_err(|e| Fail::Other(e.to_string()))
    }

    async fn token_call(&self, form: Vec<(String, String)>, refreshing: bool) -> Result<Token, Fail> {
        let req = WebRequest::post_form(format!("{}/api/token", self.ends.accounts), form);
        let r = self.web.send(req).await.map_err(net)?;
        if r.ok() {
            let t: TokenAnswer = serde_json::from_slice(&r.body).map_err(|_| Fail::Other("The sign-in answer couldn't be read.".into()))?;
            let access = Token::new(t.access_token).map_err(|_| Fail::Other("The sign-in answer had no access token.".into()))?;
            if let Some(rt) = t.refresh_token.filter(|s| !s.trim().is_empty()) {
                let rt = Token::new(rt).map_err(|e| Fail::Other(e.to_string()))?;
                self.keys.set(REFRESH_ACCOUNT, &rt).map_err(|e| Fail::Other(format!("Couldn't keep the sign-in in the keychain: {e}")))?;
            }
            let life = t.expires_in.unwrap_or(3600).max(60);
            let until = (self.clock)() + (life - 30) * 1000;
            *self.access.lock() = Some((access.clone(), until));
            return Ok(access);
        }
        if r.status == 429 {
            return Err(Fail::RateLimited(r.retry_after.unwrap_or(60)));
        }
        let e: TokenError = serde_json::from_slice(&r.body).unwrap_or_default();
        Err(match e.error.as_str() {
            "invalid_client" => Fail::Refused("The Client ID wasn't accepted. Check it in Settings.".into()),
            "invalid_grant" if refreshing => Fail::Expired,
            "invalid_grant" => Fail::Refused("The sign-in was refused. Try again.".into()),
            _ if r.status >= 500 => Fail::Other(format!("The sign-in service had a problem ({}). Try again later.", r.status)),
            _ if refreshing => Fail::Expired,
            _ => Fail::Refused("The sign-in was refused. Check the Client ID and the redirect URI in your developer app.".into()),
        })
    }

    /// Trades the sign-in code for tokens and keeps the refresh token.
    pub async fn exchange(&self, client_id: &str, code: &str, redirect: &str, verifier: &str) -> Result<(), Fail> {
        let form = vec![
            ("grant_type".into(), "authorization_code".into()),
            ("code".into(), code.into()),
            ("redirect_uri".into(), redirect.into()),
            ("client_id".into(), client_id.into()),
            ("code_verifier".into(), verifier.into()),
        ];
        self.token_call(form, false).await.map(|_| ())
    }

    /// A new access token from the kept refresh token. A refresh token
    /// that's no longer accepted is removed ([`Fail::Expired`]).
    pub async fn refresh(&self, client_id: &str) -> Result<Token, Fail> {
        let Some(rt) = self.keys.get(REFRESH_ACCOUNT).map_err(|e| Fail::Other(e.to_string()))? else {
            return Err(Fail::Expired);
        };
        let form = vec![
            ("grant_type".into(), "refresh_token".into()),
            ("refresh_token".into(), rt.expose().to_string()),
            ("client_id".into(), client_id.into()),
        ];
        match self.token_call(form, true).await {
            Err(Fail::Expired) => {
                let _ = self.forget();
                Err(Fail::Expired)
            }
            other => other,
        }
    }

    async fn access_token(&self, client_id: &str) -> Result<Token, Fail> {
        let now = (self.clock)();
        if let Some((t, until)) = self.access.lock().clone()
            && now < until
        {
            return Ok(t);
        }
        self.refresh(client_id).await
    }

    /// Sleeps, waking early when stopped.
    async fn pause(&self, secs: u64) -> Result<(), Fail> {
        let steps = secs * 4;
        for _ in 0..steps.max(1) {
            if self.cancel.load(Ordering::SeqCst) {
                return Err(Fail::Stopped);
            }
            tokio::time::sleep(Duration::from_millis(if secs == 0 { 1 } else { 250 })).await;
        }
        Ok(())
    }

    /// GETs `url` (on the API) as JSON with the access token, refreshing it
    /// once on a 401 and waiting out short rate limits.
    pub async fn get(&self, client_id: &str, url: &str) -> Result<Value, Fail> {
        if !url.starts_with(&self.ends.api) {
            return Err(Fail::Other("A link pointed away from the service, so it wasn't followed.".into()));
        }
        let mut refreshed = false;
        let mut waits = 0;
        let mut server_retry = false;
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Err(Fail::Stopped);
            }
            let tok = self.access_token(client_id).await?;
            let req = WebRequest::get(url).header("Authorization", format!("Bearer {}", tok.expose()));
            let r: HttpResponse = self.web.send(req).await.map_err(net)?;
            match r.status {
                200..=299 => return serde_json::from_slice(&r.body).map_err(|_| Fail::Other("The service's answer couldn't be read.".into())),
                401 if !refreshed => {
                    refreshed = true;
                    *self.access.lock() = None;
                }
                401 => return Err(Fail::Expired),
                403 | 404 => return Err(Fail::Forbidden(error_message(&r.body))),
                429 => {
                    let secs = r.retry_after.unwrap_or(5);
                    if secs > MAX_WAIT_S || waits >= MAX_WAITS {
                        return Err(Fail::RateLimited(secs));
                    }
                    waits += 1;
                    self.pause(secs).await?;
                }
                500..=599 if !server_retry => {
                    server_retry = true;
                    self.pause(1).await?;
                }
                s => return Err(Fail::Other(format!("The service had a problem ({s}). Try again later."))),
            }
        }
    }

    /// Every item of a paged list, following `next` links on the API.
    pub async fn all_items(&self, client_id: &str, first: &str, mut on_page: impl FnMut(usize)) -> Result<Vec<Value>, Fail> {
        let mut out = Vec::new();
        let mut url = first.to_string();
        for _ in 0..MAX_PAGES {
            let page = self.get(client_id, &url).await?;
            if let Some(items) = page.get("items").and_then(Value::as_array) {
                out.extend(items.iter().cloned());
            }
            on_page(out.len());
            match page.get("next").and_then(Value::as_str) {
                Some(n) if !n.is_empty() && n.starts_with(&self.ends.api) => url = n.to_string(),
                _ => break,
            }
        }
        Ok(out)
    }
}

/// The service's `error.message`, or a plain line.
fn error_message(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "not allowed".into())
}
