//! Whether the computer is running on battery power (background work like
//! lyrics transcription waits while it is). Unknown counts as plugged in.

/// Blocking (macOS asks `pmset`); call from a blocking thread.
pub fn on_battery() -> bool {
    imp::on_battery()
}

#[cfg(target_os = "windows")]
mod imp {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    pub fn on_battery() -> bool {
        let mut s = SYSTEM_POWER_STATUS::default();
        // SAFETY: a plain out-parameter call.
        let ok = unsafe { GetSystemPowerStatus(&mut s) }.is_ok();
        // ACLineStatus: 0 offline (battery), 1 online, 255 unknown.
        ok && s.ACLineStatus == 0
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn on_battery() -> bool {
        std::process::Command::new("/usr/bin/pmset")
            .args(["-g", "batt"])
            .output()
            .ok()
            .is_some_and(|o| String::from_utf8_lossy(&o.stdout).contains("'Battery Power'"))
    }
}

/// The phone: on battery unless it's charging or full (UIDevice's
/// battery state: 1 unplugged, 2 charging, 3 full; 0 unknown counts as
/// plugged in, as elsewhere).
#[cfg(target_os = "ios")]
mod imp {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    pub fn on_battery() -> bool {
        // SAFETY: UIDevice's shared instance and two documented properties;
        // reading the battery state needs monitoring switched on.
        unsafe {
            let Some(class) = AnyClass::get(c"UIDevice") else { return false };
            let device: *mut AnyObject = msg_send![class, currentDevice];
            if device.is_null() {
                return false;
            }
            let _: () = msg_send![device, setBatteryMonitoringEnabled: true];
            let state: isize = msg_send![device, batteryState];
            state == 1
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios")))]
mod imp {
    pub fn on_battery() -> bool {
        false
    }
}
