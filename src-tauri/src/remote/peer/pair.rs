//! The control API's part in pairing two of the user's devices with the
//! 6-digit code (the exchange itself is in `crate::sync::devices::pair`).
//! Plugged into the control port's extra routes ([`PairRoutes`]) beside key
//! copy and device sync. Requests are signed and checked like every other
//! control request, so only devices of the same Plex account get this far;
//! the code then proves it's the device the user is looking at.
//!
//! - `GET  /platter/v1/pair/waiting` → [`PairWaiting`]: is a code showing here?
//! - `POST /platter/v1/pair/hello`   ← [`PairHello`] → [`PairHelloReply`]
//! - `POST /platter/v1/pair/finish`  ← [`PairFinish`] → [`PairFinishReply`]

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::auth::{Verifier, now_ms};
use super::server::{ExtraRoutes, KeySource, app_err, check, json_err, refused};
use crate::error::AppResult;

pub const PATH_PAIR_WAITING: &str = "/platter/v1/pair/waiting";
pub const PATH_PAIR_HELLO: &str = "/platter/v1/pair/hello";
pub const PATH_PAIR_FINISH: &str = "/platter/v1/pair/finish";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairWaiting {
    pub waiting: bool,
}

/// The joining device's half of the key exchange (hex).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairHello {
    pub msg: String,
}

/// The showing device's half of the key exchange (hex). Nothing derived
/// from the key: the joining device proves it has the code first, in its
/// finish, so a wrong guess can only be tested against the showing device
/// (which counts it).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairHelloReply {
    pub msg: String,
}

impl std::fmt::Debug for PairHelloReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairHelloReply(..)")
    }
}

/// The showing device's answer to a finish that proved the code: proof it
/// reached the same key (hex) and its side of the link, sealed with it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairFinishReply {
    pub confirm: String,
    pub sealed: String,
}

impl std::fmt::Debug for PairFinishReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairFinishReply(..)")
    }
}

/// The joining device's side of the link, sealed with the agreed key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairFinish {
    pub sealed: String,
}

impl std::fmt::Debug for PairFinish {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairFinish(..)")
    }
}

/// The showing side of a pairing.
#[async_trait]
pub trait PeerPair: Send + Sync {
    async fn waiting(&self) -> bool;
    /// `from` is the joining device's id, from its request signature.
    async fn hello(&self, from: &str, req: PairHello) -> AppResult<PairHelloReply>;
    async fn finish(&self, from: &str, req: PairFinish) -> AppResult<PairFinishReply>;
}

/// The pairing requests as extra routes on the control port.
pub struct PairRoutes {
    own_id: String,
    keys: Arc<dyn KeySource>,
    handler: Arc<dyn PeerPair>,
    verifier: Verifier,
}

impl PairRoutes {
    pub fn new(own_id: String, keys: Arc<dyn KeySource>, handler: Arc<dyn PeerPair>) -> Self {
        Self { own_id, keys, handler, verifier: Verifier::new() }
    }
}

#[async_trait]
impl ExtraRoutes for PairRoutes {
    fn handles(&self, path: &str) -> bool {
        matches!(path, PATH_PAIR_WAITING | PATH_PAIR_HELLO | PATH_PAIR_FINISH)
    }

    async fn respond(&self, method: &str, target: &str, auth: Option<&str>, body: &[u8]) -> (u16, String) {
        let path = target.split('?').next().unwrap_or(target);
        let want = if path == PATH_PAIR_WAITING { "GET" } else { "POST" };
        if method != want {
            return (404, json_err("notFound", "Not found"));
        }
        let from = match check(&self.verifier, self.keys.as_ref(), auth, method, target, &self.own_id, body, now_ms()).await {
            Ok(from) => from,
            Err(r) => {
                tracing::info!(reason = ?r, "refused a pairing request");
                return refused(r);
            }
        };
        let h = &self.handler;
        match path {
            PATH_PAIR_WAITING => ok_json(&PairWaiting { waiting: h.waiting().await }),
            PATH_PAIR_HELLO => match serde_json::from_slice::<PairHello>(body) {
                Ok(req) => match h.hello(&from, req).await {
                    Ok(reply) => ok_json(&reply),
                    Err(e) => app_err(e),
                },
                Err(_) => (400, json_err("invalid", "Bad request")),
            },
            _ => match serde_json::from_slice::<PairFinish>(body) {
                Ok(req) => match h.finish(&from, req).await {
                    Ok(reply) => ok_json(&reply),
                    Err(e) => app_err(e),
                },
                Err(_) => (400, json_err("invalid", "Bad request")),
            },
        }
    }
}

/// A 200 with this answer, or a 500 (logged) when it can't be written.
pub(crate) fn ok_json<T: Serialize>(v: &T) -> (u16, String) {
    match serde_json::to_string(v) {
        Ok(text) => (200, text),
        Err(e) => {
            tracing::warn!("couldn't write an answer: {e}");
            (500, json_err("internal", "Couldn't write the answer"))
        }
    }
}
