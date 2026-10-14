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
use std::time::Duration;

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

/// `output_devices` result and `output://changed` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputState {
    pub devices: Vec<OutputDevice>,
    /// The chosen device id; None follows the system default.
    pub selected: Option<String>,
    /// The device playing, or the one the next play would open.
    pub current: Option<OutputDevice>,
}

impl OutputState {
    pub fn build(
        devices: Vec<OutputDevice>,
        selected: Option<String>,
        opened: Option<OutputDevice>,
    ) -> Self {
        let current = match opened {
            // The listed entry has a fresh default flag.
            Some(o) => Some(devices.iter().find(|d| d.id == o.id).cloned().unwrap_or(o)),
            None => pick_device(&devices, selected.as_deref()),
        };
        Self {
            devices,
            selected,
            current,
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
}

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
        self.shared.stop.store(false, Ordering::SeqCst);
        *self.shared.pending.lock() = None;
        {
            let devices = self.shared.devices.lock();
            let sel = self.shared.selected.lock();
            *self.shared.opened.lock() = pick_device(&devices, sel.as_deref());
        }
        self.shared.opens.fetch_add(1, Ordering::SeqCst);
        let shared = self.shared.clone();
        let fmt = self.format;
        let pace = self.pace;
        self.thread = Some(std::thread::spawn(move || {
            let ch = fmt.channels as usize;
            let block = 256;
            let mut buf = vec![0.0f32; block * ch];
            let frame_time = Duration::from_secs_f64(block as f64 / fmt.sample_rate as f64);
            while !shared.stop.load(Ordering::SeqCst) {
                let ev = shared.pending.lock().take();
                if let Some(ev) = ev {
                    let ends = !matches!(ev, OutputEvent::Rerouted);
                    on_event(ev);
                    if ends {
                        return;
                    }
                }
                let filled = renderer.render(&mut buf);
                if filled > 0 {
                    shared
                        .samples
                        .lock()
                        .extend_from_slice(&buf[..filled * ch]);
                }
                match pace {
                    Some(_) => std::thread::sleep(frame_time),
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
    stop_tx: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    /// Name and sample format of the last opened device (diagnostics).
    pub last_device: Arc<Mutex<Option<(String, String)>>>,
    selected: Arc<Mutex<Option<String>>>,
    opened: Arc<Mutex<Option<OutputDevice>>>,
}

impl CpalOutput {
    pub fn new() -> Self {
        Self::default()
    }
}

fn cpal_device_info(device: &cpal::Device, default_id: Option<&str>) -> Option<OutputDevice> {
    use cpal::traits::DeviceTrait;
    let name = device.description().ok()?.name().to_string();
    let id = device.id().map(|i| i.to_string()).unwrap_or_else(|_| name.clone());
    Some(OutputDevice {
        is_default: default_id == Some(id.as_str()),
        id,
        name,
    })
}

fn cpal_default_id(host: &cpal::Host) -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    host.default_output_device()
        .and_then(|d| d.id().ok())
        .map(|i| i.to_string())
}

/// Lists output devices. Runs on the calling thread.
fn cpal_list() -> Vec<OutputDevice> {
    use cpal::traits::HostTrait;
    let host = cpal::default_host();
    let default_id = cpal_default_id(&host);
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    let mut out: Vec<OutputDevice> = devices
        .filter_map(|d| cpal_device_info(&d, default_id.as_deref()))
        .collect();
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// Lists on a short-lived thread, so COM setup on Windows stays off the
/// caller's (async runtime) thread.
fn cpal_list_isolated() -> Vec<OutputDevice> {
    std::thread::Builder::new()
        .name("audio-devices".into())
        .spawn(cpal_list)
        .ok()
        .and_then(|t| t.join().ok())
        .unwrap_or_default()
}

struct StreamCtx {
    on_event: OutputCallback,
    selected: Arc<Mutex<Option<String>>>,
    opened: Arc<Mutex<Option<OutputDevice>>>,
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
        move |data: &mut [T], _info| {
            if scratch.len() != data.len() {
                scratch.resize(data.len(), 0.0);
            }
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

impl AudioOutput for CpalOutput {
    fn open(
        &mut self,
        renderer: Arc<dyn Renderer>,
        on_event: OutputCallback,
    ) -> AppResult<OutputFormat> {
        self.close();
        let (ready_tx, ready_rx) = mpsc::channel::<AppResult<OutputFormat>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let last = self.last_device.clone();
        let selected = self.selected.clone();
        let opened = self.opened.clone();
        let thread = std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || {
                use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
                let host = cpal::default_host();
                let want = selected.lock().clone();
                // A chosen device that isn't there falls back to the default.
                let chosen = want
                    .as_deref()
                    .and_then(|id| id.parse::<cpal::DeviceId>().ok())
                    .and_then(|id| host.device_by_id(&id));
                let Some(device) = chosen.or_else(|| host.default_output_device()) else {
                    let _ = ready_tx.send(Err(AppError::Io("no audio output device".into())));
                    return;
                };
                let supported = match device.default_output_config() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(AppError::Io(format!("audio device: {e}"))));
                        return;
                    }
                };
                let config = supported.config();
                let fmt = OutputFormat {
                    sample_rate: config.sample_rate,
                    channels: config.channels,
                };
                let info = cpal_device_info(&device, cpal_default_id(&host).as_deref());
                let name = info.as_ref().map_or_else(|| "unknown device".into(), |d| d.name.clone());
                *last.lock() = Some((name, format!("{:?}", supported.sample_format())));
                *opened.lock() = info;
                let ctx = StreamCtx {
                    on_event,
                    selected,
                    opened: opened.clone(),
                };
                use cpal::SampleFormat as F;
                let stream = match supported.sample_format() {
                    F::F32 => build_stream::<f32>(&device, config, renderer, ctx),
                    F::I16 => build_stream::<i16>(&device, config, renderer, ctx),
                    F::U16 => build_stream::<u16>(&device, config, renderer, ctx),
                    F::I32 => build_stream::<i32>(&device, config, renderer, ctx),
                    F::U32 => build_stream::<u32>(&device, config, renderer, ctx),
                    F::I8 => build_stream::<i8>(&device, config, renderer, ctx),
                    F::U8 => build_stream::<u8>(&device, config, renderer, ctx),
                    F::F64 => build_stream::<f64>(&device, config, renderer, ctx),
                    other => {
                        let _ = ready_tx.send(Err(AppError::Io(format!(
                            "unsupported device sample format {other:?}"
                        ))));
                        return;
                    }
                };
                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(AppError::Io(format!("open audio stream: {e}"))));
                        return;
                    }
                };
                if let Err(e) = stream.play() {
                    let _ = ready_tx.send(Err(AppError::Io(format!("start audio stream: {e}"))));
                    return;
                }
                let _ = ready_tx.send(Ok(fmt));
                // Keep the stream alive until asked to stop.
                let _ = stop_rx.recv();
                drop(stream);
            })
            .map_err(|e| AppError::Internal(e.to_string()))?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(fmt)) => {
                self.stop_tx = Some(stop_tx);
                self.thread = Some(thread);
                Ok(fmt)
            }
            Ok(Err(e)) => {
                let _ = thread.join();
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
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
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

    #[test]
    fn state_json_is_camel_case() {
        let s = OutputState::build(vec![dev("spk", true)], None, None);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["devices"][0]["isDefault"], true);
        assert!(v["selected"].is_null());
        assert_eq!(v["current"]["id"], "spk");
    }
}
