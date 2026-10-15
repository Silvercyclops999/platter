//! The helper's protocol with the worker side on a thread, over pipes, and
//! a fake recognizer behind it. The real binary is started in
//! `tests/speech_helper.rs` (ignored by default).

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use super::*;
use crate::lyrics::transcribe::lines::Word;

/// Echoes what it was given; blocks on 7 samples, panics on 13.
#[derive(Default)]
struct Fake {
    unloads: AtomicUsize,
}

impl Recognizer for Fake {
    fn unavailable(&self) -> Option<String> {
        None
    }
    fn transcribe(
        &self,
        model: &Path,
        _kind: ModelKind,
        pcm: &[f32],
        threads: usize,
        language: &str,
        cancel: Arc<AtomicBool>,
        progress: Progress,
    ) -> Result<Vec<Segment>, RecognizeError> {
        match pcm.len() {
            7 => {
                while !cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                return Err(RecognizeError::Cancelled);
            }
            13 => panic!("the speech library fell over"),
            _ => {}
        }
        if model.ends_with("missing.bin") {
            return Err(RecognizeError::Failed("model didn't load".into()));
        }
        progress(10);
        progress(10);
        progress(50);
        let text = format!("{} {} {threads} {language}", model.display(), pcm.iter().sum::<f32>());
        Ok(vec![Segment { words: vec![Word { start_ms: 0, end_ms: 10, text }] }])
    }
    fn unload(&self) {
        self.unloads.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Launches {
    started: AtomicUsize,
    ended: Arc<AtomicUsize>,
    lowest: Arc<Mutex<Vec<bool>>>,
    /// Answer with something that isn't the protocol.
    garbage: AtomicBool,
}

struct ThreadLaunch {
    rec: Arc<Fake>,
    n: Arc<Launches>,
}

struct ThreadProcess(Arc<AtomicUsize>);

impl WorkerProcess for ThreadProcess {
    fn kill(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Launch for ThreadLaunch {
    fn launch(&self, _lowest: bool) -> io::Result<Worker> {
        self.n.started.fetch_add(1, Ordering::SeqCst);
        let (req_r, req_w) = io::pipe()?;
        let (rep_r, mut rep_w) = io::pipe()?;
        if self.n.garbage.load(Ordering::SeqCst) {
            rep_w.write_all(b"Hello! This is not a frame.\n")?;
            drop(req_r);
        } else {
            let rec = self.rec.clone();
            let lowest = self.n.lowest.clone();
            std::thread::spawn(move || {
                let _ = worker::serve(Box::new(req_r), Box::new(rep_w), rec, &|l| lowest.lock().push(l));
            });
        }
        Ok(Worker { input: Box::new(req_w), output: Box::new(rep_r), process: Box::new(ThreadProcess(self.n.ended.clone())) })
    }
}

struct Fx {
    helper: Arc<Helper>,
    rec: Arc<Fake>,
    n: Arc<Launches>,
    light: Arc<AtomicBool>,
}

fn fx(idle: Duration) -> Fx {
    let rec = Arc::new(Fake::default());
    let n = Arc::new(Launches::default());
    let light = Arc::new(AtomicBool::new(true));
    let l = light.clone();
    let helper = Arc::new(Helper::new(
        Arc::new(ThreadLaunch { rec: rec.clone(), n: n.clone() }),
        Arc::new(move || l.load(Ordering::SeqCst)),
        idle,
    ));
    Fx { helper, rec, n, light }
}

fn run(h: &Helper, pcm: &[f32], cancel: Arc<AtomicBool>) -> (Result<Vec<Segment>, RecognizeError>, Vec<u8>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let res = h.transcribe(Path::new("m/small.bin"), ModelKind::Small, pcm, 2, "en", cancel, Arc::new(move |p| s.lock().push(p)));
    let seen = seen.lock().clone();
    (res, seen)
}

fn never() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn text(r: Result<Vec<Segment>, RecognizeError>) -> String {
    r.unwrap()[0].words[0].text.clone()
}

#[test]
fn a_run_goes_through_the_helper_and_back() {
    let f = fx(IDLE_EXIT);
    assert!(!f.helper.running(), "nothing starts before work arrives");
    let (res, progress) = run(&f.helper, &[0.25, 0.5, 1.0], never());
    assert_eq!(text(res), "m/small.bin 1.75 2 en");
    assert_eq!(progress, vec![10, 50], "each step once");
    // The second run uses the same helper.
    f.light.store(false, Ordering::SeqCst);
    let (res, _) = run(&f.helper, &[1.0], never());
    assert_eq!(text(res), "m/small.bin 1 2 en");
    assert_eq!(f.n.started.load(Ordering::SeqCst), 1);
    assert_eq!(*f.n.lowest.lock(), vec![true, false], "Light runs lowest, the others below normal");
    // A failure is just that run's.
    let res = f.helper.transcribe(Path::new("missing.bin"), ModelKind::Small, &[1.0], 2, "en", never(), Arc::new(|_| {}));
    assert_eq!(res.unwrap_err(), RecognizeError::Failed("model didn't load".into()));
    assert!(f.helper.running() && !f.helper.crashed_too_often());
}

#[test]
fn a_long_song_round_trips() {
    let f = fx(IDLE_EXIT);
    // Four minutes of audio.
    let pcm: Vec<f32> = (0..16_000 * 240).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
    let (res, _) = run(&f.helper, &pcm, never());
    assert_eq!(text(res), "m/small.bin 0 2 en");
}

#[test]
fn cancel_stops_the_run_and_keeps_the_helper() {
    let f = fx(IDLE_EXIT);
    let cancel = never();
    let c = cancel.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        c.store(true, Ordering::SeqCst);
    });
    let (res, _) = run(&f.helper, &[0.0; 7], cancel);
    stopper.join().unwrap();
    assert_eq!(res.unwrap_err(), RecognizeError::Cancelled);
    assert!(f.helper.running());
    // Already cancelled: nothing is sent.
    let gone = Arc::new(AtomicBool::new(true));
    assert_eq!(run(&f.helper, &[1.0], gone).0.unwrap_err(), RecognizeError::Cancelled);
    // A cancel that came late doesn't stop the next run.
    assert_eq!(text(run(&f.helper, &[1.0], never()).0), "m/small.bin 1 2 en");
    assert_eq!(f.n.started.load(Ordering::SeqCst), 1);
}

#[test]
fn a_crash_fails_that_run_and_the_next_one_starts_a_new_helper() {
    let f = fx(IDLE_EXIT);
    assert_eq!(run(&f.helper, &[0.0; 13], never()).0.unwrap_err(), RecognizeError::Crashed);
    assert!(!f.helper.running());
    assert_eq!(f.n.ended.load(Ordering::SeqCst), 1);
    assert_eq!(text(run(&f.helper, &[1.0], never()).0), "m/small.bin 1 2 en");
    assert_eq!(f.n.started.load(Ordering::SeqCst), 2);
    assert!(!f.helper.crashed_too_often());
    // Two more and background work should stop.
    run(&f.helper, &[0.0; 13], never()).0.unwrap_err();
    assert!(!f.helper.crashed_too_often());
    run(&f.helper, &[0.0; 13], never()).0.unwrap_err();
    assert!(f.helper.crashed_too_often());
}

#[test]
fn something_that_isnt_the_helper_counts_as_a_crash() {
    let f = fx(IDLE_EXIT);
    f.n.garbage.store(true, Ordering::SeqCst);
    assert_eq!(run(&f.helper, &[1.0], never()).0.unwrap_err(), RecognizeError::Crashed);
    assert!(!f.helper.running());
}

#[test]
fn the_helper_ends_after_a_while_without_work() {
    let f = fx(Duration::from_millis(60));
    run(&f.helper, &[1.0], never()).0.unwrap();
    f.helper.unload();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(f.rec.unloads.load(Ordering::SeqCst), 1, "the model is freed at once");
    assert!(f.helper.running(), "the helper itself stays a moment");
    for _ in 0..100 {
        if !f.helper.running() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!f.helper.running());
    assert_eq!(f.n.ended.load(Ordering::SeqCst), 1);
    // Work again: a new one.
    run(&f.helper, &[1.0], never()).0.unwrap();
    assert_eq!(f.n.started.load(Ordering::SeqCst), 2);
    assert!(!f.helper.crashed_too_often());
}

#[test]
fn unload_without_a_helper_starts_nothing() {
    let f = fx(IDLE_EXIT);
    f.helper.unload();
    assert_eq!(f.n.started.load(Ordering::SeqCst), 0);
}
