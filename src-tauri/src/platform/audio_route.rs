//! Where the phone's sound is going: the speaker, headphones, a Bluetooth
//! speaker, AirPlay, the car. The UI names it in the device sheet and the
//! mini player. `audio_route` answers now, and `audio://route` says so
//! again on every route change (iOS only; computers pick their output in
//! the output sheet).

use serde::Serialize;

pub const EV_ROUTE: &str = "audio://route";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioRoute {
    /// "speaker", "headphones", "bluetooth", "airplay", "car", "usb",
    /// "hdmi", "receiver" or "other".
    pub kind: &'static str,
    /// The port's own name ("Adam's AirPods Pro", "Living Room").
    pub name: String,
}

/// A route's kind from AVAudioSession's port type (its string value).
pub fn kind_of(port_type: &str) -> &'static str {
    match port_type {
        "Speaker" => "speaker",
        "Headphones" | "LineOut" => "headphones",
        "BluetoothA2DPOutput" | "BluetoothLE" | "BluetoothHFP" => "bluetooth",
        "AirPlay" => "airplay",
        "CarAudio" => "car",
        "USBAudio" => "usb",
        "HDMI" | "DisplayPort" => "hdmi",
        "Receiver" => "receiver",
        _ => "other",
    }
}

/// Where the sound goes now; None off the phone.
#[tauri::command]
pub fn audio_route() -> Option<AudioRoute> {
    #[cfg(target_os = "ios")]
    return super::audio_session::current_route();
    #[cfg(not(target_os = "ios"))]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_types_read_as_plain_kinds() {
        assert_eq!(kind_of("Speaker"), "speaker");
        assert_eq!(kind_of("Headphones"), "headphones");
        assert_eq!(kind_of("BluetoothA2DPOutput"), "bluetooth");
        assert_eq!(kind_of("BluetoothLE"), "bluetooth");
        assert_eq!(kind_of("AirPlay"), "airplay");
        assert_eq!(kind_of("CarAudio"), "car");
        assert_eq!(kind_of("USBAudio"), "usb");
        assert_eq!(kind_of("Receiver"), "receiver");
        assert_eq!(kind_of("Something new"), "other");
    }

    #[test]
    fn a_route_goes_to_the_ui_in_camel_case() {
        let r = AudioRoute { kind: "airplay", name: "Living Room".into() };
        assert_eq!(serde_json::to_value(&r).unwrap(), serde_json::json!({ "kind": "airplay", "name": "Living Room" }));
    }
}
