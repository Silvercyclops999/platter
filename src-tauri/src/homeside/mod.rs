//! Platter Bridge: Platter's home side, headless, for a server that's on
//! when no desktop is. Optional: nothing here runs on the server unless the
//! user installs the Bridge, and Platter works fully without it.
//!
//! The Bridge and desktops talk only through the sealed relay (the same
//! ntfy relay and sealing as listen along), over outbound connections. They
//! pair once with a code the desktop shows (`code`, `pair`); after that
//! every message is sealed for its link (`link`), split to fit the relay and
//! put back together, with replays dropped (`peer`).
//!
//! - [`run`]: the Bridge (the `platter-bridge` binary): Lidarr catch-up,
//!   Bring in my listening once a desktop hands it over,
//!   promotions and swaps when the music folder is mounted read-write, the
//!   home Platter route, shared state with the desktops.
//! - [`desktop`]: Settings > Bridge, and the desktop's half of the link.
//! - [`route`]: the home Platter route, both halves.
//! - [`links`]: pairings and per-link state; [`keys`]: the Bridge's secrets file.
//! - [`health`]: `/health` on 127.0.0.1 for Docker's healthcheck.
//!
//! The Lidarr side of it (settings writes queued, modes shared by desktops,
//! the Bridge route) is in `crate::lidarr::homeside`.

pub mod code;
pub mod commands;
pub mod desktop;
pub mod health;
pub mod keys;
pub mod link;
pub mod links;
pub mod pair;
pub mod peer;
pub mod route;
pub mod run;
pub mod wire;

#[cfg(test)]
mod tests;

pub use crate::social::listen::DEFAULT_RELAY;

/// The build's version, as the Bridge reports it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
