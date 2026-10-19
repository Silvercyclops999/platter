//! The car's screen: Apple's templates filled from [`super`]'s lists.
//!
//! CarPlay connects its own scene. Tauri's window layer hands every scene to
//! its own delegate, so [`install`] (run before the app starts) points the
//! CarPlay scene's configuration at [`CarDelegate`] instead. The lists need
//! the library, which exists once the app has set up; a car that connects
//! sooner sees "Loading" until [`attach`] fills the tabs.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, ProtocolObject, Sel};
use objc2::{AnyThread, ClassType, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_car_play::{
    CPInterfaceController, CPListItem, CPListItemAccessoryType, CPListSection, CPListTemplate, CPListTemplateItem,
    CPNowPlayingTemplate, CPSelectableListItem, CPTabBarTemplate, CPTemplate, CPTemplateApplicationScene,
    CPTemplateApplicationSceneDelegate, CPTemplateApplicationSceneSessionRoleApplication,
};
use objc2_foundation::{NSArray, NSData, NSObject, NSObjectProtocol, NSString};
use objc2_ui_kit::UISceneDelegate;
use tauri::{AppHandle, Manager, Wry};

use super::{CarAction, CarItem, CarList};
use crate::audio::player::PlayerHandle;
use crate::db::DbHandle;
use crate::db::artcache::ArtCache;

std::arch::global_asm!(".linker_option \"-framework\", \"CarPlay\"");

static APP: OnceLock<AppHandle<Wry>> = OnceLock::new();

thread_local! {
    /// The car's interface while it is connected (main thread only).
    static CAR: RefCell<Option<Retained<CPInterfaceController>>> = const { RefCell::new(None) };
}

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
    let home = read("home", &super::home);
    let downloaded = read("downloads", &super::downloaded);
    let radio = read("radio", &|c| super::radio(c, None));
    let library = if APP.get().is_some() { super::library() } else { Vec::new() };
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
        CarAction::Open(list) => open_list(app, *list),
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

fn open_list(app: &AppHandle<Wry>, which: CarList) {
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
