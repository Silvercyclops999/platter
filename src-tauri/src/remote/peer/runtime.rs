//! Keeps this computer announced and answering while "Show this computer to
//! my other devices" is on and Plex is signed in: binds the control API on
//! the LAN addresses, announces it over mDNS, and publishes the addresses to
//! plex.tv. Follows the setting, the name, sign-in and address changes.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;
use url::Url;

use super::directory::{Announce, PeerDirectory};
use super::server::{self, PeerServer, PlayerPeer};
use crate::remote::service::RemoteService;

/// This computer's LAN addresses (the real one reads the interfaces).
pub type AddrSource = Arc<dyn Fn() -> Vec<Ipv4Addr> + Send + Sync>;

#[derive(Clone)]
pub struct RuntimeConfig {
    pub first_port: u16,
    pub tries: u16,
    /// Look again at sign-in and addresses this often.
    pub recheck: Duration,
    /// plex.tv, to publish the addresses to (None: don't).
    pub publish_to: Option<Url>,
    pub platform: String,
    pub version: String,
}

impl RuntimeConfig {
    pub fn app(platform: String, version: String) -> Self {
        Self {
            first_port: super::FIRST_PORT,
            tries: super::PORT_TRIES,
            recheck: Duration::from_secs(30),
            publish_to: Some(Url::parse("https://plex.tv").expect("static url")),
            platform,
            version,
        }
    }
}

struct Running {
    stop: watch::Sender<bool>,
    ips: Vec<Ipv4Addr>,
    port: u16,
    announced: Announce,
    published: Option<Instant>,
}

/// Run until the switches go away. The port it listens on (once it does)
/// is sent on `port`.
pub async fn run(remote: RemoteService, dir: Arc<dyn PeerDirectory>, addrs: AddrSource, cfg: RuntimeConfig, port: watch::Sender<Option<u16>>) {
    let switches = remote.switches().clone();
    let mut on = switches.announce.subscribe();
    let mut name_rx = switches.name.subscribe();
    let own_id = remote.own_id().to_string();
    let name_switches = switches.clone();
    let handler = Arc::new(PlayerPeer {
        local: remote.local(),
        env: remote.env(),
        id: own_id.clone(),
        platform: cfg.platform.clone(),
        version: cfg.version.clone(),
        name: Arc::new(move || name_switches.computer_name()),
    });
    let keys = remote.peer_keys();
    let server = Arc::new(PeerServer::new(own_id.clone(), handler, keys.clone()).with_extra(remote.extra_routes()));
    let mut running: Option<Running> = None;
    // Addresses seen at the last check while running: a new set (or none)
    // takes over only once it holds for two checks in a row, so a phone's
    // network flapping between checks doesn't take the port down and up.
    let mut last_seen: Vec<Ipv4Addr> = Vec::new();
    loop {
        let want = *on.borrow_and_update();
        let _ = name_rx.borrow_and_update();
        let tag = if want { remote.account_tag().await } else { None };
        let mut ips = if want { addrs() } else { Vec::new() };
        ips.sort_unstable();
        ips.dedup();
        let moved = want && running.as_ref().is_some_and(|r| r.ips != ips);
        let settled = !moved || ips == last_seen;
        last_seen = ips.clone();
        if !settled {
            tokio::select! {
                r = on.changed() => if r.is_err() { break },
                r = name_rx.changed() => if r.is_err() { break },
                _ = tokio::time::sleep(cfg.recheck) => {}
            }
            continue;
        }
        match (want, tag) {
            (true, Some(tag)) if !ips.is_empty() => {
                if running.as_ref().is_some_and(|r| r.ips != ips) {
                    stop(&mut running, dir.as_ref(), &port);
                }
                if running.is_none() {
                    match server::bind(&ips, cfg.first_port, cfg.tries).await {
                        Ok((listeners, p)) => {
                            tracing::info!(port = p, addresses = ips.len(), "showing this computer to your other devices");
                            let (tx, rx) = watch::channel(false);
                            tokio::spawn(server::serve(listeners, server.clone(), rx));
                            running = Some(Running {
                                stop: tx,
                                ips: ips.clone(),
                                port: p,
                                announced: Announce { id: String::new(), txt: Vec::new(), ips: Vec::new(), port: 0 },
                                published: None,
                            });
                            let _ = port.send(Some(p));
                        }
                        Err(e) => tracing::warn!("couldn't open the control port: {e}"),
                    }
                }
                if let Some(r) = running.as_mut() {
                    // Keep the account's device list fresh, so the first
                    // request from another device doesn't wait on plex.tv.
                    let k = keys.clone();
                    tokio::spawn(async move { k.warm().await });
                    let a = Announce {
                        id: own_id.clone(),
                        txt: super::txt(&own_id, &switches.computer_name(), &cfg.platform, &cfg.version, &tag),
                        ips: r.ips.clone(),
                        port: r.port,
                    };
                    if a != r.announced {
                        dir.announce(&a);
                        r.announced = a;
                    }
                    if let Some(plex_tv) = &cfg.publish_to
                        && r.published.is_none_or(|t| t.elapsed() >= super::publish::EVERY)
                    {
                        r.published = Some(Instant::now());
                        let bases: Vec<Url> = r
                            .ips
                            .iter()
                            .filter_map(|ip| Url::parse(&format!("http://{ip}:{}", r.port)).ok())
                            .collect();
                        if let Some(token) = remote.env().account_token().await
                            && let Err(e) = super::publish::publish(&remote.http(), plex_tv, &token, &own_id, &bases).await
                        {
                            tracing::info!(kind = e.kind(), "couldn't publish this computer's addresses to plex.tv");
                        }
                    }
                }
            }
            _ => stop(&mut running, dir.as_ref(), &port),
        }
        tokio::select! {
            r = on.changed() => if r.is_err() { break },
            r = name_rx.changed() => if r.is_err() { break },
            // Off: nothing to recheck, so wait for the switch alone.
            _ = tokio::time::sleep(cfg.recheck), if want => {}
        }
    }
    stop(&mut running, dir.as_ref(), &port);
}

fn stop(running: &mut Option<Running>, dir: &dyn PeerDirectory, port: &watch::Sender<Option<u16>>) {
    if let Some(r) = running.take() {
        let _ = r.stop.send(true);
        dir.withdraw();
        let _ = port.send(None);
        tracing::info!("stopped showing this computer to your other devices");
    }
}
