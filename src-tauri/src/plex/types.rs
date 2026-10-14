//! IPC shapes for auth, servers and health (docs/IPC.md "Auth & servers" and
//! "Health & app states"). camelCase on the wire; mirrored in
//! `src/lib/types.ts`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthState {
    SignedOut,
    Pending,
    SignedIn,
}

/// How the app reaches the server. Ordering = preference (local best).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionKind {
    Local,
    Remote,
    Relay,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub machine_id: String,
    pub name: String,
    pub owned: bool,
    pub online: bool,
    pub version: Option<String>,
    pub connection: Option<ConnectionKind>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub state: AuthState,
    pub username: Option<String>,
    /// The account's profile picture as an `/ext/` art path for the
    /// art:// scheme (never a plex.tv URL with a token).
    pub avatar: Option<String>,
    pub server: Option<ServerInfo>,
    pub needs_server_pick: bool,
    /// Plex rejected the saved sign-in. The app stays open (cached music
    /// still plays) with a "Please sign in again" banner.
    pub reauth_required: bool,
}

impl AuthStatus {
    pub fn signed_out() -> Self {
        AuthStatus {
            state: AuthState::SignedOut,
            username: None,
            avatar: None,
            server: None,
            needs_server_pick: false,
            reauth_required: false,
        }
    }
}

/// Returned by `auth_start`. `authUrl` contains only the client id and PIN
/// code — never a token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinInfo {
    pub code: String,
    pub auth_url: String,
    /// ISO 8601 from plex.tv, when present.
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HealthStatus {
    Local,
    Remote,
    Relay,
    Offline,
    Starting,
    Updating,
    NoAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub done: u32,
    pub total: u32,
}

/// Banner kinds: `offline`, `noInternet`, `starting`, `updating`,
/// `startTimeout`, `relay`, `signIn`, `accessRemoved`, `filesUnavailable`,
/// `noMusicLibrary`, `diskFull`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Banner {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub status: HealthStatus,
    pub server_name: Option<String>,
    pub connection: Option<ConnectionKind>,
    pub latency_ms: Option<u32>,
    pub server_version: Option<String>,
    /// Unix time in milliseconds of the last successful ping.
    pub last_seen_at: Option<i64>,
    pub scan: Option<ScanProgress>,
    pub banner: Option<Banner>,
    pub files_available: bool,
    /// Only music already on this computer can play (server offline, signed
    /// out by the server, or files unavailable). The UI dims the rest.
    pub cached_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Toast {
    /// "info" | "error"
    pub kind: String,
    pub message: String,
}

/// Event names (docs/IPC.md).
pub const EVENT_AUTH_CHANGED: &str = "auth://changed";
pub const EVENT_HEALTH_CHANGED: &str = "health://changed";
pub const EVENT_TOAST: &str = "toast";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_is_camel_case() {
        let s = AuthStatus {
            state: AuthState::SignedIn,
            username: Some("u".into()),
            avatar: Some("/ext/https://plex.tv/users/a/avatar".into()),
            server: Some(ServerInfo {
                machine_id: "m".into(),
                name: "n".into(),
                owned: true,
                online: true,
                version: None,
                connection: Some(ConnectionKind::Relay),
            }),
            needs_server_pick: false,
            reauth_required: true,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["state"], "signedIn");
        assert_eq!(v["needsServerPick"], false);
        assert_eq!(v["avatar"], "/ext/https://plex.tv/users/a/avatar");
        assert_eq!(v["reauthRequired"], true);
        assert_eq!(v["server"]["machineId"], "m");
        assert_eq!(v["server"]["connection"], "relay");
        let h = serde_json::to_value(HealthStatus::NoAuth).unwrap();
        assert_eq!(h, "noAuth");
    }

    #[test]
    fn connection_kind_orders_by_preference() {
        assert!(ConnectionKind::Local < ConnectionKind::Remote);
        assert!(ConnectionKind::Remote < ConnectionKind::Relay);
    }
}
