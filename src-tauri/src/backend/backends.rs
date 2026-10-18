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
    /// Plex libraries included besides the main one (from Settings).
    plex_extras: Mutex<Vec<crate::db::sync::ExtraLibrary>>,
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
                plex_extras: Mutex::new(Vec::new()),
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
        let record = ServerRecord::new(&tag, BackendKind::Plex, &name);
        // The connected Plex server stays primary unless another kind of
        // server was made primary.
        let plex_primary = file.primary.as_deref().and_then(|p| file.get(p)).is_none_or(|r| r.kind == BackendKind::Plex);
        let mut changed = file.upsert(record);
        if plex_primary {
            changed |= file.set_primary(&tag);
        }
        if changed && let Err(e) = file.save(dir) {
            tracing::warn!("servers not saved: {e}");
        }
        changed
    }

    /// Signed out of Plex: its servers leave the saved list (another kind of
    /// server becomes primary). True when the list changed.
    pub fn forget_plex(&self) -> bool {
        let mut g = self.inner.records.lock();
        let Some((dir, file)) = g.as_mut() else { return false };
        let plex: Vec<String> = file.servers.iter().filter(|r| r.kind == BackendKind::Plex).map(|r| r.tag.clone()).collect();
        let mut changed = false;
        for t in plex {
            changed |= file.remove(&t);
        }
        if changed && let Err(e) = file.save(dir) {
            tracing::warn!("servers not saved: {e}");
        }
        changed
    }

    /// Change the saved list (then save it). Returns what `f` returns.
    pub fn edit_records<T>(&self, f: impl FnOnce(&mut ServersFile) -> T) -> AppResult<T> {
        let mut g = self.inner.records.lock();
        let (dir, file) = g.as_mut().ok_or_else(|| AppError::Internal("server list not loaded".into()))?;
        let out = f(file);
        file.save(dir)?;
        Ok(out)
    }

    /// The registered server with this tag (not the Plex account).
    pub fn server(&self, tag: &str) -> Option<Arc<dyn MusicServer>> {
        self.inner.servers.read().get(tag).cloned()
    }

    /// Plex libraries included besides the main one.
    pub fn set_plex_extras(&self, libs: Vec<crate::db::sync::ExtraLibrary>) {
        *self.inner.plex_extras.lock() = libs;
    }

    /// Every library to sync besides the main one: the Plex ones from
    /// Settings, then each other server's included libraries.
    pub fn extra_libraries(&self) -> Vec<crate::db::sync::ExtraLibrary> {
        let mut out = self.inner.plex_extras.lock().clone();
        let mut servers: Vec<(String, Arc<dyn MusicServer>)> =
            self.inner.servers.read().iter().map(|(t, s)| (t.clone(), s.clone())).collect();
        servers.sort_by(|a, b| a.0.cmp(&b.0));
        for (tag, s) in servers {
            for (view, source) in s.libraries() {
                out.push(crate::db::sync::ExtraLibrary { src: crate::db::store::src_of(&tag, &view), tag: tag.clone(), source });
            }
        }
        out
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

    /// The primary server: the one Settings names, when it's in use; else
    /// the Plex account.
    pub fn primary(&self) -> AppResult<Arc<dyn MusicServer>> {
        let named = self.inner.records.lock().as_ref().and_then(|(_, f)| f.primary.clone());
        if let Some(tag) = named
            && let Some(s) = self.server(&tag)
        {
            return Ok(s);
        }
        self.default_server().ok_or_else(|| AppError::Network(NO_SERVER.into()))
    }

    /// The server whose library is the main one in the cache: the Plex
    /// account while one is set up (so its library stays as it was), else
    /// the primary server.
    pub fn main_server(&self) -> AppResult<Arc<dyn MusicServer>> {
        if self.plex_set_up() && let Some(p) = self.default_server() {
            return Ok(p);
        }
        self.primary()
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

    /// Whether a Plex account is part of the setup: no servers saved yet
    /// (Plex is how most start), or a Plex one among them.
    fn plex_set_up(&self) -> bool {
        let g = self.inner.records.lock();
        match g.as_ref() {
            Some((_, f)) => f.servers.is_empty() || f.servers.iter().any(|r| r.kind == BackendKind::Plex),
            None => true,
        }
    }

    /// Every server in use.
    pub fn all(&self) -> Vec<Arc<dyn MusicServer>> {
        let mut out: Vec<Arc<dyn MusicServer>> = self.default_server().into_iter().collect();
        out.extend(self.inner.servers.read().values().cloned());
        out
    }

    /// The servers whose features count: like [`Self::all`], without the
    /// Plex account when only Jellyfin or Subsonic servers are set up, so
    /// Plex-only features don't show for them.
    pub fn in_use(&self) -> Vec<Arc<dyn MusicServer>> {
        let mut out: Vec<Arc<dyn MusicServer>> = if self.plex_set_up() { self.default_server().into_iter().collect() } else { Vec::new() };
        out.extend(self.inner.servers.read().values().cloned());
        out
    }

    /// What any server in use can do.
    pub fn caps_all(&self) -> Caps {
        self.in_use().iter().fold(Caps::NONE, |a, s| a | s.caps())
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
