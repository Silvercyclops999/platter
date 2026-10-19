//! The phone's own AirPlay and Bluetooth picker (`AVRoutePickerView`), opened
//! from the device sheet. iOS has no call to show it directly, so a picker
//! view sits unseen on the web view and its button is pressed for the user.

use tauri::{AppHandle, Runtime};

use crate::error::{AppError, AppResult};

#[tauri::command]
pub async fn show_route_picker<R: Runtime>(app: AppHandle<R>) -> AppResult<()> {
    #[cfg(target_os = "ios")]
    return ios::show(&app);
    #[cfg(not(target_os = "ios"))]
    {
        let _ = app;
        Err(AppError::Invalid("The system's speaker picker is only on the phone.".into()))
    }
}

#[cfg(target_os = "ios")]
mod ios {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use tauri::{AppHandle, Manager, Runtime};

    use crate::error::{AppError, AppResult};

    // AVRoutePickerView lives in AVKit, which the app's Xcode project
    // doesn't link on its own (see ios_session.rs).
    std::arch::global_asm!(".linker_option \"-framework\", \"AVKit\"");

    /// UIControlEventTouchUpInside
    const TOUCH_UP_INSIDE: usize = 1 << 6;

    pub fn show<R: Runtime>(app: &AppHandle<R>) -> AppResult<()> {
        let w = app
            .get_webview_window("main")
            .ok_or_else(|| AppError::Internal("no main window".into()))?;
        w.with_webview(|pw| {
            let web = pw.inner() as *mut AnyObject;
            if web.is_null() {
                return;
            }
            // SAFETY: on the main thread (with_webview runs there); UIKit and
            // AVKit objects looked up by name and sent documented messages.
            unsafe {
                let Some(class) = AnyClass::get(c"AVRoutePickerView") else {
                    tracing::warn!("no AVRoutePickerView");
                    return;
                };
                let picker: *mut AnyObject = msg_send![class, new];
                if picker.is_null() {
                    return;
                }
                let _: () = msg_send![picker, setAlpha: 0.0f64];
                let _: () = msg_send![picker, setUserInteractionEnabled: false];
                let _: () = msg_send![web, addSubview: picker];
                let subviews: *mut AnyObject = msg_send![picker, subviews];
                let count: usize = msg_send![subviews, count];
                let button_class = AnyClass::get(c"UIButton");
                for i in 0..count {
                    let v: *mut AnyObject = msg_send![subviews, objectAtIndex: i];
                    let is_button = button_class.is_some_and(|b| {
                        let yes: bool = msg_send![v, isKindOfClass: b];
                        yes
                    });
                    if is_button {
                        let _: () = msg_send![v, sendActionsForControlEvents: TOUCH_UP_INSIDE];
                        break;
                    }
                }
                // The web view holds the picker (it presents from there);
                // a tiny unseen view per press, gone with the window.
                let _: () = msg_send![picker, release];
            }
        })
        .map_err(|e| AppError::Internal(e.to_string()))
    }
}
