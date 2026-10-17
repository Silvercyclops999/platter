//! The home Platter route: a desktop away from home reaches Lidarr through
//! its Bridge. The desktop side is [`BridgeRoute`] (a Lidarr route like the
//! home and away addresses); the Bridge side is [`serve`], which checks
//! every request again before it goes to Lidarr's home address.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use super::link::Link;
use super::peer::Wire;
use super::wire::Msg;
use crate::artistinfo::http::HttpResponse;
use crate::error::{AppError, AppResult};
use crate::lidarr::api::{DirectRoute, LidarrGet, Route};
use crate::lidarr::relayed;
use crate::lidarr::write::{LidarrWrite, RelayedWrite, WriteCall};
use crate::social::listen::seal;

/// How long an answer through the relay may take.
pub const ANSWER_WAIT: Duration = Duration::from_secs(45);

type Waiting = Mutex<HashMap<u64, oneshot::Sender<Result<HttpResponse, String>>>>;

/// Requests waiting for the Bridge's answer.
#[derive(Default)]
pub struct Pending {
    next: AtomicU64,
    waiting: Waiting,
}

impl Pending {
    /// An answer (or a refusal) arrived.
    pub fn resolve(&self, id: u64, r: Result<HttpResponse, String>) {
        if let Some(tx) = self.waiting.lock().remove(&id) {
            let _ = tx.send(r);
        }
    }
}

/// Lidarr's answer as the Bridge sent it (body base64).
pub fn response(status: u16, body: &str) -> HttpResponse {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body).unwrap_or_default();
    HttpResponse { status, body: Bytes::from(bytes), ..Default::default() }
}

pub struct BridgeRoute {
    pub wire: Arc<Wire>,
    pub link: Link,
    pub pending: Arc<Pending>,
}

impl BridgeRoute {
    async fn ask(&self, make: impl FnOnce(u64) -> Msg) -> AppResult<HttpResponse> {
        let id = self.pending.next.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending.waiting.lock().insert(id, tx);
        if let Err(e) = self.wire.send(&self.link, &make(id)).await {
            self.pending.waiting.lock().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(ANSWER_WAIT, rx).await {
            Ok(Ok(Ok(r))) => Ok(r),
            Ok(Ok(Err(why))) => Err(AppError::Network(why)),
            _ => {
                self.pending.waiting.lock().remove(&id);
                Err(AppError::Network("Your Bridge didn't answer in time.".into()))
            }
        }
    }
}

#[async_trait]
impl LidarrGet for BridgeRoute {
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> AppResult<HttpResponse> {
        let path = path.to_string();
        let query: Vec<(String, String)> = query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.ask(|id| Msg::Get { id, path, query }).await
    }

    fn route(&self) -> Route {
        Route::Bridge
    }
}

#[async_trait]
impl LidarrWrite for BridgeRoute {
    async fn write(&self, call: &WriteCall) -> AppResult<HttpResponse> {
        let (method, path, body) = (call.method().as_str().to_string(), call.path(), call.body());
        self.ask(|id| Msg::Write { id, method, path, body }).await
    }

    fn route(&self) -> Route {
        Route::Bridge
    }
}

/// A read path the Bridge passes on: Lidarr's API, nothing odd.
fn ok_path(p: &str) -> bool {
    p.starts_with('/')
        && p.len() <= 200
        && !p.contains("..")
        && !p.contains("//")
        && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.'))
}

/// On the Bridge: answer one request from a paired desktop. None for
/// anything that isn't a request. Reads are limited to the pages Platter
/// asks for, with keys and passwords taken out of the answer. Every write's
/// body is checked against Lidarr and `staging` (Lidarr's staging folder)
/// and rebuilt from only what Platter sends (`lidarr::relayed`).
pub async fn serve(home: Option<&DirectRoute>, staging: &str, msg: &Msg) -> Option<Msg> {
    let id = match msg {
        Msg::Get { id, .. } | Msg::Write { id, .. } => *id,
        _ => return None,
    };
    let Some(home) = home else {
        return Some(Msg::Failed { id, message: "Your Bridge doesn't know Lidarr's address yet.".into() });
    };
    let refused = || Some(Msg::Failed { id, message: "Your Bridge didn't pass that on.".into() });
    let r = match msg {
        Msg::Get { path, query, .. } => {
            if !ok_path(path) || !relayed::read_allowed(path, query) {
                return refused();
            }
            match relayed::folder_read_ok(home, path, query, staging).await {
                Ok(true) => {}
                Ok(false) => return refused(),
                Err(e) => return Some(Msg::Failed { id, message: e.to_string() }),
            }
            let q: Vec<(&str, &str)> = query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            home.get(path, &q).await.map(|mut resp| {
                resp.body = relayed::strip_secrets(&resp.body);
                resp
            })
        }
        Msg::Write { method, path, body, .. } => {
            let checked = match RelayedWrite::check(method, path, body) {
                Ok(w) => relayed::vet(&w, home, staging).await,
                Err(e) => Err(e),
            };
            match checked {
                Ok(w) => home.send_write(&WriteCall::Relayed(w)).await,
                Err(e) => return Some(Msg::Failed { id, message: e.to_string() }),
            }
        }
        _ => return None,
    };
    Some(match r {
        Ok(resp) => Msg::Answer { id, status: resp.status, body: seal::b64(&resp.body) },
        Err(_) => Msg::Failed { id, message: "Your Bridge can't reach Lidarr right now.".into() },
    })
}
