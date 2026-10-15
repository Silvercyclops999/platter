//! The local control API for a Stream Deck (or any paired local program):
//! what's playing, and play/pause, next, previous, like, volume, request the
//! album through Lidarr and start Listen along.
//!
//! It sits next to the now-playing feed with the same rules and more:
//! it binds 127.0.0.1 only, answers only a loopback peer that names it by
//! `127.0.0.1:<port>` or `localhost:<port>`, turns away anything a browser
//! sends (an `Origin` or `Sec-Fetch-Site` header), and is rate limited.
//! Apart from `/v1/hello` and `/v1/pair`, every request needs the token a
//! client gets by pairing with the short code shown in Settings. Only a hash
//! of that token is kept, in the OS keychain; Revoke forgets it.
//!
//! - `GET  /v1/hello` the app and whether the bearer token is paired
//! - `POST /v1/pair` `{"code": "123456"}` gives `{"token": "..."}`
//! - `GET  /v1/state?cover=<key>` see [`DeckState`]; the cover is left out
//!   when the client already has the one named by `cover`
//! - `POST /v1/command` `{"command": "playPause"}` and the others in
//!   [`Command`]
//!
//! Automation (scripts, launchers, `platter://` command links and the
//! `platter --next` command line) uses the same API with its own pairing
//! (`"client": "automation"`), shown under Allow automation in Settings.
//!
//! Off by default (Settings, this device only); the port is open while
//! Allow Stream Deck or Allow automation is on.
//!
//! Layout: [`pairing`] the code and the token, [`server`] the listener and
//! the HTTP handling, [`app`] the real player, likes, Lidarr and Listen
//! along behind [`DeckHost`], [`commands`] the Tauri commands,
//! [`automation`] the `platter://` command links, [`cli`] the command line.

pub mod app;
pub mod automation;
pub mod cli;
pub mod commands;
pub mod pairing;
pub mod server;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::db::source::ArtBytes;
use crate::error::AppResult;
use crate::model::Track;

/// The control API's port on 127.0.0.1 (the feed's port plus one).
pub const DECK_PORT: u16 = 32541;
/// The version of the API, in `/v1/hello` and every state.
pub const API_VERSION: u32 = 1;
/// Cover size on the key, in pixels (a key is 72 points, 144 pixels on high DPI).
pub const COVER_SIZE: u32 = 144;

/// What the control API should be doing (from Settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeckConfig {
    /// Allow Stream Deck.
    pub enabled: bool,
    pub private_session: bool,
    /// Allow automation.
    pub automation: bool,
}

impl DeckConfig {
    /// Whether the port should be open.
    pub fn listening(&self) -> bool {
        self.enabled || self.automation
    }
}

/// Settings push changes here; the server follows. Cheap to clone.
#[derive(Clone)]
pub struct DeckSwitch(Arc<watch::Sender<DeckConfig>>);

impl Default for DeckSwitch {
    fn default() -> Self {
        Self(Arc::new(watch::Sender::new(DeckConfig::default())))
    }
}

impl DeckSwitch {
    pub fn set(&self, c: DeckConfig) {
        self.0.send_if_modified(|cur| {
            let changed = *cur != c;
            *cur = c;
            changed
        });
    }

    pub fn get(&self) -> DeckConfig {
        *self.0.borrow()
    }

    pub fn subscribe(&self) -> watch::Receiver<DeckConfig> {
        self.0.subscribe()
    }
}

/// Whether the port is open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Listening {
    Off,
    On { port: u16 },
    PortInUse { port: u16 },
    Failed { port: u16, message: String },
}

/// The Settings row (`deck_status`, `deck://status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckStatus {
    #[serde(flatten)]
    pub listening: Listening,
    /// The pairing code, while the API is on.
    pub code: Option<String>,
    /// A Stream Deck is paired.
    pub paired: bool,
    /// The automation pairing code, while automation is on.
    pub automation_code: Option<String>,
    /// A script or the command line is paired.
    pub automation_paired: bool,
}

/// What's playing, as the control API needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeckNow {
    pub track: Option<Track>,
    pub playing: bool,
    /// 0..1, the player's slider value.
    pub volume: f32,
    pub muted: bool,
}

/// `/v1/state`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckState {
    pub api: u32,
    pub playing: bool,
    /// A private session is on: title, artist, album and cover are left out.
    pub private: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_id: Option<String>,
    pub liked: bool,
    /// 0 to 100.
    pub volume: u8,
    pub muted: bool,
    /// Names the cover; send it back as `?cover=` to skip a cover you have.
    pub cover_key: Option<String>,
    /// The cover as a small data image, when the client doesn't have it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// Lidarr is set up, so Request album can work.
    pub can_request: bool,
}

/// `/v1/command`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", tag = "command")]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    /// Like the playing song, or unlike it when it's liked.
    Like,
    /// Volume to `value` percent.
    VolumeSet { value: f64 },
    /// Volume up or down by `value` percent points.
    VolumeStep { value: f64 },
    /// Open the Lidarr request for the playing song's album.
    RequestAlbum,
    /// Host a Listen along session and show its invite.
    ListenAlong,
    /// Play (resume), and pause.
    Play,
    Pause,
    /// Play a playlist found by id or name.
    PlayPlaylist {
        playlist: String,
        #[serde(default)]
        shuffle: bool,
    },
    /// Open Search with this text in the window.
    Search { query: String },
    /// Open a page in the window (see [`automation::page_route`]).
    Open { page: String },
}

/// The player, likes, art, Lidarr and Listen along (faked in tests).
#[async_trait]
pub trait DeckHost: Send + Sync {
    async fn now(&self) -> Option<DeckNow>;
    async fn liked(&self, track_id: &str) -> bool;
    async fn cover(&self, thumb: &str, size: u32) -> AppResult<ArtBytes>;
    fn can_request(&self) -> bool;
    async fn toggle(&self) -> AppResult<()>;
    async fn next(&self) -> AppResult<()>;
    async fn previous(&self) -> AppResult<()>;
    async fn set_liked(&self, track_id: &str, liked: bool) -> AppResult<()>;
    /// 0..1; unmutes when above 0.
    async fn set_volume(&self, volume: f32) -> AppResult<()>;
    async fn request_album(&self, track: &Track) -> AppResult<()>;
    async fn listen_along(&self) -> AppResult<()>;
    async fn pause(&self) -> AppResult<()>;
    async fn resume(&self) -> AppResult<()>;
    /// Play the playlist with this id, or the best match for this name.
    async fn play_playlist(&self, query: &str, shuffle: bool) -> AppResult<()>;
    /// Bring the window up on a page (a UI route).
    async fn show(&self, route: serde_json::Value) -> AppResult<()>;
    /// Tell the window something didn't happen (a toast).
    fn notice(&self, message: &str);
}

/// A token bucket: `rate` requests a second, up to `burst` at once.
#[derive(Debug, Clone)]
pub struct RateLimit {
    rate: f64,
    burst: f64,
    tokens: f64,
    at: Option<Instant>,
}

impl RateLimit {
    pub fn new(rate: f64, burst: f64) -> Self {
        Self { rate, burst, tokens: burst, at: None }
    }

    /// Take one request's worth; false when the bucket is empty.
    pub fn allow(&mut self, now: Instant) -> bool {
        if let Some(at) = self.at {
            let dt = now.saturating_duration_since(at).as_secs_f64();
            self.tokens = (self.tokens + dt * self.rate).min(self.burst);
        }
        self.at = Some(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Requests a second, all clients together (a key press is one; state
/// polling is about one a second).
pub const RATE: f64 = 20.0;
pub const BURST: f64 = 40.0;
/// Pairing attempts: one every two seconds, three at once.
pub const PAIR_RATE: f64 = 0.5;
pub const PAIR_BURST: f64 = 3.0;
/// How long a connection may take to send its request.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawn the server task. It follows `switch` for the life of the app.
pub fn spawn(
    switch: DeckSwitch,
    host: Arc<dyn DeckHost>,
    pairing: Arc<pairing::Pairing>,
    events: Arc<dyn crate::plex::state::EventSink>,
) -> server::DeckServer {
    let s = server::DeckServer::new(host, pairing, events);
    let run = s.clone();
    tauri::async_runtime::spawn(async move { run.follow(switch.subscribe(), DECK_PORT).await });
    s
}
