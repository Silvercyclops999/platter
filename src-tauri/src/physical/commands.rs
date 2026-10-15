//! The physical collection service and its Tauri commands (docs/IPC.md
//! "Physical collection").

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::discogs::DiscogsClient;
use super::{DiscogsReport, OwnedAlbum, SetOwned, ShelfStats};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::discover::CatAlbum;
use crate::error::{AppError, AppResult};
use crate::identity::fold::{fold_album, fold_artist};
use crate::plex::keychain::TokenStore;
use crate::plex::state::EventSink;
use crate::plex::token::Token;
use crate::sync::SyncStore;

pub const EV_CHANGED: &str = "physical://changed";
/// The keychain account the Discogs token is kept under.
pub const TOKEN_ACCOUNT: &str = "discogs-token";
const USER_KEY: &str = "physical.discogs.user";
const REPORT_KEY: &str = "physical.discogs.report.v1";

/// Where Discogs stands, for Settings and the import report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscogsStatus {
    /// A token is saved on this computer.
    pub saved: bool,
    /// Whose collection it reads.
    pub username: Option<String>,
    /// An import is going now.
    pub importing: bool,
    /// What the last import found.
    pub report: Option<DiscogsReport>,
}

struct Inner {
    db: DbHandle,
    store: SyncStore,
    keys: Arc<dyn TokenStore>,
    client: DiscogsClient,
    events: Arc<dyn EventSink>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    importing: AtomicBool,
    /// The token as read from the keychain once this run.
    token: Mutex<Option<Option<Token>>>,
}

/// Marks, stats and the Discogs import. Cheap to clone.
#[derive(Clone)]
pub struct Physical {
    inner: Arc<Inner>,
}

struct ImportGuard<'a>(&'a AtomicBool);
impl Drop for ImportGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn blocking_err(e: tokio::task::JoinError) -> AppError {
    AppError::Internal(e.to_string())
}

impl Physical {
    pub fn new(
        db: DbHandle,
        store: SyncStore,
        keys: Arc<dyn TokenStore>,
        client: DiscogsClient,
        events: Arc<dyn EventSink>,
        clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner { db, store, keys, client, events, clock, importing: AtomicBool::new(false), token: Mutex::new(None) }),
        }
    }

    fn emit(&self) {
        self.inner.events.emit(EV_CHANGED, serde_json::json!({}));
    }

    /// Every owned album (also ones the library no longer lists), by artist then title.
    pub async fn list(&self) -> AppResult<Vec<OwnedAlbum>> {
        let mut all: Vec<OwnedAlbum> = self.inner.store.run(super::list).await?.into_iter().map(|(k, o)| OwnedAlbum::new(k, o)).collect();
        all.sort_by(|a, b| (a.artist.to_lowercase(), a.title.to_lowercase(), &a.album_id).cmp(&(b.artist.to_lowercase(), b.title.to_lowercase(), &b.album_id)));
        Ok(all)
    }

    /// Mark an album as owned (or, with no copies, not owned).
    pub async fn set(&self, album_id: String, s: SetOwned) -> AppResult<Option<OwnedAlbum>> {
        let now = (self.inner.clock)();
        let id = album_id.clone();
        let o = self.inner.store.run(move |st| super::set(st, &id, s, now)).await?;
        self.emit();
        Ok(o.map(|o| OwnedAlbum::new(album_id.trim().to_string(), o)))
    }

    /// The shelf for the Stats page.
    pub async fn stats(&self) -> AppResult<ShelfStats> {
        let marks = self.inner.store.run(super::list).await?;
        let missing_on = crate::settings::load(&self.inner.db).shelf_missing_list;
        let lib = self.inner.db.read_async(super::library).await?;
        Ok(super::stats(&lib, &marks, missing_on))
    }

    // ------------------------------------------------------------ Discogs

    fn token(&self) -> Option<Token> {
        if let Some(t) = self.inner.token.lock().clone() {
            return t;
        }
        let t = self.inner.keys.get(TOKEN_ACCOUNT).unwrap_or_else(|e| {
            tracing::warn!("reading the Discogs token: {}", e.kind());
            None
        });
        *self.inner.token.lock() = Some(t.clone());
        t
    }

    pub async fn status(&self) -> AppResult<DiscogsStatus> {
        let me = self.clone();
        let saved = tokio::task::spawn_blocking(move || me.token().is_some()).await.map_err(blocking_err)?;
        let (username, report) = self
            .inner
            .db
            .read_async(|c| {
                let user = kv_get(c, USER_KEY)?.filter(|u| !u.is_empty());
                let report = kv_get(c, REPORT_KEY)?.and_then(|s| serde_json::from_str::<DiscogsReport>(&s).ok()).map(DiscogsReport::with_links);
                Ok((user, report))
            })
            .await?;
        Ok(DiscogsStatus {
            saved,
            username: if saved { username } else { None },
            importing: self.inner.importing.load(Ordering::SeqCst),
            report: if saved { report } else { None },
        })
    }

    /// Save the user's personal token after Discogs says whose it is, or
    /// with None, forget it (and the last import's report).
    pub async fn set_token(&self, token: Option<String>) -> AppResult<DiscogsStatus> {
        let keys = self.inner.keys.clone();
        match token.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => {
                if t.len() > 200 || !t.chars().all(|c| c.is_ascii_graphic()) {
                    return Err(AppError::Invalid(super::discogs::REFUSED.into()));
                }
                let tok = Token::new(t).map_err(|_| AppError::Invalid("No token to save.".into()))?;
                let user = self.inner.client.identity(&tok).await?;
                let t2 = tok.clone();
                tokio::task::spawn_blocking(move || keys.set(TOKEN_ACCOUNT, &t2)).await.map_err(blocking_err)??;
                *self.inner.token.lock() = Some(Some(tok));
                self.inner
                    .db
                    .write_async(move |c| {
                        if kv_get(c, USER_KEY)?.as_deref() != Some(user.as_str()) {
                            kv_set(c, REPORT_KEY, "")?;
                        }
                        kv_set(c, USER_KEY, &user)
                    })
                    .await?;
            }
            None => {
                tokio::task::spawn_blocking(move || keys.delete(TOKEN_ACCOUNT)).await.map_err(blocking_err)??;
                *self.inner.token.lock() = Some(None);
                self.inner
                    .db
                    .write_async(|c| {
                        kv_set(c, USER_KEY, "")?;
                        kv_set(c, REPORT_KEY, "")
                    })
                    .await?;
            }
        }
        self.emit();
        self.status().await
    }

    /// Read the Discogs collection and mark the albums the library has.
    pub async fn import(&self) -> AppResult<DiscogsReport> {
        let me = self.clone();
        let token = tokio::task::spawn_blocking(move || me.token())
            .await
            .map_err(blocking_err)?
            .ok_or_else(|| AppError::Invalid("Add your Discogs token in Settings first.".into()))?;
        if self.inner.importing.swap(true, Ordering::SeqCst) {
            return Err(AppError::Invalid("Your Discogs collection is already being read.".into()));
        }
        let _guard = ImportGuard(&self.inner.importing);
        self.emit();
        let out = self.import_with(&token).await;
        drop(_guard);
        self.emit();
        out
    }

    async fn import_with(&self, token: &Token) -> AppResult<DiscogsReport> {
        let user = match self.inner.db.read_async(|c| kv_get(c, USER_KEY)).await?.filter(|u| !u.is_empty()) {
            Some(u) => u,
            None => self.inner.client.identity(token).await?,
        };
        let items = self.inner.client.collection(token, &user).await?;
        let lib = self.inner.db.read_async(super::library).await?;
        if lib.is_empty() {
            return Err(AppError::Invalid("Your library is still loading. Try again once it's there.".into()));
        }
        let marks = self.inner.store.run(super::list).await?;
        let now = (self.inner.clock)();
        let plan = super::plan_import(&lib, &marks, &items, &user, now);
        let report = plan.report.clone();
        self.inner.store.run(move |st| super::apply_plan(st, &plan)).await?;
        let json = serde_json::to_string(&report).map_err(|e| AppError::Internal(e.to_string()))?;
        let u = user.clone();
        self.inner
            .db
            .write_async(move |c| {
                kv_set(c, REPORT_KEY, &json)?;
                kv_set(c, USER_KEY, &u)
            })
            .await?;
        tracing::info!(total = report.total, matched = report.matched, "Discogs collection read");
        Ok(report.with_links())
    }
}

/// The MusicBrainz album a shelf item is, from catalogue search results:
/// the same folded artist and title, else the same title.
pub fn pick_release(albums: Vec<CatAlbum>, artist: &str, title: &str) -> Option<CatAlbum> {
    let (a, t) = (fold_artist(&super::clean_discogs_artist(artist)), fold_album(title));
    if t.is_empty() {
        return None;
    }
    let same_title: Vec<CatAlbum> = albums.into_iter().filter(|x| fold_album(&x.title) == t).collect();
    let artist_too = same_title.iter().position(|x| fold_artist(&x.artist) == a);
    match artist_too {
        Some(i) => same_title.into_iter().nth(i),
        None if a.is_empty() => same_title.into_iter().next(),
        None => None,
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub async fn physical_list(physical: State<'_, Physical>) -> AppResult<Vec<OwnedAlbum>> {
    physical.list().await
}

/// Mark an album as owned with these copies; no copies takes the mark off.
/// `physical://changed` fires.
#[tauri::command]
pub async fn physical_set(physical: State<'_, Physical>, album_id: String, owned: SetOwned) -> AppResult<Option<OwnedAlbum>> {
    physical.set(album_id, owned).await
}

#[tauri::command]
pub async fn physical_stats(physical: State<'_, Physical>) -> AppResult<ShelfStats> {
    physical.stats().await
}

#[tauri::command]
pub async fn discogs_status(physical: State<'_, Physical>) -> AppResult<DiscogsStatus> {
    physical.status().await
}

/// Save the Discogs personal token in the keychain (checked with Discogs
/// first), or with null remove it.
#[tauri::command]
pub async fn discogs_token_set(physical: State<'_, Physical>, token: Option<String>) -> AppResult<DiscogsStatus> {
    physical.set_token(token).await
}

#[tauri::command]
pub async fn discogs_import(physical: State<'_, Physical>) -> AppResult<DiscogsReport> {
    physical.import().await
}

/// The MusicBrainz album a record on the shelf is, to request it. None when
/// MusicBrainz doesn't know it.
#[tauri::command]
pub async fn physical_find_release(app: AppHandle, artist: String, title: String) -> AppResult<Option<CatAlbum>> {
    let discover = app
        .try_state::<crate::discover::Discover>()
        .ok_or_else(|| AppError::Invalid("Search for music isn't available.".into()))?;
    let q = format!("{} {}", super::clean_discogs_artist(&artist), title.trim());
    let found = discover.search_albums(&q).await?;
    Ok(pick_release(found, &artist, &title))
}
