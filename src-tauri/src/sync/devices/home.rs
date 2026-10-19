//! The home side on device links, from the asking end: which of the
//! user's paired computers answer Lidarr for this device and are on now,
//! the route through the best of them, and sending one request to it (the
//! local network first, with time for Lidarr to answer, then the sealed
//! relay when it's allowed).

use std::sync::Arc;

use serde::Serialize;

use super::run::{DeviceSync, PathState, RELAY_MAX};
use crate::error::{AppError, AppResult};
use crate::homeside::computer::ComputerRoute;
use crate::homeside::link::{self, Link};
use crate::homeside::route::ANSWER_WAIT;
use crate::homeside::wire::Msg;
use crate::lidarr::api::LidarrRoute;
use crate::remote::peer::client::PeerError;
use crate::remote::peer::sync::{LanBody, PATH_SYNC_ENTRIES};

/// Answers taken from one reply on the local network at most.
const REPLY_MAX: usize = 64;

/// One paired device that answers Lidarr for this one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSide {
    pub id: String,
    pub name: String,
    /// Heard from lately, checked, and still offering it.
    pub on: bool,
}

impl DeviceSync {
    /// A device heard from within this long counts as on (rounds go every
    /// few minutes while both are open).
    fn online_ms(&self) -> u64 {
        2 * self.timing().every.as_millis() as u64 + 60_000
    }

    /// The paired devices that said they answer Lidarr for this one, the
    /// ones on now first (then the one heard from last).
    pub fn home_computers(&self) -> Vec<ComputerSide> {
        let now = self.peer.deps().store.now();
        let on = self.is_on();
        let mut out: Vec<(ComputerSide, u64)> = self
            .wire
            .links
            .all()
            .into_iter()
            .filter_map(|p| {
                let seen = self.peer.seen(&p.id).unwrap_or_default();
                if !(seen.lidarr || p.offers_lidarr) {
                    return None;
                }
                let heard = seen.heard_ms.unwrap_or(0);
                let fresh = seen.heard_ms.is_some_and(|t| now.saturating_sub(t) <= self.online_ms());
                let up = on
                    && seen.lidarr
                    && seen.verified
                    && seen.paused.is_none()
                    && fresh
                    && self.path(&p.id) != PathState::Away;
                let name = if seen.name.is_empty() { p.name.clone() } else { seen.name };
                Some((ComputerSide { id: p.id, name, on: up }, heard))
            })
            .collect();
        out.sort_by_key(|(c, heard)| (!c.on, std::cmp::Reverse(*heard)));
        out.into_iter().map(|(c, _)| c).collect()
    }

    /// The route through the best computer that's on now.
    pub fn computer_route(self: &Arc<Self>) -> Option<Arc<dyn LidarrRoute>> {
        let best = self.home_computers().into_iter().find(|c| c.on)?;
        let link = self.wire.links.link(&best.id)?;
        Some(Arc::new(ComputerRoute { sync: self.clone(), link, name: best.name }))
    }

    /// Send one Lidarr request to the computer on `link`. Its answer comes
    /// to `peer.answers` (at once on the local network; later through the
    /// relay).
    pub async fn ask_home(&self, link: &Link, name: &str, msg: Msg) -> AppResult<()> {
        if !self.is_on() {
            return Err(AppError::Invalid(format!("Turn on Keep my devices in sync to use Lidarr through {name}.")));
        }
        if let (Some(lan), Some(device)) = (self.lan(), self.wire.links.get(&link.id).and_then(|p| p.device)) {
            let (reply, follow) = {
                // In order with everything else on the local network.
                let lock = self.wire.sending(link);
                let _one = lock.lock().await;
                let frames = self.wire.seal_lan(link, &msg, false)?;
                let body = LanBody { link: link.id.clone(), msgs: vec![frames] };
                let r = lan.post_wait(&device, PATH_SYNC_ENTRIES, &body, ANSWER_WAIT).await;
                let mut follow = Vec::new();
                if let Ok(reply) = &r {
                    for frames in reply.msgs.iter().take(REPLY_MAX) {
                        if let Some(m) = self.wire.open_lan(link, frames, true) {
                            follow.extend(self.peer.handle(link, m).await);
                        }
                    }
                }
                (r, follow)
            };
            match reply {
                Ok(_) => {
                    self.lan_ok(&link.id);
                    // Anything else the computer said (a hello) is answered as usual.
                    self.send(link, follow).await;
                    return Ok(());
                }
                Err(PeerError::Unreachable) => {}
                Err(PeerError::Refused(why) | PeerError::Failed(why)) => {
                    return Err(AppError::Network(format!("{name} didn't take the request: {why}.")));
                }
            }
        }
        if !self.relay_allowed() {
            return Err(AppError::Network(format!(
                "{name} isn't on this network, and Sync when I'm away from home is off here, so Lidarr can't be asked through it."
            )));
        }
        if link::packed_len(&msg) > RELAY_MAX {
            return Err(AppError::Invalid("That's too much to send through the relay at once.".into()));
        }
        match self.wire.send(link, &msg).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let busy = e.to_string().contains(&crate::homeside::peer::relay_error(crate::social::listen::relay::RelayError::Limited));
                self.set_path_state(&link.id, if busy { PathState::RelayBusy } else { PathState::Away });
                Err(e)
            }
        }
    }
}
