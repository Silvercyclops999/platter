//! The rest of the computer, as background speech work sees it: how busy
//! the processor is with other programs, whether a fullscreen game or a
//! presentation is in front, whether the computer is running hot, how long
//! since the last keyboard or mouse input; and running the speech helper
//! process at a lower priority (see `lyrics/transcribe/pace.rs` and
//! `lyrics/transcribe/helper`).

use std::process::{Child, Command};
use std::time::Duration;

use parking_lot::Mutex;

/// Processor time counters at one moment, in seconds summed over every core.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// All time that passed (busy and idle).
    pub total: f64,
    /// Time no program was running.
    pub idle: f64,
    /// Time Platter itself used (the app and its speech helper).
    pub own: f64,
    /// The speech helper's process id when it was counted in `own`.
    pub helper: Option<u32>,
    /// The speech helper's share of `own`.
    pub helper_own: f64,
}

/// Percent of the whole processor that other programs used between two
/// readings (Platter's own work left out). None when no time passed.
pub fn others_percent(before: Reading, after: Reading) -> Option<f32> {
    let total = after.total - before.total;
    if total <= 0.0 {
        return None;
    }
    let busy = total - (after.idle - before.idle);
    // A helper that started or stopped in between: count only what's known.
    let mut own = (after.own - after.helper_own) - (before.own - before.helper_own);
    own += match (before.helper, after.helper) {
        (Some(a), Some(b)) if a == b => after.helper_own - before.helper_own,
        (_, Some(_)) => after.helper_own,
        _ => 0.0,
    };
    Some((((busy - own) / total) * 100.0).clamp(0.0, 100.0) as f32)
}

/// The running speech helper, so its work counts as Platter's own.
static HELPER: Mutex<Option<u32>> = Mutex::new(None);

/// The speech helper started (Some) or ended (None).
pub fn set_helper(pid: Option<u32>) {
    *HELPER.lock() = pid;
}

/// System-wide readings, each compared with the one before.
#[derive(Default)]
pub struct SystemLoad {
    last: Mutex<Option<Reading>>,
}

impl SystemLoad {
    /// Other programs' share since the last call. None on the first call or
    /// where the counters can't be read.
    pub fn others_percent(&self) -> Option<f32> {
        let helper = *HELPER.lock();
        let mut now = imp::read()?;
        if let Some(pid) = helper
            && let Some(t) = imp::process_secs(pid)
        {
            now.helper = Some(pid);
            now.helper_own = t;
            now.own += t;
        }
        let before = self.last.lock().replace(now)?;
        others_percent(before, now)
    }

    /// Forget the last reading (sampling stopped; the next one starts fresh).
    pub fn reset(&self) {
        *self.last.lock() = None;
    }
}

/// A fullscreen game (Direct3D) or a presentation is in front (Windows).
pub fn fullscreen_in_front() -> bool {
    imp::fullscreen_in_front()
}

/// The computer reports it's warm enough to slow down: macOS thermal
/// state Fair or worse.
pub fn running_hot() -> bool {
    imp::running_hot()
}

/// Time since the last keyboard or mouse input. None where unknown.
pub fn since_input() -> Option<Duration> {
    imp::since_input()
}

/// Windows: the speech helper starts without a window, below normal
/// priority (idle priority when `lowest`).
pub fn lower_command(cmd: &mut Command, lowest: bool) {
    imp::lower_command(cmd, lowest)
}

/// Windows: the speech helper's priority class, and power throttling
/// (EcoQoS) so the processor runs it at low clocks. Called at start and
/// again before each run, as the setting may have changed.
pub fn lower_child(child: &Child, lowest: bool) {
    imp::lower_child(child, lowest)
}

/// macOS and Linux: the helper lowers itself (background priority on
/// macOS, nice on Linux).
pub fn lower_this_process(lowest: bool) {
    imp::lower_this_process(lowest)
}

#[cfg(target_os = "windows")]
mod imp {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};
    use std::time::Duration;

    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::System::Threading::{
        BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW, GetCurrentProcess, GetProcessTimes, GetSystemTimes,
        IDLE_PRIORITY_CLASS, OpenProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_LIMITED_INFORMATION, ProcessPowerThrottling, SetPriorityClass,
        SetProcessInformation,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::Win32::UI::Shell::{QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};

    use super::Reading;

    fn secs(f: FILETIME) -> f64 {
        // 100 ns units.
        (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as f64 / 10_000_000.0
    }

    fn times(h: HANDLE) -> Option<f64> {
        let (mut created, mut exited, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        // SAFETY: a process handle and plain out-parameters.
        unsafe { GetProcessTimes(h, &mut created, &mut exited, &mut k, &mut u) }.ok()?;
        Some(secs(k) + secs(u))
    }

    pub fn read() -> Option<Reading> {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        // SAFETY: plain out-parameters.
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
        // SAFETY: the pseudo handle for this process.
        let own = times(unsafe { GetCurrentProcess() })?;
        // Kernel time includes idle time.
        Some(Reading { total: secs(kernel) + secs(user), idle: secs(idle), own, helper: None, helper_own: 0.0 })
    }

    pub fn process_secs(pid: u32) -> Option<f64> {
        // SAFETY: opening a process for its times only; closed below.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let t = times(h);
        // SAFETY: the handle opened above.
        let _ = unsafe { CloseHandle(h) };
        t
    }

    pub fn fullscreen_in_front() -> bool {
        // SAFETY: no arguments besides the returned state.
        match unsafe { SHQueryUserNotificationState() } {
            Ok(s) => s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE,
            Err(_) => false,
        }
    }

    pub fn running_hot() -> bool {
        false
    }

    pub fn since_input() -> Option<Duration> {
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        // SAFETY: a sized out-parameter.
        if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
            return None;
        }
        // SAFETY: no arguments. Both are milliseconds since boot (wrapping).
        let now = unsafe { GetTickCount() };
        Some(Duration::from_millis(now.wrapping_sub(info.dwTime) as u64))
    }

    pub fn lower_command(cmd: &mut Command, lowest: bool) {
        let class = if lowest { IDLE_PRIORITY_CLASS } else { BELOW_NORMAL_PRIORITY_CLASS };
        cmd.creation_flags(CREATE_NO_WINDOW.0 | class.0);
    }

    pub fn lower_child(child: &Child, lowest: bool) {
        let h = HANDLE(child.as_raw_handle());
        let class = if lowest { IDLE_PRIORITY_CLASS } else { BELOW_NORMAL_PRIORITY_CLASS };
        // SAFETY: the child's own handle, alive as long as `child`.
        if let Err(e) = unsafe { SetPriorityClass(h, class) } {
            tracing::debug!("speech helper priority not set: {e}");
        }
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        };
        // SAFETY: as above, with a sized struct for this information class.
        let res = unsafe {
            SetProcessInformation(
                h,
                ProcessPowerThrottling,
                &state as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            )
        };
        if let Err(e) = res {
            // Older Windows 10 versions don't have it.
            tracing::debug!("speech helper power throttling not set: {e}");
        }
    }

    pub fn lower_this_process(_lowest: bool) {}
}

#[cfg(target_os = "macos")]
mod imp {
    use std::process::{Child, Command};
    use std::sync::OnceLock;
    use std::time::Duration;

    use super::Reading;

    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }

    /// The start of `rusage_info_v0` (the fields after these aren't read).
    #[repr(C)]
    struct RusageInfo {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        rest: [u64; 10],
    }

    unsafe extern "C" {
        fn mach_host_self() -> u32;
        fn host_statistics(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
        fn clock_gettime(clock: u32, tp: *mut Timespec) -> i32;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfo) -> i32;
        fn setpriority(which: i32, who: u32, prio: i32) -> i32;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }

    const HOST_CPU_LOAD_INFO: i32 = 3;
    /// user, system, idle, nice
    const CPU_STATES: usize = 4;
    const CLOCK_PROCESS_CPUTIME_ID: u32 = 12;
    /// Clock ticks per second for the host counters.
    const TICKS: f64 = 100.0;
    const RUSAGE_INFO_V0: i32 = 0;
    const PRIO_DARWIN_PROCESS: i32 = 4;
    const PRIO_DARWIN_BG: i32 = 0x1000;
    const HID_SYSTEM_STATE: i32 = 1;
    const ANY_INPUT: u32 = !0;

    pub fn read() -> Option<Reading> {
        static HOST: OnceLock<u32> = OnceLock::new();
        // SAFETY: returns a send right to this host, kept for the app's life.
        let host = *HOST.get_or_init(|| unsafe { mach_host_self() });
        let mut ticks = [0i32; CPU_STATES];
        let mut count = CPU_STATES as u32;
        // SAFETY: `ticks` holds HOST_CPU_LOAD_INFO_COUNT integers.
        if unsafe { host_statistics(host, HOST_CPU_LOAD_INFO, ticks.as_mut_ptr(), &mut count) } != 0 {
            return None;
        }
        let t: Vec<f64> = ticks.iter().map(|&v| v as u32 as f64 / TICKS).collect();
        let mut ts = Timespec { sec: 0, nsec: 0 };
        // SAFETY: a plain out-parameter.
        if unsafe { clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &mut ts) } != 0 {
            return None;
        }
        let own = ts.sec as f64 + ts.nsec as f64 / 1e9;
        Some(Reading { total: t.iter().sum(), idle: t[2], own, helper: None, helper_own: 0.0 })
    }

    pub fn process_secs(pid: u32) -> Option<f64> {
        static BASE: OnceLock<(u32, u32)> = OnceLock::new();
        let (numer, denom) = *BASE.get_or_init(|| {
            let mut tb = Timebase::default();
            // SAFETY: a plain out-parameter.
            unsafe { mach_timebase_info(&mut tb) };
            if tb.denom == 0 { (1, 1) } else { (tb.numer, tb.denom) }
        });
        let mut info = RusageInfo { uuid: [0; 16], user_time: 0, system_time: 0, rest: [0; 10] };
        // SAFETY: `info` is at least as large as rusage_info_v0.
        if unsafe { proc_pid_rusage(pid as i32, RUSAGE_INFO_V0, &mut info) } != 0 {
            return None;
        }
        // Mach time units (nanoseconds on Intel).
        let ticks = (info.user_time + info.system_time) as f64;
        Some(ticks * numer as f64 / denom as f64 / 1e9)
    }

    pub fn fullscreen_in_front() -> bool {
        false
    }

    pub fn running_hot() -> bool {
        use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};
        let state = NSProcessInfo::processInfo().thermalState();
        state != NSProcessInfoThermalState::Nominal
    }

    pub fn since_input() -> Option<Duration> {
        // SAFETY: a plain call with constant arguments.
        let s = unsafe { CGEventSourceSecondsSinceLastEventType(HID_SYSTEM_STATE, ANY_INPUT) };
        (s.is_finite() && s >= 0.0).then(|| Duration::from_secs_f64(s))
    }

    pub fn lower_command(_: &mut Command, _: bool) {}

    pub fn lower_child(_: &Child, _: bool) {}

    pub fn lower_this_process(_lowest: bool) {
        // Background: lowest scheduling, throttled disk use, and the
        // efficiency cores on Apple silicon.
        // SAFETY: a plain call on this process.
        let _ = unsafe { setpriority(PRIO_DARWIN_PROCESS, 0, PRIO_DARWIN_BG) };
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use std::process::{Child, Command};
    use std::time::Duration;

    use super::Reading;

    /// Clock ticks per second in /proc (USER_HZ, 100 on every common build).
    const HZ: f64 = 100.0;

    unsafe extern "C" {
        fn setpriority(which: i32, who: u32, prio: i32) -> i32;
    }

    /// utime + stime of a /proc/<pid>/stat line, in seconds.
    fn stat_secs(stat: &str) -> Option<f64> {
        // The command name can hold spaces; fields restart after its ')'.
        let rest = &stat[stat.rfind(')')? + 2..];
        let f: Vec<&str> = rest.split_whitespace().collect();
        let (u, s): (f64, f64) = (f.get(11)?.parse().ok()?, f.get(12)?.parse().ok()?);
        Some((u + s) / HZ)
    }

    pub fn read() -> Option<Reading> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let cpu: Vec<f64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
        // user nice system idle iowait irq softirq steal
        if cpu.len() < 5 {
            return None;
        }
        let total: f64 = cpu.iter().take(8).sum::<f64>() / HZ;
        let idle = (cpu[3] + cpu[4]) / HZ;
        let own = stat_secs(&std::fs::read_to_string("/proc/self/stat").ok()?)?;
        Some(Reading { total, idle, own, helper: None, helper_own: 0.0 })
    }

    pub fn process_secs(pid: u32) -> Option<f64> {
        stat_secs(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
    }

    pub fn fullscreen_in_front() -> bool {
        false
    }

    pub fn running_hot() -> bool {
        false
    }

    pub fn since_input() -> Option<Duration> {
        None
    }

    pub fn lower_command(_: &mut Command, _: bool) {}

    pub fn lower_child(_: &Child, _: bool) {}

    pub fn lower_this_process(lowest: bool) {
        // Threads started after this inherit it.
        // SAFETY: a plain call on the calling thread (PRIO_PROCESS, 0).
        let _ = unsafe { setpriority(0, 0, if lowest { 19 } else { 10 }) };
    }

    #[cfg(test)]
    #[test]
    fn reads_proc_stat() {
        let line = "1234 (my (odd) app) S 1 2 3 4 5 6 7 8 9 10 250 50 0 0";
        assert_eq!(stat_secs(line), Some(3.0));
        assert!(read().is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(total: f64, idle: f64, own: f64) -> Reading {
        Reading { total, idle, own, helper: None, helper_own: 0.0 }
    }

    #[test]
    fn others_share_leaves_platter_out() {
        let a = r(100.0, 50.0, 10.0);
        // 40 s passed over every core: 10 idle, 20 Platter's, 10 others'.
        let b = r(140.0, 60.0, 30.0);
        assert_eq!(others_percent(a, b), Some(25.0));
        assert_eq!(others_percent(a, a), None, "no time passed");
        // Platter's counter running ahead never goes below zero.
        let c = r(140.0, 60.0, 90.0);
        assert_eq!(others_percent(a, c), Some(0.0));
    }

    #[test]
    fn the_speech_helper_counts_as_platter() {
        // The helper (pid 9) used 20 of the 30 busy seconds; others used 10.
        let a = Reading { helper: Some(9), helper_own: 5.0, ..r(100.0, 50.0, 15.0) };
        let b = Reading { helper: Some(9), helper_own: 25.0, ..r(140.0, 60.0, 35.0) };
        assert_eq!(others_percent(a, b), Some(25.0));
        // A new helper since the last reading: all its time is in this span.
        let c = Reading { helper: Some(10), helper_own: 20.0, ..r(140.0, 60.0, 30.0) };
        assert_eq!(others_percent(r(100.0, 50.0, 10.0), c), Some(25.0));
        // The helper ended: its earlier time isn't taken off again.
        let d = r(140.0, 60.0, 30.0);
        let e = Reading { helper: Some(9), helper_own: 5.0, ..r(100.0, 50.0, 15.0) };
        assert_eq!(others_percent(e, d), Some(25.0));
    }

    #[test]
    fn the_first_reading_has_nothing_to_compare() {
        let l = SystemLoad::default();
        assert_eq!(l.others_percent(), None);
    }
}
