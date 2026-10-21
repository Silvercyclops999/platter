//! Playlist editing: create, add, remove, reorder, rename, cover, delete.
//!
//! Plex playlists are the store, so other Plex apps see every change. Each
//! edit is applied to the local cache at once (db::playlists) and queued;
//! the queue replays in order whenever the server is reachable, like
//! offline scrobbles in `tracking`. If Plex refuses an edit, the rest of
//! that playlist's queued edits are dropped, a toast says so, and the
//! playlist is read back from Plex.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tauri::State;

use crate::db::DbHandle;
use crate::db::playlists::{self as local, ItemRef, Op, Place, QueuedOp, SUMMARY_MAX, TITLE_MAX};
use crate::db::source::PlaylistEntry;
use crate::error::{AppError, AppResult};
use crate::model::Playlist;
use crate::plex::PlexState;

/// Server errors (5xx) are retried this many times before the edit counts
/// as refused.
const MAX_ATTEMPTS: u32 = 5;
/// Largest cover image accepted.
const COVER_MAX_BYTES: usize = 10 * 1024 * 1024;
const REPLAY_EVERY: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------- outside world

/// Where playlist edits go. The app uses the connected Plex server; tests
/// use a fake.
#[async_trait]
pub trait PlaylistRemote: Send + Sync {
    /// True when a server is connected right now.
    async fn online(&self) -> bool;
    /// Create a playlist holding `track_ids`; returns its Plex id.
    async fn create(&self, title: &str, track_ids: &[String]) -> AppResult<String>;
    async fn add(&self, id: &str, track_ids: &[String]) -> AppResult<()>;
    async fn remove(&self, id: &str, item_id: &str) -> AppResult<()>;
    async fn move_item(&self, id: &str, item_id: &str, after: Option<&str>) -> AppResult<()>;
    async fn update(&self, id: &str, title: Option<&str>, summary: Option<&str>) -> AppResult<()>;
    async fn poster(&self, id: &str, bytes: Vec<u8>, content_type: &str) -> AppResult<()>;
    async fn delete(&self, id: &str) -> AppResult<()>;
    /// The playlist and its rows as Plex has them now.
    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)>;
    /// A playlist named `title`, last changed close to `at` (unix seconds),
    /// that starts with `track_ids`: a create that got no clear answer and
    /// landed anyway. None where the server can't be asked.
    async fn find_made(&self, _title: &str, _track_ids: &[String], _at: i64) -> AppResult<Option<String>> {
        Ok(None)
    }
}

/// How far apart a lost create's send and the playlist's last change in
/// Plex may be (the two clocks, and the rest of a long create's songs).
const MADE_SLACK_S: i64 = 10 * 60;

/// A playlist last changed close enough to `at` to be the one a lost
/// create made (one of the user's own, changed long before or after, isn't).
fn made_near(updated_at: Option<i64>, at: i64) -> bool {
    updated_at.is_some_and(|t| (t - at).abs() <= MADE_SLACK_S)
}

/// Rows that are the start of `track_ids` (all of them, or the first part
/// a long create got in).
fn starts_with(rows: &[PlaylistEntry], track_ids: &[String]) -> bool {
    !rows.is_empty() && rows.len() <= track_ids.len() && rows.iter().zip(track_ids).all(|(r, t)| &r.track_id == t)
}

/// What the UI hears about.
pub trait PlaylistEvents: Send + Sync {
    /// Playlists changed (emits `library://changed`).
    fn changed(&self);
    /// An error toast.
    fn error(&self, message: &str);
}

pub struct PlexPlaylists(pub PlexState);

impl PlexPlaylists {
    /// The machine id of the server that has `id`.
    async fn machine(&self, id: &str) -> AppResult<String> {
        self.0.machine_id_for(id).await.ok_or_else(|| AppError::Network("Your server is offline.".into()))
    }
}

/// The one server all these songs are on (`crate::sid`; None for the
/// connected server). A Plex playlist holds songs of its own server only.
pub fn owner_of(track_ids: &[String]) -> AppResult<Option<String>> {
    let mut tags = track_ids.iter().map(|i| crate::sid::tag_of(i).map(str::to_string));
    let first = tags.next().flatten();
    if tags.any(|t| t.is_some() && t != first) {
        return Err(AppError::Invalid("A playlist can hold songs from one server only.".into()));
    }
    Ok(first)
}

#[async_trait]
impl PlaylistRemote for PlexPlaylists {
    async fn online(&self) -> bool {
        self.0.client().await.is_ok()
    }
    // A playlist lives on the server that has its songs (crate::sid); its
    // edits go there.
    async fn create(&self, title: &str, track_ids: &[String]) -> AppResult<String> {
        let first = track_ids.first().map(String::as_str).unwrap_or_default();
        owner_of(track_ids)?;
        let m = self.machine(first).await?;
        Ok(self.0.client_for(first).await?.create_playlist(&m, title, track_ids).await?.id)
    }
    async fn add(&self, id: &str, track_ids: &[String]) -> AppResult<()> {
        let m = self.machine(id).await?;
        self.0.client_for(id).await?.playlist_add(&m, id, track_ids).await
    }
    async fn remove(&self, id: &str, item_id: &str) -> AppResult<()> {
        self.0.client_for(id).await?.playlist_remove(id, item_id).await
    }
    async fn move_item(&self, id: &str, item_id: &str, after: Option<&str>) -> AppResult<()> {
        self.0.client_for(id).await?.playlist_move(id, item_id, after).await
    }
    async fn update(&self, id: &str, title: Option<&str>, summary: Option<&str>) -> AppResult<()> {
        self.0.client_for(id).await?.playlist_edit(id, title, summary).await
    }
    async fn poster(&self, id: &str, bytes: Vec<u8>, content_type: &str) -> AppResult<()> {
        self.0.client_for(id).await?.playlist_poster(id, bytes, content_type).await
    }
    async fn delete(&self, id: &str) -> AppResult<()> {
        self.0.client_for(id).await?.playlist_delete(id).await
    }
    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)> {
        let c = self.0.client_for(id).await?;
        let p = c.playlist(id).await?;
        let rows = c.playlist_rows(id).await?;
        Ok((p, rows.into_iter().map(|(track_id, item_id)| PlaylistEntry { track_id, item_id }).collect()))
    }
    async fn find_made(&self, title: &str, track_ids: &[String], at: i64) -> AppResult<Option<String>> {
        let Some(first) = track_ids.first() else { return Ok(None) };
        let lists = self.0.client_for(first).await?.playlists().await?;
        for p in lists.into_iter().filter(|p| !p.smart && p.title == title && made_near(p.updated_at, at)) {
            let (_, rows) = self.fetch(&p.id).await?;
            if starts_with(&rows, track_ids) {
                return Ok(Some(p.id));
            }
        }
        Ok(None)
    }
}

/// Events through the app: `library://changed` and a `toast`.
pub struct AppEvents {
    pub plex: PlexState,
    pub db: DbHandle,
}

impl PlaylistEvents for AppEvents {
    fn changed(&self) {
        if let Some(e) = self.db.events() {
            e.library_changed();
        }
    }
    fn error(&self, message: &str) {
        self.plex.toast("error", message);
    }
}

// ---------------------------------------------------------------- service

/// Managed state behind the playlist commands. Cheap to clone.
#[derive(Clone)]
pub struct Playlists {
    inner: Arc<Inner>,
}

struct Inner {
    db: DbHandle,
    remote: Arc<dyn PlaylistRemote>,
    events: Arc<dyn PlaylistEvents>,
    /// Start a replay after each edit (off in tests, which replay by hand).
    auto_replay: bool,
    replaying: tokio::sync::Mutex<()>,
    /// Wakes the retry loop when an edit is queued.
    queued: tokio::sync::Notify,
}

fn transient(e: &AppError) -> bool {
    matches!(e, AppError::Network(_) | AppError::Auth(_) | AppError::Server(_))
}

fn clean_title(title: &str) -> AppResult<String> {
    let t = title.trim();
    if t.is_empty() {
        return Err(AppError::Invalid("Playlist name is required.".into()));
    }
    if t.chars().count() > TITLE_MAX {
        return Err(AppError::Invalid(format!("Playlist names can be up to {TITLE_MAX} characters.")));
    }
    Ok(t.to_string())
}

fn clean_summary(summary: &str) -> AppResult<String> {
    let s = summary.trim();
    if s.chars().count() > SUMMARY_MAX {
        return Err(AppError::Invalid(format!("Descriptions can be up to {SUMMARY_MAX} characters.")));
    }
    Ok(s.to_string())
}

enum Replayed {
    /// Sent; the playlist (maybe with a new id) needs reading back.
    Done(Option<String>),
    /// A new playlist without songs yet: it waits.
    Waiting,
}

impl Playlists {
    pub fn new(db: DbHandle, remote: Arc<dyn PlaylistRemote>, events: Arc<dyn PlaylistEvents>) -> Self {
        Self::build(db, remote, events, true)
    }

    pub fn build(
        db: DbHandle,
        remote: Arc<dyn PlaylistRemote>,
        events: Arc<dyn PlaylistEvents>,
        auto_replay: bool,
    ) -> Self {
        Self {
            inner: Arc::new(Inner { db, remote, events, auto_replay, replaying: Default::default(), queued: Default::default() }),
        }
    }

    fn db(&self) -> &DbHandle {
        &self.inner.db
    }

    /// Tell the UI and start sending the edit.
    fn edited(&self) {
        self.inner.events.changed();
        self.inner.queued.notify_one();
        if self.inner.auto_replay
            && let Ok(rt) = tokio::runtime::Handle::try_current()
        {
            let me = self.clone();
            rt.spawn(async move { me.replay().await });
        }
    }

    // ------------------------------------------------------------ commands

    /// A new playlist, named from the Settings template unless `title` is
    /// given, holding `track_ids`.
    pub async fn create(&self, title: Option<String>, track_ids: Vec<String>) -> AppResult<Playlist> {
        let template = crate::settings::load(self.db()).playlist_new_name;
        let p = self
            .db()
            .write_async(move |c| {
                let tx = c.transaction()?;
                let title = match title {
                    Some(t) => clean_title(&t)?,
                    None => local::next_name(&tx, &template)?,
                };
                let p = local::create_local(&tx, &title)?;
                if !track_ids.is_empty() {
                    local::add(&tx, &p.id, &track_ids, &Place::End)?;
                }
                local::enqueue(&tx, &p.id, &Op::Create)?;
                let p = local::playlist(&tx, &p.id)?.unwrap_or(p);
                tx.commit()?;
                Ok(p)
            })
            .await?;
        self.edited();
        Ok(p)
    }

    /// Add songs at the end, at the top, or after a row.
    pub async fn add(&self, id: String, track_ids: Vec<String>, place: Place) -> AppResult<Playlist> {
        if track_ids.is_empty() {
            return Err(AppError::Invalid("No songs to add.".into()));
        }
        let p = self
            .db()
            .write_async(move |c| {
                let tx = c.transaction()?;
                let p = local::editable(&tx, &id)?;
                let new_ids = local::add(&tx, &p.id, &track_ids, &place)?;
                if !local::is_local_only(&tx, &p.id)? {
                    local::enqueue(&tx, &p.id, &Op::Add { track_ids: track_ids.clone(), item_ids: new_ids.clone(), sent: false })?;
                    if place != Place::End {
                        // Plex appends; then each new row moves into place.
                        let rows = local::items(&tx, &p.id)?;
                        let refs: Vec<ItemRef> = rows
                            .iter()
                            .enumerate()
                            .map(|(index, r)| ItemRef { item_id: r.item_id.clone(), track_id: r.track_id.clone(), index })
                            .collect();
                        for id in &new_ids {
                            let Some(pos) = refs.iter().position(|r| &r.item_id == id) else { continue };
                            let after = pos.checked_sub(1).map(|p| refs[p].clone());
                            local::enqueue(&tx, &p.id, &Op::Move { item: refs[pos].clone(), after })?;
                        }
                    }
                }
                let p = local::playlist(&tx, &p.id)?.unwrap_or(p);
                tx.commit()?;
                Ok(p)
            })
            .await?;
        self.edited();
        Ok(p)
    }

    /// Which of these songs the playlist already has.
    pub async fn duplicates(&self, id: String, track_ids: Vec<String>) -> AppResult<Vec<String>> {
        self.db().read_async(move |c| local::duplicates(c, &id, &track_ids)).await
    }

    /// Remove rows; returns what was removed and where (for undo).
    pub async fn remove(&self, id: String, item_ids: Vec<String>) -> AppResult<Vec<ItemRef>> {
        let removed = self
            .db()
            .write_async(move |c| {
                let tx = c.transaction()?;
                let p = local::editable(&tx, &id)?;
                let removed = local::remove(&tx, &p.id, &item_ids)?;
                if !local::is_local_only(&tx, &p.id)? {
                    for item in &removed {
                        local::enqueue(&tx, &p.id, &Op::Remove { item: item.clone() })?;
                    }
                }
                tx.commit()?;
                Ok(removed)
            })
            .await?;
        if !removed.is_empty() {
            self.edited();
        }
        Ok(removed)
    }

    /// Move rows (in their current order) to just after `after`, or to the
    /// top without it.
    pub async fn move_items(&self, id: String, item_ids: Vec<String>, after: Option<String>) -> AppResult<()> {
        let moved = self
            .db()
            .write_async(move |c| {
                let tx = c.transaction()?;
                let p = local::editable(&tx, &id)?;
                let moves = local::move_items(&tx, &p.id, &item_ids, after.as_deref())?;
                if !local::is_local_only(&tx, &p.id)? {
                    for m in &moves {
                        local::enqueue(&tx, &p.id, &Op::Move { item: m.item.clone(), after: m.after.clone() })?;
                    }
                }
                tx.commit()?;
                Ok(!moves.is_empty())
            })
            .await?;
        if moved {
            self.edited();
        }
        Ok(())
    }

    /// Rename and/or change the description ("" clears it).
    pub async fn update(&self, id: String, title: Option<String>, summary: Option<String>) -> AppResult<Playlist> {
        let title = title.map(|t| clean_title(&t)).transpose()?;
        let summary = summary.map(|s| clean_summary(&s)).transpose()?;
        let p = self
            .db()
            .write_async(move |c| {
                let tx = c.transaction()?;
                let p = local::editable(&tx, &id)?;
                let title = title.filter(|t| *t != p.title);
                let summary = summary.filter(|s| s.as_str() != p.summary.as_deref().unwrap_or(""));
                if title.is_none() && summary.is_none() {
                    return Ok(p);
                }
                local::update_details(&tx, &p.id, title.as_deref(), summary.as_deref())?;
                if !local::is_local_only(&tx, &p.id)? {
                    local::enqueue(&tx, &p.id, &Op::Update { title, summary })?;
                }
                let p = local::playlist(&tx, &p.id)?.unwrap_or(p);
                tx.commit()?;
                Ok(p)
            })
            .await?;
        self.edited();
        Ok(p)
    }

    /// Upload a cover image for the playlist.
    pub async fn set_cover(&self, id: String, bytes: Vec<u8>, content_type: String) -> AppResult<()> {
        if !matches!(content_type.as_str(), "image/jpeg" | "image/png" | "image/webp") {
            return Err(AppError::Invalid("Choose a JPEG, PNG or WebP image.".into()));
        }
        if bytes.is_empty() || bytes.len() > COVER_MAX_BYTES {
            return Err(AppError::Invalid("Choose an image under 10 MB.".into()));
        }
        self.db()
            .write_async(move |c| {
                let p = local::editable(c, &id)?;
                local::enqueue(c, &p.id, &Op::Poster { bytes, content_type })
            })
            .await?;
        self.edited();
        Ok(())
    }

    /// Delete the playlist (from Plex too).
    pub async fn delete(&self, id: String) -> AppResult<()> {
        self.db()
            .write_async(move |c| {
                let tx = c.transaction()?;
                let p = local::editable(&tx, &id)?;
                let local_only = local::is_local_only(&tx, &p.id)?;
                // A create that went out without an answer stays queued: if
                // it landed, its replay finds that playlist and deletes it.
                let sent: Vec<Op> = local::queued(&tx)?
                    .into_iter()
                    .filter(|q| q.playlist_id == p.id && matches!(q.op, Op::CreateSent { .. }))
                    .map(|q| q.op)
                    .collect();
                local::delete(&tx, &p.id)?;
                local::drop_ops_for(&tx, &p.id)?;
                for op in &sent {
                    local::enqueue(&tx, &p.id, op)?;
                }
                if !local_only {
                    local::enqueue(&tx, &p.id, &Op::Delete)?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.edited();
        Ok(())
    }

    // ------------------------------------------------------------ replay

    /// Send queued edits to Plex, oldest first. Stops (keeping the edit) when
    /// the server can't be reached; a refused edit drops that playlist's
    /// queued edits, shows a toast and reads the playlist back from Plex.
    pub async fn replay(&self) {
        let _one_at_a_time = self.inner.replaying.lock().await;
        if !self.inner.remote.online().await {
            return;
        }
        let mut waiting: HashSet<String> = HashSet::new();
        // Edits already handled this round: one that comes round again
        // couldn't be taken off the queue, so stop rather than loop.
        let mut handled: HashSet<i64> = HashSet::new();
        let mut touched: Vec<String> = Vec::new();
        let mut changed = false;
        loop {
            let queue = match self.db().read_async(local::queued).await {
                Ok(q) => q,
                Err(e) => {
                    tracing::warn!("playlist queue: {e}");
                    return;
                }
            };
            let Some(q) = queue.into_iter().find(|q| !waiting.contains(&q.playlist_id)) else { break };
            if !handled.insert(q.seq) {
                tracing::warn!("playlist edit {} is still queued after it was handled; trying again later", q.seq);
                break;
            }
            match self.replay_one(&q).await {
                Ok(Replayed::Done(id)) => {
                    let seq = q.seq;
                    if let Err(e) = self.db().write_async(move |c| local::drop_op(c, seq)).await {
                        tracing::warn!("sent playlist edit not taken off the queue: {e}");
                        break;
                    }
                    if let Some(id) = id
                        && !touched.contains(&id)
                    {
                        touched.push(id);
                    }
                }
                Ok(Replayed::Waiting) => {
                    waiting.insert(q.playlist_id.clone());
                }
                Err(e) if transient(&e) => {
                    let seq = q.seq;
                    let attempts = if matches!(e, AppError::Server(_)) {
                        self.db().write_async(move |c| local::bump_attempts(c, seq)).await.unwrap_or(0)
                    } else {
                        0
                    };
                    if attempts < MAX_ATTEMPTS {
                        tracing::debug!("playlist edit waits: {}", e.kind());
                        break;
                    }
                    self.refused(&q, &e).await;
                    changed = true;
                }
                Err(e) => {
                    self.refused(&q, &e).await;
                    changed = true;
                }
            }
        }
        for id in touched {
            let has = { let id = id.clone(); self.db().read_async(move |c| local::has_queued(c, &id)).await };
            if matches!(has, Ok(false)) {
                self.resync(&id).await;
                changed = true;
            }
        }
        if changed {
            self.inner.events.changed();
        }
    }

    async fn replay_one(&self, q: &QueuedOp) -> AppResult<Replayed> {
        let remote = &self.inner.remote;
        let pid = q.playlist_id.as_str();
        match &q.op {
            Op::Create => self.replay_create(q, None).await,
            Op::CreateSent { at, title, track_ids } => self.replay_create(q, Some((*at, title, track_ids))).await,
            Op::Add { track_ids, item_ids, sent } => {
                // An earlier send that got no clear answer may have landed:
                // if the playlist already ends with these songs, don't add
                // them twice.
                // A long add goes out in parts, so the first ones may have
                // landed alone: only the rest is sent again.
                let landed = if *sent {
                    let (_, rows) = remote.fetch(pid).await?;
                    (1..=track_ids.len())
                        .rev()
                        .find(|&k| rows.len() >= k && rows[rows.len() - k..].iter().map(|r| &r.track_id).eq(track_ids[..k].iter()))
                        .unwrap_or(0)
                } else {
                    0
                };
                if landed < track_ids.len() {
                    let seq = q.seq;
                    self.db().write_async(move |c| local::mark_sent(c, seq)).await?;
                    remote.add(pid, &track_ids[landed..]).await?;
                }
                // Plex appends: the last rows are the new ones. Learn their ids
                // so queued removes and moves can name them. The add is done
                // either way; without the ids those edits look them up.
                if let Ok((_, rows)) = remote.fetch(pid).await {
                    let start = rows.len().saturating_sub(item_ids.len());
                    let pairs: Vec<(String, String)> = item_ids
                        .iter()
                        .zip(&rows[start..])
                        .zip(track_ids)
                        .filter_map(|((local_id, row), track)| {
                            (row.track_id == *track).then(|| row.item_id.clone().map(|r| (local_id.clone(), r)))?
                        })
                        .collect();
                    let mapped = self
                        .db()
                        .write_async(move |c| {
                            for (l, r) in &pairs {
                                local::map_id(c, l, r)?;
                            }
                            Ok(())
                        })
                        .await;
                    if let Err(e) = mapped {
                        tracing::warn!("new playlist rows not matched to their ids: {e}");
                    }
                }
                Ok(Replayed::Done(Some(pid.to_string())))
            }
            Op::Remove { item } => {
                if let Some(id) = self.plex_item(pid, item).await? {
                    remote.remove(pid, &id).await?;
                }
                Ok(Replayed::Done(Some(pid.to_string())))
            }
            Op::Move { item, after } => {
                let Some(id) = self.plex_item(pid, item).await? else {
                    return Ok(Replayed::Done(Some(pid.to_string())));
                };
                let after = match after {
                    Some(a) => Some(
                        self.plex_item(pid, a)
                            .await?
                            .ok_or_else(|| AppError::NotFound("playlist item".into()))?,
                    ),
                    None => None,
                };
                remote.move_item(pid, &id, after.as_deref()).await?;
                Ok(Replayed::Done(Some(pid.to_string())))
            }
            Op::Update { title, summary } => {
                remote.update(pid, title.as_deref(), summary.as_deref()).await?;
                Ok(Replayed::Done(Some(pid.to_string())))
            }
            Op::Poster { bytes, content_type } => {
                if local::is_local(pid) {
                    // Its playlist isn't in Plex yet.
                    return Ok(Replayed::Waiting);
                }
                remote.poster(pid, bytes.clone(), content_type).await?;
                Ok(Replayed::Done(Some(pid.to_string())))
            }
            Op::Delete => match remote.delete(pid).await {
                Ok(()) | Err(AppError::NotFound(_)) => Ok(Replayed::Done(None)),
                Err(e) => Err(e),
            },
        }
    }

    /// `sent`: when an earlier try went out without a clear answer.
    async fn replay_create(&self, q: &QueuedOp, sent: Option<(i64, &String, &Vec<String>)>) -> AppResult<Replayed> {
        let local_id = q.playlist_id.clone();
        let lid = local_id.clone();
        let snapshot = self
            .db()
            .read_async(move |c| Ok(local::playlist(c, &lid)?.map(|p| (p, local::items(c, &lid)))))
            .await?;
        let remote = &self.inner.remote;
        // An earlier try may have made it already (named and filled as it
        // was sent): that one is used, not a second.
        let made = match sent {
            Some((at, title, tracks)) => remote.find_made(title, tracks, at).await?.map(|id| (id, title.clone())),
            None => None,
        };
        let Some((p, items)) = snapshot else {
            // Deleted before it reached Plex; one an earlier try made goes too.
            if let Some((id, _)) = made {
                match remote.delete(&id).await {
                    Ok(()) | Err(AppError::NotFound(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            return Ok(Replayed::Done(None));
        };
        let items = items?;
        if items.is_empty() {
            return Ok(Replayed::Waiting);
        }
        let tracks: Vec<String> = items.iter().map(|i| i.track_id.clone()).collect();
        // What Plex has it named, when an earlier try made it.
        let made_as = made.as_ref().map(|(_, t)| t.clone());
        let plex_id = match made {
            Some((id, _)) => id,
            None => {
                let seq = q.seq;
                self.db().write_async(move |c| local::mark_sent(c, seq)).await?;
                remote.create(&p.title, &tracks).await?
            }
        };
        if let Some(s) = p.summary.as_deref().filter(|s| !s.is_empty())
            && let Err(e) = remote.update(&plex_id, None, Some(s)).await
        {
            // The playlist exists now; don't create it twice over a description.
            tracing::warn!("new playlist description not sent: {}", e.kind());
        }
        let rows = remote.fetch(&plex_id).await.map(|(_, rows)| rows).unwrap_or_default();
        let pid = plex_id.clone();
        let adopted = self
            .db()
            .write_async(move |c| {
                let tx = c.transaction()?;
                let Some(here) = local::playlist(&tx, &local_id)? else {
                    // Deleted here while Plex was making it: delete it there too.
                    local::enqueue(&tx, &pid, &Op::Delete)?;
                    tx.commit()?;
                    return Ok(false);
                };
                local::adopt(&tx, &local_id, &pid)?;
                local::drop_ops_after_create(&tx, &pid)?;
                // Renamed or described here meanwhile: Plex has the old ones.
                let plex_title = made_as.as_deref().unwrap_or(&p.title);
                let title = Some(here.title.clone()).filter(|t| t != plex_title);
                let summary = Some(here.summary.clone().unwrap_or_default()).filter(|s| s.as_str() != p.summary.as_deref().unwrap_or(""));
                if title.is_some() || summary.is_some() {
                    local::enqueue(&tx, &pid, &Op::Update { title, summary })?;
                }
                let now = local::items(&tx, &pid)?;
                // An earlier try made it with other songs than it has now:
                // Plex's rows go and these are added, in this order.
                if made_as.is_some() && !rows.iter().map(|r| &r.track_id).eq(items.iter().map(|i| &i.track_id)) {
                    for (index, r) in rows.iter().enumerate() {
                        if let Some(rid) = &r.item_id {
                            let item = ItemRef { item_id: rid.clone(), track_id: r.track_id.clone(), index };
                            local::enqueue(&tx, &pid, &Op::Remove { item })?;
                        }
                    }
                    if !now.is_empty() {
                        let track_ids = now.iter().map(|i| i.track_id.clone()).collect();
                        let item_ids = now.iter().map(|i| i.item_id.clone()).collect();
                        local::enqueue(&tx, &pid, &Op::Add { track_ids, item_ids, sent: false })?;
                    }
                    tx.commit()?;
                    return Ok(true);
                }
                // Plex made it without its last songs (a long playlist goes
                // out in parts and a later part failed): those are added.
                let landed = rows.len();
                let short = landed > 0
                    && landed < items.len()
                    && rows.iter().zip(&items).all(|(r, i)| r.track_id == i.track_id);
                let in_plex = if short { landed } else { items.len() };
                let missing: Vec<&local::Item> =
                    items[in_plex..].iter().filter(|m| now.iter().any(|n| n.item_id == m.item_id)).collect();
                if !missing.is_empty() {
                    let track_ids = missing.iter().map(|i| i.track_id.clone()).collect();
                    let item_ids = missing.iter().map(|i| i.item_id.clone()).collect();
                    local::enqueue(&tx, &pid, &Op::Add { track_ids, item_ids, sent: false })?;
                }
                // Edits made here while Plex was making the playlist weren't
                // queued (it was only here then): queue what changed since.
                for (index, gone) in items[..in_plex].iter().enumerate() {
                    if !now.iter().any(|n| n.item_id == gone.item_id) {
                        let item = ItemRef { item_id: gone.item_id.clone(), track_id: gone.track_id.clone(), index };
                        local::enqueue(&tx, &pid, &Op::Remove { item })?;
                    }
                }
                // Rows Plex has, moved here meanwhile: put them in the same order there.
                let kept: Vec<&local::Item> = now.iter().filter(|n| items[..in_plex].iter().any(|s| s.item_id == n.item_id)).collect();
                let sent_order: Vec<&local::Item> = items[..in_plex].iter().filter(|s| kept.iter().any(|k| k.item_id == s.item_id)).collect();
                if kept.iter().map(|k| &k.item_id).ne(sent_order.iter().map(|s| &s.item_id)) {
                    let at = |i: &local::Item| ItemRef {
                        item_id: i.item_id.clone(),
                        track_id: i.track_id.clone(),
                        index: items.iter().position(|s| s.item_id == i.item_id).unwrap_or(0),
                    };
                    for (k, i) in kept.iter().enumerate() {
                        let after = k.checked_sub(1).map(|p| at(kept[p]));
                        local::enqueue(&tx, &pid, &Op::Move { item: at(i), after })?;
                    }
                }
                let added: Vec<usize> = (0..now.len()).filter(|&i| !items.iter().any(|s| s.item_id == now[i].item_id)).collect();
                if !added.is_empty() {
                    let track_ids = added.iter().map(|&i| now[i].track_id.clone()).collect();
                    let item_ids = added.iter().map(|&i| now[i].item_id.clone()).collect();
                    local::enqueue(&tx, &pid, &Op::Add { track_ids, item_ids, sent: false })?;
                    // Plex appends; a row added elsewhere moves into place.
                    let tail = now.len() - added.len();
                    for (k, &i) in added.iter().enumerate() {
                        if i != tail + k {
                            let item = ItemRef { item_id: now[i].item_id.clone(), track_id: now[i].track_id.clone(), index: i };
                            let after = i.checked_sub(1).map(|p| ItemRef { item_id: now[p].item_id.clone(), track_id: now[p].track_id.clone(), index: p });
                            local::enqueue(&tx, &pid, &Op::Move { item, after })?;
                        }
                    }
                }
                if rows.len() == items.len() {
                    for (i, r) in items.iter().zip(&rows) {
                        if let Some(rid) = r.item_id.as_deref().filter(|_| r.track_id == i.track_id) {
                            local::map_id(&tx, &i.item_id, rid)?;
                        }
                    }
                }
                tx.commit()?;
                Ok(true)
            })
            .await?;
        Ok(Replayed::Done(adopted.then_some(plex_id)))
    }

    /// The Plex id of a row. A row only known here is looked up in Plex by
    /// its place and song; None when it's gone.
    async fn plex_item(&self, pid: &str, item: &ItemRef) -> AppResult<Option<String>> {
        if !local::is_local(&item.item_id) {
            return Ok(Some(item.item_id.clone()));
        }
        let (_, rows) = self.inner.remote.fetch(pid).await?;
        let at = rows.get(item.index).filter(|r| r.track_id == item.track_id);
        let found = at.or_else(|| rows.iter().find(|r| r.track_id == item.track_id));
        Ok(found.and_then(|r| r.item_id.clone()))
    }

    async fn refused(&self, q: &QueuedOp, e: &AppError) {
        let pid = q.playlist_id.clone();
        tracing::warn!("playlist edit refused: {}", e.kind());
        let title_now = || {
            let p = pid.clone();
            async move { self.db().read_async(move |c| Ok(local::playlist(c, &p)?.map(|p| p.title))).await.ok().flatten() }
        };
        let edited = title_now().await;
        let p = pid.clone();
        let _ = self.db().write_async(move |c| local::drop_ops_for(c, &p)).await;
        self.resync(&pid).await;
        // Name it as Plex has it (a refused rename shouldn't show its new name).
        let title = title_now().await.or(edited);
        let name = title.map(|t| format!("\"{t}\"")).unwrap_or_else(|| "a playlist".into());
        let message = match &q.op {
            Op::Create | Op::CreateSent { .. } => format!("Your server didn't accept the new playlist {name}."),
            Op::Delete => format!("Your server didn't delete {name}."),
            _ => format!("Your server didn't accept a change to {name}. Showing it as it is there."),
        };
        self.inner.events.error(&message);
    }

    /// Read a playlist back from Plex into the cache. Gone from Plex: gone
    /// here too.
    async fn resync(&self, id: &str) {
        if local::is_local(id) {
            return;
        }
        let id = id.to_string();
        match self.inner.remote.fetch(&id).await {
            Ok((p, rows)) => {
                let _ = self
                    .db()
                    .write_async(move |c| {
                        if local::has_queued(c, &p.id)? {
                            return Ok(());
                        }
                        let tx = c.transaction()?;
                        local::apply_remote(&tx, &p, &rows)?;
                        tx.commit()?;
                        Ok(())
                    })
                    .await;
            }
            Err(AppError::NotFound(_)) => {
                let _ = self
                    .db()
                    .write_async(move |c| {
                        local::delete(c, &id)?;
                        local::drop_ops_for(c, &id)
                    })
                    .await;
            }
            Err(e) => tracing::debug!("playlist not read back: {}", e.kind()),
        }
    }
}

/// Replay queued edits now and then (they also replay right after each
/// edit). With nothing queued the loop sleeps until an edit is queued.
pub fn spawn(playlists: Playlists) {
    tauri::async_runtime::spawn(async move {
        loop {
            playlists.replay().await;
            playlists.wait_for_work(REPLAY_EVERY).await;
        }
    });
}

impl Playlists {
    /// Whether edits are still waiting to reach Plex.
    pub async fn pending(&self) -> bool {
        self.db().read_async(|c| Ok(!local::queued(c)?.is_empty())).await.unwrap_or(true)
    }

    /// The retry loop's wait: `retry` while edits wait, otherwise until one
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
pub async fn playlist_create(
    playlists: State<'_, Playlists>,
    title: Option<String>,
    track_ids: Option<Vec<String>>,
) -> AppResult<Playlist> {
    playlists.create(title, track_ids.unwrap_or_default()).await
}

#[tauri::command]
pub async fn playlist_add(
    playlists: State<'_, Playlists>,
    id: String,
    track_ids: Vec<String>,
    after: Option<String>,
    top: Option<bool>,
) -> AppResult<Playlist> {
    let place = match (after, top) {
        (Some(a), _) => Place::After(a),
        (None, Some(true)) => Place::Top,
        _ => Place::End,
    };
    playlists.add(id, track_ids, place).await
}

#[tauri::command]
pub async fn playlist_duplicates(
    playlists: State<'_, Playlists>,
    id: String,
    track_ids: Vec<String>,
) -> AppResult<Vec<String>> {
    playlists.duplicates(id, track_ids).await
}

#[tauri::command]
pub async fn playlist_remove(
    playlists: State<'_, Playlists>,
    id: String,
    item_ids: Vec<String>,
) -> AppResult<Vec<ItemRef>> {
    playlists.remove(id, item_ids).await
}

#[tauri::command]
pub async fn playlist_move(
    playlists: State<'_, Playlists>,
    id: String,
    item_ids: Vec<String>,
    after: Option<String>,
) -> AppResult<()> {
    playlists.move_items(id, item_ids, after).await
}

#[tauri::command]
pub async fn playlist_update(
    playlists: State<'_, Playlists>,
    id: String,
    title: Option<String>,
    summary: Option<String>,
) -> AppResult<Playlist> {
    playlists.update(id, title, summary).await
}

#[tauri::command]
pub async fn playlist_set_cover(
    playlists: State<'_, Playlists>,
    id: String,
    bytes: Vec<u8>,
    content_type: String,
) -> AppResult<()> {
    playlists.set_cover(id, bytes, content_type).await
}

#[tauri::command]
pub async fn playlist_delete(playlists: State<'_, Playlists>, id: String) -> AppResult<()> {
    playlists.delete(id).await
}

#[cfg(test)]
#[path = "playlists_tests.rs"]
mod tests;
