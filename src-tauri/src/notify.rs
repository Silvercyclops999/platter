//! Desktop notification when a new track starts (Settings, App, "Notify on
//! track change"). Off by default. Skipped while the Platter window has
//! focus, during a private session, and when tracks are changing fast.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast;

use crate::audio::events::TrackingEvent;
use crate::db::ArtCache;
use crate::model::Track;

/// A track that starts within this long of the previous one isn't announced
/// (skipping through songs shouldn't pile up notifications).
pub const FAST_SKIP: Duration = Duration::from_secs(5);
/// Cover size for the notification image.
const COVER_PX: u32 = 256;

/// What the gate needs to know about the moment a track starts.
#[derive(Debug, Clone, Copy, Default)]
pub struct Conditions {
    pub enabled: bool,
    pub window_focused: bool,
    pub private_session: bool,
}

/// Decides which track changes get a notification.
#[derive(Debug, Default)]
pub struct Gate {
    last_change: Option<Instant>,
}

impl Gate {
    /// Call on every new track. True when this one should be announced.
    pub fn on_track(&mut self, now: Instant, c: Conditions) -> bool {
        let fast = self
            .last_change
            .is_some_and(|t| now.saturating_duration_since(t) < FAST_SKIP);
        self.last_change = Some(now);
        c.enabled && !c.window_focused && !c.private_session && !fast
    }
}

/// "Artist · Album" (the track's own artist when it has one).
pub fn body(track: &Track) -> String {
    let artist = track
        .track_artist
        .as_deref()
        .filter(|a| !a.is_empty())
        .unwrap_or(&track.artist_title);
    match (artist.is_empty(), track.album_title.is_empty()) {
        (false, false) => format!("{artist} · {}", track.album_title),
        (false, true) => artist.to_owned(),
        (true, _) => track.album_title.clone(),
    }
}

/// The switches the notifier reads.
#[derive(Clone)]
pub struct Switches {
    pub enabled: Arc<AtomicBool>,
    pub private_session: Arc<AtomicBool>,
}

fn main_window<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::WebviewWindow<R>> {
    app.get_webview_window("main")
}

/// Whether the main window has focus (a phone app in use always has).
#[cfg(desktop)]
fn is_focused<R: Runtime>(app: &AppHandle<R>) -> bool {
    main_window(app).and_then(|w| w.is_focused().ok()).unwrap_or(false)
}

#[cfg(mobile)]
fn is_focused<R: Runtime>(app: &AppHandle<R>) -> bool {
    main_window(app).is_some()
}

/// Brings the window forward (a click on a notification).
fn bring_forward<R: Runtime>(app: &AppHandle<R>) {
    crate::platform::show_window(app);
}

/// Saves the cover where the notification can read it. None on any failure
/// (the notification goes out without it).
async fn cover_file(art: &ArtCache, thumb: &str, dir: &Path) -> Option<PathBuf> {
    let img = art.get_or_fetch(thumb, COVER_PX, COVER_PX).await.ok()?;
    let ext = match img.content_type.as_str() {
        "image/png" => "png",
        "image/webp" => "webp",
        _ => "jpg",
    };
    let path = dir.join(format!("notify-cover.{ext}"));
    let bytes = img.bytes.clone();
    let p = path.clone();
    tokio::task::spawn_blocking(move || std::fs::write(&p, &bytes))
        .await
        .ok()?
        .ok()?;
    Some(path)
}

/// Listens for new tracks and shows the notifications.
pub fn spawn<R: Runtime>(
    app: AppHandle<R>,
    mut events: broadcast::Receiver<TrackingEvent>,
    switches: Switches,
    art: ArtCache,
    cover_dir: PathBuf,
) {
    let click = app.clone();
    if let Err(e) = app.notification().on_action(move |_| bring_forward(&click)) {
        tracing::debug!("notification clicks unavailable: {e}");
    }
    tauri::async_runtime::spawn(async move {
        let mut gate = Gate::default();
        loop {
            let track = match events.recv().await {
                Ok(TrackingEvent::TrackStarted { track, .. }) => track,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            };
            let c = Conditions {
                enabled: switches.enabled.load(Ordering::Relaxed),
                window_focused: is_focused(&app),
                private_session: switches.private_session.load(Ordering::Relaxed),
            };
            if !gate.on_track(Instant::now(), c) {
                continue;
            }
            let icon = match &track.thumb {
                Some(t) => cover_file(&art, t, &cover_dir).await,
                None => None,
            };
            let mut n = app.notification().builder().title(&track.title).body(body(&track));
            if let Some(p) = icon {
                n = n.icon(p.to_string_lossy());
            }
            if let Err(e) = n.show() {
                tracing::debug!("track notification failed: {e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::testutil::track;

    fn on() -> Conditions {
        Conditions { enabled: true, ..Default::default() }
    }

    #[test]
    fn announces_a_track_when_on_and_the_window_is_in_the_background() {
        let mut g = Gate::default();
        assert!(g.on_track(Instant::now(), on()));
    }

    #[test]
    fn off_by_setting_focus_or_private_session() {
        let t0 = Instant::now();
        let mut g = Gate::default();
        assert!(!g.on_track(t0, Conditions::default()));
        let mut g = Gate::default();
        assert!(!g.on_track(t0, Conditions { window_focused: true, ..on() }));
        let mut g = Gate::default();
        assert!(!g.on_track(t0, Conditions { private_session: true, ..on() }));
    }

    #[test]
    fn fast_skipping_is_quiet_until_a_track_has_played_5_s() {
        let t0 = Instant::now();
        let mut g = Gate::default();
        assert!(g.on_track(t0, on()));
        // Skips 1 s and 4.9 s apart: quiet.
        assert!(!g.on_track(t0 + Duration::from_secs(1), on()));
        assert!(!g.on_track(t0 + Duration::from_millis(5_900), on()));
        // The next one comes 5 s after the last change.
        assert!(g.on_track(t0 + Duration::from_millis(10_900), on()));
    }

    #[test]
    fn suppressed_changes_still_count_for_fast_skipping() {
        let t0 = Instant::now();
        let mut g = Gate::default();
        assert!(!g.on_track(t0, Conditions { window_focused: true, ..on() }));
        assert!(!g.on_track(t0 + Duration::from_secs(2), on()));
        assert!(g.on_track(t0 + Duration::from_secs(8), on()));
    }

    #[test]
    fn body_is_artist_and_album() {
        let mut t = track("1", "Band", "/p/1");
        t.album_title = "Record".into();
        assert_eq!(body(&t), "Band · Record");
        t.track_artist = Some("Guest".into());
        assert_eq!(body(&t), "Guest · Record");
        t.album_title.clear();
        assert_eq!(body(&t), "Guest");
    }
}
