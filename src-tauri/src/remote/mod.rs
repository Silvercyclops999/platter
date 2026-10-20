//! Listen on other Plex players: find them, move playback there through a
//! server play queue, follow and control them, and bring playback back.
//! Optionally Platter answers as a Plex player itself, so other Plex apps
//! can play on this computer (off by default; it opens a local port).
//!
//! Layout:
//! - [`types`]: IPC shapes (`RemotePlayer`, `RemoteState`, `RemoteCommand`).
//! - [`xml`]: the small XML reader players and plex.tv answers need.
//! - [`discovery`]: plex.tv resources, the server's `/clients`, Sonos, and
//!   `/status/sessions`, merged by client identifier.
//! - [`companion`]: commands to a player (direct, through the server, or
//!   through the Sonos relay) and its timeline.
//! - [`playqueue`]: server play queues (create and read).
//! - [`service`]: the move / follow / return state machine.
//! - [`target`]: Platter as a player (HTTP companion endpoints and GDM).
//! - [`lan`]: Cast devices and AirPlay receivers found over mDNS, and the
//!   link the service follows for them.
//! - [`cast`]: Google Cast (Plex's receiver with a play queue, or the
//!   default media receiver).
//! - [`airplay`]: AirPlay audio, played through a network output.
//! - [`dlna`]: DLNA / UPnP renderers found with SSDP, fed stream URLs.
//! - [`netif`]: which network adapters mDNS uses (virtual ones left out).
//! - [`peer`]: the user's other Platter devices (announce, find, control).
//! - [`commands`]: Tauri commands.
//!
//! Protocol notes and which calls are confirmed live: docs/PLEX-API.md.

pub mod airplay;
pub mod cast;
pub mod commands;
pub mod companion;
pub mod dlna;
pub mod discovery;
pub mod lan;
pub mod netif;
pub mod peer;
pub mod playqueue;
pub mod service;
pub mod target;
pub mod types;
pub mod xml;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::watch;

pub use service::{LanEnv, LocalPlayer, RemoteConfig, RemoteEnv, RemoteService, ServerCtx};
pub use types::{RemoteCommand, RemotePlayer, RemoteSession, RemoteState};

/// The Listen on settings the core reads directly.
#[derive(Clone)]
pub struct RemoteSwitches {
    /// Show Plex players in the device picker.
    pub show_players: Arc<AtomicBool>,
    /// Show Cast devices and AirPlay receivers (mDNS runs only while on).
    pub show_cast: Arc<AtomicBool>,
    /// Show DLNA / UPnP renderers (searched for only while on).
    pub show_dlna: Arc<AtomicBool>,
    /// Moving playback stops it on the device it leaves.
    pub stop_old: Arc<AtomicBool>,
    /// Let other Plex apps play on this computer (starts the companion
    /// server and the network announcement).
    pub allow_casting: watch::Sender<bool>,
    /// Show this computer to the user's other Platter devices (announce
    /// and answer the control API).
    pub announce: watch::Sender<bool>,
    /// The name other devices see (the setting, or the OS name).
    pub name: watch::Sender<String>,
    /// List AirPlay receivers that need a password or pairing.
    pub show_airplay_locked: Arc<AtomicBool>,
}

impl Default for RemoteSwitches {
    fn default() -> Self {
        Self {
            show_players: Arc::new(AtomicBool::new(true)),
            show_cast: Arc::new(AtomicBool::new(true)),
            show_dlna: Arc::new(AtomicBool::new(true)),
            stop_old: Arc::new(AtomicBool::new(true)),
            allow_casting: watch::channel(false).0,
            announce: watch::channel(true).0,
            name: watch::channel(String::new()).0,
            show_airplay_locked: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl RemoteSwitches {
    pub fn set(&self, show_players: bool, stop_old: bool, allow_casting: bool) {
        self.show_players.store(show_players, Ordering::Relaxed);
        self.stop_old.store(stop_old, Ordering::Relaxed);
        self.allow_casting.send_if_modified(|v| {
            let changed = *v != allow_casting;
            *v = allow_casting;
            changed
        });
    }
    /// The Platter device settings: announce, name (empty: the OS name),
    /// and AirPlay receivers with a password.
    pub fn set_peer(&self, announce: bool, name: &str, airplay_locked: bool) {
        self.announce.send_if_modified(|v| {
            let changed = *v != announce;
            *v = announce;
            changed
        });
        let name = peer::display_name(name);
        self.name.send_if_modified(|v| {
            let changed = *v != name;
            *v = name;
            changed
        });
        self.show_airplay_locked.store(airplay_locked, Ordering::Relaxed);
    }
    /// The name other devices see.
    pub fn computer_name(&self) -> String {
        let n = self.name.borrow().clone();
        if n.is_empty() { peer::os_computer_name() } else { n }
    }
    pub fn show_airplay_locked(&self) -> bool {
        self.show_airplay_locked.load(Ordering::Relaxed)
    }
    pub fn set_show_cast(&self, on: bool) {
        self.show_cast.store(on, Ordering::Relaxed);
    }
    pub fn show_cast(&self) -> bool {
        self.show_cast.load(Ordering::Relaxed)
    }
    pub fn set_show_dlna(&self, on: bool) {
        self.show_dlna.store(on, Ordering::Relaxed);
    }
    pub fn show_dlna(&self) -> bool {
        self.show_dlna.load(Ordering::Relaxed)
    }
    pub fn show_players(&self) -> bool {
        self.show_players.load(Ordering::Relaxed)
    }
    pub fn stop_old(&self) -> bool {
        self.stop_old.load(Ordering::Relaxed)
    }
}
