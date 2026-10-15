//! Platter devices finding and controlling each other on the local network.
//!
//! With "Show this computer to my other devices" on (the default), Platter
//! announces itself over mDNS as `_platter._tcp` and answers a small control
//! API over HTTP on the first free port from [`FIRST_PORT`], bound only to
//! this computer's private LAN addresses. Other Platter devices signed in to
//! the same Plex account list it first in the device picker and can move
//! playback to it and back.
//!
//! - [`auth`]: the request signatures (same Plex account only, no replays).
//! - [`devices`]: the keys of the account's other devices, from plex.tv.
//! - [`server`]: the receiving side (the local player as a target).
//! - [`client`]: the sending side, and the link the Listen on service
//!   follows.
//! - [`directory`]: announcing and browsing over mDNS (and a fake network
//!   for tests).
//! - [`publish`]: telling plex.tv where this computer answers.
//! - [`keys`]: the requests that copy keys between devices.

pub mod auth;
pub mod client;
pub mod devices;
pub mod directory;
pub mod keys;
pub mod publish;
pub mod runtime;
pub mod server;


use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use serde::{Deserialize, Serialize};
use url::Url;

use super::lan::MdnsAnswer;
use crate::audio::queue::{ContextInfo, Repeat};

/// The mDNS service type.
pub const SERVICE: &str = "_platter._tcp.local.";
/// The control API listens on the first free port from here.
pub const FIRST_PORT: u16 = 32520;
pub const PORT_TRIES: u16 = 10;
/// The product name other Platter devices have in Plex.
pub const PRODUCT: &str = "Platter";

pub const PATH_HELLO: &str = "/platter/v1/hello";
pub const PATH_STATE: &str = "/platter/v1/state";
pub const PATH_PLAY: &str = "/platter/v1/play";
pub const PATH_COMMAND: &str = "/platter/v1/command";
pub const PATH_STOP: &str = "/platter/v1/stop";

/// `GET /platter/v1/hello`: who answers, and what it's playing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub version: String,
    /// "Song • Artist", with "Paused: " in front when paused.
    pub playing: Option<String>,
}

/// `GET /platter/v1/state`: the playback the sender follows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerTimeline {
    /// playing, paused, buffering, stopped.
    pub state: String,
    pub time_ms: u64,
    pub duration_ms: u64,
    pub track_id: Option<String>,
    /// 0..1.
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: Repeat,
}

/// `POST /platter/v1/play`: take over this queue at this spot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRequest {
    /// The Plex server the songs are on (machine id); the receiver plays
    /// from its own connection to it.
    pub server_id: String,
    pub current: String,
    #[serde(default)]
    pub user_queue: Vec<String>,
    #[serde(default)]
    pub next_up: Vec<String>,
    #[serde(default)]
    pub context: Option<ContextInfo>,
    pub position_ms: u64,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub play: bool,
}

/// The most songs one move sends (current, user queue and next up).
pub const MAX_SONGS: usize = 500;

/// Another Platter device and where it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    /// Control API base URLs, IPv4 first.
    pub bases: Vec<Url>,
}

impl PeerDevice {
    pub fn ips(&self) -> Vec<IpAddr> {
        self.bases
            .iter()
            .filter_map(|u| u.host_str())
            .filter_map(|h| h.trim_matches(|c| c == '[' || c == ']').parse().ok())
            .collect()
    }
}

/// A `_platter._tcp` answer as a device of this account: the account tag
/// must match and it mustn't be this computer.
pub fn from_mdns(a: &MdnsAnswer, own_id: &str, account_tag: &str) -> Option<PeerDevice> {
    if !a.service.starts_with("_platter.") {
        return None;
    }
    let id = a.txt("id")?.to_string();
    if id == own_id || a.txt("acct") != Some(account_tag) {
        return None;
    }
    let mut addrs: Vec<IpAddr> = a.addrs.clone();
    addrs.sort_by_key(|ip| !ip.is_ipv4());
    let bases = addrs
        .into_iter()
        .filter(|ip| !ip.is_loopback() || cfg!(test))
        .filter_map(|ip| Url::parse(&format!("http://{}", SocketAddr::new(ip, a.port))).ok())
        .collect::<Vec<_>>();
    if bases.is_empty() {
        return None;
    }
    let name = a.txt("name").unwrap_or(&a.instance).chars().take(60).collect();
    Some(PeerDevice {
        id,
        name,
        platform: a.txt("platform").unwrap_or_default().to_string(),
        bases,
    })
}

/// The TXT record this computer announces.
pub fn txt(id: &str, name: &str, platform: &str, version: &str, account_tag: &str) -> Vec<(String, String)> {
    vec![
        ("id".into(), id.into()),
        ("name".into(), name.chars().take(60).collect()),
        ("platform".into(), platform.into()),
        ("version".into(), version.into()),
        ("acct".into(), account_tag.into()),
    ]
}

/// A private LAN address the control API may listen on: the RFC 1918
/// ranges and link-local. Not loopback, not the shared range VPNs such
/// as Tailscale use (100.64/10), and nothing public.
pub fn is_lan(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local()
}

/// This computer's LAN IPv4 addresses.
pub fn lan_addrs() -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            IpAddr::V4(v4) if is_lan(v4) => Some(v4),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The OS computer name ("Studio PC", "Adam's MacBook Air").
pub fn os_computer_name() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("scutil").args(["--get", "ComputerName"]).output()
            && out.status.success()
        {
            let n = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !n.is_empty() {
                return n;
            }
        }
    }
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    if let Ok(out) = std::process::Command::new("hostname").output()
        && out.status.success()
    {
        let n = String::from_utf8_lossy(&out.stdout).trim().trim_end_matches(".local").to_string();
        if !n.is_empty() {
            return n;
        }
    }
    "This computer".into()
}

/// The name to show: the setting, or the OS name when it's empty.
pub fn display_name(setting: &str) -> String {
    let s = setting.trim();
    if s.is_empty() { os_computer_name() } else { s.chars().take(60).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::lan::answer;

    #[test]
    fn only_private_lan_addresses_are_listened_on() {
        for ip in ["10.0.0.5", "172.16.4.2", "169.254.10.1"] {
            assert!(is_lan(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["127.0.0.1", "100.64.0.7", "192.0.2.5", "8.8.8.8", "0.0.0.0"] {
            assert!(!is_lan(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn announcements_of_this_account_become_devices() {
        let tag = auth::account_tag("acct");
        let txt = txt("mac", "Studio Mac", "macOS", "0.3.0", &tag);
        let pairs: Vec<(&str, &str)> = txt.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let a = answer(SERVICE, "mac", "192.0.2.7", 32520, &pairs);
        let d = from_mdns(&a, "pc", &tag).unwrap();
        assert_eq!((d.id.as_str(), d.name.as_str(), d.platform.as_str()), ("mac", "Studio Mac", "macOS"));
        assert_eq!(d.bases[0].as_str(), "http://192.0.2.7:32520/");
        assert_eq!(d.ips(), vec!["192.0.2.7".parse::<IpAddr>().unwrap()]);
        assert!(from_mdns(&a, "mac", &tag).is_none(), "this computer");
        assert!(from_mdns(&a, "pc", &auth::account_tag("other")).is_none(), "another account");
        let wrong = answer("_raop._tcp.local.", "mac", "192.0.2.7", 1, &pairs);
        assert!(from_mdns(&wrong, "pc", &tag).is_none());
        // The TXT record carries no token and no addresses.
        let keys: Vec<&str> = txt.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["id", "name", "platform", "version", "acct"]);
    }

    #[test]
    fn names_fall_back_to_the_computer() {
        assert_eq!(display_name("  Den  "), "Den");
        assert_eq!(display_name(""), os_computer_name());
        assert!(!os_computer_name().is_empty());
    }
}
