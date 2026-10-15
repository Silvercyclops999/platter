//! Last.fm: desktop sign-in and scrobbling (API 2.0).
//!
//! Sign-in uses Last.fm's desktop flow: ask for a token, the user approves
//! Platter on last.fm in their browser, then the token is swapped for a
//! session key. The user's API key (shared with artist info), its shared
//! secret and the session key live in the OS keychain. Every call is signed
//! (md5 of the sorted parameters and the secret).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use md5::{Digest, Md5};
use parking_lot::Mutex;
use serde::Deserialize;

use super::{Listen, ScrobbleService, SendError, Service};
use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;
use crate::web::{Web, WebRequest};

pub const API: &str = "https://ws.audioscrobbler.com/2.0/";
pub const AUTH_PAGE: &str = "https://www.last.fm/api/auth/";
pub const KEY_ACCOUNT: &str = "lastfm-api-key";
pub const SECRET_ACCOUNT: &str = "lastfm-api-secret";
pub const SESSION_ACCOUNT: &str = "lastfm-session";

/// The signature Last.fm asks for: every parameter but `format` and
/// `callback`, sorted by name, names and values run together, then the
/// secret, as md5 hex.
pub fn api_sig(params: &BTreeMap<String, String>, secret: &str) -> String {
    let mut s = String::new();
    for (k, v) in params {
        if k == "format" || k == "callback" {
            continue;
        }
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(secret);
    Md5::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Deserialize)]
struct ErrorReply {
    error: i64,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct TokenReply {
    token: String,
}

#[derive(Deserialize)]
struct SessionReply {
    session: SessionBody,
}

#[derive(Deserialize)]
struct SessionBody {
    name: String,
    key: String,
}

/// What Last.fm error codes mean here.
fn classify(code: i64, message: &str) -> SendError {
    match code {
        // Invalid session, API key, signature; suspended key.
        4 | 9 | 10 | 13 | 26 => SendError::Rejected,
        // Service offline, temporary error, rate limit.
        8 | 11 | 16 | 29 => SendError::Later(format!("Last.fm error {code}")),
        _ => SendError::Refused(format!("Last.fm error {code}: {message}")),
    }
}

pub struct Lastfm {
    web: Arc<dyn Web>,
    keys: Arc<dyn TokenStore>,
    /// The session key once read (None inside: not signed in).
    session: Mutex<Option<Option<Token>>>,
}

/// A sign-in started in the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAuth {
    pub token: String,
    pub url: String,
}

impl Lastfm {
    pub fn new(web: Arc<dyn Web>, keys: Arc<dyn TokenStore>) -> Self {
        Self { web, keys, session: Mutex::new(None) }
    }

    fn read(&self, account: &str) -> Option<Token> {
        self.keys.get(account).unwrap_or_else(|e| {
            tracing::warn!("keychain: {e}");
            None
        })
    }

    pub fn has_key(&self) -> bool {
        self.read(KEY_ACCOUNT).is_some()
    }

    pub fn has_secret(&self) -> bool {
        self.read(SECRET_ACCOUNT).is_some()
    }

    fn session_key(&self) -> Option<Token> {
        let mut s = self.session.lock();
        if let Some(k) = s.as_ref() {
            return k.clone();
        }
        let k = self.read(SESSION_ACCOUNT);
        *s = Some(k.clone());
        k
    }

    /// Save (or remove, with None) the shared secret.
    pub fn set_secret(&self, secret: Option<&str>) -> AppResult<()> {
        match secret.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => self.keys.set(SECRET_ACCOUNT, &Token::new(s)?),
            None => self.keys.delete(SECRET_ACCOUNT),
        }
    }

    pub fn sign_out(&self) -> AppResult<()> {
        self.keys.delete(SESSION_ACCOUNT)?;
        *self.session.lock() = Some(None);
        Ok(())
    }

    fn key_and_secret(&self) -> AppResult<(Token, Token)> {
        let key = self.read(KEY_ACCOUNT).ok_or_else(|| AppError::Invalid("Add your Last.fm API key first.".into()))?;
        let secret = self.read(SECRET_ACCOUNT).ok_or_else(|| AppError::Invalid("Add your Last.fm shared secret first.".into()))?;
        Ok((key, secret))
    }

    fn signed(params: Vec<(&str, String)>, key: &Token, secret: &Token) -> Vec<(String, String)> {
        let mut m: BTreeMap<String, String> = params.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        m.insert("api_key".into(), key.expose().to_string());
        let sig = api_sig(&m, secret.expose());
        m.insert("api_sig".into(), sig);
        m.insert("format".into(), "json".into());
        m.into_iter().collect()
    }

    async fn call(&self, form: Vec<(String, String)>) -> Result<serde_json::Value, SendError> {
        let resp = self.web.send(WebRequest::post_form(API, form)).await.map_err(|e| SendError::Later(e.kind().to_string()))?;
        let v: serde_json::Value = serde_json::from_slice(&resp.body).unwrap_or(serde_json::Value::Null);
        if let Ok(err) = serde_json::from_value::<ErrorReply>(v.clone()) {
            return Err(classify(err.error, &err.message));
        }
        if resp.status >= 500 || resp.status == 429 {
            return Err(SendError::Later(format!("Last.fm answered {}", resp.status)));
        }
        if !resp.ok() {
            return Err(SendError::Refused(format!("Last.fm answered {}", resp.status)));
        }
        Ok(v)
    }

    /// Step 1: a token, and the page where the user approves it.
    pub async fn auth_start(&self) -> AppResult<PendingAuth> {
        let (key, secret) = self.key_and_secret()?;
        let form = Self::signed(vec![("method", "auth.getToken".into())], &key, &secret);
        let v = self.call(form).await.map_err(auth_error)?;
        let t: TokenReply = serde_json::from_value(v).map_err(|e| AppError::Decode(format!("Last.fm token: {e}")))?;
        let url = format!("{AUTH_PAGE}?api_key={}&token={}", enc(key.expose()), enc(&t.token));
        Ok(PendingAuth { token: t.token, url })
    }

    /// Step 2, after the user approved: the session key (saved) and the
    /// user name.
    pub async fn auth_finish(&self, token: &str) -> AppResult<String> {
        let (key, secret) = self.key_and_secret()?;
        let form = Self::signed(vec![("method", "auth.getSession".into()), ("token", token.to_string())], &key, &secret);
        let resp = self.web.send(WebRequest::post_form(API, form)).await?;
        let v: serde_json::Value = serde_json::from_slice(&resp.body).unwrap_or(serde_json::Value::Null);
        if let Ok(err) = serde_json::from_value::<ErrorReply>(v.clone()) {
            return Err(match err.error {
                14 => AppError::Invalid("Allow Platter on the Last.fm page first, then finish here.".into()),
                15 | 4 => AppError::Invalid("That sign-in expired. Start again.".into()),
                _ => auth_error(classify(err.error, &err.message)),
            });
        }
        let s: SessionReply = serde_json::from_value(v).map_err(|e| AppError::Decode(format!("Last.fm session: {e}")))?;
        self.keys.set(SESSION_ACCOUNT, &Token::new(s.session.key)?)?;
        *self.session.lock() = None;
        Ok(s.session.name)
    }

    fn track_params(l: &Listen, i: Option<usize>) -> Vec<(String, String)> {
        let n = |k: &str| match i {
            Some(i) => format!("{k}[{i}]"),
            None => k.to_string(),
        };
        let mut p = vec![(n("artist"), l.artist.clone()), (n("track"), l.title.clone())];
        if !l.album.is_empty() {
            p.push((n("album"), l.album.clone()));
        }
        if let Some(a) = &l.album_artist {
            p.push((n("albumArtist"), a.clone()));
        }
        if l.duration_s > 0 {
            p.push((n("duration"), l.duration_s.to_string()));
        }
        if l.track_number > 0 {
            p.push((n("trackNumber"), l.track_number.to_string()));
        }
        if i.is_some() {
            p.push((n("timestamp"), l.started_at.to_string()));
        }
        p
    }

    async fn send(&self, method: &str, extra: Vec<(String, String)>) -> Result<serde_json::Value, SendError> {
        let sk = self.session_key().ok_or(SendError::NotSignedIn)?;
        let (key, secret) = self.key_and_secret().map_err(|_| SendError::NotSignedIn)?;
        let mut params: Vec<(&str, String)> = vec![("method", method.to_string()), ("sk", sk.expose().to_string())];
        let extra_refs: Vec<(&str, String)> = extra.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        params.extend(extra_refs);
        self.call(Self::signed(params, &key, &secret)).await
    }
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn auth_error(e: SendError) -> AppError {
    match e {
        SendError::Rejected => AppError::Auth("Last.fm didn't accept your API key or shared secret.".into()),
        SendError::Later(_) => AppError::Network("Couldn't reach Last.fm. Try again in a moment.".into()),
        SendError::NotSignedIn => AppError::Invalid("Add your Last.fm API key and shared secret first.".into()),
        SendError::Refused(m) => AppError::Server(m),
    }
}

#[async_trait]
impl ScrobbleService for Lastfm {
    fn service(&self) -> Service {
        Service::Lastfm
    }

    fn signed_in(&self) -> bool {
        self.session_key().is_some()
    }

    async fn now_playing(&self, l: &Listen) -> Result<(), SendError> {
        self.send("track.updateNowPlaying", Self::track_params(l, None)).await.map(|_| ())
    }

    async fn scrobble(&self, ls: &[Listen]) -> Result<(), SendError> {
        let mut extra = Vec::new();
        for (i, l) in ls.iter().take(50).enumerate() {
            extra.extend(Self::track_params(l, Some(i)));
        }
        let v = self.send("track.scrobble", extra).await?;
        let ignored = v.pointer("/scrobbles/@attr/ignored").and_then(|x| x.as_i64().or_else(|| x.as_str()?.parse().ok())).unwrap_or(0);
        if ignored > 0 {
            tracing::debug!("Last.fm ignored {ignored} scrobbles");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_sorts_and_skips_format() {
        let mut m = BTreeMap::new();
        m.insert("method".to_string(), "auth.getToken".to_string());
        m.insert("api_key".to_string(), "k".to_string());
        m.insert("format".to_string(), "json".to_string());
        // md5("api_keykmethodauth.getTokens")
        let want: String = Md5::digest(b"api_keykmethodauth.getTokens").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(api_sig(&m, "s"), want);
    }
}
