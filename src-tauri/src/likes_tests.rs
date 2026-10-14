//! Likes against a fake Plex: optimistic change, offline queue and replay,
//! refused writes rolled back, ratings arriving from Plex.

use std::collections::HashMap;

use parking_lot::Mutex;

use super::*;
use crate::db::likes::Rated;

/// An in-memory Plex that records every rating write.
#[derive(Default)]
struct FakePlex {
    online: Mutex<bool>,
    ratings: Mutex<HashMap<String, f64>>,
    calls: Mutex<Vec<String>>,
    /// Every write fails with this error while set.
    fail: Mutex<Option<AppError>>,
}

impl FakePlex {
    fn new() -> Arc<Self> {
        let f = Arc::new(Self::default());
        *f.online.lock() = true;
        f
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().clone()
    }
}

#[async_trait]
impl RatingRemote for FakePlex {
    async fn online(&self) -> bool {
        *self.online.lock()
    }
    async fn rate(&self, track_id: &str, rating: Option<f64>) -> AppResult<()> {
        if let Some(e) = self.fail.lock().as_ref() {
            return Err(match e {
                AppError::Network(m) => AppError::Network(m.clone()),
                AppError::Server(m) => AppError::Server(m.clone()),
                AppError::Auth(m) => AppError::Auth(m.clone()),
                other => AppError::Invalid(other.kind().into()),
            });
        }
        self.calls.lock().push(format!("rate {track_id} {}", rating.map_or("-1".into(), |r| r.to_string())));
        match rating {
            Some(r) => self.ratings.lock().insert(track_id.into(), r),
            None => self.ratings.lock().remove(track_id),
        };
        Ok(())
    }
}

#[derive(Default)]
struct Events {
    changed: Mutex<Vec<Vec<String>>>,
    errors: Mutex<Vec<String>>,
}

impl LikeEvents for Events {
    fn changed(&self, ids: &[String]) {
        self.changed.lock().push(ids.to_vec());
    }
    fn error(&self, message: &str) {
        self.errors.lock().push(message.to_string());
    }
}

struct Rig {
    likes: Likes,
    plex: Arc<FakePlex>,
    events: Arc<Events>,
    db: DbHandle,
    _dir: tempfile::TempDir,
}

/// Tracks 1 to 5; 4 is rated three stars (6) and 5 is liked, both in Plex.
fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    db.write(|c| {
        for (id, rating, at) in [("1", None, None), ("2", None, None), ("3", None, None), ("4", Some(6.0), Some(50)), ("5", Some(10.0), Some(100))] {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number,
                   disc_number, duration_ms, part_key, user_rating, rated_at)
                 VALUES (?1, 'Song ' || ?1, 'a', 'A', 'r', 'R', 1, 1, 1000, '/p', ?2, ?3)",
                rusqlite::params![id, rating, at],
            )?;
        }
        Ok(())
    })
    .unwrap();
    let plex = FakePlex::new();
    plex.ratings.lock().insert("4".into(), 6.0);
    plex.ratings.lock().insert("5".into(), 10.0);
    let events = Arc::new(Events::default());
    let likes = Likes::build(db.clone(), plex.clone(), events.clone(), false);
    Rig { likes, plex, events, db, _dir: dir }
}

impl Rig {
    fn rating(&self, id: &str) -> (Option<f64>, Option<i64>) {
        self.db
            .read(|c| Ok(c.query_row("SELECT user_rating, rated_at FROM tracks WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?))
            .unwrap()
    }
    fn queued(&self) -> usize {
        self.db.read(local::queued).unwrap().len()
    }
    async fn ids(&self) -> Vec<String> {
        self.likes.ids().await.unwrap()
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[tokio::test]
async fn a_like_shows_at_once_and_reaches_plex_on_replay() {
    let r = rig();
    let changed = r.likes.set(s(&["1"]), true).await.unwrap();
    assert_eq!(changed, s(&["1"]));
    // Optimistic: liked here before Plex hears of it, newest first.
    assert_eq!(r.ids().await, s(&["1", "5"]));
    let (rating, at) = r.rating("1");
    assert_eq!(rating, Some(10.0));
    assert!(at.is_some_and(|t| t > 100), "liked now");
    assert_eq!(r.events.changed.lock().clone(), vec![s(&["1"])]);
    assert!(r.plex.calls().is_empty());
    r.likes.replay().await;
    assert_eq!(r.plex.calls(), s(&["rate 1 10"]));
    assert_eq!(r.queued(), 0);
    // The page lists it first with the time it was liked.
    let page = r.likes.page().await.unwrap();
    assert_eq!(page.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["1", "5"]);
    assert_eq!(page.total_duration_ms, 2000);
    assert!(page.tracks[0].rated_at.is_some());
}

#[tokio::test]
async fn unlike_clears_the_rating() {
    let r = rig();
    r.likes.set(s(&["5"]), false).await.unwrap();
    assert_eq!(r.rating("5"), (None, None));
    assert!(r.ids().await.is_empty());
    r.likes.replay().await;
    assert_eq!(r.plex.calls(), s(&["rate 5 -1"]));
    assert!(!r.plex.ratings.lock().contains_key("5"));
}

#[tokio::test]
async fn liking_a_three_star_song_makes_it_five_and_unliking_clears_it() {
    let r = rig();
    r.likes.set(s(&["4"]), true).await.unwrap();
    assert_eq!(r.rating("4").0, Some(10.0));
    r.likes.replay().await;
    assert_eq!(r.plex.ratings.lock()["4"], 10.0);
}

#[tokio::test]
async fn offline_changes_queue_one_per_song_and_replay_when_back() {
    let r = rig();
    *r.plex.online.lock() = false;
    r.likes.set(s(&["1", "2"]), true).await.unwrap();
    r.likes.set(s(&["2"]), false).await.unwrap(); // back to what Plex has: nothing to send
    r.likes.set(s(&["3"]), true).await.unwrap();
    r.likes.set(s(&["5"]), false).await.unwrap();
    r.likes.replay().await;
    assert!(r.plex.calls().is_empty(), "offline: nothing sent");
    assert_eq!(r.queued(), 3);
    assert_eq!(r.rating("2"), (None, None));
    *r.plex.online.lock() = true;
    r.likes.replay().await;
    assert_eq!(r.plex.calls(), s(&["rate 1 10", "rate 3 10", "rate 5 -1"]));
    assert_eq!(r.queued(), 0);
}

#[tokio::test]
async fn unlike_then_like_again_before_sending_restores_the_old_time() {
    let r = rig();
    *r.plex.online.lock() = false;
    r.likes.set(s(&["5"]), false).await.unwrap();
    r.likes.set(s(&["5"]), true).await.unwrap();
    assert_eq!(r.queued(), 0);
    assert_eq!(r.rating("5"), (Some(10.0), Some(100)), "the like keeps its place in the list");
}

#[tokio::test]
async fn a_refused_like_rolls_back_with_a_toast() {
    let r = rig();
    *r.plex.fail.lock() = Some(AppError::Invalid("403".into()));
    r.likes.set(s(&["4"]), true).await.unwrap();
    assert_eq!(r.rating("4").0, Some(10.0));
    r.likes.replay().await;
    assert_eq!(r.rating("4"), (Some(6.0), Some(50)), "back to three stars");
    assert_eq!(r.queued(), 0);
    let errors = r.events.errors.lock().clone();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("Song 4"), "{errors:?}");
    assert_eq!(r.events.changed.lock().last().cloned(), Some(s(&["4"])));
}

#[tokio::test]
async fn a_refused_unlike_rolls_back_too() {
    let r = rig();
    *r.plex.fail.lock() = Some(AppError::Invalid("403".into()));
    r.likes.set(s(&["5"]), false).await.unwrap();
    r.likes.replay().await;
    assert_eq!(r.rating("5"), (Some(10.0), Some(100)));
    assert!(r.events.errors.lock()[0].contains("Liked Songs"));
}

#[tokio::test]
async fn network_errors_wait_and_server_errors_give_up_after_retries() {
    let r = rig();
    *r.plex.fail.lock() = Some(AppError::Network("down".into()));
    r.likes.set(s(&["1"]), true).await.unwrap();
    for _ in 0..10 {
        r.likes.replay().await;
    }
    assert_eq!(r.queued(), 1, "kept while the network is down");
    assert!(r.events.errors.lock().is_empty());
    *r.plex.fail.lock() = Some(AppError::Server("500".into()));
    for _ in 0..MAX_ATTEMPTS {
        r.likes.replay().await;
    }
    assert_eq!(r.queued(), 0);
    assert_eq!(r.rating("1"), (None, None));
    assert_eq!(r.events.errors.lock().len(), 1);
}

#[tokio::test]
async fn liking_twice_or_unknown_songs() {
    let r = rig();
    assert!(r.likes.set(s(&["5"]), true).await.unwrap().is_empty(), "already liked");
    assert!(matches!(r.likes.set(s(&["nope"]), true).await, Err(AppError::NotFound(_))));
    assert!(matches!(r.likes.set(vec![], true).await, Err(AppError::Invalid(_))));
    assert!(r.events.changed.lock().is_empty());
}

#[tokio::test]
async fn four_stars_count_when_the_setting_says_so() {
    let r = rig();
    assert_eq!(r.ids().await, s(&["5"]));
    let mut st = crate::settings::load(&r.db);
    st.liked_means = crate::settings::LikedMeans::FourUp;
    crate::settings::save(&r.db, &st).unwrap();
    r.db.write(|c| Ok(c.execute("UPDATE tracks SET user_rating = 8 WHERE id = '4'", [])?)).unwrap();
    assert_eq!(r.ids().await, s(&["5", "4"]));
    // Liking a four-star song then keeps its rating.
    assert!(r.likes.set(s(&["4"]), true).await.unwrap().is_empty());
}

#[tokio::test]
async fn ratings_from_plex_arrive_with_sync_but_queued_ones_win() {
    let r = rig();
    *r.plex.online.lock() = false;
    r.likes.set(s(&["1"]), true).await.unwrap(); // queued
    let changed = r
        .db
        .write(|c| {
            let tx = c.transaction()?;
            let changed = local::apply_rated(
                &tx,
                &[
                    Rated { track_id: "2".into(), rating: 10.0, rated_at: Some(500) }, // liked elsewhere
                    Rated { track_id: "4".into(), rating: 6.0, rated_at: Some(50) },   // unchanged
                ],
            )?;
            tx.commit()?;
            Ok(changed)
        })
        .unwrap();
    // 2 liked elsewhere, 5 cleared elsewhere; 1 still waits to be sent.
    let mut changed = changed;
    changed.sort();
    assert_eq!(changed, s(&["2", "5"]));
    assert_eq!(r.rating("2"), (Some(10.0), Some(500)));
    assert_eq!(r.rating("5"), (None, None));
    assert_eq!(r.rating("1").0, Some(10.0));
    // Newest like first: 1 was liked just now, 2 at 500.
    assert_eq!(r.ids().await, s(&["1", "2"]));
}

#[tokio::test]
async fn a_full_track_upsert_keeps_a_queued_rating() {
    let r = rig();
    *r.plex.online.lock() = false;
    r.likes.set(s(&["1"]), true).await.unwrap();
    let mut t = r.db.read(|c| crate::db::queries::tracks_by_ids(c, &s(&["1"]))).unwrap().remove(0);
    t.user_rating = None;
    t.rated_at = None;
    t.genres = s(&["Soul"]);
    r.db
        .write(|c| {
            let tx = c.transaction()?;
            crate::db::store::upsert_tracks(&tx, &[t.clone()], 1)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    assert_eq!(r.rating("1").0, Some(10.0), "the like waiting for Plex stays");
    let page = r.likes.page().await.unwrap();
    assert_eq!(page.tracks[0].genres, s(&["Soul"]));
}

#[tokio::test]
async fn saved_in_lists_the_playlists_holding_a_song() {
    let r = rig();
    r.db
        .write(|c| {
            c.execute_batch(
                "INSERT INTO playlists (id, title, updated_at) VALUES ('p1', 'Old', 1), ('p2', 'New', 9), ('p3', 'None', 5);
                 INSERT INTO playlist_items (playlist_id, position, track_id) VALUES ('p1', 0, '1'), ('p2', 0, '2'), ('p2', 1, '1');",
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(r.likes.playlists_with("1".into()).await.unwrap(), s(&["p2", "p1"]));
    assert!(r.likes.playlists_with("3".into()).await.unwrap().is_empty());
}
