//! Keeping speech-model work gentle on the processor: how many threads a
//! run gets, waiting while the rest of the computer is busy, a little more
//! while nobody is using it, and a breather between background songs.
//! Shared by transcription and the lyrics check. (The lower priority comes
//! from the helper process the model runs in.)
//!
//! The busy check reads the computer every few seconds, and only while
//! background speech work is waiting or running. Nothing is read when
//! there's no such work.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// "Processor use" in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessorUse {
    /// Two threads in the background, four for the song someone is waiting on.
    #[default]
    Light,
    /// Four for everything.
    Balanced,
    /// Half the logical cores (at most eight) for everything.
    Fast,
}

/// Threads for one run. `now`: the one song someone is waiting on. Never
/// more than half the logical cores.
pub fn thread_budget(cores: usize, usage: ProcessorUse, now: bool) -> usize {
    let half = (cores / 2).max(1);
    let want = match usage {
        ProcessorUse::Light => {
            if now {
                4
            } else {
                2
            }
        }
        ProcessorUse::Balanced => 4,
        ProcessorUse::Fast => half.min(8),
    };
    want.clamp(1, half)
}

/// Threads for a run on the graphics card. The processor still turns the
/// audio into the model's input, picks each word and lines the words up
/// with the audio; with one thread that starved the card (a song took 10
/// to 20 s on a fast card). The work is short, so a few threads at idle
/// priority: three on Light, four otherwise (never more than half the cores).
pub fn gpu_thread_budget(cores: usize, usage: ProcessorUse) -> usize {
    let want = if usage == ProcessorUse::Light { 3 } else { 4 };
    want.clamp(1, (cores / 2).max(1))
}

/// Logical cores on this computer.
pub fn cores() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

/// What the busy check and away mode read. Blocking calls are fine (they
/// run on a blocking thread).
pub trait Signals: Send + Sync {
    /// Other programs' share of the processor since the last reading,
    /// percent (0 to 100). None on the first reading or when unknown.
    fn others_percent(&self) -> Option<f32>;
    /// Sampling stopped; the next reading starts fresh.
    fn reset(&self) {}
    /// A fullscreen game or a presentation is in front, or the computer
    /// is running warm.
    fn in_front(&self) -> bool {
        false
    }
    /// Time since the last keyboard or mouse input.
    fn since_input(&self) -> Option<Duration> {
        None
    }
    fn on_battery(&self) -> bool {
        false
    }
}

/// The real computer.
#[derive(Default)]
pub struct SystemSignals(crate::platform::cpu::SystemLoad);

impl Signals for SystemSignals {
    fn others_percent(&self) -> Option<f32> {
        self.0.others_percent()
    }
    fn reset(&self) {
        self.0.reset()
    }
    fn in_front(&self) -> bool {
        crate::platform::cpu::fullscreen_in_front() || crate::platform::cpu::running_hot()
    }
    fn since_input(&self) -> Option<Duration> {
        crate::platform::cpu::since_input()
    }
    fn on_battery(&self) -> bool {
        crate::platform::power::on_battery()
    }
}

/// Other programs above this share of the processor count as busy.
pub const BUSY_PERCENT: f32 = 50.0;

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Between readings.
    pub sample: Duration,
    /// Readings averaged over this long.
    pub window: Duration,
    /// Calm this long before background work goes on again.
    pub resume: Duration,
    /// How often paused work looks again.
    pub poll: Duration,
    /// The breather between background songs.
    pub rest: Duration,
    /// No input for this long (on mains power) counts as away.
    pub away: Duration,
    /// A lyrics check with no progress for this long is reported as
    /// couldn't check yet.
    pub stuck: Duration,
    /// How often idle upkeep (moving old transcription timing, looking for
    /// official lyrics again) looks for work.
    pub upkeep: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            sample: Duration::from_secs(3),
            window: Duration::from_secs(10),
            resume: Duration::from_secs(30),
            poll: Duration::from_secs(5),
            rest: Duration::from_secs(5),
            away: Duration::from_secs(5 * 60),
            stuck: Duration::from_secs(120),
            upkeep: Duration::from_secs(10 * 60),
        }
    }
}

impl Timing {
    /// Readings that make a full window.
    fn needed(&self) -> usize {
        let s = self.sample.as_millis().max(1);
        ((self.window.as_millis() / s) as usize).max(1)
    }
}

/// What the readings say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Load {
    /// Not enough readings yet.
    Unknown,
    Calm,
    Busy,
}

/// One look at the computer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    /// Other programs' share of the processor (None: it couldn't be read).
    pub others: Option<f32>,
    /// A fullscreen game or presentation in front, or running warm.
    pub in_front: bool,
}

/// The decision from the readings: busy when the average over the window
/// is above [`BUSY_PERCENT`] or something fullscreen is in front, calm
/// again after `resume` with neither.
#[derive(Debug, Default)]
pub struct BusyGate {
    samples: VecDeque<(Instant, f32)>,
    attempts: usize,
    busy: bool,
    calm_since: Option<Instant>,
}

impl BusyGate {
    pub fn observe(&mut self, at: Instant, s: Sample, t: &Timing) {
        self.attempts += 1;
        if let Some(pct) = s.others {
            self.samples.push_back((at, pct));
        }
        while self.samples.front().is_some_and(|(first, _)| at.duration_since(*first) > t.window) {
            self.samples.pop_front();
        }
        let full = self.samples.len() >= t.needed();
        let avg = if self.samples.is_empty() {
            0.0
        } else {
            self.samples.iter().map(|s| s.1).sum::<f32>() / self.samples.len() as f32
        };
        let high = s.in_front || (avg > BUSY_PERCENT && (full || self.busy));
        if !self.busy {
            if high {
                self.busy = true;
                self.calm_since = None;
            }
        } else if high {
            self.calm_since = None;
        } else {
            let since = *self.calm_since.get_or_insert(at);
            if at.duration_since(since) >= t.resume {
                self.busy = false;
                self.calm_since = None;
            }
        }
    }

    pub fn load(&self, t: &Timing) -> Load {
        if self.busy {
            Load::Busy
        } else if self.samples.len() < t.needed() && self.attempts <= t.needed() {
            // Readings that never come (an unsupported system) don't hold work up.
            Load::Unknown
        } else {
            Load::Calm
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Options {
    usage: ProcessorUse,
    pause_when_busy: bool,
    faster_away: bool,
}

impl Options {
    fn sampling(&self) -> bool {
        self.pause_when_busy || self.faster_away
    }
}

#[derive(Default)]
struct Run {
    /// Holders of a [`PaceGuard`] (background work waiting or running).
    demand: usize,
    sampling: bool,
}

/// The shared pace for speech-model work. Settings change it live.
pub struct Pacer {
    signals: Arc<dyn Signals>,
    timing: Timing,
    cores: usize,
    opts: Mutex<Options>,
    gate: Mutex<BusyGate>,
    /// Nobody has used the computer for a while and it's on mains power.
    away: AtomicBool,
    /// Lyrics checks running now.
    checks: AtomicUsize,
    run: Mutex<Run>,
}

impl Pacer {
    pub fn new(signals: Arc<dyn Signals>, timing: Timing) -> Arc<Self> {
        Self::with_cores(signals, timing, cores())
    }

    pub fn with_cores(signals: Arc<dyn Signals>, timing: Timing, cores: usize) -> Arc<Self> {
        Arc::new(Self {
            signals,
            timing,
            cores,
            opts: Mutex::new(Options { usage: ProcessorUse::default(), pause_when_busy: true, faster_away: true }),
            gate: Mutex::new(BusyGate::default()),
            away: AtomicBool::new(false),
            checks: AtomicUsize::new(0),
            run: Mutex::new(Run::default()),
        })
    }

    pub fn timing(&self) -> &Timing {
        &self.timing
    }

    /// Settings: "Processor use", "Pause while the computer is busy" and
    /// "Work faster while I'm away".
    pub fn set_options(self: &Arc<Self>, usage: ProcessorUse, pause_when_busy: bool, faster_away: bool) {
        let opts = Options { usage, pause_when_busy, faster_away };
        *self.opts.lock() = opts;
        if !pause_when_busy {
            *self.gate.lock() = BusyGate::default();
        }
        if !faster_away {
            self.away.store(false, Ordering::SeqCst);
        }
        if opts.sampling() {
            let mut run = self.run.lock();
            self.start(&mut run);
        }
    }

    /// "Processor use" is Light.
    pub fn light(&self) -> bool {
        self.opts.lock().usage == ProcessorUse::Light
    }

    /// Threads for a run (`now`: the song someone is waiting on). While
    /// nobody's at the computer, background work gets at least Balanced.
    pub fn threads(&self, now: bool) -> usize {
        self.threads_boosted(now).0
    }

    /// Threads, and whether they're more than usual because nobody's there.
    /// On Light, a background song that starts while a lyrics check runs
    /// gets one, so the two together stay within the budget.
    pub fn threads_boosted(&self, now: bool) -> (usize, bool) {
        let usage = self.opts.lock().usage;
        let n = thread_budget(self.cores, usage, now);
        if now {
            return (n, false);
        }
        if !self.away() {
            let checking = self.checks.load(Ordering::SeqCst) > 0;
            return (if checking && usage == ProcessorUse::Light { 1 } else { n }, false);
        }
        let away = n.max(thread_budget(self.cores, ProcessorUse::Balanced, false));
        (away, away > n)
    }

    /// Threads for a run on the graphics card.
    pub fn gpu_threads(&self) -> usize {
        gpu_thread_budget(self.cores, self.opts.lock().usage)
    }

    /// Threads for a lyrics check (a 30 s window): two, within half the cores.
    pub fn check_threads(&self) -> usize {
        2.min((self.cores / 2).max(1))
    }

    /// A lyrics check is running until the guard drops.
    pub fn checking(self: &Arc<Self>) -> CheckGuard {
        self.checks.fetch_add(1, Ordering::SeqCst);
        CheckGuard(self.clone())
    }

    /// Nobody's at the computer and it's on mains power (with the setting on).
    pub fn away(&self) -> bool {
        self.away.load(Ordering::SeqCst)
    }

    /// Whether background work should wait. Always calm with the setting off.
    pub fn load(&self) -> Load {
        if !self.opts.lock().pause_when_busy {
            return Load::Calm;
        }
        self.gate.lock().load(&self.timing)
    }

    /// Waits until background work may go on.
    pub async fn wait_calm(&self) {
        while self.load() != Load::Calm {
            tokio::time::sleep(self.timing.poll).await;
        }
    }

    /// Background work is waiting or running: keep readings coming until
    /// the guard drops. Call on a Tokio runtime.
    pub fn want(self: &Arc<Self>) -> PaceGuard {
        let mut run = self.run.lock();
        run.demand += 1;
        self.start(&mut run);
        PaceGuard(self.clone())
    }

    fn start(self: &Arc<Self>, run: &mut Run) {
        if run.sampling || run.demand == 0 || !self.opts.lock().sampling() {
            return;
        }
        // Settings can change off the runtime; the next want() starts it then.
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        run.sampling = true;
        let me = self.clone();
        rt.spawn(async move { me.sample_loop().await });
    }

    /// One look at the computer (blocking).
    fn look(&self, opts: Options) -> (Sample, bool) {
        let s = &self.signals;
        let sample = if opts.pause_when_busy { Sample { others: s.others_percent(), in_front: s.in_front() } } else { Sample::default() };
        // The battery is only asked about once input has stopped.
        let away = opts.faster_away && s.since_input().is_some_and(|d| d >= self.timing.away) && !s.on_battery();
        (sample, away)
    }

    async fn sample_loop(self: Arc<Self>) {
        loop {
            let opts = {
                let mut run = self.run.lock();
                let opts = *self.opts.lock();
                if run.demand == 0 || !opts.sampling() {
                    run.sampling = false;
                    drop(run);
                    *self.gate.lock() = BusyGate::default();
                    self.away.store(false, Ordering::SeqCst);
                    self.signals.reset();
                    return;
                }
                opts
            };
            let me = self.clone();
            let Ok((sample, away)) = tokio::task::spawn_blocking(move || me.look(opts)).await else { return };
            if opts.pause_when_busy {
                self.gate.lock().observe(Instant::now(), sample, &self.timing);
            }
            self.away.store(away, Ordering::SeqCst);
            tokio::time::sleep(self.timing.sample).await;
        }
    }
}

/// Held while a lyrics check runs.
pub struct CheckGuard(Arc<Pacer>);

impl Drop for CheckGuard {
    fn drop(&mut self) {
        self.0.checks.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Held while background speech work is waiting or running.
pub struct PaceGuard(Arc<Pacer>);

impl Drop for PaceGuard {
    fn drop(&mut self) {
        let mut run = self.0.run.lock();
        run.demand = run.demand.saturating_sub(1);
    }
}

/// Signals the tests set by hand, and quick timing.
#[cfg(test)]
pub mod fake {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use parking_lot::Mutex;

    use super::{Signals, Timing};

    pub struct FakeLoad {
        pub pct: Mutex<Option<f32>>,
        pub reads: AtomicUsize,
        pub in_front: AtomicBool,
        /// Time since input, when set.
        pub idle: Mutex<Option<Duration>>,
        pub battery: AtomicBool,
    }

    impl FakeLoad {
        pub fn new(pct: f32) -> Arc<Self> {
            Arc::new(Self {
                pct: Mutex::new(Some(pct)),
                reads: AtomicUsize::new(0),
                in_front: AtomicBool::new(false),
                idle: Mutex::new(None),
                battery: AtomicBool::new(false),
            })
        }
        pub fn set(&self, pct: f32) {
            *self.pct.lock() = Some(pct);
        }
        pub fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }
        pub fn idle_for(&self, d: Option<Duration>) {
            *self.idle.lock() = d;
        }
    }

    impl Signals for FakeLoad {
        fn others_percent(&self) -> Option<f32> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            *self.pct.lock()
        }
        fn in_front(&self) -> bool {
            self.in_front.load(Ordering::SeqCst)
        }
        fn since_input(&self) -> Option<Duration> {
            *self.idle.lock()
        }
        fn on_battery(&self) -> bool {
            self.battery.load(Ordering::SeqCst)
        }
    }

    pub fn timing() -> Timing {
        let ms = Duration::from_millis;
        Timing { sample: ms(5), window: ms(15), resume: ms(20), poll: ms(5), rest: ms(0), away: ms(1000), stuck: ms(300), upkeep: Duration::from_secs(3600) }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeLoad;
    use super::*;

    #[test]
    fn thread_budget_per_setting_and_job() {
        use ProcessorUse::*;
        // A 32-thread desktop.
        assert_eq!(thread_budget(32, Light, false), 2);
        assert_eq!(thread_budget(32, Light, true), 4);
        assert_eq!(thread_budget(32, Balanced, false), 4);
        assert_eq!(thread_budget(32, Balanced, true), 4);
        assert_eq!(thread_budget(32, Fast, false), 8);
        assert_eq!(thread_budget(32, Fast, true), 8);
        // Eight threads: Fast is half of them.
        assert_eq!(thread_budget(8, Fast, false), 4);
        // Never more than half the logical cores.
        assert_eq!(thread_budget(4, Light, true), 2);
        assert_eq!(thread_budget(4, Balanced, false), 2);
        assert_eq!(thread_budget(6, Balanced, true), 3);
        assert_eq!(thread_budget(2, Fast, true), 1);
        assert_eq!(thread_budget(1, Light, true), 1);
    }

    fn t() -> Timing {
        Timing { away: Duration::from_secs(300), ..Timing::default() }
    }

    fn pct(p: f32) -> Sample {
        Sample { others: Some(p), in_front: false }
    }

    #[test]
    fn busy_after_a_high_average_and_calm_after_thirty_seconds_below() {
        let t = t();
        let mut g = BusyGate::default();
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        assert_eq!(g.load(&t), Load::Unknown);
        g.observe(at(0), Sample::default(), &t);
        g.observe(at(3), pct(90.0), &t);
        g.observe(at(6), pct(90.0), &t);
        assert_eq!(g.load(&t), Load::Unknown, "not a full window yet");
        g.observe(at(9), pct(90.0), &t);
        assert_eq!(g.load(&t), Load::Busy);
        // The average drops below the line at 15 s; it stays paused for 30 s more.
        let mut s = 12;
        while s <= 42 {
            g.observe(at(s), pct(10.0), &t);
            assert_eq!(g.load(&t), Load::Busy, "at {s} s");
            s += 3;
        }
        g.observe(at(45), pct(10.0), &t);
        assert_eq!(g.load(&t), Load::Calm);
        // Busy again: back above the line for a full window.
        for s in [48, 51, 54, 57] {
            g.observe(at(s), pct(80.0), &t);
        }
        assert_eq!(g.load(&t), Load::Busy);
    }

    #[test]
    fn a_fullscreen_game_is_busy_at_once_and_calm_thirty_seconds_after() {
        let t = t();
        let mut g = BusyGate::default();
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        g.observe(at(0), Sample { others: Some(5.0), in_front: true }, &t);
        assert_eq!(g.load(&t), Load::Busy, "no need to wait for a full window");
        g.observe(at(3), pct(5.0), &t);
        g.observe(at(30), pct(5.0), &t);
        assert_eq!(g.load(&t), Load::Busy);
        g.observe(at(33), pct(5.0), &t);
        assert_eq!(g.load(&t), Load::Calm);
    }

    #[test]
    fn a_short_spike_is_not_busy() {
        let t = t();
        let mut g = BusyGate::default();
        let start = Instant::now();
        for (i, p) in [20.0, 20.0, 100.0, 20.0, 20.0].into_iter().enumerate() {
            g.observe(start + Duration::from_secs(3 * i as u64), pct(p), &t);
        }
        assert_eq!(g.load(&t), Load::Calm);
    }

    #[test]
    fn no_readings_at_all_never_hold_work_up() {
        let t = t();
        let mut g = BusyGate::default();
        let start = Instant::now();
        for i in 0..5 {
            g.observe(start + Duration::from_secs(3 * i), Sample::default(), &t);
        }
        assert_eq!(g.load(&t), Load::Calm);
    }

    #[tokio::test]
    async fn readings_only_while_work_wants_them() {
        let load = FakeLoad::new(10.0);
        let p = Pacer::with_cores(load.clone(), fake::timing(), 16);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(load.reads(), 0, "nothing reads with no work");
        let g = p.want();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(load.reads() >= 2);
        assert_eq!(p.load(), Load::Calm);
        drop(g);
        tokio::time::sleep(Duration::from_millis(30)).await;
        let n = load.reads();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(load.reads(), n, "stopped once the work was gone");
        // Both off: no readings even with work.
        p.set_options(ProcessorUse::Light, false, false);
        let _g = p.want();
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(load.reads(), n);
        assert_eq!(p.load(), Load::Calm);
    }

    #[test]
    fn a_check_gets_two_threads_and_light_drops_the_batch_to_one_meanwhile() {
        let p = Pacer::with_cores(FakeLoad::new(0.0), fake::timing(), 32);
        assert_eq!(p.check_threads(), 2);
        let g = p.checking();
        assert_eq!((p.threads(false), p.threads(true)), (1, 4), "the song asked for keeps its four");
        p.opts.lock().usage = ProcessorUse::Balanced;
        assert_eq!(p.threads(false), 4);
        p.opts.lock().usage = ProcessorUse::Light;
        drop(g);
        assert_eq!(p.threads(false), 2);
        assert_eq!(Pacer::with_cores(FakeLoad::new(0.0), fake::timing(), 2).check_threads(), 1);
    }

    #[test]
    fn threads_follow_the_setting() {
        let p = Pacer::with_cores(FakeLoad::new(0.0), fake::timing(), 32);
        assert_eq!((p.threads(false), p.threads(true)), (2, 4));
        p.opts.lock().usage = ProcessorUse::Fast;
        assert_eq!((p.threads(false), p.threads(true)), (8, 8));
    }

    async fn until(what: &str, f: impl Fn() -> bool) {
        for _ in 0..300 {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test]
    async fn away_raises_background_threads_and_drops_back_when_input_returns() {
        let load = FakeLoad::new(5.0);
        let p = Pacer::with_cores(load.clone(), fake::timing(), 32);
        let _g = p.want();
        assert_eq!(p.threads(false), 2);
        load.idle_for(Some(Duration::from_secs(2)));
        until("away", || p.away()).await;
        assert_eq!(p.threads(false), 4, "Balanced while nobody's there");
        assert_eq!(p.threads(true), 4);
        load.idle_for(Some(Duration::from_millis(10)));
        until("back", || !p.away()).await;
        assert_eq!(p.threads(false), 2);
        // On battery it never counts as away.
        load.battery.store(true, Ordering::SeqCst);
        load.idle_for(Some(Duration::from_secs(2)));
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(!p.away());
        // Fast already uses more; away doesn't lower it.
        load.battery.store(false, Ordering::SeqCst);
        p.set_options(ProcessorUse::Fast, true, true);
        until("away", || p.away()).await;
        assert_eq!(p.threads(false), 8);
        // The setting off.
        p.set_options(ProcessorUse::Light, true, false);
        assert!(!p.away());
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(p.threads(false), 2);
    }

    #[tokio::test]
    async fn away_mode_reads_even_with_the_busy_pause_off() {
        let load = FakeLoad::new(95.0);
        let p = Pacer::with_cores(load.clone(), fake::timing(), 32);
        p.set_options(ProcessorUse::Light, false, true);
        load.idle_for(Some(Duration::from_secs(2)));
        let _g = p.want();
        until("away", || p.away()).await;
        assert_eq!(load.reads(), 0, "the processor isn't read for away mode");
        assert_eq!(p.load(), Load::Calm);
    }
}
