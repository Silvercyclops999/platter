//! The car's screen: Apple's templates filled from [`super`]'s lists.
//!
//! CarPlay connects its own scene. Tauri's window layer hands every scene to
//! its own delegate, so [`install`] (run before the app starts) points the
//! CarPlay scene's configuration at [`CarDelegate`] instead. The lists need
//! the library, which exists once the app has set up; a car that connects
//! sooner sees "Loading" until [`attach`] fills the tabs.
//!
//! Now Playing gets shuffle, repeat and a heart, and its Up next button
//! lists the queue.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, ProtocolObject, Sel};
use objc2::{AnyThread, ClassType, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_car_play::{
    CPInterfaceController, CPListItem, CPListItemAccessoryType, CPListSection, CPListTemplate, CPListTemplateItem,
    CPNowPlayingButton, CPNowPlayingImageButton, CPNowPlayingRepeatButton, CPNowPlayingShuffleButton,
    CPNowPlayingTemplate, CPNowPlayingTemplateObserver, CPSelectableListItem, CPTabBarTemplate, CPTemplate,
    CPTemplateApplicationScene, CPTemplateApplicationSceneDelegate, CPTemplateApplicationSceneSessionRoleApplication,
};
use objc2_foundation::{NSArray, NSData, NSObject, NSObjectProtocol, NSString};
use objc2_ui_kit::UISceneDelegate;
use tauri::{AppHandle, Listener, Manager, Wry};

use super::{CarAction, CarItem, CarList, Reach, Tab};
use crate::audio::player::PlayerHandle;
use crate::audio::queue::Repeat;
use crate::backend::Backends;
use crate::backend::registry::BackendKind;
use crate::db::DbHandle;
use crate::db::artcache::ArtCache;
use crate::likes::Likes;
use crate::plex::PlexState;

std::arch::global_asm!(".linker_option \"-framework\", \"CarPlay\"");

static APP: OnceLock<AppHandle<Wry>> = OnceLock::new();

thread_local! {
    /// The car's interface while it is connected (main thread only).
    static CAR: RefCell<Option<Retained<CPInterfaceController>>> = const { RefCell::new(None) };
    /// What the tabs were last built for, so a change rebuilds them.
    static REACH: Cell<Option<Reach>> = const { Cell::new(None) };
    /// Now Playing's buttons and Up next observer, once set up.
    static NOW: RefCell<Option<NowButtons>> = const { RefCell::new(None) };
}

struct NowButtons {
    shuffle: Retained<CPNowPlayingShuffleButton>,
    repeat: Retained<CPNowPlayingRepeatButton>,
    like: Retained<CPNowPlayingImageButton>,
    /// Kept alive here: the template doesn't own its observers.
    _observer: Retained<UpNextObserver>,
}

/// What the buttons show: the playing song (and whether it's liked),
/// shuffle and repeat.
#[derive(Debug, Clone, Default, PartialEq)]
struct Shown {
    track: Option<String>,
    liked: bool,
    shuffle: bool,
    repeat: Repeat,
}

static SHOWN: parking_lot::Mutex<Option<Shown>> = parking_lot::const_mutex(None);

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "PlatterCarPlayDelegate"]
    #[thread_kind = MainThreadOnly]
    pub struct CarDelegate;

    unsafe impl NSObjectProtocol for CarDelegate {}
    unsafe impl UISceneDelegate for CarDelegate {}

    #[allow(non_snake_case)]
    unsafe impl CPTemplateApplicationSceneDelegate for CarDelegate {
        #[unsafe(method(templateApplicationScene:didConnectInterfaceController:))]
        fn templateApplicationScene_didConnectInterfaceController(
            &self,
            _scene: &CPTemplateApplicationScene,
            controller: &CPInterfaceController,
        ) {
            tracing::info!("CarPlay connected");
            CAR.with(|c| *c.borrow_mut() = Some(controller.retain()));
            show_tabs();
        }

        #[unsafe(method(templateApplicationScene:didDisconnectInterfaceController:))]
        fn templateApplicationScene_didDisconnectInterfaceController(
            &self,
            _scene: &CPTemplateApplicationScene,
            _controller: &CPInterfaceController,
        ) {
            tracing::info!("CarPlay disconnected");
            CAR.with(|c| *c.borrow_mut() = None);
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "PlatterCarPlayUpNext"]
    #[thread_kind = MainThreadOnly]
    struct UpNextObserver;

    unsafe impl NSObjectProtocol for UpNextObserver {}

    #[allow(non_snake_case)]
    unsafe impl CPNowPlayingTemplateObserver for UpNextObserver {
        #[unsafe(method(nowPlayingTemplateUpNextButtonTapped:))]
        fn nowPlayingTemplateUpNextButtonTapped(&self, _template: &CPNowPlayingTemplate) {
            show_up_next();
        }
    }
);

/// The original `-[UISceneConfiguration setDelegateClass:]`.
static SET_DELEGATE_CLASS: OnceLock<Imp> = OnceLock::new();

/// `setDelegateClass:` that keeps CarPlay scenes on [`CarDelegate`].
extern "C-unwind" fn set_delegate_class(this: *mut AnyObject, cmd: Sel, class: *const AnyClass) {
    // SAFETY: `this` is a UISceneConfiguration; its role is an NSString.
    let class = unsafe {
        let role: *mut NSString = msg_send![this, role];
        let car = !role.is_null() && (*role).isEqualToString(CPTemplateApplicationSceneSessionRoleApplication);
        if car { CarDelegate::class() as *const AnyClass } else { class }
    };
    if let Some(original) = SET_DELEGATE_CLASS.get() {
        // SAFETY: the original has this exact signature.
        unsafe {
            let f: extern "C-unwind" fn(*mut AnyObject, Sel, *const AnyClass) = std::mem::transmute(*original);
            f(this, cmd, class);
        }
    }
}

/// Runs before the app starts, so a car that is already connected (or
/// launches Platter) is routed here from the first scene.
pub fn install() {
    let _ = CarDelegate::class();
    let Some(config) = AnyClass::get(c"UISceneConfiguration") else { return };
    // SAFETY: swaps one setter for a function of the same shape that calls
    // the original; done once, before any scene connects.
    unsafe {
        let ours: Imp = std::mem::transmute(
            set_delegate_class as extern "C-unwind" fn(*mut AnyObject, Sel, *const AnyClass),
        );
        if let Some(original) = objc2::ffi::class_replaceMethod(
            config as *const AnyClass as *mut AnyClass,
            sel!(setDelegateClass:),
            ours,
            c"v@:#".as_ptr(),
        ) {
            let _ = SET_DELEGATE_CLASS.set(original);
        }
    }
}

/// The app is set up: the lists can be read. A car already connected gets
/// its tabs filled.
pub fn attach(app: &AppHandle<Wry>) {
    let _ = APP.set(app.clone());
    let _ = app.run_on_main_thread(show_tabs);
    // Signing in or out, or the server coming and going, changes what the
    // tabs can show.
    for event in [crate::plex::types::EVENT_AUTH_CHANGED, crate::plex::types::EVENT_HEALTH_CHANGED] {
        let a = app.clone();
        app.listen_any(event, move |_| {
            let _ = a.run_on_main_thread(|| {
                if APP.get().map(reach) != REACH.with(Cell::get) {
                    show_tabs();
                }
            });
        });
    }
    let a = app.clone();
    app.listen_any(crate::audio::events::EV_STATE, move |e| {
        let Ok(s) = serde_json::from_str::<Brief>(e.payload()) else { return };
        playing_changed(&a, s);
    });
    let a = app.clone();
    app.listen_any(crate::likes::EVENT_LIKES_CHANGED, move |_| {
        let track = SHOWN.lock().as_ref().and_then(|s| s.track.clone());
        refresh_like(&a, track);
    });
}

/// The parts of the player's state the buttons show.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Brief {
    track: Option<BriefTrack>,
    shuffle: bool,
    repeat: Repeat,
}

#[derive(serde::Deserialize)]
struct BriefTrack {
    id: String,
}

fn playing_changed(app: &AppHandle<Wry>, s: Brief) {
    let track = s.track.map(|t| t.id);
    let new_track = {
        let mut shown = SHOWN.lock();
        let cur = shown.get_or_insert_with(Shown::default);
        let new_track = cur.track != track;
        if !new_track && cur.shuffle == s.shuffle && cur.repeat == s.repeat {
            return;
        }
        cur.shuffle = s.shuffle;
        cur.repeat = s.repeat;
        if new_track {
            cur.track = track.clone();
            cur.liked = false;
        }
        new_track
    };
    let _ = app.run_on_main_thread(show_buttons);
    if new_track {
        refresh_like(app, track);
    }
}

/// Looks up whether `track` is liked and shows it on the heart.
fn refresh_like(app: &AppHandle<Wry>, track: Option<String>) {
    let Some(track) = track else { return };
    let Some(likes) = app.try_state::<Likes>().map(|l| l.inner().clone()) else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let liked = likes.is_liked(&track).await.unwrap_or(false);
        if let Some(s) = SHOWN.lock().as_mut().filter(|s| s.track.as_deref() == Some(track.as_str())) {
            s.liked = liked;
        }
        let _ = app.run_on_main_thread(show_buttons);
    });
}

/// Whether the library can be shown: signed in, and the server in reach.
fn reach(app: &AppHandle<Wry>) -> Reach {
    let Some(plex) = app.try_state::<PlexState>() else { return Reach::Online };
    let others = app
        .try_state::<Backends>()
        .is_some_and(|b| b.records().servers.iter().any(|r| r.kind != BackendKind::Plex));
    super::reach(&plex.health(), others)
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

/// Builds and shows the tab bar (main thread).
fn show_tabs() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(controller) = CAR.with(|c| c.borrow().clone()) else { return };
    let read = |name: &str, f: &dyn Fn(&rusqlite::Connection) -> crate::error::AppResult<Vec<CarItem>>| match APP.get() {
        Some(app) => app.state::<DbHandle>().read(|c| f(c)).unwrap_or_else(|e| {
            tracing::warn!("CarPlay {name} not read: {e}");
            Vec::new()
        }),
        None => Vec::new(),
    };
    let reach = APP.get().map(reach).unwrap_or(Reach::Online);
    REACH.with(|r| r.set(APP.get().map(|_| reach)));
    let home = super::tab_rows(Tab::Home, reach, || read("home", &super::home));
    let downloaded = super::tab_rows(Tab::Downloaded, reach, || read("downloads", &super::downloaded));
    let radio = super::tab_rows(Tab::Radio, reach, || read("radio", &|c| super::radio(c, None)));
    let library = if APP.get().is_some() { super::tab_rows(Tab::Library, reach, super::library) } else { Vec::new() };
    let loading = APP.get().is_none();
    let tabs = [
        ("Home", "house", home),
        ("Library", "books.vertical", library),
        ("Downloaded", "arrow.down.circle", downloaded),
        ("Radio", "dot.radiowaves.left.and.right", radio),
    ]
        .into_iter()
        .map(|(title, symbol, items)| {
            let t = list_template(mtm, title, &items, loading);
            // SAFETY: plain setters on a template we own.
            unsafe {
                t.setTabTitle(Some(&ns(title)));
                t.setTabImage(symbol_image(symbol).as_deref());
            }
            Retained::into_super(t)
        })
        .collect::<Vec<Retained<CPTemplate>>>();
    // SAFETY: CarPlay calls on the main thread with templates we built.
    unsafe {
        let bar = CPTabBarTemplate::initWithTemplates(CPTabBarTemplate::alloc(mtm), &NSArray::from_retained_slice(&tabs));
        controller.setRootTemplate_animated_completion(&bar, false, None);
    }
    set_up_now_playing(mtm);
}

/// Gives the shared Now Playing its buttons (shuffle, repeat, a heart) and
/// the Up next button, once.
fn set_up_now_playing(mtm: MainThreadMarker) {
    if NOW.with(|n| n.borrow().is_some()) {
        return;
    }
    let shuffle_tapped = RcBlock::new(|_b: NonNull<CPNowPlayingButton>| {
        let on = !SHOWN.lock().as_ref().is_some_and(|s| s.shuffle);
        with_player(move |p| async move { p.set_shuffle(on).await });
    });
    let repeat_tapped = RcBlock::new(|_b: NonNull<CPNowPlayingButton>| {
        let r = super::next_repeat(SHOWN.lock().as_ref().map(|s| s.repeat).unwrap_or_default());
        with_player(move |p| async move { p.set_repeat(r).await });
    });
    let like_tapped = RcBlock::new(|_b: NonNull<CPNowPlayingButton>| toggle_like());
    let Some(heart) = symbol_image("heart.fill") else { return };
    let observer: Retained<UpNextObserver> = {
        let this = UpNextObserver::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's init on our own subclass.
        unsafe { msg_send![super(this), init] }
    };
    // SAFETY: the system buttons and the shared template, set up on the main
    // thread; CarPlay copies the handlers.
    unsafe {
        let shuffle = CPNowPlayingShuffleButton::initWithHandler(CPNowPlayingShuffleButton::alloc(mtm), Some(&shuffle_tapped));
        let repeat = CPNowPlayingRepeatButton::initWithHandler(CPNowPlayingRepeatButton::alloc(mtm), Some(&repeat_tapped));
        let like = CPNowPlayingImageButton::initWithImage_handler(CPNowPlayingImageButton::alloc(mtm), &heart, Some(&like_tapped));
        let np = CPNowPlayingTemplate::sharedTemplate(mtm);
        let buttons: [Retained<CPNowPlayingButton>; 3] = [
            Retained::into_super(shuffle.clone()),
            Retained::into_super(repeat.clone()),
            Retained::into_super(like.clone()),
        ];
        np.updateNowPlayingButtons(&NSArray::from_retained_slice(&buttons));
        np.setUpNextButtonEnabled(true);
        np.addObserver(ProtocolObject::from_ref(&*observer));
        NOW.with(|n| *n.borrow_mut() = Some(NowButtons { shuffle, repeat, like, _observer: observer }));
    }
    show_buttons();
}

/// Shows shuffle, repeat and the heart as they stand (main thread).
fn show_buttons() {
    let shown = SHOWN.lock().clone().unwrap_or_default();
    NOW.with(|n| {
        let Some(b) = &*n.borrow() else { return };
        // SAFETY: setters on our own buttons, on the main thread.
        unsafe {
            b.shuffle.setSelected(shown.shuffle);
            b.repeat.setSelected(shown.repeat != Repeat::Off);
            b.like.setEnabled(shown.track.is_some());
            b.like.setSelected(shown.liked);
        }
    });
}

/// Runs a player call in the background, logging a failure.
fn with_player<F, Fut>(f: F)
where
    F: FnOnce(PlayerHandle) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = crate::error::AppResult<()>> + Send + 'static,
{
    let Some(app) = APP.get() else { return };
    let player = app.state::<PlayerHandle>().inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = f(player).await {
            tracing::warn!("CarPlay button failed: {e}");
        }
    });
}

/// The heart: likes the playing song, or takes the like back.
fn toggle_like() {
    let Some(app) = APP.get() else { return };
    let Some(likes) = app.try_state::<Likes>().map(|l| l.inner().clone()) else { return };
    let Some((track, liked)) = SHOWN.lock().as_ref().and_then(|s| Some((s.track.clone()?, s.liked))) else { return };
    // Shown at once; `likes://changed` confirms it.
    if let Some(s) = SHOWN.lock().as_mut() {
        s.liked = !liked;
    }
    show_buttons();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = likes.set(vec![track], !liked).await {
            tracing::warn!("CarPlay like failed: {e}");
        }
    });
}

/// Up next: the queue as a list; picking a song plays it.
fn show_up_next() {
    let Some(app) = APP.get() else { return };
    let player = app.state::<PlayerHandle>().inner().clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let items = match player.state().await {
            Ok(s) => super::up_next(&s.queue),
            Err(e) => {
                tracing::warn!("CarPlay queue not read: {e}");
                Vec::new()
            }
        };
        let _ = app.run_on_main_thread(move || {
            let Some(controller) = CAR.with(|c| c.borrow().clone()) else { return };
            let Some(mtm) = MainThreadMarker::new() else { return };
            let t = list_template(mtm, "Up next", &items, false);
            // SAFETY: main thread, our template.
            unsafe { controller.pushTemplate_animated_completion(&t, true, None) };
        });
    });
}

/// An SF Symbol, for tab icons.
fn symbol_image(name: &str) -> Option<Retained<objc2_ui_kit::UIImage>> {
    // A class method that returns nil for an unknown name.
    objc2_ui_kit::UIImage::systemImageNamed(&ns(name))
}

fn list_template(mtm: MainThreadMarker, title: &str, items: &[CarItem], loading: bool) -> Retained<CPListTemplate> {
    let rows: Vec<Retained<ProtocolObject<dyn CPListTemplateItem>>> = if loading {
        vec![ProtocolObject::from_retained(text_row(mtm, "Loading…", None))]
    } else {
        items.iter().map(|i| ProtocolObject::from_retained(row(mtm, i))).collect()
    };
    // SAFETY: a section and a template made from rows we own.
    unsafe {
        let section =
            CPListSection::initWithItems(CPListSection::alloc(), &NSArray::from_retained_slice(&rows));
        let t = CPListTemplate::initWithTitle_sections(
            CPListTemplate::alloc(mtm),
            Some(&ns(title)),
            &NSArray::from_retained_slice(&[section]),
        );
        if !loading && items.is_empty() {
            t.setEmptyViewTitleVariants(&NSArray::from_retained_slice(&[ns("Nothing here yet")]));
        }
        t
    }
}

fn text_row(mtm: MainThreadMarker, text: &str, detail: Option<&str>) -> Retained<CPListItem> {
    // SAFETY: the designated initialiser with our strings.
    unsafe {
        CPListItem::initWithText_detailText_image(
            CPListItem::alloc(mtm),
            Some(&ns(text)),
            detail.map(ns).as_deref(),
            None,
        )
    }
}

fn row(mtm: MainThreadMarker, item: &CarItem) -> Retained<CPListItem> {
    let r = text_row(mtm, &item.title, item.subtitle.as_deref());
    if let (Some(thumb), Some(app)) = (&item.thumb, APP.get()) {
        load_art(app, thumb.clone(), OnMain(r.clone()));
    }
    let action = item.action.clone();
    let opens = matches!(action, CarAction::Open(_));
    if action == CarAction::Info {
        return r;
    }
    let handler = RcBlock::new(
        move |_item: NonNull<ProtocolObject<dyn CPSelectableListItem>>, done: *mut block2::DynBlock<dyn Fn()>| {
            picked(&action);
            if !done.is_null() {
                // SAFETY: CarPlay's completion block, called once.
                unsafe { (*done).call(()) };
            }
        },
    );
    // SAFETY: setters on our own row; the handler is copied by CarPlay.
    unsafe {
        if opens {
            r.setAccessoryType(CPListItemAccessoryType::DisclosureIndicator);
        }
        r.setHandler(Some(&handler));
    }
    r
}

/// Fetches a row's cover through the art cache and sets it when it comes.
/// A row handed to a background task and back; it is only touched again on
/// the main thread.
struct OnMain(Retained<CPListItem>);
// SAFETY: the row is created on the main thread and only used there again
// (inside `run_on_main_thread`); the task in between just carries it.
unsafe impl Send for OnMain {}

impl OnMain {
    fn into_row(self) -> Retained<CPListItem> {
        self.0
    }
}

fn load_art(app: &AppHandle<Wry>, thumb: String, row: OnMain) {
    let art = app.state::<ArtCache>().inner().clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(img) = art.get_or_fetch(&thumb, ART_PX, ART_PX).await else { return };
        let _ = app.run_on_main_thread(move || {
            let row = row.into_row();
            // SAFETY: UIImage from bytes (nil for bytes it can't read), set on
            // the main thread.
            unsafe {
                let data = NSData::with_bytes(&img.bytes);
                if let Some(image) = objc2_ui_kit::UIImage::imageWithData(&data) {
                    row.setImage(Some(&image));
                }
            }
        });
    });
}

/// Row covers: CarPlay draws them small.
const ART_PX: u32 = 120;

fn picked(action: &CarAction) {
    let Some(app) = APP.get() else { return };
    match action {
        CarAction::Open(list) => open_list(app, list),
        CarAction::Jump(item_id) => {
            let item_id = item_id.clone();
            with_player(move |p| async move { p.queue_jump(item_id).await });
            // Back to Now Playing, which Up next was opened from.
            if let Some(controller) = CAR.with(|c| c.borrow().clone()) {
                // SAFETY: main thread (list handlers run there).
                unsafe { controller.popTemplateAnimated_completion(true, None) };
            }
        }
        CarAction::Info => {}
        CarAction::Play(ctx) => {
            let player = app.state::<PlayerHandle>().inner().clone();
            let ctx = ctx.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = player.play_context_from(ctx, None, None, None).await {
                    tracing::warn!("CarPlay play failed: {e}");
                }
            });
            show_now_playing();
        }
    }
}

fn open_list(app: &AppHandle<Wry>, which: &CarList) {
    let db = app.state::<DbHandle>();
    let items = db.read(|c| super::list(c, which)).unwrap_or_else(|e| {
        tracing::warn!("CarPlay list not read: {e}");
        Vec::new()
    });
    let Some(controller) = CAR.with(|c| c.borrow().clone()) else { return };
    let Some(mtm) = MainThreadMarker::new() else { return };
    let t = list_template(mtm, which.title(), &items, false);
    // SAFETY: main thread (list handlers run there), our template.
    unsafe { controller.pushTemplate_animated_completion(&t, true, None) };
}

fn show_now_playing() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(controller) = CAR.with(|c| c.borrow().clone()) else { return };
    // SAFETY: the shared Now Playing template, pushed on the main thread.
    unsafe {
        let np = CPNowPlayingTemplate::sharedTemplate(mtm);
        let np: Retained<CPTemplate> = Retained::into_super(np);
        controller.pushTemplate_animated_completion(&np, true, None);
    }
}
