//! The Jellyfin HTTP API as Platter uses it. Every request carries the
//! `Authorization: MediaBrowser ...` header with the app's name, this
//! computer's device id and, once signed in, the token. The token never goes
//! into a URL; the header is marked sensitive so it never prints.

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::HeaderValue;
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use url::Url;

use super::types::{PublicInfo, version};
use crate::error::{AppError, AppResult};
use crate::plex::token::{Token, scrub};

/// The oldest Jellyfin Platter works with (the 10.9 paths for favourites,
/// played items, lyrics and playlists).
pub const MIN_VERSION: (u32, u32) = (10, 9);

pub const OFFLINE: &str = "Platter can't reach your Jellyfin server.";
pub const SIGN_IN_AGAIN: &str = "Jellyfin asked Platter to sign in again.";
pub const NOT_ALLOWED: &str = "Your Jellyfin account isn't allowed to do that.";
pub const NOT_JELLYFIN: &str = "That address doesn't answer like a Jellyfin server.";

/// Who is asking: the app, its version and this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub device_id: String,
    pub device_name: String,
    pub version: String,
}

impl Identity {
    pub fn new(device_id: &str) -> Self {
        let device_name = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .ok()
            .map(|n| ascii_name(&n))
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Desktop".into());
        Self { device_id: device_id.into(), device_name, version: env!("CARGO_PKG_VERSION").into() }
    }
}

/// A value for the header: no quotes or commas, which would end the field,
/// and only printable ASCII, which is all a header takes.
fn field(v: &str) -> String {
    v.chars().filter(|c| (c.is_ascii_graphic() || *c == ' ') && !matches!(c, '"' | ',')).collect()
}

/// A computer name as a header can carry it: accents dropped from the
/// letters that have them, anything else outside ASCII left out.
pub fn ascii_name(name: &str) -> String {
    name.chars()
        .filter_map(|c| match c {
            'à'..='å' => Some('a'),
            'À'..='Å' => Some('A'),
            'è'..='ë' => Some('e'),
            'È'..='Ë' => Some('E'),
            'ì'..='ï' => Some('i'),
            'Ì'..='Ï' => Some('I'),
            'ò'..='ö' | 'ø' => Some('o'),
            'Ò'..='Ö' | 'Ø' => Some('O'),
            'ù'..='ü' => Some('u'),
            'Ù'..='Ü' => Some('U'),
            'ç' => Some('c'),
            'Ç' => Some('C'),
            'ñ' => Some('n'),
            'Ñ' => Some('N'),
            'ß' => Some('s'),
            c if c.is_ascii_graphic() || c == ' ' => Some(c),
            _ => None,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// The `Authorization` header value, with the token when there is one.
pub fn auth_header(id: &Identity, token: Option<&Token>) -> AppResult<HeaderValue> {
    let mut v = format!(
        "MediaBrowser Client=\"Platter\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
        field(&id.device_name),
        field(&id.device_id),
        field(&id.version)
    );
    if let Some(t) = token {
        v.push_str(&format!(", Token=\"{}\"", field(t.expose())));
    }
    let mut h = HeaderValue::from_str(&v).map_err(|_| AppError::Auth("token has invalid characters".into()))?;
    h.set_sensitive(true);
    Ok(h)
}

/// The address as typed, tidied: a scheme, no trailing slash, no `/web`
/// page path or fragment.
pub fn normalize_url(raw: &str) -> AppResult<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::Invalid("Type your Jellyfin server's address.".into()));
    }
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("http://{raw}") };
    let mut u = Url::parse(&with_scheme).map_err(|_| AppError::Invalid("That isn't a web address.".into()))?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        return Err(AppError::Invalid("That isn't a web address.".into()));
    }
    u.set_fragment(None);
    u.set_query(None);
    let mut path = u.path().trim_end_matches('/').to_string();
    for page in ["/web/index.html", "/web"] {
        if let Some(p) = path.strip_suffix(page) {
            path = p.to_string();
        }
    }
    u.set_path(&path);
    Ok(u.as_str().trim_end_matches('/').to_string())
}

/// A connection to one server, signed in or not. Cheap to clone.
#[derive(Clone)]
pub struct JellyfinClient {
    http: reqwest::Client,
    base: String,
    id: Arc<Identity>,
    token: Option<Token>,
    user_id: Option<String>,
}

impl std::fmt::Debug for JellyfinClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JellyfinClient").field("signed_in", &self.token.is_some()).finish()
    }
}

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(30))
        // The sign-in rides on every request: never to another host.
        .redirect(crate::web::same_origin_redirects())
        .build()
        .unwrap_or_default()
}

impl JellyfinClient {
    pub fn new(http: reqwest::Client, base: &str, id: Arc<Identity>) -> AppResult<Self> {
        Ok(Self { http, base: normalize_url(base)?, id, token: None, user_id: None })
    }

    pub fn signed_in(mut self, token: Token, user_id: &str) -> Self {
        self.token = Some(token);
        self.user_id = Some(user_id.to_string());
        self
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn identity(&self) -> &Identity {
        &self.id
    }

    pub fn user_id(&self) -> AppResult<&str> {
        self.user_id.as_deref().ok_or_else(|| AppError::Auth(SIGN_IN_AGAIN.into()))
    }

    pub fn token(&self) -> Option<&Token> {
        self.token.as_ref()
    }

    /// The full address of a server path (`/Items`); query pairs added.
    pub fn url(&self, path: &str, query: &[(&str, String)]) -> AppResult<Url> {
        let mut u = Url::parse(&format!("{}{path}", self.base)).map_err(|e| AppError::Internal(format!("url: {e}")))?;
        if !query.is_empty() {
            let mut q = u.query_pairs_mut();
            for (k, v) in query {
                q.append_pair(k, v);
            }
        }
        Ok(u)
    }

    pub fn header(&self) -> AppResult<HeaderValue> {
        auth_header(&self.id, self.token.as_ref())
    }

    /// Headers for a stream the audio engine opens.
    pub fn stream_headers(&self) -> AppResult<Vec<(String, String)>> {
        let h = self.header()?;
        Ok(vec![("Authorization".into(), h.to_str().unwrap_or_default().to_string())])
    }

    fn scrub(&self, s: String) -> String {
        match &self.token {
            Some(t) => scrub(&s, t),
            None => s,
        }
    }

    /// Send a request; errors come back as the app's error kinds.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&serde_json::Value>,
    ) -> AppResult<reqwest::Response> {
        let url = self.url(path, query)?;
        let mut req = self.http.request(method, url).header("Authorization", self.header()?).header("Accept", "application/json");
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| {
            let cert = crate::web::certificate_problem(&e);
            tracing::debug!("jellyfin request failed: {}", self.scrub(e.without_url().to_string()));
            if cert {
                AppError::Network(crate::web::CERT_REFUSED.into())
            } else {
                AppError::Network(OFFLINE.into())
            }
        })?;
        if let Some(note) = crate::web::redirect_note(&resp) {
            return Err(AppError::Invalid(note));
        }
        check(resp.status())?;
        Ok(resp)
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> AppResult<T> {
        let resp = self.send(Method::GET, path, query, None).await?;
        let bytes = crate::web::body_capped(resp, crate::web::SERVER_MAX, |_| AppError::Network(OFFLINE.into())).await?;
        serde_json::from_slice(&bytes).map_err(|e| AppError::Decode(format!("{path}: {e}")))
    }

    pub async fn post_json<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)], body: &serde_json::Value) -> AppResult<T> {
        let resp = self.send(Method::POST, path, query, Some(body)).await?;
        let bytes = crate::web::body_capped(resp, crate::web::SERVER_MAX, |_| AppError::Network(OFFLINE.into())).await?;
        serde_json::from_slice(&bytes).map_err(|e| AppError::Decode(format!("{path}: {e}")))
    }

    /// A request whose answer isn't read.
    pub async fn call(&self, method: Method, path: &str, query: &[(&str, String)], body: Option<&serde_json::Value>) -> AppResult<()> {
        self.send(method, path, query, body).await.map(|_| ())
    }

    /// Who the server is, without signing in. Fails for an address that
    /// isn't a Jellyfin server, or one older than [`MIN_VERSION`].
    pub async fn public_info(&self) -> AppResult<PublicInfo> {
        let resp = self.send(Method::GET, "/System/Info/Public", &[], None).await.map_err(|e| match e {
            AppError::NotFound(_) | AppError::Auth(_) | AppError::Server(_) => AppError::Invalid(NOT_JELLYFIN.into()),
            other => other,
        })?;
        let bytes = crate::web::body_capped(resp, crate::web::SERVER_MAX, |_| AppError::Network(OFFLINE.into())).await?;
        let info: PublicInfo = serde_json::from_slice(&bytes).map_err(|_| AppError::Invalid(NOT_JELLYFIN.into()))?;
        check_info(&info)?;
        Ok(info)
    }
}

/// A server's public info is Jellyfin's, new enough.
pub fn check_info(info: &PublicInfo) -> AppResult<()> {
    let jellyfin = info.id.as_deref().is_some_and(|i| !i.trim().is_empty())
        && info.product_name.as_deref().is_none_or(|p| p.to_ascii_lowercase().contains("jellyfin"));
    if !jellyfin {
        return Err(AppError::Invalid(NOT_JELLYFIN.into()));
    }
    let v = info.version.as_deref().unwrap_or("");
    match version(v) {
        Some(found) if found >= MIN_VERSION => Ok(()),
        _ => Err(AppError::Invalid(format!(
            "This Jellyfin server is version {}. Platter needs {}.{} or later; update the server to use it here.",
            if v.is_empty() { "unknown" } else { v },
            MIN_VERSION.0,
            MIN_VERSION.1
        ))),
    }
}

/// Status codes as the app's errors.
pub fn check(s: StatusCode) -> AppResult<()> {
    match s {
        s if s.is_success() => Ok(()),
        StatusCode::UNAUTHORIZED => Err(AppError::Auth(SIGN_IN_AGAIN.into())),
        StatusCode::FORBIDDEN => Err(AppError::Auth(NOT_ALLOWED.into())),
        StatusCode::NOT_FOUND => Err(AppError::NotFound("not on the Jellyfin server".into())),
        s if s.is_server_error() => Err(AppError::Server(format!("Jellyfin answered {}", s.as_u16()))),
        s => Err(AppError::Server(format!("Jellyfin answered {}", s.as_u16()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_computer_name_outside_ascii_still_makes_a_header() {
        assert_eq!(ascii_name("Jürgen's Büro-PC"), "Jurgen's Buro-PC");
        assert_eq!(ascii_name("音楽"), "");
        let id = Identity { device_id: "d1".into(), device_name: "Café 音楽".into(), version: "1".into() };
        let h = auth_header(&id, None).unwrap();
        assert!(h.to_str().unwrap().contains("Device=\"Caf \""), "{h:?}");
    }

    #[test]
    fn addresses_are_tidied() {
        assert_eq!(normalize_url("media.example:8096").unwrap(), "http://media.example:8096");
        assert_eq!(normalize_url(" https://Media.example/jf/web/index.html#!/home ").unwrap(), "https://media.example/jf");
        assert_eq!(normalize_url("http://media.example:8096/").unwrap(), "http://media.example:8096");
        assert!(normalize_url("").is_err());
        assert!(normalize_url("ftp://x.example").is_err());
    }

    #[test]
    fn header_has_no_way_out_of_its_fields() {
        let id = Identity { device_id: "dev\"ice".into(), device_name: "Den, PC".into(), version: "1.0".into() };
        let t = Token::new("tok").unwrap();
        let h = auth_header(&id, Some(&t)).unwrap();
        assert!(h.is_sensitive());
        let s = h.to_str().unwrap();
        assert!(s.starts_with("MediaBrowser Client=\"Platter\""));
        assert!(s.contains("DeviceId=\"device\"") && s.contains("Device=\"Den PC\"") && s.ends_with("Token=\"tok\""));
        assert!(!auth_header(&id, None).unwrap().to_str().unwrap().contains("Token"));
    }

    #[test]
    fn only_new_enough_jellyfin_passes() {
        let info = |id: &str, product: Option<&str>, v: &str| PublicInfo {
            id: Some(id.into()),
            product_name: product.map(str::to_string),
            version: Some(v.into()),
            ..Default::default()
        };
        assert!(check_info(&info("abc", Some("Jellyfin Server"), "10.9.11")).is_ok());
        assert!(check_info(&info("abc", None, "10.10.0")).is_ok());
        let old = check_info(&info("abc", Some("Jellyfin Server"), "10.8.13")).unwrap_err();
        assert!(old.to_string().contains("10.8.13") && old.to_string().contains("10.9"));
        assert_eq!(check_info(&info("", Some("Jellyfin Server"), "10.9.0")).unwrap_err().to_string(), format!("invalid: {NOT_JELLYFIN}"));
        assert!(check_info(&info("abc", Some("Emby Server"), "4.8.0")).is_err());
    }
}
