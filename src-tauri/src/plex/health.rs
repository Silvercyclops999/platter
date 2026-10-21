//! Health state machine behind the `health` command and `health://changed`
//!.
//!
//! Pure: inputs are probe outcomes, server events and a clock value; the
//! output is a [`Health`] snapshot plus an action for the caller (re-race
//! connections, require sign-in). The async loop that feeds it lives in
//! `state.rs`.

use std::collections::HashSet;
use std::time::Duration;

use super::client::status_error;
use super::notifications::PlexEvent;
use super::types::{Banner, ConnectionKind, Health, HealthStatus, ScanProgress};
use crate::error::AppError;

/// Ping interval.
pub const PING_INTERVAL: Duration = Duration::from_secs(30);
/// Starting / restarting / updating is only an error after this long.
pub const START_GRACE_MS: i64 = 120_000;
/// Right after waking (or coming back to the front on the phone) the network
/// often isn't up yet: failed checks in this window don't count as offline.
pub const WAKE_GRACE_MS: i64 = 20_000;
/// How soon a check that failed in the wake window is tried again.
pub const WAKE_RETRY: Duration = Duration::from_secs(3);
/// Over remote or relay while the server has a home address: how long
/// between tries of the home addresses, one step further after each miss.
/// Waking or a network change starts over at once.
pub const LOCAL_RETRY_MS: [i64; 4] = [30_000, 60_000, 120_000, 300_000];

/// Result of one ping of the current connection.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeOutcome {
    Ok {
        connection: ConnectionKind,
        latency_ms: u32,
        version: Option<String>,
    },
    /// HTTP 503: server starting / in maintenance.
    Starting,
    /// HTTP 401/403 on an authenticated call: token revoked or expired.
    Unauthorized,
    /// Connect error or timeout.
    Unreachable,
}

/// The server sent a reply, even if Platter couldn't use it: a body that
/// wouldn't decode (a broken character, a cut-off read), a 404 or another
/// unexpected status. A server that answered is reachable, so none of these
/// may count as offline.
pub fn server_answered(e: &AppError) -> bool {
    matches!(e, AppError::Decode(_) | AppError::NotFound(_) | AppError::Invalid(_))
}

impl ProbeOutcome {
    /// Classify an error from a ping. Errors where the server answered
    /// ([`server_answered`]) are not offline; the ping treats them as a
    /// reply before it gets here.
    pub fn from_error(e: &AppError) -> Self {
        match e {
            AppError::Auth(_) => ProbeOutcome::Unauthorized,
            AppError::Server(m) if m.contains("503") => ProbeOutcome::Starting,
            _ => ProbeOutcome::Unreachable,
        }
    }

    pub fn from_status(code: u16) -> Self {
        match reqwest::StatusCode::from_u16(code) {
            Ok(s) => Self::from_error(&status_error(s, "")),
            Err(_) => ProbeOutcome::Unreachable,
        }
    }
}

/// What the caller should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthAction {
    None,
    /// Re-race the server's connections (address changed, Wi-Fi switch,
    /// local went away). Report the result with `on_probe` or
    /// `on_rediscover_failed`.
    Rediscover,
    /// Token rejected: clear it and show sign-in.
    SignInRequired,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HealthMachine {
    signed_in: bool,
    status: HealthStatus,
    server_name: Option<String>,
    connection: Option<ConnectionKind>,
    latency_ms: Option<u32>,
    version: Option<String>,
    last_seen_at: Option<i64>,
    /// When a starting/restart/update window began.
    starting_since: Option<i64>,
    /// The server announced an update install.
    updating: bool,
    access_removed: bool,
    scan: Option<ScanProgress>,
    files_available: bool,
    /// False when the last failed re-race couldn't reach plex.tv either, or
    /// the computer has no network route at all.
    internet: bool,
    /// The server has no music library.
    no_music_library: bool,
    /// The disk holding the caches is nearly full.
    low_disk: bool,
    /// macOS Local Network privacy is keeping Platter off the home network:
    /// the last race had every local address refused by the system.
    local_network_blocked: bool,
    /// Go offline is on: no network use, only music on this computer plays.
    offline_by_choice: bool,
    /// When the computer woke or the app came back to the front, until the
    /// server next answers (see [`WAKE_GRACE_MS`]).
    woke_at: Option<i64>,
    /// Over remote or relay: when to try the home addresses next, and how
    /// many tries have missed (see [`LOCAL_RETRY_MS`]).
    local_try_at: i64,
    local_misses: usize,
}

/// What the macOS Local Network banner says.
pub const LOCAL_NETWORK_BLOCKED: &str = "macOS is blocking Platter from your home network. Open System Settings, Privacy & Security, Local Network, and turn on Platter.";

impl Default for HealthMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthMachine {
    pub fn new() -> Self {
        HealthMachine {
            signed_in: false,
            status: HealthStatus::NoAuth,
            server_name: None,
            connection: None,
            latency_ms: None,
            version: None,
            last_seen_at: None,
            starting_since: None,
            updating: false,
            access_removed: false,
            scan: None,
            files_available: true,
            internet: true,
            no_music_library: false,
            low_disk: false,
            local_network_blocked: false,
            offline_by_choice: false,
            woke_at: None,
            local_try_at: 0,
            local_misses: 0,
        }
    }

    /// The computer woke, the app came back to the front, or the network
    /// changed: the next few failures may be the network still coming up.
    pub fn on_woke(&mut self, now_ms: i64) {
        self.woke_at = Some(now_ms);
        // Back home after a sleep or a network change: the home address may
        // work now even though the remote one still answers.
        self.local_try_at = 0;
        self.local_misses = 0;
    }

    /// Within [`WAKE_GRACE_MS`] of [`Self::on_woke`], with the server not
    /// heard from since.
    pub fn in_wake_grace(&self, now_ms: i64) -> bool {
        self.woke_at.is_some_and(|at| now_ms - at < WAKE_GRACE_MS)
    }

    /// Go offline turned on or off. On, the status is offline until it's
    /// turned off and the server answers again.
    pub fn set_offline_by_choice(&mut self, on: bool) {
        self.offline_by_choice = on;
        if on {
            self.status = HealthStatus::Offline;
            self.connection = None;
            self.latency_ms = None;
        }
    }

    pub fn offline_by_choice(&self) -> bool {
        self.offline_by_choice
    }

    /// Whether the last connection race found the home network blocked by
    /// the system (only reported on macOS).
    pub fn set_local_network_blocked(&mut self, blocked: bool) {
        self.local_network_blocked = blocked;
    }

    pub fn local_network_blocked(&self) -> bool {
        self.local_network_blocked
    }

    /// Connected over remote or relay only because the home network was
    /// blocked: worth racing again to see if local works now.
    pub fn retry_local(&self) -> bool {
        self.local_network_blocked
            && matches!(self.status, HealthStatus::Remote | HealthStatus::Relay)
    }

    /// Over remote or relay and time to see if the home address answers
    /// again. `has_local` is whether the server lists one. After waking the
    /// local network can be slower to come up than the remote route, so the
    /// race picks remote and the ping keeps it; this moves back home.
    pub fn local_try_due(&self, now_ms: i64, has_local: bool) -> bool {
        has_local && matches!(self.status, HealthStatus::Remote | HealthStatus::Relay) && now_ms >= self.local_try_at
    }

    /// A try of the home addresses found none answering.
    pub fn on_local_missed(&mut self, now_ms: i64) {
        let step = self.local_misses.min(LOCAL_RETRY_MS.len() - 1);
        self.local_try_at = now_ms + LOCAL_RETRY_MS[step];
        self.local_misses += 1;
    }

    pub fn status(&self) -> HealthStatus {
        self.status
    }

    /// Signed in with a server picked (or switched servers). Until the first
    /// probe the status is Offline with no banner.
    pub fn on_server_selected(&mut self, name: &str, version: Option<String>) {
        let low_disk = self.low_disk;
        let offline_by_choice = self.offline_by_choice;
        *self = HealthMachine {
            signed_in: true,
            status: HealthStatus::Offline,
            server_name: Some(name.to_string()),
            version,
            low_disk,
            offline_by_choice,
            ..HealthMachine::new()
        };
    }

    /// The token was rejected. Unlike sign-out, the app stays open (cached
    /// music keeps playing) and shows "Please sign in again".
    pub fn on_token_rejected(&mut self) {
        self.signed_in = true;
        self.status = HealthStatus::NoAuth;
        self.connection = None;
        self.latency_ms = None;
        self.scan = None;
    }

    pub fn on_signed_out(&mut self) {
        *self = HealthMachine::new();
    }

    pub fn on_probe(&mut self, outcome: ProbeOutcome, now_ms: i64) -> HealthAction {
        if !self.signed_in || self.offline_by_choice {
            return HealthAction::None;
        }
        match outcome {
            ProbeOutcome::Ok {
                connection,
                latency_ms,
                version,
            } => {
                self.status = match connection {
                    ConnectionKind::Local => HealthStatus::Local,
                    ConnectionKind::Remote => HealthStatus::Remote,
                    ConnectionKind::Relay => HealthStatus::Relay,
                };
                self.connection = Some(connection);
                self.latency_ms = Some(latency_ms);
                if version.is_some() {
                    self.version = version;
                }
                self.last_seen_at = Some(now_ms);
                self.woke_at = None;
                self.starting_since = None;
                self.updating = false;
                self.access_removed = false;
                self.internet = true;
                if connection == ConnectionKind::Local {
                    self.local_network_blocked = false;
                    self.local_try_at = 0;
                    self.local_misses = 0;
                }
                HealthAction::None
            }
            ProbeOutcome::Unauthorized => {
                self.status = HealthStatus::NoAuth;
                self.connection = None;
                HealthAction::SignInRequired
            }
            ProbeOutcome::Starting => {
                self.starting_since.get_or_insert(now_ms);
                self.status = if self.updating {
                    HealthStatus::Updating
                } else {
                    HealthStatus::Starting
                };
                self.connection = None;
                HealthAction::None
            }
            ProbeOutcome::Unreachable => {
                if self.in_restart_window(now_ms) {
                    // Restart/update in progress: keep waiting.
                    self.status = if self.updating {
                        HealthStatus::Updating
                    } else {
                        HealthStatus::Starting
                    };
                    self.connection = None;
                    HealthAction::None
                } else {
                    // Try other routes before calling it offline (address
                    // changed, network switch). Status is unchanged until
                    // the re-race reports back.
                    HealthAction::Rediscover
                }
            }
        }
    }

    fn in_restart_window(&self, now_ms: i64) -> bool {
        self.starting_since
            .is_some_and(|since| now_ms - since < START_GRACE_MS)
    }

    /// Re-race found no working connection. `internet` is whether plex.tv
    /// answered during the same attempt: if it didn't either, the problem is
    /// this computer's connection, not the server.
    pub fn on_rediscover_failed(&mut self, now_ms: i64, internet: bool) {
        if !self.signed_in || self.in_wake_grace(now_ms) {
            // Just woke: the network may not be up yet. Keep the last state;
            // the caller tries again in a few seconds.
            return;
        }
        self.internet = internet;
        if self.in_restart_window(now_ms) {
            return;
        }
        self.status = HealthStatus::Offline;
        self.connection = None;
        self.latency_ms = None;
    }

    /// The computer has no network route at all (cable out, Wi-Fi off).
    pub fn on_no_network(&mut self) {
        if !self.signed_in || self.status == HealthStatus::NoAuth {
            return;
        }
        if self.woke_at.is_some() && self.connected() {
            // Just woke: the network is still coming up. The wake checks
            // say offline if it doesn't.
            return;
        }
        self.internet = false;
        // With no route at all every address fails at once; that isn't a
        // block on the home network.
        self.local_network_blocked = false;
        self.status = HealthStatus::Offline;
        self.connection = None;
        self.latency_ms = None;
    }

    /// Server announced an update (notification `update.statechange`).
    pub fn on_update_state(&mut self, installing: bool, now_ms: i64) {
        self.updating = installing;
        if installing {
            self.starting_since.get_or_insert(now_ms);
        }
    }

    /// Server is going down (notification); treat the next failures as a
    /// restart for up to the grace window.
    pub fn on_shutdown_notice(&mut self, now_ms: i64) {
        self.starting_since.get_or_insert(now_ms);
    }

    /// plex.tv no longer lists the selected server for this account.
    pub fn on_access_removed(&mut self) {
        if !self.signed_in {
            return;
        }
        self.access_removed = true;
        self.status = HealthStatus::NoAuth;
        self.connection = None;
    }

    pub fn on_scan(&mut self, scan: Option<ScanProgress>) {
        self.scan = scan;
    }

    pub fn set_files_available(&mut self, available: bool) {
        self.files_available = available;
    }

    pub fn files_available(&self) -> bool {
        self.files_available
    }

    pub fn set_no_music_library(&mut self, none: bool) {
        self.no_music_library = none;
    }

    pub fn no_music_library(&self) -> bool {
        self.no_music_library
    }

    pub fn set_low_disk(&mut self, low: bool) {
        self.low_disk = low;
    }

    /// Connected to the server (any route).
    pub fn connected(&self) -> bool {
        !self.offline_by_choice
            && matches!(
            self.status,
            HealthStatus::Local | HealthStatus::Remote | HealthStatus::Relay
        )
    }

    /// Only music already on this computer can play right now.
    pub fn cached_only(&self) -> bool {
        self.signed_in && (!self.connected() || !self.files_available)
    }

    fn banner(&self, now_ms: i64) -> Option<Banner> {
        let b = |kind: &str, message: &str| {
            Some(Banner {
                kind: kind.to_string(),
                message: message.to_string(),
            })
        };
        if !self.signed_in {
            return None;
        }
        if self.access_removed {
            return b("accessRemoved", "You no longer have access to this server.");
        }
        if self.offline_by_choice {
            return b(
                "goneOffline",
                "You're offline. Only music on this computer plays. Turn off Go offline in Settings to use your server again.",
            );
        }
        match self.status {
            HealthStatus::NoAuth => return b("signIn", "Please sign in again."),
            HealthStatus::Starting | HealthStatus::Updating => {
                let late = self
                    .starting_since
                    .is_some_and(|s| now_ms - s >= START_GRACE_MS);
                return if late {
                    b(
                        "startTimeout",
                        "Your server is taking too long to come back. Check that Plex is running.",
                    )
                } else if self.status == HealthStatus::Updating {
                    b("updating", "Server is updating…")
                } else {
                    b("starting", "Server is starting…")
                };
            }
            HealthStatus::Offline => {
                if !self.internet {
                    return b("noInternet", "You're offline.");
                }
                if self.local_network_blocked {
                    return b("localNetwork", LOCAL_NETWORK_BLOCKED);
                }
                let msg = match self.last_seen_at {
                    Some(seen) => format!(
                        "Your server is offline. Last seen {}.",
                        ago(now_ms.saturating_sub(seen))
                    ),
                    None => "Your server is offline.".into(),
                };
                return b("offline", &msg);
            }
            HealthStatus::Local => {}
            // Playing over the internet from home: still worth fixing, since
            // remote and relay streams are lower quality.
            HealthStatus::Remote | HealthStatus::Relay if self.local_network_blocked => {
                return b("localNetwork", LOCAL_NETWORK_BLOCKED);
            }
            HealthStatus::Remote | HealthStatus::Relay => {}
        }
        if !self.files_available {
            return b(
                "filesUnavailable",
                "Your server is up, but your music files aren't available.",
            );
        }
        if self.no_music_library {
            return b("noMusicLibrary", "No music library found.");
        }
        if self.low_disk {
            return b("diskFull", "Not enough space.");
        }
        if self.status == HealthStatus::Relay {
            return b(
                "relay",
                "Connected through Plex Relay, so quality is reduced. Turn on Remote Access in Plex for full quality.",
            );
        }
        None
    }

    pub fn snapshot(&self, now_ms: i64) -> Health {
        Health {
            status: self.status,
            server_name: self.server_name.clone(),
            connection: self.connection,
            latency_ms: self.latency_ms,
            server_version: self.version.clone(),
            last_seen_at: self.last_seen_at,
            scan: self.scan.clone(),
            banner: self.banner(now_ms),
            files_available: self.files_available,
            cached_only: self.cached_only(),
        }
    }
}

/// Counts new tracks while Plex scans the library, for "Adding new music…
/// 120 of 340". Plex reports a scan as an activity (percent only), and each
/// item it adds as a timeline entry: state 0 when created, 5 when fully
/// processed. Only tracks first seen as created count, so a rescan of an
/// unchanged library shows nothing.
#[derive(Debug, Default, Clone)]
pub struct ScanTracker {
    activities: HashSet<String>,
    seen: HashSet<String>,
    done: HashSet<String>,
}

const TRACK_TYPE: i64 = 10;
const STATE_CREATED: i64 = 0;
const STATE_DONE: i64 = 5;

impl ScanTracker {
    fn is_scan(kind: &str) -> bool {
        kind.starts_with("library.update") || kind.starts_with("library.refresh")
    }

    /// Feed one server event. Returns true if the progress changed.
    pub fn on_event(&mut self, ev: &PlexEvent) -> bool {
        let before = self.progress();
        match ev {
            PlexEvent::Activity { event, uuid, kind, .. } if Self::is_scan(kind) => {
                if event == "ended" {
                    self.activities.remove(uuid);
                    if self.activities.is_empty() {
                        self.seen.clear();
                        self.done.clear();
                    }
                } else {
                    self.activities.insert(uuid.clone());
                }
            }
            PlexEvent::LibraryUpdate {
                item_id: Some(id),
                item_type: Some(TRACK_TYPE),
                state: Some(state),
                ..
            } => {
                if *state == STATE_CREATED {
                    self.seen.insert(id.clone());
                } else if *state == STATE_DONE && self.seen.contains(id) {
                    self.done.insert(id.clone());
                }
            }
            // The socket dropped: whatever was running may have ended unseen.
            PlexEvent::Disconnected => {
                self.activities.clear();
                self.seen.clear();
                self.done.clear();
            }
            _ => {}
        }
        before != self.progress()
    }

    /// `Some` while a scan is running and has found new tracks.
    pub fn progress(&self) -> Option<ScanProgress> {
        if self.activities.is_empty() || self.seen.is_empty() {
            return None;
        }
        Some(ScanProgress {
            done: self.done.len() as u32,
            total: self.seen.len() as u32,
        })
    }
}

/// "just now", "1 min ago", "4 min ago", "2 h ago", "3 days ago".
pub fn ago(ms: i64) -> String {
    let mins = ms / 60_000;
    if mins < 1 {
        "just now".into()
    } else if mins < 60 {
        format!("{mins} min ago")
    } else if mins < 48 * 60 {
        format!("{} h ago", mins / 60)
    } else {
        format!("{} days ago", mins / (24 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_760_000_000_000;

    fn ok(kind: ConnectionKind) -> ProbeOutcome {
        ProbeOutcome::Ok {
            connection: kind,
            latency_ms: 42,
            version: Some("1.43.4".into()),
        }
    }

    fn signed_in() -> HealthMachine {
        let mut m = HealthMachine::new();
        m.on_server_selected("Home", None);
        m
    }

    #[test]
    fn a_reply_that_couldnt_be_used_is_still_an_answer() {
        for e in [
            AppError::Decode("error decoding response body".into()),
            AppError::Decode("/: invalid unicode code point".into()),
            AppError::NotFound("/".into()),
            AppError::Invalid("Unexpected response 418.".into()),
        ] {
            assert!(server_answered(&e), "{e}");
        }
        for e in [AppError::Network("Couldn't connect.".into()), AppError::Network("The request timed out.".into())] {
            assert!(!server_answered(&e), "{e}");
        }
    }

    #[test]
    fn failures_right_after_waking_dont_count_as_offline() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.on_woke(T0 + 1_000);
        // The network isn't up yet: no route, then nothing answers.
        m.on_no_network();
        m.on_rediscover_failed(T0 + 2_000, false);
        m.on_rediscover_failed(T0 + 8_000, false);
        assert!(m.connected() && !m.cached_only(), "still in the wake window");
        // Still nothing once the window is over: offline.
        m.on_rediscover_failed(T0 + 1_000 + WAKE_GRACE_MS, false);
        assert!(m.cached_only());
        // An answer ends the window, so a later failure counts at once.
        let mut m = signed_in();
        m.on_woke(T0);
        m.on_probe(ok(ConnectionKind::Local), T0 + 1_000);
        assert!(!m.in_wake_grace(T0 + 2_000));
        m.on_rediscover_failed(T0 + 2_000, true);
        assert!(m.cached_only());
    }

    #[test]
    fn offline_recovers_on_the_next_answer() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.on_rediscover_failed(T0 + 1_000, true);
        assert!(m.cached_only());
        // However long it was out, one answer brings it back.
        m.on_probe(ok(ConnectionKind::Remote), T0 + 3_600_000);
        assert!(!m.cached_only());
        assert_eq!(m.snapshot(T0 + 3_600_000).banner, None);
    }

    #[test]
    fn signed_out_is_no_auth_without_banner() {
        let m = HealthMachine::new();
        let h = m.snapshot(T0);
        assert_eq!(h.status, HealthStatus::NoAuth);
        assert!(h.banner.is_none());
    }

    #[test]
    fn local_ok_has_no_banner() {
        let mut m = signed_in();
        assert_eq!(
            m.on_probe(ok(ConnectionKind::Local), T0),
            HealthAction::None
        );
        let h = m.snapshot(T0);
        assert_eq!(h.status, HealthStatus::Local);
        assert_eq!(h.latency_ms, Some(42));
        assert_eq!(h.server_version.as_deref(), Some("1.43.4"));
        assert_eq!(h.last_seen_at, Some(T0));
        assert!(h.banner.is_none());
    }

    #[test]
    fn remote_only_shows_remote_without_banner() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Remote), T0);
        let h = m.snapshot(T0);
        assert_eq!(h.status, HealthStatus::Remote);
        assert_eq!(h.connection, Some(ConnectionKind::Remote));
        assert!(h.banner.is_none());
    }

    #[test]
    fn relay_only_shows_relay_tip() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Relay), T0);
        let h = m.snapshot(T0);
        assert_eq!(h.status, HealthStatus::Relay);
        assert_eq!(h.banner.unwrap().kind, "relay");
    }

    #[test]
    fn offline_after_failed_rediscover_with_last_seen() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        let later = T0 + 4 * 60_000;
        assert_eq!(
            m.on_probe(ProbeOutcome::Unreachable, later),
            HealthAction::Rediscover
        );
        // Not offline until the re-race fails.
        assert_eq!(m.status(), HealthStatus::Local);
        m.on_rediscover_failed(later, true);
        let h = m.snapshot(later);
        assert_eq!(h.status, HealthStatus::Offline);
        let b = h.banner.unwrap();
        assert_eq!(b.kind, "offline");
        assert_eq!(b.message, "Your server is offline. Last seen 4 min ago.");
        assert_eq!(h.last_seen_at, Some(T0));
        // comes back
        m.on_probe(ok(ConnectionKind::Local), later + 1000);
        assert_eq!(m.snapshot(later + 1000).status, HealthStatus::Local);
    }

    #[test]
    fn address_changed_rediscovers_silently() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        assert_eq!(
            m.on_probe(ProbeOutcome::Unreachable, T0 + 30_000),
            HealthAction::Rediscover
        );
        // banner never shown during the re-race
        assert!(m.snapshot(T0 + 30_000).banner.is_none());
        // re-race found the server on its new address
        m.on_probe(ok(ConnectionKind::Local), T0 + 31_000);
        let h = m.snapshot(T0 + 31_000);
        assert_eq!(h.status, HealthStatus::Local);
        assert!(h.banner.is_none());
    }

    #[test]
    fn starting_waits_then_errors_after_two_minutes() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        assert_eq!(
            m.on_probe(ProbeOutcome::Starting, T0 + 1000),
            HealthAction::None
        );
        let h = m.snapshot(T0 + 1000);
        assert_eq!(h.status, HealthStatus::Starting);
        assert_eq!(h.banner.unwrap().kind, "starting");
        // restart: connection refused for a while is still "starting"
        assert_eq!(
            m.on_probe(ProbeOutcome::Unreachable, T0 + 60_000),
            HealthAction::None
        );
        assert_eq!(m.status(), HealthStatus::Starting);
        // still 503 after 2 minutes → error banner
        m.on_probe(ProbeOutcome::Starting, T0 + 1000 + START_GRACE_MS);
        let h = m.snapshot(T0 + 1000 + START_GRACE_MS);
        assert_eq!(h.banner.unwrap().kind, "startTimeout");
        // unreachable after the window → rediscover, then offline
        assert_eq!(
            m.on_probe(ProbeOutcome::Unreachable, T0 + 200_000),
            HealthAction::Rediscover
        );
        m.on_rediscover_failed(T0 + 200_000, true);
        assert_eq!(m.status(), HealthStatus::Offline);
    }

    #[test]
    fn updating_shows_updating() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.on_update_state(true, T0 + 10);
        assert_eq!(
            m.on_probe(ProbeOutcome::Unreachable, T0 + 5000),
            HealthAction::None
        );
        let h = m.snapshot(T0 + 5000);
        assert_eq!(h.status, HealthStatus::Updating);
        assert_eq!(h.banner.unwrap().kind, "updating");
        m.on_probe(ProbeOutcome::Starting, T0 + 60_000);
        assert_eq!(m.status(), HealthStatus::Updating);
        m.on_probe(ok(ConnectionKind::Local), T0 + 90_000);
        let h = m.snapshot(T0 + 90_000);
        assert_eq!(h.status, HealthStatus::Local);
        assert!(h.banner.is_none());
    }

    #[test]
    fn shutdown_notice_opens_restart_window() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.on_shutdown_notice(T0 + 1);
        assert_eq!(
            m.on_probe(ProbeOutcome::Unreachable, T0 + 2000),
            HealthAction::None
        );
        assert_eq!(m.status(), HealthStatus::Starting);
    }

    #[test]
    fn token_revoked_requires_sign_in() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        assert_eq!(
            m.on_probe(ProbeOutcome::Unauthorized, T0 + 1),
            HealthAction::SignInRequired
        );
        let h = m.snapshot(T0 + 1);
        assert_eq!(h.status, HealthStatus::NoAuth);
        assert_eq!(h.banner.unwrap().kind, "signIn");
    }

    #[test]
    fn access_removed_banner() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Remote), T0);
        m.on_access_removed();
        let h = m.snapshot(T0);
        assert_eq!(h.status, HealthStatus::NoAuth);
        let b = h.banner.unwrap();
        assert_eq!(b.kind, "accessRemoved");
        assert!(b.message.contains("no longer have access"));
    }

    #[test]
    fn plex_tv_down_with_cached_server_is_silent() {
        // plex.tv being down never reaches the machine: the cached
        // connection answers and the status is normal.
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        assert!(m.snapshot(T0).banner.is_none());
    }

    #[test]
    fn files_unavailable_banner_while_up() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.set_files_available(false);
        let h = m.snapshot(T0);
        assert!(!h.files_available);
        assert_eq!(h.banner.unwrap().kind, "filesUnavailable");
    }

    #[test]
    fn scan_progress_is_reported() {
        let mut m = signed_in();
        m.on_scan(Some(ScanProgress {
            done: 120,
            total: 340,
        }));
        assert_eq!(
            m.snapshot(T0).scan,
            Some(ScanProgress {
                done: 120,
                total: 340
            })
        );
    }

    #[test]
    fn outcome_classification() {
        assert_eq!(ProbeOutcome::from_status(401), ProbeOutcome::Unauthorized);
        assert_eq!(ProbeOutcome::from_status(403), ProbeOutcome::Unauthorized);
        assert_eq!(ProbeOutcome::from_status(503), ProbeOutcome::Starting);
        assert_eq!(ProbeOutcome::from_status(500), ProbeOutcome::Unreachable);
        assert_eq!(
            ProbeOutcome::from_error(&AppError::Network("x".into())),
            ProbeOutcome::Unreachable
        );
    }

    #[test]
    fn ago_formats() {
        assert_eq!(ago(10_000), "just now");
        assert_eq!(ago(60_000), "1 min ago");
        assert_eq!(ago(4 * 60_000), "4 min ago");
        assert_eq!(ago(3 * 3_600_000), "3 h ago");
        assert_eq!(ago(72 * 3_600_000), "3 days ago");
    }

    #[test]
    fn no_internet_says_you_are_offline() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.on_probe(ProbeOutcome::Unreachable, T0 + 1000);
        // plex.tv didn't answer during the re-race either
        m.on_rediscover_failed(T0 + 1000, false);
        let h = m.snapshot(T0 + 1000);
        assert_eq!(h.status, HealthStatus::Offline);
        let b = h.banner.unwrap();
        assert_eq!(b.kind, "noInternet");
        assert_eq!(b.message, "You're offline.");
        assert!(h.cached_only);
        // network back, server back
        m.on_probe(ok(ConnectionKind::Local), T0 + 5000);
        let h = m.snapshot(T0 + 5000);
        assert!(h.banner.is_none());
        assert!(!h.cached_only);
    }

    #[test]
    fn no_network_route_is_offline_at_once() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Remote), T0);
        m.on_no_network();
        let h = m.snapshot(T0);
        assert_eq!(h.status, HealthStatus::Offline);
        assert_eq!(h.banner.unwrap().kind, "noInternet");
    }

    #[test]
    fn local_network_blocked_with_nothing_else_says_how_to_fix_it() {
        let mut m = signed_in();
        m.set_local_network_blocked(true);
        m.on_rediscover_failed(T0, true);
        let h = m.snapshot(T0);
        assert_eq!(h.status, HealthStatus::Offline);
        let b = h.banner.unwrap();
        assert_eq!(b.kind, "localNetwork");
        assert_eq!(b.message, LOCAL_NETWORK_BLOCKED);
        assert!(h.cached_only);
        // Turned on in System Settings: the next race wins locally.
        m.on_probe(ok(ConnectionKind::Local), T0 + 1000);
        let h = m.snapshot(T0 + 1000);
        assert!(h.banner.is_none());
        assert!(!h.cached_only);
    }

    #[test]
    fn local_network_blocked_but_remote_works_keeps_playing_and_says_so() {
        let mut m = signed_in();
        m.set_local_network_blocked(true);
        m.on_probe(ok(ConnectionKind::Remote), T0);
        let h = m.snapshot(T0);
        assert_eq!(h.status, HealthStatus::Remote);
        assert!(!h.cached_only);
        assert_eq!(h.banner.unwrap().kind, "localNetwork");
        // Each tick races again so turning access on moves back to local.
        assert!(m.retry_local());
        // Relay too, ahead of the relay tip.
        m.on_probe(ok(ConnectionKind::Relay), T0);
        assert_eq!(m.snapshot(T0).banner.unwrap().kind, "localNetwork");
        m.set_local_network_blocked(false);
        assert_eq!(m.snapshot(T0).banner.unwrap().kind, "relay");
        assert!(!m.retry_local());
    }

    #[test]
    fn stuck_on_remote_tries_home_again_less_often_and_at_once_after_waking() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Remote), T0);
        assert!(m.local_try_due(T0, true));
        assert!(!m.local_try_due(T0, false), "no home address listed");
        let mut at = T0;
        for wait in LOCAL_RETRY_MS.iter().chain([&LOCAL_RETRY_MS[3], &LOCAL_RETRY_MS[3]]) {
            m.on_local_missed(at);
            assert!(!m.local_try_due(at + wait - 1, true));
            at += wait;
            assert!(m.local_try_due(at, true));
        }
        // Waking (or a new network) tries at once, from the first step.
        m.on_local_missed(at);
        m.on_woke(at + 1);
        m.on_probe(ok(ConnectionKind::Remote), at + 2);
        assert!(m.local_try_due(at + 2, true));
        m.on_local_missed(at + 2);
        assert!(m.local_try_due(at + 2 + LOCAL_RETRY_MS[0], true));
        // Home again: nothing more to try.
        m.on_probe(ok(ConnectionKind::Local), at + 3);
        assert!(!m.local_try_due(at + 3, true));
        // Relay counts too.
        m.on_probe(ok(ConnectionKind::Relay), at + 4);
        assert!(m.local_try_due(at + 4, true));
    }

    #[test]
    fn no_route_at_all_is_offline_not_a_blocked_network() {
        let mut m = signed_in();
        m.set_local_network_blocked(true);
        m.on_no_network();
        assert_eq!(m.snapshot(T0).banner.unwrap().kind, "noInternet");
        // plex.tv not answering either means this computer is offline.
        let mut m = signed_in();
        m.set_local_network_blocked(true);
        m.on_rediscover_failed(T0, false);
        assert_eq!(m.snapshot(T0).banner.unwrap().kind, "noInternet");
    }

    #[test]
    fn server_offline_is_cached_only() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        assert!(!m.cached_only());
        m.on_rediscover_failed(T0 + 1, true);
        assert!(m.snapshot(T0 + 1).cached_only);
    }

    #[test]
    fn going_offline_by_choice_is_cached_only_until_turned_off() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.set_offline_by_choice(true);
        let h = m.snapshot(T0 + 1);
        assert!(h.cached_only && !m.connected());
        assert_eq!(h.status, HealthStatus::Offline);
        assert_eq!(h.banner.unwrap().kind, "goneOffline");
        // Pings are ignored while it lasts, and switching servers keeps it.
        assert_eq!(m.on_probe(ok(ConnectionKind::Local), T0 + 2), HealthAction::None);
        assert!(!m.connected());
        m.on_server_selected("Other", None);
        assert!(m.offline_by_choice());
        m.set_offline_by_choice(false);
        m.on_probe(ok(ConnectionKind::Local), T0 + 3);
        assert!(m.connected() && !m.snapshot(T0 + 3).cached_only);
        assert_eq!(m.snapshot(T0 + 3).banner, None);
    }

    #[test]
    fn files_unavailable_is_cached_only_and_recovers() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.set_files_available(false);
        let h = m.snapshot(T0);
        assert!(h.cached_only);
        assert_eq!(
            h.banner.unwrap().message,
            "Your server is up, but your music files aren't available."
        );
        m.set_files_available(true);
        assert!(m.snapshot(T0).banner.is_none());
    }

    #[test]
    fn token_rejected_keeps_app_open_with_sign_in_banner() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.on_token_rejected();
        let h = m.snapshot(T0);
        assert_eq!(h.status, HealthStatus::NoAuth);
        assert_eq!(h.banner.unwrap().message, "Please sign in again.");
        assert!(h.cached_only);
    }

    #[test]
    fn no_music_library_banner() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.set_no_music_library(true);
        assert_eq!(
            m.snapshot(T0).banner.unwrap().message,
            "No music library found."
        );
        m.set_no_music_library(false);
        assert!(m.snapshot(T0).banner.is_none());
    }

    #[test]
    fn low_disk_banner_survives_server_switch() {
        let mut m = signed_in();
        m.on_probe(ok(ConnectionKind::Local), T0);
        m.set_low_disk(true);
        assert_eq!(m.snapshot(T0).banner.unwrap().message, "Not enough space.");
        m.on_server_selected("Other", None);
        m.on_probe(ok(ConnectionKind::Local), T0);
        assert_eq!(m.snapshot(T0).banner.unwrap().kind, "diskFull");
        m.set_low_disk(false);
        assert!(m.snapshot(T0).banner.is_none());
    }

    fn activity(event: &str, uuid: &str) -> PlexEvent {
        PlexEvent::Activity {
            event: event.into(),
            uuid: uuid.into(),
            kind: "library.update.section".into(),
            title: None,
            subtitle: None,
            progress: Some(10),
            section_id: Some("3".into()),
        }
    }

    fn timeline(id: &str, ty: i64, state: i64) -> PlexEvent {
        PlexEvent::LibraryUpdate {
            section_id: Some("3".into()),
            item_id: Some(id.into()),
            item_type: Some(ty),
            state: Some(state),
        }
    }

    #[test]
    fn scan_counts_new_tracks() {
        let mut t = ScanTracker::default();
        assert!(!t.on_event(&activity("started", "a")));
        assert_eq!(t.progress(), None, "nothing new yet");
        for i in 0..340 {
            t.on_event(&timeline(&format!("t{i}"), 10, 0));
        }
        // an album and an existing track being refreshed don't count
        t.on_event(&timeline("al1", 9, 0));
        t.on_event(&timeline("old", 10, 5));
        for i in 0..120 {
            t.on_event(&timeline(&format!("t{i}"), 10, 5));
        }
        assert_eq!(
            t.progress(),
            Some(ScanProgress {
                done: 120,
                total: 340
            })
        );
        assert!(t.on_event(&activity("ended", "a")));
        assert_eq!(t.progress(), None);
    }

    #[test]
    fn scan_progress_clears_on_disconnect() {
        let mut t = ScanTracker::default();
        t.on_event(&activity("started", "a"));
        t.on_event(&timeline("x", 10, 0));
        assert!(t.progress().is_some());
        assert!(t.on_event(&PlexEvent::Disconnected));
        assert_eq!(t.progress(), None);
    }

    #[test]
    fn probes_ignored_when_signed_out() {
        let mut m = HealthMachine::new();
        assert_eq!(
            m.on_probe(ProbeOutcome::Unauthorized, T0),
            HealthAction::None
        );
        assert_eq!(m.status(), HealthStatus::NoAuth);
    }
}
