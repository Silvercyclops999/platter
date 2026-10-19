//! The iPhone's audio session. Platter is a music player, so it asks for the
//! playback category: music plays with the ringer switch on silent, keeps
//! playing when the screen locks (with the audio background mode in
//! Info.ios.plist), and other apps' audio stops when it starts.

use objc2_avf_audio::{
    AVAudioSession, AVAudioSessionCategoryPlayback, AVAudioSessionPortBluetoothA2DP, AVAudioSessionPortBluetoothLE,
    AVAudioSessionPortHeadphones, AVAudioSessionPortUSBAudio,
};

/// Sets up the audio session before any output opens. A refusal is logged
/// and playback carries on with the system's default session.
pub fn start() {
    // SAFETY: plain calls on the shared session; the category constant is
    // a static string the framework provides.
    unsafe {
        let session = AVAudioSession::sharedInstance();
        let Some(category) = AVAudioSessionCategoryPlayback else {
            tracing::warn!("audio session: no playback category");
            return;
        };
        if let Err(e) = session.setCategory_error(category) {
            tracing::warn!("audio session: category refused: {}", e.localizedDescription());
        }
        if let Err(e) = session.setActive_error(true) {
            tracing::warn!("audio session: not activated: {}", e.localizedDescription());
        }
    }
}

/// The name of the headphones playing now ("Adam's AirPods Pro"), or None
/// when the sound goes to the speaker, AirPlay, a car or anything else.
pub fn headphones_name() -> Option<String> {
    // SAFETY: reading the shared session's current route; the port type
    // constants are the framework's own strings.
    unsafe {
        let route = AVAudioSession::sharedInstance().currentRoute();
        let port = route.outputs().firstObject()?;
        let kind = port.portType();
        let wired_or_bt = [
            AVAudioSessionPortHeadphones,
            AVAudioSessionPortBluetoothA2DP,
            AVAudioSessionPortBluetoothLE,
            AVAudioSessionPortUSBAudio,
        ]
        .into_iter()
        .flatten()
        .any(|k| kind.isEqualToString(k));
        wired_or_bt.then(|| port.portName().to_string())
    }
}
