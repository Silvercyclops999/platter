//! Siri inside the app (media apps may take Siri's media requests in the
//! app process since iOS 14): "play … on Platter", "I like this song" and
//! "add this to my playlist". The app delegate is Tauri's, so [`install_early`]
//! adds `application:handlerForIntent:` to its class at run time; the
//! handler reads what Siri heard, matches it with [`super::siri`] and acts.
//!
//! Siri only routes requests here with the Siri entitlement
//! (`com.apple.developer.siri`), the intents in `INIntentsSupported` and the
//! music category in `INSupportedMediaCategories` (Info.plist).
//!
//! Siri may wake Platter in the background with no screen (Platter closed,
//! the phone locked, AirPods). The app core normally starts with the first
//! screen, so the handler starts it then, and requests wait for it.

use std::ffi::CStr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Imp, NSObject, Sel};
use objc2::{AnyThread, ClassType, MainThreadMarker, define_class, msg_send, sel};
use objc2_foundation::{NSNumber, NSString};
use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Wry};

use super::siri::{self, Getting, Playing, SiriAnswer, SiriAsk, SiriKind, SiriPlay, SiriQueue, SiriSort, Situation};
use super::{Kept, Reach};
use crate::audio::events::PlayerStatus;
use crate::audio::player::PlayerHandle;
use crate::audio::queue::AddAt;
use crate::db::DbHandle;
use crate::lidarr::tap::TapInput;

std::arch::global_asm!(".linker_option \"-framework\", \"Intents\"");

static APP: OnceLock<AppHandle<Wry>> = OnceLock::new();

/// How long a request waits for the app core to start.
const START_WAIT: Duration = Duration::from_secs(8);
/// How long a request waits for a song you don't have to start being got.
const GET_WAIT: Duration = Duration::from_secs(5);
/// How long looking a song up among music you don't have may take.
const LOOKUP_WAIT: Duration = Duration::from_secs(5);

/// A response class and its success and failure codes.
#[derive(Clone, Copy)]
struct Reply {
    class: &'static CStr,
    success: isize,
    failure: isize,
}

/// `INPlayMediaIntentResponseCode`: success 4, failure 6.
const PLAY: Reply = Reply { class: c"INPlayMediaIntentResponse", success: 4, failure: 6 };
/// `INUpdateMediaAffinityIntentResponseCode`: success 3, failure 4.
const AFFINITY: Reply = Reply { class: c"INUpdateMediaAffinityIntentResponse", success: 3, failure: 4 };
/// `INAddMediaIntentResponseCode`: success 3, failure 5.
const ADD: Reply = Reply { class: c"INAddMediaIntentResponse", success: 3, failure: 5 };

/// `INMediaItemType` for what Siri says back.
const ITEM_SONG: isize = 1;
const ITEM_ALBUM: isize = 2;
const ITEM_ARTIST: isize = 3;
const ITEM_PLAYLIST: isize = 5;
const ITEM_STATION: isize = 17;
const ITEM_MUSIC: isize = 18;
/// `INMediaAffinityType`.
const AFFINITY_DISLIKE: isize = 2;
/// `INMediaDestinationType`.
const DESTINATION_PLAYLIST: isize = 2;

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "PlatterMediaHandler"]
    struct MediaHandler;

    #[allow(non_snake_case)]
    impl MediaHandler {
        #[unsafe(method(resolveMediaItemsForPlayMedia:withCompletion:))]
        fn resolveMediaItemsForPlayMedia(&self, intent: &AnyObject, completion: &DynBlock<dyn Fn(*mut AnyObject)>) {
            let ask = read(|| read_play(intent));
            answer_later(completion, Fallback::Unsupported { class: c"INPlayMediaMediaItemResolutionResult", array: true }, move |done| {
                let item = ask.and_then(|ask| decide_play(ask)).map(|(_, d)| d);
                let result = match item {
                    Some(Decided::Play(p)) => successes(c"INPlayMediaMediaItemResolutionResult", &p.title, item_type(&p)),
                    Some(Decided::Get(t)) => successes(c"INPlayMediaMediaItemResolutionResult", &format!("{} by {}", t.title, t.artist), ITEM_SONG),
                    Some(Decided::Resume) => one_in_array(not_required(c"INPlayMediaMediaItemResolutionResult")),
                    None => one_in_array(unsupported(c"INPlayMediaMediaItemResolutionResult")),
                };
                done.send(result);
            });
        }

        #[unsafe(method(handlePlayMedia:completion:))]
        fn handlePlayMedia_completion(&self, intent: &AnyObject, completion: &DynBlock<dyn Fn(*mut AnyObject)>) {
            let ask = read(|| read_play(intent));
            answer_later(completion, Fallback::Fail(PLAY), move |done| {
                let ok = ask.and_then(|ask| decide_play(ask)).is_some_and(|(app, d)| act_play(&app, d));
                done.code(PLAY, ok);
            });
        }

        #[unsafe(method(resolveMediaItemsForUpdateMediaAffinity:withCompletion:))]
        fn resolveMediaItemsForUpdateMediaAffinity(&self, intent: &AnyObject, completion: &DynBlock<dyn Fn(*mut AnyObject)>) {
            let ask = read(|| read_search_of(intent));
            let class = c"INUpdateMediaAffinityMediaItemResolutionResult";
            answer_later(completion, Fallback::Unsupported { class, array: true }, move |done| {
                let song = ask.and_then(|ask| Some(song_for(&wait_for_app()?, &ask)?));
                done.send(match song {
                    Some((_, title)) => successes(class, &title, ITEM_SONG),
                    None => one_in_array(unsupported(class)),
                });
            });
        }

        #[unsafe(method(handleUpdateMediaAffinity:completion:))]
        fn handleUpdateMediaAffinity_completion(&self, intent: &AnyObject, completion: &DynBlock<dyn Fn(*mut AnyObject)>) {
            let read_both = || {
                // SAFETY: INUpdateMediaAffinityIntent's affinityType, an NSInteger enum.
                let kind: isize = unsafe { msg_send![intent, affinityType] };
                (read_search_of(intent), kind)
            };
            let asked = read(read_both);
            answer_later(completion, Fallback::Fail(AFFINITY), move |done| {
                let ok = asked.is_some_and(|(ask, kind)| {
                    wait_for_app().is_some_and(|app| affinity(&app, &ask, kind == AFFINITY_DISLIKE))
                });
                done.code(AFFINITY, ok);
            });
        }

        #[unsafe(method(resolveMediaItemsForAddMedia:withCompletion:))]
        fn resolveMediaItemsForAddMedia(&self, intent: &AnyObject, completion: &DynBlock<dyn Fn(*mut AnyObject)>) {
            let ask = read(|| read_search_of(intent));
            let class = c"INAddMediaMediaItemResolutionResult";
            answer_later(completion, Fallback::Unsupported { class, array: true }, move |done| {
                let song = ask.and_then(|ask| Some(song_for(&wait_for_app()?, &ask)?));
                done.send(match song {
                    Some((_, title)) => successes(class, &title, ITEM_SONG),
                    None => one_in_array(unsupported(class)),
                });
            });
        }

        #[unsafe(method(resolveMediaDestinationForAddMedia:withCompletion:))]
        fn resolveMediaDestinationForAddMedia(&self, intent: &AnyObject, completion: &DynBlock<dyn Fn(*mut AnyObject)>) {
            let to = read(|| read_destination(intent));
            let class = c"INAddMediaMediaDestinationResolutionResult";
            answer_later(completion, Fallback::Unsupported { class, array: false }, move |done| {
                let result = match to.flatten() {
                    // No playlist named: your library (a like).
                    None => not_required(class),
                    Some(name) => match wait_for_app().and_then(|app| playlist_named(&app, &name)) {
                        Some((_, title)) => playlist_destination(class, &title),
                        None => unsupported(class),
                    },
                };
                done.send(result);
            });
        }

        #[unsafe(method(handleAddMedia:completion:))]
        fn handleAddMedia_completion(&self, intent: &AnyObject, completion: &DynBlock<dyn Fn(*mut AnyObject)>) {
            let asked = read(|| (read_search_of(intent), read_destination(intent)));
            answer_later(completion, Fallback::Fail(ADD), move |done| {
                let ok = asked.is_some_and(|(ask, to)| wait_for_app().is_some_and(|app| add(&app, &ask, to.as_deref())));
                done.code(ADD, ok);
            });
        }
    }
);

/// Reads what Siri sent on the calling thread; a panic reads as nothing.
fn read<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// What a completion answers if it's dropped unanswered (a panic on the way).
#[derive(Clone, Copy)]
enum Fallback {
    /// A handle's failure response.
    Fail(Reply),
    /// A resolution: unsupported, in an array for media items.
    Unsupported { class: &'static CStr, array: bool },
}

/// Siri's completion block, copied so it can be called once from another
/// thread.
struct Completion {
    block: Option<block2::RcBlock<dyn Fn(*mut AnyObject)>>,
    fallback: Fallback,
}
// SAFETY: Intents' completion blocks may be called from any thread.
unsafe impl Send for Completion {}

impl Drop for Completion {
    fn drop(&mut self) {
        if let Some(block) = self.block.take() {
            let answer = match self.fallback {
                Fallback::Fail(r) => response(r, false),
                Fallback::Unsupported { class, array: true } => one_in_array(unsupported(class)),
                Fallback::Unsupported { class, array: false } => unsupported(class),
            };
            call(&block, answer.as_deref());
        }
    }
}

impl Completion {
    /// Answers with a response for `ok`.
    fn code(mut self, reply: Reply, ok: bool) {
        if let Some(block) = self.block.take() {
            call(&block, response(reply, ok).as_deref());
        }
    }

    /// Answers with `result` (a resolution result, or an array of them).
    fn send(mut self, result: Option<Retained<AnyObject>>) {
        if let Some(block) = self.block.take() {
            call(&block, result.as_deref());
        }
    }
}

/// Copies Siri's block and runs `work` off the calling thread (matching
/// reads the library, and the app may still be starting).
fn answer_later(
    completion: &DynBlock<dyn Fn(*mut AnyObject)>,
    fallback: Fallback,
    work: impl FnOnce(Completion) + Send + 'static,
) {
    let done = Completion { block: Some(completion.copy()), fallback };
    std::thread::spawn(move || {
        objc2::rc::autoreleasepool(|_| {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(done)));
        });
    });
}

fn call(block: &block2::RcBlock<dyn Fn(*mut AnyObject)>, obj: Option<&AnyObject>) {
    let ptr = obj.map_or(std::ptr::null_mut(), |o| o as *const AnyObject as *mut AnyObject);
    block.call((ptr,));
}

/// `[[class alloc] initWithCode:userActivity:nil]`.
fn response(reply: Reply, ok: bool) -> Option<Retained<AnyObject>> {
    let code = if ok { reply.success } else { reply.failure };
    // SAFETY: the response classes' designated initialiser.
    unsafe {
        let class = AnyClass::get(reply.class)?;
        let alloc: *mut AnyObject = msg_send![class, alloc];
        let r: *mut AnyObject = msg_send![alloc, initWithCode: code, userActivity: std::ptr::null::<AnyObject>()];
        Retained::from_raw(r)
    }
}

/// An `INMediaItem` for Siri to say back.
fn media_item(title: &str, kind: isize) -> Option<Retained<AnyObject>> {
    // SAFETY: INMediaItem's initialiser (iOS 13); strings are copied.
    unsafe {
        let class = AnyClass::get(c"INMediaItem")?;
        let alloc: *mut AnyObject = msg_send![class, alloc];
        let id = NSString::from_str("platter");
        let title = NSString::from_str(title);
        let r: *mut AnyObject = msg_send![
            alloc,
            initWithIdentifier: &*id,
            title: &*title,
            type: kind,
            artwork: std::ptr::null::<AnyObject>(),
            artist: std::ptr::null::<NSString>()
        ];
        Retained::from_raw(r)
    }
}

fn one_in_array(obj: Option<Retained<AnyObject>>) -> Option<Retained<AnyObject>> {
    let obj = obj?;
    // SAFETY: NSArray's arrayWithObject: with a live object.
    unsafe { msg_send![AnyClass::get(c"NSArray")?, arrayWithObject: &*obj] }
}

/// `[class successesWithResolvedMediaItems:@[item]]`.
fn successes(class: &CStr, title: &str, kind: isize) -> Option<Retained<AnyObject>> {
    let items = one_in_array(media_item(title, kind))?;
    // SAFETY: the media item resolution classes' class method.
    unsafe { msg_send![AnyClass::get(class)?, successesWithResolvedMediaItems: &*items] }
}

fn unsupported(class: &CStr) -> Option<Retained<AnyObject>> {
    // SAFETY: INIntentResolutionResult's class method, on a subclass.
    unsafe { msg_send![AnyClass::get(class)?, unsupported] }
}

fn not_required(class: &CStr) -> Option<Retained<AnyObject>> {
    // SAFETY: as above.
    unsafe { msg_send![AnyClass::get(class)?, notRequired] }
}

fn playlist_destination(class: &CStr, title: &str) -> Option<Retained<AnyObject>> {
    // SAFETY: INMediaDestination's class constructor and the resolution
    // class's success constructor.
    unsafe {
        let name = NSString::from_str(title);
        let to: Option<Retained<AnyObject>> = msg_send![AnyClass::get(c"INMediaDestination")?, playlistDestinationWithName: &*name];
        let to = to?;
        msg_send![AnyClass::get(class)?, successWithResolvedMediaDestination: &*to]
    }
}

fn item_type(p: &SiriPlay) -> isize {
    use crate::audio::source::ContextKind;
    if p.start_track_id.is_some() {
        return ITEM_SONG;
    }
    match p.context.kind {
        ContextKind::Album => ITEM_ALBUM,
        ContextKind::Artist => ITEM_ARTIST,
        ContextKind::Playlist | ContextKind::Liked | ContextKind::Mix => ITEM_PLAYLIST,
        ContextKind::Radio => ITEM_STATION,
        _ => ITEM_MUSIC,
    }
}

/// An NSString property, trimmed; None when missing or blank.
///
/// SAFETY: `obj` is null or an object answering `sel` with an NSString.
unsafe fn text(obj: *mut AnyObject, sel: Sel) -> Option<String> {
    let obj = unsafe { obj.as_ref() }?;
    let s: *mut NSString = unsafe { msg_send![obj, performSelector: sel] };
    unsafe { s.as_ref() }.map(|s| s.to_string()).filter(|s| !s.trim().is_empty())
}

/// An `NSArray<NSString>` property's strings.
///
/// SAFETY: `obj` answers `sel` with nil or an NSArray of NSStrings.
unsafe fn texts(obj: &AnyObject, sel: Sel) -> Vec<String> {
    let arr: *mut AnyObject = unsafe { msg_send![obj, performSelector: sel] };
    let Some(arr) = (unsafe { arr.as_ref() }) else { return Vec::new() };
    let n: usize = unsafe { msg_send![arr, count] };
    (0..n)
        .filter_map(|i| {
            let s: *mut NSString = unsafe { msg_send![arr, objectAtIndex: i] };
            unsafe { s.as_ref() }.map(|s| s.to_string())
        })
        .collect()
}

/// What an `INMediaSearch` says.
///
/// SAFETY: `search` is null or an INMediaSearch.
unsafe fn read_search(search: *mut AnyObject) -> SiriAsk {
    let mut ask = SiriAsk {
        name: unsafe { text(search, sel!(mediaName)) },
        artist: unsafe { text(search, sel!(artistName)) },
        album: unsafe { text(search, sel!(albumName)) },
        ..Default::default()
    };
    if let Some(s) = unsafe { search.as_ref() } {
        // SAFETY: INMediaSearch's documented NSInteger enums and arrays.
        unsafe {
            let t: isize = msg_send![s, mediaType];
            ask.kind = SiriKind::from_media_type(t);
            let order: isize = msg_send![s, sortOrder];
            ask.sort = SiriSort::from_order(order);
            // INMediaReference: currently playing 1.
            let r: isize = msg_send![s, reference];
            ask.this = r == 1;
            ask.genres = texts(s, sel!(genreNames));
            ask.moods = texts(s, sel!(moodNames));
        }
    }
    ask
}

/// What Siri heard, from an INPlayMediaIntent.
fn read_play(intent: &AnyObject) -> SiriAsk {
    // SAFETY: INPlayMediaIntent's documented properties: an INMediaSearch,
    // NSNumbers and NSInteger enums.
    unsafe {
        let search: *mut AnyObject = msg_send![intent, mediaSearch];
        let mut ask = read_search(search);
        let shuffled: *mut NSNumber = msg_send![intent, playShuffled];
        ask.shuffle = shuffled.as_ref().is_some_and(|n| n.boolValue());
        let resume: *mut NSNumber = msg_send![intent, resumePlayback];
        ask.resume = resume.as_ref().is_some_and(|n| n.boolValue());
        let repeat: isize = msg_send![intent, playbackRepeatMode];
        ask.repeat = siri::repeat_from(repeat);
        let at: isize = msg_send![intent, playbackQueueLocation];
        ask.queue = SiriQueue::from_location(at);
        ask
    }
}

/// The media search of a like or add request ("this song" when it has none).
fn read_search_of(intent: &AnyObject) -> SiriAsk {
    // SAFETY: both intents have an INMediaSearch `mediaSearch`.
    unsafe {
        let search: *mut AnyObject = msg_send![intent, mediaSearch];
        let mut ask = read_search(search);
        if search.is_null() {
            ask.this = true;
        }
        ask
    }
}

/// The playlist an add request names; None for your library.
fn read_destination(intent: &AnyObject) -> Option<String> {
    // SAFETY: INAddMediaIntent's INMediaDestination and its documented properties.
    unsafe {
        let to: *mut AnyObject = msg_send![intent, mediaDestination];
        let to_ref = to.as_ref()?;
        let kind: isize = msg_send![to_ref, mediaDestinationType];
        if kind != DESTINATION_PLAYLIST {
            return None;
        }
        text(to, sel!(playlistName))
    }
}

/// The app once it's set up; waits for it when Siri started Platter.
fn wait_for_app() -> Option<AppHandle<Wry>> {
    let until = Instant::now() + START_WAIT;
    loop {
        if let Some(app) = APP.get() {
            return Some(app.clone());
        }
        if Instant::now() >= until {
            tracing::warn!("Siri: Platter didn't start in time");
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// What a play request comes to.
#[derive(Clone)]
enum Decided {
    Play(SiriPlay),
    Resume,
    /// A song you don't have, to get.
    Get(TapInput),
}

/// The last play request worked out, so handling doesn't repeat the
/// resolving (and its catalogue lookup) a moment later.
static LAST: Mutex<Option<(SiriAsk, Instant, Decided)>> = Mutex::new(None);

fn decide_play(ask: SiriAsk) -> Option<(AppHandle<Wry>, Decided)> {
    let app = wait_for_app()?;
    if let Some((a, at, d)) = LAST.lock().clone()
        && a == ask
        && at.elapsed() < Duration::from_secs(30)
    {
        return Some((app, d));
    }
    let d = work_out(&app, &ask)?;
    *LAST.lock() = Some((ask, Instant::now(), d.clone()));
    Some((app, d))
}

/// The situation the request is heard in: what's playing, whether it can
/// carry on, and (with no signal) what's on the phone.
fn situation(app: &AppHandle<Wry>, db: &DbHandle) -> Situation {
    let state = app
        .try_state::<PlayerHandle>()
        .and_then(|p| tauri::async_runtime::block_on(p.inner().state()).ok());
    let playing = state
        .as_ref()
        .and_then(|s| s.track.as_ref())
        .map(|t| Playing { track_id: t.id.clone(), title: t.title.clone() });
    let can_resume = state
        .as_ref()
        .is_some_and(|s| s.track.is_some() && matches!(s.status, PlayerStatus::Paused | PlayerStatus::Idle));
    let offline = (super::ios::reach(app) == Reach::Unreachable).then(|| db.read(Kept::read).unwrap_or_default());
    Situation { playing, can_resume, offline }
}

fn work_out(app: &AppHandle<Wry>, ask: &SiriAsk) -> Option<Decided> {
    tracing::info!(kind = ?ask.kind, shuffle = ask.shuffle, queue = ?ask.queue, this = ask.this, "Siri asked to play");
    let db = app.try_state::<DbHandle>()?.inner().clone();
    let at = situation(app, &db);
    let offline = at.offline.is_some();
    let all = || {
        if offline { db.read(super::shuffle_downloads_context) } else { db.read(super::shuffle_library_context) }
    };
    match siri::resolve(&db, ask, &at, all) {
        Ok(Some(SiriAnswer::Play(p))) => return Some(Decided::Play(p)),
        Ok(Some(SiriAnswer::Resume)) => return Some(Decided::Resume),
        Ok(None) => {}
        Err(e) => {
            tracing::warn!("Siri: the library couldn't be searched: {e}");
            return None;
        }
    }
    if offline {
        tracing::info!("Siri: nothing on the phone fits, and there's no signal");
        return None;
    }
    // Music you don't have: looked up, to get through Lidarr.
    let query = siri::catalogue_query(ask)?;
    let discover = app.try_state::<crate::discover::Discover>()?.inner().clone();
    let look = async move { tokio::time::timeout(LOOKUP_WAIT, discover.search_part(&query, "songs")).await };
    match tauri::async_runtime::block_on(look) {
        Ok(Ok(part)) => {
            let found = siri::tap_for(ask, &part.songs).map(Decided::Get);
            if found.is_none() {
                tracing::info!("Siri: nothing in the library or the catalogue fits");
            }
            found
        }
        Ok(Err(e)) => {
            tracing::warn!("Siri: the catalogue couldn't be searched: {e}");
            None
        }
        Err(_) => {
            tracing::warn!("Siri: looking the song up took too long");
            None
        }
    }
}

/// Plays, queues, carries on or gets; false when it can't.
fn act_play(app: &AppHandle<Wry>, d: Decided) -> bool {
    *LAST.lock() = None;
    let Some(player) = app.try_state::<PlayerHandle>().map(|p| p.inner().clone()) else { return false };
    match d {
        Decided::Resume => {
            tauri::async_runtime::spawn(async move {
                if let Err(e) = player.resume().await {
                    tracing::warn!("Siri resume failed: {e}");
                }
            });
            true
        }
        Decided::Play(p) => {
            tauri::async_runtime::spawn(async move {
                let done = match (p.queue, p.start_track_id.clone()) {
                    (SiriQueue::Now, start) => player.play_context_from(p.context, start, None, Some(p.shuffle)).await,
                    (SiriQueue::Next, Some(id)) => player.queue_add(vec![id], AddAt::Next).await,
                    (SiriQueue::Later, Some(id)) => player.queue_add(vec![id], AddAt::End).await,
                    (SiriQueue::Next, None) => player.queue_context_at(p.context, AddAt::Next).await,
                    (SiriQueue::Later, None) => player.queue_context_at(p.context, AddAt::End).await,
                };
                if let Err(e) = done {
                    tracing::warn!("Siri play failed: {e}");
                    return;
                }
                if let (Some(r), SiriQueue::Now) = (p.repeat, p.queue)
                    && let Err(e) = player.set_repeat(r).await
                {
                    tracing::warn!("Siri repeat failed: {e}");
                }
            });
            true
        }
        Decided::Get(input) => {
            let Some(taps) = app.try_state::<crate::lidarr::tap::Taps>().map(|t| t.inner().clone()) else { return false };
            let get = async move {
                let out = taps.tap(input).await?;
                // Someone the server is shared with: the tap became a request to its owner.
                Ok(out.tap.is_some() || out.needs.as_ref().is_some_and(|n| n.kind == "requested"))
            };
            match tauri::async_runtime::block_on(siri::get_within(get, GET_WAIT)) {
                Getting::Started | Getting::StillGoing => {
                    tracing::info!("Siri: getting a song you don't have");
                    true
                }
                Getting::NothingFits | Getting::Failed => false,
            }
        }
    }
}

/// The song a like or add is about, and its title.
fn song_for(app: &AppHandle<Wry>, ask: &SiriAsk) -> Option<(String, String)> {
    let db = app.try_state::<DbHandle>()?.inner().clone();
    let playing = situation(app, &db).playing;
    match siri::song_for(&db, ask, playing.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Siri: the song couldn't be looked up: {e}");
            None
        }
    }
}

fn playlist_named(app: &AppHandle<Wry>, name: &str) -> Option<(String, String)> {
    let db = app.try_state::<DbHandle>()?.inner().clone();
    siri::playlist_named(&db, name).ok().flatten()
}

/// "I like this song" likes it. "I don't like this song" skips it if it's
/// playing and, with the setting on, hides it.
fn affinity(app: &AppHandle<Wry>, ask: &SiriAsk, dislike: bool) -> bool {
    let Some((id, title)) = song_for(app, ask) else {
        tracing::info!("Siri: no song to like");
        return false;
    };
    tracing::info!(dislike, "Siri: like or dislike");
    if !dislike {
        let Some(likes) = app.try_state::<crate::likes::Likes>().map(|l| l.inner().clone()) else { return false };
        return tauri::async_runtime::block_on(likes.set(vec![id], true))
            .inspect_err(|e| tracing::warn!("Siri like failed: {e}"))
            .is_ok();
    }
    let Some(db) = app.try_state::<DbHandle>().map(|d| d.inner().clone()) else { return false };
    if crate::settings::load(&db).siri_dislike_hides
        && let Some(hidden) = app.try_state::<crate::hidden::Hidden>()
    {
        let req = crate::hidden::HideRequest { kind: crate::hidden::HiddenKind::Track, id: id.clone(), title, subtitle: None, thumb: None };
        if let Err(e) = hidden.set(req, true) {
            tracing::warn!("Siri: the song couldn't be hidden: {e}");
            return false;
        }
    }
    let Some(player) = app.try_state::<PlayerHandle>().map(|p| p.inner().clone()) else { return true };
    tauri::async_runtime::block_on(async move {
        let playing = player.state().await.ok().and_then(|s| s.track).is_some_and(|t| t.id == id);
        if playing && let Err(e) = player.next().await {
            tracing::warn!("Siri: the song couldn't be skipped: {e}");
        }
    });
    true
}

/// "Add this to my library" likes it; "add this to my Road Trip playlist"
/// adds it at the end.
fn add(app: &AppHandle<Wry>, ask: &SiriAsk, to: Option<&str>) -> bool {
    let Some(name) = to else { return affinity(app, ask, false) };
    let (Some((track, _)), Some((playlist, _))) = (song_for(app, ask), playlist_named(app, name)) else {
        tracing::info!("Siri: no song or no playlist to add to");
        return false;
    };
    let Some(playlists) = app.try_state::<crate::playlists::Playlists>().map(|p| p.inner().clone()) else { return false };
    tauri::async_runtime::block_on(playlists.add(playlist, vec![track], crate::db::playlists::Place::End))
        .inspect_err(|e| tracing::warn!("Siri: the song couldn't be added: {e}"))
        .is_ok()
}

/// The handler object, made once.
fn handler() -> *mut AnyObject {
    struct Held(Retained<MediaHandler>);
    // SAFETY: the handler holds no state and is only messaged by Siri.
    unsafe impl Send for Held {}
    unsafe impl Sync for Held {}
    static HANDLER: OnceLock<Held> = OnceLock::new();
    let h = HANDLER.get_or_init(|| {
        let this = MediaHandler::alloc().set_ivars(());
        // SAFETY: NSObject's init on our own subclass.
        Held(unsafe { msg_send![super(this), init] })
    });
    Retained::as_ptr(&h.0) as *mut AnyObject
}

/// The intents the handler takes.
const INTENTS: [&CStr; 3] = [c"INPlayMediaIntent", c"INUpdateMediaAffinityIntent", c"INAddMediaIntent"];
/// The handling protocols it answers to, declared so Siri sees them.
const PROTOCOLS: [&CStr; 3] = [c"INPlayMediaIntentHandling", c"INUpdateMediaAffinityIntentHandling", c"INAddMediaIntentHandling"];

/// `-[AppDelegate application:handlerForIntent:]`: our handler for the
/// media requests, nothing for anything else. Siri may have just started
/// Platter with no screen: the app core is started here.
extern "C-unwind" fn handler_for_intent(
    _this: *mut AnyObject,
    _cmd: Sel,
    _app: *mut AnyObject,
    intent: *mut AnyObject,
) -> *mut AnyObject {
    // SAFETY: `intent` is an INIntent; the classes are looked up by name.
    let ours = unsafe {
        intent.as_ref().is_some_and(|i| {
            INTENTS.iter().filter_map(|n| AnyClass::get(n)).any(|c| {
                let yes: bool = msg_send![i, isKindOfClass: c];
                yes
            })
        })
    };
    if !ours {
        return std::ptr::null_mut();
    }
    start_core();
    handler()
}

/// Starts the app core when nothing has (a background start with no screen
/// connected). tao starts the app on its first scene: it is told one
/// connected, with stand-ins it only checks the kind of (no window scene,
/// so it attaches nothing). A real screen connecting later is handled as
/// for a car-first start. Main thread only; does nothing once started.
fn start_core() {
    if APP.get().is_some() || MainThreadMarker::new().is_none() {
        return;
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: main thread. UIApplication's connected scenes is a set;
        // tao's scene delegate holds no state, and its willConnect only asks
        // the scene's kind (and runs the app's start once).
        unsafe {
            let Some(ui_app) = AnyClass::get(c"UIApplication") else { return };
            let shared: *mut AnyObject = msg_send![ui_app, sharedApplication];
            let Some(shared) = shared.as_ref() else { return };
            let scenes: *mut AnyObject = msg_send![shared, connectedScenes];
            let count: usize = scenes.as_ref().map_or(0, |s| msg_send![s, count]);
            if count > 0 {
                // A screen or the car is connecting: tao starts the app from it.
                return;
            }
            let Some(tao) = AnyClass::get(c"TaoSceneDelegate") else {
                tracing::warn!("Siri: no TaoSceneDelegate to start the app with");
                return;
            };
            tracing::info!("Siri started Platter with no screen: starting the app core");
            let delegate: Retained<AnyObject> = msg_send![tao, new];
            let stand_in: Retained<NSObject> = NSObject::new();
            let _: () = msg_send![&*delegate, scene: &*stand_in, willConnectToSession: &*stand_in, options: &*stand_in];
        }
    });
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
/// caches what it answers to). Declares the handler's protocols, keeps the
/// original setter, and only then puts ours in its place; once.
pub fn install_early() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let class = MediaHandler::class();
        for name in PROTOCOLS {
            match AnyProtocol::get(name) {
                // SAFETY: adding a protocol the class implements the methods of.
                Some(p) => unsafe {
                    objc2::ffi::class_addProtocol(class as *const AnyClass as *mut AnyClass, p);
                },
                None => tracing::warn!("Siri: no {name:?} protocol"),
            }
        }
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

/// The app is up: requests can be handled.
pub fn attach(app: &AppHandle<Wry>) {
    let _ = APP.set(app.clone());
}
