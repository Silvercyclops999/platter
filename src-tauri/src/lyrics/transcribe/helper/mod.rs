//! The speech model runs in a helper process: the same Platter binary
//! started with [`WORKER_FLAG`]. The helper runs at a lower priority than
//! the player (and on Windows with power throttling on, so the processor
//! clocks stay down), which settings on our own threads can't do: the
//! speech library starts its own worker threads for every step.
//!
//! [`Helper`] is a [`Recognizer`], so the queue and the lyrics check don't
//! know the difference. The helper starts when work arrives, stays while
//! there's work, and stops after about a minute without any, which frees
//! the model's memory. A crash fails that one run; after a few in one
//! session, background work stops until Platter restarts.

pub mod proto;
pub mod worker;

#[cfg(test)]
mod tests;

use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::engine::{self, Progress, RecognizeError, Recognizer};
use super::lines::Segment;
use super::model::ModelKind;
use proto::{Reply, Request};

/// The first argument that makes Platter the helper instead of the app.
pub const WORKER_FLAG: &str = "--speech-worker";
/// The helper stops after this long without work.
pub const IDLE_EXIT: Duration = Duration::from_secs(60);
/// Crashes in one session before background work stops.
pub const CRASH_LIMIT: usize = 3;
/// A cancelled run that hasn't stopped after this long is ended by force.
const CANCEL_GRACE: Duration = Duration::from_secs(10);

/// A running helper's process.
pub trait WorkerProcess: Send {
    /// "Processor use" is Light: the lowest priority (Windows; the helper
    /// sets its own elsewhere).
    fn set_lowest(&mut self, _lowest: bool) {}
    /// Ends it now.
    fn kill(&mut self);
    /// Its input is closed: wait a moment for it to end, then end it.
    fn finish(&mut self) {
        self.kill()
    }
}

/// A started helper: its stdin, its stdout and its process.
pub struct Worker {
    pub input: Box<dyn Write + Send>,
    pub output: Box<dyn Read + Send>,
    pub process: Box<dyn WorkerProcess>,
}

/// Starts helpers (the real one starts this binary; tests start a thread).
pub trait Launch: Send + Sync {
    fn launch(&self, lowest: bool) -> io::Result<Worker>;
}

struct Live {
    input: Mutex<Box<dyn Write + Send>>,
    output: Box<dyn Read + Send>,
    process: Mutex<Box<dyn WorkerProcess>>,
}

impl Live {
    fn stop(self) {
        let Live { input, process, .. } = self;
        drop(input);
        process.into_inner().finish();
    }
}

#[derive(Default)]
struct Slot {
    live: Option<Live>,
    last_used: Option<Instant>,
    reaper: bool,
}

/// The speech model in a helper process.
pub struct Helper {
    launch: Arc<dyn Launch>,
    /// Whether to run at the lowest priority ("Processor use" is Light).
    lowest: Arc<dyn Fn() -> bool + Send + Sync>,
    idle_exit: Duration,
    slot: Arc<Mutex<Slot>>,
    crashes: AtomicUsize,
}

impl Helper {
    pub fn new(launch: Arc<dyn Launch>, lowest: Arc<dyn Fn() -> bool + Send + Sync>, idle_exit: Duration) -> Self {
        Self { launch, lowest, idle_exit, slot: Arc::new(Mutex::new(Slot::default())), crashes: AtomicUsize::new(0) }
    }

    /// The helper is running.
    pub fn running(&self) -> bool {
        self.slot.lock().live.is_some()
    }

    fn crashed(&self, slot: &mut Slot, what: &str) -> RecognizeError {
        let n = self.crashes.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::warn!(crashes = n, "speech helper stopped unexpectedly: {what}");
        if let Some(live) = slot.live.take() {
            live.process.lock().kill();
        }
        RecognizeError::Crashed
    }

    /// Stops the helper once it has been idle long enough (one at a time).
    fn watch_idle(&self, slot: &mut Slot) {
        if slot.reaper {
            return;
        }
        slot.reaper = true;
        let weak: Weak<Mutex<Slot>> = Arc::downgrade(&self.slot);
        let idle = self.idle_exit;
        let tick = (idle / 4).clamp(Duration::from_millis(5), Duration::from_secs(5));
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(tick);
                let Some(slot) = weak.upgrade() else { return };
                // Busy: a run holds the slot.
                let Some(mut s) = slot.try_lock() else { continue };
                if s.live.is_none() {
                    s.reaper = false;
                    return;
                }
                if s.last_used.is_some_and(|t| t.elapsed() >= idle) {
                    let live = s.live.take();
                    s.reaper = false;
                    drop(s);
                    if let Some(l) = live {
                        tracing::debug!("speech helper idle, stopping it");
                        l.stop();
                    }
                    return;
                }
            }
        });
    }

    fn start(&self, slot: &mut Slot, lowest: bool) -> Result<(), RecognizeError> {
        if slot.live.is_some() {
            return Ok(());
        }
        let w = self.launch.launch(lowest).map_err(|e| {
            tracing::warn!("speech helper didn't start: {e}");
            RecognizeError::Failed(format!("the speech helper didn't start: {e}"))
        })?;
        let mut live = Live { input: Mutex::new(w.input), output: w.output, process: Mutex::new(w.process) };
        match proto::read_msg::<_, Reply>(&mut live.output) {
            Ok(Some(Reply::Hello { version })) if version == proto::VERSION => {}
            other => {
                live.process.lock().kill();
                return Err(self.crashed(slot, &format!("no hello: {other:?}")));
            }
        }
        slot.live = Some(live);
        self.watch_idle(slot);
        Ok(())
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        if let Some(l) = self.slot.lock().live.take() {
            l.stop();
        }
    }
}

impl Recognizer for Helper {
    fn unavailable(&self) -> Option<String> {
        engine::unavailable_here()
    }

    fn transcribe(
        &self,
        model: &Path,
        kind: ModelKind,
        pcm: &[f32],
        threads: usize,
        language: &str,
        cancel: Arc<AtomicBool>,
        progress: Progress,
    ) -> Result<Vec<Segment>, RecognizeError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(RecognizeError::Cancelled);
        }
        // One run at a time (transcription and the lyrics check have one each).
        let mut guard = self.slot.lock();
        let slot = &mut *guard;
        let lowest = (self.lowest)();
        self.start(slot, lowest)?;
        slot.last_used = Some(Instant::now());
        let res = {
            let live = slot.live.as_mut().expect("just started");
            live.process.lock().set_lowest(lowest);
            let req = Request::Transcribe {
                model: model.to_path_buf(),
                kind,
                threads,
                language: language.to_owned(),
                samples: pcm.len(),
                lowest,
            };
            let sent = {
                let mut input = live.input.lock();
                proto::write_msg(&mut *input, &req).and_then(|_| proto::write_samples(&mut *input, pcm))
            };
            match sent {
                Err(e) => Err(format!("couldn't send the song: {e}")),
                Ok(()) => Ok(run(live, &cancel, &progress)),
            }
        };
        slot.last_used = Some(Instant::now());
        match res {
            Err(what) => Err(self.crashed(slot, &what)),
            Ok(Ended::Reply(Reply::Done { segments })) => Ok(segments),
            Ok(Ended::Reply(Reply::Failed { message })) => Err(RecognizeError::Failed(message)),
            Ok(Ended::Reply(_)) => Err(RecognizeError::Cancelled),
            Ok(Ended::Killed) => {
                // Cancelled and it didn't stop in time: not a crash.
                slot.live = None;
                Err(RecognizeError::Cancelled)
            }
            Ok(Ended::Lost(what)) => {
                if cancel.load(Ordering::Relaxed) {
                    slot.live = None;
                    return Err(RecognizeError::Cancelled);
                }
                Err(self.crashed(slot, &what))
            }
        }
    }

    fn unload(&self) {
        // A run in progress holds the slot: the model is in use, so it stays.
        let Some(slot) = self.slot.try_lock() else { return };
        if let Some(live) = slot.live.as_ref() {
            let _ = proto::write_msg(&mut *live.input.lock(), &Request::Unload);
        }
    }

    fn crashed_too_often(&self) -> bool {
        self.crashes.load(Ordering::SeqCst) >= CRASH_LIMIT
    }
}

enum Ended {
    /// Done, failed or cancelled.
    Reply(Reply),
    /// Cancelled, and ended by force when it didn't stop.
    Killed,
    /// The helper's output ended or made no sense.
    Lost(String),
}

/// Reads replies until the run ends, passing a cancel on meanwhile.
fn run(live: &mut Live, cancel: &AtomicBool, progress: &Progress) -> Ended {
    let finished = AtomicBool::new(false);
    let killed = AtomicBool::new(false);
    let Live { input, output, process } = live;
    std::thread::scope(|s| {
        s.spawn(|| {
            let mut asked: Option<Instant> = None;
            while !finished.load(Ordering::Relaxed) {
                match asked {
                    None if cancel.load(Ordering::Relaxed) => {
                        let _ = proto::write_msg(&mut *input.lock(), &Request::Cancel);
                        asked = Some(Instant::now());
                    }
                    Some(t) if t.elapsed() >= CANCEL_GRACE => {
                        killed.store(true, Ordering::Relaxed);
                        process.lock().kill();
                        return;
                    }
                    _ => {}
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let end = loop {
            match proto::read_msg::<_, Reply>(output) {
                Ok(Some(Reply::Progress { percent })) => progress(percent),
                Ok(Some(r @ (Reply::Done { .. } | Reply::Failed { .. } | Reply::Cancelled))) => break Ended::Reply(r),
                Ok(Some(Reply::Hello { .. })) => break Ended::Lost("a second hello".into()),
                Ok(None) => break Ended::Lost("its output ended".into()),
                Err(e) => break Ended::Lost(e.to_string()),
            }
        };
        finished.store(true, Ordering::Relaxed);
        if killed.load(Ordering::Relaxed) && matches!(end, Ended::Lost(_)) { Ended::Killed } else { end }
    })
}

// ---------------------------------------------------------------- the real one

/// Starts this binary as the helper.
pub struct ProcessLaunch {
    pub exe: PathBuf,
}

impl ProcessLaunch {
    /// This binary.
    pub fn current() -> Self {
        Self { exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("platter")) }
    }
}

struct ChildProcess(Child);

impl WorkerProcess for ChildProcess {
    fn set_lowest(&mut self, lowest: bool) {
        crate::platform::cpu::lower_child(&self.0, lowest);
    }
    fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        crate::platform::cpu::set_helper(None);
    }
    fn finish(&mut self) {
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until {
            if let Ok(Some(_)) = self.0.try_wait() {
                crate::platform::cpu::set_helper(None);
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.kill();
    }
}

impl Launch for ProcessLaunch {
    fn launch(&self, lowest: bool) -> io::Result<Worker> {
        let mut cmd = Command::new(&self.exe);
        cmd.arg(WORKER_FLAG).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        crate::platform::cpu::lower_command(&mut cmd, lowest);
        let mut child = cmd.spawn()?;
        crate::platform::cpu::lower_child(&child, lowest);
        let input = child.stdin.take().ok_or_else(|| io::Error::other("no stdin"))?;
        let output = child.stdout.take().ok_or_else(|| io::Error::other("no stdout"))?;
        if let Some(err) = child.stderr.take() {
            // The helper's log lines (and the speech library's) go to ours.
            std::thread::spawn(move || {
                for line in io::BufReader::new(err).lines().map_while(Result::ok) {
                    tracing::debug!(target: "speech_helper", "{line}");
                }
            });
        }
        tracing::debug!(pid = child.id(), "speech helper started");
        crate::platform::cpu::set_helper(Some(child.id()));
        Ok(Worker { input: Box::new(input), output: Box::new(output), process: Box::new(ChildProcess(child)) })
    }
}
