//! Announcing this computer and finding the others: `_platter._tcp` over
//! mDNS ([`MdnsPeers`]), or a shared in-memory network for tests and the
//! two-instance checks ([`MemoryLan`]).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use parking_lot::Mutex;

use super::SERVICE;
use crate::remote::lan::{FIRST_SCAN, MdnsAnswer, answer_of};

/// What this computer announces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announce {
    pub id: String,
    pub txt: Vec<(String, String)>,
    pub ips: Vec<Ipv4Addr>,
    pub port: u16,
}

/// IPv4 and IPv6 from Bonjour's `sockaddr` blobs.
pub(crate) fn ip_of(sockaddr: &[u8]) -> Option<IpAddr> {
    match sockaddr.get(1)? {
        2 if sockaddr.len() >= 8 => Some(IpAddr::V4(Ipv4Addr::new(sockaddr[4], sockaddr[5], sockaddr[6], sockaddr[7]))),
        30 if sockaddr.len() >= 24 => {
            let b: [u8; 16] = sockaddr[8..24].try_into().ok()?;
            Some(IpAddr::V6(Ipv6Addr::from(b)))
        }
        _ => None,
    }
}

#[async_trait]
pub trait PeerDirectory: Send + Sync {
    /// Announce (or update the announcement).
    fn announce(&self, a: &Announce);
    /// Stop announcing (the setting turned off, or quitting).
    fn withdraw(&self);
    /// Platter devices seen now (this one may be among them).
    async fn scan(&self) -> Vec<MdnsAnswer>;
}

struct Daemon {
    daemon: mdns_sd::ServiceDaemon,
    seen: Arc<Mutex<HashMap<String, MdnsAnswer>>>,
    started: Instant,
    registered: Option<String>,
}

/// mDNS on this computer's network interfaces.
#[derive(Default)]
pub struct MdnsPeers {
    state: Mutex<Option<Daemon>>,
}

impl MdnsPeers {
    pub fn new() -> Self {
        Self::default()
    }

    fn with<R>(&self, f: impl FnOnce(&mut Daemon) -> R) -> Option<R> {
        let mut g = self.state.lock();
        if g.is_none() {
            let daemon = match mdns_sd::ServiceDaemon::new() {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("mDNS unavailable: {e}");
                    return None;
                }
            };
            let seen: Arc<Mutex<HashMap<String, MdnsAnswer>>> = Arc::default();
            if let Ok(rx) = daemon.browse(SERVICE) {
                let seen = seen.clone();
                let _ = std::thread::Builder::new().name("mdns-platter".into()).spawn(move || {
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
            *g = Some(Daemon { daemon, seen, started: Instant::now(), registered: None });
        }
        g.as_mut().map(f)
    }
}

#[async_trait]
impl PeerDirectory for MdnsPeers {
    fn announce(&self, a: &Announce) {
        self.with(|d| {
            if let Some(old) = d.registered.take() {
                let _ = d.daemon.unregister(&old);
            }
            let short: String = a.id.chars().filter(char::is_ascii_alphanumeric).take(12).collect();
            let host = format!("platter-{short}.local.");
            let ips: Vec<IpAddr> = a.ips.iter().map(|ip| IpAddr::V4(*ip)).collect();
            let props: HashMap<String, String> = a.txt.iter().cloned().collect();
            match mdns_sd::ServiceInfo::new(SERVICE, &a.id, &host, &ips[..], a.port, props) {
                Ok(info) => {
                    let name = info.get_fullname().to_string();
                    match d.daemon.register(info) {
                        Ok(()) => d.registered = Some(name),
                        Err(e) => tracing::warn!("couldn't announce this computer: {e}"),
                    }
                }
                Err(e) => tracing::warn!("couldn't announce this computer: {e}"),
            }
        });
    }

    fn withdraw(&self) {
        let mut g = self.state.lock();
        if let Some(d) = g.as_mut()
            && let Some(old) = d.registered.take()
        {
            let _ = d.daemon.unregister(&old);
        }
    }

    async fn scan(&self) -> Vec<MdnsAnswer> {
        let Some(started) = self.with(|d| d.started) else {
            return Vec::new();
        };
        let waited = started.elapsed();
        if waited < FIRST_SCAN {
            tokio::time::sleep(FIRST_SCAN - waited).await;
        }
        self.with(|d| d.seen.lock().values().cloned().collect()).unwrap_or_default()
    }
}

/// A pretend network that several in-process directories share.
#[derive(Default)]
pub struct MemoryLan {
    entries: Mutex<HashMap<String, MdnsAnswer>>,
}

impl MemoryLan {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }
    /// Put an answer on the network directly (a device from another account).
    pub fn insert(&self, a: MdnsAnswer) {
        self.entries.lock().insert(a.instance.clone(), a);
    }
    pub fn answers(&self) -> Vec<MdnsAnswer> {
        self.entries.lock().values().cloned().collect()
    }
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One computer on a [`MemoryLan`].
pub struct MemoryDirectory {
    pub lan: Arc<MemoryLan>,
    mine: Mutex<Option<String>>,
}

impl MemoryDirectory {
    pub fn new(lan: Arc<MemoryLan>) -> Self {
        Self { lan, mine: Mutex::new(None) }
    }
}

#[async_trait]
impl PeerDirectory for MemoryDirectory {
    fn announce(&self, a: &Announce) {
        self.withdraw();
        self.lan.insert(MdnsAnswer {
            service: SERVICE.into(),
            instance: a.id.clone(),
            addrs: a.ips.iter().map(|ip| IpAddr::V4(*ip)).collect(),
            port: a.port,
            txt: a.txt.iter().cloned().collect(),
        });
        *self.mine.lock() = Some(a.id.clone());
    }
    fn withdraw(&self) {
        if let Some(id) = self.mine.lock().take() {
            self.lan.entries.lock().remove(&id);
        }
    }
    async fn scan(&self) -> Vec<MdnsAnswer> {
        let mut v: Vec<MdnsAnswer> = self.lan.entries.lock().values().cloned().collect();
        v.sort_by(|a, b| a.instance.cmp(&b.instance));
        v
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Read-only browse for Platter devices on this network (nothing is
    /// announced). Prints only a count. Run by hand:
    /// `cargo test --lib live_platter_count -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_platter_count() {
        let m = MdnsPeers::new();
        m.scan().await;
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        let n = m.scan().await.len();
        println!("platter devices announcing: {n}");
    }
}

#[cfg(test)]
mod bonjour_tests {
    use super::ip_of;

    #[test]
    fn sockaddrs_read_as_addresses() {
        let v4 = [16u8, 2, 0x7f, 0x50, 192, 168, 1, 20, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(ip_of(&v4), Some("192.168.1.20".parse().unwrap()));
        let mut v6 = [0u8; 28];
        v6[1] = 30;
        v6[8] = 0xfe;
        v6[9] = 0x80;
        v6[23] = 1;
        assert_eq!(ip_of(&v6), Some("fe80::1".parse().unwrap()));
        assert_eq!(ip_of(&[16, 99, 0, 0]), None);
    }
}
