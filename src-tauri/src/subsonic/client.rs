//! The OpenSubsonic HTTP API as Platter uses it: `/rest/<method>` with
//! `f=json`, the protocol version and the app's name, and the sign-in on
//! every request. The sign-in is an API key (preferred), or the user name
//! with `t = md5(password + salt)` and a fresh salt `s`; the password itself
//! never goes to the server. Because Subsonic puts these in the address,
//! no address ever goes into a log line, a `Debug` or an error: requests
//! are POSTed as a form where the server allows it, and the log redaction
//! hides `apiKey`, `t`, `s` and `p` values as a second line of defence.

use std::time::Duration;

use md5::{Digest, Md5};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use url::Url;

use super::types::{Extension, Status};
use crate::error::{AppError, AppResult};
use crate::plex::token::Token;

/// The protocol version Platter speaks.
pub const API_VERSION: &str = "1.16.1";
pub const CLIENT_NAME: &str = "Platter";

pub const OFFLINE: &str = "Platter can't reach your music server.";
pub const NOT_SUBSONIC: &str = "That address doesn't answer like a Navidrome or other Subsonic server.";
pub const WRONG_PASSWORD: &str = "That user name and password didn't work.";
pub const KEY_REFUSED: &str = "The server didn't accept that API key. Make a new one on the server and try again.";
pub const NO_API_KEYS: &str = "This server doesn't take API keys. Sign in with your user name and password instead.";
pub const KEYS_ONLY: &str = "This server only takes API keys. Make one on the server and sign in with it.";
pub const NO_TOKEN_AUTH: &str =
    "This server can't check a password the safe way for your account. Make an API key on the server and sign in with it.";
pub const NOT_ALLOWED: &str = "Your account on this server isn't allowed to do that.";
pub const SIGN_IN_AGAIN: &str = "Your music server asked Platter to sign in again.";
pub const SERVER_TOO_OLD: &str = "This server speaks an older version of the Subsonic API than Platter needs.";

/// How Platter signs in. Neither form prints.
#[derive(Clone, PartialEq, Eq)]
pub enum Secret {
    ApiKey(Token),
    Password { user: String, password: Token },
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Secret::ApiKey(_) => f.write_str("ApiKey(<redacted>)"),
            Secret::Password { .. } => f.write_str("Password(<redacted>)"),
        }
    }
}

impl Secret {
    pub fn is_key(&self) -> bool {
        matches!(self, Secret::ApiKey(_))
    }

    /// As kept in the keychain.
    pub fn stored(&self) -> AppResult<Token> {
        match self {
            Secret::ApiKey(k) => Token::new(format!("apikey:{}", k.expose())),
            Secret::Password { password, .. } => Token::new(format!("password:{}", password.expose())),
        }
    }

    /// Back from the keychain; `user` is the record's user name.
    pub fn from_stored(t: &Token, user: Option<&str>) -> Option<Secret> {
        let s = t.expose();
        if let Some(k) = s.strip_prefix("apikey:") {
            return Token::new(k).ok().map(Secret::ApiKey);
        }
        let pw = s.strip_prefix("password:")?;
        let user = user.filter(|u| !u.trim().is_empty())?;
        Some(Secret::Password { user: user.to_string(), password: Token::new(pw).ok()? })
    }

    fn values(&self) -> Vec<&str> {
        match self {
            Secret::ApiKey(k) => vec![k.expose()],
            Secret::Password { password, .. } => vec![password.expose()],
        }
    }
}

/// `md5(password + salt)` as lower-case hex.
pub fn token_for(password: &str, salt: &str) -> String {
    Md5::digest(format!("{password}{salt}").as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn salt() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

/// The address as typed, tidied: a scheme, no trailing slash, no `/rest`,
/// `/app` page or fragment.
pub fn normalize_url(raw: &str) -> AppResult<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::Invalid("Type your server's address.".into()));
    }
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("http://{raw}") };
    let mut u = Url::parse(&with_scheme).map_err(|_| AppError::Invalid("That isn't a web address.".into()))?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        return Err(AppError::Invalid("That isn't a web address.".into()));
    }
    u.set_fragment(None);
    u.set_query(None);
    let _ = u.set_username("");
    let _ = u.set_password(None);
    let mut path = u.path().trim_end_matches('/').to_string();
    loop {
        let before = path.len();
        for page in ["/rest", "/app", "/index.html"] {
            if let Some(p) = path.strip_suffix(page) {
                path = p.trim_end_matches('/').to_string();
            }
        }
        if path.len() == before {
            break;
        }
    }
    u.set_path(&path);
    Ok(u.as_str().trim_end_matches('/').to_string())
}

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(60))
        // The sign-in rides on every request: never to another host.
        .redirect(crate::web::same_origin_redirects())
        .build()
        .unwrap_or_default()
}

/// A connection to one server, signed in or not. Cheap to clone.
#[derive(Clone)]
pub struct SubsonicClient {
    http: reqwest::Client,
    base: String,
    secret: Option<Secret>,
    /// The server takes requests as a POSTed form (`formPost`), which keeps
    /// the sign-in out of addresses.
    form_post: bool,
}

impl std::fmt::Debug for SubsonicClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubsonicClient").field("signed_in", &self.secret.is_some()).field("form_post", &self.form_post).finish()
    }
}

impl SubsonicClient {
    pub fn new(http: reqwest::Client, base: &str) -> AppResult<Self> {
        Ok(Self { http, base: normalize_url(base)?, secret: None, form_post: false })
    }

    pub fn signed_in(mut self, secret: Secret) -> Self {
        self.secret = Some(secret);
        self
    }

    pub fn with_form_post(mut self, on: bool) -> Self {
        self.form_post = on;
        self
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn secret(&self) -> Option<&Secret> {
        self.secret.as_ref()
    }

    /// The account name, when signed in with a password.
    pub fn user(&self) -> Option<&str> {
        match &self.secret {
            Some(Secret::Password { user, .. }) => Some(user),
            _ => None,
        }
    }

    /// The common parameters and the sign-in (a new salt each time).
    fn common(&self) -> Vec<(&'static str, String)> {
        let mut q = vec![("f", "json".to_string()), ("v", API_VERSION.to_string()), ("c", CLIENT_NAME.to_string())];
        match &self.secret {
            Some(Secret::ApiKey(k)) => q.push(("apiKey", k.expose().to_string())),
            Some(Secret::Password { user, password }) => {
                let s = salt();
                q.push(("u", user.clone()));
                q.push(("t", token_for(password.expose(), &s)));
                q.push(("s", s));
            }
            None => {}
        }
        q
    }

    /// The full address of a method with its parameters and the sign-in.
    /// For streams and images the player and art cache open themselves;
    /// it carries the sign-in, so it's never logged.
    pub fn url(&self, method: &str, params: &[(&str, String)]) -> AppResult<Url> {
        let mut u = self.endpoint(method)?;
        {
            let mut q = u.query_pairs_mut();
            for (k, v) in params {
                q.append_pair(k, v);
            }
            for (k, v) in self.common() {
                q.append_pair(k, &v);
            }
        }
        Ok(u)
    }

    fn endpoint(&self, method: &str) -> AppResult<Url> {
        Url::parse(&format!("{}/rest/{method}", self.base)).map_err(|e| AppError::Internal(format!("url: {e}")))
    }

    /// Remove the secret from text built from an answer.
    pub fn scrub(&self, text: String) -> String {
        let mut t = text;
        if let Some(s) = &self.secret {
            for v in s.values() {
                if !v.is_empty() {
                    t = t.replace(v, "<redacted>");
                }
            }
        }
        t
    }

    /// Send a method call; a failed answer comes back as the app's errors.
    pub async fn send(&self, method: &str, params: &[(&str, String)]) -> AppResult<reqwest::Response> {
        let req = if self.form_post {
            let mut form: Vec<(String, String)> = params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
            form.extend(self.common().into_iter().map(|(k, v)| (k.to_string(), v)));
            self.http.request(Method::POST, self.endpoint(method)?).form(&form)
        } else {
            self.http.get(self.url(method, params)?)
        };
        let resp = req.header("Accept", "application/json").send().await.map_err(|e| {
            tracing::debug!("subsonic request failed: {}", self.scrub(e.without_url().to_string()));
            AppError::Network(OFFLINE.into())
        })?;
        check_http(resp.status())?;
        Ok(resp)
    }

    /// The answer's `key` part as `T` (`None`: only check it succeeded).
    pub async fn get<T: DeserializeOwned + Default>(&self, method: &str, params: &[(&str, String)], key: Option<&str>) -> AppResult<T> {
        let resp = self.send(method, params).await?;
        let bytes = crate::web::body_capped(resp, crate::web::SERVER_MAX, |_| AppError::Network(OFFLINE.into())).await?;
        let (status, body) = envelope(&bytes)?;
        check_status(&status, self.secret.as_ref())?;
        let Some(key) = key else { return Ok(T::default()) };
        match body.get(key) {
            None | Some(serde_json::Value::Null) => Ok(T::default()),
            Some(v) => serde_json::from_value(v.clone()).map_err(|e| AppError::Decode(format!("{method}: {}", self.scrub(e.to_string())))),
        }
    }

    /// A call whose answer is only checked.
    pub async fn call(&self, method: &str, params: &[(&str, String)]) -> AppResult<()> {
        self.get::<serde_json::Value>(method, params, None).await.map(|_| ())
    }

    /// The envelope of `ping` (status, product, version), signed in or not.
    pub async fn ping(&self) -> AppResult<Status> {
        let resp = self.send("ping", &[]).await?;
        let bytes = crate::web::body_capped(resp, crate::web::SERVER_MAX, |_| AppError::Network(OFFLINE.into())).await?;
        let (status, _) = envelope(&bytes)?;
        check_status(&status, self.secret.as_ref())?;
        Ok(status)
    }

    /// Whether an address is a Subsonic server: `ping` answers with the
    /// envelope (even when it refuses an unsigned request). Also its
    /// product and version.
    pub async fn probe(&self) -> AppResult<Status> {
        let resp = self.send("ping", &[]).await.map_err(|e| match e {
            AppError::NotFound(_) | AppError::Auth(_) | AppError::Server(_) => AppError::Invalid(NOT_SUBSONIC.into()),
            other => other,
        })?;
        let bytes = crate::web::body_capped(resp, crate::web::SERVER_MAX, |_| AppError::Network(OFFLINE.into())).await?;
        let (status, _) = envelope(&bytes).map_err(|_| AppError::Invalid(NOT_SUBSONIC.into()))?;
        if status.status != "ok" && status.status != "failed" {
            return Err(AppError::Invalid(NOT_SUBSONIC.into()));
        }
        if let Some(e) = &status.error
            && matches!(e.code, Some(30))
        {
            return Err(AppError::Invalid(SERVER_TOO_OLD.into()));
        }
        Ok(status)
    }

    /// The server's OpenSubsonic extensions (public). A server without the
    /// call has none.
    pub async fn extensions(&self) -> Vec<Extension> {
        match self.get::<Vec<Extension>>("getOpenSubsonicExtensions", &[], Some("openSubsonicExtensions")).await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("no opensubsonic extensions: {}", e.kind());
                Vec::new()
            }
        }
    }
}

/// The `subsonic-response` object: its status fields and the whole object.
pub fn envelope(bytes: &[u8]) -> AppResult<(Status, serde_json::Value)> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| AppError::Invalid(NOT_SUBSONIC.into()))?;
    let body = v.get("subsonic-response").cloned().ok_or_else(|| AppError::Invalid(NOT_SUBSONIC.into()))?;
    let status: Status = serde_json::from_value(body.clone()).map_err(|_| AppError::Invalid(NOT_SUBSONIC.into()))?;
    Ok((status, body))
}

/// A failed answer as the app's error. What a sign-in error means depends
/// on how Platter signed in.
pub fn check_status(s: &Status, secret: Option<&Secret>) -> AppResult<()> {
    if s.status == "ok" {
        return Ok(());
    }
    let e = s.error.clone().unwrap_or_default();
    let key = secret.is_some_and(|s| s.is_key());
    Err(match e.code.unwrap_or(0) {
        10 if secret.is_none() => AppError::Auth(SIGN_IN_AGAIN.into()),
        10 => AppError::Invalid("The server said a value was missing.".into()),
        20 => AppError::Invalid("This server needs a newer app than Platter speaks.".into()),
        30 => AppError::Invalid(SERVER_TOO_OLD.into()),
        40 if key => AppError::Auth(KEY_REFUSED.into()),
        40 => AppError::Auth(WRONG_PASSWORD.into()),
        41 => AppError::Auth(NO_TOKEN_AUTH.into()),
        42 if key => AppError::Auth(NO_API_KEYS.into()),
        42 => AppError::Auth(KEYS_ONLY.into()),
        43 => AppError::Internal("conflicting sign-in sent".into()),
        44 => AppError::Auth(KEY_REFUSED.into()),
        50 => AppError::Auth(NOT_ALLOWED.into()),
        70 => AppError::NotFound("not on the server".into()),
        code => AppError::Server(format!("The server answered with error {code}.")),
    })
}

/// Status codes as the app's errors.
pub fn check_http(s: StatusCode) -> AppResult<()> {
    match s {
        s if s.is_success() => Ok(()),
        StatusCode::UNAUTHORIZED => Err(AppError::Auth(SIGN_IN_AGAIN.into())),
        StatusCode::FORBIDDEN => Err(AppError::Auth(NOT_ALLOWED.into())),
        StatusCode::NOT_FOUND | StatusCode::GONE => Err(AppError::NotFound("not on the server".into())),
        s => Err(AppError::Server(format!("The server answered {}.", s.as_u16()))),
    }
}

/// Sign-in errors that mean the saved sign-in no longer works.
pub fn is_sign_in_again(e: &AppError) -> bool {
    matches!(e, AppError::Auth(m) if m == SIGN_IN_AGAIN || m == WRONG_PASSWORD || m == KEY_REFUSED || m == NO_TOKEN_AUTH || m == KEYS_ONLY || m == NO_API_KEYS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_tidied() {
        assert_eq!(normalize_url("music.example:4533").unwrap(), "http://music.example:4533");
        assert_eq!(normalize_url(" https://Music.example/nd/app/#/album ").unwrap(), "https://music.example/nd");
        assert_eq!(normalize_url("http://music.example:4533/rest/").unwrap(), "http://music.example:4533");
        assert_eq!(normalize_url("http://me:pw@music.example/").unwrap(), "http://music.example");
        assert!(normalize_url("").is_err());
        assert!(normalize_url("ftp://x.example").is_err());
    }

    #[test]
    fn the_token_is_md5_of_password_and_salt() {
        // The example from the Subsonic API description.
        assert_eq!(token_for("sesame", "c19b2d"), "26719a1196d2a940705a59634eb18eab");
    }

    #[test]
    fn password_sign_in_sends_a_token_never_the_password() {
        let c = SubsonicClient::new(reqwest::Client::new(), "http://music.example")
            .unwrap()
            .signed_in(Secret::Password { user: "me".into(), password: Token::new("sesame").unwrap() });
        let u = c.url("ping", &[]).unwrap();
        let q: std::collections::HashMap<String, String> = u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        assert_eq!((q["u"].as_str(), q["f"].as_str(), q["v"].as_str(), q["c"].as_str()), ("me", "json", API_VERSION, "Platter"));
        assert_eq!(q["t"], token_for("sesame", &q["s"]));
        assert!(!q.contains_key("p") && !u.as_str().contains("sesame"));
        let again = c.url("ping", &[]).unwrap();
        assert_ne!(u, again, "a fresh salt every time");
        let k = SubsonicClient::new(reqwest::Client::new(), "http://music.example").unwrap().signed_in(Secret::ApiKey(Token::new("key1").unwrap()));
        let ku = k.url("ping", &[]).unwrap();
        assert!(ku.query_pairs().any(|(k, v)| k == "apiKey" && v == "key1"));
        assert!(!ku.query_pairs().any(|(k, _)| k == "u" || k == "t" || k == "s"), "an API key goes alone");
    }

    #[test]
    fn nothing_secret_prints() {
        let s = Secret::Password { user: "me".into(), password: Token::new("sesame").unwrap() };
        let c = SubsonicClient::new(reqwest::Client::new(), "http://music.example").unwrap().signed_in(s.clone());
        let printed = format!("{s:?} {c:?} {:?}", Secret::ApiKey(Token::new("key1").unwrap()));
        assert!(!printed.contains("sesame") && !printed.contains("key1"), "{printed}");
        assert_eq!(c.scrub("bad sesame here".into()), "bad <redacted> here");
    }

    #[test]
    fn secrets_round_trip_through_the_keychain_form() {
        let k = Secret::ApiKey(Token::new("abc").unwrap());
        assert_eq!(Secret::from_stored(&k.stored().unwrap(), None), Some(k));
        let p = Secret::Password { user: "me".into(), password: Token::new("pw:with:colons").unwrap() };
        assert_eq!(Secret::from_stored(&p.stored().unwrap(), Some("me")), Some(p.clone()));
        assert_eq!(Secret::from_stored(&p.stored().unwrap(), None), None, "a password needs its user");
        assert_eq!(Secret::from_stored(&Token::new("junk").unwrap(), Some("me")), None);
    }

    #[test]
    fn error_codes_say_what_to_do() {
        let failed = |code: u64| Status { status: "failed".into(), error: Some(super::super::types::ApiError { code: Some(code), message: None }), ..Default::default() };
        let key = Secret::ApiKey(Token::new("k").unwrap());
        let pw = Secret::Password { user: "u".into(), password: Token::new("p").unwrap() };
        let msg = |code, s: Option<&Secret>| check_status(&failed(code), s).unwrap_err().to_string();
        assert_eq!(msg(40, Some(&pw)), format!("auth: {WRONG_PASSWORD}"));
        assert_eq!(msg(44, Some(&key)), format!("auth: {KEY_REFUSED}"));
        assert_eq!(msg(42, Some(&key)), format!("auth: {NO_API_KEYS}"));
        assert_eq!(msg(42, Some(&pw)), format!("auth: {KEYS_ONLY}"));
        assert_eq!(msg(41, Some(&pw)), format!("auth: {NO_TOKEN_AUTH}"));
        assert_eq!(msg(50, Some(&pw)), format!("auth: {NOT_ALLOWED}"));
        assert!(matches!(check_status(&failed(70), Some(&pw)), Err(AppError::NotFound(_))));
        assert!(check_status(&Status { status: "ok".into(), ..Default::default() }, None).is_ok());
    }
}
