//! Edit in Plex: change an album cover or artist photo, fix or undo a
//! match, edit details with field locks, merge and split artists, and
//! refresh. Everything goes through Plex's metadata API (`plex::edit`), so
//! no files are touched and it works away from home.
//!
//! Only servers this account owns can be edited; the UI hides the actions
//! elsewhere, and a server that refuses an edit (a Home user without admin
//! rights) is treated as not editable for the rest of the run.
//!
//! Every change is kept in a short local log with what it replaced (the
//! previous field values and locks, the previous match, the previous cover
//! and a copy of its image), so it can be undone from a toast or from the
//! item's Recent changes.

pub mod commands;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::plex::PlexClient;
use crate::plex::edit::{EditKind, EditableItem, ImageChoice, ImageKind, MatchCandidate, NO_PERMISSION, Snapshot, field_query};

/// Largest image accepted for upload.
pub const IMAGE_MAX_BYTES: usize = 10 * 1024 * 1024;
/// Changes kept in the log.
const KEEP: usize = 100;
const LOG_FILE: &str = "changes.json";
const SAVED_DIR: &str = "images";
/// Size the previous cover is kept at.
const SAVED_PX: u32 = 1200;

// ---------------------------------------------------------------- outside world

/// Which servers can be edited, and a client for an item's server.
#[async_trait]
pub trait EditServers: Send + Sync {
    async fn client_for(&self, id: &str) -> AppResult<PlexClient>;
    /// Tags of the servers this account owns, and whether the connected
    /// server (whose ids may have no tag) is one of them.
    async fn owned(&self) -> (Vec<String>, bool);
}

/// The rest of the app.
#[async_trait]
pub trait EditHooks: Send + Sync {
    /// The album counts as unreleased (the archive's release rules).
    async fn unreleased(&self, album_id: &str) -> bool;
    /// Items changed on the server: drop their cached art when `art`, read
    /// them into the library cache again, and run a sync when `resync`
    /// (merge and split move albums between artists).
    async fn changed(&self, kind: EditKind, ids: &[String], art: bool, resync: bool);
    fn settings(&self) -> EditSettings;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditSettings {
    pub show: bool,
    pub lock: bool,
    pub match_unreleased: bool,
}

impl Default for EditSettings {
    fn default() -> Self {
        Self { show: true, lock: true, match_unreleased: true }
    }
}

// ---------------------------------------------------------------- IPC shapes

/// IPC `plex_edit_access`: what the UI may offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditAccess {
    /// The Settings switch.
    pub show: bool,
    /// Tags of servers that can be edited.
    pub servers: Vec<String>,
    /// Items without a tag (the connected server's) can be edited.
    pub connected: bool,
    pub lock_fields: bool,
    pub match_unreleased: bool,
}

/// IPC `plex_edit_item`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditInfo {
    #[serde(flatten)]
    pub item: EditableItem,
    pub matched: bool,
    /// Albums only.
    pub unreleased: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditAction {
    Image,
    Match,
    Unmatch,
    Details,
    Merge,
    Split,
    Refresh,
}

/// One change, as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditChange {
    pub id: String,
    pub item_id: String,
    pub kind: EditKind,
    pub action: EditAction,
    /// "Changed the cover", "Matched to …".
    pub label: String,
    /// Unix ms.
    pub at: i64,
    pub undoable: bool,
    pub undone: bool,
}

/// What undoing a change takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
enum Undo {
    Image { which: ImageKind, previous: Option<String>, saved: Option<String> },
    Match { previous: Option<String>, name: String, year: Option<i64> },
    Details { section: String, before: Box<Snapshot>, after: Box<Snapshot>, locks: Vec<String> },
    Merge { target: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Logged {
    change: EditChange,
    undo: Option<Undo>,
}

// ---------------------------------------------------------------- service

#[derive(Clone)]
pub struct PlexEditor {
    inner: Arc<Inner>,
}

struct Inner {
    servers: Arc<dyn EditServers>,
    hooks: Arc<dyn EditHooks>,
    dir: PathBuf,
    log: Mutex<Vec<Logged>>,
    /// Servers that refused an edit this run ("" for the connected one).
    denied: Mutex<HashSet<String>>,
    seq: AtomicU64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn server_of(id: &str) -> String {
    crate::sid::tag_of(id).unwrap_or_default().to_string()
}

fn year_of(released: &str) -> Option<i64> {
    released.get(..4).and_then(|y| y.parse().ok())
}

fn image_noun(kind: EditKind, which: ImageKind) -> &'static str {
    match (kind, which) {
        (EditKind::Album, _) => "cover",
        (EditKind::Artist, ImageKind::Poster) => "photo",
        (EditKind::Artist, ImageKind::Art) => "background",
    }
}

impl PlexEditor {
    /// The log and kept images live in `dir`.
    pub fn new(dir: PathBuf, servers: Arc<dyn EditServers>, hooks: Arc<dyn EditHooks>) -> Self {
        let log = std::fs::read(dir.join(LOG_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<Logged>>(&b).ok())
            .unwrap_or_default();
        Self {
            inner: Arc::new(Inner {
                servers,
                hooks,
                dir,
                log: Mutex::new(log),
                denied: Mutex::new(HashSet::new()),
                seq: AtomicU64::new(0),
            }),
        }
    }

    pub async fn access(&self) -> EditAccess {
        let s = self.inner.hooks.settings();
        let (servers, connected) = self.inner.servers.owned().await;
        let denied = self.inner.denied.lock().clone();
        EditAccess {
            show: s.show,
            servers: servers.into_iter().filter(|t| !denied.contains(t)).collect(),
            connected: connected && !denied.contains(""),
            lock_fields: s.lock,
            match_unreleased: s.match_unreleased,
        }
    }

    /// The item's server can be edited, and editing is on.
    async fn guard(&self, id: &str) -> AppResult<PlexClient> {
        let a = self.access().await;
        if !a.show {
            return Err(AppError::Invalid("Editing in Plex is turned off in Settings.".into()));
        }
        let ok = match crate::sid::tag_of(id) {
            Some(t) => a.servers.iter().any(|s| s == t),
            None => a.connected,
        };
        if !ok {
            return Err(AppError::Auth(NO_PERMISSION.into()));
        }
        self.inner.servers.client_for(id).await
    }

    /// A refusal from the server: hide editing for it from now on.
    fn note<T>(&self, id: &str, r: AppResult<T>) -> AppResult<T> {
        if let Err(AppError::Auth(_)) = &r {
            self.inner.denied.lock().insert(server_of(id));
        }
        r
    }

    pub async fn item(&self, id: &str) -> AppResult<EditInfo> {
        let c = self.guard(id).await?;
        let item = self.note(id, c.editable(id).await)?;
        let unreleased = item.kind == Some(EditKind::Album) && self.inner.hooks.unreleased(id).await;
        Ok(EditInfo { matched: item.matched(), item, unreleased })
    }

    pub async fn images(&self, id: &str, which: ImageKind) -> AppResult<Vec<ImageChoice>> {
        let c = self.guard(id).await?;
        self.note(id, c.images(id, which).await)
    }

    pub async fn matches(&self, id: &str, title: Option<String>, year: Option<i64>) -> AppResult<Vec<MatchCandidate>> {
        let c = self.guard(id).await?;
        self.note(id, c.matches(id, title.as_deref(), year).await)
    }

    pub async fn history(&self, id: &str) -> Vec<EditChange> {
        let log = self.inner.log.lock();
        log.iter().rev().filter(|l| l.change.item_id == id).map(|l| l.change.clone()).collect()
    }

    // ---- images

    /// What's in use now, and a copy of it, before an image change.
    async fn before_image(&self, c: &PlexClient, id: &str, which: ImageKind, change: &str) -> (Option<String>, Option<String>) {
        let previous = c.images(id, which).await.ok().and_then(|l| l.into_iter().find(|i| i.selected)).map(|i| i.key);
        let current = c.editable(id).await.ok().and_then(|it| match which {
            ImageKind::Poster => it.thumb,
            ImageKind::Art => it.art,
        });
        let mut saved = None;
        if let Some(path) = current
            && let Ok(bytes) = c.photo(&path, SAVED_PX, SAVED_PX).await
            && !bytes.is_empty()
        {
            let name = format!("{}-{change}.img", crate::sid::file_part(id));
            let dir = self.inner.dir.join(SAVED_DIR);
            if std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(&name), &bytes)).is_ok() {
                saved = Some(name);
            }
        }
        (previous, saved)
    }

    async fn image_change<F, Fut>(&self, id: &str, kind: EditKind, which: ImageKind, label: String, write: F) -> AppResult<EditChange>
    where
        F: FnOnce(PlexClient) -> Fut,
        Fut: std::future::Future<Output = AppResult<()>>,
    {
        let c = self.guard(id).await?;
        let change = self.next_id();
        let (previous, saved) = self.before_image(&c, id, which, &change).await;
        if let Err(e) = self.note(id, write(c).await) {
            if let Some(f) = &saved {
                let _ = std::fs::remove_file(self.inner.dir.join(SAVED_DIR).join(f));
            }
            return Err(e);
        }
        self.inner.hooks.changed(kind, &[id.to_string()], true, false).await;
        let undoable = previous.is_some() || saved.is_some();
        Ok(self.record(change, id, kind, EditAction::Image, label, undoable, Some(Undo::Image { which, previous, saved })))
    }

    /// Use one of the images Plex offers.
    pub async fn select_image(&self, id: &str, kind: EditKind, which: ImageKind, key: String) -> AppResult<EditChange> {
        let label = format!("Changed the {}", image_noun(kind, which));
        let id2 = id.to_string();
        self.image_change(id, kind, which, label, |c| async move { c.select_image(&id2, which, &key).await }).await
    }

    /// Upload an image file.
    pub async fn upload_image(&self, id: &str, kind: EditKind, which: ImageKind, bytes: Vec<u8>, content_type: String) -> AppResult<EditChange> {
        if bytes.is_empty() {
            return Err(AppError::Invalid("That image is empty.".into()));
        }
        if bytes.len() > IMAGE_MAX_BYTES {
            return Err(AppError::Invalid("Choose an image under 10 MB.".into()));
        }
        if !matches!(content_type.as_str(), "image/jpeg" | "image/png" | "image/webp") {
            return Err(AppError::Invalid("Choose a JPEG, PNG or WebP image.".into()));
        }
        let label = format!("Uploaded a new {}", image_noun(kind, which));
        let id2 = id.to_string();
        self.image_change(id, kind, which, label, |c| async move { c.upload_image(&id2, which, bytes, &content_type).await }).await
    }

    /// Have the server fetch an image from a link.
    pub async fn image_link(&self, id: &str, kind: EditKind, which: ImageKind, link: String) -> AppResult<EditChange> {
        let link = link.trim().to_string();
        let ok = url::Url::parse(&link).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
        if !ok {
            return Err(AppError::Invalid("Paste a link that starts with https://.".into()));
        }
        let label = format!("Set the {} from a link", image_noun(kind, which));
        let id2 = id.to_string();
        self.image_change(id, kind, which, label, |c| async move { c.image_from_link(&id2, which, &link).await }).await
    }

    // ---- matches

    pub async fn set_match(&self, id: &str, kind: EditKind, guid: String, name: String, year: Option<i64>) -> AppResult<EditChange> {
        let c = self.guard(id).await?;
        if kind == EditKind::Album && !self.inner.hooks.settings().match_unreleased && self.inner.hooks.unreleased(id).await {
            return Err(AppError::Invalid("Fix match is turned off for unreleased albums in Settings.".into()));
        }
        let before = self.note(id, c.editable(id).await)?;
        self.note(id, c.set_match(id, &guid, &name, year).await)?;
        self.inner.hooks.changed(kind, &[id.to_string()], true, false).await;
        let undo = Undo::Match {
            previous: before.matched().then(|| before.guid.clone()).flatten(),
            name: before.fields.title.clone(),
            year: year_of(&before.fields.released),
        };
        let label = format!("Matched to {name}");
        Ok(self.record(self.next_id(), id, kind, EditAction::Match, label, true, Some(undo)))
    }

    pub async fn unmatch(&self, id: &str, kind: EditKind) -> AppResult<EditChange> {
        let c = self.guard(id).await?;
        let before = self.note(id, c.editable(id).await)?;
        self.note(id, c.unmatch(id).await)?;
        self.inner.hooks.changed(kind, &[id.to_string()], true, false).await;
        let undo = before.matched().then(|| Undo::Match {
            previous: before.guid.clone(),
            name: before.fields.title.clone(),
            year: year_of(&before.fields.released),
        });
        Ok(self.record(self.next_id(), id, kind, EditAction::Unmatch, "Unmatched".into(), undo.is_some(), undo))
    }

    // ---- details

    pub async fn edit_details(&self, id: &str, to: Snapshot) -> AppResult<EditChange> {
        let c = self.guard(id).await?;
        let before = self.note(id, c.editable(id).await)?;
        let kind = before.kind.ok_or_else(|| AppError::Invalid("Only albums and artists can be edited here.".into()))?;
        let section = before.section_id.clone().ok_or_else(|| AppError::Internal("item has no library section".into()))?;
        let key = c.raw(id)?.to_string();
        let lock = self.inner.hooks.settings().lock;
        let changed = crate::plex::edit::changed_fields(kind, &before.fields, &to);
        if changed.is_empty() {
            return Err(AppError::Invalid("Nothing changed.".into()));
        }
        let q = field_query(kind, &key, &before.fields, &to, lock, None)?;
        self.note(id, c.edit_fields(&section, &q).await)?;
        self.inner.hooks.changed(kind, &[id.to_string()], false, false).await;
        let label = match changed.len() {
            1 => format!("Edited {}", field_label(changed[0])),
            n => format!("Edited {n} details"),
        };
        let undo = Undo::Details { section, before: Box::new(before.fields), after: Box::new(to), locks: before.locked };
        Ok(self.record(self.next_id(), id, kind, EditAction::Details, label, true, Some(undo)))
    }

    // ---- artists

    /// Merge `others` into `target`, which keeps its name. With
    /// `photo_from` naming one of the others, that artist's photo is put on
    /// the merged artist.
    pub async fn merge(&self, target: &str, others: Vec<String>, photo_from: Option<String>) -> AppResult<EditChange> {
        let c = self.guard(target).await?;
        for o in &others {
            if crate::sid::tag_of(o) != crate::sid::tag_of(target) {
                return Err(AppError::Invalid("Artists on different servers can't be merged.".into()));
            }
        }
        let keep_photo = match photo_from.as_deref().filter(|p| *p != target) {
            Some(p) if others.iter().any(|o| o == p) => {
                let thumb = self.note(target, c.editable(p).await)?.thumb;
                match thumb {
                    Some(t) => c.photo(&t, SAVED_PX, SAVED_PX).await.ok().filter(|b| !b.is_empty()),
                    None => None,
                }
            }
            Some(_) => return Err(AppError::Invalid("The photo has to come from one of the merged artists.".into())),
            None => None,
        };
        let name = self.note(target, c.editable(target).await)?.fields.title;
        self.note(target, c.merge(target, &others).await)?;
        if let Some(bytes) = keep_photo {
            let ct = crate::db::artcache::sniff_content_type(&bytes).to_string();
            if let Err(e) = c.upload_image(target, ImageKind::Poster, bytes.to_vec(), &ct).await {
                tracing::warn!("merge: couldn't move the photo over: {e}");
            }
        }
        let mut ids = vec![target.to_string()];
        ids.extend(others.iter().cloned());
        self.inner.hooks.changed(EditKind::Artist, &ids, true, true).await;
        let label = match others.len() {
            1 => format!("Merged another artist into {name}"),
            n => format!("Merged {n} artists into {name}"),
        };
        Ok(self.record(
            self.next_id(),
            target,
            EditKind::Artist,
            EditAction::Merge,
            label,
            true,
            Some(Undo::Merge { target: target.to_string() }),
        ))
    }

    /// Merge albums Plex shows apart into `target` (the library doctor's
    /// "split in two" fix). Undo splits them again.
    pub async fn merge_albums(&self, target: &str, others: Vec<String>) -> AppResult<EditChange> {
        let c = self.guard(target).await?;
        if others.is_empty() || others.iter().any(|o| o == target) {
            return Err(AppError::Invalid("Pick another album to merge with.".into()));
        }
        for o in &others {
            if crate::sid::tag_of(o) != crate::sid::tag_of(target) {
                return Err(AppError::Invalid("Albums on different servers can't be merged.".into()));
            }
        }
        let name = self.note(target, c.editable(target).await)?.fields.title;
        self.note(target, c.merge(target, &others).await)?;
        let mut ids = vec![target.to_string()];
        ids.extend(others.iter().cloned());
        self.inner.hooks.changed(EditKind::Album, &ids, true, true).await;
        let label = match others.len() {
            1 => format!("Merged the two parts of {name}"),
            n => format!("Merged {} parts of {name}", n + 1),
        };
        Ok(self.record(
            self.next_id(),
            target,
            EditKind::Album,
            EditAction::Merge,
            label,
            true,
            Some(Undo::Merge { target: target.to_string() }),
        ))
    }

    pub async fn split(&self, id: &str) -> AppResult<EditChange> {
        let c = self.guard(id).await?;
        self.note(id, c.split(id).await)?;
        self.inner.hooks.changed(EditKind::Artist, &[id.to_string()], true, true).await;
        Ok(self.record(self.next_id(), id, EditKind::Artist, EditAction::Split, "Split".into(), false, None))
    }

    pub async fn refresh(&self, id: &str, kind: EditKind) -> AppResult<EditChange> {
        let c = self.guard(id).await?;
        self.note(id, c.refresh_item(id).await)?;
        self.inner.hooks.changed(kind, &[id.to_string()], true, false).await;
        Ok(self.record(self.next_id(), id, kind, EditAction::Refresh, "Refreshed details".into(), false, None))
    }

    // ---- undo

    pub async fn undo(&self, change_id: &str) -> AppResult<EditChange> {
        let entry = {
            let log = self.inner.log.lock();
            log.iter().find(|l| l.change.id == change_id).cloned()
        };
        let Some(entry) = entry else { return Err(AppError::NotFound("That change isn't in the list any more.".into())) };
        if entry.change.undone {
            return Err(AppError::Invalid("That change was already undone.".into()));
        }
        let Some(undo) = entry.undo.clone().filter(|_| entry.change.undoable) else {
            return Err(AppError::Invalid("That change can't be undone.".into()));
        };
        let id = entry.change.item_id.as_str();
        let kind = entry.change.kind;
        let c = self.guard(id).await?;
        match undo {
            Undo::Image { which, previous, saved } => {
                let listed = match &previous {
                    Some(p) => self.note(id, c.images(id, which).await)?.into_iter().any(|i| &i.key == p),
                    None => false,
                };
                if let (true, Some(p)) = (listed, &previous) {
                    self.note(id, c.select_image(id, which, p).await)?;
                } else if let Some(bytes) = saved.and_then(|f| std::fs::read(self.inner.dir.join(SAVED_DIR).join(f)).ok()) {
                    let ct = crate::db::artcache::sniff_content_type(&bytes).to_string();
                    self.note(id, c.upload_image(id, which, bytes, &ct).await)?;
                } else {
                    return Err(AppError::NotFound(format!("The old {} isn't available any more.", image_noun(kind, which))));
                }
                self.inner.hooks.changed(kind, &[id.to_string()], true, false).await;
            }
            Undo::Match { previous, name, year } => {
                match previous {
                    Some(g) => self.note(id, c.set_match(id, &g, &name, year).await)?,
                    None => self.note(id, c.unmatch(id).await)?,
                }
                self.inner.hooks.changed(kind, &[id.to_string()], true, false).await;
            }
            Undo::Details { section, before, after, locks } => {
                let key = c.raw(id)?.to_string();
                let q = field_query(kind, &key, &after, &before, false, Some(&locks))?;
                self.note(id, c.edit_fields(&section, &q).await)?;
                self.inner.hooks.changed(kind, &[id.to_string()], false, false).await;
            }
            Undo::Merge { target } => {
                self.note(id, c.split(&target).await)?;
                self.inner.hooks.changed(kind, &[target], true, true).await;
            }
        }
        let mut log = self.inner.log.lock();
        let mut out = entry.change.clone();
        if let Some(l) = log.iter_mut().find(|l| l.change.id == change_id) {
            l.change.undone = true;
            out = l.change.clone();
        }
        self.save(&log);
        Ok(out)
    }

    // ---- log

    fn next_id(&self) -> String {
        format!("{}-{}", now_ms(), self.inner.seq.fetch_add(1, Ordering::Relaxed))
    }

    #[allow(clippy::too_many_arguments)]
    fn record(&self, change_id: String, id: &str, kind: EditKind, action: EditAction, label: String, undoable: bool, undo: Option<Undo>) -> EditChange {
        let change = EditChange { id: change_id, item_id: id.to_string(), kind, action, label, at: now_ms(), undoable, undone: false };
        let mut log = self.inner.log.lock();
        log.push(Logged { change: change.clone(), undo });
        while log.len() > KEEP {
            let old = log.remove(0);
            if let Some(Undo::Image { saved: Some(f), .. }) = old.undo {
                let _ = std::fs::remove_file(self.inner.dir.join(SAVED_DIR).join(f));
            }
        }
        self.save(&log);
        change
    }

    fn save(&self, log: &[Logged]) {
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&self.inner.dir)?;
            let tmp = self.inner.dir.join(format!("{LOG_FILE}.tmp"));
            std::fs::write(&tmp, serde_json::to_vec(log).unwrap_or_default())?;
            std::fs::rename(&tmp, self.inner.dir.join(LOG_FILE))
        };
        if let Err(e) = write() {
            tracing::warn!("couldn't save the Plex edit log: {e}");
        }
    }
}

fn field_label(plex: &str) -> &'static str {
    match plex {
        "title" => "the title",
        "titleSort" => "the sort title",
        "studio" => "the record label",
        "summary" => "the summary",
        "originallyAvailableAt" => "the release date",
        "genre" => "the genres",
        "style" => "the styles",
        "mood" => "the moods",
        _ => "details",
    }
}

// ---------------------------------------------------------------- app wiring

/// The account's servers, through the Plex state.
pub struct PlexEditServers(pub crate::plex::PlexState);

#[async_trait]
impl EditServers for PlexEditServers {
    async fn client_for(&self, id: &str) -> AppResult<PlexClient> {
        self.0.client_for(id).await
    }

    async fn owned(&self) -> (Vec<String>, bool) {
        let servers = self.0.account_servers().await;
        let owned: Vec<&str> = servers.iter().filter(|s| s.owned).map(|s| s.machine_id.as_str()).collect();
        let connected = match self.0.machine_id().await {
            Some(m) => owned.contains(&m.as_str()),
            None => false,
        };
        (owned.iter().map(|m| crate::sid::tag(m)).collect(), connected)
    }
}

/// The library cache, the art cache and settings.
pub struct AppEditHooks {
    pub db: crate::db::DbHandle,
    pub art: crate::db::ArtCache,
}

#[async_trait]
impl EditHooks for AppEditHooks {
    async fn unreleased(&self, album_id: &str) -> bool {
        let id = album_id.to_string();
        self.db
            .read_async(move |c| {
                use rusqlite::OptionalExtension;
                let row = c
                    .query_row(
                        "SELECT al.format, al.subformats, coalesce(at.version, at.release), al.studio, al.matched, mc.found FROM albums al
                         LEFT JOIN album_tags at ON at.album_id = al.id
                         LEFT JOIN album_mb_check mc ON mc.album_id = al.id WHERE al.id = ?1",
                        [&id],
                        |r| {
                            Ok((
                                r.get::<_, Option<String>>(0)?,
                                r.get::<_, Option<String>>(1)?,
                                r.get::<_, Option<String>>(2)?,
                                r.get::<_, Option<String>>(3)?,
                                r.get::<_, Option<bool>>(4)?,
                                r.get::<_, Option<bool>>(5)?,
                            ))
                        },
                    )
                    .optional()?;
                Ok(row.map(|(format, subs, version, studio, matched, mb)| {
                    let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
                    crate::db::release::album_unreleased(&id, format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref(), mb)
                }))
            })
            .await
            .ok()
            .flatten()
            // Not in the cache: the cautious answer.
            .unwrap_or(true)
    }

    async fn changed(&self, kind: EditKind, ids: &[String], art: bool, resync: bool) {
        use crate::db::live::{Batch, ItemKindKey, LiveChange};
        if art {
            let (cache, list) = (self.art.clone(), ids.to_vec());
            let _ = tokio::task::spawn_blocking(move || {
                for id in list {
                    if let Err(e) = cache.forget_item(&id) {
                        tracing::debug!("art cache: {e}");
                    }
                }
            })
            .await;
        }
        let k = match kind {
            EditKind::Artist => ItemKindKey::Artist,
            EditKind::Album => ItemKindKey::Album,
        };
        let mut batch = Batch::default();
        for id in ids {
            batch.add(LiveChange::Upsert(k, id.clone()));
        }
        if let Some(source) = self.db.source()
            && let Err(e) = crate::db::live::apply(&self.db, source.as_ref(), batch).await
        {
            tracing::debug!("edit: cache update failed: {e}");
        }
        if resync && let Err(e) = self.db.start_sync(false) {
            tracing::debug!("edit: sync start failed: {e}");
        }
        // A fix from the library doctor (cover, match, merge): check again.
        crate::doctor::poke();
    }

    fn settings(&self) -> EditSettings {
        let s = crate::settings::load(&self.db);
        EditSettings { show: s.plex_edit_show, lock: s.plex_edit_lock, match_unreleased: s.plex_edit_match_unreleased }
    }
}
