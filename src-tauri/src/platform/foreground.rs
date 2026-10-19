//! Whether the app is in the foreground. On the phone, music keeps the app
//! running with the screen off, and so would every background check; the
//! non-essential ones (file checks, analysis, tag scans, release radar, the
//! doctor, Lidarr catch-up, sound ID) wait in [`settled`] while the app is
//! in the background, and carry on the moment it comes back. Playback,
//! scrobbling and the server timeline never wait. On a computer the app
//! always counts as in the foreground.

use std::sync::OnceLock;

use tokio::sync::watch;

fn channel() -> &'static watch::Sender<bool> {
    static TX: OnceLock<watch::Sender<bool>> = OnceLock::new();
    TX.get_or_init(|| watch::channel(true).0)
}

/// The app moved to the foreground (`true`) or the background.
pub fn set(foreground: bool) {
    channel().send_if_modified(|v| std::mem::replace(v, foreground) != foreground);
}

pub fn is_foreground() -> bool {
    *channel().borrow()
}

/// Returns at once in the foreground; in the background, waits until the
/// app comes back.
pub async fn settled() {
    let mut rx = channel().subscribe();
    let _ = rx.wait_for(|fg| *fg).await;
}

/// Follows iOS's own notifications. Call once at startup, on any thread.
#[cfg(target_os = "ios")]
pub fn watch_app_state() {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;

    // SAFETY: NSNotificationCenter's block observer API with UIKit's
    // notification names; the blocks only flip an atomic-backed channel and
    // the observers live for the app's life.
    unsafe {
        let Some(center_class) = AnyClass::get(c"NSNotificationCenter") else { return };
        let center: *mut AnyObject = msg_send![center_class, defaultCenter];
        if center.is_null() {
            return;
        }
        for (name, fg) in [
            ("UIApplicationDidEnterBackgroundNotification", false),
            ("UIApplicationWillEnterForegroundNotification", true),
        ] {
            let block = RcBlock::new(move |_note: *mut AnyObject| {
                tracing::debug!(foreground = fg, "app state");
                set(fg);
            });
            let observer: *mut AnyObject = msg_send![center,
                addObserverForName: &*NSString::from_str(name),
                object: std::ptr::null::<AnyObject>(),
                queue: std::ptr::null::<AnyObject>(),
                usingBlock: &*block];
            // Kept for the app's life.
            let _ = objc2::rc::Retained::retain(observer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn background_work_waits_for_the_foreground() {
        set(false);
        assert!(!is_foreground());
        let waiting = tokio::spawn(settled());
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!waiting.is_finished(), "waits while in the background");
        set(true);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiting).await.unwrap().unwrap();
        // In the foreground it doesn't wait at all.
        tokio::time::timeout(std::time::Duration::from_millis(50), settled()).await.unwrap();
    }
}
