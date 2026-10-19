//! The iPhone's web view as an app, not a page: the phone layout scrolls in
//! its own panels, so the outer view never pans, bounces or zooms, and the
//! page runs under the status bar and home indicator (the layout keeps its
//! controls clear of them with the safe-area insets). The keyboard comes
//! without the form bar (previous, next, Done) a page gets in Safari.

use std::sync::atomic::{AtomicIsize, Ordering};

use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{msg_send, sel};
use tauri::{Manager, Runtime};

/// UIScrollViewContentInsetAdjustmentBehavior.never
const INSETS_NEVER: isize = 2;

/// `inputAccessoryView` for the web view's content: no form bar.
extern "C-unwind" fn no_accessory(_this: *mut AnyObject, _cmd: Sel) -> *mut AnyObject {
    std::ptr::null_mut()
}

fn hide_form_bar() {
    let Some(class) = AnyClass::get(c"WKContentView") else { return };
    // SAFETY: replaces one getter on WebKit's content view with a function
    // of the same shape (object, selector) -> object, returning nil, which
    // UIKit reads as "no accessory view".
    unsafe {
        let imp: Imp = std::mem::transmute(no_accessory as extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject);
        objc2::ffi::class_replaceMethod(
            class as *const AnyClass as *mut AnyClass,
            sel!(inputAccessoryView),
            imp,
            c"@@:".as_ptr(),
        );
    }
}

/// UIStatusBarStyle: 0 default (iOS picks from what's behind it), 1 light
/// text, 3 dark text.
static STATUS_STYLE: AtomicIsize = AtomicIsize::new(0);

extern "C-unwind" fn preferred_status_bar_style(_this: *mut AnyObject, _cmd: Sel) -> isize {
    STATUS_STYLE.load(Ordering::Relaxed)
}

/// Lets the window's view controller answer `preferredStatusBarStyle` from
/// [`STATUS_STYLE`].
fn own_status_bar() {
    let Some(class) = AnyClass::get(c"TaoUIViewController") else { return };
    // SAFETY: adds (or replaces) one getter with a function of the same
    // shape (object, selector) -> NSInteger.
    unsafe {
        let imp: Imp = std::mem::transmute(
            preferred_status_bar_style as extern "C-unwind" fn(*mut AnyObject, Sel) -> isize,
        );
        objc2::ffi::class_replaceMethod(
            class as *const AnyClass as *mut AnyClass,
            sel!(preferredStatusBarStyle),
            imp,
            c"q@:".as_ptr(),
        );
    }
}

/// Dark status bar text for a light look, light text for a dark one.
pub fn set_status_bar_style<R: Runtime>(app: &tauri::AppHandle<R>, dark: bool) {
    STATUS_STYLE.store(if dark { 3 } else { 1 }, Ordering::Relaxed);
    let Some(w) = app.get_webview_window("main") else { return };
    let _ = w.with_webview(|pw| {
        let web = pw.inner() as *mut AnyObject;
        if web.is_null() {
            return;
        }
        // SAFETY: main thread (with_webview runs there); UIView and
        // UIViewController messages on the window's own objects.
        unsafe {
            let window: *mut AnyObject = msg_send![web, window];
            if window.is_null() {
                return;
            }
            let root: *mut AnyObject = msg_send![window, rootViewController];
            if !root.is_null() {
                let _: () = msg_send![root, setNeedsStatusBarAppearanceUpdate];
            }
        }
    });
}

pub fn settle<R: Runtime>(app: &tauri::AppHandle<R>) {
    hide_form_bar();
    own_status_bar();
    let Some(w) = app.get_webview_window("main") else { return };
    let _ = w.with_webview(|pw| {
        let web = pw.inner() as *mut AnyObject;
        if web.is_null() {
            return;
        }
        // SAFETY: `inner` is the window's WKWebView, used on the main thread
        // (with_webview runs there); these are UIScrollView setters.
        unsafe {
            let scroll: *mut AnyObject = msg_send![web, scrollView];
            if scroll.is_null() {
                return;
            }
            let _: () = msg_send![scroll, setScrollEnabled: false];
            let _: () = msg_send![scroll, setBounces: false];
            let _: () = msg_send![scroll, setAlwaysBounceHorizontal: false];
            let _: () = msg_send![scroll, setAlwaysBounceVertical: false];
            let _: () = msg_send![scroll, setContentInsetAdjustmentBehavior: INSETS_NEVER];
            let pinch: *mut AnyObject = msg_send![scroll, pinchGestureRecognizer];
            if !pinch.is_null() {
                let _: () = msg_send![pinch, setEnabled: false];
            }
        }
    });
}
