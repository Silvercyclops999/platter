//! Server discovery and the connection race.
//!
//! Servers come only from plex.tv's resources list: no
//! VPN or hard-coded address is ever assumed. Each server's connections are
//! probed in parallel at `/identity`; the winner is the best *class* that
//! answers (local > remote > relay), not simply the first responder — on the
//! real network remote answered faster than local (docs/PLEX-API.md). Once
//! any connection answers we wait a short grace window for a better class.
//!
//! The server list and last winner are cached to disk (no tokens) so the app
//! still connects when plex.tv is down.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde::{Deserialize, Deserializer, Serialize};
use tokio::time::Instant;

use super::auth::PlexTv;
use super::lenient;
use super::token::Token;
use super::types::{ConnectionKind, ServerInfo};
use crate::error::{AppError, AppResult};

const CACHE_FILE: &str = "plex-servers.json";

// ---------------------------------------------------------------------------
// plex.tv resources

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceConnection {
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub address: String,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    pub port: Option<i64>,
    pub uri: String,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub local: bool,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub relay: bool,
    #[serde(default, rename = "IPv6", deserialize_with = "lenient::bool")]
    pub ipv6: bool,
}

impl ResourceConnection {
    pub fn kind(&self) -> ConnectionKind {
        if self.relay {
            ConnectionKind::Relay
        } else if self.local {
            ConnectionKind::Local
        } else {
            ConnectionKind::Remote
        }
    }
}

fn de_opt_token<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Token>, D::Error> {
    let s = Option::<String>::deserialize(d)?;
    Ok(s.and_then(|s| Token::new(s).ok()))
}

/// One entry of `GET plex.tv/api/v2/resources`. `access_token` is a `Token`
/// so `Debug` never prints it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub product: String,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    pub product_version: Option<String>,
    #[serde(default)]
    pub client_identifier: String,
    #[serde(default)]
    pub provides: String,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub owned: bool,
    #[serde(default, deserialize_with = "lenient::bool")]
    pub presence: bool,
    #[serde(default, deserialize_with = "de_opt_token")]
    pub access_token: Option<Token>,
    #[serde(default)]
    pub connections: Vec<ResourceConnection>,
}

impl Resource {
    pub fn is_server(&self) -> bool {
        self.provides.split(',').any(|p| p.trim() == "server")
    }
}

// ---------------------------------------------------------------------------
// Servers and connections

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub uri: String,
    pub kind: ConnectionKind,
}

/// A server known from plex.tv (or from the disk cache).
#[derive(Debug, Clone)]
pub struct DiscoveredServer {
    pub machine_id: String,
    pub name: String,
    pub owned: bool,
    pub presence: bool,
    pub version: Option<String>,
    pub connections: Vec<Connection>,
    /// The server's own access token from resources. `None` for servers
    /// loaded from the cache (the state fills it from the keychain).
    pub access_token: Option<Token>,
}

impl DiscoveredServer {
    pub fn from_resource(r: Resource) -> Self {
        let mut connections: Vec<Connection> = Vec::new();
        for c in r.connections {
            let conn = Connection {
                uri: c.uri.trim_end_matches('/').to_string(),
                kind: c.kind(),
            };
            if !conn.uri.is_empty() && !connections.contains(&conn) {
                connections.push(conn);
            }
        }
        DiscoveredServer {
            machine_id: r.client_identifier,
            name: r.name,
            owned: r.owned,
            presence: r.presence,
            version: r.product_version,
            connections,
            access_token: r.access_token,
        }
    }

    pub fn info(&self, online: bool, connection: Option<ConnectionKind>) -> ServerInfo {
        ServerInfo {
            machine_id: self.machine_id.clone(),
            name: self.name.clone(),
            owned: self.owned,
            online,
            version: self.version.clone(),
            connection,
        }
    }
}

/// Servers from a resources response (only resources that provide "server").
pub fn servers_from_resources(resources: Vec<Resource>) -> Vec<DiscoveredServer> {
    resources
        .into_iter()
        .filter(Resource::is_server)
        .map(DiscoveredServer::from_resource)
        .collect()
}

/// Debug builds only: `PLEX_URL` replaces the discovered connections (local
/// development against a known address). Ignored in release builds.
pub fn dev_override() -> Option<Connection> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let raw = std::env::var("PLEX_URL").ok()?;
    let url = url::Url::parse(raw.trim()).ok()?;
    Some(Connection {
        uri: url.as_str().trim_end_matches('/').to_string(),
        kind: ConnectionKind::Local,
    })
}

#[derive(Debug, Clone)]
pub struct ServerList {
    pub servers: Vec<DiscoveredServer>,
    /// True when plex.tv was unreachable and the list came from disk.
    pub from_cache: bool,
}

/// Ask plex.tv for servers; if plex.tv is unreachable (network / 5xx), fall
/// back to the disk cache. Auth errors (revoked token) are returned as-is.
pub async fn load_servers(
    tv: &PlexTv,
    token: &Token,
    cache: &DiscoveryCache,
) -> AppResult<ServerList> {
    match tv.resources(token).await {
        Ok(resources) => Ok(ServerList {
            servers: servers_from_resources(resources),
            from_cache: false,
        }),
        Err(e @ (AppError::Network(_) | AppError::Server(_) | AppError::Decode(_)))
            if !cache.servers.is_empty() =>
        {
            tracing::warn!(kind = e.kind(), "plex.tv unavailable, using cached servers");
            Ok(ServerList {
                servers: cache
                    .servers
                    .iter()
                    .map(CachedServer::to_discovered)
                    .collect(),
                from_cache: true,
            })
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Connection race

/// A connection that answered `/identity`.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub uri: String,
    pub kind: ConnectionKind,
    pub latency: Duration,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct RaceConfig {
    /// Per-connection probe timeout and overall cap.
    pub timeout: Duration,
    /// After the first answer, how long to wait for a better class.
    pub grace: Duration,
}

impl Default for RaceConfig {
    fn default() -> Self {
        RaceConfig {
            timeout: Duration::from_secs(4),
            grace: Duration::from_millis(300),
        }
    }
}

/// Pure selection: best class first (local > remote > relay), then lowest
/// latency within the class.
pub fn select_best(answers: &[Probe]) -> Option<&Probe> {
    answers.iter().min_by_key(|p| (p.kind, p.latency))
}

#[derive(Deserialize)]
struct IdentityEnvelope {
    #[serde(rename = "MediaContainer")]
    media_container: IdentityBody,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityBody {
    #[serde(default)]
    machine_identifier: String,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    version: Option<String>,
}

/// Why a connection didn't answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeFail {
    /// The system refused the connection at once with "no route to host",
    /// "network unreachable" or "not permitted". On macOS this is what an app
    /// gets when Local Network privacy blocks it from the home network.
    Refused,
    /// Anything else: timeout, connection refused, TLS, bad answer, another
    /// server at the address.
    Other,
}

/// A refusal faster than this is the system saying no, not a host that
/// stopped answering (a missing host on the network takes seconds to fail).
pub const INSTANT_REFUSAL: Duration = Duration::from_millis(500);

/// Classify a failed request: an instant host/network-unreachable or
/// permission error anywhere in the error's source chain is a refusal.
pub fn classify_failure(err: &(dyn std::error::Error + 'static), elapsed: Duration) -> ProbeFail {
    if elapsed >= INSTANT_REFUSAL {
        return ProbeFail::Other;
    }
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = cur {
        if let Some(io) = e.downcast_ref::<std::io::Error>()
            && is_refusal(io)
        {
            return ProbeFail::Refused;
        }
        cur = e.source();
    }
    ProbeFail::Other
}

fn is_refusal(io: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        io.kind(),
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable | ErrorKind::PermissionDenied
    )
}

async fn probe_one(
    http: &reqwest::Client,
    conn: &Connection,
    expect_machine_id: Option<&str>,
    timeout: Duration,
) -> Result<Probe, ProbeFail> {
    let started = Instant::now();
    let resp = match http
        .get(format!("{}/identity", conn.uri))
        .timeout(timeout)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let fail = classify_failure(&e, started.elapsed());
            tracing::debug!(kind = ?conn.kind, ?fail, "connection failed");
            return Err(fail);
        }
    };
    if !resp.status().is_success() {
        return Err(ProbeFail::Other);
    }
    // Read like every other Plex reply, so a broken character doesn't turn a
    // server that answered into one that didn't.
    let bytes = resp.bytes().await.map_err(|_| ProbeFail::Other)?;
    let body: IdentityEnvelope = super::client::decode_json(&bytes).map_err(|_| ProbeFail::Other)?;
    let latency = started.elapsed();
    if let Some(expected) = expect_machine_id
        && body.media_container.machine_identifier != expected
    {
        // Address now belongs to another server (address changed).
        tracing::debug!(kind = ?conn.kind, "identity mismatch, ignoring connection");
        return Err(ProbeFail::Other);
    }
    Ok(Probe {
        uri: conn.uri.clone(),
        kind: conn.kind,
        latency,
        version: body.media_container.version,
    })
}

/// What a race found: the winner, and whether the system refused every
/// home-network address (see [`ProbeFail::Refused`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RaceOutcome {
    pub best: Option<Probe>,
    pub local_refused: bool,
}

/// Probe every connection in parallel and pick per [`select_best`]. Returns
/// as soon as the best possible class answers, else after the grace window
/// following the first answer, else when everything failed or timed out.
pub async fn race(
    http: &reqwest::Client,
    connections: &[Connection],
    expect_machine_id: Option<&str>,
    cfg: RaceConfig,
) -> Option<Probe> {
    race_detailed(http, connections, expect_machine_id, cfg).await.best
}

/// [`race`], also saying whether the home-network addresses were refused.
/// Remote and relay addresses are always raced alongside the local ones, so
/// a blocked home network falls back to them.
pub async fn race_detailed(
    http: &reqwest::Client,
    connections: &[Connection],
    expect_machine_id: Option<&str>,
    cfg: RaceConfig,
) -> RaceOutcome {
    let Some(best_possible) = connections.iter().map(|c| c.kind).min() else {
        return RaceOutcome::default();
    };
    let locals = connections.iter().filter(|c| c.kind == ConnectionKind::Local).count();
    let start = Instant::now();
    let hard_deadline = start + cfg.timeout;
    let mut pending: FuturesUnordered<_> = connections
        .iter()
        .map(|c| async move { (c.kind, probe_one(http, c, expect_machine_id, cfg.timeout).await) })
        .collect();
    let mut answers: Vec<Probe> = Vec::new();
    let mut local_refusals = 0usize;
    let mut grace_deadline: Option<Instant> = None;
    loop {
        let deadline = grace_deadline.map_or(hard_deadline, |g| g.min(hard_deadline));
        tokio::select! {
            next = pending.next() => match next {
                None => break,
                Some((_, Ok(p))) => {
                    let top = p.kind == best_possible;
                    answers.push(p);
                    if top {
                        break;
                    }
                    if grace_deadline.is_none() {
                        grace_deadline = Some(Instant::now() + cfg.grace);
                    }
                }
                Some((ConnectionKind::Local, Err(ProbeFail::Refused))) => local_refusals += 1,
                Some((_, Err(_))) => {}
            },
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }
    RaceOutcome {
        best: select_best(&answers).cloned(),
        local_refused: locals > 0 && local_refusals == locals,
    }
}

// ---------------------------------------------------------------------------
// Disk cache (no tokens)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedServer {
    pub machine_id: String,
    pub name: String,
    pub owned: bool,
    pub version: Option<String>,
    pub connections: Vec<Connection>,
    pub last_uri: Option<String>,
    pub last_kind: Option<ConnectionKind>,
}

impl CachedServer {
    pub fn from_discovered(s: &DiscoveredServer) -> Self {
        CachedServer {
            machine_id: s.machine_id.clone(),
            name: s.name.clone(),
            owned: s.owned,
            version: s.version.clone(),
            connections: s.connections.clone(),
            last_uri: None,
            last_kind: None,
        }
    }

    pub fn to_discovered(&self) -> DiscoveredServer {
        // Put the last winner first so it is tried with the others.
        let mut connections = self.connections.clone();
        if let (Some(uri), Some(kind)) = (&self.last_uri, self.last_kind) {
            let c = Connection {
                uri: uri.clone(),
                kind,
            };
            connections.retain(|x| x != &c);
            connections.insert(0, c);
        }
        DiscoveredServer {
            machine_id: self.machine_id.clone(),
            name: self.name.clone(),
            owned: self.owned,
            presence: true,
            version: self.version.clone(),
            connections,
            access_token: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCache {
    #[serde(default)]
    pub servers: Vec<CachedServer>,
    #[serde(default)]
    pub selected: Option<String>,
    /// Servers whose library settings were already checked.
    #[serde(default)]
    pub settings_checked: Vec<String>,
}

impl DiscoveryCache {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(CACHE_FILE)
    }

    /// Missing or unreadable cache → empty.
    pub fn load(dir: &Path) -> Self {
        std::fs::read(Self::path(dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> AppResult<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{CACHE_FILE}.tmp"));
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| AppError::Internal(format!("cache encode: {e}")))?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, Self::path(dir))?;
        Ok(())
    }

    /// Replace the server list, keeping last winners for servers still present.
    pub fn update_servers(&mut self, servers: &[DiscoveredServer]) {
        let old = std::mem::take(&mut self.servers);
        self.servers = servers
            .iter()
            .map(|s| {
                let mut c = CachedServer::from_discovered(s);
                if let Some(prev) = old.iter().find(|o| o.machine_id == s.machine_id) {
                    c.last_uri = prev.last_uri.clone();
                    c.last_kind = prev.last_kind;
                }
                c
            })
            .collect();
    }

    pub fn record_winner(&mut self, machine_id: &str, probe: &Probe) {
        if let Some(s) = self.servers.iter_mut().find(|s| s.machine_id == machine_id) {
            s.last_uri = Some(probe.uri.clone());
            s.last_kind = Some(probe.kind);
            if probe.version.is_some() {
                s.version = probe.version.clone();
            }
        }
        self.selected = Some(machine_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::identity::ClientIdentity;
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn probe(kind: ConnectionKind, ms: u64) -> Probe {
        Probe {
            uri: format!("{kind:?}-{ms}"),
            kind,
            latency: Duration::from_millis(ms),
            version: None,
        }
    }

    #[test]
    fn select_prefers_class_over_latency() {
        use ConnectionKind::*;
        let a = [probe(Remote, 117), probe(Local, 155), probe(Relay, 1500)];
        assert_eq!(select_best(&a).unwrap().kind, Local);
        let b = [probe(Relay, 50), probe(Remote, 900)];
        assert_eq!(select_best(&b).unwrap().kind, Remote);
        let c = [probe(Relay, 1500)];
        assert_eq!(select_best(&c).unwrap().kind, Relay);
        let d = [probe(Remote, 300), probe(Remote, 100)];
        assert_eq!(select_best(&d).unwrap().latency, Duration::from_millis(100));
        assert!(select_best(&[]).is_none());
    }

    #[test]
    fn resources_parse_and_filter_servers() {
        let json = serde_json::json!([
            {"name": "Home", "product": "Plex Media Server", "productVersion": "1.43.4",
             "clientIdentifier": "mid1", "provides": "server", "owned": true, "presence": true,
             "accessToken": "srv-secret-token",
             "connections": [
                {"protocol": "https", "address": "10.0.0.2", "port": 32400, "uri": "https://a.plex.direct:32400", "local": true, "relay": false, "IPv6": false},
                {"protocol": "https", "address": "1.2.3.4", "port": 1234, "uri": "https://b.plex.direct:1234", "local": false, "relay": false, "IPv6": false},
                {"protocol": "https", "address": "5.6.7.8", "port": 8443, "uri": "https://c.plex.direct:8443/", "local": false, "relay": true, "IPv6": false}
             ]},
            {"name": "Phone", "product": "Plexamp", "clientIdentifier": "p1", "provides": "client,player", "connections": []}
        ]);
        let resources: Vec<Resource> = serde_json::from_value(json).unwrap();
        let dbg = format!("{resources:?}");
        assert!(!dbg.contains("srv-secret-token"), "Debug leaked token");
        let servers = servers_from_resources(resources);
        assert_eq!(servers.len(), 1);
        let s = &servers[0];
        assert_eq!(s.machine_id, "mid1");
        assert_eq!(s.version.as_deref(), Some("1.43.4"));
        let kinds: Vec<_> = s.connections.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ConnectionKind::Local,
                ConnectionKind::Remote,
                ConnectionKind::Relay
            ]
        );
        assert_eq!(s.connections[2].uri, "https://c.plex.direct:8443");
        assert_eq!(
            s.access_token.as_ref().unwrap().expose(),
            "srv-secret-token"
        );
    }

    async fn identity_server(delay_ms: u64, machine: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/identity"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(delay_ms))
                    .set_body_json(serde_json::json!({
                        "MediaContainer": {"size": 0, "claimed": true, "machineIdentifier": machine, "version": "1.43.4"}
                    })),
            )
            .mount(&server)
            .await;
        server
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .default_headers(ClientIdentity::with_id("t").headers().unwrap())
            .build()
            .unwrap()
    }

    fn conn(s: &MockServer, kind: ConnectionKind) -> Connection {
        Connection { uri: s.uri(), kind }
    }

    /// A dead address: bound then dropped, so connects are refused.
    fn dead() -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn slow_local_within_grace_beats_fast_remote() {
        let local = identity_server(150, "m").await;
        let remote = identity_server(10, "m").await;
        let relay = identity_server(0, "m").await;
        let conns = [
            conn(&relay, ConnectionKind::Relay),
            conn(&remote, ConnectionKind::Remote),
            conn(&local, ConnectionKind::Local),
        ];
        let cfg = RaceConfig {
            timeout: Duration::from_secs(20),
            grace: Duration::from_secs(5),
        };
        let p = race(&http(), &conns, Some("m"), cfg).await.unwrap();
        assert_eq!(p.kind, ConnectionKind::Local);
        assert_eq!(p.version.as_deref(), Some("1.43.4"));
    }

    #[tokio::test]
    async fn local_slower_than_grace_loses_to_remote() {
        let local = identity_server(10_000, "m").await;
        let remote = identity_server(10, "m").await;
        let conns = [
            conn(&local, ConnectionKind::Local),
            conn(&remote, ConnectionKind::Remote),
        ];
        let cfg = RaceConfig {
            timeout: Duration::from_secs(20),
            grace: Duration::from_millis(100),
        };
        let started = std::time::Instant::now();
        let p = race(&http(), &conns, Some("m"), cfg).await.unwrap();
        assert_eq!(p.kind, ConnectionKind::Remote);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "should not wait for local"
        );
    }

    /// A network whose DNS answers nothing (rebind protection, a filter):
    /// the plex.direct connection still wins, by the address in its name.
    #[tokio::test]
    async fn plex_direct_answers_when_dns_gives_nothing() {
        use crate::plex::direct::Resolver;
        use reqwest::dns::Addrs;
        let local = identity_server(0, "m").await;
        let port = url::Url::parse(&local.uri()).unwrap().port().unwrap();
        let no_dns = Resolver::with_lookup(|_| Box::pin(async { Ok(Box::new(std::iter::empty()) as Addrs) }));
        let http = reqwest::Client::builder()
            .default_headers(ClientIdentity::with_id("t").headers().unwrap())
            .no_proxy()
            .dns_resolver(no_dns)
            .build()
            .unwrap();
        let conns = [Connection {
            uri: format!("http://127-0-0-1.example.plex.direct:{port}"),
            kind: ConnectionKind::Local,
        }];
        let p = race(&http, &conns, Some("m"), RaceConfig::default()).await.unwrap();
        assert_eq!(p.kind, ConnectionKind::Local);
    }

    #[tokio::test]
    async fn relay_only_when_nothing_else_answers() {
        let relay = identity_server(50, "m").await;
        let conns = [
            Connection {
                uri: dead(),
                kind: ConnectionKind::Local,
            },
            Connection {
                uri: dead(),
                kind: ConnectionKind::Remote,
            },
            conn(&relay, ConnectionKind::Relay),
        ];
        let p = race(&http(), &conns, Some("m"), RaceConfig::default())
            .await
            .unwrap();
        assert_eq!(p.kind, ConnectionKind::Relay);
    }

    #[tokio::test]
    async fn remote_preferred_over_faster_relay() {
        let remote = identity_server(120, "m").await;
        let relay = identity_server(0, "m").await;
        let conns = [
            conn(&relay, ConnectionKind::Relay),
            conn(&remote, ConnectionKind::Remote),
        ];
        let p = race(&http(), &conns, None, RaceConfig { timeout: Duration::from_secs(20), grace: Duration::from_secs(5) })
            .await
            .unwrap();
        assert_eq!(p.kind, ConnectionKind::Remote);
    }

    #[tokio::test]
    async fn nothing_answers_gives_none_within_timeout() {
        let slow = identity_server(30_000, "m").await;
        let conns = [
            Connection {
                uri: dead(),
                kind: ConnectionKind::Local,
            },
            conn(&slow, ConnectionKind::Remote),
        ];
        let cfg = RaceConfig {
            timeout: Duration::from_millis(300),
            grace: Duration::from_millis(100),
        };
        let started = std::time::Instant::now();
        assert!(race(&http(), &conns, None, cfg).await.is_none());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// An error that wraps another, like reqwest wrapping hyper wrapping io.
    #[derive(Debug)]
    struct Wrapped(std::io::Error);
    impl std::fmt::Display for Wrapped {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("request failed")
        }
    }
    impl std::error::Error for Wrapped {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn instant_no_route_or_not_permitted_is_a_refusal() {
        use std::io::{Error, ErrorKind};
        let fast = Duration::from_millis(3);
        for kind in [ErrorKind::HostUnreachable, ErrorKind::NetworkUnreachable, ErrorKind::PermissionDenied] {
            let e = Wrapped(Error::from(kind));
            assert_eq!(classify_failure(&e, fast), ProbeFail::Refused, "{kind:?}");
        }
        // A host that's just gone takes seconds to say no route.
        let slow = Wrapped(Error::from(ErrorKind::HostUnreachable));
        assert_eq!(classify_failure(&slow, Duration::from_secs(3)), ProbeFail::Other);
        // A server that's down refuses the port; nothing is blocking us.
        for kind in [ErrorKind::ConnectionRefused, ErrorKind::TimedOut, ErrorKind::ConnectionReset] {
            let e = Wrapped(Error::from(kind));
            assert_eq!(classify_failure(&e, fast), ProbeFail::Other, "{kind:?}");
        }
    }

    #[tokio::test]
    async fn a_closed_port_is_not_a_blocked_network() {
        let remote = identity_server(0, "m").await;
        let conns = [
            Connection {
                uri: dead(),
                kind: ConnectionKind::Local,
            },
            conn(&remote, ConnectionKind::Remote),
        ];
        let out = race_detailed(&http(), &conns, Some("m"), RaceConfig::default()).await;
        assert_eq!(out.best.unwrap().kind, ConnectionKind::Remote);
        assert!(!out.local_refused);
        // No local addresses at all: nothing to be blocked from.
        let only_remote = [conn(&remote, ConnectionKind::Remote)];
        let out = race_detailed(&http(), &only_remote, Some("m"), RaceConfig::default()).await;
        assert!(!out.local_refused);
    }

    #[tokio::test]
    async fn wrong_machine_is_ignored() {
        let other = identity_server(0, "someone-else").await;
        let conns = [conn(&other, ConnectionKind::Local)];
        assert!(
            race(&http(), &conns, Some("m"), RaceConfig::default())
                .await
                .is_none()
        );
    }

    #[test]
    fn cache_round_trip_has_no_token_and_keeps_winner() {
        let dir = tempfile::tempdir().unwrap();
        let s = DiscoveredServer {
            machine_id: "m".into(),
            name: "Home".into(),
            owned: true,
            presence: true,
            version: Some("1".into()),
            connections: vec![
                Connection {
                    uri: "https://l".into(),
                    kind: ConnectionKind::Local,
                },
                Connection {
                    uri: "https://r".into(),
                    kind: ConnectionKind::Remote,
                },
            ],
            access_token: Some(Token::new("cache-secret").unwrap()),
        };
        let mut cache = DiscoveryCache::default();
        cache.update_servers(std::slice::from_ref(&s));
        cache.record_winner(
            "m",
            &Probe {
                uri: "https://r".into(),
                kind: ConnectionKind::Remote,
                latency: Duration::from_millis(5),
                version: Some("2".into()),
            },
        );
        cache.save(dir.path()).unwrap();
        let raw = std::fs::read_to_string(DiscoveryCache::path(dir.path())).unwrap();
        assert!(!raw.contains("cache-secret"));
        let loaded = DiscoveryCache::load(dir.path());
        assert_eq!(loaded, cache);
        assert_eq!(loaded.selected.as_deref(), Some("m"));
        let d = loaded.servers[0].to_discovered();
        assert_eq!(d.connections[0].uri, "https://r");
        assert_eq!(d.connections.len(), 2);
        assert_eq!(d.version.as_deref(), Some("2"));
        // refreshing the list keeps the winner
        cache.update_servers(&[s]);
        assert_eq!(cache.servers[0].last_uri.as_deref(), Some("https://r"));
    }

    #[test]
    fn missing_cache_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(DiscoveryCache::load(dir.path()), DiscoveryCache::default());
    }

    #[tokio::test]
    async fn plex_tv_unreachable_uses_cached_servers() {
        let tv = PlexTv::with_base(Arc::new(ClientIdentity::with_id("c")), &dead()).unwrap();
        let mut cache = DiscoveryCache::default();
        cache.servers.push(CachedServer {
            machine_id: "m".into(),
            name: "Home".into(),
            owned: true,
            version: None,
            connections: vec![],
            last_uri: Some("https://l".into()),
            last_kind: Some(ConnectionKind::Local),
        });
        let tok = Token::new("t").unwrap();
        let list = load_servers(&tv, &tok, &cache).await.unwrap();
        assert!(list.from_cache);
        assert_eq!(list.servers[0].machine_id, "m");
        assert_eq!(list.servers[0].connections[0].uri, "https://l");
        // with no cache the network error surfaces
        let err = load_servers(&tv, &tok, &DiscoveryCache::default())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "network");
    }

    #[tokio::test]
    async fn plex_tv_401_is_not_masked_by_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/resources"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let tv = PlexTv::with_base(Arc::new(ClientIdentity::with_id("c")), &server.uri()).unwrap();
        let mut cache = DiscoveryCache::default();
        cache.servers.push(CachedServer {
            machine_id: "m".into(),
            name: "Home".into(),
            owned: true,
            version: None,
            connections: vec![],
            last_uri: None,
            last_kind: None,
        });
        let err = load_servers(&tv, &Token::new("t").unwrap(), &cache)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "auth");
    }
}
