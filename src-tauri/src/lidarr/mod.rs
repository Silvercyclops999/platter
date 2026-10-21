//! Lidarr: the connection (address in Settings, API key in the OS keychain),
//! the read-only setup check that must pass before Platter ever asks Lidarr
//! for anything, and requests. The check only reads (`api` is GET only);
//! requests go through the three fixed writes in `write`, and only after
//! the whole check has passed again.

pub mod api;
pub mod audiomatch;
pub mod changes;
pub mod check;
pub mod catchup;
pub mod commands;
pub mod better;
pub mod hunt;
pub mod lookup;
pub mod homeside;
pub mod conventions;
pub mod editions;
pub mod grabbed;
pub mod mbpick;
pub mod modes;
pub mod paths;
pub mod plexside;
pub mod promote;
pub mod provenance;
pub mod quality;
pub mod quick;
pub mod rematch;
pub mod relayed;
pub mod request;
pub mod shared;
pub mod status;
pub mod swap;
pub mod tap;
pub mod tagcarry;
pub mod upgrade;
pub mod upmatch;
pub mod waves;
pub mod waves_bridge;
pub mod write;

#[cfg(test)]
mod catchup_tests;
#[cfg(test)]
mod changes_tests;
#[cfg(test)]
mod get_tests;
#[cfg(test)]
mod homeside_tests;
#[cfg(test)]
mod modes_tests;
#[cfg(test)]
mod request_keep_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod waves_tests;
#[cfg(test)]
mod waves_bridge_tests;
#[cfg(test)]
mod tap_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod upgrade_tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;

use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::plex::PlexState;
use crate::plex::client::MetaType;
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;
use crate::web::Web;

use api::{DirectRoute, LidarrClient, LidarrGet, LidarrRoute, Route, SystemStatus, parse_version};
use check::{CheckContext, CheckReport, LidarrSnapshot, PlexFile, PlexFiles, PlexIndex, dedup_wanted, evaluate};
use paths::NormPath;

/// Keychain account for the API key.
pub const KEY_ACCOUNT: &str = "lidarr-api-key";
/// Keychain account for extra headers the away address needs (JSON pairs).
pub const HEADERS_ACCOUNT: &str = "lidarr-headers";
/// The oldest Lidarr the check is written against.
pub const MIN_VERSION: (u64, u64, u64) = (2, 0, 0);
/// Root folders the user said are separate, per Lidarr address (this device).
const CONFIRMED_KEY: &str = "lidarr.confirmedRoots";
/// Most folder listings one check asks Lidarr for.
pub const MAX_LISTINGS: usize = 48;
/// Rounds of listings (each round follows matches one folder deeper).
const MAX_ROUNDS: usize = 5;

/// Plex's music folders and files, read only.
#[async_trait]
pub trait PlexMusic: Send + Sync {
    async fn music_files(&self) -> AppResult<PlexFiles>;
}

/// The connected server's music sections: their folders and every file.
pub struct PlexMusicFiles(pub PlexState);

#[async_trait]
impl PlexMusic for PlexMusicFiles {
    async fn music_files(&self) -> AppResult<PlexFiles> {
        // Only a server that didn't answer is "can't reach"; a reply that
        // couldn't be read says so.
        let unreachable = |e: AppError| match e {
            AppError::Network(_) => AppError::Network("Platter can't reach your Plex server, so it can't compare folders. Try again when it's back.".into()),
            AppError::Decode(m) => AppError::Decode(format!("Your Plex server answered, but Platter couldn't read its file list, so it can't compare folders: {m}")),
            e => e,
        };
        let c = self.0.client().await.map_err(unreachable)?;
        let mut out = PlexFiles::default();
        for s in c.music_sections().await.map_err(unreachable)? {
            out.locations.extend(s.locations.iter().cloned());
            for t in c.list_all(&s.id, MetaType::Track, &[]).await.map_err(unreachable)? {
                for p in t.media.iter().flat_map(|m| m.parts.iter()) {
                    if let Some(f) = p.file.as_deref().filter(|f| !f.trim().is_empty()) {
                        out.files.push(PlexFile { path: f.to_string(), size: p.size });
                    }
                }
            }
        }
        Ok(out)
    }
}

/// A library artist's MusicBrainz id (from Plex or the artist info cache).
#[async_trait]
pub trait ArtistMbid: Send + Sync {
    async fn artist_mbid(&self, artist_id: &str, name: &str) -> Option<String>;
}

/// The artist info service's id lookup (Plex's, else Wikidata's).
pub struct InfoMbid(pub crate::artistinfo::ArtistInfoService);

#[async_trait]
impl ArtistMbid for InfoMbid {
    async fn artist_mbid(&self, artist_id: &str, name: &str) -> Option<String> {
        self.0.artist_mbid(artist_id, name).await
    }
}

pub struct LidarrDeps {
    pub web: Arc<dyn Web>,
    pub keys: Arc<dyn TokenStore>,
    pub db: DbHandle,
    pub plex: Arc<dyn PlexMusic>,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// MusicBrainz, paced and cached.
    pub mb: Arc<dyn mbpick::MbGet>,
    pub artist_mbid: Arc<dyn ArtistMbid>,
    /// How long to wait between looks for a newly added artist's albums.
    pub album_wait: Duration,
    /// State shared with the user's other devices.
    pub shared: shared::Shared,
    /// Plex's scan settings, partial scans and what it shows.
    pub plex_server: Arc<dyn plexside::PlexServer>,
}

/// A check that passed or not, with the connection it used.
pub struct Checked {
    pub report: CheckReport,
    pub client: LidarrClient,
    pub writer: write::LidarrWriter,
    pub snap: LidarrSnapshot,
    /// Plex's music folders at the check.
    pub plex_locations: Vec<String>,
    /// Every artist's mode at the check.
    pub modes: Vec<modes::ArtistMode>,
}

/// Whether any file's tags (as the scan cached them) mark it as not
/// released: its release status (ARCHIVE_VERSION, else the status the
/// provenance reader found) is anything but released, on a file or an album.
pub fn library_has_unreleased(c: &rusqlite::Connection) -> AppResult<bool> {
    let q = "SELECT EXISTS (SELECT 1 FROM file_tags WHERE trim(coalesce(version, release, '')) NOT IN ('', 'released'))
             OR EXISTS (SELECT 1 FROM album_tags WHERE trim(coalesce(version, release, '')) NOT IN ('', 'released'))";
    Ok(c.query_row(q, [], |r| r.get::<_, bool>(0))?)
}

/// The last check's outcome, kept on this device.
const LAST_CHECK_KEY: &str = "lidarr.lastCheck";
/// What the last check is kept under on a device with no Lidarr address.
const HOME_SIDE_KEY: &str = "homeside";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastCheck {
    pub passed: bool,
    /// Nothing stops Managed artists (older checks: same as `passed`).
    #[serde(default)]
    pub managed_ok: Option<bool>,
    pub checked_at: i64,
    /// The address it was for (a changed address needs a new check).
    pub url: String,
    /// Lidarr's root folders then, for where queued requests will land.
    pub roots: Vec<String>,
}

/// What Settings shows before a check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LidarrStatus {
    /// A home address is set in Settings.
    pub url_set: bool,
    /// An away address is set.
    pub away_set: bool,
    /// An API key is in the keychain.
    pub has_key: bool,
    /// Extra proxy headers are in the keychain (their names, never values).
    pub header_names: Vec<String>,
    /// Addresses that would send the key unencrypted over the internet.
    pub unencrypted: Vec<String>,
    /// An address came from another device with a host the key wasn't
    /// entered for, so the key isn't sent there: the words to show.
    pub moved: Option<String>,
    /// Set up, with a key, and the last setup check for this address passed:
    /// Request buttons show (each request still checks again first). Or,
    /// with nothing set up here, a home side answers and its check passed.
    pub ready: bool,
    /// One of the user's computers at home, or a Bridge, answers Lidarr
    /// requests for this device now (no address or key needed here).
    pub home_side: bool,
    /// Signed in as someone who doesn't own the server: Lidarr isn't used
    /// from here at all.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub not_owner: bool,
}

impl LidarrStatus {
    /// Lidarr can be asked from this device: set up here, or through a
    /// home side.
    pub fn usable(&self) -> bool {
        !self.not_owner && (((self.url_set || self.away_set) && self.has_key) || self.home_side)
    }
}

/// When nothing answers for Lidarr away from home.
pub const HOME_SIDE_OFF: &str = "Lidarr isn't reachable away from home. Turn on a computer at home or add the Bridge.";
/// Signed in as someone who doesn't own the server: Lidarr isn't asked,
/// here or through a Bridge or a computer at home.
pub const NOT_OWNER: &str = "Only the owner of this Plex server can get music through Lidarr.";
/// A Bridge is there but isn't answering (it lost its link, say).
pub const BRIDGE_QUIET: &str = "Your Bridge isn't answering, so Lidarr can't be reached away from home yet. Platter is linking to it again by itself. If this is still here in a few minutes, check that the Bridge is running on your server.";

/// Worked-out modes and when (unix seconds).
type ModesAt = (i64, Vec<modes::ArtistMode>);

#[derive(Clone)]
pub struct Lidarr {
    inner: Arc<LidarrDeps>,
    /// One request (or retry) at a time.
    lock: Arc<tokio::sync::Mutex<()>>,
    /// The last modes worked out, with when (unix seconds).
    modes_cache: Arc<parking_lot::Mutex<Option<ModesAt>>>,
    /// Changes shown and waiting for a yes.
    proposals: Arc<changes::Proposals>,
    /// Tells the user about new music (set by the app).
    announcer: Arc<parking_lot::RwLock<Option<Arc<dyn catchup::Announcer>>>>,
    /// Where files are reached for adding albums to the library (set by the app).
    hosts: Arc<parking_lot::RwLock<Option<Arc<dyn upgrade::Hosts>>>>,
    /// Platter Bridge: bridge mode, and the route through a paired Bridge.
    home: Arc<homeside::Hooks>,
}

/// The address as a key for stored confirmations (and, from the home
/// address, for the shared store).
pub fn url_key(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// A plain http address to a host outside the home network.
pub fn unencrypted(url: &str) -> bool {
    url::Url::parse(url.trim())
        .ok()
        .is_some_and(|u| u.scheme() == "http" && u.host_str().is_some_and(|h| !paths::is_private_host(h)))
}

/// How long the home address gets when an away address is waiting.
const HOME_TRY: Duration = Duration::from_secs(4);
/// Most extra headers.
pub const MAX_HEADERS: usize = 5;
/// Headers Platter sets itself or that would break the request.
const RESERVED_HEADERS: [&str; 8] =
    ["x-api-key", "host", "content-length", "content-type", "transfer-encoding", "connection", "user-agent", "accept-encoding"];

/// `Name: value` lines (blank lines skipped) into checked header pairs.
pub fn parse_headers(text: &str) -> AppResult<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let bad = || AppError::Invalid("Write each header on its own line as Name: value.".into());
        let (name, value) = line.split_once(':').ok_or_else(bad)?;
        let (name, value) = (name.trim(), value.trim());
        let name_ok = !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        let value_ok = !value.is_empty() && value.len() <= 4096 && value.chars().all(|c| c == '\t' || (' '..='~').contains(&c));
        if !name_ok || !value_ok {
            return Err(bad());
        }
        if RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(AppError::Invalid(format!("Platter sets {name} itself; leave it out.")));
        }
        if out.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return Err(AppError::Invalid(format!("{name} is there twice.")));
        }
        out.push((name.to_string(), value.to_string()));
    }
    if out.len() > MAX_HEADERS {
        return Err(AppError::Invalid(format!("Up to {MAX_HEADERS} extra headers.")));
    }
    Ok(out)
}

impl Lidarr {
    pub fn new(deps: LidarrDeps) -> Self {
        Self { inner: Arc::new(deps), lock: Arc::new(tokio::sync::Mutex::new(())), modes_cache: Default::default(), proposals: Default::default(), announcer: Default::default(), hosts: Default::default(), home: Default::default() }
    }

    /// (home, away) addresses from Settings; empty when not set.
    fn urls(&self) -> (String, String) {
        let s = crate::settings::load(&self.inner.db);
        (s.lidarr_url, s.lidarr_away_url)
    }

    fn headers(&self) -> Vec<(String, String)> {
        self.inner
            .keys
            .get(HEADERS_ACCOUNT)
            .ok()
            .flatten()
            .and_then(|t| serde_json::from_str(t.expose()).ok())
            .unwrap_or_default()
    }

    pub fn status(&self) -> LidarrStatus {
        let (home, away) = self.urls();
        let has_key = self.inner.keys.get(KEY_ACCOUNT).ok().flatten().is_some();
        let set = !home.trim().is_empty() || !away.trim().is_empty();
        let moved = if has_key { self.moved(&home, &away) } else { None };
        let home_side = self.home_side_up();
        let owner = self.is_owner();
        let direct = owner && set && has_key && moved.is_none();
        LidarrStatus {
            url_set: !home.trim().is_empty(),
            away_set: !away.trim().is_empty(),
            has_key,
            ready: (direct || home_side) && self.last_check().is_some_and(|l| l.managed_ok.unwrap_or(l.passed)),
            home_side,
            header_names: self.headers().into_iter().map(|(n, _)| n).collect(),
            unencrypted: [home, away].into_iter().filter(|u| unencrypted(u)).collect(),
            moved,
            not_owner: !owner,
        }
    }

    /// Save (or with None remove) the API key in the keychain.
    pub fn set_key(&self, key: Option<&str>) -> AppResult<LidarrStatus> {
        match key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => {
                if k.len() > 200 || !k.chars().all(|c| c.is_ascii_alphanumeric()) {
                    return Err(AppError::Invalid("A Lidarr API key is letters and numbers, from Lidarr's Settings > General.".into()));
                }
                self.inner.keys.set(KEY_ACCOUNT, &Token::new(k)?)?;
                // Entered for the addresses set now: only their hosts get it.
                let (home, away) = self.urls();
                crate::keyhost::bind(&self.inner.db, KEY_ACCOUNT, &[&home, &away])?;
            }
            None => self.inner.keys.delete(KEY_ACCOUNT)?,
        }
        Ok(self.status())
    }

    /// Save (or with None or blank text remove) extra headers for the away
    /// address, as `Name: value` lines. Kept in the keychain.
    pub fn set_headers(&self, text: Option<&str>) -> AppResult<LidarrStatus> {
        let list = parse_headers(text.unwrap_or_default())?;
        if list.is_empty() {
            self.inner.keys.delete(HEADERS_ACCOUNT)?;
        } else {
            let json = serde_json::to_string(&list).map_err(|e| AppError::Internal(e.to_string()))?;
            self.inner.keys.set(HEADERS_ACCOUNT, &Token::new(json)?)?;
        }
        Ok(self.status())
    }

    fn confirmations(&self) -> HashMap<String, Vec<String>> {
        self.inner
            .db
            .read(|c| kv_get(c, CONFIRMED_KEY))
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Remember (or forget) that an empty root folder is separate from the
    /// Plex music folder. Checked again on every check. Kept per Lidarr
    /// (its home address).
    pub fn confirm_root(&self, path: &str, confirmed: bool) -> AppResult<()> {
        let (home, _) = self.urls();
        if home.trim().is_empty() {
            return Err(AppError::Invalid("Set Lidarr's address first.".into()));
        }
        let key = NormPath::parse(path).key();
        if key.is_empty() {
            return Err(AppError::Invalid("That isn't a folder.".into()));
        }
        let mut all = self.confirmations();
        let list = all.entry(url_key(&home)).or_default();
        list.retain(|k| *k != key);
        if confirmed {
            list.push(key);
        }
        let json = serde_json::to_string(&all).map_err(|e| AppError::Internal(e.to_string()))?;
        self.inner.db.write(|c| kv_set(c, CONFIRMED_KEY, &json))
    }

    /// Whether the key may go to this address (it was entered for its host).
    fn key_goes_to(&self, url: &str, home: &str, away: &str) -> bool {
        crate::keyhost::trusted(&self.inner.db, KEY_ACCOUNT, url, &[home, away])
    }

    /// Words for an address set here whose host the key wasn't entered for.
    fn moved(&self, home: &str, away: &str) -> Option<String> {
        [home, away]
            .into_iter()
            .find(|u| !u.trim().is_empty() && !self.key_goes_to(u, home, away))
            .map(|u| crate::keyhost::moved_words("Lidarr", u))
    }

    /// The routes to try: home first, then away, then one of the user's
    /// computers at home that answers for this device, then a paired
    /// Bridge. An address whose host the key wasn't entered for (one that
    /// came from another device) is left out: the key never goes there.
    /// With no address or key here, only the home sides (they hold the key).
    fn routes(&self) -> AppResult<Vec<Arc<dyn LidarrRoute>>> {
        if !self.is_owner() {
            return Err(AppError::Invalid(NOT_OWNER.into()));
        }
        let (home_set, away_set) = self.urls();
        let computer = self.computer_route();
        let bridge = self.bridge_route();
        let home_sides = computer.is_some() || bridge.is_some();
        let nothing_here = home_set.trim().is_empty() && away_set.trim().is_empty();
        if nothing_here && !home_sides {
            return Err(AppError::Invalid("Set Lidarr's address first.".into()));
        }
        let keep = |u: &str| if !u.trim().is_empty() && self.key_goes_to(u, &home_set, &away_set) { u.to_string() } else { String::new() };
        let (home, away) = (keep(&home_set), keep(&away_set));
        let key = self.inner.keys.get(KEY_ACCOUNT)?;
        if key.is_none() && !home_sides {
            return Err(AppError::Invalid("Add Lidarr's API key first.".into()));
        }
        let mut out: Vec<Arc<dyn LidarrRoute>> = Vec::new();
        if let Some(key) = key {
            if !home.trim().is_empty() {
                let mut r = DirectRoute::new(self.inner.web.clone(), &home, key.clone(), vec![], Route::Home)?;
                if !away.trim().is_empty() || home_sides {
                    r = r.with_timeout(HOME_TRY);
                }
                out.push(Arc::new(r));
            }
            if !away.trim().is_empty() {
                let headers = self.headers().into_iter().filter_map(|(n, v)| Token::new(v).ok().map(|t| (n, t))).collect();
                out.push(Arc::new(DirectRoute::new(self.inner.web.clone(), &away, key, headers, Route::Away)?));
            }
        }
        out.extend(computer);
        out.extend(bridge);
        if out.is_empty() {
            return Err(AppError::Invalid(self.moved(&home_set, &away_set).unwrap_or_else(|| "Set Lidarr's address first.".into())));
        }
        Ok(out)
    }

    /// The first route that answers like a Lidarr Platter can check. When
    /// none does, the home address's error unless it simply wasn't reachable.
    pub async fn connect(routes: Vec<Arc<dyn LidarrRoute>>) -> AppResult<(LidarrClient, SystemStatus, Arc<dyn LidarrRoute>)> {
        let mut first_err: Option<AppError> = None;
        for t in routes {
            let c = LidarrClient::new(t.clone() as Arc<dyn LidarrGet>);
            match c.system_status().await {
                Ok(s) => return Ok((c, s, t)),
                Err(e) => {
                    let keep = match &first_err {
                        None => true,
                        Some(prev) => prev.kind() == "network" && e.kind() != "network",
                    };
                    if keep {
                        first_err = Some(e);
                    }
                }
            }
        }
        Err(first_err.unwrap_or_else(|| AppError::Network(api::UNREACHABLE.into())))
    }

    /// Read everything the check needs from Lidarr (GET only).
    pub async fn snapshot(client: &LidarrClient, status: SystemStatus) -> AppResult<LidarrSnapshot> {
        if status.app_name.as_deref().is_some_and(|n| !n.eq_ignore_ascii_case("lidarr")) {
            return Err(AppError::Invalid(api::NOT_LIDARR.into()));
        }
        match parse_version(&status.version) {
            Some(v) if v >= MIN_VERSION => {}
            _ => {
                let shown = if status.version.trim().is_empty() { "an unknown version".to_string() } else { format!("version {}", status.version.trim()) };
                return Err(AppError::Invalid(format!(
                    "This Lidarr is {shown}. Platter needs Lidarr {}.{} or later. Update Lidarr, then check again.",
                    MIN_VERSION.0, MIN_VERSION.1
                )));
            }
        }
        let (roots, artists, naming, metadata, media, profiles, lists) = futures::try_join!(
            client.root_folders(),
            client.artists(),
            client.naming(),
            client.metadata_provider(),
            client.media_management(),
            client.quality_profiles(),
            client.import_lists(),
        )?;
        // Newer reads: an older Lidarr without them counts as having none.
        let (exclusions, unmapped, notifications) = futures::join!(client.exclusions(), client.unmapped_files(), client.notifications());
        Ok(LidarrSnapshot {
            notifications: notifications.unwrap_or_default(),
            exclusions: exclusions.unwrap_or_default(),
            unmapped: unmapped.unwrap_or_default(),
            version: status.version.trim().to_string(),
            roots,
            artists,
            naming,
            metadata,
            media,
            profiles,
            lists,
            listings: HashMap::new(),
            album_folders: Vec::new(),
        })
    }

    /// The whole check: reach Lidarr (home address first, then away), read
    /// it and Plex, then follow up with folder listings until the
    /// comparisons are settled (or the budget runs out).
    pub async fn check(&self) -> AppResult<CheckReport> {
        Ok(self.check_full().await?.report)
    }

    /// The last check's outcome on this device, when it was for the
    /// address set now.
    pub fn last_check(&self) -> Option<LastCheck> {
        let url = self.check_key();
        self.inner
            .db
            .read(|c| kv_get(c, LAST_CHECK_KEY))
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<LastCheck>(&s).ok())
            .filter(|l| l.url == url && !url.is_empty())
    }

    /// What the last check is kept under: the address set here, or the
    /// home side when none is (a phone that reaches Lidarr only that way).
    fn check_key(&self) -> String {
        let (home, away) = self.urls();
        let url = url_key(if home.trim().is_empty() { &away } else { &home });
        if url.is_empty() { HOME_SIDE_KEY.to_string() } else { url }
    }

    fn remember_check(&self, report: &CheckReport, snap: &LidarrSnapshot) {
        let last = LastCheck {
            passed: report.passed,
            managed_ok: Some(report.managed_ok),
            checked_at: report.checked_at,
            url: self.check_key(),
            roots: snap.roots.iter().map(|r| r.path.clone()).collect(),
        };
        if let Ok(json) = serde_json::to_string(&last) {
            let _ = self.inner.db.write(|c| kv_set(c, LAST_CHECK_KEY, &json));
        }
    }

    /// [`Self::check`], keeping the connection for a request.
    pub async fn check_full(&self) -> AppResult<Checked> {
        let routes = self.routes()?;
        let (home, away) = self.urls();
        let connect = async {
            let (client, status, route) = Self::connect(routes).await?;
            let snap = Self::snapshot(&client, status).await?;
            Ok::<_, AppError>((client, snap, route))
        };
        let ((client, mut snap, route), plex) = futures::try_join!(connect, self.inner.plex.music_files())?;
        let index = PlexIndex::new(&plex);
        let modes = self.modes_with(&client, &snap).await?;
        let protected_ids: Vec<i64> = modes.iter().filter(|m| m.protected()).filter_map(|m| m.lidarr_id).collect();
        snap.album_folders = self.album_folders(&client, &snap, &protected_ids).await;
        let share = crate::musicshare::load(&self.inner.db);
        let prefs = self.inner.plex_server.prefs().await.ok();
        let ctx = CheckContext {
            protected: modes.iter().filter(|m| m.protected()).map(check::ProtectedRef::from).collect(),
            share: share.music.iter().map(|p| (p.server.clone(), p.local.clone())).collect(),
            plex_auto_scan: prefs.and_then(|p| match (p.auto_scan, p.periodic_scan) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (None, None) => None,
                _ => Some(false),
            }),
            confirmed_roots: self.confirmations().remove(&url_key(if home.trim().is_empty() { &away } else { &home })).unwrap_or_default(),
            unencrypted: [home, away].into_iter().filter(|u| unencrypted(u)).collect(),
            route: Some(client.route()),
            now: (self.inner.clock)(),
            staging: Some(crate::settings::load(&self.inner.db).lidarr_staging_folder).filter(|s| !s.trim().is_empty()),
            library_has_unreleased: self.inner.db.read(library_has_unreleased).unwrap_or(false),
        };
        let mut budget = MAX_LISTINGS;
        for _ in 0..MAX_ROUNDS {
            let (_, wanted) = evaluate(&index, &snap, &ctx);
            let wanted = dedup_wanted(wanted, &snap.listings);
            if wanted.is_empty() || budget == 0 {
                break;
            }
            let take: Vec<String> = wanted.into_iter().take(budget).collect();
            budget -= take.len();
            let got = futures::future::join_all(take.iter().map(|p| client.list_folder(p))).await;
            for (p, r) in take.into_iter().zip(got) {
                snap.listings.insert(p, r.ok());
            }
        }
        let (report, _) = evaluate(&index, &snap, &ctx);
        tracing::info!(
            red = report.findings.iter().filter(|f| f.level == check::Level::Red).count(),
            amber = report.findings.iter().filter(|f| f.level == check::Level::Amber).count(),
            listings = snap.listings.len(),
            route = ?client.route(),
            "lidarr setup check"
        );
        self.remember_check(&report, &snap);
        if report.managed_ok {
            self.start_practice();
        }
        let plex_locations = plex.locations.clone();
        Ok(Checked { report, client, writer: self.writer_for(route), snap, plex_locations, modes })
    }
}