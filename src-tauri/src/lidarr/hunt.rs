//! "Look for better copies" for protected artists (off by default, per
//! artist). Managed artists get better copies from Lidarr's own profile; a
//! protected artist's library is never touched by Lidarr, so Platter asks
//! for each official album below the target (Lossless 16-bit or Hi-res
//! 24-bit) into the staging folder, with a profile whose cutoff is the
//! target, and each copy that lands goes through the usual swap rules.
//!
//! FLAC and FLAC 24bit are separate qualities in Lidarr, and a usual
//! Lossless cutoff groups both, so hi-res needs its own profile ("Platter
//! Hi-Res", offered as a change that asks first).
//!
//! What was tried is kept per album (shared): the release group and the
//! quality asked for. An album is looked at again once a week, and asked
//! for again only when the last try changed something; one whose remaining
//! songs never swap settles as "partly upgraded" and stops. Albums without
//! a confirmed official release are out of scope and say so.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::modes::{ArtistPrefs, Target};
use super::quality::{CD, Rank};
use super::status::RequestState;
use super::upgrade::{Confirmed, Upgrades};
use crate::error::AppResult;

/// How long before an album is looked at again.
pub const RECHECK_S: i64 = 7 * 86_400;

/// What was tried for one album (shared, `hunt:<album id>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tried {
    pub release_group: String,
    /// The quality asked for, as a rank.
    pub aim: u32,
    pub at: i64,
    /// Swaps the album had when this try was made.
    pub swaps_before: usize,
    /// Partly upgraded: the rest never swaps; not asked for again.
    #[serde(default)]
    pub settled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HuntState {
    /// At or above the target.
    AtTarget,
    /// Asked for now.
    Requested,
    /// Asked for within the last week.
    Waiting,
    /// Some songs swapped, the rest never will.
    PartlyUpgraded,
    /// No confirmed official release to ask for.
    OutOfScope,
    /// Hi-res needs the Platter Hi-Res profile.
    NeedsProfile,
    /// Asking failed; why is in the note.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HuntItem {
    pub album_id: String,
    pub title: String,
    pub artist: String,
    /// The album now ("MP3 320").
    pub now: String,
    pub state: HuntState,
    pub note: Option<String>,
}

/// The lowest rank that meets a target.
pub fn target_rank(t: Target) -> Rank {
    match t {
        Target::Lossless => CD,
        Target::HiRes => Rank::lossless(Some(24)),
    }
}

/// Settles an album whose last try changed nothing.
pub fn settles(prev: &Tried, swaps_now: usize) -> bool {
    prev.swaps_before > 0 && swaps_now == prev.swaps_before
}

impl Upgrades {
    /// One pass for every protected artist with "Look for better copies" on.
    pub async fn hunt(&self) -> AppResult<Vec<HuntItem>> {
        let l = self.lidarr().clone();
        let cx = l.ctx().await?;
        let prefs = l.artist_prefs();
        let mut out = Vec::new();
        for m in cx.modes.iter().filter(|m| m.protected()) {
            let Some(p) = prefs.get(&m.key).filter(|p| p.hunt) else { continue };
            let Some(artist_id) = m.artist_id.clone() else { continue };
            out.extend(self.hunt_artist(&artist_id, p).await);
        }
        Ok(out)
    }

    async fn hunt_artist(&self, artist_id: &str, p: &ArtistPrefs) -> Vec<HuntItem> {
        let aid = artist_id.to_string();
        let ids: Vec<String> = self
            .inner_db()
            .read_async(move |c| {
                let mut st = c.prepare("SELECT id FROM albums WHERE artist_id = ?1 ORDER BY year, title")?;
                Ok(st.query_map([&aid], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?)
            })
            .await
            .unwrap_or_default();
        let confirmed = self.confirmations();
        let mut out = Vec::new();
        for id in ids {
            if let Some(item) = self.hunt_album(&id, p.target, &confirmed).await {
                out.push(item);
            }
        }
        out
    }

    /// One album: at the target, out of scope, waiting on its last try,
    /// settled, or asked for now into staging. None for an album that
    /// isn't there any more or isn't released.
    pub(super) async fn hunt_album(&self, id: &str, aim: Target, confirmed: &HashMap<String, Confirmed>) -> Option<HuntItem> {
        let a = self.album(id).await.ok()?;
        // Unreleased in any part, or a swap undone before: left as it is.
        if !a.all_released() || self.kept_as_is(id) {
            return None;
        }
        let (rank, label) = a.current()?;
        let target = target_rank(aim);
        let floor = Rank(target.0 - 1);
        let shared = self.lidarr().shared();
        let now = self.now();
        let mut item = HuntItem { album_id: id.to_string(), title: a.title.clone(), artist: a.artist.clone(), now: label, state: HuntState::AtTarget, note: None };
        if rank >= target {
            return Some(item);
        }
        let Some(c) = confirmed.get(id) else {
            item.state = HuntState::OutOfScope;
            item.note = Some("No confirmed official studio release to ask for. Confirm which release it is from the album's menu.".into());
            return Some(item);
        };
        let key = format!("hunt:{id}");
        let swaps_now = self.records().iter().filter(|r| r.album_id == id).map(|r| r.swaps.iter().filter(|s| !s.undone).count()).sum::<usize>();
        let prev: Option<Tried> = shared.get(&key);
        if let Some(t) = &prev {
            if t.settled {
                item.state = HuntState::PartlyUpgraded;
                return Some(item);
            }
            if now - t.at < RECHECK_S && t.aim == floor.0 && t.release_group == c.release_group_id {
                item.state = HuntState::Waiting;
                return Some(item);
            }
            if settles(t, swaps_now) {
                let _ = shared.put(&key, &Tried { settled: true, ..t.clone() });
                item.state = HuntState::PartlyUpgraded;
                item.note = Some("Some songs were upgraded; the rest don't match a better copy, so Platter stopped looking.".into());
                return Some(item);
            }
        }
        // A request still in progress isn't asked for again.
        if self.records().iter().any(|r| r.album_id == id && !r.request.state.settled() && r.request.state != RequestState::Failed) {
            item.state = HuntState::Waiting;
            return Some(item);
        }
        match self.request_aimed_one(id, Some(floor)).await {
            Ok(_) => {
                let _ = shared.put(&key, &Tried { release_group: c.release_group_id.clone(), aim: floor.0, at: now, swaps_before: swaps_now, settled: false });
                item.state = HuntState::Requested;
            }
            Err(e) => {
                let msg = e.to_string();
                item.state = if aim == Target::HiRes && msg.contains("profile") { HuntState::NeedsProfile } else { HuntState::Failed };
                item.note = Some(if item.state == HuntState::NeedsProfile {
                    format!("Hi-res needs a profile that stops at FLAC 24bit. Platter can add the {} profile (it asks first).", super::changes::HIRES_PROFILE)
                } else {
                    msg
                });
            }
        }
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_and_settling() {
        assert_eq!(target_rank(Target::Lossless), CD);
        assert!(Rank::lossless(Some(16)) >= target_rank(Target::Lossless));
        assert!(Rank::lossless(Some(16)) < target_rank(Target::HiRes), "FLAC is not hi-res");
        assert!(Rank::lossy(Some(320)) < target_rank(Target::Lossless));
        let t = Tried { swaps_before: 1, ..Default::default() };
        assert!(settles(&t, 1), "the last try swapped nothing more");
        assert!(!settles(&t, 2));
        assert!(!settles(&Tried::default(), 0), "never swapped: not partly upgraded");
    }
}
