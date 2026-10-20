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
//!
//! Both families are looked at, each on its own: the IPv4 address, and the
//! IPv6 /64 (a phone's IPv6 address rotates within its /64 every so often,
//! and a cellular IPv4 route can come and go between looks while the network
//! stays the same). A change in both at once is a new network at once; a
//! change in only one counts once it's held for a second look, so flapping
//! doesn't reconnect every 15 s.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, UdpSocket};
use std::time::{Duration, SystemTime};

use crate::plex::PlexState;

/// One look costs a socket bind, so a slower tick saves wakeups while idle;
/// the health ping still runs every 30 s.
pub const TICK: Duration = Duration::from_secs(15);
/// A change waiting for its second look gets it this soon, not a whole
/// tick later.
pub const RELOOK: Duration = Duration::from_secs(3);
/// Extra wall-clock time beyond the tick that counts as a sleep.
const WAKE_SLACK: Duration = Duration::from_secs(20);

/// The local address the default route would use for one family.
fn route_for(bind: &str, target: &str) -> Option<IpAddr> {
    let sock = UdpSocket::bind(bind).ok()?;
    sock.connect(target).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// Which network this is, per family. Public anycast resolvers are used
/// only as routing targets; nothing is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Route {
    pub v4: Option<Ipv4Addr>,
    /// The /64 only: the address within it rotates on the same network.
    pub v6: Option<[u16; 4]>,
}

impl Route {
    pub fn of(v4: Option<Ipv4Addr>, v6: Option<Ipv6Addr>) -> Self {
        Self {
            v4,
            v6: v6.map(|a| {
                let s = a.segments();
                [s[0], s[1], s[2], s[3]]
            }),
        }
    }

    pub fn is_none(&self) -> bool {
        self.v4.is_none() && self.v6.is_none()
    }
}

/// Both families' routes now.
pub fn route() -> Route {
    let v4 = match route_for("0.0.0.0:0", "1.1.1.1:53") {
        Some(IpAddr::V4(a)) => Some(a),
        _ => None,
    };
    let v6 = match route_for("[::]:0", "[2606:4700:4700::1111]:53") {
        Some(IpAddr::V6(a)) => Some(a),
        _ => None,
    };
    Route::of(v4, v6)
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
    route: Route,
    /// A change in one family only, waiting for a second look.
    pending: Option<Route>,
    last_wall: SystemTime,
    started: bool,
}

impl NetWatch {
    pub fn new(now: SystemTime) -> Self {
        Self {
            route: Route::default(),
            pending: None,
            last_wall: now,
            started: false,
        }
    }

    /// The route last taken as this network.
    pub fn current(&self) -> Route {
        self.route
    }

    /// A change in one family is waiting for its second look.
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// One look. `route` is [`route`]'s answer, `now` the wall clock,
    /// `tick` the interval the caller slept for.
    pub fn tick(&mut self, route: Route, now: SystemTime, tick: Duration) -> Vec<NetSignal> {
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
        if route == self.route {
            self.pending = None;
            return out;
        }
        // Every family either side has moved: a new network.
        let moved = |was: bool, is: bool, same: bool| !(was || is) || !same;
        let all_moved = moved(self.route.v4.is_some(), route.v4.is_some(), self.route.v4 == route.v4)
            && moved(self.route.v6.is_some(), route.v6.is_some(), self.route.v6 == route.v6);
        let at_once = route.is_none() || self.route.is_none() || all_moved || self.pending == Some(route);
        if at_once {
            self.pending = None;
            out.push(if route.is_none() { NetSignal::Lost } else { NetSignal::Changed });
            self.route = route;
        } else {
            self.pending = Some(route);
        }
        out
    }
}

/// What kind of change a route change was, without the addresses.
pub fn describe_change(old: Route, new: Route) -> String {
    let family = |name: &str, was: bool, is: bool, same: bool| match (was, is) {
        (false, true) => Some(format!("{name} came")),
        (true, false) => Some(format!("{name} went")),
        (true, true) if !same => Some(format!("another {name} network")),
        _ => None,
    };
    let parts: Vec<String> = [
        family("IPv4", old.v4.is_some(), new.v4.is_some(), old.v4 == new.v4),
        family("IPv6", old.v6.is_some(), new.v6.is_some(), old.v6 == new.v6),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() { "no change".into() } else { parts.join(", ") }
}

/// The app came back to the front (the phone resumed it): check the server
/// now, as after a wake, instead of at the next ping.
pub async fn follow_foreground<F, Fut>(mut rx: tokio::sync::watch::Receiver<bool>, on_front: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    while rx.changed().await.is_ok() {
        let fg = *rx.borrow_and_update();
        if fg {
            tracing::info!("network: back in the foreground");
            on_front().await;
        }
    }
}

/// Background loop: wake or network change → reconnect now; no network →
/// "You're offline" at once.
pub fn spawn(plex: PlexState) {
    let front = plex.clone();
    tauri::async_runtime::spawn(async move {
        follow_foreground(crate::platform::foreground::subscribe(), || {
            let p = front.clone();
            async move { p.reconnect().await }
        })
        .await
    });
    tauri::async_runtime::spawn(async move {
        let mut watch = NetWatch::new(SystemTime::now());
        loop {
            let Ok(route) = tokio::task::spawn_blocking(route).await else {
                tokio::time::sleep(TICK).await;
                continue;
            };
            let before = watch.current();
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
            tokio::time::sleep(if watch.waiting() { RELOOK } else { TICK }).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn coming_back_to_the_front_checks_the_server_now() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (tx, rx) = tokio::sync::watch::channel(true);
        let n = Arc::new(AtomicUsize::new(0));
        let seen = n.clone();
        let follow = tokio::spawn(follow_foreground(rx, move || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        }));
        tx.send(false).unwrap(); // to the background: nothing
        tx.send(true).unwrap(); // back to the front: a check
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while n.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline, "no check after coming to the front");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(n.load(Ordering::SeqCst), 1, "only coming to the front checks");
        follow.abort();
    }

    fn v4(a: u8, b: u8) -> Option<Ipv4Addr> {
        Some(Ipv4Addr::new(192, 168, a, b))
    }
    fn v6(net: u16, host: u16) -> Option<Ipv6Addr> {
        Some(Ipv6Addr::new(0x2600, net, 0, 0, 0, 0, 0, host))
    }
    const NONE: Route = Route { v4: None, v6: None };
    fn home() -> Route {
        Route::of(v4(1, 20), None)
    }
    fn cafe() -> Route {
        Route::of(Some(Ipv4Addr::new(10, 0, 0, 7)), None)
    }

    fn at(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_760_000_000 + s)
    }

    #[test]
    fn steady_network_is_quiet() {
        let mut w = NetWatch::new(at(0));
        assert!(w.tick(home(), at(0), TICK).is_empty());
        assert!(w.tick(home(), at(5), TICK).is_empty());
        assert!(w.tick(home(), at(11), TICK).is_empty());
    }

    #[test]
    fn wake_from_sleep_is_seen_from_the_clock_jump() {
        let mut w = NetWatch::new(at(0));
        w.tick(home(), at(0), TICK);
        // laptop lid closed for an hour
        assert_eq!(w.tick(home(), at(3600), TICK), vec![NetSignal::Woke]);
        assert!(w.tick(home(), at(3605), TICK).is_empty());
    }

    #[test]
    fn wifi_switch_and_cable_pull() {
        let mut w = NetWatch::new(at(0));
        w.tick(home(), at(0), TICK);
        assert_eq!(w.tick(cafe(), at(5), TICK), vec![NetSignal::Changed]);
        assert_eq!(w.tick(NONE, at(10), TICK), vec![NetSignal::Lost]);
        assert!(w.tick(NONE, at(15), TICK).is_empty());
        assert_eq!(w.tick(home(), at(20), TICK), vec![NetSignal::Changed]);
    }

    #[test]
    fn starting_without_network_reports_lost() {
        let mut w = NetWatch::new(at(0));
        assert_eq!(w.tick(NONE, at(0), TICK), vec![NetSignal::Lost]);
    }

    #[test]
    fn wake_on_another_network() {
        let mut w = NetWatch::new(at(0));
        w.tick(home(), at(0), TICK);
        assert_eq!(w.tick(cafe(), at(900), TICK), vec![NetSignal::Woke, NetSignal::Changed]);
    }

    #[test]
    fn ipv6_address_rotating_in_its_64_is_the_same_network() {
        let mut w = NetWatch::new(at(0));
        w.tick(Route::of(None, v6(1, 1)), at(0), TICK);
        assert!(w.tick(Route::of(None, v6(1, 2)), at(15), TICK).is_empty());
        assert!(w.tick(Route::of(None, v6(1, 3)), at(30), TICK).is_empty());
    }

    #[test]
    fn a_cellular_ipv4_route_coming_and_going_doesnt_reconnect() {
        // The 5G case: IPv6 steady, IPv4 there on one look and not the next.
        let mut w = NetWatch::new(at(0));
        let both = Route::of(Some(Ipv4Addr::new(100, 64, 0, 9)), v6(1, 1));
        let v6_only = Route::of(None, v6(1, 1));
        w.tick(both, at(0), TICK);
        for i in 1..10 {
            let r = if i % 2 == 0 { both } else { v6_only };
            assert!(w.tick(r, at(i * 15), TICK).is_empty(), "flap {i} reconnected");
        }
    }

    #[test]
    fn a_change_in_one_family_counts_once_it_holds() {
        let mut w = NetWatch::new(at(0));
        let both = Route::of(Some(Ipv4Addr::new(100, 64, 0, 9)), v6(1, 1));
        let v6_only = Route::of(None, v6(1, 1));
        w.tick(both, at(0), TICK);
        assert!(w.tick(v6_only, at(15), TICK).is_empty());
        assert_eq!(w.tick(v6_only, at(30), TICK), vec![NetSignal::Changed]);
    }

    #[test]
    fn wifi_to_cellular_on_both_families_is_at_once() {
        let mut w = NetWatch::new(at(0));
        w.tick(Route::of(v4(1, 20), v6(1, 1)), at(0), TICK);
        let cell = Route::of(Some(Ipv4Addr::new(100, 64, 0, 9)), v6(7, 1));
        assert_eq!(w.tick(cell, at(15), TICK), vec![NetSignal::Changed]);
    }

    fn dual() -> Route {
        Route::of(Some(Ipv4Addr::new(100, 64, 0, 9)), v6(1, 1))
    }
    fn v6_only() -> Route {
        Route::of(None, v6(1, 1))
    }

    #[test]
    fn a_pending_change_is_dropped_when_the_route_returns() {
        let mut w = NetWatch::new(at(0));
        w.tick(dual(), at(0), TICK);
        assert!(w.tick(v6_only(), at(3), TICK).is_empty());
        assert!(w.waiting());
        assert!(w.tick(dual(), at(6), TICK).is_empty());
        assert!(!w.waiting());
        // Back to v6 only later starts over: one look isn't enough.
        assert!(w.tick(v6_only(), at(21), TICK).is_empty());
    }

    #[test]
    fn a_pending_change_replaced_by_a_third_route_needs_its_own_second_look() {
        let mut w = NetWatch::new(at(0));
        w.tick(dual(), at(0), TICK);
        assert!(w.tick(v6_only(), at(3), TICK).is_empty());
        let other_v4 = Route::of(Some(Ipv4Addr::new(100, 64, 0, 10)), v6(1, 1));
        assert!(w.tick(other_v4, at(6), TICK).is_empty());
        assert_eq!(w.tick(other_v4, at(9), TICK), vec![NetSignal::Changed]);
        assert_eq!(w.current(), other_v4);
    }

    #[test]
    fn losing_the_network_while_a_change_waits_is_at_once() {
        let mut w = NetWatch::new(at(0));
        w.tick(dual(), at(0), TICK);
        assert!(w.tick(v6_only(), at(3), TICK).is_empty());
        assert_eq!(w.tick(NONE, at(6), TICK), vec![NetSignal::Lost]);
        assert!(!w.waiting());
    }

    #[test]
    fn an_ipv6_only_switch_between_64s_is_at_once() {
        let mut w = NetWatch::new(at(0));
        w.tick(Route::of(None, v6(1, 1)), at(0), TICK);
        assert_eq!(w.tick(Route::of(None, v6(2, 1)), at(15), TICK), vec![NetSignal::Changed]);
    }

    #[test]
    fn dual_stack_to_ipv6_only_holds_for_a_second_look() {
        let mut w = NetWatch::new(at(0));
        w.tick(dual(), at(0), TICK);
        assert!(w.tick(v6_only(), at(15), TICK).is_empty());
        assert_eq!(w.tick(v6_only(), at(18), TICK), vec![NetSignal::Changed]);
    }

    #[test]
    fn the_64_ends_at_the_fourth_segment() {
        let a = Ipv6Addr::new(0x2600, 1, 2, 3, 0, 0, 0, 1);
        // The fourth segment is part of the /64: another network.
        let b = Ipv6Addr::new(0x2600, 1, 2, 4, 0, 0, 0, 1);
        // The fifth and on are the host part: the same network.
        let c = Ipv6Addr::new(0x2600, 1, 2, 3, 9, 9, 9, 9);
        assert_ne!(Route::of(None, Some(a)), Route::of(None, Some(b)));
        assert_eq!(Route::of(None, Some(a)), Route::of(None, Some(c)));
    }

    #[test]
    fn waking_with_a_change_waiting_says_both() {
        let mut w = NetWatch::new(at(0));
        w.tick(dual(), at(0), TICK);
        assert!(w.tick(v6_only(), at(15), TICK).is_empty());
        assert_eq!(w.tick(v6_only(), at(900), TICK), vec![NetSignal::Woke, NetSignal::Changed]);
    }

    #[test]
    fn changes_are_described_without_addresses() {
        let both = Route::of(v4(1, 20), v6(1, 1));
        assert_eq!(describe_change(both, Route::of(None, v6(1, 1))), "IPv4 went");
        assert_eq!(describe_change(Route::of(None, v6(1, 1)), both), "IPv4 came");
        assert_eq!(describe_change(both, Route::of(v4(1, 20), v6(2, 1))), "another IPv6 network");
        assert_eq!(describe_change(home(), cafe()), "another IPv4 network");
        assert_eq!(describe_change(NONE, home()), "IPv4 came");
    }

    #[test]
    fn route_lookup_does_not_panic() {
        // Depends on the machine; only checks it returns a sane value.
        let r = route();
        assert!(r.v4.is_none_or(|a| !a.is_loopback()));
    }
}
