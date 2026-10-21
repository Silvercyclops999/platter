//! Better copies by themselves: every released album in the library below
//! the aim gets a better copy looked for, a few a day, and swapped in under
//! the usual swap rules. Lidarr only ever downloads into the staging folder;
//! the library's files are changed by Platter's swap (`swap`), file by
//! file, with the old file kept in the backup folder and Undo.
//!
//! Extra care on top of the swap rules:
//!
//! - An album holding even one unreleased file (its tags, the archive's
//!   marks, a demo, leak or build) is never asked for, so a released album
//!   with unreleased songs mixed in is left exactly as it is.
//! - Only albums with a confirmed official release. Platter confirms one by
//!   itself only when exactly one official release has the same title and
//!   the very same tracklist (every song in order, same names, lengths
//!   within 3 seconds); otherwise the album waits for a person.
//! - The first week after better copies turns on, nothing is swapped: what
//!   would be is written to the Changes log.
//! - At most [`DAY_ASKS`] albums asked for and [`DAY_SWAPS`] swapped a day,
//!   counted for all the user's devices. A swap on its own that fails, or
//!   one the user undoes, stops swaps on their own for a day, and an undone
//!   album is never upgraded again.
//! - Only one of the user's devices ever asks for an album (it claims the
//!   album first, as for Lidarr events), and only a device that can reach
//!   the music files.

use std::collections::HashSet;

use super::hunt::{HuntItem, HuntState};
use super::mbpick::Candidate;
use super::shared::ClaimState;
use super::upgrade::{FolderSong, Upgrades};
use crate::error::AppResult;
use crate::identity::fold::fold_title;

/// The first week only lists what would be swapped (s).
pub const PRACTICE_S: i64 = 7 * 86_400;
/// Most albums asked for a day.
pub const DAY_ASKS: usize = super::upgrade::BULK_MAX;
/// Most albums looked up on MusicBrainz for a release a day.
pub const DAY_LOOKUPS: usize = 40;
/// Most albums swapped on their own a day.
pub const DAY_SWAPS: usize = 10;
/// An album no release could be confirmed for is looked at again after this (s).
pub const NO_MATCH_RETRY_S: i64 = 30 * 86_400;
/// Lengths further apart than this aren't the same track (ms).
const LENGTH_SLACK_MS: u64 = 3_000;

/// When better copies first ran (shared, unix seconds).
const SINCE_KEY: &str = "betterCopies:since";
/// Swaps on their own wait until this (shared, unix seconds).
const PAUSED_KEY: &str = "betterCopies:pausedUntil";
/// Albums no release could be confirmed for, and when (shared).
const NO_MATCH: &str = "betterCopies:noMatch:";
/// When the last daily pass ran, on any of the user's devices (shared).
const RAN_KEY: &str = "betterCopies:ranAt";
/// What today's passes and swaps used (shared).
const DAY_KEY: &str = "betterCopies:day";
/// Albums whose swap was undone: kept as they are (shared, never pruned).
const UNDONE: &str = "betterCopies:undone:";
/// The device that asks for an album, once it has (shared).
const BY: &str = "betterCopies:by:";
/// A pass runs again after this (s).
const PASS_EVERY_S: i64 = 20 * 3_600;

const DAY: i64 = 86_400;

/// What one day's passes and swaps used, for all the user's devices.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct DayUse {
    day: i64,
    asked: usize,
    looked: usize,
    swapped: usize,
}

/// Where better copies stand, for Settings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetterStatus {
    /// The practice week ends then (unix seconds); None before it starts.
    pub practice_until: Option<i64>,
    /// Swaps wait a day (a failure or an undo).
    pub paused: bool,
}

/// The one official release a folder certainly is: the same title (version
/// notes included) and the same tracklist, song for song. None when no
/// release, or more than one, fits.
pub fn auto_pick(title: &str, songs: &[FolderSong], candidates: &[Candidate]) -> Option<String> {
    let want = fold_title(title);
    let fits: Vec<&Candidate> = candidates.iter().filter(|c| fold_title(&c.group.title) == want && same_tracklist(songs, c)).collect();
    match fits.as_slice() {
        [one] => Some(one.group.id.clone()),
        _ => None,
    }
}

fn same_tracklist(songs: &[FolderSong], c: &Candidate) -> bool {
    if songs.is_empty() || songs.len() != c.tracks.len() {
        return false;
    }
    let mut ours: Vec<&FolderSong> = songs.iter().collect();
    ours.sort_by_key(|s| (s.disc.max(1), s.track));
    let mut theirs: Vec<&super::mbpick::ListedTrack> = c.tracks.iter().collect();
    theirs.sort_by_key(|t| (t.disc.max(1), t.position));
    ours.iter().zip(theirs).all(|(s, t)| {
        s.disc.max(1) == t.disc.max(1)
            && s.track == t.position
            && fold_title(&s.title) == fold_title(&t.title)
            && t.length_ms.is_some_and(|l| l.abs_diff(s.duration_ms) <= LENGTH_SLACK_MS)
    })
}

impl Upgrades {
    fn shared_i64(&self, key: &str) -> Option<i64> {
        self.lidarr().shared().get::<i64>(key)
    }

    /// The practice week starts with the first pass that can run.
    fn start_better(&self) {
        if self.shared_i64(SINCE_KEY).is_none() {
            let _ = self.lidarr().shared().put(SINCE_KEY, &self.now());
        }
    }

    /// Better copies were turned off: a practice week comes first again
    /// when they're turned back on.
    pub(super) fn reset_better(&self) {
        if self.shared_i64(SINCE_KEY).is_some()
            && let Err(e) = self.lidarr().shared().remove(SINCE_KEY)
        {
            tracing::warn!("better copies: practice week not reset: {e}");
        }
    }

    /// Better copies' first week (or before the first pass): swaps are only listed.
    pub fn better_practice(&self) -> bool {
        self.shared_i64(SINCE_KEY).is_none_or(|t| self.now() - t < PRACTICE_S)
    }

    /// When the practice week ends (unix seconds), once it has started.
    pub fn better_practice_until(&self) -> Option<i64> {
        self.shared_i64(SINCE_KEY).map(|t| t + PRACTICE_S)
    }

    pub fn better_status(&self) -> BetterStatus {
        BetterStatus { practice_until: self.better_practice_until().filter(|&t| t > self.now()), paused: self.better_paused() }
    }

    /// Swaps on their own are paused (a failure or an undo in the last day).
    pub fn better_paused(&self) -> bool {
        self.shared_i64(PAUSED_KEY).is_some_and(|t| self.now() < t)
    }

    /// Stop swaps on their own for a day.
    pub(super) fn pause_better(&self) {
        if let Err(e) = self.lidarr().shared().put(PAUSED_KEY, &(self.now() + DAY)) {
            tracing::warn!("better copies: pause not kept: {e}");
        }
    }

    /// An album's swap was undone: it stays as it is from now on, on every device.
    pub(super) fn keep_as_is(&self, album_id: &str) {
        if let Err(e) = self.lidarr().shared().put(&format!("{UNDONE}{album_id}"), &self.now()) {
            tracing::warn!("better copies: undone album not kept: {e}");
        }
    }

    /// The album's swap was undone once (on any device).
    pub(super) fn kept_as_is(&self, album_id: &str) -> bool {
        self.lidarr().shared().get::<i64>(&format!("{UNDONE}{album_id}")).is_some()
    }

    fn day_use(&self) -> DayUse {
        let today = self.now().div_euclid(DAY);
        self.lidarr().shared().get::<DayUse>(DAY_KEY).filter(|d| d.day == today).unwrap_or(DayUse { day: today, ..Default::default() })
    }

    /// Keep today's use, never lowering what another device counted meanwhile.
    fn put_day(&self, d: &DayUse) -> AppResult<()> {
        let now = self.day_use();
        let d = if now.day == d.day {
            DayUse { day: d.day, asked: d.asked.max(now.asked), looked: d.looked.max(now.looked), swapped: d.swapped.max(now.swapped) }
        } else {
            *d
        };
        self.lidarr().shared().put(DAY_KEY, &d)
    }

    /// Albums swapped on their own today, on all the user's devices.
    pub(super) fn swapped_today(&self) -> usize {
        self.day_use().swapped
    }

    /// Count a swap made on its own.
    pub(super) fn count_swap(&self) {
        let mut d = self.day_use();
        d.swapped += 1;
        if let Err(e) = self.put_day(&d) {
            tracing::warn!("better copies: swap not counted: {e}");
        }
    }

    /// One daily pass over the whole library (for all the user's devices).
    /// Does nothing while better copies are off, without a staging folder
    /// or a ready Lidarr (the owner's), or on a device that can't reach the
    /// music files.
    pub async fn better_copies(&self) -> AppResult<Vec<HuntItem>> {
        let s = crate::settings::load(self.inner_db());
        if !s.better_copies || s.lidarr_staging_folder.trim().is_empty() || !self.lidarr().status().ready || !self.can_swap_here() {
            return Ok(Vec::new());
        }
        let now = self.now();
        if self.shared_i64(RAN_KEY).is_some_and(|t| now - t < PASS_EVERY_S) {
            return Ok(Vec::new());
        }
        let mut day = self.day_use();
        if day.asked >= DAY_ASKS {
            return Ok(Vec::new());
        }
        let shared = self.lidarr().shared().clone();
        shared.put(RAN_KEY, &now)?;
        self.start_better();
        let me = shared.device();
        // Albums of artists Lidarr manages get better copies from Lidarr's
        // own profile, and excluded artists are left alone.
        let cx = self.lidarr().ctx().await?;
        let skip: HashSet<String> =
            cx.modes.iter().filter(|m| m.excluded || (!m.protected() && m.lidarr_id.is_some())).filter_map(|m| m.artist_id.clone()).collect();
        let ids: Vec<String> = self
            .inner_db()
            .read_async(|c| {
                let mut st = c.prepare("SELECT id FROM albums ORDER BY artist_title, title")?;
                Ok(st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?)
            })
            .await?;
        let target = super::hunt::target_rank(s.better_copies_aim);
        // Released albums below the aim, lowest first. An album with even
        // one unreleased song is never asked for, nor one undone before, nor
        // one another device asks for.
        let mut below = Vec::new();
        for id in ids {
            if self.kept_as_is(&id) || shared.get::<String>(&format!("{BY}{id}")).is_some_and(|d| d != me) {
                continue;
            }
            let Ok(a) = self.album(&id).await else { continue };
            if !a.all_released() || skip.contains(&a.artist_id) {
                continue;
            }
            if let Some((rank, _)) = a.current()
                && rank < target
            {
                below.push((rank, id));
            }
        }
        below.sort();
        let mut confirmed = self.confirmations();
        let mut settling = false;
        let mut out = Vec::new();
        for (_, id) in below {
            if day.asked >= DAY_ASKS {
                break;
            }
            if !confirmed.contains_key(&id) {
                let key = format!("{NO_MATCH}{id}");
                if day.looked >= DAY_LOOKUPS || shared.get::<i64>(&key).is_some_and(|t| now - t < NO_MATCH_RETRY_S) {
                    continue;
                }
                day.looked += 1;
                self.put_day(&day)?;
                match self.auto_confirm(&id).await {
                    Ok(Some(c)) => {
                        confirmed.insert(id.clone(), c);
                    }
                    Ok(None) => {
                        let _ = shared.put(&key, &now);
                        continue;
                    }
                    // MusicBrainz or Lidarr didn't answer: try again another day.
                    Err(e) => {
                        tracing::debug!("better copies: {e}");
                        continue;
                    }
                }
            }
            // One device asks for an album, ever: the first to claim it.
            match self.lidarr().claim(&format!("better:{id}")) {
                Ok(ClaimState::Ours) => {}
                Ok(ClaimState::Settling) => {
                    settling = true;
                    continue;
                }
                Ok(ClaimState::Theirs(_)) => continue,
                Err(e) => {
                    tracing::debug!("better copies: claim: {e}");
                    continue;
                }
            }
            shared.put(&format!("{BY}{id}"), &me)?;
            if let Some(item) = self.hunt_album(&id, s.better_copies_aim, &confirmed).await {
                if item.state == HuntState::Requested {
                    day.asked += 1;
                    self.put_day(&day)?;
                }
                out.push(item);
            }
        }
        // New claims settle in a minute: look again in an hour, not
        // tomorrow, while today's limit has room.
        if settling && day.asked < DAY_ASKS {
            shared.put(RAN_KEY, &(now - PASS_EVERY_S + 3_600))?;
        }
        Ok(out)
    }

    /// Confirm the album's release when exactly one official release fits
    /// it. None when none or several do (or the one found won't confirm).
    async fn auto_confirm(&self, album_id: &str) -> AppResult<Option<super::upgrade::Confirmed>> {
        let c = self.candidates(album_id).await?;
        let Some(rgid) = auto_pick(&c.title, &c.songs, &c.candidates) else { return Ok(None) };
        Ok(self.confirm(album_id, Some(&rgid)).await.ok().flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::super::mbpick::{Candidate, ListedTrack};
    use super::*;

    fn song(track: u32, title: &str, ms: u64) -> FolderSong {
        FolderSong { disc: 1, track, title: title.into(), duration_ms: ms }
    }

    fn cand(id: &str, title: &str, tracks: &[(u32, &str, u64)]) -> Candidate {
        let group = super::super::mbpick::Group {
            id: id.into(),
            title: title.into(),
            primary: Some("album".into()),
            secondary: vec![],
            first_release: "2015".into(),
            statuses: vec![],
            credit: vec![],
        };
        let mut c = Candidate { group, tracks: vec![] };
        c.tracks = tracks.iter().map(|(p, t, l)| ListedTrack { disc: 1, position: *p, title: (*t).into(), length_ms: Some(*l) }).collect();
        c
    }

    #[test]
    fn only_one_release_with_the_very_same_tracklist_is_picked() {
        let songs = [song(1, "Dawn", 200_000), song(2, "Noon", 180_000)];
        let same = cand("rg1", "First Light", &[(1, "Dawn", 201_000), (2, "Noon", 179_000)]);
        assert_eq!(auto_pick("First Light", &songs, std::slice::from_ref(&same)), Some("rg1".into()));
        // Two that fit: a person decides.
        let twin = cand("rg2", "First Light", &[(1, "Dawn", 200_000), (2, "Noon", 180_000)]);
        assert_eq!(auto_pick("First Light", &songs, &[same.clone(), twin]), None);
        // A deluxe edition isn't the album.
        assert_eq!(auto_pick("First Light", &songs, &[cand("rg3", "First Light (Deluxe)", &[(1, "Dawn", 200_000), (2, "Noon", 180_000)])]), None);
        // A song more, a song renamed, a length 4 s off, a missing length.
        assert_eq!(auto_pick("First Light", &songs, &[cand("rg4", "First Light", &[(1, "Dawn", 200_000), (2, "Noon", 180_000), (3, "Dusk", 1)])]), None);
        assert_eq!(auto_pick("First Light", &songs, &[cand("rg5", "First Light", &[(1, "Dawn", 200_000), (2, "Midday", 180_000)])]), None);
        assert_eq!(auto_pick("First Light", &songs, &[cand("rg6", "First Light", &[(1, "Dawn", 204_001), (2, "Noon", 180_000)])]), None);
        let mut no_len = same.clone();
        no_len.tracks[0].length_ms = None;
        assert_eq!(auto_pick("First Light", &songs, &[no_len]), None);
        // Songs in another order on the release aren't the same album.
        assert_eq!(auto_pick("First Light", &songs, &[cand("rg7", "First Light", &[(1, "Noon", 180_000), (2, "Dawn", 200_000)])]), None);
        assert_eq!(auto_pick("First Light", &[], &[same]), None);
    }
}
