//! The now-playing feed for widgets: a small HTTP server on 127.0.0.1 only
//! (Settings, off by default, this device only) for OBS browser sources,
//! Rainmeter skins and other local widgets.
//!
//! - `GET /now-playing.json` the playing song: title, artists, album, art (a
//!   local URL, or a data URL with `?art=data`), position, duration, playing
//! - `GET /now-playing.html` a simple overlay page; the query string picks
//!   `size` (small, medium, large), `theme` (dark, light, clear), `art` and
//!   `progress` (1 or 0)
//! - `GET /art?t=<track id>` the playing song's cover from the art cache
//!
//! It binds the loopback address only, never the LAN, and answers only
//! requests addressed to it by that name (`Host: 127.0.0.1:<port>` or
//! `localhost:<port>`), so a web page can't reach it through a DNS trick.
//! JSON has no CORS header, so other sites can't read it. A private session
//! blanks everything. Turning the setting off closes the port.
//!
//! Layout: [`server`] the listener and HTTP handling, [`page`] the overlay
//! page, [`commands`] the Tauri command (docs/IPC.md "Now-playing feed").

pub mod commands;
pub mod deck;
pub mod page;
pub mod server;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::watch;

use crate::db::ArtCache;
use crate::db::source::ArtBytes;
use crate::error::AppResult;
use crate::model::Track;

/// The port the setting starts at.
pub const DEFAULT_PORT: u16 = 32540;
/// Lowest port the setting accepts (below it needs admin rights on some systems).
pub const PORT_MIN: u16 = 1024;

/// What the feed should be doing (from Settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedConfig {
    pub enabled: bool,
    pub port: u16,
    pub private_session: bool,
}

impl Default for FeedConfig {
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_PORT, private_session: false }
    }
}

/// Settings push changes here; the server follows. Cheap to clone.
#[derive(Clone)]
pub struct FeedSwitch(Arc<watch::Sender<FeedConfig>>);

impl Default for FeedSwitch {
    fn default() -> Self {
        Self(Arc::new(watch::Sender::new(FeedConfig::default())))
    }
}

impl FeedSwitch {
    pub fn set(&self, c: FeedConfig) {
        self.0.send_if_modified(|cur| {
            let changed = *cur != c;
            *cur = c;
            changed
        });
    }

    pub fn get(&self) -> FeedConfig {
        *self.0.borrow()
    }

    pub fn subscribe(&self) -> watch::Receiver<FeedConfig> {
        self.0.subscribe()
    }
}

/// The feed's state for its Settings row (`feed_status`, `feed://status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum FeedStatus {
    Off,
    /// Serving at `url` (the JSON; the overlay page is next to it).
    On { port: u16, url: String },
    /// Another program has the port.
    PortInUse { port: u16 },
    /// The port couldn't be opened for another reason.
    Failed { port: u16, message: String },
}

/// What's playing, as the feed needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Now {
    pub track: Track,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
}

/// Where the feed reads the player and covers (faked in tests).
#[async_trait]
pub trait FeedSource: Send + Sync {
    async fn now(&self) -> Option<Now>;
    async fn art(&self, path: &str, size: u32) -> AppResult<ArtBytes>;
}

/// The real source: the player and the art cache.
pub struct AppSource {
    pub player: crate::audio::player::PlayerHandle,
    pub art: ArtCache,
}

#[async_trait]
impl FeedSource for AppSource {
    async fn now(&self) -> Option<Now> {
        use crate::audio::events::PlayerStatus;
        let s = self.player.state().await.ok()?;
        let track = s.track?;
        Some(Now {
            playing: matches!(s.status, PlayerStatus::Playing | PlayerStatus::Buffering),
            position_ms: s.position_ms,
            duration_ms: if s.duration_ms > 0 { s.duration_ms } else { track.duration_ms },
            track,
        })
    }

    async fn art(&self, path: &str, size: u32) -> AppResult<ArtBytes> {
        self.art.get_or_fetch(path, size, size).await
    }
}

/// The JSON at `/now-playing.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedJson {
    /// Something is playing (not paused or stopped).
    pub playing: bool,
    /// A private session is on: everything else is empty.
    pub private: bool,
    pub title: Option<String>,
    /// The artist line as one string ("A feat. B").
    pub artist: Option<String>,
    /// The names in the artist line.
    pub artists: Vec<String>,
    pub album: Option<String>,
    /// The cover: a URL on this server, or a data URL when asked for.
    pub art: Option<String>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub track_id: Option<String>,
}

/// The track's artist line: the track artists, else the album artist.
pub fn artist_line(t: &Track) -> String {
    t.track_artist.as_deref().map(str::trim).filter(|a| !a.is_empty()).unwrap_or(&t.artist_title).to_owned()
}

/// Spawn the server task. It follows `switch` for the life of the app.
pub fn spawn(switch: FeedSwitch, source: Arc<dyn FeedSource>, events: Arc<dyn crate::plex::state::EventSink>) -> server::FeedServer {
    let s = server::FeedServer::new(source, events);
    let run = s.clone();
    tauri::async_runtime::spawn(async move { run.follow(switch.subscribe()).await });
    s
}
