//! Jellyfin servers (10.9 and later) as music servers next to Plex.
//!
//! - [`client`]: the HTTP API with the `MediaBrowser` authorization header.
//! - [`auth`]: Quick Connect, password sign-in, the token in the keychain.
//! - [`types`]: what Jellyfin sends.
//! - [`map`]: its items as the app's artists, albums, songs and playlists.
//! - [`server`]: one server behind every interface the app uses.
//! - [`likes`]: when each favourite was first seen.
//! - [`live`]: library changes from the server's websocket.
//! - [`manager`] + [`commands`]: adding, signing in again, libraries,
//!   primary server, removing.

pub mod auth;
pub mod client;
pub mod commands;
pub mod likes;
pub mod live;
pub mod manager;
pub mod map;
pub mod server;
pub mod types;

pub use manager::Jellyfin;
pub use server::JellyfinServer;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod live_tests;
