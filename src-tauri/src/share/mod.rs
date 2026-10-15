//! Sharing: public links for released music and `platter://` links for
//! friends on the same server, plus opening those links.
//!
//! Layout:
//! - [`matching`] close-match rules for store results (pure)
//! - [`itunes`]   the iTunes Search request and reply
//! - [`link`]     building and reading `platter://` links
//! - [`cache`]    the `share_links` table (30 days for a found link)
//! - [`commands`] Tauri commands and the deep link handler (docs/IPC.md "Sharing")
//!
//! A public link is found by searching the iTunes store for the song or
//! album (only its artist, title and album leave the computer) and taking
//! the song.link page for the matching store item. Unreleased music never
//! gets one, and a doubtful match gets none: the user can copy the Platter
//! link instead.

pub mod cache;
pub mod card;
pub mod commands;
pub mod itunes;
pub mod link;
pub mod matching;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::artistinfo::http::InfoHttp;
use crate::artistinfo::limit::Limiter;
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::db::release::is_unreleased;
use crate::error::{AppError, AppResult};
pub use link::ShareKind;
use matching::{AlbumQuery, SongQuery};

/// Apple asks for about 20 searches a minute at most.
pub const SEARCH_GAP: Duration = Duration::from_secs(3);

pub const MSG_UNRELEASED: &str = "Not released";
pub const MSG_OFF: &str = "Public links are off in Settings";
pub const MSG_NOT_IN_LIBRARY: &str = "This isn't in your library";
pub const MSG_UNREACHABLE: &str = "Couldn't reach the link search. Try again in a minute.";

/// The id of the server the library came from (for the link's server tag).
#[async_trait]
pub trait ServerId: Send + Sync {
    async fn machine_id(&self) -> Option<String>;
}

/// Tells the UI a link is waiting (and brings the window forward).
pub trait ShareEvents: Send + Sync {
    fn link_waiting(&self);
}

/// Whether the item can get a public link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicLink {
    /// Released music: a link can be looked up.
    Available,
    /// Unreleased (a demo): never.
    Unreleased,
    /// Public links are off in Settings.
    Off,
    /// Not offered for this kind (artists, playlists).
    None,
}

/// What the Share submenu needs (`share_info`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareInfo {
    /// The `platter://` link (with the start offset when one was asked for).
    pub app_link: String,
    pub public: PublicLink,
}

/// How opening a link went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeepLinkStatus {
    Open,
    /// Not on this library (another server, or removed).
    NotInLibrary,
    /// Not a link Platter can read.
    Invalid,
}

/// A link to open (`deep_link_take`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepLinkOpen {
    pub status: DeepLinkStatus,
    pub kind: Option<ShareKind>,
    pub id: Option<String>,
    /// For a song: its album (the page that opens).
    pub album_id: Option<String>,
    /// For a song: where to start when the user presses play.
    pub at_seconds: Option<u32>,
    /// A listening session invite (`platter://join/...`), to join.
    pub join: Option<String>,
}

impl DeepLinkOpen {
    fn failed(status: DeepLinkStatus) -> Self {
        Self { status, kind: None, id: None, album_id: None, at_seconds: None, join: None }
    }
}

/// A library item as sharing sees it.
#[derive(Debug, Clone, PartialEq)]
enum Item {
    Track { song: SongQuery, album_id: String, unreleased: bool },
    Album { album: AlbumQuery, unreleased: bool },
    Artist,
    Playlist,
}

fn json_vec(s: String) -> Vec<String> {
    serde_json::from_str(&s).unwrap_or_default()
}

fn load_item(c: &Connection, kind: ShareKind, id: &str) -> AppResult<Option<Item>> {
    Ok(match kind {
        ShareKind::Track => c
            .query_row(
                "SELECT t.title, t.album_id, t.album_title, t.artist_title, t.track_artist, t.duration_ms,
                        al.format, al.subformats, ft.version, al.studio, al.matched
                 FROM tracks t LEFT JOIN albums al ON al.id = t.album_id
                 LEFT JOIN file_tags ft ON ft.part_key = t.part_key WHERE t.id = ?1",
                params![id],
                |r| {
                    let album_artist: String = r.get(3)?;
                    let track_artist: Option<String> = r.get(4)?;
                    let format: Option<String> = r.get(6)?;
                    let subs = json_vec(r.get::<_, Option<String>>(7)?.unwrap_or_default());
                    let version: Option<String> = r.get(8)?;
                    let studio: Option<String> = r.get(9)?;
                    let matched: Option<bool> = r.get(10)?;
                    Ok(Item::Track {
                        song: SongQuery {
                            artist: track_artist.filter(|a| !a.trim().is_empty()).unwrap_or_else(|| album_artist.clone()),
                            album_artist,
                            title: r.get(0)?,
                            album: r.get(2)?,
                            duration_ms: r.get(5)?,
                        },
                        album_id: r.get(1)?,
                        unreleased: is_unreleased(format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref()),
                    })
                },
            )
            .optional()?,
        ShareKind::Album => c
            .query_row(
                "SELECT al.title, al.artist_title, al.track_count, al.format, al.subformats, at.version, al.studio, al.matched
                 FROM albums al LEFT JOIN album_tags at ON at.album_id = al.id WHERE al.id = ?1",
                params![id],
                |r| {
                    let format: Option<String> = r.get(3)?;
                    let subs = json_vec(r.get(4)?);
                    let version: Option<String> = r.get(5)?;
                    let studio: Option<String> = r.get(6)?;
                    let matched: Option<bool> = r.get(7)?;
                    Ok(Item::Album {
                        album: AlbumQuery { title: r.get(0)?, artist: r.get(1)?, track_count: r.get(2)? },
                        unreleased: is_unreleased(format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref()),
                    })
                },
            )
            .optional()?,
        ShareKind::Artist => c
            .query_row("SELECT 1 FROM artists WHERE id = ?1", params![id], |_| Ok(Item::Artist))
            .optional()?,
        ShareKind::Playlist => c
            .query_row("SELECT 1 FROM playlists WHERE id = ?1", params![id], |_| Ok(Item::Playlist))
            .optional()?,
    })
}

/// Cache key for a lookup: the kind and the folded fields that are sent or
/// matched on.
fn song_key(q: &SongQuery) -> String {
    use matching::{exact_key, title_key};
    format!(
        "song|{}|{}|{}|{}",
        title_key(&q.artist),
        title_key(&q.title),
        exact_key(&q.album),
        (q.duration_ms + 500) / 1000
    )
}

fn album_key(q: &AlbumQuery) -> String {
    use matching::{exact_key, title_key};
    format!("album|{}|{}|{}", title_key(&q.artist), exact_key(&q.title), q.track_count)
}

fn not_found(kind: ShareKind) -> AppError {
    AppError::NotFound(match kind {
        ShareKind::Album => "Couldn't find this album online".into(),
        _ => "Couldn't find this song online".into(),
    })
}

/// Public and app links, and opening `platter://` links. Cheap to clone.
#[derive(Clone)]
pub struct ShareService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DbHandle,
    http: Arc<dyn InfoHttp>,
    limiter: Limiter,
    /// Public links for released music (Settings).
    enabled: Arc<AtomicBool>,
    server: Arc<dyn ServerId>,
    events: Mutex<Option<Arc<dyn ShareEvents>>>,
    /// The last link opened, until the UI takes it.
    pending: Mutex<Option<DeepLinkOpen>>,
}

impl ShareService {
    pub fn new(db: DbHandle, http: Arc<dyn InfoHttp>, server: Arc<dyn ServerId>, enabled: Arc<AtomicBool>) -> Self {
        Self::with_gap(db, http, server, enabled, SEARCH_GAP)
    }

    pub fn with_gap(db: DbHandle, http: Arc<dyn InfoHttp>, server: Arc<dyn ServerId>, enabled: Arc<AtomicBool>, gap: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                http,
                limiter: Limiter::with_gap(gap),
                enabled,
                server,
                events: Mutex::new(None),
                pending: Mutex::new(None),
            }),
        }
    }

    pub fn set_events(&self, events: Arc<dyn ShareEvents>) {
        *self.inner.events.lock() = Some(events);
    }

    fn public_on(&self) -> bool {
        self.inner.enabled.load(Ordering::Relaxed)
    }

    async fn item(&self, kind: ShareKind, id: &str) -> AppResult<Option<Item>> {
        let id = id.to_string();
        self.inner.db.read_async(move |c| load_item(c, kind, &id)).await
    }

    fn status(&self, item: &Item) -> PublicLink {
        match item {
            Item::Track { unreleased: true, .. } | Item::Album { unreleased: true, .. } => PublicLink::Unreleased,
            Item::Track { .. } | Item::Album { .. } if !self.public_on() => PublicLink::Off,
            Item::Track { .. } | Item::Album { .. } => PublicLink::Available,
            Item::Artist | Item::Playlist => PublicLink::None,
        }
    }

    /// The app link and whether a public link can be had.
    pub async fn info(&self, kind: ShareKind, id: &str, at_seconds: Option<u32>) -> AppResult<ShareInfo> {
        let item = self.item(kind, id).await?.ok_or_else(|| AppError::NotFound(MSG_NOT_IN_LIBRARY.into()))?;
        let machine = self.inner.server.machine_id().await;
        Ok(ShareInfo { app_link: link::build(kind, id, at_seconds, machine.as_deref()), public: self.status(&item) })
    }

    /// Find the public link for a released song or album.
    pub async fn public_link(&self, kind: ShareKind, id: &str) -> AppResult<String> {
        let item = self.item(kind, id).await?.ok_or_else(|| AppError::NotFound(MSG_NOT_IN_LIBRARY.into()))?;
        match self.status(&item) {
            PublicLink::Available => {}
            PublicLink::Unreleased => return Err(AppError::Invalid(MSG_UNRELEASED.into())),
            PublicLink::Off => return Err(AppError::Invalid(MSG_OFF.into())),
            PublicLink::None => return Err(AppError::Invalid("Only songs and albums get public links".into())),
        }
        let (key, term, entity) = match &item {
            Item::Track { song, .. } => (song_key(song), matching::search_term(&song.artist, &song.title), itunes::Entity::Song),
            Item::Album { album, .. } => (album_key(album), matching::search_term(&album.artist, &album.title), itunes::Entity::Album),
            _ => unreachable!("status is Available only for songs and albums"),
        };
        let now = now_s();
        let k = key.clone();
        if let Some(row) = self.inner.db.read_async(move |c| cache::get(c, &k)).await?
            && row.expires_at > now
        {
            return row.url.ok_or_else(|| not_found(kind));
        }

        let resp = self
            .inner
            .limiter
            .get(self.inner.http.as_ref(), &itunes::search_url(&term, entity), &[])
            .await
            .map_err(|e| {
                tracing::debug!("link search failed: {e}");
                AppError::Network(MSG_UNREACHABLE.into())
            })?;
        if !resp.ok() {
            tracing::debug!(status = resp.status, "link search answered with an error");
            return Err(AppError::Server(MSG_UNREACHABLE.into()));
        }
        let found = match &item {
            Item::Track { song, .. } => matching::best_song(song, &itunes::parse_songs(&resp.body)?).map(itunes::song_page),
            Item::Album { album, .. } => matching::best_album(album, &itunes::parse_albums(&resp.body)?).map(itunes::album_page),
            _ => None,
        };
        let stored = found.clone();
        self.inner
            .db
            .write_async(move |c| {
                cache::prune(c, now)?;
                cache::put(c, &key, stored.as_deref(), now)
            })
            .await?;
        found.ok_or_else(|| not_found(kind))
    }

    /// Work out what a `platter://` link opens.
    pub async fn resolve(&self, raw: &str) -> DeepLinkOpen {
        if crate::social::listen::parse_invite(raw).is_some() {
            return DeepLinkOpen { join: Some(raw.trim().to_string()), ..DeepLinkOpen::failed(DeepLinkStatus::Open) };
        }
        let Some(l) = link::parse(raw) else {
            return DeepLinkOpen::failed(DeepLinkStatus::Invalid);
        };
        if let Some(tag) = &l.server
            && let Some(m) = self.inner.server.machine_id().await
            && link::server_tag(&m) != *tag
        {
            return DeepLinkOpen::failed(DeepLinkStatus::NotInLibrary);
        }
        let item = match self.item(l.kind, &l.id).await {
            Ok(Some(item)) => item,
            Ok(None) => return DeepLinkOpen::failed(DeepLinkStatus::NotInLibrary),
            Err(e) => {
                tracing::warn!("deep link lookup failed: {e}");
                return DeepLinkOpen::failed(DeepLinkStatus::NotInLibrary);
            }
        };
        let album_id = match item {
            Item::Track { album_id, .. } => Some(album_id),
            _ => None,
        };
        DeepLinkOpen { status: DeepLinkStatus::Open, kind: Some(l.kind), id: Some(l.id), album_id, at_seconds: l.t, join: None }
    }

    /// A link was opened: keep it for the UI and tell it.
    pub async fn open(&self, raw: &str) {
        let target = self.resolve(raw).await;
        tracing::info!(status = ?target.status, "link opened");
        *self.inner.pending.lock() = Some(target);
        if let Some(ev) = self.inner.events.lock().clone() {
            ev.link_waiting();
        }
    }

    /// The link waiting to be shown, once.
    pub fn take(&self) -> Option<DeepLinkOpen> {
        self.inner.pending.lock().take()
    }
}
