//! The sound of one song, worked out from its decoded audio: the tempo and
//! a beat grid for its start and its end (with where the bars begin), the
//! key, how energetic and loud it is, loudness over time, where the music
//! starts and stops, and a small feature vector that places it among the
//! rest of the library.
//!
//! Everything runs on a mono copy at [`RATE`]: a short-time Fourier
//! transform gives an onset curve (how much new sound each frame has), a
//! low-band onset curve, loudness, brightness and pitch classes. The tempo
//! comes from the onset curve's autocorrelation, the beats from a dynamic
//! programming tracker that follows that tempo, and the grid from a straight
//! line fitted through the beats. Bars start where the low band and the
//! harmony change most often. The key is the major or minor key profile the
//! song's pitch classes correlate with best.

use std::sync::Arc;

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

/// Analysis sample rate (mono).
pub const RATE: u32 = 11_025;
/// Transform length (186 ms) and hop (23 ms).
const N: usize = 2048;
const HOP: usize = 256;
/// Changes when the results would differ: older rows are done again.
pub const VERSION: i64 = 1;
/// Songs longer than this aren't analysed (long mixes, whole concerts).
pub const MAX_SECONDS: u32 = 20 * 60;
/// Too little to find a beat in.
pub const MIN_SECONDS: u32 = 8;
/// Beat grids are fitted over this much of the start and of the end.
const GRID_SPAN_S: f64 = 60.0;
/// Loudness over time, one value per this many seconds.
pub const SECTION_S: u32 = 2;
/// Quieter than this (dBFS) counts as silence.
const SILENT_DB: f32 = -50.0;
/// Tempo search range.
const MIN_BPM: f64 = 55.0;
const MAX_BPM: f64 = 215.0;

/// A steady beat over part of a song.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grid {
    /// A beat that starts a bar (ms into the song). Other bars start every
    /// four beats from it.
    pub downbeat_ms: f64,
    /// One beat (ms).
    pub beat_ms: f64,
    /// How well the beats fit a steady grid, 0 to 1.
    pub conf: f32,
    /// How sure the bar starts are, 0 to 1.
    pub downbeat_conf: f32,
}

impl Grid {
    pub fn bpm(&self) -> f64 {
        60_000.0 / self.beat_ms
    }
    /// The first bar start at or after `ms`.
    pub fn bar_at_or_after(&self, ms: f64) -> f64 {
        let bar = 4.0 * self.beat_ms;
        let k = ((ms - self.downbeat_ms) / bar).ceil();
        self.downbeat_ms + k * bar
    }
    /// The last bar start at or before `ms`.
    pub fn bar_at_or_before(&self, ms: f64) -> f64 {
        let bar = 4.0 * self.beat_ms;
        let k = ((ms - self.downbeat_ms) / bar).floor();
        self.downbeat_ms + k * bar
    }
}

/// What the analysis found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub duration_ms: u64,
    /// Tempo over the whole song, when there's a beat.
    pub bpm: Option<f32>,
    /// How steady and clear the beat is over the whole song, 0 to 1.
    pub beat_conf: f32,
    /// The grid over the song's first minute and its last.
    pub intro: Option<Grid>,
    pub outro: Option<Grid>,
    /// 0 to 11: C to B major; 12 to 23: C to B minor.
    pub key: Option<u8>,
    pub key_conf: f32,
    /// 0 (calm) to 1 (intense).
    pub energy: f32,
    /// Average loudness of the music, dBFS.
    pub loudness_db: f32,
    /// First and last sound (ms).
    pub cue_in_ms: u64,
    pub cue_out_ms: u64,
    /// Where the song is fully in, and where it starts to wind down (ms).
    pub intro_end_ms: u64,
    pub outro_start_ms: u64,
    /// The song begins or ends at full level (a segue, or a cut).
    pub starts_hot: bool,
    pub ends_hot: bool,
    /// Loudness every [`SECTION_S`] seconds, dBFS.
    pub sections: Vec<i8>,
    /// See [`feature_names`].
    pub features: Vec<f32>,
}

/// What each place in [`Analysis::features`] holds (all 0 to 1).
pub fn feature_names() -> [&'static str; 20] {
    [
        "tempo", "energy", "loudness", "onsets", "brightness", "dynamics", "major", "tonal", "c", "c#", "d", "d#", "e",
        "f", "f#", "g", "g#", "a", "a#", "b",
    ]
}

/// Mixes interleaved audio down to mono at [`RATE`] as it's decoded: each
/// output sample is the average of the input samples it covers.
pub struct Downmix {
    step: f64,
    /// Where the next output sample ends, in input samples.
    edge: f64,
    at: f64,
    sum: f64,
    n: u32,
    pub out: Vec<f32>,
}

impl Downmix {
    pub fn new(rate: u32) -> Self {
        let step = rate.max(1) as f64 / RATE as f64;
        Self { step, edge: step, at: 0.0, sum: 0.0, n: 0, out: Vec::new() }
    }

    pub fn push(&mut self, block: &[f32], channels: usize) {
        let ch = channels.max(1);
        for frame in block.chunks_exact(ch) {
            let v = frame.iter().sum::<f32>() / ch as f32;
            self.sum += v as f64;
            self.n += 1;
            self.at += 1.0;
            if self.at >= self.edge {
                self.out.push((self.sum / self.n as f64) as f32);
                self.sum = 0.0;
                self.n = 0;
                self.edge += self.step;
            }
        }
    }

    pub fn seconds(&self) -> f64 {
        self.out.len() as f64 / RATE as f64
    }
}

/// Per-frame measurements.
struct Frames {
    /// New sound per frame, over the whole band and below 150 Hz.
    flux: Vec<f32>,
    low: Vec<f32>,
    rms_db: Vec<f32>,
    /// Spectral centroid, Hz.
    centroid: Vec<f32>,
    /// Pitch classes per frame (C first), unnormalised.
    chroma: Vec<[f32; 12]>,
}

fn hann(n: usize) -> Vec<f32> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).collect()
}

/// Bin to pitch class (55 Hz to 2 kHz; others None).
fn chroma_map() -> Vec<Option<usize>> {
    (0..=N / 2)
        .map(|b| {
            let f = b as f64 * RATE as f64 / N as f64;
            if !(55.0..=2000.0).contains(&f) {
                return None;
            }
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            Some((midi.round() as i64).rem_euclid(12) as usize)
        })
        .collect()
}

fn frames(x: &[f32]) -> Frames {
    let mut planner = RealFftPlanner::<f32>::new();
    let fft: Arc<dyn realfft::RealToComplex<f32>> = planner.plan_fft_forward(N);
    let win = hann(N);
    let cmap = chroma_map();
    let bins = N / 2 + 1;
    let low_bins = (150.0 * N as f64 / RATE as f64) as usize;
    let mut input = fft.make_input_vec();
    let mut spec: Vec<Complex<f32>> = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    let mut prev = vec![0f32; bins];
    let mut cur = vec![0f32; bins];
    let count = x.len() / HOP + 1;
    let mut f = Frames {
        flux: Vec::with_capacity(count),
        low: Vec::with_capacity(count),
        rms_db: Vec::with_capacity(count),
        centroid: Vec::with_capacity(count),
        chroma: Vec::with_capacity(count),
    };
    let half = N / 2;
    for k in 0..count {
        // Centred: frame k is about the sample at k * HOP.
        let centre = k * HOP;
        for (i, slot) in input.iter_mut().enumerate() {
            let at = (centre + i) as isize - half as isize;
            let v = if at >= 0 && (at as usize) < x.len() { x[at as usize] } else { 0.0 };
            *slot = v * win[i];
        }
        if fft.process_with_scratch(&mut input, &mut spec, &mut scratch).is_err() {
            break;
        }
        let mut flux = 0.0f32;
        let mut low = 0.0f32;
        let mut wsum = 0.0f32;
        let mut msum = 0.0f32;
        let mut chroma = [0f32; 12];
        for b in 0..bins {
            let mag = spec[b].norm() * (2.0 / N as f32);
            let lm = (1.0 + 1000.0 * mag).ln();
            cur[b] = lm;
            let d = lm - prev[b];
            if d > 0.0 {
                flux += d;
                if (1..=low_bins).contains(&b) {
                    low += d;
                }
            }
            let hz = b as f32 * RATE as f32 / N as f32;
            wsum += hz * mag;
            msum += mag;
            if let Some(pc) = cmap[b] {
                chroma[pc] += mag;
            }
        }
        std::mem::swap(&mut prev, &mut cur);
        let lo = centre.min(x.len());
        let hi = (centre + HOP).min(x.len());
        let energy = if hi > lo { x[lo..hi].iter().map(|v| v * v).sum::<f32>() / (hi - lo) as f32 } else { 0.0 };
        f.flux.push(if k == 0 { 0.0 } else { flux });
        f.low.push(if k == 0 { 0.0 } else { low });
        f.rms_db.push(10.0 * (energy + 1e-12).log10());
        f.centroid.push(if msum > 1e-9 { wsum / msum } else { 0.0 });
        f.chroma.push(chroma);
    }
    f
}

pub fn fps() -> f64 {
    RATE as f64 / HOP as f64
}

/// The onset curve peaks when a sharp sound is a quarter of the window
/// ahead of a frame's centre (where the window rises fastest), so beats
/// found on it are this much early.
fn onset_lag_ms() -> f64 {
    (N / 4) as f64 * 1000.0 / RATE as f64
}

fn ms_of(frame: f64) -> f64 {
    frame * 1000.0 / fps()
}

/// The onset curve with its slow trend taken out (half-wave rectified)
/// and scaled to unit spread.
fn onset_env(flux: &[f32]) -> Vec<f32> {
    let w = 10usize;
    let n = flux.len();
    let mut pre = vec![0f64; n + 1];
    for i in 0..n {
        pre[i + 1] = pre[i] + flux[i] as f64;
    }
    let mut env: Vec<f32> = (0..n)
        .map(|i| {
            let lo = i.saturating_sub(w);
            let hi = (i + w + 1).min(n);
            let mean = (pre[hi] - pre[lo]) / (hi - lo) as f64;
            (flux[i] as f64 - mean).max(0.0) as f32
        })
        .collect();
    let sd = (env.iter().map(|v| (v * v) as f64).sum::<f64>() / n.max(1) as f64).sqrt();
    if sd > 1e-9 {
        for v in env.iter_mut() {
            *v /= sd as f32;
        }
    }
    env
}

/// The beat period (frames, fractional) and how clear the pulse is (0 to 1).
fn tempo(env: &[f32]) -> Option<(f64, f32)> {
    let fps = fps();
    let lmin = (fps * 60.0 / MAX_BPM).floor() as usize;
    let lmax = (fps * 60.0 / MIN_BPM).ceil() as usize;
    let n = env.len();
    if n < lmax * 4 {
        return None;
    }
    let ac = |l: usize| -> f64 {
        if l >= n {
            return 0.0;
        }
        let mut s = 0.0f64;
        for i in 0..n - l {
            s += env[i] as f64 * env[i + l] as f64;
        }
        s / (n - l) as f64
    };
    let ac0 = env.iter().map(|v| (v * v) as f64).sum::<f64>() / n as f64;
    if ac0 <= 1e-12 {
        return None;
    }
    let raw: Vec<f64> = (0..=lmax * 2 + 2).map(|l| if l >= lmin / 2 { ac(l) } else { 0.0 }).collect();
    // A preference for tempos near 120 BPM (one octave either way).
    let prior = |l: f64| {
        let bpm = 60.0 * fps / l;
        (-0.5 * ((bpm / 120.0).log2() / 1.0).powi(2)).exp()
    };
    let score = |l: usize| raw[l] + 0.5 * raw[(2 * l).min(raw.len() - 1)];
    let mut best = (lmin, f64::MIN);
    for l in lmin..=lmax {
        let s = score(l) * prior(l as f64);
        if s > best.1 {
            best = (l, s);
        }
    }
    let mut l = best.0;
    // Double time when its pulse is about as strong (a fast song read at
    // half speed), up to about 185 BPM.
    let half = l / 2;
    if half >= lmin && 60.0 * fps / half as f64 <= 185.0 {
        let peak = (half.saturating_sub(1)..=half + 1).max_by(|a, b| raw[*a].total_cmp(&raw[*b])).unwrap_or(half);
        if raw[peak] >= 0.9 * raw[l] {
            l = peak;
        }
    }
    // Between frames: a parabola through the peak and its neighbours.
    let (a, b, c) = (score(l.saturating_sub(1)), score(l), score(l + 1));
    let denom = a - 2.0 * b + c;
    let off = if denom.abs() > 1e-12 { (0.5 * (a - c) / denom).clamp(-0.5, 0.5) } else { 0.0 };
    let clarity = (raw[l] / ac0) as f32;
    Some((l as f64 + off, clarity.clamp(0.0, 1.0)))
}

/// Beat frames following `period`, by dynamic programming (each beat is
/// strong and about one period after the last).
fn track_beats(env: &[f32], period: f64) -> Vec<usize> {
    let n = env.len();
    if n == 0 || period < 2.0 {
        return Vec::new();
    }
    let tight = 100.0f64;
    let mut cum = vec![0f64; n];
    let mut back = vec![usize::MAX; n];
    let far = (2.0 * period).round() as usize;
    let near = ((period / 2.0).round() as usize).max(1);
    for t in 0..n {
        let mut best = f64::MIN;
        let mut bi = usize::MAX;
        if t >= near {
            let lo = t.saturating_sub(far);
            for (p, c) in cum.iter().enumerate().take(t - near + 1).skip(lo) {
                let r = ((t - p) as f64 / period).ln();
                let s = c - tight * r * r;
                if s > best {
                    best = s;
                    bi = p;
                }
            }
        }
        cum[t] = env[t] as f64 + if bi == usize::MAX { 0.0 } else { best.max(0.0) };
        back[t] = if best > 0.0 { bi } else { usize::MAX };
    }
    let tail = (period.ceil() as usize).min(n);
    let mut t = (n - tail..n).max_by(|a, b| cum[*a].total_cmp(&cum[*b])).unwrap_or(n - 1);
    let mut beats = vec![t];
    while back[t] != usize::MAX {
        t = back[t];
        beats.push(t);
    }
    beats.reverse();
    beats
}

/// A straight line through beats (frames): (time of beat 0, period) in
/// frames, each beat's index on it, and the fit (0 to 1).
fn fit_grid(beats: &[f64], period: f64) -> Option<(f64, f64, Vec<i64>, f32)> {
    if beats.len() < 8 {
        return None;
    }
    let mut idx = vec![0i64];
    for w in beats.windows(2) {
        let steps = ((w[1] - w[0]) / period).round().max(1.0) as i64;
        idx.push(idx.last().unwrap() + steps);
    }
    let n = beats.len() as f64;
    let mx = idx.iter().map(|&i| i as f64).sum::<f64>() / n;
    let my = beats.iter().sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, y) in idx.iter().zip(beats) {
        let dx = *i as f64 - mx;
        sxy += dx * (y - my);
        sxx += dx * dx;
    }
    if sxx <= 0.0 {
        return None;
    }
    let p = sxy / sxx;
    let t0 = my - p * mx;
    let resid = (idx.iter().zip(beats).map(|(i, y)| (y - (t0 + p * *i as f64)).powi(2)).sum::<f64>() / n).sqrt();
    // Steady to within a few percent of a beat is a good fit.
    let fit = (1.0 - (resid / (0.12 * p))).clamp(0.0, 1.0) as f32;
    Some((t0, p, idx, fit))
}

/// Of the four places a bar could start, the one where the low band and
/// the harmony change most, and how sure that is.
fn downbeat_phase(f: &Frames, t0: f64, p: f64, from: usize, to: usize) -> (usize, f32) {
    let n = f.low.len();
    let at = |frame: f64| -> usize { (frame.round().max(0.0) as usize).min(n.saturating_sub(1)) };
    let novelty = |i: usize| -> f32 {
        if i < 4 || i + 4 >= n {
            return 0.0;
        }
        let mean = |a: usize, b: usize| {
            let mut c = [0f32; 12];
            for fr in &f.chroma[a..b] {
                for (x, y) in c.iter_mut().zip(fr) {
                    *x += y;
                }
            }
            let norm = c.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
            c.map(|v| v / norm)
        };
        let (b, a) = (mean(i - 4, i), mean(i, i + 4));
        1.0 - b.iter().zip(&a).map(|(x, y)| x * y).sum::<f32>()
    };
    let mut score = [0f64; 4];
    let mut count = [0u32; 4];
    let k0 = ((from as f64 - t0) / p).ceil() as i64;
    let k1 = ((to as f64 - t0) / p).floor() as i64;
    for k in k0..=k1 {
        let i = at(t0 + p * k as f64);
        let s = f.low[i] as f64 + 4.0 * novelty(i) as f64;
        let ph = k.rem_euclid(4) as usize;
        score[ph] += s;
        count[ph] += 1;
    }
    for i in 0..4 {
        if count[i] > 0 {
            score[i] /= count[i] as f64;
        }
    }
    let mut order = [0usize, 1, 2, 3];
    order.sort_by(|a, b| score[*b].total_cmp(&score[*a]));
    let (best, second) = (score[order[0]], score[order[1]]);
    let conf = if best > 1e-9 { ((best - second) / best * 2.5).clamp(0.0, 1.0) as f32 } else { 0.0 };
    (order[0], conf)
}

/// The grid over frames `from..to` (beats from the whole-song tracker).
fn grid_for(f: &Frames, beats: &[usize], period: f64, from: usize, to: usize, clarity: f32) -> Option<Grid> {
    let part: Vec<f64> = beats.iter().filter(|&&b| b >= from && b < to).map(|&b| b as f64).collect();
    let (t0, p, _, fit) = fit_grid(&part, period)?;
    let (phase, down_conf) = downbeat_phase(f, t0, p, from, to);
    // The first bar start inside the span.
    let mut first = t0 + p * phase as f64;
    while first - 4.0 * p >= from as f64 {
        first -= 4.0 * p;
    }
    Some(Grid {
        downbeat_ms: ms_of(first) + onset_lag_ms(),
        beat_ms: ms_of(p),
        conf: fit.min(pulse(clarity)),
        downbeat_conf: down_conf,
    })
}

/// How clear the pulse is, as a confidence.
fn pulse(clarity: f32) -> f32 {
    ((clarity - 0.04) / 0.2).clamp(0.0, 1.0)
}

/// Krumhansl and Kessler's key profiles.
const MAJOR: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

fn pearson(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let ma = a.iter().sum::<f32>() / 12.0;
    let mb = b.iter().sum::<f32>() / 12.0;
    let (mut s, mut sa, mut sb) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        let (x, y) = (a[i] - ma, b[i] - mb);
        s += x * y;
        sa += x * x;
        sb += y * y;
    }
    if sa <= 1e-12 || sb <= 1e-12 { 0.0 } else { s / (sa * sb).sqrt() }
}

/// The best key (0 to 23) for a pitch-class profile, and how sure.
pub fn key_of(chroma: &[f32; 12]) -> (Option<u8>, f32) {
    let mut scores: Vec<(u8, f32)> = Vec::with_capacity(24);
    for tonic in 0..12 {
        let rot = |p: &[f32; 12]| -> [f32; 12] { std::array::from_fn(|i| p[(i + 12 - tonic) % 12]) };
        scores.push((tonic as u8, pearson(chroma, &rot(&MAJOR))));
        scores.push((tonic as u8 + 12, pearson(chroma, &rot(&MINOR))));
    }
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, second) = (scores[0], scores[1]);
    if best.1 < 0.3 {
        return (None, 0.0);
    }
    // Relative major and minor share their notes: being torn between them
    // still says a lot about which keys fit.
    let relative = |a: u8, b: u8| (a < 12) != (b < 12) && (if a < 12 { (a + 9) % 12 == b - 12 } else { (b + 9) % 12 == a - 12 });
    let gap = if relative(best.0, second.0) { best.1 - scores[2].1 } else { best.1 - second.1 };
    let conf = (gap * 8.0).clamp(0.0, 1.0) * ((best.1 - 0.3) / 0.4).clamp(0.0, 1.0);
    (Some(best.0), conf)
}

/// Key names for display ("A minor").
pub fn key_name(key: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C♯", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B"];
    let k = (key % 12) as usize;
    if key < 12 { format!("{} major", NAMES[k]) } else { format!("{} minor", NAMES[k]) }
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// Analyses mono audio at [`RATE`]. None when it's too short.
pub fn analyse(x: &[f32]) -> Option<Analysis> {
    let secs = x.len() as f64 / RATE as f64;
    if secs < MIN_SECONDS as f64 {
        return None;
    }
    let duration_ms = (secs * 1000.0).round() as u64;
    let f = frames(x);
    let n = f.flux.len();
    let fps = fps();

    // Loudness, where the music starts and stops.
    let loud: Vec<f32> = f.rms_db.iter().copied().filter(|d| *d > SILENT_DB).collect();
    let loudness_db = if loud.is_empty() {
        SILENT_DB
    } else {
        10.0 * (loud.iter().map(|d| 10f32.powf(d / 10.0)).sum::<f32>() / loud.len() as f32).log10()
    };
    let gate = (loudness_db - 30.0).max(SILENT_DB);
    let first = f.rms_db.iter().position(|d| *d > gate).unwrap_or(0);
    let last = f.rms_db.iter().rposition(|d| *d > gate).unwrap_or(n.saturating_sub(1));
    let cue_in_ms = ms_of(first as f64).round() as u64;
    let cue_out_ms = (ms_of(last as f64 + 1.0).round() as u64).min(duration_ms);

    // Loudness every SECTION_S seconds.
    let per = (SECTION_S as f64 * fps).round() as usize;
    let sections: Vec<i8> = f
        .rms_db
        .chunks(per.max(1))
        .map(|c| {
            let p = c.iter().map(|d| 10f32.powf(d / 10.0)).sum::<f32>() / c.len() as f32;
            (10.0 * (p + 1e-12).log10()).clamp(-90.0, 0.0).round() as i8
        })
        .collect();
    // Fully in / winding down: the first and last stretch within 6 dB of the
    // song's loudness.
    let full = loudness_db - 6.0;
    let sec_ms = SECTION_S as u64 * 1000;
    let intro_end_ms = sections.iter().position(|d| *d as f32 >= full).map_or(cue_in_ms, |i| i as u64 * sec_ms).max(cue_in_ms);
    let outro_start_ms = sections
        .iter()
        .rposition(|d| *d as f32 >= full)
        .map_or(cue_out_ms, |i| (i as u64 + 1) * sec_ms)
        .min(cue_out_ms);
    // Starts or ends at full level: the first or last 300 ms of sound are
    // as loud as the song.
    let edge = (0.3 * fps).round() as usize;
    let level = |r: std::ops::Range<usize>| -> f32 {
        let r = r.start.min(n)..r.end.min(n);
        if r.is_empty() {
            return SILENT_DB;
        }
        let p = f.rms_db[r.clone()].iter().map(|d| 10f32.powf(d / 10.0)).sum::<f32>() / r.len() as f32;
        10.0 * (p + 1e-12).log10()
    };
    let starts_hot = first <= edge && level(0..edge) > loudness_db - 8.0;
    let ends_hot = n.saturating_sub(last + 1) <= edge && level(n.saturating_sub(edge)..n) > loudness_db - 8.0;

    // Tempo and beats.
    let env = onset_env(&f.flux);
    let (mut bpm, mut beat_conf, mut intro, mut outro) = (None, 0.0f32, None, None);
    if let Some((period, clarity)) = tempo(&env) {
        let beats: Vec<usize> = track_beats(&env, period).into_iter().filter(|&b| b >= first && b <= last).collect();
        let all: Vec<f64> = beats.iter().map(|&b| b as f64).collect();
        if let Some((_, p, _, fit)) = fit_grid(&all, period) {
            bpm = Some((60.0 * fps / p) as f32);
            beat_conf = fit.min(pulse(clarity));
            let span = (GRID_SPAN_S * fps) as usize;
            intro = grid_for(&f, &beats, p, first, (first + span).min(last + 1), clarity);
            outro = grid_for(&f, &beats, p, (last + 1).saturating_sub(span).max(first), last + 1, clarity);
        }
    }

    // Key, from pitch classes weighted by loudness.
    let mut chroma = [0f32; 12];
    for (c, d) in f.chroma.iter().zip(&f.rms_db) {
        if *d <= gate {
            continue;
        }
        let norm = c.iter().sum::<f32>().max(1e-9);
        for (x, y) in chroma.iter_mut().zip(c) {
            *x += y / norm;
        }
    }
    let (key, key_conf) = key_of(&chroma);

    // Energy: loudness, how busy, how bright.
    let active: Vec<usize> = (first..=last.min(n.saturating_sub(1))).collect();
    let onsets = if active.is_empty() { 0.0 } else { active.iter().map(|&i| f.flux[i]).sum::<f32>() / active.len() as f32 };
    let mut cents: Vec<f32> = active.iter().map(|&i| f.centroid[i]).collect();
    let centroid = median(&mut cents);
    let loud_n = ((loudness_db + 30.0) / 24.0).clamp(0.0, 1.0);
    let onset_n = (onsets / 60.0).clamp(0.0, 1.0);
    let bright_n = (centroid / 2500.0).clamp(0.0, 1.0);
    let energy = (0.5 * loud_n + 0.3 * onset_n + 0.2 * bright_n).clamp(0.0, 1.0);
    let mut secs_db: Vec<f32> = sections.iter().filter(|d| (**d as f32) > SILENT_DB).map(|d| *d as f32).collect();
    secs_db.sort_by(|a, b| a.total_cmp(b));
    let dynamics = if secs_db.len() >= 4 {
        let lo = secs_db[secs_db.len() / 10];
        let hi = secs_db[secs_db.len() * 9 / 10];
        ((hi - lo) / 20.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let total = chroma.iter().sum::<f32>().max(1e-9);
    let mut features = vec![
        bpm.map_or(0.0, |b| ((b - 50.0) / 170.0).clamp(0.0, 1.0)),
        energy,
        loud_n,
        onset_n,
        bright_n,
        dynamics,
        match key {
            Some(k) if k < 12 => 1.0,
            _ => 0.0,
        },
        key_conf,
    ];
    features.extend(chroma.iter().map(|c| c / total));
    let round = |v: f32| (v * 1000.0).round() / 1000.0;
    Some(Analysis {
        duration_ms,
        bpm: bpm.map(|b| (b * 10.0).round() / 10.0),
        beat_conf: round(beat_conf),
        intro,
        outro,
        key,
        key_conf: round(key_conf),
        energy: round(energy),
        loudness_db: (loudness_db * 10.0).round() / 10.0,
        cue_in_ms,
        cue_out_ms,
        intro_end_ms,
        outro_start_ms,
        starts_hot,
        ends_hot,
        sections,
        features: features.into_iter().map(round).collect(),
    })
}

/// Decodes a whole file to mono at [`RATE`]. Err when it can't be read;
/// Ok(None) when it's longer than [`MAX_SECONDS`].
pub fn decode_mono(bytes: Vec<u8>, hint: Option<&str>) -> Result<Option<Vec<f32>>, String> {
    use crate::audio::decode::{DecodeError, TrackDecoder};
    let mut dec = TrackDecoder::open(Box::new(std::io::Cursor::new(bytes)), hint).map_err(|e| e.to_string())?;
    let mut mix = Downmix::new(dec.rate);
    let channels = dec.channels;
    loop {
        match dec.next_block() {
            Ok(Some(b)) => mix.push(b, channels),
            Ok(None) => break,
            // A damaged end: what was decoded is the song.
            Err(DecodeError::Io(_) | DecodeError::Corrupt(_)) if mix.seconds() >= MIN_SECONDS as f64 => break,
            Err(e) => return Err(e.to_string()),
        }
        if mix.seconds() > MAX_SECONDS as f64 {
            return Ok(None);
        }
    }
    Ok(Some(mix.out))
}

#[cfg(test)]
pub(crate) mod synth {
    //! Test audio with known beats and pitches.

    use super::RATE;

    /// A click track: a short burst every beat (`bpm`), the first beat of
    /// every bar louder and lower, starting at `offset_s`, over a quiet
    /// chord of `notes` (Hz).
    pub fn clicks(bpm: f64, secs: f64, offset_s: f64, notes: &[f64]) -> Vec<f32> {
        let n = (secs * RATE as f64) as usize;
        let mut x = vec![0f32; n];
        let rate = RATE as f64;
        for (i, s) in x.iter_mut().enumerate() {
            let t = i as f64 / rate;
            let mut v = 0.0;
            for f in notes {
                v += (2.0 * std::f64::consts::PI * f * t).sin() * 0.08;
            }
            *s = v as f32;
        }
        let beat = 60.0 / bpm;
        let mut k = 0usize;
        loop {
            let t = offset_s + k as f64 * beat;
            if t >= secs - 0.1 {
                break;
            }
            let down = k % 4 == 0;
            let (amp, freq) = if down { (0.9, 60.0) } else { (0.45, 1800.0) };
            let start = (t * rate) as usize;
            let len = (0.06 * rate) as usize;
            for j in 0..len.min(n - start) {
                let tt = j as f64 / rate;
                let env = (-tt * 60.0).exp();
                let noise = if down { 0.0 } else { ((j * 7919 % 97) as f64 / 48.5 - 1.0) * 0.5 };
                x[start + j] += (amp * env * ((2.0 * std::f64::consts::PI * freq * tt).sin() + noise)) as f32;
            }
            k += 1;
        }
        x
    }
}

#[cfg(test)]
mod tests {
    use super::synth::clicks;
    use super::*;

    const A_MINOR: [f64; 3] = [220.0, 261.63, 329.63];
    const C_MAJOR: [f64; 3] = [261.63, 329.63, 392.0];

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn finds_the_tempo_of_click_tracks() {
        for bpm in [90.0, 120.0, 128.0, 140.0, 174.0] {
            let a = analyse(&clicks(bpm, 60.0, 0.5, &C_MAJOR)).unwrap();
            let got = a.bpm.expect("a beat") as f64;
            assert!(close(got, bpm, bpm * 0.01), "{bpm} BPM came out as {got}");
            assert!(a.beat_conf > 0.6, "{bpm} BPM: confidence {}", a.beat_conf);
        }
    }

    #[test]
    fn the_grid_lands_on_the_beats_and_the_bars() {
        let bpm = 120.0;
        let a = analyse(&clicks(bpm, 90.0, 0.75, &C_MAJOR)).unwrap();
        for g in [a.intro.unwrap(), a.outro.unwrap()] {
            assert!(close(g.beat_ms, 500.0, 5.0), "beat {}", g.beat_ms);
            // Bars start on the loud low clicks: 0.75 s + 2 s per bar.
            let off = (g.downbeat_ms - 750.0).rem_euclid(2000.0);
            let err = off.min(2000.0 - off);
            assert!(err < 30.0, "bar start {} is {err} ms off", g.downbeat_ms);
            assert!(g.conf > 0.6 && g.downbeat_conf > 0.3, "{g:?}");
        }
        let o = a.outro.unwrap();
        assert!(o.downbeat_ms > 25_000.0, "the outro grid is from the end: {}", o.downbeat_ms);
    }

    #[test]
    fn finds_the_key() {
        let a = analyse(&clicks(100.0, 30.0, 0.2, &A_MINOR)).unwrap();
        assert_eq!(a.key, Some(9 + 12), "A minor, got {:?}", a.key.map(key_name));
        let c = analyse(&clicks(100.0, 30.0, 0.2, &C_MAJOR)).unwrap();
        assert_eq!(c.key, Some(0), "C major, got {:?}", c.key.map(key_name));
        assert_eq!(key_name(21), "A minor");
    }

    #[test]
    fn cue_points_skip_silence() {
        let mut x = vec![0f32; RATE as usize * 3];
        x.extend(clicks(120.0, 40.0, 0.0, &C_MAJOR));
        x.extend(vec![0f32; RATE as usize * 4]);
        let a = analyse(&x).unwrap();
        assert!(close(a.cue_in_ms as f64, 3000.0, 80.0), "in {}", a.cue_in_ms);
        assert!(close(a.cue_out_ms as f64, 43_000.0, 200.0), "out {}", a.cue_out_ms);
        assert_eq!(a.duration_ms, 47_000);
        assert!(!a.starts_hot && !a.ends_hot);
        assert!(a.intro_end_ms >= a.cue_in_ms && a.outro_start_ms <= a.cue_out_ms);
        assert_eq!(a.features.len(), feature_names().len());
        assert!(a.features.iter().all(|v| (0.0..=1.0).contains(v)), "{:?}", a.features);
    }

    #[test]
    fn a_song_that_runs_on_is_hot_at_both_ends() {
        let x = clicks(120.0, 30.0, 0.0, &[220.0, 440.0, 660.0, 880.0].map(|f| f * 1.0));
        let a = analyse(&x).unwrap();
        assert!(a.starts_hot && a.ends_hot, "{a:?}");
    }

    #[test]
    fn no_beat_in_a_held_chord() {
        let n = RATE as usize * 30;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / RATE as f64;
                (A_MINOR.iter().map(|f| (2.0 * std::f64::consts::PI * f * t).sin() * 0.2).sum::<f64>()) as f32
            })
            .collect();
        let a = analyse(&x).unwrap();
        assert!(a.beat_conf < 0.3, "steady tone has no beat: {}", a.beat_conf);
    }

    #[test]
    fn too_short_is_none() {
        assert!(analyse(&vec![0.1f32; RATE as usize * 3]).is_none());
    }

    #[test]
    fn downmix_averages_to_the_analysis_rate() {
        let mut d = Downmix::new(44_100);
        let block: Vec<f32> = (0..44_100 * 2).map(|i| if i % 2 == 0 { 1.0 } else { 0.0 }).collect();
        d.push(&block, 2);
        assert!((d.out.len() as i64 - RATE as i64).abs() <= 1, "{}", d.out.len());
        assert!(d.out.iter().all(|v| (*v - 0.5).abs() < 1e-6));
    }
}
