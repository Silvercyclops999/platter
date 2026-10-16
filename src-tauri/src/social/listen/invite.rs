//! Invite links: `platter://join/<token>`.
//!
//! The token is base64url of
//!
//! ```text
//! version (1) ‖ session id (16) ‖ invite secret (16) ‖ expiry (u32 BE, unix s) ‖ [relay URL]
//! ```
//!
//! Everything in it is random except the expiry, and the relay URL, which
//! is there only when the host uses its own relay instead of the default.
//! No IP, host, port, Plex address, machine id or token is ever in it. The
//! secret is good once: the first person who answers it gets in, and the
//! host turns away anyone else (see `seal::pake_host`).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

use crate::remote::peer::auth::hex;

const VERSION: u8 = 1;
const FIXED: usize = 1 + 16 + 16 + 4;
const RELAY_MAX: usize = 200;

#[derive(Clone, PartialEq, Eq)]
pub struct Token {
    pub session: [u8; 16],
    pub secret: [u8; 16],
    /// Unix seconds.
    pub expires: u32,
    /// The relay, when it isn't the default one.
    pub relay: Option<String>,
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(..)")
    }
}

impl Token {
    /// The session id as shown to the UI (random, says nothing).
    pub fn session_hex(&self) -> String {
        hex(&self.session)
    }

    /// The secret the invite's channels and key agreement come from.
    pub fn seed(&self) -> Vec<u8> {
        [self.session.as_slice(), self.secret.as_slice()].concat()
    }

    pub fn link(&self) -> String {
        self.link_as("join")
    }

    /// The same token under another link word (`platter://<word>/<token>`):
    /// collaborative playlists and Blends use their own.
    pub fn link_as(&self, word: &str) -> String {
        let mut b = Vec::with_capacity(FIXED + RELAY_MAX);
        b.push(VERSION);
        b.extend_from_slice(&self.session);
        b.extend_from_slice(&self.secret);
        b.extend_from_slice(&self.expires.to_be_bytes());
        if let Some(r) = &self.relay {
            b.extend_from_slice(r.as_bytes());
        }
        format!("platter://{word}/{}", B64.encode(b))
    }
}

/// A relay URL a user typed: `http(s)://host[:port][/path]`, no query,
/// no user name or password. Normalised without a trailing slash.
pub fn relay_url(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_end_matches('/');
    if raw.is_empty() || raw.len() > RELAY_MAX {
        return None;
    }
    let u = url::Url::parse(raw).ok()?;
    let ok = matches!(u.scheme(), "https" | "http")
        && u.host_str().is_some_and(|h| !h.is_empty())
        && u.username().is_empty()
        && u.password().is_none()
        && u.query().is_none()
        && u.fragment().is_none();
    ok.then(|| raw.to_string())
}

/// Read an invite link. None when it isn't one.
pub fn parse(raw: &str) -> Option<Token> {
    parse_as(raw, "join")
}

/// Read an invite link made with [`Token::link_as`] and this word.
pub fn parse_as(raw: &str, want: &str) -> Option<Token> {
    let u = url::Url::parse(raw.trim()).ok()?;
    if !u.scheme().eq_ignore_ascii_case("platter") {
        return None;
    }
    let mut segs: Vec<String> = Vec::new();
    if let Some(h) = u.host_str().filter(|h| !h.is_empty()) {
        segs.push(h.to_ascii_lowercase());
    }
    segs.extend(u.path().split('/').filter(|s| !s.is_empty()).map(str::to_string));
    let [word, token] = segs.as_slice() else { return None };
    if word != want || token.len() > 400 || u.query().is_some() {
        return None;
    }
    let b = B64.decode(token).ok()?;
    if b.len() < FIXED || b[0] != VERSION {
        return None;
    }
    let session: [u8; 16] = b[1..17].try_into().ok()?;
    let secret: [u8; 16] = b[17..33].try_into().ok()?;
    let expires = u32::from_be_bytes(b[33..37].try_into().ok()?);
    let relay = if b.len() > FIXED {
        let s = std::str::from_utf8(&b[FIXED..]).ok()?;
        Some(relay_url(s).filter(|r| r == s)?)
    } else {
        None
    };
    Some(Token { session, secret, expires, relay })
}
