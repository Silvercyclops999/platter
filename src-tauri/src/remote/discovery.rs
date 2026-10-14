//! Finding Plex players. Three sources, merged by client identifier:
//! - plex.tv resources that provide `player` (signed-in Plex apps),
//! - the server's `/clients` (players on the network that registered with
//!   the server; these accept commands proxied through the server),
//! - the Sonos relay's resources when the account has Sonos linked.
//!
//! The server's `/status/sessions` adds what each player is playing now.
//! All parsing is lenient: a source that fails or answers oddly is skipped.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use super::types::{PlayerKind, RemotePlayer};
use super::xml;
use crate::plex::Token;
use crate::plex::discovery::ResourceConnection;

/// A player and how to reach it.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub player: RemotePlayer,
    /// Base URLs for direct commands, local addresses first.
    pub direct: Vec<Url>,
    /// The player's own access token from plex.tv, for direct commands.
    pub token: Option<Token>,
    /// Registered with the server: commands can go through it.
    pub via_server: bool,
    /// Reached through the Sonos relay.
    pub sonos: bool,
    /// A Cast or AirPlay device on this network, driven by Platter itself.
    pub lan: Option<super::lan::LanDevice>,
    /// Another Platter device's control API.
    pub peer: Option<super::peer::PeerDevice>,
    /// When plex.tv last saw it (unix seconds).
    pub last_seen: Option<i64>,
}

fn s(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn b(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// The icon a player gets, from its product, platform and device names.
pub fn kind_of(product: &str, platform: &str, device: &str) -> PlayerKind {
    let all = format!("{product} {platform} {device}").to_ascii_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| all.contains(x));
    if has(&["sonos", "speaker", "echo", "homepod", "chromecast audio"]) {
        PlayerKind::Speaker
    } else if has(&["plex web", "chrome", "firefox", "safari", "edge", "opera", "browser"]) {
        PlayerKind::Browser
    } else if has(&["ios", "iphone", "ipad", "android"]) && !has(&["android tv", "androidtv", "shield", "fire tv"]) {
        PlayerKind::Phone
    } else if has(&[
        "tizen", "webos", "roku", "tvos", "apple tv", "android tv", "androidtv", "shield", "fire tv", "samsung", " lg",
        "vizio", "xbox", "playstation", "tv",
    ]) {
        PlayerKind::Tv
    } else {
        PlayerKind::Computer
    }
}

fn found(id: String, name: String, product: String, kind: PlayerKind) -> Found {
    Found {
        player: RemotePlayer {
            id,
            name,
            product,
            kind,
            playing: None,
            online: false,
            via: Default::default(),
            needs_password: false,
            unsupported: None,
            unreachable: false,
        },
        direct: Vec::new(),
        token: None,
        via_server: false,
        sonos: false,
        lan: None,
        peer: None,
        last_seen: None,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerResource {
    #[serde(default)]
    name: Value,
    #[serde(default)]
    product: Value,
    #[serde(default)]
    platform: Value,
    #[serde(default)]
    device: Value,
    #[serde(default)]
    client_identifier: Value,
    #[serde(default)]
    provides: Value,
    #[serde(default)]
    presence: Value,
    #[serde(default)]
    access_token: Value,
    #[serde(default)]
    connections: Vec<Value>,
    #[serde(default)]
    last_seen_at: Value,
}

/// plex.tv times: unix seconds, or an ISO date ("2026-10-14T09:30:00Z").
fn seen_at(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok().or_else(|| iso_seconds(s)),
        _ => None,
    }
}

/// Seconds since 1970 for "YYYY-MM-DDTHH:MM:SSZ" (UTC; fractions ignored).
fn iso_seconds(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from civil (Howard Hinnant).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se)
}

/// Players from plex.tv `GET /api/v2/resources` (JSON array). `own_id` (this
/// app) is left out.
pub fn from_resources(body: &[u8], own_id: &str) -> Vec<Found> {
    let Ok(list) = serde_json::from_slice::<Vec<PlayerResource>>(body) else {
        return Vec::new();
    };
    list.into_iter()
        .filter_map(|r| {
            let provides = s(&r.provides).unwrap_or_default();
            if !provides.split(',').any(|p| p.trim() == "player") {
                return None;
            }
            let id = s(&r.client_identifier)?;
            if id == own_id {
                return None;
            }
            let product = s(&r.product).unwrap_or_else(|| "Plex".into());
            let name = s(&r.name).unwrap_or_else(|| product.clone());
            let kind = kind_of(
                &product,
                &s(&r.platform).unwrap_or_default(),
                &s(&r.device).unwrap_or_default(),
            );
            let mut f = found(id, name, product, kind);
            f.player.online = b(&r.presence);
            f.last_seen = seen_at(&r.last_seen_at);
            f.token = s(&r.access_token).and_then(|t| Token::new(t).ok());
            let mut conns: Vec<ResourceConnection> = r
                .connections
                .into_iter()
                .filter_map(|c| serde_json::from_value(c).ok())
                .filter(|c: &ResourceConnection| !c.relay)
                .collect();
            // Local addresses first; plain http before https for the same
            // place (players answer the companion port over http).
            conns.sort_by_key(|c| (!c.local, c.protocol != "http"));
            for c in conns {
                if let Ok(u) = Url::parse(c.uri.trim_end_matches('/'))
                    && !f.direct.contains(&u)
                {
                    f.direct.push(u);
                }
            }
            Some(f)
        })
        .collect()
}

/// Players from the server's `GET /clients` (JSON `MediaContainer.Server`).
pub fn from_clients(body: &[u8], own_id: &str) -> Vec<Found> {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    let Some(list) = v["MediaContainer"]["Server"].as_array() else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|c| {
            let id = s(&c["machineIdentifier"])?;
            if id == own_id {
                return None;
            }
            let product = s(&c["product"]).unwrap_or_else(|| "Plex".into());
            let name = s(&c["name"]).unwrap_or_else(|| product.clone());
            let kind = kind_of(&product, &s(&c["platform"]).unwrap_or_default(), &s(&c["deviceClass"]).unwrap_or_default());
            let mut f = found(id, name, product, kind);
            f.player.online = true;
            f.via_server = true;
            let host = s(&c["address"]).or_else(|| s(&c["host"]));
            let port = s(&c["port"]);
            let proto = s(&c["protocol"]).filter(|p| p == "http" || p == "https").unwrap_or_else(|| "http".into());
            if let (Some(h), Some(p)) = (host, port)
                && let Ok(u) = Url::parse(&format!("{proto}://{h}:{p}"))
            {
                f.direct.push(u);
            }
            Some(f)
        })
        .collect()
}

/// Players from the Sonos relay's `GET /resources` (XML `Player` elements).
pub fn from_sonos(body: &str) -> Vec<Found> {
    xml::elements(body)
        .into_iter()
        .filter(|e| e.name == "Player")
        .filter_map(|e| {
            let id = e.get("machineIdentifier")?.to_string();
            let name = e.get("title").or(e.get("name")).unwrap_or("Sonos").to_string();
            let product = e.get("product").unwrap_or("Sonos").to_string();
            let mut f = found(id, name, product, PlayerKind::Speaker);
            f.player.online = true;
            f.sonos = true;
            Some(f)
        })
        .collect()
}

/// What each player is playing now, from `GET /status/sessions`: player id
/// to its grey line. Also says which players are active.
pub fn now_playing(body: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return out;
    };
    for m in v["MediaContainer"]["Metadata"].as_array().into_iter().flatten() {
        let Some(id) = s(&m["Player"]["machineIdentifier"]) else {
            continue;
        };
        let title = s(&m["title"]).unwrap_or_default();
        let artist = s(&m["originalTitle"]).or_else(|| s(&m["grandparentTitle"])).unwrap_or_default();
        let what = match (title.is_empty(), artist.is_empty()) {
            (false, false) => format!("{title} • {artist}"),
            (false, true) => title,
            _ => continue,
        };
        let paused = s(&m["Player"]["state"]).as_deref() == Some("paused");
        out.insert(id, if paused { format!("Paused: {what}") } else { what });
    }
    out
}

/// Merge the sources (earlier lists win for names) and add what's playing.
/// Online players come first, then by name.
pub fn merge(lists: Vec<Vec<Found>>, playing: &HashMap<String, String>) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    for f in lists.into_iter().flatten() {
        match out.iter_mut().find(|o| o.player.id == f.player.id) {
            Some(o) => {
                o.player.online |= f.player.online;
                o.via_server |= f.via_server;
                o.sonos |= f.sonos;
                if o.token.is_none() {
                    o.token = f.token;
                }
                o.last_seen = o.last_seen.max(f.last_seen);
                for u in f.direct {
                    if !o.direct.contains(&u) {
                        o.direct.push(u);
                    }
                }
            }
            None => out.push(f),
        }
    }
    for o in &mut out {
        if let Some(p) = playing.get(&o.player.id) {
            o.player.playing = Some(p.clone());
            o.player.online = true;
        }
    }
    out.sort_by(|a, b| {
        b.player
            .online
            .cmp(&a.player.online)
            .then_with(|| a.player.name.to_lowercase().cmp(&b.player.name.to_lowercase()))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resources() -> Vec<u8> {
        serde_json::to_vec(&json!([
            {"name": "Server", "product": "Plex Media Server", "provides": "server", "clientIdentifier": "srv"},
            {"name": "Desk", "product": "Plexamp", "platform": "Windows", "device": "Windows",
             "provides": "client,player,pubsub-player", "clientIdentifier": "amp-pc", "presence": false,
             "accessToken": "player-token",
             "connections": [
                {"protocol": "http", "address": "203.0.113.5", "port": 32500, "uri": "http://203.0.113.5:32500", "local": false},
                {"protocol": "http", "address": "192.0.2.20", "port": 32500, "uri": "http://192.0.2.20:32500", "local": true},
                {"protocol": "https", "address": "x", "port": 443, "uri": "https://relay.example", "relay": true}
             ]},
            {"name": "Phone", "product": "Plexamp", "platform": "iOS", "device": "iOS", "provides": "client,player",
             "clientIdentifier": "amp-phone", "presence": "1", "connections": []},
            {"name": "TV", "product": "Plex for Samsung", "platform": "Tizen", "provides": "client,player",
             "clientIdentifier": "tv", "connections": []},
            {"name": "Me", "product": "Platter", "provides": "player", "clientIdentifier": "me"}
        ]))
        .unwrap()
    }

    #[test]
    fn resources_keep_players_only_local_first_without_relay_or_self() {
        let f = from_resources(&resources(), "me");
        let ids: Vec<_> = f.iter().map(|f| f.player.id.as_str()).collect();
        assert_eq!(ids, ["amp-pc", "amp-phone", "tv"]);
        let pc = &f[0];
        assert_eq!(pc.direct[0].as_str(), "http://192.0.2.20:32500/");
        assert_eq!(pc.direct.len(), 2);
        assert!(pc.token.is_some());
        assert!(!pc.player.online);
        assert_eq!(pc.player.kind, PlayerKind::Computer);
        assert!(f[1].player.online);
        assert_eq!(f[1].player.kind, PlayerKind::Phone);
        assert_eq!(f[2].player.kind, PlayerKind::Tv);
        assert!(from_resources(b"not json", "me").is_empty());
    }

    #[test]
    fn clients_are_online_and_reachable_through_the_server() {
        let body = serde_json::to_vec(&json!({"MediaContainer": {"size": 2, "Server": [
            {"name": "Desk", "host": "192.0.2.20", "address": "192.0.2.20", "port": "32500",
             "machineIdentifier": "amp-pc", "product": "Plexamp", "protocol": "plex", "protocolCapabilities": "timeline,playback,playqueues"},
            {"name": "Web", "address": "192.0.2.30", "port": 32400, "machineIdentifier": "web", "product": "Plex Web", "platform": "Chrome"}
        ]}}))
        .unwrap();
        let f = from_clients(&body, "me");
        assert_eq!(f.len(), 2);
        assert!(f[0].via_server && f[0].player.online);
        assert_eq!(f[0].direct[0].as_str(), "http://192.0.2.20:32500/");
        assert_eq!(f[1].player.kind, PlayerKind::Browser);
        assert!(from_clients(br#"{"MediaContainer":{"size":0}}"#, "me").is_empty());
    }

    #[test]
    fn sonos_players_come_from_xml() {
        let x = r#"<MediaContainer size="1"><Player title="Kitchen" machineIdentifier="RINCON_1" product="Sonos" platform="Sonos"/></MediaContainer>"#;
        let f = from_sonos(x);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].player.name, "Kitchen");
        assert!(f[0].sonos);
        assert_eq!(f[0].player.kind, PlayerKind::Speaker);
    }

    #[test]
    fn sessions_give_the_grey_line() {
        let body = serde_json::to_vec(&json!({"MediaContainer": {"size": 2, "Metadata": [
            {"type": "track", "title": "Song", "grandparentTitle": "Band", "Player": {"machineIdentifier": "amp-pc", "state": "playing"}},
            {"type": "track", "title": "Other", "originalTitle": "Guest", "grandparentTitle": "Band", "Player": {"machineIdentifier": "tv", "state": "paused"}}
        ]}}))
        .unwrap();
        let p = now_playing(&body);
        assert_eq!(p["amp-pc"], "Song • Band");
        assert_eq!(p["tv"], "Paused: Other • Guest");
    }

    #[test]
    fn merge_unions_routes_and_sorts_online_first() {
        let clients = from_clients(
            &serde_json::to_vec(&json!({"MediaContainer": {"Server": [
                {"name": "Desk (LAN)", "address": "192.0.2.20", "port": 32500, "machineIdentifier": "amp-pc", "product": "Plexamp"}
            ]}}))
            .unwrap(),
            "me",
        );
        let res = from_resources(&resources(), "me");
        let mut playing = HashMap::new();
        playing.insert("tv".to_string(), "Song • Band".to_string());
        let m = merge(vec![clients, res], &playing);
        assert_eq!(m.len(), 3);
        let pc = m.iter().find(|f| f.player.id == "amp-pc").unwrap();
        assert_eq!(pc.player.name, "Desk (LAN)");
        assert!(pc.via_server && pc.token.is_some());
        assert_eq!(pc.direct.len(), 2, "the shared address is listed once");
        let tv = m.iter().find(|f| f.player.id == "tv").unwrap();
        assert!(tv.player.online);
        assert_eq!(tv.player.playing.as_deref(), Some("Song • Band"));
        // Offline last.
        assert!(m.iter().take(3).map(|f| f.player.online).collect::<Vec<_>>().windows(2).all(|w| w[0] >= w[1]));
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of("Plexamp", "Android", "Pixel"), PlayerKind::Phone);
        assert_eq!(kind_of("Plex for Android (TV)", "Android", "SHIELD Android TV"), PlayerKind::Tv);
        assert_eq!(kind_of("Plex for Windows", "Windows", "PC"), PlayerKind::Computer);
        assert_eq!(kind_of("Plex Web", "Firefox", ""), PlayerKind::Browser);
        assert_eq!(kind_of("Sonos", "Sonos", ""), PlayerKind::Speaker);
        assert_eq!(kind_of("Plex for LG", "webOS", ""), PlayerKind::Tv);
    }
}
