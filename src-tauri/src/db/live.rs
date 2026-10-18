//! Live library updates: items the server says were added, changed or
//! removed (its `timeline` notifications) go straight into the cache, a
//! batch at a time, without a sync.
//!
//! The caller collects changes for [`BATCH_WINDOW`] after the first one,
//! then calls [`apply`]. A batch too big to fetch item by item (a big scan),
//! or one the source can't answer, comes back as [`LiveOutcome::NeedsSync`]
//! and the caller runs the usual incremental sync instead.

use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use crate::error::AppResult;

use super::DbHandle;
use super::source::{ItemKind, LibrarySource};
use super::store;

/// How long changes are gathered before they're applied.
pub const BATCH_WINDOW: Duration = Duration::from_secs(2);
/// More items than this in one batch: run a sync instead.
pub const MAX_BATCH: usize = 200;

/// Plex timeline states (`TimelineEntry.state`).
const STATE_CREATED: i64 = 0;
const STATE_DONE: i64 = 5;
const STATE_DELETED: i64 = 9;

/// One item changed on the server.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LiveChange {
    /// Added or changed: read it again.
    Upsert(ItemKindKey, String),
    /// Removed from the library.
    Remove(ItemKindKey, String),
}

/// [`ItemKind`] with an order, so batches are stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemKindKey {
    Artist,
    Album,
    Track,
}

impl From<ItemKindKey> for ItemKind {
    fn from(k: ItemKindKey) -> Self {
        match k {
            ItemKindKey::Artist => ItemKind::Artist,
            ItemKindKey::Album => ItemKind::Album,
            ItemKindKey::Track => ItemKind::Track,
        }
    }
}

/// What a timeline entry means for the cache. Only artists, albums and
/// tracks in the music section, once Plex has created them (state 0), is
/// done with them (state 5) or has deleted them (state 9). A new item is
/// read as soon as it exists, so it shows up even when the server never
/// says it's done; the done entry or the sync after the scan reads it
/// again. Entries in between (matching, analysing) are left out.
pub fn change_from_timeline(
    section_id: Option<&str>,
    item_id: Option<&str>,
    item_type: Option<i64>,
    state: Option<i64>,
    music_section: Option<&str>,
) -> Option<LiveChange> {
    if let (Some(s), Some(m)) = (section_id, music_section)
        && s != m
    {
        return None;
    }
    let id = item_id.filter(|i| !i.is_empty())?.to_string();
    let kind = match item_type? {
        8 => ItemKindKey::Artist,
        9 => ItemKindKey::Album,
        10 => ItemKindKey::Track,
        _ => return None,
    };
    match state? {
        STATE_CREATED | STATE_DONE => Some(LiveChange::Upsert(kind, id)),
        STATE_DELETED => Some(LiveChange::Remove(kind, id)),
        _ => None,
    }
}

/// Changes gathered for one apply. The last word on an item wins: removed
/// then added back is an upsert, added then removed is a removal.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Batch {
    upsert: BTreeSet<(ItemKindKey, String)>,
    remove: BTreeSet<(ItemKindKey, String)>,
}

impl Batch {
    pub fn add(&mut self, c: LiveChange) {
        match c {
            LiveChange::Upsert(k, id) => {
                self.remove.remove(&(k, id.clone()));
                self.upsert.insert((k, id));
            }
            LiveChange::Remove(k, id) => {
                self.upsert.remove(&(k, id.clone()));
                self.remove.insert((k, id));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.upsert.len() + self.remove.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Ids to read again, in a stable order.
    pub fn upsert_ids(&self) -> Vec<String> {
        self.upsert.iter().map(|(_, id)| id.clone()).collect()
    }

    fn removals(&self, kind: ItemKindKey) -> Vec<String> {
        self.remove.iter().filter(|(k, _)| *k == kind).map(|(_, id)| id.clone()).collect()
    }
}

/// What applying a batch did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveOutcome {
    /// Nothing to do (empty batch, or nothing in it was found).
    Nothing,
    Applied {
        /// Artists, albums and tracks written.
        updated: u64,
        removed: u64,
        /// Albums that weren't in the cache before (What's New looks at these).
        new_albums: Vec<String>,
    },
    /// Too big, no music section yet, or the source can't fetch items: sync.
    NeedsSync,
}

/// Write a batch into the cache. Upserted items are read from `source`;
/// items it no longer has count as removed. Tells the UI through the db's
/// events (`library://changed`) when anything changed.
pub async fn apply(db: &DbHandle, source: &dyn LibrarySource, batch: Batch) -> AppResult<LiveOutcome> {
    if batch.is_empty() {
        return Ok(LiveOutcome::Nothing);
    }
    // Too big, or the cache still owes its server tags (a sync adds them).
    if batch.len() > MAX_BATCH || db.ids_pending() && source.server_tag().await.is_some() {
        return Ok(LiveOutcome::NeedsSync);
    }
    let Some(state) = db.read_async(store::sync_state).await? else {
        return Ok(LiveOutcome::NeedsSync);
    };
    let ids = batch.upsert_ids();
    let mut changes = if ids.is_empty() {
        Default::default()
    } else {
        match source.items(&state.section_id, &ids).await? {
            Some(c) => c,
            None => return Ok(LiveOutcome::NeedsSync),
        }
    };
    // A new song's album and a new album's artist: the server may only name
    // the song (or the album), and pages list albums and artists from their
    // own rows, so read the ones the cache doesn't have yet.
    let parents = missing_parents(db, &changes).await?;
    if !parents.is_empty() {
        match source.items(&state.section_id, &parents).await? {
            Some(more) => {
                let have: HashSet<String> = changes.artists.iter().map(|a| a.id.clone()).chain(changes.albums.iter().map(|a| a.id.clone())).collect();
                changes.artists.extend(more.artists.into_iter().filter(|a| !have.contains(&a.id)));
                changes.albums.extend(more.albums.into_iter().filter(|a| !have.contains(&a.id)));
            }
            None => return Ok(LiveOutcome::NeedsSync),
        }
    }
    // Asked for but not found: gone from the server.
    let found: HashSet<&str> = changes
        .artists
        .iter()
        .map(|a| a.id.as_str())
        .chain(changes.albums.iter().map(|a| a.id.as_str()))
        .chain(changes.tracks.iter().map(|t| t.id.as_str()))
        .collect();
    let mut gone: Vec<(ItemKindKey, String)> = batch
        .upsert
        .iter()
        .filter(|(_, id)| !found.contains(id.as_str()))
        .cloned()
        .collect();
    for k in [ItemKindKey::Track, ItemKindKey::Album, ItemKindKey::Artist] {
        gone.extend(batch.removals(k).into_iter().map(|id| (k, id)));
    }
    let album_ids: Vec<String> = changes.albums.iter().map(|a| a.id.clone()).collect();
    let gen_ = state.generation;
    let (updated, removed, new_albums) = db
        .write_async(move |c| {
            let tx = c.transaction()?;
            let new_albums = {
                let mut st = tx.prepare_cached("SELECT 1 FROM albums WHERE id = ?1")?;
                let mut out = Vec::new();
                for id in &album_ids {
                    if !st.exists([id])? {
                        out.push(id.clone());
                    }
                }
                out
            };
            store::upsert_artists(&tx, &changes.artists, gen_)?;
            store::upsert_albums(&tx, &changes.albums, gen_)?;
            store::upsert_tracks(&tx, &changes.tracks, gen_)?;
            let mut removed = 0;
            for (kind, id) in &gone {
                removed += store::delete_ids(&tx, (*kind).into(), std::slice::from_ref(id))?;
            }
            if !changes.albums.is_empty() || removed > 0 {
                store::refresh_artist_recency(&tx)?;
            }
            tx.commit()?;
            let updated = (changes.artists.len() + changes.albums.len() + changes.tracks.len()) as u64;
            Ok((updated, removed, new_albums))
        })
        .await?;
    if updated == 0 && removed == 0 {
        return Ok(LiveOutcome::Nothing);
    }
    db.bump_generation();
    if let Some(ev) = db.events() {
        ev.library_changed();
    }
    Ok(LiveOutcome::Applied { updated, removed, new_albums })
}

/// Albums and artists the fetched items belong to that are neither in the
/// fetch nor in the cache.
async fn missing_parents(db: &DbHandle, changes: &super::source::Changes) -> AppResult<Vec<String>> {
    let fetched: HashSet<&str> =
        changes.artists.iter().map(|a| a.id.as_str()).chain(changes.albums.iter().map(|a| a.id.as_str())).collect();
    let mut albums: BTreeSet<String> = BTreeSet::new();
    let mut artists: BTreeSet<String> = BTreeSet::new();
    for t in &changes.tracks {
        albums.insert(t.album_id.clone());
        artists.insert(t.artist_id.clone());
    }
    for a in &changes.albums {
        artists.insert(a.artist_id.clone());
    }
    albums.retain(|id| !id.is_empty() && !fetched.contains(id.as_str()));
    artists.retain(|id| !id.is_empty() && !fetched.contains(id.as_str()));
    if albums.is_empty() && artists.is_empty() {
        return Ok(Vec::new());
    }
    db.read_async(move |c| {
        let mut out = Vec::new();
        let mut al = c.prepare_cached("SELECT 1 FROM albums WHERE id = ?1")?;
        for id in albums {
            if !al.exists([&id])? {
                out.push(id);
            }
        }
        let mut ar = c.prepare_cached("SELECT 1 FROM artists WHERE id = ?1")?;
        for id in artists {
            if !ar.exists([&id])? {
                out.push(id);
            }
        }
        Ok(out)
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;

    use super::*;
    use crate::db::fake::{self, FakeLibrary, FakeSource};
    use crate::db::source::LibrarySource;
    use crate::db::{DbEvents, SyncOutcome, SyncStatus};

    #[derive(Default)]
    struct Changed(Mutex<u32>);
    impl DbEvents for Changed {
        fn sync_progress(&self, _: &SyncStatus) {}
        fn library_changed(&self) {
            *self.0.lock() += 1;
        }
    }

    struct T {
        db: DbHandle,
        src: Arc<FakeSource>,
        ev: Arc<Changed>,
        _dir: tempfile::TempDir,
    }

    async fn synced() -> T {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("library.db")).unwrap();
        let src = Arc::new(FakeSource::new(fake::generated(2, 2, 3)));
        let out = db.sync_once(src.clone() as Arc<dyn LibrarySource>, false).await.unwrap();
        assert!(matches!(out, SyncOutcome::Full { .. }));
        let ev = Arc::new(Changed::default());
        db.set_events(ev.clone());
        T { db, src, ev, _dir: dir }
    }

    fn count(db: &DbHandle, table: &str) -> i64 {
        db.read(|c| Ok(c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)).unwrap()
    }

    fn title(db: &DbHandle, table: &str, id: &str) -> Option<String> {
        db.read(|c| {
            Ok(c.query_row(&format!("SELECT title FROM {table} WHERE id = ?1"), [id], |r| r.get(0)).ok())
        })
        .unwrap()
    }

    fn up(kind: ItemKindKey, id: &str) -> LiveChange {
        LiveChange::Upsert(kind, id.into())
    }

    #[test]
    fn timeline_entries_map_to_changes() {
        let m = Some("7");
        assert_eq!(change_from_timeline(Some("7"), Some("10"), Some(10), Some(5), m), Some(up(ItemKindKey::Track, "10")));
        assert_eq!(
            change_from_timeline(Some("7"), Some("9"), Some(9), Some(9), m),
            Some(LiveChange::Remove(ItemKindKey::Album, "9".into()))
        );
        // Just created: read it now.
        assert_eq!(change_from_timeline(Some("7"), Some("10"), Some(10), Some(0), m), Some(up(ItemKindKey::Track, "10")));
        // Being matched or analysed, another library, not music, no id.
        assert_eq!(change_from_timeline(Some("7"), Some("10"), Some(10), Some(2), m), None);
        assert_eq!(change_from_timeline(Some("7"), Some("10"), Some(10), Some(6), m), None);
        assert_eq!(change_from_timeline(Some("2"), Some("10"), Some(10), Some(5), m), None);
        assert_eq!(change_from_timeline(Some("7"), Some("10"), Some(1), Some(5), m), None);
        assert_eq!(change_from_timeline(Some("7"), None, Some(10), Some(5), m), None);
        // The music section isn't known yet: take it.
        assert!(change_from_timeline(Some("2"), Some("1"), Some(8), Some(5), None).is_some());
    }

    #[test]
    fn the_last_word_on_an_item_wins() {
        let mut b = Batch::default();
        b.add(up(ItemKindKey::Track, "1"));
        b.add(LiveChange::Remove(ItemKindKey::Track, "1".into()));
        assert_eq!(b.len(), 1);
        assert!(b.upsert_ids().is_empty());
        b.add(up(ItemKindKey::Track, "1"));
        b.add(up(ItemKindKey::Track, "1"));
        assert_eq!(b.upsert_ids(), vec!["1".to_string()]);
        assert_eq!(b.len(), 1);
    }

    #[tokio::test]
    async fn added_and_changed_items_land_without_a_sync() {
        let t = synced().await;
        let before = count(&t.db, "albums");
        {
            let mut l = t.src.lib.lock();
            let artist = l.artists[0].clone();
            let album = fake::album("new-album", "Fresh Album", &artist, 2026, Some("album"), &[]);
            let track = fake::track("new-track", "Fresh Song", &album, 1, 1);
            l.albums.push(album);
            l.tracks.push(track);
            l.tracks[0].title = "Renamed".into();
        }
        let changed_id = t.src.lib.lock().tracks[0].id.clone();
        let pages = t.src.page_calls.lock().len();
        let mut b = Batch::default();
        b.add(up(ItemKindKey::Album, "new-album"));
        b.add(up(ItemKindKey::Track, "new-track"));
        b.add(up(ItemKindKey::Track, &changed_id));
        let out = apply(&t.db, t.src.as_ref(), b).await.unwrap();
        assert_eq!(out, LiveOutcome::Applied { updated: 3, removed: 0, new_albums: vec!["new-album".into()] });
        assert_eq!(count(&t.db, "albums"), before + 1);
        assert_eq!(title(&t.db, "tracks", "new-track").as_deref(), Some("Fresh Song"));
        assert_eq!(title(&t.db, "tracks", &changed_id).as_deref(), Some("Renamed"));
        assert_eq!(*t.ev.0.lock(), 1, "pages hear about it once per batch");
        // One fetch for the whole batch, and no paging (no sync ran).
        assert_eq!(t.src.item_calls.lock().len(), 1);
        assert_eq!(t.src.page_calls.lock().len(), pages);
    }

    /// The changes in notification frames as the server sends them.
    fn from_frames(frames: &[String]) -> Batch {
        use crate::plex::notifications::{PlexEvent, parse};
        let mut b = Batch::default();
        for f in frames {
            for ev in parse(f) {
                if let PlexEvent::LibraryUpdate { section_id, item_id, item_type, state } = ev
                    && let Some(c) = change_from_timeline(section_id.as_deref(), item_id.as_deref(), item_type, state, Some("1"))
                {
                    b.add(c);
                }
            }
        }
        b
    }

    fn timeline(id: &str, kind: i64, state: i64) -> String {
        format!(
            r#"{{"NotificationContainer":{{"type":"timeline","size":1,"TimelineEntry":[
            {{"identifier":"com.plexapp.plugins.library","sectionID":"1","itemID":"{id}","type":{kind},"title":"x","state":{state}}}]}}}}"#
        )
    }

    #[tokio::test]
    async fn a_new_album_by_a_new_artist_shows_when_the_server_only_names_its_songs() {
        let t = synced().await;
        let section = t.db.read(store::sync_state).unwrap().unwrap().section_id;
        assert_eq!(section, "1", "the fake library's music section");
        {
            let mut l = t.src.lib.lock();
            let artist = fake::artist("new-artist", "Brand New Band");
            let mut album = fake::album("new-album", "Debut", &artist, 2026, Some("album"), &[]);
            album.added_at = 2_000_000_000;
            l.tracks.push(fake::track("new-1", "Opener", &album, 1, 1));
            l.tracks.push(fake::track("new-2", "Closer", &album, 1, 2));
            l.albums.push(album);
            l.artists.push(artist);
        }
        // Only the songs, created and then done, plus noise from elsewhere.
        let frames = [
            timeline("new-1", 10, 0),
            timeline("new-2", 10, 0),
            timeline("new-1", 10, 2),
            timeline("new-1", 10, 5),
            timeline("new-2", 10, 5),
            r#"{"NotificationContainer":{"type":"activity","ActivityNotification":[{"event":"started","uuid":"u","Activity":{"type":"library.update.section"}}]}}"#.to_string(),
        ];
        let b = from_frames(&frames);
        assert_eq!(b.upsert_ids(), ["new-1", "new-2"]);
        let out = apply(&t.db, t.src.as_ref(), b).await.unwrap();
        assert_eq!(out, LiveOutcome::Applied { updated: 4, removed: 0, new_albums: vec!["new-album".into()] });
        assert_eq!(*t.ev.0.lock(), 1, "pages hear library://changed");
        // The album and its artist are in the lists the pages read.
        assert_eq!(title(&t.db, "albums", "new-album").as_deref(), Some("Debut"));
        assert_eq!(title(&t.db, "artists", "new-artist").as_deref(), Some("Brand New Band"));
        let recent = t.db.read(|c| crate::db::queries::recently_added(c, 5)).unwrap();
        assert_eq!(recent[0].id, "new-album");
        let page = t.db.read(|c| crate::db::queries::artist_page(c, "new-artist")).unwrap();
        assert_eq!(page.artist.title, "Brand New Band");
        // Two reads: the songs, then the album and artist they belong to.
        let calls = t.src.item_calls.lock().clone();
        assert_eq!(calls, vec![vec!["new-1".to_string(), "new-2".into()], vec!["new-album".into(), "new-artist".into()]]);
    }

    #[tokio::test]
    async fn removed_items_leave_the_cache() {
        let t = synced().await;
        let gone = t.src.lib.lock().tracks.remove(0);
        let mut b = Batch::default();
        b.add(LiveChange::Remove(ItemKindKey::Track, gone.id.clone()));
        let out = apply(&t.db, t.src.as_ref(), b).await.unwrap();
        assert_eq!(out, LiveOutcome::Applied { updated: 0, removed: 1, new_albums: vec![] });
        assert_eq!(title(&t.db, "tracks", &gone.id), None);
        assert!(t.src.item_calls.lock().is_empty(), "nothing to fetch for a removal");
    }

    #[tokio::test]
    async fn an_update_for_an_item_the_server_no_longer_has_removes_it() {
        let t = synced().await;
        let gone = t.src.lib.lock().tracks.remove(1);
        let mut b = Batch::default();
        b.add(up(ItemKindKey::Track, &gone.id));
        let out = apply(&t.db, t.src.as_ref(), b).await.unwrap();
        assert!(matches!(out, LiveOutcome::Applied { removed: 1, .. }));
        assert_eq!(title(&t.db, "tracks", &gone.id), None);
    }

    #[tokio::test]
    async fn big_batches_and_no_section_ask_for_a_sync() {
        let t = synced().await;
        let mut b = Batch::default();
        for i in 0..=MAX_BATCH {
            b.add(up(ItemKindKey::Track, &format!("t{i}")));
        }
        assert_eq!(apply(&t.db, t.src.as_ref(), b).await.unwrap(), LiveOutcome::NeedsSync);
        assert!(t.src.item_calls.lock().is_empty());

        let dir = tempfile::tempdir().unwrap();
        let fresh = DbHandle::open(dir.path().join("library.db")).unwrap();
        let src = FakeSource::new(FakeLibrary::default());
        let mut b = Batch::default();
        b.add(up(ItemKindKey::Track, "1"));
        assert_eq!(apply(&fresh, &src, b).await.unwrap(), LiveOutcome::NeedsSync);
        assert_eq!(apply(&fresh, &src, Batch::default()).await.unwrap(), LiveOutcome::Nothing);
    }

    #[tokio::test]
    async fn a_failed_fetch_changes_nothing() {
        let t = synced().await;
        *t.src.fail_items.lock() = true;
        let id = t.src.lib.lock().tracks[0].id.clone();
        let mut b = Batch::default();
        b.add(up(ItemKindKey::Track, &id));
        assert!(apply(&t.db, t.src.as_ref(), b).await.is_err());
        assert!(title(&t.db, "tracks", &id).is_some());
        assert_eq!(*t.ev.0.lock(), 0);
    }
}
