//! Health state machine behind the `health` command and `health://changed`
//! (spec 07 "Server and connection" / "Account").
//!
//! Pure: inputs are probe outcomes, server events and a clock value; the
//! output is a [`Health`] snapshot plus an action for the caller (re-race
//! connections, require sign-in). The async loop that feeds it lives in
//! `state.rs`.

use std::time::Duration;

use super::client::status_error;
use super::types::{Banner, ConnectionKind, Health, HealthStatus, ScanProgress};
use crate::error::AppError;

/// Ping interval.
pub const PING_INTERVAL: Duration = Duration::from_secs(30);
/// Starting / restarting / updating is only an error after this long.
pub const START_GRACE_MS: i64 = 120_000;

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

impl ProbeOutcome {
    /// Classify an error from a ping.
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
}

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
        }
    }

    pub fn status(&self) -> HealthStatus {
        self.status
    }

    /// Signed in with a server picked (or switched servers). Until the first
    /// probe the status is Offline with no banner.
    pub fn on_server_selected(&mut self, name: &str, version: Option<String>) {
        *self = HealthMachine {
            signed_in: true,
            status: HealthStatus::Offline,
            server_name: Some(name.to_string()),
            version,
            ..HealthMachine::new()
        };
    }

    pub fn on_signed_out(&mut self) {
        *self = HealthMachine::new();
    }

    pub fn on_probe(&mut self, outcome: ProbeOutcome, now_ms: i64) -> HealthAction {
        if !self.signed_in {
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
                self.starting_since = None;
                self.updating = false;
                self.access_removed = false;
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

    /// Re-race found no working connection.
    pub fn on_rediscover_failed(&mut self, now_ms: i64) {
        if !self.signed_in {
            return;
        }
        if self.in_restart_window(now_ms) {
            return;
        }
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

    fn banner(&self, now_ms: i64) -> Option<Banner> {
        let b = |kind: &str, message: String| {
            Some(Banner {
                kind: kind.to_string(),
                message,
            })
        };
        if !self.signed_in {
            return None;
        }
        if self.access_removed {
            return b(
                "accessRemoved",
                "You no longer have access to this server.".into(),
            );
        }
        match self.status {
            HealthStatus::NoAuth => b("signIn", "Please sign in again.".into()),
            HealthStatus::Starting | HealthStatus::Updating => {
                let late = self
                    .starting_since
                    .is_some_and(|s| now_ms - s >= START_GRACE_MS);
                if late {
                    b(
                        "startTimeout",
                        "Your server is taking too long to come back. Check that Plex is running."
                            .into(),
                    )
                } else if self.status == HealthStatus::Updating {
                    b("updating", "Server is updating…".into())
                } else {
                    b("starting", "Server is starting…".into())
                }
            }
            HealthStatus::Offline => {
                let msg = match self.last_seen_at {
                    Some(seen) => format!(
                        "Your server is offline. Last seen {}.",
                        ago(now_ms.saturating_sub(seen))
                    ),
                    None => "Your server is offline.".into(),
                };
                b("offline", msg)
            }
            HealthStatus::Relay => b(
                "relay",
                "Connected through Plex Relay, so quality is reduced. Turn on Remote Access in Plex for full quality."
                    .into(),
            ),
            HealthStatus::Local | HealthStatus::Remote => {
                if !self.files_available {
                    b(
                        "filesUnavailable",
                        "Your server is up, but your music files aren't available.".into(),
                    )
                } else {
                    None
                }
            }
        }
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
        }
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
        m.on_rediscover_failed(later);
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
        m.on_rediscover_failed(T0 + 200_000);
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
    fn probes_ignored_when_signed_out() {
        let mut m = HealthMachine::new();
        assert_eq!(
            m.on_probe(ProbeOutcome::Unauthorized, T0),
            HealthAction::None
        );
        assert_eq!(m.status(), HealthStatus::NoAuth);
    }
}
