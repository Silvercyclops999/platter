//! Payloads the player emits: UI events (docs/IPC.md "Playback & queue") and
//! the internal play-tracking stream.

use serde::{Deserialize, Serialize};

use super::queue::{ContextInfo, QueueView, Repeat};
use crate::model::Track;

pub const EV_STATE: &str = "player://state";
pub const EV_POSITION: &str = "player://position";
pub const EV_ERROR: &str = "player://error";

/// The message shown when a track can't be played.
pub const SKIPPED_MESSAGE: &str = "Skipped: file unavailable";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayerStatus {
    Idle,
    Loading,
    Playing,
    Paused,
    Buffering,
    Error,
}

/// `PlayerState` in docs/IPC.md.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub status: PlayerStatus,
    pub track: Option<Track>,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// Linear slider value 0..1 (the perceptual curve is applied inside).
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub queue: QueueView,
}

/// `player://position` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionPayload {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub buffered_ms: u64,
}

/// `player://error` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub track_id: Option<String>,
    pub kind: String,
    pub message: String,
}

/// One entry of the Problems list (tracks that couldn't be played).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub kind: String,
    pub message: String,
    /// Unix ms.
    pub at: i64,
}

/// Where the UI events go. The app uses [`TauriSink`]; tests capture.
pub trait EventSink: Send + Sync {
    fn state(&self, state: &PlayerState);
    fn position(&self, pos: &PositionPayload);
    fn error(&self, err: &ErrorPayload);
}

/// Emits to every webview via Tauri's event system.
pub struct TauriSink<R: tauri::Runtime>(pub tauri::AppHandle<R>);

impl<R: tauri::Runtime> EventSink for TauriSink<R> {
    fn state(&self, state: &PlayerState) {
        use tauri::Emitter;
        let _ = self.0.emit(EV_STATE, state);
    }
    fn position(&self, pos: &PositionPayload) {
        use tauri::Emitter;
        let _ = self.0.emit(EV_POSITION, pos);
    }
    fn error(&self, err: &ErrorPayload) {
        use tauri::Emitter;
        let _ = self.0.emit(EV_ERROR, err);
    }
}

/// Signals for play tracking: Plex `/:/timeline` + `/:/scrobble`
/// and the local listen log. Delivered on a `tokio::sync::broadcast`
/// channel from [`super::player::PlayerHandle::subscribe_tracking`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum TrackingEvent {
    /// A track became audible (also after gapless transitions).
    #[serde(rename_all = "camelCase")]
    TrackStarted {
        track: Track,
        /// Queue entry id (Plex play queue item mapping later).
        item_id: String,
        /// Unix ms.
        at: i64,
        context: Option<ContextInfo>,
    },
    /// About once per second while playing.
    #[serde(rename_all = "camelCase")]
    Progress {
        track_id: String,
        position_ms: u64,
        duration_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    Paused { track_id: String, position_ms: u64 },
    #[serde(rename_all = "camelCase")]
    Resumed { track_id: String, position_ms: u64 },
    #[serde(rename_all = "camelCase")]
    Seeked {
        track_id: String,
        from_ms: u64,
        to_ms: u64,
    },
    /// The track stopped being the current one.
    #[serde(rename_all = "camelCase")]
    TrackEnded {
        track: Track,
        item_id: String,
        /// Audible time actually played (seeks excluded).
        listened_ms: u64,
        /// Played to its natural end.
        completed: bool,
        /// Position when the user skipped away (next/previous/jump/new
        /// context/stop), if it didn't complete.
        skipped_at_ms: Option<u64>,
    },
}

/// Perceptual volume: linear slider 0..1 → amplitude. A cubic curve tracks
/// loudness perception closely (half the slider ≈ -18 dB) and hits 0 at 0.
pub fn perceptual_gain(slider: f32) -> f32 {
    let v = if slider.is_finite() { slider.clamp(0.0, 1.0) } else { 0.0 };
    v * v * v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_curve_is_monotonic_and_bounded() {
        assert_eq!(perceptual_gain(0.0), 0.0);
        assert_eq!(perceptual_gain(1.0), 1.0);
        let mut prev = -1.0;
        for i in 0..=1000 {
            let g = perceptual_gain(i as f32 / 1000.0);
            assert!(g > prev || (i == 0 && g == 0.0), "not increasing at {i}");
            assert!((0.0..=1.0).contains(&g));
            prev = g;
        }
        // Out-of-range input is clamped.
        assert_eq!(perceptual_gain(2.0), 1.0);
        assert_eq!(perceptual_gain(-1.0), 0.0);
        assert_eq!(perceptual_gain(f32::NAN), 0.0);
        // Half the slider is roughly -18 dB, i.e. clearly quieter but audible.
        let db = 20.0 * perceptual_gain(0.5).log10();
        assert!((-19.0..-17.0).contains(&db), "{db}");
    }

    #[test]
    fn tracking_event_json_shape() {
        let e = TrackingEvent::Seeked {
            track_id: "1".into(),
            from_ms: 1,
            to_ms: 2,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "seeked");
        assert_eq!(v["fromMs"], 1);
    }

    #[test]
    fn state_json_is_camel_case() {
        let s = PlayerState {
            status: PlayerStatus::Buffering,
            track: None,
            position_ms: 5,
            duration_ms: 6,
            volume: 0.5,
            muted: false,
            shuffle: true,
            repeat: Repeat::One,
            queue: QueueView::default(),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["status"], "buffering");
        assert_eq!(v["positionMs"], 5);
        assert_eq!(v["repeat"], "one");
        assert!(v["queue"]["userQueue"].is_array());
        assert!(v["queue"]["nextUp"].is_array());
    }
}
