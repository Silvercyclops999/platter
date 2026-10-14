//! Lazy fetch and caching of artist info, the per-source switches and keys.
//!
//! `info` answers from the cache at once (Plex data plus whatever sources
//! are cached and on). Missing or stale sources refresh in the background and
//! `artistinfo://changed` tells the page to ask again. A source that's offline
//! or busy is skipped silently and tried on a later page open. A rejected key
//! turns its source off until a new key is saved or the app restarts.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
use super::merge::{self, PlexArtist, Sources};
use super::wiki::Wiki;
use super::{ArtistInfo, AudioDbData, FanartData, Fetch, FetchError, InfoSource, LastfmData, WikiData, fold_name, is_mbid};
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::{AppError, AppResult};
use crate::plex::Token;
use crate::plex::keychain::TokenStore;

/// A well-known artist used to check a newly entered key.
const PROBE_MBID: &str = "a74b1b7f-71a5-4011-9441-d0b5e4122711";

/// On/off per source (from Settings). Cheap to clone.
#[derive(Clone)]
pub struct Switches(Arc<[AtomicBool; 4]>);

impl Default for Switches {
    /// Wikipedia on, the keyed sources off (the Settings defaults).
    fn default() -> Self {
        Self(Arc::new([AtomicBool::new(true), AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)]))
    }
}

fn slot(s: InfoSource) -> usize {
    match s {
        InfoSource::Wikipedia => 0,
        InfoSource::Lastfm => 1,
        InfoSource::Fanart => 2,
        InfoSource::Audiodb => 3,
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
}

pub trait InfoEvents: Send + Sync {
    fn info_changed(&self, id: &str);
}

impl<R: tauri::Runtime> InfoEvents for tauri::AppHandle<R> {
    fn info_changed(&self, id: &str) {
        use tauri::Emitter;
        if let Err(e) = self.emit("artistinfo://changed", serde_json::json!({ "id": id })) {
            tracing::warn!("emit artistinfo://changed: {e}");
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
}

struct Limiters {
    wiki: Limiter,
    lastfm: Limiter,
    fanart: Limiter,
    audiodb: Limiter,
}

impl Default for Limiters {
    fn default() -> Self {
        Self {
            wiki: Limiter::per_second(5.0),
            lastfm: Limiter::per_second(5.0),
            fanart: Limiter::per_second(2.0),
            // Paid TheAudioDB keys allow 100 a minute; stay well under.
            audiodb: Limiter::per_second(1.0),
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
    lang: String,
}

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
                lang: "en".into(),
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

    // ------------------------------------------------------------ keys

    fn key(&self, s: InfoSource) -> Option<Token> {
        if !s.needs_key() {
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
        }
    }

    /// Save (or with None, remove) a source's key. A new key is checked with
    /// one lookup of a well-known artist; a refusal shows on the Settings row.
    pub async fn set_key(&self, s: InfoSource, key: Option<String>) -> AppResult<KeyState> {
        if !s.needs_key() {
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
        }
    }

    // ------------------------------------------------------------ cache

    /// Forget every cached answer. Returns how many artists had some.
    pub async fn clear_cache(&self) -> AppResult<u64> {
        self.inner.db.write_async(|c| cache::clear(c)).await
    }

    fn snapshot(&self, c: &Connection, id: &str, now: i64, usable: &[InfoSource]) -> AppResult<Option<Snapshot>> {
        let Some(plex) = plex_artist(c, id)? else { return Ok(None) };
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
            }
        }
        let names = sources.wiki.as_ref().map(|w| w.members.clone()).unwrap_or_default();
        let library = library_ids(c, &names)?;
        Ok(Some(Snapshot { plex, sources, library, stale }))
    }

    fn usable_sources(&self) -> Vec<InfoSource> {
        InfoSource::ALL.into_iter().filter(|s| self.usable(*s)).collect()
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
        let me = self.clone();
        let key = id.to_owned();
        let snap = self
            .inner
            .db
            .read_async(move |c| me.snapshot(c, &key, now, &usable))
            .await?
            .ok_or_else(|| AppError::NotFound(format!("artist {id}")))?;
        let info = merge::merge(&snap.plex, &snap.sources, &snap.library);
        Ok((info, !snap.stale.is_empty()))
    }

    /// The artist's MusicBrainz id: Plex's (cached), else Wikidata's.
    async fn mbid(&self, id: &str, now: i64) -> Option<String> {
        let key = id.to_owned();
        let row = self.inner.db.read_async(move |c| cache::get(c, &key, MBID_ROW)).await.ok().flatten();
        if let Some(r) = &row
            && !r.stale(now)
        {
            return r.parse::<String>();
        }
        let lookup = self.inner.mbid.lock().clone();
        let fetched = match lookup {
            Some(l) => l.artist_mbid(id).await,
            None => Err(AppError::Invalid("no Plex lookup".into())),
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
            // Plex unreachable: use what was cached, stale or not.
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
        if !self.inner.in_flight.lock().insert(id.to_owned()) {
            return Ok(false);
        }
        let out = self.refresh_inner(id, now_s()).await;
        self.inner.in_flight.lock().remove(id);
        let changed = out?;
        if changed && let Some(ev) = self.inner.events.lock().clone() {
            ev.info_changed(id);
        }
        Ok(changed)
    }

    async fn refresh_inner(&self, id: &str, now: i64) -> AppResult<bool> {
        let usable = self.usable_sources();
        let me = self.clone();
        let key = id.to_owned();
        let Some(snap) = self.inner.db.read_async(move |c| me.snapshot(c, &key, now, &usable)).await? else {
            return Ok(false);
        };
        if snap.stale.is_empty() {
            return Ok(false);
        }
        let name = snap.plex.title.clone();
        let mut mbid = self.mbid(id, now).await;
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

        let mbid = mbid.as_deref();
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
                    (Some(lf), true) => Some(lf.lookup(mbid, &name).await),
                    _ => None,
                }
            },
            async {
                match (&fanart_key, want(InfoSource::Fanart)) {
                    (Some(k), true) => Some(Fanart { http, limiter: &l.fanart, key: k.expose() }.lookup(mbid).await),
                    _ => None,
                }
            },
            async {
                match (&audiodb_key, want(InfoSource::Audiodb)) {
                    (Some(k), true) => Some(AudioDb { http, limiter: &l.audiodb, key: k.expose() }.lookup(mbid, &name).await),
                    _ => None,
                }
            },
        );
        if let Some(r) = lastfm {
            let max_age = lf.as_ref().and_then(|lf| *lf.max_age.lock());
            changed |= self.record(id, InfoSource::Lastfm, r, max_age, now).await;
        }
        if let Some(r) = fanart {
            changed |= self.record(id, InfoSource::Fanart, r, None, now).await;
        }
        if let Some(r) = audiodb {
            changed |= self.record(id, InfoSource::Audiodb, r, None, now).await;
        }
        Ok(changed)
    }
}

/// [`MbidLookup`] backed by the Plex client (artist detail carries
/// `Guid: [{id: "mbid://…"}]`).
pub struct PlexMbid(pub crate::plex::PlexState);

#[async_trait]
impl MbidLookup for PlexMbid {
    async fn artist_mbid(&self, artist_id: &str) -> AppResult<Option<String>> {
        Ok(self.0.client().await?.metadata(artist_id).await?.mbid())
    }
}
