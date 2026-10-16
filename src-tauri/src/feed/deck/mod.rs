//! The local control API for a Stream Deck (or any paired local program):
//! what's playing, and play/pause, next, previous, like, volume, request the
//! album through Lidarr and start Listen along.
//!
//! It sits next to the now-playing feed with the same rules and more:
//! it binds 127.0.0.1 only, answers only a loopback peer that names it by
//! `127.0.0.1:<port>` or `localhost:<port>`, turns away anything a browser
//! sends (an `Origin` or `Sec-Fetch-Site` header), and is rate limited.
//! Apart from `/v1/hello` and the `/v1/pair` calls, every request needs the token a
//! client gets by pairing: allowed in Platter's window, or with the short
//! code shown in Settings. Only a hash
//! of that token is kept, in the OS keychain; Revoke forgets it.
//!
//! - `GET  /v1/hello` the app and whether the bearer token is paired
//! - `POST /v1/pair` `{"code": "123456"}` gives `{"token": "..."}`
//! - `POST /v1/pair/request` asks the person in Platter's window, gives
//!   `{"request": "<id>"}`; `POST /v1/pair/status` `{"request": "<id>"}`
//!   gives `{"state": "waiting" | "allowed" | "denied" | "gone"}`, with the
//!   token once allowed (see [`ask`])
//! - `GET  /v1/state?cover=<key>` see [`DeckState`]; the cover is left out
//!   when the client already has the one named by `cover`
//! - `POST /v1/command` `{"command": "playPause"}` and the others in
//!   [`Command`]
//! - `GET  /v1/cover?key=<key>&size=<px>` a cover named by a state or a list
//! - `GET  /v1/library?kind=playlists|albums|artists|mixes` what a key can play
//! - `GET  /v1/queue` the songs coming up
//! - `GET  /v1/devices` the Listen on list
//!
//! Automation (scripts, launchers, `platter://` command links and the
//! `platter --next` command line) uses the same API with its own pairing
//! (`"client": "automation"`), shown under Allow automation in Settings.
//!
//! Off by default (Settings, this device only); the port is open while
//! Allow Stream Deck or Allow automation is on.
//!
//! Layout: [`pairing`] the code and the token, [`ask`] pairing allowed in
//! the window, [`install`] the bundled plugin, [`server`] the listener and
//! the HTTP handling, [`app`] the real player, likes, Lidarr and Listen
//! along behind [`DeckHost`], [`commands`] the Tauri commands,
//! [`automation`] the `platter://` command links, [`cli`] the command line.

pub mod app;
pub mod ask;
pub mod automation;
pub mod cli;
pub mod commands;
pub mod install;
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
use crate::audio::queue::Repeat;
use crate::audio::source::{ContextKind, PlayContext};
use crate::model::Track;

/// The control API's port on 127.0.0.1 (the feed's port plus one).
pub const DECK_PORT: u16 = 32541;
/// The version of the API, in `/v1/hello` and every state.
pub const API_VERSION: u32 = 1;
/// Cover size on the key, in pixels (a key is 72 points, 144 pixels on high DPI).
pub const COVER_SIZE: u32 = 144;
/// The largest cover a client may ask for (an XL or high-DPI key).
pub const COVER_SIZE_MAX: u32 = 288;
/// Songs `/v1/queue` lists.
pub const QUEUE_MAX: usize = 30;
/// Items `/v1/library` lists for one kind.
pub const LIBRARY_MAX: usize = 500;

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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeckNow {
    pub track: Option<Track>,
    pub playing: bool,
    /// 0..1, the player's slider value.
    pub volume: f32,
    pub muted: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub shuffle: bool,
    pub smart_shuffle: bool,
    pub repeat: Repeat,
    /// The song after this one (the queue first, then the context).
    pub next: Option<Track>,
    /// What's playing from: a playlist, an album...
    pub context: Option<(ContextKind, Option<String>)>,
}

/// The rest of the state, outside the player.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeckExtras {
    /// Crossfade is on in Settings (smart or fixed).
    pub crossfade: bool,
    /// The miniplayer window is open.
    pub miniplayer: bool,
    /// The main window is showing.
    pub window: bool,
    /// Playback is on another device (its name).
    pub device: Option<String>,
    /// The playing song's star rating, 0 to 5 (0 unrated).
    pub stars: u8,
    /// The playing song's server has star ratings (Plex).
    pub can_rate: bool,
    /// Ids of the playlists holding the playing song.
    pub playlists: Vec<String>,
}

/// What the window says about itself (`deck_ui_report`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiReport {
    /// The expanded view shows lyrics.
    #[serde(default)]
    pub lyrics: bool,
}

/// One thing a key can play (`/v1/library`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckItem {
    pub id: String,
    pub title: String,
    /// The artist, a song count or what the mix is.
    pub subtitle: Option<String>,
    /// The picture, as an art path (sent to clients as a cover key).
    #[serde(skip)]
    pub thumb: Option<String>,
    pub cover_key: Option<String>,
}

/// Which list `/v1/library` gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LibraryKind {
    #[serde(alias = "playlist")]
    Playlists,
    #[serde(alias = "album")]
    Albums,
    #[serde(alias = "artist")]
    Artists,
    #[serde(alias = "mix")]
    Mixes,
}

impl LibraryKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "playlists" | "playlist" => Self::Playlists,
            "albums" | "album" => Self::Albums,
            "artists" | "artist" => Self::Artists,
            "mixes" | "mix" => Self::Mixes,
            _ => return None,
        })
    }

    pub fn context(self) -> ContextKind {
        match self {
            Self::Playlists => ContextKind::Playlist,
            Self::Albums => ContextKind::Album,
            Self::Artists => ContextKind::Artist,
            Self::Mixes => ContextKind::Mix,
        }
    }
}

/// One song coming up (`/v1/queue`).
#[derive(Debug, Clone, PartialEq)]
pub struct DeckQueued {
    pub item_id: String,
    pub track: Track,
}

/// A device playback can move to (`/v1/devices`). `local` is this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckDevice {
    pub id: String,
    pub name: String,
    /// Playing there now.
    pub active: bool,
    /// Moving there needs Platter's window (a password, or it can't be used).
    pub needs_window: bool,
}

/// The id `/v1/devices` gives this computer.
pub const LOCAL_DEVICE: &str = "local";

/// A link from sharing (`platter://album/...`), found in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeckLink {
    Album(String),
    Artist(String),
    Playlist(String),
    /// A song: its album, and where to start.
    Track { id: String, album_id: String, at_seconds: Option<u32> },
}

/// The sleep timer, as `/v1/state` shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SleepView {
    /// Time left, for a timer in minutes.
    pub remaining_ms: Option<u64>,
    /// Pauses when the playing song ends.
    pub end_of_song: bool,
}

/// The playing song after the one on now (`/v1/state`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NextUp {
    pub title: String,
    pub artist: String,
    pub cover_key: Option<String>,
}

/// What's playing from (`/v1/state`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextRef {
    pub kind: ContextKind,
    pub id: Option<String>,
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
    /// Where the song is and how long it is.
    pub position_ms: u64,
    pub duration_ms: u64,
    pub shuffle: bool,
    pub smart_shuffle: bool,
    /// `off`, `all` or `one`.
    pub repeat: Repeat,
    pub crossfade: bool,
    /// The expanded view shows lyrics.
    pub lyrics: bool,
    pub miniplayer: bool,
    /// The main window is showing.
    pub window: bool,
    /// Playback is on another device (its name).
    pub device: Option<String>,
    /// 0 to 5; `canRate` says whether the song's server has star ratings.
    pub stars: u8,
    pub can_rate: bool,
    /// Ids of the playlists holding the playing song.
    pub playlists: Vec<String>,
    /// The song after this one (left out in a private session).
    pub next: Option<NextUp>,
    pub context: Option<ContextRef>,
    pub sleep: Option<SleepView>,
}

/// `/v1/command`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "command")]
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
    /// Seek by `value` seconds (back when negative).
    SeekBy { value: f64 },
    /// Shuffle on or off; `smart` turns smart shuffle on or off.
    Shuffle {
        #[serde(default)]
        smart: bool,
    },
    /// Repeat: off, all, one, off...
    Repeat,
    Mute,
    /// Crossfade on (smart) or off in Settings.
    Crossfade,
    PrivateSession,
    /// Add the playing song to a playlist; without one, Platter asks which.
    AddToPlaylist {
        #[serde(default)]
        playlist: Option<String>,
    },
    RemoveFromPlaylist { playlist: String },
    /// Star rating 1 to 5; the rating the song has already clears it.
    Rate { stars: u8 },
    /// Play a playlist, album, artist or mix by id, or add it to the
    /// queue; `start` seconds into its first song.
    PlayItem {
        kind: LibraryKind,
        id: String,
        #[serde(default)]
        shuffle: bool,
        #[serde(default)]
        queue: bool,
        #[serde(default)]
        start: Option<f64>,
    },
    /// Play what a `platter://` sharing link points at.
    PlayLink { link: String },
    /// Play the top songs a search finds.
    PlaySearch { query: String },
    /// Radio from the playing song.
    Radio,
    /// `daylist`, `discover` or `liked`.
    PlayShortcut { name: String },
    QueueJump { item_id: String },
    /// Play on a device from `/v1/devices`; without one, Platter asks.
    ListenOn {
        #[serde(default)]
        device: Option<String>,
    },
    /// Open or close the lyrics, the miniplayer, the window.
    Lyrics,
    Miniplayer,
    Window,
    /// Bring the window up on the now playing view, or the queue.
    NowPlaying,
    Queue,
    /// Pause after `minutes`, or when the song ends.
    Sleep {
        #[serde(default)]
        minutes: Option<f64>,
        #[serde(default)]
        end_of_song: bool,
    },
    SleepCancel,
    /// Bookmark the playing song at this moment.
    Bookmark,
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
    /// Bring the window up (to ask about pairing).
    fn raise(&self);
    /// Crossfade, the windows, the device and the song's rating and playlists.
    async fn extras(&self, track: Option<&Track>) -> DeckExtras;
    async fn seek(&self, position_ms: u64) -> AppResult<()>;
    async fn set_shuffle(&self, on: bool) -> AppResult<()>;
    async fn set_smart_shuffle(&self, on: bool) -> AppResult<()>;
    async fn set_repeat(&self, repeat: Repeat) -> AppResult<()>;
    async fn set_muted(&self, muted: bool) -> AppResult<()>;
    /// Change Settings (a `settings_set` patch) and tell the window.
    async fn set_settings(&self, patch: serde_json::Value) -> AppResult<()>;
    /// `None` clears the rating.
    async fn rate(&self, track_id: &str, stars: Option<u8>) -> AppResult<()>;
    async fn playlist_add(&self, playlist_id: &str, track_id: &str) -> AppResult<()>;
    async fn playlist_remove(&self, playlist_id: &str, track_id: &str) -> AppResult<()>;
    /// Start a context (shuffled or not), at a song and time when given.
    async fn play(&self, ctx: PlayContext, start: Option<String>, start_ms: u64, shuffle: bool) -> AppResult<()>;
    /// Add a context's songs to the end of the queue.
    async fn queue_context(&self, ctx: PlayContext) -> AppResult<()>;
    /// What a sharing link points at, when it's in the library.
    async fn resolve_link(&self, link: &str) -> Option<DeckLink>;
    /// Ids of the top songs for a search.
    async fn search_tracks(&self, query: &str, limit: usize) -> AppResult<Vec<String>>;
    /// The made-for-you mix of a kind (`daylist`, `discover`).
    async fn mix_of_kind(&self, kind: &str) -> AppResult<Option<String>>;
    async fn library(&self, kind: LibraryKind) -> AppResult<Vec<DeckItem>>;
    /// The user queue, then what follows.
    async fn queue(&self) -> Vec<DeckQueued>;
    async fn queue_jump(&self, item_id: &str) -> AppResult<()>;
    async fn devices(&self) -> Vec<DeckDevice>;
    /// Move playback to a device ([`LOCAL_DEVICE`] brings it back here).
    async fn listen_on(&self, device: &str) -> AppResult<()>;
    async fn set_miniplayer(&self, open: bool) -> AppResult<()>;
    async fn set_window(&self, visible: bool) -> AppResult<()>;
    /// Ask the window to do something (`deck://action`); `bring_up` shows it first.
    fn window_action(&self, payload: serde_json::Value, bring_up: bool);
    async fn bookmark(&self, track: &Track, position_ms: u64) -> AppResult<()>;
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
