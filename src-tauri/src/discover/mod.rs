//! Discover and get music that isn't in the library.
//!
//! - Search: below the library's results, artists, albums and songs from
//!   MusicBrainz. Anything the library has folds into it and is left out.
//! - Catalogue pages for an artist or album the library lacks: the
//!   discography of official release groups, the tracklist, store links,
//!   Request (through Lidarr's guarded flow) and Add to Missing.
//! - "Fans also like" with artists outside the library, and "New to you":
//!   artists like the ones played most, from ListenBrainz's similarity data
//!   and MusicBrainz relationships, minus the library and anyone the user
//!   said "Not interested" to.
//!
//! Every MusicBrainz request goes through the shared one-a-second limiter
//! and the week-long answer cache (`lidarr::mbpick::MbCached`); ListenBrainz
//! is paced and cached the same way ([`lb`]). Nothing is asked while
//! offline or while the setting for it is off. Only names, the search text
//! and MusicBrainz ids leave the computer.

pub mod commands;
pub mod lb;
pub mod library;
pub mod mb;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::artistinfo::is_mbid;
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::import::store::{StoreLinks, album_store_links, wanted_albums};
use crate::lidarr::ArtistMbid;
use crate::lidarr::mbpick::{Group, MbGet, Picker, Refusal, VARIOUS_ARTISTS, Want, group_url, library_albums, parse_group, vet};
use crate::lidarr::request::cover_thumb;
use crate::lidarr::status::RequestState;
use crate::radar::TopPlayed;
use lb::SimilarArtists;
use library::LibraryIndex;

/// Results per kind in search.
pub const SEARCH_SHOWN: usize = 6;
/// Artists in "Fans also like" from outside the library.
pub const FANS_MAX: usize = 12;
/// Artists played most that seed "New to you".
pub const SEEDS: usize = 8;
/// Similar artists read per seed.
pub const PER_SEED: usize = 25;
/// Most artists on "New to you".
pub const NEW_MAX: usize = 30;
/// How long a worked-out "New to you" list is reused.
pub const NEW_TTL: i64 = 12 * 3600;
const NEW_KEY: &str = "discover.newToYou.v1";
/// Most dismissed artists kept.
pub const DISMISSED_MAX: usize = 1000;

// ---------------------------------------------------------------- settings

/// An artist the user said "Not interested" to (synced).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Dismissed {
    pub mbid: String,
    pub name: String,
}

/// One entry per artist, real ids only, newest kept when the list is full.
pub fn tidy_dismissed(list: &[Dismissed]) -> Vec<Dismissed> {
    let mut out: Vec<Dismissed> = Vec::new();
    for d in list {
        let mbid = d.mbid.trim().to_ascii_lowercase();
        if is_mbid(&mbid) && !out.iter().any(|o| o.mbid == mbid) {
            out.push(Dismissed { mbid, name: d.name.trim().chars().take(200).collect() });
        }
    }
    if out.len() > DISMISSED_MAX {
        out.drain(..out.len() - DISMISSED_MAX);
    }
    out
}

// ---------------------------------------------------------------- what the UI reads

/// An artist from MusicBrainz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatArtist {
    pub mbid: String,
    pub name: String,
    pub disambiguation: Option<String>,
    /// "Person", "Group"...
    pub kind: Option<String>,
    /// Art path for a photo, when the artist info sources have one.
    pub photo: Option<String>,
    /// The library artist they are, if any.
    pub library_id: Option<String>,
}

/// A release group from MusicBrainz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatAlbum {
    pub release_group_id: String,
    pub title: String,
    pub artist: String,
    pub artist_mbid: Option<String>,
    /// `album`, `ep` or `single`.
    pub kind: String,
    /// `compilation`, `live`... (lower case).
    pub secondary: Vec<String>,
    pub year: Option<i32>,
    /// The Cover Art Archive's front cover, through the art cache.
    pub thumb: Option<String>,
    /// The library album it is, if any.
    pub library_album_id: Option<String>,
    /// A request made for it.
    pub request: Option<RequestState>,
    /// Why it can't be requested.
    pub refusal: Option<String>,
    /// On the Missing list.
    pub in_missing: bool,
}

/// A song from MusicBrainz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatSong {
    pub recording_id: String,
    pub title: String,
    pub artist: String,
    pub artist_mbid: Option<String>,
    /// The album it's on (official), which its row opens.
    pub album: Option<String>,
    pub release_group_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub thumb: Option<String>,
}

/// `catalogue_search`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueSearch {
    /// The setting is on and Platter is online.
    pub shown: bool,
    pub artists: Vec<CatArtist>,
    pub albums: Vec<CatAlbum>,
    pub songs: Vec<CatSong>,
}

/// One discography section on a catalogue artist page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatSection {
    /// `album`, `ep`, `single` or `other` (compilations, live albums...).
    pub kind: String,
    pub albums: Vec<CatAlbum>,
}

/// A bio and pictures for an artist outside the library.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CatalogueBio {
    pub bio: Option<String>,
    /// The Wikipedia article, for the credit under the bio.
    pub article_url: Option<String>,
    pub photo: Option<String>,
    pub banner: Option<String>,
}

/// `catalogue_artist`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueArtistPage {
    pub artist: CatArtist,
    pub area: Option<String>,
    pub life_span: Option<String>,
    pub bio: Option<String>,
    /// Where the bio is from ("Wikipedia"), with its link.
    pub bio_credit: Option<String>,
    pub bio_url: Option<String>,
    pub banner: Option<String>,
    pub sections: Vec<CatSection>,
    /// Artists like them; library ones carry their library id.
    pub similar: Vec<CatArtist>,
    pub dismissed: bool,
}

/// One track on a catalogue album page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatTrack {
    /// Set when the release has more than one disc.
    pub disc: Option<u32>,
    pub number: u32,
    pub title: String,
    /// When it differs from the album's credit.
    pub artist: Option<String>,
    pub duration_ms: Option<u64>,
}

/// `catalogue_album`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueAlbumPage {
    pub album: CatAlbum,
    pub tracks: Vec<CatTrack>,
    /// The release date of the release shown.
    pub released: Option<String>,
    pub label: Option<String>,
    /// Store searches (no previews from streaming stores).
    pub links: StoreLinks,
    /// The artist's library id, when the library has some of their music.
    pub artist_library_id: Option<String>,
}

/// One artist on "New to you".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewArtist {
    pub mbid: String,
    pub name: String,
    /// The artists played most that led here, closest first.
    pub because: Vec<String>,
}

/// `new_to_you`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewToYou {
    pub shown: bool,
    pub artists: Vec<NewArtist>,
}

// ---------------------------------------------------------------- what it needs

/// A bio and pictures for an artist outside the library (the artist info
/// sources).
#[async_trait]
pub trait CatalogueInfo: Send + Sync {
    async fn bio(&self, mbid: &str, name: &str) -> Option<CatalogueBio>;
}

#[async_trait]
impl CatalogueInfo for crate::artistinfo::ArtistInfoService {
    async fn bio(&self, mbid: &str, name: &str) -> Option<CatalogueBio> {
        self.catalogue_bio(mbid, name).await
    }
}

/// Lidarr requests made on this device, by release group.
pub trait RequestStates: Send + Sync {
    fn states(&self) -> HashMap<String, RequestState>;
}

impl RequestStates for crate::lidarr::Lidarr {
    fn states(&self) -> HashMap<String, RequestState> {
        self.requests().into_iter().map(|r| (r.release_group_id, r.state)).collect()
    }
}

/// The switches this reads (from the settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    pub search: bool,
    pub fans: bool,
    pub new_to_you: bool,
    pub dismissed: HashSet<String>,
}

pub trait PrefsSource: Send + Sync {
    fn prefs(&self) -> Prefs;
}

/// The saved settings.
pub struct SettingsPrefs(pub DbHandle);

impl PrefsSource for SettingsPrefs {
    fn prefs(&self) -> Prefs {
        let s = crate::settings::load(&self.0);
        Prefs {
            search: s.catalogue_search,
            fans: s.fans_also_like_catalogue,
            new_to_you: s.home_new_to_you,
            dismissed: s.dismissed_artists.into_iter().map(|d| d.mbid).collect(),
        }
    }
}

pub struct DiscoverDeps {
    pub db: DbHandle,
    /// MusicBrainz, paced and cached.
    pub mb: Arc<dyn MbGet>,
    pub similar: Arc<dyn SimilarArtists>,
    /// A library artist's MusicBrainz id.
    pub mbids: Arc<dyn ArtistMbid>,
    pub top: Arc<dyn TopPlayed>,
    pub info: Arc<dyn CatalogueInfo>,
    pub requests: Arc<dyn RequestStates>,
    pub prefs: Arc<dyn PrefsSource>,
    /// Go offline (and no connection): nothing is asked.
    pub offline: Arc<AtomicBool>,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

#[derive(Clone)]
pub struct Discover {
    inner: Arc<DiscoverDeps>,
    /// One "New to you" worked out at a time.
    new_lock: Arc<tokio::sync::Mutex<()>>,
}

/// A candidate on "New to you": name, score, and the seeds that led there with their points.
type Scored = (String, f64, Vec<(f64, String)>);

#[derive(Serialize, Deserialize)]
struct StoredNew {
    at: i64,
    artists: Vec<NewArtist>,
}

fn year_of(date: &str) -> Option<i32> {
    date.get(..4).and_then(|y| y.parse().ok())
}

fn section_of(g: &Group) -> &'static str {
    if !g.secondary.is_empty() {
        return "other";
    }
    match g.primary.as_deref() {
        Some("ep") => "ep",
        Some("single") => "single",
        _ => "album",
    }
}

impl Discover {
    pub fn new(deps: DiscoverDeps) -> Self {
        Self { inner: Arc::new(deps), new_lock: Arc::new(tokio::sync::Mutex::new(())) }
    }

    fn online(&self) -> bool {
        !self.inner.offline.load(Ordering::SeqCst)
    }

    fn picker(&self) -> Picker {
        Picker { mb: self.inner.mb.clone() }
    }

    async fn index(&self) -> AppResult<LibraryIndex> {
        self.inner.db.read_async(LibraryIndex::load).await
    }

    async fn missing_ids(&self) -> HashSet<String> {
        self.inner
            .db
            .read_async(wanted_albums)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|w| w.release_group_id)
            .collect()
    }

    fn album(&self, g: &Group, artist: String, artist_mbid: Option<String>, requests: &HashMap<String, RequestState>, missing: &HashSet<String>) -> CatAlbum {
        CatAlbum {
            thumb: cover_thumb(&g.id),
            request: requests.get(&g.id).copied(),
            in_missing: missing.contains(&g.id),
            release_group_id: g.id.clone(),
            title: g.title.clone(),
            artist,
            artist_mbid,
            kind: g.primary.clone().unwrap_or_else(|| "album".into()),
            secondary: g.secondary.clone(),
            year: year_of(&g.first_release),
            library_album_id: None,
            refusal: None,
        }
    }

    /// Artists, albums and songs from MusicBrainz that the library doesn't
    /// have, for the search page.
    pub async fn search(&self, query: &str) -> AppResult<CatalogueSearch> {
        let prefs = self.inner.prefs.prefs();
        if !prefs.search || !self.online() {
            return Ok(CatalogueSearch::default());
        }
        let q: String = query.trim().chars().take(200).collect();
        if q.chars().count() < 2 {
            return Ok(CatalogueSearch { shown: true, ..Default::default() });
        }
        let ix = self.index().await?;
        let requests = self.inner.requests.states();
        let missing = self.missing_ids().await;

        let mut artists = Vec::new();
        if let Some(v) = self.inner.mb.get(&mb::artist_search_url(&q)).await? {
            for a in mb::parse_artists(&v) {
                // An artist the library has is in the library results already.
                if ix.artist(Some(&a.mbid), &a.name).is_none() {
                    artists.push(a);
                }
                if artists.len() == SEARCH_SHOWN {
                    break;
                }
            }
        }

        let mut albums = Vec::new();
        if let Some(v) = self.inner.mb.get(&mb::group_search_url(&q)).await? {
            for (g, credit, first) in mb::parse_groups(&v) {
                let lib_artist = ix.artist(first.as_deref(), g.credit.first().map(|c| c.name.as_str()).unwrap_or(&credit));
                if let Some(aid) = lib_artist {
                    let (aid, title) = (aid.clone(), g.title.clone());
                    if self.inner.db.read_async(move |c| library::album_of(c, &aid, &title)).await?.is_some() {
                        continue;
                    }
                }
                albums.push(self.album(&g, credit, first, &requests, &missing));
                if albums.len() == SEARCH_SHOWN {
                    break;
                }
            }
        }

        let mut songs = Vec::new();
        if let Some(v) = self.inner.mb.get(&mb::recording_search_url(&q)).await? {
            for s in mb::parse_recordings(&v) {
                let (artist, title) = (s.artist.clone(), s.title.clone());
                let first = artist.split([',', '&']).next().unwrap_or(&artist).trim().to_string();
                let have = self
                    .inner
                    .db
                    .read_async(move |c| Ok(library::has_song(c, &artist, &title)? || library::has_song(c, &first, &title)?))
                    .await?;
                if !have {
                    songs.push(s);
                }
                if songs.len() == SEARCH_SHOWN {
                    break;
                }
            }
        }
        Ok(CatalogueSearch { shown: true, artists, albums, songs })
    }

    /// Albums from MusicBrainz for a search, whatever the library has (to
    /// find the release group of a record on the user's shelf).
    pub async fn search_albums(&self, query: &str) -> AppResult<Vec<CatAlbum>> {
        if !self.online() {
            return Err(AppError::Network("You're offline.".into()));
        }
        let q: String = query.trim().chars().take(200).collect();
        if q.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let requests = self.inner.requests.states();
        let missing = self.missing_ids().await;
        let Some(v) = self.inner.mb.get(&mb::group_search_url(&q)).await? else { return Ok(Vec::new()) };
        Ok(mb::parse_groups(&v).into_iter().map(|(g, credit, first)| self.album(&g, credit, first, &requests, &missing)).collect())
    }

    /// Similar artists from ListenBrainz with the library ones marked.
    async fn similar_of(&self, mbid: &str, ix: &LibraryIndex) -> Vec<CatArtist> {
        let list = match self.inner.similar.similar(mbid).await {
            Ok(l) => l,
            Err(e) => {
                tracing::debug!("similar artists for {mbid}: {e}");
                Vec::new()
            }
        };
        list.into_iter()
            .filter(|a| a.mbid != VARIOUS_ARTISTS)
            .map(|a| CatArtist {
                library_id: ix.artist(Some(&a.mbid), &a.name),
                mbid: a.mbid,
                name: a.name,
                disambiguation: None,
                kind: None,
                photo: None,
            })
            .collect()
    }

    /// A catalogue artist page. An artist the library has comes back with
    /// `library_id` set, for the page to open theirs instead.
    pub async fn artist_page(&self, mbid: &str) -> AppResult<CatalogueArtistPage> {
        let mbid = mbid.trim().to_ascii_lowercase();
        if !is_mbid(&mbid) {
            return Err(AppError::NotFound(format!("artist {mbid}")));
        }
        if !self.online() {
            return Err(AppError::Network("You're offline.".into()));
        }
        let v = self.inner.mb.get(&mb::artist_url(&mbid)).await?.ok_or_else(|| AppError::NotFound("MusicBrainz doesn't know this artist.".into()))?;
        let head = mb::parse_head(&v);
        let mut artist = head.artist.ok_or_else(|| AppError::NotFound("MusicBrainz doesn't know this artist.".into()))?;
        let ix = self.index().await?;
        artist.library_id = ix.artist(Some(&artist.mbid), &artist.name);
        let prefs = self.inner.prefs.prefs();

        let groups = self.picker().artist_groups(&mbid).await?;
        let want = Want { name: artist.name.clone(), mbid: Some(mbid.clone()), artist_id: artist.library_id.clone() };
        let (lib_id, name) = (artist.library_id.clone(), artist.name.clone());
        let lib = self.inner.db.read_async(move |c| library_albums(c, lib_id.as_deref(), &name)).await?;
        let requests = self.inner.requests.states();
        let missing = self.missing_ids().await;
        let mut by_kind: HashMap<&'static str, Vec<CatAlbum>> = HashMap::new();
        for g in groups {
            let mut a = self.album(&g, artist.name.clone(), Some(mbid.clone()), &requests, &missing);
            match vet(&g, &want, &lib, true) {
                Ok(()) => {}
                Err(Refusal::TitleClash { title, unreleased: false }) => {
                    a.library_album_id = lib.iter().find(|l| l.title == title).map(|l| l.id.clone());
                }
                Err(r @ Refusal::TitleClash { .. }) => a.refusal = Some(r.message()),
                // Someone else's release, a bootleg, Various Artists: not theirs to show.
                Err(_) => continue,
            }
            by_kind.entry(section_of(&g)).or_default().push(a);
        }
        let sections = ["album", "ep", "single", "other"]
            .into_iter()
            .filter_map(|k| {
                let mut albums = by_kind.remove(k)?;
                albums.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| a.title.cmp(&b.title)));
                Some(CatSection { kind: k.into(), albums })
            })
            .collect();

        let bio = self.inner.info.bio(&mbid, &artist.name).await.unwrap_or_default();
        artist.photo = bio.photo.clone();
        let mut similar = self.similar_of(&mbid, &ix).await;
        similar.retain(|a| a.library_id.is_some() || !prefs.dismissed.contains(&a.mbid));
        similar.truncate(FANS_MAX);
        Ok(CatalogueArtistPage {
            dismissed: prefs.dismissed.contains(&mbid),
            area: head.area,
            life_span: head.life_span,
            bio_credit: bio.bio.as_ref().and(bio.article_url.as_ref()).map(|_| "Wikipedia".to_string()),
            bio_url: bio.bio.as_ref().and(bio.article_url.clone()),
            bio: bio.bio,
            banner: bio.banner,
            artist,
            sections,
            similar,
        })
    }

    /// A catalogue album page: the tracklist of its earliest official
    /// release, whether it can be requested, and store links.
    pub async fn album_page(&self, rgid: &str) -> AppResult<CatalogueAlbumPage> {
        let rgid = rgid.trim().to_ascii_lowercase();
        if !is_mbid(&rgid) {
            return Err(AppError::NotFound(format!("album {rgid}")));
        }
        if !self.online() {
            return Err(AppError::Network("You're offline.".into()));
        }
        let v = self.inner.mb.get(&group_url(&rgid)).await?.ok_or_else(|| AppError::NotFound("MusicBrainz doesn't know this album.".into()))?;
        let g = parse_group(&v).ok_or_else(|| AppError::NotFound("MusicBrainz doesn't know this album.".into()))?;
        let credit = mb::credit_line(&v);
        let artist_mbid = mb::first_artist_id(&v);
        let first_name = g.credit.first().map(|c| c.name.clone()).unwrap_or_else(|| credit.clone());
        let ix = self.index().await?;
        let artist_library_id = ix.artist(artist_mbid.as_deref(), &first_name);

        let (tracks, label, released) = match mb::pick_release(&v) {
            Some(id) => match self.inner.mb.get(&mb::release_url(&id)).await? {
                Some(r) => mb::parse_release(&r),
                None => (Vec::new(), None, None),
            },
            None => (Vec::new(), None, None),
        };

        let want = Want { name: first_name.clone(), mbid: artist_mbid.clone(), artist_id: artist_library_id.clone() };
        let (lib_id, name) = (artist_library_id.clone(), first_name.clone());
        let lib = self.inner.db.read_async(move |c| library_albums(c, lib_id.as_deref(), &name)).await?;
        let requests = self.inner.requests.states();
        let missing = self.missing_ids().await;
        let mut album = self.album(&g, credit.clone(), artist_mbid, &requests, &missing);
        match vet(&g, &want, &lib, true) {
            Ok(()) => {}
            Err(Refusal::TitleClash { title, unreleased: false }) => {
                album.library_album_id = lib.iter().find(|l| l.title == title).map(|l| l.id.clone());
            }
            Err(r) => album.refusal = Some(r.message()),
        }
        Ok(CatalogueAlbumPage {
            links: album_store_links(&first_name, &g.title),
            album,
            tracks,
            released,
            label,
            artist_library_id,
        })
    }

    /// Artists outside the library for a library artist's "Fans also like".
    pub async fn fans_also_like(&self, artist_id: &str, name: &str) -> AppResult<Vec<CatArtist>> {
        let prefs = self.inner.prefs.prefs();
        if !prefs.fans || !self.online() {
            return Ok(Vec::new());
        }
        let Some(mbid) = self.inner.mbids.artist_mbid(artist_id, name).await else { return Ok(Vec::new()) };
        let ix = self.index().await?;
        let mut out = self.similar_of(&mbid, &ix).await;
        out.retain(|a| a.library_id.is_none() && !prefs.dismissed.contains(&a.mbid));
        out.truncate(FANS_MAX);
        Ok(out)
    }

    /// "New to you": worked out at most every [`NEW_TTL`], filtered again
    /// on every read (the library and the dismissed list change).
    pub async fn new_to_you(&self) -> AppResult<NewToYou> {
        let prefs = self.inner.prefs.prefs();
        if !prefs.new_to_you {
            return Ok(NewToYou::default());
        }
        let now = (self.inner.clock)();
        let stored: Option<StoredNew> =
            self.inner.db.read_async(|c| kv_get(c, NEW_KEY)).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok());
        let list = match stored {
            Some(s) if now - s.at < NEW_TTL => s.artists,
            old if !self.online() => old.map(|s| s.artists).unwrap_or_default(),
            old => {
                let _one = self.new_lock.lock().await;
                match self.work_out_new().await {
                    Ok(list) if !list.is_empty() => {
                        let json = serde_json::to_string(&StoredNew { at: now, artists: list.clone() }).unwrap_or_default();
                        let _ = self.inner.db.write_async(move |c| kv_set(c, NEW_KEY, &json)).await;
                        list
                    }
                    Ok(list) => old.map(|s| s.artists).unwrap_or(list),
                    Err(e) => {
                        tracing::debug!("new to you: {e}");
                        old.map(|s| s.artists).unwrap_or_default()
                    }
                }
            }
        };
        let ix = self.index().await?;
        let artists: Vec<NewArtist> = list
            .into_iter()
            .filter(|a| !prefs.dismissed.contains(&a.mbid) && ix.artist(Some(&a.mbid), &a.name).is_none())
            .take(NEW_MAX)
            .collect();
        Ok(NewToYou { shown: true, artists })
    }

    /// Scores artists like the ones played most: closeness on ListenBrainz
    /// (by rank, the most played seeds weigh more) plus a little for a
    /// MusicBrainz relationship (members' other bands, collaborations).
    async fn work_out_new(&self) -> AppResult<Vec<NewArtist>> {
        let top = self.inner.top.top(SEEDS);
        let mut seeds: Vec<(String, String)> = Vec::new();
        for a in top {
            if let Some(m) = self.inner.mbids.artist_mbid(a.id.as_deref().unwrap_or_default(), &a.name).await {
                seeds.push((m.to_ascii_lowercase(), a.name));
            }
        }
        let seed_ids: HashSet<String> = seeds.iter().map(|(m, _)| m.clone()).collect();
        let mut scores: HashMap<String, Scored> = HashMap::new();
        let mut add = |mbid: &str, name: &str, points: f64, because: &str| {
            if seed_ids.contains(mbid) || mbid == VARIOUS_ARTISTS || !is_mbid(mbid) {
                return;
            }
            let e = scores.entry(mbid.to_string()).or_insert_with(|| (name.to_string(), 0.0, Vec::new()));
            e.1 += points;
            if !e.2.iter().any(|(_, b)| b == because) {
                e.2.push((points, because.to_string()));
            }
        };
        for (i, (mbid, seed)) in seeds.iter().enumerate() {
            let weight = 1.0 / ((i + 1) as f64).sqrt();
            if let Ok(list) = self.inner.similar.similar(mbid).await {
                for (rank, a) in list.iter().take(PER_SEED).enumerate() {
                    add(&a.mbid, &a.name, weight / (rank as f64 + 1.0).sqrt(), seed);
                }
            }
            if let Ok(Some(v)) = self.inner.mb.get(&mb::artist_url(mbid)).await
                && let Some(d) = crate::artistinfo::mbdetails::parse_artist(&v)
            {
                for r in d.related {
                    if let Some(m) = r.mbid {
                        add(&m.to_ascii_lowercase(), &r.name, 0.25 * weight, seed);
                    }
                }
            }
        }
        let mut list: Vec<(String, Scored)> = scores.into_iter().collect();
        list.sort_by(|a, b| b.1.1.total_cmp(&a.1.1).then_with(|| a.1.0.cmp(&b.1.0)));
        Ok(list
            .into_iter()
            .take(NEW_MAX * 3)
            .map(|(mbid, (name, _, mut because))| {
                because.sort_by(|a, b| b.0.total_cmp(&a.0));
                NewArtist { mbid, name, because: because.into_iter().map(|(_, b)| b).take(3).collect() }
            })
            .collect())
    }
}
