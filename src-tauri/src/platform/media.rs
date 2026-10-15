//! The system now-playing session: Windows media transport controls and
//! macOS Now Playing.
//!
//! [`Controller`] turns player state into session updates and owns the
//! session's lifetime (created when the setting is on, dropped when it's
//! off). The OS side sits behind [`MediaSession`] so tests use a fake; the
//! real one is in `session.rs`. If the OS refuses to give us a session we log
//! it and carry on without one: the app works the same, just without the
//! system controls.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::audio::events::{PlayerState, PlayerStatus, PositionPayload};

/// What the OS shows for the current track.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    /// A local file from the art cache. Never a server URL.
    pub cover: Option<PathBuf>,
}

/// Playback state for the session, with the position in ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Playback {
    Stopped,
    Paused(u64),
    Playing(u64),
}

/// Something the OS (session buttons, media keys, tray menu) asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsCommand {
    Toggle,
    Play,
    Pause,
    Next,
    Previous,
    /// Seek to an absolute position in ms.
    SeekTo(u64),
    /// Seek forward (positive) or back by ms.
    SeekBy(i64),
    /// Bring the window to the front.
    Show,
    Quit,
}

/// How far the OS's fast-forward and rewind buttons jump.
pub const SEEK_STEP_MS: i64 = 10_000;

/// A live OS session. Dropping it detaches from the OS.
pub trait MediaSession: Send {
    fn set_metadata(&mut self, now: &NowPlaying) -> Result<(), String>;
    fn set_playback(&mut self, playback: Playback) -> Result<(), String>;
}

/// Makes a session wired to send OS commands to `on_command`.
pub type CommandFn = Box<dyn Fn(OsCommand) + Send + Sync>;
pub trait SessionFactory: Send {
    fn create(&mut self, on_command: CommandFn) -> Result<Box<dyn MediaSession>, String>;
}

impl<F> SessionFactory for F
where
    F: FnMut(CommandFn) -> Result<Box<dyn MediaSession>, String> + Send,
{
    fn create(&mut self, on_command: CommandFn) -> Result<Box<dyn MediaSession>, String> {
        self(on_command)
    }
}

/// The OS's idea of where the track is drifts from ours only on seeks and
/// stalls; refresh it at least this often while playing.
const POSITION_REFRESH: Duration = Duration::from_secs(5);
/// A position this far from where we expect it is a seek: push it at once.
const SEEK_JUMP_MS: u64 = 1_500;

/// The text and duration of a track, as the OS will show it.
pub fn now_playing_for(state: &PlayerState) -> Option<NowPlaying> {
    let t = state.track.as_ref()?;
    Some(NowPlaying {
        title: t.title.clone(),
        artist: t.track_artist.clone().unwrap_or_else(|| t.artist_title.clone()),
        album: t.album_title.clone(),
        duration_ms: if state.duration_ms > 0 { state.duration_ms } else { t.duration_ms },
        cover: None,
    })
}

fn playback_for(status: PlayerStatus, has_track: bool, pos: u64) -> Playback {
    match status {
        _ if !has_track => Playback::Stopped,
        PlayerStatus::Playing | PlayerStatus::Loading | PlayerStatus::Buffering => Playback::Playing(pos),
        PlayerStatus::Paused => Playback::Paused(pos),
        PlayerStatus::Idle | PlayerStatus::Error => Playback::Stopped,
    }
}

/// Drives one session from player events and settings.
pub struct Controller {
    factory: Box<dyn SessionFactory>,
    on_command: std::sync::Arc<dyn Fn(OsCommand) + Send + Sync>,
    session: Option<Box<dyn MediaSession>>,
    enabled: bool,
    track_id: Option<String>,
    thumb: Option<String>,
    now: Option<NowPlaying>,
    playback: Playback,
    /// The last position sent and when.
    pushed: Option<(Instant, u64)>,
}

impl Controller {
    pub fn new(factory: Box<dyn SessionFactory>, on_command: std::sync::Arc<dyn Fn(OsCommand) + Send + Sync>) -> Self {
        Self {
            factory,
            on_command,
            session: None,
            enabled: false,
            track_id: None,
            thumb: None,
            now: None,
            playback: Playback::Stopped,
            pushed: None,
        }
    }

    pub fn is_attached(&self) -> bool {
        self.session.is_some()
    }

    /// Thumb path of the current track (the art cache key), if any.
    pub fn thumb(&self) -> Option<&str> {
        self.thumb.as_deref()
    }

    pub fn track_id(&self) -> Option<&str> {
        self.track_id.as_deref()
    }

    /// "Show in the system media controls" changed (or startup).
    pub fn set_enabled(&mut self, on: bool) {
        if on == self.enabled {
            return;
        }
        self.enabled = on;
        if !on {
            if self.session.take().is_some() {
                tracing::debug!("media session detached");
            }
            return;
        }
        let cb = self.on_command.clone();
        match self.factory.create(Box::new(move |c| cb(c))) {
            Ok(s) => {
                self.session = Some(s);
                tracing::debug!("media session attached");
                self.push_all(Instant::now());
            }
            // The app works without it; the user sees nothing.
            Err(e) => tracing::warn!("the OS refused the media session, carrying on without it: {e}"),
        }
    }

    /// A `player://state` event.
    pub fn on_state(&mut self, state: &PlayerState, at: Instant) {
        let id = state.track.as_ref().map(|t| t.id.clone());
        let playback = playback_for(state.status, id.is_some(), state.position_ms);
        if id != self.track_id {
            self.track_id = id;
            self.thumb = state.track.as_ref().and_then(|t| t.thumb.clone());
            self.now = now_playing_for(state);
            self.playback = playback;
            self.push_all(at);
            return;
        }
        // Same track: the duration can arrive late (on first decode).
        if let (Some(now), Some(fresh)) = (self.now.as_mut(), now_playing_for(state))
            && now.duration_ms != fresh.duration_ms
        {
            now.duration_ms = fresh.duration_ms;
            let now = now.clone();
            self.with_session(|s| s.set_metadata(&now));
        }
        if std::mem::discriminant(&playback) != std::mem::discriminant(&self.playback) {
            self.playback = playback;
            self.push_playback(at);
        } else {
            self.playback = playback;
        }
    }

    /// A `player://position` event.
    pub fn on_position(&mut self, pos: &PositionPayload, at: Instant) {
        let p = pos.position_ms;
        self.playback = match self.playback {
            Playback::Playing(_) => Playback::Playing(p),
            Playback::Paused(_) => Playback::Paused(p),
            Playback::Stopped => return,
        };
        let due = match (self.pushed, self.playback) {
            (None, _) => true,
            (Some((when, sent)), Playback::Playing(_)) => {
                let expected = sent + at.saturating_duration_since(when).as_millis() as u64;
                at.saturating_duration_since(when) >= POSITION_REFRESH || p.abs_diff(expected) > SEEK_JUMP_MS
            }
            (Some((_, sent)), _) => p.abs_diff(sent) > SEEK_JUMP_MS,
        };
        if due {
            self.push_playback(at);
        }
    }

    /// The cover for `track_id` is ready as a local file.
    pub fn on_cover(&mut self, track_id: &str, cover: PathBuf) {
        if self.track_id.as_deref() != Some(track_id) {
            return;
        }
        if let Some(now) = self.now.as_mut() {
            now.cover = Some(cover);
            let now = now.clone();
            self.with_session(|s| s.set_metadata(&now));
        }
    }

    fn push_all(&mut self, at: Instant) {
        let now = self.now.clone().unwrap_or_default();
        self.with_session(|s| s.set_metadata(&now));
        self.push_playback(at);
    }

    fn push_playback(&mut self, at: Instant) {
        let pb = self.playback;
        self.pushed = match pb {
            Playback::Playing(p) | Playback::Paused(p) => Some((at, p)),
            Playback::Stopped => None,
        };
        self.with_session(|s| s.set_playback(pb));
    }

    fn with_session(&mut self, f: impl FnOnce(&mut dyn MediaSession) -> Result<(), String>) {
        if let Some(s) = self.session.as_deref_mut()
            && let Err(e) = f(s)
        {
            tracing::debug!("media session update failed: {e}");
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::audio::queue::{QueueView, Repeat};
    use crate::model::Track;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Log {
        meta: Mutex<Vec<NowPlaying>>,
        playback: Mutex<Vec<Playback>>,
        dropped: AtomicBool,
        created: AtomicUsize,
        command: Mutex<Option<CommandFn>>,
    }

    struct Fake(Arc<Log>);
    impl MediaSession for Fake {
        fn set_metadata(&mut self, now: &NowPlaying) -> Result<(), String> {
            self.0.meta.lock().push(now.clone());
            Ok(())
        }
        fn set_playback(&mut self, p: Playback) -> Result<(), String> {
            self.0.playback.lock().push(p);
            Ok(())
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            self.0.dropped.store(true, Ordering::SeqCst);
        }
    }

    fn controller(log: &Arc<Log>, refuse: bool) -> (Controller, Arc<Mutex<Vec<OsCommand>>>) {
        let l = log.clone();
        let factory = move |cb: CommandFn| -> Result<Box<dyn MediaSession>, String> {
            if refuse {
                return Err("access denied".into());
            }
            l.created.fetch_add(1, Ordering::SeqCst);
            l.dropped.store(false, Ordering::SeqCst);
            *l.command.lock() = Some(cb);
            Ok(Box::new(Fake(l.clone())))
        };
        let got: Arc<Mutex<Vec<OsCommand>>> = Arc::default();
        let g = got.clone();
        (Controller::new(Box::new(factory), Arc::new(move |c| g.lock().push(c))), got)
    }

    pub(crate) fn track(id: &str) -> Track {
        Track { mbid: None,
            id: id.into(),
            title: format!("Song {id}"),
            album_id: "al".into(),
            album_title: "Album".into(),
            artist_id: "ar".into(),
            artist_title: "Album Artist".into(),
            track_artist: None,
            track_number: 1,
            disc_number: 1,
            duration_ms: 200_000,
            thumb: Some(format!("/library/metadata/{id}/thumb/1")),
            part_key: "/p".into(),
            codec: None,
            container: None,
            bitrate: None,
            bit_depth: None,
            sample_rate: None,
            user_rating: None,
            view_count: 0,
            last_viewed_at: None,
            rating_count: None,
            file: None,
            other_versions: None,
            song_status: None,
            rated_at: None,
            genres: Vec::new(),
        }
    }

    pub(crate) fn state(track: Option<Track>, status: PlayerStatus, pos: u64) -> PlayerState {
        PlayerState {
            status,
            duration_ms: track.as_ref().map_or(0, |t| t.duration_ms),
            track,
            position_ms: pos,
            volume: 1.0,
            muted: false,
            shuffle: false,
            shuffled_contexts: vec![],
            repeat: Repeat::Off,
            queue: QueueView::default(),
            ab_loop: None,
            smart_shuffle: false,
            smart_contexts: vec![],
            smooth_transitions: false,
        }
    }

    fn pos(ms: u64) -> PositionPayload {
        PositionPayload { position_ms: ms, duration_ms: 200_000, buffered_ms: 0 }
    }

    #[test]
    fn metadata_is_pushed_on_track_change_pause_and_seek() {
        let log = Arc::new(Log::default());
        let (mut c, _) = controller(&log, false);
        c.set_enabled(true);
        let t0 = Instant::now();
        let mut t = track("1");
        t.track_artist = Some("Guest".into());
        c.on_state(&state(Some(t), PlayerStatus::Playing, 0), t0);
        let meta = log.meta.lock().last().cloned().unwrap();
        assert_eq!(
            (meta.title.as_str(), meta.artist.as_str(), meta.album.as_str(), meta.duration_ms),
            ("Song 1", "Guest", "Album", 200_000)
        );
        assert_eq!(log.playback.lock().last(), Some(&Playback::Playing(0)));

        // Ordinary ticks don't spam the OS.
        let n = log.playback.lock().len();
        c.on_position(&pos(1_000), t0 + Duration::from_secs(1));
        c.on_position(&pos(2_000), t0 + Duration::from_secs(2));
        assert_eq!(log.playback.lock().len(), n);
        // A seek goes out at once.
        c.on_position(&pos(90_000), t0 + Duration::from_secs(3));
        assert_eq!(log.playback.lock().last(), Some(&Playback::Playing(90_000)));
        // And a periodic refresh while playing.
        let n = log.playback.lock().len();
        c.on_position(&pos(98_000), t0 + Duration::from_secs(11));
        assert_eq!(log.playback.lock().len(), n + 1);

        // Pause.
        c.on_state(&state(Some(track("1")), PlayerStatus::Paused, 98_500), t0 + Duration::from_secs(12));
        assert_eq!(log.playback.lock().last(), Some(&Playback::Paused(98_500)));

        // Next track: new metadata.
        c.on_state(&state(Some(track("2")), PlayerStatus::Playing, 0), t0 + Duration::from_secs(13));
        let meta = log.meta.lock().last().cloned().unwrap();
        assert_eq!((meta.title.as_str(), meta.artist.as_str()), ("Song 2", "Album Artist"));
        assert_eq!(c.thumb(), Some("/library/metadata/2/thumb/1"));

        // Queue ends.
        c.on_state(&state(None, PlayerStatus::Idle, 0), t0 + Duration::from_secs(14));
        assert_eq!(log.playback.lock().last(), Some(&Playback::Stopped));
    }

    #[test]
    fn cover_is_added_only_for_the_current_track() {
        let log = Arc::new(Log::default());
        let (mut c, _) = controller(&log, false);
        c.set_enabled(true);
        c.on_state(&state(Some(track("1")), PlayerStatus::Playing, 0), Instant::now());
        c.on_cover("old", PathBuf::from("x.jpg"));
        assert!(log.meta.lock().iter().all(|m| m.cover.is_none()));
        c.on_cover("1", PathBuf::from("cover.jpg"));
        assert_eq!(log.meta.lock().last().unwrap().cover, Some(PathBuf::from("cover.jpg")));
    }

    #[test]
    fn os_commands_reach_the_handler() {
        let log = Arc::new(Log::default());
        let (mut c, got) = controller(&log, false);
        c.set_enabled(true);
        let cb = log.command.lock().take().unwrap();
        cb(OsCommand::Next);
        cb(OsCommand::SeekTo(5_000));
        assert_eq!(*got.lock(), vec![OsCommand::Next, OsCommand::SeekTo(5_000)]);
    }

    #[test]
    fn turning_the_setting_off_detaches_and_on_reattaches_with_current_track() {
        let log = Arc::new(Log::default());
        let (mut c, _) = controller(&log, false);
        assert!(!c.is_attached(), "nothing until the setting says so");
        c.set_enabled(true);
        c.on_state(&state(Some(track("1")), PlayerStatus::Paused, 4_000), Instant::now());
        c.set_enabled(false);
        assert!(!c.is_attached());
        assert!(log.dropped.load(Ordering::SeqCst));
        // Updates while off go nowhere.
        let n = log.meta.lock().len();
        c.on_state(&state(Some(track("2")), PlayerStatus::Playing, 0), Instant::now());
        assert_eq!(log.meta.lock().len(), n);
        c.set_enabled(true);
        assert_eq!(log.created.load(Ordering::SeqCst), 2);
        assert_eq!(log.meta.lock().last().unwrap().title, "Song 2");
        assert_eq!(log.playback.lock().last(), Some(&Playback::Playing(0)));
    }

    #[test]
    fn os_refusing_the_session_is_logged_and_playback_carries_on() {
        let log = Arc::new(Log::default());
        let (mut c, _) = controller(&log, true);
        c.set_enabled(true);
        assert!(!c.is_attached());
        // Player updates keep flowing without a session and without panics.
        c.on_state(&state(Some(track("1")), PlayerStatus::Playing, 0), Instant::now());
        c.on_position(&pos(50_000), Instant::now());
        c.on_cover("1", PathBuf::from("c.jpg"));
        assert_eq!(c.track_id(), Some("1"));
        assert!(log.meta.lock().is_empty());
    }
}
