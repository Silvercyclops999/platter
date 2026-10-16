//! Likes live on each song's server, so every app on that server sees the
//! same likes. On Plex a liked song is one with a five-star rating.
//!
//! A like or unlike is applied to the local cache at once (db::likes) and
//! the write is queued; the queue replays whenever the server is
//! reachable, like playlist edits. If the server refuses a write, the like
//! goes back to what the server has, a toast says so, and the UI hears
//! about it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::OptionalExtension;
use serde::Serialize;
use tauri::State;

use crate::db::DbHandle;
use crate::db::likes::{self as local, LIKED_RATING, RatingOp};
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::plex::PlexState;

/// Server errors (5xx) are retried this many times before the write counts
/// as refused.
const MAX_ATTEMPTS: u32 = 5;
const REPLAY_EVERY: Duration = Duration::from_secs(15);
/// Event: likes changed here, on rollback, or from Plex. Payload `{ ids }`.
pub const EVENT_LIKES_CHANGED: &str = "likes://changed";

// ---------------------------------------------------------------- outside world

/// Where like writes go. The app sends each to the song's server
/// (`crate::backend`); tests use a fake.
#[async_trait]
pub trait LikeRemote: Send + Sync {
    /// True when a server is connected right now.
    async fn online(&self) -> bool;
    /// Like or unlike a track.
    async fn set_liked(&self, track_id: &str, liked: bool) -> AppResult<()>;
    /// Set (Some, 0 to 10) or clear (None) a star rating. A server without
    /// star ratings takes any rating as a like.
    async fn set_rating(&self, track_id: &str, rating: Option<f64>) -> AppResult<()> {
        self.set_liked(track_id, rating.is_some()).await
    }
}

/// A server with star ratings (Plex).
#[async_trait]
pub trait RatingRemote: Send + Sync {
    /// True when a server is connected right now.
    async fn online(&self) -> bool;
    /// Set (Some) or clear (None) a track's rating.
    async fn rate(&self, track_id: &str, rating: Option<f64>) -> AppResult<()>;
}

/// On a server with star ratings, liked is five stars and unliked clears
/// the rating.
#[async_trait]
impl<T: RatingRemote + ?Sized> LikeRemote for T {
    async fn online(&self) -> bool {
        RatingRemote::online(self).await
    }
    async fn set_liked(&self, track_id: &str, liked: bool) -> AppResult<()> {
        self.rate(track_id, liked.then_some(LIKED_RATING)).await
    }
    async fn set_rating(&self, track_id: &str, rating: Option<f64>) -> AppResult<()> {
        self.rate(track_id, rating).await
    }
}

/// What the UI hears about.
pub trait LikeEvents: Send + Sync {
    /// These tracks' liked state changed (emits `likes://changed`).
    fn changed(&self, ids: &[String]);
    /// An error toast.
    fn error(&self, message: &str);
}

pub struct PlexRatings(pub PlexState);

#[async_trait]
impl RatingRemote for PlexRatings {
    async fn online(&self) -> bool {
        self.0.client().await.is_ok()
    }
    async fn rate(&self, track_id: &str, rating: Option<f64>) -> AppResult<()> {
        self.0.client_for(track_id).await?.rate(track_id, rating).await
    }
}

/// Events through the app: `likes://changed` and a `toast`.
pub struct AppLikeEvents {
    pub app: tauri::AppHandle,
    pub plex: PlexState,
}

#[derive(Serialize, Clone)]
struct ChangedPayload<'a> {
    ids: &'a [String],
}

impl LikeEvents for AppLikeEvents {
    fn changed(&self, ids: &[String]) {
        use tauri::Emitter;
        if let Err(e) = self.app.emit(EVENT_LIKES_CHANGED, ChangedPayload { ids }) {
            tracing::warn!("emit {EVENT_LIKES_CHANGED}: {e}");
        }
    }
    fn error(&self, message: &str) {
        self.plex.toast("error", message);
    }
}

// ---------------------------------------------------------------- service

/// The Liked Songs page: liked tracks, newest like first.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LikedPage {
    pub tracks: Vec<Track>,
    pub total_duration_ms: u64,
}

/// Managed state behind the like commands. Cheap to clone.
#[derive(Clone)]
pub struct Likes {
    inner: Arc<Inner>,
}

struct Inner {
    db: DbHandle,
    remote: Arc<dyn LikeRemote>,
    events: Arc<dyn LikeEvents>,
    /// Start a replay after each change (off in tests, which replay by hand).
    auto_replay: bool,
    replaying: tokio::sync::Mutex<()>,
    /// Wakes the retry loop when a write is queued.
    queued: tokio::sync::Notify,
}

fn transient(e: &AppError) -> bool {
    matches!(e, AppError::Network(_) | AppError::Auth(_) | AppError::Server(_))
}

impl Likes {
    pub fn new(db: DbHandle, remote: Arc<dyn LikeRemote>, events: Arc<dyn LikeEvents>) -> Self {
        Self::build(db, remote, events, true)
    }

    pub fn build(db: DbHandle, remote: Arc<dyn LikeRemote>, events: Arc<dyn LikeEvents>, auto_replay: bool) -> Self {
        Self {
            inner: Arc::new(Inner { db, remote, events, auto_replay, replaying: Default::default(), queued: Default::default() }),
        }
    }

    fn db(&self) -> &DbHandle {
        &self.inner.db
    }

    fn min_rating(&self) -> f64 {
        crate::settings::load(self.db()).liked_min_rating()
    }

    // ------------------------------------------------------------ commands

    /// Like (five stars) or unlike (clear the rating) songs. Returns the ids
    /// whose state changed.
    pub async fn set(&self, track_ids: Vec<String>, liked: bool) -> AppResult<Vec<String>> {
        if track_ids.is_empty() {
            return Err(AppError::Invalid("No songs to change.".into()));
        }
        let min = self.min_rating();
        let changed = self
            .db()
            .write_async(move |c| {
                let tx = c.transaction()?;
                let mut changed = Vec::new();
                for id in &track_ids {
                    let rating: Option<f64> = tx
                        .query_row("SELECT user_rating FROM tracks WHERE id = ?1", [id], |r| r.get(0))
                        .optional()?
                        .ok_or_else(|| AppError::NotFound(format!("track {id}")))?;
                    let is_liked = rating.is_some_and(|r| r >= min);
                    // Liking a song already liked keeps its rating (four
                    // stars stays four when four counts as liked).
                    if is_liked == liked {
                        continue;
                    }
                    let target = liked.then_some(LIKED_RATING);
                    if local::set_rating(&tx, id, target)? && !changed.contains(id) {
                        changed.push(id.clone());
                    }
                }
                tx.commit()?;
                Ok(changed)
            })
            .await?;
        if !changed.is_empty() {
            self.inner.events.changed(&changed);
            self.inner.queued.notify_one();
            if self.inner.auto_replay
                && let Ok(rt) = tokio::runtime::Handle::try_current()
            {
                let me = self.clone();
                rt.spawn(async move { me.replay().await });
            }
        }
        Ok(changed)
    }

    /// Give a song 1 to 5 stars, or clear its rating (None). Only for
    /// servers with star ratings (the caller checks). Liked follows the
    /// rating (four or five stars may count as liked).
    pub async fn rate(&self, track_id: &str, stars: Option<u8>) -> AppResult<()> {
        if stars.is_some_and(|s| !(1..=5).contains(&s)) {
            return Err(AppError::Invalid("A rating is 1 to 5 stars.".into()));
        }
        let id = track_id.to_owned();
        let changed = self
            .db()
            .write_async(move |c| {
                let tx = c.transaction()?;
                let exists: Option<String> = tx.query_row("SELECT id FROM tracks WHERE id = ?1", [&id], |r| r.get(0)).optional()?;
                if exists.is_none() {
                    return Err(AppError::NotFound(format!("track {id}")));
                }
                let changed = local::set_rating(&tx, &id, stars.map(|s| f64::from(s) * 2.0))?;
                tx.commit()?;
                Ok(changed.then_some(id))
            })
            .await?;
        if let Some(id) = changed {
            self.inner.events.changed(&[id]);
            self.inner.queued.notify_one();
            if self.inner.auto_replay
                && let Ok(rt) = tokio::runtime::Handle::try_current()
            {
                let me = self.clone();
                rt.spawn(async move { me.replay().await });
            }
        }
        Ok(())
    }

    /// A song's star rating, 0 (unrated) to 5.
    pub async fn stars(&self, track_id: &str) -> AppResult<u8> {
        let id = track_id.to_owned();
        self.db()
            .read_async(move |c| {
                let rating: Option<f64> =
                    c.query_row("SELECT user_rating FROM tracks WHERE id = ?1", [&id], |r| r.get(0)).optional()?.flatten();
                Ok(rating.map_or(0, |r| (r / 2.0).round().clamp(0.0, 5.0) as u8))
            })
            .await
    }

    /// Whether one song is liked (false when it isn't in the cache).
    pub async fn is_liked(&self, track_id: &str) -> AppResult<bool> {
        let min = self.min_rating();
        let id = track_id.to_owned();
        self.db()
            .read_async(move |c| {
                let rating: Option<f64> =
                    c.query_row("SELECT user_rating FROM tracks WHERE id = ?1", [&id], |r| r.get(0)).optional()?.flatten();
                Ok(rating.is_some_and(|r| r >= min))
            })
            .await
    }

    /// Every liked track id, newest like first.
    pub async fn ids(&self) -> AppResult<Vec<String>> {
        let min = self.min_rating();
        self.db().read_async(move |c| local::liked_ids(c, min)).await
    }

    pub async fn page(&self) -> AppResult<LikedPage> {
        let min = self.min_rating();
        let server = self.db().server_view();
        let tracks = self
            .db()
            .read_async(move |c| {
                let mut t = local::liked_tracks(c, min)?;
                // A song liked on two servers shows once (db::merge).
                crate::db::merge::ViewFilter::load(c, &server)?.tracks(&mut t);
                Ok(t)
            })
            .await?;
        let total_duration_ms = tracks.iter().map(|t| t.duration_ms).sum();
        Ok(LikedPage { tracks, total_duration_ms })
    }

    /// Playlists that hold the track (the "Saved in" menu).
    pub async fn playlists_with(&self, track_id: String) -> AppResult<Vec<String>> {
        self.db().read_async(move |c| local::playlists_with(c, &track_id)).await
    }

    // ------------------------------------------------------------ replay

    /// Send queued writes to the server, oldest first. Stops (keeping the write)
    /// when the server can't be reached; a refused write is rolled back with
    /// a toast.
    pub async fn replay(&self) {
        let _one_at_a_time = self.inner.replaying.lock().await;
        if !self.inner.remote.online().await {
            return;
        }
        let queue = match self.db().read_async(local::queued).await {
            Ok(q) => q,
            Err(e) => {
                tracing::warn!("rating queue: {e}");
                return;
            }
        };
        let mut rolled_back: Vec<String> = Vec::new();
        for op in queue {
            // A queued like is five stars, an unlike none (Likes::set), a
            // star rating its stars (Likes::rate).
            match self.inner.remote.set_rating(&op.track_id, op.rating).await {
                Ok(()) => {
                    let o = op.clone();
                    let _ = self.db().write_async(move |c| local::sent(c, &o)).await;
                }
                Err(e) if transient(&e) => {
                    let attempts = if matches!(e, AppError::Server(_)) {
                        let seq = op.seq;
                        self.db().write_async(move |c| local::bump_attempts(c, seq)).await.unwrap_or(0)
                    } else {
                        0
                    };
                    if attempts < MAX_ATTEMPTS {
                        tracing::debug!("rating write waits: {}", e.kind());
                        break;
                    }
                    self.refused(&op, &e).await;
                    rolled_back.push(op.track_id.clone());
                }
                Err(e) => {
                    self.refused(&op, &e).await;
                    rolled_back.push(op.track_id.clone());
                }
            }
        }
        if !rolled_back.is_empty() {
            self.inner.events.changed(&rolled_back);
        }
    }

    async fn refused(&self, op: &RatingOp, e: &AppError) {
        tracing::warn!("rating write refused: {}", e.kind());
        let o = op.clone();
        let title = self
            .db()
            .write_async(move |c| {
                local::roll_back(c, &o)?;
                Ok(c.query_row("SELECT title FROM tracks WHERE id = ?1", [&o.track_id], |r| r.get::<_, String>(0)).ok())
            })
            .await
            .ok()
            .flatten();
        let name = title.map(|t| format!("\"{t}\"")).unwrap_or_else(|| "a song".into());
        let message = if op.rating.is_some() {
            format!("Plex didn't save the like for {name}. It's back as it was.")
        } else {
            format!("Plex didn't remove {name} from Liked Songs. It's back as it was.")
        };
        self.inner.events.error(&message);
    }
}

/// Replay queued writes now and then (they also replay right after each
/// change). With nothing queued the loop sleeps until a write is queued.
pub fn spawn(likes: Likes) {
    tauri::async_runtime::spawn(async move {
        loop {
            likes.replay().await;
            likes.wait_for_work(REPLAY_EVERY).await;
        }
    });
}

impl Likes {
    /// Whether writes are still waiting to reach Plex.
    pub async fn pending(&self) -> bool {
        self.db().read_async(|c| Ok(!local::queued(c)?.is_empty())).await.unwrap_or(true)
    }

    /// The retry loop's wait: `retry` while writes wait, otherwise until one
    /// is queued.
    pub async fn wait_for_work(&self, retry: Duration) {
        if self.pending().await {
            tokio::time::sleep(retry).await;
        } else {
            self.inner.queued.notified().await;
        }
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub async fn likes_set(likes: State<'_, Likes>, track_ids: Vec<String>, liked: bool) -> AppResult<Vec<String>> {
    likes.set(track_ids, liked).await
}

#[tauri::command]
pub async fn liked_ids(likes: State<'_, Likes>) -> AppResult<Vec<String>> {
    likes.ids().await
}

#[tauri::command]
pub async fn liked_page(likes: State<'_, Likes>) -> AppResult<LikedPage> {
    likes.page().await
}

#[tauri::command]
pub async fn playlists_with_track(likes: State<'_, Likes>, track_id: String) -> AppResult<Vec<String>> {
    likes.playlists_with(track_id).await
}

#[cfg(test)]
#[path = "likes_tests.rs"]
mod tests;
