//! Which MusicBrainz release group a request is for, with every guard.
//!
//! Lidarr only knows official MusicBrainz releases, and leaks often sit on
//! MusicBrainz as bootlegs, so the rules are strict:
//!
//! - at least one release in the group is official (bootleg, promo and
//!   pseudo-release only groups are refused)
//! - primary type Album, then EP, then Single
//! - credited to the artist alone (never Various Artists, never someone else)
//! - the earliest first release wins
//! - no secondary type (Compilation, Live...) unless asked for
//! - never a title that folds to the title of any album by that artist
//!   already in the library: released, unreleased builds, editions, albums
//!   with ARCHIVE_* tags or probably unreleased. Plex can merge two
//!   same-titled albums by one artist, which would mix a download into an
//!   unreleased build.
//!
//! MusicBrainz is asked through the shared one-a-second limiter with
//! Platter's user agent, and answers are cached for a week.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::artistinfo::http::InfoHttp;
use crate::artistinfo::is_mbid;
use crate::artistinfo::limit::Limiter;
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::identity::fold::{fold_album, fold_artist, fold_title};
use crate::share::matching::title_key;

pub const WS: &str = "https://musicbrainz.org/ws/2";
/// MusicBrainz's Various Artists.
pub const VARIOUS_ARTISTS: &str = "89ad4ac3-39f7-470e-963a-56509c546377";
/// How long a MusicBrainz answer is kept.
pub const CACHE_TTL: i64 = 7 * 24 * 3600;
/// Pages of an artist's releases read (100 each).
const MAX_PAGES: usize = 4;
/// Release groups looked up one by one for a song or album.
const MAX_LOOKUPS: usize = 4;

pub const MB_DOWN: &str = "MusicBrainz isn't answering right now. Try again in a minute.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditArtist {
    pub id: String,
    pub name: String,
}

/// A release group as the rules see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub title: String,
    /// `album`, `ep`, `single`, `broadcast`, `other` (lower case).
    pub primary: Option<String>,
    pub secondary: Vec<String>,
    /// `YYYY`, `YYYY-MM` or `YYYY-MM-DD`; empty when unknown.
    pub first_release: String,
    /// Status of each release in the group (lower case).
    #[serde(skip)]
    pub statuses: Vec<String>,
    /// Who the group is credited to.
    #[serde(skip)]
    pub credit: Vec<CreditArtist>,
}

/// The artist a request is for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Want {
    pub name: String,
    pub mbid: Option<String>,
    /// Library artist id, when the request starts on an artist page.
    pub artist_id: Option<String>,
}

impl Want {
    pub fn fold(&self) -> String {
        fold_artist(&self.name)
    }
}

/// An album by the artist already in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryAlbum {
    /// The library album's id.
    pub id: String,
    pub title: String,
    /// Other names it goes by (its ARCHIVE_* group), checked as well.
    pub also: Vec<String>,
    pub unreleased: bool,
}

/// Why a release group can't be requested.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Refusal {
    /// MusicBrainz has no release like it.
    NotFound,
    /// Only bootleg, promo or other unofficial releases.
    NotOfficial,
    /// Not an album, EP or single.
    NotAlbum,
    /// Credited to Various Artists.
    VariousArtists,
    /// Credited to someone else, or to the artist with others.
    OtherArtist,
    /// A compilation, live album or the like, not asked for.
    Secondary { types: Vec<String> },
    /// Has the title of an album already in the library.
    TitleClash { title: String, unreleased: bool },
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Refusal::NotFound => "MusicBrainz doesn't have an official release for this, so Lidarr can't get it.".into(),
            Refusal::NotOfficial => "MusicBrainz lists this only as a bootleg or promo, not an official release. Platter never asks Lidarr for those.".into(),
            Refusal::NotAlbum => "This isn't an album, EP or single on MusicBrainz.".into(),
            Refusal::VariousArtists => "This is only on a various artists release. Platter only requests the artist's own releases.".into(),
            Refusal::OtherArtist => "MusicBrainz credits this release to someone else, so Platter won't request it for this artist.".into(),
            Refusal::Secondary { types } => format!("This is only on a {} release.", types.join(" / ").to_lowercase()),
            Refusal::TitleClash { title, unreleased: true } => format!(
                "It has the same title as \"{title}\", an unreleased album already in your library. The two could get mixed together, so it can't be requested."
            ),
            Refusal::TitleClash { title, unreleased: false } => {
                format!("It has the same title as \"{title}\", already in your library. The two could get mixed together, so it can't be requested.")
            }
        }
    }

    fn into_error(self) -> AppError {
        AppError::Invalid(self.message())
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn lower_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(|x| x.to_ascii_lowercase()).collect())
        .unwrap_or_default()
}

fn credit(v: Option<&Value>) -> Vec<CreditArtist> {
    v.and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .map(|c| {
                    let a = c.get("artist");
                    CreditArtist {
                        id: a.map(|a| s(a, "id")).unwrap_or_default().to_ascii_lowercase(),
                        name: a.map(|a| s(a, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| s(c, "name")),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn opt_lower(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_ascii_lowercase).filter(|x| !x.is_empty())
}

/// `GET /release-group/{id}?inc=artist-credits+releases`.
pub fn parse_group(v: &Value) -> Option<Group> {
    let id = s(v, "id").to_ascii_lowercase();
    if !is_mbid(&id) {
        return None;
    }
    let statuses = v
        .get("releases")
        .and_then(Value::as_array)
        .map(|rs| rs.iter().map(|r| s(r, "status").to_ascii_lowercase()).collect())
        .unwrap_or_default();
    Some(Group {
        id,
        title: s(v, "title"),
        primary: opt_lower(v, "primary-type"),
        secondary: lower_list(v.get("secondary-types")),
        first_release: s(v, "first-release-date"),
        statuses,
        credit: credit(v.get("artist-credit")),
    })
}

/// Releases browsed by artist (`inc=release-groups+artist-credits`), folded
/// into their release groups. Each group takes its credit from an official
/// release.
pub fn groups_from_releases(v: &Value, into: &mut BTreeMap<String, Group>) {
    for r in v.get("releases").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        let Some(rg) = r.get("release-group") else { continue };
        let id = s(rg, "id").to_ascii_lowercase();
        if !is_mbid(&id) {
            continue;
        }
        let status = s(r, "status").to_ascii_lowercase();
        let rel_credit = credit(r.get("artist-credit"));
        let g = into.entry(id.clone()).or_insert_with(|| Group {
            id,
            title: s(rg, "title"),
            primary: opt_lower(rg, "primary-type"),
            secondary: lower_list(rg.get("secondary-types")),
            first_release: s(rg, "first-release-date"),
            statuses: Vec::new(),
            credit: Vec::new(),
        });
        // The first official release's credit, else any release's.
        let had_official = g.statuses.iter().any(|x| x == "official");
        if g.credit.is_empty() || (status == "official" && !had_official) {
            g.credit = rel_credit;
        }
        g.statuses.push(status);
    }
}

fn type_rank(primary: Option<&str>) -> Option<u8> {
    match primary {
        Some("album") => Some(0),
        Some("ep") => Some(1),
        Some("single") => Some(2),
        _ => None,
    }
}

fn date_key(d: &str) -> String {
    if d.trim().is_empty() { "9999".into() } else { d.trim().to_string() }
}

/// Sort key: Album before EP before Single, groups without a secondary
/// type first, then the earliest.
fn rank(g: &Group) -> (u8, bool, String) {
    (type_rank(g.primary.as_deref()).unwrap_or(9), !g.secondary.is_empty(), date_key(&g.first_release))
}

/// The title with every bracketed part and every " - " part taken off,
/// whatever they say ("Record (Build 2)" and "Record - Sessions" are
/// "record").
fn base_title(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut depth = 0usize;
    for ch in t.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    let dashed = out.replace(['\u{2013}', '\u{2014}'], "-");
    fold_album(dashed.split(" - ").next().unwrap_or_default())
}

/// Folded forms a title is compared by: two titles clash when any form
/// matches (stricter together than any one).
fn title_forms(t: &str) -> [String; 3] {
    [fold_album(t), title_key(t), base_title(t)]
}

/// The library album a title clashes with.
pub fn clash<'a>(title: &str, library: &'a [LibraryAlbum]) -> Option<&'a LibraryAlbum> {
    let want = title_forms(title);
    if want.iter().all(String::is_empty) {
        return None;
    }
    let hit = |t: &str| {
        let have = title_forms(t);
        want.iter().zip(have.iter()).any(|(a, b)| !a.is_empty() && a == b)
    };
    // An unreleased match is the one worth naming.
    let mut found: Option<&LibraryAlbum> = None;
    for a in library {
        if hit(&a.title) || a.also.iter().any(|t| hit(t)) {
            if a.unreleased {
                return Some(a);
            }
            found.get_or_insert(a);
        }
    }
    found
}

/// Whether the group passes every rule.
pub fn vet(g: &Group, want: &Want, library: &[LibraryAlbum], allow_secondary: bool) -> Result<(), Refusal> {
    if !g.statuses.iter().any(|x| x == "official") {
        return Err(Refusal::NotOfficial);
    }
    if type_rank(g.primary.as_deref()).is_none() {
        return Err(Refusal::NotAlbum);
    }
    if g.credit.iter().any(|c| c.id == VARIOUS_ARTISTS) || g.credit.iter().any(|c| fold_artist(&c.name).is_empty()) {
        return Err(Refusal::VariousArtists);
    }
    let alone = match g.credit.as_slice() {
        [only] => match want.mbid.as_deref().filter(|m| is_mbid(m)) {
            Some(m) => only.id.eq_ignore_ascii_case(m),
            None => !want.fold().is_empty() && fold_artist(&only.name) == want.fold(),
        },
        _ => false,
    };
    if !alone {
        return Err(Refusal::OtherArtist);
    }
    if !allow_secondary && !g.secondary.is_empty() {
        return Err(Refusal::Secondary { types: g.secondary.clone() });
    }
    if let Some(a) = clash(&g.title, library) {
        return Err(Refusal::TitleClash { title: a.title.clone(), unreleased: a.unreleased });
    }
    Ok(())
}

/// The best group that passes, else why the best-ranked one doesn't.
pub fn pick(mut groups: Vec<Group>, want: &Want, library: &[LibraryAlbum], allow_secondary: bool) -> Result<Group, Refusal> {
    groups.sort_by_key(rank);
    let mut first_refusal: Option<Refusal> = None;
    for g in groups {
        match vet(&g, want, library, allow_secondary) {
            Ok(()) => return Ok(g),
            Err(r) => {
                // A title clash is the answer that matters most.
                let clash_now = matches!(r, Refusal::TitleClash { .. });
                if first_refusal.is_none() || clash_now && !matches!(first_refusal, Some(Refusal::TitleClash { .. })) {
                    first_refusal = Some(r);
                }
            }
        }
    }
    Err(first_refusal.unwrap_or(Refusal::NotFound))
}

/// The artist's official albums and EPs the library lacks, oldest first.
/// Anything whose title matches a library album is left out (so nothing
/// that shares a name with an unreleased build is ever offered).
pub fn missing(groups: Vec<Group>, want: &Want, library: &[LibraryAlbum]) -> Vec<Group> {
    let mut out: Vec<Group> = groups
        .into_iter()
        .filter(|g| matches!(g.primary.as_deref(), Some("album") | Some("ep")))
        .filter(|g| vet(g, want, library, false).is_ok())
        .collect();
    out.sort_by_key(|g| (date_key(&g.first_release), g.title.to_lowercase()));
    out
}

// ---------------------------------------------------------------- the library

/// Albums by the artist in the library: by library artist id, and by
/// folded artist name (the same artist on another server or spelled
/// another way).
///
/// Whether each counts as unreleased follows the archive's rule with the
/// user's marks: an album marked unreleased (or released) counts that way,
/// and an album holding any song that counts as unreleased (its own mark,
/// or its file's ARCHIVE_VERSION) counts as unreleased too.
pub fn library_albums(c: &Connection, artist_id: Option<&str>, artist_name: &str) -> AppResult<Vec<LibraryAlbum>> {
    use crate::db::release::{album_unreleased, track_unreleased};
    let want = fold_artist(artist_name);
    let mut st = c.prepare(
        "SELECT al.id, al.title, al.artist_id, al.artist_title, al.format, al.subformats, at.version, at.grp, al.matched, al.studio, mc.found
         FROM albums al
         LEFT JOIN album_tags at ON at.album_id = al.id
         LEFT JOIN album_mb_check mc ON mc.album_id = al.id",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            (r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?),
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<bool>>(8)?,
            r.get::<_, Option<String>>(9)?,
            r.get::<_, Option<bool>>(10)?,
        ))
    })?;
    let mut songs = c.prepare_cached("SELECT t.id, ft.version FROM tracks t LEFT JOIN file_tags ft ON ft.part_key = t.part_key WHERE t.album_id = ?1")?;
    let mut out = Vec::new();
    for row in rows {
        let ((id, title, aid, artist), format, subs, version, grp, matched, studio, mb) = row?;
        let ours = artist_id.is_some_and(|a| a == aid) || (!want.is_empty() && fold_artist(&artist) == want);
        if !ours {
            continue;
        }
        let subs: Vec<String> = subs.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let (format, studio, version) = (format.as_deref(), studio.as_deref(), version.as_deref());
        let mut unreleased = album_unreleased(&id, format, &subs, matched, studio, version, mb);
        if !unreleased {
            let list = songs.query_map([&id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
            for song in list {
                let (tid, file_version) = song?;
                let v = file_version.as_deref().filter(|v| !v.trim().is_empty()).or(version);
                if track_unreleased(&tid, &id, format, &subs, matched, studio, v, mb) {
                    unreleased = true;
                    break;
                }
            }
        }
        out.push(LibraryAlbum { id, title, also: grp.into_iter().filter(|g| !g.trim().is_empty()).collect(), unreleased });
    }
    Ok(out)
}

// ---------------------------------------------------------------- MusicBrainz

/// A GET to MusicBrainz's web service: `Ok(None)` when it has no such thing.
#[async_trait]
pub trait MbGet: Send + Sync {
    async fn get(&self, url: &str) -> AppResult<Option<Value>>;
}

/// The real one: paced by the shared limiter, cached in the local store.
pub struct MbCached {
    pub http: Arc<dyn InfoHttp>,
    pub limiter: Arc<Limiter>,
    pub db: DbHandle,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// How long an answer is kept (seconds): [`CACHE_TTL`] for requests,
    /// shorter for the release radar's daily look.
    pub ttl: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Cached {
    at: i64,
    body: Option<Value>,
}

#[async_trait]
impl MbGet for MbCached {
    async fn get(&self, url: &str) -> AppResult<Option<Value>> {
        let key = format!("lidarr.mb:{url}");
        let now = (self.clock)();
        let k = key.clone();
        let hit: Option<Cached> = self.db.read_async(move |c| kv_get(c, &k)).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok());
        if let Some(h) = &hit
            && now - h.at < self.ttl
        {
            return Ok(h.body.clone());
        }
        let fresh = match self.limiter.get(self.http.as_ref(), url, &[]).await {
            Ok(r) if r.ok() => Some(r.json::<Value>().map_err(|_| AppError::Network(MB_DOWN.into()))?),
            Ok(r) if r.status == 404 || r.status == 400 => None,
            // Busy or unreachable: an old answer is better than none.
            _ => return hit.map(|h| h.body).ok_or_else(|| AppError::Network(MB_DOWN.into())),
        };
        let json = serde_json::to_string(&Cached { at: now, body: fresh.clone() }).unwrap_or_default();
        let _ = self.db.write_async(move |c| kv_set(c, &key, &json)).await;
        Ok(fresh)
    }
}

fn phrase(t: &str) -> String {
    let mut out = String::from("\"");
    for ch in t.trim().chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

pub fn group_url(rgid: &str) -> String {
    format!("{WS}/release-group/{rgid}?inc=artist-credits+releases&fmt=json")
}

pub fn artist_releases_url(mbid: &str, offset: usize) -> String {
    let mut u = Url::parse(&format!("{WS}/release")).expect("static url");
    u.query_pairs_mut()
        .append_pair("artist", mbid)
        .append_pair("status", "official")
        .append_pair("type", "album|ep|single")
        .append_pair("inc", "release-groups+artist-credits")
        .append_pair("limit", "100")
        .append_pair("offset", &offset.to_string())
        .append_pair("fmt", "json");
    u.into()
}

pub fn group_search_url(artist: &str, title: &str) -> String {
    let mut u = Url::parse(&format!("{WS}/release-group")).expect("static url");
    let t = title_key(title);
    let t = if t.is_empty() { title.trim().to_string() } else { t };
    u.query_pairs_mut()
        .append_pair("query", &format!("releasegroup:{} AND artist:{}", phrase(&t), phrase(artist)))
        .append_pair("limit", "15")
        .append_pair("fmt", "json");
    u.into()
}

pub fn recording_search_url(artist: &str, title: &str, isrc: Option<&str>) -> String {
    let mut u = Url::parse(&format!("{WS}/recording")).expect("static url");
    let q = match isrc.map(str::trim).filter(|i| i.len() == 12 && i.chars().all(|c| c.is_ascii_alphanumeric())) {
        Some(i) => format!("isrc:{}", i.to_ascii_uppercase()),
        None => format!("recording:{} AND artist:{}", phrase(&fold_title(title).title), phrase(artist)),
    };
    u.query_pairs_mut().append_pair("query", &q).append_pair("limit", "10").append_pair("fmt", "json");
    u.into()
}

pub fn artist_search_url(name: &str) -> String {
    crate::artistinfo::musicbrainz::search_url(name)
}

/// Release group ids a search for a song points at, best first.
pub fn groups_of_recordings(v: &Value, want: &Want, title: &str, by_isrc: bool) -> Vec<String> {
    let want_title = fold_title(title).title;
    let mut seen: BTreeMap<String, (u8, bool, String)> = BTreeMap::new();
    for rec in v.get("recordings").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        if !by_isrc {
            let score = rec.get("score").and_then(Value::as_u64).unwrap_or(0);
            let same_title = fold_title(&s(rec, "title")).title == want_title;
            let same_artist = credit(rec.get("artist-credit")).first().is_some_and(|c| fold_artist(&c.name) == want.fold());
            if score < 90 || !same_title || !same_artist {
                continue;
            }
        }
        for r in rec.get("releases").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
            let Some(rg) = r.get("release-group") else { continue };
            let id = s(rg, "id").to_ascii_lowercase();
            let Some(t) = type_rank(opt_lower(rg, "primary-type").as_deref()) else { continue };
            if !is_mbid(&id) {
                continue;
            }
            let key = (t, !lower_list(rg.get("secondary-types")).is_empty(), date_key(&s(r, "date")));
            let e = seen.entry(id).or_insert(key.clone());
            if key < *e {
                *e = key;
            }
        }
    }
    let mut ids: Vec<(String, (u8, bool, String))> = seen.into_iter().collect();
    ids.sort_by(|a, b| a.1.cmp(&b.1));
    ids.into_iter().map(|(id, _)| id).collect()
}

/// Release group ids a search for an album points at (same folded title).
pub fn groups_of_search(v: &Value, title: &str) -> Vec<String> {
    let want = title_forms(title);
    let mut out: Vec<(String, (u8, bool, String))> = Vec::new();
    for rg in v.get("release-groups").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        let id = s(rg, "id").to_ascii_lowercase();
        let have = title_forms(&s(rg, "title"));
        let same = want.iter().zip(have.iter()).any(|(a, b)| !a.is_empty() && a == b);
        if !is_mbid(&id) || !same {
            continue;
        }
        let key = (
            type_rank(opt_lower(rg, "primary-type").as_deref()).unwrap_or(9),
            !lower_list(rg.get("secondary-types")).is_empty(),
            date_key(&s(rg, "first-release-date")),
        );
        out.push((id, key));
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out.into_iter().map(|(id, _)| id).collect()
}

/// The picker over MusicBrainz.
#[derive(Clone)]
pub struct Picker {
    pub mb: Arc<dyn MbGet>,
}

/// What a pick found, or why there's nothing to request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Picked {
    pub group: Option<Group>,
    pub artist: String,
    /// Why there's nothing to request.
    pub refusal: Option<Refusal>,
    pub reason: Option<String>,
}

impl Picked {
    fn from(r: Result<Group, Refusal>, artist: &str) -> Self {
        match r {
            Ok(g) => Picked { group: Some(g), artist: artist.to_string(), refusal: None, reason: None },
            Err(e) => Picked { group: None, artist: artist.to_string(), reason: Some(e.message()), refusal: Some(e) },
        }
    }
}

impl Picker {
    pub async fn group(&self, rgid: &str) -> AppResult<Option<Group>> {
        if !is_mbid(rgid) {
            return Ok(None);
        }
        Ok(self.mb.get(&group_url(&rgid.to_ascii_lowercase())).await?.as_ref().and_then(parse_group))
    }

    /// The artist's MusicBrainz id by an exact name match.
    pub async fn artist_mbid(&self, name: &str) -> AppResult<Option<String>> {
        if name.trim().is_empty() {
            return Ok(None);
        }
        Ok(self.mb.get(&artist_search_url(name)).await?.and_then(|v| crate::artistinfo::musicbrainz::pick(&v, name)))
    }

    /// Every release group with an official release credited to the artist.
    pub async fn artist_groups(&self, mbid: &str) -> AppResult<Vec<Group>> {
        let mut all = BTreeMap::new();
        for page in 0..MAX_PAGES {
            let Some(v) = self.mb.get(&artist_releases_url(mbid, page * 100)).await? else { break };
            let n = v.get("releases").and_then(Value::as_array).map_or(0, Vec::len);
            groups_from_releases(&v, &mut all);
            let total = v.get("release-count").and_then(Value::as_u64).unwrap_or(0) as usize;
            if n < 100 || (page + 1) * 100 >= total {
                break;
            }
        }
        Ok(all.into_values().collect())
    }

    async fn lookup_all(&self, ids: Vec<String>) -> AppResult<Vec<Group>> {
        let mut out = Vec::new();
        for id in ids.into_iter().take(MAX_LOOKUPS) {
            if let Some(g) = self.group(&id).await? {
                out.push(g);
            }
        }
        Ok(out)
    }

    /// An album by its artist and title.
    pub async fn pick_album(&self, want: &Want, title: &str, library: &[LibraryAlbum]) -> AppResult<Picked> {
        let Some(v) = self.mb.get(&group_search_url(&want.name, title)).await? else {
            return Ok(Picked::from(Err(Refusal::NotFound), &want.name));
        };
        let groups = self.lookup_all(groups_of_search(&v, title)).await?;
        // Naming an album asks for it, live or compilation as it may be.
        Ok(Picked::from(pick(groups, want, library, true), &want.name))
    }

    /// The album a song should come from.
    pub async fn pick_song(&self, want: &Want, title: &str, isrc: Option<&str>, library: &[LibraryAlbum]) -> AppResult<Picked> {
        let url = recording_search_url(&want.name, title, isrc);
        let by_isrc = url.contains("isrc%3A");
        let Some(v) = self.mb.get(&url).await? else { return Ok(Picked::from(Err(Refusal::NotFound), &want.name)) };
        let groups = self.lookup_all(groups_of_recordings(&v, want, title, by_isrc)).await?;
        // A live recording may come from a live album.
        let live = fold_title(title).version.contains("live");
        Ok(Picked::from(pick(groups, want, library, live), &want.name))
    }

    /// Check a release group again right before a request.
    pub async fn vet_group(&self, rgid: &str, want: &Want, library: &[LibraryAlbum]) -> AppResult<Group> {
        let g = self.group(rgid).await?.ok_or_else(|| Refusal::NotFound.into_error())?;
        vet(&g, want, library, true).map_err(Refusal::into_error)?;
        Ok(g)
    }
}

#[cfg(test)]
#[path = "mbpick_tests.rs"]
pub(crate) mod tests;
