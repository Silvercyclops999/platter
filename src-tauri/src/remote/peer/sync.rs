//! The control API's part in keeping the user's devices in step (the
//! protocol itself is in `crate::sync::devices`). Plugged into the control
//! port's extra routes ([`SyncRoutes`]) beside key copy. Requests are
//! signed and checked like every other control request, so only devices
//! of the same Plex account get this far, and every message inside is
//! sealed for the device link it belongs to.
//!
//! - `POST /platter/v1/sync/hello`   ← [`LanBody`] → [`LanBody`]: a round
//! - `POST /platter/v1/sync/entries` ← [`LanBody`] → [`LanBody`]: changes,
//!   and the rest of a round

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::auth::{Verifier, now_ms};
use super::server::{ExtraRoutes, KeySource, app_err, check, json_err, refused};
use crate::error::AppResult;

pub const PATH_SYNC_HELLO: &str = "/platter/v1/sync/hello";
pub const PATH_SYNC_ENTRIES: &str = "/platter/v1/sync/entries";

/// Sealed messages on one device link: each message is its frames, as
/// they would go to the relay (only under the link's network keys).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanBody {
    /// The device link's id.
    pub link: String,
    pub msgs: Vec<Vec<String>>,
}

impl std::fmt::Debug for LanBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanBody").field("msgs", &self.msgs.len()).finish_non_exhaustive()
    }
}

/// The receiving side of device sync on the local network.
#[async_trait]
pub trait PeerSync: Send + Sync {
    /// `from` is the sender's device id, from its request signature (the
    /// signature proved it's a device of this account).
    async fn exchange(&self, from: &str, req: LanBody) -> AppResult<LanBody>;
}

/// The device sync requests as extra routes on the control port.
pub struct SyncRoutes {
    own_id: String,
    keys: Arc<dyn KeySource>,
    handler: Arc<dyn PeerSync>,
    verifier: Verifier,
}

impl SyncRoutes {
    pub fn new(own_id: String, keys: Arc<dyn KeySource>, handler: Arc<dyn PeerSync>) -> Self {
        Self { own_id, keys, handler, verifier: Verifier::new() }
    }
}

#[async_trait]
impl ExtraRoutes for SyncRoutes {
    fn handles(&self, path: &str) -> bool {
        matches!(path, PATH_SYNC_HELLO | PATH_SYNC_ENTRIES)
    }

    async fn respond(&self, method: &str, target: &str, auth: Option<&str>, body: &[u8]) -> (u16, String) {
        if method != "POST" {
            return (404, json_err("notFound", "Not found"));
        }
        let from = match check(&self.verifier, self.keys.as_ref(), auth, method, target, &self.own_id, body, now_ms()).await {
            Ok(from) => from,
            Err(r) => {
                tracing::info!(reason = ?r, "refused a device sync request");
                return refused(r);
            }
        };
        match serde_json::from_slice::<LanBody>(body) {
            Ok(req) => match self.handler.exchange(&from, req).await {
                Ok(reply) => super::pair::ok_json(&reply),
                Err(e) => app_err(e),
            },
            Err(_) => (400, json_err("invalid", "Bad request")),
        }
    }
}
