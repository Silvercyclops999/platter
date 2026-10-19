//! The visualizer's tap: a read-only copy of what goes out, taken after the
//! sound settings (EQ, leveling, limiter) and before the volume, so the
//! picture doesn't shrink when the volume goes down. It also works while
//! the output is bit-perfect, since it only reads.
//!
//! The output callback does as little as it can and never waits or
//! allocates: while someone watches, it copies each frame into a ring of
//! atomics made in advance by [`VisTap::watch`], then publishes how far it
//! wrote. Everything else happens off the audio thread. The player's own
//! task asks for a [`Spectrum`] at the watched frame rate
//! ([`VisTap::frame`]), which reads the last [`WINDOW`] frames being heard
//! (held back by the output's delay), runs a Hann window and a real FFT per
//! channel, and folds the result into [`BANDS`] log-spaced bands, RMS and
//! peak per channel, and [`SCOPE_POINTS`] stereo pairs. One frame is made
//! and sent at a time, so nothing queues up.
//!
//! Each window renews its own watch ([`VisTap::watch`] with its caller id);
//! a watch that isn't renewed within [`WATCH_FOR`] ends by itself, and one
//! window ending its watch leaves the others running.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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
/// The ring holds the window plus the longest hold-back at this rate; above
/// it the hold-back is capped a little shorter.
const RING_RATE: u32 = 192_000;
const RING_FRAMES: usize = WINDOW + (RING_RATE as usize / 4) + 1;

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

// ---------------------------------------------------------------- the ring

/// Stereo frames as f32 bits in atomics: one writer (the output callback),
/// readers anywhere, no lock. A reader racing the writer can see a frame or
/// two from either side of a wrap, which a picture doesn't mind.
pub struct Ring {
    data: Box<[AtomicU32]>,
    /// Frames ever written (the next one goes at `written % frames`).
    written: AtomicU64,
}

impl Ring {
    fn new(frames: usize) -> Self {
        Self { data: (0..frames * 2).map(|_| AtomicU32::new(0)).collect(), written: AtomicU64::new(0) }
    }

    fn frames(&self) -> usize {
        self.data.len() / 2
    }

    /// Frames ever written.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Acquire)
    }

    /// The frame `back` frames before the newest one written by `upto`.
    fn read(&self, upto: u64, back: usize) -> (f32, f32) {
        let n = self.frames() as u64;
        if back as u64 >= upto.min(n) {
            return (0.0, 0.0);
        }
        let i = ((upto - 1 - back as u64) % n) as usize;
        (f32::from_bits(self.data[i * 2].load(Ordering::Relaxed)), f32::from_bits(self.data[i * 2 + 1].load(Ordering::Relaxed)))
    }
}

/// The output callback's handle on the ring for one block.
pub(crate) struct TapWriter<'a> {
    ring: &'a Ring,
    pos: u64,
}

impl TapWriter<'_> {
    /// Copies one output frame (mono doubled, past two channels left out).
    #[inline]
    pub(crate) fn push(&mut self, frame: &[f32]) {
        let (l, r) = match frame {
            [] => (0.0, 0.0),
            [m] => (*m, *m),
            [l, r, ..] => (*l, *r),
        };
        let i = (self.pos % self.ring.frames() as u64) as usize;
        self.ring.data[i * 2].store(l.to_bits(), Ordering::Relaxed);
        self.ring.data[i * 2 + 1].store(r.to_bits(), Ordering::Relaxed);
        self.pos += 1;
    }

    /// `n` frames of silence (the part of a block nothing filled).
    pub(crate) fn silence(&mut self, n: usize) {
        for _ in 0..n.min(self.ring.frames()) {
            self.push(&[0.0, 0.0]);
        }
        self.pos += n.saturating_sub(self.ring.frames()) as u64;
    }

    /// Publishes the block.
    pub(crate) fn finish(self) {
        self.ring.written.store(self.pos, Ordering::Release);
    }
}

// ---------------------------------------------------------------- watching

#[derive(Debug, Clone, Copy)]
struct Watcher {
    until_ms: u64,
    fps: u32,
    scope: bool,
}

/// What the watchers want together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    pub fps: u32,
    pub scope: bool,
}

pub struct VisTap {
    epoch: Instant,
    /// Watched until this many ms after `epoch` (0: not watched): the
    /// latest of every watcher's.
    until_ms: AtomicU64,
    /// The asked frame rate (the highest any watcher wants).
    fps: AtomicU32,
    scope: AtomicBool,
    /// How long a renewal lasts, in ms (tests shorten it).
    watch_ms: AtomicU64,
    watchers: Mutex<HashMap<String, Watcher>>,
    /// Made on the first watch, never on the audio thread.
    ring: OnceLock<Ring>,
    /// Written by the callback: the output rate, the last block's length,
    /// and the device's reported delay past it (ns; u64::MAX: not reported).
    rate: AtomicU32,
    last_block: AtomicU32,
    latency_ns: AtomicU64,
    /// `ring.written` when the current watch began.
    start: AtomicU64,
    /// The analysis buffers, used off the audio thread only.
    state: Mutex<Analysis>,
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
            fps: AtomicU32::new(30),
            scope: AtomicBool::new(false),
            watch_ms: AtomicU64::new(WATCH_FOR.as_millis() as u64),
            watchers: Mutex::new(HashMap::new()),
            ring: OnceLock::new(),
            rate: AtomicU32::new(48_000),
            last_block: AtomicU32::new(0),
            latency_ns: AtomicU64::new(u64::MAX),
            start: AtomicU64::new(0),
            state: Mutex::new(Analysis::default()),
        }
    }

    fn now_ms(&self) -> u64 {
        // Never 0, so 0 can mean "not watched".
        self.epoch.elapsed().as_millis() as u64 + 1
    }

    /// Starts, renews or ends `caller`'s watch at `fps` (clamped to 1..=60).
    /// `scope` adds the stereo pairs. Buffers are made here, off the audio
    /// thread. Returns what all the watchers want together, or None when
    /// nobody watches now.
    pub fn watch(&self, caller: &str, on: bool, fps: u32, scope: bool) -> Option<Wanted> {
        let now = self.now_ms();
        if on {
            // Build first, outside any lock the callback could meet.
            let ring = self.ring.get_or_init(|| Ring::new(RING_FRAMES));
            let was_on = self.active();
            let mut st = self.state.lock();
            st.prepare();
            if !was_on {
                st.last_written = None;
                // A fresh watch counts its clock from here. Published
                // before `until_ms`, so the first frame reads it.
                self.start.store(ring.written(), Ordering::SeqCst);
            }
        }
        let mut w = self.watchers.lock();
        if on {
            w.insert(caller.to_string(), Watcher { until_ms: now + self.watch_ms.load(Ordering::SeqCst), fps: fps.clamp(MIN_FPS, MAX_FPS), scope });
        } else {
            w.remove(caller);
        }
        w.retain(|_, x| x.until_ms > now);
        let until = w.values().map(|x| x.until_ms).max().unwrap_or(0);
        let wanted = (until != 0).then(|| Wanted {
            fps: w.values().map(|x| x.fps).max().unwrap_or(MIN_FPS),
            scope: w.values().any(|x| x.scope),
        });
        if let Some(x) = wanted {
            self.fps.store(x.fps, Ordering::SeqCst);
            self.scope.store(x.scope, Ordering::SeqCst);
        }
        self.until_ms.store(until, Ordering::SeqCst);
        wanted
    }

    /// Someone is watching now.
    pub fn active(&self) -> bool {
        let until = self.until_ms.load(Ordering::Relaxed);
        until != 0 && self.now_ms() < until
    }

    /// The frame rate asked for.
    pub fn fps(&self) -> u32 {
        self.fps.load(Ordering::Relaxed)
    }

    /// How long a renewal lasts.
    pub fn watch_for(&self) -> Duration {
        Duration::from_millis(self.watch_ms.load(Ordering::SeqCst))
    }

    #[cfg(test)]
    pub(crate) fn set_watch_for(&self, d: Duration) {
        self.watch_ms.store(d.as_millis() as u64, Ordering::SeqCst);
    }

    /// The ring, once a watch made it.
    pub fn ring(&self) -> Option<&Ring> {
        self.ring.get()
    }

    // ------------------------------------------------ the output callback

    /// For the output callback: a writer for a block of `frames` at `rate`
    /// while someone watches. Lock-free and allocation-free.
    #[inline]
    pub(crate) fn writer(&self, rate: u32, frames: usize) -> Option<TapWriter<'_>> {
        if !self.active() {
            return None;
        }
        let ring = self.ring.get()?;
        self.rate.store(rate.max(1), Ordering::Relaxed);
        self.last_block.store(frames.min(u32::MAX as usize) as u32, Ordering::Relaxed);
        Some(TapWriter { ring, pos: ring.written.load(Ordering::Relaxed) })
    }

    /// The output's own delay past the block being written, so the picture
    /// is held back to match what is heard. Lock-free.
    pub fn set_latency(&self, latency: Duration) {
        let ns = latency.min(MAX_LATENCY).as_nanos() as u64;
        self.latency_ns.store(ns, Ordering::Relaxed);
    }

    // ------------------------------------------------ off the audio thread

    /// Delay the picture is held back by, in frames: the block just written
    /// plays after the device's own delay.
    pub fn latency_frames(&self) -> usize {
        let rate = self.rate.load(Ordering::Relaxed) as u64;
        let ns = self.latency_ns.load(Ordering::Relaxed);
        let delay = if ns == u64::MAX { 0 } else { (ns * rate / 1_000_000_000) as usize };
        let frames = self.last_block.load(Ordering::Relaxed) as usize + delay;
        frames.min(RING_FRAMES - WINDOW - 1)
    }

    /// The next frame to send, made here (off the audio thread): None while
    /// nobody watches, or when no audio has gone out since the last one.
    pub fn frame(&self) -> Option<Spectrum> {
        if !self.active() {
            return None;
        }
        let ring = self.ring.get()?;
        let written = ring.written();
        let rate = self.rate.load(Ordering::Relaxed).max(1);
        let lat = self.latency_frames();
        let start = self.start.load(Ordering::SeqCst);
        let scope = self.scope.load(Ordering::Relaxed);
        let mut st = self.state.lock();
        if st.last_written == Some(written) {
            return None;
        }
        st.last_written = Some(written);
        let shown = written.saturating_sub(start).saturating_sub(lat as u64);
        let mut sp = st.analyse(rate, |back| ring.read(written, lat + back), scope);
        sp.t = shown * 1000 / rate as u64;
        Some(sp)
    }
}

// ---------------------------------------------------------------- analysis

/// The FFT and its buffers, made once by the first watch.
#[derive(Default)]
pub struct Analysis {
    fft: Option<Arc<dyn RealToComplex<f32>>>,
    hann: Vec<f32>,
    /// Left and right, windowed.
    input: [Vec<f32>; 2],
    spec: [Vec<Complex<f32>>; 2],
    scratch: Vec<Complex<f32>>,
    /// Power per bin, both channels averaged.
    power: Vec<f32>,
    /// Bin range per band, for `bins_rate`.
    bins: Vec<(usize, usize)>,
    bins_rate: u32,
    /// The ring position the last frame was made from.
    last_written: Option<u64>,
}

impl Analysis {
    fn prepare(&mut self) {
        if self.fft.is_some() {
            return;
        }
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(WINDOW);
        self.input = [fft.make_input_vec(), fft.make_input_vec()];
        self.spec = [fft.make_output_vec(), fft.make_output_vec()];
        self.scratch = fft.make_scratch_vec();
        self.power = vec![0.0; WINDOW / 2 + 1];
        self.hann = (0..WINDOW)
            .map(|i| {
                let x = std::f32::consts::PI * 2.0 * i as f32 / WINDOW as f32;
                0.5 - 0.5 * x.cos()
            })
            .collect();
        self.fft = Some(fft);
    }

    /// The band table for `rate` (only when it changes).
    fn bins_for(&mut self, rate: u32) {
        if rate == self.bins_rate && !self.bins.is_empty() {
            return;
        }
        self.bins_rate = rate;
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

    /// Analyses [`WINDOW`] frames; `frame(back)` gives the frame `back`
    /// before the newest one shown. `t` is left 0 for the caller.
    fn analyse(&mut self, rate: u32, frame: impl Fn(usize) -> (f32, f32), scope: bool) -> Spectrum {
        self.prepare();
        self.bins_for(rate);
        let mut sum = [0.0f64; 2];
        let mut peak = [0.0f32; 2];
        let mut pairs = scope.then(|| Vec::with_capacity(SCOPE_POINTS * 2));
        let step = WINDOW / SCOPE_POINTS;
        for i in 0..WINDOW {
            // Oldest first.
            let (l, r) = frame(WINDOW - 1 - i);
            sum[0] += (l * l) as f64;
            sum[1] += (r * r) as f64;
            peak[0] = peak[0].max(l.abs());
            peak[1] = peak[1].max(r.abs());
            self.input[0][i] = l * self.hann[i];
            self.input[1][i] = r * self.hann[i];
            if let Some(s) = pairs.as_mut()
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
        Spectrum { t: 0, bands, rms: sum.map(|s| to_db((s / WINDOW as f64).sqrt() as f32)), peak: peak.map(to_db), scope: pairs }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// A watched tap fed `frames` frames of `f` in blocks of 512, as the
    /// callback would, asking for a frame every `rate / fps` frames.
    fn run(tap: &VisTap, rate: u32, fps: u32, frames: usize, f: impl Fn(usize) -> (f32, f32)) -> Vec<Spectrum> {
        let every = (rate / fps) as usize;
        let mut out = Vec::new();
        let mut i = 0;
        while i < frames {
            let n = 512.min(frames - i);
            let mut w = tap.writer(rate, n).expect("watched");
            for k in 0..n {
                let (l, r) = f(i + k);
                w.push(&[l, r]);
                if (i + k + 1) % every == 0 {
                    // A frame made mid-block sees what was published so far.
                    let pos = w.pos;
                    w.ring.written.store(pos, Ordering::Release);
                    if let Some(s) = tap.frame() {
                        out.push(s);
                    }
                }
            }
            w.finish();
            i += n;
        }
        out
    }

    fn watched(rate: u32, scope: bool) -> VisTap {
        let t = VisTap::new();
        t.watch("test", true, 30, scope);
        t.set_latency(Duration::ZERO);
        // The block length counts as delay: the test frames read at once.
        let w = t.writer(rate, 0).unwrap();
        w.finish();
        t
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

    /// Like `run` but with no hold-back at all (the block length is 0).
    fn analyse(rate: u32, frames: usize, scope: bool, f: impl Fn(usize) -> (f32, f32)) -> Spectrum {
        let t = watched(rate, scope);
        let w = t.writer(rate, 0).unwrap();
        let mut w = w;
        for i in 0..frames {
            let (l, r) = f(i);
            w.push(&[l, r]);
        }
        w.finish();
        t.frame().unwrap()
    }

    #[test]
    fn bands_run_log_spaced_from_20_hz_to_20_khz() {
        assert!((band_edge(0) - 20.0).abs() < 1e-3);
        assert!((band_edge(BANDS) - 20_000.0).abs() < 0.5);
        let r = band_edge(1) / band_edge(0);
        assert!((band_edge(40) / band_edge(39) - r).abs() < 1e-4);
    }

    #[test]
    fn a_1_khz_sine_lands_in_its_band_at_full_scale() {
        for rate in [44_100, 48_000, 96_000] {
            let last = analyse(rate, rate as usize, false, sine(1000.0, 1.0, rate));
            assert_eq!(loudest(&last), band_of(1000.0), "{rate} Hz");
            let db = last.bands[band_of(1000.0)];
            // Hann scalloping is at most about 1.4 dB off a bin centre.
            assert!((-1.5..=0.2).contains(&db), "{rate}: {db} dB");
            assert!(last.bands[band_of(100.0)] < -60.0, "{}", last.bands[band_of(100.0)]);
            assert!(last.bands[band_of(10_000.0)] < -60.0);
        }
    }

    #[test]
    fn half_the_amplitude_reads_6_db_lower() {
        let rate = 48_000;
        let a = analyse(rate, 8192, false, sine(1000.0, 0.5, rate));
        let b = analyse(rate, 8192, false, sine(1000.0, 0.25, rate));
        let k = band_of(1000.0);
        assert!((a.bands[k] - b.bands[k] - 6.02).abs() < 0.05, "{} vs {}", a.bands[k], b.bands[k]);
        assert!((a.peak[0] - to_db(0.5)).abs() < 0.05);
        assert!((a.peak[0] - a.rms[0] - 3.01).abs() < 0.1, "{} {}", a.peak[0], a.rms[0]);
    }

    #[test]
    fn low_bands_narrower_than_a_bin_still_read_their_tone() {
        let rate = 48_000;
        let out = analyse(rate, 16_384, false, sine(30.0, 1.0, rate));
        assert!(out.bands[band_of(30.0)] > -6.0, "{}", out.bands[band_of(30.0)]);
    }

    #[test]
    fn silence_is_the_floor() {
        let out = analyse(48_000, 8192, true, |_| (0.0, 0.0));
        assert!(out.bands.iter().all(|&b| b == FLOOR_DB));
        assert_eq!(out.rms, [FLOOR_DB; 2]);
        assert_eq!(out.peak, [FLOOR_DB; 2]);
        assert!(out.scope.unwrap().iter().all(|&v| v == 0.0));
    }

    #[test]
    fn bands_past_what_the_rate_holds_are_the_floor() {
        let rate = 32_000;
        let out = analyse(rate, 8192, false, sine(1000.0, 1.0, rate));
        assert_eq!(out.bands[BANDS - 1], FLOOR_DB);
    }

    #[test]
    fn the_scope_of_a_mono_song_is_a_line_and_a_wide_one_is_not() {
        let rate = 48_000;
        let mono = analyse(rate, 8192, true, sine(220.0, 0.8, rate));
        let pairs = mono.scope.unwrap();
        assert_eq!(pairs.len(), SCOPE_POINTS * 2);
        assert!(pairs.chunks(2).all(|p| p[0] == p[1]));
        let wide = analyse(rate, 8192, true, |i| {
            let t = i as f32 / rate as f32 * 2.0 * std::f32::consts::PI * 220.0;
            (t.sin() * 0.8, t.cos() * 0.8)
        });
        let spread = wide.scope.unwrap().chunks(2).map(|p| (p[0] - p[1]).abs()).fold(0.0f32, f32::max);
        assert!(spread > 0.5, "{spread}");
        let none = analyse(rate, 4096, false, |_| (0.1, 0.1));
        assert!(none.scope.is_none());
        assert!(serde_json::to_value(&none).unwrap().get("scope").is_none());
    }

    #[test]
    fn frames_come_only_when_new_audio_went_out() {
        let rate = 48_000;
        let t = watched(rate, false);
        assert_eq!(run(&t, rate, 30, rate as usize, |_| (0.1, 0.1)).len(), 30);
        // Nothing new written since: no frame.
        assert!(t.frame().is_none());
    }

    #[test]
    fn the_picture_is_held_back_by_the_output_latency() {
        let rate = 48_000;
        let lat = Duration::from_millis(100);
        let feed = |t: &VisTap| {
            let mut w = t.writer(rate, 512).unwrap();
            for i in 0..rate as usize + 4_800 {
                let v = if i < rate as usize { 0.5 } else { 0.0 };
                w.push(&[v, v]);
            }
            w.finish();
        };
        let held = watched(rate, false);
        held.set_latency(lat);
        feed(&held);
        assert_eq!(held.latency_frames(), 4_800 + 512);
        let plain = watched(rate, false);
        plain.set_latency(Duration::ZERO);
        feed(&plain);
        let h = held.frame().unwrap();
        let p = plain.frame().unwrap();
        assert_eq!(p.peak[0], FLOOR_DB, "no delay: already silent");
        assert!((h.peak[0] - to_db(0.5)).abs() < 0.01, "delayed: still loud");
        assert_eq!(p.t - h.t, 100);
        // A silly delay is capped.
        held.set_latency(Duration::from_secs(5));
        assert_eq!(held.latency_frames(), 512 + (rate as f64 * MAX_LATENCY.as_secs_f64()) as usize);
    }

    #[test]
    fn a_watch_ends_by_itself_unless_renewed() {
        let t = VisTap::new();
        assert!(!t.active(), "nothing watches at first");
        assert!(t.writer(48_000, 512).is_none(), "and the callback writes nothing");
        assert_eq!(t.watch_for(), Duration::from_secs(3));
        t.set_watch_for(Duration::from_millis(80));
        t.watch("a", true, 60, false);
        assert!(t.active());
        std::thread::sleep(Duration::from_millis(50));
        t.watch("a", true, 60, false);
        std::thread::sleep(Duration::from_millis(50));
        assert!(t.active(), "the renewal kept it going");
        std::thread::sleep(Duration::from_millis(60));
        assert!(!t.active(), "ended without a renewal");
        assert!(t.frame().is_none());
        t.watch("a", true, 60, false);
        t.watch("a", false, 60, false);
        assert!(!t.active(), "ended when asked");
    }

    #[test]
    fn each_window_has_its_own_watch() {
        let t = VisTap::new();
        assert_eq!(t.watch("main", true, 60, false), Some(Wanted { fps: 60, scope: false }));
        assert_eq!(t.watch("phone", true, 30, true), Some(Wanted { fps: 60, scope: true }));
        // One window going leaves the other running, with what it wants.
        assert_eq!(t.watch("main", false, 0, false), Some(Wanted { fps: 30, scope: true }));
        assert!(t.active());
        assert_eq!(t.fps(), 30);
        assert_eq!(t.watch("phone", false, 0, false), None);
        assert!(!t.active());
    }

    #[test]
    fn a_fresh_watch_starts_its_clock_where_the_audio_is() {
        let rate = 48_000;
        let t = watched(rate, false);
        let mut w = t.writer(rate, 0).unwrap();
        for _ in 0..rate {
            w.push(&[0.1, 0.1]);
        }
        w.finish();
        assert_eq!(t.frame().unwrap().t, 1000);
        t.watch("test", false, 0, false);
        t.watch("test", true, 30, false);
        let mut w = t.writer(rate, 0).unwrap();
        for _ in 0..rate / 2 {
            w.push(&[0.1, 0.1]);
        }
        w.finish();
        assert_eq!(t.frame().unwrap().t, 500);
    }

    #[test]
    fn the_asked_frame_rate_is_kept_in_range() {
        let t = VisTap::new();
        t.watch("a", true, 500, false);
        assert_eq!(t.fps(), MAX_FPS);
        t.watch("a", true, 0, false);
        assert_eq!(t.fps(), MIN_FPS);
    }

    #[test]
    fn json_shape() {
        let sp = analyse(48_000, 4096, true, |_| (0.25, -0.25));
        let v = serde_json::to_value(&sp).unwrap();
        assert_eq!(v["bands"].as_array().unwrap().len(), BANDS);
        assert_eq!(v["rms"].as_array().unwrap().len(), 2);
        assert_eq!(v["peak"].as_array().unwrap().len(), 2);
        assert_eq!(v["scope"].as_array().unwrap().len(), SCOPE_POINTS * 2);
        assert!(v["t"].is_u64());
    }

    /// The cost on and off the audio thread (run with `--ignored --nocapture`).
    #[test]
    #[ignore]
    fn cost_per_frame() {
        let rate = 48_000;
        let t = watched(rate, true);
        let f = sine(1000.0, 0.5, rate);
        let n = 2_000u32;
        // On the audio thread: copying a second of audio into the ring.
        let start = Instant::now();
        let mut w = t.writer(rate, 512).unwrap();
        for i in 0..rate as usize {
            let (l, r) = f(i);
            w.push(std::hint::black_box(&[l, r]));
        }
        w.finish();
        let copy = start.elapsed();
        // Off it: one frame of analysis.
        let start = Instant::now();
        for _ in 0..n {
            t.state.lock().last_written = None;
            std::hint::black_box(t.frame());
        }
        let per = start.elapsed() / n;
        println!(
            "audio thread: {copy:?} per second of audio ({:.4}% of a core); player task: {per:?} per frame ({:.3}% of a core at 60 fps)",
            copy.as_secs_f64() * 100.0,
            per.as_secs_f64() * 60.0 * 100.0
        );
    }
}
