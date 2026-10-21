//! The Bridge itself: Platter's home side without a window. Signed in to
//! Plex through plex.tv/link, it links by itself to every Platter on the
//! same account (`super::account`). Without that sign-in it pairs once,
//! either with the code from `BRIDGE_PAIR_CODE` (made on a computer) or by
//! showing a code of its own in its log while it isn't paired (entered on a
//! computer or phone). It takes how to reach Lidarr from its own settings
//! (its environment) when given them, else from a computer (sealed), and
//! then, every few minutes, catches up with Lidarr the way an open desktop
//! would. Shared Lidarr state goes back and forth over the link so it and
//! the desktops never act on the same event.
//!
//! Plex: never the account's own sign-in. The Bridge uses PLEX_TOKEN when
//! set, else a server's own token a computer sends (a server shared with
//! the user), else a sign-in of its own through plex.tv/link.
//!
//! With "Bring in my listening" handed over by a desktop, it also runs the
//! request waves (`crate::lidarr::waves_bridge`).
//!
//! It only ever opens outbound connections (the relay, Lidarr, Plex,
//! plex.tv, MusicBrainz). The health check listens on 127.0.0.1 inside the
//! container.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::{mpsc, watch};

use super::account::{self, AccountDevice, Kind, Me};
use super::code::PairCode;
use super::keys::FileKeys;
use super::link::Link;
use super::links::{Links, Outbox};
use super::pair::{Joining, Offer};
use super::peer::{self, Clock, Incoming, Wire};
use super::route;
use super::wire::{BridgeConfig, BridgeStatus, CatchUpLine, MusicAccess, Msg};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::lidarr::Lidarr;
use crate::lidarr::api::{DirectRoute, Route};
use crate::lidarr::waves::{Waves, WavesDeps};
use crate::lidarr::waves_bridge::PlanAlbums;
use crate::plex::PlexState;
use crate::plex::keychain::{ACCOUNT_TOKEN, TokenStore};
use crate::plex::token::Token;
use crate::remote::peer::auth::PeerKey;
use crate::social::listen::relay::{RelayEvent, RelayFactory};
use crate::sync::SyncStore;
use crate::sync::store::{LIDARR, TRANSCRIPTS};
use super::lyrics::{BridgeLyrics, Opts};

/// How often the Bridge says it's alive when nothing else happened.
pub const HEARTBEAT: Duration = Duration::from_secs(60 * 60);
/// How a Bridge's device id starts in the sync store.
pub const DEVICE_PREFIX: &str = "bridge-";
/// How often the library cache is brought up to date.
const LIBRARY_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Wait after a shared change before sending it (changes come in bursts).
const SETTLE: Duration = Duration::from_secs(3);
const USED_CODES_KEY: &str = "bridge.usedCodes";
const CONFIG_KEY: &str = "bridge.config";
const DEVICE_KEY: &str = "bridge.device";
const SERVER_KEY: &str = "bridge.serverKey";
/// "0" when the computers switched Lidarr from anywhere off.
const LIDARR_ON_KEY: &str = "bridge.lidarrOn";
/// Said while Lidarr from anywhere is off.
pub const LIDARR_OFF: &str = "Lidarr from anywhere is off for this Bridge (Settings > Bridge on your computer).";
/// In the keys file: a Plex server's own token (never the account's).
pub const PLEX_SERVER_TOKEN: &str = "bridge-plex-server-token";
/// Where that token works: `{"machine": .., "uris": [..]}`.
const PLEX_DIRECT_KEY: &str = "bridge.plexDirect";
/// The Plex server the computers use (machine id).
const PLEX_SERVER_KEY: &str = "bridge.plexServer";
/// The Plex sign-in in the keys file is the Bridge's own (plex.tv/link).
pub const PLEX_OWN_KEY: &str = "bridge.plexOwn";
/// An account sign-in an older desktop left here was given up.
pub const PLEX_MIGRATED_KEY: &str = "bridge.plexMigrated";
/// How long a plex.tv/link code is waited on before a new one.
const PLEX_LINK_WAIT: Duration = Duration::from_secs(10 * 60);

/// What came of BRIDGE_PAIR_CODE at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FromEnv {
    /// Not set.
    NoCode,
    /// Paired with it now.
    Paired,
    /// Used before: skipped.
    UsedUp,
    /// Never used and past its hour: skipped.
    Expired,
}

pub const USED_UP: &str = "Already paired. BRIDGE_PAIR_CODE is used up and can be removed from the container.";
const USED_UNPAIRED: &str = "BRIDGE_PAIR_CODE was used before and works only once. Remove it from the container, then make a new code in Platter or enter the code this log shows.";
const EXPIRED: &str = "Already paired. BRIDGE_PAIR_CODE has expired and can be removed from the container.";
const EXPIRED_UNPAIRED: &str = "BRIDGE_PAIR_CODE has expired. Make a new one in Platter (Settings > Bridge > Pair a Bridge), or remove it and enter the code this log shows.";

/// Settings from the container's environment.
#[derive(Clone, Default)]
pub struct Env {
    pub data: PathBuf,
    pub name: String,
    pub pair_code: Option<String>,
    pub relay: Option<String>,
    /// Lidarr's address and key, when set here instead of from the desktop.
    pub lidarr_url: Option<String>,
    pub lidarr_key: Option<String>,
    /// Plex's address and a token for that server, when set here.
    pub plex_url: Option<String>,
    pub plex_token: Option<String>,
    /// The music folder: how Plex sees it, and where it's mounted here.
    pub music: Option<(String, PathBuf)>,
    /// Where the music folder is mounted here (MUSIC_DIR, else /music),
    /// MUSIC_PLEX_PATH or not.
    pub music_dir: PathBuf,
    /// The staging folder: where it's mounted here (Lidarr's path for it
    /// comes from the desktop).
    pub staging: Option<PathBuf>,
    pub health_port: u16,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env").field("data", &self.data).field("name", &self.name).finish_non_exhaustive()
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub const DEFAULT_HEALTH_PORT: u16 = 8787;

impl Env {
    pub fn from_env() -> Self {
        let music_dir = PathBuf::from(var("MUSIC_DIR").unwrap_or_else(|| "/music".into()));
        let music = var("MUSIC_PLEX_PATH").map(|p| (p, music_dir.clone()));
        Self {
            plex_url: var("PLEX_URL"),
            plex_token: var("PLEX_TOKEN"),
            music_dir,
            data: PathBuf::from(var("BRIDGE_DATA").unwrap_or_else(|| "/data".into())),
            name: var("BRIDGE_NAME").unwrap_or_else(|| "Platter Bridge".into()),
            pair_code: var("BRIDGE_PAIR_CODE"),
            relay: var("BRIDGE_RELAY"),
            lidarr_url: var("LIDARR_URL"),
            lidarr_key: var("LIDARR_API_KEY"),
            music,
            staging: var("STAGING_DIR").map(PathBuf::from).filter(|p| p.is_dir()),
            health_port: var("BRIDGE_HEALTH_PORT").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_HEALTH_PORT),
        }
    }

    /// The settings set here, which win over what a computer sends.
    pub fn set_here(&self) -> Vec<String> {
        [
            ("lidarrUrl", self.lidarr_url.is_some()),
            ("lidarrKey", self.lidarr_key.is_some()),
            ("plexUrl", self.plex_url.is_some()),
            ("plexToken", self.plex_token.is_some()),
            ("musicPlexPath", self.music.is_some()),
        ]
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(k, _)| k.to_string())
        .collect()
    }
}

/// Take Lidarr's address and key from the Bridge's own settings, so it
/// works before any computer has shared them.
fn apply_env(p: &Parts) -> AppResult<()> {
    if let Some(url) = &p.env.lidarr_url {
        let mut s = crate::settings::try_load(&p.db)?;
        if s.lidarr_url != *url || !s.lidarr_away_url.is_empty() {
            s.lidarr_url = url.clone();
            s.lidarr_away_url = String::new();
            crate::settings::save(&p.db, &s)?;
        }
        let server = crate::lidarr::url_key(url);
        p.db.write(|c| kv_set(c, SERVER_KEY, &server))?;
        p.lidarr.set_server_key(Some(server));
    }
    if let Some(k) = &p.env.lidarr_key {
        // Bound to the address set now (above), as a key entered on a desktop is.
        p.lidarr.set_key(Some(k))?;
    }
    Ok(())
}

/// A server's own token among the account's servers, when Plex gives one
/// that isn't the account's sign-in: (machine id, token, addresses).
pub fn server_only_token(servers: &[crate::plex::discovery::DiscoveredServer], account: &Token, want: Option<&str>) -> Option<(String, Token, Vec<String>)> {
    let s = match want {
        Some(id) => servers.iter().find(|s| s.machine_id == id)?,
        None if servers.len() == 1 => &servers[0],
        None => return None,
    };
    let t = s.access_token.clone().filter(|t| t != account)?;
    Some((s.machine_id.clone(), t, s.connections.iter().map(|c| c.uri.clone()).collect()))
}

/// The first Platter that stopped sending the Plex account's sign-in.
pub const ACCOUNT_SIGN_IN_STOPPED: (u64, u64, u64) = (0, 2, 30);

/// The log line for a Plex account sign-in a computer sent, naming that
/// computer and its Platter version. A config without a version came from
/// a build before 0.2.31, and one with a sign-in from before 0.2.30.
pub fn account_sign_in_note(desktop: &str, version: &str) -> String {
    let from = if desktop.trim().is_empty() { "a computer".to_string() } else { desktop.trim().to_string() };
    let version = version.trim();
    match crate::lidarr::api::parse_version(version) {
        Some(v) if v >= ACCOUNT_SIGN_IN_STOPPED => {
            format!("Ignored a Plex account sign-in from Platter {version} on {from}. The Bridge never keeps one.")
        }
        Some(_) => format!("Ignored the Plex account sign-in Platter {version} on {from} sent, as Platter before 0.2.30 does. Update Platter on that computer."),
        None => format!("Ignored the Plex account sign-in {from} sent. It runs Platter older than 0.2.30, which doesn't say its version. Update Platter on that computer."),
    }
}

/// What MUSIC_PLEX_PATH should be: Plex's folder that the files mounted at
/// `mount` sit under, from some of the library's file paths as Plex gives
/// them. Each path is matched by the longest tail of it that's there under
/// the mount; the answer is the start most of them agree on.
pub fn suggest_plex_path(files: &[String], mount: &std::path::Path, exists: impl Fn(&std::path::Path) -> bool) -> Option<String> {
    let mut votes: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut tried = 0;
    for f in files.iter().filter(|f| !f.trim().is_empty()).take(40) {
        tried += 1;
        let sep = if f.contains('\\') && !f.contains('/') { '\\' } else { '/' };
        let parts: Vec<&str> = f.split(sep).collect();
        // Keep at least the file and one folder above it, so a lone file
        // name doesn't match by chance.
        for k in 1..parts.len().saturating_sub(1) {
            let tail: PathBuf = parts[k..].iter().filter(|p| !p.is_empty()).collect();
            if exists(&mount.join(&tail)) {
                let head = parts[..k].join(&sep.to_string());
                let head = if head.is_empty() { sep.to_string() } else { head };
                *votes.entry(head).or_default() += 1;
                break;
            }
        }
    }
    let (best, n) = votes.into_iter().max_by_key(|(_, n)| *n)?;
    (n * 2 >= tried).then_some(best)
}

/// Whether the music folder is there, and whether the Bridge may write.
pub fn music_access(env: &Env) -> MusicAccess {
    let Some((_, dir)) = &env.music else { return MusicAccess::None };
    if !dir.is_dir() {
        return MusicAccess::None;
    }
    let probe = dir.join(".platter-bridge-write-check");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            MusicAccess::ReadWrite
        }
        Err(_) => MusicAccess::ReadOnly,
    }
}

/// Everything the Bridge runs on (real ones from [`build`], fakes in tests).
pub struct Parts {
    pub env: Env,
    pub db: DbHandle,
    pub keys: Arc<dyn TokenStore>,
    pub relays: Arc<dyn RelayFactory>,
    pub clock: Clock,
    pub store: SyncStore,
    pub lidarr: Lidarr,
    pub web: Arc<dyn crate::web::Web>,
    /// None in tests.
    pub plex: Option<PlexState>,
    pub music: MusicAccess,
}

pub struct Bridge {
    pub p: Parts,
    pub wire: Arc<Wire>,
    status: Mutex<BridgeStatus>,
    /// Last time the main loop came round (unix s), for the health check.
    pub alive: AtomicI64,
    hello: Mutex<HashSet<String>>,
    library_now: tokio::sync::Notify,
    configured: AtomicBool,
    /// Bring in my listening, from the album list a desktop handed over.
    pub waves: Waves,
    /// The code the Bridge shows while it isn't paired.
    shown: Mutex<Option<String>>,
    /// Links paired from the Bridge's own code: say hello at their first
    /// message (the computer may not have been listening for the first one).
    greet: Mutex<HashSet<String>>,
    /// Welcomes waiting for their link's listener to open: (relay, topic, body).
    welcomes: Mutex<std::collections::HashMap<String, (String, String, String)>>,
    links_changed: tokio::sync::Notify,
    plex_now: tokio::sync::Notify,
    plex_linking: AtomicBool,
    /// Lyrics for every song (None: not set up, as in most tests).
    lyrics: parking_lot::RwLock<Option<Arc<BridgeLyrics>>>,
    /// Lidarr from anywhere.
    lidarr_on: AtomicBool,
    /// The lyrics line last sent, so a change goes out.
    lyrics_sent: Mutex<Option<super::wire::LyricsLine>>,
    /// Look at the account's devices now (just signed in).
    account_now: tokio::sync::Notify,
}

impl Bridge {
    pub fn new(p: Parts) -> Arc<Self> {
        p.lidarr.set_bridge_mode(true);
        p.lidarr.set_server_key(p.db.read(|c| kv_get(c, SERVER_KEY)).ok().flatten());
        let links = Links::new(p.db.clone(), p.keys.clone());
        let wire = Arc::new(Wire::new(links, p.relays.clone(), p.clock.clone()));
        let status = BridgeStatus {
            name: p.env.name.clone(),
            version: super::VERSION.into(),
            doing: "Starting".into(),
            music: p.music,
            from_env: p.env.set_here(),
            ..Default::default()
        };
        if let Err(e) = apply_env(&p) {
            tracing::warn!("Couldn't use the Lidarr settings set on the Bridge: {e}");
        }
        let configured = p.db.read(|c| kv_get(c, CONFIG_KEY)).ok().flatten().is_some() || p.env.lidarr_url.is_some();
        let lidarr_on = p.db.read(|c| kv_get(c, LIDARR_ON_KEY)).ok().flatten().as_deref() != Some("0");
        let day_db = p.db.clone();
        let waves = Waves::with_albums(
            WavesDeps {
                lidarr: p.lidarr.clone(),
                notifier: Arc::new(LogNotifier),
                day_of: Arc::new(move |at| crate::lidarr::waves::local_day(&day_db, at)),
                hidden: Arc::new(Vec::new),
                utc_offset: None,
            },
            Arc::new(PlanAlbums { db: p.db.clone() }),
        );
        Arc::new(Self {
            p,
            wire,
            status: Mutex::new(status),
            alive: AtomicI64::new(0),
            hello: Mutex::default(),
            library_now: tokio::sync::Notify::new(),
            configured: AtomicBool::new(configured),
            waves,
            shown: Mutex::default(),
            greet: Mutex::default(),
            welcomes: Mutex::default(),
            links_changed: tokio::sync::Notify::new(),
            plex_now: tokio::sync::Notify::new(),
            plex_linking: AtomicBool::new(false),
            lyrics: Default::default(),
            lidarr_on: AtomicBool::new(lidarr_on),
            lyrics_sent: Mutex::default(),
            account_now: tokio::sync::Notify::new(),
        })
    }

    /// Lyrics for every song, once its parts are built (the binary does it).
    pub fn set_lyrics(&self, l: Arc<BridgeLyrics>) {
        *self.lyrics.write() = Some(l);
    }

    pub fn lyrics(&self) -> Option<Arc<BridgeLyrics>> {
        self.lyrics.read().clone()
    }

    /// Lidarr from anywhere is on.
    pub fn lidarr_on(&self) -> bool {
        self.lidarr_on.load(Ordering::SeqCst)
    }

    /// The code the Bridge shows while it isn't paired.
    pub fn shown_code(&self) -> Option<String> {
        self.shown.lock().clone()
    }

    fn now_s(&self) -> i64 {
        ((self.p.clock)() / 1000) as i64
    }

    pub fn status(&self) -> BridgeStatus {
        let mut s = self.status.lock().clone();
        s.lyrics = self.lyrics().map(|l| l.line());
        s
    }

    fn doing(&self, what: &str) {
        let mut s = self.status.lock();
        if s.doing != what {
            tracing::info!("{what}");
            s.doing = what.to_string();
        }
    }

    fn hello_msg(&self) -> Msg {
        Msg::Hello { name: self.p.env.name.clone(), version: super::VERSION.into(), music: self.p.music, waves: true, device: self.plex_id() }
    }

    /// The Bridge's Plex client id.
    fn plex_id(&self) -> String {
        self.p.plex.as_ref().map(|p| p.identity().client_id.clone()).unwrap_or_default()
    }

    // ------------------------------------------------------------ by account

    /// Signed in to Plex as its own device (plex.tv/link): the sign-in that
    /// finds the account's devices.
    fn own_sign_in(&self) -> Option<Token> {
        self.p.db.read(|c| kv_get(c, PLEX_OWN_KEY)).ok().flatten()?;
        self.p.keys.get(ACCOUNT_TOKEN).ok().flatten()
    }

    /// Link to every computer and phone on the account the Bridge isn't
    /// linked to yet, and drop links to devices that left it. Gives the
    /// keys of the devices newly linked.
    pub fn account_links(&self, me: &Me, list: &[AccountDevice]) -> Vec<PeerKey> {
        let links = &self.wire.links;
        let Some(plan) = account::plan(Kind::Bridge, me, list, &links.all(), &links.apart(), super::DEFAULT_RELAY, |_| true) else { return Vec::new() };
        let changed = !plan.add.is_empty() || !plan.drop.is_empty();
        for id in &plan.drop {
            tracing::info!("A device left your Plex account; its link is gone.");
            let _ = links.remove(id);
        }
        let mut fresh = Vec::new();
        for w in &plan.add {
            match links.add_auto(w, self.now_s()) {
                Ok(()) => {
                    tracing::info!("Linked with {} on your Plex account.", if w.name.is_empty() { "a device" } else { &w.name });
                    fresh.push(w.key.clone());
                }
                Err(e) => tracing::warn!("Couldn't link with a device on your Plex account: {e}"),
            }
        }
        if changed {
            self.links_changed.notify_one();
        }
        fresh
    }

    // ------------------------------------------------------------ pairing

    fn used_codes(&self) -> Vec<String> {
        self.p.db.read(|c| kv_get(c, USED_CODES_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn mark_used(&self, code: &PairCode) {
        let mut l = self.used_codes();
        l.push(code.fingerprint());
        if let Ok(json) = serde_json::to_string(&l) {
            let _ = self.p.db.write(|c| kv_set(c, USED_CODES_KEY, &json));
        }
    }

    /// Pair with the code from the environment, unless it was used before
    /// or has expired (said once in the log, and skipped). A code is
    /// remembered as used by its fingerprint, never the code itself.
    pub async fn pair_from_env(&self) -> AppResult<FromEnv> {
        let Some(raw) = self.p.env.pair_code.clone() else { return Ok(FromEnv::NoCode) };
        let Some(code) = PairCode::parse(&raw) else {
            return Err(AppError::Invalid("BRIDGE_PAIR_CODE isn't a pair code. Copy it again from Platter (Settings > Bridge > Pair a Bridge).".into()));
        };
        let paired = !self.wire.links.all().is_empty();
        if self.used_codes().contains(&code.fingerprint()) {
            tracing::info!("{}", if paired { USED_UP } else { USED_UNPAIRED });
            return Ok(FromEnv::UsedUp);
        }
        if code.expired(self.now_s()) {
            tracing::info!("{}", if paired { EXPIRED } else { EXPIRED_UNPAIRED });
            return Ok(FromEnv::Expired);
        }
        self.pair(code).await.map(|_| FromEnv::Paired)
    }

    /// Pair with a code: publish the join, wait for the desktop's welcome.
    pub async fn pair(&self, code: PairCode) -> AppResult<()> {
        let relay_url = code.relay.clone().or_else(|| self.p.env.relay.clone()).unwrap_or_else(|| super::DEFAULT_RELAY.to_string());
        let relay = self.p.relays.relay(&relay_url);
        let expires = i64::from(code.expires);
        let (joining, (topic, body)) = Joining::start(code.clone(), &self.p.env.name, super::VERSION, (self.p.clock)())?;
        self.doing("Pairing: waiting for Platter on your computer to answer");
        let mut rx = relay.subscribe(&[joining.listen_topic()], None).await.map_err(|e| AppError::Network(peer::relay_error(e)))?;
        // Listening first, so the answer can't arrive before it.
        loop {
            match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
                Ok(Some(RelayEvent::Open)) => break,
                Ok(Some(_)) => {}
                _ => return Err(AppError::Network("The relay didn't answer.".into())),
            }
        }
        relay.publish(&topic, &body).await.map_err(|e| AppError::Network(peer::relay_error(e)))?;
        loop {
            let left = (expires - self.now_s()).max(0) as u64;
            let ev = tokio::time::timeout(Duration::from_secs(left.max(1)), rx.recv()).await;
            match ev {
                Ok(Some(RelayEvent::Message { body, .. })) if joining.is_welcome(&body, (self.p.clock)()) => {
                    let d = joining.finish(&body, (self.p.clock)())?;
                    self.wire.links.add(&d.link, &d.name, "", self.now_s())?;
                    self.mark_used(&code);
                    tracing::info!("Paired with {}.", d.name);
                    return Ok(());
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    // The relay dropped us: listen again.
                    rx = relay.subscribe(&[joining.listen_topic()], None).await.map_err(|e| AppError::Network(peer::relay_error(e)))?;
                }
                Err(_) => {
                    return Err(AppError::Invalid("The pair code expired before Platter answered. Make a new one and restart the Bridge with it.".into()));
                }
            }
        }
    }

    /// While nothing is paired, show a code of the Bridge's own in the log
    /// (and on the status page on 127.0.0.1) for a computer or phone to
    /// enter: a new one each hour. Ends when `stop` turns true.
    pub async fn show_code_while_unpaired(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        loop {
            if *stop.borrow() {
                return;
            }
            if !self.wire.links.all().is_empty() || self.own_sign_in().is_some() {
                *self.shown.lock() = None;
                tokio::select! {
                    _ = stop.changed() => return,
                    _ = tokio::time::sleep(Duration::from_secs(60)) => continue,
                }
            }
            let code = PairCode::new(self.p.env.relay.clone(), self.now_s());
            let text = code.bridge_text();
            *self.shown.lock() = Some(text.clone());
            tracing::info!(
                "Not paired yet. In Platter on your computer or phone, open Settings > Bridge > Add the Bridge and enter this code. It works once, for the next hour: {text}"
            );
            let out = self.wait_for_join(code, &mut stop).await;
            *self.shown.lock() = None;
            match out {
                Ok(()) => {}
                Err(e) => {
                    tracing::info!("Pairing: {e}");
                    tokio::select! {
                        _ = stop.changed() => return,
                        _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                    }
                }
            }
        }
    }

    /// Wait for a computer or phone to enter the code, then welcome it.
    /// Only the first good join is answered; the code is never used again.
    async fn wait_for_join(&self, code: PairCode, stop: &mut watch::Receiver<bool>) -> AppResult<()> {
        let relay_url = code.relay.clone().unwrap_or_else(|| super::DEFAULT_RELAY.to_string());
        let relay = self.p.relays.relay(&relay_url);
        let mut offer = Offer::from_bridge(code.clone(), &self.p.env.name);
        let topic = offer.listen_topic();
        loop {
            let mut rx = relay.subscribe(std::slice::from_ref(&topic), None).await.map_err(|e| AppError::Network(peer::relay_error(e)))?;
            loop {
                let left = (i64::from(code.expires) - self.now_s()).max(0) as u64;
                if left == 0 {
                    return Ok(());
                }
                tokio::select! {
                    _ = stop.changed() => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_secs(left)) => return Ok(()),
                    ev = rx.recv() => match ev {
                        Some(RelayEvent::Message { body, .. }) => {
                            if let Some(((wtopic, wbody), joined)) = offer.answer(&body, (self.p.clock)()) {
                                self.wire.links.add(&joined.link, &joined.name, &joined.version, self.now_s())?;
                                self.greet.lock().insert(joined.link.id.clone());
                                // Welcomed once this end listens on the link, so the
                                // computer's first message can't arrive before that.
                                self.welcomes.lock().insert(joined.link.id.clone(), (relay_url.clone(), wtopic, wbody));
                                self.links_changed.notify_one();
                                tracing::info!("Paired with {}.", joined.name);
                                return Ok(());
                            }
                        }
                        Some(RelayEvent::Open) => {}
                        None => break,
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    // ------------------------------------------------------------ messages

    /// Handle one message from a paired desktop.
    pub async fn handle(&self, link: &Link, msg: Msg) {
        if !matches!(msg, Msg::Unpair) && self.greet.lock().remove(&link.id) {
            let hello = self.hello_msg();
            let _ = self.wire.send(link, &hello).await;
        }
        match msg {
            Msg::Config(mut cfg) => {
                if !cfg.device.is_empty() && cfg.device.len() <= 100 {
                    let device = cfg.device.clone();
                    let _ = self.wire.links.update(&link.id, |p| {
                        if !p.auto {
                            p.device = Some(device);
                        }
                    });
                }
                if cfg.version.is_empty()
                    && let Some(p) = self.wire.links.get(&link.id)
                {
                    cfg.version = p.version;
                }
                if let Err(e) = self.apply_config(&cfg).await {
                    tracing::warn!("Couldn't use the settings from {}: {e}", cfg.desktop);
                }
                self.send_status_to(link).await;
            }
            Msg::Entries { entries } => {
                if let Err(e) = self.p.store.merge(&entries, &[LIDARR.name()]) {
                    tracing::warn!("Couldn't take shared Lidarr state: {e}");
                }
                let _ = Outbox::new(&self.p.db, &link.id).mark(&entries);
            }
            Msg::Get { id, .. } | Msg::Write { id, .. } if !self.lidarr_on() => {
                let _ = self.wire.send(link, &Msg::Failed { id, message: LIDARR_OFF.into() }).await;
            }
            Msg::Get { .. } | Msg::Write { .. } => {
                let home = self.home_route();
                let staging = crate::settings::load(&self.p.db).lidarr_staging_folder;
                if let Some(answer) = route::serve(home.as_ref(), &staging, &msg).await {
                    let _ = self.wire.send(link, &answer).await;
                }
            }
            Msg::Unpair => {
                tracing::info!("Unpaired by the desktop.");
                self.wire.links.unpaired(&link.id);
                if let Some(l) = self.lyrics() {
                    l.forget_link(&link.id).await;
                }
                let _ = self.wire.links.remove(&link.id);
            }
            Msg::LyricsHave { ids } => {
                if let Some(l) = self.lyrics() {
                    l.set_have(&link.id, ids).await;
                }
            }
            Msg::WavesPlan(plan) => {
                if let Err(e) = self.waves.take_plan(&plan).await {
                    tracing::warn!("Couldn't take Bring in my listening's list: {e}");
                }
                let progress = Msg::WavesProgress(Box::new(self.waves.progress()));
                let _ = self.wire.send(link, &progress).await;
                self.waves.poke();
            }
            Msg::Hello { .. } | Msg::Status(_) | Msg::Answer { .. } | Msg::Failed { .. } | Msg::WavesProgress(_) => {}
            // Device link messages and kinds from a newer build.
            Msg::SyncHello(_) | Msg::SyncDiff { .. } | Msg::SyncWant { .. } | Msg::SyncEntries { .. } | Msg::SyncCopyWant { .. } | Msg::SyncCopy { .. } | Msg::SyncBye | Msg::Unknown => {}
        }
    }

    /// Lidarr's home address, for requests from desktops away from home.
    fn home_route(&self) -> Option<DirectRoute> {
        let url = crate::settings::load(&self.p.db).lidarr_url;
        if !crate::keyhost::trusted(&self.p.db, crate::lidarr::KEY_ACCOUNT, &url, &[&url]) {
            return None;
        }
        let key = self.p.keys.get(crate::lidarr::KEY_ACCOUNT).ok().flatten()?;
        DirectRoute::new(self.p.web.clone(), &url, key, vec![], Route::Home).ok()
    }

    /// Take what the desktop sent: Lidarr's address and key (unless set in
    /// the environment), the Plex server, and the Lidarr settings. The
    /// Bridge's own settings always win.
    pub async fn apply_config(&self, cfg: &BridgeConfig) -> AppResult<()> {
        let mut s = crate::settings::try_load(&self.p.db)?;
        // A computer or phone without Lidarr set up (every device links now)
        // leaves the Bridge's Lidarr as it is.
        let has_lidarr = self.p.env.lidarr_url.is_some() || !cfg.lidarr_url.trim().is_empty();
        if has_lidarr {
            s.lidarr_url = self.p.env.lidarr_url.clone().unwrap_or_else(|| cfg.lidarr_url.clone());
            s.lidarr_away_url = String::new();
            s.lidarr_staging_folder = cfg.staging_folder.clone();
            s.lidarr_scan_after_import = cfg.scan_after_import;
            s.lidarr_free_staging = cfg.free_staging;
            s.lidarr_practice_until = cfg.practice_until;
            s.upgrades_ask = cfg.upgrades_ask;
            crate::settings::save(&self.p.db, &s)?;
            let server = crate::lidarr::url_key(&s.lidarr_url);
            self.p.db.write(|c| kv_set(c, SERVER_KEY, &server))?;
            self.p.lidarr.set_server_key(Some(server));
        }
        // A computer before the switches sends neither: Lidarr stays on, lyrics stay as they are.
        // Lidarr's switch comes only from a device with Lidarr set up.
        let lidarr_on = if has_lidarr { cfg.lidarr.unwrap_or(true) } else { self.lidarr_on.load(Ordering::SeqCst) };
        if self.lidarr_on.swap(lidarr_on, Ordering::SeqCst) != lidarr_on {
            tracing::info!("Lidarr from anywhere: {}.", if lidarr_on { "on" } else { "off" });
            self.p.db.write(|c| kv_set(c, LIDARR_ON_KEY, if lidarr_on { "1" } else { "0" }))?;
        }
        if let (Some(on), Some(l)) = (cfg.lyrics, self.lyrics()) {
            l.set_opts(Opts { on, hours: cfg.lyrics_hours, utc_offset_s: cfg.utc_offset_s.clamp(-14 * 3600, 14 * 3600), lrclib: cfg.lrclib }).await;
        }
        let key = self.p.env.lidarr_key.clone().or_else(|| cfg.lidarr_key.clone().filter(|_| has_lidarr));
        if let Some(k) = key {
            self.p.lidarr.set_key(Some(&k))?;
        }
        // The music share: where the Bridge reaches Plex's music folder and
        // Lidarr's staging folder.
        if let Some((server, local)) = &self.p.env.music {
            let map = crate::musicshare::ShareMap {
                music: vec![crate::musicshare::FolderPair { server: server.clone(), local: local.to_string_lossy().into() }],
                staging: self.p.env.staging.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                backup: String::new(),
            };
            let _ = crate::musicshare::save(&self.p.db, &map);
        }
        let first = !self.configured.swap(true, Ordering::SeqCst);
        // Nothing secret in what's kept here (the key and token are in the keys file).
        let kept = serde_json::json!({ "desktop": cfg.desktop, "at": self.now_s() }).to_string();
        self.p.db.write(|c| kv_set(c, CONFIG_KEY, &kept))?;
        if cfg.plex_token.is_some() {
            // Platter before 0.2.30 sends the account's own sign-in: never kept.
            tracing::info!("{}", account_sign_in_note(&cfg.desktop, &cfg.version));
        }
        let mut plex_changed = first;
        if let Some(id) = cfg.plex_server.as_deref().filter(|id| !id.is_empty() && id.len() <= 100) {
            let before = self.p.db.read(|c| kv_get(c, PLEX_SERVER_KEY)).ok().flatten();
            if before.as_deref() != Some(id) {
                self.p.db.write(|c| kv_set(c, PLEX_SERVER_KEY, id))?;
                plex_changed = true;
            }
        }
        // A server's own token, unless the Bridge has Plex settings of its own.
        if self.p.env.plex_token.is_none()
            && let Some(token) = cfg.plex_server_token.as_deref()
        {
            let token = Token::new(token)?;
            let direct = serde_json::json!({ "machine": cfg.plex_server, "uris": cfg.plex_connections.iter().take(20).collect::<Vec<_>>() }).to_string();
            let same = self.p.keys.get(PLEX_SERVER_TOKEN).ok().flatten().is_some_and(|t| t == token)
                && self.p.db.read(|c| kv_get(c, PLEX_DIRECT_KEY)).ok().flatten().as_deref() == Some(direct.as_str());
            if !same {
                self.p.keys.set(PLEX_SERVER_TOKEN, &token)?;
                self.p.db.write(|c| kv_set(c, PLEX_DIRECT_KEY, &direct))?;
                plex_changed = true;
            }
        }
        if plex_changed {
            self.plex_now.notify_one();
        }
        tracing::info!("Using the settings from {}.", if cfg.desktop.is_empty() { "your desktop" } else { &cfg.desktop });
        Ok(())
    }

    // ------------------------------------------------------------ plex

    /// Once: give up a Plex account sign-in an older desktop left here.
    /// When the account's server has a token of its own (a shared server),
    /// that's kept instead; otherwise the Bridge signs in on its own.
    pub async fn migrate_plex(&self) {
        let kv = |k: &str| self.p.db.read(|c| kv_get(c, k)).ok().flatten();
        if kv(PLEX_MIGRATED_KEY).is_some() {
            return;
        }
        if kv(PLEX_OWN_KEY).is_none()
            && let Some(account) = self.p.keys.get(ACCOUNT_TOKEN).ok().flatten()
        {
            if let Some(plex) = &self.p.plex
                && self.p.keys.get(PLEX_SERVER_TOKEN).ok().flatten().is_none()
                && let Ok(res) = plex.plex_tv().resources(&account).await
            {
                let servers = crate::plex::discovery::servers_from_resources(res);
                if let Some((machine, token, uris)) = server_only_token(&servers, &account, kv(PLEX_SERVER_KEY).as_deref()) {
                    let direct = serde_json::json!({ "machine": machine, "uris": uris }).to_string();
                    if self.p.keys.set(PLEX_SERVER_TOKEN, &token).is_ok() {
                        let _ = self.p.db.write(|c| kv_set(c, PLEX_DIRECT_KEY, &direct));
                    }
                }
            }
            let gone = self.p.keys.delete(ACCOUNT_TOKEN).is_ok() & self.p.keys.delete(crate::plex::keychain::SERVER_TOKEN).is_ok();
            if !gone {
                // Tried again at the next start, until the old sign-in is really gone.
                tracing::warn!("The Bridge couldn't remove your Plex account's old sign-in from its keys file. It tries again when it next starts. Check that /data is writable.");
                return;
            }
            tracing::info!("The Bridge no longer keeps your Plex account's sign-in. It uses a sign-in of its own (or PLEX_TOKEN) instead.");
        }
        let _ = self.p.db.write(|c| kv_set(c, PLEX_MIGRATED_KEY, "1"));
    }

    /// Connect to Plex the best way the Bridge has: its own settings, a
    /// server's own token from a computer, its own sign-in. With none of
    /// those, it asks for one through plex.tv/link.
    pub async fn connect_plex(self: &Arc<Self>) {
        let Some(plex) = self.p.plex.clone() else { return };
        let kv = |k: &str| self.p.db.read(|c| kv_get(c, k)).ok().flatten();
        let want = kv(PLEX_SERVER_KEY);
        let env_url: Vec<String> = self.p.env.plex_url.iter().cloned().collect();
        let result = if let Some(t) = &self.p.env.plex_token {
            if env_url.is_empty() {
                Err(AppError::Invalid("PLEX_TOKEN is set without PLEX_URL. Set both, or neither.".into()))
            } else {
                match Token::new(t.as_str()) {
                    Ok(t) => plex.connect_direct(want.as_deref(), &env_url, t).await.map(|_| ()),
                    Err(e) => Err(e),
                }
            }
        } else if let Some(t) = self.p.keys.get(PLEX_SERVER_TOKEN).ok().flatten() {
            let direct: serde_json::Value = kv(PLEX_DIRECT_KEY).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            let machine = direct["machine"].as_str().map(str::to_string).or(want.clone());
            let mut uris = env_url.clone();
            uris.extend(direct["uris"].as_array().into_iter().flatten().filter_map(|u| u.as_str().map(str::to_string)));
            plex.connect_direct(machine.as_deref(), &uris, t).await.map(|_| ())
        } else if kv(PLEX_OWN_KEY).is_some()
            && let Some(t) = self.p.keys.get(ACCOUNT_TOKEN).ok().flatten()
        {
            if env_url.is_empty() {
                match plex.restore().await {
                    Ok(_) => match &want {
                        Some(id) if plex.machine_id().await.as_deref() != Some(id.as_str()) => plex.select_server(id).await,
                        _ if plex.machine_id().await.is_none() => Err(AppError::Invalid("Your Plex account has more than one server. Open Platter on a computer once so the Bridge knows which.".into())),
                        _ => Ok(()),
                    },
                    Err(e) => Err(e),
                }
            } else {
                plex.connect_direct(want.as_deref(), &env_url, t).await.map(|_| ())
            }
        } else {
            self.ask_plex_link(plex);
            return;
        };
        match result {
            Ok(()) => {
                self.status.lock().plex = true;
                tracing::info!("Connected to Plex.");
                self.library_now.notify_one();
            }
            Err(e) => {
                self.status.lock().plex = false;
                tracing::warn!("Couldn't connect to Plex: {e}");
            }
        }
    }

    /// Sign in to Plex as the Bridge's own device: a short code for
    /// plex.tv/link in the log, on the status page and in Settings on the
    /// computers, a new one until it's used.
    fn ask_plex_link(self: &Arc<Self>, plex: PlexState) {
        if self.plex_linking.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let (_keep, cancel) = watch::channel(false);
            loop {
                let pin = match plex.plex_tv().create_link_pin().await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::info!("Couldn't ask plex.tv for a sign-in code: {e}");
                        tokio::time::sleep(Duration::from_secs(120)).await;
                        continue;
                    }
                };
                tracing::info!("To let the Bridge read your Plex library and find your devices, open https://plex.tv/link and enter {}.", pin.code);
                me.status.lock().plex_link = Some(pin.code.clone());
                me.send_status().await;
                match plex.plex_tv().poll_pin(&pin.id, Duration::from_secs(5), PLEX_LINK_WAIT, cancel.clone()).await {
                    Ok(token) => {
                        me.status.lock().plex_link = None;
                        if let Err(e) = me.p.keys.set(ACCOUNT_TOKEN, &token) {
                            tracing::warn!("Couldn't keep the Bridge's Plex sign-in: {e}");
                            continue;
                        }
                        let _ = me.p.db.write(|c| kv_set(c, PLEX_OWN_KEY, "1"));
                        tracing::info!("Signed in to Plex. The Bridge shows in Plex's Authorized Devices as Platter, where you can sign it out. Every Platter on your Plex account now finds it by itself.");
                        me.plex_linking.store(false, Ordering::SeqCst);
                        me.plex_now.notify_one();
                        me.account_now.notify_one();
                        return;
                    }
                    Err(e) => tracing::debug!("plex.tv/link: {e}"),
                }
            }
        });
    }

    /// Compare the music folder mounted here with Plex's own paths and say
    /// what MUSIC_PLEX_PATH should be when it isn't that already.
    fn check_music_mount(&self) {
        let dir = self.p.env.music_dir.clone();
        if dir.as_os_str().is_empty() || !dir.is_dir() {
            return;
        }
        let files: Vec<String> = self
            .p
            .db
            .read(|c| {
                let mut st = c.prepare("SELECT file FROM tracks WHERE file IS NOT NULL AND file != '' ORDER BY random() LIMIT 40")?;
                let rows = st.query_map([], |r| r.get::<_, String>(0))?;
                Ok(rows.filter_map(Result::ok).collect())
            })
            .unwrap_or_default();
        let Some(path) = suggest_plex_path(&files, &dir, |p| p.exists()) else { return };
        // Lyrics only read, so they go by the match until MUSIC_PLEX_PATH is set.
        if self.p.env.music.is_none()
            && let Some(l) = self.lyrics()
        {
            l.suggest_root(&path);
        }
        let set = self.p.env.music.as_ref().map(|(p, _)| p.trim_end_matches(['/', '\\']).to_string());
        if set.as_deref() == Some(path.trim_end_matches(['/', '\\'])) {
            self.status.lock().music_plex_path = None;
            return;
        }
        if self.status.lock().music_plex_path.as_deref() != Some(path.as_str()) {
            tracing::info!("The music folder mounted at {} looks like Plex's {path}. Set MUSIC_PLEX_PATH={path} on the Bridge.", dir.display());
        }
        self.status.lock().music_plex_path = Some(path);
    }

    /// Send shared Lidarr state the desktops don't have yet.
    pub async fn flush_entries(&self) {
        for link in self.wire.links.links() {
            let ob = Outbox::new(&self.p.db, &link.id);
            // Shared Lidarr state, and the lyrics transcribed here.
            let Ok(batches) = ob.pending(&self.p.store, &[LIDARR.name(), TRANSCRIPTS.name()]) else { continue };
            for b in batches {
                if self.wire.send(&link, &Msg::Entries { entries: b.clone() }).await.is_ok() {
                    let _ = ob.mark(&b);
                } else {
                    break;
                }
            }
        }
    }

    async fn send_status_to(&self, link: &Link) {
        let mut st = self.status();
        st.at = self.now_s();
        let _ = self.wire.send(link, &Msg::Status(Box::new(st))).await;
    }

    pub async fn send_status(&self) {
        for link in self.wire.links.links() {
            self.send_status_to(&link).await;
        }
    }

    // ------------------------------------------------------------ work

    /// One catch-up, the way an open desktop does it.
    pub async fn catch_up_once(&self) -> Option<CatchUpLine> {
        if self.wire.links.all().is_empty() {
            self.doing("Not paired yet. Enter the code from this log in Platter (Settings > Bridge > Add the Bridge), or start the Bridge with BRIDGE_PAIR_CODE set to a code from Platter.");
            return None;
        }
        if !self.lidarr_on() {
            self.doing(LIDARR_OFF);
            return None;
        }
        let st = self.p.lidarr.status();
        if !(st.url_set && st.has_key) {
            self.doing("Waiting for how to reach Lidarr: set LIDARR_URL and LIDARR_API_KEY on the Bridge, or open Platter on a computer at home once.");
            return None;
        }
        if !self.p.lidarr.has_shared_modes() {
            self.doing("Waiting for a desktop Platter to share which artists are protected (open Platter once, with Lidarr set up).");
            return None;
        }
        self.doing("Catching up with Lidarr");
        let line = match self.p.lidarr.catch_up().await {
            Ok(r) => CatchUpLine {
                at: self.now_s(),
                handled: r.handled,
                acted: r.acted,
                scanned: r.scanned.len(),
                announced: r.announced.clone(),
                healed: r.healed.len(),
                paused: r.paused.clone(),
                error: None,
            },
            Err(e) => CatchUpLine { at: self.now_s(), error: Some(e.to_string()), ..Default::default() },
        };
        // The catch-up logs its own summary line, every time.
        if let Some(e) = &line.error {
            tracing::warn!("Catch-up didn't finish: {e}");
        }
        {
            let mut s = self.status.lock();
            s.last_catch_up = Some(line.clone());
            s.lidarr = line.error.is_none();
            s.problem = line.paused.clone().or(line.error.clone());
        }
        self.doing("Waiting for the next catch-up");
        Some(line)
    }

    /// One pass of Bring in my listening, when a desktop handed it over,
    /// then its progress to the desktops.
    pub async fn waves_once(&self) -> Option<crate::lidarr::waves::PassOutcome> {
        if !self.waves.plan_running() || !self.lidarr_on() {
            return None;
        }
        let st = self.p.lidarr.status();
        if st.url_set && st.has_key && self.p.lidarr.last_check().is_none() {
            self.doing("Running Lidarr's setup check");
            if let Err(e) = self.p.lidarr.check().await {
                tracing::info!("Lidarr's setup check: {e}");
            }
        }
        self.doing("Bringing in your listening");
        let out = match self.waves.pass().await {
            Ok(o) => {
                if !o.requested.is_empty() || !o.practiced.is_empty() || !o.needs_you.is_empty() {
                    tracing::info!(
                        requested = o.requested.len(),
                        practice = o.practiced.len(),
                        needs_you = o.needs_you.len(),
                        "Bring in my listening: pass done"
                    );
                }
                Some(o)
            }
            Err(e) => {
                tracing::warn!("Bring in my listening: {e}");
                None
            }
        };
        self.send_waves_progress().await;
        self.flush_entries().await;
        self.doing("Waiting for the next catch-up");
        out
    }

    pub async fn send_waves_progress(&self) {
        let msg = Msg::WavesProgress(Box::new(self.waves.progress()));
        for link in self.wire.links.links() {
            let _ = self.wire.send(&link, &msg).await;
        }
    }

    async fn sync_library(&self) {
        let Some(plex) = &self.p.plex else { return };
        if plex.client().await.is_err() {
            return;
        }
        self.doing("Reading your Plex library");
        let source: Arc<dyn crate::db::LibrarySource> = Arc::new(crate::bridge::PlexLibrary::new(plex.clone()));
        match crate::db::sync::run(self.p.db.clone(), source, false, Arc::new(AtomicBool::new(false))).await {
            Ok(_) => {
                self.status.lock().plex = true;
                self.check_music_mount();
            }
            Err(e) => {
                self.status.lock().plex = false;
                tracing::warn!("Couldn't read the Plex library: {e}");
            }
        }
        self.doing("Waiting for the next catch-up");
    }

    /// Lyrics for every song: the next songs in at night, and the lyrics
    /// line to the computers when it changed.
    pub async fn lyrics_tick(&self) {
        let Some(l) = self.lyrics() else { return };
        if self.wire.links.all().is_empty() {
            return;
        }
        l.tick().await;
        let line = l.line();
        let changed = {
            let mut sent = self.lyrics_sent.lock();
            let changed = sent.as_ref().is_none_or(|s| s.state != line.state || s.problem != line.problem);
            *sent = Some(line);
            changed
        };
        if changed {
            self.send_status().await;
        }
    }

    /// The main loop: listens on every link, catches up every few minutes,
    /// sends shared changes and a heartbeat. Ends when `stop` turns true.
    pub async fn run(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let (tx, mut rx) = mpsc::channel::<Incoming>(64);
        let mut listening: HashMap<String, watch::Sender<bool>> = HashMap::new();
        let mut changes = self.p.store.subscribe();
        let mut catch = tokio::time::interval(crate::lidarr::catchup::EVERY);
        let mut beat = tokio::time::interval(HEARTBEAT);
        let mut library = tokio::time::interval(LIBRARY_EVERY);
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        let mut waves = tokio::time::interval(crate::lidarr::waves::LOOK_EVERY);
        let mut dirty: Option<tokio::time::Instant> = None;
        beat.tick().await;
        self.migrate_plex().await;
        self.connect_plex().await;
        if self.own_sign_in().is_none() && self.p.plex.is_some() {
            // The sign-in that finds the account's devices, even when the
            // Bridge reaches Plex some other way.
            if let Some(plex) = self.p.plex.clone() {
                self.ask_plex_link(plex);
            }
        }
        tokio::spawn(self.clone().show_code_while_unpaired(stop.clone()));
        let looker = account::Looker::new(Arc::new(BridgeAccount(Arc::downgrade(&self))), self.p.relays.clone(), super::DEFAULT_RELAY, self.p.clock.clone());
        tokio::spawn(looker.clone().run(stop.clone()));
        looker.look_now();
        loop {
            // A listener for every link (new pairings included), and none
            // for a link that's gone.
            let links = self.wire.links.links();
            listening.retain(|id, s| {
                let keep = links.iter().any(|l| &l.id == id);
                if !keep {
                    let _ = s.send(true);
                    self.hello.lock().remove(id);
                }
                keep
            });
            for link in links {
                if !listening.contains_key(&link.id) {
                    let (s, stop_one) = watch::channel(false);
                    listening.insert(link.id.clone(), s);
                    tokio::spawn(peer::listen(self.wire.clone(), link, tx.clone(), stop_one));
                }
            }
            self.alive.store(self.now_s(), Ordering::SeqCst);
            let flush_at = dirty.map(|d| d + SETTLE);
            tokio::select! {
                _ = stop.changed() => {
                    for s in listening.values() {
                        let _ = s.send(true);
                    }
                    return;
                }
                Some(inc) = rx.recv() => match inc {
                    Incoming::Opened(id) => {
                        let w = self.welcomes.lock().remove(&id);
                        if let Some((relay, topic, body)) = w {
                            if let Err(e) = self.p.relays.relay(&relay).publish(&topic, &body).await {
                                tracing::info!("Pairing: {}", peer::relay_error(e));
                            }
                        } else if self.hello.lock().insert(id.clone())
                            && let Some(link) = self.wire.links.link(&id)
                        {
                            let hello = self.hello_msg();
                            let _ = self.wire.send(&link, &hello).await;
                            dirty = Some(tokio::time::Instant::now());
                        }
                    }
                    Incoming::Msg(id, msg) => {
                        if let Some(link) = self.wire.links.link(&id) {
                            self.handle(&link, msg).await;
                        }
                    }
                },
                c = changes.recv() => {
                    if c.is_ok_and(|c| c.namespaces.iter().any(|n| n == LIDARR.name())) || dirty.is_none() {
                        dirty = Some(tokio::time::Instant::now());
                    }
                }
                _ = async { if let Some(t) = flush_at { tokio::time::sleep_until(t).await } else { std::future::pending::<()>().await } } => {
                    dirty = None;
                    self.flush_entries().await;
                }
                _ = catch.tick() => {
                    let before = self.status().last_catch_up;
                    let line = self.catch_up_once().await;
                    self.flush_entries().await;
                    let worth = line.as_ref().is_some_and(|l| l.acted > 0 || l.error.is_some() || l.paused.is_some())
                        && before.as_ref().map(|b| (b.acted, &b.error, &b.paused)) != line.as_ref().map(|l| (l.acted, &l.error, &l.paused));
                    if worth {
                        self.send_status().await;
                    }
                }
                _ = beat.tick() => self.send_status().await,
                _ = waves.tick() => {
                    self.waves_once().await;
                }
                _ = self.waves.woken() => {
                    self.waves_once().await;
                }
                _ = library.tick() => self.sync_library().await,
                _ = self.library_now.notified() => self.sync_library().await,
                _ = self.plex_now.notified() => self.connect_plex().await,
                _ = self.links_changed.notified() => {}
                _ = self.account_now.notified() => looker.look_now(),
                _ = tick.tick() => self.lyrics_tick().await,
            }
        }
    }
}

/// The real parts, from the environment.
pub fn build(env: Env) -> AppResult<Parts> {
    std::fs::create_dir_all(&env.data).map_err(|_| AppError::Internal(format!("Can't use the data folder {}.", env.data.display())))?;
    // A damaged file is set aside (logged) so the bridge still starts.
    let (db, _) = DbHandle::open_or_recover(env.data.join("bridge.db"))?;
    let keys: Arc<dyn TokenStore> = Arc::new(FileKeys::new(env.data.join("keys.json")));
    let device = match db.read(|c| kv_get(c, DEVICE_KEY))? {
        Some(d) => d,
        None => {
            let d = format!("{DEVICE_PREFIX}{}", crate::remote::peer::auth::hex(&crate::social::listen::seal::random16()[..8]));
            db.write(|c| kv_set(c, DEVICE_KEY, &d))?;
            d
        }
    };
    let store = SyncStore::new(db.clone(), device);
    let plex = PlexState::with_parts(env.data.join("plex"), keys.clone(), crate::plex::auth::PLEX_TV, Default::default())?;
    let web: Arc<dyn crate::web::Web> = Arc::new(crate::web::ReqwestWeb::new()?);
    let name = env.name.clone();
    let shared = crate::lidarr::shared::Shared::new(store.clone(), Arc::new(move || name.clone()));
    let lidarr = Lidarr::new(crate::lidarr::LidarrDeps {
        web: web.clone(),
        keys: keys.clone(),
        db: db.clone(),
        plex: Arc::new(crate::lidarr::PlexMusicFiles(plex.clone())),
        clock: Arc::new(crate::db::queries::now_s),
        // Each album a wave asks for is checked on MusicBrainz first, as on a desktop.
        mb: Arc::new(crate::lidarr::mbpick::MbCached {
            http: Arc::new(crate::artistinfo::http::ReqwestHttp::new()?),
            // Half MusicBrainz's one a second: a desktop at home shares the
            // same address and has its own limiter, so together they stay near it.
            limiter: Arc::new(crate::artistinfo::limit::Limiter::per_second(0.5)),
            db: db.clone(),
            clock: Arc::new(crate::db::queries::now_s),
            ttl: crate::lidarr::mbpick::CACHE_TTL,
        }),
        artist_mbid: Arc::new(NoMbid),
        album_wait: Duration::from_secs(2),
        shared,
        plex_server: Arc::new(crate::lidarr::plexside::PlexLive(plex.clone())),
    });
    let music = music_access(&env);
    if music == MusicAccess::ReadWrite {
        lidarr.set_hosts(Arc::new(crate::lidarr::upgrade::DesktopHosts { db: db.clone() }));
    }
    lidarr.set_announcer(Arc::new(LogAnnouncer));
    let relays: Arc<dyn RelayFactory> = Arc::new(crate::social::listen::relay::Ntfy::new().map_err(AppError::Internal)?);
    Ok(Parts { env, db, keys, relays, clock: Arc::new(peer::system_ms), store, lidarr, web, plex: Some(plex), music })
}

/// The Bridge's side of links by account (`super::account`).
struct BridgeAccount(std::sync::Weak<Bridge>);

#[async_trait::async_trait]
impl account::AccountSide for BridgeAccount {
    async fn me(&self) -> Option<(Me, Token)> {
        let b = self.0.upgrade()?;
        let token = b.own_sign_in()?;
        let id = b.plex_id();
        (!id.is_empty()).then(|| (Me { id, key: PeerKey::from_token(&token), bridge: true }, token))
    }

    async fn list(&self, token: &Token) -> AppResult<Vec<AccountDevice>> {
        let b = self.0.upgrade().ok_or_else(|| AppError::Internal("stopped".into()))?;
        let plex = b.p.plex.as_ref().ok_or_else(|| AppError::Invalid("no Plex".into()))?;
        let list = account::parse_list(&plex.plex_tv().devices_json(token).await?)?;
        // Devices reach Lidarr through the Bridge, so it links only to the
        // server owner's.
        let servers = crate::plex::discovery::servers_from_resources(plex.plex_tv().resources(token).await?);
        let machine = b.p.db.read(|c| kv_get(c, PLEX_SERVER_KEY)).ok().flatten();
        if account::owns_server(&servers, machine.as_deref()) {
            return Ok(list);
        }
        tracing::warn!("The Bridge is signed in to a Plex account that doesn't own your server, so it doesn't link to any device. Sign it in with the Plex account that owns the server.");
        Ok(account::only_bridges(&list))
    }

    async fn apply(&self, me: &Me, list: &[AccountDevice]) -> Vec<PeerKey> {
        self.0.upgrade().map(|b| b.account_links(me, list)).unwrap_or_default()
    }
}

/// New albums go to the log (desktops show their own).
struct LogAnnouncer;

impl crate::lidarr::catchup::Announcer for LogAnnouncer {
    fn album_added(&self, artist: &str, album: &str) {
        tracing::info!("New in your library: {album} by {artist}.");
    }
}

/// Bring in my listening's daily summary goes to the log.
struct LogNotifier;

impl crate::social::Notifier for LogNotifier {
    fn notify(&self, title: &str, body: &str) {
        tracing::info!("{title}: {body}");
    }
}

struct NoMbid;

#[async_trait::async_trait]
impl crate::lidarr::ArtistMbid for NoMbid {
    async fn artist_mbid(&self, _artist_id: &str, _name: &str) -> Option<String> {
        None
    }
}
