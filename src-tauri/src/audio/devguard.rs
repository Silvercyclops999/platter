//! Keeps the audio device's slow calls (opening, closing, listing) off the
//! caller's thread, with a time limit. A device that stops answering (a DAC
//! unplugged mid-call, a stuck driver) then can't hold up the player: the
//! call gives up after the limit, and the device is handed back when the
//! stuck call returns by itself. Until then the output counts as busy.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use parking_lot::Mutex;

use super::output::{AudioOutput, OutputDevice};

type Slot = Arc<Mutex<Option<Box<dyn AudioOutput>>>>;

pub(crate) struct GuardedOutput {
    slot: Slot,
    timeout: Duration,
}

impl GuardedOutput {
    pub fn new(out: Box<dyn AudioOutput>, timeout: Duration) -> Self {
        Self { slot: Arc::new(Mutex::new(Some(out))), timeout }
    }

    /// Runs a call that can block on its own thread and waits at most the
    /// time limit. None: the device is still busy with an earlier call, or
    /// this one didn't return in time.
    pub fn slow<R: Send + 'static>(
        &self,
        what: &'static str,
        f: impl FnOnce(&mut dyn AudioOutput) -> R + Send + 'static,
    ) -> Option<R> {
        let mut out = self.slot.lock().take()?;
        let slot = self.slot.clone();
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("audio-device".into()).spawn(move || {
            let r = f(&mut *out);
            // Back first, so a caller woken by the answer finds it.
            *slot.lock() = Some(out);
            let _ = tx.send(r);
        });
        if let Err(e) = spawned {
            // The device went down with the closure; play on nothing.
            tracing::warn!("audio device: no thread for {what}: {e}");
            *self.slot.lock() = Some(Box::new(super::output::null_output()));
            return None;
        }
        match rx.recv_timeout(self.timeout) {
            Ok(r) => Some(r),
            Err(_) => {
                tracing::warn!("audio device: {what} didn't return in {:?}", self.timeout);
                None
            }
        }
    }

    /// A call that doesn't block, run here. None while a slow call holds
    /// the device.
    pub fn quick<R>(&self, f: impl FnOnce(&mut dyn AudioOutput) -> R) -> Option<R> {
        self.slot.lock().as_mut().map(|o| f(&mut **o))
    }

    pub fn devices(&self) -> Vec<OutputDevice> {
        self.slow("listing devices", |o| o.devices()).unwrap_or_default()
    }
}
