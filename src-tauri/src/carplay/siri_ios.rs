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
use objc2::{AnyThread, ClassType, define_class, msg_send, sel};
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
            let done = Completion(Some(completion.copy()));
            let ask = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read_ask(intent))).ok();
            let Some(app) = APP.get().cloned() else {
                done.finish(FAILURE);
                return;
            };
            // Matching reads the library: off this (possibly main) thread.
            tauri::async_runtime::spawn_blocking(move || {
                let ok = ask
                    .and_then(|ask| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| play(&app, ask))).ok())
                    .unwrap_or(false);
                done.finish(if ok { SUCCESS } else { FAILURE });
            });
        }
    }
);

/// Siri's completion block, copied so it can be called once from another
/// thread. Dropped unanswered (a panic on the way), it answers Failure.
struct Completion(Option<block2::RcBlock<dyn Fn(*mut AnyObject)>>);
// SAFETY: Intents' completion blocks may be called from any thread.
unsafe impl Send for Completion {}

impl Drop for Completion {
    fn drop(&mut self) {
        if let Some(block) = self.0.take() {
            answer(&block, FAILURE);
        }
    }
}

impl Completion {
    /// Answers Siri with `code`; the block is called once.
    fn finish(mut self, code: isize) {
        if let Some(block) = self.0.take() {
            answer(&block, code);
        }
    }
}

/// Calls Siri's block with a response for `code`.
fn answer(block: &block2::RcBlock<dyn Fn(*mut AnyObject)>, code: isize) {
    {
        // SAFETY: the response class's designated initialiser; the response
        // is handed to the block and released after.
        unsafe {
            let response: Option<Retained<AnyObject>> = AnyClass::get(c"INPlayMediaIntentResponse").and_then(|class| {
                let alloc: *mut AnyObject = msg_send![class, alloc];
                let r: *mut AnyObject =
                    msg_send![alloc, initWithCode: code, userActivity: std::ptr::null::<AnyObject>()];
                Retained::from_raw(r)
            });
            let ptr = response.as_ref().map_or(std::ptr::null_mut(), |r| Retained::as_ptr(r) as *mut AnyObject);
            block.call((ptr,));
        }
    }
}

/// What Siri heard, from an INPlayMediaIntent.
fn read_ask(intent: &AnyObject) -> SiriAsk {
    // SAFETY: INPlayMediaIntent's documented properties: an INMediaSearch
    // (strings and an INMediaItemType) and an optional NSNumber.
    unsafe {
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
    }
}

/// Matches the request and starts playing it; false when nothing fits.
fn play(app: &AppHandle<Wry>, ask: SiriAsk) -> bool {
    tracing::info!(kind = ?ask.kind, shuffle = ask.shuffle, "Siri asked to play");
    let (Some(db), Some(player)) = (app.try_state::<DbHandle>(), app.try_state::<PlayerHandle>()) else {
        return false;
    };
    let db = db.inner().clone();
    let player = player.inner().clone();
    let p = match super::siri::resolve(&db, &ask, || db.read(super::shuffle_library_context)) {
        Ok(Some(p)) => p,
        Ok(None) => {
            // A song you don't have: got through Lidarr and played when it's here.
            if let Some(q) = super::siri::catalogue_query(&ask) {
                return tap_catalogue(app, &ask, &q);
            }
            tracing::info!("Siri: nothing in the library fits");
            return false;
        }
        Err(e) => {
            tracing::warn!("Siri: the library couldn't be searched: {e}");
            return false;
        }
    };
    tauri::async_runtime::spawn(async move {
        if let Err(e) = player.play_context_from(p.context, p.start_track_id, None, Some(p.shuffle)).await {
            tracing::warn!("Siri play failed: {e}");
        }
    });
    true
}

/// Looks the song up among music you don't have and taps it (Lidarr gets it
/// and it plays when it's here). False when nothing fits or a tap can't run.
fn tap_catalogue(app: &AppHandle<Wry>, ask: &SiriAsk, query: &str) -> bool {
    let (Some(discover), Some(taps)) = (app.try_state::<crate::discover::Discover>(), app.try_state::<crate::lidarr::tap::Taps>()) else {
        return false;
    };
    let (discover, taps) = (discover.inner().clone(), taps.inner().clone());
    let ask = ask.clone();
    let query = query.to_string();
    // Siri waits a few seconds for an answer.
    let run = async move {
        let part = discover.search_part(&query, "songs").await?;
        let Some(input) = super::siri::tap_for(&ask, &part.songs) else { return Ok(false) };
        let out = taps.tap(input).await?;
        Ok::<bool, crate::error::AppError>(out.tap.is_some())
    };
    match tauri::async_runtime::block_on(async { tokio::time::timeout(std::time::Duration::from_secs(8), run).await }) {
        Ok(Ok(true)) => {
            tracing::info!("Siri: getting a song you don't have");
            true
        }
        Ok(Ok(false)) => {
            tracing::info!("Siri: nothing in the library or the catalogue fits");
            false
        }
        Ok(Err(e)) => {
            tracing::warn!("Siri: the song couldn't be got: {e}");
            false
        }
        Err(_) => {
            tracing::warn!("Siri: looking the song up took too long");
            false
        }
    }
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

/// The original `-[UIApplication setDelegate:]`.
static SET_DELEGATE: OnceLock<Imp> = OnceLock::new();

/// Adds `application:handlerForIntent:` to a delegate's class (if it has
/// none), before UIKit looks at what the delegate answers to.
fn add_handler_to(delegate: *mut AnyObject) {
    // SAFETY: adding one method to the delegate's class; the class is the
    // object's own.
    unsafe {
        let Some(d) = delegate.as_ref() else { return };
        let class = d.class() as *const AnyClass as *mut AnyClass;
        let imp: Imp = std::mem::transmute(
            handler_for_intent as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> *mut AnyObject,
        );
        if !objc2::ffi::class_addMethod(class, sel!(application:handlerForIntent:), imp, c"@@:@@".as_ptr()).as_bool() {
            tracing::debug!("Siri: the app delegate already handles intents");
        }
    }
}

/// `setDelegate:` that gives the delegate the intent handler first.
extern "C-unwind" fn set_delegate(this: *mut AnyObject, cmd: Sel, delegate: *mut AnyObject) {
    add_handler_to(delegate);
    match SET_DELEGATE.get() {
        // SAFETY: the original has this exact signature.
        Some(original) => unsafe {
            let f: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = std::mem::transmute(*original);
            f(this, cmd, delegate);
        },
        // Can't happen: ours is only put in after the original is kept.
        None => tracing::error!("Siri: UIApplication's setDelegate: is missing"),
    }
}

/// Runs before the app starts (before UIKit sets the app delegate and
/// caches what it answers to). Keeps the original setter, and only then
/// puts ours in its place; once.
pub fn install_early() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = PlayMediaHandler::class();
        let Some(ui_app) = AnyClass::get(c"UIApplication") else { return };
        // SAFETY: the runtime's own lookup of an existing method, and
        // swapping its implementation for one of the same shape that calls
        // the kept original.
        unsafe {
            let method = objc2::ffi::class_getInstanceMethod(ui_app, sel!(setDelegate:));
            if method.is_null() {
                tracing::warn!("Siri: UIApplication has no setDelegate:");
                return;
            }
            let Some(original) = objc2::ffi::method_getImplementation(method) else { return };
            if SET_DELEGATE.set(original).is_err() {
                return;
            }
            let ours: Imp = std::mem::transmute(set_delegate as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject));
            objc2::ffi::method_setImplementation(method as *mut _, ours);
        }
    });
}

/// The app is up: requests can be played.
pub fn attach(app: &AppHandle<Wry>) {
    let _ = APP.set(app.clone());
}
