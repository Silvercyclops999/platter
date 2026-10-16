//! Adding a protected artist's new album from the staging folder to the
//! library. Lidarr only ever downloads protected artists into staging;
//! Platter makes every change in the library itself, by copying.
//!
//! An album is added only when it's complete (a partial import never is),
//! its specific release is Official with no secondary types, it passes the
//! artist's release group allowlist, and its title clashes with nothing the
//! artist has. Anything else waits in staging as "Waiting for you", with why.
//!
//! The copy follows the artist's own conventions (`conventions`): folder and
//! file names, disc tags and tag habits, with the provenance note saying it
//! came from Lidarr. Existing files are never renamed or retagged. The first
//! album per artist always asks, with a preview of the exact names and
//! tags; "Do this automatically" makes later ones automatic, except when
//! the conventions are low confidence. In practice mode, automatic adds are
//! only logged.
//!
//! After a copy the order is fixed, so nothing is downloaded twice:
//! (1) the files are in the library, (2) the album is unmonitored in Lidarr
//! (kept monitored while "Look for better copies" waits for the cutoff),
//! (3) only then, with "Free up staging space" on, its staging copy is
//! freed. Staging is never freed while the album is monitored.
//!
//! Undo moves the added folder into the ignored backup folder. Plex keeps
//! items across moves only while its trash isn't emptied, so with "Empty
//! trash automatically after every scan" on, Platter asks before turning it
//! off for the move and turns it back on after.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Lidarr;
use super::api::{AlbumDetail, LidarrTrack, TrackFile};
use super::catchup::{CatchUpReport, Ctx};
use super::changes::Undo;
use super::conventions::{self, AlbumSample, Conventions, FileSample, NewFile, PlannedAlbum};
use super::mbpick::{Want, clash};
use super::modes::{ArtistMode, ArtistPrefs, parent_raw};
use super::swap::{self, SwapHost};
use super::write::{StagingFiles, WriteCall};
use crate::error::{AppError, AppResult};

/// Most existing albums read to learn an artist's conventions.
const SAMPLE_ALBUMS: usize = 6;
/// A week of practice after setup.
pub const PRACTICE_S: i64 = 7 * 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StagedState {
    /// In staging, waiting for the user, with why.
    Waiting,
    /// Ready to add; asks first (with the preview).
    Ready,
    /// Practice mode: would have been added on its own.
    Practice,
    Added,
    /// Added, then put in the backup folder.
    Undone,
}

/// A protected album that landed in staging (shared).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Staged {
    pub id: String,
    pub album_id: i64,
    pub lidarr_artist_id: i64,
    pub artist: String,
    pub title: String,
    pub release_group: String,
    /// Lidarr imported only part of it.
    pub partial: bool,
    pub state: StagedState,
    pub reason: Option<String>,
    /// The exact folder, file names and tags it would get.
    #[serde(default)]
    pub plan: Option<PlannedAlbum>,
    /// The library folder (server path), planned or added.
    #[serde(default)]
    pub folder: Option<String>,
    /// Conventions learned from two or more agreeing albums.
    #[serde(default)]
    pub confident: bool,
    #[serde(default)]
    pub doubts: Vec<String>,
    /// Added on its own (after the user's "Do this automatically").
    #[serde(default)]
    pub auto: bool,
    pub at: i64,
    pub updated_at: i64,
}

/// What a staged album can become now.
enum Decision {
    Wait(String),
    Ready { plan: Box<PlannedAlbum>, folder: String, conv: Box<Conventions> },
}

/// Whether the release itself is Official: the specific release (the one
/// Lidarr has the album on), not Withdrawn or Cancelled, no secondary types.
pub fn official(a: &AlbumDetail) -> Result<(), String> {
    let r = a.current_release().ok_or_else(|| "Lidarr doesn't say which release it is".to_string())?;
    let st = r.status.trim().to_ascii_lowercase();
    if st != "official" {
        let shown = if st.is_empty() { "no status".to_string() } else { r.status.clone() };
        return Err(format!("the release is {shown}, not Official"));
    }
    if ["withdrawn", "cancelled", "canceled"].iter().any(|w| r.title.to_ascii_lowercase().contains(w)) {
        return Err("the release was withdrawn or cancelled".into());
    }
    if let Some(t) = a.secondary_types.iter().find(|t| !t.trim().is_empty()) {
        return Err(format!("it's a {t} release, not a studio album"));
    }
    Ok(())
}

/// The new album's files with their disc, track and title from Lidarr.
pub fn new_files(files: &[TrackFile], tracks: &[LidarrTrack], tags: &dyn Fn(&str) -> Vec<(String, String)>) -> Vec<NewFile> {
    let discs = tracks.iter().map(|t| t.medium_number).max().unwrap_or(1).max(1) as u32;
    let mut out: Vec<NewFile> = files
        .iter()
        .filter_map(|f| {
            let t = tracks.iter().find(|t| t.track_file_id == f.id)?;
            let disc_tracks: i64 = tracks.iter().filter(|x| x.medium_number < t.medium_number).count() as i64;
            let ext = Path::new(&f.path).extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
            Some(NewFile {
                src: f.path.clone(),
                ext,
                disc: t.medium_number.max(1) as u32,
                track: (t.absolute_track_number - disc_tracks).max(1) as u32,
                discs,
                title: t.title.clone(),
                tags: tags(&f.path),
            })
        })
        .collect();
    out.sort_by_key(|f| (f.disc, f.track));
    out
}

/// A quality's name from a track file ("FLAC", "FLAC 24bit", "MP3-320").
fn quality_name(f: &TrackFile) -> String {
    f.quality.as_ref().and_then(|q| q["quality"]["name"].as_str().map(str::to_string)).unwrap_or_else(|| "unknown quality".into())
}

impl Lidarr {
    /// Practice mode: automatic adds and swaps are only logged.
    pub fn practice_active(&self) -> bool {
        let now = (self.inner.clock)();
        self.practice_until().is_some_and(|u| u > now)
    }

    /// When practice mode ends (unix seconds): the setting when the user
    /// chose (0 is off), else a week after setup.
    pub fn practice_until(&self) -> Option<i64> {
        match crate::settings::load(&self.inner.db).lidarr_practice_until {
            -1 => self.inner.shared.get::<i64>("practiceStart").map(|s| s + PRACTICE_S),
            0 => None,
            u => Some(u),
        }
    }

    /// True the first time `what` is noted in practice mode (shared).
    pub fn practice_noted(&self, what: &str) -> bool {
        let key = format!("practice:{what}");
        if self.inner.shared.get::<i64>(&key).is_some() {
            return false;
        }
        self.inner.shared.put(&key, &(self.inner.clock)()).is_ok()
    }

    /// Practice mode for the first week after setup: started at the first
    /// check that lets Managed artists work.
    pub(super) fn start_practice(&self) {
        if self.inner.shared.get::<i64>("practiceStart").is_none() {
            let _ = self.inner.shared.put("practiceStart", &(self.inner.clock)());
        }
    }

    /// Where files are reached (the app sets it).
    pub fn set_hosts(&self, h: std::sync::Arc<dyn super::upgrade::Hosts>) {
        *self.hosts.write() = Some(h);
    }

    fn host(&self) -> Option<std::sync::Arc<dyn SwapHost>> {
        self.hosts.read().as_ref().and_then(|h| h.host())
    }

    fn staged_key(server: &str, album_id: i64) -> String {
        format!("staged:{server}:{album_id}")
    }

    /// Every staged album, newest first.
    pub fn staged(&self) -> Vec<Staged> {
        let mut l: Vec<Staged> = self.inner.shared.list::<Staged>("staged:").into_iter().map(|(_, s)| s).collect();
        l.sort_by_key(|s| std::cmp::Reverse(s.at));
        l
    }

    /// A release group Platter added to the library.
    pub(super) fn promoted_group(&self, rgid: &str) -> bool {
        self.staged().iter().any(|s| s.state == StagedState::Added && s.release_group.eq_ignore_ascii_case(rgid))
    }

    /// A protected album landed in staging (from the catch-up).
    pub(super) async fn on_staged(&self, cx: &Ctx, album_id: i64, partial: bool) -> AppResult<()> {
        let key = Self::staged_key(&cx.server, album_id);
        let now = (self.inner.clock)();
        let d = cx.client.album_detail(album_id).await?;
        let mut s: Staged = self.inner.shared.get(&key).unwrap_or(Staged {
            id: format!("{}:{album_id}", cx.server),
            album_id,
            lidarr_artist_id: d.artist_id,
            artist: cx.artist_name(d.artist_id),
            title: d.title.clone(),
            release_group: d.foreign_album_id.to_ascii_lowercase(),
            partial,
            state: StagedState::Waiting,
            reason: None,
            plan: None,
            folder: None,
            confident: false,
            doubts: Vec::new(),
            auto: false,
            at: now,
            updated_at: now,
        });
        if matches!(s.state, StagedState::Added) {
            return Ok(());
        }
        // A later complete import replaces a partial one.
        s.partial = partial;
        s.state = StagedState::Waiting;
        s.updated_at = now;
        self.inner.shared.put(&key, &s)
    }

    /// Decide every staged album (from the catch-up).
    pub(super) async fn promote_waiting(&self, cx: &Ctx, rep: &mut CatchUpReport) -> AppResult<()> {
        let prefs = self.artist_prefs();
        for mut s in self.staged().into_iter().filter(|s| matches!(s.state, StagedState::Waiting | StagedState::Ready | StagedState::Practice)) {
            if !s.id.starts_with(&format!("{}:", cx.server)) {
                continue;
            }
            let mode = cx.modes.iter().find(|m| m.lidarr_id == Some(s.lidarr_artist_id)).cloned();
            let p = mode.as_ref().and_then(|m| prefs.get(&m.key)).cloned().unwrap_or_default();
            let before = s.clone();
            match self.decide(cx, &s, mode.as_ref()).await {
                Decision::Wait(why) => {
                    s.state = StagedState::Waiting;
                    s.reason = Some(why);
                }
                Decision::Ready { plan, folder, conv } => {
                    s.plan = Some(*plan);
                    s.folder = Some(folder);
                    s.confident = conv.confident;
                    s.doubts = conv.doubts.clone();
                    let auto = p.auto_add == Some(true) && conv.confident;
                    if auto && self.practice_active() {
                        if s.state != StagedState::Practice {
                            self.log_practice(&format!("Would add {} by {}", s.title, s.artist), "In staging", s.folder.as_deref().unwrap_or_default())?;
                        }
                        s.state = StagedState::Practice;
                        s.reason = Some("Practice mode: Platter would have added this on its own.".into());
                    } else if auto {
                        s.auto = true;
                        match self.add_to_library(cx, &mut s, &p).await {
                            Ok(()) => rep.announced.push(s.title.clone()),
                            Err(e) => {
                                s.state = StagedState::Waiting;
                                s.reason = Some(e.to_string());
                            }
                        }
                    } else {
                        s.state = StagedState::Ready;
                        s.reason = Some(if !conv.confident {
                            format!("Waiting for you: Platter isn't sure how this artist's folder is kept ({}).", conv.doubts.join("; "))
                        } else if p.auto_add.is_none() {
                            format!("Waiting for you: the first new album for {} is always shown first.", s.artist)
                        } else {
                            "Waiting for you to add it.".into()
                        });
                    }
                }
            }
            if s != before {
                s.updated_at = (self.inner.clock)();
                self.inner.shared.put(&Self::staged_key(&cx.server, s.album_id), &s)?;
            }
        }
        Ok(())
    }

    async fn decide(&self, cx: &Ctx, s: &Staged, mode: Option<&ArtistMode>) -> Decision {
        if s.partial {
            return Decision::Wait("Waiting for you: Lidarr imported only part of this album. Platter never adds a partial album.".into());
        }
        if let Some(p) = self.paused() {
            return Decision::Wait(p.reason);
        }
        let Some(mode) = mode.filter(|m| m.artist_id.is_some()) else {
            return Decision::Wait(format!("Waiting for you: {} has no folder in your library yet, so Platter won't start one. Add the first album yourself.", s.artist));
        };
        let Some(artist_folder) = mode.folder.clone() else {
            return Decision::Wait(format!("Waiting for you: Platter can't tell where {}'s folder is.", s.artist));
        };
        let d = match cx.client.album_detail(s.album_id).await {
            Ok(d) => d,
            Err(e) => return Decision::Wait(e.to_string()),
        };
        if let Err(why) = official(&d) {
            return Decision::Wait(format!("Waiting for you: {why}."));
        }
        let want = Want { name: mode.name.clone(), mbid: mode.mbid.clone(), artist_id: mode.artist_id.clone() };
        if !self.allowed(&want, &s.release_group) {
            return Decision::Wait("Waiting for you: it isn't on the albums you limited this artist to.".into());
        }
        let library = match self.library(&want).await {
            Ok(l) => l,
            Err(e) => return Decision::Wait(e.to_string()),
        };
        if let Some(c) = clash(&d.title, &library) {
            return Decision::Wait(format!("Waiting for you: its title is the same as {} in your library.", c.title));
        }
        let Some(host) = self.host() else {
            return Decision::Wait("Waiting for you: set where this computer reaches your music (Settings > Music share) so Platter can add it.".into());
        };
        let (files, tracks) = match futures::try_join!(cx.client.track_files(s.album_id), cx.client.tracks(s.album_id)) {
            Ok(x) => x,
            Err(e) => return Decision::Wait(e.to_string()),
        };
        if files.is_empty() {
            return Decision::Wait("Waiting for you: its files aren't in the staging folder any more.".into());
        }
        if tracks.iter().any(|t| t.track_file_id <= 0) {
            return Decision::Wait("Waiting for you: Lidarr is missing some of its tracks.".into());
        }
        let conv = conventions::learn(&self.samples(host.as_ref(), &library));
        let read_tags = |p: &str| {
            host.staging_path(p).and_then(|l| super::tagcarry::read(&l).ok()).map(|b| b.pairs).unwrap_or_default()
        };
        let new = new_files(&files, &tracks, &read_tags);
        if new.len() != files.len() {
            return Decision::Wait("Waiting for you: Platter couldn't match every file to a track.".into());
        }
        let year = d.release_date.as_deref().and_then(|r| r.get(..4)).and_then(|y| y.parse().ok());
        let note = format!("Lidarr, {}, {}", quality_name(&files[0]), super::upgrade::ymd((self.inner.clock)()));
        let plan = conventions::apply(&conv, &d.title, year, &new, &note);
        let folder = join(&artist_folder, &plan.folder);
        if host.music_path(&folder).is_some_and(|l| host.fs().exists(&l)) {
            return Decision::Wait(format!("Waiting for you: {} is already there in the library.", plan.folder));
        }
        Decision::Ready { plan: Box::new(plan), folder, conv: Box::new(conv) }
    }

    /// The artist's official albums as files on this computer.
    fn samples(&self, host: &dyn SwapHost, library: &[super::mbpick::LibraryAlbum]) -> Vec<AlbumSample> {
        let ids: Vec<String> = library.iter().filter(|a| !a.unreleased).map(|a| a.id.clone()).take(SAMPLE_ALBUMS).collect();
        let mut out = Vec::new();
        for id in ids {
            let rows: Vec<(String, i64, i64, String, Option<i32>)> = self
                .inner
                .db
                .read(|c| {
                    let mut st = c.prepare(
                        "SELECT t.file, t.disc_number, t.track_number, al.title, al.year FROM tracks t JOIN albums al ON al.id = t.album_id
                         WHERE t.album_id = ?1 AND t.file IS NOT NULL ORDER BY t.disc_number, t.track_number",
                    )?;
                    let v = st.query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<Vec<_>, _>>()?;
                    Ok(v)
                })
                .unwrap_or_default();
            let Some((first, _, _, title, year)) = rows.first().cloned() else { continue };
            let Some(dir) = parent_raw(&first) else { continue };
            let mut files = Vec::new();
            for (file, disc, track, _, _) in &rows {
                let Some(local) = host.music_path(file) else { continue };
                let Ok(bag) = super::tagcarry::read(&local) else { continue };
                files.push(FileSample {
                    name: local.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                    disc: (*disc).max(1) as u32,
                    track: (*track).max(0) as u32,
                    embedded_cover: bag.pictures.iter().map(|p| p.data().len()).max(),
                    tags: bag.pairs,
                });
            }
            let other_files = host
                .music_path(&dir)
                .and_then(|d| host.fs().list(&d).ok())
                .unwrap_or_default()
                .iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .filter(|n| !files.iter().any(|f| f.name == *n))
                .collect();
            if !files.is_empty() {
                out.push(AlbumSample { folder_name: dir.rsplit(['/', '\\']).next().unwrap_or_default().to_string(), title, year, files, other_files });
            }
        }
        out
    }

    /// Copy into the library, then unmonitor, then (only then) free staging.
    async fn add_to_library(&self, cx: &Ctx, s: &mut Staged, prefs: &ArtistPrefs) -> AppResult<()> {
        let host = self.host().ok_or_else(|| AppError::Invalid("Set where this computer reaches your music first.".into()))?;
        let plan = s.plan.clone().ok_or_else(|| AppError::Invalid("Look at the album again first.".into()))?;
        let folder = s.folder.clone().ok_or_else(|| AppError::Invalid("Look at the album again first.".into()))?;
        // (1) The files, in the library.
        self.copy_in(host.as_ref(), &plan, &folder)?;
        s.state = StagedState::Added;
        s.reason = None;
        self.log_change(
            &format!("Added {} by {} to your library", s.title, s.artist),
            "In the staging folder",
            &format!("{folder} ({} files)", plan.files.len()),
            !s.auto,
            None,
        )?;
        // (2) Unmonitor, unless better copies are still being looked for.
        let files = cx.client.track_files(s.album_id).await.unwrap_or_default();
        let hunting = prefs.hunt && files.iter().any(|f| f.quality_cutoff_not_met);
        if !hunting {
            cx.writer.monitor(vec![s.album_id], false).await?;
            self.log_change(&format!("Stopped Lidarr looking for {}", s.title), "Monitored", "Not monitored", false, Some(Undo::Monitor { album_ids: vec![s.album_id], monitored: true }))?;
        }
        // (3) Free staging, never while monitored.
        if crate::settings::load(&self.inner.db).lidarr_free_staging
            && !cx.client.album_detail(s.album_id).await?.monitored
            && let Some(st) = cx.staging.clone()
        {
            self.free_staging(cx, s, &files, &st).await?;
        }
        if self.inner.plex_server.scan_folder(&folder).await.is_err() {
            tracing::debug!("plex scan after adding {folder} failed; Plex's own scan will find it");
        }
        self.announce(&s.artist, &s.title);
        Ok(())
    }

    pub(super) async fn free_staging(&self, cx: &Ctx, s: &Staged, files: &[TrackFile], staging: &str) -> AppResult<()> {
        if files.is_empty() {
            return Ok(());
        }
        let ok = StagingFiles::check(files, staging)?;
        cx.writer.send_call(WriteCall::FreeStaging(ok)).await?;
        let bin = cx.snap.media.recycle_bin.trim().to_string();
        self.log_change(
            &format!("Freed {}'s copy in the staging folder", s.title),
            &format!("{} files in staging", files.len()),
            &if bin.is_empty() { "Deleted".to_string() } else { format!("In Lidarr's recycling bin ({bin}); the space comes back after Lidarr's recycling bin cleanup") },
            false,
            None,
        )?;
        Ok(())
    }

    /// Copy every file under its new name, set its tags, and only then give
    /// it its name. Any failure removes what was copied.
    fn copy_in(&self, host: &dyn SwapHost, plan: &PlannedAlbum, folder: &str) -> AppResult<()> {
        let dir = host.music_path(folder).ok_or_else(|| AppError::Invalid(format!("This computer can't reach {folder}. Check Settings > Music share.")))?;
        if host.fs().exists(&dir) {
            return Err(AppError::Invalid(format!("{folder} is already there. Nothing was copied.")));
        }
        host.fs().create_dir_all(&dir).map_err(|e| AppError::Internal(format!("Couldn't make {folder}: {e}")))?;
        let mut done: Vec<PathBuf> = Vec::new();
        let result = (|| -> AppResult<()> {
            for f in &plan.files {
                let src = host.staging_path(&f.src).ok_or_else(|| AppError::Invalid(format!("This computer can't reach {} in the staging folder.", f.src)))?;
                let tmp = dir.join(format!(".platter-{}", f.name));
                host.fs().copy(&src, &tmp).map_err(|e| AppError::Internal(format!("Couldn't copy {}: {e}", f.src)))?;
                done.push(tmp.clone());
                super::tagcarry::apply_edits(&tmp, &f.edits, None)?;
                let fin = dir.join(&f.name);
                host.fs().rename(&tmp, &fin).map_err(|e| AppError::Internal(format!("Couldn't name {}: {e}", f.name)))?;
                done.pop();
                done.push(fin);
            }
            Ok(())
        })();
        if let Err(e) = result {
            for p in &done {
                let _ = host.fs().remove(p);
            }
            let _ = host.fs().remove_empty_dir(&dir);
            return Err(e);
        }
        Ok(())
    }

    /// The user said yes to adding a staged album. With `always`, later
    /// albums by the artist are added on their own.
    pub async fn promotion_approve(&self, id: &str, always: bool) -> AppResult<Staged> {
        let _one = self.lock.lock().await;
        let cx = self.ctx().await?;
        let key = format!("staged:{id}");
        let mut s: Staged = self.inner.shared.get(&key).ok_or_else(|| AppError::NotFound("That album isn't waiting any more.".into()))?;
        let mode = cx.modes.iter().find(|m| m.lidarr_id == Some(s.lidarr_artist_id)).cloned();
        match self.decide(&cx, &s, mode.as_ref()).await {
            Decision::Wait(why) => return Err(AppError::Invalid(why)),
            Decision::Ready { plan, folder, conv } => {
                s.plan = Some(*plan);
                s.folder = Some(folder);
                s.confident = conv.confident;
            }
        }
        let mut prefs = mode.as_ref().map(|m| self.prefs_for(&m.key)).unwrap_or_default();
        s.auto = false;
        self.add_to_library(&cx, &mut s, &prefs).await?;
        if let Some(m) = &mode {
            prefs.auto_add = Some(always || prefs.auto_add == Some(true));
            prefs.name = m.name.clone();
            self.save_prefs(&m.key, &prefs)?;
        }
        s.updated_at = (self.inner.clock)();
        self.inner.shared.put(&key, &s)?;
        Ok(s)
    }

    /// Take an added album out of the library: its folder moves into the
    /// ignored backup folder. `trash_ok`: the user agreed to Plex's
    /// automatic trash emptying being off for the move.
    pub async fn promotion_undo(&self, id: &str, trash_ok: bool) -> AppResult<Staged> {
        let _one = self.lock.lock().await;
        let key = format!("staged:{id}");
        let mut s: Staged = self.inner.shared.get(&key).ok_or_else(|| AppError::NotFound("That album isn't in the list.".into()))?;
        if s.state != StagedState::Added {
            return Err(AppError::Invalid("Only an added album can be taken out again.".into()));
        }
        let folder = s.folder.clone().ok_or_else(|| AppError::Invalid("Platter doesn't know where it put this album.".into()))?;
        let host = self.host().ok_or_else(|| AppError::Invalid("Set where this computer reaches your music first.".into()))?;
        let dir = host.music_path(&folder).ok_or_else(|| AppError::Invalid(format!("This computer can't reach {folder}.")))?;
        let place = host.backup_place(&dir).map_err(|f| AppError::Invalid(f.message()))?;
        let empties = self.inner.plex_server.prefs().await.ok().and_then(|p| p.empty_trash).unwrap_or(true);
        if empties && !trash_ok {
            return Err(AppError::Invalid(TRASH_ASK.into()));
        }
        if let Some(root) = &place.ignore_in {
            swap::ensure_ignored(host.fs(), root).map_err(|e| AppError::Internal(format!("Couldn't keep Plex out of the backup folder: {e}")))?;
        }
        if empties {
            self.inner.plex_server.set_empty_trash(false).await?;
            self.log_change("Paused Plex's automatic trash emptying", "On", "Off, while an album moves", true, None)?;
        }
        let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "album".into());
        let to = place.root.join(format!("{name} (added {})", super::upgrade::ymd(s.updated_at)));
        let moved = host.fs().create_dir_all(&place.root).and_then(|_| host.fs().rename_dir(&dir, &to));
        if moved.is_ok() {
            let parent = parent_raw(&folder).unwrap_or(folder.clone());
            let _ = self.inner.plex_server.scan_folder(&parent).await;
        }
        if empties {
            self.inner.plex_server.set_empty_trash(true).await?;
            self.log_change("Turned Plex's automatic trash emptying back on", "Off", "On", true, None)?;
        }
        moved.map_err(|e| AppError::Internal(format!("Couldn't move {folder}: {e}")))?;
        self.log_change(&format!("Took {} out of your library", s.title), &folder, &to.to_string_lossy(), true, None)?;
        s.state = StagedState::Undone;
        s.updated_at = (self.inner.clock)();
        self.inner.shared.put(&key, &s)?;
        Ok(s)
    }
}

/// What Undo says when Plex empties its trash after every scan.
pub const TRASH_ASK: &str = "Plex empties its trash after every scan, which would drop this album's play counts and ratings from Plex during the move. Platter can turn that off just for the move and back on right after.";

fn join(base: &str, name: &str) -> String {
    let sep = if base.contains('\\') && !base.contains('/') { '\\' } else { '/' };
    format!("{}{sep}{name}", base.trim_end_matches(['/', '\\']))
}
