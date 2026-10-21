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
    CPInterfaceController, CPListImageRowItem, CPListImageRowItemRowElement, CPListItem, CPListItemAccessoryType, CPListSection, CPListTemplate, CPListTemplateItem,
    CPNowPlayingButton, CPNowPlayingImageButton, CPNowPlayingRepeatButton, CPNowPlayingShuffleButton,
    CPNowPlayingTemplate, CPNowPlayingTemplateObserver, CPSelectableListItem, CPTabBarTemplate, CPTemplate,
    CPTemplateApplicationScene, CPTemplateApplicationSceneDelegate, CPTemplateApplicationSceneSessionRoleApplication,
};
use objc2_foundation::{NSArray, NSData, NSObject, NSObjectProtocol, NSString};
use objc2_ui_kit::{UIScene, UISceneConnectionOptions, UISceneDelegate, UISceneSession};
use tauri::{AppHandle, Listener, Manager, Wry};

use super::{CarAction, CarItem, CarList, CarRow, CarSection, Connect, ExplicitRows, Kept, NowKey, OnConnect, Reach, Tab};
use crate::audio::player::PlayerHandle;
use crate::audio::queue::Repeat;
use crate::backend::Backends;
use crate::backend::registry::BackendKind;
use crate::db::DbHandle;
use crate::db::artcache::ArtCache;
use crate::likes::Likes;
use crate::plex::PlexState;
use crate::settings::SettingsState;

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

/// When the car last left during this run, on both clocks, to tell a blink
/// from a new drive ([`super::since_left`]).
static LAST_LEFT: parking_lot::Mutex<Option<(std::time::Instant, std::time::SystemTime)>> = parking_lot::const_mutex(None);
/// Resume was looked at for this connection: didConnect and attach can both
/// ask, and only the first counts.
static RESUMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "PlatterCarPlayDelegate"]
    #[thread_kind = MainThreadOnly]
    pub struct CarDelegate;

    unsafe impl NSObjectProtocol for CarDelegate {}
    #[allow(non_snake_case)]
    unsafe impl UISceneDelegate for CarDelegate {
        // A car can launch Platter with no phone screen at all. tao starts
        // the app (Tauri's setup, so the library and the player) only when
        // the first scene connects, and it counts the scene through its own
        // delegate; this scene is ours, so it's handed on, or the car would
        // sit on "Loading…" until something brought the phone's window up.
        #[unsafe(method(scene:willConnectToSession:options:))]
        fn scene_willConnectToSession_options(
            &self,
            scene: &UIScene,
            session: &UISceneSession,
            options: &UISceneConnectionOptions,
        ) {
            start_app_from_car(scene, session, options);
        }
    }

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
            resume_on_connect();
        }

        #[unsafe(method(templateApplicationScene:didDisconnectInterfaceController:))]
        fn templateApplicationScene_didDisconnectInterfaceController(
            &self,
            _scene: &CPTemplateApplicationScene,
            _controller: &CPInterfaceController,
        ) {
            tracing::info!("CarPlay disconnected");
            CAR.with(|c| *c.borrow_mut() = None);
            *LAST_LEFT.lock() = Some((std::time::Instant::now(), std::time::SystemTime::now()));
            RESUMED.store(false, std::sync::atomic::Ordering::SeqCst);
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

/// Lets tao see the car's scene connect, so the app starts up (tao starts
/// it on the first scene of any kind and attaches windows only to window
/// scenes). Once started, it does nothing.
fn start_app_from_car(scene: &UIScene, session: &UISceneSession, options: &UISceneConnectionOptions) {
    let Some(tao) = AnyClass::get(c"TaoSceneDelegate") else {
        tracing::warn!("CarPlay: no TaoSceneDelegate to start the app with");
        return;
    };
    // SAFETY: tao's scene delegate holds no state; its willConnect takes
    // any UIScene and only does window work for window scenes. Main thread.
    unsafe {
        let delegate: Retained<AnyObject> = msg_send![tao, new];
        let _: () = msg_send![&*delegate, scene: scene, willConnectToSession: session, options: options];
    }
}

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
    super::siri_ios::install_early();
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
    super::siri_ios::attach(app);
    // A car that connected before the app was set up couldn't resume yet.
    let _ = app.run_on_main_thread(|| {
        show_tabs();
        if CAR.with(|c| c.borrow().is_some()) {
            resume_on_connect();
        }
    });
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

/// The parts of the player's state the buttons and the playing marks show.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Brief {
    track: Option<BriefTrack>,
    shuffle: bool,
    repeat: Repeat,
    #[serde(default)]
    queue: BriefQueue,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BriefTrack {
    id: String,
    #[serde(default)]
    album_id: String,
}

#[derive(serde::Deserialize, Default)]
struct BriefQueue {
    #[serde(default)]
    context: Option<BriefContext>,
}

#[derive(serde::Deserialize)]
struct BriefContext {
    kind: crate::audio::source::ContextKind,
    id: Option<String>,
}

/// Rows on the car's screen that could be playing, to mark as it changes.
/// Capped: lists opened long ago drop off the front.
const MARKED_MAX: usize = 800;

thread_local! {
    static MARKABLE: RefCell<std::collections::VecDeque<(Retained<CPListItem>, CarItem)>> =
        const { RefCell::new(std::collections::VecDeque::new()) };
    static NOW_KEY: RefCell<NowKey> = RefCell::new(NowKey::default());
    /// Rows to give the explicit mark as the next list is built (read off
    /// the main thread with it; rows already shown keep theirs).
    static EXPLICIT: RefCell<ExplicitRows> = RefCell::new(ExplicitRows::default());
}

/// Marks every row on screen that plays what's playing now (main thread).
fn mark_rows() {
    NOW_KEY.with(|k| {
        let now = k.borrow();
        MARKABLE.with(|m| {
            for (row, item) in m.borrow().iter() {
                // SAFETY: a setter on our own row, on the main thread.
                unsafe { row.setPlaying(super::is_playing(item, &now)) };
            }
        });
    });
}

fn playing_changed(app: &AppHandle<Wry>, s: Brief) {
    let key = NowKey {
        context: s.queue.context.and_then(|c| Some((c.kind, c.id?))),
        album_id: s.track.as_ref().map(|t| t.album_id.clone()).filter(|a| !a.is_empty()),
        track_id: s.track.as_ref().map(|t| t.id.clone()),
    };
    let _ = app.run_on_main_thread(move || {
        let changed = NOW_KEY.with(|k| {
            let mut k = k.borrow_mut();
            let changed = *k != key;
            *k = key;
            changed
        });
        if changed {
            mark_rows();
        }
    });
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

/// What the tabs show, read off the main thread.
struct TabData {
    reach: Reach,
    /// The app isn't set up yet: every tab says "Loading".
    loading: bool,
    home: Vec<CarSection>,
    library: Vec<CarItem>,
    downloaded: Vec<CarItem>,
    radio: Vec<CarItem>,
}

fn read_tabs(app: Option<&AppHandle<Wry>>) -> TabData {
    let Some(app) = app else {
        return TabData {
            reach: Reach::Online,
            loading: true,
            home: Vec::new(),
            library: Vec::new(),
            downloaded: Vec::new(),
            radio: Vec::new(),
        };
    };
    let db = app.state::<DbHandle>();
    let read = |name: &str, f: &dyn Fn(&rusqlite::Connection) -> crate::error::AppResult<Vec<CarItem>>| {
        db.read(|c| f(c)).unwrap_or_else(|e| {
            tracing::warn!("CarPlay {name} not read: {e}");
            Vec::new()
        })
    };
    let reach = reach(app);
    let sections = || {
        db.read(super::home_sections).unwrap_or_else(|e| {
            tracing::warn!("CarPlay home not read: {e}");
            Vec::new()
        })
    };
    let message = |tab| {
        vec![CarSection {
            header: None,
            rows: super::tab_rows(tab, reach, Vec::new).into_iter().map(CarRow::Item).collect(),
        }]
    };
    // Out of reach, the lists keep what plays from the phone (and, with the
    // setting off, say what needs a connection).
    if reach == Reach::Unreachable {
        let kept = db.read(Kept::read).unwrap_or_default();
        let only = offline_only(app);
        let home = super::offline_sections(sections(), &kept, only);
        return TabData {
            reach,
            loading: false,
            home: if home.is_empty() { message(Tab::Home) } else { home },
            library: super::offline_tab(super::library(), &kept, only),
            downloaded: read("downloads", &super::downloaded),
            radio: super::offline_tab(read("radio", &|c| super::radio(c, None)), &kept, only),
        };
    }
    // Home keeps its sections and its row of covers; a message (signed
    // out) is a single plain section.
    let home = match reach {
        Reach::Online => sections(),
        _ => message(Tab::Home),
    };
    TabData {
        reach,
        loading: false,
        home,
        library: super::tab_rows(Tab::Library, reach, super::library),
        downloaded: super::tab_rows(Tab::Downloaded, reach, || read("downloads", &super::downloaded)),
        radio: super::tab_rows(Tab::Radio, reach, || read("radio", &|c| super::radio(c, None))),
    }
}

/// "Show downloads only when offline" (blocking: off the main thread only).
fn offline_only(app: &AppHandle<Wry>) -> bool {
    let Some(s) = app.try_state::<SettingsState>() else { return true };
    tauri::async_runtime::block_on(s.get()).map(|s| s.carplay_offline_downloads).unwrap_or(true)
}

/// Plays on where you left off as the car connects, if the setting says
/// so ([`super::on_connect`] decides), once per connection. Main thread;
/// the player is asked off it.
fn resume_on_connect() {
    let Some(app) = APP.get().cloned() else { return };
    if RESUMED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let since_disconnect_s = LAST_LEFT
        .lock()
        .map(|(mono, wall)| super::since_left(mono.elapsed().as_secs(), wall.elapsed().ok().map(|d| d.as_secs())));
    tauri::async_runtime::spawn(async move {
        // How things stand when the player is asked, not when the car came.
        let reach = reach(&app);
        let Some(settings) = app.try_state::<SettingsState>() else { return };
        let resume_on = settings.get().await.map(|s| s.carplay_resume).unwrap_or(true);
        let player = app.state::<PlayerHandle>().inner().clone();
        let Ok(state) = player.state().await else { return };
        let track = match state.track {
            Some(t) => {
                let id = t.id.clone();
                let kept = app
                    .state::<DbHandle>()
                    .read_async(move |c| Ok(crate::downloads::store::file(c, &id)?.is_some_and(|f| f.state == "done")))
                    .await
                    .unwrap_or(false);
                Some((t.id, kept))
            }
            None => None,
        };
        let c = Connect { resume_on, reach, status: state.status, track, since_disconnect_s };
        if super::on_connect(&c) == OnConnect::Resume {
            tracing::info!("CarPlay: playing on where it left off");
            if let Err(e) = player.resume().await {
                tracing::warn!("CarPlay resume failed: {e}");
            }
        }
    });
}

/// Each tabs rebuild asked for; an older read that lands after a newer one
/// is dropped.
static TABS_ASKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Reads the tabs off the main thread, then builds and shows them there,
/// so a slow read never holds up the car's screen.
fn show_tabs() {
    use std::sync::atomic::Ordering;
    if CAR.with(|c| c.borrow().is_none()) {
        return;
    }
    let asked = TABS_ASKED.fetch_add(1, Ordering::SeqCst) + 1;
    let Some(app) = APP.get().cloned() else {
        build_tabs(read_tabs(None), asked);
        return;
    };
    tauri::async_runtime::spawn_blocking(move || {
        let data = read_tabs(Some(&app));
        let marks = tab_marks(&app, &data);
        let _ = app.run_on_main_thread(move || {
            EXPLICIT.with(|e| *e.borrow_mut() = marks);
            build_tabs(data, asked)
        });
    });
}

/// The explicit marks for every tab's rows (blocking: off the main thread).
fn tab_marks(app: &AppHandle<Wry>, data: &TabData) -> ExplicitRows {
    let db = app.state::<DbHandle>();
    db.read(|c| {
        let mut m = super::explicit_sections(c, &data.home)?;
        m.extend(super::explicit_rows(c, data.library.iter().chain(&data.downloaded).chain(&data.radio))?);
        Ok(m)
    })
    .unwrap_or_else(|e| {
        tracing::debug!("CarPlay explicit marks not read: {e}");
        ExplicitRows::default()
    })
}

/// Builds and shows the tab bar (main thread), unless a newer rebuild was
/// asked for meanwhile.
fn build_tabs(data: TabData, asked: u64) {
    if TABS_ASKED.load(std::sync::atomic::Ordering::SeqCst) != asked {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(controller) = CAR.with(|c| c.borrow().clone()) else { return };
    REACH.with(|r| r.set((!data.loading).then_some(data.reach)));
    // The old tabs' rows are gone with them.
    COVER_ROWS.with(|m| m.borrow_mut().clear());
    MARKABLE.with(|m| m.borrow_mut().clear());
    let home = sections_template(mtm, "Home", &data.home, data.loading);
    // SAFETY: plain setters on a template we own.
    unsafe {
        home.setTabTitle(Some(&ns("Home")));
        home.setTabImage(symbol_image("house").as_deref());
    }
    let lists = [
        ("Library", "books.vertical", data.library),
        ("Downloaded", "arrow.down.circle", data.downloaded),
        ("Radio", "dot.radiowaves.left.and.right", data.radio),
    ];
    let mut tabs: Vec<Retained<CPTemplate>> = vec![Retained::into_super(home)];
    for (title, symbol, items) in lists {
        let t = list_template(mtm, title, &items, data.loading);
        // SAFETY: plain setters on a template we own.
        unsafe {
            t.setTabTitle(Some(&ns(title)));
            t.setTabImage(symbol_image(symbol).as_deref());
        }
        tabs.push(Retained::into_super(t));
    }
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
        let (items, marks) = match player.state().await {
            Ok(s) => {
                let items = super::up_next(&s.queue);
                let db = app.state::<DbHandle>().inner().clone();
                let marks = db.read_async(move |c| super::explicit_queue(c, &s.queue)).await.unwrap_or_default();
                (items, marks)
            }
            Err(e) => {
                tracing::warn!("CarPlay queue not read: {e}");
                (Vec::new(), ExplicitRows::default())
            }
        };
        let _ = app.run_on_main_thread(move || {
            EXPLICIT.with(|e| *e.borrow_mut() = marks);
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
    let section = CarSection { header: None, rows: items.iter().cloned().map(CarRow::Item).collect() };
    sections_template(mtm, title, &[section], loading)
}

fn sections_template(mtm: MainThreadMarker, title: &str, sections: &[CarSection], loading: bool) -> Retained<CPListTemplate> {
    let empty = sections.iter().all(|s| s.rows.is_empty());
    let built: Vec<Retained<CPListSection>> = if loading {
        let rows: Vec<Retained<ProtocolObject<dyn CPListTemplateItem>>> =
            vec![ProtocolObject::from_retained(text_row(mtm, "Loading…", None))];
        // SAFETY: a section made from rows we own.
        vec![unsafe { CPListSection::initWithItems(CPListSection::alloc(), &NSArray::from_retained_slice(&rows)) }]
    } else {
        sections
            .iter()
            .filter(|s| !s.rows.is_empty())
            .map(|s| {
                let rows: Vec<Retained<ProtocolObject<dyn CPListTemplateItem>>> = s
                    .rows
                    .iter()
                    .map(|r| match r {
                        CarRow::Item(i) => ProtocolObject::from_retained(row(mtm, i)),
                        CarRow::Covers { title, items } => ProtocolObject::from_retained(covers_row(mtm, title, items)),
                    })
                    .collect();
                let rows = NSArray::from_retained_slice(&rows);
                // SAFETY: a section made from rows we own.
                unsafe {
                    match &s.header {
                        Some(h) => CPListSection::initWithItems_header_sectionIndexTitle(
                            CPListSection::alloc(),
                            &rows,
                            Some(&ns(h)),
                            None,
                        ),
                        None => CPListSection::initWithItems(CPListSection::alloc(), &rows),
                    }
                }
            })
            .collect()
    };
    // SAFETY: a template made from sections we own.
    unsafe {
        let t = CPListTemplate::initWithTitle_sections(
            CPListTemplate::alloc(mtm),
            Some(&ns(title)),
            &NSArray::from_retained_slice(&built),
        );
        if !loading && empty {
            t.setEmptyViewTitleVariants(&NSArray::from_retained_slice(&[ns("Nothing here yet")]));
        }
        t
    }
}

/// A row of covers under a title; tapping a cover plays it. Each cover
/// starts as a note and is swapped in as it loads. iOS 26 and later take
/// one element per cover (each updated on its own); older ones a list of
/// images, replaced whole.
fn covers_row(mtm: MainThreadMarker, title: &str, items: &[CarItem]) -> Retained<CPListImageRowItem> {
    let items: Vec<CarItem> = items.iter().take(super::COVERS_MAX).cloned().collect();
    let placeholder = symbol_image("music.note");
    let images: Vec<Retained<objc2_ui_kit::UIImage>> = items.iter().filter_map(|_| placeholder.clone()).collect();
    let titles: Vec<Retained<NSString>> = items.iter().map(|i| ns(&i.title)).collect();
    let elements_api = AnyClass::get(c"CPListImageRowItemRowElement").is_some() && images.len() == items.len();
    let (r, elements) = if elements_api {
        let elements: Vec<Retained<CPListImageRowItemRowElement>> = images
            .iter()
            .zip(&items)
            .map(|(img, i)| {
                // SAFETY: the element's initialiser with our image and strings.
                unsafe {
                    CPListImageRowItemRowElement::initWithImage_title_subtitle(
                        CPListImageRowItemRowElement::alloc(mtm),
                        img,
                        Some(&ns(&i.title)),
                        i.subtitle.as_deref().map(ns).as_deref(),
                    )
                }
            })
            .collect();
        // SAFETY: the row's initialiser with elements we own.
        let r = unsafe {
            CPListImageRowItem::initWithText_elements_allowsMultipleLines(
                CPListImageRowItem::alloc(mtm),
                Some(&ns(title)),
                &NSArray::from_slice(&elements.iter().map(|e| &**e).collect::<Vec<_>>()),
                false,
            )
        };
        (r, elements)
    } else {
        // SAFETY: the older initialiser, our images and titles, one each.
        #[allow(deprecated)]
        let r = unsafe {
            CPListImageRowItem::initWithText_images_imageTitles(
                CPListImageRowItem::alloc(mtm),
                &ns(title),
                &NSArray::from_retained_slice(&images),
                &NSArray::from_retained_slice(&titles),
            )
        };
        (r, Vec::new())
    };
    let actions: Vec<CarAction> = items.iter().map(|i| i.action.clone()).collect();
    let handler = RcBlock::new(
        move |_row: NonNull<CPListImageRowItem>, index: isize, done: *mut block2::DynBlock<dyn Fn()>| {
            if let Some(a) = usize::try_from(index).ok().and_then(|i| actions.get(i)) {
                picked(a);
            }
            if !done.is_null() {
                // SAFETY: CarPlay's completion block, called once.
                unsafe { (*done).call(()) };
            }
        },
    );
    // SAFETY: setter on our own row; CarPlay copies the handler.
    unsafe { r.setListImageRowHandler(Some(&handler)) };
    if let Some(app) = APP.get() {
        let id = COVER_IDS.with(|n| {
            let id = n.get() + 1;
            n.set(id);
            id
        });
        COVER_ROWS.with(|m| {
            m.borrow_mut().insert(id, CoverRow { row: r.clone(), images, elements });
        });
        for (i, item) in items.iter().enumerate() {
            if let Some(thumb) = &item.thumb {
                load_cover(app, thumb.clone(), id, i);
            }
        }
    }
    r
}

/// A row of covers being filled in (main thread only).
struct CoverRow {
    row: Retained<CPListImageRowItem>,
    /// The older API: every image, replaced whole.
    images: Vec<Retained<objc2_ui_kit::UIImage>>,
    /// iOS 26 and later: one element per cover.
    elements: Vec<Retained<CPListImageRowItemRowElement>>,
}

thread_local! {
    static COVER_ROWS: RefCell<std::collections::HashMap<u64, CoverRow>> = RefCell::new(std::collections::HashMap::new());
    static COVER_IDS: Cell<u64> = const { Cell::new(0) };
}

/// Fetches cover `i` of row `id` and shows it; only the numbers cross
/// threads, the row stays on the main thread.
fn load_cover(app: &AppHandle<Wry>, thumb: String, id: u64, i: usize) {
    let art = app.state::<ArtCache>().inner().clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(img) = art.get_or_fetch(&thumb, COVER_PX, COVER_PX).await else { return };
        let _ = app.run_on_main_thread(move || {
            // SAFETY: setting an image on our own row on the main thread.
            unsafe {
                let Some(image) = cover_image(&img.bytes) else { return };
                COVER_ROWS.with(|m| {
                    let mut m = m.borrow_mut();
                    let Some(c) = m.get_mut(&id) else { return };
                    if let Some(e) = c.elements.get(i) {
                        e.setImage(&image);
                    } else if let Some(slot) = c.images.get_mut(i) {
                        *slot = image;
                        #[allow(deprecated)]
                        c.row.updateImages(&NSArray::from_retained_slice(&c.images));
                    }
                });
            }
        });
    });
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
    if EXPLICIT.with(|e| e.borrow().marks(item)) {
        // SAFETY: CPListItem's explicitContent setter (iOS 14), on our own
        // row on the main thread.
        let _: () = unsafe { msg_send![&*r, setExplicitContent: true] };
    }
    if let (Some(thumb), Some(app)) = (&item.thumb, APP.get()) {
        load_art(app, thumb.clone(), OnMain(r.clone()));
    }
    let action = item.action.clone();
    let opens = matches!(action, CarAction::Open(_));
    if matches!(
        action,
        CarAction::Play(_) | CarAction::Shuffle(_) | CarAction::Open(CarList::Songs { .. }) | CarAction::PlayFrom { .. }
    ) {
        // SAFETY: a setter on our own row, on the main thread.
        unsafe { r.setPlaying(NOW_KEY.with(|k| super::is_playing(item, &k.borrow()))) };
        MARKABLE.with(|m| {
            let mut m = m.borrow_mut();
            if m.len() >= MARKED_MAX {
                m.pop_front();
            }
            m.push_back((r.clone(), item.clone()));
        });
    }
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
            // SAFETY: setting an image on our own row on the main thread.
            unsafe {
                if let Some(image) = cover_image(&img.bytes) {
                    row.setImage(Some(&image));
                }
            }
        });
    });
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

unsafe extern "C" {
    fn CGImageGetWidth(image: *mut std::ffi::c_void) -> usize;
    fn CGImageGetHeight(image: *mut std::ffi::c_void) -> usize;
    fn CGImageCreateWithImageInRect(image: *mut std::ffi::c_void, rect: Rect) -> *mut std::ffi::c_void;
    fn CGImageRelease(image: *mut std::ffi::c_void);
}

/// A cover from bytes, cropped to a square around its middle: CarPlay
/// draws a wide or tall picture narrower, which shifts the row's text.
fn cover_image(bytes: &[u8]) -> Option<Retained<objc2_ui_kit::UIImage>> {
    // SAFETY: UIImage from bytes; CoreGraphics' own create/release pair on
    // the image's CGImage; the cropped image is wrapped before release.
    unsafe {
        let image = objc2_ui_kit::UIImage::imageWithData(&NSData::with_bytes(bytes))?;
        let cg: *mut std::ffi::c_void = msg_send![&*image, CGImage];
        if cg.is_null() {
            return Some(image);
        }
        let Some((x, y, side)) = super::square_crop(CGImageGetWidth(cg), CGImageGetHeight(cg)) else {
            return Some(image);
        };
        let rect = Rect { x: x as f64, y: y as f64, w: side as f64, h: side as f64 };
        let cropped = CGImageCreateWithImageInRect(cg, rect);
        if cropped.is_null() {
            return Some(image);
        }
        // Keep the picture's own scale and orientation.
        let scale: f64 = msg_send![&*image, scale];
        let orientation: isize = msg_send![&*image, imageOrientation];
        let ui_class = objc2_ui_kit::UIImage::class();
        let square: Option<Retained<objc2_ui_kit::UIImage>> =
            msg_send![ui_class, imageWithCGImage: cropped, scale: scale, orientation: orientation];
        CGImageRelease(cropped);
        square.or(Some(image))
    }
}

/// Row covers: CarPlay draws them small.
const ART_PX: u32 = 120;
/// Covers in a row of covers are bigger.
const COVER_PX: u32 = 240;

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
        CarAction::ShuffleLibrary => {
            let offline = REACH.with(Cell::get) == Some(Reach::Unreachable);
            let player = app.state::<PlayerHandle>().inner().clone();
            let db = app.state::<DbHandle>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let read = if offline { super::shuffle_downloads_context } else { super::shuffle_library_context };
                let ctx = match db.read_async(read).await {
                    Ok(c) => c,
                    Err(e) => return tracing::warn!("CarPlay shuffle not read: {e}"),
                };
                if let Err(e) = player.play_context_from(ctx, None, None, Some(true)).await {
                    tracing::warn!("CarPlay play failed: {e}");
                }
            });
            show_now_playing();
        }
        CarAction::Play(ctx)
        | CarAction::Shuffle(ctx)
        | CarAction::PlayFrom { context: ctx, .. }
        | CarAction::Start { context: ctx, .. } => {
            let player = app.state::<PlayerHandle>().inner().clone();
            let ctx = ctx.clone();
            let shuffle = matches!(action, CarAction::Shuffle(_) | CarAction::Start { shuffle: true, .. }).then_some(true);
            let (start, index) = match action {
                CarAction::PlayFrom { track_id, index, .. } => (Some(track_id.clone()), Some(*index)),
                _ => (None, None),
            };
            tauri::async_runtime::spawn(async move {
                if let Err(e) = player.play_context_from(ctx, start, index, shuffle).await {
                    tracing::warn!("CarPlay play failed: {e}");
                }
            });
            show_now_playing();
        }
    }
}

/// Reads a list off the main thread, then pushes it there.
fn open_list(app: &AppHandle<Wry>, which: &CarList) {
    let app = app.clone();
    let which = which.clone();
    let offline = REACH.with(Cell::get) == Some(Reach::Unreachable);
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<DbHandle>();
        let liked_min = crate::settings::load(&db).liked_min_rating();
        let mut items = db.read(|c| super::list(c, &which, liked_min)).unwrap_or_else(|e| {
            tracing::warn!("CarPlay list not read: {e}");
            Vec::new()
        });
        if offline {
            let kept = db.read(Kept::read).unwrap_or_default();
            items = super::offline_rows(items, &kept, offline_only(&app));
        }
        let marks = db.read(|c| super::explicit_rows(c, &items)).unwrap_or_default();
        let _ = app.run_on_main_thread(move || {
            EXPLICIT.with(|e| *e.borrow_mut() = marks);
            let Some(controller) = CAR.with(|c| c.borrow().clone()) else { return };
            let Some(mtm) = MainThreadMarker::new() else { return };
            let t = list_template(mtm, which.title(), &items, false);
            // SAFETY: main thread, our template.
            unsafe { controller.pushTemplate_animated_completion(&t, true, None) };
        });
    });
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
