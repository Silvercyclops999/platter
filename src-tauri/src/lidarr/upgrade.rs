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
use super::upmatch::{self, Keep, NewSong, OldSong, Outcome};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};

const KEY: &str = "lidarr.upgrades";
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
    /// Platter's own folder, for backups when none is set.
    pub fallback: PathBuf,
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
            fallback_backup: self.fallback.clone(),
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
}

#[derive(Debug, Clone)]
struct AlbumFiles {
    title: String,
    artist: String,
    artist_id: String,
    unreleased: bool,
    songs: Vec<Song>,
}

impl AlbumFiles {
    /// The lowest quality among released songs.
    fn current(&self) -> Option<(Rank, String)> {
        self.songs.iter().filter(|s| !s.unreleased).min_by_key(|s| s.rank).map(|s| (s.rank, s.rank.label(s.codec.as_deref())))
    }
}

fn album_files(c: &Connection, album_id: &str) -> AppResult<Option<AlbumFiles>> {
    use crate::db::release::{album_unreleased, track_unreleased};
    let row = c
        .query_row(
            "SELECT al.title, al.artist_id, al.artist_title, al.format, al.subformats, at.version, al.matched, al.studio, mc.found
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
        "SELECT t.id, t.title, t.disc_number, t.track_number, t.codec, t.bitrate, t.bit_depth, t.file, ft.version
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
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|(id, title, disc, track, codec, kbps, bits, file, fv)| {
            let v = fv.as_deref().filter(|v| !v.trim().is_empty()).or(version.as_deref());
            let unreleased = unreleased || track_unreleased(&id, album_id, format.as_deref(), &subs, matched, studio.as_deref(), v, mb);
            Song { rank: Rank::of_file(codec.as_deref(), kbps, bits), id, title, disc, track, codec, file, unreleased }
        })
        .collect();
    Ok(Some(AlbumFiles { title, artist, artist_id, unreleased, songs }))
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
    /// Track id → (Plex path, Lidarr path) for each song that upgrades.
    paths: HashMap<String, (String, String)>,
}

impl Upgrades {
    pub fn new(deps: UpgradeDeps) -> Self {
        Self { inner: Arc::new(deps), lock: Arc::new(tokio::sync::Mutex::new(())) }
    }

    fn lidarr(&self) -> &Lidarr {
        &self.inner.lidarr
    }

    fn now(&self) -> i64 {
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

    async fn album(&self, album_id: &str) -> AppResult<AlbumFiles> {
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
        Ok(AlbumUpgrade { available, record, quality })
    }

    /// Ask Lidarr for a better copy of a library album, into the staging folder.
    pub async fn request(&self, album_id: &str) -> AppResult<UpgradeRecord> {
        let _one = self.lock.lock().await;
        self.request_locked(album_id).await
    }

    async fn request_locked(&self, album_id: &str) -> AppResult<UpgradeRecord> {
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
        let picked = l.picker().pick_upgrade(&want, &a.title, &library, album_id).await?;
        let Some(group) = picked.group else {
            return Err(AppError::Invalid(picked.reason.unwrap_or_else(|| super::mbpick::Refusal::NotFound.message())));
        };
        let input = RequestInput {
            release_group_id: group.id.clone(),
            title: group.title.clone(),
            artist: a.artist.clone(),
            artist_id: Some(a.artist_id.clone()),
            ..Default::default()
        };
        let rec = l.new_record(&input, group.id.clone(), false);
        let dest = Dest::Staging { album_id: album_id.to_string(), current, current_label: label.clone() };
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
        let host = self.inner.hosts.host();
        let backup = host
            .as_ref()
            .map(|h| h.backup_root().to_string_lossy().into_owned())
            .unwrap_or_else(|| crate::musicshare::load(&self.inner.db).backup);
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
        tokio::task::spawn_blocking(move || compare(h.as_ref(), offer, &a, &tracks, &files))
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
            .filter_map(|s| worked.paths.get(&s.track_id).map(|(p, l)| Planned { track_id: s.track_id.clone(), plex_path: p.clone(), lidarr_path: l.clone() }))
            .collect();
        let mut folders: Vec<String> = items.iter().map(|i| folder_of(&i.plex_path)).collect();
        folders.sort();
        folders.dedup();
        if self.inner.plex.can_scan(&folders).await.is_err() {
            return Err(self.fail(r, Failure::PlexUnreachable));
        }
        let Some(host) = self.inner.hosts.host() else { return Err(self.fail(r, Failure::NoMapping)) };
        let swap_id = uuid::Uuid::new_v4().to_string();
        let backup_dir = swap::backup_folder(&host.backup_root(), &offer.artist, &offer.title, &swap_id);
        let (h, b, it) = (host.clone(), backup_dir.clone(), items.clone());
        let ran = tokio::task::spawn_blocking(move || swap::run(h.as_ref(), &b, &it)).await.map_err(|e| AppError::Internal(e.to_string()))?;
        let done = match ran {
            Ok(d) => d,
            Err(f) => return Err(self.fail(r, f)),
        };
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
        r.swaps.push(SwapRun {
            id: swap_id,
            at: self.now(),
            backup_dir: backup_dir.to_string_lossy().into_owned(),
            files: done,
            undone: false,
            summary: summary.clone(),
        });
        r.failure = None;
        r.request.updated_at = self.now();
        self.upsert(&r)?;
        Ok(SwapResult { swapped: n, backup_dir: backup_dir.to_string_lossy().into_owned(), summary, record: r })
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
        for r in ready {
            let Ok(o) = self.offer(&r.request.id).await else { continue };
            if o.status == OfferStatus::Ready && !o.ask_each && !o.upgrade.is_empty() && self.swap(&r.request.id, None).await.is_ok() {
                n += 1;
            }
        }
        n
    }
}

/// Compare the old files with the better copy (blocking: reads files).
fn compare(host: &dyn SwapHost, mut offer: Offer, a: &AlbumFiles, tracks: &[super::api::LidarrTrack], files: &[super::api::TrackFile]) -> Worked {
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
        if !folders.contains(&folder) {
            folders.push(folder);
        }
        locals.push(local);
        olds.push(OldSong {
            track_id: s.id.clone(),
            title: s.title.clone(),
            disc: s.disc,
            track: s.track,
            plex_path: file.to_string(),
            rank: s.rank,
            codec: s.codec.clone(),
            unreleased: s.unreleased,
            tags,
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
                plex_path: p.to_string_lossy().into_owned(),
                rank: Rank(0),
                codec: None,
                unreleased: false,
                tags: Default::default(),
            });
        }
    }

    // The better copy.
    let by_id: HashMap<i64, &super::api::TrackFile> = files.iter().map(|f| (f.id, f)).collect();
    let mut news: Vec<NewSong> = Vec::new();
    for t in tracks.iter().filter(|t| t.track_file_id > 0) {
        let Some(f) = by_id.get(&t.track_file_id) else { continue };
        let Some(local) = host.staging_path(&f.path) else { return needs_mapping(offer, "the staging folder") };
        if !fs.exists(&local) {
            if !fs.is_dir(local.parent().unwrap_or(Path::new(""))) {
                return unreachable(offer, local.parent().unwrap_or(Path::new("")));
            }
            continue;
        }
        let tags = host.tags().read(&local).unwrap_or_default();
        let audio = host.tags().audio(&local).ok();
        let (codec, rank) = match &audio {
            Some(x) => (x.codec.clone(), Rank::of_file(Some(&x.codec), x.kbps, x.bit_depth)),
            None => (String::new(), Rank(0)),
        };
        news.push(NewSong { lidarr_path: f.path.clone(), medium: t.medium_number.max(1), lidarr_recording: t.foreign_recording_id.clone(), rank, codec, tags });
    }
    let mediums: BTreeSet<i64> = tracks.iter().map(|t| t.medium_number.max(1)).collect();

    let decisions = upmatch::decide(&olds, &news, &mediums);
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
                paths.insert(o.track_id.clone(), (o.plex_path.clone(), n.lidarr_path.clone()));
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
            "Some files in this album's folders are kept as they are (unreleased, archive tags, an extra disc or no proof of a match), so Platter asks about each song."
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
    loop {
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
