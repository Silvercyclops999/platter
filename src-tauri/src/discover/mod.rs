//! Discover and get music that isn't in the library.
//!
//! - Search: artists, albums and songs the library doesn't have, in with the
//!   library's results. Artists and albums come from Lidarr's own search
//!   when Lidarr is set up (fast, no limit), else from MusicBrainz; songs
//!   from MusicBrainz. Each kind arrives on its own and is kept a while per
//!   query. Anything the library has folds into it and is left out.
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
use crate::db::searchtext::{canonical, sounds_like};
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::import::store::{StoreLinks, album_store_links, wanted_albums};
use crate::lidarr::ArtistMbid;
use crate::lidarr::mbpick::{Group, MbGet, Picker, RELEASE_SECTIONS, Refusal, VARIOUS_ARTISTS, Want, group_url, library_albums, parse_group, vet, wanted_section};
use crate::lidarr::request::cover_thumb;
use crate::lidarr::status::RequestState;
use crate::radar::TopPlayed;
use lb::SimilarArtists;
use library::LibraryIndex;

/// Results per kind in search.
pub const SEARCH_SHOWN: usize = 6;
/// How long one kind of search results is kept per query (this session).
pub const SEARCH_TTL: i64 = 15 * 60;
/// Most queries kept.
const SEARCH_KEPT: usize = 120;
/// How long a found photo is kept; a miss is asked again sooner.
pub const PHOTO_TTL: i64 = 30 * 24 * 3600;
pub const PHOTO_MISS_TTL: i64 = 3 * 24 * 3600;
/// Albums on a catalogue artist page's "Popular".
pub const POPULAR_SHOWN: usize = 5;
/// The kinds "Get albums" can take, and the one it takes by default.
pub const GET_KINDS: [&str; 4] = ["studio", "ep", "live", "compilation"];
pub const DEFAULT_GET_KINDS: [&str; 1] = ["studio"];
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
    /// Where the photo is from, when it needs a credit.
    #[serde(default)]
    pub photo_credit: Option<String>,
}

/// A photo of an artist outside the library, and its credit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CataloguePhoto {
    pub photo: Option<String>,
    pub credit: Option<String>,
}

/// One kind of catalogue search results (`catalogue_search_part`), as it
/// arrives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CataloguePart {
    /// The setting is on and Platter is online.
    pub shown: bool,
    /// `artists`, `albums` or `songs`.
    pub part: String,
    /// `lidarr` or `musicBrainz`.
    pub source: String,
    pub artists: Vec<CatArtist>,
    pub albums: Vec<CatAlbum>,
    pub songs: Vec<CatSong>,
    /// Library artists and albums a catalogue result folded into (they get
    /// a small "In your library" cue).
    pub in_library: Vec<String>,
}

/// "Get albums" for an artist: what it would queue, most popular first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAlbumsPlan {
    pub artist: String,
    pub artist_mbid: String,
    pub kinds: Vec<String>,
    pub albums: Vec<CatAlbum>,
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
    /// The section, with the library's section ids: `albums`, `singlesEps`,
    /// `compilations`, `live`, `soundtracks`, `remixes`, `mixtapes`, `demos`.
    pub kind: String,
    pub albums: Vec<CatAlbum>,
}

/// `featured_artist`: what the catalogue knows about an artist the library
/// only has as a featured guest (a `credit:` page).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeaturedArtist {
    /// Lookups ran (online, and the name was found on MusicBrainz).
    pub shown: bool,
    pub mbid: Option<String>,
    pub photo: Option<String>,
    pub photo_credit: Option<String>,
    /// Their releases by section, newest first, of the "Get albums" kinds.
    pub sections: Vec<CatSection>,
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
    /// Their most popular albums and EPs (most editions first).
    pub popular: Vec<CatAlbum>,
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
    /// A photo from the free picture sources (Wikidata on Commons, then
    /// TheAudioDB).
    async fn photo(&self, _mbid: &str, _name: &str) -> Option<CataloguePhoto> {
        None
    }
}

#[async_trait]
impl CatalogueInfo for crate::artistinfo::ArtistInfoService {
    async fn bio(&self, mbid: &str, name: &str) -> Option<CatalogueBio> {
        self.catalogue_bio(mbid, name).await
    }

    async fn photo(&self, mbid: &str, name: &str) -> Option<CataloguePhoto> {
        self.catalogue_photo(mbid, name).await
    }
}

/// Lidarr's own artist and album search, when it's set up.
#[async_trait]
pub trait CatalogueLookup: Send + Sync {
    /// An address and a key are set.
    fn ready(&self) -> bool;
    async fn artists(&self, term: &str) -> AppResult<Vec<LArtist>>;
    async fn albums(&self, term: &str) -> AppResult<Vec<LAlbum>>;
    async fn artist(&self, mbid: &str) -> AppResult<Option<LArtist>>;
}

#[async_trait]
impl CatalogueLookup for crate::lidarr::Lidarr {
    fn ready(&self) -> bool {
        self.lookups_ready()
    }
    async fn artists(&self, term: &str) -> AppResult<Vec<LArtist>> {
        self.lookup_artists(term).await
    }
    async fn albums(&self, term: &str) -> AppResult<Vec<LAlbum>> {
        self.lookup_albums(term).await
    }
    async fn artist(&self, mbid: &str) -> AppResult<Option<LArtist>> {
        self.lookup_artist(mbid).await
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
    /// Catalogue source: Lidarr when it's set up (else MusicBrainz).
    pub lidarr_source: bool,
    /// The "Get albums" kinds: which releases you don't have show.
    pub get_kinds: Vec<String>,
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
            lidarr_source: s.catalogue_source != "musicbrainz",
            get_kinds: clean_kinds(&s.get_albums_kinds),
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
    /// Lidarr's search, used for artists and albums when it's set up.
    pub lookup: Arc<dyn CatalogueLookup>,
    /// Go offline (and no connection): nothing is asked.
    pub offline: Arc<AtomicBool>,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

#[derive(Clone)]
pub struct Discover {
    inner: Arc<DiscoverDeps>,
    /// One "New to you" worked out at a time.
    new_lock: Arc<tokio::sync::Mutex<()>>,
    /// Search results by kind, source and query, with when they came.
    searches: Arc<parking_lot::Mutex<HashMap<String, (i64, CataloguePart)>>>,
    /// One lookup per search at a time: a second ask for the same one waits
    /// for the first and takes its answer.
    searching: Arc<parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

/// Lidarr's lookup answers, as Discover reads them.
pub type LArtist = crate::lidarr::api::Artist;
pub type LAlbum = crate::lidarr::api::Album;

#[derive(Serialize, Deserialize)]
struct StoredPhoto {
    at: i64,
    photo: CataloguePhoto,
}

/// Whether a release group is one of the kinds "Get albums" was asked for:
/// `studio` (albums with no secondary type), `ep`, `live`, `compilation`.
/// Singles, soundtracks, remixes and the like never are.
pub fn kind_wanted(g: &Group, kinds: &[String]) -> bool {
    let has = |k: &str| kinds.iter().any(|x| x == k);
    let sec = |k: &str| g.secondary.iter().any(|s| s == k);
    match g.primary.as_deref() {
        Some("album") if g.secondary.is_empty() => has("studio"),
        Some("ep") if g.secondary.is_empty() => has("ep"),
        Some("album" | "ep") if g.secondary.len() == 1 && sec("live") => has("live"),
        Some("album") if g.secondary.len() == 1 && sec("compilation") => has("compilation"),
        _ => false,
    }
}

/// The kinds asked for, known ones only, studio when none.
pub fn clean_kinds(kinds: &[String]) -> Vec<String> {
    let mut out: Vec<String> = GET_KINDS.iter().filter(|k| kinds.iter().any(|x| x.eq_ignore_ascii_case(k))).map(|k| k.to_string()).collect();
    if out.is_empty() {
        out = DEFAULT_GET_KINDS.iter().map(|k| k.to_string()).collect();
    }
    out
}

/// Most popular first: the most editions on MusicBrainz (a well-loved
/// album keeps being reissued), then the oldest.
fn by_popularity(a: &Group, b: &Group) -> std::cmp::Ordering {
    b.statuses.len().cmp(&a.statuses.len()).then_with(|| {
        let (x, y) = (if a.first_release.is_empty() { "9999" } else { &a.first_release }, if b.first_release.is_empty() { "9999" } else { &b.first_release });
        x.cmp(y)
    })
}

fn lookup_kind(t: Option<&str>) -> String {
    match t.map(str::to_ascii_lowercase).as_deref() {
        Some("ep") => "ep".into(),
        Some("single") => "single".into(),
        _ => "album".into(),
    }
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

/// The most popular albums and EPs (no singles, nothing refused).
fn popular_of(vetted: &[(Group, CatAlbum)]) -> Vec<CatAlbum> {
    let mut popular: Vec<&(Group, CatAlbum)> = vetted
        .iter()
        .filter(|(g, a)| g.secondary.is_empty() && matches!(g.primary.as_deref(), Some("album" | "ep")) && a.refusal.is_none())
        .collect();
    popular.sort_by(|a, b| by_popularity(&a.0, &b.0));
    popular.into_iter().take(POPULAR_SHOWN).map(|(_, a)| a.clone()).collect()
}

/// Catalogue albums in discography sections (page order, newest first in
/// each), only the releases of the "Get albums" `kinds`.
pub fn sectioned(vetted: Vec<(Group, CatAlbum)>, kinds: &[String]) -> Vec<CatSection> {
    let mut by_kind: HashMap<&'static str, Vec<CatAlbum>> = HashMap::new();
    for (g, a) in vetted {
        if let Some(s) = wanted_section(&g, kinds) {
            by_kind.entry(s).or_default().push(a);
        }
    }
    RELEASE_SECTIONS
        .into_iter()
        .filter_map(|k| {
            let mut albums = by_kind.remove(k)?;
            albums.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| a.title.cmp(&b.title)));
            Some(CatSection { kind: k.into(), albums })
        })
        .collect()
}

impl Discover {
    pub fn new(deps: DiscoverDeps) -> Self {
        Self { inner: Arc::new(deps), new_lock: Arc::new(tokio::sync::Mutex::new(())), searches: Default::default(), searching: Default::default() }
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

    /// Artists, albums and songs the library doesn't have, all at once (the
    /// three kinds of [`Self::search_part`] together).
    pub async fn search(&self, query: &str) -> AppResult<CatalogueSearch> {
        let mut out = CatalogueSearch::default();
        for part in ["artists", "albums", "songs"] {
            let p = self.search_part(query, part).await?;
            out.shown = p.shown;
            out.artists.extend(p.artists);
            out.albums.extend(p.albums);
            out.songs.extend(p.songs);
        }
        Ok(out)
    }

    /// One kind of search results (`artists`, `albums` or `songs`) the
    /// library doesn't have. Artists and albums come from Lidarr's search
    /// when it's set up and the setting allows (MusicBrainz when it fails);
    /// songs from MusicBrainz. Kept [`SEARCH_TTL`] per query.
    pub async fn search_part(&self, query: &str, part: &str) -> AppResult<CataloguePart> {
        if !matches!(part, "artists" | "albums" | "songs") {
            return Err(AppError::Invalid(format!("unknown search part {part}")));
        }
        let prefs = self.inner.prefs.prefs();
        let mut out = CataloguePart { part: part.into(), ..Default::default() };
        if !prefs.search || !self.online() {
            return Ok(out);
        }
        out.shown = true;
        let q: String = query.trim().chars().take(200).collect();
        if q.chars().count() < 2 {
            return Ok(out);
        }
        let lidarr = prefs.lidarr_source && part != "songs" && self.inner.lookup.ready();
        let key = format!("{part}|{}|{}", if lidarr { "lidarr" } else { "mb" }, q.to_lowercase());
        let now = (self.inner.clock)();
        let one = self.searching.lock().entry(key.clone()).or_default().clone();
        let _one = one.lock().await;
        let hit = self.searches.lock().get(&key).filter(|(at, _)| now - *at < SEARCH_TTL).map(|(_, p)| p.clone());
        if let Some(mut p) = hit {
            // Request states change; the rest holds.
            let requests = self.inner.requests.states();
            for a in &mut p.albums {
                a.request = requests.get(&a.release_group_id).copied();
            }
            return Ok(p);
        }
        let started = std::time::Instant::now();
        let fresh = if lidarr {
            match self.lidarr_part(&q, part).await {
                // Lidarr's metadata service sometimes answers with nothing.
                Ok(p) if p.artists.is_empty() && p.albums.is_empty() && p.in_library.is_empty() => {
                    tracing::info!(part, "catalogue search through Lidarr found nothing; asking MusicBrainz");
                    self.mb_part(&q, part).await
                }
                Ok(p) => Ok(p),
                Err(e) => {
                    tracing::info!(part, "catalogue search through Lidarr: {e}; asking MusicBrainz");
                    self.mb_part(&q, part).await
                }
            }
        } else {
            self.mb_part(&q, part).await
        };
        let fresh = match fresh {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(part, ms = started.elapsed().as_millis() as u64, "catalogue search failed: {e}");
                return Err(e);
            }
        };
        let fresh = if part == "artists" { self.named_loosely(&q, fresh).await } else { fresh };
        tracing::info!(
            part,
            source = %fresh.source,
            artists = fresh.artists.len(),
            albums = fresh.albums.len(),
            songs = fresh.songs.len(),
            in_library = fresh.in_library.len(),
            ms = started.elapsed().as_millis() as u64,
            "catalogue search"
        );
        let mut cache = self.searches.lock();
        if cache.len() >= SEARCH_KEPT {
            cache.retain(|_, (at, _)| now - *at < SEARCH_TTL);
            if cache.len() >= SEARCH_KEPT {
                cache.clear();
            }
        }
        cache.insert(key, (now, fresh.clone()));
        drop(cache);
        // Searches nobody is waiting on any more.
        self.searching.lock().retain(|_, m| Arc::strong_count(m) > 1);
        Ok(fresh)
    }

    /// When a name of two to four words found nobody named that way,
    /// MusicBrainz asked again forgiving a letter or two per word: an artist
    /// it finds named the way the query spells it comes first ("juice
    /// world" is Juice WRLD). Not when an artist found or in the library is
    /// named that way already, or named by the query's first words with one
    /// more after ("northlight live").
    async fn named_loosely(&self, q: &str, mut p: CataloguePart) -> CataloguePart {
        let words: Vec<String> = canonical(q).split(' ').map(str::to_string).collect();
        if !(2..=4).contains(&words.len()) {
            return p;
        }
        let named = {
            let (q, words) = (q.to_string(), words.clone());
            move |name: &str| {
                let nw: Vec<String> = canonical(name).split(' ').map(str::to_string).collect();
                sounds_like(&q, name) || (nw.len() + 1 == words.len() && words.starts_with(&nw))
            }
        };
        if p.artists.iter().any(|a| named(&a.name)) {
            return p;
        }
        let in_library = self
            .inner
            .db
            .read_async({
                let named = named.clone();
                move |c| {
                    let mut st = c.prepare_cached("SELECT title FROM artists")?;
                    let titles = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                    Ok(titles.iter().any(|t| named(t)))
                }
            })
            .await;
        if in_library.unwrap_or(true) {
            return p;
        }
        let found = match self.inner.mb.get(&mb::artist_loose_search_url(q)).await {
            Ok(Some(v)) => mb::parse_artists(&v),
            Ok(None) => return p,
            Err(e) => {
                tracing::debug!("loose artist search: {e}");
                return p;
            }
        };
        let named: Vec<CatArtist> = found.into_iter().filter(|a| sounds_like(q, &a.name)).collect();
        if named.is_empty() {
            return p;
        }
        let Ok(ix) = self.index().await else { return p };
        let mut more = CataloguePart::default();
        Self::keep_artists(&ix, named, &mut more);
        for id in more.in_library {
            if !p.in_library.contains(&id) {
                p.in_library.push(id);
            }
        }
        p.artists.retain(|a| !more.artists.iter().any(|m| m.mbid == a.mbid));
        more.artists.append(&mut p.artists);
        more.artists.truncate(SEARCH_SHOWN);
        p.artists = more.artists;
        p
    }

    /// Artists a search found, minus the library's (noted in `in_library`).
    fn keep_artists(ix: &LibraryIndex, found: Vec<CatArtist>, out: &mut CataloguePart) {
        for a in found {
            if a.mbid == VARIOUS_ARTISTS {
                continue;
            }
            match ix.artist(Some(&a.mbid), &a.name) {
                Some(id) => {
                    if !out.in_library.contains(&id) {
                        out.in_library.push(id);
                    }
                }
                None if out.artists.len() < SEARCH_SHOWN && !out.artists.iter().any(|x| x.mbid == a.mbid) => out.artists.push(a),
                None => {}
            }
        }
    }

    /// Albums a search found, minus the library's (noted in `in_library`).
    async fn keep_albums(&self, ix: &LibraryIndex, found: Vec<(CatAlbum, String)>, out: &mut CataloguePart) -> AppResult<()> {
        for (a, first_name) in found {
            if out.albums.len() >= SEARCH_SHOWN {
                break;
            }
            if let Some(aid) = ix.artist(a.artist_mbid.as_deref(), &first_name) {
                let title = a.title.clone();
                if let Some(id) = self.inner.db.read_async(move |c| library::album_of(c, &aid, &title)).await? {
                    if !out.in_library.contains(&id) {
                        out.in_library.push(id);
                    }
                    continue;
                }
            }
            if !out.albums.iter().any(|x| x.release_group_id == a.release_group_id) {
                out.albums.push(a);
            }
        }
        Ok(())
    }

    async fn mb_part(&self, q: &str, part: &str) -> AppResult<CataloguePart> {
        let mut out = CataloguePart { shown: true, part: part.into(), source: "musicBrainz".into(), ..Default::default() };
        let ix = self.index().await?;
        match part {
            "artists" => {
                if let Some(v) = self.inner.mb.get(&mb::artist_search_url(q)).await? {
                    Self::keep_artists(&ix, mb::parse_artists(&v), &mut out);
                }
            }
            "albums" => {
                if let Some(v) = self.inner.mb.get(&mb::group_search_url(q)).await? {
                    let requests = self.inner.requests.states();
                    let missing = self.missing_ids().await;
                    let found = mb::parse_groups(&v)
                        .into_iter()
                        .map(|(g, credit, first)| {
                            let first_name = g.credit.first().map(|c| c.name.clone()).unwrap_or_else(|| credit.clone());
                            (self.album(&g, credit, first, &requests, &missing), first_name)
                        })
                        .collect();
                    self.keep_albums(&ix, found, &mut out).await?;
                }
            }
            _ => {
                if let Some(v) = self.inner.mb.get(&mb::recording_search_url(q)).await? {
                    for s in mb::parse_recordings(&v) {
                        let (artist, title) = (s.artist.clone(), s.title.clone());
                        let first = artist.split([',', '&']).next().unwrap_or(&artist).trim().to_string();
                        let have = self
                            .inner
                            .db
                            .read_async(move |c| Ok(library::has_song(c, &artist, &title)? || library::has_song(c, &first, &title)?))
                            .await?;
                        if !have {
                            out.songs.push(s);
                        }
                        if out.songs.len() == SEARCH_SHOWN {
                            break;
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    async fn lidarr_part(&self, q: &str, part: &str) -> AppResult<CataloguePart> {
        let mut out = CataloguePart { shown: true, part: part.into(), source: "lidarr".into(), ..Default::default() };
        let ix = self.index().await?;
        if part == "artists" {
            let found = self
                .inner
                .lookup
                .artists(q)
                .await?
                .into_iter()
                .filter_map(|a| {
                    let mbid = a.foreign_artist_id.trim().to_ascii_lowercase();
                    let name = a.artist_name.trim().to_string();
                    (is_mbid(&mbid) && !name.is_empty()).then(|| CatArtist {
                        photo: crate::lidarr::lookup::artist_photo(&a),
                        mbid,
                        name,
                        disambiguation: a.disambiguation.clone().filter(|d| !d.trim().is_empty()),
                        kind: a.artist_type.clone().filter(|d| !d.trim().is_empty()),
                        library_id: None,
                        photo_credit: None,
                    })
                })
                .collect();
            Self::keep_artists(&ix, found, &mut out);
        } else {
            let requests = self.inner.requests.states();
            let missing = self.missing_ids().await;
            let found = self
                .inner
                .lookup
                .albums(q)
                .await?
                .into_iter()
                .filter_map(|a| {
                    let rg = a.foreign_album_id.trim().to_ascii_lowercase();
                    let artist = a.artist.clone().unwrap_or_default();
                    let artist_mbid = Some(artist.foreign_artist_id.trim().to_ascii_lowercase()).filter(|m| is_mbid(m));
                    if !is_mbid(&rg) || a.title.trim().is_empty() || artist_mbid.as_deref() == Some(VARIOUS_ARTISTS) {
                        return None;
                    }
                    let name = artist.artist_name.trim().to_string();
                    Some((
                        CatAlbum {
                            thumb: crate::lidarr::lookup::album_cover(&a).or_else(|| cover_thumb(&rg)),
                            request: requests.get(&rg).copied(),
                            in_missing: missing.contains(&rg),
                            kind: lookup_kind(a.album_type.as_deref()),
                            secondary: a.secondary_types.iter().map(|s| s.to_ascii_lowercase()).collect(),
                            year: a.release_date.as_deref().and_then(year_of),
                            title: a.title.trim().to_string(),
                            artist: name.clone(),
                            artist_mbid,
                            release_group_id: rg,
                            library_album_id: None,
                            refusal: None,
                        },
                        name,
                    ))
                })
                .collect();
            self.keep_albums(&ix, found, &mut out).await?;
        }
        Ok(out)
    }

    /// A photo for an artist outside the library: Lidarr's lookup when it's
    /// set up, else Wikidata's picture on Commons, else TheAudioDB. Kept
    /// [`PHOTO_TTL`] (a miss [`PHOTO_MISS_TTL`]). Empty while offline.
    pub async fn artist_photo(&self, mbid: &str, name: &str) -> AppResult<CataloguePhoto> {
        let mbid = mbid.trim().to_ascii_lowercase();
        if !is_mbid(&mbid) {
            return Ok(CataloguePhoto::default());
        }
        let key = format!("discover.photo:{mbid}");
        let now = (self.inner.clock)();
        let k = key.clone();
        let stored: Option<StoredPhoto> = self.inner.db.read_async(move |c| kv_get(c, &k)).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok());
        if let Some(s) = &stored {
            let ttl = if s.photo.photo.is_some() { PHOTO_TTL } else { PHOTO_MISS_TTL };
            if now - s.at < ttl {
                return Ok(s.photo.clone());
            }
        }
        if !self.online() {
            return Ok(stored.map(|s| s.photo).unwrap_or_default());
        }
        let mut photo = CataloguePhoto::default();
        if self.inner.prefs.prefs().lidarr_source && self.inner.lookup.ready() {
            match self.inner.lookup.artist(&mbid).await {
                Ok(Some(a)) => photo.photo = crate::lidarr::lookup::artist_photo(&a),
                Ok(None) => {}
                Err(e) => tracing::debug!("artist photo through Lidarr: {e}"),
            }
        }
        if photo.photo.is_none()
            && let Some(p) = self.inner.info.photo(&mbid, name).await
        {
            photo = p;
        }
        let json = serde_json::to_string(&StoredPhoto { at: now, photo: photo.clone() }).unwrap_or_default();
        let _ = self.inner.db.write_async(move |c| kv_set(c, &key, &json)).await;
        Ok(photo)
    }

    /// An artist's official release groups that are theirs and that the
    /// library lacks, each as a catalogue album (with a refusal or the
    /// library album it matched where there's one).
    async fn vetted(&self, mbid: &str, name: &str, library_id: Option<String>) -> AppResult<Vec<(Group, CatAlbum)>> {
        let groups = self.picker().artist_groups(mbid).await?;
        let want = Want { name: name.to_string(), mbid: Some(mbid.to_string()), artist_id: library_id.clone() };
        let n = name.to_string();
        let lib = self.inner.db.read_async(move |c| library_albums(c, library_id.as_deref(), &n)).await?;
        let requests = self.inner.requests.states();
        let missing = self.missing_ids().await;
        let mut out = Vec::new();
        for g in groups {
            let mut a = self.album(&g, name.to_string(), Some(mbid.to_string()), &requests, &missing);
            match vet(&g, &want, &lib, true) {
                Ok(()) => {}
                Err(Refusal::TitleClash { title, unreleased: false }) => {
                    a.library_album_id = lib.iter().find(|l| l.title == title).map(|l| l.id.clone());
                }
                Err(r @ Refusal::TitleClash { .. }) => a.refusal = Some(r.message()),
                // Someone else's release, a bootleg, Various Artists: not theirs to show.
                Err(_) => continue,
            }
            out.push((g, a));
        }
        Ok(out)
    }

    /// "Get albums": the artist's albums of these kinds that the library
    /// doesn't have and nobody asked for yet, most popular first.
    pub async fn get_albums_plan(&self, mbid: &str, name: &str, kinds: &[String]) -> AppResult<GetAlbumsPlan> {
        let mbid = mbid.trim().to_ascii_lowercase();
        if !is_mbid(&mbid) {
            return Err(AppError::NotFound(format!("artist {mbid}")));
        }
        if !self.online() {
            return Err(AppError::Network("You're offline.".into()));
        }
        let kinds = clean_kinds(kinds);
        let ix = self.index().await?;
        let library_id = ix.artist(Some(&mbid), name);
        let mut list: Vec<(Group, CatAlbum)> = self
            .vetted(&mbid, name, library_id)
            .await?
            .into_iter()
            .filter(|(g, a)| {
                kind_wanted(g, &kinds)
                    && a.library_album_id.is_none()
                    && a.refusal.is_none()
                    && a.request.is_none_or(|s| s == RequestState::Failed)
            })
            .collect();
        list.sort_by(|a, b| by_popularity(&a.0, &b.0));
        Ok(GetAlbumsPlan { artist: name.to_string(), artist_mbid: mbid, kinds, albums: list.into_iter().map(|(_, a)| a).collect() })
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
                photo_credit: None,
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

        let vetted = self.vetted(&mbid, &artist.name, artist.library_id.clone()).await?;
        let popular = popular_of(&vetted);
        let sections = sectioned(vetted, &prefs.get_kinds);

        let bio = self.inner.info.bio(&mbid, &artist.name).await.unwrap_or_default();
        artist.photo = bio.photo.clone();
        if artist.photo.is_none() {
            let p = self.artist_photo(&mbid, &artist.name).await.unwrap_or_default();
            artist.photo = p.photo;
            artist.photo_credit = p.credit;
        }
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
            popular,
        })
    }

    /// A featured guest's MusicBrainz id, while online.
    async fn featured_mbid(&self, id: &str, name: &str) -> Option<String> {
        if name.is_empty() || !self.online() {
            return None;
        }
        self.inner.mbids.artist_mbid(id, name).await.map(|m| m.trim().to_ascii_lowercase()).filter(|m| is_mbid(m))
    }

    /// Just the photo of a featured guest (search's top result card), without
    /// their releases.
    pub async fn featured_photo(&self, id: &str, name: &str) -> AppResult<CataloguePhoto> {
        let name = name.trim();
        match self.featured_mbid(id, name).await {
            Some(mbid) => self.artist_photo(&mbid, name).await,
            None => Ok(CataloguePhoto::default()),
        }
    }

    /// What the catalogue knows about a featured guest (a `credit:` page,
    /// `id`): their MusicBrainz id (an exact name match, cached), a photo
    /// from the free picture sources and their releases by section, each
    /// with Get or "In your library". Nothing while offline or when
    /// MusicBrainz doesn't know the name.
    pub async fn featured_artist(&self, id: &str, name: &str) -> AppResult<FeaturedArtist> {
        let name = name.trim();
        let Some(mbid) = self.featured_mbid(id, name).await else { return Ok(FeaturedArtist::default()) };
        let prefs = self.inner.prefs.prefs();
        let ix = self.index().await?;
        let library_id = ix.artist(Some(&mbid), name);
        let vetted = self.vetted(&mbid, name, library_id).await?;
        let sections = sectioned(vetted, &prefs.get_kinds);
        let photo = self.artist_photo(&mbid, name).await.unwrap_or_default();
        Ok(FeaturedArtist { shown: true, mbid: Some(mbid), photo: photo.photo, photo_credit: photo.credit, sections })
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
