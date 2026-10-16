//! Upgrades: better copies of official albums already in the library,
//! swapped in file by file. Lidarr never touches the library.
//!
//! 1. "Look for a better copy" asks Lidarr for the album's official release
//!    group through the same guarded writes as any request, but into the
//!    staging folder (a Lidarr root folder Plex doesn't scan), with a
//!    quality profile whose cutoff is above the files there now. The setup
//!    check, with its staging findings, runs again first.
//! 2. Once Lidarr has imported it, the offer compares the old files with the
//!    new ones (`upmatch`): what upgrades and what's kept as it is, and why.
//! 3. The swap (`swap`) runs where the files are, through the music share
//!    mapping on this computer, then asks Plex to rescan just those folders.
//!    Old files go to a backup folder, never deleted, and Undo puts them
//!    back byte for byte.
//!
//! With "Upgrades: ask before each swap" off, an imported copy is swapped on
//! its own under the same rules, unless the album folder holds anything
//! kept for a reason that needs a person (then it always asks per song).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::Lidarr;
use super::paths::NormPath;
use super::quality::{CD, Rank};
use super::request::{Dest, QUEUED_NOTE, RequestInput, RequestRecord};
use super::status::{RequestState, map_status};
use super::swap::{self, Done, Failure, Planned, SwapHost};
use super::provenance::{NoteTag, StatusReader};
use super::swap::NoteEdit;
use super::upmatch::{self, Keep, NewSong, OfficialTrack, OldSong, Outcome};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};

const KEY: &str = "lidarr.upgrades";
/// Albums the user confirmed a release group for.
const CONFIRM_KEY: &str = "lidarr.upgradeConfirmed";
const NOT_CONFIRMED: &str = "Confirm which MusicBrainz release this album is first.";
/// How the default backup place is described.
pub const BACKUP_DEFAULT: &str = "a _to_delete folder in your music folder, which Plex ignores";
const MAX_KEPT: usize = 200;
/// Most albums one "look for better copies" run asks for.
pub const BULK_MAX: usize = 25;
const AUDIO_EXT: [&str; 13] = ["mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "opus", "wav", "aif", "aiff", "ape", "wv"];

// ---------------------------------------------------------------- outside world

/// Plex, for rescanning the folders a swap changed.
#[async_trait]
pub trait PlexFolders: Send + Sync {
    /// Ok when a Plex music library covers every folder and Plex answers.
    async fn can_scan(&self, folders: &[String]) -> AppResult<()>;
    async fn scan(&self, folder: &str) -> AppResult<()>;
}

/// Where swaps run. None when nothing can reach the files yet.
pub trait Hosts: Send + Sync {
    fn host(&self) -> Option<Arc<dyn SwapHost>>;
}

/// This computer, when the music share mapping is set.
pub struct DesktopHosts {
    pub db: DbHandle,
}

impl Hosts for DesktopHosts {
    fn host(&self) -> Option<Arc<dyn SwapHost>> {
        let map = crate::musicshare::load(&self.db);
        if !map.is_set() {
            return None;
        }
        Some(Arc::new(swap::DesktopHost {
            map,
            lidarr_staging: crate::settings::load(&self.db).lidarr_staging_folder,
            fs: Box::new(swap::LocalFs),
            tags: Box::new(swap::LoftyTags),
        }))
    }
}

/// The connected Plex server's music sections.
pub struct PlexRescan(pub crate::plex::PlexState);

impl PlexRescan {
    async fn section_for(&self, folder: &str) -> AppResult<(crate::plex::PlexClient, String)> {
        let unreachable = |_| AppError::Network("Platter can't reach your Plex server.".into());
        let c = self.0.client().await.map_err(unreachable)?;
        let f = NormPath::parse(folder);
        let id = c
            .music_sections()
            .await
            .map_err(unreachable)?
            .into_iter()
            .find(|s| s.locations.iter().any(|l| f.starts_with(&NormPath::parse(l))))
            .map(|s| s.id)
            .ok_or_else(|| AppError::NotFound(format!("No Plex music library includes {folder}.")))?;
        Ok((c, id))
    }
}

#[async_trait]
impl PlexFolders for PlexRescan {
    async fn can_scan(&self, folders: &[String]) -> AppResult<()> {
        for f in folders {
            self.section_for(f).await?;
        }
        Ok(())
    }

    async fn scan(&self, folder: &str) -> AppResult<()> {
        let (c, id) = self.section_for(folder).await?;
        c.scan_folder(&id, folder).await
    }
}

pub struct UpgradeDeps {
    pub lidarr: Lidarr,
    pub db: DbHandle,
    pub hosts: Arc<dyn Hosts>,
    pub plex: Arc<dyn PlexFolders>,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// Each file's release status and provenance note.
    pub status: Arc<dyn StatusReader>,
}

#[derive(Clone)]
pub struct Upgrades {
    inner: Arc<UpgradeDeps>,
    /// One swap, undo or request at a time.
    lock: Arc<tokio::sync::Mutex<()>>,
}

// ---------------------------------------------------------------- records

/// One swap that happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapRun {
    pub id: String,
    pub at: i64,
    /// Where the old files are now.
    pub backup_dir: String,
    pub files: Vec<Done>,
    pub undone: bool,
    /// "Upgraded 12 songs (MP3 320 to FLAC)".
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeRecord {
    #[serde(flatten)]
    pub request: RequestRecord,
    /// The library album.
    pub album_id: String,
    /// What it was when the upgrade was asked for ("MP3 320").
    pub from_quality: String,
    #[serde(default)]
    pub swaps: Vec<SwapRun>,
    /// Why the last swap or undo stopped.
    #[serde(default)]
    pub failure: Option<Failure>,
}

impl UpgradeRecord {
    pub fn last_swap(&self) -> Option<&SwapRun> {
        self.swaps.last().filter(|s| !s.undone)
    }
}

/// The album menu's view: whether "Look for a better copy" shows, and any upgrade.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumUpgrade {
    pub available: bool,
    pub record: Option<UpgradeRecord>,
    /// The album now, "MP3 320".
    pub quality: Option<String>,
    /// The MusicBrainz release the user confirmed this album is.
    pub confirmed: Option<Confirmed>,
}

/// The user's word that a library album is this MusicBrainz release group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Confirmed {
    pub release_group_id: String,
    pub title: String,
    pub at: i64,
}

/// One song of the library album, beside a candidate's tracklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSong {
    pub disc: u32,
    pub track: u32,
    pub title: String,
    pub duration_ms: u64,
}

/// What confirming needs: the album's songs and the releases it could be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidates {
    pub title: String,
    pub artist: String,
    pub songs: Vec<FolderSong>,
    pub confirmed: Option<Confirmed>,
    pub candidates: Vec<super::mbpick::Candidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OfferStatus {
    /// Lidarr hasn't imported the better copy yet.
    Waiting,
    /// The share mapping isn't set (or doesn't cover the files).
    NeedsMapping,
    /// The share or Lidarr can't be reached now.
    Unreachable,
    Ready,
    /// The better copy is here but no song can be swapped.
    Nothing,
    /// Swapped (Undo is offered).
    Swapped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferSong {
    /// Empty for a file Plex doesn't list on the album.
    pub track_id: String,
    pub title: String,
    pub disc: u32,
    pub track: u32,
    pub from: String,
    pub to: Option<String>,
    /// Why it's kept.
    pub reason: Option<String>,
    pub keep: Option<Keep>,
    /// How it was matched: `isrc` or `recordingId`.
    pub by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub upgrade_id: String,
    pub album_id: String,
    pub title: String,
    pub artist: String,
    pub status: OfferStatus,
    pub message: Option<String>,
    /// "Upgrade 12 of 14 songs (MP3 320 to FLAC)".
    pub summary: Option<String>,
    pub from: String,
    pub to: String,
    pub upgrade: Vec<OfferSong>,
    pub kept: Vec<OfferSong>,
    /// Ask about each song (the folder holds something kept for a reason
    /// that needs a person), whatever the setting says.
    pub ask_each: bool,
    pub ask_reason: Option<String>,
    /// "Upgrades: ask before each swap".
    pub ask: bool,
    /// Where old files go.
    pub backup_dir: String,
    pub last_swap: Option<SwapRun>,
    pub failure: Option<Failure>,
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapResult {
    pub record: UpgradeRecord,
    pub swapped: usize,
    pub backup_dir: String,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Under {
    /// MP3, AAC and other lossy copies.
    Lossy,
    /// Lossy, or lossless below 16-bit.
    BelowCd,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkSkip {
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkResult {
    pub requested: Vec<UpgradeRecord>,
    pub skipped: Vec<BulkSkip>,
    /// Albums left for another run (past [`BULK_MAX`]).
    pub more: usize,
}

// ---------------------------------------------------------------- the library

#[derive(Debug, Clone)]
struct Song {
    id: String,
    title: String,
    disc: u32,
    track: u32,
    rank: Rank,
    codec: Option<String>,
    file: Option<String>,
    unreleased: bool,
    duration_ms: u64,
}

#[derive(Debug, Clone)]
pub(super) struct AlbumFiles {
    pub(super) title: String,
    pub(super) artist: String,
    artist_id: String,
    pub(super) unreleased: bool,
    songs: Vec<Song>,
}

impl AlbumFiles {
    /// The lowest quality among released songs.
    pub(super) fn current(&self) -> Option<(Rank, String)> {
        self.songs.iter().filter(|s| !s.unreleased).min_by_key(|s| s.rank).map(|s| (s.rank, s.rank.label(s.codec.as_deref())))
    }
}

fn album_files(c: &Connection, album_id: &str) -> AppResult<Option<AlbumFiles>> {
    use crate::db::release::{album_unreleased, track_unreleased};
    let row = c
        .query_row(
            "SELECT al.title, al.artist_id, al.artist_title, al.format, al.subformats, coalesce(at.version, at.release), al.matched, al.studio, mc.found
             FROM albums al LEFT JOIN album_tags at ON at.album_id = al.id LEFT JOIN album_mb_check mc ON mc.album_id = al.id
             WHERE al.id = ?1",
            [album_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<bool>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<bool>>(8)?,
                ))
            },
        )
        .optional()?;
    let Some((title, artist_id, artist, format, subs, version, matched, studio, mb)) = row else { return Ok(None) };
    let subs: Vec<String> = subs.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let unreleased = album_unreleased(album_id, format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref(), mb);
    let mut st = c.prepare_cached(
        "SELECT t.id, t.title, t.disc_number, t.track_number, t.codec, t.bitrate, t.bit_depth, t.file, coalesce(ft.version, ft.release), t.duration_ms
         FROM tracks t LEFT JOIN file_tags ft ON ft.part_key = t.part_key
         WHERE t.album_id = ?1 ORDER BY t.disc_number, t.track_number, t.title",
    )?;
    let songs = st
        .query_map([album_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<u32>>(5)?,
                r.get::<_, Option<u32>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, i64>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|(id, title, disc, track, codec, kbps, bits, file, fv, duration_ms)| {
            let v = fv.as_deref().filter(|v| !v.trim().is_empty()).or(version.as_deref());
            let unreleased = unreleased || track_unreleased(&id, album_id, format.as_deref(), &subs, matched, studio.as_deref(), v, mb);
            Song { rank: Rank::of_file(codec.as_deref(), kbps, bits), id, title, disc, track, codec, file, unreleased, duration_ms: duration_ms.max(0) as u64 }
        })
        .collect();
    Ok(Some(AlbumFiles { title, artist, artist_id, unreleased, songs }))
}

/// "YYYY-MM-DD" for unix seconds (UTC).
pub fn ymd(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A server path's folder, in its own style.
fn folder_of(path: &str) -> String {
    match path.rfind(['/', '\\']) {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

fn is_audio(p: &Path) -> bool {
    p.extension().is_some_and(|e| AUDIO_EXT.contains(&e.to_string_lossy().to_ascii_lowercase().as_str()))
}

/// Short words for a swap.
fn summary_words(n: usize, of: Option<usize>, from: &str, to: &str, done: bool) -> String {
    let songs = |k: usize| if k == 1 { "song".to_string() } else { "songs".to_string() };
    match (done, of) {
        (true, _) => format!("Upgraded {n} {} ({from} to {to})", songs(n)),
        (false, Some(total)) => format!("Upgrade {n} of {total} {} ({from} to {to})", songs(total)),
        (false, None) => format!("Upgrade {n} {} ({from} to {to})", songs(n)),
    }
}

/// What the offer worked out, with the paths the swap needs.
struct Worked {
    offer: Offer,
    /// Track id → (Plex path, Lidarr path, the note to add) for each song that upgrades.
    paths: HashMap<String, (String, String, Option<NoteEdit>)>,
}

impl Upgrades {
    pub fn new(deps: UpgradeDeps) -> Self {
        Self { inner: Arc::new(deps), lock: Arc::new(tokio::sync::Mutex::new(())) }
    }

    pub(super) fn inner_db(&self) -> &DbHandle {
        &self.inner.db
    }

    pub(super) fn lidarr(&self) -> &Lidarr {
        &self.inner.lidarr
    }

    pub(super) fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    pub fn records(&self) -> Vec<UpgradeRecord> {
        let mut l: Vec<UpgradeRecord> =
            self.inner.db.read(|c| kv_get(c, KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        l.sort_by_key(|r| std::cmp::Reverse(r.request.created_at));
        l
    }

    fn save(&self, list: &[UpgradeRecord]) -> AppResult<()> {
        let mut list = list.to_vec();
        list.sort_by_key(|r| std::cmp::Reverse(r.request.created_at));
        // Records with a swap that can still be undone are never dropped.
        let mut kept = 0;
        list.retain(|r| {
            kept += 1;
            kept <= MAX_KEPT || r.last_swap().is_some()
        });
        let json = serde_json::to_string(&list).map_err(|e| AppError::Internal(e.to_string()))?;
        self.inner.db.write(|c| kv_set(c, KEY, &json))
    }

    fn upsert(&self, r: &UpgradeRecord) -> AppResult<()> {
        let mut list = self.records();
        match list.iter_mut().find(|x| x.request.id == r.request.id) {
            Some(x) => *x = r.clone(),
            None => list.push(r.clone()),
        }
        self.save(&list)
    }

    fn record(&self, id: &str) -> AppResult<UpgradeRecord> {
        self.records().into_iter().find(|r| r.request.id == id).ok_or_else(|| AppError::NotFound("That upgrade isn't on this computer any more.".into()))
    }

    fn staging(&self) -> String {
        crate::settings::load(&self.inner.db).lidarr_staging_folder
    }

    pub(super) async fn album(&self, album_id: &str) -> AppResult<AlbumFiles> {
        let id = album_id.to_string();
        self.inner
            .db
            .read_async(move |c| album_files(c, &id))
            .await?
            .ok_or_else(|| AppError::NotFound("That album isn't in the library any more.".into()))
    }

    /// Whether the album menu offers "Look for a better copy", and its upgrade.
    pub async fn for_album(&self, album_id: &str) -> AppResult<AlbumUpgrade> {
        let record = self.records().into_iter().find(|r| r.album_id == album_id);
        let on = !self.staging().trim().is_empty() && self.lidarr().status().ready;
        let a = if on { self.album(album_id).await.ok() } else { None };
        let quality = a.as_ref().and_then(|a| a.current()).map(|(_, l)| l);
        let available = a.as_ref().is_some_and(|a| !a.unreleased && a.current().is_some_and(|(r, _)| r < Rank::lossless(Some(24))));
        let confirmed = self.confirmations().get(album_id).cloned();
        Ok(AlbumUpgrade { available, record, quality, confirmed })
    }

    pub(super) fn confirmations(&self) -> HashMap<String, Confirmed> {
        self.inner.db.read(|c| kv_get(c, CONFIRM_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// The album's songs and the official studio releases it could be, each
    /// with its tracklist, for the user to confirm one.
    pub async fn candidates(&self, album_id: &str) -> AppResult<Candidates> {
        let a = self.album(album_id).await?;
        let l = self.lidarr();
        let want = l.want(&a.artist, Some(&a.artist_id), None).await;
        let mut candidates = l.picker().upgrade_candidates(&want, &a.title).await?;
        candidates.retain(|c| l.allowed(&want, &c.group.id));
        let songs = a.songs.iter().map(|s| FolderSong { disc: s.disc, track: s.track, title: s.title.clone(), duration_ms: s.duration_ms }).collect();
        Ok(Candidates { title: a.title, artist: a.artist, songs, confirmed: self.confirmations().get(album_id).cloned(), candidates })
    }

    /// Confirm (or with None forget) which release group a library album is.
    /// The group must pass every request rule but the title clash.
    pub async fn confirm(&self, album_id: &str, rgid: Option<&str>) -> AppResult<Option<Confirmed>> {
        let _one = self.lock.lock().await;
        let mut all = self.confirmations();
        let out = match rgid {
            None => {
                all.remove(album_id);
                None
            }
            Some(id) => {
                let a = self.album(album_id).await?;
                let l = self.lidarr();
                let want = l.want(&a.artist, Some(&a.artist_id), None).await;
                let library = l.library(&want).await?;
                let g = l.picker().vet_upgrade_group(id, &want, &library, album_id).await?;
                if !l.allowed(&want, &g.id) {
                    return Err(AppError::Invalid(super::mbpick::Refusal::NotAllowed.message()));
                }
                let c = Confirmed { release_group_id: g.id, title: g.title, at: self.now() };
                all.insert(album_id.to_string(), c.clone());
                Some(c)
            }
        };
        let json = serde_json::to_string(&all).map_err(|e| AppError::Internal(e.to_string()))?;
        self.inner.db.write(|c| kv_set(c, CONFIRM_KEY, &json))?;
        Ok(out)
    }

    /// Ask Lidarr for a better copy of a library album, into the staging folder.
    pub async fn request(&self, album_id: &str) -> AppResult<UpgradeRecord> {
        let _one = self.lock.lock().await;
        self.request_locked(album_id).await
    }

    async fn request_locked(&self, album_id: &str) -> AppResult<UpgradeRecord> {
        self.request_aimed(album_id, None).await
    }

    /// [`Self::request`], asking Lidarr for at least `floor` (a profile
    /// whose cutoff is above it), for "Look for better copies".
    pub(super) async fn request_aimed(&self, album_id: &str, floor: Option<Rank>) -> AppResult<UpgradeRecord> {
        if self.staging().trim().is_empty() {
            return Err(AppError::Invalid("Set the Lidarr staging folder for upgrades in Settings first.".into()));
        }
        if let Some(r) = self.records().into_iter().find(|r| r.album_id == album_id && r.request.state != RequestState::Failed && r.swaps.is_empty()) {
            return Ok(r);
        }
        let a = self.album(album_id).await?;
        if a.unreleased {
            return Err(AppError::Invalid(super::mbpick::Refusal::UnreleasedAlbum.message()));
        }
        let Some((current, label)) = a.current() else {
            return Err(AppError::Invalid(super::mbpick::Refusal::UnreleasedAlbum.message()));
        };
        if current >= Rank::lossless(Some(24)) {
            return Err(AppError::Invalid(format!("This album is already {label}, as good as Lidarr gets.")));
        }
        let l = self.lidarr();
        let want = l.want(&a.artist, Some(&a.artist_id), None).await;
        let library = l.library(&want).await?;
        let Some(confirmed) = self.confirmations().get(album_id).cloned() else {
            return Err(AppError::Invalid(NOT_CONFIRMED.into()));
        };
        let group = l.picker().vet_upgrade_group(&confirmed.release_group_id, &want, &library, album_id).await?;
        let input = RequestInput {
            release_group_id: group.id.clone(),
            title: group.title.clone(),
            artist: a.artist.clone(),
            artist_id: Some(a.artist_id.clone()),
            ..Default::default()
        };
        let rec = l.new_record(&input, group.id.clone(), false);
        let aim = floor.map_or(current, |f| f.max(current));
        let dest = Dest::Staging { album_id: album_id.to_string(), current: aim, current_label: label.clone() };
        let sent = {
            let _req = l.lock.lock().await;
            l.send_to(rec.clone(), &dest).await
        };
        let request = match sent {
            Ok(r) => r,
            Err(e) if e.kind() == "network" => RequestRecord { message: Some(QUEUED_NOTE.into()), destination: Some(self.staging()), ..rec },
            Err(e) => return Err(e),
        };
        let out = UpgradeRecord { request, album_id: album_id.to_string(), from_quality: label, swaps: Vec::new(), failure: None };
        self.upsert(&out)?;
        Ok(out)
    }

    /// Ask for better copies of every released album under a quality.
    pub async fn bulk(&self, under: Under) -> AppResult<BulkResult> {
        let _one = self.lock.lock().await;
        let ids: Vec<String> = self.inner.db.read_async(|c| {
            let mut st = c.prepare("SELECT id FROM albums ORDER BY artist_title, title")?;
            Ok(st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
        let have: BTreeSet<String> = self.records().into_iter().map(|r| r.album_id).collect();
        // Only albums the user has confirmed a release for.
        let confirmed = self.confirmations();
        let ids: Vec<String> = ids.into_iter().filter(|i| confirmed.contains_key(i)).collect();
        let limit = match under {
            Under::Lossy => Rank(1000),
            Under::BelowCd => CD,
        };
        let mut wanted = Vec::new();
        for id in ids.into_iter().filter(|i| !have.contains(i)) {
            if let Ok(a) = self.album(&id).await
                && !a.unreleased
                && a.current().is_some_and(|(r, _)| r < limit)
            {
                wanted.push((id, a.title));
            }
        }
        let more = wanted.len().saturating_sub(BULK_MAX);
        let mut out = BulkResult { requested: Vec::new(), skipped: Vec::new(), more };
        for (id, title) in wanted.into_iter().take(BULK_MAX) {
            match self.request_locked(&id).await {
                Ok(r) => out.requested.push(r),
                Err(e) => out.skipped.push(BulkSkip { title, reason: e.to_string() }),
            }
        }
        Ok(out)
    }

    /// Clear an upgrade from the list. One with a swap that can be undone stays.
    pub fn clear(&self, id: &str) -> AppResult<Vec<UpgradeRecord>> {
        let mut list = self.records();
        if list.iter().any(|r| r.request.id == id && r.last_swap().is_some()) {
            return Err(AppError::Invalid("Undo the swap first, or keep it in the list so it can be undone.".into()));
        }
        list.retain(|r| r.request.id != id);
        self.save(&list)?;
        Ok(self.records())
    }

    /// Send waiting upgrades and follow sent ones in Lidarr (reads).
    pub async fn refresh(&self) -> AppResult<Vec<UpgradeRecord>> {
        let _one = self.lock.lock().await;
        let mut list = self.records();
        for r in list.iter_mut().filter(|r| r.request.state == RequestState::Queued) {
            let Ok(a) = self.album(&r.album_id).await else { continue };
            let Some((current, label)) = a.current() else { continue };
            let dest = Dest::Staging { album_id: r.album_id.clone(), current, current_label: label };
            let l = self.lidarr();
            let sent = {
                let _req = l.lock.lock().await;
                l.send_to(r.request.clone(), &dest).await
            };
            let now = self.now();
            match sent {
                Ok(x) => r.request = x,
                Err(e) if e.kind() == "network" => r.request.message = Some(QUEUED_NOTE.into()),
                Err(e) if e.to_string().contains("red items") => r.request.message = Some(e.to_string()),
                Err(e) => {
                    r.request.state = RequestState::Failed;
                    r.request.message = Some(e.to_string());
                }
            }
            r.request.updated_at = now;
        }
        let live: Vec<usize> = (0..list.len())
            .filter(|&i| list[i].request.lidarr_album_id.is_some() && !list[i].request.state.settled() && list[i].request.state != RequestState::Queued)
            .collect();
        if !live.is_empty() {
            let (client, _, _) = Lidarr::connect(self.lidarr().routes()?).await?;
            let ids: Vec<i64> = live.iter().filter_map(|&i| list[i].request.lidarr_album_id).collect();
            let queue = client.queue(&ids).await.unwrap_or_default();
            for i in live {
                let rgid = list[i].request.release_group_id.clone();
                let Some(album) = client.album_by_release_group(&rgid).await? else { continue };
                let history = client.history(album.id).await.unwrap_or_default();
                let state = map_status(&album, &queue, &history);
                if state != list[i].request.state {
                    list[i].request.state = state;
                    list[i].request.updated_at = self.now();
                    list[i].request.message =
                        (state == RequestState::Failed).then(|| "Lidarr couldn't download or import it. Lidarr's Activity page says why.".to_string());
                }
            }
        }
        self.save(&list)?;
        Ok(self.records())
    }

    // ------------------------------------------------------------ the offer

    fn base_offer(&self, r: &UpgradeRecord, a: Option<&AlbumFiles>) -> Offer {
        let chosen = crate::musicshare::load(&self.inner.db).backup;
        let backup = if chosen.trim().is_empty() { BACKUP_DEFAULT.to_string() } else { chosen };
        Offer {
            upgrade_id: r.request.id.clone(),
            album_id: r.album_id.clone(),
            title: a.map(|a| a.title.clone()).unwrap_or_else(|| r.request.title.clone()),
            artist: r.request.artist.clone(),
            status: OfferStatus::Waiting,
            message: None,
            summary: None,
            from: r.from_quality.clone(),
            to: String::new(),
            upgrade: Vec::new(),
            kept: Vec::new(),
            ask_each: false,
            ask_reason: None,
            ask: crate::settings::load(&self.inner.db).upgrades_ask,
            backup_dir: backup,
            last_swap: r.last_swap().cloned(),
            failure: r.failure.clone(),
            failure_message: r.failure.as_ref().map(Failure::message),
        }
    }

    /// What swapping would do now. Reads Lidarr (GET only) and the files.
    pub async fn offer(&self, id: &str) -> AppResult<Offer> {
        Ok(self.work(&self.record(id)?).await?.offer)
    }

    async fn work(&self, r: &UpgradeRecord) -> AppResult<Worked> {
        let a = self.album(&r.album_id).await.ok();
        let mut offer = self.base_offer(r, a.as_ref());
        let empty = |offer| Ok(Worked { offer, paths: HashMap::new() });
        if let Some(s) = r.last_swap() {
            offer.status = OfferStatus::Swapped;
            offer.summary = Some(s.summary.clone());
            return empty(offer);
        }
        let Some(a) = a else {
            offer.status = OfferStatus::Nothing;
            offer.message = Some("That album isn't in the library any more.".into());
            return empty(offer);
        };
        if r.request.state != RequestState::Imported {
            offer.message = Some(match r.request.state {
                RequestState::Failed => r.request.message.clone().unwrap_or_else(|| "Lidarr couldn't get a better copy.".into()),
                RequestState::Queued => QUEUED_NOTE.into(),
                _ => "Lidarr is still getting the better copy.".into(),
            });
            return empty(offer);
        }
        let Some(host) = self.inner.hosts.host() else {
            offer.status = OfferStatus::NeedsMapping;
            offer.message = Some("Set where this computer reaches your music share (Settings > Integrations > Lidarr) to see what would change.".into());
            return empty(offer);
        };
        let Some(lidarr_album) = r.request.lidarr_album_id else {
            offer.status = OfferStatus::Nothing;
            return empty(offer);
        };
        let lidarr_unreachable = |mut offer: Offer, e: AppError| {
            offer.status = OfferStatus::Unreachable;
            offer.message = Some(e.to_string());
            Ok(Worked { offer, paths: HashMap::new() })
        };
        let client = match self.lidarr().routes().map(Lidarr::connect) {
            Ok(f) => match f.await {
                Ok((c, _, _)) => c,
                Err(e) => return lidarr_unreachable(offer, e),
            },
            Err(e) => return lidarr_unreachable(offer, e),
        };
        let (tracks, files) = match futures::try_join!(client.tracks(lidarr_album), client.track_files(lidarr_album)) {
            Ok(x) => x,
            Err(e) => return lidarr_unreachable(offer, e),
        };
        let h = host.clone();
        let s = crate::settings::load(&self.inner.db);
        let cx = CompareCtx { prefix: s.upgrade_official_prefix, today: ymd(self.now()), status: self.inner.status.clone() };
        tokio::task::spawn_blocking(move || compare(h.as_ref(), offer, &a, &tracks, &files, &cx))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))
    }

    // ------------------------------------------------------------ the swap

    fn fail(&self, mut r: UpgradeRecord, f: Failure) -> AppError {
        let msg = f.message();
        r.failure = Some(f);
        r.request.updated_at = self.now();
        let _ = self.upsert(&r);
        AppError::Invalid(msg)
    }

    /// Swap in the better copies of `track_ids` (None: every song that
    /// upgrades, only allowed when the album doesn't need asking per song).
    pub async fn swap(&self, id: &str, track_ids: Option<Vec<String>>) -> AppResult<SwapResult> {
        let _one = self.lock.lock().await;
        let r = self.record(id)?;
        let worked = self.work(&r).await?;
        let offer = worked.offer;
        match offer.status {
            OfferStatus::Ready => {}
            OfferStatus::NeedsMapping => return Err(self.fail(r, Failure::NoMapping)),
            _ => return Err(AppError::Invalid(offer.message.unwrap_or_else(|| "There's nothing to swap.".into()))),
        }
        let chosen: Vec<&OfferSong> = match &track_ids {
            Some(ids) => offer.upgrade.iter().filter(|s| ids.contains(&s.track_id)).collect(),
            None if offer.ask_each => {
                return Err(AppError::Invalid(offer.ask_reason.unwrap_or_else(|| "Pick each song to upgrade.".into())));
            }
            None => offer.upgrade.iter().collect(),
        };
        if chosen.is_empty() {
            return Err(AppError::Invalid("Pick at least one song to upgrade.".into()));
        }
        let items: Vec<Planned> = chosen
            .iter()
            .filter_map(|s| {
                worked.paths.get(&s.track_id).map(|(p, l, note)| Planned { track_id: s.track_id.clone(), plex_path: p.clone(), lidarr_path: l.clone(), note: note.clone() })
            })
            .collect();
        let mut folders: Vec<String> = items.iter().map(|i| folder_of(&i.plex_path)).collect();
        folders.sort();
        folders.dedup();
        if self.inner.plex.can_scan(&folders).await.is_err() {
            return Err(self.fail(r, Failure::PlexUnreachable));
        }
        let Some(host) = self.inner.hosts.host() else { return Err(self.fail(r, Failure::NoMapping)) };
        let swap_id = uuid::Uuid::new_v4().to_string();
        let folder = swap::backup_folder(&offer.artist, &offer.title, &swap_id);
        let (h, b, it) = (host.clone(), folder.clone(), items.clone());
        let ran = tokio::task::spawn_blocking(move || swap::run(h.as_ref(), &b, &it)).await.map_err(|e| AppError::Internal(e.to_string()))?;
        let (done, ignored) = match ran {
            Ok(x) => (x.done, x.ignored),
            Err(f) => return Err(self.fail(r, f)),
        };
        let backup_dir = done
            .first()
            .and_then(|d| Path::new(&d.backup).parent().and_then(Path::parent).map(|p| p.to_string_lossy().into_owned()))
            .unwrap_or_default();
        for f in &folders {
            if self.inner.plex.scan(f).await.is_err() {
                let (h, d) = (host.clone(), done.clone());
                let back = tokio::task::spawn_blocking(move || swap::undo(h.as_ref(), &d)).await.map_err(|e| AppError::Internal(e.to_string()))?;
                return Err(self.fail(r, back.err().unwrap_or(Failure::RescanFailed)));
            }
        }
        let from = chosen.iter().map(|s| s.from.clone()).next().unwrap_or_default();
        let to = chosen.iter().filter_map(|s| s.to.clone()).next().unwrap_or_default();
        let summary = summary_words(done.len(), None, &from, &to, true);
        let mut r = r;
        let n = done.len();
        for root in ignored {
            r.request.notes.push(format!("Added {} to .plexignore in {}, so Plex never shows the old files.", swap::IGNORE_RULE, root.to_string_lossy()));
        }
        r.swaps.push(SwapRun {
            id: swap_id,
            at: self.now(),
            backup_dir: backup_dir.clone(),
            files: done,
            undone: false,
            summary: summary.clone(),
        });
        r.failure = None;
        r.request.updated_at = self.now();
        self.upsert(&r)?;
        Ok(SwapResult { swapped: n, backup_dir, summary, record: r })
    }

    /// Put the old files of the last swap back exactly.
    pub async fn undo(&self, id: &str) -> AppResult<UpgradeRecord> {
        let _one = self.lock.lock().await;
        let mut r = self.record(id)?;
        let Some(run) = r.last_swap().cloned() else { return Err(AppError::Invalid("There's no swap to undo.".into())) };
        let Some(host) = self.inner.hosts.host() else { return Err(self.fail(r, Failure::NoMapping)) };
        let (h, d) = (host.clone(), run.files.clone());
        let back = tokio::task::spawn_blocking(move || swap::undo(h.as_ref(), &d)).await.map_err(|e| AppError::Internal(e.to_string()))?;
        if let Err(f) = back {
            return Err(self.fail(r, f));
        }
        let mut folders: Vec<String> = run.files.iter().map(|f| folder_of(&f.plex_path)).collect();
        folders.sort();
        folders.dedup();
        let mut rescanned = true;
        for f in &folders {
            rescanned &= self.inner.plex.scan(f).await.is_ok();
        }
        if let Some(s) = r.swaps.iter_mut().find(|s| s.id == run.id) {
            s.undone = true;
        }
        r.failure = None;
        if !rescanned {
            r.request.notes.push("The old files are back. Plex didn't take the rescan, so it shows them after its next scan.".into());
        }
        r.request.updated_at = self.now();
        self.upsert(&r)?;
        Ok(r)
    }

    /// Swap on its own what the rules allow, for upgrades Lidarr has
    /// imported, when "ask before each swap" is off.
    pub async fn auto_swap(&self) -> usize {
        if crate::settings::load(&self.inner.db).upgrades_ask {
            return 0;
        }
        let mut n = 0;
        let ready: Vec<UpgradeRecord> =
            self.records().into_iter().filter(|r| r.request.state == RequestState::Imported && r.swaps.is_empty() && r.failure.is_none()).collect();
        let practice = self.inner.lidarr.practice_active();
        for r in ready {
            let Ok(o) = self.offer(&r.request.id).await else { continue };
            if o.status == OfferStatus::Ready && !o.ask_each && !o.upgrade.is_empty() {
                if practice {
                    // Practice mode: say what would have been swapped, once.
                    if self.inner.lidarr.practice_noted(&format!("swap:{}", r.request.id)) {
                        let _ = self.inner.lidarr.log_practice(
                            &format!("Would swap {} songs on {} by {}", o.upgrade.len(), r.request.title, r.request.artist),
                            "Current copies",
                            "Better copies from staging",
                        );
                    }
                    continue;
                }
                if self.swap(&r.request.id, None).await.is_ok() {
                    n += 1;
                }
            }
        }
        n
    }
}

/// The provenance note a swap leaves on the new file: the old note with
/// where the better copy came from and when, in the tag the file uses.
fn note_edit(o: &OldSong, to: &str, today: &str) -> Option<NoteEdit> {
    let add = format!("Upgraded to {to} from Lidarr, {today}");
    let key = if o.tags.source.is_some() { NoteTag::ArchiveSource } else { o.status.note_tag };
    let value = match o.status.note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => format!("{n}; {add}"),
        None => add,
    };
    Some(NoteEdit { key: key.key().to_string(), value })
}

/// Compare the old files with the better copy (blocking: reads files).
/// What [`compare`] needs besides the files.
struct CompareCtx {
    /// A provenance note starting with this says official.
    prefix: String,
    /// "2026-10-16", for the note a swap adds.
    today: String,
    status: Arc<dyn StatusReader>,
}

fn compare(host: &dyn SwapHost, mut offer: Offer, a: &AlbumFiles, tracks: &[super::api::LidarrTrack], files: &[super::api::TrackFile], cx: &CompareCtx) -> Worked {
    let fs = host.fs();
    let unreachable = |mut offer: Offer, path: &Path| {
        offer.status = OfferStatus::Unreachable;
        offer.message = Some(format!("Platter can't reach {} from {}. Check the share is connected.", path.to_string_lossy(), host.label()));
        Worked { offer, paths: HashMap::new() }
    };
    let needs_mapping = |mut offer: Offer, what: &str| {
        offer.status = OfferStatus::NeedsMapping;
        offer.message = Some(format!("The share mapping doesn't cover {what}. Set it in Settings > Integrations > Lidarr."));
        Worked { offer, paths: HashMap::new() }
    };

    // The old files, and anything else in their folders.
    let mut olds: Vec<OldSong> = Vec::new();
    let mut locals: Vec<PathBuf> = Vec::new();
    let mut folders: Vec<PathBuf> = Vec::new();
    for s in &a.songs {
        let Some(file) = s.file.as_deref().filter(|f| !f.trim().is_empty()) else { continue };
        let Some(local) = host.music_path(file) else { return needs_mapping(offer, "your music folder") };
        let folder = local.parent().map(Path::to_path_buf).unwrap_or_default();
        if !fs.is_dir(&folder) {
            return unreachable(offer, &folder);
        }
        let tags = host.tags().read(&local).unwrap_or_default();
        let status = cx.status.file_status(&tags);
        if !folders.contains(&folder) {
            folders.push(folder);
        }
        locals.push(local);
        olds.push(OldSong {
            track_id: s.id.clone(),
            title: s.title.clone(),
            disc: s.disc,
            track: s.track,
            duration_ms: s.duration_ms,
            plex_path: file.to_string(),
            rank: s.rank,
            codec: s.codec.clone(),
            unreleased: s.unreleased,
            tags,
            status,
        });
    }
    let known: BTreeSet<String> = locals.iter().map(|p| NormPath::parse(&p.to_string_lossy()).key()).collect();
    for folder in &folders {
        let Ok(list) = fs.list(folder) else { return unreachable(offer, folder) };
        for p in list.into_iter().filter(|p| is_audio(p) && !known.contains(&NormPath::parse(&p.to_string_lossy()).key())) {
            olds.push(OldSong {
                track_id: String::new(),
                title: swap::name(&p),
                disc: 0,
                track: 0,
                duration_ms: 0,
                plex_path: p.to_string_lossy().into_owned(),
                rank: Rank(0),
                codec: None,
                unreleased: false,
                tags: Default::default(),
                status: cx.status.file_status(&Default::default()),
            });
        }
    }

    // The better copy, and the confirmed release's tracklist as Lidarr has it.
    let by_id: HashMap<i64, &super::api::TrackFile> = files.iter().map(|f| (f.id, f)).collect();
    let mut news: Vec<NewSong> = Vec::new();
    let mut new_locals: Vec<PathBuf> = Vec::new();
    let mut official: Vec<OfficialTrack> = Vec::new();
    for t in tracks {
        let mut entry = OfficialTrack {
            title: t.title.clone(),
            recording: t.foreign_recording_id.clone(),
            medium: t.medium_number.max(1),
            duration_ms: t.duration.max(0) as u64,
            new: None,
        };
        let file = by_id.get(&t.track_file_id).filter(|_| t.track_file_id > 0);
        let Some(f) = file else {
            official.push(entry);
            continue;
        };
        let Some(local) = host.staging_path(&f.path) else { return needs_mapping(offer, "the staging folder") };
        if !fs.exists(&local) {
            if !fs.is_dir(local.parent().unwrap_or(Path::new(""))) {
                return unreachable(offer, local.parent().unwrap_or(Path::new("")));
            }
            official.push(entry);
            continue;
        }
        let tags = host.tags().read(&local).unwrap_or_default();
        let audio = host.tags().audio(&local).ok();
        let (codec, rank) = match &audio {
            Some(x) => (x.codec.clone(), Rank::of_file(Some(&x.codec), x.kbps, x.bit_depth)),
            None => (String::new(), Rank(0)),
        };
        let duration_ms = audio.as_ref().and_then(|x| x.duration_ms).unwrap_or(entry.duration_ms);
        entry.new = Some(news.len());
        official.push(entry);
        news.push(NewSong { lidarr_path: f.path.clone(), duration_ms, rank, codec, tags });
        new_locals.push(local);
    }

    let old_locals: Vec<Option<PathBuf>> = olds.iter().map(|o| if o.track_id.is_empty() { None } else { host.music_path(&o.plex_path) }).collect();
    let mut corr = |o: usize, n: usize| -> Option<f64> {
        let a = old_locals.get(o).cloned().flatten()?;
        host.tags().correlate(&a, &new_locals[n]).ok()
    };
    let decisions = upmatch::decide(&olds, &official, &news, &cx.prefix, &mut corr);
    let mut paths = HashMap::new();
    for d in decisions {
        let o = &olds[d.old];
        let from = o.rank.label(o.codec.as_deref());
        let mut song = OfferSong { track_id: o.track_id.clone(), title: o.title.clone(), disc: o.disc, track: o.track, from, to: None, reason: None, keep: None, by: None };
        match d.outcome {
            Outcome::Upgrade { new, by } => {
                let n = &news[new];
                song.to = Some(n.rank.label(Some(&n.codec)));
                song.by = Some(by.to_string());
                let note = note_edit(o, &n.rank.label(Some(&n.codec)), &cx.today);
                paths.insert(o.track_id.clone(), (o.plex_path.clone(), n.lidarr_path.clone(), note));
                offer.upgrade.push(song);
            }
            Outcome::Keep(k) => {
                song.reason = Some(k.reason().to_string());
                song.keep = Some(k);
                offer.kept.push(song);
            }
        }
    }
    offer.ask_each = offer.kept.iter().any(|s| s.keep.is_some_and(Keep::asks_each));
    if offer.ask_each {
        offer.ask_reason = Some(
            "Some files in this album's folders are kept as they are (unreleased, not on the confirmed release, or no proof of a match), so Platter asks about each song."
                .into(),
        );
    }
    let total = a.songs.len();
    if offer.upgrade.is_empty() {
        offer.status = OfferStatus::Nothing;
        offer.message = Some("None of the songs can be swapped. Each one is kept as it is, with the reason below.".into());
    } else {
        offer.status = OfferStatus::Ready;
        let from = offer.upgrade.iter().map(|s| s.from.clone()).min_by_key(|s| s.len()).unwrap_or_default();
        let lowest = olds.iter().filter(|o| paths.contains_key(&o.track_id)).min_by_key(|o| o.rank).map(|o| o.rank.label(o.codec.as_deref()));
        let to = offer.upgrade.iter().filter_map(|s| s.to.clone()).next().unwrap_or_default();
        offer.from = lowest.unwrap_or(from);
        offer.to = to;
        offer.summary = Some(summary_words(offer.upgrade.len(), Some(total), &offer.from, &offer.to, false));
    }
    Worked { offer, paths }
}

/// The background loop: waiting upgrades are sent, sent ones followed, and
/// with asking off, imported ones swapped.
pub async fn watch(u: Upgrades) {
    tokio::time::sleep(Duration::from_secs(45)).await;
    let mut last_hunt: Option<std::time::Instant> = None;
    loop {
        // "Look for better copies" for protected artists, once a day (each
        // album at most weekly).
        if last_hunt.is_none_or(|t| t.elapsed() > Duration::from_secs(86_400)) {
            last_hunt = Some(std::time::Instant::now());
            if let Err(e) = u.hunt().await {
                tracing::debug!("better copies: {e}");
            }
        }
        let list = u.records();
        if list.iter().any(|r| !r.request.state.settled())
            && let Err(e) = u.refresh().await
        {
            tracing::debug!("upgrades: {e}");
        }
        let n = u.auto_swap().await;
        if n > 0 {
            tracing::info!(albums = n, "upgrades swapped on their own");
        }
        tokio::time::sleep(super::request::RETRY_EVERY).await;
    }
}
