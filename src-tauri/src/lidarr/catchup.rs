//! Catching up with Lidarr. Safety never depends on Platter running (the
//! setup check keeps Lidarr's own configuration safe for protected
//! artists), but finishing the work does: on start, on wake and every few
//! minutes, Platter reads everything Lidarr did since the last event it
//! handled and finishes it.
//!
//! - Managed albums Lidarr imported (`downloadImported`; there is no
//!   "albumImported") that Plex doesn't show within about two minutes get a
//!   scan of just their folder, then an announcement.
//! - Protected albums landing in staging are queued for adding to the
//!   library (`promote`); `albumImportIncomplete` is a partial import that
//!   is never added and waits for the user.
//! - Self-healing: protected albums already in the library that Lidarr still
//!   monitors (hunting off or cutoff met), and staging albums whose files are
//!   gone while monitored, are unmonitored before Lidarr's next search.
//! - Rename Tracks, Write Audio Tags and the recycling bin are read every
//!   time; a change toward unsafe while protected artists exist pauses
//!   protected work and says why.
//!
//! The last event handled is kept per Lidarr in the shared store, so any of
//! the user's devices can catch up, and each event is claimed there first
//! (`shared`) so two devices never act on the same one. When Lidarr's
//! history no longer reaches back to the last handled event, Platter says
//! some activity may be missed and checks Plex against Lidarr instead.
//!
//! Nothing here touches the UI; announcements go through [`Announcer`].

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::Lidarr;
use super::api::{HistoryItem, LidarrClient};
use super::check::LidarrSnapshot;
use super::modes::{ArtistMode, parent_raw};
use super::paths::{NormPath, inside_any};
use super::shared::ClaimState;
use super::write::LidarrWriter;
use crate::error::AppResult;
use crate::jellyfin::types::{iso, unix_s};

/// How often a catch-up runs while Platter is open.
pub const EVERY: Duration = Duration::from_secs(4 * 60);
/// How long Plex gets to find a Managed import on its own.
pub const PLEX_GRACE_S: i64 = 120;
/// Most folders one gap cross-check scans.
const GAP_SCANS: usize = 25;
/// Claims older than this are forgotten.
const CLAIM_DAYS: u64 = 30;

pub const GAP_NOTE: &str = "Some Lidarr activity while Platter was closed may be missed: Lidarr no longer keeps history that far back. Platter checked Plex against Lidarr instead.";

/// Tells the user about new music (What's New and notifications).
pub trait Announcer: Send + Sync {
    fn album_added(&self, artist: &str, album: &str);
}

/// The last event handled, per Lidarr (shared).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub last_id: i64,
    /// ISO 8601, as Lidarr gave it.
    pub last_date: String,
    /// The device that did the work, and when (unix seconds).
    pub device: String,
    pub at: i64,
}

/// A Managed import Platter is making sure Plex shows (shared).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingScan {
    pub album_id: i64,
    pub artist: String,
    pub album: String,
    pub folder: String,
    /// When Plex has had long enough (unix seconds).
    pub due: i64,
    #[serde(default)]
    pub scanned: bool,
}

/// Lidarr's settings as last seen safe (shared).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    pub rename_tracks: bool,
    pub write_audio_tags: String,
    pub recycle_bin: String,
}

/// Protected work is paused (shared).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pause {
    pub reason: String,
    pub at: i64,
}

/// What one catch-up did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatchUpReport {
    /// Events read and settled (handled here or by another device).
    pub handled: usize,
    /// Events another device has.
    pub elsewhere: usize,
    /// Claimed here, acted on at a later catch-up.
    pub settling: usize,
    /// Events this device acted on.
    pub acted: usize,
    pub gap: Option<String>,
    /// Folders Plex was asked to scan.
    pub scanned: Vec<String>,
    /// Albums announced.
    pub announced: Vec<String>,
    /// Protected albums that landed in staging.
    pub staged: Vec<i64>,
    /// Self-healing: albums unmonitored, by title.
    pub healed: Vec<String>,
    /// Protected work is paused, and why.
    pub paused: Option<String>,
}

/// Where catching up stands, for Settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatchUpStatus {
    /// The device that last did the Lidarr work, and when (unix seconds).
    pub last_device: Option<String>,
    pub last_at: Option<i64>,
    pub gap: Option<String>,
    pub paused: Option<String>,
}

/// What an event means for Platter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// A whole album imported for a Managed artist.
    ManagedImported,
    /// A whole album imported into staging for a protected artist.
    StagedComplete,
    /// Part of an album imported: never added to the library.
    StagedPartial,
    /// Grabbed, failed: request statuses follow.
    Status,
    Ignore,
}

pub fn classify(ev: &HistoryItem, protected: bool) -> EventKind {
    match (ev.event_type.trim(), protected) {
        ("downloadImported", false) => EventKind::ManagedImported,
        ("downloadImported", true) => EventKind::StagedComplete,
        ("albumImportIncomplete", true) => EventKind::StagedPartial,
        ("grabbed" | "downloadFailed" | "albumImportIncomplete", _) => EventKind::Status,
        _ => EventKind::Ignore,
    }
}

/// Changes toward unsafe while protected artists exist.
pub fn unsafe_changes(was: &Baseline, now: &Baseline) -> Vec<&'static str> {
    let mut out = Vec::new();
    if now.rename_tracks && !was.rename_tracks {
        out.push("Rename Tracks was turned on");
    }
    let tags = |t: &str| !t.trim().is_empty() && !t.trim().eq_ignore_ascii_case("no");
    if tags(&now.write_audio_tags) && !tags(&was.write_audio_tags) {
        out.push("Write Audio Tags was turned on");
    }
    if now.recycle_bin.trim().is_empty() && !was.recycle_bin.trim().is_empty() {
        out.push("the recycling bin was cleared");
    }
    out
}

/// One pass's view of Lidarr.
pub(super) struct Ctx {
    pub client: LidarrClient,
    pub writer: LidarrWriter,
    pub snap: LidarrSnapshot,
    pub modes: Vec<ArtistMode>,
    /// The Lidarr this is, as a store key.
    pub server: String,
    pub staging: Option<String>,
}

impl Ctx {
    /// Whether a Lidarr artist is protected (by its mode, or by living in staging).
    pub fn protected_artist(&self, lidarr_artist_id: i64) -> bool {
        self.modes.iter().any(|m| m.protected() && m.lidarr_id == Some(lidarr_artist_id))
            || self.snap.artists.iter().any(|a| {
                a.id == lidarr_artist_id && self.staging.as_deref().is_some_and(|s| inside_any(&a.path, [s]).is_some())
            })
    }

    pub fn artist_name(&self, lidarr_artist_id: i64) -> String {
        self.snap.artists.iter().find(|a| a.id == lidarr_artist_id).map(|a| a.artist_name.clone()).unwrap_or_default()
    }
}

impl Lidarr {
    /// The announcer for new music (set by the app).
    pub fn set_announcer(&self, a: Arc<dyn Announcer>) {
        *self.announcer.write() = Some(a);
    }

    pub(super) fn server_key(&self) -> String {
        let (home, away) = self.urls();
        super::url_key(if home.trim().is_empty() { &away } else { &home })
    }

    /// Where catching up stands.
    pub fn catch_up_status(&self) -> CatchUpStatus {
        let server = self.server_key();
        let cursor: Option<Cursor> = self.inner.shared.get(&format!("cursor:{server}"));
        let gap: Option<String> = self.inner.shared.get(&format!("gap:{server}"));
        CatchUpStatus {
            last_device: cursor.as_ref().map(|c| c.device.clone()),
            last_at: cursor.map(|c| c.at),
            gap,
            paused: self.paused().map(|p| p.reason),
        }
    }

    /// Protected work is paused, and why.
    pub fn paused(&self) -> Option<Pause> {
        self.inner.shared.get(&format!("pause:{}", self.server_key()))
    }

    /// The user looked at Lidarr's settings and wants protected work to go on.
    pub fn resume_protected(&self) -> AppResult<()> {
        let server = self.server_key();
        self.inner.shared.remove(&format!("pause:{server}"))?;
        self.inner.shared.remove(&format!("baseline:{server}"))
    }

    pub(super) async fn ctx(&self) -> AppResult<Ctx> {
        let (client, status, route) = Self::connect(self.routes()?).await?;
        let snap = Self::snapshot(&client, status).await?;
        let modes = self.modes_with(&client, &snap).await?;
        let staging = Some(crate::settings::load(&self.inner.db).lidarr_staging_folder).filter(|s| !s.trim().is_empty());
        Ok(Ctx { client, writer: self.writer_for(route), snap, modes, server: self.server_key(), staging })
    }

    /// One catch-up. Does nothing until Lidarr is set up.
    pub async fn catch_up(&self) -> AppResult<CatchUpReport> {
        let st = self.status();
        if !(st.url_set || st.away_set) || !st.has_key {
            return Ok(CatchUpReport::default());
        }
        let _one = self.lock.lock().await;
        let cx = self.ctx().await?;
        let mut rep = CatchUpReport::default();
        let now = (self.inner.clock)();
        let shared = &self.inner.shared;

        // ---- Lidarr's settings, every time.
        rep.paused = self.recheck_settings(&cx, now)?;

        // ---- History since the last handled event.
        let ckey = format!("cursor:{}", cx.server);
        let mut cursor: Cursor = match shared.get(&ckey) {
            Some(c) => c,
            None => {
                // First run: start from now, without acting on old history.
                let c = Cursor { last_id: 0, last_date: iso(now), device: shared.label(), at: now };
                shared.put(&ckey, &c)?;
                c
            }
        };
        let events = cx.client.history_since(&cursor.last_date).await?;
        // History ids only go up: when the oldest record Lidarr keeps is
        // newer than the last one handled and isn't the very next one,
        // records in between are gone.
        let gap = match cx.client.oldest_history().await {
            Ok(Some(o)) if cursor.last_id > 0 => {
                o.id > cursor.last_id + 1 && unix_s(&o.date).zip(unix_s(&cursor.last_date)).is_some_and(|(oldest, last)| oldest > last)
            }
            _ => false,
        };
        if gap {
            rep.gap = Some(GAP_NOTE.into());
            rep.scanned.extend(self.gap_cross_check(&cx).await);
            shared.put(&format!("gap:{}", cx.server), &GAP_NOTE.to_string())?;
        }
        let (from_id, from_date) = (cursor.last_id, unix_s(&cursor.last_date));
        for ev in events.iter().filter(|e| e.id > from_id || unix_s(&e.date) > from_date) {
            match shared.claim(&format!("{}:{}", cx.server, ev.id))? {
                ClaimState::Settling => {
                    rep.settling += 1;
                    break;
                }
                ClaimState::Theirs(_) => rep.elsewhere += 1,
                ClaimState::Ours => {
                    self.act(&cx, ev, now, &mut rep).await?;
                    rep.acted += 1;
                }
            }
            rep.handled += 1;
            cursor.last_id = cursor.last_id.max(ev.id);
            cursor.last_date = ev.date.clone();
        }
        // The device shown is the one that last did the work.
        if rep.acted > 0 {
            cursor.device = shared.label();
            cursor.at = now;
        }
        if shared.get::<Cursor>(&ckey).as_ref() != Some(&cursor) {
            shared.put(&ckey, &cursor)?;
        }
        if !gap && rep.handled > 0 {
            let _ = shared.remove(&format!("gap:{}", cx.server));
        }
        shared.prune_claims(shared.now().saturating_sub(CLAIM_DAYS * 86_400_000));

        // ---- Managed imports Plex hasn't shown yet.
        self.finish_scans(&cx, now, &mut rep).await;
        // ---- Protected albums waiting in staging.
        if rep.paused.is_none() {
            self.promote_waiting(&cx, &mut rep).await?;
        }
        // ---- Self-healing.
        self.self_heal(&cx, &mut rep).await?;
        tracing::info!(handled = rep.handled, elsewhere = rep.elsewhere, gap, "lidarr catch-up");
        Ok(rep)
    }

    async fn act(&self, cx: &Ctx, ev: &HistoryItem, now: i64, rep: &mut CatchUpReport) -> AppResult<()> {
        let Some(album_id) = ev.album_id.filter(|a| *a > 0) else { return Ok(()) };
        let artist_id = ev.artist_id.unwrap_or(0);
        match classify(ev, cx.protected_artist(artist_id)) {
            EventKind::ManagedImported => {
                let files = cx.client.track_files(album_id).await.unwrap_or_default();
                let Some(folder) = files.first().and_then(|f| parent_raw(&f.path)) else { return Ok(()) };
                let album = cx.client.album_detail(album_id).await.map(|a| a.title).unwrap_or_default();
                let p = PendingScan { album_id, artist: cx.artist_name(artist_id), album, folder, due: unix_s(&ev.date).unwrap_or(now) + PLEX_GRACE_S, scanned: false };
                self.inner.shared.put(&format!("scan:{}:{album_id}", cx.server), &p)?;
            }
            EventKind::StagedComplete => {
                self.on_staged(cx, album_id, false).await?;
                rep.staged.push(album_id);
            }
            EventKind::StagedPartial => {
                self.on_staged(cx, album_id, true).await?;
                rep.staged.push(album_id);
            }
            EventKind::Status | EventKind::Ignore => {}
        }
        Ok(())
    }

    /// Scans for Managed imports Plex didn't find on its own, once their two
    /// minutes are up; announces each album once Plex has it.
    async fn finish_scans(&self, cx: &Ctx, now: i64, rep: &mut CatchUpReport) {
        let scan_on = crate::settings::load(&self.inner.db).lidarr_scan_after_import;
        let prefix = format!("scan:{}:", cx.server);
        for (k, mut p) in self.inner.shared.list::<PendingScan>(&prefix) {
            if p.due > now {
                continue;
            }
            let key = format!("{prefix}{k}");
            match self.inner.plex_server.sees_folder(&p.folder).await {
                Ok(true) => {
                    self.announce(&p.artist, &p.album);
                    rep.announced.push(p.album.clone());
                    let _ = self.inner.shared.remove(&key);
                }
                Ok(false) if scan_on && !p.scanned => {
                    if self.inner.plex_server.scan_folder(&p.folder).await.is_ok() {
                        rep.scanned.push(p.folder.clone());
                        p.scanned = true;
                        p.due = now + PLEX_GRACE_S / 2;
                        let _ = self.inner.shared.put(&key, &p);
                    }
                }
                Ok(false) if !scan_on => {
                    // Plex's own scan will find it; nothing to follow.
                    let _ = self.inner.shared.remove(&key);
                }
                // Scanned already and still missing, or Plex unreachable: try again later,
                // up to a day.
                _ if now - p.due > 86_400 => {
                    let _ = self.inner.shared.remove(&key);
                }
                _ => {}
            }
        }
    }

    pub(super) fn announce(&self, artist: &str, album: &str) {
        if let Some(a) = self.announcer.read().clone() {
            a.album_added(artist, album);
        }
    }

    /// When history doesn't reach back far enough: Managed artists' folders
    /// with no files in Plex get a scan.
    async fn gap_cross_check(&self, cx: &Ctx) -> Vec<String> {
        let Ok(plex) = self.inner.plex.music_files().await else { return Vec::new() };
        let have: BTreeSet<NormPath> = plex.files.iter().filter_map(|f| parent_raw(&f.path)).filter_map(|d| parent_raw(&d)).map(|d| NormPath::parse(&d)).collect();
        let mut out = Vec::new();
        for a in cx.snap.artists.iter().filter(|a| !cx.protected_artist(a.id) && !a.path.trim().is_empty()) {
            if out.len() >= GAP_SCANS {
                break;
            }
            let p = NormPath::parse(&a.path);
            if inside_any(&a.path, plex.locations.iter().map(String::as_str)).is_none() || have.contains(&p) {
                continue;
            }
            let files = cx.client.albums_of(a.id).await.unwrap_or_default();
            if !files.iter().any(|al| al.statistics.as_ref().is_some_and(|s| s.track_file_count > 0)) {
                continue;
            }
            if self.inner.plex_server.scan_folder(&a.path).await.is_ok() {
                out.push(a.path.clone());
            }
        }
        out
    }

    /// Rename Tracks, Write Audio Tags and the recycling bin, read every
    /// time. A change toward unsafe while artists are protected pauses
    /// protected work until the user looks (or it changes back).
    fn recheck_settings(&self, cx: &Ctx, now: i64) -> AppResult<Option<String>> {
        let shared = &self.inner.shared;
        let bkey = format!("baseline:{}", cx.server);
        let pkey = format!("pause:{}", cx.server);
        let current = Baseline {
            rename_tracks: cx.snap.naming.rename_tracks,
            write_audio_tags: cx.snap.metadata.write_audio_tags.clone(),
            recycle_bin: cx.snap.media.recycle_bin.clone(),
        };
        let protected = cx.modes.iter().any(ArtistMode::protected);
        let Some(was) = shared.get::<Baseline>(&bkey) else {
            shared.put(&bkey, &current)?;
            return Ok(self.paused().map(|p| p.reason));
        };
        let worse = unsafe_changes(&was, &current);
        if protected && !worse.is_empty() {
            let reason = format!(
                "Protected work is paused: in Lidarr, {}. Change it back, or look at Settings > Lidarr and resume.",
                worse.join(" and ")
            );
            if self.paused().is_none_or(|p| p.reason != reason) {
                shared.put(&pkey, &Pause { reason: reason.clone(), at: now })?;
                self.log_change("Paused protected work", "", &reason, false, None)?;
            }
            return Ok(Some(reason));
        }
        if worse.is_empty() {
            let _ = shared.remove(&pkey);
            shared.put(&bkey, &current)?;
        }
        Ok(self.paused().map(|p| p.reason))
    }

    /// Protected albums Lidarr still monitors that it shouldn't: already in
    /// the library with hunting off or the cutoff met, or in staging with
    /// their files gone after an import.
    async fn self_heal(&self, cx: &Ctx, rep: &mut CatchUpReport) -> AppResult<()> {
        let prefs = self.artist_prefs();
        let mut off: Vec<(i64, String)> = Vec::new();
        for a in cx.snap.artists.iter().filter(|a| cx.protected_artist(a.id)) {
            let hunting = cx.modes.iter().find(|m| m.lidarr_id == Some(a.id)).and_then(|m| prefs.get(&m.key)).is_some_and(|p| p.hunt);
            for al in cx.client.albums_of(a.id).await.unwrap_or_default().into_iter().filter(|al| al.monitored) {
                let files = al.statistics.as_ref().map_or(0, |s| s.track_file_count);
                let in_library = self.library_has_group(&al.foreign_album_id) || self.promoted_group(&al.foreign_album_id);
                let heal = if in_library {
                    if !hunting {
                        true
                    } else {
                        let tf = cx.client.track_files(al.id).await.unwrap_or_default();
                        !tf.is_empty() && tf.iter().all(|f| !f.quality_cutoff_not_met)
                    }
                } else if files == 0 {
                    let h = cx.client.history(al.id).await.unwrap_or_default();
                    h.iter().any(|e| matches!(e.event_type.as_str(), "downloadImported" | "trackFileImported"))
                } else {
                    false
                };
                if heal {
                    off.push((al.id, al.title));
                }
            }
        }
        if off.is_empty() {
            return Ok(());
        }
        cx.writer.monitor(off.iter().map(|(id, _)| *id).collect(), false).await?;
        let titles: Vec<String> = off.iter().map(|(_, t)| t.clone()).collect();
        self.log_change(
            "Stopped Lidarr looking for protected albums it no longer needs",
            "Monitored",
            &format!("Not monitored: {}", titles.join(", ")),
            false,
            Some(super::changes::Undo::Monitor { album_ids: off.iter().map(|(id, _)| *id).collect(), monitored: true }),
        )?;
        rep.healed = titles;
        Ok(())
    }
}

/// The catch-up loop: on start, every few minutes, and at once after the
/// computer wakes (a pass that comes much later than planned).
pub async fn watch(l: Lidarr) {
    tokio::time::sleep(Duration::from_secs(20)).await;
    let wall = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    loop {
        if let Err(e) = l.catch_up().await {
            tracing::debug!("lidarr catch-up: {e}");
        }
        let step = Duration::from_secs(15);
        let mut waited = Duration::ZERO;
        let mut before = wall();
        while waited < EVERY {
            tokio::time::sleep(step).await;
            waited += step;
            // Woke from sleep: the wall clock jumped much further than the step.
            let now = wall();
            if now.saturating_sub(before) > step.as_secs() * 4 {
                break;
            }
            before = now;
        }
    }
}
