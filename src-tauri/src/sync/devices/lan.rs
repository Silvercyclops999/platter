//! The local network path for device links: when two linked devices can
//! reach each other, messages go as signed requests on the control port
//! (`remote::peer::sync`) instead of through the relay. A request and its
//! answer carry a whole exchange, so a round, a first copy or anything big
//! goes here. Every message is still sealed for its link.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use url::Url;

use super::run::DeviceSync;
use crate::error::{AppError, AppResult};
use crate::homeside::link::{Link, Side};
use crate::homeside::wire::Msg;
use crate::remote::peer::client::{PeerClient, PeerError, SIGNED_OUT};
use crate::remote::peer::directory::PeerDirectory;
use crate::remote::peer::from_mdns;
use crate::remote::peer::sync::{LanBody, PATH_SYNC_ENTRIES, PATH_SYNC_HELLO, PeerSync};
use crate::sync::engine::AccountTag;

/// Sealed messages in one request at most (the control port takes 512 KB).
const REQUEST_MAX: usize = 400 * 1024;
/// Exchanges in one go before the rest waits for the next round.
const EXCHANGES_MAX: usize = 32;
/// Messages taken from one request.
const MSGS_MAX: usize = 64;
/// How long one exchange may take on the local network.
const LAN_TIMEOUT: Duration = Duration::from_secs(6);
/// After a device isn't reached on the local network, it isn't tried
/// there again for this long (doubling each time, up to `DOWN_MAX`).
const DOWN_FIRST: Duration = Duration::from_secs(30);
const DOWN_MAX: Duration = Duration::from_secs(5 * 60);

/// Sending to a linked device on the local network.
#[async_trait]
pub trait LanSend: Send + Sync {
    /// Post sealed messages to the device with this id; its sealed answers.
    async fn post(&self, device: &str, path: &str, body: &LanBody) -> Result<LanBody, PeerError>;
}

/// A client signed with this device's key; None when signed out.
#[async_trait]
pub trait ClientSource: Send + Sync {
    async fn client(&self) -> Option<PeerClient>;
}

#[async_trait]
impl<F: Fn() -> Option<PeerClient> + Send + Sync> ClientSource for F {
    async fn client(&self) -> Option<PeerClient> {
        self()
    }
}

#[async_trait]
impl ClientSource for crate::remote::RemoteService {
    async fn client(&self) -> Option<PeerClient> {
        self.peer_client().await
    }
}

/// Finds the user's other devices over mDNS (same account only) and posts
/// to them, signed.
pub struct PeerLan {
    pub client: Arc<dyn ClientSource>,
    pub directory: Arc<dyn PeerDirectory>,
    pub account: Arc<dyn AccountTag>,
    pub own_id: String,
    pub timeout: Duration,
    known: Mutex<HashMap<String, Vec<Url>>>,
    /// Devices not reached here lately: until when, and the next wait.
    down: Mutex<HashMap<String, (std::time::Instant, Duration)>>,
}

impl PeerLan {
    pub fn new(
        client: Arc<dyn ClientSource>,
        directory: Arc<dyn PeerDirectory>,
        account: Arc<dyn AccountTag>,
        own_id: String,
    ) -> Self {
        Self { client, directory, account, own_id, timeout: LAN_TIMEOUT, known: Mutex::default(), down: Mutex::default() }
    }

    /// Not reached lately: the relay is used without waiting on this path.
    fn is_down(&self, device: &str) -> bool {
        self.down.lock().get(device).is_some_and(|(until, _)| std::time::Instant::now() < *until)
    }

    fn mark(&self, device: &str, reached: bool) {
        let mut d = self.down.lock();
        if reached {
            d.remove(device);
        } else {
            let wait = d.get(device).map_or(DOWN_FIRST, |(_, w)| (*w * 2).min(DOWN_MAX));
            d.insert(device.to_string(), (std::time::Instant::now() + wait, wait));
        }
    }

    async fn find(&self, device: &str) -> Vec<Url> {
        let Some(tag) = self.account.tag().await else { return Vec::new() };
        let found: Vec<Url> = self
            .directory
            .scan()
            .await
            .iter()
            .filter_map(|a| from_mdns(a, &self.own_id, &tag))
            .filter(|d| d.id == device)
            .flat_map(|d| d.bases)
            .collect();
        if !found.is_empty() {
            self.known.lock().insert(device.to_string(), found.clone());
        }
        found
    }
}

#[async_trait]
impl LanSend for PeerLan {
    async fn post(&self, device: &str, path: &str, body: &LanBody) -> Result<LanBody, PeerError> {
        let client = self.client.client().await.ok_or_else(|| PeerError::Refused(SIGNED_OUT.into()))?;
        if self.is_down(device) {
            return Err(PeerError::Unreachable);
        }
        let r = self.post_now(&client, device, path, body).await;
        self.mark(device, !matches!(r, Err(PeerError::Unreachable)));
        r
    }
}

impl PeerLan {
    async fn post_now(&self, client: &PeerClient, device: &str, path: &str, body: &LanBody) -> Result<LanBody, PeerError> {
        let cached = self.known.lock().get(device).cloned();
        let tries = match cached {
            Some(c) => vec![c, Vec::new()],
            None => vec![Vec::new()],
        };
        for bases in tries {
            let bases = if bases.is_empty() { self.find(device).await } else { bases };
            for b in &bases {
                match client.sync_exchange(b, device, path, body, self.timeout).await {
                    Err(PeerError::Unreachable) => continue,
                    r => return r,
                }
            }
            self.known.lock().remove(device);
        }
        Err(PeerError::Unreachable)
    }
}

impl DeviceSync {
    /// Send messages on a link: over the local network when the other
    /// device answers there (with every answer handled until the exchange
    /// is done), otherwise through the relay.
    pub async fn send(&self, link: &Link, msgs: Vec<Msg>) {
        if msgs.is_empty() {
            return;
        }
        let left = self.try_lan(link, msgs).await;
        if !left.is_empty() {
            self.deliver(link, left).await;
        }
    }

    /// The local network part of [`send`](Self::send): what's left for the
    /// relay (everything, when the other device isn't reachable here).
    async fn try_lan(&self, link: &Link, mut msgs: Vec<Msg>) -> Vec<Msg> {
        let Some(lan) = self.lan() else { return msgs };
        let Some(device) = self.wire.links.get(&link.id).and_then(|p| p.device) else { return msgs };
        let lock = self.wire.sending(link);
        let _one = lock.lock().await;
        let mut exchanges = 0;
        while !msgs.is_empty() {
            if exchanges == EXCHANGES_MAX {
                tracing::debug!("device sync: more next round");
                return Vec::new();
            }
            exchanges += 1;
            // As many as fit in one request.
            let mut sealed = Vec::new();
            let mut size = 0;
            let mut n = 0;
            for m in &msgs {
                let Ok(frames) = self.wire.seal_lan(link, m, false) else { break };
                let len: usize = frames.iter().map(String::len).sum();
                if n > 0 && size + len > REQUEST_MAX {
                    break;
                }
                size += len;
                n += 1;
                sealed.push(frames);
            }
            if n == 0 {
                // Couldn't seal (too big): the relay path decides.
                return msgs;
            }
            let path = if matches!(msgs[0], Msg::SyncHello(_)) { PATH_SYNC_HELLO } else { PATH_SYNC_ENTRIES };
            let body = LanBody { link: link.id.clone(), msgs: sealed };
            match lan.post(&device, path, &body).await {
                Ok(reply) => {
                    let sent: Vec<Msg> = msgs.drain(..n).collect();
                    let link2 = link.clone();
                    self.peer.blocking(move |p| p.sent_all(&link2, &sent)).await;
                    self.lan_ok(&link.id);
                    for frames in reply.msgs.iter().take(MSGS_MAX) {
                        if let Some(m) = self.wire.open_lan(link, frames, true) {
                            msgs.extend(self.peer.handle(link, m).await);
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(?e, "device sync: not on the local network, using the relay");
                    return msgs;
                }
            }
        }
        // The exchange ran to its end, every answer taken: in step.
        if self.peer.verified(&link.id) {
            self.peer.mark_synced(&link.id);
        }
        Vec::new()
    }
}

#[async_trait]
impl PeerSync for DeviceSync {
    async fn exchange(&self, from: &str, req: LanBody) -> AppResult<LanBody> {
        if !self.is_on() {
            return Err(AppError::NotFound("Keep my devices in sync is off on that device.".into()));
        }
        let gone = || AppError::NotFound("That device isn't linked here.".into());
        let link = self.wire.links.link(&req.link).filter(|l| l.side == Side::Device).ok_or_else(gone)?;
        // A link learns the other device's id from its hello; a signed
        // request from any other device of the account isn't its own.
        if self.wire.links.get(&link.id).and_then(|p| p.device).is_some_and(|d| d != from) {
            return Err(gone());
        }
        let lock = self.reply_lock(&link.id);
        let _one = lock.lock().await;
        let mut replies = Vec::new();
        for frames in req.msgs.iter().take(MSGS_MAX) {
            let Some(m) = self.wire.open_lan(&link, frames, false) else { continue };
            // The request's signature proved the account.
            self.peer.verified_on_lan(&link.id);
            replies.extend(self.peer.handle(&link, m).await);
        }
        let mut out = LanBody { link: link.id.clone(), msgs: Vec::new() };
        let mut sent = Vec::new();
        for r in replies {
            match self.wire.seal_lan(&link, &r, true) {
                Ok(frames) => {
                    sent.push(r);
                    out.msgs.push(frames);
                }
                Err(e) => tracing::warn!("device sync: couldn't seal an answer: {e}"),
            }
        }
        let link2 = link.clone();
        self.peer.blocking(move |p| p.sent_all(&link2, &sent)).await;
        self.lan_ok(&link.id);
        Ok(out)
    }
}
