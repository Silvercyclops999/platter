//! The control API's part in copying keys between devices (the exchange
//! itself is in `crate::sync::keycopy`). Plugged into the control port's
//! extra routes ([`KeyCopyRoutes`]). Requests are signed with the account
//! key like every other control request, so only devices of the same Plex
//! account get this far; the code then proves it's the device the user is
//! looking at.
//!
//! - `GET  /platter/v1/keys/waiting` → [`KeysWaiting`]: is a code showing here?
//! - `POST /platter/v1/keys/hello`   ← [`KeysHello`] → [`KeysHelloReply`]
//! - `POST /platter/v1/keys/deliver` ← [`KeysDeliver`]

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::auth::{Verifier, now_ms};
use super::server::{ExtraRoutes, KeySource, app_err, json_err, refused};
use crate::error::AppResult;

pub const PATH_KEYS_WAITING: &str = "/platter/v1/keys/waiting";
pub const PATH_KEYS_HELLO: &str = "/platter/v1/keys/hello";
pub const PATH_KEYS_DELIVER: &str = "/platter/v1/keys/deliver";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysWaiting {
    pub waiting: bool,
}

/// The sender's half of the key exchange (hex).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysHello {
    pub msg: String,
    /// The sender's name, to show on the receiving side.
    #[serde(default)]
    pub name: String,
}

/// The receiver's half, and proof it reached the same key (hex).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysHelloReply {
    pub msg: String,
    pub confirm: String,
}

/// The sealed keys (hex).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysDeliver {
    pub nonce: String,
    pub sealed: String,
}

impl std::fmt::Debug for KeysDeliver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeysDeliver(..)")
    }
}

/// The receiving side of a key copy.
#[async_trait]
pub trait PeerKeys: Send + Sync {
    async fn waiting(&self) -> bool;
    /// `from` is the sender's device id, from its request signature.
    async fn hello(&self, from: &str, req: KeysHello) -> AppResult<KeysHelloReply>;
    async fn deliver(&self, from: &str, req: KeysDeliver) -> AppResult<()>;
}

/// The key copy requests as extra routes on the control port.
pub struct KeyCopyRoutes {
    own_id: String,
    keys: Arc<dyn KeySource>,
    handler: Arc<dyn PeerKeys>,
    verifier: Verifier,
}

impl KeyCopyRoutes {
    pub fn new(own_id: String, keys: Arc<dyn KeySource>, handler: Arc<dyn PeerKeys>) -> Self {
        Self { own_id, keys, handler, verifier: Verifier::new() }
    }
}

#[async_trait]
impl ExtraRoutes for KeyCopyRoutes {
    fn handles(&self, path: &str) -> bool {
        matches!(path, PATH_KEYS_WAITING | PATH_KEYS_HELLO | PATH_KEYS_DELIVER)
    }

    async fn respond(&self, method: &str, target: &str, auth: Option<&str>, body: &[u8]) -> (u16, String) {
        let path = target.split('?').next().unwrap_or(target);
        let want = if path == PATH_KEYS_WAITING { "GET" } else { "POST" };
        if method != want {
            return (404, json_err("notFound", "Not found"));
        }
        let key = self.keys.key().await;
        let from = match self.verifier.verify(key.as_ref(), auth, method, target, &self.own_id, body, now_ms()) {
            Ok(from) => from,
            Err(r) => {
                tracing::info!(reason = ?r, "refused a key copy request");
                return refused(r);
            }
        };
        let h = &self.handler;
        match path {
            PATH_KEYS_WAITING => (200, serde_json::to_string(&KeysWaiting { waiting: h.waiting().await }).unwrap_or_default()),
            PATH_KEYS_HELLO => match serde_json::from_slice::<KeysHello>(body) {
                Ok(req) => match h.hello(&from, req).await {
                    Ok(reply) => (200, serde_json::to_string(&reply).unwrap_or_default()),
                    Err(e) => app_err(e),
                },
                Err(_) => (400, json_err("invalid", "Bad request")),
            },
            _ => match serde_json::from_slice::<KeysDeliver>(body) {
                Ok(req) => match h.deliver(&from, req).await {
                    Ok(()) => (200, "{}".into()),
                    Err(e) => app_err(e),
                },
                Err(_) => (400, json_err("invalid", "Bad request")),
            },
        }
    }
}

/// Several sets of extra routes on one port (the first that handles a path answers).
pub struct RouteList(pub Vec<Arc<dyn ExtraRoutes>>);

#[async_trait]
impl ExtraRoutes for RouteList {
    fn handles(&self, path: &str) -> bool {
        self.0.iter().any(|r| r.handles(path))
    }

    async fn respond(&self, method: &str, target: &str, auth: Option<&str>, body: &[u8]) -> (u16, String) {
        let path = target.split('?').next().unwrap_or(target);
        match self.0.iter().find(|r| r.handles(path)) {
            Some(r) => r.respond(method, target, auth, body).await,
            None => (404, json_err("notFound", "Not found")),
        }
    }
}
