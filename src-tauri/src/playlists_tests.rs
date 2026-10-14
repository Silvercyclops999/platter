//! Playlist editing against a fake Plex: offline queue and replay, refused
//! edits, new playlists waiting for their first song, row id mapping.

use std::collections::HashMap;

use parking_lot::Mutex;

use super::*;
use crate::db::queries;
use crate::db::store;

/// (track, playlistItemID) rows.
type Rows = Vec<(String, String)>;

/// An in-memory Plex playlist store that records every write.
#[derive(Default)]
struct FakePlex {
    online: Mutex<bool>,
    lists: Mutex<HashMap<String, (Playlist, Rows)>>,
    next: Mutex<u64>,
    calls: Mutex<Vec<String>>,
    /// The next write fails with this error.
    fail_next: Mutex<Option<AppError>>,
    /// Every write fails with this error while set.
    fail_all: Mutex<Option<AppError>>,
}

impl FakePlex {
    fn new() -> Arc<Self> {
        let f = Arc::new(Self::default());
        *f.online.lock() = true;
        *f.next.lock() = 1000;
        f
    }
    fn id(&self) -> String {
        let mut n = self.next.lock();
        *n += 1;
        n.to_string()
    }
    fn seed(&self, id: &str, title: &str, tracks: &[&str]) -> (Playlist, Vec<PlaylistEntry>) {
        let rows: Vec<(String, String)> = tracks.iter().map(|t| (t.to_string(), self.id())).collect();
        let p = playlist(id, title, rows.len());
        self.lists.lock().insert(id.into(), (p.clone(), rows.clone()));
        (p, entries(&rows))
    }
    fn write(&self, call: String) -> AppResult<()> {
        if let Some(e) = self.fail_all.lock().as_ref() {
            return Err(match e {
                AppError::Network(m) => AppError::Network(m.clone()),
                AppError::Server(m) => AppError::Server(m.clone()),
                other => AppError::Invalid(other.kind().into()),
            });
        }
        if let Some(e) = self.fail_next.lock().take() {
            return Err(e);
        }
        self.calls.lock().push(call);
        Ok(())
    }
    fn tracks(&self, id: &str) -> Vec<String> {
        self.lists.lock()[id].1.iter().map(|(t, _)| t.clone()).collect()
    }
    fn title(&self, id: &str) -> String {
        self.lists.lock()[id].0.title.clone()
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().clone()
    }
}

fn playlist(id: &str, title: &str, n: usize) -> Playlist {
    Playlist {
        id: id.into(),
        title: title.into(),
        smart: false,
        track_count: n as u32,
        duration_ms: 0,
        thumb: None,
        summary: None,
        updated_at: Some(1),
    }
}

fn entries(rows: &[(String, String)]) -> Vec<PlaylistEntry> {
    rows.iter().map(|(t, i)| PlaylistEntry { track_id: t.clone(), item_id: Some(i.clone()) }).collect()
}

fn not_found() -> AppError {
    AppError::NotFound("playlist".into())
}

#[async_trait]
impl PlaylistRemote for FakePlex {
    async fn online(&self) -> bool {
        *self.online.lock()
    }
    async fn create(&self, title: &str, track_ids: &[String]) -> AppResult<String> {
        self.write(format!("create {title} {}", track_ids.join(",")))?;
        let id = self.id();
        let rows: Vec<(String, String)> = track_ids.iter().map(|t| (t.clone(), self.id())).collect();
        self.lists.lock().insert(id.clone(), (playlist(&id, title, rows.len()), rows));
        Ok(id)
    }
    async fn add(&self, id: &str, track_ids: &[String]) -> AppResult<()> {
        self.write(format!("add {id} {}", track_ids.join(",")))?;
        let rows: Vec<(String, String)> = track_ids.iter().map(|t| (t.clone(), self.id())).collect();
        self.lists.lock().get_mut(id).ok_or_else(not_found)?.1.extend(rows);
        Ok(())
    }
    async fn remove(&self, id: &str, item_id: &str) -> AppResult<()> {
        self.write(format!("remove {id} {item_id}"))?;
        let mut l = self.lists.lock();
        let rows = &mut l.get_mut(id).ok_or_else(not_found)?.1;
        let at = rows.iter().position(|(_, i)| i == item_id).ok_or_else(not_found)?;
        rows.remove(at);
        Ok(())
    }
    async fn move_item(&self, id: &str, item_id: &str, after: Option<&str>) -> AppResult<()> {
        self.write(format!("move {id} {item_id} after {}", after.unwrap_or("-")))?;
        let mut l = self.lists.lock();
        let rows = &mut l.get_mut(id).ok_or_else(not_found)?.1;
        let at = rows.iter().position(|(_, i)| i == item_id).ok_or_else(not_found)?;
        let row = rows.remove(at);
        let to = match after {
            None => 0,
            Some(a) => rows.iter().position(|(_, i)| i == a).ok_or_else(not_found)? + 1,
        };
        rows.insert(to, row);
        Ok(())
    }
    async fn update(&self, id: &str, title: Option<&str>, summary: Option<&str>) -> AppResult<()> {
        self.write(format!("update {id} {} {}", title.unwrap_or("-"), summary.unwrap_or("-")))?;
        let mut l = self.lists.lock();
        let p = &mut l.get_mut(id).ok_or_else(not_found)?.0;
        if let Some(t) = title {
            p.title = t.into();
        }
        if let Some(s) = summary {
            p.summary = Some(s.into());
        }
        Ok(())
    }
    async fn poster(&self, id: &str, bytes: Vec<u8>, content_type: &str) -> AppResult<()> {
        self.write(format!("poster {id} {} {content_type}", bytes.len()))?;
        self.lists.lock().get_mut(id).ok_or_else(not_found)?.0.thumb = Some(format!("/library/metadata/{id}/thumb/1"));
        Ok(())
    }
    async fn delete(&self, id: &str) -> AppResult<()> {
        self.write(format!("delete {id}"))?;
        self.lists.lock().remove(id).map(|_| ()).ok_or_else(not_found)
    }
    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)> {
        let l = self.lists.lock();
        let (p, rows) = l.get(id).ok_or_else(not_found)?;
        let mut p = p.clone();
        p.track_count = rows.len() as u32;
        Ok((p, entries(rows)))
    }
}

#[derive(Default)]
struct Events {
    changed: Mutex<u32>,
    errors: Mutex<Vec<String>>,
}

impl PlaylistEvents for Events {
    fn changed(&self) {
        *self.changed.lock() += 1;
    }
    fn error(&self, message: &str) {
        self.errors.lock().push(message.to_string());
    }
}

struct Rig {
    pl: Playlists,
    plex: Arc<FakePlex>,
    events: Arc<Events>,
    db: DbHandle,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    db.write(|c| {
        for id in ["1", "2", "3", "4", "5"] {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number,
                   disc_number, duration_ms, part_key) VALUES (?1, ?1, 'a', 'A', 'r', 'R', 1, 1, 1000, '/p')",
                [id],
            )?;
        }
        Ok(())
    })
    .unwrap();
    let plex = FakePlex::new();
    let events = Arc::new(Events::default());
    let pl = Playlists::build(db.clone(), plex.clone(), events.clone(), false);
    Rig { pl, plex, events, db, _dir: dir }
}

impl Rig {
    /// A playlist that exists in Plex and in the cache.
    fn existing(&self, id: &str, title: &str, tracks: &[&str]) {
        let (p, rows) = self.plex.seed(id, title, tracks);
        self.db.write(|c| local::apply_remote(c, &p, &rows)).unwrap();
    }
    fn local_tracks(&self, id: &str) -> Vec<String> {
        self.db.read(|c| queries::playlist_page(c, id)).unwrap().tracks.into_iter().map(|t| t.id).collect()
    }
    fn page(&self, id: &str) -> crate::db::PlaylistPage {
        self.db.read(|c| queries::playlist_page(c, id)).unwrap()
    }
    fn queued(&self) -> usize {
        self.db.read(local::queued).unwrap().len()
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[tokio::test]
async fn edits_made_offline_queue_and_replay_in_order() {
    let r = rig();
    r.existing("50", "Road", &["1", "2", "3"]);
    *r.plex.online.lock() = false;

    r.pl.add("50".into(), s(&["4"]), Place::End).await.unwrap();
    let ids = r.page("50").item_ids;
    r.pl.remove("50".into(), vec![ids[0].clone()]).await.unwrap();
    // The new song (still only known here) to the top.
    r.pl.move_items("50".into(), vec![ids[3].clone()], None).await.unwrap();
    r.pl.update("50".into(), Some("Road Trip".into()), None).await.unwrap();
    assert_eq!(r.local_tracks("50"), ["4", "2", "3"], "the page changes at once");
    assert!(*r.events.changed.lock() >= 4);

    r.pl.replay().await;
    assert!(r.plex.calls().is_empty(), "nothing is sent while offline");
    assert_eq!(r.queued(), 4);

    *r.plex.online.lock() = true;
    r.pl.replay().await;
    assert_eq!(r.queued(), 0);
    assert_eq!(r.plex.tracks("50"), ["4", "2", "3"]);
    assert_eq!(r.plex.title("50"), "Road Trip");
    let calls = r.plex.calls();
    assert!(calls[0].starts_with("add 50 4") && calls[1].starts_with("remove 50") && calls[2].starts_with("move 50"));
    assert!(calls[3].starts_with("update 50 Road Trip"));
    // Read back from Plex: every row has Plex's id now.
    assert!(r.page("50").item_ids.iter().all(|i| !local::is_local(i)));
    assert!(r.events.errors.lock().is_empty());
}

#[tokio::test]
async fn a_new_playlist_waits_for_its_first_song_then_is_created() {
    let r = rig();
    let p = r.pl.create(None, vec![]).await.unwrap();
    assert_eq!(p.title, "My playlist #1");
    assert!(local::is_local(&p.id));
    r.pl.replay().await;
    assert!(r.plex.calls().is_empty(), "Plex makes playlists from songs, so an empty one waits");

    r.pl.update(p.id.clone(), Some("Night".into()), Some("For late".into())).await.unwrap();
    r.pl.add(p.id.clone(), s(&["2", "1"]), Place::End).await.unwrap();
    assert_eq!(r.queued(), 1, "a playlist only made here keeps one create, not every edit");
    r.pl.replay().await;

    let calls = r.plex.calls();
    assert_eq!(calls[0], "create Night 2,1");
    assert!(calls[1].starts_with("update ") && calls[1].ends_with("- For late"));
    assert_eq!(r.queued(), 0);
    // The id made here still opens it, now under the Plex id.
    let page = r.page(&p.id);
    assert!(!local::is_local(&page.playlist.id));
    assert_eq!(page.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["2", "1"]);
    assert_eq!(page.playlist.summary.as_deref(), Some("For late"));
    // And later edits go to Plex one by one.
    r.pl.add(p.id.clone(), s(&["3"]), Place::End).await.unwrap();
    r.pl.replay().await;
    assert_eq!(r.plex.tracks(&page.playlist.id), ["2", "1", "3"]);
}

#[tokio::test]
async fn names_count_up_from_the_settings_template() {
    let r = rig();
    assert_eq!(r.pl.create(None, vec![]).await.unwrap().title, "My playlist #1");
    assert_eq!(r.pl.create(None, vec![]).await.unwrap().title, "My playlist #2");
    crate::settings::save(
        &r.db,
        &crate::settings::Settings { playlist_new_name: "Mix #N".into(), ..Default::default() },
    )
    .unwrap();
    assert_eq!(r.pl.create(None, vec![]).await.unwrap().title, "Mix #1");
    assert_eq!(r.pl.create(Some("  Named ".into()), vec![]).await.unwrap().title, "Named");
    assert!(r.pl.create(Some(" ".into()), vec![]).await.is_err());
}

#[tokio::test]
async fn deleting_a_playlist_only_made_here_never_reaches_plex() {
    let r = rig();
    let p = r.pl.create(None, vec![]).await.unwrap();
    r.pl.delete(p.id.clone()).await.unwrap();
    r.pl.replay().await;
    assert!(r.plex.calls().is_empty());
    assert_eq!(r.queued(), 0);
    assert!(r.db.read(|c| local::playlist(c, &p.id)).unwrap().is_none());
}

#[tokio::test]
async fn delete_reaches_plex() {
    let r = rig();
    r.existing("50", "Road", &["1"]);
    r.pl.delete("50".into()).await.unwrap();
    assert!(r.db.read(|c| local::playlist(c, "50")).unwrap().is_none());
    r.pl.replay().await;
    assert_eq!(r.plex.calls(), ["delete 50"]);
    assert!(!r.plex.lists.lock().contains_key("50"));
}

#[tokio::test]
async fn a_refused_edit_drops_the_rest_shows_a_toast_and_reads_plex_back() {
    let r = rig();
    r.existing("50", "Road", &["1", "2"]);
    *r.plex.online.lock() = false;
    r.pl.add("50".into(), s(&["3"]), Place::End).await.unwrap();
    r.pl.update("50".into(), Some("New".into()), None).await.unwrap();
    *r.plex.fail_next.lock() = Some(AppError::Invalid("Unexpected response 400.".into()));
    *r.plex.online.lock() = true;
    r.pl.replay().await;

    assert_eq!(r.queued(), 0, "the playlist's other queued edits are dropped");
    assert!(r.plex.calls().is_empty());
    let errors = r.events.errors.lock().clone();
    assert_eq!(errors, ["Plex didn't accept a change to \"New\". Showing it as it is in Plex."]);
    assert_eq!(r.local_tracks("50"), ["1", "2"], "back to what Plex has");
    assert_eq!(r.page("50").playlist.title, "Road");
}

#[tokio::test]
async fn server_errors_retry_and_then_count_as_refused() {
    let r = rig();
    r.existing("50", "Road", &["1"]);
    r.pl.add("50".into(), s(&["2"]), Place::End).await.unwrap();
    *r.plex.fail_all.lock() = Some(AppError::Server("Server error 500.".into()));
    for _ in 0..MAX_ATTEMPTS - 1 {
        r.pl.replay().await;
        assert_eq!(r.queued(), 1, "kept for the next try");
        assert!(r.events.errors.lock().is_empty());
    }
    r.pl.replay().await;
    assert_eq!(r.queued(), 0);
    assert_eq!(r.events.errors.lock().len(), 1);
}

#[tokio::test]
async fn network_errors_keep_the_edit_without_counting() {
    let r = rig();
    r.existing("50", "Road", &["1"]);
    r.pl.add("50".into(), s(&["2"]), Place::End).await.unwrap();
    *r.plex.fail_all.lock() = Some(AppError::Network("Couldn't connect.".into()));
    for _ in 0..10 {
        r.pl.replay().await;
    }
    assert_eq!(r.queued(), 1);
    assert!(r.events.errors.lock().is_empty());
    *r.plex.fail_all.lock() = None;
    r.pl.replay().await;
    assert_eq!(r.plex.tracks("50"), ["1", "2"]);
}

#[tokio::test]
async fn a_playlist_deleted_in_plex_goes_away_here() {
    let r = rig();
    r.existing("50", "Road", &["1", "2"]);
    r.plex.lists.lock().remove("50");
    let ids = r.page("50").item_ids;
    r.pl.remove("50".into(), vec![ids[0].clone()]).await.unwrap();
    r.pl.replay().await;
    assert!(r.db.read(|c| local::playlist(c, "50")).unwrap().is_none());
    assert_eq!(r.events.errors.lock().len(), 1);
    assert!(r.db.read(|c| queries::playlist_page(c, "50")).is_err());
}

#[tokio::test]
async fn rows_added_here_get_their_plex_ids_for_later_edits() {
    let r = rig();
    r.existing("50", "Road", &["1"]);
    *r.plex.online.lock() = false;
    r.pl.add("50".into(), s(&["2", "3"]), Place::End).await.unwrap();
    let ids = r.page("50").item_ids;
    r.pl.remove("50".into(), vec![ids[1].clone()]).await.unwrap();
    *r.plex.online.lock() = true;
    r.pl.replay().await;
    assert_eq!(r.plex.tracks("50"), ["1", "3"]);
    assert!(r.events.errors.lock().is_empty());
}

#[tokio::test]
async fn undo_puts_songs_back_where_they_were() {
    let r = rig();
    r.existing("50", "Road", &["1", "2", "3", "4"]);
    let ids = r.page("50").item_ids;
    let gone = r.pl.remove("50".into(), vec![ids[1].clone(), ids[2].clone()]).await.unwrap();
    assert_eq!(gone.iter().map(|g| g.index).collect::<Vec<_>>(), [1, 2]);
    // Undo: the run goes back after the row before it.
    r.pl.add("50".into(), s(&["2", "3"]), Place::After(ids[0].clone())).await.unwrap();
    assert_eq!(r.local_tracks("50"), ["1", "2", "3", "4"]);
    r.pl.replay().await;
    assert_eq!(r.plex.tracks("50"), ["1", "2", "3", "4"]);
    // And at the top.
    let ids = r.page("50").item_ids;
    r.pl.remove("50".into(), vec![ids[0].clone()]).await.unwrap();
    r.pl.add("50".into(), s(&["1"]), Place::Top).await.unwrap();
    r.pl.replay().await;
    assert_eq!(r.plex.tracks("50"), ["1", "2", "3", "4"]);
}

#[tokio::test]
async fn reorder_matches_plex() {
    let r = rig();
    r.existing("50", "Road", &["1", "2", "3", "4", "5"]);
    let ids = r.page("50").item_ids;
    r.pl.move_items("50".into(), vec![ids[0].clone(), ids[2].clone()], Some(ids[4].clone())).await.unwrap();
    assert_eq!(r.local_tracks("50"), ["2", "4", "5", "1", "3"]);
    r.pl.replay().await;
    assert_eq!(r.plex.tracks("50"), ["2", "4", "5", "1", "3"]);
}

#[tokio::test]
async fn cover_and_description() {
    let r = rig();
    r.existing("50", "Road", &["1"]);
    assert!(r.pl.set_cover("50".into(), vec![1, 2, 3], "image/gif".into()).await.is_err());
    r.pl.set_cover("50".into(), vec![1, 2, 3], "image/jpeg".into()).await.unwrap();
    r.pl.update("50".into(), None, Some("Songs for driving".into())).await.unwrap();
    assert!(r.pl.update("50".into(), None, Some("x".repeat(301))).await.is_err());
    assert!(r.pl.update("50".into(), Some("y".repeat(101)), None).await.is_err());
    r.pl.replay().await;
    let page = r.page("50");
    assert_eq!(page.playlist.thumb.as_deref(), Some("/library/metadata/50/thumb/1"));
    assert_eq!(page.playlist.summary.as_deref(), Some("Songs for driving"));
}

#[tokio::test]
async fn smart_playlists_refuse_edits() {
    let r = rig();
    let (mut p, rows) = r.plex.seed("60", "Most Played", &["1"]);
    p.smart = true;
    r.db.write(|c| local::apply_remote(c, &p, &rows)).unwrap();
    assert!(matches!(r.pl.add("60".into(), s(&["2"]), Place::End).await, Err(AppError::Invalid(_))));
    assert!(matches!(r.pl.delete("60".into()).await, Err(AppError::Invalid(_))));
    assert_eq!(r.queued(), 0);
}

#[tokio::test]
async fn sync_leaves_playlists_with_local_changes_alone() {
    let r = rig();
    r.existing("50", "Road", &["1", "2"]);
    let local_only = r.pl.create(Some("Mine".into()), vec![]).await.unwrap();
    *r.plex.online.lock() = false;
    r.pl.add("50".into(), s(&["3"]), Place::End).await.unwrap();
    // Plex's list doesn't know the new song or the new playlist yet.
    let stale = r
        .db
        .write(|c| {
            let tx = c.transaction()?;
            let stale = store::replace_playlists(&tx, &[playlist("50", "Road", 2)], true)?;
            tx.commit()?;
            Ok(stale)
        })
        .unwrap();
    assert!(stale.is_empty(), "items of a playlist with queued edits aren't refetched");
    assert_eq!(r.local_tracks("50"), ["1", "2", "3"]);
    assert!(r.db.read(|c| local::playlist(c, &local_only.id)).unwrap().is_some());
}
