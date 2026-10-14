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

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    pub fn on_battery() -> bool {
        false
    }
}
