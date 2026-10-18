//! Putting an album's sleeve notes together, and the lazy lookups behind
//! them.
//!
//! `notes` answers from the library cache at once: the files' tags, Plex's
//! album facts, the song credits Plex gave so far, and the MusicBrainz and
//! Discogs releases cached on disk. When something is missing or stale it
//! starts one background refresh for the album (`looking` is true) and
//! `sleeve://changed` follows: Plex's song credits (the user's own server),
//! then with "Look up credits online" on, MusicBrainz (through the artist
//! info service, so its one-request-a-second pacing and cache are shared)
//! and Discogs when a token is saved. Unreleased albums are never looked up
//! online. A failed lookup waits [`RETRY_S`] before the next try, and
//! offline the notes show what's cached.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

use super::discogs::{self, DiscogsRelease};
use super::gather::{self, AlbumBasics, DISCOGS_ROW, PLEX_ROW, PlexCredit, track_key};
use super::{Lookups, Part, SleeveNotes, Source, merge, store};
use crate::artistinfo::service::{ArtistInfoService, album_key, album_subject};
use crate::artistinfo::{InfoSource, cache};
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::{AppError, AppResult};

/// How long a failed lookup waits before the page may ask again.
pub const RETRY_S: i64 = 10 * 60;
/// How long Plex's song credits are reused.
pub const PLEX_TTL_S: i64 = 7 * 24 * 3600;
/// Most songs whose Plex credits one refresh asks for.
pub const PLEX_MAX: usize = 60;

/// The two settings (Settings > Library). Cheap to clone.
#[derive(Clone)]
pub struct SleeveSwitches {
    /// Show sleeve notes.
    pub show: Arc<AtomicBool>,
    /// Look up credits online (and the app isn't offline).
    pub online: Arc<AtomicBool>,
}

impl Default for SleeveSwitches {
    fn default() -> Self {
        Self { show: Arc::new(AtomicBool::new(true)), online: Arc::new(AtomicBool::new(true)) }
    }
}

/// The user's Discogs access (the physical collection keeps the token).
#[async_trait]
pub trait DiscogsAccess: Send + Sync {
    async fn has_token(&self) -> bool;
    /// The Discogs release of an owned copy imported from Discogs.
    async fn owned_release(&self, album_id: &str) -> Option<u64>;
    /// A database search for releases by artist and title.
    async fn search(&self, artist: &str, title: &str) -> AppResult<Value>;
    /// One release; None when Discogs doesn't have it.
    async fn release(&self, id: u64) -> AppResult<Option<Value>>;
}

/// The credits Plex keeps on one song.
#[async_trait]
pub trait PlexCredits: Send + Sync {
    async fn track_credits(&self, track_id: &str) -> AppResult<Vec<PlexCredit>>;
}

pub trait SleeveEvents: Send + Sync {
    fn changed(&self, album_id: &str);
}

impl<R: tauri::Runtime> SleeveEvents for tauri::AppHandle<R> {
    fn changed(&self, album_id: &str) {
        use tauri::Emitter;
        if let Err(e) = self.emit("sleeve://changed", serde_json::json!({ "id": album_id })) {
            tracing::warn!("emit sleeve://changed: {e}");
        }
    }
}

struct Inner {
    db: DbHandle,
    info: ArtistInfoService,
    switches: SleeveSwitches,
    discogs: Mutex<Option<Arc<dyn DiscogsAccess>>>,
    plex: Mutex<Option<Arc<dyn PlexCredits>>>,
    events: Mutex<Option<Arc<dyn SleeveEvents>>>,
    in_flight: Mutex<HashSet<String>>,
    /// Failed lookups by (album, source), when: not asked again for a while.
    failed: Mutex<HashMap<(String, Source), i64>>,
    /// Albums indexed for credit pages this run (from cached releases).
    backfilled: Mutex<HashSet<String>>,
}

/// Sleeve notes and credit pages. Cheap to clone.
#[derive(Clone)]
pub struct SleeveService {
    inner: Arc<Inner>,
}

/// IPC `SongCredits`: one song's part of the notes, for the Credits dialog.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongCredits {
    pub track_id: String,
    pub album_id: String,
    pub looking: bool,
    pub lookups: Lookups,
    pub song: Option<super::SleeveTrack>,
    pub links: Vec<super::Link>,
}

/// What's due for an album's refresh.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Due {
    pub plex: bool,
    pub musicbrainz: bool,
    pub discogs: bool,
}

impl Due {
    pub fn any(self) -> bool {
        self.plex || self.musicbrainz || self.discogs
    }
}

impl SleeveService {
    pub fn new(db: DbHandle, info: ArtistInfoService, switches: SleeveSwitches) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                info,
                switches,
                discogs: Mutex::new(None),
                plex: Mutex::new(None),
                events: Mutex::new(None),
                in_flight: Mutex::new(HashSet::new()),
                failed: Mutex::new(HashMap::new()),
                backfilled: Mutex::new(HashSet::new()),
            }),
        }
    }

    pub fn set_discogs(&self, d: Arc<dyn DiscogsAccess>) {
        *self.inner.discogs.lock() = Some(d);
    }

    pub fn set_plex(&self, p: Arc<dyn PlexCredits>) {
        *self.inner.plex.lock() = Some(p);
    }

    pub fn set_events(&self, e: Arc<dyn SleeveEvents>) {
        *self.inner.events.lock() = Some(e);
    }

    pub fn switches(&self) -> &SleeveSwitches {
        &self.inner.switches
    }

    fn online(&self) -> bool {
        self.inner.switches.online.load(Ordering::Relaxed)
    }

    fn recently_failed(&self, album_id: &str, src: Source, now: i64) -> bool {
        self.inner.failed.lock().get(&(album_id.to_owned(), src)).is_some_and(|t| now - t < RETRY_S)
    }

    fn mark(&self, album_id: &str, src: Source, ok: bool, now: i64) {
        let mut f = self.inner.failed.lock();
        if ok {
            f.remove(&(album_id.to_owned(), src));
        } else {
            f.insert((album_id.to_owned(), src), now);
        }
    }

    async fn discogs_token(&self) -> bool {
        let d = self.inner.discogs.lock().clone();
        match d {
            Some(d) => d.has_token().await,
            None => false,
        }
    }

    /// An album's notes from the cache, at once. Starts a background refresh
    /// for what's missing or stale (`sleeve://changed` follows).
    pub async fn notes(&self, album_id: &str) -> AppResult<SleeveNotes> {
        let now = now_s();
        let (mut notes, due) = self.cached(album_id, now).await?;
        if due.any() && self.inner.switches.show.load(Ordering::Relaxed) {
            notes.looking = self.spawn_refresh(album_id, due);
        }
        Ok(notes)
    }

    /// The notes as the cache has them, and what a refresh would fetch. The
    /// credit index is brought up to date with them.
    pub async fn cached(&self, album_id: &str, now: i64) -> AppResult<(SleeveNotes, Due)> {
        let online = self.online();
        let mb_on = online && self.inner.info.usable(InfoSource::Musicbrainz);
        let discogs_on = online && self.discogs_token().await;
        let id = album_id.to_owned();
        let (notes, unreleased, mb_row, discogs_row, plex_due) = self
            .inner
            .db
            .read_async(move |c| {
                let Some(b) = gather::basics(c, &id)? else { return Err(AppError::NotFound(format!("album {id}"))) };
                let unreleased = album_subject(c, &id)?.is_some_and(|s| s.unreleased);
                let mb_row = cache::get(c, &album_key(&id), InfoSource::Musicbrainz.as_str())?;
                let discogs_row = cache::get(c, &album_key(&id), DISCOGS_ROW)?;
                let mut plex_due = false;
                for s in b.songs.iter().take(PLEX_MAX) {
                    plex_due |= gather::plex_cached(c, &s.id, now)?.1;
                }
                let notes = build(c, &id, &b, now)?;
                Ok((notes, unreleased, mb_row, discogs_row, plex_due))
            })
            .await?;
        let mut notes = notes;
        notes.lookups = Lookups { online, musicbrainz: mb_on, discogs: discogs_on, unreleased };
        let mb_due = mb_on
            && !unreleased
            && !self.recently_failed(album_id, Source::Musicbrainz, now)
            && mb_row.as_ref().is_none_or(|r| {
                r.stale(now) || r.parse::<crate::artistinfo::mbdetails::MbRelease>().is_some_and(|m| m.parsed < crate::artistinfo::mbdetails::PARSE_VERSION)
            });
        let discogs_due = discogs_on
            && !unreleased
            && !self.recently_failed(album_id, Source::Discogs, now)
            && discogs_row.as_ref().is_none_or(|r| r.stale(now));
        let due = Due {
            plex: plex_due && self.inner.plex.lock().is_some() && !self.recently_failed(album_id, Source::Plex, now),
            musicbrainz: mb_due,
            discogs: discogs_due,
        };
        let stored = notes.clone();
        self.inner.db.write_async(move |c| store::index_album(c, &stored)).await?;
        Ok((notes, due))
    }

    /// Start a refresh unless one is running for the album. Returns whether
    /// one is running now.
    fn spawn_refresh(&self, album_id: &str, due: Due) -> bool {
        if !self.inner.in_flight.lock().insert(album_id.to_owned()) {
            return true;
        }
        let me = self.clone();
        let id = album_id.to_owned();
        tokio::spawn(async move {
            me.refresh_with(&id, due).await;
            me.inner.in_flight.lock().remove(&id);
            // Index what was found, then tell the page.
            if let Err(e) = me.cached(&id, now_s()).await {
                tracing::debug!("sleeve notes for {id}: {e}");
            }
            if let Some(ev) = me.inner.events.lock().clone() {
                ev.changed(&id);
            }
        });
        true
    }

    /// Fetch what's due, one source after another. Failures are remembered
    /// and leave the cache as it was.
    pub async fn refresh_with(&self, album_id: &str, due: Due) {
        let now = now_s();
        if due.plex {
            let ok = self.refresh_plex(album_id, now).await;
            self.mark(album_id, Source::Plex, ok, now);
        }
        if due.musicbrainz {
            // The artist info service paces MusicBrainz (shared one a second)
            // and caches the release for the album page too.
            let ok = self.inner.info.refresh_album(album_id).await.is_ok() && self.mb_cached(album_id).await;
            self.mark(album_id, Source::Musicbrainz, ok, now);
        }
        if due.discogs {
            let ok = self.refresh_discogs(album_id, now).await;
            self.mark(album_id, Source::Discogs, ok, now);
        }
    }

    async fn mb_cached(&self, album_id: &str) -> bool {
        let id = album_id.to_owned();
        self.inner.db.read_async(move |c| Ok(cache::get(c, &album_key(&id), InfoSource::Musicbrainz.as_str())?.is_some())).await.unwrap_or(false)
    }

    async fn refresh_plex(&self, album_id: &str, now: i64) -> bool {
        let Some(plex) = self.inner.plex.lock().clone() else { return true };
        let id = album_id.to_owned();
        let songs: Vec<String> = match self
            .inner
            .db
            .read_async(move |c| {
                let Some(b) = gather::basics(c, &id)? else { return Ok(vec![]) };
                let mut due = Vec::new();
                for s in b.songs.iter().take(PLEX_MAX) {
                    if gather::plex_cached(c, &s.id, now)?.1 {
                        due.push(s.id.clone());
                    }
                }
                Ok(due)
            })
            .await
        {
            Ok(s) => s,
            Err(_) => return false,
        };
        for track in songs {
            match plex.track_credits(&track).await {
                Ok(credits) => {
                    let key = track_key(&track);
                    let _ = self.inner.db.write_async(move |c| cache::put(c, &key, PLEX_ROW, Some(&credits), now, PLEX_TTL_S)).await;
                }
                Err(e) => {
                    tracing::debug!(kind = e.kind(), "plex song credits unavailable");
                    return false;
                }
            }
        }
        true
    }

    async fn refresh_discogs(&self, album_id: &str, now: i64) -> bool {
        let Some(d) = self.inner.discogs.lock().clone() else { return true };
        let id = album_id.to_owned();
        let Ok(Some(b)) = self.inner.db.read_async(move |c| gather::basics(c, &id)).await else { return false };
        let release_id = match d.owned_release(album_id).await {
            Some(r) => Some(r),
            None => match d.search(&b.artist, &b.title).await {
                Ok(v) => discogs::pick_search(&v, &b.artist, &b.title),
                Err(e) => {
                    tracing::debug!(kind = e.kind(), "Discogs search unavailable");
                    return false;
                }
            },
        };
        let release: Option<DiscogsRelease> = match release_id {
            None => None,
            Some(rid) => match d.release(rid).await {
                Ok(v) => v.as_ref().and_then(discogs::parse_release),
                Err(e) => {
                    tracing::debug!(kind = e.kind(), "Discogs release unavailable");
                    return false;
                }
            },
        };
        let key = album_key(album_id);
        let found = release.is_some();
        self.inner
            .db
            .write_async(move |c| cache::put(c, &key, DISCOGS_ROW, release.as_ref(), now, cache::ttl_for(None, found)))
            .await
            .is_ok()
    }

    /// One song's credits (the album's notes, narrowed to the song).
    pub async fn song(&self, track_id: &str) -> AppResult<SongCredits> {
        let tid = track_id.to_owned();
        let album_id: String = self
            .inner
            .db
            .read_async(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row("SELECT album_id FROM tracks WHERE id = ?1", [&tid], |r| r.get(0))
                    .optional()?
                    .ok_or_else(|| AppError::NotFound(format!("track {tid}")))
            })
            .await?;
        let notes = self.notes(&album_id).await?;
        Ok(SongCredits {
            track_id: track_id.to_owned(),
            album_id,
            looking: notes.looking,
            lookups: notes.lookups,
            song: notes.tracks.into_iter().find(|t| t.track_id == track_id),
            links: notes.links,
        })
    }

    /// A person's credit page. Albums with a cached MusicBrainz release that
    /// were never shown are indexed first (from the cache, no lookups).
    pub async fn credit_page(&self, name: &str) -> AppResult<Option<store::CreditPage>> {
        let now = now_s();
        let pending: Vec<String> = self
            .inner
            .db
            .read_async(|c| {
                let mut st = c.prepare_cached(
                    "SELECT substr(ai.artist_id, 7) FROM artist_info ai
                     WHERE ai.artist_id LIKE 'album:%' AND ai.source IN ('musicbrainz', 'discogs') AND ai.data IS NOT NULL
                       AND NOT EXISTS (SELECT 1 FROM sleeve_credits sc WHERE sc.album_id = substr(ai.artist_id, 7))
                     LIMIT 200",
                )?;
                Ok(st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?)
            })
            .await?;
        for id in pending {
            if self.inner.backfilled.lock().insert(id.clone()) {
                let _ = self.cached(&id, now).await;
            }
        }
        let n = name.to_owned();
        self.inner.db.read_async(move |c| store::credit_page(c, &n)).await
    }
}

/// The notes from the cache: every source's part merged, with the files'
/// release status and how this edition differs from the others.
pub fn build(c: &rusqlite::Connection, album_id: &str, b: &AlbumBasics, now: i64) -> AppResult<SleeveNotes> {
    let mut parts: Vec<(Source, Part)> = vec![(Source::Tags, gather::tag_part(c, b)?), (Source::Plex, gather::plex_part(c, b, now)?)];
    if let Some(r) = gather::cached_mb(c, album_id)? {
        parts.push((Source::Musicbrainz, gather::mb_part(&r, &b.songs)));
    }
    if let Some(r) = cache::get(c, &album_key(album_id), DISCOGS_ROW)?.and_then(|r| r.parse::<DiscogsRelease>()) {
        parts.push((Source::Discogs, discogs::to_part(&r, &b.songs)));
    }
    let mut names: Vec<String> = Vec::new();
    for (_, p) in &parts {
        names.extend(p.credits.iter().map(|x| x.name.clone()));
        for t in p.tracks.values() {
            names.extend(t.credits.iter().map(|x| x.name.clone()));
        }
    }
    let library = gather::library_ids(c, &names)?;
    let mut notes = merge(album_id, &parts, &b.songs, &library);
    notes.status = b.status.map(|s| super::Fact { value: s.key().to_owned(), sources: vec![Source::Tags] });
    notes.edition_notes = gather::edition_notes(c, album_id, b)?;
    Ok(notes)
}
