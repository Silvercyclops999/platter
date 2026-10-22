//! The daily look on Platter Bridge, so new albums by followed artists
//! are got while every device is closed.
//!
//! - An owner's device hands its Bridges a plan (`RadarPlan`): whether new
//!   albums are got, singles too, the daily cap, and the followed artists
//!   set to get them (name, Plex id, MusicBrainz id, when followed).
//!   Nothing else leaves the device, and only the owner's devices send it.
//!   A device sends it only when it changed since it last sent one to that
//!   Bridge, and one that never had it on sends none, so a new device not
//!   set up yet doesn't stop the others'. The newest plan wins.
//! - The Bridge runs the same look and asks through the same request path
//!   as a device. Devices keep looking too, for their own page. Each
//!   release is claimed first (`lidarr::shared`), so only one of them asks,
//!   and the daily cap counts every one's asks (`FOLLOW_DAY_KEY`).
//! - What the Bridge asked for goes back to every device (`RadarMade`),
//!   which adds it to its Requests list, so it shows and is followed there.
//!   A request cleared on a device doesn't come back.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{RadarArtist, TopPlayed};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::identity::fold::fold_artist;
use crate::lidarr::request::RequestRecord;
use crate::lidarr::status::RequestState;
use crate::social::follows::{FollowStore, FollowedArtist};

/// The plan the look follows, on the Bridge (the newest a device sent).
const PLAN_KEY: &str = "radar.bridgePlan";
/// Links whose device has sent a plan.
const LINKS_KEY: &str = "radar.bridgeLinks";
/// What the Bridge asked for, on the Bridge.
const MADE_KEY: &str = "radar.bridgeMade";
/// Requests sent back to devices: the newest this many, from this long.
pub const KEEP_MADE: usize = 100;
pub const MADE_FOR_S: i64 = 14 * 24 * 3600;

/// One followed artist whose new albums are got.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFollow {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub mbid: Option<String>,
    pub followed_at: i64,
}

/// What a device hands its Bridge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RadarPlan {
    /// New albums are got (the radar and "Get their new albums" are on).
    pub on: bool,
    pub singles: bool,
    pub cap: u32,
    pub follows: Vec<PlanFollow>,
}

/// The plan the Bridge was last handed.
pub fn plan(db: &DbHandle) -> RadarPlan {
    db.read(|c| kv_get(c, PLAN_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn known(db: &DbHandle) -> Vec<String> {
    db.read(|c| kv_get(c, LINKS_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// The links whose device has sent a plan, so it reads what the Bridge asked for.
pub fn plan_links(db: &DbHandle) -> Vec<String> {
    known(db)
}

fn tidy(mut p: RadarPlan) -> RadarPlan {
    p.follows.retain(|f| !f.id.trim().is_empty() && f.id.len() <= 300 && !f.title.trim().is_empty());
    p.follows.truncate(super::MAX_FOLLOWED);
    if !super::CAP_CHOICES.contains(&p.cap) {
        p.cap = super::DEFAULT_CAP;
    }
    if !p.on {
        p.follows.clear();
    }
    p
}

/// Take a device's plan (from `link`): the newest one wins, so an unfollow
/// or turning it off on any device holds. Devices send one only when theirs
/// changed, so a closed one's older plan never comes back by itself. `None`
/// forgets the device (it unpaired). The plan becomes the Bridge's
/// settings, so the look follows it. Says whether the plan changed.
pub fn take_plan(db: &DbHandle, link: &str, p: Option<&RadarPlan>) -> AppResult<bool> {
    let mut links = known(db);
    links.retain(|l| l != link);
    if p.is_some() {
        links.push(link.to_string());
    }
    let json = serde_json::to_string(&links).map_err(|e| AppError::Internal(e.to_string()))?;
    db.write(|c| kv_set(c, LINKS_KEY, &json))?;
    let Some(p) = p else { return Ok(false) };
    let next = tidy(p.clone());
    if plan(db) == next {
        return Ok(false);
    }
    // Read for real: a settings read that fails mustn't save defaults over
    // the Bridge's Lidarr settings.
    let mut s = crate::settings::try_load(db)?;
    s.release_radar = next.on;
    s.follow_get = next.on;
    s.follow_get_singles = next.singles;
    s.auto_request_cap = next.cap;
    // Nobody reads the Bridge's notifications.
    s.radar_notify = false;
    crate::settings::save(db, &s)?;
    let json = serde_json::to_string(&next).map_err(|e| AppError::Internal(e.to_string()))?;
    db.write(|c| kv_set(c, PLAN_KEY, &json))?;
    Ok(true)
}

/// The followed artists, from the plan (all of them get new albums).
pub struct PlanFollows(pub DbHandle);

impl FollowStore for PlanFollows {
    fn list(&self) -> AppResult<Vec<FollowedArtist>> {
        Ok(plan(&self.0)
            .follows
            .into_iter()
            .map(|f| FollowedArtist { id: f.id, title: f.title, thumb: None, followed_at: f.followed_at, just_show: false })
            .collect())
    }
    /// The list belongs to the devices.
    fn save(&self, _all: &[FollowedArtist]) -> AppResult<()> {
        Ok(())
    }
}

/// Artists' MusicBrainz ids as the devices know them.
pub struct PlanMbids(pub DbHandle);

#[async_trait]
impl crate::lidarr::ArtistMbid for PlanMbids {
    async fn artist_mbid(&self, artist_id: &str, name: &str) -> Option<String> {
        let want = fold_artist(name);
        let follows = plan(&self.0).follows;
        // By id when there is one (a same-name artist isn't this one), else by name.
        let found = if artist_id.is_empty() {
            follows.iter().find(|f| !want.is_empty() && fold_artist(&f.title) == want)
        } else {
            follows.iter().find(|f| f.id == artist_id)
        };
        found
            .and_then(|f| f.mbid.clone())
            .filter(|m| crate::artistinfo::is_mbid(m))
    }
}

/// The Bridge has no listening of its own.
pub struct NoTop;

impl TopPlayed for NoTop {
    fn top(&self, _limit: usize) -> Vec<RadarArtist> {
        Vec::new()
    }
}

/// One release the Bridge's look asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Made {
    release_group_id: String,
    at: i64,
}

fn made(db: &DbHandle, now: i64) -> Vec<Made> {
    let all: Vec<Made> = db.read(|c| kv_get(c, MADE_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    all.into_iter().filter(|m| now - m.at < MADE_FOR_S).collect()
}

/// Keep what a look asked for (refusals stay on the Bridge's own list).
pub fn note_made(db: &DbHandle, recs: &[RequestRecord], now: i64) -> AppResult<()> {
    let mut all = made(db, now);
    for r in recs.iter().filter(|r| r.state != RequestState::Failed) {
        if !all.iter().any(|m| m.release_group_id == r.release_group_id) {
            all.insert(0, Made { release_group_id: r.release_group_id.clone(), at: now });
        }
    }
    all.truncate(KEEP_MADE);
    let json = serde_json::to_string(&all).map_err(|e| AppError::Internal(e.to_string()))?;
    db.write(|c| kv_set(c, MADE_KEY, &json))
}

/// What goes back to the devices: those requests as the Bridge's list has
/// them now, once Lidarr has them (one still waiting to be sent isn't, so
/// a device never sends it too).
pub fn made_records(db: &DbHandle, list: &[RequestRecord], now: i64) -> Vec<RequestRecord> {
    made(db, now)
        .into_iter()
        .filter_map(|m| list.iter().find(|r| r.release_group_id == m.release_group_id))
        .filter(|r| !matches!(r.state, RequestState::Queued | RequestState::Failed))
        .cloned()
        .collect()
}

/// Shared by the Bridge's radar and its tests.
pub fn deps(
    db: DbHandle,
    mb: Arc<dyn crate::lidarr::mbpick::MbGet>,
    lidarr: Arc<dyn super::RadarLidarr>,
    events: Arc<dyn crate::plex::state::EventSink>,
    notifier: Arc<dyn crate::social::Notifier>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
) -> super::RadarDeps {
    super::RadarDeps {
        mbids: Arc::new(PlanMbids(db.clone())),
        follows: Arc::new(PlanFollows(db.clone())),
        top: Arc::new(NoTop),
        db,
        mb,
        lidarr,
        notifier,
        events,
        usable: Arc::new(|| true),
        private_session: Arc::default(),
        clock,
    }
}
