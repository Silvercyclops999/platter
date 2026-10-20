//! Siri's "play … on Platter", handled inside the app (media apps may take
//! `INPlayMediaIntent` in the app process since iOS 14). The app delegate is
//! Tauri's, so [`install`] adds `application:handlerForIntent:` to its class
//! at run time; the handler reads what Siri heard, matches it with
//! [`super::siri::resolve`] and plays it.
//!
//! Siri only routes requests here with the Siri entitlement
//! (`com.apple.developer.siri`) and `INIntentsSupported` in Info.plist.

use std::sync::OnceLock;

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, NSObject, Sel};
use objc2::{AnyThread, define_class, msg_send, sel};
use objc2_foundation::{NSNumber, NSString};
use tauri::{AppHandle, Manager, Wry};

use super::siri::{SiriAsk, SiriKind};
use crate::audio::player::PlayerHandle;
use crate::db::DbHandle;

std::arch::global_asm!(".linker_option \"-framework\", \"Intents\"");

static APP: OnceLock<AppHandle<Wry>> = OnceLock::new();

/// `INPlayMediaIntentResponseCode`.
const SUCCESS: isize = 4;
const FAILURE: isize = 6;

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "PlatterPlayMediaHandler"]
    struct PlayMediaHandler;

    #[allow(non_snake_case)]
    impl PlayMediaHandler {
        #[unsafe(method(handlePlayMedia:completion:))]
        fn handlePlayMedia_completion(&self, intent: &AnyObject, completion: &DynBlock<dyn Fn(*mut AnyObject)>) {
            let code = if play(intent) { SUCCESS } else { FAILURE };
            // SAFETY: the response class's designated initialiser; the
            // completion block is Siri's, called once with it.
            unsafe {
                let Some(class) = AnyClass::get(c"INPlayMediaIntentResponse") else { return };
                let response: *mut AnyObject = msg_send![class, alloc];
                let response: *mut AnyObject =
                    msg_send![response, initWithCode: code, userActivity: std::ptr::null::<AnyObject>()];
                completion.call((response,));
            }
        }
    }
);

/// Reads Siri's request and starts playing it; false when nothing fits.
fn play(intent: &AnyObject) -> bool {
    let Some(app) = APP.get() else { return false };
    // SAFETY: INPlayMediaIntent's documented properties: an INMediaSearch
    // (strings and an INMediaItemType) and an optional NSNumber.
    let ask = unsafe {
        let text = |obj: *mut AnyObject, sel: Sel| -> Option<String> {
            let obj = obj.as_ref()?;
            let s: *mut NSString = msg_send![obj, performSelector: sel];
            s.as_ref().map(|s| s.to_string()).filter(|s| !s.trim().is_empty())
        };
        let search: *mut AnyObject = msg_send![intent, mediaSearch];
        let kind = match search.as_ref() {
            Some(s) => {
                let t: isize = msg_send![s, mediaType];
                SiriKind::from_media_type(t)
            }
            None => SiriKind::Unknown,
        };
        let shuffled: *mut NSNumber = msg_send![intent, playShuffled];
        SiriAsk {
            name: text(search, sel!(mediaName)),
            artist: text(search, sel!(artistName)),
            album: text(search, sel!(albumName)),
            kind,
            shuffle: shuffled.as_ref().is_some_and(|n| n.boolValue()),
        }
    };
    tracing::info!(kind = ?ask.kind, shuffle = ask.shuffle, "Siri asked to play");
    let db = app.state::<DbHandle>().inner().clone();
    let picked = super::siri::resolve(&db, &ask, || db.read(super::shuffle_library_context));
    let Ok(Some(p)) = picked else {
        tracing::info!("Siri: nothing in the library fits");
        return false;
    };
    let player = app.state::<PlayerHandle>().inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = player.play_context_from(p.context, p.start_track_id, None, Some(p.shuffle)).await {
            tracing::warn!("Siri play failed: {e}");
        }
    });
    true
}

/// The handler object, made once.
fn handler() -> *mut AnyObject {
    struct Kept(Retained<PlayMediaHandler>);
    // SAFETY: the handler holds no state and is only messaged by Siri.
    unsafe impl Send for Kept {}
    unsafe impl Sync for Kept {}
    static HANDLER: OnceLock<Kept> = OnceLock::new();
    let h = HANDLER.get_or_init(|| {
        let this = PlayMediaHandler::alloc().set_ivars(());
        // SAFETY: NSObject's init on our own subclass.
        Kept(unsafe { msg_send![super(this), init] })
    });
    Retained::as_ptr(&h.0) as *mut AnyObject
}

/// `-[AppDelegate application:handlerForIntent:]`: our handler for a play
/// request, nothing for anything else.
extern "C-unwind" fn handler_for_intent(
    _this: *mut AnyObject,
    _cmd: Sel,
    _app: *mut AnyObject,
    intent: *mut AnyObject,
) -> *mut AnyObject {
    // SAFETY: `intent` is an INIntent; the class is looked up by name.
    let play_media = unsafe {
        match (intent.as_ref(), AnyClass::get(c"INPlayMediaIntent")) {
            (Some(i), Some(c)) => {
                let yes: bool = msg_send![i, isKindOfClass: c];
                yes
            }
            _ => false,
        }
    };
    if play_media { handler() } else { std::ptr::null_mut() }
}

/// Adds the intent handler to the app delegate's class (main thread, once
/// the app is up).
pub fn install(app: &AppHandle<Wry>) {
    let _ = APP.set(app.clone());
    // SAFETY: reading the app's delegate on the main thread, and adding one
    // method (that it doesn't have) to its class.
    unsafe {
        let Some(ui_app) = AnyClass::get(c"UIApplication") else { return };
        let shared: *mut AnyObject = msg_send![ui_app, sharedApplication];
        let Some(shared) = shared.as_ref() else { return };
        let delegate: *mut AnyObject = msg_send![shared, delegate];
        let Some(delegate) = delegate.as_ref() else {
            tracing::warn!("Siri: no app delegate to hand requests to");
            return;
        };
        let class = delegate.class() as *const AnyClass as *mut AnyClass;
        let imp: Imp = std::mem::transmute(
            handler_for_intent as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> *mut AnyObject,
        );
        let added = objc2::ffi::class_addMethod(class, sel!(application:handlerForIntent:), imp, c"@@:@@".as_ptr());
        tracing::debug!(added = added.as_bool(), "Siri intent handler");
    }
}
