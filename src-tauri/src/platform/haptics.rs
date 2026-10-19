//! The phone's taptic engine, for controls that should feel pressed
//! (play, skip, like, a picked option). The phone UI calls it when the
//! "Haptics on controls" setting is on; off the phone it does nothing.

use tauri::{AppHandle, Runtime};

use crate::error::{AppError, AppResult};

/// What the tap felt like, in iOS's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Haptic {
    /// A button press (play, skip, like).
    Light,
    /// A bigger moment (a sheet snapping open, a drag dropping).
    Medium,
    /// Moving through a set of choices (a picker, a sort option).
    Selection,
    /// Something finished well (a download done, a playlist made).
    Success,
    /// Something didn't work.
    Error,
}

#[tauri::command]
pub async fn haptic<R: Runtime>(app: AppHandle<R>, kind: Haptic) -> AppResult<()> {
    #[cfg(target_os = "ios")]
    return app
        .run_on_main_thread(move || ios::play(kind))
        .map_err(|e| AppError::Internal(e.to_string()));
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (app, kind);
        Ok::<(), AppError>(())
    }
}

#[cfg(target_os = "ios")]
mod ios {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    use super::Haptic;

    /// UIImpactFeedbackStyle
    const LIGHT: isize = 0;
    const MEDIUM: isize = 1;
    /// UINotificationFeedbackType
    const SUCCESS: isize = 0;
    const ERROR: isize = 2;

    pub fn play(kind: Haptic) {
        // SAFETY: UIKit feedback generators, made and used on the main thread
        // (the caller runs this there), sent their documented messages.
        unsafe {
            let (class, style): (&std::ffi::CStr, Option<isize>) = match kind {
                Haptic::Light => (c"UIImpactFeedbackGenerator", Some(LIGHT)),
                Haptic::Medium => (c"UIImpactFeedbackGenerator", Some(MEDIUM)),
                Haptic::Selection => (c"UISelectionFeedbackGenerator", None),
                Haptic::Success | Haptic::Error => (c"UINotificationFeedbackGenerator", None),
            };
            let Some(cls) = AnyClass::get(class) else { return };
            let alloc: *mut AnyObject = msg_send![cls, alloc];
            let g: *mut AnyObject = match style {
                Some(s) => msg_send![alloc, initWithStyle: s],
                None => msg_send![alloc, init],
            };
            if g.is_null() {
                return;
            }
            match kind {
                Haptic::Light | Haptic::Medium => {
                    let _: () = msg_send![g, impactOccurred];
                }
                Haptic::Selection => {
                    let _: () = msg_send![g, selectionChanged];
                }
                Haptic::Success => {
                    let _: () = msg_send![g, notificationOccurred: SUCCESS];
                }
                Haptic::Error => {
                    let _: () = msg_send![g, notificationOccurred: ERROR];
                }
            }
            let _: () = msg_send![g, release];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Haptic;

    #[test]
    fn kinds_read_as_the_ui_names_them() {
        let k: Haptic = serde_json::from_str("\"selection\"").unwrap();
        assert_eq!(k, Haptic::Selection);
        assert!(serde_json::from_str::<Haptic>("\"buzz\"").is_err());
    }
}
