//! The iPhone's web view as an app, not a page: the phone layout scrolls in
//! its own panels, so the outer view never pans, bounces or zooms, and the
//! page runs under the status bar and home indicator (the layout keeps its
//! controls clear of them with the safe-area insets). The keyboard comes
//! without the form bar (previous, next, Done) a page gets in Safari.

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

pub fn settle<R: Runtime>(app: &tauri::AppHandle<R>) {
    hide_form_bar();
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
