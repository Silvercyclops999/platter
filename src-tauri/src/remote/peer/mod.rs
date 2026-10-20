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
//! - [`sync`]: the requests that keep the user's devices in step.
//! - [`pair`]: the requests that pair two of them with the 6-digit code.

pub mod auth;
#[cfg(target_os = "ios")]
pub mod bonjour;
pub mod client;
pub mod devices;
pub mod directory;
pub mod keys;
pub mod pair;
pub mod publish;
pub mod runtime;
pub mod server;
pub mod sync;


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

/// The longest name, platform and now-playing line kept from another
/// device's announcement or hello.
pub const MAX_NAME: usize = 60;
pub const MAX_PLATFORM: usize = 40;
pub const MAX_PLAYING: usize = 200;

/// Text from another device, cut to `max` characters with control
/// characters taken out.
pub fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

impl Hello {
    /// The answer with its text cut to size.
    pub fn cleaned(self) -> Self {
        Self {
            name: clean(&self.name, MAX_NAME),
            platform: clean(&self.platform, MAX_PLATFORM),
            version: clean(&self.version, MAX_PLATFORM),
            playing: self.playing.map(|p| clean(&p, MAX_PLAYING)),
            id: self.id,
        }
    }
}

/// An address another Platter device may be asked at: a private LAN
/// address (not the cloud metadata address in the link-local range), or
/// for IPv6 a link-local or unique local one. Loopback only in tests.
pub fn is_peer_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => (is_lan(v4) && v4 != Ipv4Addr::new(169, 254, 169, 254)) || (v4.is_loopback() && cfg!(test)),
        IpAddr::V6(v6) => {
            let s0 = v6.segments()[0];
            (s0 & 0xffc0) == 0xfe80 || (s0 & 0xfe00) == 0xfc00 || (v6.is_loopback() && cfg!(test))
        }
    }
}

/// A control API base that may be probed: http to a LAN address
/// ([`is_peer_ip`]) on a real port.
pub fn is_peer_base(u: &Url) -> bool {
    let ip = match u.host() {
        Some(url::Host::Ipv4(v4)) => IpAddr::V4(v4),
        Some(url::Host::Ipv6(v6)) => IpAddr::V6(v6),
        _ => return false,
    };
    u.scheme() == "http" && u.port_or_known_default().is_some_and(|p| p != 0) && is_peer_ip(ip)
}

/// A `_platter._tcp` answer as a device of this account: the account tag
/// must match and it mustn't be this computer. Only LAN addresses are kept.
pub fn from_mdns(a: &MdnsAnswer, own_id: &str, account_tag: &str) -> Option<PeerDevice> {
    if !a.service.starts_with("_platter.") || a.port == 0 {
        return None;
    }
    let id = a.txt("id")?.to_string();
    if id == own_id || a.txt("acct") != Some(account_tag) {
        return None;
    }
    let mut addrs: Vec<IpAddr> = a.addrs.iter().copied().filter(|ip| is_peer_ip(*ip)).collect();
    addrs.sort_by_key(|ip| !ip.is_ipv4());
    let bases = addrs
        .into_iter()
        .filter_map(|ip| Url::parse(&format!("http://{}", SocketAddr::new(ip, a.port))).ok())
        .collect::<Vec<_>>();
    if bases.is_empty() {
        return None;
    }
    Some(PeerDevice {
        id,
        name: clean(a.txt("name").unwrap_or(&a.instance), MAX_NAME),
        platform: clean(a.txt("platform").unwrap_or_default(), MAX_PLATFORM),
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

/// This computer's LAN IPv4 addresses, the default route's adapter first
/// and virtual adapters (WSL, Hyper-V, VPNs) left out when there's a real one.
pub fn lan_addrs() -> Vec<Ipv4Addr> {
    let ifaces = crate::remote::netif::system();
    crate::remote::netif::lan_v4(&ifaces, crate::remote::netif::default_route_v4())
}

/// The OS computer name ("Studio PC", "Adam's MacBook Air").
pub fn os_computer_name() -> String {
    // An iPhone app can't run commands, and iOS hands apps only the model
    // name anyway.
    if cfg!(target_os = "ios") {
        return "iPhone".into();
    }
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
        let a = answer(SERVICE, "mac", "10.0.0.7", 32520, &pairs);
        let d = from_mdns(&a, "pc", &tag).unwrap();
        assert_eq!((d.id.as_str(), d.name.as_str(), d.platform.as_str()), ("mac", "Studio Mac", "macOS"));
        assert_eq!(d.bases[0].as_str(), "http://10.0.0.7:32520/");
        assert_eq!(d.ips(), vec!["10.0.0.7".parse::<IpAddr>().unwrap()]);
        assert!(from_mdns(&a, "mac", &tag).is_none(), "this computer");
        assert!(from_mdns(&a, "pc", &auth::account_tag("other")).is_none(), "another account");
        let wrong = answer("_raop._tcp.local.", "mac", "10.0.0.7", 1, &pairs);
        assert!(from_mdns(&wrong, "pc", &tag).is_none());
        // The TXT record carries no token and no addresses.
        let keys: Vec<&str> = txt.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["id", "name", "platform", "version", "acct"]);
    }

    #[test]
    fn announcements_off_the_lan_or_without_a_port_are_dropped() {
        let tag = auth::account_tag("acct");
        let txt = txt("mac", "Studio Mac", "macOS", "0.3.0", &tag);
        let pairs: Vec<(&str, &str)> = txt.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        for ip in ["8.8.8.8", "192.0.2.7", "169.254.169.254", "100.64.0.7", "0.0.0.0", "2001:db8::7", "ff02::fb"] {
            assert!(from_mdns(&answer(SERVICE, "mac", ip, 32520, &pairs), "pc", &tag).is_none(), "{ip}");
        }
        assert!(from_mdns(&answer(SERVICE, "mac", "10.0.0.7", 0, &pairs), "pc", &tag).is_none(), "port 0");
        for ip in ["172.16.4.2", "169.254.10.1", "fe80::1", "fd12:3456::1"] {
            assert!(from_mdns(&answer(SERVICE, "mac", ip, 32520, &pairs), "pc", &tag).is_some(), "{ip}");
        }
        // A public address beside a LAN one: only the LAN one is kept.
        let mut both = answer(SERVICE, "mac", "10.0.0.7", 32520, &pairs);
        both.addrs.push("8.8.8.8".parse().unwrap());
        assert_eq!(from_mdns(&both, "pc", &tag).unwrap().bases.len(), 1);
        // Addresses from plex.tv get the same check.
        for (u, ok) in [
            ("http://10.0.0.7:32520", true),
            ("http://[fe80::1]:32520", true),
            ("http://8.8.8.8:32520", false),
            ("http://169.254.169.254:80", false),
            ("http://10.0.0.7:0", false),
            ("https://10.0.0.7:32520", false),
            ("http://10-0-0-7.abc.plex.direct:32520", false),
        ] {
            assert_eq!(is_peer_base(&Url::parse(u).unwrap()), ok, "{u}");
        }
    }

    #[test]
    fn long_or_odd_names_are_cut() {
        let tag = auth::account_tag("acct");
        let long = "N".repeat(500);
        let name = format!("Bad\nName{long}");
        let platform = format!("i\u{7}OS{}", "x".repeat(100));
        let a = answer(SERVICE, "mac", "10.0.0.7", 32520, &[("id", "mac"), ("name", &name), ("platform", &platform), ("acct", &tag)]);
        let d = from_mdns(&a, "pc", &tag).unwrap();
        assert_eq!(d.name.chars().count(), MAX_NAME);
        assert!(d.name.starts_with("BadName"), "{}", d.name);
        assert_eq!(d.platform.chars().count(), MAX_PLATFORM);
        assert!(d.platform.starts_with("iOS"), "{}", d.platform);
        let playing = format!("\u{1b}[2J{}", "p".repeat(1000));
        let h = Hello { id: "mac".into(), name: long, platform, version: "1".into(), playing: Some(playing) }.cleaned();
        assert_eq!((h.name.chars().count(), h.platform.chars().count()), (MAX_NAME, MAX_PLATFORM));
        let p = h.playing.unwrap();
        assert_eq!(p.chars().count(), MAX_PLAYING);
        assert!(p.starts_with("[2J"), "the escape character is gone: {p}");
    }

    #[test]
    fn names_fall_back_to_the_computer() {
        assert_eq!(display_name("  Den  "), "Den");
        assert_eq!(display_name(""), os_computer_name());
        assert!(!os_computer_name().is_empty());
    }
}
