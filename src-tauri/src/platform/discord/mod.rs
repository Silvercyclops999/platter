//! Discord status: what's playing, shown on the user's Discord profile
//! through the Discord desktop client's local IPC socket. No bot, no server.
//!
//! Layout:
//! - [`ipc`]      the socket, frames, handshake and SET_ACTIVITY
//! - [`activity`] what Discord shows (pure), and which URLs may go there
//! - [`art`]      public cover art (Cover Art Archive, Imgur, catbox.moe) and its cache
//! - [`commands`] Tauri commands and the real Plex, art cache and share glue
//!
//! The service follows the player's tracking events on its own task, so a
//! missing, slow or closing Discord never touches playback. Updates are
//! coalesced: at most one per [`Config::gap`], always with the latest state.

pub mod activity;
pub mod art;
pub mod commands;
pub mod ipc;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;

use crate::artistinfo::service::KeyState;
use crate::audio::events::TrackingEvent;
use activity::{Kind, Song, StatusLine};
use art::{ArtOutcome, ArtRequest, ArtResolver};
use ipc::{Connector, IpcClient, Reply};

/// The Discord application id, set when the app is built
/// (`PLATTER_DISCORD_APP_ID`). Without one the feature is hidden.
pub fn app_id() -> Option<&'static str> {
    option_env!("PLATTER_DISCORD_APP_ID")
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

/// The Settings this module acts on. All are per device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscordSettings {
    pub enabled: bool,
    pub show_unreleased: bool,
    pub unreleased_art: bool,
    pub clear_on_pause: bool,
    pub find_song: bool,
    pub cover_archive: bool,
    pub upload_art: bool,
    /// Upload covers to catbox.moe (no key).
    pub upload_catbox: bool,
    /// What the member list shows after "Listening to".
    pub status_line: StatusLine,
    /// A private session hides everything.
    pub private_session: bool,
}

impl Default for DiscordSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            show_unreleased: true,
            unreleased_art: true,
            clear_on_pause: true,
            find_song: true,
            cover_archive: true,
            upload_art: false,
            upload_catbox: false,
            status_line: StatusLine::App,
            private_session: false,
        }
    }
}

/// Shared switch the settings code writes and the service watches.
#[derive(Clone)]
pub struct DiscordSwitches(Arc<watch::Sender<DiscordSettings>>);

impl Default for DiscordSwitches {
    fn default() -> Self {
        Self(Arc::new(watch::channel(DiscordSettings::default()).0))
    }
}

impl DiscordSwitches {
    pub fn set(&self, s: DiscordSettings) {
        self.0.send_if_modified(|cur| std::mem::replace(cur, s) != s);
    }
    pub fn get(&self) -> DiscordSettings {
        *self.0.borrow()
    }
    pub fn subscribe(&self) -> watch::Receiver<DiscordSettings> {
        self.0.subscribe()
    }
}

/// `discord_status` and the `discord://status` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordStatus {
    /// This build has a Discord application id.
    pub available: bool,
    /// Talking to the Discord client right now.
    pub connected: bool,
    pub imgur_key: KeyState,
    /// The last cover upload failed (Discord shows the app icon).
    pub upload_failed: bool,
}

/// Where status changes go (the UI event in the app).
pub trait StatusSink: Send + Sync {
    fn changed(&self, status: &DiscordStatus);
}

/// What the service asks the library.
#[async_trait]
pub trait Library: Send + Sync {
    /// Whether the album counts as unreleased (`db::release::is_unreleased`).
    async fn unreleased(&self, album_id: &str) -> bool;
    /// The song.link page for a released song, if a sure match exists.
    async fn song_link(&self, track_id: &str) -> Option<String>;
}

/// Public cover art (the real one is [`ArtResolver`]).
#[async_trait]
pub trait CoverArt: Send + Sync {
    async fn resolve(&self, req: &ArtRequest) -> ArtOutcome;
    fn key_state(&self) -> KeyState;
}

#[async_trait]
impl CoverArt for ArtResolver {
    async fn resolve(&self, req: &ArtRequest) -> ArtOutcome {
        ArtResolver::resolve(self, req).await
    }
    fn key_state(&self) -> KeyState {
        ArtResolver::key_state(self)
    }
}

/// Wall-clock unix ms (Discord's timestamps are wall-clock).
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

#[derive(Clone)]
pub struct Config {
    pub app_id: String,
    pub clock: Clock,
    /// The least time between two updates (Discord allows about one per 15 s).
    pub gap: Duration,
    /// Changes are gathered this long before an update (a track ending and
    /// the next starting is one change).
    pub settle: Duration,
    /// How often to look for Discord while it isn't running.
    pub retry: Duration,
    /// How long a new song waits for its cover before showing without it.
    pub art_wait: Duration,
    /// Discord must answer within this.
    pub reply_timeout: Duration,
}

impl Config {
    pub fn new(app_id: &str) -> Self {
        Self {
            app_id: app_id.to_owned(),
            clock: Arc::new(now_ms),
            gap: Duration::from_secs(15),
            settle: Duration::from_millis(300),
            retry: Duration::from_secs(15),
            art_wait: Duration::from_millis(1500),
            reply_timeout: Duration::from_secs(5),
        }
    }
}

pub struct Deps {
    pub connector: Arc<dyn Connector>,
    pub art: Arc<dyn CoverArt>,
    pub library: Arc<dyn Library>,
    pub status: Arc<dyn StatusSink>,
}

/// Managed state for the commands.
#[derive(Clone)]
pub struct DiscordHandle {
    available: bool,
    live: Arc<Mutex<DiscordStatus>>,
    art: Arc<dyn CoverArt>,
    sink: Arc<dyn StatusSink>,
}

impl DiscordHandle {
    pub fn status(&self) -> DiscordStatus {
        DiscordStatus { available: self.available, imgur_key: self.art.key_state(), ..*self.live.lock() }
    }

    /// After the client id changed: a new key clears the upload failure.
    pub fn key_changed(&self) {
        self.live.lock().upload_failed = false;
        self.sink.changed(&self.status());
    }
}

/// The handle for a build without an app id: nothing runs.
pub fn unavailable(art: Arc<dyn CoverArt>, sink: Arc<dyn StatusSink>) -> DiscordHandle {
    DiscordHandle { available: false, live: Arc::default(), art, sink }
}

/// Start following the player.
pub fn spawn(
    cfg: Config,
    deps: Deps,
    events: broadcast::Receiver<TrackingEvent>,
    settings: watch::Receiver<DiscordSettings>,
) -> (DiscordHandle, tokio::task::JoinHandle<()>) {
    let handle = DiscordHandle {
        available: true,
        live: Arc::default(),
        art: deps.art.clone(),
        sink: deps.status.clone(),
    };
    let (side_tx, side_rx) = mpsc::unbounded_channel();
    let runner = Runner {
        settings: *settings.borrow(),
        cfg,
        deps,
        handle: handle.clone(),
        side_tx,
        cur: None,
        conn: None,
        kind: Kind::Listening,
        with_line: true,
        last_sent: None,
        last_send_at: None,
        reconnect_at: None,
        dirty: false,
        not_before: Instant::now(),
    };
    let task = tokio::spawn(runner.run(events, settings, side_rx));
    (handle, task)
}

/// Answers from the lookups started for a song.
enum Side {
    Art { track_id: String, outcome: ArtOutcome },
    Link { track_id: String, url: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lookup {
    NotAsked,
    Pending,
    Done,
}

struct Current {
    track_id: String,
    album_id: String,
    /// The album artist (for the cover search).
    album_artist: String,
    thumb: Option<String>,
    song: Song,
    /// Position while paused.
    paused_at_ms: u64,
    started: Instant,
    art: Lookup,
    link: Lookup,
}

/// A position this far from where it should be is a seek or a stall.
const DRIFT_MS: i64 = 3_000;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

struct Runner {
    cfg: Config,
    deps: Deps,
    handle: DiscordHandle,
    settings: DiscordSettings,
    side_tx: mpsc::UnboundedSender<Side>,
    cur: Option<Current>,
    conn: Option<IpcClient>,
    kind: Kind,
    /// Send `status_display_type` (off once Discord refuses it).
    with_line: bool,
    /// What Discord shows now (None: nothing, or a fresh connection).
    last_sent: Option<Value>,
    last_send_at: Option<Instant>,
    reconnect_at: Option<Instant>,
    dirty: bool,
    not_before: Instant,
}

impl Runner {
    async fn run(
        mut self,
        mut events: broadcast::Receiver<TrackingEvent>,
        mut settings: watch::Receiver<DiscordSettings>,
        mut side: mpsc::UnboundedReceiver<Side>,
    ) {
        loop {
            let wake = self.dirty.then_some(self.not_before);
            tokio::select! {
                ev = events.recv() => match ev {
                    Ok(e) => self.on_event(e).await,
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                r = settings.changed() => {
                    if r.is_err() {
                        break;
                    }
                    let s = *settings.borrow_and_update();
                    self.on_settings(s);
                }
                Some(m) = side.recv() => self.on_side(m),
                _ = tokio::time::sleep_until(wake.unwrap_or_else(Instant::now)), if wake.is_some() => self.flush().await,
            }
        }
    }

    fn mark(&mut self) {
        let at = Instant::now() + self.cfg.settle;
        if !self.dirty || self.not_before < at {
            self.not_before = at;
        }
        self.dirty = true;
    }

    fn set_live(&self, f: impl FnOnce(&mut DiscordStatus)) {
        let changed = {
            let mut l = self.handle.live.lock();
            let before = *l;
            f(&mut l);
            *l != before
        };
        if changed {
            self.deps.status.changed(&self.handle.status());
        }
    }

    async fn on_event(&mut self, e: TrackingEvent) {
        let now = (self.cfg.clock)();
        match e {
            TrackingEvent::TrackStarted { track, at, .. } => {
                let unreleased = self.deps.library.unreleased(&track.album_id).await;
                let artist = track
                    .track_artist
                    .clone()
                    .filter(|a| !a.trim().is_empty())
                    .unwrap_or_else(|| track.artist_title.clone());
                self.cur = Some(Current {
                    track_id: track.id.clone(),
                    album_id: track.album_id.clone(),
                    album_artist: track.artist_title.clone(),
                    thumb: track.thumb.clone(),
                    song: Song {
                        title: track.title.clone(),
                        artist,
                        album: track.album_title.clone(),
                        duration_ms: track.duration_ms,
                        unreleased,
                        playing: true,
                        started_at_ms: if at > 0 { at } else { now },
                        image: None,
                        link: None,
                    },
                    paused_at_ms: 0,
                    started: Instant::now(),
                    art: Lookup::NotAsked,
                    link: Lookup::NotAsked,
                });
                self.lookups();
                self.mark();
            }
            TrackingEvent::Progress { track_id, position_ms, duration_ms } => {
                let Some(c) = self.cur.as_mut().filter(|c| c.track_id == track_id) else { return };
                if c.song.duration_ms == 0 && duration_ms > 0 {
                    c.song.duration_ms = duration_ms;
                    self.mark();
                    return;
                }
                if c.song.playing && (now - position_ms as i64 - c.song.started_at_ms).abs() > DRIFT_MS {
                    c.song.started_at_ms = now - position_ms as i64;
                    self.mark();
                }
            }
            TrackingEvent::Seeked { track_id, to_ms, .. } => {
                let Some(c) = self.cur.as_mut().filter(|c| c.track_id == track_id) else { return };
                c.song.started_at_ms = now - to_ms as i64;
                c.paused_at_ms = to_ms;
                self.mark();
            }
            TrackingEvent::Paused { track_id, position_ms } => {
                let Some(c) = self.cur.as_mut().filter(|c| c.track_id == track_id) else { return };
                c.song.playing = false;
                c.paused_at_ms = position_ms;
                self.mark();
            }
            TrackingEvent::Resumed { track_id, position_ms } => {
                let Some(c) = self.cur.as_mut().filter(|c| c.track_id == track_id) else { return };
                c.song.playing = true;
                c.song.started_at_ms = now - position_ms as i64;
                self.mark();
            }
            TrackingEvent::TrackEnded { track, .. } => {
                if self.cur.as_ref().is_some_and(|c| c.track_id == track.id) {
                    self.cur = None;
                    self.mark();
                }
            }
        }
    }

    fn on_settings(&mut self, s: DiscordSettings) {
        if s == self.settings {
            return;
        }
        // A cover that may now be shown differently is looked up again.
        if let Some(c) = self.cur.as_mut()
            && (s.cover_archive != self.settings.cover_archive
                || s.upload_art != self.settings.upload_art
                || s.upload_catbox != self.settings.upload_catbox)
            && c.art == Lookup::Done
        {
            c.art = Lookup::NotAsked;
            c.song.image = None;
        }
        self.settings = s;
        self.lookups();
        self.mark();
    }

    fn on_side(&mut self, m: Side) {
        match m {
            Side::Art { track_id, outcome } => {
                self.set_live(|l| match &outcome {
                    ArtOutcome::UploadFailed(_) => l.upload_failed = true,
                    ArtOutcome::Url(_) => l.upload_failed = false,
                    ArtOutcome::None => {}
                });
                let Some(c) = self.cur.as_mut().filter(|c| c.track_id == track_id) else { return };
                c.art = Lookup::Done;
                c.song.image = match outcome {
                    ArtOutcome::Url(u) => Some(u),
                    _ => None,
                };
            }
            Side::Link { track_id, url } => {
                let Some(c) = self.cur.as_mut().filter(|c| c.track_id == track_id) else { return };
                c.link = Lookup::Done;
                c.song.link = url;
            }
        }
        self.mark();
    }

    /// Start the cover and link lookups the current song needs, only while
    /// Discord is connected and the song may be shown.
    fn lookups(&mut self) {
        let s = self.settings;
        if self.conn.is_none() || !s.enabled || s.private_session {
            return;
        }
        let Some(c) = self.cur.as_mut() else { return };
        if c.song.unreleased && !s.show_unreleased {
            return;
        }
        let art_wanted = (s.cover_archive || s.upload_art || s.upload_catbox) && (!c.song.unreleased || s.unreleased_art);
        if c.art == Lookup::NotAsked && art_wanted {
            c.art = Lookup::Pending;
            let req = ArtRequest {
                album_id: c.album_id.clone(),
                thumb: c.thumb.clone(),
                artist: c.album_artist.clone(),
                album: c.song.album.clone(),
                unreleased: c.song.unreleased,
                use_archive: s.cover_archive,
                upload: s.upload_art,
                catbox: s.upload_catbox,
            };
            let (art, tx, track_id) = (self.deps.art.clone(), self.side_tx.clone(), c.track_id.clone());
            tokio::spawn(async move {
                let outcome = art.resolve(&req).await;
                let _ = tx.send(Side::Art { track_id, outcome });
            });
        }
        if c.link == Lookup::NotAsked && s.find_song && !c.song.unreleased {
            c.link = Lookup::Pending;
            let (lib, tx, track_id) = (self.deps.library.clone(), self.side_tx.clone(), c.track_id.clone());
            tokio::spawn(async move {
                let url = lib.song_link(&track_id).await;
                let _ = tx.send(Side::Link { track_id, url });
            });
        }
    }

    fn desired(&self) -> Option<Value> {
        let song = self.cur.as_ref().map(|c| &c.song);
        activity::build(song, &self.settings, self.kind, self.with_line)
    }

    fn drop_conn(&mut self, now: Instant, retry_in: Duration) {
        self.conn = None;
        self.last_sent = None;
        self.last_send_at = None;
        self.reconnect_at = Some(now + retry_in);
        self.set_live(|l| l.connected = false);
    }

    async fn flush(&mut self) {
        let now = Instant::now();
        // A new song waits a moment for its cover.
        if let Some(c) = &self.cur
            && c.art == Lookup::Pending
            && now < c.started + self.cfg.art_wait
        {
            self.not_before = c.started + self.cfg.art_wait;
            return;
        }
        let want = self.desired();

        if self.conn.is_none() {
            if want.is_none() {
                self.dirty = false;
                return;
            }
            if let Some(at) = self.reconnect_at
                && now < at
            {
                self.not_before = at;
                return;
            }
            let open = IpcClient::open(self.deps.connector.as_ref(), &self.cfg.app_id);
            match tokio::time::timeout(self.cfg.reply_timeout, open).await {
                Ok(Ok(c)) => {
                    tracing::info!("connected to Discord");
                    self.conn = Some(c);
                    self.reconnect_at = None;
                    self.set_live(|l| l.connected = true);
                    // Lookups wait for a connection; give the cover its moment.
                    let had_art = self.cur.as_ref().is_some_and(|c| c.art != Lookup::NotAsked);
                    self.lookups();
                    if !had_art && let Some(c) = self.cur.as_mut().filter(|c| c.art == Lookup::Pending) {
                        c.started = now;
                        self.not_before = now + self.cfg.art_wait;
                        return;
                    }
                }
                Ok(Err(e)) => {
                    tracing::debug!("Discord isn't available: {e}");
                    self.drop_conn(now, self.cfg.retry);
                    self.not_before = now + self.cfg.retry;
                    return;
                }
                Err(_) => {
                    tracing::debug!("Discord didn't answer the handshake");
                    self.drop_conn(now, self.cfg.retry);
                    self.not_before = now + self.cfg.retry;
                    return;
                }
            }
        }

        if want == self.last_sent {
            self.dirty = false;
            self.disconnect_if_off();
            return;
        }
        if let Some(t) = self.last_send_at
            && now < t + self.cfg.gap
        {
            self.not_before = t + self.cfg.gap;
            return;
        }
        let Some(conn) = self.conn.as_mut() else { return };
        let sent = tokio::time::timeout(
            self.cfg.reply_timeout,
            conn.set_activity(std::process::id(), want.as_ref()),
        )
        .await;
        match sent {
            Ok(Ok(Reply::Ok)) => {
                self.last_sent = want;
                self.last_send_at = Some(now);
                self.dirty = false;
                self.disconnect_if_off();
            }
            Ok(Ok(Reply::Refused { code, message }))
                if self.with_line && want.as_ref().is_some_and(|w| w.get("status_display_type").is_some()) =>
            {
                tracing::info!("Discord refused the status line choice ({code} {message}); leaving it out");
                self.with_line = false;
                self.not_before = now;
            }
            Ok(Ok(Reply::Refused { code, message })) if self.kind == Kind::Listening && want.is_some() => {
                tracing::info!("Discord refused a Listening status ({code} {message}); using Playing");
                self.kind = Kind::Playing;
                self.not_before = now;
            }
            Ok(Ok(Reply::Refused { code, message })) => {
                tracing::warn!("Discord refused the status: {code} {message}");
                self.last_sent = want;
                self.last_send_at = Some(now);
                self.dirty = false;
            }
            Ok(Err(e)) => {
                tracing::info!("lost Discord: {e}");
                // Try again at once (Discord may have restarted); if it's
                // gone, the retry delay applies after that.
                self.drop_conn(now, Duration::ZERO);
                self.not_before = now;
            }
            Err(_) => {
                tracing::info!("Discord stopped answering");
                self.drop_conn(now, self.cfg.retry);
                self.not_before = now + self.cfg.retry;
            }
        }
    }

    /// Turning the status off leaves Discord entirely (after clearing).
    fn disconnect_if_off(&mut self) {
        if !self.settings.enabled && self.last_sent.is_none() && self.conn.is_some() {
            self.conn = None;
            self.last_send_at = None;
            self.set_live(|l| l.connected = false);
        }
    }
}
