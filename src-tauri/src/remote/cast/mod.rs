//! Google Cast devices as Listen on targets.
//!
//! Playing starts Plex's own Cast receiver with a server play queue, the way
//! Plex apps cast: the device then streams from the server and walks the
//! queue by itself. If that receiver doesn't start or refuses the load, the
//! default media receiver gets the queue as stream URLs instead.
//!
//! - [`proto`]: message framing.
//! - [`conn`]: the TLS connection (heartbeat, requests, status).
//! - [`link`]: launch, load, controls and status as a [`super::lan::LanLink`].

pub mod conn;
pub mod link;
pub mod plan;
pub mod proto;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;

use std::net::SocketAddr;

use super::lan::{MdnsAnswer, lan_player};
use super::types::{PlayerKind, PlayerVia, RemotePlayer};

pub use link::{CastLink, CastPlan, DirectItem, PlexLoad};

/// Plex's Cast receiver app.
pub const PLEX_APP: &str = "9AC194DC";
/// The default media receiver.
pub const DEFAULT_APP: &str = "CC1AD845";

/// A Cast device or group found on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastDevice {
    /// "cast:<uuid>".
    pub id: String,
    pub name: String,
    /// Model name ("Chromecast", "Google Home", "Google Cast Group"...).
    pub model: String,
    pub addr: SocketAddr,
    pub kind: PlayerKind,
    /// What the device shows now (its status text), if anything.
    pub status: Option<String>,
}

/// Cast capability bit for video output.
const CA_VIDEO_OUT: u64 = 1;

fn kind_of(model: &str, ca: Option<u64>) -> PlayerKind {
    let m = model.to_ascii_lowercase();
    if m.contains("group") {
        PlayerKind::Group
    } else {
        let speaker = match ca {
            Some(c) => c & CA_VIDEO_OUT == 0,
            None => ["audio", "home", "nest", "speaker"].iter().any(|w| m.contains(w)) && !m.contains("tv"),
        };
        if speaker { PlayerKind::Speaker } else { PlayerKind::Tv }
    }
}

/// A `_googlecast._tcp` answer as a device. None without an id or address.
pub fn from_mdns(a: &MdnsAnswer) -> Option<CastDevice> {
    let id = a.txt("id")?;
    let addr = a.socket_addr()?;
    let model = a.txt("md").unwrap_or("Chromecast").to_string();
    let name = a.txt("fn").map(str::to_string).unwrap_or_else(|| model.clone());
    let ca = a.txt("ca").and_then(|c| c.parse::<u64>().ok());
    Some(CastDevice {
        id: format!("cast:{id}"),
        name,
        kind: kind_of(&model, ca),
        model,
        addr,
        status: a.txt("rs").map(str::to_string),
    })
}

impl CastDevice {
    pub fn player(&self) -> RemotePlayer {
        let product = if self.kind == PlayerKind::Group {
            "Speaker group".to_string()
        } else {
            self.model.clone()
        };
        let mut p = lan_player(self.id.clone(), self.name.clone(), product, self.kind, PlayerVia::Cast);
        p.playing = self.status.clone();
        p
    }
}
