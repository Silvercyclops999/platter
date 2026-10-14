//! Hardware media keys while Platter is in the background.
//!
//! When Platter is in the system media controls, the OS sends the keys to
//! that session, so nothing else is needed. With the system controls off (or
//! refused by the OS), the keys are registered as global shortcuts instead.

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};

use super::PlatformSettings;
use super::media::OsCommand;

const KEYS: [(Code, OsCommand); 4] = [
    (Code::MediaPlayPause, OsCommand::Toggle),
    (Code::MediaTrackNext, OsCommand::Next),
    (Code::MediaTrackPrevious, OsCommand::Previous),
    (Code::MediaStop, OsCommand::Pause),
];

/// Whether Platter registers the media keys itself, given whether the
/// system media session is attached.
pub fn wants_shortcuts(s: &PlatformSettings, session_attached: bool) -> bool {
    s.media_keys && !(s.media_session && session_attached)
}

/// The command for a pressed media key.
pub fn key_command(code: Code) -> Option<OsCommand> {
    KEYS.iter().find(|(k, _)| *k == code).map(|(_, c)| *c)
}

/// Register or unregister the media keys.
pub fn apply<R: Runtime>(app: &AppHandle<R>, on: bool, on_command: impl Fn(OsCommand) + Send + Sync + Clone + 'static) {
    let gs = app.global_shortcut();
    for (code, _) in KEYS {
        let sc = Shortcut::new(None, code);
        let registered = gs.is_registered(sc);
        if on && !registered {
            let cb = on_command.clone();
            let r = gs.on_shortcut(sc, move |_, s, e| {
                if e.state == ShortcutState::Pressed
                    && let Some(c) = key_command(s.key)
                {
                    cb(c)
                }
            });
            if let Err(e) = r {
                // Another app may own the key; Platter carries on.
                tracing::warn!("media key {code:?} not available: {e}");
            }
        } else if !on
            && registered
            && let Err(e) = gs.unregister(sc)
        {
            tracing::debug!("media key {code:?} unregister: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_only_when_the_system_controls_are_off() {
        let s = |media_keys, media_session| PlatformSettings { media_keys, media_session, ..Default::default() };
        assert!(!wants_shortcuts(&s(true, true), true), "the session gets the keys");
        assert!(wants_shortcuts(&s(true, true), false), "session refused: fall back to shortcuts");
        assert!(wants_shortcuts(&s(true, false), false));
        assert!(!wants_shortcuts(&s(false, false), false));
        assert!(!wants_shortcuts(&s(false, true), false));
    }

    #[test]
    fn media_keys_map_to_commands() {
        assert_eq!(key_command(Code::MediaPlayPause), Some(OsCommand::Toggle));
        assert_eq!(key_command(Code::MediaTrackNext), Some(OsCommand::Next));
        assert_eq!(key_command(Code::MediaTrackPrevious), Some(OsCommand::Previous));
        assert_eq!(key_command(Code::KeyA), None);
    }
}
