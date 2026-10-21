//! Tap a song the library doesn't have, and it plays.
//!
//! A tap sends a quick request ([`Speed::Quick`]: only the album goes into
//! Lidarr, and the fastest good release is grabbed at once), then follows
//! the download every couple of seconds. As soon as the song's file is
//! complete and this device can open it, it plays from the download; when
//! Lidarr has imported it, Plex is asked to scan just that folder, and once
//! the library has the song, playback moves to Plex's copy at the same spot.
//! Without the file nearby it plays the moment the library has it.
//!
//! Every guard of a request stays: the same MusicBrainz checks, the whole
//! setup check before and after the writes, one search of the indexers per
//! tap, and a second tap on the same song while it's coming joins the
//! first. The app plays and shows it through [`TapHost`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::Lidarr;
use super::paths::NormPath;
use super::request::{GetNeeds, RequestInput, RequestRecord, Speed};
use super::status::{RequestState, map_status};
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};

/// How often a tap in flight is followed.
pub const TAP_EVERY: Duration = Duration::from_secs(2);
/// A download that hasn't moved for this long is said to have stalled.
pub const STALLED_AFTER: Duration = Duration::from_secs(120);
/// How long after the import Plex may take to show the song before it's
/// flagged (the download's copy keeps playing meanwhile).
pub const PLEX_WAIT: Duration = Duration::from_secs(10 * 60);
/// Taps kept in the list (the newest).
const KEEP_TAPS: usize = 20;
const LAST_GET_KEY: &str = "lidarr.lastGet";

pub const STALLED_NOTE: &str = "The download has stalled. Platter keeps waiting and plays it as soon as it's here.";
pub const AWAY_NOTE: &str = "Platter can't reach Lidarr from here right now. It keeps trying.";
pub const PLEX_LATE_NOTE: &str = "Plex hasn't shown this song yet, so it's playing from the download. Library doctor lists it.";

/// What the user sees in place of the seek bar while a tapped song comes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TapPhase {
    /// Asking the indexers.
    Searching,
    Downloading,
    /// Nothing usable on the indexers yet: it stays on the list and is
    /// searched again on the usual schedule.
    Waiting,
    /// Downloaded; Lidarr is moving it into the library.
    Importing,
    /// Playing (from the download or the library).
    Playing,
    /// In the library (a tap that wasn't the latest, or played already).
    Ready,
    Failed,
}

impl TapPhase {
    /// Still followed.
    pub fn live(self) -> bool {
        !matches!(self, TapPhase::Ready | TapPhase::Failed)
    }
}

/// When each step was reached, in ms after the tap.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TapTimings {
    /// The release was grabbed.
    pub found_ms: Option<u64>,
    /// The file was complete.
    pub downloaded_ms: Option<u64>,
    /// The song started.
    pub playing_ms: Option<u64>,
    /// The library had it.
    pub in_library_ms: Option<u64>,
}

/// A library track for a tapped song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryTrack {
    pub track_id: String,
    pub album_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tap {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub release_group_id: String,
    pub thumb: Option<String>,
    pub duration_ms: Option<u64>,
    pub phase: TapPhase,
    pub progress: Option<u8>,
    /// Why it's waiting, stalled or failed, in plain words.
    pub message: Option<String>,
    pub request_id: String,
    pub timings: TapTimings,
    /// The latest tap: the one that plays when it lands.
    pub wanted: bool,
    /// Playing from the finished download (until the library has it).
    pub from_download: bool,
    #[serde(skip)]
    pub file: Option<PathBuf>,
    #[serde(skip)]
    pub track: Option<LibraryTrack>,
    #[serde(skip)]
    started: Option<tokio::time::Instant>,
    #[serde(skip)]
    scanned: bool,
    #[serde(skip)]
    imported_at: Option<tokio::time::Instant>,
    #[serde(skip)]
    moved_at: Option<tokio::time::Instant>,
    #[serde(skip)]
    flagged: bool,
}

impl Tap {
    fn since(&self) -> Option<u64> {
        self.started.map(|s| s.elapsed().as_millis() as u64)
    }
}

/// What the app sends for a tap.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TapInput {
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub artist_id: Option<String>,
    #[serde(default)]
    pub artist_mbid: Option<String>,
    /// The album it's on (the catalogue's official one), when known.
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub release_group_id: Option<String>,
    #[serde(default)]
    pub isrc: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub thumb: Option<String>,
    /// The album's name typed, for the very first request ever.
    #[serde(default)]
    pub typed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TapOutcome {
    pub tap: Option<Tap>,
    pub needs: Option<GetNeeds>,
}

/// The last get's timings, for Settings > Lidarr.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastGet {
    pub title: String,
    pub at: i64,
    pub timings: TapTimings,
}

/// The app's side: showing taps and playing them.
#[async_trait]
pub trait TapHost: Send + Sync {
    /// A tap changed (for Now playing's "Getting it" line).
    fn changed(&self, tap: &Tap);
    /// Play the finished download now (the wanted tap).
    async fn play_file(&self, tap: &Tap, file: &Path) -> AppResult<()>;
    /// The library has it: play it, or move playback from the download to
    /// it at the same spot when the download is still playing. Returns
    /// whether it played.
    async fn play_library(&self, tap: &Tap, track: &LibraryTrack, from_download: bool) -> AppResult<bool>;
    /// Ask the library to catch up with the server.
    fn refresh_library(&self);
    /// Plex never showed it (for Library doctor).
    fn flag_missing(&self, _tap: &Tap) {}
    /// The library has a song that was flagged.
    fn found(&self, _tap: &Tap) {}
}

/// Taps on this device, followed until they play.
#[derive(Clone)]
pub struct Taps {
    lidarr: Lidarr,
    host: Arc<dyn TapHost>,
    list: Arc<Mutex<Vec<Tap>>>,
    /// One follow at a time.
    following: Arc<tokio::sync::Mutex<()>>,
}

/// Folded for comparing names: letters and digits, lower case.
pub fn fold(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

const AUDIO: &[&str] = &["flac", "mp3", "m4a", "aac", "ogg", "opus", "wav", "aiff", "aif", "alac", "wv", "ape", "dsf"];

fn is_audio(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO.contains(&e.to_ascii_lowercase().as_str()))
}

fn audio_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth > 0 {
            audio_files(&p, depth - 1, out);
        } else if p.is_file() && is_audio(&p) {
            out.push(p);
        }
    }
}

/// The song's file in a finished download: the file itself, the only audio
/// file in the folder, or the one whose name (else title tag) is the song's.
pub fn find_song(at: &Path, title: &str) -> Option<PathBuf> {
    if at.is_file() {
        return is_audio(at).then(|| at.to_path_buf());
    }
    let mut files = Vec::new();
    audio_files(at, 2, &mut files);
    files.sort();
    if files.len() == 1 {
        return files.pop();
    }
    let want = fold(title);
    if want.is_empty() {
        return None;
    }
    let stem = |p: &PathBuf| fold(p.file_stem().and_then(|s| s.to_str()).unwrap_or_default());
    // The name with its track number off ("01 - Dawn"), then a name with it in.
    let exact = files.iter().find(|p| stem(p).trim_start_matches(|c: char| c.is_ascii_digit()) == want);
    if let Some(p) = exact {
        return Some(p.clone());
    }
    let tagged = files.iter().find(|p| {
        use lofty::file::TaggedFileExt;
        use lofty::prelude::Accessor;
        lofty::read_from_path(p).ok().and_then(|f| f.primary_tag().and_then(|t| t.title().map(|s| fold(&s)))).is_some_and(|t| t == want)
    });
    tagged.or_else(|| files.iter().find(|p| stem(p).contains(&want))).cloned()
}

/// Where this device opens a path Lidarr names: through the music share
/// mapping, or as it is when Lidarr runs here.
pub fn local_path(share: &crate::musicshare::ShareMap, lidarr_path: &str) -> Option<PathBuf> {
    if lidarr_path.trim().is_empty() {
        return None;
    }
    share.music_path(lidarr_path).filter(|p| p.exists()).or_else(|| {
        let p = PathBuf::from(lidarr_path);
        (p.is_absolute() && p.exists()).then_some(p)
    })
}

/// The library's track for a tapped song: on the album Plex matched to the
/// release group, else on the album with its name by its artist.
pub fn library_track(c: &rusqlite::Connection, rgid: &str, album: &str, artist: &str, title: &str) -> AppResult<Option<LibraryTrack>> {
    let mut ids: Vec<String> = Vec::new();
    let mut st = c.prepare_cached("SELECT id FROM albums WHERE lower(mbid) = ?1")?;
    ids.extend(st.query_map([rgid.trim().to_ascii_lowercase()], |r| r.get::<_, String>(0))?.flatten());
    if ids.is_empty() && !album.trim().is_empty() {
        let (fa, fr) = (fold(album), fold(artist));
        let mut st = c.prepare_cached("SELECT id, title, artist_title FROM albums ORDER BY added_at DESC LIMIT 2000")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        ids.extend(rows.flatten().filter(|(_, t, a)| fold(t) == fa && fold(a) == fr).map(|(id, _, _)| id));
    }
    let want = fold(title);
    for id in ids {
        let tracks = crate::db::queries::album_tracks(c, &id)?;
        if let Some(t) = tracks.iter().find(|t| fold(&t.title) == want).or_else(|| (tracks.len() == 1).then(|| &tracks[0])) {
            return Ok(Some(LibraryTrack { track_id: t.id.clone(), album_id: id }));
        }
    }
    Ok(None)
}

/// What one look at Lidarr found for a tapped album.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probe {
    pub state: Option<RequestState>,
    pub progress: Option<u8>,
    /// The download is complete (or being imported) at this path.
    pub finished_at: Option<String>,
    /// Imported files, as Lidarr sees them.
    pub imported: Vec<String>,
}

impl Lidarr {
    /// Request a song's album the quick way.
    pub async fn tap_request(&self, input: RequestInput) -> AppResult<RequestRecord> {
        self.request_with(input, Speed::Quick).await
    }

    /// Lidarr's queue, history and files for one album. Reads only.
    pub async fn probe(&self, album_id: i64, rgid: &str) -> AppResult<Probe> {
        let (client, _) = self.connection().await?;
        let Some(album) = client.album_by_release_group(rgid).await? else {
            return Ok(Probe { state: Some(RequestState::Failed), ..Default::default() });
        };
        let queue = client.queue(&[album_id]).await.unwrap_or_default();
        let history = client.history(album_id).await.unwrap_or_default();
        let state = map_status(&album, &queue, &history);
        let mine = queue.iter().find(|q| q.album_id == Some(album_id));
        let done = mine.is_some_and(|q| {
            let t = q.tracked_download_state.as_deref().unwrap_or_default().to_ascii_lowercase();
            q.status.eq_ignore_ascii_case("completed") || matches!(t.as_str(), "importpending" | "importing") || q.sizeleft.is_some_and(|l| l <= 0.0)
        });
        let finished_at = mine.filter(|_| done).and_then(|q| q.output_path.clone()).filter(|p| !p.trim().is_empty());
        let imported = if state == RequestState::Imported {
            client.track_files(album_id).await.unwrap_or_default().into_iter().map(|f| f.path).collect()
        } else {
            Vec::new()
        };
        Ok(Probe { state: Some(state), progress: super::status::progress(album_id, &queue), finished_at, imported })
    }

    /// The last get's timings.
    pub fn last_get(&self) -> Option<LastGet> {
        self.inner.db.read(|c| kv_get(c, LAST_GET_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
    }

    fn save_last_get(&self, g: &LastGet) {
        if let Ok(json) = serde_json::to_string(g) {
            let _ = self.inner.db.write(|c| kv_set(c, LAST_GET_KEY, &json));
        }
    }

    fn plex_scan(&self) -> Arc<dyn super::plexside::PlexServer> {
        self.inner.plex_server.clone()
    }

    fn db(&self) -> &crate::db::DbHandle {
        &self.inner.db
    }
}

fn phase_of(r: &RequestRecord) -> TapPhase {
    match r.state {
        RequestState::Queued | RequestState::Requested | RequestState::Searching if r.message.as_deref().is_some_and(|m| m.contains(super::quick::NO_COPY)) => TapPhase::Waiting,
        RequestState::Queued | RequestState::Requested | RequestState::Searching => TapPhase::Searching,
        RequestState::NotFound => TapPhase::Waiting,
        RequestState::Downloading => TapPhase::Downloading,
        RequestState::Importing | RequestState::Imported => TapPhase::Importing,
        RequestState::Failed => TapPhase::Failed,
    }
}

/// The folder Plex scans for an imported song: its file's folder.
fn song_folder(imported: &[String], title: &str) -> Option<String> {
    let want = fold(title);
    let file = imported.iter().find(|p| fold(p.rsplit(['/', '\\']).next().unwrap_or_default()).contains(&want)).or(imported.first())?;
    let n = NormPath::parse(file);
    (n.len() > 1).then(|| file[..file.rfind(['/', '\\']).unwrap_or(0)].to_string()).filter(|f| !f.is_empty())
}

impl Taps {
    pub fn new(lidarr: Lidarr, host: Arc<dyn TapHost>) -> Self {
        Self { lidarr, host, list: Default::default(), following: Default::default() }
    }

    pub fn list(&self) -> Vec<Tap> {
        self.list.lock().clone()
    }

    fn settings(&self) -> crate::settings::Settings {
        crate::settings::load(self.lidarr.db())
    }

    /// A song was tapped.
    pub async fn tap(&self, input: TapInput) -> AppResult<TapOutcome> {
        if !self.settings().lidarr_tap_to_get {
            return Err(AppError::Invalid("Getting songs with a tap is off. Turn it on in Settings > Lidarr > More.".into()));
        }
        if !self.lidarr.status().usable() {
            return Err(AppError::Invalid("Set up Lidarr in Settings to get songs you don't have.".into()));
        }
        let started = tokio::time::Instant::now();
        let rgid = match input.release_group_id.as_deref().map(str::trim).filter(|r| crate::artistinfo::is_mbid(r)) {
            Some(r) => r.to_ascii_lowercase(),
            None => {
                let picked = self.lidarr.pick(&input.artist, input.artist_id.as_deref(), &input.title, true, input.isrc.as_deref()).await?;
                match picked.group {
                    Some(g) => g.id,
                    None => {
                        let reason = picked.reason.unwrap_or_else(|| super::mbpick::Refusal::NotFound.message());
                        return Ok(TapOutcome { tap: None, needs: Some(GetNeeds { kind: "refused".into(), reason }) });
                    }
                }
            }
        };
        // Coming already: the same tap, no second grab.
        let fold_title = fold(&input.title);
        if let Some(t) = {
            let mut l = self.list.lock();
            for t in l.iter_mut() {
                t.wanted = false;
            }
            let hit = l.iter_mut().find(|t| t.phase != TapPhase::Failed && t.release_group_id == rgid && fold(&t.title) == fold_title);
            hit.map(|t| {
                t.wanted = true;
                t.clone()
            })
        } {
            tracing::info!(title = %t.title, "lidarr tap: already coming");
            self.host.changed(&t);
            return Ok(TapOutcome { tap: Some(t), needs: None });
        }
        let album = input.album.clone().unwrap_or_default();
        let req = RequestInput {
            release_group_id: rgid.clone(),
            title: if album.trim().is_empty() { input.title.clone() } else { album.clone() },
            artist: input.artist.clone(),
            artist_id: input.artist_id.clone(),
            artist_mbid: input.artist_mbid.clone(),
            typed: input.typed.clone(),
        };
        if !self.lidarr.typed_confirmed() && req.typed.is_none() {
            return Ok(TapOutcome { tap: None, needs: Some(GetNeeds { kind: "firstTime".into(), reason: "Your first request is confirmed by typing the album's name, once.".into() }) });
        }
        tracing::info!(title = %input.title, release_group = %rgid, "lidarr tap: sent");
        let rec = match self.lidarr.tap_request(req).await {
            Ok(r) => r,
            Err(e) if matches!(e.kind(), "invalid" | "notFound") => {
                tracing::info!(reason = %e.message(), "lidarr tap: needs you");
                return Ok(TapOutcome { tap: None, needs: Some(GetNeeds { kind: "refused".into(), reason: e.message() }) });
            }
            Err(e) => return Err(e),
        };
        let mut t = Tap {
            id: uuid::Uuid::new_v4().to_string(),
            title: input.title.trim().to_string(),
            artist: input.artist.trim().to_string(),
            album: if album.trim().is_empty() { rec.title.clone() } else { album },
            release_group_id: rgid,
            thumb: input.thumb.clone(),
            duration_ms: input.duration_ms,
            phase: phase_of(&rec),
            progress: rec.progress,
            message: rec.message.clone(),
            request_id: rec.id.clone(),
            timings: TapTimings::default(),
            wanted: true,
            from_download: false,
            file: None,
            track: None,
            started: Some(started),
            scanned: false,
            imported_at: None,
            moved_at: Some(started),
            flagged: false,
        };
        if rec.state == RequestState::Downloading {
            t.timings.found_ms = t.since();
        }
        if rec.state == RequestState::Queued && rec.message.as_deref() == Some(super::request::QUEUED_NOTE) {
            t.message = Some(AWAY_NOTE.into());
        }
        {
            let mut l = self.list.lock();
            l.insert(0, t.clone());
            l.truncate(KEEP_TAPS);
        }
        self.host.changed(&t);
        Ok(TapOutcome { tap: Some(t), needs: None })
    }

    /// Follow every live tap once.
    pub async fn follow(&self) {
        let _one = self.following.lock().await;
        let live: Vec<Tap> = self.list.lock().iter().filter(|t| t.phase.live()).cloned().collect();
        for t in live {
            let next = self.step(t).await;
            let changed = {
                let mut l = self.list.lock();
                match l.iter_mut().find(|x| x.id == next.id) {
                    Some(x) => {
                        // A tap that happened meanwhile owns `wanted`.
                        let wanted = x.wanted;
                        let changed = *x != Tap { wanted, ..next.clone() };
                        *x = Tap { wanted, ..next };
                        changed.then(|| x.clone())
                    }
                    None => None,
                }
            };
            if let Some(t) = changed {
                self.host.changed(&t);
            }
        }
    }

    /// One look at one tap.
    async fn step(&self, mut t: Tap) -> Tap {
        let Some(rec) = self.lidarr.requests().into_iter().find(|r| r.id == t.request_id) else {
            t.phase = TapPhase::Failed;
            t.message = Some("The request for this song isn't on this device any more.".into());
            return t;
        };
        let Some(album_id) = rec.lidarr_album_id else {
            // Still waiting to reach Lidarr: the background retry sends it.
            t.phase = if rec.state == RequestState::Failed { TapPhase::Failed } else { TapPhase::Searching };
            t.message = if rec.state == RequestState::Failed { rec.message.clone() } else { Some(AWAY_NOTE.into()) };
            if rec.state != RequestState::Failed {
                self.lidarr.retry_queued().await.ok();
            }
            return t;
        };
        // In the library already (an earlier get, or Plex was quick).
        if t.track.is_none() {
            let (rgid, album, artist, title) = (t.release_group_id.clone(), t.album.clone(), t.artist.clone(), t.title.clone());
            if let Ok(Some(track)) = self.lidarr.db().read(|c| library_track(c, &rgid, &album, &artist, &title)) {
                return self.in_library(t, track).await;
            }
        }
        let probe = match self.lidarr.probe(album_id, &rec.release_group_id).await {
            Ok(p) => p,
            Err(e) if e.kind() == "network" => {
                if t.phase != TapPhase::Playing {
                    t.message = Some(AWAY_NOTE.into());
                }
                return t;
            }
            Err(e) => {
                tracing::info!(error = %e, title = %t.title, "lidarr tap: couldn't look at the download; looks again");
                return t;
            }
        };
        let state = probe.state.unwrap_or(rec.state);
        let was = (t.phase, t.progress);
        match state {
            RequestState::Failed => {
                t.phase = TapPhase::Failed;
                t.message = rec.message.clone().or_else(|| Some("Lidarr couldn't download or import it. Lidarr's Activity page says why.".into()));
                tracing::info!(title = %t.title, reason = t.message.as_deref().unwrap_or_default(), "lidarr tap: failed");
                return t;
            }
            RequestState::Searching | RequestState::NotFound | RequestState::Requested | RequestState::Queued => {
                if t.phase != TapPhase::Waiting {
                    t.phase = phase_of(&rec);
                }
                t.message = rec.message.clone();
            }
            RequestState::Downloading => {
                if t.timings.found_ms.is_none() {
                    t.timings.found_ms = t.since();
                }
                if t.phase != TapPhase::Playing {
                    t.phase = TapPhase::Downloading;
                }
                t.progress = probe.progress;
                t.message = None;
            }
            RequestState::Importing | RequestState::Imported => {
                if t.timings.found_ms.is_none() {
                    t.timings.found_ms = t.since();
                }
                if t.phase != TapPhase::Playing {
                    t.phase = TapPhase::Importing;
                }
                t.progress = None;
                t.message = None;
            }
        }
        if (t.phase, t.progress) != was {
            t.moved_at = Some(tokio::time::Instant::now());
        } else if t.phase == TapPhase::Downloading && t.moved_at.is_some_and(|m| m.elapsed() >= STALLED_AFTER) {
            if t.message.is_none() {
                tracing::info!(title = %t.title, progress = ?t.progress, "lidarr tap: download stalled");
            }
            t.message = Some(STALLED_NOTE.into());
        }

        // The file is complete: play it from the download when it's in reach.
        if let Some(path) = probe.finished_at.as_deref() {
            if t.timings.downloaded_ms.is_none() {
                t.timings.downloaded_ms = t.since();
                tracing::info!(title = %t.title, ms = t.timings.downloaded_ms, "lidarr tap: downloaded");
            }
            if t.file.is_none() && self.settings().lidarr_play_arriving {
                let share = crate::musicshare::load(self.lidarr.db());
                t.file = local_path(&share, path).and_then(|p| find_song(&p, &t.title));
            }
            if let Some(file) = t.file.clone()
                && t.wanted
                && !t.from_download
                && t.phase != TapPhase::Playing
            {
                match self.host.play_file(&t, &file).await {
                    Ok(()) => {
                        t.phase = TapPhase::Playing;
                        t.from_download = true;
                        t.timings.playing_ms = t.since();
                        tracing::info!(title = %t.title, ms = t.timings.playing_ms, "lidarr tap: playing from the download");
                    }
                    Err(e) => tracing::info!(error = %e, "lidarr tap: couldn't play the download; waits for the library"),
                }
            }
        }

        // Imported: scan just its folder, and look for it in the library.
        if state == RequestState::Imported {
            if t.timings.downloaded_ms.is_none() {
                t.timings.downloaded_ms = t.since();
            }
            if t.imported_at.is_none() {
                t.imported_at = Some(tokio::time::Instant::now());
            }
            if !t.scanned
                && let Some(folder) = song_folder(&probe.imported, &t.title)
            {
                t.scanned = true;
                match self.lidarr.plex_scan().scan_folder(&folder).await {
                    Ok(()) => tracing::info!(title = %t.title, "lidarr tap: asked Plex to scan the album's folder"),
                    Err(e) => tracing::info!(error = %e, "lidarr tap: Plex's scan didn't go out; its own scan finds it"),
                }
            }
            self.host.refresh_library();
            if !t.flagged && t.imported_at.is_some_and(|i| i.elapsed() >= PLEX_WAIT) {
                t.flagged = true;
                tracing::info!(title = %t.title, "lidarr tap: Plex hasn't shown it");
                self.host.flag_missing(&t);
                if t.from_download {
                    t.message = Some(PLEX_LATE_NOTE.into());
                }
            }
        }
        t
    }

    /// The library has the song: play it (or move to it), and remember the timings.
    async fn in_library(&self, mut t: Tap, track: LibraryTrack) -> Tap {
        if t.timings.in_library_ms.is_none() {
            t.timings.in_library_ms = t.since();
        }
        if t.timings.downloaded_ms.is_none() {
            t.timings.downloaded_ms = t.timings.in_library_ms;
        }
        let played = if t.wanted || t.from_download { self.host.play_library(&t, &track, t.from_download).await.unwrap_or(false) } else { false };
        if played && t.timings.playing_ms.is_none() {
            t.timings.playing_ms = t.since();
        }
        tracing::info!(title = %t.title, found = ?t.timings.found_ms, downloaded = ?t.timings.downloaded_ms, playing = ?t.timings.playing_ms, "lidarr tap: in the library");
        self.lidarr.save_last_get(&LastGet { title: t.title.clone(), at: (self.lidarr.inner.clock)(), timings: t.timings.clone() });
        if t.flagged {
            self.host.found(&t);
        }
        t.track = Some(track);
        t.phase = TapPhase::Ready;
        t.from_download = false;
        t.progress = None;
        t.message = None;
        t
    }

    /// Follows taps while any is live.
    pub async fn run(self) {
        loop {
            if self.list.lock().iter().any(|t| t.phase.live()) {
                self.follow().await;
            }
            tokio::time::sleep(TAP_EVERY).await;
        }
    }
}

#[cfg(test)]
pub mod tests_support {
    use super::*;

    /// A tap as the app would see one, for tests outside this module.
    pub fn tap(id: &str, title: &str, artist: &str, album: &str) -> Tap {
        Tap {
            id: id.into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            release_group_id: String::new(),
            thumb: None,
            duration_ms: Some(201_000),
            phase: TapPhase::Downloading,
            progress: None,
            message: None,
            request_id: String::new(),
            timings: TapTimings::default(),
            wanted: true,
            from_download: false,
            file: None,
            track: None,
            started: None,
            scanned: false,
            imported_at: None,
            moved_at: None,
            flagged: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_fold_to_letters_and_digits() {
        assert_eq!(fold("Dawn (Live) - 2019!"), "dawnlive2019");
        assert_eq!(fold("  "), "");
    }

    #[test]
    fn the_songs_file_is_found_in_a_finished_download() {
        let d = tempfile::tempdir().unwrap();
        let album = d.path().join("Northlight - Second Wind (2024) [FLAC]");
        std::fs::create_dir_all(album.join("CD1")).unwrap();
        for f in ["CD1/01 - Rise.flac", "CD1/02 - Dawn.flac", "CD1/03 - Dawn Again.flac", "cover.jpg"] {
            std::fs::write(album.join(f), b"x").unwrap();
        }
        assert_eq!(find_song(&album, "Dawn").unwrap(), album.join("CD1/02 - Dawn.flac"));
        assert_eq!(find_song(&album, "Rise").unwrap(), album.join("CD1/01 - Rise.flac"));
        assert!(find_song(&album, "Nothing Like It").is_none());
        let single = d.path().join("single");
        std::fs::create_dir(&single).unwrap();
        std::fs::write(single.join("track.mp3"), b"x").unwrap();
        assert_eq!(find_song(&single, "Whatever It's Called").unwrap(), single.join("track.mp3"), "the only song there");
        assert_eq!(find_song(&single.join("track.mp3"), "x").unwrap(), single.join("track.mp3"));
        assert!(find_song(&album.join("cover.jpg"), "x").is_none());
    }

    #[test]
    fn a_download_is_opened_through_the_share_mapping_or_where_lidarr_has_it() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("complete/Second Wind")).unwrap();
        let share = crate::musicshare::ShareMap {
            music: vec![crate::musicshare::FolderPair { server: "/data".into(), local: d.path().to_string_lossy().into() }],
            ..Default::default()
        };
        assert_eq!(local_path(&share, "/data/complete/Second Wind").unwrap(), d.path().join("complete/Second Wind"));
        assert!(local_path(&share, "/data/complete/Nope").is_none(), "not there (yet)");
        let here = d.path().join("complete/Second Wind");
        assert_eq!(local_path(&Default::default(), &here.to_string_lossy()).unwrap(), here, "Lidarr on this computer");
        assert!(local_path(&Default::default(), "").is_none());
    }

    #[test]
    fn plex_scans_the_imported_songs_folder() {
        let files = vec!["/music/Northlight/Second Wind (2024)/01 - Rise.flac".to_string(), "/music/Northlight/Second Wind (2024)/02 - Dawn.flac".to_string()];
        assert_eq!(song_folder(&files, "Dawn").as_deref(), Some("/music/Northlight/Second Wind (2024)"));
        assert_eq!(song_folder(&[], "Dawn"), None);
    }
}
