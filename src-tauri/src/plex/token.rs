//! The Plex token as a secret value.
//!
//! `Token` deliberately has no `Display`, no `Serialize`, and a `Debug` that
//! prints `Token(<redacted>)`. The only way to read it is [`Token::expose`],
//! which is used to set the `X-Plex-Token` header (marked sensitive) and to
//! write it to the keychain.

use reqwest::header::HeaderValue;

use crate::error::{AppError, AppResult};

#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    pub fn new(value: impl Into<String>) -> AppResult<Self> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(AppError::Auth("empty token".into()));
        }
        Ok(Token(trimmed.to_string()))
    }

    /// The raw token. Only for request headers and the keychain.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Header value for `X-Plex-Token`, flagged sensitive so reqwest/http
    /// never print it.
    pub fn header_value(&self) -> AppResult<HeaderValue> {
        let mut v = HeaderValue::from_str(&self.0)
            .map_err(|_| AppError::Auth("token has invalid characters".into()))?;
        v.set_sensitive(true);
        Ok(v)
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// Remove any occurrence of `secret` from `text`. Used as a belt-and-braces
/// pass over error messages built from server responses.
pub fn scrub(text: &str, secret: &Token) -> String {
    if secret.0.is_empty() {
        return text.to_string();
    }
    text.replace(&secret.0, "<redacted>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_redacted() {
        let t = Token::new("super-secret-abc").unwrap();
        let s = format!("{t:?} {:?}", Some(t.clone()));
        assert!(!s.contains("super-secret-abc"));
        assert!(s.contains("redacted"));
    }

    #[test]
    fn empty_token_rejected() {
        assert!(Token::new("  ").is_err());
    }

    #[test]
    fn header_is_sensitive() {
        let t = Token::new("abc").unwrap();
        let h = t.header_value().unwrap();
        assert!(h.is_sensitive());
        assert!(!format!("{h:?}").contains("abc"));
    }

    #[test]
    fn scrub_removes_token() {
        let t = Token::new("tok123").unwrap();
        assert_eq!(scrub("x tok123 y", &t), "x <redacted> y");
    }
}
