//! Commands to a Plex player (the companion protocol): `GET
//! /player/<controller>/<command>?…&commandID=N` with the target's id in
//! `X-Plex-Target-Client-Identifier`. A command can go straight to the
//! player's address, through the server (players registered in `/clients`),
//! or through the Sonos relay. Each player remembers the route that last
//! worked and tries it first.
//!
//! The token travels only in the `X-Plex-Token` header (sensitive), except
//! the server token inside `playMedia`, which the protocol needs so the player
//! can reach the server; that URL is never logged.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::Value;
use url::Url;

use super::discovery::Found;
use super::xml;
use crate::error::{AppError, AppResult};
use crate::plex::Token;
use crate::plex::client::map_send_error;

/// Where a command goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// The player's own address.
    Direct { base: Url, token: Option<Token> },
    /// The selected server, which forwards to a registered player.
    Server { base: Url, token: Token },
    /// The Sonos relay.
    Relay { base: Url, token: Token },
}

impl Route {
    fn key(&self) -> String {
        match self {
            Route::Direct { base, .. } => format!("d:{base}"),
            Route::Server { .. } => "s".into(),
            Route::Relay { .. } => "r".into(),
        }
    }
}

/// The routes to try for a player, best guess first.
pub fn routes(f: &Found, server: Option<(&Url, &Token)>, relay: Option<(&Url, &Token)>) -> Vec<Route> {
    let mut out = Vec::new();
    if f.sonos {
        if let Some((base, token)) = relay {
            out.push(Route::Relay { base: base.clone(), token: token.clone() });
        }
        return out;
    }
    for base in &f.direct {
        out.push(Route::Direct {
            base: base.clone(),
            // A player's own token when plex.tv gave one, else the server's.
            token: f.token.clone().or_else(|| server.map(|(_, t)| t.clone())),
        });
    }
    if let Some((base, token)) = server {
        // Registered players accept it for sure; others may (worth a try
        // when there's no address at all).
        if f.via_server || f.direct.is_empty() {
            out.push(Route::Server { base: base.clone(), token: token.clone() });
        }
    }
    out
}

/// A music timeline from `/player/timeline/poll`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timeline {
    /// playing, paused, buffering, stopped.
    pub state: String,
    pub time_ms: u64,
    pub duration_ms: u64,
    pub rating_key: Option<String>,
    pub play_queue_id: Option<String>,
    pub play_queue_item_id: Option<String>,
    /// Bumped by the server whenever the play queue changes.
    pub play_queue_version: Option<String>,
    /// 0..100.
    pub volume: Option<i64>,
    pub shuffle: bool,
    pub repeat: i64,
}

fn rating_key_from(key: Option<&str>) -> Option<String> {
    key.and_then(|k| k.rsplit('/').next()).filter(|k| !k.is_empty()).map(str::to_string)
}

/// The music timeline in a poll answer (XML, or JSON from some players).
/// None when the player reports no music timeline at all.
pub fn parse_timeline(body: &str) -> Option<Timeline> {
    let t = body.trim_start();
    if t.starts_with('{') {
        let v: Value = serde_json::from_str(t).ok()?;
        let list = v["MediaContainer"]["Timeline"].as_array()?;
        let m = list.iter().find(|x| x["type"].as_str() == Some("music"))?;
        let int = |k: &str| match &m[k] {
            Value::Number(n) => n.as_f64().map(|f| f as i64),
            Value::String(s) => s.parse::<f64>().ok().map(|f| f as i64),
            _ => None,
        };
        let str_ = |k: &str| match &m[k] {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        return Some(Timeline {
            state: str_("state").unwrap_or_else(|| "stopped".into()),
            time_ms: int("time").unwrap_or(0).max(0) as u64,
            duration_ms: int("duration").unwrap_or(0).max(0) as u64,
            rating_key: str_("ratingKey").or_else(|| rating_key_from(str_("key").as_deref())),
            play_queue_id: str_("playQueueID"),
            play_queue_item_id: str_("playQueueItemID"),
            play_queue_version: str_("playQueueVersion"),
            volume: int("volume"),
            shuffle: int("shuffle").unwrap_or(0) != 0,
            repeat: int("repeat").unwrap_or(0),
        });
    }
    let els = xml::elements(t);
    let m = els.iter().find(|e| e.name == "Timeline" && e.get("type") == Some("music"))?;
    Some(Timeline {
        state: m.get("state").unwrap_or("stopped").to_string(),
        time_ms: m.int("time").unwrap_or(0).max(0) as u64,
        duration_ms: m.int("duration").unwrap_or(0).max(0) as u64,
        rating_key: m
            .get("ratingKey")
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| rating_key_from(m.get("key"))),
        play_queue_id: m.get("playQueueID").filter(|s| !s.is_empty()).map(str::to_string),
        play_queue_item_id: m.get("playQueueItemID").filter(|s| !s.is_empty()).map(str::to_string),
        play_queue_version: m.get("playQueueVersion").filter(|s| !s.is_empty()).map(str::to_string),
        volume: m.int("volume"),
        shuffle: m.int("shuffle").unwrap_or(0) != 0,
        repeat: m.int("repeat").unwrap_or(0),
    })
}

/// Sends companion commands; one per app.
pub struct Companion {
    http: reqwest::Client,
    command_id: AtomicU64,
    /// Player id -> the route key that last worked.
    preferred: Mutex<HashMap<String, String>>,
}

impl Companion {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            command_id: AtomicU64::new(0),
            preferred: Mutex::default(),
        }
    }

    /// Next `commandID` (counts up from 1 for the app's lifetime).
    pub fn next_command_id(&self) -> u64 {
        self.command_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    async fn send_one(&self, route: &Route, target: &str, path: &str, params: &[(&str, String)]) -> AppResult<String> {
        let (base, token) = match route {
            Route::Direct { base, token } => (base, token.as_ref()),
            Route::Server { base, token } | Route::Relay { base, token } => (base, Some(token)),
        };
        let mut url = base.clone();
        let joined = format!("{}{}", base.path().trim_end_matches('/'), path);
        url.set_path(&joined);
        let mut q: Vec<(&str, String)> = params.to_vec();
        q.push(("commandID", self.next_command_id().to_string()));
        let mut rb = self
            .http
            .get(url)
            .query(&q)
            .header("x-plex-target-client-identifier", target);
        if let Some(t) = token {
            rb = rb.header("x-plex-token", t.header_value()?);
        }
        let resp = rb.send().await.map_err(map_send_error)?;
        let status = resp.status();
        let body = resp.text().await.map_err(map_send_error)?;
        if !status.is_success() {
            return Err(AppError::Invalid(format!("The player refused the command ({}).", status.as_u16())));
        }
        // Some players answer 200 with an error document.
        if let Some(r) = xml::elements(&body).into_iter().find(|e| e.name == "Response")
            && let Some(code) = r.int("code")
            && code >= 400
        {
            return Err(AppError::Invalid(format!("The player refused the command ({code}).")));
        }
        Ok(body)
    }

    /// Send `path` to `target` over the first route that answers, starting
    /// with the one that worked last time.
    pub async fn send(&self, target: &str, routes: &[Route], path: &str, params: &[(&str, String)]) -> AppResult<String> {
        if routes.is_empty() {
            return Err(AppError::Network("No way to reach the player.".into()));
        }
        let pref = self.preferred.lock().get(target).cloned();
        let mut order: Vec<&Route> = routes.iter().collect();
        if let Some(p) = pref {
            order.sort_by_key(|r| r.key() != p);
        }
        let mut last = AppError::Network("Couldn't connect.".into());
        for r in order {
            match self.send_one(r, target, path, params).await {
                Ok(body) => {
                    self.preferred.lock().insert(target.to_string(), r.key());
                    return Ok(body);
                }
                Err(e) => {
                    tracing::debug!(kind = e.kind(), path, "player command failed on one route");
                    last = e;
                }
            }
        }
        Err(last)
    }
}

/// HTTP client for player commands: identity headers, short timeouts.
pub fn http_client(identity: &crate::plex::identity::ClientIdentity, timeout: Duration) -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .default_headers(identity.headers()?)
        .dns_resolver(crate::plex::direct::Resolver::system())
        .connect_timeout(timeout.min(Duration::from_secs(4)))
        .timeout(timeout)
        .build()
        .map_err(|e| AppError::Internal(format!("http client: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::types::{PlayerKind, RemotePlayer};

    fn f(direct: &[&str], via_server: bool, sonos: bool) -> Found {
        Found {
            player: RemotePlayer {
                id: "p".into(),
                name: "P".into(),
                product: "Plexamp".into(),
                kind: PlayerKind::Computer,
                playing: None,
                online: true,
                via: Default::default(),
                needs_password: false,
                unsupported: None,
                unreachable: false,
                asleep: false,
            },
            direct: direct.iter().map(|u| Url::parse(u).unwrap()).collect(),
            token: None,
            via_server,
            sonos,
            lan: None,
            peer: None,
            last_seen: None,
        }
    }

    #[test]
    fn routes_try_direct_then_the_server() {
        let srv = Url::parse("http://srv:32400").unwrap();
        let tok = Token::new("t").unwrap();
        let r = routes(&f(&["http://a:32500"], true, false), Some((&srv, &tok)), None);
        assert!(matches!(r[0], Route::Direct { .. }));
        assert!(matches!(r[1], Route::Server { .. }));
        // Not registered, but has an address: direct only.
        assert_eq!(routes(&f(&["http://a:32500"], false, false), Some((&srv, &tok)), None).len(), 1);
        // No address at all: the server is the only try.
        assert!(matches!(routes(&f(&[], false, false), Some((&srv, &tok)), None)[0], Route::Server { .. }));
        // Sonos goes through the relay only.
        let relay = Url::parse("https://relay").unwrap();
        let r = routes(&f(&["http://a"], true, true), Some((&srv, &tok)), Some((&relay, &tok)));
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0], Route::Relay { .. }));
    }

    #[test]
    fn timeline_from_xml_and_json() {
        let x = r#"<MediaContainer commandID="3"><Timeline type="video" state="stopped"/><Timeline type="music" state="paused" time="12000" duration="180000" key="/library/metadata/77" playQueueID="9" playQueueItemID="901" volume="40" shuffle="1" repeat="2"/></MediaContainer>"#;
        let t = parse_timeline(x).unwrap();
        assert_eq!(t.state, "paused");
        assert_eq!(t.time_ms, 12_000);
        assert_eq!(t.rating_key.as_deref(), Some("77"));
        assert_eq!(t.play_queue_id.as_deref(), Some("9"));
        assert_eq!((t.volume, t.shuffle, t.repeat), (Some(40), true, 2));
        let j = r#"{"MediaContainer":{"Timeline":[{"type":"music","state":"playing","time":5000,"duration":"9000","ratingKey":"5"}]}}"#;
        let t = parse_timeline(j).unwrap();
        assert_eq!((t.state.as_str(), t.time_ms, t.duration_ms), ("playing", 5_000, 9_000));
        assert_eq!(t.rating_key.as_deref(), Some("5"));
        assert!(parse_timeline("<MediaContainer/>").is_none());
    }

    #[test]
    fn command_ids_count_up() {
        let c = Companion::new(reqwest::Client::new());
        assert_eq!(c.next_command_id(), 1);
        assert_eq!(c.next_command_id(), 2);
    }
}
