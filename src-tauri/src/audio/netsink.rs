//! Network outputs: the engine's sound sent to a device on the network (an
//! AirPlay receiver) instead of this computer's speakers, behind the same
//! [`AudioOutput`] trait.
//!
//! [`RoutedOutput`] wraps the computer's output. While a [`NetTarget`] is
//! registered on its [`NetRoute`] and selected (its id starts with
//! [`NET_PREFIX`]), opening the output opens a [`NetOutput`] instead: a
//! thread that pulls the renderer in real time, converts to 16-bit and hands
//! fixed-size blocks to the target's [`NetStream`]. A stream that breaks is
//! reported as a lost device, and the next open falls back to the computer.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::output::{AudioOutput, OutputCallback, OutputDevice, OutputEvent, OutputFormat, Renderer};
use crate::error::{AppError, AppResult};

/// Output ids of network targets start with this.
pub const NET_PREFIX: &str = "net:";

/// One open stream to a network device.
pub trait NetStream: Send {
    /// Frames per block (each `send` gets exactly this many).
    fn block_frames(&self) -> usize;
    /// Sends one block of interleaved 16-bit audio. An error ends the stream.
    fn send(&mut self, pcm: &[i16]) -> std::io::Result<()>;
    /// Ends the session with the device.
    fn close(&mut self);
}

/// A network device the output can play on.
pub trait NetTarget: Send + Sync {
    /// The output entry (id starts with [`NET_PREFIX`]).
    fn device(&self) -> OutputDevice;
    fn format(&self) -> OutputFormat;
    /// Opens a stream (connects and sets up the session).
    fn connect(&self) -> AppResult<Box<dyn NetStream>>;
    /// How far ahead of real time blocks are sent.
    fn lead(&self) -> Duration {
        Duration::from_millis(40)
    }
}

/// The network target the output may switch to. Shared between the output
/// (inside the engine) and whoever chooses the target.
#[derive(Clone, Default)]
pub struct NetRoute {
    target: Arc<Mutex<Option<Arc<dyn NetTarget>>>>,
}

impl NetRoute {
    pub fn set(&self, target: Option<Arc<dyn NetTarget>>) {
        *self.target.lock() = target;
    }
    pub fn get(&self) -> Option<Arc<dyn NetTarget>> {
        self.target.lock().clone()
    }
    fn get_id(&self, id: &str) -> Option<Arc<dyn NetTarget>> {
        self.get().filter(|t| t.device().id == id)
    }
}

/// Converts the renderer's floats to 16-bit with clipping.
pub fn to_i16(src: &[f32], dst: &mut [i16]) {
    for (d, s) in dst.iter_mut().zip(src) {
        let v = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
        *d = (v * 32767.0).round() as i16;
    }
}

/// An open network stream fed in real time from the renderer.
pub struct NetOutput {
    target: Arc<dyn NetTarget>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl NetOutput {
    pub fn new(target: Arc<dyn NetTarget>) -> Self {
        Self {
            target,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }
}

impl AudioOutput for NetOutput {
    fn open(&mut self, renderer: Arc<dyn Renderer>, on_event: OutputCallback) -> AppResult<OutputFormat> {
        self.close();
        let fmt = self.target.format();
        let mut stream = self.target.connect()?;
        let name = self.target.device().name;
        let lead = self.target.lead();
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("audio-net-output".into())
            .spawn(move || {
                let ch = fmt.channels as usize;
                let block = stream.block_frames().max(1);
                let mut buf = vec![0.0f32; block * ch];
                let mut pcm = vec![0i16; block * ch];
                let start = Instant::now();
                let mut sent: u64 = 0;
                while !stop.load(Ordering::SeqCst) {
                    let due = Duration::from_secs_f64(sent as f64 / f64::from(fmt.sample_rate));
                    let now = start.elapsed() + lead;
                    if now < due {
                        std::thread::sleep((due - now).min(Duration::from_millis(5)));
                        continue;
                    }
                    let filled = renderer.render(&mut buf);
                    buf[filled * ch..].fill(0.0);
                    to_i16(&buf, &mut pcm);
                    if let Err(e) = stream.send(&pcm) {
                        stream.close();
                        on_event(OutputEvent::DeviceLost {
                            why: format!("network output: {e}"),
                            fell_back_from: Some(name),
                        });
                        return;
                    }
                    sent += block as u64;
                }
                stream.close();
            })
            .map_err(|e| AppError::Internal(e.to_string()))?;
        self.thread = Some(thread);
        Ok(fmt)
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    fn is_open(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    fn opened(&self) -> Option<OutputDevice> {
        self.is_open().then(|| self.target.device())
    }
}

impl Drop for NetOutput {
    fn drop(&mut self) {
        self.close();
    }
}

/// The computer's output, or a network target while one is selected.
pub struct RoutedOutput {
    local: Box<dyn AudioOutput>,
    route: NetRoute,
    /// The selected network target id.
    net_id: Arc<Mutex<Option<String>>>,
    net: Option<NetOutput>,
}

impl RoutedOutput {
    pub fn new(local: Box<dyn AudioOutput>, route: NetRoute) -> Self {
        Self {
            local,
            route,
            net_id: Arc::default(),
            net: None,
        }
    }
}

impl AudioOutput for RoutedOutput {
    fn open(&mut self, renderer: Arc<dyn Renderer>, on_event: OutputCallback) -> AppResult<OutputFormat> {
        self.close();
        let id = self.net_id.lock().clone();
        if let Some(id) = id {
            let Some(target) = self.route.get_id(&id) else {
                // The target went away: back to this computer.
                *self.net_id.lock() = None;
                return self.local.open(renderer, on_event);
            };
            let net_id = self.net_id.clone();
            let cb: OutputCallback = Arc::new(move |ev: OutputEvent| {
                if matches!(ev, OutputEvent::DeviceLost { .. }) {
                    *net_id.lock() = None;
                }
                on_event(ev);
            });
            let mut out = NetOutput::new(target);
            let fmt = out.open(renderer, cb)?;
            self.net = Some(out);
            return Ok(fmt);
        }
        self.local.open(renderer, on_event)
    }

    fn close(&mut self) {
        if let Some(mut n) = self.net.take() {
            n.close();
        }
        self.local.close();
    }

    fn is_open(&self) -> bool {
        self.net.as_ref().map_or_else(|| self.local.is_open(), NetOutput::is_open)
    }

    fn devices(&self) -> Vec<OutputDevice> {
        self.local.devices()
    }

    fn select(&mut self, id: Option<String>) {
        match id {
            Some(id) if id.starts_with(NET_PREFIX) => {
                if self.route.get_id(&id).is_some() {
                    *self.net_id.lock() = Some(id);
                }
            }
            other => {
                *self.net_id.lock() = None;
                self.local.select(other);
            }
        }
    }

    fn selected(&self) -> Option<String> {
        self.net_id.lock().clone().or_else(|| self.local.selected())
    }

    fn opened(&self) -> Option<OutputDevice> {
        match &self.net {
            Some(n) => n.opened(),
            None => self.local.opened(),
        }
    }

    // A network receiver keeps its own clock from the stream, so only the
    // local device is held still.
    fn set_idle(&mut self, idle: bool) {
        if self.net.is_none() {
            self.local.set_idle(idle);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::audio::output::CaptureOutput;

    struct Tone;
    impl Renderer for Tone {
        fn render(&self, out: &mut [f32]) -> usize {
            out.fill(0.5);
            out.len() / 2
        }
    }

    #[derive(Default)]
    struct Fake {
        blocks: Arc<Mutex<Vec<Vec<i16>>>>,
        fail_after: Option<usize>,
        connects: AtomicUsize,
        refuse: bool,
    }

    struct FakeStream {
        blocks: Arc<Mutex<Vec<Vec<i16>>>>,
        fail_after: Option<usize>,
    }

    impl NetStream for FakeStream {
        fn block_frames(&self) -> usize {
            352
        }
        fn send(&mut self, pcm: &[i16]) -> std::io::Result<()> {
            let mut b = self.blocks.lock();
            if self.fail_after.is_some_and(|n| b.len() >= n) {
                return Err(std::io::Error::other("receiver went away"));
            }
            b.push(pcm.to_vec());
            Ok(())
        }
        fn close(&mut self) {}
    }

    impl NetTarget for Fake {
        fn device(&self) -> OutputDevice {
            OutputDevice {
                id: "net:airplay:k".into(),
                name: "Kitchen".into(),
                is_default: false,
            }
        }
        fn format(&self) -> OutputFormat {
            OutputFormat {
                sample_rate: 44_100,
                channels: 2,
            }
        }
        fn connect(&self) -> AppResult<Box<dyn NetStream>> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            if self.refuse {
                return Err(AppError::Network("no".into()));
            }
            Ok(Box::new(FakeStream {
                blocks: self.blocks.clone(),
                fail_after: self.fail_after,
            }))
        }
    }

    fn noop() -> OutputCallback {
        Arc::new(|_| {})
    }

    #[test]
    fn samples_are_clipped_into_16_bits() {
        let mut d = [0i16; 5];
        to_i16(&[0.0, 1.0, -1.0, 2.0, f32::NAN], &mut d);
        assert_eq!(d, [0, 32767, -32767, 32767, 0]);
    }

    #[test]
    fn a_selected_target_gets_real_time_blocks() {
        let route = NetRoute::default();
        let fake = Arc::new(Fake::default());
        route.set(Some(fake.clone()));
        let (cap, handle) = CaptureOutput::new(OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        });
        let mut out = RoutedOutput::new(Box::new(cap), route);
        out.select(Some("net:airplay:k".into()));
        assert_eq!(out.selected().as_deref(), Some("net:airplay:k"));
        let fmt = out.open(Arc::new(Tone), noop()).unwrap();
        assert_eq!(fmt.sample_rate, 44_100);
        assert_eq!(out.opened().unwrap().name, "Kitchen");
        std::thread::sleep(Duration::from_millis(120));
        out.close();
        let blocks = fake.blocks.lock().clone();
        // ~120 ms plus the lead is ~20 blocks of 8 ms; never seconds ahead.
        assert!((5..40).contains(&blocks.len()), "{} blocks", blocks.len());
        assert!(blocks.iter().all(|b| b.len() == 704 && b[0] == 16384));
        assert_eq!(handle.opens(), 0, "the computer's output stayed closed");
        // Choosing a computer device leaves the target.
        out.select(None);
        assert_eq!(out.selected(), None);
        out.open(Arc::new(Tone), noop()).unwrap();
        assert_eq!(handle.opens(), 1);
        out.close();
    }

    #[test]
    fn an_unknown_target_id_is_ignored() {
        let mut out = RoutedOutput::new(Box::new(CaptureOutput::new(OutputFormat { sample_rate: 48_000, channels: 2 }).0), NetRoute::default());
        out.select(Some("net:gone".into()));
        assert_eq!(out.selected(), None);
    }

    #[test]
    fn a_broken_stream_is_a_lost_device_and_falls_back() {
        let route = NetRoute::default();
        let fake = Arc::new(Fake {
            fail_after: Some(3),
            ..Default::default()
        });
        route.set(Some(fake.clone()));
        let (cap, handle) = CaptureOutput::new(OutputFormat { sample_rate: 48_000, channels: 2 });
        let mut out = RoutedOutput::new(Box::new(cap), route);
        out.select(Some("net:airplay:k".into()));
        let (tx, rx) = std::sync::mpsc::channel();
        out.open(Arc::new(Tone), Arc::new(move |ev| { let _ = tx.send(ev); })).unwrap();
        let ev = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(ev, OutputEvent::DeviceLost { fell_back_from: Some(ref n), .. } if n == "Kitchen"));
        assert_eq!(out.selected(), None, "the next open uses this computer");
        out.open(Arc::new(Tone), noop()).unwrap();
        assert_eq!(handle.opens(), 1);
        out.close();
    }

    #[test]
    fn a_target_that_refuses_fails_the_open() {
        let route = NetRoute::default();
        route.set(Some(Arc::new(Fake { refuse: true, ..Default::default() })));
        let mut out = RoutedOutput::new(Box::new(CaptureOutput::new(OutputFormat { sample_rate: 48_000, channels: 2 }).0), route);
        out.select(Some("net:airplay:k".into()));
        assert!(out.open(Arc::new(Tone), noop()).is_err());
        assert!(!out.is_open());
    }
}
