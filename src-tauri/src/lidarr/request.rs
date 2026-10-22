//! Requests: ask Lidarr for one album, and follow it until it's imported.
//!
//! Before every request the release group is checked against MusicBrainz
//! again (official, the artist's own, no title clash with the library) and
//! the whole setup check runs again; any red finding stops it before a
//! single write. A new artist is added with nothing monitored and new
//! albums off, then read back: anything else Lidarr monitored is turned off
//! and reported, and only then is the album monitored and searched.
//!
//! When Lidarr can't be reached a request waits on this device ("Sends
//! when Platter can reach Lidarr") and a background retry sends it, full
//! check first.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::api::{Album, Artist, Route};
use super::check::{CheckReport, Level};
use super::mbpick::{Group, LibraryAlbum, MissingGroup, Picked, Picker, Want, library_albums, missing_in};
use super::status::{RequestState, map_status};
use super::write::{AddAlbum, AddArtist, clear_exclusion};
use super::paths::NormPath;
use super::{Checked, Lidarr};
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};

const REQUESTS_KEY: &str = "lidarr.requests";
/// Per-artist "only these release groups" lists.
const ALLOW_KEY: &str = "lidarr.allowlists";
/// Set once the user has typed an album's name for their first request.
const TYPED_KEY: &str = "lidarr.typedConfirmed";
pub const QUEUED_NOTE: &str = "Sends when Platter can reach Lidarr.";
/// Monitored in Lidarr, the search not sent yet.
pub const SEARCH_PENDING: &str = "Lidarr has it marked for download. Platter sends the search shortly.";
/// Looks for a newly added artist's album before trying again later.
const ALBUM_TRIES: usize = 30;
/// Most finished requests kept in the list (requests still going are always kept).
pub(super) const MAX_KEPT: usize = 200;
/// Release groups of requests that left the list, so they're never asked for again.
const ASKED_KEY: &str = "lidarr.requests.asked";
const KEEP_ASKED: usize = 5000;
/// Release groups a Bridge asked for that this device took into its list.
const ADOPTED_KEY: &str = "lidarr.requests.fromBridge";
const KEEP_ADOPTED: usize = 2000;
/// Most searches again sent in one look, so a long time away doesn't end in a burst.
pub const MAX_RESEARCH_PER_LOOK: usize = 3;
/// Tries of a waiting request that keeps going wrong (Lidarr answering with
/// an error, or still reading a new artist) before it's marked failed.
pub const MAX_ATTEMPTS: u32 = 12;
/// How often the background retry runs.
pub const RETRY_EVERY: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRecord {
    pub id: String,
    pub release_group_id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub artist_id: Option<String>,
    /// The artist's MusicBrainz id when the request named it (an artist
    /// not in the library, from a catalogue page).
    #[serde(default)]
    pub artist_mbid: Option<String>,
    pub state: RequestState,
    /// Where the files will land, as Lidarr sees it.
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub lidarr_album_id: Option<i64>,
    /// Platter added the artist to Lidarr for this request.
    #[serde(default)]
    pub added_artist: bool,
    /// Lidarr already had the album; nothing was sent.
    #[serde(default)]
    pub already_in_lidarr: bool,
    pub created_at: i64,
    pub updated_at: i64,
    /// Why it's waiting or failed.
    #[serde(default)]
    pub message: Option<String>,
    /// What Platter did along the way (monitoring it turned off...).
    #[serde(default)]
    pub notes: Vec<String>,
    /// Asked for by itself for a followed artist's new release, not by hand.
    #[serde(default)]
    pub auto: bool,
    /// For a protected artist: it lands in the staging folder and waits
    /// there until Platter adds it to the library.
    #[serde(default)]
    pub protected: bool,
    /// How far the download has got (0 to 100) while it's downloading.
    #[serde(default)]
    pub progress: Option<u8>,
    /// When the first search went out; the searches again count from here.
    #[serde(default)]
    pub searched_at: Option<i64>,
    /// Searches sent so far, the first one included.
    #[serde(default)]
    pub searches: u32,
    /// When Lidarr is asked to search again while nothing was grabbed.
    #[serde(default)]
    pub next_search_at: Option<i64>,
    /// Tries of a waiting request that went wrong for a passing reason.
    #[serde(default)]
    pub attempts: u32,
    /// Sent for a tapped song: a try again goes the quick way too.
    #[serde(default)]
    pub quick: bool,
}

/// A request that left the list, remembered so it's never asked for again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asked {
    pub release_group_id: String,
    pub artist: String,
    pub title: String,
    pub state: RequestState,
}

/// When an album nothing was grabbed for is searched again, counted from the
/// first search: an hour, six hours, a day, then once a day for a week.
pub const RESEARCH_AFTER: [i64; 10] = [3600, 6 * 3600, 24 * 3600, 2 * DAY, 3 * DAY, 4 * DAY, 5 * DAY, 6 * DAY, 7 * DAY, 8 * DAY];
const DAY: i64 = 24 * 3600;
/// With nothing grabbed this long after the first search, it's "Not found yet".
pub const GIVE_UP_AFTER: i64 = 9 * DAY;
pub const NOT_FOUND_NOTE: &str = "Nothing turned up on your indexers yet. Search again, or try another edition.";

/// When the next search goes out after `searches` searches, or None when
/// the schedule is used up.
pub fn next_search(searched_at: i64, searches: u32) -> Option<i64> {
    RESEARCH_AFTER.get((searches as usize).checked_sub(1)?).map(|d| searched_at + d)
}

/// The first time on the schedule after `now`, or None when it's used up.
/// Times missed while Platter was closed are skipped, not caught up on.
pub fn next_slot_after(searched_at: i64, now: i64) -> Option<i64> {
    RESEARCH_AFTER.iter().map(|d| searched_at + d).find(|&t| t > now)
}

/// The list as it's kept: every request still going (waiting here, or one
/// Lidarr can still change), then the newest finished ones up to
/// [`MAX_KEPT`] in all. Imported ones leave first, then failed ones, oldest
/// first. Returns what's kept, newest first, and what left.
pub(super) fn keep_records(mut list: Vec<RequestRecord>) -> (Vec<RequestRecord>, Vec<RequestRecord>) {
    list.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    let Some(over) = list.len().checked_sub(MAX_KEPT).filter(|n| *n > 0) else { return (list, Vec::new()) };
    let rank = |r: &RequestRecord| match r.state {
        _ if !r.state.settled() || followed(r) => None,
        RequestState::Imported => Some(0),
        RequestState::Failed => Some(1),
        _ => Some(2),
    };
    let mut out: Vec<(u8, i64, String)> = list.iter().filter_map(|r| rank(r).map(|k| (k, r.created_at, r.id.clone()))).collect();
    out.sort();
    let gone: std::collections::HashSet<String> = out.into_iter().take(over).map(|(_, _, id)| id).collect();
    list.into_iter().partition(|r| !gone.contains(&r.id))
}

/// What one-click Get came to: a request, or why it needs the user.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetOutcome {
    pub record: Option<RequestRecord>,
    pub needs: Option<GetNeeds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetNeeds {
    /// `firstTime` (the first request ever is confirmed by typing the
    /// album's name) or `refused` (a check said no; `reason` says why).
    pub kind: String,
    pub reason: String,
}

/// Most albums one "Request these" sends.
pub const BATCH_MAX: usize = 10;

/// Requests saved on this device, read straight from the database (for
/// lists that show each row's request state).
pub fn saved_requests(c: &rusqlite::Connection) -> Vec<RequestRecord> {
    kv_get(c, REQUESTS_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// The albums of a "Request these", once each, or why not: none, or more
/// than [`BATCH_MAX`].
pub fn batch_inputs(inputs: Vec<RequestInput>) -> AppResult<Vec<RequestInput>> {
    let mut out: Vec<RequestInput> = Vec::new();
    for i in inputs {
        let rg = i.release_group_id.trim().to_ascii_lowercase();
        if !out.iter().any(|x| x.release_group_id.trim().eq_ignore_ascii_case(&rg)) {
            out.push(i);
        }
    }
    if out.is_empty() {
        return Err(AppError::Invalid("Pick at least one album to request.".into()));
    }
    if out.len() > BATCH_MAX {
        return Err(AppError::Invalid(format!("Request up to {BATCH_MAX} albums at a time.")));
    }
    Ok(out)
}

/// How one album of a "Request these" went.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchItem {
    pub release_group_id: String,
    pub title: String,
    pub record: Option<RequestRecord>,
    /// Why it wasn't sent.
    pub error: Option<String>,
}

/// What the UI sends to request (or preview) an album.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestInput {
    pub release_group_id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub artist_id: Option<String>,
    /// The artist's MusicBrainz id, when known (an artist not in the library).
    #[serde(default)]
    pub artist_mbid: Option<String>,
    /// The album's name typed by the user (the first request ever).
    #[serde(default)]
    pub typed: Option<String>,
}

/// What a request will do, shown before it's sent.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub release_group_id: String,
    pub title: String,
    pub artist: String,
    /// Where the files will land.
    pub destination: Option<String>,
    /// From the last check, since Lidarr can't be reached now.
    pub destination_is_guess: bool,
    /// The first request ever: the album's name must be typed.
    pub needs_typed: bool,
    /// Lidarr has the album already: its state; nothing will be sent.
    pub existing: Option<RequestState>,
    /// Lidarr answered; false means the request will wait.
    pub reachable: bool,
    pub route: Option<Route>,
    /// The artist is new to Lidarr and will be added with only this album.
    pub adds_artist: bool,
    /// A protected artist: the album downloads into the staging folder and
    /// waits there until Platter adds it to the library.
    pub protected: bool,
}

/// An artist's missing official releases, for the artist page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingReleases {
    /// Show them (setting on, Lidarr set up, last check green).
    pub shown: bool,
    pub items: Vec<MissingItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingItem {
    pub release_group_id: String,
    pub title: String,
    /// `album`, `ep` or `single`.
    pub kind: String,
    /// Secondary types (`live`, `compilation`...), lower case.
    pub secondary: Vec<String>,
    /// The discography section it shows in (`albums`, `singlesEps`,
    /// `compilations`, `live`...).
    pub section: String,
    pub year: Option<i32>,
    /// A request made for it, if any.
    pub request: Option<RequestState>,
    /// Its front cover from the Cover Art Archive, small, as an art path.
    pub thumb: Option<String>,
    /// Why Get would be refused (a live album or compilation isn't asked
    /// for on its own).
    pub refusal: Option<String>,
}

/// The art path of a release group's small front cover (the Cover Art
/// Archive answers 404 when it has none, and the card keeps its placeholder).
pub fn cover_thumb(rgid: &str) -> Option<String> {
    crate::artistinfo::is_mbid(rgid)
        .then(|| format!("{}https://coverartarchive.org/release-group/{}/front-250", crate::artistinfo::art::EXT_PREFIX, rgid.to_ascii_lowercase()))
}

/// The folded way a typed name is compared.
fn typed_matches(typed: &str, title: &str) -> bool {
    let f = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    !typed.trim().is_empty() && f(typed) == f(title)
}

/// Whether the check lets work go ahead: Managed artists need no red
/// finding that stops everyone; protected artists need no red at all.
pub(super) fn gate(r: &CheckReport, protected: bool) -> AppResult<()> {
    let ok = if protected { r.passed } else { r.managed_ok };
    if ok { Ok(()) } else { Err(red_error(r, protected)) }
}

fn red_error(r: &CheckReport, protected: bool) -> AppError {
    let titles: Vec<&str> =
        r.findings.iter().filter(|f| f.level == Level::Red && (protected || !f.protected_only)).map(|f| f.title.as_str()).collect();
    AppError::Invalid(format!(
        "Platter won't send Lidarr anything while its setup check has red items: {}. Fix them in Lidarr, then check again in Settings.",
        titles.join("; ")
    ))
}

/// Whether a request is for a protected artist: the artist (by MusicBrainz
/// id, the library artist, or the artist Lidarr credits) or the release
/// group is protected.
pub(super) fn is_protected(modes: &[super::modes::ArtistMode], want: &Want, rgid: &str, foreign: Option<&str>) -> bool {
    let rg = rgid.trim().to_ascii_lowercase();
    modes.iter().filter(|m| m.protected()).any(|m| {
        let by_id = |id: Option<&str>| id.is_some_and(|x| m.mbid.as_deref().is_some_and(|y| y.eq_ignore_ascii_case(x.trim())));
        by_id(want.mbid.as_deref())
            || by_id(foreign)
            || (want.artist_id.is_some() && m.artist_id == want.artist_id)
            || m.release_groups.contains(&rg)
    })
}

fn join_folder(root: &str, name: &str) -> String {
    let sep = if root.contains('\\') && !root.contains('/') { '\\' } else { '/' };
    format!("{}{sep}{}", root.trim_end_matches(['/', '\\']), name.trim_matches(['/', '\\']))
}

/// How a request goes to Lidarr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    /// Lidarr reads the artist, then searches the album itself.
    Usual,
    /// Someone is waiting to hear it: only the album goes in, and Platter
    /// grabs the fastest good release itself (`quick`).
    Quick,
}

/// Where a request's files go.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Dest {
    /// The library's download folder (any root folder but the staging one).
    Library,
    /// The staging folder, for a better copy of a library album.
    Staging { album_id: String, current: super::quality::Rank, current_label: String },
}

/// What sending needs from Lidarr, all read.
struct Plan {
    group: Group,
    existing: Option<Album>,
    /// The artist as Lidarr has it, or as its lookup found it.
    artist: Artist,
    in_lidarr: bool,
    root: Option<super::api::RootFolder>,
    destination: String,
}

impl Lidarr {
    pub(super) fn records(&self) -> Vec<RequestRecord> {
        self.inner.db.read(|c| kv_get(c, REQUESTS_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub(super) fn save_records(&self, list: &[RequestRecord]) -> AppResult<()> {
        let (list, gone) = keep_records(list.to_vec());
        if !gone.is_empty() {
            let mut asked = self.asked_archive();
            for r in gone {
                asked.retain(|a| a.release_group_id != r.release_group_id);
                asked.push(Asked { release_group_id: r.release_group_id, artist: r.artist, title: r.title, state: r.state });
            }
            if asked.len() > KEEP_ASKED {
                asked.drain(..asked.len() - KEEP_ASKED);
            }
            let json = serde_json::to_string(&asked).map_err(|e| AppError::Internal(e.to_string()))?;
            self.inner.db.write(|c| kv_set(c, ASKED_KEY, &json))?;
        }
        let json = serde_json::to_string(&list).map_err(|e| AppError::Internal(e.to_string()))?;
        self.inner.db.write(|c| kv_set(c, REQUESTS_KEY, &json))
    }

    /// Requests that left the list.
    pub(super) fn asked_archive(&self) -> Vec<Asked> {
        self.inner.db.read(|c| kv_get(c, ASKED_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Everything asked for: the list, and what left it.
    pub(super) fn asked_all(&self) -> Vec<Asked> {
        let mut all = self.asked_archive();
        all.extend(self.records().into_iter().map(|r| Asked { release_group_id: r.release_group_id, artist: r.artist, title: r.title, state: r.state }));
        all
    }

    /// Whether a release group was asked for before and didn't fail.
    pub(super) fn asked_before(&self, rgid: &str) -> bool {
        self.asked_all().iter().any(|a| a.release_group_id.eq_ignore_ascii_case(rgid.trim()) && a.state != RequestState::Failed)
    }

    /// Release groups asked for before that didn't fail.
    pub(super) fn asked_groups(&self) -> Vec<String> {
        let mut v: Vec<String> = self.asked_all().into_iter().filter(|a| a.state != RequestState::Failed).map(|a| a.release_group_id).collect();
        v.sort();
        v.dedup();
        v
    }

    pub(super) fn upsert(&self, r: &RequestRecord) -> AppResult<()> {
        let mut list = self.records();
        match list.iter_mut().find(|x| x.id == r.id) {
            Some(x) => *x = r.clone(),
            None => list.push(r.clone()),
        }
        self.save_records(&list)
    }

    fn allowlists(&self) -> std::collections::HashMap<String, Vec<String>> {
        self.inner.db.read(|c| kv_get(c, ALLOW_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// The key an artist's allowlist is kept under: its MusicBrainz id, else its folded name.
    fn allow_key(want: &Want) -> String {
        match &want.mbid {
            Some(m) => m.to_ascii_lowercase(),
            None => format!("name:{}", want.fold()),
        }
    }

    /// Whether the artist's allowlist (when the user set one) has this group.
    pub(super) fn allowed(&self, want: &Want, rgid: &str) -> bool {
        match self.allowlists().get(&Self::allow_key(want)) {
            Some(l) if !l.is_empty() => l.iter().any(|x| x.eq_ignore_ascii_case(rgid)),
            _ => true,
        }
    }

    /// The release groups an artist is limited to (empty: no limit).
    pub async fn allowlist(&self, artist: &str, artist_id: Option<&str>) -> Vec<String> {
        let want = self.want(artist, artist_id, None).await;
        self.allowlists().get(&Self::allow_key(&want)).cloned().unwrap_or_default()
    }

    /// Limit an artist to these release groups (empty removes the limit).
    pub async fn set_allowlist(&self, artist: &str, artist_id: Option<&str>, rgids: Vec<String>) -> AppResult<Vec<String>> {
        let want = self.want(artist, artist_id, None).await;
        let mut ids: Vec<String> = rgids.into_iter().map(|r| r.trim().to_ascii_lowercase()).filter(|r| crate::artistinfo::is_mbid(r)).collect();
        ids.sort();
        ids.dedup();
        let mut all = self.allowlists();
        if ids.is_empty() {
            all.remove(&Self::allow_key(&want));
        } else {
            all.insert(Self::allow_key(&want), ids.clone());
        }
        let json = serde_json::to_string(&all).map_err(|e| AppError::Internal(e.to_string()))?;
        self.inner.db.write(|c| kv_set(c, ALLOW_KEY, &json))?;
        Ok(ids)
    }

    /// The staging folder setting, folded; None when upgrades are off.
    pub(super) fn staging_norm(&self) -> Option<NormPath> {
        let s = crate::settings::load(&self.inner.db).lidarr_staging_folder;
        Some(NormPath::parse(&s)).filter(|n| !n.is_empty())
    }

    /// Plex matched an album in the library to this release group.
    pub(super) fn library_has_group(&self, rgid: &str) -> bool {
        let rg = rgid.trim().to_ascii_lowercase();
        self.inner
            .db
            .read(|c| Ok(c.query_row("SELECT EXISTS (SELECT 1 FROM albums WHERE lower(mbid) = ?1)", [&rg], |r| r.get::<_, bool>(0))?))
            .unwrap_or(false)
    }

    pub fn typed_confirmed(&self) -> bool {
        self.inner.db.read(|c| kv_get(c, TYPED_KEY)).ok().flatten().is_some()
    }

    pub(super) fn picker(&self) -> Picker {
        Picker { mb: self.inner.mb.clone() }
    }

    /// The artist a request is for, with its MusicBrainz id when known.
    pub(super) async fn want(&self, artist: &str, artist_id: Option<&str>, artist_mbid: Option<&str>) -> Want {
        let named = artist_mbid.map(|m| m.trim().to_ascii_lowercase()).filter(|m| crate::artistinfo::is_mbid(m));
        if named.is_some() {
            return Want { name: artist.to_string(), mbid: named, artist_id: artist_id.map(str::to_string) };
        }
        let mbid = match artist_id {
            Some(id) => self.inner.artist_mbid.artist_mbid(id, artist).await,
            None => None,
        };
        let mbid = match mbid {
            Some(m) => Some(m),
            None => self.picker().artist_mbid(artist).await.ok().flatten(),
        };
        Want { name: artist.to_string(), mbid, artist_id: artist_id.map(str::to_string) }
    }

    pub(super) async fn library(&self, want: &Want) -> AppResult<Vec<LibraryAlbum>> {
        let (id, name) = (want.artist_id.clone(), want.name.clone());
        self.inner.db.read_async(move |c| library_albums(c, id.as_deref(), &name)).await
    }

    /// Which release group to request for an album or a song.
    pub async fn pick(&self, artist: &str, artist_id: Option<&str>, title: &str, song: bool, isrc: Option<&str>) -> AppResult<Picked> {
        let want = self.want(artist, artist_id, None).await;
        let library = self.library(&want).await?;
        if song { self.picker().pick_song(&want, title, isrc, &library).await } else { self.picker().pick_album(&want, title, &library).await }
    }

    /// Read what a request needs from Lidarr.
    async fn plan(&self, c: &Checked, group: Group, dest: &Dest, protected: bool) -> AppResult<Plan> {
        let client = &c.client;
        let rgid = group.id.clone();
        let existing = client.album_by_release_group(&rgid).await?;
        let staging = self.staging_norm();
        let is_staging = |r: &super::api::RootFolder| staging.as_ref().is_some_and(|s| NormPath::parse(&r.path) == *s);
        // Managed artists download into a library root, never the staging
        // folder; protected artists only ever into the staging folder.
        let to_staging = protected || matches!(dest, Dest::Staging { .. });
        let root = if to_staging {
            c.snap.roots.iter().find(|r| r.accessible && is_staging(r)).cloned()
        } else {
            c.snap.roots.iter().filter(|r| r.accessible && !is_staging(r)).min_by_key(|r| r.id).cloned()
        };
        if to_staging && root.is_none() {
            return Err(AppError::Invalid("Lidarr doesn't have the staging folder as a root folder it can open. Check it in Lidarr, then run the setup check again.".into()));
        }
        if let Some(a) = &existing {
            let artist = c.snap.artists.iter().find(|x| x.id == a.artist_id).cloned().or_else(|| a.artist.clone()).unwrap_or_default();
            let destination = artist.path.clone();
            return Ok(Plan { group, existing, artist, in_lidarr: true, root, destination });
        }
        let found = client.album_lookup(&rgid).await?;
        let album = found
            .iter()
            .find(|a| a.foreign_album_id.eq_ignore_ascii_case(&rgid))
            .or(found.first())
            .cloned()
            .ok_or_else(|| AppError::NotFound("Lidarr doesn't know this album. Its metadata may catch up later; try again in a day or two.".into()))?;
        let looked = album.artist.clone().unwrap_or_default();
        if looked.foreign_artist_id.trim().is_empty() {
            return Err(AppError::NotFound("Lidarr doesn't know who made this album. Try again later.".into()));
        }
        let have = c.snap.artists.iter().find(|x| x.foreign_artist_id.eq_ignore_ascii_case(&looked.foreign_artist_id)).cloned();
        let (artist, in_lidarr) = match have {
            Some(a) => (a, true),
            None => (looked, false),
        };
        let destination = if in_lidarr {
            artist.path.clone()
        } else {
            let root = root.as_ref().ok_or_else(|| AppError::Invalid("Lidarr has no root folder to download into. Add a separate one in Lidarr first.".into()))?;
            let folder = artist.folder.clone().filter(|f| !f.trim().is_empty()).unwrap_or_else(|| artist.artist_name.clone());
            join_folder(&root.path, &folder)
        };
        Ok(Plan { group, existing: None, artist, in_lidarr, root, destination })
    }

    async fn existing_state(c: &Checked, a: &Album) -> RequestState {
        let queue = c.client.queue(&[a.id]).await.unwrap_or_default();
        let history = c.client.history(a.id).await.unwrap_or_default();
        map_status(a, &queue, &history)
    }

    /// Where a waiting request will land, from the last check.
    fn guessed_destination(&self, artist: &str) -> Option<String> {
        let staging = self.staging_norm();
        self.last_check()
            .and_then(|l| l.roots.into_iter().find(|r| staging.as_ref().is_none_or(|s| NormPath::parse(r) != *s)))
            .map(|r| join_folder(&r, artist))
    }

    /// What a request will do. Reads only.
    pub async fn preview(&self, input: &RequestInput) -> AppResult<Preview> {
        let want = self.want(&input.artist, input.artist_id.as_deref(), input.artist_mbid.as_deref()).await;
        let library = self.library(&want).await?;
        let group = self.picker().vet_group(&input.release_group_id, &want, &library).await?;
        let mut p = Preview {
            release_group_id: group.id.clone(),
            title: group.title.clone(),
            artist: input.artist.clone(),
            destination: None,
            destination_is_guess: false,
            needs_typed: !self.typed_confirmed(),
            existing: None,
            reachable: false,
            route: None,
            adds_artist: false,
            protected: false,
        };
        let c = match self.check_full().await {
            Ok(c) => c,
            Err(e) if e.kind() == "network" => {
                p.destination = self.guessed_destination(&input.artist);
                p.destination_is_guess = true;
                return Ok(p);
            }
            Err(e) => return Err(e),
        };
        gate(&c.report, false)?;
        let mut protected = is_protected(&c.modes, &want, &group.id, None);
        gate(&c.report, protected)?;
        let mut plan = self.plan(&c, group.clone(), &Dest::Library, protected).await?;
        if !protected && is_protected(&c.modes, &want, &group.id, Some(&plan.artist.foreign_artist_id)) {
            protected = true;
            gate(&c.report, true)?;
            plan = self.plan(&c, group, &Dest::Library, true).await?;
        }
        p.protected = protected;
        p.reachable = true;
        p.route = c.report.route;
        p.destination = Some(plan.destination.clone());
        p.adds_artist = !plan.in_lidarr;
        if let Some(a) = &plan.existing
            && (a.monitored || Self::existing_state(&c, a).await == RequestState::Imported)
        {
            p.existing = Some(Self::existing_state(&c, a).await);
        }
        Ok(p)
    }

    /// Request an album. Sends now, or waits when Lidarr can't be reached.
    pub async fn request(&self, input: RequestInput) -> AppResult<RequestRecord> {
        self.request_with(input, Speed::Usual).await
    }

    /// [`Self::request`] at either speed.
    pub(super) async fn request_with(&self, input: RequestInput, speed: Speed) -> AppResult<RequestRecord> {
        let _one = self.lock.lock().await;
        if !self.typed_confirmed() && !typed_matches(input.typed.as_deref().unwrap_or_default(), &input.title) {
            return Err(AppError::Invalid(format!("Type the album's name, {}, to send your first request.", input.title)));
        }
        let rgid = input.release_group_id.trim().to_ascii_lowercase();
        if let Some(r) = self.records().into_iter().find(|r| r.release_group_id == rgid && r.state != RequestState::Failed) {
            return Ok(r);
        }
        let rec = RequestRecord { quick: speed == Speed::Quick, ..self.new_record(&input, rgid, false) };
        let out = match self.send_to(rec.clone(), &Dest::Library, speed).await {
            Ok(r) => r,
            Err(e) if e.kind() == "network" => {
                let mut r = rec;
                r.destination = self.guessed_destination(&r.artist);
                r
            }
            Err(e) => return Err(e),
        };
        self.upsert(&out)?;
        self.inner.db.write(|c| kv_set(c, TYPED_KEY, "1"))?;
        Ok(out)
    }

    /// "Request these": up to [`BATCH_MAX`] albums the user confirmed at
    /// once, each sent like a single request (full check and every guard,
    /// one after another). One that fails doesn't stop the rest.
    pub async fn request_batch(&self, inputs: Vec<RequestInput>) -> AppResult<Vec<BatchItem>> {
        let inputs = batch_inputs(inputs)?;
        let mut out = Vec::with_capacity(inputs.len());
        for input in inputs {
            let (release_group_id, title) = (input.release_group_id.clone(), input.title.clone());
            match self.request(input).await {
                Ok(r) => out.push(BatchItem { release_group_id, title, record: Some(r), error: None }),
                Err(e) => out.push(BatchItem { release_group_id, title, record: None, error: Some(e.message()) }),
            }
        }
        Ok(out)
    }

    pub(super) fn new_record(&self, input: &RequestInput, rgid: String, auto: bool) -> RequestRecord {
        let now = (self.inner.clock)();
        RequestRecord {
            id: uuid::Uuid::new_v4().to_string(),
            release_group_id: rgid,
            title: input.title.trim().to_string(),
            artist: input.artist.trim().to_string(),
            artist_id: input.artist_id.clone(),
            artist_mbid: input.artist_mbid.clone(),
            state: RequestState::Queued,
            destination: None,
            lidarr_album_id: None,
            added_artist: false,
            already_in_lidarr: false,
            created_at: now,
            updated_at: now,
            message: Some(QUEUED_NOTE.into()),
            notes: Vec::new(),
            auto,
            protected: false,
            progress: None,
            searched_at: None,
            searches: 0,
            next_search_at: None,
            attempts: 0,
            quick: false,
        }
    }

    /// One-click Get: straight into [`Self::request`] with every check. Only
    /// the very first request ever, or a check that says no, needs the user
    /// (the request dialog, with the reason).
    pub async fn get(&self, input: RequestInput) -> AppResult<GetOutcome> {
        let needs = |kind: &str, reason: String| Ok(GetOutcome { record: None, needs: Some(GetNeeds { kind: kind.into(), reason }) });
        if !self.typed_confirmed() && input.typed.is_none() {
            return needs("firstTime", "Your first request is confirmed by typing the album's name, once.".into());
        }
        tracing::info!(release_group = %input.release_group_id, title = %input.title, "lidarr get: sent");
        match self.request(input).await {
            Ok(r) => Ok(GetOutcome { record: Some(r), needs: None }),
            Err(e) if matches!(e.kind(), "invalid" | "notFound") => {
                tracing::info!(reason = %e.message(), "lidarr get: needs you");
                needs("refused", e.message())
            }
            Err(e) => Err(e),
        }
    }

    /// An automatic request for a new release (the artist is followed,
    /// which stands in for the typed name). The same MusicBrainz
    /// guards and the same full setup check run before anything is sent.
    /// A release asked for before (by hand or automatically, whatever came
    /// of it) is never asked for again. Anything refused stays in the list
    /// as failed, with why; a red setup check or an unreachable Lidarr
    /// keeps it waiting, and the background retry checks everything again.
    pub async fn request_auto(&self, input: RequestInput) -> AppResult<RequestRecord> {
        let _one = self.lock.lock().await;
        let rgid = input.release_group_id.trim().to_ascii_lowercase();
        if let Some(r) = self.records().into_iter().find(|r| r.release_group_id == rgid) {
            return Ok(r);
        }
        // Asked for long ago (it has left the list): shown as it ended, not sent.
        if let Some(a) = self.asked_archive().into_iter().find(|a| a.release_group_id == rgid) {
            let mut r = self.new_record(&input, rgid, true);
            r.state = a.state;
            r.message = Some("Asked for before, so Platter didn't send it again.".into());
            return Ok(r);
        }
        let rec = self.new_record(&input, rgid, true);
        let out = match self.send(rec.clone()).await {
            Ok(r) => r,
            Err(e) if e.kind() == "network" => RequestRecord { destination: self.guessed_destination(&rec.artist), ..rec },
            Err(e) => self.after_send_error(rec, &e),
        };
        self.upsert(&out)?;
        Ok(out)
    }

    /// A waiting or automatic request whose send went wrong: it keeps
    /// waiting when the reason may pass (Lidarr out of reach, a red setup
    /// check, Lidarr answering with an error), and fails when a check
    /// refused it or it went wrong [`MAX_ATTEMPTS`] times.
    pub(super) fn after_send_error(&self, rec: RequestRecord, e: &AppError) -> RequestRecord {
        let now = (self.inner.clock)();
        let msg = e.to_string();
        match e.kind() {
            "network" => RequestRecord { state: RequestState::Queued, message: Some(QUEUED_NOTE.into()), updated_at: now, ..rec },
            _ if msg.contains("red items") => RequestRecord { state: RequestState::Queued, message: Some(msg), updated_at: now, ..rec },
            "invalid" | "notFound" => RequestRecord { state: RequestState::Failed, message: Some(msg), updated_at: now, ..rec },
            _ => self.another_try(rec, msg),
        }
    }

    /// One more try that went wrong: waiting again, or failed after [`MAX_ATTEMPTS`].
    fn another_try(&self, rec: RequestRecord, why: String) -> RequestRecord {
        let now = (self.inner.clock)();
        let attempts = rec.attempts + 1;
        if attempts >= MAX_ATTEMPTS {
            tracing::info!(title = %rec.title, attempts, "lidarr request: failed after repeated tries");
            let message = Some(format!("Platter tried {attempts} times and gave up. The last try said: {why} Get it again to try once more."));
            return RequestRecord { state: RequestState::Failed, attempts, message, updated_at: now, ..rec };
        }
        let message = if why.ends_with("shortly.") { why } else { format!("{why} Platter tries again shortly.") };
        RequestRecord { state: RequestState::Queued, attempts, message: Some(message), updated_at: now, ..rec }
    }

    /// Send one request: check MusicBrainz and the whole setup again, then
    /// the writes. Network failures come back as errors so the caller keeps
    /// the request waiting.
    pub(super) async fn send(&self, rec: RequestRecord) -> AppResult<RequestRecord> {
        self.send_to(rec, &Dest::Library, Speed::Usual).await
    }

    /// [`Self::send`] to the library's download folder, or to the staging
    /// folder for an upgrade (with a quality profile aiming above the
    /// files there now).
    ///
    /// [`Speed::Quick`] (a song someone tapped to hear now) adds only the
    /// album, with its artist when new and without reading the artist's
    /// other albums, and grabs the fastest good release itself instead of
    /// leaving the search to Lidarr. Every check is the same.
    pub(super) async fn send_to(&self, mut rec: RequestRecord, dest: &Dest, speed: Speed) -> AppResult<RequestRecord> {
        tracing::info!(release_group = %rec.release_group_id, title = %rec.title, "lidarr request: checking");
        let want = self.want(&rec.artist, rec.artist_id.as_deref(), rec.artist_mbid.as_deref()).await;
        let library = self.library(&want).await?;
        let group = match dest {
            Dest::Library => self.picker().vet_group(&rec.release_group_id, &want, &library).await?,
            Dest::Staging { album_id, .. } => self.picker().vet_upgrade_group(&rec.release_group_id, &want, &library, album_id).await?,
        };
        if !self.allowed(&want, &group.id) {
            return Err(AppError::Invalid(super::mbpick::Refusal::NotAllowed.message()));
        }
        if !matches!(dest, Dest::Staging { .. }) && self.library_has_group(&group.id) {
            return Err(AppError::Invalid(format!("{} is already in your library, so Platter didn't ask Lidarr for it.", group.title)));
        }
        let c = self.check_full().await?;
        gate(&c.report, false)?;
        let upgrade = matches!(dest, Dest::Staging { .. });
        let mut protected = upgrade || is_protected(&c.modes, &want, &group.id, None);
        gate(&c.report, protected)?;
        let mut plan = self.plan(&c, group.clone(), dest, protected).await?;
        if !protected && is_protected(&c.modes, &want, &group.id, Some(&plan.artist.foreign_artist_id)) {
            protected = true;
            gate(&c.report, true)?;
            plan = self.plan(&c, group, dest, true).await?;
        }
        rec.protected = protected && !upgrade;
        let staging = self.staging_norm();
        let in_staging = |path: &str| staging.as_ref().is_some_and(|s| NormPath::parse(path).starts_with(s));
        let mut quality_choice: Option<i64> = None;
        match dest {
            Dest::Library if protected => {
                if plan.in_lidarr && !in_staging(&plan.artist.path) {
                    return Err(AppError::Invalid(format!(
                        "{} is protected, but Lidarr keeps it in {}, outside the staging folder. Platter didn't send anything. In Lidarr, set the artist's path inside the staging folder.",
                        plan.artist.artist_name, plan.artist.path
                    )));
                }
            }
            Dest::Library => {
                if plan.in_lidarr && in_staging(&plan.artist.path) {
                    return Err(AppError::Invalid(format!(
                        "Lidarr keeps {} in the staging folder for upgrades, so a request would never reach your library. Platter didn't send it.",
                        plan.artist.artist_name
                    )));
                }
            }
            Dest::Staging { current, current_label, .. } => {
                if plan.in_lidarr && !in_staging(&plan.artist.path) {
                    return Err(AppError::Invalid(format!(
                        "Lidarr already has {} in {}, outside the staging folder, so a better copy would land there instead. Platter didn't send anything.",
                        plan.artist.artist_name, plan.artist.path
                    )));
                }
                if plan.in_lidarr {
                    let p = plan.artist.quality_profile_id.and_then(|id| c.snap.profiles.iter().find(|p| p.id == id));
                    if !p.is_some_and(|p| super::quality::qualifies(p, *current)) {
                        return Err(AppError::Invalid(format!(
                            "{} uses the {} profile in Lidarr, which doesn't aim higher than {current_label}. Platter never changes profiles, so it didn't send anything.",
                            plan.artist.artist_name,
                            p.map(|p| p.name.as_str()).unwrap_or("unknown")
                        )));
                    }
                } else {
                    let choice = super::quality::choose_profile(&c.snap.profiles, *current, current_label).map_err(AppError::Invalid)?;
                    rec.notes.push(format!("Asked Lidarr for {} or better with its {} profile.", choice.cutoff, choice.name));
                    quality_choice = Some(choice.id);
                }
            }
        }
        rec.destination = Some(plan.destination.clone());
        rec.title = plan.group.title.clone();
        rec.updated_at = (self.inner.clock)();

        // Lidarr has it already, and not from this request: show it, send nothing.
        if let Some(a) = &plan.existing
            && rec.lidarr_album_id != Some(a.id)
        {
            let state = Self::existing_state(&c, a).await;
            if a.monitored || state == RequestState::Imported {
                rec.already_in_lidarr = true;
                rec.lidarr_album_id = Some(a.id);
                rec.state = state;
                rec.message = Some("Lidarr already has this album, so Platter didn't send it again.".into());
                return Ok(rec);
            }
        }

        let quick = speed == Speed::Quick && plan.existing.is_none();
        let mut quick_album: Option<Album> = None;
        let artist_id = match (plan.in_lidarr, plan.existing.as_ref()) {
            (_, Some(a)) => a.artist_id,
            (true, None) if !quick => plan.artist.id,
            (in_lidarr, None) => {
                let root = plan.root.as_ref().ok_or_else(|| AppError::Invalid("Lidarr has no root folder to download into.".into()))?;
                let quality = quality_choice
                    .or(root.default_quality_profile_id)
                    .or_else(|| c.snap.profiles.iter().find(|p| !p.upgrade_allowed).or(c.snap.profiles.first()).map(|p| p.id))
                    .ok_or_else(|| AppError::Invalid("Lidarr has no quality profile.".into()))?;
                let metadata = match root.default_metadata_profile_id {
                    Some(m) => m,
                    None => c
                        .client
                        .metadata_profiles()
                        .await?
                        .first()
                        .map(|p| p.id)
                        .ok_or_else(|| AppError::Invalid("Lidarr has no metadata profile.".into()))?,
                };
                // Lidarr's exclusions as they are right now, checked by id.
                let exclusions = c.client.exclusions().await?;
                let cleared = clear_exclusion(&exclusions, &plan.artist.foreign_artist_id, &plan.artist.artist_name)?;
                let payload = if protected {
                    AddArtist::protected(cleared, &plan.artist.artist_name, quality, metadata, &root.path)
                } else {
                    AddArtist::managed(cleared, &plan.artist.artist_name, quality, metadata, &root.path)
                };
                if quick {
                    // Just this album, read back at once: no discography to wait for.
                    let id = c.writer.add_album(AddAlbum::new(payload, &rec.release_group_id, &plan.group.title)).await?;
                    let a = c
                        .client
                        .album_by_release_group(&rec.release_group_id)
                        .await?
                        .filter(|a| a.id == id)
                        .ok_or_else(|| AppError::Server("Lidarr didn't show the album it just added. Nothing more was sent.".into()))?;
                    tracing::info!(album = id, artist = a.artist_id, new_artist = !in_lidarr, title = %rec.title, "lidarr request: album added on its own");
                    if !in_lidarr {
                        rec.added_artist = true;
                        rec.notes.push(format!("Added {} to Lidarr with only this album.", plan.artist.artist_name));
                    }
                    let artist = a.artist_id;
                    quick_album = Some(a);
                    artist
                } else {
                    let id = c.writer.add_artist(payload).await?;
                    tracing::info!(artist = id, title = %rec.title, "lidarr request: artist added with nothing monitored");
                    rec.added_artist = true;
                    rec.notes.push(format!("Added {} to Lidarr with only this album to download.", plan.artist.artist_name));
                    id
                }
            }
        };

        // A new artist's albums take Lidarr a moment to read.
        let mut album: Option<Album> = quick_album.clone().or_else(|| plan.existing.clone());
        let mut albums: Vec<Album> = quick_album.iter().cloned().collect();
        for i in 0..ALBUM_TRIES {
            if quick_album.is_some() {
                break;
            }
            albums = c.client.albums_of(artist_id).await?;
            if let Some(a) = albums.iter().find(|a| a.foreign_album_id.eq_ignore_ascii_case(&rec.release_group_id)) {
                album = Some(a.clone());
                break;
            }
            if i + 1 < ALBUM_TRIES {
                tokio::time::sleep(self.inner.album_wait).await;
            }
        }
        let Some(mut album) = album else {
            rec.state = RequestState::Queued;
            rec.message = Some("Lidarr is still reading the artist's albums. Platter will finish the request shortly.".into());
            return Ok(rec);
        };

        // A new artist's refresh keeps running after the album appears: until
        // it ends, the album has no length (a search rejects everything as
        // "duration is 0") and the refresh's own "monitor none" can undo the
        // monitoring. So wait for it to finish before monitoring or searching.
        if rec.added_artist && quick_album.is_none() {
            let mut ready = false;
            for i in 0..ALBUM_TRIES {
                let refreshing = match c.client.commands().await {
                    Ok(cmds) => cmds.iter().any(|x| x.refreshing(artist_id)),
                    Err(e) => {
                        tracing::warn!(error = %e, "lidarr request: couldn't read Lidarr's commands; going by the album alone");
                        false
                    }
                };
                albums = c.client.albums_of(artist_id).await?;
                if let Some(a) = albums.iter().find(|a| a.id == album.id) {
                    album = a.clone();
                }
                if !refreshing && album.duration != Some(0) {
                    ready = true;
                    break;
                }
                if i + 1 < ALBUM_TRIES {
                    tokio::time::sleep(self.inner.album_wait).await;
                }
            }
            if ready {
                tracing::info!(album = album.id, "lidarr request: waited for Lidarr's refresh of the new artist");
            } else {
                tracing::info!(album = album.id, "lidarr request: artist still refreshing, queued");
                rec.state = RequestState::Queued;
                rec.message = Some("Lidarr is still reading the artist's albums. Platter will finish the request shortly.".into());
                return Ok(rec);
            }
        }

        // Read back: only this album may be monitored on an artist Platter
        // added, and the artist's folder must not be in a Plex music folder.
        if rec.added_artist {
            let a = c.client.artist(artist_id).await?;
            let ap = NormPath::parse(&a.path);
            if protected && !in_staging(&a.path) {
                rec.state = RequestState::Failed;
                rec.message = Some(format!(
                    "Lidarr put protected {} in {}, outside the staging folder. Platter stopped before marking anything for download. In Lidarr, set the artist's path inside the staging folder.",
                    a.artist_name, a.path
                ));
                return Ok(rec);
            }
            if let Some(l) = c.plex_locations.iter().find(|l| protected && !ap.is_empty() && ap.starts_with(&NormPath::parse(l))) {
                rec.state = RequestState::Failed;
                rec.message = Some(format!(
                    "Lidarr put {} in {}, inside your Plex music folder ({l}). Platter stopped before marking anything for download. Remove the artist in Lidarr (leave its files) and check the root folder.",
                    a.artist_name, a.path
                ));
                return Ok(rec);
            }
            if a.monitor_new_items.as_deref().is_some_and(|m| !m.eq_ignore_ascii_case("none")) {
                rec.notes.push(format!(
                    "Lidarr set {} to download new albums too. Turn that off in Lidarr (Edit artist > Monitor new albums: None).",
                    a.artist_name
                ));
            }
            let others: Vec<&Album> = albums.iter().filter(|x| x.monitored && x.id != album.id).collect();
            if !others.is_empty() {
                let names: Vec<&str> = others.iter().map(|x| x.title.as_str()).collect();
                c.writer.monitor(others.iter().map(|x| x.id).collect(), false).await?;
                rec.notes.push(format!(
                    "Lidarr had also marked {} other album{} for download ({}). Platter turned {} off.",
                    others.len(),
                    if others.len() == 1 { "" } else { "s" },
                    names.join(", "),
                    if others.len() == 1 { "it" } else { "them" }
                ));
            }
        }
        // The edition the request goes ahead with; Lidarr may switch it.
        let planned = self.release_now(&c.client, album.id).await?.1.map(|r| r.id);
        c.writer.monitor(vec![album.id], true).await?;
        tracing::info!(album = album.id, "lidarr request: monitored");
        rec.lidarr_album_id = Some(album.id);
        if !matches!(dest, Dest::Library) {
            return self.search_step(&c, rec, album.id, artist_id, planned, protected, speed).await;
        }
        // Lidarr has it marked for download now: the request is kept from
        // here, and a search that doesn't go out is tried again.
        rec.state = RequestState::Queued;
        rec.message = Some(SEARCH_PENDING.into());
        self.upsert(&rec)?;
        match self.search_step(&c, rec.clone(), album.id, artist_id, planned, protected, speed).await {
            Ok(r) => Ok(r),
            Err(e) => {
                tracing::info!(album = album.id, error = %e, "lidarr request: the search didn't go out; tries again");
                rec.message = Some(format!("{SEARCH_PENDING} {}", e.message()));
                Ok(rec)
            }
        }
    }

    /// After the album is monitored: read back, check everything again, and search.
    #[allow(clippy::too_many_arguments)]
    async fn search_step(
        &self,
        c: &Checked,
        mut rec: RequestRecord,
        album_id: i64,
        artist_id: i64,
        planned: Option<i64>,
        protected: bool,
        speed: Speed,
    ) -> AppResult<RequestRecord> {
        let after = c.client.albums_of(artist_id).await?;
        let target_on = after.iter().any(|x| x.id == album_id && x.monitored);
        let extra = rec.added_artist && after.iter().any(|x| x.id != album_id && x.monitored);
        if !target_on || extra {
            rec.state = RequestState::Failed;
            rec.message = Some(if !target_on {
                "Lidarr didn't mark the album for download, so Platter didn't search for it.".into()
            } else {
                "Lidarr kept other albums by this artist marked for download, so Platter stopped before searching. Check the artist in Lidarr.".into()
            });
            tracing::info!(album = album_id, reason = rec.message.as_deref().unwrap_or_default(), "lidarr request: failed");
            return Ok(rec);
        }
        // The whole check again after the writes, before anything is searched.
        let after = self.check_full().await?;
        if let Err(e) = gate(&after.report, protected) {
            rec.state = RequestState::Failed;
            rec.message = Some(format!("{e} Platter stopped before searching."));
            return Ok(rec);
        }
        if let Err(e) = self.same_release(&c.client, album_id, planned, "Platter didn't search for it. It's still marked for download in Lidarr.").await {
            if e.kind() == "network" {
                return Err(e);
            }
            rec.state = RequestState::Failed;
            rec.message = Some(e.to_string());
            return Ok(rec);
        }
        let now = (self.inner.clock)();
        if speed == Speed::Quick {
            return self.grab_step(&after, rec, album_id, artist_id, now).await;
        }
        c.writer.search(vec![album_id]).await?;
        tracing::info!(album = album_id, added_artist = rec.added_artist, "lidarr request: search sent");
        rec.state = RequestState::Searching;
        rec.message = None;
        rec.searched_at = Some(now);
        rec.searches = 1;
        rec.next_search_at = next_search(now, 1);
        Ok(rec)
    }

    /// Send the requests waiting on this device. Each runs the full check.
    /// The lock is taken for one request at a time, so a status look or a
    /// cancel isn't held up behind the whole list.
    pub async fn retry_queued(&self) -> AppResult<usize> {
        let waiting: Vec<String> = self.records().into_iter().filter(|r| r.state == RequestState::Queued).map(|r| r.id).collect();
        let mut sent = 0;
        for id in waiting {
            let _one = self.lock.lock().await;
            let Some(rec) = self.records().into_iter().find(|r| r.id == id && r.state == RequestState::Queued) else { continue };
            let speed = if rec.quick { Speed::Quick } else { Speed::Usual };
            let next = match self.send_to(rec.clone(), &Dest::Library, speed).await {
                Ok(r) if r.state == RequestState::Queued => {
                    let why = r.message.clone().unwrap_or_default();
                    self.another_try(r, why)
                }
                Ok(r) => {
                    sent += 1;
                    r
                }
                Err(e) => self.after_send_error(rec, &e),
            };
            self.upsert(&next)?;
        }
        Ok(sent)
    }

    /// Update sent requests from Lidarr's albums, queue and history (reads),
    /// and search again for albums nothing was grabbed for when their time
    /// comes ([`RESEARCH_AFTER`]; an AlbumSearch for that album only).
    pub async fn refresh_statuses(&self) -> AppResult<Vec<RequestRecord>> {
        let _one = self.lock.lock().await;
        let mut list = self.records();
        let live: Vec<usize> = (0..list.len()).filter(|&i| followed(&list[i])).collect();
        if live.is_empty() {
            return Ok(list);
        }
        let (client, _, route) = Self::connect(self.routes()?).await?;
        let writer = self.writer_for(route);
        let ids: Vec<i64> = live.iter().filter_map(|&i| list[i].lidarr_album_id).collect();
        let queue = client.queue(&ids).await.unwrap_or_default();
        let now = (self.inner.clock)();
        let ready = self.status().ready;
        // Searches again sent in this look, and the setup check they need
        // (run once, only when one is due).
        let mut sent_again = 0;
        let mut check: Option<Option<CheckReport>> = None;
        for i in live {
            let rgid = list[i].release_group_id.clone();
            let album = match client.album_by_release_group(&rgid).await {
                Ok(a) => a,
                Err(e) => {
                    // One album Lidarr can't answer for doesn't hold up the rest.
                    tracing::info!(title = %list[i].title, error = %e, "lidarr request: couldn't read the album; looks again later");
                    continue;
                }
            };
            let Some(album) = album else {
                list[i].state = RequestState::Failed;
                list[i].message = Some("The album isn't in Lidarr any more.".into());
                list[i].updated_at = now;
                tracing::info!(title = %list[i].title, "lidarr request: failed, the album isn't in Lidarr any more");
                continue;
            };
            let history = client.history(album.id).await.unwrap_or_default();
            let mut state = map_status(&album, &queue, &history);
            // Nothing found yet stays that way until something turns up.
            if list[i].state == RequestState::NotFound && state == RequestState::Searching {
                continue;
            }
            // Only albums this request searched for are searched again: never
            // one Lidarr already had, which is Lidarr's (or another device's) to follow.
            let ours = !list[i].already_in_lidarr;
            if state == RequestState::Searching && ours && list[i].searched_at.is_none() {
                // A request from before searches again were scheduled.
                let r = &mut list[i];
                r.searched_at = Some(r.updated_at.min(now));
                r.searches = r.searches.max(1);
                r.next_search_at = next_search(r.searched_at.unwrap_or(now), r.searches);
            }
            let due = state == RequestState::Searching
                && ours
                && list[i].searches >= 1
                && album.monitored
                && ready
                && list[i].next_search_at.is_some_and(|at| now >= at);
            let mut searched = false;
            let mut held = false;
            if due && sent_again < MAX_RESEARCH_PER_LOOK {
                if check.is_none() {
                    // A recent green check, not the one saved some time ago.
                    check = Some(match self.check_full().await {
                        Ok(c) => Some(c.report),
                        Err(e) => {
                            tracing::info!(error = %e, "lidarr request: setup check before searching again didn't run; tries on the next look");
                            None
                        }
                    });
                }
                let green = check.as_ref().and_then(|c| c.as_ref()).is_some_and(|c| gate(c, list[i].protected).is_ok());
                held = !green;
                if green {
                    match writer.search(vec![album.id]).await {
                        Ok(()) => searched = true,
                        Err(e) => {
                            held = true;
                            tracing::warn!(album = album.id, error = %e, "lidarr request: couldn't search again; tries again in a few minutes");
                        }
                    }
                }
            }
            let r = &mut list[i];
            if held {
                // The setup check didn't run or pass, or the search didn't go
                // out. The check reads every file Plex has, so it isn't run
                // again on every look.
                r.next_search_at = Some(now + RETRY_EVERY.as_secs() as i64);
            }
            if searched {
                sent_again += 1;
                r.searches += 1;
                // Times missed while Platter was closed are skipped.
                r.next_search_at = next_slot_after(r.searched_at.unwrap_or(now), now);
                r.updated_at = now;
                tracing::info!(album = album.id, title = %r.title, searches = r.searches, "lidarr request: nothing grabbed yet, search sent again");
            }
            if state == RequestState::Searching && ours && r.next_search_at.is_none() && now - r.searched_at.unwrap_or(now) >= GIVE_UP_AFTER {
                state = RequestState::NotFound;
            }
            let progress = if state == RequestState::Downloading { super::status::progress(album.id, &queue) } else { None };
            if progress != r.progress {
                if let Some(p) = progress
                    && r.progress.is_none_or(|old| p.abs_diff(old) >= 5 || p == 100)
                {
                    tracing::info!(album = album.id, title = %r.title, progress = p, "lidarr request: downloading");
                }
                r.progress = progress;
            }
            if state != r.state {
                tracing::info!(album = album.id, title = %r.title, from = ?r.state, to = ?state, "lidarr request: state changed");
                r.state = state;
                r.updated_at = now;
                r.message = match state {
                    RequestState::Failed => {
                        let why = queue
                            .iter()
                            .filter(|q| q.album_id == Some(album.id))
                            .find_map(|q| q.error_message.clone().filter(|m| !m.trim().is_empty()));
                        let msg = match why {
                            Some(w) => format!("Lidarr couldn't download or import it: {}", w.trim()),
                            None => "Lidarr couldn't download or import it. Lidarr's Activity page says why.".into(),
                        };
                        tracing::info!(album = album.id, reason = %msg, "lidarr request: failed");
                        Some(msg)
                    }
                    RequestState::NotFound => Some(NOT_FOUND_NOTE.into()),
                    _ => None,
                };
            }
            if searched {
                // Saved at once, so a look that stops part way never sends it twice.
                self.save_records(&list)?;
            }
        }
        self.save_records(&list)?;
        Ok(self.records())
    }

    /// "Search again": an AlbumSearch for that one album now, and the
    /// schedule of searches again starts over.
    pub async fn search_again(&self, id: &str) -> AppResult<RequestRecord> {
        let _one = self.lock.lock().await;
        let mut list = self.records();
        let i = list.iter().position(|r| r.id == id).ok_or_else(|| AppError::NotFound("That request isn't on this device any more.".into()))?;
        if !matches!(list[i].state, RequestState::Searching | RequestState::NotFound) {
            return Err(AppError::Invalid("Lidarr is already working on this album.".into()));
        }
        if !self.status().ready {
            return Err(AppError::Invalid("Lidarr's setup check hasn't passed, so Platter won't ask it for anything. Run the check in Settings.".into()));
        }
        let (client, writer) = self.connection().await?;
        let album = client
            .album_by_release_group(&list[i].release_group_id)
            .await?
            .ok_or_else(|| AppError::NotFound("The album isn't in Lidarr any more. Get it again.".into()))?;
        if !album.monitored {
            return Err(AppError::Invalid("Lidarr no longer has this album marked for download, so Platter didn't search. Get it again.".into()));
        }
        writer.search(vec![album.id]).await?;
        let now = (self.inner.clock)();
        let r = &mut list[i];
        tracing::info!(album = album.id, title = %r.title, "lidarr request: search again (asked)");
        r.state = RequestState::Searching;
        r.message = None;
        r.searched_at = Some(now);
        r.searches = 1;
        r.next_search_at = next_search(now, 1);
        r.updated_at = now;
        let out = r.clone();
        self.save_records(&list)?;
        Ok(out)
    }
    pub fn requests(&self) -> Vec<RequestRecord> {
        let mut l = self.records();
        l.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        l
    }

    /// Requests a Bridge's daily look made, into this device's list so they
    /// show and are followed here. Each is taken once: one already here,
    /// asked for before, or taken before and cleared since isn't. Returns
    /// how many were added.
    pub async fn adopt_requests(&self, recs: Vec<RequestRecord>) -> AppResult<usize> {
        let _one = self.lock.lock().await;
        let mut taken: Vec<String> = self.inner.db.read(|c| kv_get(c, ADOPTED_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let asked: std::collections::HashSet<String> = self.asked_all().into_iter().map(|a| a.release_group_id).collect();
        let mut list = self.records();
        let mut n = 0;
        for mut r in recs.into_iter().take(crate::radar::bridge::KEEP_MADE) {
            r.release_group_id = r.release_group_id.trim().to_ascii_lowercase();
            if !crate::artistinfo::is_mbid(&r.release_group_id) || asked.contains(&r.release_group_id) || taken.contains(&r.release_group_id) {
                continue;
            }
            if matches!(r.state, RequestState::Queued | RequestState::Failed) || list.iter().any(|x| x.id == r.id) {
                continue;
            }
            taken.push(r.release_group_id.clone());
            r.auto = true;
            list.push(r);
            n += 1;
        }
        if n == 0 {
            return Ok(0);
        }
        if taken.len() > KEEP_ADOPTED {
            taken.drain(..taken.len() - KEEP_ADOPTED);
        }
        let json = serde_json::to_string(&taken).map_err(|e| AppError::Internal(e.to_string()))?;
        self.inner.db.write(|c| kv_set(c, ADOPTED_KEY, &json))?;
        self.save_records(&list)?;
        Ok(n)
    }

    /// Cancel a waiting request, or clear a finished one from the list.
    pub async fn cancel(&self, id: &str) -> AppResult<Vec<RequestRecord>> {
        let _one = self.lock.lock().await;
        let mut list = self.records();
        let Some(r) = list.iter().find(|r| r.id == id) else { return Ok(self.requests()) };
        if !(r.state == RequestState::Queued || r.state.settled() || r.already_in_lidarr) {
            return Err(AppError::Invalid("Lidarr already has this request. Remove the album in Lidarr to stop it.".into()));
        }
        list.retain(|r| r.id != id);
        self.save_records(&list)?;
        Ok(self.requests())
    }

    /// An artist's official releases the library lacks, of the "Get albums"
    /// kinds (albums, singles and EPs, live albums, compilations), each with
    /// its section, when the setting is on, Lidarr is set up and its last
    /// check passed.
    pub async fn missing_releases(&self, artist_id: &str, artist: &str) -> AppResult<MissingReleases> {
        let hidden = MissingReleases { shown: false, items: Vec::new() };
        let s = crate::settings::load(&self.inner.db);
        if !s.show_missing_releases {
            return Ok(hidden);
        }
        let st = self.status();
        if !st.usable() || !self.last_check().is_some_and(|l| l.managed_ok.unwrap_or(l.passed)) {
            return Ok(hidden);
        }
        let want = self.want(artist, Some(artist_id), None).await;
        let Some(mbid) = want.mbid.clone() else { return Ok(MissingReleases { shown: true, items: Vec::new() }) };
        let library = self.library(&want).await?;
        let groups = self.picker().artist_groups(&mbid).await?;
        let records = self.records();
        let items = missing_in(groups, &want, &library, &crate::discover::clean_kinds(&s.get_albums_kinds))
            .into_iter()
            .filter(|m| self.allowed(&want, &m.group.id))
            .map(|MissingGroup { group: g, section, refusal }| MissingItem {
                request: records.iter().find(|r| r.release_group_id == g.id).map(|r| r.state),
                year: g.first_release.get(..4).and_then(|y| y.parse().ok()),
                kind: g.primary.clone().unwrap_or_default(),
                secondary: g.secondary.clone(),
                section: section.to_string(),
                thumb: cover_thumb(&g.id),
                refusal: refusal.map(|r| r.message()),
                release_group_id: g.id,
                title: g.title,
            })
            .collect();
        Ok(MissingReleases { shown: true, items })
    }
}

/// A sent request whose state Lidarr can still change.
fn followed(r: &RequestRecord) -> bool {
    r.lidarr_album_id.is_some() && r.state != RequestState::Queued && (!r.state.settled() || r.state == RequestState::NotFound)
}

/// How often requests in flight are followed in Lidarr's queue and history.
pub const FOLLOW_EVERY: Duration = Duration::from_secs(15);

/// The background retry: waiting requests are sent every [`RETRY_EVERY`];
/// requests Lidarr is working on are followed every [`FOLLOW_EVERY`] (reads,
/// plus the scheduled searches again), the rest every [`RETRY_EVERY`].
pub async fn watch(l: Lidarr) {
    tokio::time::sleep(Duration::from_secs(30)).await;
    let mut last_slow: Option<tokio::time::Instant> = None;
    loop {
        let list = l.requests();
        let slow_due = last_slow.is_none_or(|t| t.elapsed() >= RETRY_EVERY);
        if slow_due
            && list.iter().any(|r| r.state == RequestState::Queued)
            && let Err(e) = l.retry_queued().await
        {
            tracing::debug!("lidarr retry: {e}");
        }
        let busy = list.iter().any(|r| r.state.in_flight());
        if (busy || (slow_due && list.iter().any(followed)))
            && let Err(e) = l.refresh_statuses().await
        {
            tracing::debug!("lidarr status: {e}");
        }
        if slow_due {
            last_slow = Some(tokio::time::Instant::now());
        }
        tokio::time::sleep(FOLLOW_EVERY).await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_names_compare_loosely_but_not_too_loosely() {
        assert!(typed_matches("  first   light ", "First Light"));
        assert!(!typed_matches("First", "First Light"));
        assert!(!typed_matches("", ""));
    }

    #[test]
    fn searches_again_after_an_hour_six_hours_a_day_then_daily_for_a_week() {
        let t0 = 1_000_000;
        let at: Vec<i64> = (1..=11).map_while(|n| next_search(t0, n)).map(|t| t - t0).collect();
        assert_eq!(at, [3600, 6 * 3600, 24 * 3600, 2 * DAY, 3 * DAY, 4 * DAY, 5 * DAY, 6 * DAY, 7 * DAY, 8 * DAY]);
        assert_eq!(next_search(t0, 0), None);
        assert_eq!(next_search(t0, 11), None, "then it's Not found yet");
    }

    #[test]
    fn destinations_keep_the_root_style() {
        assert_eq!(join_folder("/downloads/music/", "Northlight"), "/downloads/music/Northlight");
        assert_eq!(join_folder("D:\\Lidarr", "Northlight"), "D:\\Lidarr\\Northlight");
    }
}
