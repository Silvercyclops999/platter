//! OpenSubsonic-compatible servers (Navidrome first; gonic, LMS and
//! Airsonic-Advanced as they allow) as music servers next to Plex and
//! Jellyfin.
//!
//! - [`client`]: the `/rest` API, signing in with an API key or a salted
//!   password token, and keeping both out of addresses that get logged.
//! - [`types`]: what the server sends.
//! - [`map`]: its items as the app's artists, albums, songs and playlists.
//! - [`server`]: one server behind every interface the app uses.
//! - [`manager`] + [`commands`]: adding, signing in again, folders,
//!   checking for changes, removing.

pub mod client;
pub mod commands;
pub mod manager;
pub mod map;
pub mod server;
pub mod types;

pub use manager::Subsonic;
pub use server::SubsonicServer;

#[cfg(test)]
mod tests;
