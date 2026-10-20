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
use crate::lyrics::transcribe::engine::Step;
use crate::lyrics::transcribe::lines::Word;

/// Echoes what it was given; blocks on 7 samples, panics on 13, and on 17
/// ignores the cancel until its "process" is ended.
#[derive(Default)]
struct Fake {
    unloads: AtomicUsize,
    /// Set when the helper is ended by force.
    killed: Arc<AtomicBool>,
    /// The models loaded, in order (a new one each time it changes).
    loads: Mutex<Vec<ModelKind>>,
    /// Runs going on in every helper at once, and the most seen.
    busy: Arc<AtomicUsize>,
    most_busy: Arc<AtomicUsize>,
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
        cancel: Stop,
        progress: Progress,
    ) -> Result<Vec<Segment>, RecognizeError> {
        if self.loads.lock().last() != Some(&_kind) {
            progress(engine::Step::Loading);
            self.loads.lock().push(_kind);
        }
        let n = self.busy.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_busy.fetch_max(n, Ordering::SeqCst);
        let res = self.run(model, pcm, threads, language, cancel, progress);
        self.busy.fetch_sub(1, Ordering::SeqCst);
        res
    }
    fn unload(&self) {
        self.unloads.fetch_add(1, Ordering::SeqCst);
    }
}

impl Fake {
    fn run(&self, model: &Path, pcm: &[f32], threads: usize, language: &str, cancel: Stop, progress: Progress) -> Result<Vec<Segment>, RecognizeError> {
        match pcm.len() {
            17 => {
                while !self.killed.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                panic!("ended by force");
            }
            7 => {
                while !cancel.is_set() {
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
        progress(engine::Step::Percent(10));
        progress(engine::Step::Percent(10));
        progress(engine::Step::Percent(50));
        let text = format!("{} {} {threads} {language}", model.display(), pcm.iter().sum::<f32>());
        Ok(vec![Segment { words: vec![Word { start_ms: 0, end_ms: 10, text }] }])
    }
}

#[derive(Default)]
struct Launches {
    started: AtomicUsize,
    ended: Arc<AtomicUsize>,
    /// The most helpers alive at once.
    most_alive: AtomicUsize,
    lowest: Arc<Mutex<Vec<bool>>>,
    /// Whether each helper was allowed the graphics card.
    gpu: Mutex<Vec<bool>>,
    /// Answer with something that isn't the protocol.
    garbage: AtomicBool,
}

struct ThreadLaunch {
    rec: Arc<Fake>,
    n: Arc<Launches>,
}

struct ThreadProcess(Arc<AtomicUsize>, Arc<AtomicBool>);

impl WorkerProcess for ThreadProcess {
    fn kill(&mut self) {
        self.1.store(true, Ordering::SeqCst);
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Launch for ThreadLaunch {
    fn launch_with(&self, lowest: bool, gpu: bool) -> io::Result<Worker> {
        self.n.gpu.lock().push(gpu);
        self.launch(lowest)
    }

    fn launch(&self, _lowest: bool) -> io::Result<Worker> {
        let started = self.n.started.fetch_add(1, Ordering::SeqCst) + 1;
        let alive = started - self.n.ended.load(Ordering::SeqCst);
        self.n.most_alive.fetch_max(alive, Ordering::SeqCst);
        self.rec.killed.store(false, Ordering::SeqCst);
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
        Ok(Worker { input: Box::new(req_w), output: Box::new(rep_r), process: Box::new(ThreadProcess(self.n.ended.clone(), self.rec.killed.clone())) })
    }
}

struct Fx {
    helper: Arc<Helper>,
    rec: Arc<Fake>,
    n: Arc<Launches>,
    light: Arc<AtomicBool>,
}

fn fx_only(idle: Duration, only: Option<ModelKind>) -> Fx {
    let rec = Arc::new(Fake::default());
    let n = Arc::new(Launches::default());
    let light = Arc::new(AtomicBool::new(true));
    let l = light.clone();
    let mut helper =
        Helper::new(Arc::new(ThreadLaunch { rec: rec.clone(), n: n.clone() }), Arc::new(move || l.load(Ordering::SeqCst)), idle);
    if let Some(k) = only {
        helper = helper.only(k);
    }
    Fx { helper: Arc::new(helper), rec, n, light }
}

fn fx(idle: Duration) -> Fx {
    fx_only(idle, None)
}

fn run_kind(h: &dyn Recognizer, kind: ModelKind, pcm: &[f32], cancel: Stop) -> (Result<Vec<Segment>, RecognizeError>, Vec<Step>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let res = h.transcribe(Path::new("m/small.bin"), kind, pcm, 2, "en", cancel, Arc::new(move |p| s.lock().push(p)));
    let seen = seen.lock().clone();
    (res, seen)
}

fn run(h: &Helper, pcm: &[f32], cancel: Stop) -> (Result<Vec<Segment>, RecognizeError>, Vec<Step>) {
    run_kind(h, ModelKind::Small, pcm, cancel)
}

fn never() -> Stop {
    Stop::new()
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    for _ in 0..500 {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}");
}

fn text(r: Result<Vec<Segment>, RecognizeError>) -> String {
    r.unwrap()[0].words[0].text.clone()
}

#[test]
fn the_helper_never_starts_a_program_found_on_path() {
    for exe in ["", "platter"] {
        let l = ProcessLaunch { exe: std::path::PathBuf::from(exe) };
        assert!(l.launch(false).is_err(), "{exe:?}");
    }
}

#[test]
fn a_run_goes_through_the_helper_and_back() {
    let f = fx(IDLE_EXIT);
    assert!(!f.helper.running(), "nothing starts before work arrives");
    let (res, progress) = run(&f.helper, &[0.25, 0.5, 1.0], never());
    assert_eq!(text(res), "m/small.bin 1.75 2 en");
    assert_eq!(progress, vec![Step::Loading, Step::Percent(10), Step::Percent(50)], "each step once, loading first");
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
fn the_check_lane_runs_only_the_small_model() {
    let lane = fx_only(IDLE_EXIT, Some(ModelKind::Small));
    assert_eq!(text(run(&lane.helper, &[1.0], never()).0), "m/small.bin 1 2 en");
    let res = run_kind(&*lane.helper, ModelKind::Medium, &[1.0], never()).0;
    assert!(matches!(res, Err(RecognizeError::Failed(_))), "{res:?}");
    assert_eq!(*lane.rec.loads.lock(), vec![ModelKind::Small], "the medium model was never loaded");
    assert!(!lane.helper.crashed_too_often());
}

#[test]
fn the_check_lane_ends_when_idle() {
    let lane = fx_only(Duration::from_millis(40), Some(ModelKind::Small));
    run(&lane.helper, &[1.0], never()).0.unwrap();
    assert!(lane.helper.running());
    wait_for("the lane to end", || !lane.helper.running());
    assert_eq!(lane.n.ended.load(Ordering::SeqCst), 1);
}

#[test]
fn a_priority_run_is_wanted_while_it_waits_and_uses_the_same_helper() {
    let f = fx(IDLE_EXIT);
    let check = f.helper.priority();
    let batch = never();
    let long = {
        let (h, stop) = (f.helper.clone(), batch.clone());
        std::thread::spawn(move || run_kind(&*h, ModelKind::Medium, &[0.0; 7], stop).0)
    };
    wait_for("the batch song", || f.rec.busy.load(Ordering::SeqCst) == 1);
    assert!(!f.helper.wanted());
    let waiting = std::thread::spawn(move || text(run_kind(&*check, ModelKind::Medium, &[1.0], never()).0));
    wait_for("the check to be waiting", || f.helper.wanted());
    // What the queue does when a check is wanted: the batch song gives way.
    batch.give_way();
    assert_eq!(long.join().unwrap().unwrap_err(), RecognizeError::Cancelled);
    assert_eq!(waiting.join().unwrap(), "m/small.bin 1 2 en");
    assert!(!f.helper.wanted());
    assert_eq!(f.n.started.load(Ordering::SeqCst), 1, "one helper");
    assert_eq!(*f.rec.loads.lock(), vec![ModelKind::Medium], "loaded once");
    assert_eq!(f.rec.most_busy.load(Ordering::SeqCst), 1, "never two runs at once");
}

#[test]
fn a_run_stopped_now_ends_a_helper_that_doesnt_stop() {
    let f = fx(IDLE_EXIT);
    let stop = never();
    let s = stop.clone();
    let started = std::time::Instant::now();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        s.now();
    });
    let (res, _) = run(&f.helper, &[0.0; 17], stop);
    stopper.join().unwrap();
    assert_eq!(res.unwrap_err(), RecognizeError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(3), "within about a second: {:?}", started.elapsed());
    assert!(!f.helper.running(), "ended");
    assert!(!f.helper.crashed_too_often());
    assert_eq!(f.n.ended.load(Ordering::SeqCst), 1);
    // The next run starts a new one; never two at once.
    assert_eq!(text(run(&f.helper, &[1.0], never()).0), "m/small.bin 1 2 en");
    assert_eq!(f.n.most_alive.load(Ordering::SeqCst), 1);
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
        c.give_way();
    });
    let (res, _) = run(&f.helper, &[0.0; 7], cancel);
    stopper.join().unwrap();
    assert_eq!(res.unwrap_err(), RecognizeError::Cancelled);
    assert!(f.helper.running());
    // Already cancelled: nothing is sent.
    let gone = never();
    gone.give_way();
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
    // The fake's exit count can land a moment after running() turns false on a busy machine.
    for _ in 0..200 {
        if !f.helper.running() && f.n.ended.load(Ordering::SeqCst) == 1 {
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

// ---------------------------------------------------------------- graphics card

fn gpu_with_card() -> Arc<crate::lyrics::transcribe::gpu::Gpu> {
    use crate::lyrics::transcribe::gpu::{Device, DeviceKind, Gpu};
    let g = Gpu::new(Arc::new(|| vec![Device { index: 0, name: "Test GPU".into(), kind: DeviceKind::Discrete, memory: 8 << 30 }]));
    g.wait(Duration::from_secs(5));
    g
}

#[test]
fn the_helper_gets_the_graphics_card_and_moves_to_the_processor_after_a_crash_on_it() {
    let rec = Arc::new(Fake::default());
    let n = Arc::new(Launches::default());
    let gpu = gpu_with_card();
    let helper = Helper::new(Arc::new(ThreadLaunch { rec, n: n.clone() }), Arc::new(|| false), Duration::from_secs(60)).with_gpu(gpu.clone());
    assert!(run(&helper, &[1.0], never()).0.is_ok());
    assert_eq!(*n.gpu.lock(), vec![true]);
    // The speech library falls over on the card: that run fails...
    assert_eq!(run(&helper, &[0.0; 13], never()).0, Err(RecognizeError::Crashed));
    assert!(gpu.active().is_none(), "the card isn't used again this session");
    // ...and the next helper runs on the processor.
    assert!(run(&helper, &[1.0], never()).0.is_ok());
    assert_eq!(*n.gpu.lock(), vec![true, false]);
}

#[test]
fn turning_the_graphics_card_off_starts_a_fresh_helper_on_the_processor() {
    use crate::lyrics::transcribe::gpu::GpuMode;
    let rec = Arc::new(Fake::default());
    let n = Arc::new(Launches::default());
    let gpu = gpu_with_card();
    let helper = Helper::new(Arc::new(ThreadLaunch { rec, n: n.clone() }), Arc::new(|| false), Duration::from_secs(60)).with_gpu(gpu.clone());
    assert!(run(&helper, &[1.0], never()).0.is_ok());
    assert!(run(&helper, &[1.0], never()).0.is_ok());
    assert_eq!(*n.gpu.lock(), vec![true], "one helper while nothing changes");
    gpu.set_mode(GpuMode::Off);
    assert!(run(&helper, &[1.0], never()).0.is_ok());
    assert_eq!(*n.gpu.lock(), vec![true, false]);
    assert_eq!(n.ended.load(Ordering::SeqCst), 1, "the one on the card was stopped");
}

#[test]
fn without_a_card_helpers_never_get_the_graphics_card() {
    let f = fx(Duration::from_secs(60));
    assert!(run(&f.helper, &[1.0], never()).0.is_ok());
    assert_eq!(*f.n.gpu.lock(), vec![false]);
}
