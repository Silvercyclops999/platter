//! Smart playlists: rule sets that fill a playlist by themselves.
//!
//! - [`rules`]   the rule set the editor shows
//! - [`plexuri`] rules to and from Plex's smart playlist query
//! - [`eval`]    rules evaluated on this computer (preview, Platter playlists)
//! - [`prompt`]  a typed description turned into rules and picks (AI provider)
//! - [`commands`] Tauri commands (docs/IPC.md "Smart playlists")
//!
//! Rules Plex can express are saved as a Plex smart playlist, so every Plex
//! app sees it. Rules it can't (file tags, plays on this computer, a limit
//! in hours, a daily or weekly update) make a Platter smart playlist kept
//! behind [`SmartStore`] (the sync store's `smartPlaylists` namespace, so it
//! travels in the sync file; the songs a daily or weekly one holds stay on
//! this computer) and filled here. When Plex refuses a rule set, it's saved as a
//! Platter smart playlist instead and the answer says why.

pub mod commands;
pub mod eval;
pub mod plexuri;
pub mod prompt;
pub mod rules;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::db::DbHandle;
use crate::db::source::PlaylistEntry;
use crate::db::store::{kv_get, kv_set};
use crate::sync::store::{Op as SyncOp, SMART};
use crate::error::{AppError, AppResult};
use crate::model::{Playlist, Track};
use crate::plex::state::EventSink;
use plexuri::TagIds;
use rules::{SmartRules, Update};

pub const EVENT_SMART: &str = "smart://changed";
pub const ID_PREFIX: &str = "smart:";
pub const MAX_SMART: usize = 500;
/// Songs a preview lists.
pub const PREVIEW_ROWS: usize = 50;
/// The songs a daily or weekly playlist holds, per playlist (this computer).
const KEY_SNAPSHOT: &str = "smart.snapshot.";

/// A Platter smart playlist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartPlaylist {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub summary: Option<String>,
    pub rules: SmartRules,
    /// Songs picked for a prompt, first in the playlist, best first.
    #[serde(default)]
    pub picks: Vec<String>,
    /// The prompt it was made from.
    #[serde(default)]
    pub prompt: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Songs held until the next daily or weekly update (kept on this
    /// computer, not synced).
    #[serde(skip)]
    pub snapshot: Option<Vec<String>>,
    #[serde(skip)]
    pub snapshot_at: Option<i64>,
}

/// Where Platter smart playlists live.
pub trait SmartStore: Send + Sync {
    fn list(&self) -> AppResult<Vec<SmartPlaylist>>;
    fn save(&self, all: &[SmartPlaylist]) -> AppResult<()>;
}

/// Platter smart playlists in the sync store, their held songs in this
/// computer's database.
pub struct SyncSmart {
    pub store: crate::sync::SyncStore,
    pub db: DbHandle,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    ids: Vec<String>,
    at: i64,
}

impl SmartStore for SyncSmart {
    fn list(&self) -> AppResult<Vec<SmartPlaylist>> {
        let mut all: Vec<SmartPlaylist> = self.store.list(&SMART)?.into_iter().map(|(_, p)| p).collect();
        all.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
        for p in &mut all {
            let key = format!("{KEY_SNAPSHOT}{}", p.id);
            if let Some(s) = self.db.read(|c| kv_get(c, &key))?.and_then(|raw| serde_json::from_str::<Snapshot>(&raw).ok()) {
                p.snapshot = Some(crate::sid::resolve_all(&s.ids));
                p.snapshot_at = Some(s.at);
            }
        }
        Ok(all)
    }

    fn save(&self, all: &[SmartPlaylist]) -> AppResult<()> {
        let cur = self.list()?;
        let def = |p: &SmartPlaylist| serde_json::to_value(p).unwrap_or_default();
        let mut ops = Vec::new();
        for p in all {
            if !cur.iter().any(|c| c.id == p.id && def(c) == def(p)) {
                ops.push(SyncOp::put(&SMART, &p.id, p)?);
            }
        }
        for c in &cur {
            if !all.iter().any(|p| p.id == c.id) {
                ops.push(SyncOp::remove(&SMART, &c.id));
            }
        }
        self.store.write_many(ops)?;
        self.db.write(|c| {
            for p in all {
                let key = format!("{KEY_SNAPSHOT}{}", p.id);
                let v = match (&p.snapshot, p.snapshot_at) {
                    (Some(ids), Some(at)) => serde_json::to_string(&Snapshot { ids: ids.clone(), at }).unwrap_or_default(),
                    _ => String::new(),
                };
                kv_set(c, &key, &v)?;
            }
            for g in cur.iter().filter(|c| !all.iter().any(|p| p.id == c.id)) {
                c.execute("DELETE FROM kv WHERE key = ?1", [format!("{KEY_SNAPSHOT}{}", g.id)])?;
            }
            Ok(())
        })
    }
}

/// Plex's side: reads, and the smart playlist writes.
#[async_trait]
pub trait SmartRemote: Send + Sync {
    async fn online(&self) -> bool;
    /// The server's machine id and the music section's id.
    async fn target(&self) -> AppResult<(String, String)>;
    /// Tag ids and names for a Plex tag field (`album.genre`).
    async fn tags(&self, section_id: &str, plex_field: &str) -> AppResult<Vec<(String, String)>>;
    async fn create(&self, title: &str, uri: &str) -> AppResult<String>;
    async fn update(&self, id: &str, uri: &str) -> AppResult<()>;
    async fn rename(&self, id: &str, title: &str) -> AppResult<()>;
    /// A smart playlist's query (`content`).
    async fn content(&self, id: &str) -> AppResult<String>;
    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)>;
}

/// A playlist as the library and its page show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartSummary {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub track_count: u32,
    pub duration_ms: u64,
    pub update: Update,
    /// When a daily or weekly playlist changes next, unix seconds.
    pub next_update_at: Option<i64>,
    pub updated_at: i64,
    /// Made from a prompt.
    pub prompted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartPage {
    pub playlist: SmartSummary,
    pub rules: SmartRules,
    pub picks: Vec<String>,
    pub prompt: Option<String>,
    pub tracks: Vec<Track>,
}

/// `smart_preview`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub count: u32,
    pub duration_ms: u64,
    /// The first songs.
    pub tracks: Vec<Track>,
    /// Why these rules need a Platter smart playlist (empty: Plex can hold them).
    pub plex_gaps: Vec<String>,
}

/// `smart_save` payload.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRequest {
    /// Editing: a Platter (`smart:`) or Plex playlist id.
    #[serde(default)]
    pub id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub summary: Option<String>,
    pub rules: SmartRules,
    #[serde(default)]
    pub picks: Vec<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    /// Keep it on this computer even if Plex could hold it.
    #[serde(default)]
    pub platter: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub id: String,
    /// A Platter smart playlist (otherwise in Plex).
    pub platter: bool,
    /// Why it's a Platter one when Plex was tried.
    pub note: Option<String>,
}

pub struct SmartDeps {
    pub db: DbHandle,
    pub store: Arc<dyn SmartStore>,
    pub remote: Arc<dyn SmartRemote>,
    pub ai: Arc<prompt::Anthropic>,
    pub events: Arc<dyn EventSink>,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

/// Cheap to clone.
#[derive(Clone)]
pub struct Smart {
    inner: Arc<SmartDeps>,
    lock: Arc<tokio::sync::Mutex<()>>,
}

fn is_platter(id: &str) -> bool {
    id.starts_with(ID_PREFIX)
}

fn clean_title(t: &str) -> AppResult<String> {
    let t = t.trim();
    if t.is_empty() {
        return Err(AppError::Invalid("Playlist name is required.".into()));
    }
    if t.chars().count() > crate::db::playlists::TITLE_MAX {
        return Err(AppError::Invalid("Playlist names can be up to 100 characters.".into()));
    }
    Ok(t.to_string())
}

fn seed_of(id: &str, slot: i64) -> u64 {
    id.bytes().fold(slot as u64 ^ 0x9e37_79b9_7f4a_7c15, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Plex refused the rules (not: Plex couldn't be reached).
fn refused(e: &AppError) -> bool {
    matches!(e, AppError::Invalid(_) | AppError::Server(_) | AppError::NotFound(_) | AppError::Decode(_))
}

impl Smart {
    pub fn new(d: SmartDeps) -> Self {
        Self { inner: Arc::new(d), lock: Default::default() }
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    fn db(&self) -> &DbHandle {
        &self.inner.db
    }

    pub fn ai(&self) -> &prompt::Anthropic {
        &self.inner.ai
    }

    fn liked_min(&self) -> f64 {
        crate::settings::load(self.db()).liked_min_rating()
    }

    /// Tell the UI and the library (also after another device changed them).
    pub fn changed(&self) {
        let list = self.list().unwrap_or_default();
        self.inner.events.emit(EVENT_SMART, serde_json::to_value(&list).unwrap_or_default());
        if let Some(e) = self.db().events() {
            e.library_changed();
        }
    }

    async fn rows(&self) -> AppResult<Vec<eval::Row>> {
        self.db().read_async(eval::read_rows).await
    }

    async fn tracks(&self, ids: Vec<String>) -> AppResult<Vec<Track>> {
        self.db().read_async(move |c| crate::db::queries::tracks_by_ids(c, &ids)).await
    }

    /// The songs these rules (and picks) give, in order.
    fn songs(&self, rows: &[eval::Row], rules: &SmartRules, picks: &[String], seed: u64) -> Vec<(String, u64)> {
        let now = self.now();
        let hit = eval::evaluate(rows, rules, now, self.liked_min(), seed);
        let mut out: Vec<(String, u64)> = Vec::new();
        for p in picks {
            if let Some(r) = rows.iter().find(|r| r.id == *p) {
                out.push((r.id.clone(), r.duration_ms));
            }
        }
        for r in hit {
            if !picks.contains(&r.id) {
                out.push((r.id.clone(), r.duration_ms));
            }
        }
        if let Some(l) = rules.limit {
            match l.unit {
                rules::LimitUnit::Songs => out.truncate(l.value as usize),
                rules::LimitUnit::Hours => {
                    let cap = l.value as u64 * 3_600_000;
                    let mut total = 0;
                    out.retain(|(_, d)| {
                        let fits = total + d <= cap;
                        if fits {
                            total += d;
                        }
                        fits
                    });
                }
            }
        }
        out
    }

    /// How many songs the rules give now, and the first of them.
    pub async fn preview(&self, rules: SmartRules, picks: Vec<String>) -> AppResult<Preview> {
        rules.check()?;
        let rows = self.rows().await?;
        let songs = self.songs(&rows, &rules, &picks, 0);
        let first: Vec<String> = songs.iter().take(PREVIEW_ROWS).map(|(id, _)| id.clone()).collect();
        Ok(Preview {
            count: songs.len() as u32,
            duration_ms: songs.iter().map(|(_, d)| d).sum(),
            tracks: self.tracks(first).await?,
            plex_gaps: rules.plex_gaps(),
        })
    }

    // ---- Platter smart playlists -----------------------------------------------

    pub fn list(&self) -> AppResult<Vec<SmartSummary>> {
        let all = self.inner.store.list()?;
        Ok(all.iter().map(|p| self.summary(p, p.snapshot.as_ref().map(|s| s.len() as u32).unwrap_or(0), 0)).collect())
    }

    fn summary(&self, p: &SmartPlaylist, count: u32, duration_ms: u64) -> SmartSummary {
        let next = match (p.rules.update.period_s(), p.snapshot_at) {
            (Some(period), Some(at)) => Some(at + period),
            _ => None,
        };
        SmartSummary {
            id: p.id.clone(),
            title: p.title.clone(),
            summary: p.summary.clone(),
            track_count: count,
            duration_ms,
            update: p.rules.update,
            next_update_at: next,
            updated_at: p.updated_at,
            prompted: p.prompt.is_some(),
        }
    }

    /// A Platter smart playlist's page. Daily and weekly ones keep their
    /// songs until the next update is due; live ones are made fresh.
    pub async fn page(&self, id: &str) -> AppResult<SmartPage> {
        let _g = self.lock.lock().await;
        let mut all = self.inner.store.list()?;
        let i = all.iter().position(|p| p.id == id).ok_or_else(|| AppError::NotFound(format!("smart playlist {id}")))?;
        let now = self.now();
        let p = &all[i];
        let ids: Vec<String> = match (p.rules.update.period_s(), &p.snapshot, p.snapshot_at) {
            (Some(period), Some(snap), Some(at)) if now < at + period => snap.clone(),
            (period, _, _) => {
                let slot = now / period.unwrap_or(24 * 3600);
                let rows = self.rows().await?;
                let ids: Vec<String> = self.songs(&rows, &p.rules, &p.picks, seed_of(&p.id, slot)).into_iter().map(|(id, _)| id).collect();
                let p = &mut all[i];
                p.snapshot = Some(ids.clone());
                // Daily and weekly updates line up on whole periods.
                p.snapshot_at = Some(match period {
                    Some(period) => now - now.rem_euclid(period),
                    None => now,
                });
                self.inner.store.save(&all)?;
                ids
            }
        };
        let p = all[i].clone();
        let tracks = self.tracks(ids).await?;
        let summary = self.summary(&p, tracks.len() as u32, tracks.iter().map(|t| t.duration_ms).sum());
        Ok(SmartPage { playlist: summary, rules: p.rules, picks: p.picks, prompt: p.prompt, tracks })
    }

    pub async fn delete(&self, id: &str) -> AppResult<()> {
        if !is_platter(id) {
            return Err(AppError::Invalid("Delete Plex playlists from the playlist's menu.".into()));
        }
        {
            let _g = self.lock.lock().await;
            let mut all = self.inner.store.list()?;
            all.retain(|p| p.id != id);
            self.inner.store.save(&all)?;
        }
        self.changed();
        Ok(())
    }

    async fn save_platter(&self, req: &SaveRequest, title: String, note: Option<String>) -> AppResult<Saved> {
        let now = self.now();
        let id = {
            let _g = self.lock.lock().await;
            let mut all = self.inner.store.list()?;
            let summary = req.summary.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
            let id = match req.id.as_deref().filter(|id| is_platter(id)) {
                Some(id) => {
                    let p = all.iter_mut().find(|p| p.id == id).ok_or_else(|| AppError::NotFound(format!("smart playlist {id}")))?;
                    p.title = title;
                    p.summary = summary;
                    p.rules = req.rules.clone();
                    p.picks = req.picks.clone();
                    if req.prompt.is_some() {
                        p.prompt = req.prompt.clone();
                    }
                    p.updated_at = now;
                    p.snapshot = None;
                    p.snapshot_at = None;
                    id.to_string()
                }
                None => {
                    if all.len() >= MAX_SMART {
                        return Err(AppError::Invalid("You have as many Platter smart playlists as it can keep.".into()));
                    }
                    let id = format!("{ID_PREFIX}{}", uuid::Uuid::new_v4());
                    all.push(SmartPlaylist {
                        id: id.clone(),
                        title,
                        summary,
                        rules: req.rules.clone(),
                        picks: req.picks.clone(),
                        prompt: req.prompt.clone(),
                        created_at: now,
                        updated_at: now,
                        snapshot: None,
                        snapshot_at: None,
                    });
                    id
                }
            };
            self.inner.store.save(&all)?;
            id
        };
        self.changed();
        Ok(Saved { id, platter: true, note })
    }

    async fn tag_ids(&self, section: &str, fields: Vec<String>) -> AppResult<TagIds> {
        let mut out = TagIds::new();
        for f in fields {
            let list = self.inner.remote.tags(section, &f).await?;
            out.insert(f, list);
        }
        Ok(out)
    }

    /// The query for rules, with the server's tag ids.
    async fn plex_uri(&self, rules: &SmartRules) -> AppResult<String> {
        let (machine, section) = self.inner.remote.target().await?;
        let tags = self.tag_ids(&section, plexuri::tag_fields(rules)).await?;
        let q = plexuri::build_query(rules, self.liked_min(), &tags)?;
        Ok(plexuri::smart_uri(&machine, &section, &q))
    }

    /// Read a Plex smart playlist back into the cache.
    async fn resync(&self, id: &str) {
        match self.inner.remote.fetch(id).await {
            Ok((p, rows)) => {
                let _ = self.db().write_async(move |c| crate::db::playlists::apply_remote(c, &p, &rows)).await;
            }
            Err(e) => tracing::debug!("smart playlist not read back: {}", e.kind()),
        }
    }

    /// Save a smart playlist: in Plex when it can hold the rules, otherwise
    /// (or when asked, or with prompt picks) on this computer.
    pub async fn save(&self, req: SaveRequest) -> AppResult<Saved> {
        req.rules.check()?;
        let title = clean_title(&req.title)?;
        let gaps = req.rules.plex_gaps();
        match req.id.as_deref() {
            Some(id) if is_platter(id) => return self.save_platter(&req, title, None).await,
            Some(id) => {
                // A Plex smart playlist keeps living in Plex.
                if !gaps.is_empty() || !req.picks.is_empty() {
                    return Err(AppError::Invalid(format!(
                        "This playlist lives in Plex, and {}. Save these rules as a new Platter smart playlist instead.",
                        gaps.first().map(String::as_str).unwrap_or("Plex can't keep picked songs")
                    )));
                }
                if !self.inner.remote.online().await {
                    return Err(AppError::Network("Your server is offline. Plex smart playlists change when it's back.".into()));
                }
                let uri = self.plex_uri(&req.rules).await?;
                self.inner.remote.update(id, &uri).await?;
                self.inner.remote.rename(id, &title).await?;
                self.resync(id).await;
                self.changed();
                return Ok(Saved { id: id.to_string(), platter: false, note: None });
            }
            None => {}
        }
        if req.platter || !req.picks.is_empty() {
            return self.save_platter(&req, title, None).await;
        }
        if !gaps.is_empty() {
            let note = format!("Saved as a Platter smart playlist on this computer: {}.", gaps.join("; "));
            return self.save_platter(&req, title, Some(note)).await;
        }
        if !self.inner.remote.online().await {
            return Err(AppError::Network(
                "Your server is offline. Save it as a Platter smart playlist, or try again when the server is back.".into(),
            ));
        }
        let made = match self.plex_uri(&req.rules).await {
            Ok(uri) => self.inner.remote.create(&title, &uri).await,
            Err(e) => Err(e),
        };
        match made {
            Ok(id) => {
                self.resync(&id).await;
                self.changed();
                Ok(Saved { id, platter: false, note: None })
            }
            Err(e) if refused(&e) => {
                tracing::info!("Plex refused a smart playlist: {}", e.kind());
                let note = "Plex didn't accept these rules, so this is a Platter smart playlist on this computer.".to_string();
                self.save_platter(&req, title, Some(note)).await
            }
            Err(e) => Err(e),
        }
    }

    /// A Plex smart playlist's rules for the editor (an error when they
    /// don't map to it).
    pub async fn plex_rules(&self, id: &str) -> AppResult<SmartRules> {
        let content = self.inner.remote.content(id).await?;
        let parsed = plexuri::parse_content(&content)?;
        let section = match parsed.section_id.clone() {
            Some(s) => s,
            None => self.inner.remote.target().await?.1,
        };
        let tags = self.tag_ids(&section, plexuri::parsed_tag_fields(&parsed)).await?;
        plexuri::rules_from(&parsed, self.liked_min(), &tags)
    }

    /// A description turned into rules and picks (the AI provider in Settings).
    pub async fn prompt(&self, text: &str) -> AppResult<prompt::Prompted> {
        let s = crate::settings::load(self.db());
        if s.ai_provider != prompt::AiProvider::Anthropic || !self.inner.ai.has_key() {
            return Err(AppError::Invalid("Choose an AI provider and add its key in Settings first.".into()));
        }
        let rows = self.rows().await?;
        let model = if s.ai_model.trim().is_empty() { prompt::DEFAULT_MODEL.to_string() } else { s.ai_model.trim().to_string() };
        prompt::generate(&self.inner.ai, &model, text, &rows, self.now(), self.liked_min()).await
    }
}
