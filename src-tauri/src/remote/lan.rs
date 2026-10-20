//! Devices on this network that Platter drives itself: Google Cast devices
//! and AirPlay receivers, found over mDNS.
//!
//! [`MdnsSource`] is the network side (the real one is [`MdnsSd`]; tests use
//! fixed answers). [`lan_found`] turns answers into picker entries, and a
//! [`LanLink`] is a live connection the Listen on service follows and
//! controls the same way it does a Plex player.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;

use super::airplay::AirPlayDevice;
use super::cast::CastDevice;
use super::companion::Timeline;
use super::discovery::Found;
use super::types::{PlayerVia, RemoteCommand, RemotePlayer};
use crate::error::AppResult;

pub const CAST_SERVICE: &str = "_googlecast._tcp.local.";
pub const RAOP_SERVICE: &str = "_raop._tcp.local.";

/// One mDNS service instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsAnswer {
    /// Service type with domain, e.g. `_googlecast._tcp.local.`.
    pub service: String,
    /// Instance name without the service type.
    pub instance: String,
    pub addrs: Vec<IpAddr>,
    pub port: u16,
    /// TXT keys (lower case) and values.
    pub txt: HashMap<String, String>,
}

impl MdnsAnswer {
    pub fn txt(&self, key: &str) -> Option<&str> {
        self.txt.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }

    /// IPv4 first: link-local IPv6 needs a scope these devices rarely route.
    pub fn socket_addr(&self) -> Option<SocketAddr> {
        let ip = self
            .addrs
            .iter()
            .find(|a| a.is_ipv4())
            .or_else(|| self.addrs.first())
            .copied()?;
        Some(SocketAddr::new(ip, self.port))
    }
}

/// Where mDNS answers come from.
#[async_trait]
pub trait MdnsSource: Send + Sync {
    /// Devices seen now. The first call starts listening and waits briefly
    /// for answers.
    async fn scan(&self) -> Vec<MdnsAnswer>;
    /// Stop listening (the setting turned off).
    fn stop(&self);
}

/// A Cast or AirPlay device in the picker.
#[derive(Debug, Clone, PartialEq)]
pub enum LanDevice {
    Cast(CastDevice),
    AirPlay(AirPlayDevice),
    Dlna(Box<super::dlna::DlnaDevice>),
}

/// Picker entries for the devices in `answers`. Computers' own AirPlay
/// receivers are always left out; `show_locked` keeps receivers that need a
/// password or a pairing code.
pub fn lan_found(answers: &[MdnsAnswer], has_password: &dyn Fn(&str) -> bool, show_locked: bool) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    for a in answers {
        let (player, lan) = if a.service.starts_with("_googlecast.") {
            let Some(d) = super::cast::from_mdns(a) else { continue };
            (d.player(), LanDevice::Cast(d))
        } else if a.service.starts_with("_raop.") {
            let Some(d) = super::airplay::from_mdns(a) else { continue };
            let stored = has_password(&d.id);
            if d.computer || (!show_locked && d.locked(stored)) {
                continue;
            }
            let mut p = d.player();
            p.needs_password = d.password && !stored;
            (p, LanDevice::AirPlay(d))
        } else {
            continue;
        };
        if out.iter().any(|f| f.player.id == player.id) {
            continue;
        }
        out.push(Found {
            player,
            direct: Vec::new(),
            token: None,
            via_server: false,
            sonos: false,
            lan: Some(lan),
            peer: None,
            last_seen: None,
        });
    }
    out
}

/// The shared parts of a LAN device's picker entry.
pub fn lan_player(id: String, name: String, product: String, kind: super::types::PlayerKind, via: PlayerVia) -> RemotePlayer {
    RemotePlayer {
        id,
        name,
        product,
        kind,
        playing: None,
        online: true,
        via,
        needs_password: false,
        unsupported: None,
        unreachable: false,
    }
}

/// A live connection to a Cast device or AirPlay receiver.
#[async_trait]
pub trait LanLink: Send + Sync {
    /// The device now: Ok(None) when it stopped by itself (someone else
    /// took it over or stopped it), Err when it doesn't answer.
    async fn poll(&self) -> AppResult<Option<Timeline>>;
    async fn command(&self, cmd: &RemoteCommand) -> AppResult<()>;
    /// Leave the device. `stop`: stop playback there (Cast). AirPlay hands
    /// the sound back to this computer either way.
    async fn leave(&self, stop: bool);
    /// The sound still comes from this computer (AirPlay): when the device
    /// goes, playback here is already where it should be.
    fn plays_here(&self) -> bool {
        false
    }
    /// Playback here jumped to a new queue or entry: drop what the device
    /// has buffered.
    async fn jumped(&self) {}
}

// ---------------------------------------------------------------- mdns-sd

struct Browser {
    daemon: mdns_sd::ServiceDaemon,
    seen: Arc<Mutex<HashMap<String, MdnsAnswer>>>,
    started: Instant,
}

/// mDNS on this computer's network interfaces (read-only browsing).
#[derive(Default)]
pub struct MdnsSd {
    browser: Mutex<Option<Browser>>,
}

/// How long the first scan waits for devices to answer.
pub const FIRST_SCAN: Duration = Duration::from_millis(1500);

pub(crate) fn answer_of(r: &mdns_sd::ResolvedService) -> MdnsAnswer {
    let instance = r
        .fullname
        .strip_suffix(&r.ty_domain)
        .unwrap_or(&r.fullname)
        .trim_end_matches('.')
        .to_string();
    let mut addrs: Vec<IpAddr> = r.addresses.iter().map(|a| a.to_ip_addr()).collect();
    addrs.sort_by_key(|a| !a.is_ipv4());
    MdnsAnswer {
        service: r.ty_domain.clone(),
        instance,
        addrs,
        port: r.port,
        txt: r
            .txt_properties
            .iter()
            .map(|p| (p.key().to_ascii_lowercase(), p.val_str().to_string()))
            .collect(),
    }
}

impl MdnsSd {
    pub fn new() -> Self {
        Self::default()
    }

    fn start(&self) -> Option<Instant> {
        // iOS doesn't let an app send raw multicast, so this daemon can't
        // find anything there; worse, once iOS closes a suspended app's
        // sockets its thread reads them in a tight loop until iOS kills the
        // app for CPU use. The phone finds AirPlay through the system's
        // route picker and other Platters through Bonjour instead.
        if cfg!(target_os = "ios") {
            return None;
        }
        let mut g = self.browser.lock();
        if let Some(b) = g.as_ref() {
            return Some(b.started);
        }
        let daemon = match mdns_sd::ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("mDNS unavailable: {e}");
                return None;
            }
        };
        let seen: Arc<Mutex<HashMap<String, MdnsAnswer>>> = Arc::default();
        for ty in [CAST_SERVICE, RAOP_SERVICE] {
            let Ok(rx) = daemon.browse(ty) else { continue };
            let seen = seen.clone();
            let _ = std::thread::Builder::new().name("mdns-browse".into()).spawn(move || {
                while let Ok(ev) = rx.recv() {
                    match ev {
                        mdns_sd::ServiceEvent::ServiceResolved(r) => {
                            seen.lock().insert(r.fullname.clone(), answer_of(&r));
                        }
                        mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => {
                            seen.lock().remove(&fullname);
                        }
                        mdns_sd::ServiceEvent::SearchStopped(_) => return,
                        _ => {}
                    }
                }
            });
        }
        let started = Instant::now();
        *g = Some(Browser { daemon, seen, started });
        Some(started)
    }
}

#[async_trait]
impl MdnsSource for MdnsSd {
    async fn scan(&self) -> Vec<MdnsAnswer> {
        let Some(started) = self.start() else {
            return Vec::new();
        };
        let waited = started.elapsed();
        if waited < FIRST_SCAN {
            tokio::time::sleep(FIRST_SCAN - waited).await;
        }
        let g = self.browser.lock();
        let Some(b) = g.as_ref() else { return Vec::new() };
        let mut list: Vec<MdnsAnswer> = b.seen.lock().values().cloned().collect();
        list.sort_by(|a, b| a.instance.cmp(&b.instance));
        list
    }

    fn stop(&self) {
        if let Some(b) = self.browser.lock().take() {
            let _ = b.daemon.shutdown();
        }
    }
}

/// Fixed answers (tests and the mock).
#[derive(Default)]
pub struct StaticMdns {
    pub answers: Mutex<Vec<MdnsAnswer>>,
    pub scans: std::sync::atomic::AtomicUsize,
    pub stopped: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl MdnsSource for StaticMdns {
    async fn scan(&self) -> Vec<MdnsAnswer> {
        use std::sync::atomic::Ordering;
        self.scans.fetch_add(1, Ordering::SeqCst);
        self.stopped.store(false, Ordering::SeqCst);
        self.answers.lock().clone()
    }
    fn stop(&self) {
        self.stopped.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn answer(service: &str, instance: &str, addr: &str, port: u16, txt: &[(&str, &str)]) -> MdnsAnswer {
    MdnsAnswer {
        service: service.to_string(),
        instance: instance.to_string(),
        addrs: vec![addr.parse().unwrap()],
        port,
        txt: txt.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::types::PlayerKind;

    #[test]
    fn answers_become_picker_entries() {
        let answers = vec![
            answer(CAST_SERVICE, "Chromecast-abc", "192.0.2.30", 8009, &[("id", "abc"), ("fn", "Living Room TV"), ("md", "Chromecast"), ("ca", "201221")]),
            answer(CAST_SERVICE, "Group-g1", "192.0.2.31", 32187, &[("id", "g1"), ("fn", "Downstairs"), ("md", "Google Cast Group")]),
            answer(RAOP_SERVICE, "AABBCCDDEEFF@Kitchen", "192.0.2.40", 7000, &[("et", "0,3,5"), ("cn", "0,1,2,3"), ("pw", "true"), ("am", "AudioAccessory5,1")]),
            answer("_http._tcp.local.", "web", "192.0.2.50", 80, &[]),
            // Same device answering twice.
            answer(CAST_SERVICE, "Chromecast-abc", "192.0.2.30", 8009, &[("id", "abc"), ("fn", "Living Room TV"), ("md", "Chromecast")]),
        ];
        let found = lan_found(&answers, &|_| false, true);
        assert_eq!(found.len(), 3);
        let tv = &found[0].player;
        assert_eq!((tv.id.as_str(), tv.name.as_str(), tv.kind, tv.via), ("cast:abc", "Living Room TV", PlayerKind::Tv, PlayerVia::Cast));
        assert_eq!(found[1].player.kind, PlayerKind::Group);
        let k = &found[2].player;
        assert_eq!((k.id.as_str(), k.name.as_str(), k.via), ("airplay:AABBCCDDEEFF", "Kitchen", PlayerVia::AirPlay));
        assert!(k.needs_password);
        // A stored password: no prompt.
        let found = lan_found(&answers, &|id| id == "airplay:AABBCCDDEEFF", true);
        assert!(!found[2].player.needs_password);
    }

    #[test]
    fn computers_and_locked_receivers_are_hidden() {
        let answers = vec![
            answer(RAOP_SERVICE, "A1@Studio Mac", "192.0.2.41", 7000, &[("am", "Mac14,2"), ("pw", "true")]),
            answer(RAOP_SERVICE, "A2@Old iMac", "192.0.2.42", 7000, &[("am", "iMac21,1")]),
            answer(RAOP_SERVICE, "A3@Laptop", "192.0.2.43", 7000, &[("am", "MacBookAir10,1")]),
            answer(RAOP_SERVICE, "A4@PC receiver", "192.0.2.44", 7000, &[("ft", "0x5A7FFFF7,0x1E")]),
            answer(RAOP_SERVICE, "A5@Den TV", "192.0.2.45", 7000, &[("am", "AppleTV11,1"), ("ft", "0x5A7FFFF7,0x1E")]),
            answer(RAOP_SERVICE, "A6@Kitchen", "192.0.2.46", 7000, &[("am", "AudioAccessory5,1"), ("pw", "true")]),
            answer(RAOP_SERVICE, "A7@Bedroom", "192.0.2.47", 7000, &[("am", "AudioAccessory1,1"), ("sf", "0x8")]),
            answer(RAOP_SERVICE, "A8@Patio", "192.0.2.48", 7000, &[("am", "AirPort10,115")]),
        ];
        let ids = |show_locked: bool, stored: &dyn Fn(&str) -> bool| -> Vec<String> {
            lan_found(&answers, stored, show_locked).into_iter().map(|f| f.player.name).collect()
        };
        // Default: no computers, nothing that asks for a password or a code.
        assert_eq!(ids(false, &|_| false), ["Den TV", "Patio"]);
        // Shown on request; computers still never.
        assert_eq!(ids(true, &|_| false), ["Den TV", "Kitchen", "Bedroom", "Patio"]);
        // A password kept from before doesn't count as locked.
        assert_eq!(ids(false, &|id| id == "airplay:A6"), ["Den TV", "Kitchen", "Patio"]);
    }

    #[test]
    fn ipv4_is_preferred() {
        let mut a = answer(CAST_SERVICE, "x", "fe80::1", 8009, &[]);
        a.addrs.push("192.0.2.9".parse().unwrap());
        assert_eq!(a.socket_addr().unwrap().to_string(), "192.0.2.9:8009");
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Read-only browse of this network. Prints only counts per device kind
    /// (never names, addresses or ids). Run by hand:
    /// `cargo test --lib live_mdns_counts -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_mdns_counts() {
        let m = MdnsSd::new();
        m.scan().await;
        tokio::time::sleep(Duration::from_secs(4)).await;
        let answers = m.scan().await;
        m.stop();
        let found = lan_found(&answers, &|_| false, true);
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for f in &found {
            let key = format!(
                "{:?} {:?}{}{}",
                f.player.via,
                f.player.kind,
                if f.player.needs_password { " password" } else { "" },
                if f.player.unsupported.is_some() { " unsupported" } else { "" }
            );
            *counts.entry(key).or_default() += 1;
        }
        let computers = answers
            .iter()
            .filter(|a| a.service.starts_with("_raop.") && super::super::airplay::from_mdns(a).is_some_and(|d| d.computer))
            .count();
        println!("mdns answers: {}, devices: {counts:?}, computers' AirPlay hidden: {computers}", answers.len());
    }
}
