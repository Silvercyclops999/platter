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
use super::mbpick::{Group, LibraryAlbum, Picked, Picker, Want, library_albums, missing};
use super::status::{RequestState, map_status};
use super::write::{AddArtist, clear_exclusion};
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
/// Looks for a newly added artist's album before trying again later.
const ALBUM_TRIES: usize = 30;
/// Most requests kept in the list.
const MAX_KEPT: usize = 200;
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
    /// Made by auto-request for a new release, not by hand.
    #[serde(default)]
    pub auto: bool,
    /// For a protected artist: it lands in the staging folder and waits
    /// there until Platter adds it to the library.
    #[serde(default)]
    pub protected: bool,
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
    /// `album` or `ep`.
    pub kind: String,
    pub year: Option<i32>,
    /// A request made for it, if any.
    pub request: Option<RequestState>,
    /// Its front cover from the Cover Art Archive, small, as an art path.
    pub thumb: Option<String>,
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
    fn records(&self) -> Vec<RequestRecord> {
        self.inner.db.read(|c| kv_get(c, REQUESTS_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save_records(&self, list: &[RequestRecord]) -> AppResult<()> {
        let mut list = list.to_vec();
        list.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        list.truncate(MAX_KEPT);
        let json = serde_json::to_string(&list).map_err(|e| AppError::Internal(e.to_string()))?;
        self.inner.db.write(|c| kv_set(c, REQUESTS_KEY, &json))
    }

    fn upsert(&self, r: &RequestRecord) -> AppResult<()> {
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
        let _one = self.lock.lock().await;
        if !self.typed_confirmed() && !typed_matches(input.typed.as_deref().unwrap_or_default(), &input.title) {
            return Err(AppError::Invalid(format!("Type the album's name, {}, to send your first request.", input.title)));
        }
        let rgid = input.release_group_id.trim().to_ascii_lowercase();
        if let Some(r) = self.records().into_iter().find(|r| r.release_group_id == rgid && r.state != RequestState::Failed) {
            return Ok(r);
        }
        let rec = self.new_record(&input, rgid, false);
        let out = match self.send(rec.clone()).await {
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
        }
    }

    /// An automatic request for a new release (auto-request is on for the
    /// artist, which stands in for the typed name). The same MusicBrainz
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
        let rec = self.new_record(&input, rgid, true);
        let now = (self.inner.clock)();
        let out = match self.send(rec.clone()).await {
            Ok(r) => r,
            Err(e) if e.kind() == "network" => RequestRecord { destination: self.guessed_destination(&rec.artist), ..rec },
            Err(e) if e.to_string().contains("red items") => RequestRecord { message: Some(e.to_string()), updated_at: now, ..rec },
            Err(e) => RequestRecord { state: RequestState::Failed, message: Some(e.to_string()), updated_at: now, ..rec },
        };
        self.upsert(&out)?;
        Ok(out)
    }

    /// Send one request: check MusicBrainz and the whole setup again, then
    /// the writes. Network failures come back as errors so the caller keeps
    /// the request waiting.
    async fn send(&self, rec: RequestRecord) -> AppResult<RequestRecord> {
        self.send_to(rec, &Dest::Library).await
    }

    /// [`Self::send`] to the library's download folder, or to the staging
    /// folder for an upgrade (with a quality profile aiming above the
    /// files there now).
    pub(super) async fn send_to(&self, mut rec: RequestRecord, dest: &Dest) -> AppResult<RequestRecord> {
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

        let artist_id = match (plan.in_lidarr, plan.existing.as_ref()) {
            (_, Some(a)) => a.artist_id,
            (true, None) => plan.artist.id,
            (false, None) => {
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
                let id = c.writer.add_artist(payload).await?;
                rec.added_artist = true;
                rec.notes.push(format!("Added {} to Lidarr with only this album to download.", plan.artist.artist_name));
                id
            }
        };

        // A new artist's albums take Lidarr a moment to read.
        let mut album: Option<Album> = plan.existing.clone();
        let mut albums: Vec<Album> = Vec::new();
        for i in 0..ALBUM_TRIES {
            albums = c.client.albums_of(artist_id).await?;
            if let Some(a) = albums.iter().find(|a| a.foreign_album_id.eq_ignore_ascii_case(&rec.release_group_id)) {
                album = Some(a.clone());
                break;
            }
            if i + 1 < ALBUM_TRIES {
                tokio::time::sleep(self.inner.album_wait).await;
            }
        }
        let Some(album) = album else {
            rec.state = RequestState::Queued;
            rec.message = Some("Lidarr is still reading the artist's albums. Platter will finish the request shortly.".into());
            return Ok(rec);
        };

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
        c.writer.monitor(vec![album.id], true).await?;
        rec.lidarr_album_id = Some(album.id);

        let after = c.client.albums_of(artist_id).await?;
        let target_on = after.iter().any(|x| x.id == album.id && x.monitored);
        let extra = rec.added_artist && after.iter().any(|x| x.id != album.id && x.monitored);
        if !target_on || extra {
            rec.state = RequestState::Failed;
            rec.message = Some(if !target_on {
                "Lidarr didn't mark the album for download, so Platter didn't search for it.".into()
            } else {
                "Lidarr kept other albums by this artist marked for download, so Platter stopped before searching. Check the artist in Lidarr.".into()
            });
            return Ok(rec);
        }
        // The whole check again after the writes, before anything is searched.
        let after = self.check_full().await?;
        if let Err(e) = gate(&after.report, protected) {
            rec.state = RequestState::Failed;
            rec.message = Some(format!("{e} Platter stopped before searching."));
            return Ok(rec);
        }
        c.writer.search(vec![album.id]).await?;
        rec.state = RequestState::Searching;
        rec.message = None;
        Ok(rec)
    }

    /// Send the requests waiting on this device. Each runs the full check.
    pub async fn retry_queued(&self) -> AppResult<usize> {
        let _one = self.lock.lock().await;
        let waiting: Vec<RequestRecord> = self.records().into_iter().filter(|r| r.state == RequestState::Queued).collect();
        let mut sent = 0;
        for rec in waiting {
            let now = (self.inner.clock)();
            let next = match self.send(rec.clone()).await {
                Ok(r) => {
                    if r.state != RequestState::Queued {
                        sent += 1;
                    }
                    r
                }
                Err(e) if e.kind() == "network" => RequestRecord { message: Some(QUEUED_NOTE.into()), updated_at: now, ..rec },
                Err(e) if e.to_string().contains("red items") => RequestRecord { message: Some(e.to_string()), updated_at: now, ..rec },
                Err(e) => RequestRecord { state: RequestState::Failed, message: Some(e.to_string()), updated_at: now, ..rec },
            };
            self.upsert(&next)?;
        }
        Ok(sent)
    }

    /// Update sent requests from Lidarr's albums, queue and history (reads).
    pub async fn refresh_statuses(&self) -> AppResult<Vec<RequestRecord>> {
        let _one = self.lock.lock().await;
        let mut list = self.records();
        let live: Vec<usize> = (0..list.len())
            .filter(|&i| list[i].lidarr_album_id.is_some() && list[i].state != RequestState::Queued && !list[i].state.settled())
            .collect();
        if live.is_empty() {
            return Ok(list);
        }
        let (client, _, _) = Self::connect(self.routes()?).await?;
        let ids: Vec<i64> = live.iter().filter_map(|&i| list[i].lidarr_album_id).collect();
        let queue = client.queue(&ids).await.unwrap_or_default();
        let now = (self.inner.clock)();
        for i in live {
            let rgid = list[i].release_group_id.clone();
            let Some(album) = client.album_by_release_group(&rgid).await? else {
                list[i].state = RequestState::Failed;
                list[i].message = Some("The album isn't in Lidarr any more.".into());
                list[i].updated_at = now;
                continue;
            };
            let history = client.history(album.id).await.unwrap_or_default();
            let state = map_status(&album, &queue, &history);
            if state != list[i].state {
                list[i].state = state;
                list[i].updated_at = now;
                if state != RequestState::Failed {
                    list[i].message = None;
                } else {
                    list[i].message = Some("Lidarr couldn't download or import it. Lidarr's Activity page says why.".into());
                }
            }
        }
        self.save_records(&list)?;
        Ok(list)
    }

    pub fn requests(&self) -> Vec<RequestRecord> {
        let mut l = self.records();
        l.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        l
    }

    /// Cancel a waiting request, or clear a finished one from the list.
    pub fn cancel(&self, id: &str) -> AppResult<Vec<RequestRecord>> {
        let mut list = self.records();
        let Some(r) = list.iter().find(|r| r.id == id) else { return Ok(self.requests()) };
        if !(r.state == RequestState::Queued || r.state.settled() || r.already_in_lidarr) {
            return Err(AppError::Invalid("Lidarr already has this request. Remove the album in Lidarr to stop it.".into()));
        }
        list.retain(|r| r.id != id);
        self.save_records(&list)?;
        Ok(self.requests())
    }

    /// An artist's official albums and EPs the library lacks, when the
    /// setting is on, Lidarr is set up and its last check passed.
    pub async fn missing_releases(&self, artist_id: &str, artist: &str) -> AppResult<MissingReleases> {
        let hidden = MissingReleases { shown: false, items: Vec::new() };
        let s = crate::settings::load(&self.inner.db);
        if !s.show_missing_releases {
            return Ok(hidden);
        }
        let st = self.status();
        if !(st.url_set || st.away_set) || !st.has_key || !self.last_check().is_some_and(|l| l.managed_ok.unwrap_or(l.passed)) {
            return Ok(hidden);
        }
        let want = self.want(artist, Some(artist_id), None).await;
        let Some(mbid) = want.mbid.clone() else { return Ok(MissingReleases { shown: true, items: Vec::new() }) };
        let library = self.library(&want).await?;
        let groups = self.picker().artist_groups(&mbid).await?;
        let records = self.records();
        let items = missing(groups, &want, &library)
            .into_iter()
            .filter(|g| self.allowed(&want, &g.id))
            .map(|g| MissingItem {
                request: records.iter().find(|r| r.release_group_id == g.id).map(|r| r.state),
                year: g.first_release.get(..4).and_then(|y| y.parse().ok()),
                kind: g.primary.clone().unwrap_or_default(),
                thumb: cover_thumb(&g.id),
                release_group_id: g.id,
                title: g.title,
            })
            .collect();
        Ok(MissingReleases { shown: true, items })
    }
}

/// The background retry: waiting requests are sent, sent ones followed.
pub async fn watch(l: Lidarr) {
    tokio::time::sleep(Duration::from_secs(30)).await;
    loop {
        let list = l.requests();
        if list.iter().any(|r| r.state == RequestState::Queued)
            && let Err(e) = l.retry_queued().await
        {
            tracing::debug!("lidarr retry: {e}");
        }
        if list.iter().any(|r| r.state != RequestState::Queued && !r.state.settled())
            && let Err(e) = l.refresh_statuses().await
        {
            tracing::debug!("lidarr status: {e}");
        }
        tokio::time::sleep(RETRY_EVERY).await;
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
    fn destinations_keep_the_root_style() {
        assert_eq!(join_folder("/downloads/music/", "Northlight"), "/downloads/music/Northlight");
        assert_eq!(join_folder("D:\\Lidarr", "Northlight"), "D:\\Lidarr\\Northlight");
    }
}
