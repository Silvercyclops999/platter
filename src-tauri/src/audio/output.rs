//! Audio output behind a trait: the real cpal device, plus null and capture
//! outputs for tests (no sound device needed).
//!
//! Outputs *pull* from a [`Renderer`] (the engine's mixer) on their own
//! thread. One continuous stream is kept open across tracks, which is what
//! makes gapless joins possible.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;

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
    /// The device went away (unplugged, Bluetooth drop, route change).
    DeviceLost(String),
}

pub type OutputCallback = Arc<dyn Fn(OutputEvent) + Send + Sync>;

pub trait AudioOutput: Send {
    /// Opens the (current default) device and starts pulling from `renderer`.
    fn open(&mut self, renderer: Arc<dyn Renderer>, on_event: OutputCallback)
    -> AppResult<OutputFormat>;
    /// Stops pulling and releases the device.
    fn close(&mut self);
    fn is_open(&self) -> bool;
}

// ---------------------------------------------------------------- capture

#[derive(Default)]
struct CaptureShared {
    samples: Mutex<Vec<f32>>,
    stop: AtomicBool,
    lose: AtomicBool,
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

/// Handle for tests to read captured audio or fake a device loss.
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
        self.0.lose.store(true, Ordering::SeqCst);
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
        self.shared.lose.store(false, Ordering::SeqCst);
        let shared = self.shared.clone();
        let fmt = self.format;
        let pace = self.pace;
        self.thread = Some(std::thread::spawn(move || {
            let ch = fmt.channels as usize;
            let block = 256;
            let mut buf = vec![0.0f32; block * ch];
            let frame_time = Duration::from_secs_f64(block as f64 / fmt.sample_rate as f64);
            while !shared.stop.load(Ordering::SeqCst) {
                if shared.lose.swap(false, Ordering::SeqCst) {
                    on_event(OutputEvent::DeviceLost("capture device removed".into()));
                    return;
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

/// The system default output device via cpal (WASAPI / CoreAudio).
/// The cpal stream lives on its own thread (it isn't `Send` everywhere).
#[derive(Default)]
pub struct CpalOutput {
    stop_tx: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    /// Name and format of the last opened device (diagnostics).
    pub last_device: Arc<Mutex<Option<(String, String)>>>,
}

impl CpalOutput {
    pub fn new() -> Self {
        Self::default()
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    renderer: Arc<dyn Renderer>,
    on_event: OutputCallback,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let mut scratch: Vec<f32> = Vec::with_capacity(8192);
    let lost = Arc::new(AtomicBool::new(false));
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
            match err.kind() {
                K::DeviceNotAvailable | K::StreamInvalidated | K::DeviceChanged => {
                    if !lost.swap(true, Ordering::SeqCst) {
                        on_event(OutputEvent::DeviceLost(err.to_string()));
                    }
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
        let thread = std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || {
                use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
                let host = cpal::default_host();
                let Some(device) = host.default_output_device() else {
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
                let name = device
                    .description()
                    .map(|d| d.name().to_string())
                    .unwrap_or_else(|_| "unknown device".into());
                *last.lock() = Some((name, format!("{:?}", supported.sample_format())));
                use cpal::SampleFormat as F;
                let stream = match supported.sample_format() {
                    F::F32 => build_stream::<f32>(&device, config, renderer, on_event),
                    F::I16 => build_stream::<i16>(&device, config, renderer, on_event),
                    F::U16 => build_stream::<u16>(&device, config, renderer, on_event),
                    F::I32 => build_stream::<i32>(&device, config, renderer, on_event),
                    F::U32 => build_stream::<u32>(&device, config, renderer, on_event),
                    F::I8 => build_stream::<i8>(&device, config, renderer, on_event),
                    F::U8 => build_stream::<u8>(&device, config, renderer, on_event),
                    F::F64 => build_stream::<f64>(&device, config, renderer, on_event),
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
                Err(e)
            }
            Err(_) => {
                drop(stop_tx);
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
    }

    fn is_open(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for CpalOutput {
    fn drop(&mut self) {
        self.close();
    }
}
