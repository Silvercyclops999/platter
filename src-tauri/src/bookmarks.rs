//! Bookmarks: a song at a moment in it, or a page (album, artist,
//! playlist, mix...). Opening a song bookmark plays from that time; a page
//! bookmark opens the page.
//!
//! Kept behind [`BookmarkStore`], in the sync store's `bookmarks` namespace
//! ([`SyncBookmarks`], keyed by bookmark id), so they travel in the sync
//! file when that's on.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::audio::player::PlayerHandle;
use crate::audio::source::{ContextKind, PlayContext};
use crate::error::{AppError, AppResult};
use crate::plex::state::EventSink;
use crate::sync::SyncStore;
use crate::sync::store::{self as ss, BOOKMARKS, Op};

pub const EVENT_BOOKMARKS: &str = "bookmarks://changed";
pub const MAX_BOOKMARKS: usize = 2000;

/// Pages a bookmark can open (the UI's route names).
pub const PAGE_KINDS: &[&str] = &["album", "artist", "playlist", "mix", "radio", "liked", "smart", "discography"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BookmarkKind {
    /// A song, at `position_ms`.
    Track,
    /// A page, named by `page` and `target`.
    Page,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub kind: BookmarkKind,
    /// The song's id, or the page's id ("" for pages without one).
    pub target: String,
    /// The page's route name (pages only).
    #[serde(default)]
    pub page: Option<String>,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
    /// Where in the song (songs only).
    #[serde(default)]
    pub position_ms: Option<u64>,
    /// Unix seconds.
    pub created_at: i64,
}

/// `bookmark_add` payload.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewBookmark {
    pub kind: BookmarkKind,
    pub target: String,
    #[serde(default)]
    pub page: Option<String>,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub position_ms: Option<u64>,
}

/// Where the bookmarks live.
pub trait BookmarkStore: Send + Sync {
    fn list(&self) -> AppResult<Vec<Bookmark>>;
    fn save(&self, all: &[Bookmark]) -> AppResult<()>;
}

/// Bookmarks in the sync store.
pub struct SyncBookmarks(pub SyncStore);

fn to_entry(b: &Bookmark) -> ss::Bookmark {
    ss::Bookmark {
        position_ms: b.position_ms.unwrap_or(0),
        title: b.title.clone(),
        kind: match b.kind {
            BookmarkKind::Track => "track".into(),
            BookmarkKind::Page => "page".into(),
        },
        target: b.target.clone(),
        page: b.page.clone(),
        subtitle: b.subtitle.clone(),
        thumb: b.thumb.clone(),
        created_at: b.created_at,
    }
}

fn from_entry(id: String, e: ss::Bookmark) -> Option<Bookmark> {
    let kind = match e.kind.as_str() {
        "track" => BookmarkKind::Track,
        "page" => BookmarkKind::Page,
        _ => return None,
    };
    Some(Bookmark {
        id,
        position_ms: (kind == BookmarkKind::Track).then_some(e.position_ms),
        kind,
        target: e.target,
        page: e.page,
        title: e.title,
        subtitle: e.subtitle,
        thumb: e.thumb,
        created_at: e.created_at,
    })
}

impl BookmarkStore for SyncBookmarks {
    fn list(&self) -> AppResult<Vec<Bookmark>> {
        Ok(self.0.list(&BOOKMARKS)?.into_iter().filter_map(|(k, e)| from_entry(k, e)).collect())
    }

    fn save(&self, all: &[Bookmark]) -> AppResult<()> {
        let cur = self.list()?;
        let mut ops = Vec::new();
        for b in all {
            if !cur.contains(b) {
                ops.push(Op::put(&BOOKMARKS, &b.id, &to_entry(b))?);
            }
        }
        for b in &cur {
            if !all.iter().any(|x| x.id == b.id) {
                ops.push(Op::remove(&BOOKMARKS, &b.id));
            }
        }
        self.0.write_many(ops)
    }
}

/// Add, remove and list bookmarks. Cheap to clone.
#[derive(Clone)]
pub struct Bookmarks {
    store: Arc<dyn BookmarkStore>,
    events: Arc<dyn EventSink>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    lock: Arc<parking_lot::Mutex<()>>,
}

impl Bookmarks {
    pub fn new(store: Arc<dyn BookmarkStore>, events: Arc<dyn EventSink>, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        Self { store, events, clock, lock: Default::default() }
    }

    /// Newest first.
    pub fn list(&self) -> AppResult<Vec<Bookmark>> {
        let mut all = self.store.list()?;
        all.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        Ok(all)
    }

    /// Tell the UI (also after another device changed them).
    pub fn changed(&self) -> AppResult<Vec<Bookmark>> {
        let all = self.list()?;
        self.events.emit(EVENT_BOOKMARKS, serde_json::to_value(&all).unwrap_or_default());
        Ok(all)
    }

    /// A new bookmark. A song bookmarked again at (about) the same second,
    /// or a page bookmarked again, keeps the one already there.
    pub fn add(&self, b: NewBookmark) -> AppResult<Bookmark> {
        let target = b.target.trim().to_string();
        let page = match b.kind {
            BookmarkKind::Track => {
                if target.is_empty() {
                    return Err(AppError::Invalid("Nothing to bookmark.".into()));
                }
                None
            }
            BookmarkKind::Page => {
                let p = b.page.as_deref().map(str::trim).unwrap_or("");
                if !PAGE_KINDS.contains(&p) {
                    return Err(AppError::Invalid("This page can't be bookmarked.".into()));
                }
                Some(p.to_string())
            }
        };
        if target.len() > 300 || b.title.chars().count() > 300 {
            return Err(AppError::Invalid("Nothing to bookmark.".into()));
        }
        let position_ms = match b.kind {
            BookmarkKind::Track => Some(b.position_ms.unwrap_or(0)),
            BookmarkKind::Page => None,
        };
        let _g = self.lock.lock();
        let mut all = self.store.list()?;
        let same = all.iter().find(|x| {
            x.kind == b.kind
                && x.target == target
                && x.page == page
                && match (x.position_ms, position_ms) {
                    (Some(a), Some(c)) => a / 1000 == c / 1000,
                    _ => true,
                }
        });
        if let Some(same) = same {
            return Ok(same.clone());
        }
        if all.len() >= MAX_BOOKMARKS {
            return Err(AppError::Invalid("You have as many bookmarks as Platter can keep.".into()));
        }
        let bm = Bookmark {
            id: uuid::Uuid::new_v4().to_string(),
            kind: b.kind,
            target,
            page,
            title: b.title.trim().to_string(),
            subtitle: b.subtitle.filter(|s| !s.trim().is_empty()),
            thumb: b.thumb,
            position_ms,
            created_at: (self.clock)(),
        };
        all.push(bm.clone());
        self.store.save(&all)?;
        drop(_g);
        self.changed()?;
        Ok(bm)
    }

    pub fn remove(&self, id: &str) -> AppResult<Vec<Bookmark>> {
        let _g = self.lock.lock();
        let mut all = self.store.list()?;
        let n = all.len();
        all.retain(|b| b.id != id);
        if all.len() != n {
            self.store.save(&all)?;
        }
        drop(_g);
        self.changed()
    }

    pub fn get(&self, id: &str) -> AppResult<Bookmark> {
        self.store.list()?.into_iter().find(|b| b.id == id).ok_or_else(|| AppError::NotFound("bookmark".into()))
    }
}

/// Plays a song bookmark (the player in the app, a recorder in tests).
#[async_trait::async_trait]
pub trait BookmarkPlayer: Send + Sync {
    async fn play_at(&self, track_id: &str, position_ms: u64) -> AppResult<()>;
}

#[async_trait::async_trait]
impl BookmarkPlayer for PlayerHandle {
    async fn play_at(&self, track_id: &str, position_ms: u64) -> AppResult<()> {
        let ctx = PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(vec![track_id.to_string()]) };
        self.play_context_at(ctx, Some(track_id.to_string()), position_ms, true).await
    }
}

/// Open a song bookmark: play it from its time. Pages open in the UI.
pub async fn open(bookmarks: &Bookmarks, player: &dyn BookmarkPlayer, id: &str) -> AppResult<Bookmark> {
    let b = bookmarks.get(id)?;
    if b.kind == BookmarkKind::Track {
        player.play_at(&b.target, b.position_ms.unwrap_or(0)).await?;
    }
    Ok(b)
}

// ---- commands -----------------------------------------------------------------

#[tauri::command]
pub async fn bookmarks_list(bookmarks: State<'_, Bookmarks>) -> AppResult<Vec<Bookmark>> {
    let b = bookmarks.inner().clone();
    tokio::task::spawn_blocking(move || b.list()).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn bookmark_add(bookmarks: State<'_, Bookmarks>, bookmark: NewBookmark) -> AppResult<Bookmark> {
    let b = bookmarks.inner().clone();
    tokio::task::spawn_blocking(move || b.add(bookmark)).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn bookmark_remove(bookmarks: State<'_, Bookmarks>, id: String) -> AppResult<Vec<Bookmark>> {
    let b = bookmarks.inner().clone();
    tokio::task::spawn_blocking(move || b.remove(&id)).await.map_err(|e| AppError::Internal(e.to_string()))?
}

/// Open a bookmark: a song plays from its time; resolves to the bookmark so
/// the UI can open a page one.
#[tauri::command]
pub async fn bookmark_open(bookmarks: State<'_, Bookmarks>, player: State<'_, PlayerHandle>, id: String) -> AppResult<Bookmark> {
    open(bookmarks.inner(), player.inner(), &id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    struct NoEvents;
    impl EventSink for NoEvents {
        fn emit(&self, _: &str, _: serde_json::Value) {}
    }

    #[derive(Default)]
    struct Played(Mutex<Vec<(String, u64)>>);
    #[async_trait::async_trait]
    impl BookmarkPlayer for Played {
        async fn play_at(&self, track_id: &str, position_ms: u64) -> AppResult<()> {
            self.0.lock().push((track_id.into(), position_ms));
            Ok(())
        }
    }

    fn rig() -> (Bookmarks, SyncStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = SyncStore::new(crate::db::DbHandle::open(dir.path().join("b.db")).unwrap(), "dev1");
        let clock = Arc::new(parking_lot::Mutex::new(100i64));
        let c = clock.clone();
        let b = Bookmarks::new(Arc::new(SyncBookmarks(db.clone())), Arc::new(NoEvents), Arc::new(move || {
            let mut t = c.lock();
            *t += 1;
            *t
        }));
        (b, db, dir)
    }

    fn song(id: &str, ms: u64) -> NewBookmark {
        NewBookmark { kind: BookmarkKind::Track, target: id.into(), page: None, title: "Song".into(), subtitle: Some("Band".into()), thumb: None, position_ms: Some(ms) }
    }

    #[tokio::test]
    async fn create_list_open_and_remove() {
        let (b, db, _d) = rig();
        let first = b.add(song("7", 61_500)).unwrap();
        // The same second again keeps one; another moment is another bookmark.
        assert_eq!(b.add(song("7", 61_900)).unwrap().id, first.id);
        b.add(song("7", 120_000)).unwrap();
        let page = b
            .add(NewBookmark { kind: BookmarkKind::Page, target: "42".into(), page: Some("album".into()), title: "Record".into(), subtitle: None, thumb: None, position_ms: Some(5) })
            .unwrap();
        assert_eq!(page.position_ms, None, "pages have no time");
        assert!(b.add(NewBookmark { page: Some("settings".into()), ..song("x", 0) }.into_page()).is_err());
        let all = b.list().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, page.id, "newest first");

        let played = Played::default();
        open(&b, &played, &first.id).await.unwrap();
        open(&b, &played, &page.id).await.unwrap();
        assert_eq!(*played.0.lock(), vec![("7".to_string(), 61_500)], "only songs play");

        assert_eq!(b.remove(&first.id).unwrap().len(), 2);
        assert_eq!(SyncBookmarks(db.clone()).list().unwrap().len(), 2);
        assert!(db.get(&BOOKMARKS, &page.id).unwrap().is_some(), "kept in the sync store by id");
        assert!(open(&b, &played, &first.id).await.is_err());
    }

    impl NewBookmark {
        fn into_page(mut self) -> Self {
            self.kind = BookmarkKind::Page;
            self
        }
    }
}
