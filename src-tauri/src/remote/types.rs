//! IPC shapes for Listen on (docs/IPC.md "Listen on").

use serde::{Deserialize, Serialize};

use crate::audio::queue::Repeat;
use crate::model::Track;

pub const EV_REMOTE_STATE: &str = "remote://state";
pub const EV_REMOTE_PLAYERS: &str = "remote://players";

/// What a player is, for its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayerKind {
    Computer,
    Phone,
    Speaker,
    Browser,
    Tv,
}

/// One Plex player in the device picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePlayer {
    /// The player's Plex client identifier.
    pub id: String,
    pub name: String,
    /// "Plexamp", "Plex for Windows", "Sonos"...
    pub product: String,
    pub kind: PlayerKind,
    /// What it's playing now, from the server's sessions ("Song • Artist"),
    /// with "Paused: " in front when paused.
    pub playing: Option<String>,
    /// Seen on the network now (registered with the server, present on
    /// plex.tv, or playing). Others are listed but may not answer.
    pub online: bool,
}

/// Remote playback status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteStatus {
    /// playMedia was accepted; the player hasn't reported yet.
    Connecting,
    Playing,
    Paused,
    Buffering,
}

/// Playback on another device ("Playing on <device>").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSession {
    pub player_id: String,
    pub player_name: String,
    pub kind: PlayerKind,
    pub status: RemoteStatus,
    pub track: Option<Track>,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// 0..1, when the player reports it.
    pub volume: Option<f32>,
    pub shuffle: bool,
    pub repeat: Repeat,
}

/// `remote_state` and the `remote://state` event.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteState {
    /// Set while playback is on another device.
    pub session: Option<RemoteSession>,
    /// Set when the other device stopped by itself (someone else stopped
    /// it there); controls are local again. Cleared by the next move.
    pub stopped_on: Option<String>,
}

/// `remote_command`: what the player bar's controls send in remote mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RemoteCommand {
    Play,
    Pause,
    Next,
    Previous,
    Seek { position_ms: u64 },
    /// 0..1.
    Volume { volume: f32 },
    Shuffle { on: bool },
    Repeat { mode: Repeat },
}

/// Plex companion `repeat`: 0 off, 1 one, 2 all.
pub fn repeat_code(r: Repeat) -> u8 {
    match r {
        Repeat::Off => 0,
        Repeat::One => 1,
        Repeat::All => 2,
    }
}

pub fn repeat_from_code(c: i64) -> Repeat {
    match c {
        1 => Repeat::One,
        2 => Repeat::All,
        _ => Repeat::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_tagged_camel_case() {
        let c: RemoteCommand = serde_json::from_str(r#"{"kind":"seek","positionMs":5000}"#).unwrap();
        assert_eq!(c, RemoteCommand::Seek { position_ms: 5000 });
        let c: RemoteCommand = serde_json::from_str(r#"{"kind":"repeat","mode":"all"}"#).unwrap();
        assert_eq!(c, RemoteCommand::Repeat { mode: Repeat::All });
        let c: RemoteCommand = serde_json::from_str(r#"{"kind":"next"}"#).unwrap();
        assert_eq!(c, RemoteCommand::Next);
    }

    #[test]
    fn repeat_codes_round_trip() {
        for r in [Repeat::Off, Repeat::One, Repeat::All] {
            assert_eq!(repeat_from_code(repeat_code(r) as i64), r);
        }
    }

    #[test]
    fn state_json_shape() {
        let v = serde_json::to_value(RemoteState { session: None, stopped_on: Some("Den".into()) }).unwrap();
        assert_eq!(v["stoppedOn"], "Den");
        assert!(v["session"].is_null());
    }
}
