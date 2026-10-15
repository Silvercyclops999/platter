//! The servers the app is using right now, and which one an item belongs
//! to. An item's id carries its server's tag (`crate::sid`); a request for
//! it goes to the server registered under that tag. Ids with no registered
//! server go to the Plex account, which finds the right Plex server itself,
//! exactly as before servers of other kinds existed.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use super::plex::PlexServer;
use super::{BackendKind, Caps, ServerRecord, ServersFile};
use super::server::MusicServer;
use crate::error::{AppError, AppResult};
use crate::plex::PlexState;
use crate::plex::state::EventSink;

/// Event: the servers or what they can do changed (no payload; the UI asks
/// `backend_caps` again).
pub const EVENT_BACKENDS_CHANGED: &str = "backends://changed";

/// What the app shows about every server, whatever its kind: toasts, and
/// whether only music on this computer can play. The Plex state does this
/// today.
pub trait AppStatus: Send + Sync {
    fn toast(&self, kind: &str, message: String);
    fn cached_only(&self) -> bool;
}

impl AppStatus for PlexState {
    fn toast(&self, kind: &str, message: String) {
        PlexState::toast(self, kind, message);
    }
    fn cached_only(&self) -> bool {
        PlexState::cached_only(self)
    }
}

/// Cheap to clone.
#[derive(Clone)]
pub struct Backends {
    inner: Arc<Inner>,
}

struct Inner {
    /// Servers by tag, other than the Plex account.
    servers: RwLock<HashMap<String, Arc<dyn MusicServer>>>,
    /// The Plex account: every id no other server claims.
    plex: Option<Arc<PlexServer>>,
    /// Takes ids no server claims when there's no Plex account (tests).
    fallback: Option<Arc<dyn MusicServer>>,
    status: Option<Arc<dyn AppStatus>>,
    sink: Mutex<Option<Arc<dyn EventSink>>>,
    /// `servers.json` and the folder it's kept in, once set.
    records: Mutex<Option<(std::path::PathBuf, ServersFile)>>,
}

const NO_SERVER: &str = "Your server is offline.";

impl Backends {
    /// The app's servers: the Plex account.
    pub fn with_plex(plex: PlexServer) -> Self {
        let status: Arc<dyn AppStatus> = Arc::new(plex.state().clone());
        Self::build(Some(Arc::new(plex)), None, Some(status))
    }

    /// Servers for tests: `fallback` takes every id the others don't claim.
    pub fn with_fallback(fallback: Arc<dyn MusicServer>) -> Self {
        Self::build(None, Some(fallback), None)
    }

    /// No servers at all (tests).
    #[cfg(test)]
    pub fn empty() -> Self {
        Self::build(None, None, None)
    }

    fn build(plex: Option<Arc<PlexServer>>, fallback: Option<Arc<dyn MusicServer>>, status: Option<Arc<dyn AppStatus>>) -> Self {
        Self {
            inner: Arc::new(Inner {
                servers: RwLock::new(HashMap::new()),
                plex,
                fallback,
                status,
                sink: Mutex::new(None),
                records: Mutex::new(None),
            }),
        }
    }

    pub fn set_event_sink(&self, sink: Arc<dyn EventSink>) {
        *self.inner.sink.lock() = Some(sink);
    }

    /// Keep `servers.json` in `dir` (read now, written on changes).
    pub fn keep_records_in(&self, dir: std::path::PathBuf) {
        let file = ServersFile::load(&dir);
        *self.inner.records.lock() = Some((dir, file));
    }

    /// The saved server records.
    pub fn records(&self) -> ServersFile {
        self.inner.records.lock().as_ref().map(|(_, f)| f.clone()).unwrap_or_default()
    }

    /// Record the connected Plex server (its tag and name, no secrets) as
    /// the primary server. True when the saved list changed.
    pub async fn remember_plex(&self) -> bool {
        let Some(p) = self.plex().cloned() else { return false };
        let (Some(tag), Some(name)) = (p.tag().await, MusicServer::name(p.as_ref()).await) else { return false };
        let mut g = self.inner.records.lock();
        let Some((dir, file)) = g.as_mut() else { return false };
        let record = ServerRecord { tag: tag.clone(), kind: BackendKind::Plex, name, url: None, user: None };
        let changed = file.upsert(record) | file.set_primary(&tag);
        if changed && let Err(e) = file.save(dir) {
            tracing::warn!("servers not saved: {e}");
        }
        changed
    }

    /// Add a server under its tag (replacing one with the same tag).
    pub fn add(&self, tag: &str, server: Arc<dyn MusicServer>) {
        self.inner.servers.write().insert(tag.to_string(), server);
        self.changed();
    }

    pub fn remove(&self, tag: &str) {
        if self.inner.servers.write().remove(tag).is_some() {
            self.changed();
        }
    }

    /// Tell the UI the servers or their capabilities changed.
    pub fn changed(&self) {
        let sink = self.inner.sink.lock().clone();
        if let Some(sink) = sink {
            sink.emit(EVENT_BACKENDS_CHANGED, serde_json::json!({}));
        }
    }

    /// The Plex account, when there is one.
    pub fn plex(&self) -> Option<&Arc<PlexServer>> {
        self.inner.plex.as_ref()
    }

    fn default_server(&self) -> Option<Arc<dyn MusicServer>> {
        match &self.inner.plex {
            Some(p) => Some(p.clone() as Arc<dyn MusicServer>),
            None => self.inner.fallback.clone(),
        }
    }

    /// The server with the main library.
    pub fn primary(&self) -> AppResult<Arc<dyn MusicServer>> {
        self.default_server().ok_or_else(|| AppError::Network(NO_SERVER.into()))
    }

    /// The server an item (id or image path) belongs to.
    pub fn for_id(&self, id: &str) -> AppResult<Arc<dyn MusicServer>> {
        if let Some(tag) = crate::sid::owner(id)
            && let Some(s) = self.inner.servers.read().get(&tag)
        {
            return Ok(s.clone());
        }
        self.primary()
    }

    /// Every server in use.
    pub fn all(&self) -> Vec<Arc<dyn MusicServer>> {
        let mut out: Vec<Arc<dyn MusicServer>> = self.default_server().into_iter().collect();
        out.extend(self.inner.servers.read().values().cloned());
        out
    }

    /// What any server in use can do.
    pub fn caps_all(&self) -> Caps {
        self.all().iter().fold(Caps::NONE, |a, s| a | s.caps())
    }

    /// What the server with this item can do (none when there's no server).
    pub fn caps_for(&self, id: &str) -> Caps {
        self.for_id(id).map(|s| s.caps()).unwrap_or(Caps::NONE)
    }

    pub fn toast(&self, kind: &str, message: impl Into<String>) {
        if let Some(s) = &self.inner.status {
            s.toast(kind, message.into());
        }
    }

    /// Only music on this computer can play now.
    pub fn cached_only(&self) -> bool {
        self.inner.status.as_ref().is_some_and(|s| s.cached_only())
    }
}
