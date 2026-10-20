//! Audio output behind a trait: the real cpal device, plus null and capture
//! outputs for tests (no sound device needed).
//!
//! Outputs *pull* from a [`Renderer`] (the engine's mixer) on their own
//! thread. One continuous stream is kept open across tracks, which is what
//! makes gapless joins possible.
//!
//! An output lists the computer's audio devices and can be pointed at one of
//! them by id, or follow the system default (no selection). When a stream
//! breaks, [`classify_loss`] decides what that means: the device is still
//! there (the default moved, or the stream was reset), so reopen and play on;
//! or it's gone, so pause. A chosen device that's gone also drops the choice,
//! so the next open falls back to the system default.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Produces interleaved f32 audio in the output format.
pub trait Renderer: Send + Sync + 'static {
    /// Fills `out` (interleaved). Returns how many frames came from actual
    /// audio; the remainder is silence (paused, idle or underrun).
    fn render(&self, out: &mut [f32]) -> usize;
    /// How long after its callback a rendered block starts playing, when
    /// the device says.
    fn set_latency(&self, _latency: Duration) {}
}

#[derive(Debug, Clone, PartialEq)]
pub enum OutputEvent {
    /// The device went away (unplugged, Bluetooth drop). The stream is dead.
    /// `fell_back_from` names a chosen device whose choice was dropped, so
    /// the next open uses the system default.
    DeviceLost {
        why: String,
        fell_back_from: Option<String>,
    },
    /// The stream broke but its device, or the new system default, is there:
    /// reopen and carry on.
    Reset(String),
    /// The stream moved to the new system default by itself and still runs.
    Rerouted,
}

pub type OutputCallback = Arc<dyn Fn(OutputEvent) + Send + Sync>;

/// An audio output device on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDevice {
    /// Stable id (survives restarts and replugging where the OS allows).
    pub id: String,
    pub name: String,
    /// The system default output right now.
    pub is_default: bool,
}

/// How the open stream holds its device (bit-perfect output).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Exclusive {
    /// Shared with other apps, through the system mixer.
    #[default]
    Off,
    /// The device is ours alone, at the music's own sample rate.
    #[serde(rename_all = "camelCase")]
    On { rate: u32, bits: u16 },
    /// Exclusive was asked for but the device is playing shared.
    #[serde(rename_all = "camelCase")]
    Fallback { reason: String },
}

impl Exclusive {
    pub fn is_on(&self) -> bool {
        matches!(self, Self::On { .. })
    }
}

/// `output_devices` result and `output://changed` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputState {
    pub devices: Vec<OutputDevice>,
    /// The chosen device id; None follows the system default.
    pub selected: Option<String>,
    /// The device playing, or the one the next play would open.
    pub current: Option<OutputDevice>,
    /// Bit-perfect state of the open stream.
    #[serde(default)]
    pub exclusive: Exclusive,
}

impl OutputState {
    pub fn build(
        devices: Vec<OutputDevice>,
        selected: Option<String>,
        opened: Option<OutputDevice>,
    ) -> Self {
        let mut devices = devices;
        let current = match opened {
            // The listed entry has a fresh default flag.
            Some(o) => Some(devices.iter().find(|d| d.id == o.id).cloned().unwrap_or(o)),
            None => pick_device(&devices, selected.as_deref()),
        };
        // The device playing is always listed, even if the system's list
        // missed it this time.
        if let Some(c) = &current
            && !devices.iter().any(|d| d.id == c.id)
        {
            devices.insert(0, c.clone());
        }
        Self {
            devices,
            selected,
            current,
            exclusive: Exclusive::Off,
        }
    }
}

/// The device an open uses: the chosen one if present, else the default.
pub fn pick_device(devices: &[OutputDevice], selected: Option<&str>) -> Option<OutputDevice> {
    selected
        .and_then(|id| devices.iter().find(|d| d.id == id))
        .or_else(|| devices.iter().find(|d| d.is_default))
        .cloned()
}

/// What a broken stream means, given the devices present now. Clears
/// `selected` when the chosen device is the one that went away.
pub fn classify_loss(
    why: String,
    selected: &Mutex<Option<String>>,
    opened: Option<&OutputDevice>,
    present: &[OutputDevice],
) -> OutputEvent {
    let still_there = opened.is_some_and(|o| present.iter().any(|d| d.id == o.id));
    if still_there {
        return OutputEvent::Reset(why);
    }
    let mut sel = selected.lock();
    let fell_back_from = match (sel.as_deref(), opened) {
        (Some(id), Some(o)) if id == o.id => {
            *sel = None;
            Some(o.name.clone())
        }
        _ => None,
    };
    OutputEvent::DeviceLost {
        why,
        fell_back_from,
    }
}

pub trait AudioOutput: Send {
    /// Opens the selected device (or the system default) and starts pulling
    /// from `renderer`.
    fn open(&mut self, renderer: Arc<dyn Renderer>, on_event: OutputCallback)
    -> AppResult<OutputFormat>;
    /// Stops pulling and releases the device.
    fn close(&mut self);
    fn is_open(&self) -> bool;
    /// Output devices on this computer (empty when there's nothing to pick).
    fn devices(&self) -> Vec<OutputDevice> {
        Vec::new()
    }
    /// The device the next `open` uses; None follows the system default.
    fn select(&mut self, _id: Option<String>) {}
    fn selected(&self) -> Option<String> {
        None
    }
    /// The device the open stream plays on.
    fn opened(&self) -> Option<OutputDevice> {
        None
    }
    /// Hold the open stream still while nothing plays (true) or start it
    /// pulling again (false). The device stays open, so resuming is quick,
    /// but a paused player no longer wakes on every device period.
    fn set_idle(&mut self, _idle: bool) {}
    /// Devices (by id) that open in exclusive mode for bit-perfect output.
    fn set_exclusive_devices(&mut self, _ids: Vec<String>) {}
    /// Whether the device the next `open` uses is one of those.
    fn exclusive_wanted(&self) -> bool {
        false
    }
    /// The sample rate an exclusive open asks for (the music's own).
    fn set_preferred_rate(&mut self, _rate: Option<u32>) {}
    /// How the open stream holds its device.
    fn exclusive(&self) -> Exclusive {
        Exclusive::Off
    }
}

/// Whether `device` is in the exclusive list.
pub fn wants_exclusive(ids: &[String], device: Option<&OutputDevice>) -> bool {
    device.is_some_and(|d| ids.contains(&d.id))
}

// ---------------------------------------------------------------- capture

#[derive(Default)]
struct CaptureShared {
    samples: Mutex<Vec<f32>>,
    stop: AtomicBool,
    /// Delivered by the output thread; a loss or reset ends the stream.
    pending: Mutex<Option<OutputEvent>>,
    /// Fake devices (none: one anonymous device).
    devices: Mutex<Vec<OutputDevice>>,
    selected: Mutex<Option<String>>,
    opened: Mutex<Option<OutputDevice>>,
    opens: AtomicUsize,
    /// Held still by `set_idle`: pulls nothing.
    idle: AtomicBool,
    exclusive_ids: Mutex<Vec<String>>,
    preferred_rate: Mutex<Option<u32>>,
    /// Exclusive opens fail with this reason (a busy device, say).
    exclusive_fails: Mutex<Option<String>>,
    exclusive: Mutex<Exclusive>,
    /// This many opens fail before they work again (a route still settling).
    fail_opens: AtomicUsize,
}

/// How far a paced capture may fall behind the clock before it stops
/// trying to catch up.
const MAX_PACE_DEBT: Duration = Duration::from_millis(50);

/// Test output: pulls as fast as audio is available and records only real
/// frames (silence from underruns/pauses is not recorded), so a capture is
/// exactly the sample stream the engine produced.
pub struct CaptureOutput {
    format: OutputFormat,
    shared: Arc<CaptureShared>,
    thread: Option<JoinHandle<()>>,
    /// Sleep between blocks; `None` = as fast as possible.
    pace: Option<Duration>,
}

/// Handle for tests to read captured audio or fake device changes.
#[derive(Clone)]
pub struct CaptureHandle(Arc<CaptureShared>);

impl CaptureHandle {
    pub fn samples(&self) -> Vec<f32> {
        self.0.samples.lock().clone()
    }
    pub fn len_frames(&self, channels: usize) -> usize {
        self.0.samples.lock().len() / channels
    }
    pub fn clear(&self) {
        self.0.samples.lock().clear();
    }
    /// Simulates unplugging the device.
    pub fn lose_device(&self) {
        *self.0.pending.lock() = Some(OutputEvent::DeviceLost {
            why: "capture device removed".into(),
            fell_back_from: None,
        });
    }
    /// Fake devices; the one marked default is the system default.
    pub fn set_devices(&self, devices: Vec<OutputDevice>) {
        *self.0.devices.lock() = devices;
    }
    /// Unplugs a fake device. If the stream is on it, it breaks the way a
    /// real one does.
    pub fn remove_device(&self, id: &str) {
        let present = {
            let mut d = self.0.devices.lock();
            let was_default = d.iter().any(|x| x.id == id && x.is_default);
            d.retain(|x| x.id != id);
            // The OS picks a new default when the default goes.
            if was_default && let Some(first) = d.first_mut() {
                first.is_default = true;
            }
            d.clone()
        };
        if self.0.opened.lock().as_ref().is_some_and(|o| o.id == id) {
            self.break_stream(format!("{id} removed"), &present);
        }
    }
    /// Changes the system default. A stream following it breaks (and the
    /// player reopens it on the new default).
    pub fn set_default(&self, id: &str) {
        let present = {
            let mut d = self.0.devices.lock();
            for x in d.iter_mut() {
                x.is_default = x.id == id;
            }
            d.clone()
        };
        if self.0.selected.lock().is_none() && self.0.opened.lock().is_some() {
            self.break_stream("default device changed".into(), &present);
        }
    }
    fn break_stream(&self, why: String, present: &[OutputDevice]) {
        let opened = self.0.opened.lock().clone();
        let ev = classify_loss(why, &self.0.selected, opened.as_ref(), present);
        *self.0.pending.lock() = Some(ev);
    }
    /// Id of the device the stream was last opened on.
    pub fn opened_id(&self) -> Option<String> {
        self.0.opened.lock().as_ref().map(|d| d.id.clone())
    }
    /// How many times a stream was opened.
    pub fn opens(&self) -> usize {
        self.0.opens.load(Ordering::SeqCst)
    }
    /// Whether the stream is held still (see `AudioOutput::set_idle`).
    pub fn is_idle(&self) -> bool {
        self.0.idle.load(Ordering::SeqCst)
    }
    /// The stream resets (an audio route change): the player reopens it.
    pub fn reset_stream(&self) {
        *self.0.pending.lock() = Some(OutputEvent::Reset("route changed".into()));
    }
    /// The next `n` opens fail, the way an output does while a new route
    /// (a car, AirPods) is still coming up.
    pub fn fail_opens(&self, n: usize) {
        self.0.fail_opens.store(n, Ordering::SeqCst);
    }
    /// Failing opens not used up yet.
    pub fn fail_opens_left(&self) -> usize {
        self.0.fail_opens.load(Ordering::SeqCst)
    }
    /// Makes exclusive opens fall back to shared with `reason` (None: they work).
    pub fn fail_exclusive(&self, reason: Option<&str>) {
        *self.0.exclusive_fails.lock() = reason.map(str::to_string);
    }
}

impl CaptureOutput {
    pub fn new(format: OutputFormat) -> (Self, CaptureHandle) {
        let shared = Arc::new(CaptureShared::default());
        (
            Self {
                format,
                shared: shared.clone(),
                thread: None,
                pace: None,
            },
            CaptureHandle(shared),
        )
    }

    /// Pull roughly in real time instead of as fast as possible.
    pub fn realtime(mut self) -> Self {
        self.pace = Some(Duration::from_millis(1));
        self
    }
}

impl AudioOutput for CaptureOutput {
    fn open(
        &mut self,
        renderer: Arc<dyn Renderer>,
        on_event: OutputCallback,
    ) -> AppResult<OutputFormat> {
        self.close();
        // Test-only: one thread opens at a time.
        if self.shared.fail_opens.load(Ordering::SeqCst) > 0 {
            self.shared.fail_opens.fetch_sub(1, Ordering::SeqCst);
            return Err(AppError::Io("audio device did not open in time".into()));
        }
        self.shared.stop.store(false, Ordering::SeqCst);
        *self.shared.pending.lock() = None;
        {
            let devices = self.shared.devices.lock();
            let sel = self.shared.selected.lock();
            *self.shared.opened.lock() = pick_device(&devices, sel.as_deref());
        }
        self.shared.opens.fetch_add(1, Ordering::SeqCst);
        self.shared.idle.store(false, Ordering::SeqCst);
        let mut fmt = self.format;
        *self.shared.exclusive.lock() = if self.exclusive_wanted() {
            match self.shared.exclusive_fails.lock().clone() {
                Some(reason) => Exclusive::Fallback { reason },
                None => {
                    fmt.sample_rate = self.shared.preferred_rate.lock().unwrap_or(fmt.sample_rate);
                    Exclusive::On { rate: fmt.sample_rate, bits: 32 }
                }
            }
        } else {
            Exclusive::Off
        };
        let shared = self.shared.clone();
        let pace = self.pace;
        self.thread = Some(std::thread::spawn(move || {
            let ch = fmt.channels as usize;
            let block = 256;
            let mut buf = vec![0.0f32; block * ch];
            let frame_time = Duration::from_secs_f64(block as f64 / fmt.sample_rate as f64);
            let mut due = Instant::now();
            while !shared.stop.load(Ordering::SeqCst) {
                let ev = shared.pending.lock().take();
                if let Some(ev) = ev {
                    let ends = !matches!(ev, OutputEvent::Rerouted);
                    on_event(ev);
                    if ends {
                        return;
                    }
                }
                if shared.idle.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                // The capture is locked across the render: events sent from
                // inside it ("ended", "drained") reach a test before the
                // block is stored, and reading the capture then waits for it.
                // Nothing between this block and the "listener": no extra delay.
                renderer.set_latency(Duration::ZERO);
                let filled = {
                    let mut samples = shared.samples.lock();
                    let filled = renderer.render(&mut buf);
                    samples.extend_from_slice(&buf[..filled * ch]);
                    filled
                };
                match pace {
                    // Paced to the wall clock like a device: each block is
                    // due one block's time after the last, so oversleeping
                    // (coarse timers, a loaded machine) doesn't add up into
                    // slow motion. After a long stall it resumes from now
                    // rather than rushing to catch up.
                    Some(_) => {
                        due += frame_time;
                        let now = Instant::now();
                        if due > now {
                            std::thread::sleep(due - now);
                        } else if now - due > MAX_PACE_DEBT {
                            due = now;
                        }
                    }
                    None if filled == 0 => std::thread::sleep(Duration::from_micros(500)),
                    None => {}
                }
            }
        }));
        Ok(fmt)
    }

    fn close(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    fn is_open(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    fn devices(&self) -> Vec<OutputDevice> {
        self.shared.devices.lock().clone()
    }

    fn select(&mut self, id: Option<String>) {
        *self.shared.selected.lock() = id;
    }

    fn selected(&self) -> Option<String> {
        self.shared.selected.lock().clone()
    }

    fn opened(&self) -> Option<OutputDevice> {
        if self.is_open() {
            self.shared.opened.lock().clone()
        } else {
            None
        }
    }

    fn set_idle(&mut self, idle: bool) {
        self.shared.idle.store(idle, Ordering::SeqCst);
    }

    fn set_exclusive_devices(&mut self, ids: Vec<String>) {
        *self.shared.exclusive_ids.lock() = ids;
    }

    fn exclusive_wanted(&self) -> bool {
        let devices = self.shared.devices.lock();
        let next = pick_device(&devices, self.shared.selected.lock().as_deref());
        wants_exclusive(&self.shared.exclusive_ids.lock(), next.as_ref())
    }

    fn set_preferred_rate(&mut self, rate: Option<u32>) {
        *self.shared.preferred_rate.lock() = rate;
    }

    fn exclusive(&self) -> Exclusive {
        if self.is_open() { self.shared.exclusive.lock().clone() } else { Exclusive::Off }
    }
}

impl Drop for CaptureOutput {
    fn drop(&mut self) {
        self.close();
    }
}

/// Discards audio in real time. Useful when no device is wanted.
pub fn null_output() -> CaptureOutput {
    CaptureOutput::new(OutputFormat {
        sample_rate: 48_000,
        channels: 2,
    })
    .0
    .realtime()
}

// ---------------------------------------------------------------- cpal

/// An output device via cpal (WASAPI / CoreAudio): the chosen one, or the
/// system default. The cpal stream lives on its own thread (it isn't `Send`
/// everywhere).
#[derive(Default)]
pub struct CpalOutput {
    stop_tx: Option<mpsc::Sender<StreamCtl>>,
    thread: Option<JoinHandle<()>>,
    /// Closes when the stream thread ends.
    done_rx: Option<mpsc::Receiver<()>>,
    /// Name and sample format of the last opened device (diagnostics).
    pub last_device: Arc<Mutex<Option<(String, String)>>>,
    selected: Arc<Mutex<Option<String>>>,
    opened: Arc<Mutex<Option<OutputDevice>>>,
    /// Devices that open exclusively (bit-perfect).
    exclusive_ids: Arc<Mutex<Vec<String>>>,
    preferred_rate: Option<u32>,
    exclusive: Arc<Mutex<Exclusive>>,
}

/// What the stream thread is told: close, or hold still / pull again.
pub(crate) enum StreamCtl {
    Stop,
    Idle(bool),
}

impl CpalOutput {
    pub fn new() -> Self {
        Self::default()
    }
}

/// One device as the system reports it, before it becomes a list entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enumerated {
    /// The system's stable id, if it gave one.
    pub id: Option<String>,
    /// The system's name, if it gave one.
    pub name: Option<String>,
    /// It can play sound (inputs report false).
    pub output: bool,
}

impl Enumerated {
    fn entry(&self, default_id: Option<&str>) -> Option<OutputDevice> {
        let name = self.name.clone().or_else(|| self.id.clone())?;
        let id = self.id.clone().unwrap_or_else(|| name.clone());
        Some(OutputDevice {
            is_default: default_id == Some(id.as_str()),
            id,
            name,
        })
    }
}

/// The output list from what the system reports: every device that can play
/// sound, in the system's order, virtual ones included and each once. The
/// default output is always in it and marked, even when the system's own
/// output check skipped it (a device that is busy can fail that check).
pub fn list_outputs(all: Vec<Enumerated>, default: Option<Enumerated>) -> Vec<OutputDevice> {
    let default_entry = default.as_ref().and_then(|d| d.entry(None));
    let default_id = default_entry.as_ref().map(|d| d.id.clone());
    let mut out: Vec<OutputDevice> = Vec::new();
    for e in all.iter().filter(|e| e.output) {
        if let Some(d) = e.entry(default_id.as_deref())
            && !out.iter().any(|o| o.id == d.id)
        {
            out.push(d);
        }
    }
    if let Some(mut d) = default_entry
        && !out.iter().any(|o| o.id == d.id)
    {
        d.is_default = true;
        out.insert(0, d);
    }
    out
}

fn cpal_enumerated(device: &cpal::Device, output: bool) -> Enumerated {
    use cpal::traits::DeviceTrait;
    Enumerated {
        id: device.id().ok().map(|i| i.to_string()),
        name: device.description().ok().map(|d| d.name().to_string()),
        output,
    }
}

fn cpal_device_info(device: &cpal::Device, default_id: Option<&str>) -> Option<OutputDevice> {
    cpal_enumerated(device, true).entry(default_id)
}

fn cpal_default_id(host: &cpal::Host) -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    host.default_output_device()
        .and_then(|d| d.id().ok())
        .map(|i| i.to_string())
}

/// Lists output devices. Runs on the calling thread.
fn cpal_list() -> Vec<OutputDevice> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let default = host.default_output_device().map(|d| cpal_enumerated(&d, true));
    let all: Vec<Enumerated> = match host.devices() {
        Ok(devices) => devices.map(|d| { let out = d.supports_output(); cpal_enumerated(&d, out) }).collect(),
        Err(e) => {
            tracing::warn!("could not list audio devices: {e}");
            Vec::new()
        }
    };
    list_outputs(all, default)
}

/// How long opening a device may take before giving up on it.
const OPEN_WAIT: Duration = Duration::from_secs(6);
/// How long closing or listing may take; a stuck driver is left to finish
/// on its own.
const CLOSE_WAIT: Duration = Duration::from_secs(3);

/// Lists on a short-lived thread, so COM setup on Windows stays off the
/// caller's (async runtime) thread, and a stuck driver can't hold it.
fn cpal_list_isolated() -> Vec<OutputDevice> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("audio-devices".into()).spawn(move || {
        let _ = tx.send(cpal_list());
    });
    if spawned.is_err() {
        return Vec::new();
    }
    rx.recv_timeout(CLOSE_WAIT).unwrap_or_else(|_| {
        tracing::warn!("listing audio devices didn't return in time");
        Vec::new()
    })
}

#[derive(Clone)]
struct StreamCtx {
    on_event: OutputCallback,
    selected: Arc<Mutex<Option<String>>>,
    opened: Arc<Mutex<Option<OutputDevice>>>,
}

/// A stream on `device` in `supported`'s format.
fn open_stream(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    renderer: Arc<dyn Renderer>,
    ctx: StreamCtx,
) -> Result<cpal::Stream, String> {
    use cpal::SampleFormat as F;
    let config = supported.config();
    let built = match supported.sample_format() {
        F::F32 => build_stream::<f32>(device, config, renderer, ctx),
        F::I16 => build_stream::<i16>(device, config, renderer, ctx),
        F::U16 => build_stream::<u16>(device, config, renderer, ctx),
        F::I32 => build_stream::<i32>(device, config, renderer, ctx),
        F::U32 => build_stream::<u32>(device, config, renderer, ctx),
        F::I8 => build_stream::<i8>(device, config, renderer, ctx),
        F::U8 => build_stream::<u8>(device, config, renderer, ctx),
        F::F64 => build_stream::<f64>(device, config, renderer, ctx),
        other => return Err(format!("unsupported device sample format {other:?}")),
    };
    built.map_err(|e| e.to_string())
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    renderer: Arc<dyn Renderer>,
    ctx: StreamCtx,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let mut scratch: Vec<f32> = Vec::with_capacity(8192);
    let broken = Arc::new(AtomicBool::new(false));
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            if scratch.len() != data.len() {
                scratch.resize(data.len(), 0.0);
            }
            let ts = info.timestamp();
            renderer.set_latency(ts.playback.duration_since(ts.callback));
            renderer.render(&mut scratch);
            for (d, s) in data.iter_mut().zip(scratch.iter()) {
                *d = T::from_sample(*s);
            }
        },
        move |err: cpal::Error| {
            use cpal::ErrorKind as K;
            let kind = err.kind();
            match kind {
                K::DeviceNotAvailable | K::StreamInvalidated | K::DeviceChanged => {
                    let rerouted = matches!(kind, K::DeviceChanged);
                    if !rerouted && broken.swap(true, Ordering::SeqCst) {
                        return;
                    }
                    let why = err.to_string();
                    let on_event = ctx.on_event.clone();
                    let selected = ctx.selected.clone();
                    let opened = ctx.opened.clone();
                    let broken = broken.clone();
                    // Enumerating devices is slow: never on the audio thread.
                    let _ = std::thread::Builder::new()
                        .name("audio-device-change".into())
                        .spawn(move || {
                            let present = cpal_list();
                            let was = opened.lock().clone();
                            let still_there =
                                was.as_ref().is_some_and(|o| present.iter().any(|d| d.id == o.id));
                            if rerouted && still_there {
                                // Still playing, now on the new default.
                                *opened.lock() = present.iter().find(|d| d.is_default).cloned();
                                on_event(OutputEvent::Rerouted);
                                return;
                            }
                            broken.store(true, Ordering::SeqCst);
                            on_event(classify_loss(why, &selected, was.as_ref(), &present));
                        });
                }
                K::Xrun => tracing::debug!("audio xrun"),
                _ => tracing::warn!("audio output error: {err}"),
            }
        },
        None,
    )
}

/// After a rate change: the device as it is now (when found again), at `rate`.
fn refind_config(device: &mut cpal::Device, found: Option<cpal::Device>, rate: u32) -> Option<cpal::SupportedStreamConfig> {
    if let Some(d) = found {
        *device = d;
    }
    config_at(device, rate)
}

/// A stereo config at `rate`, f32 if the device has it.
fn config_at(device: &cpal::Device, rate: u32) -> Option<cpal::SupportedStreamConfig> {
    use cpal::traits::DeviceTrait;
    let mut fits: Vec<cpal::SupportedStreamConfig> = device
        .supported_output_configs()
        .ok()?
        .filter(|c| c.channels() == 2)
        .filter_map(|c| c.try_with_sample_rate(rate))
        .collect();
    fits.sort_by_key(|c| c.sample_format() != cpal::SampleFormat::F32);
    fits.into_iter().next()
}

impl AudioOutput for CpalOutput {
    fn open(
        &mut self,
        renderer: Arc<dyn Renderer>,
        on_event: OutputCallback,
    ) -> AppResult<OutputFormat> {
        self.close();
        let (ready_tx, ready_rx) = mpsc::channel::<AppResult<OutputFormat>>();
        let (stop_tx, stop_rx) = mpsc::channel::<StreamCtl>();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let last = self.last_device.clone();
        let selected = self.selected.clone();
        let opened = self.opened.clone();
        let exclusive_ids = self.exclusive_ids.lock().clone();
        let preferred_rate = self.preferred_rate;
        let exclusive = self.exclusive.clone();
        *exclusive.lock() = Exclusive::Off;
        let thread = std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || {
                // Dropped when this thread ends, however it ends.
                let _done = done_tx;
                use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
                let host = cpal::default_host();
                let want = selected.lock().clone();
                // A chosen device that isn't there falls back to the default.
                let chosen = want
                    .as_deref()
                    .and_then(|id| id.parse::<cpal::DeviceId>().ok())
                    .and_then(|id| host.device_by_id(&id));
                // Looked up again after a bit-perfect rate change: some devices
                // come back as a new object, and the old handle no longer works.
                let find = || {
                    want.as_deref()
                        .and_then(|id| id.parse::<cpal::DeviceId>().ok())
                        .and_then(|id| host.device_by_id(&id))
                        .or_else(|| host.default_output_device())
                };
                let Some(mut device) = chosen.or_else(|| host.default_output_device()) else {
                    let _ = ready_tx.send(Err(AppError::Io("no audio output device".into())));
                    return;
                };
                let default_config = match device.default_output_config() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(AppError::Io(format!("audio device: {e}"))));
                        return;
                    }
                };
                let info = cpal_device_info(&device, cpal_default_id(&host).as_deref());
                // Bit-perfect: the device on its own, at the music's rate.
                let mut supported = default_config;
                let mut _hog = None;
                if let Some(d) = info.as_ref().filter(|d| wants_exclusive(&exclusive_ids, Some(d))) {
                    let rate = preferred_rate.unwrap_or(default_config.sample_rate());
                    match super::exclusive::open(&d.id, rate, 2) {
                        Ok(super::exclusive::Opened::Own { stream, rate, channels, bits }) => {
                            *exclusive.lock() = Exclusive::On { rate, bits };
                            *last.lock() = Some((d.name.clone(), format!("exclusive {bits}-bit")));
                            *opened.lock() = info.clone();
                            let _ = ready_tx.send(Ok(OutputFormat { sample_rate: rate, channels }));
                            let lost_opened = opened.clone();
                            let lost: Box<dyn FnOnce(String) + Send> = Box::new(move |why| {
                                let present = cpal_list();
                                let was = lost_opened.lock().clone();
                                on_event(classify_loss(why, &selected, was.as_ref(), &present));
                            });
                            stream.run(renderer, stop_rx, lost);
                            return;
                        }
                        Ok(super::exclusive::Opened::Device { guard, rate, bits }) => match refind_config(&mut device, find(), rate) {
                            Some(c) => {
                                supported = c;
                                _hog = Some(guard);
                                *exclusive.lock() = Exclusive::On { rate, bits };
                            }
                            None => {
                                *exclusive.lock() = Exclusive::Fallback {
                                    reason: format!("The device can't play {rate} Hz directly."),
                                };
                            }
                        },
                        Err(reason) => {
                            tracing::info!("bit-perfect output fell back to shared: {reason}");
                            *exclusive.lock() = Exclusive::Fallback { reason };
                        }
                    }
                }
                let ctx = StreamCtx {
                    on_event,
                    selected,
                    opened: opened.clone(),
                };
                let mut stream = open_stream(&device, &supported, renderer.clone(), ctx.clone());
                // The device on its own wouldn't open a stream: give it back
                // and play shared, saying why, instead of not playing at all.
                if let (Err(e), Some(_)) = (&stream, &_hog) {
                    tracing::info!("bit-perfect stream failed, playing shared: {e}");
                    _hog = None;
                    *exclusive.lock() = Exclusive::Fallback {
                        reason: "The device wouldn't play on its own, so it plays shared.".into(),
                    };
                    if let Some(d) = find() {
                        device = d;
                    }
                    match device.default_output_config() {
                        Ok(c) => {
                            supported = c;
                            stream = open_stream(&device, &supported, renderer.clone(), ctx.clone());
                        }
                        Err(e) => stream = Err(format!("audio device: {e}")),
                    }
                }
                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(AppError::Io(format!("open audio stream: {e}"))));
                        return;
                    }
                };
                let config = supported.config();
                let fmt = OutputFormat {
                    sample_rate: config.sample_rate,
                    channels: config.channels,
                };
                let name = info.as_ref().map_or_else(|| "unknown device".into(), |d| d.name.clone());
                *last.lock() = Some((name, format!("{:?}", supported.sample_format())));
                *opened.lock() = info;
                if let Err(e) = stream.play() {
                    let _ = ready_tx.send(Err(AppError::Io(format!("start audio stream: {e}"))));
                    return;
                }
                let _ = ready_tx.send(Ok(fmt));
                // Keep the stream alive until asked to stop. While idle it is
                // paused, so the device callback stops firing.
                while let Ok(StreamCtl::Idle(idle)) = stop_rx.recv() {
                    let r = if idle { stream.pause() } else { stream.play() };
                    if let Err(e) = r {
                        tracing::debug!("audio stream {}: {e}", if idle { "pause" } else { "resume" });
                    }
                }
                drop(stream);
            })
            .map_err(|e| AppError::Internal(e.to_string()))?;
        match ready_rx.recv_timeout(OPEN_WAIT) {
            Ok(Ok(fmt)) => {
                self.stop_tx = Some(stop_tx);
                self.thread = Some(thread);
                self.done_rx = Some(done_rx);
                Ok(fmt)
            }
            Ok(Err(e)) => {
                if done_rx.recv_timeout(CLOSE_WAIT).is_err_and(|e| e == mpsc::RecvTimeoutError::Disconnected) {
                    let _ = thread.join();
                }
                *self.opened.lock() = None;
                Err(e)
            }
            Err(_) => {
                drop(stop_tx);
                *self.opened.lock() = None;
                Err(AppError::Io("audio device did not open in time".into()))
            }
        }
    }

    fn close(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(StreamCtl::Stop);
        }
        if let Some(t) = self.thread.take() {
            let ended = self
                .done_rx
                .take()
                .is_none_or(|rx| rx.recv_timeout(CLOSE_WAIT) != Err(mpsc::RecvTimeoutError::Timeout));
            if ended {
                let _ = t.join();
            } else {
                // A driver stuck in its teardown: let it finish on its own.
                tracing::warn!("audio stream didn't close in time; left to finish");
            }
        }
        *self.opened.lock() = None;
    }

    fn is_open(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    fn devices(&self) -> Vec<OutputDevice> {
        cpal_list_isolated()
    }

    fn select(&mut self, id: Option<String>) {
        *self.selected.lock() = id;
    }

    fn selected(&self) -> Option<String> {
        self.selected.lock().clone()
    }

    fn opened(&self) -> Option<OutputDevice> {
        if self.is_open() {
            self.opened.lock().clone()
        } else {
            None
        }
    }

    fn set_idle(&mut self, idle: bool) {
        if let Some(tx) = &self.stop_tx {
            let _ = tx.send(StreamCtl::Idle(idle));
        }
    }

    fn set_exclusive_devices(&mut self, ids: Vec<String>) {
        *self.exclusive_ids.lock() = ids;
    }

    fn exclusive_wanted(&self) -> bool {
        let ids = self.exclusive_ids.lock().clone();
        if ids.is_empty() {
            return false;
        }
        if let Some(o) = self.opened() {
            // The open stream's device is the one a reopen would use,
            // unless the choice moved since.
            let sel = self.selected.lock().clone();
            if sel.is_none() || sel.as_deref() == Some(o.id.as_str()) {
                return wants_exclusive(&ids, Some(&o));
            }
        }
        let devices = cpal_list_isolated();
        wants_exclusive(&ids, pick_device(&devices, self.selected.lock().as_deref()).as_ref())
    }

    fn set_preferred_rate(&mut self, rate: Option<u32>) {
        self.preferred_rate = rate;
    }

    fn exclusive(&self) -> Exclusive {
        if self.is_open() { self.exclusive.lock().clone() } else { Exclusive::Off }
    }
}

impl Drop for CpalOutput {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, default: bool) -> OutputDevice {
        OutputDevice {
            id: id.into(),
            name: format!("{id} name"),
            is_default: default,
        }
    }

    #[test]
    fn a_chosen_device_that_goes_away_drops_the_choice() {
        let sel = Mutex::new(Some("usb".to_string()));
        let ev = classify_loss("gone".into(), &sel, Some(&dev("usb", false)), &[dev("spk", true)]);
        assert_eq!(
            ev,
            OutputEvent::DeviceLost {
                why: "gone".into(),
                fell_back_from: Some("usb name".into())
            }
        );
        assert_eq!(*sel.lock(), None);
    }

    #[test]
    fn the_default_going_away_pauses_without_a_fallback_notice() {
        let sel = Mutex::new(None);
        let ev = classify_loss("gone".into(), &sel, Some(&dev("hp", true)), &[dev("spk", true)]);
        assert!(matches!(ev, OutputEvent::DeviceLost { fell_back_from: None, .. }));
    }

    #[test]
    fn a_device_still_present_reopens() {
        // The default moved from spk to hp; spk is still plugged in.
        let sel = Mutex::new(None);
        let ev = classify_loss("moved".into(), &sel, Some(&dev("spk", false)), &[dev("spk", false), dev("hp", true)]);
        assert_eq!(ev, OutputEvent::Reset("moved".into()));
        // A chosen device that's still there reopens and keeps the choice.
        let sel = Mutex::new(Some("usb".to_string()));
        let ev = classify_loss("reset".into(), &sel, Some(&dev("usb", false)), &[dev("usb", false), dev("spk", true)]);
        assert_eq!(ev, OutputEvent::Reset("reset".into()));
        assert_eq!(sel.lock().as_deref(), Some("usb"));
    }

    #[test]
    fn nothing_left_is_a_loss() {
        let sel = Mutex::new(None);
        assert!(matches!(
            classify_loss("x".into(), &sel, Some(&dev("spk", true)), &[]),
            OutputEvent::DeviceLost { .. }
        ));
        assert!(matches!(classify_loss("x".into(), &sel, None, &[dev("spk", true)]), OutputEvent::DeviceLost { .. }));
    }

    #[test]
    fn state_reports_the_device_that_would_open() {
        let list = vec![dev("spk", true), dev("usb", false)];
        let s = OutputState::build(list.clone(), None, None);
        assert_eq!(s.current.unwrap().id, "spk");
        let s = OutputState::build(list.clone(), Some("usb".into()), None);
        assert_eq!(s.current.unwrap().id, "usb");
        // Chosen but unplugged: the default would open.
        let s = OutputState::build(vec![dev("spk", true)], Some("usb".into()), None);
        assert_eq!(s.current.unwrap().id, "spk");
        // Open: the opened device, with a fresh default flag.
        let s = OutputState::build(list, None, Some(dev("usb", true)));
        assert_eq!(s.current, Some(dev("usb", false)));
    }

    fn en(id: &str, name: &str, output: bool) -> Enumerated {
        Enumerated {
            id: Some(id.into()),
            name: Some(name.into()),
            output,
        }
    }

    #[test]
    fn list_keeps_every_output_with_the_default_marked() {
        // Mac-style: built-in speakers (default and in use), a meeting
        // app's virtual device, a loopback cable, the built-in microphone.
        let all = vec![
            en("BuiltInMicrophoneDevice", "MacBook Pro Microphone", false),
            en("BuiltInSpeakerDevice", "MacBook Pro Speakers", true),
            en("MSLoopbackDriverDevice_UID", "Microsoft Teams Audio", true),
            en("BlackHole2ch_UID", "BlackHole 2ch", true),
        ];
        let def = en("BuiltInSpeakerDevice", "MacBook Pro Speakers", true);
        let list = list_outputs(all, Some(def));
        let names: Vec<&str> = list.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["MacBook Pro Speakers", "Microsoft Teams Audio", "BlackHole 2ch"]);
        let defaults: Vec<&str> = list.iter().filter(|d| d.is_default).map(|d| d.id.as_str()).collect();
        assert_eq!(defaults, ["BuiltInSpeakerDevice"]);
    }

    #[test]
    fn list_adds_the_default_when_the_output_check_skipped_it() {
        // The speakers in use failed the system's output check, so only the
        // virtual device came back.
        let all = vec![
            en("BuiltInSpeakerDevice", "MacBook Pro Speakers", false),
            en("MSLoopbackDriverDevice_UID", "Microsoft Teams Audio", true),
        ];
        let list = list_outputs(all, Some(en("BuiltInSpeakerDevice", "MacBook Pro Speakers", true)));
        assert_eq!(
            list[0],
            OutputDevice { id: "BuiltInSpeakerDevice".into(), name: "MacBook Pro Speakers".into(), is_default: true }
        );
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn list_has_each_device_once_and_names_unnamed_ones() {
        let all = vec![
            en("a", "Speakers", true),
            en("b", "Headphones", true),
            en("a", "Speakers", true),
            Enumerated { id: Some("c".into()), name: None, output: true },
            Enumerated { id: None, name: None, output: true },
        ];
        let list = list_outputs(all, None);
        let ids: Vec<&str> = list.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(list[2].name, "c");
        assert!(list.iter().all(|d| !d.is_default));
    }

    #[test]
    fn state_lists_the_device_in_use_even_if_the_list_missed_it() {
        let s = OutputState::build(vec![dev("teams", false)], None, Some(dev("spk", true)));
        let ids: Vec<&str> = s.devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["spk", "teams"]);
        assert_eq!(s.current.unwrap().id, "spk");
    }

    #[test]
    fn state_json_is_camel_case() {
        let s = OutputState::build(vec![dev("spk", true)], None, None);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["devices"][0]["isDefault"], true);
        assert!(v["selected"].is_null());
        assert_eq!(v["current"]["id"], "spk");
    }
}
