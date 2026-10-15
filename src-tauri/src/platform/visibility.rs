//! Whether the page in the main window can be seen.
//!
//! On Windows the webview isn't told when its window is minimised or hidden
//! to the tray, so the page keeps its timers, animations and frame
//! callbacks at full speed. Telling WebView2 the page is hidden makes
//! `document.hidden` true, so the page and the browser engine throttle
//! themselves; the window coming back tells it again. macOS WebKit notices
//! on its own.

use std::sync::atomic::{AtomicU8, Ordering};

use tauri::{Manager, Runtime, WebviewWindow, Window, WindowEvent};

const UNKNOWN: u8 = 0;
const SHOWN: u8 = 1;
const HIDDEN: u8 = 2;

/// What the page was last told, so repeated events don't call into WebView2.
static LAST: AtomicU8 = AtomicU8::new(UNKNOWN);

/// What a window event means for the page: Some(true) seen, Some(false)
/// hidden, None no change. `minimized` and `visible` are the window's state
/// after the event.
pub fn after_event(event: &WindowEvent, minimized: bool, visible: bool) -> Option<bool> {
    match event {
        // Every way back (tray, taskbar, notification) ends with focus.
        WindowEvent::Focused(true) => Some(true),
        WindowEvent::Focused(false) | WindowEvent::Resized(_) => Some(visible && !minimized),
        _ => None,
    }
}

/// Follow the main window's events.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }
    let minimized = window.is_minimized().unwrap_or(false);
    let visible = window.is_visible().unwrap_or(true);
    if let Some(seen) = after_event(event, minimized, visible)
        && let Some(w) = window.app_handle().get_webview_window("main")
    {
        set_page_visible(&w, seen);
    }
}

/// Tell the webview whether its page can be seen.
pub fn set_page_visible<R: Runtime>(w: &WebviewWindow<R>, seen: bool) {
    let want = if seen { SHOWN } else { HIDDEN };
    if LAST.swap(want, Ordering::SeqCst) == want {
        return;
    }
    tracing::debug!(seen, "main window page visibility");
    #[cfg(windows)]
    {
        let _ = w.with_webview(move |pw| {
            // SAFETY: a plain COM call on the webview's own controller, made
            // on the thread that owns it.
            unsafe {
                let _ = pw.controller().SetIsVisible(seen);
            }
        });
    }
    #[cfg(not(windows))]
    let _ = w;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resized() -> WindowEvent {
        WindowEvent::Resized(tauri::PhysicalSize::new(0, 0))
    }

    #[test]
    fn minimised_or_hidden_means_the_page_is_hidden() {
        assert_eq!(after_event(&resized(), true, true), Some(false));
        assert_eq!(after_event(&WindowEvent::Focused(false), false, false), Some(false));
        // Losing focus to another app while still on screen changes nothing.
        assert_eq!(after_event(&WindowEvent::Focused(false), false, true), Some(true));
    }

    #[test]
    fn coming_back_shows_the_page() {
        assert_eq!(after_event(&WindowEvent::Focused(true), false, true), Some(true));
        assert_eq!(after_event(&resized(), false, true), Some(true));
        assert_eq!(after_event(&WindowEvent::Moved(tauri::PhysicalPosition::new(0, 0)), false, true), None);
    }
}
