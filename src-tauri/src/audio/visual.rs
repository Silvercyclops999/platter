//! The visualizer's tap: a read-only copy of what goes out, taken after the
//! sound settings (EQ, leveling, limiter) and before the volume, so the
//! picture doesn't shrink when the volume goes down. It also works while
//! the output is bit-perfect, since it only reads.
//!
//! Nothing runs unless a window is watching: [`VisTap::watch`] has to be
//! renewed, and a watch that isn't renewed within [`WATCH_FOR`] ends by
//! itself, so a closed or hidden window can't leave it running. While
//! watched, the render callback copies each block into a ring and, every
//! `rate / fps` frames, analyses the last [`WINDOW`] frames that are
//! actually being heard (the ring is read [`TapState::latency_frames`]
//! behind the newest frame): a Hann window, a real FFT, [`BANDS`] log-spaced
//! bands in dB, RMS and peak per channel, and [`SCOPE_POINTS`] stereo pairs
//! for the phase scope. There is no thread of its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

pub const EV_SPECTRUM: &str = "player://spectrum";

/// Frames each analysis looks at.
pub const WINDOW: usize = 2048;
/// Log-spaced bands from [`LOW_HZ`] to [`HIGH_HZ`].
pub const BANDS: usize = 64;
pub const LOW_HZ: f32 = 20.0;
pub const HIGH_HZ: f32 = 20_000.0;
/// Stereo pairs sent for the phase scope.
pub const SCOPE_POINTS: usize = 256;
/// Silence, and anything quieter, reads this (dBFS).
pub const FLOOR_DB: f32 = -90.0;
/// A watch ends by itself this long after its last renewal.
pub const WATCH_FOR: Duration = Duration::from_secs(3);
/// Frame rates a watch may ask for.
pub const MAX_FPS: u32 = 60;
pub const MIN_FPS: u32 = 1;
/// The most the picture is held back to line up with the sound.
const MAX_LATENCY: Duration = Duration::from_millis(250);

/// `player://spectrum` payload. Levels are dBFS, floored at [`FLOOR_DB`]:
/// a full-scale sine reads 0 in its band.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spectrum {
    /// Milliseconds of audio heard since the watch began.
    pub t: u64,
    /// [`BANDS`] values; band k spans `LOW_HZ * (HIGH_HZ / LOW_HZ)^(k / BANDS)`
    /// up to the next edge.
    pub bands: Vec<f32>,
    /// Left, right.
    pub rms: [f32; 2],
    pub peak: [f32; 2],
    /// [`SCOPE_POINTS`] pairs, interleaved left, right (-1..1), when asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Vec<f32>>,
}

/// Lower edge of band `k` (k = BANDS is the top edge).
pub fn band_edge(k: usize) -> f32 {
    LOW_HZ * (HIGH_HZ / LOW_HZ).powf(k as f32 / BANDS as f32)
}

/// Linear amplitude to dBFS, floored.
pub fn to_db(v: f32) -> f32 {
    if v <= 0.0 || !v.is_finite() {
        return FLOOR_DB;
    }
    (20.0 * v.log10()).max(FLOOR_DB)
}

pub struct VisTap {
    epoch: Instant,
    /// Watched until this many ms after `epoch` (0: not watched).
    until_ms: AtomicU64,
    /// How long a renewal lasts, in ms (tests shorten it).
    watch_ms: AtomicU64,
    state: Mutex<TapState>,
}

impl Default for VisTap {
    fn default() -> Self {
        Self::new()
    }
}

impl VisTap {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            until_ms: AtomicU64::new(0),
            watch_ms: AtomicU64::new(WATCH_FOR.as_millis() as u64),
            state: Mutex::new(TapState::default()),
        }
    }

    fn now_ms(&self) -> u64 {
        // Never 0, so 0 can mean "not watched".
        self.epoch.elapsed().as_millis() as u64 + 1
    }

    /// Starts or renews a watch at `fps` (clamped to 1..=60), or ends it.
    /// `scope` adds the stereo pairs. The buffers are made here, not on the
    /// audio thread.
    pub fn watch(&self, on: bool, fps: u32, scope: bool) {
        if !on {
            self.until_ms.store(0, Ordering::SeqCst);
            return;
        }
        {
            let mut s = self.state.lock();
            s.fps = fps.clamp(MIN_FPS, MAX_FPS);
            s.scope = scope;
            s.prepare();
        }
        let was = self.until_ms.swap(self.now_ms() + self.watch_ms.load(Ordering::SeqCst), Ordering::SeqCst);
        if was == 0 || was < self.now_ms() {
            // A fresh watch starts its clock and forgets old audio.
            self.state.lock().restart();
        }
    }

    /// Someone is watching now.
    pub fn active(&self) -> bool {
        let until = self.until_ms.load(Ordering::Relaxed);
        until != 0 && self.now_ms() < until
    }

    /// How long a renewal lasts.
    pub fn watch_for(&self) -> Duration {
        Duration::from_millis(self.watch_ms.load(Ordering::SeqCst))
    }

    #[cfg(test)]
    pub(crate) fn set_watch_for(&self, d: Duration) {
        self.watch_ms.store(d.as_millis() as u64, Ordering::SeqCst);
    }

    /// For the render callback, only while [`active`](Self::active).
    pub(crate) fn lock(&self) -> parking_lot::MutexGuard<'_, TapState> {
        self.state.lock()
    }

    /// The output's own delay (device buffer), so the picture is held back
    /// to match what is heard.
    pub fn set_latency(&self, latency: Duration) {
        let mut s = self.state.lock();
        s.latency = Some(latency.min(MAX_LATENCY));
    }
}

/// Ring of what went out, and the analysis buffers.
pub struct TapState {
    fps: u32,
    scope: bool,
    rate: u32,
    /// Stereo frames, interleaved.
    ring: Vec<f32>,
    /// Next frame written.
    write: usize,
    /// Frames since the last analysis.
    since: usize,
    /// Frames heard since the watch began.
    clock: u64,
    /// The output's reported delay past the block being written.
    latency: Option<Duration>,
    last_block: usize,
    fft: Option<Arc<dyn RealToComplex<f32>>>,
    hann: Vec<f32>,
    /// Left and right, windowed.
    input: [Vec<f32>; 2],
    spec: [Vec<Complex<f32>>; 2],
    /// Power per bin, both channels averaged.
    power: Vec<f32>,
    scratch: Vec<Complex<f32>>,
    /// Bin range per band, for `rate`.
    bins: Vec<(usize, usize)>,
}

impl Default for TapState {
    fn default() -> Self {
        Self {
            fps: 30,
            scope: false,
            rate: 0,
            ring: Vec::new(),
            write: 0,
            since: 0,
            clock: 0,
            latency: None,
            last_block: 0,
            fft: None,
            hann: Vec::new(),
            input: [Vec::new(), Vec::new()],
            spec: [Vec::new(), Vec::new()],
            power: Vec::new(),
            scratch: Vec::new(),
            bins: Vec::new(),
        }
    }
}

impl TapState {
    fn prepare(&mut self) {
        if self.fft.is_some() {
            return;
        }
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(WINDOW);
        self.input = [fft.make_input_vec(), fft.make_input_vec()];
        self.spec = [fft.make_output_vec(), fft.make_output_vec()];
        self.power = vec![0.0; WINDOW / 2 + 1];
        self.scratch = fft.make_scratch_vec();
        self.hann = (0..WINDOW)
            .map(|i| {
                let x = std::f32::consts::PI * 2.0 * i as f32 / WINDOW as f32;
                0.5 - 0.5 * x.cos()
            })
            .collect();
        self.fft = Some(fft);
    }

    fn restart(&mut self) {
        self.ring.fill(0.0);
        self.write = 0;
        self.since = 0;
        self.clock = 0;
    }

    /// Sizes the ring and band table for `rate` (only when it changes).
    fn configure(&mut self, rate: u32) {
        if rate == self.rate && !self.ring.is_empty() {
            return;
        }
        self.rate = rate;
        let frames = WINDOW + (rate as f64 * MAX_LATENCY.as_secs_f64()).ceil() as usize + 1;
        self.ring = vec![0.0; frames * 2];
        self.write = 0;
        self.since = 0;
        let bin_hz = rate as f32 / WINDOW as f32;
        let top = WINDOW / 2;
        self.bins = (0..BANDS)
            .map(|k| {
                let lo = band_edge(k);
                let hi = band_edge(k + 1);
                let a = (lo / bin_hz).ceil() as usize;
                let b = (hi / bin_hz).ceil() as usize;
                if a > top {
                    // Past what this rate can hold: always the floor.
                    (0, 0)
                } else if b <= a {
                    // Narrower than a bin: the bin nearest the band's centre.
                    let c = ((lo * hi).sqrt() / bin_hz).round() as usize;
                    let c = c.clamp(1, top);
                    (c, c + 1)
                } else {
                    (a.max(1), b.min(top + 1))
                }
            })
            .collect();
    }

    /// Delay the picture is held back by, in frames.
    pub fn latency_frames(&self) -> usize {
        // The block just written plays after the device's own delay.
        let delay = self.latency.map_or(0, |d| (d.as_secs_f64() * self.rate as f64).round() as usize);
        let frames = self.last_block + delay;
        let cap = self.ring.len() / 2 - WINDOW - 1;
        frames.min(cap)
    }

    /// Starts a render block of `frames` frames at `rate`.
    pub(crate) fn begin(&mut self, rate: u32, frames: usize) {
        self.configure(rate.max(1));
        self.last_block = frames;
    }

    /// Copies one output frame (any channel count; mono is doubled, past
    /// two the rest is left out).
    #[inline]
    pub(crate) fn push(&mut self, frame: &[f32]) {
        let (l, r) = match frame {
            [] => (0.0, 0.0),
            [m] => (*m, *m),
            [l, r, ..] => (*l, *r),
        };
        let at = self.write * 2;
        self.ring[at] = l;
        self.ring[at + 1] = r;
        self.write += 1;
        if self.write * 2 >= self.ring.len() {
            self.write = 0;
        }
        self.since += 1;
        self.clock += 1;
    }

    /// `n` frames of silence (the part of a block nothing filled).
    pub(crate) fn push_silence(&mut self, n: usize) {
        for _ in 0..n.min(self.ring.len() / 2) {
            self.push(&[0.0, 0.0]);
        }
        let skipped = n.saturating_sub(self.ring.len() / 2);
        self.since += skipped;
        self.clock += skipped as u64;
    }

    /// The analysis, when a frame is due.
    pub(crate) fn due(&mut self) -> Option<Spectrum> {
        let every = (self.rate / self.fps.max(1)).max(1) as usize;
        if self.since < every || self.fft.is_none() || self.ring.is_empty() {
            return None;
        }
        self.since %= every;
        Some(self.analyse())
    }

    fn frame_at(&self, back: usize) -> (f32, f32) {
        let n = self.ring.len() / 2;
        let i = (self.write + n - 1 - back % n) % n;
        (self.ring[i * 2], self.ring[i * 2 + 1])
    }

    /// Analyses the [`WINDOW`] frames ending `latency_frames` ago.
    pub fn analyse(&mut self) -> Spectrum {
        let lat = self.latency_frames();
        let mut sum = [0.0f64; 2];
        let mut peak = [0.0f32; 2];
        let mut scope = self.scope.then(|| Vec::with_capacity(SCOPE_POINTS * 2));
        let step = WINDOW / SCOPE_POINTS;
        for i in 0..WINDOW {
            // Oldest first.
            let (l, r) = self.frame_at(lat + WINDOW - 1 - i);
            sum[0] += (l * l) as f64;
            sum[1] += (r * r) as f64;
            peak[0] = peak[0].max(l.abs());
            peak[1] = peak[1].max(r.abs());
            self.input[0][i] = l * self.hann[i];
            self.input[1][i] = r * self.hann[i];
            if let Some(s) = scope.as_mut()
                && i % step == 0
            {
                s.push(l.clamp(-1.0, 1.0));
                s.push(r.clamp(-1.0, 1.0));
            }
        }
        let mut bands = vec![FLOOR_DB; BANDS];
        // Each channel on its own, so a wide or out-of-phase mix still shows.
        let ok = self.fft.as_ref().is_some_and(|f| {
            let [il, ir] = &mut self.input;
            let [sl, sr] = &mut self.spec;
            f.process_with_scratch(il, sl, &mut self.scratch).is_ok() && f.process_with_scratch(ir, sr, &mut self.scratch).is_ok()
        });
        if ok {
            for (p, (l, r)) in self.power.iter_mut().zip(self.spec[0].iter().zip(&self.spec[1])) {
                *p = 0.5 * (l.norm_sqr() + r.norm_sqr());
            }
            // A full-scale sine centred on a bin reads 1.0: the Hann window
            // keeps half the amplitude, and a real FFT splits it in two.
            let norm = 4.0 / WINDOW as f32;
            for (band, &(a, b)) in bands.iter_mut().zip(&self.bins) {
                if b > a {
                    let m = self.power[a..b].iter().fold(0.0f32, |m, &p| m.max(p));
                    *band = to_db(m.sqrt() * norm);
                }
            }
        }
        let rms = sum.map(|s| to_db((s / WINDOW as f64).sqrt() as f32));
        let shown = self.clock.saturating_sub(lat as u64);
        Spectrum {
            t: shown * 1000 / self.rate.max(1) as u64,
            bands,
            rms,
            peak: peak.map(to_db),
            scope,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(rate: u32, fps: u32, scope: bool) -> TapState {
        let mut s = TapState { fps, scope, ..Default::default() };
        s.prepare();
        s.configure(rate);
        s
    }

    fn feed(s: &mut TapState, frames: usize, f: impl Fn(usize) -> (f32, f32)) -> Vec<Spectrum> {
        let mut out = Vec::new();
        s.begin(s.rate, 512);
        for i in 0..frames {
            let (l, r) = f(i);
            s.push(&[l, r]);
            if let Some(sp) = s.due() {
                out.push(sp);
            }
        }
        out
    }

    fn sine(freq: f32, amp: f32, rate: u32) -> impl Fn(usize) -> (f32, f32) {
        move |i| {
            let v = (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * amp;
            (v, v)
        }
    }

    fn band_of(freq: f32) -> usize {
        (0..BANDS).find(|&k| freq >= band_edge(k) && freq < band_edge(k + 1)).unwrap()
    }

    fn loudest(sp: &Spectrum) -> usize {
        (0..BANDS).max_by(|&a, &b| sp.bands[a].total_cmp(&sp.bands[b])).unwrap()
    }

    #[test]
    fn bands_run_log_spaced_from_20_hz_to_20_khz() {
        assert!((band_edge(0) - 20.0).abs() < 1e-3);
        assert!((band_edge(BANDS) - 20_000.0).abs() < 0.5);
        // Each band is the same ratio wider than the last.
        let r = band_edge(1) / band_edge(0);
        assert!((band_edge(40) / band_edge(39) - r).abs() < 1e-4);
    }

    #[test]
    fn a_1_khz_sine_lands_in_its_band_at_full_scale() {
        for rate in [44_100, 48_000, 96_000] {
            let mut s = tap(rate, 30, false);
            let out = feed(&mut s, rate as usize, sine(1000.0, 1.0, rate));
            let last = out.last().unwrap();
            assert_eq!(loudest(last), band_of(1000.0), "{rate} Hz");
            let db = last.bands[band_of(1000.0)];
            // Hann scalloping is at most about 1.4 dB off a bin centre.
            assert!((-1.5..=0.2).contains(&db), "{rate}: {db} dB");
            // Far away it is way down.
            assert!(last.bands[band_of(100.0)] < -60.0, "{}", last.bands[band_of(100.0)]);
            assert!(last.bands[band_of(10_000.0)] < -60.0);
        }
    }

    #[test]
    fn half_the_amplitude_reads_6_db_lower() {
        let rate = 48_000;
        let a = feed(&mut tap(rate, 30, false), 8192, sine(1000.0, 0.5, rate)).pop().unwrap();
        let b = feed(&mut tap(rate, 30, false), 8192, sine(1000.0, 0.25, rate)).pop().unwrap();
        let k = band_of(1000.0);
        assert!((a.bands[k] - b.bands[k] - 6.02).abs() < 0.05, "{} vs {}", a.bands[k], b.bands[k]);
        // RMS of a sine is 3 dB under its peak; peak is the amplitude.
        assert!((a.peak[0] - to_db(0.5)).abs() < 0.05);
        assert!((a.peak[0] - a.rms[0] - 3.01).abs() < 0.1, "{} {}", a.peak[0], a.rms[0]);
    }

    #[test]
    fn low_bands_narrower_than_a_bin_still_read_their_tone() {
        let rate = 48_000;
        let out = feed(&mut tap(rate, 30, false), 16_384, sine(30.0, 1.0, rate)).pop().unwrap();
        assert!(out.bands[band_of(30.0)] > -6.0, "{}", out.bands[band_of(30.0)]);
    }

    #[test]
    fn silence_is_the_floor() {
        let out = feed(&mut tap(48_000, 30, true), 8192, |_| (0.0, 0.0)).pop().unwrap();
        assert!(out.bands.iter().all(|&b| b == FLOOR_DB));
        assert_eq!(out.rms, [FLOOR_DB; 2]);
        assert_eq!(out.peak, [FLOOR_DB; 2]);
        assert!(out.scope.unwrap().iter().all(|&v| v == 0.0));
    }

    #[test]
    fn bands_past_what_the_rate_holds_are_the_floor() {
        // 32 kHz holds up to 16 kHz.
        let rate = 32_000;
        let out = feed(&mut tap(rate, 30, false), 8192, sine(1000.0, 1.0, rate)).pop().unwrap();
        assert_eq!(out.bands[BANDS - 1], FLOOR_DB);
    }

    #[test]
    fn the_scope_of_a_mono_song_is_a_line_and_a_wide_one_is_not() {
        let rate = 48_000;
        let mono = feed(&mut tap(rate, 30, true), 8192, sine(220.0, 0.8, rate)).pop().unwrap();
        let pairs = mono.scope.unwrap();
        assert_eq!(pairs.len(), SCOPE_POINTS * 2);
        assert!(pairs.chunks(2).all(|p| p[0] == p[1]));
        let wide = feed(&mut tap(rate, 30, true), 8192, |i| {
            let t = i as f32 / rate as f32 * 2.0 * std::f32::consts::PI * 220.0;
            (t.sin() * 0.8, t.cos() * 0.8)
        })
        .pop()
        .unwrap();
        let spread = wide.scope.unwrap().chunks(2).map(|p| (p[0] - p[1]).abs()).fold(0.0f32, f32::max);
        assert!(spread > 0.5, "{spread}");
        // Not asked for: not sent.
        let none = feed(&mut tap(rate, 30, false), 4096, |_| (0.1, 0.1)).pop().unwrap();
        assert!(none.scope.is_none());
        assert!(serde_json::to_value(&none).unwrap().get("scope").is_none());
    }

    #[test]
    fn frames_come_at_the_asked_rate() {
        let rate = 48_000;
        assert_eq!(feed(&mut tap(rate, 30, false), rate as usize, |_| (0.0, 0.0)).len(), 30);
        assert_eq!(feed(&mut tap(rate, 60, false), rate as usize, |_| (0.0, 0.0)).len(), 60);
        assert_eq!(feed(&mut tap(rate, 15, false), rate as usize, |_| (0.0, 0.0)).len(), 15);
    }

    #[test]
    fn the_picture_is_held_back_by_the_output_latency() {
        let rate = 48_000;
        // Loud for a second, then silent: the frame right after the change
        // still shows the loud part while it's in the device's buffer.
        let lat = Duration::from_millis(100);
        let mut held = tap(rate, 30, false);
        held.latency = Some(lat);
        held.begin(rate, 512);
        assert_eq!(held.latency_frames(), 4_800 + 512);
        let mut plain = tap(rate, 30, false);
        plain.latency = Some(Duration::ZERO);
        let f = |i: usize| if i < rate as usize { (0.5, 0.5) } else { (0.0, 0.0) };
        let frames = rate as usize + 4_800;
        let h = feed(&mut held, frames, f).pop().unwrap();
        let p = feed(&mut plain, frames, f).pop().unwrap();
        assert_eq!(p.peak[0], FLOOR_DB, "no delay: already silent");
        assert!((h.peak[0] - to_db(0.5)).abs() < 0.01, "delayed: still loud");
        // The clock says what moment is shown.
        assert_eq!(p.t - h.t, 100);
        // Without a reported delay, the device block's length is used, and
        // a silly delay is capped.
        let mut s = tap(rate, 30, false);
        s.begin(rate, 480);
        assert_eq!(s.latency_frames(), 480);
        s.latency = Some(Duration::from_secs(5));
        assert_eq!(s.latency_frames(), (rate as f64 * MAX_LATENCY.as_secs_f64()) as usize);
    }

    #[test]
    fn a_watch_ends_by_itself_unless_renewed() {
        let t = VisTap::new();
        assert!(!t.active(), "nothing watches at first");
        assert_eq!(t.watch_for(), Duration::from_secs(3));
        t.set_watch_for(Duration::from_millis(80));
        t.watch(true, 60, false);
        assert!(t.active());
        std::thread::sleep(Duration::from_millis(50));
        t.watch(true, 60, false);
        std::thread::sleep(Duration::from_millis(50));
        assert!(t.active(), "the renewal kept it going");
        std::thread::sleep(Duration::from_millis(60));
        assert!(!t.active(), "ended without a renewal");
        t.watch(true, 60, false);
        t.watch(false, 60, false);
        assert!(!t.active(), "ended when asked");
    }

    #[test]
    fn the_asked_frame_rate_is_kept_in_range() {
        let t = VisTap::new();
        t.watch(true, 500, false);
        assert_eq!(t.lock().fps, MAX_FPS);
        t.watch(true, 0, false);
        assert_eq!(t.lock().fps, MIN_FPS);
    }

    #[test]
    fn json_shape() {
        let mut s = tap(48_000, 30, true);
        let sp = feed(&mut s, 4096, |_| (0.25, -0.25)).pop().unwrap();
        let v = serde_json::to_value(&sp).unwrap();
        assert_eq!(v["bands"].as_array().unwrap().len(), BANDS);
        assert_eq!(v["rms"].as_array().unwrap().len(), 2);
        assert_eq!(v["peak"].as_array().unwrap().len(), 2);
        assert_eq!(v["scope"].as_array().unwrap().len(), SCOPE_POINTS * 2);
        assert!(v["t"].is_u64());
    }

    /// The cost of one frame of analysis (run with `--ignored --nocapture`).
    #[test]
    #[ignore]
    fn cost_per_frame() {
        let rate = 48_000;
        let mut s = tap(rate, 60, true);
        let f = sine(1000.0, 0.5, rate);
        s.begin(rate, 512);
        for i in 0..WINDOW * 2 {
            let (l, r) = f(i);
            s.push(&[l, r]);
        }
        let n = 2_000;
        let start = Instant::now();
        for _ in 0..n {
            std::hint::black_box(s.analyse());
        }
        let per = start.elapsed() / n;
        // Copying a second of audio into the ring.
        let start = Instant::now();
        for i in 0..rate as usize {
            s.push(std::hint::black_box(&[i as f32, 0.0]));
        }
        let copy = start.elapsed();
        let share = (per.as_secs_f64() * 60.0 + copy.as_secs_f64()) * 100.0;
        println!("analysis {per:?} per frame, copy {copy:?} per second of audio, {share:.3}% of a core at 60 fps");
    }
}
