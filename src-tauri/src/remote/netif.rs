//! Which network adapters to search and announce on. A computer often has
//! virtual adapters besides the real one (Tailscale and other VPNs, WSL and
//! Hyper-V switches, VirtualBox, Docker). Multicast sent there never reaches
//! a phone on the Wi-Fi, and Windows may answer on the wrong one, so the
//! search skips them whenever a real LAN adapter exists. Nothing here
//! assumes a VPN is present.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// One address of one adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub ip: IpAddr,
}

/// The shared range VPNs such as Tailscale hand out (100.64.0.0/10).
pub fn is_cgnat(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0xc0) == 64
}

/// An adapter name that belongs to a VPN, a virtual machine or containers.
pub fn is_virtual_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const HAS: [&str; 14] = [
        "tailscale", "zerotier", "wireguard", "vethernet", "virtualbox", "vmware", "hyper-v", "docker", "hamachi",
        "nordlynx", "openvpn", "npcap", "loopback", "wsl",
    ];
    const STARTS: [&str; 9] = ["wg", "tun", "tap", "utun", "veth", "br-", "vmnet", "vboxnet", "virbr"];
    HAS.iter().any(|w| n.contains(w)) || STARTS.iter().any(|w| n.starts_with(w))
}

fn is_lan_v4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local()
}

/// The adapters' addresses (all of them, IPv4 and IPv6).
pub fn system() -> Vec<Iface> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|i| Iface { ip: i.ip(), name: i.name })
        .collect()
}

/// The IPv4 address the system would send internet traffic from: the
/// adapter that holds the default route. Connecting a UDP socket only
/// looks up the route; nothing is sent.
pub fn default_route_v4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

/// Adapters that are virtual: named like one, or holding a 100.64/10 address.
fn virtual_names(ifaces: &[Iface]) -> Vec<&str> {
    let mut out: Vec<&str> = ifaces
        .iter()
        .filter(|i| is_virtual_name(&i.name) || matches!(i.ip, IpAddr::V4(v4) if is_cgnat(v4)))
        .map(|i| i.name.as_str())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The addresses multicast should leave out: every address of a virtual
/// adapter, as long as a real adapter with a LAN address is left.
pub fn multicast_skip(ifaces: &[Iface]) -> Vec<IpAddr> {
    let bad = virtual_names(ifaces);
    let real_lan = ifaces
        .iter()
        .any(|i| !bad.contains(&i.name.as_str()) && matches!(i.ip, IpAddr::V4(v4) if is_lan_v4(v4)));
    if !real_lan {
        return Vec::new();
    }
    ifaces
        .iter()
        .filter(|i| bad.contains(&i.name.as_str()) && !i.ip.is_loopback())
        .map(|i| i.ip)
        .collect()
}

/// This computer's LAN IPv4 addresses, best first: the default route's
/// adapter, then other real adapters, then virtual ones only when there's
/// nothing else.
pub fn lan_v4(ifaces: &[Iface], default: Option<Ipv4Addr>) -> Vec<Ipv4Addr> {
    let bad = virtual_names(ifaces);
    let lan: Vec<(bool, Ipv4Addr)> = ifaces
        .iter()
        .filter_map(|i| match i.ip {
            IpAddr::V4(v4) if is_lan_v4(v4) => Some((bad.contains(&i.name.as_str()), v4)),
            _ => None,
        })
        .collect();
    let any_real = lan.iter().any(|(v, _)| !v);
    let mut out: Vec<(u8, Ipv4Addr)> = lan
        .into_iter()
        .filter(|(v, _)| !v || !any_real)
        .map(|(_, ip)| {
            let rank = if Some(ip) == default {
                0
            } else if ip.is_private() {
                1
            } else {
                2
            };
            (rank, ip)
        })
        .collect();
    out.sort();
    out.dedup();
    out.into_iter().map(|(_, ip)| ip).collect()
}

/// Leave the virtual adapters out of an mDNS daemon's searches and answers.
pub fn skip_virtual(daemon: &mdns_sd::ServiceDaemon) {
    let skip = multicast_skip(&system());
    if skip.is_empty() {
        return;
    }
    tracing::debug!(count = skip.len(), "mDNS leaves out virtual network adapters");
    let kinds: Vec<mdns_sd::IfKind> = skip.into_iter().map(mdns_sd::IfKind::Addr).collect();
    if let Err(e) = daemon.disable_interface(kinds) {
        tracing::debug!("couldn't leave out virtual adapters: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i(name: &str, ip: &str) -> Iface {
        Iface { name: name.into(), ip: ip.parse().unwrap() }
    }

    #[test]
    fn the_vpn_range_is_recognised() {
        assert!(is_cgnat("100.64.0.1".parse().unwrap()));
        assert!(is_cgnat("100.101.7.3".parse().unwrap()));
        assert!(is_cgnat("100.127.255.254".parse().unwrap()));
        assert!(!is_cgnat("100.128.0.1".parse().unwrap()));
        assert!(!is_cgnat("100.63.0.1".parse().unwrap()));
        assert!(!is_cgnat("10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn virtual_adapters_are_named_like_one() {
        for n in ["Tailscale", "vEthernet (WSL)", "vEthernet (Default Switch)", "VirtualBox Host-Only Network", "wg0", "utun3", "docker0", "ZeroTier One [abc]", "VMware Network Adapter VMnet8"] {
            assert!(is_virtual_name(n), "{n}");
        }
        for n in ["Ethernet", "Wi-Fi", "en0", "eth0", "wlan0", "Ethernet 2"] {
            assert!(!is_virtual_name(n), "{n}");
        }
    }

    #[test]
    fn multicast_skips_virtual_adapters_when_a_real_one_exists() {
        let ifaces = [
            i("Ethernet", "192.168.1.20"),
            i("Ethernet", "fe80::1"),
            i("Tailscale", "100.90.1.2"),
            i("Tailscale", "fd7a:115c:a1e0::1"),
            i("vEthernet (WSL)", "172.20.0.1"),
            // An adapter with an odd name but a VPN address.
            i("Local Area Connection 3", "100.70.0.9"),
            i("Loopback Pseudo-Interface 1", "127.0.0.1"),
        ];
        let skip = multicast_skip(&ifaces);
        let want: Vec<IpAddr> =
            ["100.90.1.2", "fd7a:115c:a1e0::1", "172.20.0.1", "100.70.0.9"].iter().map(|s| s.parse().unwrap()).collect();
        assert_eq!(skip, want);
    }

    #[test]
    fn with_no_real_lan_adapter_nothing_is_skipped() {
        // Only a VPN: better to search there than nowhere.
        assert!(multicast_skip(&[i("Tailscale", "100.90.1.2"), i("lo", "127.0.0.1")]).is_empty());
        // No VPN at all: nothing to skip.
        assert!(multicast_skip(&[i("Wi-Fi", "10.0.0.4"), i("Ethernet", "192.168.1.3")]).is_empty());
    }

    #[test]
    fn the_default_route_adapter_comes_first() {
        let ifaces = [
            i("Ethernet", "192.168.56.1"),
            i("Wi-Fi", "192.168.1.40"),
            i("vEthernet (WSL)", "172.20.0.1"),
            i("Tailscale", "100.90.1.2"),
        ];
        let got = lan_v4(&ifaces, Some("192.168.1.40".parse().unwrap()));
        let want: Vec<Ipv4Addr> = ["192.168.1.40", "192.168.56.1"].iter().map(|s| s.parse().unwrap()).collect();
        assert_eq!(got, want, "WSL and Tailscale left out");
        // Default route over the VPN (an exit node): the LAN still comes first.
        assert_eq!(lan_v4(&ifaces, Some("100.90.1.2".parse().unwrap())), want);
        // Only a virtual LAN address: kept, rather than nothing.
        assert_eq!(lan_v4(&[i("vEthernet (WSL)", "172.20.0.1")], None), vec!["172.20.0.1".parse::<Ipv4Addr>().unwrap()]);
    }
}
