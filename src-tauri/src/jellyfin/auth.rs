//! Signing in to a Jellyfin server: Quick Connect (a code the user approves
//! in another Jellyfin app they're signed in to), or a user name and
//! password typed once and never stored. The token Jellyfin gives back goes
//! to the keychain under `jf.<tag>` (with the profile prefix, like every
//! other entry).

use reqwest::Method;

use super::client::{JellyfinClient, NOT_ALLOWED, SIGN_IN_AGAIN};
use super::types::{AuthResult, PublicInfo, QuickConnectResult, User};
use crate::backend::BackendKind;
use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;

pub const QUICK_CONNECT_OFF: &str =
    "Quick Connect is turned off on this Jellyfin server. Sign in with your user name and password instead.";
pub const QUICK_CONNECT_EXPIRED: &str = "That code ran out. Start again for a new one.";
pub const WRONG_PASSWORD: &str = "That user name and password didn't work.";

/// The keychain entry for a server's token.
pub fn key_account(tag: &str) -> String {
    format!("jf.{tag}")
}

/// The tag a Jellyfin server's ids carry.
pub fn tag_of(info: &PublicInfo) -> String {
    BackendKind::Jellyfin.tag(info.id.as_deref().unwrap_or_default())
}

/// Signed in: who, where and the token.
#[derive(Debug, Clone)]
pub struct SignedIn {
    pub tag: String,
    pub info: PublicInfo,
    pub token: Token,
    pub user: User,
}

/// Whether Quick Connect is on.
pub async fn quick_connect_enabled(c: &JellyfinClient) -> AppResult<bool> {
    match c.get_json::<bool>("/QuickConnect/Enabled", &[]).await {
        Ok(on) => Ok(on),
        Err(AppError::NotFound(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// A new Quick Connect request: its code (shown) and secret (kept here).
pub async fn quick_connect_start(c: &JellyfinClient) -> AppResult<QuickConnectResult> {
    let r: QuickConnectResult = c.post_json("/QuickConnect/Initiate", &[], &serde_json::json!({})).await.map_err(|e| match e {
        // A server with Quick Connect off refuses to start one.
        AppError::Auth(_) | AppError::NotFound(_) => AppError::Invalid(QUICK_CONNECT_OFF.into()),
        other => other,
    })?;
    if r.secret.is_empty() || r.code.is_empty() {
        return Err(AppError::Invalid(QUICK_CONNECT_OFF.into()));
    }
    Ok(r)
}

/// Whether the user approved the code yet.
pub async fn quick_connect_approved(c: &JellyfinClient, secret: &str) -> AppResult<bool> {
    let r: QuickConnectResult = c
        .get_json("/QuickConnect/Connect", &[("Secret", secret.to_string())])
        .await
        .map_err(|e| match e {
            AppError::NotFound(_) | AppError::Auth(_) => AppError::Invalid(QUICK_CONNECT_EXPIRED.into()),
            other => other,
        })?;
    Ok(r.authenticated)
}

/// Trade an approved Quick Connect secret for a token.
pub async fn quick_connect_finish(c: &JellyfinClient, info: PublicInfo, secret: &str) -> AppResult<SignedIn> {
    let r: AuthResult = c
        .post_json("/Users/AuthenticateWithQuickConnect", &[], &serde_json::json!({ "Secret": secret }))
        .await
        .map_err(|e| match e {
            AppError::Auth(_) | AppError::NotFound(_) => AppError::Invalid(QUICK_CONNECT_EXPIRED.into()),
            other => other,
        })?;
    finish(info, r)
}

/// Sign in with a user name and password. The password is used for this
/// one request and dropped.
pub async fn password(c: &JellyfinClient, info: PublicInfo, user: &str, password: String) -> AppResult<SignedIn> {
    let body = serde_json::json!({ "Username": user.trim(), "Pw": password });
    drop(password);
    let r = c.post_json::<AuthResult>("/Users/AuthenticateByName", &[], &body).await;
    drop(body);
    let r = r.map_err(|e| match e {
        AppError::Auth(m) if m == SIGN_IN_AGAIN || m == NOT_ALLOWED => AppError::Auth(WRONG_PASSWORD.into()),
        AppError::NotFound(_) => AppError::Auth(WRONG_PASSWORD.into()),
        other => other,
    })?;
    finish(info, r)
}

fn finish(info: PublicInfo, r: AuthResult) -> AppResult<SignedIn> {
    let token = Token::new(r.access_token.unwrap_or_default()).map_err(|_| AppError::Auth(SIGN_IN_AGAIN.into()))?;
    let user = r.user.filter(|u| !u.id.is_empty()).ok_or_else(|| AppError::Decode("sign-in answer without a user".into()))?;
    Ok(SignedIn { tag: tag_of(&info), info, token, user })
}

/// Who the token belongs to now (`/Users/Me`): fails with an auth error
/// when the token was revoked.
pub async fn me(c: &JellyfinClient) -> AppResult<User> {
    c.get_json("/Users/Me", &[]).await
}

/// Keep the token in the keychain.
pub fn save_token(store: &dyn TokenStore, tag: &str, token: &Token) -> AppResult<()> {
    store.set(&key_account(tag), token)
}

pub fn load_token(store: &dyn TokenStore, tag: &str) -> AppResult<Option<Token>> {
    store.get(&key_account(tag))
}

/// Forget the token, and tell the server to end the session (best effort).
pub async fn sign_out(store: &dyn TokenStore, tag: &str, client: Option<&JellyfinClient>) -> AppResult<()> {
    if let Some(c) = client
        && let Err(e) = c.call(Method::POST, "/Sessions/Logout", &[], None).await
    {
        tracing::debug!("jellyfin sign out not sent: {}", e.kind());
    }
    store.delete(&key_account(tag))
}
