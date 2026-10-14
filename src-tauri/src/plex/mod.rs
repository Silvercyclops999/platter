//! Plex client: PIN sign-in, keychain, server discovery and connection race,
//! and the HTTP API used by sync, playback and tracking.
//!
//! Layout:
//! - [`token`]: the `Token` secret type (redacting `Debug`, never displayed).
//! - [`identity`]: per-install client identifier + `X-Plex-*` headers.
//! - [`keychain`]: `TokenStore` trait, OS keychain impl, in-memory impl.
//! - [`auth`]: plex.tv PIN flow, user and resources calls.
//! - [`discovery`]: server list, connection race (local > remote > relay), disk cache.
//! - [`client`]: `PlexClient`, the typed Plex Media Server API.
//! - [`notifications`]: server websocket events + reconnect backoff.
//! - [`health`]: pure health state machine behind the `health` command.
//! - [`details`]: the now-playing panel's extras (artist bio, credits, lyrics).
//! - [`types`]: IPC shapes for auth/servers/health (docs/IPC.md).
//! - [`state`] + [`commands`]: Tauri-managed state and commands.

pub mod auth;
pub mod client;
pub mod commands;
pub mod details;
pub mod discovery;
pub mod health;
pub mod identity;
pub mod keychain;
pub mod notifications;
pub mod state;
pub mod token;
pub mod types;

mod lenient;
#[cfg(test)]
mod live_tests;

pub use client::PlexClient;
pub use state::PlexState;
pub use token::Token;
