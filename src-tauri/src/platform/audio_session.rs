//! The iPhone's audio session. Platter is a music player, so it asks for the
//! playback category: music plays with the ringer switch on silent, keeps
//! playing when the screen locks (with the audio background mode in
//! Info.ios.plist), and other apps' audio stops when it starts.
//!
//! [`watch`] follows what the system does to the session: a call or another
//! app's video pauses Platter and its end may play on; headphones leaving
//! pause it, and the same ones coming back within a few seconds (AirPods
//! hopping to a Mac and back) play on; a media services reset sets the
//! session up again. After any of these the output stream is rebuilt before
//! the next sound, so Play always works.

use std::sync::Mutex;
use std::time::Instant;

use objc2_avf_audio::{
    AVAudioSession, AVAudioSessionCategoryPlayback, AVAudioSessionPortBluetoothA2DP, AVAudioSessionPortBluetoothLE,
    AVAudioSessionPortHeadphones, AVAudioSessionPortUSBAudio, AVAudioSessionRouteDescription,
};

use super::route_move::{RouteMove, route_move};
use crate::audio::PlayerHandle;

/// Makes the session active again (after another app took it). Cheap when
/// it already is.
pub fn activate() {
    // SAFETY: a plain call on the shared session.
    unsafe {
        if let Err(e) = AVAudioSession::sharedInstance().setActive_error(true) {
            tracing::warn!("audio session: not activated: {}", e.localizedDescription());
        }
    }
}

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
    // SAFETY: reading the shared session's current route.
    unsafe { route_headphones(&AVAudioSession::sharedInstance().currentRoute()) }
}

/// The headphones a route plays to, by name.
///
/// # Safety
/// `route` is a live route description.
unsafe fn route_headphones(route: &AVAudioSessionRouteDescription) -> Option<String> {
    // SAFETY: reading the route's ports; the port type constants are the
    // framework's own strings.
    unsafe {
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

/// Follows interruptions, route changes and media services resets for the
/// app's life. Call once, after the player starts.
pub fn watch(player: PlayerHandle, app: tauri::AppHandle) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| observe_session(player, app));
}

/// Where the sound goes now (the first output port).
pub fn current_route() -> Option<super::audio_route::AudioRoute> {
    // SAFETY: reading the shared session's current route.
    unsafe {
        let route = AVAudioSession::sharedInstance().currentRoute();
        let port = route.outputs().firstObject()?;
        Some(super::audio_route::AudioRoute {
            kind: super::audio_route::kind_of(&port.portType().to_string()),
            name: port.portName().to_string(),
        })
    }
}

fn observe_session(player: PlayerHandle, app: tauri::AppHandle) {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_avf_audio::{
        AVAudioSessionInterruptionNotification, AVAudioSessionInterruptionOptionKey,
        AVAudioSessionInterruptionTypeKey, AVAudioSessionMediaServicesWereResetNotification,
        AVAudioSessionRouteChangeNotification, AVAudioSessionRouteChangePreviousRouteKey,
        AVAudioSessionRouteChangeReasonKey,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSNumber, NSString};

    static LEFT: Mutex<Option<(String, Instant)>> = Mutex::new(None);

    /// A number from the notification's userInfo.
    unsafe fn info_number(note: &NSNotification, key: Option<&NSString>) -> Option<usize> {
        // SAFETY: userInfo is an NSDictionary; the values under these keys
        // are NSNumbers.
        unsafe {
            let info: *mut AnyObject = msg_send![note, userInfo];
            let info = info.as_ref()?;
            let n: *mut NSNumber = msg_send![info, objectForKey: key?];
            Some(n.as_ref()?.unsignedIntegerValue())
        }
    }

    // SAFETY: NSNotificationCenter's block observer API with AVFAudio's own
    // notification names; the blocks only read userInfo and message the
    // player, and the observers live for the app's life.
    unsafe {
        let center = NSNotificationCenter::defaultCenter();
        let observe = |name: Option<&NSString>, f: Box<dyn Fn(&NSNotification)>| {
            let Some(name) = name else { return };
            let block = RcBlock::new(move |note: std::ptr::NonNull<NSNotification>| f(note.as_ref()));
            let observer = center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block);
            std::mem::forget(observer);
        };

        let p = player.clone();
        observe(
            AVAudioSessionInterruptionNotification,
            Box::new(move |note| {
                let began = info_number(note, AVAudioSessionInterruptionTypeKey) == Some(1);
                let resume = info_number(note, AVAudioSessionInterruptionOptionKey).is_some_and(|o| o & 1 != 0);
                tracing::info!(began, resume, "audio interruption");
                p.interruption(began, resume);
            }),
        );

        let p = player.clone();
        observe(
            AVAudioSessionRouteChangeNotification,
            Box::new(move |note| {
                {
                    use tauri::Emitter;
                    let _ = app.emit(super::audio_route::EV_ROUTE, current_route());
                }
                let Some(reason) = info_number(note, AVAudioSessionRouteChangeReasonKey) else { return };
                // None: the notification didn't say what the route was.
                let before: Option<Option<String>> = AVAudioSessionRouteChangePreviousRouteKey.and_then(|key| {
                    let info: *mut AnyObject = msg_send![note, userInfo];
                    let route: *mut AVAudioSessionRouteDescription = msg_send![info.as_ref()?, objectForKey: key];
                    Some(route_headphones(route.as_ref()?))
                });
                let now = headphones_name();
                let mut left = LEFT.lock().unwrap_or_else(|e| e.into_inner());
                match route_move(reason, before, now, &left, Instant::now()) {
                    RouteMove::Left(name) => {
                        tracing::info!("headphones left: pausing");
                        *left = Some((name, Instant::now()));
                        p.headphones(false);
                    }
                    RouteMove::Back => {
                        tracing::info!("headphones back: playing on");
                        *left = None;
                        p.headphones(true);
                    }
                    RouteMove::Nothing => {}
                }
            }),
        );

        let p = player;
        observe(
            AVAudioSessionMediaServicesWereResetNotification,
            Box::new(move |_| {
                tracing::info!("audio media services reset");
                // setActive can block: off the posting (often main) thread.
                let p = p.clone();
                std::thread::spawn(move || {
                    start();
                    p.output_stale();
                });
            }),
        );
    }
}
