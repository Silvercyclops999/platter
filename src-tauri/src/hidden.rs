//! Hidden songs and artists ("Hide song", "Don't play this artist").
//!
//! Hidden items are skipped by everything Platter picks for you: mixes,
//! radio, "keep playing similar music" and shuffle play. They still play
//! when picked directly. The list lives behind [`HiddenStore`], in the sync
//! store's `hidden` namespace ([`SyncHidden`], keyed `kind:id`), so it
//! travels in the sync file when that's on. The ids the pickers check are
//! kept in memory ([`HiddenIds`]) and swapped whole on every change.

use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::plex::state::EventSink;
use crate::sync::SyncStore;
use crate::sync::store::{self as ss, HIDDEN, Op};

pub const EVENT_HIDDEN: &str = "hidden://changed";
/// Most items one list keeps.
pub const MAX_HIDDEN: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HiddenKind {
    Track,
    Artist,
}

impl HiddenKind {
    pub fn key(self) -> &'static str {
        match self {
            HiddenKind::Track => "track",
            HiddenKind::Artist => "artist",
        }
    }

    fn from_key(k: &str) -> Option<Self> {
        match k {
            "track" => Some(HiddenKind::Track),
            "artist" => Some(HiddenKind::Artist),
            _ => None,
        }
    }
}

/// One hidden song or artist, with what Settings shows for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenItem {
    pub kind: HiddenKind,
    pub id: String,
    pub title: String,
    /// A song's artist.
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
    /// Unix seconds.
    pub hidden_at: i64,
}

/// Where the hidden list lives.
pub trait HiddenStore: Send + Sync {
    fn list(&self) -> AppResult<Vec<HiddenItem>>;
    fn save(&self, all: &[HiddenItem]) -> AppResult<()>;
}

/// The hidden list in the sync store.
pub struct SyncHidden(pub SyncStore);

fn entry_key(kind: HiddenKind, id: &str) -> String {
    format!("{}:{id}", kind.key())
}

impl HiddenStore for SyncHidden {
    /// Songs and artists only (other kinds and snoozes are another feature's).
    fn list(&self) -> AppResult<Vec<HiddenItem>> {
        Ok(self
            .0
            .list(&HIDDEN)?
            .into_iter()
            .filter(|(_, h)| h.until.is_none())
            .filter_map(|(key, h)| {
                let kind = HiddenKind::from_key(&h.kind)?;
                let id = key.split_once(':')?.1.to_string();
                Some(HiddenItem { kind, id, title: h.title, subtitle: h.subtitle, thumb: h.thumb, hidden_at: h.hidden_at })
            })
            .collect())
    }

    fn save(&self, all: &[HiddenItem]) -> AppResult<()> {
        let cur = self.list()?;
        let mut ops = Vec::new();
        for h in all {
            if !cur.contains(h) {
                let v = ss::Hidden {
                    kind: h.kind.key().into(),
                    title: h.title.clone(),
                    until: None,
                    subtitle: h.subtitle.clone(),
                    thumb: h.thumb.clone(),
                    hidden_at: h.hidden_at,
                };
                ops.push(Op::put(&HIDDEN, entry_key(h.kind, &h.id), &v)?);
            }
        }
        for h in &cur {
            if !all.iter().any(|x| x.kind == h.kind && x.id == h.id) {
                ops.push(Op::remove(&HIDDEN, entry_key(h.kind, &h.id)));
            }
        }
        self.0.write_many(ops)
    }
}

/// The ids the pickers skip. `version` changes with every edit, so cached
/// snapshots know to rebuild.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenSet {
    pub tracks: HashSet<String>,
    pub artists: HashSet<String>,
    pub version: u64,
}

impl HiddenSet {
    pub fn from_items(items: &[HiddenItem], version: u64) -> Self {
        let mut s = HiddenSet { version, ..Default::default() };
        for i in items {
            match i.kind {
                HiddenKind::Track => s.tracks.insert(i.id.clone()),
                HiddenKind::Artist => s.artists.insert(i.id.clone()),
            };
        }
        s
    }

    /// Whether a song (by its id and its artist's id) is hidden.
    pub fn hides(&self, track_id: &str, artist_id: &str) -> bool {
        self.tracks.contains(track_id) || self.artists.contains(artist_id)
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.artists.is_empty()
    }
}

/// The current hidden ids, shared with the mix engine and the player's lookup.
#[derive(Clone, Default)]
pub struct HiddenIds(Arc<RwLock<Arc<HiddenSet>>>);

impl HiddenIds {
    pub fn get(&self) -> Arc<HiddenSet> {
        self.0.read().clone()
    }

    pub fn set(&self, s: HiddenSet) {
        *self.0.write() = Arc::new(s);
    }
}

/// `hidden_set` payload: what to hide or unhide.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideRequest {
    pub kind: HiddenKind,
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
}

/// Hide and unhide. Cheap to clone.
#[derive(Clone)]
pub struct Hidden {
    store: Arc<dyn HiddenStore>,
    ids: HiddenIds,
    events: Arc<dyn EventSink>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    lock: Arc<parking_lot::Mutex<()>>,
}

impl Hidden {
    pub fn new(
        store: Arc<dyn HiddenStore>,
        ids: HiddenIds,
        events: Arc<dyn EventSink>,
        clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        let me = Self { store, ids, events, clock, lock: Default::default() };
        match me.store.list() {
            Ok(all) => me.ids.set(HiddenSet::from_items(&all, 1)),
            Err(e) => tracing::warn!("hidden list: {e}"),
        }
        me
    }

    pub fn ids(&self) -> HiddenIds {
        self.ids.clone()
    }

    /// Read the list again (another device changed it) and tell the UI.
    pub fn reload(&self) {
        match self.list() {
            Ok(all) => {
                let version = self.ids.get().version + 1;
                self.ids.set(HiddenSet::from_items(&all, version));
                self.events.emit(EVENT_HIDDEN, serde_json::to_value(&all).unwrap_or_default());
            }
            Err(e) => tracing::warn!("hidden list: {e}"),
        }
    }

    pub fn list(&self) -> AppResult<Vec<HiddenItem>> {
        let mut all = self.store.list()?;
        all.sort_by_key(|a| std::cmp::Reverse(a.hidden_at));
        Ok(all)
    }

    /// Hide (`on`) or unhide one song or artist; resolves to the whole list.
    /// Hiding again keeps the first date.
    pub fn set(&self, req: HideRequest, on: bool) -> AppResult<Vec<HiddenItem>> {
        let id = req.id.trim();
        if id.is_empty() || id.len() > 300 {
            return Err(AppError::Invalid("Nothing to hide.".into()));
        }
        let _g = self.lock.lock();
        let mut all = self.store.list()?;
        let had = all.iter().any(|h| h.kind == req.kind && h.id == id);
        match (on, had) {
            (true, false) => {
                if all.len() >= MAX_HIDDEN {
                    return Err(AppError::Invalid("You've hidden as much as Platter can keep.".into()));
                }
                all.push(HiddenItem {
                    kind: req.kind,
                    id: id.to_string(),
                    title: req.title.trim().to_string(),
                    subtitle: req.subtitle.filter(|s| !s.trim().is_empty()),
                    thumb: req.thumb,
                    hidden_at: (self.clock)(),
                });
            }
            (false, true) => all.retain(|h| !(h.kind == req.kind && h.id == id)),
            _ => return self.list(),
        }
        self.store.save(&all)?;
        let version = self.ids.get().version + 1;
        self.ids.set(HiddenSet::from_items(&all, version));
        let sorted = self.list()?;
        self.events.emit(EVENT_HIDDEN, serde_json::to_value(&sorted).unwrap_or_default());
        Ok(sorted)
    }
}

// ---- commands -----------------------------------------------------------------

#[tauri::command]
pub async fn hidden_list(hidden: State<'_, Hidden>) -> AppResult<Vec<HiddenItem>> {
    let h = hidden.inner().clone();
    tokio::task::spawn_blocking(move || h.list()).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn hidden_set(hidden: State<'_, Hidden>, item: HideRequest, on: bool) -> AppResult<Vec<HiddenItem>> {
    let h = hidden.inner().clone();
    tokio::task::spawn_blocking(move || h.set(item, on)).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct Events(Mutex<Vec<String>>);
    impl EventSink for Events {
        fn emit(&self, event: &str, _: serde_json::Value) {
            self.0.lock().push(event.to_string());
        }
    }

    fn rig() -> (Hidden, SyncStore, Arc<Events>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::DbHandle::open(dir.path().join("h.db")).unwrap();
        let store = SyncStore::new(db, "dev1");
        let events = Arc::new(Events::default());
        let h = Hidden::new(Arc::new(SyncHidden(store.clone())), HiddenIds::default(), events.clone(), Arc::new(|| 100));
        (h, store, events, dir)
    }

    fn req(kind: HiddenKind, id: &str) -> HideRequest {
        HideRequest { kind, id: id.into(), title: format!("T{id}"), subtitle: None, thumb: None }
    }

    #[test]
    fn hide_unhide_and_the_shared_ids_follow() {
        let (h, store, events, _d) = rig();
        let ids = h.ids();
        h.set(req(HiddenKind::Track, "5"), true).unwrap();
        let all = h.set(req(HiddenKind::Artist, "9"), true).unwrap();
        assert_eq!(all.len(), 2);
        assert!(ids.get().hides("5", "x"));
        assert!(ids.get().hides("6", "9"), "an artist hides every song of theirs");
        assert!(!ids.get().hides("6", "8"));
        // Hiding twice keeps one.
        assert_eq!(h.set(req(HiddenKind::Track, "5"), true).unwrap().len(), 2);
        let v = ids.get().version;
        let all = h.set(req(HiddenKind::Track, "5"), false).unwrap();
        assert_eq!(all.len(), 1);
        assert!(ids.get().version > v);
        assert!(!ids.get().hides("5", "x"));
        // Kept in the sync store (keyed kind:id) and read again by a new service.
        assert!(store.get(&HIDDEN, "artist:9").unwrap().is_some());
        assert!(store.get(&HIDDEN, "track:5").unwrap().is_none(), "unhidden leaves a tombstone");
        let again = Hidden::new(Arc::new(SyncHidden(store.clone())), HiddenIds::default(), events.clone(), Arc::new(|| 1));
        assert!(again.ids().get().hides("1", "9"));
        // Another device hides a song: a reload picks it up.
        store.put(&HIDDEN, "track:77", &ss::Hidden { kind: "track".into(), title: "x".into(), ..Default::default() }).unwrap();
        // Something else's hidden kind is left alone.
        store.put(&HIDDEN, "album:3", &ss::Hidden { kind: "album".into(), ..Default::default() }).unwrap();
        h.reload();
        assert!(ids.get().hides("77", "x"));
        h.set(req(HiddenKind::Track, "8"), true).unwrap();
        assert!(store.get(&HIDDEN, "album:3").unwrap().is_some());
        assert!(events.0.lock().iter().all(|e| e == EVENT_HIDDEN));
        assert!(h.set(req(HiddenKind::Track, " "), true).is_err());
    }
}
