//! Release radar: new official albums, EPs and singles from followed
//! artists and the artists played most, looked up on MusicBrainz about once
//! a day, plus optional auto-request of new albums through Lidarr.
//!
//! The look is light: artists' MusicBrainz ids come from the artist info
//! cache, and their recent release groups are asked for a few artists at a
//! time in one search, through the shared one-a-second MusicBrainz limiter,
//! with answers cached for most of a day. Every release group shown passes
//! the same rules a request does (an official release, album, EP or single,
//! credited to the artist alone, no compilation or live album).
//!
//! What was found lives on this device. Whether each release is in the
//! library, can be requested or was requested is worked out when the page
//! is read. An artist's first look only sets the scene: nothing found then
//! is announced or requested automatically.
//!
//! Auto-request is per artist (and a switch for all of them). It only asks
//! for new albums found after it was turned on, at most the daily cap a day,
//! and each goes through Lidarr's full request path: the MusicBrainz guards
//! (title clash with anything in the library, bootleg or promo only) and the
//! whole setup check. Refusals stay in the Requests list with why.

pub mod commands;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::artistinfo::is_mbid;
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::identity::fold::fold_artist;
use crate::lidarr::ArtistMbid;
use crate::lidarr::mbpick::{self, Group, MbGet, Refusal, Want, clash, library_albums};
use crate::lidarr::request::{RequestInput, RequestRecord, cover_thumb};
use crate::lidarr::status::RequestState;
use crate::plex::state::EventSink;
use crate::social::Notifier;
use crate::social::follows::FollowStore;

pub const EVENT_CHANGED: &str = "radar://changed";

const DAY: i64 = 24 * 3600;
/// Releases from the last this many days (and any announced for later).
pub const WINDOW_DAYS: i64 = 90;
/// A new look once this long has passed since the last ("about daily").
pub const CHECK_EVERY: i64 = 20 * 3600;
/// How long a MusicBrainz answer is kept for the radar.
pub const MB_TTL: i64 = 18 * 3600;
/// After a look that failed (offline, MusicBrainz busy), the next try waits this long.
const RETRY_AFTER: i64 = 3600;
/// Most artists looked at.
pub const MAX_ARTISTS: usize = 60;
/// Artists played most (last 90 days) added to the followed ones.
pub const TOP_ARTISTS: usize = 10;
/// Artists per MusicBrainz search.
pub const BATCH: usize = 8;
/// Pages of one search read at most (100 each).
const MAX_PAGES: usize = 2;
/// Most releases kept.
pub const MAX_ITEMS: usize = 80;
/// Daily cap choices for auto-request.
pub const CAP_CHOICES: [u32; 5] = [1, 2, 3, 5, 10];
pub const DEFAULT_CAP: u32 = 3;
/// Most artists with auto-request on.
pub const MAX_AUTO: usize = 200;

const STORE_KEY: &str = "radar.state.v1";
const AUTO_KEY: &str = "radar.auto.v1";

// ---------------------------------------------------------------- stored

/// A release the radar found (kept on this device).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub release_group_id: String,
    pub title: String,
    /// The artist as the library (or the follow) names them.
    pub artist: String,
    #[serde(default)]
    pub artist_id: Option<String>,
    pub artist_mbid: String,
    /// `album`, `ep` or `single`.
    pub kind: String,
    /// The first release date (`YYYY-MM-DD`, `YYYY-MM` or `YYYY`).
    pub date: String,
    pub found_at: i64,
    /// Found on the artist's first look: shown, never announced or auto-requested.
    #[serde(default)]
    pub baseline: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    checked_at: Option<i64>,
    /// When the last look was tried (it may have failed).
    tried_at: Option<i64>,
    items: Vec<Found>,
    /// Artists (MusicBrainz ids) looked at before.
    known: Vec<String>,
    /// Release groups auto-request has asked for (whatever came of it).
    auto_tried: Vec<String>,
}

/// An artist with auto-request on (this device).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoArtist {
    pub id: String,
    pub name: String,
    pub enabled_at: i64,
}

// ---------------------------------------------------------------- what the UI reads

/// One release on the radar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarItem {
    pub release_group_id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: Option<String>,
    pub kind: String,
    pub date: String,
    /// The Cover Art Archive's front cover, through the art cache.
    pub thumb: Option<String>,
    /// The library album it matches ("In your library").
    pub library_album_id: Option<String>,
    /// A request made for it.
    pub request: Option<RequestState>,
    /// The request was made by auto-request.
    pub auto: bool,
    /// Why it can't be requested (it shares a title with an unreleased album).
    pub refusal: Option<String>,
    /// Found since the last look before this one.
    pub new: bool,
}

/// `release_radar`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarPage {
    /// The setting is on.
    pub shown: bool,
    pub items: Vec<RadarItem>,
    pub checked_at: Option<i64>,
    /// A look is going now.
    pub checking: bool,
    /// Request buttons show.
    pub lidarr_ready: bool,
    /// Artists looked at: followed ones and those played most.
    pub artists: u32,
}

// ---------------------------------------------------------------- what the radar needs

/// An artist to look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadarArtist {
    pub id: Option<String>,
    pub name: String,
}

/// The artists played most lately.
pub trait TopPlayed: Send + Sync {
    fn top(&self, limit: usize) -> Vec<RadarArtist>;
}

/// The stats tables' top artists over the last 90 days.
pub struct StatsTop(pub DbHandle);

impl TopPlayed for StatsTop {
    fn top(&self, limit: usize) -> Vec<RadarArtist> {
        self.0
            .read(|c| {
                let today = crate::stats::today(c)?;
                let d = crate::stats::Days { lo: today - WINDOW_DAYS, hi: today, all: false };
                crate::stats::top_artists(c, d, limit as u32)
            })
            .unwrap_or_default()
            .into_iter()
            .map(|a| RadarArtist { id: a.artist_id, name: a.name })
            .collect()
    }
}

/// Lidarr, as the radar uses it.
#[async_trait]
pub trait RadarLidarr: Send + Sync {
    /// Set up and the last setup check passed.
    fn ready(&self) -> bool;
    fn requests(&self) -> Vec<RequestRecord>;
    async fn request_auto(&self, input: RequestInput) -> AppResult<RequestRecord>;
}

#[async_trait]
impl RadarLidarr for crate::lidarr::Lidarr {
    fn ready(&self) -> bool {
        self.status().ready
    }
    fn requests(&self) -> Vec<RequestRecord> {
        crate::lidarr::Lidarr::requests(self)
    }
    async fn request_auto(&self, input: RequestInput) -> AppResult<RequestRecord> {
        crate::lidarr::Lidarr::request_auto(self, input).await
    }
}

pub struct RadarDeps {
    pub db: DbHandle,
    /// MusicBrainz, paced and cached for most of a day.
    pub mb: Arc<dyn MbGet>,
    pub mbids: Arc<dyn ArtistMbid>,
    pub follows: Arc<dyn FollowStore>,
    pub top: Arc<dyn TopPlayed>,
    pub lidarr: Arc<dyn RadarLidarr>,
    pub notifier: Arc<dyn Notifier>,
    pub events: Arc<dyn EventSink>,
    /// MusicBrainz may be asked (its source is on and the app is online).
    pub usable: Arc<dyn Fn() -> bool + Send + Sync>,
    pub private_session: Arc<AtomicBool>,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

/// What a look found and did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// Releases new since the last look (not an artist's first look).
    pub new: Vec<Found>,
    /// Auto-requests made, refused ones included.
    pub auto: Vec<RequestRecord>,
    /// New albums left for another day by the daily cap.
    pub waiting: usize,
}

#[derive(Clone)]
pub struct Radar {
    inner: Arc<RadarDeps>,
    checking: Arc<AtomicBool>,
    one: Arc<tokio::sync::Mutex<()>>,
}

// ---------------------------------------------------------------- MusicBrainz

/// `YYYY-MM-DD` for a unix time.
pub fn ymd(unix: i64) -> String {
    let (y, m, d) = crate::stats::civil_from_days(unix.div_euclid(DAY));
    format!("{y:04}-{m:02}-{d:02}")
}

/// One search for the release groups of several artists that first came
/// out on or after `from`.
pub fn search_url(mbids: &[String], from: &str, offset: usize) -> String {
    let arids: Vec<String> = mbids.iter().map(|m| format!("arid:{m}")).collect();
    let mut u = Url::parse(&format!("{}/release-group", mbpick::WS)).expect("static url");
    u.query_pairs_mut()
        .append_pair("query", &format!("({}) AND firstreleasedate:[{from} TO 9999]", arids.join(" OR ")))
        .append_pair("limit", "100")
        .append_pair("offset", &offset.to_string())
        .append_pair("fmt", "json");
    u.into()
}

/// The release groups in a search answer, read like a lookup (statuses
/// from their releases, credit from the group).
pub fn parse_search(v: &Value) -> Vec<Group> {
    v.get("release-groups").and_then(Value::as_array).map(|a| a.iter().filter_map(mbpick::parse_group).collect()).unwrap_or_default()
}

/// Whether a group belongs on the radar for this artist: every request
/// rule but the title clash (shown separately), and a known date.
pub fn fits(g: &Group, want: &Want) -> bool {
    !g.first_release.trim().is_empty() && mbpick::vet(g, want, &[], false).is_ok()
}

// ---------------------------------------------------------------- texts

fn kind_word(kind: &str) -> &'static str {
    match kind {
        "ep" => "EP",
        "single" => "single",
        _ => "album",
    }
}

/// The notification for new releases: (title, body).
pub fn notice(found: &[Found]) -> Option<(String, String)> {
    match found {
        [] => None,
        [f] => Some((format!("New {} from {}", kind_word(&f.kind), f.artist), f.title.clone())),
        [f, rest @ ..] => {
            let artists: HashSet<String> = found.iter().map(|f| fold_artist(&f.artist)).collect();
            if artists.len() == 1 {
                Some((format!("New from {}", f.artist), format!("{} and {} more", f.title, rest.len())))
            } else {
                Some(("Release radar".into(), format!("{} new releases from artists you follow and play", found.len())))
            }
        }
    }
}

/// The summary after auto-request ran: (title, body).
pub fn summary(made: &[RequestRecord], waiting: usize, cap: u32) -> Option<(String, String)> {
    let sent: Vec<&RequestRecord> = made.iter().filter(|r| r.state != RequestState::Failed).collect();
    let refused = made.len() - sent.len();
    if made.is_empty() && waiting == 0 {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    match sent.as_slice() {
        [] => {}
        [r] => parts.push(format!("Requested {} by {}.", r.title, r.artist)),
        many => parts.push(format!(
            "Requested {} new albums: {}.",
            many.len(),
            many.iter().map(|r| r.title.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
    match refused {
        0 => {}
        1 => parts.push("1 couldn't be requested. Requests in Settings say why.".into()),
        n => parts.push(format!("{n} couldn't be requested. Requests in Settings say why.")),
    }
    match waiting {
        0 => {}
        1 => parts.push(format!("1 more waits for tomorrow (up to {cap} a day).")),
        n => parts.push(format!("{n} more wait for tomorrow (up to {cap} a day).")),
    }
    Some(("Auto-request".into(), parts.join(" ")))
}

// ---------------------------------------------------------------- the radar

impl Radar {
    pub fn new(deps: RadarDeps) -> Self {
        Self { inner: Arc::new(deps), checking: Arc::default(), one: Arc::default() }
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    fn stored(&self) -> Stored {
        self.inner.db.read(|c| kv_get(c, STORE_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save(&self, s: &Stored) -> AppResult<()> {
        let json = serde_json::to_string(s).map_err(|e| AppError::Internal(e.to_string()))?;
        self.inner.db.write(|c| kv_set(c, STORE_KEY, &json))
    }

    pub fn auto_artists(&self) -> Vec<AutoArtist> {
        self.inner.db.read(|c| kv_get(c, AUTO_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Turn auto-request on or off for an artist. Turning it on also turns
    /// on auto-request as a whole (the user just asked for it).
    pub fn set_auto(&self, id: &str, name: &str, on: bool) -> AppResult<Vec<AutoArtist>> {
        if id.trim().is_empty() || id.len() > 300 || name.trim().is_empty() {
            return Err(AppError::Invalid("Not an artist.".into()));
        }
        let mut all = self.auto_artists();
        let had = all.iter().any(|a| a.id == id);
        match (on, had) {
            (true, false) => {
                if all.len() >= MAX_AUTO {
                    return Err(AppError::Invalid(format!("Auto-request can be on for up to {MAX_AUTO} artists.")));
                }
                all.push(AutoArtist { id: id.to_string(), name: name.trim().to_string(), enabled_at: self.now() });
            }
            (false, true) => all.retain(|a| a.id != id),
            _ => {}
        }
        let json = serde_json::to_string(&all).map_err(|e| AppError::Internal(e.to_string()))?;
        self.inner.db.write(|c| kv_set(c, AUTO_KEY, &json))?;
        if on {
            let s = crate::settings::try_load(&self.inner.db)?;
            if !s.auto_request {
                let s = crate::settings::Settings { auto_request: true, ..s };
                crate::settings::save(&self.inner.db, &s)?;
                self.inner.events.emit("settings://changed", serde_json::to_value(&s).unwrap_or_default());
            }
        }
        self.inner.events.emit(EVENT_CHANGED, Value::Null);
        Ok(all)
    }

    /// The artists to look at: followed ones, those played most, and any
    /// with auto-request on. One entry per artist (by folded name).
    pub fn artists(&self) -> Vec<RadarArtist> {
        let mut out: Vec<RadarArtist> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut add = |a: RadarArtist| {
            let k = fold_artist(&a.name);
            if !k.is_empty() && seen.insert(k) && out.len() < MAX_ARTISTS {
                out.push(a);
            }
        };
        for a in self.auto_artists() {
            add(RadarArtist { id: Some(a.id), name: a.name });
        }
        for f in self.inner.follows.list().unwrap_or_default() {
            add(RadarArtist { id: Some(f.id), name: f.title });
        }
        for a in self.inner.top.top(TOP_ARTISTS) {
            add(a);
        }
        out
    }

    /// Whether a look is due: the setting on, MusicBrainz usable, and about
    /// a day since the last (an hour after one that failed).
    pub fn due(&self) -> bool {
        let s = crate::settings::load(&self.inner.db);
        if !s.release_radar || !(self.inner.usable)() {
            return false;
        }
        let st = self.stored();
        let now = self.now();
        st.checked_at.is_none_or(|t| now - t >= CHECK_EVERY) && st.tried_at.is_none_or(|t| now - t >= RETRY_AFTER)
    }

    pub fn checking(&self) -> bool {
        self.checking.load(Ordering::Relaxed)
    }

    /// One look: MusicBrainz for every artist, then announcements and
    /// auto-requests. Fails when MusicBrainz can't be asked (what was found
    /// before stays).
    pub async fn check(&self) -> AppResult<Outcome> {
        let _one = self.one.lock().await;
        let s = crate::settings::load(&self.inner.db);
        if !s.release_radar {
            return Ok(Outcome::default());
        }
        if !(self.inner.usable)() {
            return Err(AppError::Network("MusicBrainz can't be reached right now.".into()));
        }
        self.checking.store(true, Ordering::Relaxed);
        self.inner.events.emit(EVENT_CHANGED, Value::Null);
        let r = self.look(&s).await;
        self.checking.store(false, Ordering::Relaxed);
        self.inner.events.emit(EVENT_CHANGED, Value::Null);
        r
    }

    async fn look(&self, s: &crate::settings::Settings) -> AppResult<Outcome> {
        let now = self.now();
        let mut st = self.stored();
        st.tried_at = Some(now);
        let _ = self.save(&st);

        // Who, with their MusicBrainz ids.
        let mut wants: BTreeMap<String, Want> = BTreeMap::new();
        for a in self.artists() {
            let mbid = self.inner.mbids.artist_mbid(a.id.as_deref().unwrap_or_default(), &a.name).await;
            if let Some(m) = mbid.map(|m| m.to_ascii_lowercase()).filter(|m| is_mbid(m)) {
                wants.entry(m.clone()).or_insert(Want { name: a.name, mbid: Some(m), artist_id: a.id });
            }
        }

        // Their release groups since the window opened.
        let from = ymd(now - WINDOW_DAYS * DAY);
        let ids: Vec<String> = wants.keys().cloned().collect();
        let mut groups: Vec<(String, Group)> = Vec::new();
        for chunk in ids.chunks(BATCH) {
            for page in 0..MAX_PAGES {
                let Some(v) = self.inner.mb.get(&search_url(chunk, &from, page * 100)).await? else { break };
                let got = parse_search(&v);
                let n = v.get("release-groups").and_then(Value::as_array).map_or(0, Vec::len);
                for g in got {
                    let Some(m) = g.credit.first().map(|c| c.id.clone()) else { continue };
                    if let Some(w) = wants.get(&m)
                        && fits(&g, w)
                        && g.first_release.as_str() >= from.as_str()
                    {
                        groups.push((m, g));
                    }
                }
                let total = v.get("count").and_then(Value::as_u64).unwrap_or(0) as usize;
                if n < 100 || (page + 1) * 100 >= total {
                    break;
                }
            }
        }

        // Merge with what was found before.
        let known: HashSet<String> = st.known.iter().cloned().collect();
        let before: HashMap<String, Found> = st.items.iter().map(|f| (f.release_group_id.clone(), f.clone())).collect();
        let mut items: Vec<Found> = Vec::new();
        let mut new: Vec<Found> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (m, g) in groups {
            if !seen.insert(g.id.clone()) {
                continue;
            }
            let w = &wants[&m];
            let f = match before.get(&g.id) {
                Some(old) => Found { title: g.title.clone(), date: g.first_release.clone(), ..old.clone() },
                None => {
                    let f = Found {
                        release_group_id: g.id.clone(),
                        title: g.title.clone(),
                        artist: w.name.clone(),
                        artist_id: w.artist_id.clone(),
                        artist_mbid: m.clone(),
                        kind: g.primary.clone().unwrap_or_default(),
                        date: g.first_release.clone(),
                        found_at: now,
                        baseline: !known.contains(&m),
                    };
                    if !f.baseline {
                        new.push(f.clone());
                    }
                    f
                }
            };
            items.push(f);
        }
        items.sort_by(|a, b| b.date.cmp(&a.date).then(a.title.cmp(&b.title)));
        items.truncate(MAX_ITEMS);
        st.items = items;
        st.known = wants.keys().cloned().collect();
        st.checked_at = Some(now);
        self.save(&st)?;

        if s.radar_notify
            && !self.inner.private_session.load(Ordering::Relaxed)
            && let Some((t, b)) = notice(&new)
        {
            self.inner.notifier.notify(&t, &b);
        }

        let (auto, waiting) = self.auto_request(s, &mut st, now).await?;
        if let Some((t, b)) = summary(&auto, waiting, s.auto_request_cap) {
            self.inner.notifier.notify(&t, &b);
        }
        Ok(Outcome { new, auto, waiting })
    }

    /// Ask Lidarr for new albums by artists with auto-request on, up to
    /// the daily cap. Returns what was made, and how many wait for another day.
    async fn auto_request(&self, s: &crate::settings::Settings, st: &mut Stored, now: i64) -> AppResult<(Vec<RequestRecord>, usize)> {
        let artists = self.auto_artists();
        if !s.auto_request || artists.is_empty() || !self.inner.lidarr.ready() {
            return Ok((Vec::new(), 0));
        }
        let on: HashMap<String, &AutoArtist> = artists.iter().map(|a| (fold_artist(&a.name), a)).collect();
        let tried: HashSet<String> = st.auto_tried.iter().cloned().collect();
        let mut todo: Vec<Found> = st
            .items
            .iter()
            .filter(|f| f.kind == "album" && !f.baseline && !tried.contains(&f.release_group_id))
            .filter(|f| on.get(&fold_artist(&f.artist)).is_some_and(|a| f.found_at > a.enabled_at))
            .cloned()
            .collect();
        todo.sort_by(|a, b| a.date.cmp(&b.date).then(a.title.cmp(&b.title)));
        let today = now.div_euclid(DAY);
        let used = self
            .inner
            .lidarr
            .requests()
            .iter()
            .filter(|r| r.auto && r.state != RequestState::Failed && r.created_at.div_euclid(DAY) == today)
            .count();
        let mut left = (s.auto_request_cap as usize).saturating_sub(used);
        let mut made = Vec::new();
        let mut waiting = 0;
        for f in todo {
            if left == 0 {
                waiting += 1;
                continue;
            }
            let input = RequestInput {
                release_group_id: f.release_group_id.clone(),
                title: f.title.clone(),
                artist: f.artist.clone(),
                artist_id: f.artist_id.clone(),
                artist_mbid: None,
                typed: None,
            };
            let rec = self.inner.lidarr.request_auto(input).await?;
            st.auto_tried.push(f.release_group_id.clone());
            self.save(st)?;
            if rec.state != RequestState::Failed {
                left -= 1;
            }
            made.push(rec);
        }
        Ok((made, waiting))
    }

    /// The page (and the Home shelf).
    pub fn page(&self) -> AppResult<RadarPage> {
        let s = crate::settings::load(&self.inner.db);
        let st = self.stored();
        let lidarr_ready = self.inner.lidarr.ready();
        let artists = self.artists().len() as u32;
        if !s.release_radar {
            return Ok(RadarPage { shown: false, items: Vec::new(), checked_at: st.checked_at, checking: false, lidarr_ready, artists });
        }
        let requests = self.inner.lidarr.requests();
        let from = ymd(self.now() - WINDOW_DAYS * DAY);
        let last = st.items.iter().map(|f| f.found_at).max().unwrap_or(0);
        let items = self.inner.db.read(|c| {
            let mut libs: HashMap<String, Vec<mbpick::LibraryAlbum>> = HashMap::new();
            let mut out = Vec::new();
            for f in st.items.iter().filter(|f| f.date.as_str() >= from.as_str()) {
                let key = fold_artist(&f.artist);
                if !libs.contains_key(&key) {
                    libs.insert(key.clone(), library_albums(c, f.artist_id.as_deref(), &f.artist)?);
                }
                let lib = &libs[&key];
                let hit = clash(&f.title, lib);
                let req = requests.iter().find(|r| r.release_group_id == f.release_group_id);
                out.push(RadarItem {
                    release_group_id: f.release_group_id.clone(),
                    title: f.title.clone(),
                    artist: f.artist.clone(),
                    artist_id: f.artist_id.clone(),
                    kind: f.kind.clone(),
                    date: f.date.clone(),
                    thumb: cover_thumb(&f.release_group_id),
                    library_album_id: hit.filter(|a| !a.unreleased).map(|a| a.id.clone()),
                    request: req.map(|r| r.state),
                    auto: req.is_some_and(|r| r.auto),
                    refusal: hit
                        .filter(|a| a.unreleased)
                        .map(|a| Refusal::TitleClash { title: a.title.clone(), unreleased: true }.message()),
                    new: !f.baseline && f.found_at == last && st.checked_at.is_some_and(|t| t == last),
                });
            }
            Ok(out)
        })?;
        Ok(RadarPage { shown: true, items, checked_at: st.checked_at, checking: self.checking(), lidarr_ready, artists })
    }
}

/// Startup work goes first.
const START_DELAY: Duration = Duration::from_secs(180);
/// How often it asks whether a look is due.
const LOOK_EVERY: Duration = Duration::from_secs(30 * 60);

/// The daily look, for the life of the app.
pub async fn watch(r: Radar) {
    tokio::time::sleep(START_DELAY).await;
    loop {
        if r.due() {
            match r.check().await {
                Ok(o) => tracing::debug!(new = o.new.len(), auto = o.auto.len(), "release radar"),
                Err(e) => tracing::debug!("release radar: {e}"),
            }
        }
        tokio::time::sleep(LOOK_EVERY).await;
        crate::platform::foreground::settled().await;
    }
}
