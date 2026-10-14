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
//! - [`commands`]: Tauri commands.
//!
//! Protocol notes and which calls are confirmed live: docs/PLEX-API.md.

pub mod commands;
pub mod companion;
pub mod discovery;
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

pub use service::{LocalPlayer, RemoteConfig, RemoteEnv, RemoteService, ServerCtx};
pub use types::{RemoteCommand, RemotePlayer, RemoteSession, RemoteState};

/// The Listen on settings the core reads directly.
#[derive(Clone)]
pub struct RemoteSwitches {
    /// Show Plex players in the device picker.
    pub show_players: Arc<AtomicBool>,
    /// Moving playback stops it on the device it leaves.
    pub stop_old: Arc<AtomicBool>,
    /// Let other Plex apps play on this computer (starts the companion
    /// server and the network announcement).
    pub allow_casting: watch::Sender<bool>,
}

impl Default for RemoteSwitches {
    fn default() -> Self {
        Self {
            show_players: Arc::new(AtomicBool::new(true)),
            stop_old: Arc::new(AtomicBool::new(true)),
            allow_casting: watch::channel(false).0,
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
    pub fn show_players(&self) -> bool {
        self.show_players.load(Ordering::Relaxed)
    }
    pub fn stop_old(&self) -> bool {
        self.stop_old.load(Ordering::Relaxed)
    }
}
