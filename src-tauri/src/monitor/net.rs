//! Sleep/wake and network changes, for an immediate health recheck and a
//! fresh notification socket instead of waiting up to 30 s for the next ping.
//!
//! Every few seconds this looks at two things:
//! - Wall-clock time since the last look. Timers don't run while the
//!   computer sleeps, so a gap far longer than the interval means it just
//!   woke up.
//! - Which local address the OS would use to reach the internet. Asking
//!   costs nothing: "connecting" a UDP socket only picks a route, no packet
//!   is sent. A different address means the network changed (Wi-Fi switch,
//!   cable pulled with Wi-Fi still up); no address means no network at all.

use std::net::{IpAddr, UdpSocket};
use std::time::{Duration, SystemTime};

use crate::plex::PlexState;

/// One look costs a socket bind, so a slower tick saves wakeups while idle;
/// the health ping still runs every 30 s.
pub const TICK: Duration = Duration::from_secs(15);
/// Extra wall-clock time beyond the tick that counts as a sleep.
const WAKE_SLACK: Duration = Duration::from_secs(20);

/// The local address of the default route, if there is one.
pub fn route_ip() -> Option<IpAddr> {
    // Public anycast resolvers, used only as a routing target.
    for (bind, target) in [("0.0.0.0:0", "1.1.1.1:53"), ("[::]:0", "[2606:4700:4700::1111]:53")] {
        if let Ok(sock) = UdpSocket::bind(bind)
            && sock.connect(target).is_ok()
            && let Ok(addr) = sock.local_addr()
            && !addr.ip().is_unspecified()
            && !addr.ip().is_loopback()
        {
            return Some(addr.ip());
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetSignal {
    /// The computer woke from sleep.
    Woke,
    /// A different network (or the network came back).
    Changed,
    /// No network route at all.
    Lost,
}

#[derive(Debug)]
pub struct NetWatch {
    route: Option<IpAddr>,
    last_wall: SystemTime,
    started: bool,
}

impl NetWatch {
    pub fn new(now: SystemTime) -> Self {
        Self {
            route: None,
            last_wall: now,
            started: false,
        }
    }

    /// One look. `route` is [`route_ip`]'s answer, `now` the wall clock,
    /// `tick` the interval the caller slept for.
    pub fn tick(&mut self, route: Option<IpAddr>, now: SystemTime, tick: Duration) -> Vec<NetSignal> {
        let mut out = Vec::new();
        let elapsed = now.duration_since(self.last_wall).unwrap_or_default();
        self.last_wall = now;
        if !self.started {
            self.started = true;
            self.route = route;
            if route.is_none() {
                out.push(NetSignal::Lost);
            }
            return out;
        }
        if elapsed > tick + WAKE_SLACK {
            out.push(NetSignal::Woke);
        }
        if route != self.route {
            out.push(if route.is_some() {
                NetSignal::Changed
            } else {
                NetSignal::Lost
            });
            self.route = route;
        }
        out
    }
}

/// What kind of change a route change was, without the addresses: the
/// families and, within one family, whether it's another network or the
/// same network with a new address (an IPv6 privacy address rotating, a
/// carrier handing out another IPv4 address).
pub fn describe_change(old: Option<IpAddr>, new: Option<IpAddr>) -> &'static str {
    match (old, new) {
        (None, Some(_)) => "network back",
        (Some(IpAddr::V4(_)), Some(IpAddr::V6(_))) => "IPv4 to IPv6",
        (Some(IpAddr::V6(_)), Some(IpAddr::V4(_))) => "IPv6 to IPv4",
        (Some(IpAddr::V6(a)), Some(IpAddr::V6(b))) if a.segments()[..4] == b.segments()[..4] => "same IPv6 /64, new address",
        (Some(IpAddr::V6(_)), Some(IpAddr::V6(_))) => "another IPv6 /64",
        (Some(IpAddr::V4(a)), Some(IpAddr::V4(b))) if a.octets()[..3] == b.octets()[..3] => "same IPv4 /24, new address",
        (Some(IpAddr::V4(_)), Some(IpAddr::V4(_))) => "another IPv4 network",
        _ => "no change",
    }
}

/// Background loop: wake or network change → reconnect now; no network →
/// "You're offline" at once.
pub fn spawn(plex: PlexState) {
    tauri::async_runtime::spawn(async move {
        let mut watch = NetWatch::new(SystemTime::now());
        loop {
            let route = tokio::task::spawn_blocking(route_ip).await.ok().flatten();
            let before = watch.route;
            for s in watch.tick(route, SystemTime::now(), TICK) {
                match s {
                    NetSignal::Changed => tracing::info!("network: Changed ({})", describe_change(before, route)),
                    _ => tracing::info!("network: {s:?}"),
                }
                match s {
                    NetSignal::Woke | NetSignal::Changed => plex.reconnect().await,
                    NetSignal::Lost => {
                        plex.on_network_lost();
                        plex.recheck();
                    }
                }
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    const HOME: Option<IpAddr> = Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)));
    const CAFE: Option<IpAddr> = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)));

    fn at(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_760_000_000 + s)
    }

    #[test]
    fn steady_network_is_quiet() {
        let mut w = NetWatch::new(at(0));
        assert!(w.tick(HOME, at(0), TICK).is_empty());
        assert!(w.tick(HOME, at(5), TICK).is_empty());
        assert!(w.tick(HOME, at(11), TICK).is_empty());
    }

    #[test]
    fn wake_from_sleep_is_seen_from_the_clock_jump() {
        let mut w = NetWatch::new(at(0));
        w.tick(HOME, at(0), TICK);
        // laptop lid closed for an hour
        assert_eq!(w.tick(HOME, at(3600), TICK), vec![NetSignal::Woke]);
        assert!(w.tick(HOME, at(3605), TICK).is_empty());
    }

    #[test]
    fn wifi_switch_and_cable_pull() {
        let mut w = NetWatch::new(at(0));
        w.tick(HOME, at(0), TICK);
        assert_eq!(w.tick(CAFE, at(5), TICK), vec![NetSignal::Changed]);
        assert_eq!(w.tick(None, at(10), TICK), vec![NetSignal::Lost]);
        assert!(w.tick(None, at(15), TICK).is_empty());
        assert_eq!(w.tick(HOME, at(20), TICK), vec![NetSignal::Changed]);
    }

    #[test]
    fn starting_without_network_reports_lost() {
        let mut w = NetWatch::new(at(0));
        assert_eq!(w.tick(None, at(0), TICK), vec![NetSignal::Lost]);
    }

    #[test]
    fn wake_on_another_network() {
        let mut w = NetWatch::new(at(0));
        w.tick(HOME, at(0), TICK);
        assert_eq!(
            w.tick(CAFE, at(900), TICK),
            vec![NetSignal::Woke, NetSignal::Changed]
        );
    }

    #[test]
    fn changes_are_described_without_addresses() {
        use std::net::Ipv6Addr;
        let v6 = |last: u16| Some(IpAddr::V6(Ipv6Addr::new(0x2600, 1, 2, 3, 0, 0, 0, last)));
        let other64 = Some(IpAddr::V6(Ipv6Addr::new(0x2600, 9, 2, 3, 0, 0, 0, 1)));
        assert_eq!(describe_change(HOME, v6(1)), "IPv4 to IPv6");
        assert_eq!(describe_change(v6(1), HOME), "IPv6 to IPv4");
        assert_eq!(describe_change(v6(1), v6(2)), "same IPv6 /64, new address");
        assert_eq!(describe_change(v6(1), other64), "another IPv6 /64");
        assert_eq!(describe_change(HOME, CAFE), "another IPv4 network");
        assert_eq!(describe_change(None, HOME), "network back");
    }

    #[test]
    fn route_lookup_does_not_panic() {
        // Depends on the machine; only checks it returns a sane value.
        if let Some(ip) = route_ip() {
            assert!(!ip.is_loopback());
        }
    }
}
