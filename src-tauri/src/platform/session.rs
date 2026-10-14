//! The real OS session through souvlaki: the system media transport controls
//! on Windows (needs the window handle) and Now Playing plus the remote
//! command center on macOS.

use std::time::Duration;

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};

use super::art::cover_uri;
use super::media::{CommandFn, MediaSession, NowPlaying, OsCommand, Playback, SEEK_STEP_MS};

/// What an OS event asks the player to do.
pub fn map_event(e: MediaControlEvent) -> Option<OsCommand> {
    Some(match e {
        MediaControlEvent::Play => OsCommand::Play,
        MediaControlEvent::Pause | MediaControlEvent::Stop => OsCommand::Pause,
        MediaControlEvent::Toggle => OsCommand::Toggle,
        MediaControlEvent::Next => OsCommand::Next,
        MediaControlEvent::Previous => OsCommand::Previous,
        MediaControlEvent::Seek(SeekDirection::Forward) => OsCommand::SeekBy(SEEK_STEP_MS),
        MediaControlEvent::Seek(SeekDirection::Backward) => OsCommand::SeekBy(-SEEK_STEP_MS),
        MediaControlEvent::SeekBy(dir, d) => {
            let ms = d.as_millis().min(i64::MAX as u128) as i64;
            OsCommand::SeekBy(if dir == SeekDirection::Forward { ms } else { -ms })
        }
        MediaControlEvent::SetPosition(MediaPosition(d)) => OsCommand::SeekTo(d.as_millis() as u64),
        MediaControlEvent::Raise => OsCommand::Show,
        MediaControlEvent::Quit => OsCommand::Quit,
        MediaControlEvent::SetVolume(_) | MediaControlEvent::OpenUri(_) => return None,
    })
}

pub struct OsSession(MediaControls);

impl OsSession {
    /// Create and attach. `hwnd` is the main window's handle on Windows
    /// (ignored elsewhere).
    pub fn create(hwnd: Option<isize>, on_command: CommandFn) -> Result<Self, String> {
        let config = PlatformConfig {
            display_name: "Platter",
            dbus_name: "app.platter.player",
            hwnd: hwnd.map(|h| h as *mut std::ffi::c_void),
        };
        if cfg!(target_os = "windows") && hwnd.is_none() {
            return Err("no window handle".into());
        }
        let mut controls = MediaControls::new(config).map_err(|e| format!("{e:?}"))?;
        controls
            .attach(move |e| {
                if let Some(c) = map_event(e) {
                    on_command(c)
                }
            })
            .map_err(|e| format!("{e:?}"))?;
        Ok(Self(controls))
    }
}

impl MediaSession for OsSession {
    fn set_metadata(&mut self, now: &NowPlaying) -> Result<(), String> {
        let cover = now.cover.as_deref().map(cover_uri);
        let meta = |cover_url| MediaMetadata {
            title: Some(&now.title),
            album: Some(&now.album),
            artist: Some(&now.artist),
            cover_url,
            duration: (now.duration_ms > 0).then(|| Duration::from_millis(now.duration_ms)),
        };
        match self.0.set_metadata(meta(cover.as_deref())) {
            // A cover the OS can't read shouldn't cost the title and artist.
            Err(e) if cover.is_some() => {
                tracing::debug!("media session cover refused: {e:?}");
                self.0.set_metadata(meta(None)).map_err(|e| format!("{e:?}"))
            }
            r => r.map_err(|e| format!("{e:?}")),
        }
    }

    fn set_playback(&mut self, playback: Playback) -> Result<(), String> {
        let at = |ms| Some(MediaPosition(Duration::from_millis(ms)));
        self.0
            .set_playback(match playback {
                Playback::Stopped => MediaPlayback::Stopped,
                Playback::Paused(ms) => MediaPlayback::Paused { progress: at(ms) },
                Playback::Playing(ms) => MediaPlayback::Playing { progress: at(ms) },
            })
            .map_err(|e| format!("{e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_events_map_to_commands() {
        assert_eq!(map_event(MediaControlEvent::Toggle), Some(OsCommand::Toggle));
        assert_eq!(map_event(MediaControlEvent::Play), Some(OsCommand::Play));
        assert_eq!(map_event(MediaControlEvent::Pause), Some(OsCommand::Pause));
        assert_eq!(map_event(MediaControlEvent::Stop), Some(OsCommand::Pause));
        assert_eq!(map_event(MediaControlEvent::Next), Some(OsCommand::Next));
        assert_eq!(map_event(MediaControlEvent::Previous), Some(OsCommand::Previous));
        assert_eq!(
            map_event(MediaControlEvent::SetPosition(MediaPosition(Duration::from_secs(61)))),
            Some(OsCommand::SeekTo(61_000))
        );
        assert_eq!(map_event(MediaControlEvent::Seek(SeekDirection::Backward)), Some(OsCommand::SeekBy(-SEEK_STEP_MS)));
        assert_eq!(
            map_event(MediaControlEvent::SeekBy(SeekDirection::Forward, Duration::from_secs(5))),
            Some(OsCommand::SeekBy(5_000))
        );
        assert_eq!(map_event(MediaControlEvent::Raise), Some(OsCommand::Show));
        assert_eq!(map_event(MediaControlEvent::SetVolume(0.5)), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_without_a_window_handle_is_refused_not_a_panic() {
        // souvlaki would panic on a missing handle; we refuse first.
        assert!(OsSession::create(None, Box::new(|_| {})).is_err());
    }
}
