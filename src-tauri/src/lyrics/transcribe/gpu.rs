//! Transcription on the graphics card (Vulkan on Windows and Linux, any
//! vendor; Metal on macOS). The app itself never touches the graphics card: the helper
//! process does, and a separate short-lived probe process lists the cards
//! once, so a broken driver can only stop a helper (and then the processor
//! is used for the rest of the session).
//!
//! Vulkan comes with the graphics driver, not with Platter. whisper.cpp's
//! Vulkan code asks the loader for everything through one function,
//! `vkGetInstanceProcAddr`, and this module provides it: it opens the
//! driver's loader (`vulkan-1.dll` or `libvulkan.so.1`) when a process is
//! allowed to use the card, and otherwise answers that there's no Vulkan.
//! So the binary doesn't depend on the loader at all, and Platter starts
//! on a computer without any Vulkan driver (transcription uses the
//! processor there).

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};

use super::engine::{self, Hints, Progress, RecognizeError, Recognizer, Stop};
use super::lines::Segment;
use super::model::ModelKind;

/// The first argument that makes Platter list the graphics cards and exit.
pub const PROBE_FLAG: &str = "--speech-probe";
/// Set to 1 in the helper's environment when it may use the graphics card.
pub const GPU_ENV: &str = "PLATTER_SPEECH_GPU";
/// A probe that hasn't answered by then counts as no graphics card.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// Cards reporting less memory than this can't hold the bigger models.
pub const MIN_MEMORY: u64 = 1024 * 1024 * 1024;

/// "Use the graphics card" in Settings (this device only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuMode {
    /// When there's a usable one.
    #[default]
    Auto,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    /// Its own chip and memory.
    Discrete,
    /// Part of the processor, sharing its memory.
    Integrated,
}

/// One graphics card the speech library can use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Its place among the cards (whisper.cpp's `gpu_device`).
    pub index: usize,
    pub name: String,
    pub kind: DeviceKind,
    /// Total memory in bytes (0: unknown).
    pub memory: u64,
}

/// The card to use: a discrete one over an integrated one, then the one
/// with the most memory, then the first. Cards too small are left out.
pub fn pick(devices: &[Device]) -> Option<&Device> {
    devices
        .iter()
        .filter(|d| d.memory == 0 || d.memory >= MIN_MEMORY)
        .max_by_key(|d| (d.kind == DeviceKind::Discrete, d.memory, std::cmp::Reverse(d.index)))
}

/// Lists the cards (the real one runs the probe process).
pub type Probe = Arc<dyn Fn() -> Vec<Device> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Found {
    NotYet,
    Looking,
    Done(Option<Device>),
}

/// The app's view of the graphics card: the setting, the card found and
/// whether it failed this session.
pub struct Gpu {
    probe: Probe,
    mode: Mutex<GpuMode>,
    found: Mutex<Found>,
    done: Condvar,
    failed: AtomicBool,
    on_found: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Gpu {
    pub fn new(probe: Probe) -> Arc<Self> {
        Arc::new(Self {
            probe,
            mode: Mutex::new(GpuMode::Auto),
            found: Mutex::new(Found::NotYet),
            done: Condvar::new(),
            failed: AtomicBool::new(false),
            on_found: Mutex::new(None),
        })
    }

    /// Never a graphics card (builds and computers without one, and tests).
    pub fn none() -> Arc<Self> {
        Self::new(Arc::new(Vec::new))
    }

    /// The probe process, for this binary. Nothing to look for in builds
    /// without transcription.
    pub fn for_this_build() -> Arc<Self> {
        if !cfg!(feature = "transcribe") || engine::unavailable_here().is_some() {
            return Self::none();
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("platter"));
        Self::new(Arc::new(move || run_probe(&exe)))
    }

    /// Called once the card is known (to send the new status out).
    pub fn on_found(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *self.on_found.lock() = Some(f);
    }

    pub fn set_mode(&self, mode: GpuMode) {
        *self.mode.lock() = mode;
    }

    pub fn mode(&self) -> GpuMode {
        *self.mode.lock()
    }

    /// Starts looking for the card in the background, once.
    pub fn look(self: &Arc<Self>) {
        {
            let mut f = self.found.lock();
            if *f != Found::NotYet {
                return;
            }
            *f = Found::Looking;
        }
        let me = self.clone();
        std::thread::spawn(move || me.finish_look());
    }

    fn finish_look(&self) {
        let devices = (self.probe)();
        let chosen = pick(&devices).cloned();
        match &chosen {
            Some(d) => tracing::info!(card = %d.name, kind = ?d.kind, "graphics card found for transcription"),
            None => tracing::info!(cards = devices.len(), "no graphics card for transcription, the processor is used"),
        }
        *self.found.lock() = Found::Done(chosen);
        self.done.notify_all();
        let cb = self.on_found.lock().clone();
        if let Some(cb) = cb {
            cb();
        }
    }

    /// Looks for the card if nobody has yet, and waits for the answer (at
    /// most `timeout`). Blocking.
    pub fn wait(self: &Arc<Self>, timeout: Duration) {
        self.look();
        let until = Instant::now() + timeout;
        let mut f = self.found.lock();
        while !matches!(*f, Found::Done(_)) {
            if self.done.wait_until(&mut f, until).timed_out() {
                return;
            }
        }
    }

    /// The card has been looked for.
    pub fn known(&self) -> bool {
        matches!(*self.found.lock(), Found::Done(_))
    }

    /// The usable card found, whether or not it's used.
    pub fn device(&self) -> Option<Device> {
        match &*self.found.lock() {
            Found::Done(d) => d.clone(),
            _ => None,
        }
    }

    /// The card transcription uses now: one was found, the setting is
    /// Auto and it hasn't failed this session.
    pub fn active(&self) -> Option<Device> {
        if self.mode() == GpuMode::Off || self.failed.load(Ordering::SeqCst) {
            return None;
        }
        self.device()
    }

    /// The card failed (the helper stopped while using it): the processor
    /// is used until Platter restarts.
    pub fn fail(&self) {
        if !self.failed.swap(true, Ordering::SeqCst) {
            tracing::warn!("the graphics card failed for transcription, using the processor until Platter restarts");
            let cb = self.on_found.lock().clone();
            if let Some(cb) = cb {
                cb();
            }
        }
    }
}

/// Runs `exe --speech-probe` and reads the cards it prints. A probe that
/// fails, crashes or hangs means no card.
pub fn run_probe(exe: &std::path::Path) -> Vec<Device> {
    let mut cmd = Command::new(exe);
    cmd.arg(PROBE_FLAG).env(GPU_ENV, "1").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    crate::platform::cpu::lower_command(&mut cmd, true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::info!("graphics card probe didn't start: {e}");
            return Vec::new();
        }
    };
    let mut out = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let until = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => {
                tracing::info!(%status, "graphics card probe failed");
                return Vec::new();
            }
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                tracing::info!("graphics card probe didn't answer");
                let _ = child.kill();
                let _ = child.wait();
                return Vec::new();
            }
        }
    }
    parse_probe(&reader.join().unwrap_or_default())
}

/// The probe's output: one JSON line with the cards.
pub fn parse_probe(out: &str) -> Vec<Device> {
    out.lines().rev().find_map(|l| serde_json::from_str::<Vec<Device>>(l.trim()).ok()).unwrap_or_default()
}

/// The probe process's whole life: prints the cards as one JSON line.
pub fn probe_main() -> i32 {
    let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).with_ansi(false).with_target(false).try_init();
    allow_from_env();
    let devices = if engine::unavailable_here().is_none() { list_devices() } else { Vec::new() };
    println!("{}", serde_json::to_string(&devices).unwrap_or_else(|_| "[]".into()));
    0
}

/// Lets this process use the graphics card when its environment says so.
/// Call before anything uses the speech library.
pub fn allow_from_env() {
    vulkan::allow(std::env::var(GPU_ENV).is_ok_and(|v| v == "1"));
}

/// This process may use the graphics card (the helper with the setting on).
pub fn allowed() -> bool {
    vulkan::allowed()
}

/// The cards the speech library sees, in its order.
#[cfg(feature = "transcribe")]
pub fn list_devices() -> Vec<Device> {
    use std::ffi::CStr;
    use whisper_rs::whisper_rs_sys as sys;
    let mut out = Vec::new();
    // SAFETY: plain calls into ggml's device registry; the strings it
    // returns live as long as the registry (the whole process).
    unsafe {
        for i in 0..sys::ggml_backend_dev_count() {
            let dev = sys::ggml_backend_dev_get(i);
            if dev.is_null() {
                continue;
            }
            let kind = match sys::ggml_backend_dev_type(dev) {
                sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU => DeviceKind::Discrete,
                sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU => DeviceKind::Integrated,
                _ => continue,
            };
            let text = |p: *const std::os::raw::c_char| if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().trim().to_owned() };
            let mut name = text(sys::ggml_backend_dev_description(dev));
            if name.is_empty() {
                name = text(sys::ggml_backend_dev_name(dev));
            }
            let (mut free, mut total) = (0usize, 0usize);
            sys::ggml_backend_dev_memory(dev, &mut free, &mut total);
            out.push(Device { index: out.len(), name, kind, memory: total as u64 });
        }
    }
    out
}

#[cfg(not(feature = "transcribe"))]
pub fn list_devices() -> Vec<Device> {
    Vec::new()
}

/// Where the lyrics check runs. The small model on the processor has a
/// short lane of its own (`own`, a helper with [`Helper::only`] Small), so
/// a check never waits behind a song. Anything else goes through the
/// transcription helper's priority lane (`shared`, [`Helper::priority`]),
/// ahead of background songs: a bigger model is never loaded twice, and on
/// the graphics card only one model sits on the card.
///
/// [`Helper::only`]: super::helper::Helper::only
/// [`Helper::priority`]: super::helper::Helper::priority
pub struct CheckRoute {
    pub gpu: Arc<Gpu>,
    pub own: Arc<dyn Recognizer>,
    pub shared: Arc<dyn Recognizer>,
}

impl CheckRoute {
    fn lane(&self, kind: ModelKind) -> &Arc<dyn Recognizer> {
        if kind != ModelKind::Small || self.gpu.active().is_some() { &self.shared } else { &self.own }
    }
}

impl Recognizer for CheckRoute {
    fn unavailable(&self) -> Option<String> {
        self.own.unavailable()
    }

    fn transcribe(
        &self,
        model: &std::path::Path,
        kind: ModelKind,
        pcm: &[f32],
        threads: usize,
        hints: &Hints,
        cancel: Stop,
        progress: Progress,
    ) -> Result<Vec<Segment>, RecognizeError> {
        if self.gpu.mode() == GpuMode::Auto && self.own.unavailable().is_none() {
            self.gpu.wait(PROBE_TIMEOUT);
        }
        let rec = self.lane(kind).clone();
        if !Arc::ptr_eq(&rec, &self.own) {
            // Its own lane may still hold a model from before.
            self.own.unload();
        }
        rec.transcribe(model, kind, pcm, threads, hints, cancel, progress)
    }

    /// Only its own: the transcription queue frees the shared one.
    fn unload(&self) {
        self.own.unload();
    }

    /// The lane the check's usual (small) runs take now.
    fn crashed_too_often(&self) -> bool {
        self.lane(ModelKind::Small).crashed_too_often()
    }
}

/// The loader stand-in (see the module comment). Besides
/// `vkGetInstanceProcAddr`, whisper.cpp's Vulkan code calls two functions
/// directly; they're passed on to the loader the same way (they're only
/// reached once Vulkan is running, so the loader is there).
#[cfg(all(feature = "transcribe", any(target_os = "windows", target_os = "linux")))]
mod vulkan {
    use std::ffi::{CStr, c_char, c_void};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};

    type VoidFn = unsafe extern "system" fn();
    type GetProc = unsafe extern "system" fn(*mut c_void, *const c_char) -> Option<VoidFn>;
    type Features2 = unsafe extern "system" fn(*mut c_void, *mut c_void);
    type CopyBuffer = unsafe extern "system" fn(*mut c_void, u64, u64, u32, *const c_void);

    static ALLOWED: AtomicBool = AtomicBool::new(false);
    static LOADER: OnceLock<Option<Loader>> = OnceLock::new();

    struct Loader {
        get_proc: GetProc,
        features2: Option<Features2>,
        copy_buffer: Option<CopyBuffer>,
    }

    pub fn allow(yes: bool) {
        ALLOWED.store(yes, Ordering::SeqCst);
        // Keeps these in the link ahead of the system loader (they're only
        // called from C).
        std::hint::black_box((
            vkGetInstanceProcAddr as GetProc,
            vkGetPhysicalDeviceFeatures2 as Features2,
            vkCmdCopyBuffer as CopyBuffer,
        ));
    }

    pub fn allowed() -> bool {
        ALLOWED.load(Ordering::SeqCst)
    }

    /// The driver's loader, opened once.
    fn loader() -> Option<&'static Loader> {
        LOADER
            .get_or_init(|| {
                // SAFETY: the symbols are the loader's exports with these signatures.
                let l = unsafe { open() };
                if l.is_none() {
                    tracing::info!("no Vulkan driver on this computer");
                }
                l
            })
            .as_ref()
    }

    /// # Safety
    /// `T` must be a function pointer type matching the export.
    unsafe fn cast<T: Copy>(f: *const c_void) -> Option<T> {
        // SAFETY: per the caller; a pointer and a function pointer are the same size.
        (!f.is_null()).then(|| unsafe { std::mem::transmute_copy::<*const c_void, T>(&f) })
    }

    #[cfg(target_os = "linux")]
    unsafe fn open() -> Option<Loader> {
        // SAFETY: dlopen and dlsym with constant names; the library stays
        // loaded for the life of the process.
        unsafe {
            let lib = libc::dlopen(c"libvulkan.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if lib.is_null() {
                return None;
            }
            let sym = |name: &CStr| libc::dlsym(lib, name.as_ptr()) as *const c_void;
            Some(Loader {
                get_proc: cast(sym(c"vkGetInstanceProcAddr"))?,
                features2: cast(sym(c"vkGetPhysicalDeviceFeatures2")),
                copy_buffer: cast(sym(c"vkCmdCopyBuffer")),
            })
        }
    }

    #[cfg(target_os = "windows")]
    unsafe fn open() -> Option<Loader> {
        use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
        use windows::core::{PCSTR, w};
        // SAFETY: the loader the driver installs in System32 (never one
        // next to Platter); it stays loaded for the life of the process.
        unsafe {
            let lib = LoadLibraryExW(w!("vulkan-1.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
            let sym = |name: &CStr| GetProcAddress(lib, PCSTR(name.as_ptr().cast())).map_or(std::ptr::null(), |f| f as *const c_void);
            Some(Loader {
                get_proc: cast(sym(c"vkGetInstanceProcAddr"))?,
                features2: cast(sym(c"vkGetPhysicalDeviceFeatures2")),
                copy_buffer: cast(sym(c"vkCmdCopyBuffer")),
            })
        }
    }

    fn real() -> Option<&'static Loader> {
        if allowed() { loader() } else { None }
    }

    /// `vkEnumerateInstanceVersion` without a loader: initialization
    /// failed, so the speech library goes on without Vulkan.
    unsafe extern "system" fn no_vulkan(_version: *mut u32) -> i32 {
        -3 // VK_ERROR_INITIALIZATION_FAILED
    }

    /// What whisper.cpp's Vulkan code links against.
    ///
    /// # Safety
    /// Called by the Vulkan code with a valid instance (or null) and a
    /// valid C string.
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn vkGetInstanceProcAddr(instance: *mut c_void, name: *const c_char) -> Option<VoidFn> {
        if let Some(l) = real() {
            // SAFETY: forwarded unchanged to the real loader.
            return unsafe { (l.get_proc)(instance, name) };
        }
        // SAFETY: the caller passes a valid C string.
        let asked = unsafe { CStr::from_ptr(name) };
        if instance.is_null() && asked == c"vkEnumerateInstanceVersion" {
            // SAFETY: Vulkan casts this back to its real type before calling.
            return Some(unsafe { std::mem::transmute::<unsafe extern "system" fn(*mut u32) -> i32, VoidFn>(no_vulkan) });
        }
        None
    }

    /// # Safety
    /// As the Vulkan function.
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn vkGetPhysicalDeviceFeatures2(device: *mut c_void, features: *mut c_void) {
        if let Some(f) = real().and_then(|l| l.features2) {
            // SAFETY: forwarded unchanged.
            unsafe { f(device, features) }
        }
    }

    /// # Safety
    /// As the Vulkan function.
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn vkCmdCopyBuffer(cmd: *mut c_void, src: u64, dst: u64, count: u32, regions: *const c_void) {
        if let Some(f) = real().and_then(|l| l.copy_buffer) {
            // SAFETY: forwarded unchanged.
            unsafe { f(cmd, src, dst, count, regions) }
        }
    }
}

#[cfg(not(all(feature = "transcribe", any(target_os = "windows", target_os = "linux"))))]
mod vulkan {
    use std::sync::atomic::{AtomicBool, Ordering};
    static ALLOWED: AtomicBool = AtomicBool::new(false);
    pub fn allow(yes: bool) {
        ALLOWED.store(yes, Ordering::SeqCst);
    }
    pub fn allowed() -> bool {
        ALLOWED.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(index: usize, name: &str, kind: DeviceKind, gb: u64) -> Device {
        Device { index, name: name.into(), kind, memory: gb << 30 }
    }

    #[test]
    fn a_discrete_card_beats_an_integrated_one() {
        let list = vec![dev(0, "Intel Graphics", DeviceKind::Integrated, 16), dev(1, "RTX 5090", DeviceKind::Discrete, 32)];
        assert_eq!(pick(&list).unwrap().name, "RTX 5090");
        assert_eq!(pick(&list).unwrap().index, 1, "whisper.cpp's gpu_device for it");
        // Two discrete cards: the one with more memory, then the first.
        let two = vec![dev(0, "Small", DeviceKind::Discrete, 4), dev(1, "Big", DeviceKind::Discrete, 16), dev(2, "Big too", DeviceKind::Discrete, 16)];
        assert_eq!(pick(&two).unwrap().name, "Big");
    }

    #[test]
    fn an_integrated_card_is_used_when_its_the_only_one() {
        let list = vec![dev(0, "Radeon Graphics", DeviceKind::Integrated, 8)];
        assert_eq!(pick(&list).unwrap().name, "Radeon Graphics");
    }

    #[test]
    fn no_card_or_too_little_memory_means_the_processor() {
        assert_eq!(pick(&[]), None);
        let tiny = vec![Device { index: 0, name: "Old".into(), kind: DeviceKind::Discrete, memory: 512 << 20 }];
        assert_eq!(pick(&tiny), None);
        // Unknown memory is given the benefit of the doubt.
        let unknown = vec![Device { index: 0, name: "Card".into(), kind: DeviceKind::Discrete, memory: 0 }];
        assert_eq!(pick(&unknown).unwrap().name, "Card");
    }

    #[test]
    fn the_probe_output_is_read_and_anything_else_is_no_card() {
        let list = vec![dev(0, "RTX 5090", DeviceKind::Discrete, 32)];
        let out = format!("ggml_vulkan: Found 1 Vulkan devices\n{}\n", serde_json::to_string(&list).unwrap());
        assert_eq!(parse_probe(&out), list);
        assert_eq!(parse_probe(""), vec![]);
        assert_eq!(parse_probe("Segmentation fault"), vec![]);
    }

    #[test]
    fn a_probe_that_cant_start_finds_no_card() {
        assert_eq!(run_probe(std::path::Path::new("/nonexistent/platter")), vec![]);
    }

    #[test]
    fn the_setting_and_a_failure_take_the_card_out_of_use() {
        let g = Gpu::new(Arc::new(|| vec![dev(0, "Integrated", DeviceKind::Integrated, 8), dev(1, "RTX", DeviceKind::Discrete, 16)]));
        assert!(!g.known());
        assert_eq!(g.active(), None, "not looked for yet");
        g.wait(Duration::from_secs(5));
        assert_eq!(g.active().unwrap().name, "RTX");
        g.set_mode(GpuMode::Off);
        assert_eq!(g.active(), None);
        assert_eq!(g.device().unwrap().name, "RTX", "still found, just not used");
        g.set_mode(GpuMode::Auto);
        let told = Arc::new(AtomicBool::new(false));
        let t = told.clone();
        g.on_found(Arc::new(move || t.store(true, Ordering::SeqCst)));
        g.fail();
        assert_eq!(g.active(), None, "a failure lasts the session");
        assert!(told.load(Ordering::SeqCst), "the status goes out");
        assert_eq!(Gpu::none().active(), None);
    }

    /// Counts runs (the check's own helper or the shared one).
    #[derive(Default)]
    struct Counting {
        runs: std::sync::atomic::AtomicUsize,
        unloads: std::sync::atomic::AtomicUsize,
    }

    impl Recognizer for Counting {
        fn unavailable(&self) -> Option<String> {
            None
        }
        fn transcribe(&self, _: &std::path::Path, _: ModelKind, _: &[f32], _: usize, _: &Hints, _: Stop, _: Progress) -> Result<Vec<Segment>, RecognizeError> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        }
        fn unload(&self) {
            self.unloads.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn the_check_shares_the_transcription_helper_on_the_card_or_for_a_bigger_model() {
        let g = Gpu::new(Arc::new(|| vec![dev(0, "RTX", DeviceKind::Discrete, 16)]));
        let (own, shared) = (Arc::new(Counting::default()), Arc::new(Counting::default()));
        let route = CheckRoute { gpu: g.clone(), own: own.clone(), shared: shared.clone() };
        let go = |kind| route.transcribe(std::path::Path::new("m.bin"), kind, &[0.0], 2, &Hints::english(), Stop::new(), Arc::new(|_| {}));
        let runs = || (own.runs.load(Ordering::SeqCst), shared.runs.load(Ordering::SeqCst));
        // The card is looked for first, then used: one helper, one model on it.
        go(ModelKind::Small).unwrap();
        assert_eq!(runs(), (0, 1));
        assert_eq!(own.unloads.load(Ordering::SeqCst), 1, "its own lane lets go of any model");
        route.unload();
        assert_eq!(shared.unloads.load(Ordering::SeqCst), 0, "the transcription queue frees the shared one");
        // On the processor the small model keeps its own lane.
        g.set_mode(GpuMode::Off);
        go(ModelKind::Small).unwrap();
        assert_eq!(runs(), (1, 1));
        // A bigger model is never loaded twice: the shared helper runs it.
        go(ModelKind::Medium).unwrap();
        go(ModelKind::Turbo).unwrap();
        assert_eq!(runs(), (1, 3));
        // A failure on the card: the processor, and the small lane again.
        g.set_mode(GpuMode::Auto);
        g.fail();
        go(ModelKind::Small).unwrap();
        assert_eq!(runs(), (2, 3));
    }
}
