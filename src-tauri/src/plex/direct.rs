//! Reaching a server by its `*.plex.direct` name when the network's DNS won't
//! answer for it.
//!
//! plex.tv hands out connections like `https://10-0-0-5.<id>.plex.direct:32400`.
//! The name spells the address (10.0.0.5, or an IPv6 address with `-` for
//! `:`), and Plex's DNS only ever answers with that address. Many routers
//! drop that answer as rebind protection, and some filters block the domain,
//! so the server looks offline although it is right there. DNS is still asked
//! first (a home DNS may point the name somewhere better on purpose), but when
//! it answers nothing, fails, answers with a blocked 0.0.0.0, or takes too
//! long, the address is read from the name. TLS still checks the server's
//! certificate against the full name, so nothing about trust changes.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tokio::net::TcpStream;

/// How long DNS gets for a plex.direct name before the name's own address is used.
const DNS_WAIT: Duration = Duration::from_millis(1500);

/// The address a `<address>.<id>.plex.direct` name stands for, or `None` for
/// any other name.
pub fn address(host: &str) -> Option<IpAddr> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let rest = host.strip_suffix(".plex.direct")?;
    let (label, id) = rest.split_once('.')?;
    if id.is_empty() || id.contains('.') {
        return None;
    }
    if let Ok(v4) = label.replace('-', ".").parse::<Ipv4Addr>() {
        return Some(v4.into());
    }
    label.replace('-', ":").parse::<Ipv6Addr>().ok().map(IpAddr::V6)
}

type Lookup = Arc<dyn Fn(String) -> Resolving + Send + Sync>;

/// DNS for HTTP clients that talk to Plex servers and players: the system's
/// answer, and for a plex.direct name the address in the name when DNS
/// doesn't give a usable one.
#[derive(Clone)]
pub struct Resolver {
    lookup: Lookup,
    wait: Duration,
}

impl Resolver {
    pub fn system() -> Self {
        Self::with_lookup(|host| {
            Box::pin(async move {
                let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
                Ok(Box::new(addrs.into_iter()) as Addrs)
            })
        })
    }

    /// Ask `lookup` instead of the system (tests fake the network's DNS).
    pub fn with_lookup(lookup: impl Fn(String) -> Resolving + Send + Sync + 'static) -> Self {
        Resolver { lookup: Arc::new(lookup), wait: DNS_WAIT }
    }

    #[cfg(test)]
    fn waiting(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    /// Addresses for `host`: DNS's answer, or for a plex.direct name its own
    /// address when DNS has nothing usable.
    async fn addrs(&self, host: String) -> Result<Vec<SocketAddr>, BoxError> {
        let Some(ip) = address(&host) else {
            return Ok((self.lookup)(host).await?.collect());
        };
        let asked = tokio::time::timeout(self.wait, (self.lookup)(host)).await;
        let answer: Vec<SocketAddr> = match asked {
            Ok(Ok(a)) => a.filter(|a| !a.ip().is_unspecified()).collect(),
            _ => Vec::new(),
        };
        Ok(if answer.is_empty() { vec![SocketAddr::new(ip, 0)] } else { answer })
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        let r = self.clone();
        let host = name.as_str().to_owned();
        Box::pin(async move { Ok(Box::new(r.addrs(host).await?.into_iter()) as Addrs) })
    }
}

/// Open a TCP connection to `host` the same way (for the notification socket).
pub async fn connect(host: &str, port: u16) -> std::io::Result<TcpStream> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return TcpStream::connect((ip, port)).await;
    }
    let addrs = Resolver::system().addrs(host.to_owned()).await.map_err(std::io::Error::other)?;
    let addrs: Vec<SocketAddr> = addrs.into_iter().map(|a| SocketAddr::new(a.ip(), port)).collect();
    TcpStream::connect(&addrs[..]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_address_from_the_name() {
        assert_eq!(address("10-0-0-5.example.plex.direct"), Some("10.0.0.5".parse().unwrap()));
        assert_eq!(address("10-20-30-40.EXAMPLE.Plex.Direct."), Some("10.20.30.40".parse().unwrap()));
        assert_eq!(address("2001-db8--1.example.plex.direct"), Some("2001:db8::1".parse().unwrap()));
        assert_eq!(
            address("2001-db8-85a3-0-0-8a2e-370-7334.example.plex.direct"),
            Some("2001:db8:85a3::8a2e:370:7334".parse().unwrap())
        );
    }

    #[test]
    fn other_names_are_left_to_dns() {
        for host in [
            "plex.direct",
            "example.plex.direct",
            "10-0-0-5.plex.direct",
            "10-0-0-5.a.b.plex.direct",
            "300-0-0-5.example.plex.direct",
            "server.example.plex.direct",
            "10-0-0-5.example.plex.direct.example.com",
            "music.example.com",
            "10.0.0.5",
        ] {
            assert_eq!(address(host), None, "{host}");
        }
    }

    fn answers(addrs: &'static [&'static str]) -> Resolver {
        Resolver::with_lookup(move |_| {
            Box::pin(async move { Ok(Box::new(addrs.iter().map(|a| a.parse().unwrap())) as Addrs) })
        })
    }

    async fn resolve(r: &Resolver, host: &str) -> Vec<SocketAddr> {
        r.resolve(host.parse().unwrap()).await.unwrap().collect()
    }

    const NAME: &str = "10-0-0-5.example.plex.direct";

    fn from_name() -> Vec<SocketAddr> {
        vec!["10.0.0.5:0".parse().unwrap()]
    }

    #[tokio::test]
    async fn a_dns_that_answers_nothing_still_finds_plex_direct() {
        let r = answers(&[]);
        assert_eq!(resolve(&r, NAME).await, from_name());
        assert!(resolve(&r, "music.example.com").await.is_empty());
    }

    #[tokio::test]
    async fn a_failing_or_blocking_dns_still_finds_plex_direct() {
        let failing = Resolver::with_lookup(|_| Box::pin(async { Err("refused".into()) }));
        assert_eq!(resolve(&failing, NAME).await, from_name());
        assert!(failing.resolve("music.example.com".parse().unwrap()).await.is_err());

        assert_eq!(resolve(&answers(&["0.0.0.0:0", "[::]:0"]), NAME).await, from_name());
    }

    #[tokio::test]
    async fn a_dns_that_never_answers_is_not_waited_on() {
        let stuck = Resolver::with_lookup(|_| Box::pin(std::future::pending())).waiting(Duration::from_millis(50));
        let started = std::time::Instant::now();
        assert_eq!(resolve(&stuck, NAME).await, from_name());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_home_dns_answer_for_the_name_is_kept() {
        let r = answers(&["10.0.0.9:0"]);
        assert_eq!(resolve(&r, NAME).await, ["10.0.0.9:0".parse().unwrap()]);
    }

    #[tokio::test]
    async fn connect_uses_the_address_in_the_name() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let s = connect("127-0-0-1.example.plex.direct", port).await.unwrap();
        assert_eq!(s.peer_addr().unwrap(), l.local_addr().unwrap());
    }
}
