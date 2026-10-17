//! Lazy fetch and caching of artist info, the per-source switches and keys.
//!
//! `info` answers from the cache at once (Plex data plus whatever sources
//! are cached and on). Missing or stale sources refresh in the background and
//! `artistinfo://changed` tells the page to ask again. A source that's offline
//! or busy is skipped silently and tried on a later page open. A rejected key
//! turns its source off until a new key is saved or the app restarts.
//!
//! The same refresh fetches the global Popular list (ListenBrainz, else
//! Last.fm top tracks) when Popular ranks by it. Credited artists
//! (`credit:` ids) have no Plex entry: their name comes from the tracks that
//! credit them and their MusicBrainz id from a MusicBrainz search; Plex is
//! never asked about them.
//!
//! Albums get the same treatment for their MusicBrainz release (original
//! date, label, credits): `album_credits` answers from the cache at once and
//! `albumcredits://changed` follows a lookup. Unreleased albums are never
//! looked up.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use super::audiodb::AudioDb;
use super::cache::{self, MBID_ROW};
use super::fanart::Fanart;
use super::http::InfoHttp;
use super::lastfm::Lastfm;
use super::limit::Limiter;
use super::listenbrainz::{LbStats, ListenBrainz};
use super::mbdetails::{self, AlbumCredits, CreditsState, LibTrack, MbArtist, MbRelease};
use super::merge::{self, PlexArtist, Sources};
use super::musicbrainz::MusicBrainz;
use super::wiki::Wiki;
use super::{ArtistInfo, AudioDbData, FanartData, Fetch, FetchError, InfoSource, LastfmData, WikiData, fold_name, is_mbid};
use crate::db::popular::{self, GlobalTop};
use crate::db::queries::now_s;
use crate::db::{DbHandle, PopularSource};
use crate::error::{AppError, AppResult};
use crate::plex::Token;
use crate::plex::keychain::TokenStore;

/// A well-known artist used to check a newly entered key.
const PROBE_MBID: &str = "a74b1b7f-71a5-4011-9441-d0b5e4122711";
/// How long a Popular list (or "none found") is reused.
pub const POPULAR_TTL_S: i64 = 7 * 24 * 3600;

/// On/off per source (from Settings). Cheap to clone.
#[derive(Clone)]
pub struct Switches(Arc<[AtomicBool; 7]>);

impl Default for Switches {
    /// Wikipedia, ListenBrainz and MusicBrainz on, the keyed sources off
    /// (the Settings defaults).
    fn default() -> Self {
        Self(Arc::new([
            AtomicBool::new(true),
            AtomicBool::new(false),
            AtomicBool::new(false),
            AtomicBool::new(false),
            AtomicBool::new(true),
            AtomicBool::new(true),
            AtomicBool::new(true),
        ]))
    }
}

fn slot(s: InfoSource) -> usize {
    match s {
        InfoSource::Wikipedia => 0,
        InfoSource::Lastfm => 1,
        InfoSource::Fanart => 2,
        InfoSource::Audiodb => 3,
        InfoSource::Listenbrainz => 4,
        InfoSource::Musicbrainz => 5,
        InfoSource::ListenbrainzStats => 6,
    }
}

impl Switches {
    pub fn set(&self, s: InfoSource, on: bool) {
        self.0[slot(s)].store(on, Ordering::Relaxed);
    }

    pub fn get(&self, s: InfoSource) -> bool {
        self.0[slot(s)].load(Ordering::Relaxed)
    }
}

/// Reads an artist's MusicBrainz id from Plex (artist detail `Guid`).
#[async_trait]
pub trait MbidLookup: Send + Sync {
    async fn artist_mbid(&self, artist_id: &str) -> AppResult<Option<String>>;
    /// An album's MusicBrainz release id (album detail `Guid`).
    async fn album_mbid(&self, _album_id: &str) -> AppResult<Option<String>> {
        Ok(None)
    }
}

pub trait InfoEvents: Send + Sync {
    fn info_changed(&self, id: &str);
    /// A fetch of the global Popular list finished, found or not. The page
    /// stops waiting for it.
    fn popular_done(&self, _id: &str) {}
    /// An album's MusicBrainz credits were looked up (found or not).
    fn album_credits_changed(&self, _album_id: &str) {}
    /// The MusicBrainz release check changed an album's release status.
    fn release_status_changed(&self, _album_id: &str) {}
}

impl<R: tauri::Runtime> InfoEvents for tauri::AppHandle<R> {
    fn info_changed(&self, id: &str) {
        use tauri::Emitter;
        if let Err(e) = self.emit("artistinfo://changed", serde_json::json!({ "id": id })) {
            tracing::warn!("emit artistinfo://changed: {e}");
        }
    }

    fn popular_done(&self, id: &str) {
        use tauri::Emitter;
        if let Err(e) = self.emit("artistinfo://popular", serde_json::json!({ "id": id })) {
            tracing::warn!("emit artistinfo://popular: {e}");
        }
    }

    fn album_credits_changed(&self, album_id: &str) {
        use tauri::Emitter;
        if let Err(e) = self.emit("albumcredits://changed", serde_json::json!({ "id": album_id })) {
            tracing::warn!("emit albumcredits://changed: {e}");
        }
    }

    fn release_status_changed(&self, album_id: &str) {
        use tauri::Emitter;
        if let Err(e) = self.emit("releasestatus://changed", serde_json::json!({ "id": album_id })) {
            tracing::warn!("emit releasestatus://changed: {e}");
        }
    }
}

/// A key's state for its Settings row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyState {
    pub saved: bool,
    /// The service refused it this session.
    pub rejected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysStatus {
    pub lastfm: KeyState,
    pub fanart: KeyState,
    pub audiodb: KeyState,
    /// The optional ListenBrainz user token.
    pub listenbrainz: KeyState,
}

struct Limiters {
    wiki: Limiter,
    lastfm: Limiter,
    fanart: Limiter,
    audiodb: Limiter,
    listenbrainz: Limiter,
    /// Shared with the Discord cover search (one request a second in all).
    musicbrainz: Arc<Limiter>,
}

impl Default for Limiters {
    fn default() -> Self {
        Self {
            wiki: Limiter::per_second(5.0),
            lastfm: Limiter::per_second(5.0),
            fanart: Limiter::per_second(2.0),
            // Paid TheAudioDB keys allow 100 a minute; stay well under.
            audiodb: Limiter::per_second(1.0),
            listenbrainz: Limiter::per_second(1.0),
            // MusicBrainz allows one request a second.
            musicbrainz: Arc::new(Limiter::per_second(1.0)),
        }
    }
}

struct Inner {
    db: DbHandle,
    http: Arc<dyn InfoHttp>,
    keys: Arc<dyn TokenStore>,
    switches: Switches,
    mbid: Mutex<Option<Arc<dyn MbidLookup>>>,
    events: Mutex<Option<Arc<dyn InfoEvents>>>,
    rejected: Mutex<HashSet<InfoSource>>,
    /// Keys read from the keychain once per session.
    key_cache: Mutex<HashMap<InfoSource, Option<Token>>>,
    limits: Limiters,
    in_flight: Mutex<HashSet<String>>,
    /// Albums whose lookup failed (offline, busy), when: not asked again for a while.
    album_failed: Mutex<HashMap<String, i64>>,
    lang: String,
    prefetch: Mutex<Prefetch>,
}

/// The background Popular prefetch queue (see
/// [`ArtistInfoService::queue_popular`]).
struct Prefetch {
    queue: VecDeque<String>,
    queued: HashSet<String>,
    running: bool,
    /// Pause after each artist that needed a fetch.
    gap: Duration,
}

/// Pause between prefetched artists: three a minute.
pub const PREFETCH_GAP: Duration = Duration::from_secs(20);
/// Artists picked for one prefetch round.
pub const PREFETCH_MAX: usize = 60;

#[derive(Clone)]
pub struct ArtistInfoService {
    inner: Arc<Inner>,
}

/// What's cached for one artist.
struct Snapshot {
    plex: PlexArtist,
    sources: Sources,
    library: HashMap<String, String>,
    stale: Vec<InfoSource>,
    /// The Popular list is wanted and missing or stale.
    popular_stale: bool,
}

/// The artist a page id names: a Plex artist, or a credited artist built
/// from the tracks that credit the name.
fn subject(c: &Connection, id: &str) -> AppResult<Option<PlexArtist>> {
    if let Some(key) = popular::credit_key(id) {
        return Ok(popular::credited_display_name(c, key)?.map(|title| PlexArtist {
            id: id.to_owned(),
            title,
            summary: None,
            thumb: None,
            art: None,
        }));
    }
    plex_artist(c, id)
}

fn plex_artist(c: &Connection, id: &str) -> AppResult<Option<PlexArtist>> {
    Ok(c.query_row("SELECT id, title, summary, thumb, art FROM artists WHERE id = ?1", [id], |r| {
        Ok(PlexArtist { id: r.get(0)?, title: r.get(1)?, summary: r.get(2)?, thumb: r.get(3)?, art: r.get(4)? })
    })
    .optional()?)
}

/// Library artists named like the members, folded name -> id.
fn library_ids(c: &Connection, names: &[String]) -> AppResult<HashMap<String, String>> {
    let mut out = HashMap::new();
    let mut stmt = c.prepare_cached("SELECT id FROM artists WHERE title = ?1 COLLATE NOCASE LIMIT 1")?;
    for n in names {
        if let Some(id) = stmt.query_row([n], |r| r.get::<_, String>(0)).optional()? {
            out.insert(fold_name(n), id);
        }
    }
    Ok(out)
}

/// Library artists among names found in a bio, folded name -> id. One pass
/// over the artists (titles have no index to look each name up by).
fn library_names_in(c: &Connection, candidates: &[String]) -> AppResult<HashMap<String, String>> {
    if candidates.is_empty() {
        return Ok(HashMap::new());
    }
    let want: HashSet<String> = candidates.iter().map(|n| fold_name(n)).collect();
    let mut out = HashMap::new();
    let mut stmt = c.prepare_cached("SELECT id, title FROM artists")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, title) = row?;
        let key = fold_name(&title);
        if want.contains(&key) {
            out.entry(key).or_insert(id);
        }
    }
    Ok(out)
}

/// Artists worth a Popular list before their page opens: Home's shortcuts
/// and recently played (album artists of albums too), then the most played,
/// at most [`PREFETCH_MAX`], in that order.
pub fn prefetch_targets(c: &Connection, now: i64) -> AppResult<Vec<String>> {
    use crate::db::types::HomeItem;
    let home = crate::db::queries::home(c, now)?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let home_ids = home.shortcuts.iter().chain(&home.recently_played).filter_map(|i| match i {
        HomeItem::Artist { artist } => Some(artist.id.clone()),
        HomeItem::Album { album } => Some(album.artist_id.clone()),
        HomeItem::Playlist { .. } => None,
    });
    let top = crate::db::queries::top_artists(c, PREFETCH_MAX)?.into_iter().map(|a| a.id);
    for id in home_ids.chain(top) {
        if out.len() >= PREFETCH_MAX {
            break;
        }
        if !id.is_empty() && seen.insert(id.clone()) {
            out.push(id);
        }
    }
    Ok(out)
}

impl ArtistInfoService {
    pub fn new(db: DbHandle, http: Arc<dyn InfoHttp>, keys: Arc<dyn TokenStore>, switches: Switches) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                http,
                keys,
                switches,
                mbid: Mutex::new(None),
                events: Mutex::new(None),
                rejected: Mutex::new(HashSet::new()),
                key_cache: Mutex::new(HashMap::new()),
                limits: Limiters::default(),
                in_flight: Mutex::new(HashSet::new()),
                album_failed: Mutex::new(HashMap::new()),
                lang: "en".into(),
                prefetch: Mutex::new(Prefetch {
                    queue: VecDeque::new(),
                    queued: HashSet::new(),
                    running: false,
                    gap: PREFETCH_GAP,
                }),
            }),
        }
    }

    pub fn set_mbid_lookup(&self, m: Arc<dyn MbidLookup>) {
        *self.inner.mbid.lock() = Some(m);
    }

    pub fn set_events(&self, e: Arc<dyn InfoEvents>) {
        *self.inner.events.lock() = Some(e);
    }

    pub fn switches(&self) -> &Switches {
        &self.inner.switches
    }

    pub fn http(&self) -> Arc<dyn InfoHttp> {
        self.inner.http.clone()
    }

    /// The MusicBrainz pacing, for other lookups that ask MusicBrainz.
    pub fn musicbrainz_limiter(&self) -> Arc<Limiter> {
        self.inner.limits.musicbrainz.clone()
    }

    /// The library cache (the release status check keeps its answers there).
    pub fn db(&self) -> &DbHandle {
        &self.inner.db
    }

    pub(crate) fn events(&self) -> Option<Arc<dyn InfoEvents>> {
        self.inner.events.lock().clone()
    }

    // ------------------------------------------------------------ keys

    fn key(&self, s: InfoSource) -> Option<Token> {
        if !s.takes_key() {
            return None;
        }
        if let Some(k) = self.inner.key_cache.lock().get(&s) {
            return k.clone();
        }
        let k = self.inner.keys.get(s.key_account()).unwrap_or_else(|e| {
            tracing::warn!("reading the {} key: {e}", s.display());
            None
        });
        self.inner.key_cache.lock().insert(s, k.clone());
        k
    }

    pub fn is_rejected(&self, s: InfoSource) -> bool {
        self.inner.rejected.lock().contains(&s)
    }

    fn reject(&self, s: InfoSource) {
        tracing::info!("{} rejected the API key; off for this session", s.display());
        self.inner.rejected.lock().insert(s);
    }

    /// The source is on, has its key and the key wasn't refused.
    pub fn usable(&self, s: InfoSource) -> bool {
        self.inner.switches.get(s) && !self.is_rejected(s) && (!s.needs_key() || self.key(s).is_some())
    }

    fn key_state(&self, s: InfoSource) -> KeyState {
        KeyState { saved: self.key(s).is_some(), rejected: self.is_rejected(s) }
    }

    pub fn keys_status(&self) -> KeysStatus {
        KeysStatus {
            lastfm: self.key_state(InfoSource::Lastfm),
            fanart: self.key_state(InfoSource::Fanart),
            audiodb: self.key_state(InfoSource::Audiodb),
            listenbrainz: self.key_state(InfoSource::Listenbrainz),
        }
    }

    /// Save (or with None, remove) a source's key. A new key is checked with
    /// one lookup of a well-known artist; a refusal shows on the Settings row.
    pub async fn set_key(&self, s: InfoSource, key: Option<String>) -> AppResult<KeyState> {
        if !s.takes_key() {
            return Err(AppError::Invalid(format!("{} has no key", s.display())));
        }
        let keys = self.inner.keys.clone();
        let account = s.key_account();
        let token = match key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => {
                let t = Token::new(k).map_err(|_| AppError::Invalid("empty key".into()))?;
                let t2 = t.clone();
                tokio::task::spawn_blocking(move || keys.set(account, &t2))
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))??;
                Some(t)
            }
            None => {
                tokio::task::spawn_blocking(move || keys.delete(account))
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))??;
                None
            }
        };
        self.inner.key_cache.lock().insert(s, token.clone());
        self.inner.rejected.lock().remove(&s);
        if s == InfoSource::Listenbrainz || s == InfoSource::Lastfm {
            // Artists ListenBrainz wouldn't answer without a token (or that
            // had no Last.fm fallback) are asked again.
            let _ = self
                .inner
                .db
                .write_async(|c| {
                    c.execute("DELETE FROM artist_info WHERE source = ?1 AND data IS NULL", [popular::GLOBAL_ROW])?;
                    Ok(())
                })
                .await;
        }
        if let Some(t) = token
            && self.probe(s, t.expose()).await == Err(FetchError::KeyRejected)
        {
            self.reject(s);
        }
        Ok(self.key_state(s))
    }

    async fn probe(&self, s: InfoSource, key: &str) -> Result<(), FetchError> {
        let http = self.inner.http.as_ref();
        let l = &self.inner.limits;
        match s {
            InfoSource::Wikipedia => Ok(()),
            InfoSource::Lastfm => {
                let lf = Lastfm { http, limiter: &l.lastfm, key, lang: &self.inner.lang, max_age: Default::default() };
                lf.lookup(Some(PROBE_MBID), "").await.map(|_| ())
            }
            InfoSource::Fanart => Fanart { http, limiter: &l.fanart, key }.lookup(Some(PROBE_MBID)).await.map(|_| ()),
            InfoSource::Audiodb => AudioDb { http, limiter: &l.audiodb, key }.lookup(Some(PROBE_MBID), "").await.map(|_| ()),
            InfoSource::Listenbrainz => ListenBrainz { http, limiter: &l.listenbrainz, token: Some(key) }.validate().await,
            InfoSource::Musicbrainz | InfoSource::ListenbrainzStats => Ok(()),
        }
    }

    // ------------------------------------------------------------ cache

    /// Forget every cached answer. Returns how many artists had some.
    pub async fn clear_cache(&self) -> AppResult<u64> {
        self.inner.db.write_async(|c| cache::clear(c)).await
    }

    fn snapshot(&self, c: &Connection, id: &str, now: i64, usable: &[InfoSource], popular_wanted: bool) -> AppResult<Option<Snapshot>> {
        let Some(plex) = subject(c, id)? else { return Ok(None) };
        let popular_stale =
            popular_wanted && cache::get(c, id, popular::GLOBAL_ROW)?.as_ref().is_none_or(|r| r.stale(now));
        let mut sources = Sources::default();
        let mut stale = Vec::new();
        for &s in usable {
            let row = cache::get(c, id, s.as_str())?;
            if row.as_ref().is_none_or(|r| r.stale(now)) {
                stale.push(s);
            }
            let Some(row) = row else { continue };
            match s {
                InfoSource::Wikipedia => sources.wiki = row.parse::<WikiData>(),
                InfoSource::Lastfm => sources.lastfm = row.parse::<LastfmData>(),
                InfoSource::Fanart => sources.fanart = row.parse::<FanartData>(),
                InfoSource::Audiodb => sources.audiodb = row.parse::<AudioDbData>(),
                InfoSource::Musicbrainz => sources.musicbrainz = row.parse::<MbArtist>(),
                InfoSource::ListenbrainzStats => sources.lb_stats = row.parse::<LbStats>(),
                InfoSource::Listenbrainz => {}
            }
        }
        let names = sources.people();
        let mut library = library_ids(c, &names)?;
        if let Some((bio, _)) = merge::pick_bio(plex.summary.as_deref(), &sources) {
            library.extend(library_names_in(c, &merge::bio_candidates(&bio))?);
        }
        Ok(Some(Snapshot { plex, sources, library, stale, popular_stale }))
    }

    fn usable_sources(&self) -> Vec<InfoSource> {
        InfoSource::ALL.into_iter().filter(|s| self.usable(*s)).collect()
    }

    /// Popular ranks by global popularity and a source for it is usable.
    pub fn popular_wanted(&self) -> bool {
        self.inner.db.popular_source() == PopularSource::Global
            && (self.usable(InfoSource::Listenbrainz) || self.usable(InfoSource::Lastfm))
    }

    /// What the page shows now; refreshes missing or stale sources in the
    /// background.
    pub async fn info(&self, id: &str) -> AppResult<ArtistInfo> {
        let (info, stale) = self.cached(id, now_s()).await?;
        if stale {
            let me = self.clone();
            let id = id.to_owned();
            tokio::spawn(async move {
                if let Err(e) = me.refresh(&id).await {
                    tracing::debug!("artist info for {id}: {e}");
                }
            });
        }
        Ok(info)
    }

    /// The merged view from the cache, and whether any usable source is
    /// missing or stale.
    pub async fn cached(&self, id: &str, now: i64) -> AppResult<(ArtistInfo, bool)> {
        let usable = self.usable_sources();
        let wanted = self.popular_wanted();
        let me = self.clone();
        let key = id.to_owned();
        let snap = self
            .inner
            .db
            .read_async(move |c| me.snapshot(c, &key, now, &usable, wanted))
            .await?
            .ok_or_else(|| AppError::NotFound(format!("artist {id}")))?;
        let info = merge::merge(&snap.plex, &snap.sources, &snap.library);
        Ok((info, !snap.stale.is_empty() || snap.popular_stale))
    }

    /// [`Self::mbid`] for other modules (Lidarr's release picker).
    pub async fn artist_mbid(&self, id: &str, name: &str) -> Option<String> {
        self.mbid(id, name, crate::db::queries::now_s()).await
    }

    /// The artist's MusicBrainz id: Plex's (cached), else Wikidata's. For a
    /// credited artist, an exact MusicBrainz name match (cached).
    async fn mbid(&self, id: &str, name: &str, now: i64) -> Option<String> {
        let key = id.to_owned();
        let row = self.inner.db.read_async(move |c| cache::get(c, &key, MBID_ROW)).await.ok().flatten();
        if let Some(r) = &row
            && !r.stale(now)
        {
            return r.parse::<String>();
        }
        let fetched = if popular::credit_key(id).is_some() {
            let mb = MusicBrainz { http: self.inner.http.as_ref(), limiter: &self.inner.limits.musicbrainz };
            mb.artist_id(name).await.map_err(|e| AppError::Network(format!("{e:?}")))
        } else {
            let lookup = self.inner.mbid.lock().clone();
            match lookup {
                Some(l) => l.artist_mbid(id).await,
                None => Err(AppError::Invalid("no Plex lookup".into())),
            }
        };
        match fetched {
            Ok(m) => {
                let m = m.filter(|m| is_mbid(m));
                let (key, json) = (id.to_owned(), m.clone());
                let _ = self
                    .inner
                    .db
                    .write_async(move |c| cache::put(c, &key, MBID_ROW, json.as_ref(), now, cache::TTL_S))
                    .await;
                m
            }
            // Plex or MusicBrainz unreachable: use what was cached, stale or not.
            Err(_) => row.and_then(|r| r.parse::<String>()),
        }
    }

    /// Store one source's answer. Returns whether the shown data may change.
    async fn record<T: Serialize + Send + 'static>(&self, id: &str, s: InfoSource, r: Fetch<T>, max_age: Option<u64>, now: i64) -> bool {
        let found = match r {
            Ok(v) => v,
            Err(FetchError::KeyRejected) => {
                self.reject(s);
                return true;
            }
            Err(FetchError::Unavailable(e)) => {
                tracing::debug!("{} unavailable: {e}", s.display());
                return false;
            }
        };
        let ttl = cache::ttl_for(max_age, found.is_some());
        let key = id.to_owned();
        self.inner
            .db
            .write_async(move |c| cache::put(c, &key, s.as_str(), found.as_ref(), now, ttl))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("artist info cache write: {e}");
                false
            })
    }

    /// Fetch the usable sources that are missing or stale for an artist.
    /// One refresh per artist at a time. Returns whether anything changed
    /// (and tells the page when it did).
    pub async fn refresh(&self, id: &str) -> AppResult<bool> {
        self.refresh_with(id, false).await
    }

    /// [`Self::refresh`]; `popular_only` leaves the other sources alone (the
    /// background prefetch). A new Popular list is announced as soon as it
    /// is stored; the rest once everything is in.
    async fn refresh_with(&self, id: &str, popular_only: bool) -> AppResult<bool> {
        if !self.inner.in_flight.lock().insert(id.to_owned()) {
            return Ok(false);
        }
        let out = self.refresh_inner(id, now_s(), popular_only).await;
        self.inner.in_flight.lock().remove(id);
        let (popular_changed, other_changed) = out?;
        if other_changed && let Some(ev) = self.inner.events.lock().clone() {
            ev.info_changed(id);
        }
        Ok(popular_changed || other_changed)
    }

    /// Fetch the Popular list, tell the page, and say whether it changed.
    async fn popular_step(&self, id: &str, mbid: Option<&str>, name: &str, now: i64) -> bool {
        let changed = self.refresh_popular(id, mbid, name, now).await;
        if let Some(ev) = self.inner.events.lock().clone() {
            if changed {
                ev.info_changed(id);
            }
            ev.popular_done(id);
        }
        changed
    }

    /// Returns (Popular changed, anything else changed).
    async fn refresh_inner(&self, id: &str, now: i64, popular_only: bool) -> AppResult<(bool, bool)> {
        let usable = if popular_only { Vec::new() } else { self.usable_sources() };
        let wanted = self.popular_wanted();
        let me = self.clone();
        let key = id.to_owned();
        let Some(snap) = self.inner.db.read_async(move |c| me.snapshot(c, &key, now, &usable, wanted)).await? else {
            return Ok((false, false));
        };
        if snap.stale.is_empty() && !snap.popular_stale {
            return Ok((false, false));
        }
        let name = snap.plex.title.clone();
        let mut mbid = self.mbid(id, &name, now).await;
        // With an id in hand the Popular list goes first: the page is
        // waiting on it, and the other sources only add to About.
        let mut popular_changed = false;
        let popular_early = snap.popular_stale && mbid.is_some();
        if popular_early {
            popular_changed = self.popular_step(id, mbid.as_deref(), &name, now).await;
        }
        let http = self.inner.http.as_ref();
        let l = &self.inner.limits;
        let lang = self.inner.lang.as_str();
        let mut changed = false;

        if snap.stale.contains(&InfoSource::Wikipedia) {
            let w = Wiki { http, limiter: &l.wiki, lang }.lookup(mbid.as_deref(), &name).await;
            if mbid.is_none()
                && let Ok(Some(d)) = &w
            {
                mbid = d.mbid.clone();
            }
            changed |= self.record(id, InfoSource::Wikipedia, w, None, now).await;
        } else if mbid.is_none() {
            mbid = snap.sources.wiki.as_ref().and_then(|w| w.mbid.clone());
        }

        let want = |s: InfoSource| snap.stale.contains(&s);
        let lastfm_key = self.key(InfoSource::Lastfm);
        let fanart_key = self.key(InfoSource::Fanart);
        let audiodb_key = self.key(InfoSource::Audiodb);
        let lf = lastfm_key.as_ref().map(|k| Lastfm {
            http,
            limiter: &l.lastfm,
            key: k.expose(),
            lang,
            max_age: Default::default(),
        });
        let (lastfm, fanart, audiodb) = futures::join!(
            async {
                match (&lf, want(InfoSource::Lastfm)) {
                    (Some(lf), true) => Some(lf.lookup(mbid.as_deref(), &name).await),
                    _ => None,
                }
            },
            async {
                match (&fanart_key, want(InfoSource::Fanart)) {
                    (Some(k), true) => Some(Fanart { http, limiter: &l.fanart, key: k.expose() }.lookup(mbid.as_deref()).await),
                    _ => None,
                }
            },
            async {
                match (&audiodb_key, want(InfoSource::Audiodb)) {
                    (Some(k), true) => Some(AudioDb { http, limiter: &l.audiodb, key: k.expose() }.lookup(mbid.as_deref(), &name).await),
                    _ => None,
                }
            },
        );
        if let Some(r) = lastfm {
            if mbid.is_none()
                && let Ok(Some(d)) = &r
            {
                mbid = d.mbid.clone();
            }
            let max_age = lf.as_ref().and_then(|lf| *lf.max_age.lock());
            changed |= self.record(id, InfoSource::Lastfm, r, max_age, now).await;
        } else if mbid.is_none() {
            mbid = snap.sources.lastfm.as_ref().and_then(|d| d.mbid.clone());
        }
        if let Some(r) = fanart {
            changed |= self.record(id, InfoSource::Fanart, r, None, now).await;
        }
        if let Some(r) = audiodb {
            changed |= self.record(id, InfoSource::Audiodb, r, None, now).await;
        }
        // These two need the MusicBrainz id; without one there's nothing to ask.
        if want(InfoSource::Musicbrainz) {
            let r = match mbid.as_deref() {
                Some(m) => MusicBrainz { http, limiter: &l.musicbrainz }.artist(m).await,
                None => Ok(None),
            };
            changed |= self.record(id, InfoSource::Musicbrainz, r, None, now).await;
        }
        if want(InfoSource::ListenbrainzStats) {
            let token = self.key(InfoSource::Listenbrainz);
            let r = match mbid.as_deref() {
                Some(m) => {
                    let lb = ListenBrainz { http, limiter: &l.listenbrainz, token: token.as_ref().map(|t| t.expose()) };
                    lb.artist_stats(m).await
                }
                None => Ok(None),
            };
            if r == Err(FetchError::KeyRejected) {
                self.reject(InfoSource::Listenbrainz);
            } else {
                changed |= self.record(id, InfoSource::ListenbrainzStats, r, None, now).await;
            }
        }
        if snap.popular_stale && !popular_early {
            popular_changed = self.popular_step(id, mbid.as_deref(), &name, now).await;
        }
        Ok((popular_changed, changed))
    }

    // ------------------------------------------------------------ artists outside the library

    /// A bio (Wikipedia) and pictures (Fanart.tv, with a key) for an artist
    /// that isn't in the library, by MusicBrainz id. Cached like library
    /// artists' info, under `mb:<id>`. Sources that are off give nothing.
    pub async fn catalogue_bio(&self, mbid: &str, name: &str) -> Option<crate::discover::CatalogueBio> {
        use crate::discover::CatalogueBio;
        if !is_mbid(mbid) {
            return None;
        }
        let key = format!("mb:{}", mbid.to_ascii_lowercase());
        let now = now_s();
        let k = key.clone();
        let row = self.inner.db.read_async(move |c| cache::get(c, &k, "catalogue")).await.ok().flatten();
        if let Some(r) = &row
            && !r.stale(now)
        {
            return r.parse::<CatalogueBio>();
        }
        let http = self.inner.http.as_ref();
        let l = &self.inner.limits;
        let mut out = CatalogueBio::default();
        let mut failed = false;
        if self.usable(InfoSource::Wikipedia) {
            match (Wiki { http, limiter: &l.wiki, lang: self.inner.lang.as_str() }).lookup(Some(mbid), name).await {
                Ok(Some(w)) => {
                    out.bio = w.bio;
                    out.article_url = w.article_url;
                }
                Ok(None) => {}
                Err(_) => failed = true,
            }
        }
        if self.usable(InfoSource::Fanart)
            && let Some(k) = self.key(InfoSource::Fanart)
        {
            match (Fanart { http, limiter: &l.fanart, key: k.expose() }).lookup(Some(mbid)).await {
                Ok(Some(f)) => {
                    out.photo = f.thumb.as_deref().map(super::ext_art_path);
                    out.banner = f.background.as_deref().map(super::ext_art_path);
                }
                Ok(None) => {}
                Err(_) => failed = true,
            }
        }
        if failed {
            // Offline or busy: keep what was there, and ask again next time.
            return row.and_then(|r| r.parse::<CatalogueBio>()).or(Some(out));
        }
        let found = out != CatalogueBio::default();
        let stored = out.clone();
        let _ = self
            .inner
            .db
            .write_async(move |c| cache::put(c, &key, "catalogue", Some(&stored), now, cache::ttl_for(None, found)))
            .await;
        Some(out)
    }

    /// A photo of an artist outside the library, with where it's from:
    /// Wikidata's picture on Wikimedia Commons (found through the artist's
    /// MusicBrainz id), else TheAudioDB (the user's key, else its free
    /// tier). Cached like the other answers; None when neither has one.
    pub async fn catalogue_photo(&self, mbid: &str, name: &str) -> Option<crate::discover::CataloguePhoto> {
        use crate::discover::CataloguePhoto;
        if !is_mbid(mbid) {
            return None;
        }
        let key = format!("mb:{}", mbid.to_ascii_lowercase());
        let now = now_s();
        let k = key.clone();
        let row = self.inner.db.read_async(move |c| cache::get(c, &k, "cataloguePhoto")).await.ok().flatten();
        if let Some(r) = &row
            && !r.stale(now)
        {
            return r.parse::<CataloguePhoto>().filter(|p| p.photo.is_some());
        }
        let http = self.inner.http.as_ref();
        let l = &self.inner.limits;
        let mut out = CataloguePhoto::default();
        let mut failed = false;
        if self.usable(InfoSource::Wikipedia) {
            match (Wiki { http, limiter: &l.wiki, lang: self.inner.lang.as_str() }).image(mbid).await {
                Ok(Some(url)) => {
                    out.photo = Some(super::ext_art_path(&url));
                    out.credit = Some("Wikimedia Commons".into());
                }
                Ok(None) => {}
                Err(_) => failed = true,
            }
        }
        // The free tier needs no switch of its own: this only runs for the
        // music-you-don't-have features, which are off until turned on.
        if out.photo.is_none() && !self.is_rejected(InfoSource::Audiodb) {
            let found = match self.key(InfoSource::Audiodb) {
                Some(k) => AudioDb { http, limiter: &l.audiodb, key: k.expose() }.lookup(Some(mbid), name).await,
                None => super::audiodb::free_lookup(http, &l.audiodb, mbid).await,
            };
            match found {
                Ok(Some(d)) => {
                    out.photo = d.thumb.as_deref().map(super::ext_art_path);
                    out.credit = out.photo.as_ref().map(|_| "TheAudioDB".to_string());
                }
                Ok(None) => {}
                Err(_) => failed = true,
            }
        }
        if failed && out.photo.is_none() {
            // Offline or busy: keep what was there, and ask again next time.
            return row.and_then(|r| r.parse::<CataloguePhoto>()).filter(|p| p.photo.is_some());
        }
        let found = out.photo.is_some();
        let stored = out.clone();
        let _ = self
            .inner
            .db
            .write_async(move |c| cache::put(c, &key, "cataloguePhoto", Some(&stored), now, cache::ttl_for(None, found)))
            .await;
        found.then_some(out)
    }

    // ------------------------------------------------------------ albums

    /// An album's MusicBrainz credits from the cache, at once. A missing or
    /// stale answer is looked up in the background (`albumcredits://changed`
    /// follows).
    pub async fn album_credits(&self, album_id: &str) -> AppResult<AlbumCredits> {
        let (out, stale) = self.album_cached(album_id, now_s()).await?;
        if stale {
            let me = self.clone();
            let id = album_id.to_owned();
            tokio::spawn(async move {
                if let Err(e) = me.refresh_album(&id).await {
                    tracing::debug!("album credits for {id}: {e}");
                }
            });
        }
        Ok(out)
    }

    /// The answer from the cache, and whether a lookup is due.
    pub async fn album_cached(&self, album_id: &str, now: i64) -> AppResult<(AlbumCredits, bool)> {
        if !self.usable(InfoSource::Musicbrainz) {
            return Ok((AlbumCredits::empty(album_id, CreditsState::Off), false));
        }
        let key = album_key(album_id);
        let id = album_id.to_owned();
        let (subject, row, library) = self
            .inner
            .db
            .read_async(move |c| {
                let subject = album_subject(c, &id)?;
                let row = cache::get(c, &key, InfoSource::Musicbrainz.as_str())?;
                let names = row.as_ref().and_then(|r| r.parse::<MbRelease>()).map(|r| mbdetails::credit_names(&r)).unwrap_or_default();
                Ok((subject, row, library_ids(c, &names)?))
            })
            .await?;
        let Some(subject) = subject else { return Err(AppError::NotFound(format!("album {album_id}"))) };
        if subject.unreleased {
            return Ok((AlbumCredits::empty(album_id, CreditsState::None), false));
        }
        let Some(row) = row else {
            let failed = self.inner.album_failed.lock().get(album_id).is_some_and(|t| now - t < ALBUM_RETRY_S);
            return Ok(if failed {
                (AlbumCredits::empty(album_id, CreditsState::None), false)
            } else {
                (AlbumCredits::empty(album_id, CreditsState::Pending), true)
            });
        };
        let stale = row.stale(now) && !self.inner.album_failed.lock().get(album_id).is_some_and(|t| now - t < ALBUM_RETRY_S);
        let out = match row.parse::<MbRelease>() {
            Some(r) => mbdetails::album_credits(album_id, &r, &subject.tracks, &library),
            None => AlbumCredits::empty(album_id, CreditsState::None),
        };
        Ok((out, stale))
    }

    /// The album's release id from Plex, cached like an artist's.
    async fn album_mbid(&self, album_id: &str, now: i64) -> Option<String> {
        let key = album_key(album_id);
        let k2 = key.clone();
        let row = self.inner.db.read_async(move |c| cache::get(c, &k2, MBID_ROW)).await.ok().flatten();
        if let Some(r) = &row
            && !r.stale(now)
        {
            return r.parse::<String>();
        }
        let lookup = self.inner.mbid.lock().clone();
        match lookup {
            Some(l) => match l.album_mbid(album_id).await {
                Ok(m) => {
                    let m = m.filter(|m| is_mbid(m));
                    let json = m.clone();
                    let _ = self.inner.db.write_async(move |c| cache::put(c, &key, MBID_ROW, json.as_ref(), now, cache::TTL_S)).await;
                    m
                }
                Err(_) => row.and_then(|r| r.parse::<String>()),
            },
            None => None,
        }
    }

    /// Look up an album on MusicBrainz and store the answer. One lookup per
    /// album at a time. Returns whether the stored answer changed.
    pub async fn refresh_album(&self, album_id: &str) -> AppResult<bool> {
        let key = album_key(album_id);
        if !self.inner.in_flight.lock().insert(key.clone()) {
            return Ok(false);
        }
        let out = self.refresh_album_inner(album_id, now_s()).await;
        self.inner.in_flight.lock().remove(&key);
        if out.is_ok()
            && let Some(ev) = self.inner.events.lock().clone()
        {
            ev.album_credits_changed(album_id);
        }
        out
    }

    async fn refresh_album_inner(&self, album_id: &str, now: i64) -> AppResult<bool> {
        if !self.usable(InfoSource::Musicbrainz) {
            return Ok(false);
        }
        let id = album_id.to_owned();
        let Some(subject) = self.inner.db.read_async(move |c| album_subject(c, &id)).await? else { return Ok(false) };
        if subject.unreleased {
            return Ok(false);
        }
        let known = self.album_mbid(album_id, now).await;
        let mb = MusicBrainz { http: self.inner.http.as_ref(), limiter: &self.inner.limits.musicbrainz };
        let r = mb.album_release(&subject.artist, &subject.title, subject.tracks.len(), known.as_deref()).await;
        if let Err(e) = &r {
            tracing::debug!("MusicBrainz album lookup: {e:?}");
            self.inner.album_failed.lock().insert(album_id.to_owned(), now);
            return Ok(false);
        }
        self.inner.album_failed.lock().remove(album_id);
        Ok(self.record(&album_key(album_id), InfoSource::Musicbrainz, r, None, now).await)
    }

    /// Queue artists for a background Popular fetch. One worker takes them
    /// in order, skips those with a fresh list, and pauses [`PREFETCH_GAP`]
    /// after each one it asked about, so the sources see a few requests a
    /// minute at most. Artists already queued keep their place.
    pub fn queue_popular(&self, ids: impl IntoIterator<Item = String>) {
        let start = {
            let mut p = self.inner.prefetch.lock();
            for id in ids {
                if p.queued.insert(id.clone()) {
                    p.queue.push_back(id);
                }
            }
            let start = !p.running && !p.queue.is_empty();
            p.running |= start;
            start
        };
        if start {
            let me = self.clone();
            tokio::spawn(async move { me.run_prefetch().await });
        }
    }

    /// The pause after each prefetched artist (tests shorten it).
    pub fn set_prefetch_gap(&self, gap: Duration) {
        self.inner.prefetch.lock().gap = gap;
    }

    /// Whether the prefetch worker has nothing left to do.
    pub fn prefetch_idle(&self) -> bool {
        let p = self.inner.prefetch.lock();
        !p.running && p.queue.is_empty()
    }

    async fn run_prefetch(&self) {
        loop {
            let (id, gap) = {
                let mut p = self.inner.prefetch.lock();
                match p.queue.pop_front() {
                    Some(id) => {
                        p.queued.remove(&id);
                        (id, p.gap)
                    }
                    None => {
                        p.running = false;
                        return;
                    }
                }
            };
            match self.prefetch_one(&id).await {
                Ok(true) => tokio::time::sleep(gap).await,
                Ok(false) => {}
                Err(e) => tracing::debug!("popular prefetch for {id}: {e}"),
            }
        }
    }

    /// Fetch one artist's Popular list when it's missing or stale, leaving
    /// the other sources for the page. Returns whether it asked.
    pub async fn prefetch_one(&self, id: &str) -> AppResult<bool> {
        if !self.popular_wanted() {
            return Ok(false);
        }
        let now = now_s();
        let key = id.to_owned();
        let stale = self
            .inner
            .db
            .read_async(move |c| Ok(cache::get(c, &key, popular::GLOBAL_ROW)?.is_none_or(|r| r.stale(now))))
            .await?;
        if !stale {
            return Ok(false);
        }
        self.refresh_with(id, true).await?;
        Ok(true)
    }

    /// Prefetch Popular lists at startup and after each library sync: the
    /// artists you play most and those on Home's shortcuts and recent
    /// shelves. Runs for the life of the app.
    pub async fn watch_prefetch(self) {
        // Let startup work (the first sync, Home) go first.
        tokio::time::sleep(Duration::from_secs(30)).await;
        let mut seen: Option<Option<i64>> = None;
        loop {
            let synced = self
                .inner
                .db
                .read_async(|c| Ok(crate::db::store::sync_state(c)?.and_then(|s| s.last_sync_at)))
                .await
                .unwrap_or(None);
            if seen != Some(synced) {
                seen = Some(synced);
                match self.inner.db.read_async(|c| prefetch_targets(c, now_s())).await {
                    Ok(ids) => {
                        tracing::debug!(n = ids.len(), "popular prefetch queued");
                        self.queue_popular(ids);
                    }
                    Err(e) => tracing::debug!("popular prefetch targets: {e}"),
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }

    /// Fetch and store the global Popular list: ListenBrainz first, Last.fm
    /// top tracks when ListenBrainz has nothing. Offline or a bad answer
    /// stores nothing (Popular keeps your plays order and the next page open
    /// tries again). Returns whether the stored list changed.
    async fn refresh_popular(&self, id: &str, mbid: Option<&str>, name: &str, now: i64) -> bool {
        let http = self.inner.http.as_ref();
        let l = &self.inner.limits;
        let mut top: Fetch<GlobalTop> = Ok(None);
        let mut learned: Vec<String> = Vec::new();
        if self.usable(InfoSource::Listenbrainz)
            && let Some(m) = mbid
        {
            let token = self.key(InfoSource::Listenbrainz);
            let lb = ListenBrainz { http, limiter: &l.listenbrainz, token: token.as_ref().map(|t| t.expose()) };
            match lb.top(m).await {
                Ok(Some(found)) => {
                    learned = found.credit_names;
                    top = Ok(Some(found.top));
                }
                Ok(None) => {}
                Err(FetchError::KeyRejected) => self.reject(InfoSource::Listenbrainz),
                Err(e) => top = Err(e),
            }
        }
        if matches!(top, Ok(None))
            && self.usable(InfoSource::Lastfm)
            && let Some(k) = self.key(InfoSource::Lastfm)
        {
            let lf = Lastfm { http, limiter: &l.lastfm, key: k.expose(), lang: &self.inner.lang, max_age: Default::default() };
            match lf.top_tracks(mbid, name).await {
                Err(FetchError::KeyRejected) => self.reject(InfoSource::Lastfm),
                r => top = r,
            }
        }
        let found = match top {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("popular songs unavailable: {e:?}");
                return false;
            }
        };
        let key = id.to_owned();
        self.inner
            .db
            .write_async(move |c| {
                let names = learned.iter().map(String::as_str);
                if popular::learn_names(c, names)? > 0 {
                    tracing::debug!("learned artist names with separators");
                }
                let had = popular::global_top(c, &key)?.is_some();
                let changed = cache::put(c, &key, popular::GLOBAL_ROW, found.as_ref(), now, POPULAR_TTL_S)?;
                // A first "none found" changes nothing on the page.
                Ok(changed && (had || found.is_some()))
            })
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("popular cache write: {e}");
                false
            })
    }
}

/// [`MbidLookup`] backed by the Plex client (artist and album detail carry
/// `Guid: [{id: "mbid://…"}]`; an album's is a release id).
pub struct PlexMbid(pub crate::plex::PlexState);

#[async_trait]
impl MbidLookup for PlexMbid {
    async fn artist_mbid(&self, artist_id: &str) -> AppResult<Option<String>> {
        Ok(self.0.client_for(artist_id).await?.metadata(artist_id).await?.mbid())
    }

    async fn album_mbid(&self, album_id: &str) -> AppResult<Option<String>> {
        Ok(self.0.client_for(album_id).await?.metadata(album_id).await?.mbid())
    }
}

/// Cache rows for an album are keyed like this (the table is per "artist").
pub const ALBUM_PREFIX: &str = "album:";
/// How long a failed album lookup waits before the page may ask again.
pub const ALBUM_RETRY_S: i64 = 10 * 60;

pub fn album_key(album_id: &str) -> String {
    format!("{ALBUM_PREFIX}{album_id}")
}

/// What an album lookup needs from the library cache.
pub struct AlbumSubject {
    pub title: String,
    pub artist: String,
    pub unreleased: bool,
    pub tracks: Vec<LibTrack>,
}

pub fn album_subject(c: &Connection, id: &str) -> AppResult<Option<AlbumSubject>> {
    let row = c
        .query_row(
            "SELECT al.title, al.artist_title, al.format, al.subformats, coalesce(at.version, at.release), al.matched, al.studio, mc.found FROM albums al
             LEFT JOIN album_tags at ON at.album_id = al.id LEFT JOIN album_mb_check mc ON mc.album_id = al.id WHERE al.id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<bool>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<bool>>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((title, artist, format, subformats, version, matched, studio, mb)) = row else { return Ok(None) };
    let subformats: Vec<String> = subformats.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let unreleased =
        crate::db::release::album_unreleased(id, format.as_deref(), &subformats, matched, studio.as_deref(), version.as_deref(), mb);
    let mut stmt = c.prepare_cached(
        "SELECT id, title, disc_number, track_number FROM tracks WHERE album_id = ?1 ORDER BY disc_number, track_number",
    )?;
    let tracks = stmt
        .query_map([id], |r| Ok(LibTrack { id: r.get(0)?, title: r.get(1)?, disc: r.get(2)?, number: r.get(3)? }))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(AlbumSubject { title, artist, unreleased, tracks }))
}
