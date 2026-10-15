//! The sound stage between decode and output: loudness leveling, a 10-band
//! EQ, a lookahead limiter, crossfade curves and the silence meter that
//! smart crossfades use.
//!
//! ```text
//!  segment samples ─ × leveling gain ─┐
//!                                     ├─ mix ─ EQ ─ limiter ─ × volume ─▶ output
//!  fading-out segment ─ × fade curve ─┘
//! ```
//!
//! Everything that runs in the output callback ([`Eq::process`],
//! [`Limiter::process`], [`Chain::process`]) is allocation-free: buffers are
//! sized when the output format is set ([`Chain::configure`]).

use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

pub const EQ_BANDS: usize = 10;
/// Band centres in Hz.
pub const EQ_FREQS: [f64; EQ_BANDS] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// Each band and the preamp go from -12 to +12 dB.
pub const EQ_MAX_DB: f32 = 12.0;
pub const PREAMP_MAX_DB: f32 = 12.0;
pub const CROSSFADE_MIN_S: u32 = 1;
pub const CROSSFADE_MAX_S: u32 = 12;
/// Smart crossfades never overlap more than this.
pub const SMART_OVERLAP_CAP_MS: u64 = 8_000;
/// Setting changes ramp over this long.
pub const SETTING_RAMP_MS: u64 = 200;
/// Assumed peak (linear) for tracks with no peak data: a little over full
/// scale, since lossy decodes overshoot.
pub const UNKNOWN_PEAK: f32 = 1.12;
/// Quiet means a 50 ms RMS below -40 dBFS.
const QUIET_POWER: f64 = 1e-4;
const METER_WINDOW_MS: u64 = 50;
/// Octave-wide bands.
const EQ_Q: f64 = 1.41;
/// While a band ramps, its coefficients are recomputed this often (frames).
const EQ_UPDATE_FRAMES: usize = 32;
/// The limiter keeps peaks at or under -0.5 dBFS.
pub const LIMIT_CEILING: f32 = 0.944;
const LIMIT_LOOKAHEAD_MS: f64 = 1.5;
const LIMIT_RELEASE_MS: f64 = 120.0;
/// Turning the limiter on or off dips the output this long each way (its
/// lookahead adds or removes a little delay).
const SWITCH_MS: f64 = 5.0;

/// Crossfade setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CrossfadeMode {
    /// Overlap only the quiet tail and head of the two songs.
    #[default]
    Smart,
    /// Fade over a fixed number of seconds.
    Fixed,
    Off,
}

/// Loudness leveling setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Leveling {
    /// Album gain while an album plays in order, track gain otherwise.
    #[default]
    Smart,
    Track,
    Album,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crossfade {
    Off,
    Smart,
    Fixed { ms: u64 },
}

/// What the engine's sound stage does. The default changes nothing (the
/// engine's own default; the app passes the user's settings).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DspSettings {
    pub crossfade: Crossfade,
    /// Consecutive songs of one album, in album order, never crossfade.
    pub album_gapless: bool,
    pub leveling: Leveling,
    pub preamp_db: f32,
    pub limiter: bool,
    /// Band gains in dB; None is EQ off.
    pub eq: Option<[f32; EQ_BANDS]>,
    /// Headphone correction for the output in use, run before the EQ.
    pub correction: Option<Correction>,
}

impl Default for DspSettings {
    fn default() -> Self {
        Self {
            crossfade: Crossfade::Off,
            album_gapless: true,
            leveling: Leveling::Off,
            preamp_db: 0.0,
            limiter: false,
            eq: None,
            correction: None,
        }
    }
}

impl DspSettings {
    /// Nothing that changes a sample: what bit-perfect output plays with.
    pub fn untouched() -> Self {
        Self::default()
    }
}

// ------------------------------------------------------ headphone correction

/// Most filters a correction profile can have (published ones use 10).
pub const CORRECTION_MAX: usize = 20;

/// Parametric filter shapes used by headphone correction profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

/// One parametric filter: centre (or corner) frequency in Hz, gain in dB, Q.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeqFilter {
    pub kind: FilterKind,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
}

impl PeqFilter {
    const NONE: Self = Self { kind: FilterKind::Peaking, freq: 1000.0, gain_db: 0.0, q: 1.0 };

    fn coefs(&self, rate: f64) -> Option<Coefs> {
        let ok = self.freq.is_finite()
            && self.gain_db.is_finite()
            && self.q.is_finite()
            && self.freq > 0.0
            && self.q > 0.0
            && self.gain_db != 0.0
            && (self.freq as f64) < rate * 0.45;
        if !ok {
            return None;
        }
        let (f, db, q) = (self.freq as f64, self.gain_db as f64, self.q as f64);
        Some(match self.kind {
            FilterKind::Peaking => Coefs::peaking_q(rate, f, db, q),
            FilterKind::LowShelf => Coefs::shelf(rate, f, db, q, false),
            FilterKind::HighShelf => Coefs::shelf(rate, f, db, q, true),
        })
    }
}

/// A headphone correction curve: a preamp (negative, so boosts can't clip)
/// and up to [`CORRECTION_MAX`] filters. Fixed size so settings stay `Copy`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Correction {
    pub preamp_db: f32,
    filters: [PeqFilter; CORRECTION_MAX],
    len: usize,
}

impl Correction {
    /// Extra filters past [`CORRECTION_MAX`] are dropped.
    pub fn new(preamp_db: f32, filters: &[PeqFilter]) -> Self {
        let mut f = [PeqFilter::NONE; CORRECTION_MAX];
        let len = filters.len().min(CORRECTION_MAX);
        f[..len].copy_from_slice(&filters[..len]);
        Self {
            preamp_db: if preamp_db.is_finite() { preamp_db.clamp(-30.0, 0.0) } else { 0.0 },
            filters: f,
            len,
        }
    }

    pub fn filters(&self) -> &[PeqFilter] {
        &self.filters[..self.len]
    }

    /// The curve's gain in dB at `freq`, preamp included.
    pub fn response_db(&self, rate: u32, freq: f64) -> f64 {
        let rate = rate.max(8_000) as f64;
        let w = 2.0 * PI * freq / rate;
        let m: f64 = self.filters().iter().filter_map(|f| f.coefs(rate)).map(|c| c.magnitude(w)).product();
        20.0 * m.log10() + self.preamp_db as f64
    }

    /// The loudest the curve gets in the audible range (linear), preamp
    /// included. Used for the limiter's skip test.
    pub fn max_gain(&self, rate: u32) -> f32 {
        let top = (rate.max(8_000) as f64 * 0.5).min(22_000.0);
        let steps = 240;
        let max = (0..=steps)
            .map(|i| self.response_db(rate, 20.0 * (top / 20.0).powf(i as f64 / steps as f64)))
            .fold(f64::MIN, f64::max);
        (10f64.powf(max / 20.0) * 1.06) as f32
    }
}

/// Runs a [`Correction`]: preamp, then its filters in series.
/// Allocation-free once configured.
pub struct CorrectionStage {
    rate: f64,
    ch: usize,
    on: Option<Correction>,
    gain: f32,
    coefs: Vec<Coefs>,
    state: Vec<[f64; 2]>,
}

impl CorrectionStage {
    pub fn new(rate: u32, ch: usize) -> Self {
        let mut s = Self {
            rate: 48_000.0,
            ch: 2,
            on: None,
            gain: 1.0,
            coefs: Vec::with_capacity(CORRECTION_MAX),
            state: Vec::new(),
        };
        s.configure(rate, ch);
        s
    }

    /// New output format (allocates; not for the callback).
    pub fn configure(&mut self, rate: u32, ch: usize) {
        self.rate = rate.max(8_000) as f64;
        self.ch = ch.max(1);
        self.state = vec![[0.0; 2]; CORRECTION_MAX * self.ch];
        let on = self.on;
        self.set(on);
    }

    /// Swaps the curve at once (the chain dips around it while playing).
    pub fn set(&mut self, c: Option<Correction>) {
        self.on = c;
        self.coefs.clear();
        self.state.iter_mut().for_each(|z| *z = [0.0; 2]);
        self.gain = 1.0;
        if let Some(c) = c {
            self.gain = db_to_lin(c.preamp_db);
            for f in c.filters() {
                if let Some(k) = f.coefs(self.rate) {
                    self.coefs.push(k);
                }
            }
        }
    }

    pub fn current(&self) -> Option<Correction> {
        self.on
    }

    #[inline]
    pub fn process(&mut self, frame: &mut [f32]) {
        if self.on.is_none() {
            return;
        }
        if self.gain != 1.0 {
            frame.iter_mut().for_each(|s| *s *= self.gain);
        }
        let ch = self.ch;
        for (i, c) in self.coefs.iter().enumerate() {
            let st = &mut self.state[i * ch..(i + 1) * ch];
            for (s, z) in frame.iter_mut().zip(st.iter_mut()) {
                let x = *s as f64;
                let y = c.b0 * x + z[0];
                z[0] = c.b1 * x - c.a1 * y + z[1];
                z[1] = c.b2 * x - c.a2 * y;
                *s = y as f32;
            }
        }
    }
}

impl DspSettings {
    /// The longest overlap this setting can ask for (extra decode-ahead).
    pub fn max_overlap_ms(&self) -> u64 {
        match self.crossfade {
            Crossfade::Off => 0,
            Crossfade::Smart => SMART_OVERLAP_CAP_MS,
            Crossfade::Fixed { ms } => ms,
        }
    }
}

/// Loudness data and measured silence for one track.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TrackAudio {
    pub gain_db: Option<f32>,
    pub album_gain_db: Option<f32>,
    /// Linear peaks (can be a little over 1 for lossy files).
    pub peak: Option<f32>,
    pub album_peak: Option<f32>,
    /// Quiet lead-in and run-out, measured on an earlier play.
    pub head_ms: Option<u64>,
    pub tail_ms: Option<u64>,
}

pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The leveling gain (linear) for a track and the peak it can reach after
/// that gain. `album_in_order`: the track plays as part of its album, in
/// album order. Missing gain data applies only the preamp.
pub fn level_gain(info: Option<&TrackAudio>, s: &DspSettings, album_in_order: bool) -> (f32, f32) {
    let use_album = match s.leveling {
        Leveling::Off => None,
        Leveling::Track => Some(false),
        Leveling::Album => Some(true),
        Leveling::Smart => Some(album_in_order),
    };
    let (gain, peak) = match (use_album, info) {
        (Some(true), Some(i)) => (i.album_gain_db.or(i.gain_db), i.album_peak.or(i.peak)),
        (Some(false), Some(i)) => (i.gain_db.or(i.album_gain_db), i.peak.or(i.album_peak)),
        (None, Some(i)) => (None, i.peak.or(i.album_peak)),
        (_, None) => (None, None),
    };
    let db = gain.unwrap_or(0.0) + s.preamp_db;
    let g = if db == 0.0 { 1.0 } else { db_to_lin(db) };
    let peak = peak.filter(|p| p.is_finite() && *p > 0.0).unwrap_or(UNKNOWN_PEAK);
    (g, peak * g)
}

/// Equal-power fade curves over `len` frames, sampled at frame centres.
pub fn fade_out(i: usize, len: usize) -> f32 {
    ((i as f32 + 0.5) / len.max(1) as f32 * std::f32::consts::FRAC_PI_2).cos()
}
pub fn fade_in(i: usize, len: usize) -> f32 {
    ((i as f32 + 0.5) / len.max(1) as f32 * std::f32::consts::FRAC_PI_2).sin()
}

// ------------------------------------------------------------------- EQ

#[derive(Debug, Clone, Copy, PartialEq)]
struct Coefs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coefs {
    const IDENTITY: Self = Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    /// A peaking filter (the usual bilinear-transform design): exactly `db`
    /// at `f0`, octave-wide.
    fn peaking(rate: f64, f0: f64, db: f64) -> Self {
        Self::peaking_q(rate, f0, db, EQ_Q)
    }

    fn peaking_q(rate: f64, f0: f64, db: f64, q: f64) -> Self {
        let a = 10f64.powf(db / 40.0);
        let w0 = 2.0 * PI * f0 / rate;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: -2.0 * cs / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: -2.0 * cs / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }

    /// A low or high shelf (same bilinear-transform family): `db` well past
    /// the corner, half of it at `f0`.
    fn shelf(rate: f64, f0: f64, db: f64, q: f64, high: bool) -> Self {
        let a = 10f64.powf(db / 40.0);
        let w0 = 2.0 * PI * f0 / rate;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let k = 2.0 * a.sqrt() * alpha;
        let s = if high { -1.0 } else { 1.0 };
        // Low shelf, with the sign of every (A-1) term flipped for a high one.
        let b0 = a * ((a + 1.0) - s * (a - 1.0) * cs + k);
        let b1 = 2.0 * s * a * ((a - 1.0) - s * (a + 1.0) * cs);
        let b2 = a * ((a + 1.0) - s * (a - 1.0) * cs - k);
        let a0 = (a + 1.0) + s * (a - 1.0) * cs + k;
        let a1 = -2.0 * s * ((a - 1.0) + s * (a + 1.0) * cs);
        let a2 = (a + 1.0) + s * (a - 1.0) * cs - k;
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    /// |H| at angular frequency `w` (radians per sample).
    fn magnitude(&self, w: f64) -> f64 {
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
    }
}

/// Whether a band can be used at this rate (its centre well under Nyquist).
fn band_usable(rate: f64, b: usize) -> bool {
    EQ_FREQS[b] < rate * 0.45
}

/// The loudest the EQ curve gets anywhere in the audible range (linear).
/// Used to tell whether the limiter can have anything to do.
pub fn eq_max_gain(bands: &[f32; EQ_BANDS], rate: u32) -> f32 {
    let rate = rate.max(8_000) as f64;
    let coefs: Vec<Coefs> = (0..EQ_BANDS)
        .filter(|&b| bands[b] != 0.0 && band_usable(rate, b))
        .map(|b| Coefs::peaking(rate, EQ_FREQS[b], bands[b] as f64))
        .collect();
    if coefs.is_empty() {
        return 1.0;
    }
    let top = (rate * 0.5).min(22_000.0);
    let mut max = 0.0f64;
    let steps = 240;
    for i in 0..=steps {
        let f = 20.0 * (top / 20.0).powf(i as f64 / steps as f64);
        let w = 2.0 * PI * f / rate;
        let m: f64 = coefs.iter().map(|c| c.magnitude(w)).product();
        max = max.max(m);
    }
    // A little margin for transients and the points between grid steps.
    (max * 1.06) as f32
}

/// The 10-band peaking EQ. Gain changes ramp over [`SETTING_RAMP_MS`]; a
/// band at 0 dB costs nothing and the whole EQ is bypassed when flat.
pub struct Eq {
    rate: f64,
    ch: usize,
    cur: [f32; EQ_BANDS],
    target: [f32; EQ_BANDS],
    step: [f32; EQ_BANDS],
    /// Coefficient updates left in the current ramp.
    updates_left: u32,
    countdown: usize,
    coefs: [Coefs; EQ_BANDS],
    active: [bool; EQ_BANDS],
    any_active: bool,
    /// Per band and channel: the filter's two state values.
    state: Vec<[f64; 2]>,
}

impl Eq {
    pub fn new(rate: u32, ch: usize) -> Self {
        let mut e = Self {
            rate: rate as f64,
            ch,
            cur: [0.0; EQ_BANDS],
            target: [0.0; EQ_BANDS],
            step: [0.0; EQ_BANDS],
            updates_left: 0,
            countdown: 0,
            coefs: [Coefs::IDENTITY; EQ_BANDS],
            active: [false; EQ_BANDS],
            any_active: false,
            state: Vec::new(),
        };
        e.configure(rate, ch);
        e
    }

    /// New output format: resizes the state (allocates; not for the
    /// callback) and jumps to the target curve.
    pub fn configure(&mut self, rate: u32, ch: usize) {
        self.rate = rate.max(8_000) as f64;
        self.ch = ch.max(1);
        self.state = vec![[0.0; 2]; EQ_BANDS * self.ch];
        self.snap();
    }

    /// Sets the curve to reach over `ramp_frames` (0: at once).
    pub fn set_target(&mut self, bands: [f32; EQ_BANDS], ramp_frames: usize) {
        let mut t = bands;
        for v in t.iter_mut() {
            *v = if v.is_finite() { v.clamp(-EQ_MAX_DB, EQ_MAX_DB) } else { 0.0 };
        }
        self.target = t;
        let updates = ramp_frames / EQ_UPDATE_FRAMES;
        if updates == 0 {
            self.snap();
            return;
        }
        for b in 0..EQ_BANDS {
            self.step[b] = (self.target[b] - self.cur[b]) / updates as f32;
        }
        self.updates_left = updates as u32;
        self.countdown = 0;
    }

    pub fn target(&self) -> [f32; EQ_BANDS] {
        self.target
    }

    pub fn current(&self) -> [f32; EQ_BANDS] {
        self.cur
    }

    fn snap(&mut self) {
        self.cur = self.target;
        self.updates_left = 0;
        self.update_coefs();
    }

    fn update_coefs(&mut self) {
        let mut any = false;
        for (b, &freq) in EQ_FREQS.iter().enumerate() {
            let on = self.cur[b] != 0.0 && band_usable(self.rate, b);
            if !on && self.active[b] {
                self.state[b * self.ch..(b + 1) * self.ch].fill([0.0; 2]);
            }
            self.active[b] = on;
            self.coefs[b] = if on {
                Coefs::peaking(self.rate, freq, self.cur[b] as f64)
            } else {
                Coefs::IDENTITY
            };
            any |= on;
        }
        self.any_active = any;
    }

    /// True when the EQ changes nothing (flat and not ramping).
    pub fn is_bypassed(&self) -> bool {
        !self.any_active && self.updates_left == 0
    }

    /// Filters one interleaved frame in place.
    #[inline]
    pub fn process(&mut self, frame: &mut [f32]) {
        if self.updates_left > 0 {
            if self.countdown == 0 {
                self.updates_left -= 1;
                if self.updates_left == 0 {
                    self.cur = self.target;
                } else {
                    for b in 0..EQ_BANDS {
                        self.cur[b] += self.step[b];
                    }
                }
                self.update_coefs();
                self.countdown = EQ_UPDATE_FRAMES;
            }
            self.countdown -= 1;
        }
        if !self.any_active {
            return;
        }
        let ch = self.ch;
        for b in 0..EQ_BANDS {
            if !self.active[b] {
                continue;
            }
            let c = self.coefs[b];
            let st = &mut self.state[b * ch..(b + 1) * ch];
            for (s, z) in frame.iter_mut().zip(st.iter_mut()) {
                let x = *s as f64;
                let y = c.b0 * x + z[0];
                z[0] = c.b1 * x - c.a1 * y + z[1];
                z[1] = c.b2 * x - c.a2 * y;
                *s = y as f32;
            }
        }
    }
}

// -------------------------------------------------------------- limiter

/// A lookahead peak limiter. The gain for each frame is the smallest gain
/// any frame in the next `la` frames needs, smoothed with a moving average
/// over the same window, so it is already low enough when a peak arrives
/// and never steps. Output is delayed by `la` frames.
pub struct Limiter {
    ch: usize,
    la: usize,
    delay: Vec<f32>,
    dpos: usize,
    /// Sliding minimum of the wanted gains (a monotonic queue in a ring).
    mq_idx: Vec<u64>,
    mq_val: Vec<f32>,
    mq_head: usize,
    mq_len: usize,
    n: u64,
    ring: Vec<f32>,
    rpos: usize,
    sum: f64,
    rel: f32,
    rel_coef: f32,
}

impl Limiter {
    pub fn new(rate: u32, ch: usize) -> Self {
        let mut l = Self {
            ch: 1,
            la: 1,
            delay: Vec::new(),
            dpos: 0,
            mq_idx: Vec::new(),
            mq_val: Vec::new(),
            mq_head: 0,
            mq_len: 0,
            n: 0,
            ring: Vec::new(),
            rpos: 0,
            sum: 0.0,
            rel: 1.0,
            rel_coef: 0.0,
        };
        l.configure(rate, ch);
        l
    }

    /// Sizes the buffers for a format (allocates) and clears them.
    pub fn configure(&mut self, rate: u32, ch: usize) {
        let rate = rate.max(8_000) as f64;
        self.ch = ch.max(1);
        self.la = ((rate * LIMIT_LOOKAHEAD_MS / 1000.0).round() as usize).max(1);
        self.delay = vec![0.0; self.la * self.ch];
        self.mq_idx = vec![0; self.la + 2];
        self.mq_val = vec![1.0; self.la + 2];
        self.ring = vec![1.0; self.la];
        self.rel_coef = (1.0 - (-1.0 / (rate * LIMIT_RELEASE_MS / 1000.0)).exp()) as f32;
        self.reset();
    }

    /// Empties the delay and forgets the gain history.
    pub fn reset(&mut self) {
        self.delay.iter_mut().for_each(|s| *s = 0.0);
        self.dpos = 0;
        self.mq_head = 0;
        self.mq_len = 0;
        self.n = 0;
        self.ring.iter_mut().for_each(|g| *g = 1.0);
        self.rpos = 0;
        self.sum = self.la as f64;
        self.rel = 1.0;
    }

    /// Lookahead (and delay) in frames.
    pub fn latency(&self) -> usize {
        self.la
    }

    /// No gain reduction going on or pending.
    fn settled(&self) -> bool {
        self.rel >= 1.0 && self.sum >= self.la as f64 - 1e-6
    }

    /// Takes one frame in, gives the frame from `la` frames ago out (gain
    /// applied). `quiet`: the caller knows this frame can't go over the
    /// ceiling, so the gain work is skipped once nothing is pending.
    #[inline]
    pub fn process(&mut self, frame: &mut [f32], quiet: bool) {
        let ch = self.ch;
        let mut peak = 0.0f32;
        for s in frame.iter() {
            peak = peak.max(s.abs());
        }
        let slot = &mut self.delay[self.dpos * ch..(self.dpos + 1) * ch];
        for (s, d) in frame.iter_mut().zip(slot.iter_mut()) {
            std::mem::swap(s, d);
        }
        self.dpos += 1;
        if self.dpos == self.la {
            self.dpos = 0;
        }
        let n = self.n;
        self.n += 1;
        if quiet && peak <= LIMIT_CEILING && self.settled() {
            return;
        }
        let want = if peak > LIMIT_CEILING { LIMIT_CEILING / peak } else { 1.0 };
        // Sliding minimum over frames n-la..=n.
        let cap = self.mq_idx.len();
        while self.mq_len > 0 {
            let back = (self.mq_head + self.mq_len - 1) % cap;
            if self.mq_val[back] >= want {
                self.mq_len -= 1;
            } else {
                break;
            }
        }
        let at = (self.mq_head + self.mq_len) % cap;
        self.mq_idx[at] = n;
        self.mq_val[at] = want;
        self.mq_len += 1;
        while self.mq_idx[self.mq_head] + (self.la as u64) < n {
            self.mq_head = (self.mq_head + 1) % cap;
            self.mq_len -= 1;
        }
        let hold = self.mq_val[self.mq_head];
        // Release back up smoothly, never above what's needed.
        let up = self.rel + (1.0 - self.rel) * self.rel_coef;
        self.rel = if hold < up { hold } else if 1.0 - up < 1e-5 { 1.0 } else { up };
        self.sum += self.rel as f64 - self.ring[self.rpos] as f64;
        self.ring[self.rpos] = self.rel;
        self.rpos += 1;
        if self.rpos == self.la {
            self.rpos = 0;
        }
        let g = ((self.sum / self.la as f64) as f32).min(1.0);
        if g < 1.0 {
            for s in frame.iter_mut() {
                *s *= g;
            }
        }
    }
}

/// Headphone correction, EQ, then limiter, with a short dip when the
/// limiter is switched (its delay comes or goes) or the correction changes.
pub struct Chain {
    corr: CorrectionStage,
    /// The correction to swap in at the bottom of the next dip.
    corr_want: Option<Correction>,
    corr_pending: bool,
    pub eq: Eq,
    lim: Limiter,
    lim_on: bool,
    lim_want: bool,
    /// 0: none, 1: dipping down, 2: coming back up.
    dip_phase: u8,
    dip: f32,
    dip_step: f32,
    /// Silent frames left before coming back up.
    hold: usize,
    /// Frames since the last real (non-flush) frame went in.
    since_real: usize,
    ramp_frames: usize,
}

impl Chain {
    pub fn new(rate: u32, ch: usize) -> Self {
        let mut c = Self {
            corr: CorrectionStage::new(rate, ch),
            corr_want: None,
            corr_pending: false,
            eq: Eq::new(rate, ch),
            lim: Limiter::new(rate, ch),
            lim_on: false,
            lim_want: false,
            dip_phase: 0,
            dip: 1.0,
            dip_step: 0.0,
            hold: 0,
            since_real: usize::MAX,
            ramp_frames: 0,
        };
        c.configure(rate, ch);
        c
    }

    /// New output format (allocates; not for the callback).
    pub fn configure(&mut self, rate: u32, ch: usize) {
        self.corr.configure(rate, ch);
        self.corr.set(self.corr_want);
        self.corr_pending = false;
        self.eq.configure(rate, ch);
        self.lim.configure(rate, ch);
        self.lim_on = self.lim_want;
        self.dip_phase = 0;
        self.dip = 1.0;
        self.hold = 0;
        self.dip_step =1.0 / ((rate as f64 * SWITCH_MS / 1000.0).max(1.0) as f32);
        self.since_real = usize::MAX;
        self.ramp_frames = (rate as u64 * SETTING_RAMP_MS / 1000) as usize;
    }

    /// Frames a setting change ramps over.
    pub fn ramp_frames(&self) -> usize {
        self.ramp_frames
    }

    pub fn limiter_on(&self) -> bool {
        self.lim_want
    }

    /// Turns the limiter on or off. `live`: audio is flowing, so dip around
    /// the switch; otherwise switch at once.
    pub fn set_limiter(&mut self, on: bool, live: bool) {
        self.lim_want = on;
        if self.lim_on == on {
            if self.dip_phase == 1 && !self.corr_pending {
                self.dip_phase = 2;
            }
            return;
        }
        if live {
            if self.dip_phase != 1 {
                self.dip_phase = 1;
            }
        } else {
            self.lim_on = on;
            self.lim.reset();
            self.dip_phase = 0;
            self.dip = 1.0;
        }
    }

    /// Sets the headphone correction. `live`: audio is flowing, so dip
    /// around the swap; otherwise swap at once.
    pub fn set_correction(&mut self, c: Option<Correction>, live: bool) {
        self.corr_want = c;
        if !live {
            self.corr.set(c);
            self.corr_pending = false;
            return;
        }
        if self.corr.current() == c {
            self.corr_pending = false;
            if self.dip_phase == 1 && self.lim_on == self.lim_want {
                self.dip_phase = 2;
            }
            return;
        }
        self.corr_pending = true;
        self.dip_phase = 1;
    }

    /// The correction in use (or about to be, after a dip).
    pub fn correction(&self) -> Option<Correction> {
        self.corr_want
    }

    /// Audio still sits in the limiter's delay.
    pub fn pending(&self) -> bool {
        self.lim_on && self.since_real < self.lim.latency()
    }

    /// Delay the chain adds, in frames.
    pub fn latency(&self) -> usize {
        if self.lim_on { self.lim.latency() } else { 0 }
    }

    /// Runs one frame through. `quiet`: it can't clip (limiter work skips).
    #[inline]
    pub fn process(&mut self, frame: &mut [f32], quiet: bool) {
        self.since_real = 0;
        self.run(frame, quiet);
    }

    /// Pushes silence in and takes the delayed audio out (end of playback).
    #[inline]
    pub fn flush(&mut self, frame: &mut [f32]) {
        frame.iter_mut().for_each(|s| *s = 0.0);
        self.since_real = self.since_real.saturating_add(1);
        self.run(frame, true);
    }

    #[inline]
    fn run(&mut self, frame: &mut [f32], quiet: bool) {
        self.corr.process(frame);
        self.eq.process(frame);
        if self.lim_on {
            self.lim.process(frame, quiet);
        }
        match self.dip_phase {
            0 => {}
            1 => {
                self.dip -= self.dip_step;
                if self.dip <= 0.0 {
                    self.dip = 0.0;
                    if self.corr_pending {
                        self.corr.set(self.corr_want);
                        self.corr_pending = false;
                    }
                    self.hold = 0;
                    if self.lim_on != self.lim_want {
                        self.lim_on = self.lim_want;
                        self.lim.reset();
                        // Coming on, its delay starts empty: stay silent
                        // until audio comes out of it.
                        self.hold = if self.lim_on { self.lim.latency() } else { 0 };
                    }
                    self.dip_phase = 2;
                }
                frame.iter_mut().for_each(|s| *s *= self.dip);
            }
            _ if self.hold > 0 => {
                self.hold -= 1;
                frame.iter_mut().for_each(|s| *s = 0.0);
            }
            _ => {
                self.dip += self.dip_step;
                if self.dip >= 1.0 {
                    self.dip = 1.0;
                    self.dip_phase = 0;
                } else {
                    frame.iter_mut().for_each(|s| *s *= self.dip);
                }
            }
        }
    }
}

// --------------------------------------------------------- silence meter

/// Measures a track's quiet lead-in and run-out as it is decoded, in
/// 50 ms windows: quiet is an RMS under -40 dBFS.
pub struct SilenceMeter {
    ch: usize,
    win: usize,
    acc: f64,
    count: usize,
    /// Frames seen (from `base`).
    frames: u64,
    base: u64,
    first_loud: Option<u64>,
    last_loud_end: Option<u64>,
}

impl SilenceMeter {
    /// `base`: the track frame decoding starts at (0 unless it seeked).
    pub fn new(rate: u32, ch: usize, base: u64) -> Self {
        Self {
            ch: ch.max(1),
            win: ((rate as u64 * METER_WINDOW_MS / 1000) as usize).max(1),
            acc: 0.0,
            count: 0,
            frames: 0,
            base,
            first_loud: None,
            last_loud_end: None,
        }
    }

    pub fn push(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.ch) {
            for s in frame {
                self.acc += (*s as f64) * (*s as f64);
            }
            self.count += 1;
            self.frames += 1;
            if self.count == self.win {
                self.close_window();
            }
        }
    }

    fn close_window(&mut self) {
        if self.count == 0 {
            return;
        }
        let power = self.acc / (self.count * self.ch) as f64;
        if power > QUIET_POWER {
            let start = self.base + self.frames - self.count as u64;
            self.first_loud.get_or_insert(start);
            self.last_loud_end = Some(self.base + self.frames);
        }
        self.acc = 0.0;
        self.count = 0;
    }

    /// Ends the measurement (the track decoded to its end).
    pub fn finish(&mut self) {
        self.close_window();
    }

    /// Frames of quiet lead-in, once known: when the first loud window
    /// came, or `cap` frames passed without one. None after a seek.
    pub fn head(&self, cap: u64) -> Option<u64> {
        if self.base != 0 {
            return None;
        }
        match self.first_loud {
            Some(f) => Some(f),
            None if self.frames >= cap => Some(self.frames),
            None => None,
        }
    }

    /// Frames of quiet run-out after [`finish`](Self::finish). None if
    /// nothing loud was decoded.
    pub fn tail(&self) -> Option<u64> {
        self.last_loud_end.map(|e| self.base + self.frames - e)
    }

    /// Whether the whole track was measured from its start.
    pub fn whole(&self) -> bool {
        self.base == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f64, frames: usize, amp: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = ((2.0 * PI * freq * i as f64 / rate as f64).sin() as f32) * amp;
                [v, v]
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    fn run_eq(eq: &mut Eq, input: &[f32]) -> Vec<f32> {
        let mut out = input.to_vec();
        for f in out.as_chunks_mut::<2>().0 {
            eq.process(f);
        }
        out
    }

    #[test]
    fn eq_band_gain_at_centre_is_within_half_a_db() {
        for rate in [44_100u32, 48_000, 96_000] {
            for b in 0..EQ_BANDS {
                for db in [-12.0f32, -6.0, 6.0, 12.0] {
                    let mut bands = [0.0; EQ_BANDS];
                    bands[b] = db;
                    let mut eq = Eq::new(rate, 2);
                    eq.set_target(bands, 0);
                    let f = EQ_FREQS[b];
                    // Long enough for the 31 Hz band to settle.
                    let n = (rate as f64 * 1.5) as usize;
                    let x = sine(rate, f, n, 0.1);
                    let y = run_eq(&mut eq, &x);
                    let skip = x.len() / 2;
                    let got = 20.0 * (rms(&y[skip..]) / rms(&x[skip..])).log10();
                    assert!(
                        (got - db as f64).abs() < 0.5,
                        "{rate} Hz, band {f} Hz at {db} dB measured {got:.2} dB"
                    );
                }
            }
        }
    }

    #[test]
    fn flat_eq_is_bypassed_and_exact() {
        let mut eq = Eq::new(48_000, 2);
        assert!(eq.is_bypassed());
        let x = sine(48_000, 440.0, 1000, 0.5);
        assert_eq!(run_eq(&mut eq, &x), x);
        // Ramp up and back down to flat: bypassed again afterwards.
        eq.set_target([3.0; EQ_BANDS], 9600);
        run_eq(&mut eq, &x);
        eq.set_target([0.0; EQ_BANDS], 9600);
        for _ in 0..20 {
            run_eq(&mut eq, &x);
        }
        assert!(eq.is_bypassed());
    }

    #[test]
    fn eq_changes_ramp_without_steps() {
        // A full +12 dB jump on a low band, ramped: no sample-to-sample jump
        // bigger than the signal's own slope allows.
        let rate = 48_000;
        let mut eq = Eq::new(rate, 2);
        let x = sine(rate, 125.0, rate as usize, 0.25);
        let mut y = run_eq(&mut eq, &x[..x.len() / 2]);
        eq.set_target([0.0, 0.0, 12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 9600);
        y.extend(run_eq(&mut eq, &x[x.len() / 2..]));
        let max_slope = 2.0 * PI * 125.0 / rate as f64 * 0.25 * 4.0; // +12 dB is 4x
        for w in y.as_chunks::<2>().0.windows(2) {
            let d = (w[1][0] - w[0][0]).abs() as f64;
            assert!(d < max_slope * 1.5, "step of {d}");
        }
    }

    #[test]
    fn eq_max_gain_tracks_the_curve() {
        assert_eq!(eq_max_gain(&[0.0; EQ_BANDS], 48_000), 1.0);
        let mut b = [0.0; EQ_BANDS];
        b[0] = 12.0;
        let m = eq_max_gain(&b, 48_000);
        assert!(m >= db_to_lin(12.0) && m < db_to_lin(13.0), "{m}");
        assert!(eq_max_gain(&[-6.0; EQ_BANDS], 48_000) <= 1.06);
    }

    #[test]
    fn limiter_keeps_peaks_under_the_ceiling() {
        let rate = 48_000;
        let mut lim = Limiter::new(rate, 2);
        // A +12 dB sine with sudden bursts.
        let mut x = sine(rate, 997.0, rate as usize, 4.0);
        for i in (0..x.len()).step_by(3001) {
            x[i] = 9.0;
        }
        let mut peak = 0.0f32;
        for f in x.as_chunks_mut::<2>().0 {
            lim.process(f, false);
            peak = peak.max(f[0].abs()).max(f[1].abs());
        }
        assert!(peak <= LIMIT_CEILING + 1e-5, "peak {peak}");
    }

    #[test]
    fn limiter_passes_quiet_audio_delayed_and_untouched() {
        let mut lim = Limiter::new(48_000, 2);
        let la = lim.latency();
        assert_eq!(la, 72);
        let x = sine(48_000, 440.0, 2000, 0.5);
        let mut y = x.clone();
        for f in y.as_chunks_mut::<2>().0 {
            lim.process(f, false);
        }
        assert!(y[..la * 2].iter().all(|v| *v == 0.0));
        assert_eq!(&y[la * 2..], &x[..x.len() - la * 2]);
    }

    #[test]
    fn limiter_recovers_after_a_peak() {
        let rate = 48_000;
        let mut lim = Limiter::new(rate, 2);
        let mut x = sine(rate, 440.0, rate as usize, 0.5);
        x[200] = 3.0;
        let mut y = x.clone();
        for f in y.as_chunks_mut::<2>().0 {
            lim.process(f, true);
        }
        // A second later the gain is back to 1.
        let la = lim.latency() * 2;
        let end = y.len() - 2000;
        assert!((y[end] - x[end - la]).abs() < 1e-4);
    }

    #[test]
    fn chain_limiter_switch_dips_and_keeps_going() {
        let mut c = Chain::new(48_000, 2);
        let x = sine(48_000, 440.0, 4800, 0.5);
        let mut y = x.clone();
        c.set_limiter(true, true);
        let mut min_mid = f32::MAX;
        for (i, f) in y.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            c.process(f, false);
            if (240..300).contains(&i) {
                min_mid = min_mid.min(f[0].abs());
            }
        }
        assert!(c.limiter_on() && c.latency() == 72);
        // The dip reaches silence around the switch, then comes back.
        let end = y.len() - 2;
        assert!((y[end] - x[end - 144]).abs() < 1e-6);
        assert!(min_mid < 0.1);
    }

    #[test]
    fn leveling_picks_track_or_album_gain() {
        let info = TrackAudio {
            gain_db: Some(-6.0),
            album_gain_db: Some(-3.0),
            peak: Some(0.9),
            album_peak: Some(1.0),
            ..Default::default()
        };
        let mut s = DspSettings { leveling: Leveling::Smart, ..Default::default() };
        let (g, _) = level_gain(Some(&info), &s, true);
        assert!((g - db_to_lin(-3.0)).abs() < 1e-6);
        let (g, p) = level_gain(Some(&info), &s, false);
        assert!((g - db_to_lin(-6.0)).abs() < 1e-6);
        assert!((p - 0.9 * db_to_lin(-6.0)).abs() < 1e-6);
        s.leveling = Leveling::Album;
        assert!((level_gain(Some(&info), &s, false).0 - db_to_lin(-3.0)).abs() < 1e-6);
        s.leveling = Leveling::Track;
        assert!((level_gain(Some(&info), &s, true).0 - db_to_lin(-6.0)).abs() < 1e-6);
        s.leveling = Leveling::Off;
        assert_eq!(level_gain(Some(&info), &s, true).0, 1.0);
        // Missing data: only the preamp.
        s.leveling = Leveling::Smart;
        s.preamp_db = 3.0;
        let (g, p) = level_gain(None, &s, true);
        assert!((g - db_to_lin(3.0)).abs() < 1e-6);
        assert!((p - UNKNOWN_PEAK * g).abs() < 1e-6);
        let (g, _) = level_gain(Some(&info), &s, false);
        assert!((g - db_to_lin(-3.0)).abs() < 1e-6);
    }

    #[test]
    fn equal_power_curves_sum_to_one() {
        for len in [1usize, 7, 4410] {
            for i in 0..len {
                let p = fade_out(i, len).powi(2) + fade_in(i, len).powi(2);
                assert!((p - 1.0).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn silence_meter_finds_head_and_tail() {
        let rate = 48_000;
        let quiet = vec![0.0f32; 24_000 * 2]; // 0.5 s
        let loud = sine(rate, 440.0, 48_000, 0.5);
        let faint = sine(rate, 440.0, 4800, 0.005); // -46 dBFS RMS: quiet
        let mut m = SilenceMeter::new(rate, 2, 0);
        m.push(&quiet);
        assert_eq!(m.head(rate as u64 * 8), None);
        m.push(&loud);
        assert_eq!(m.head(rate as u64 * 8), Some(24_000));
        m.push(&faint);
        m.push(&quiet);
        m.finish();
        assert_eq!(m.tail(), Some(4800 + 24_000));
        // Seeked: no head, tail still known.
        let mut m = SilenceMeter::new(rate, 2, 1000);
        m.push(&loud);
        m.finish();
        assert_eq!((m.head(1), m.tail()), (None, Some(0)));
        // A long quiet start reports the cap once passed.
        let mut m = SilenceMeter::new(rate, 2, 0);
        m.push(&quiet);
        assert_eq!(m.head(20_000), Some(24_000));
    }

    /// The whole chain (leveling gain, all EQ bands, limiter working) at
    /// 96 kHz stereo must stay under 3 % of one core. Run with
    /// `cargo test --release dsp_benchmark -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn dsp_benchmark() {
        let rate = 96_000u32;
        let secs = 20usize;
        let mut chain = Chain::new(rate, 2);
        chain.set_limiter(true, false);
        chain.eq.set_target([6.0, 4.0, -3.0, 2.0, -2.0, 3.0, -4.0, 5.0, 2.0, 6.0], 0);
        let block = sine(rate, 1000.0, 1024, 0.9);
        let mut buf = block.clone();
        let gain = db_to_lin(4.0);
        let t = std::time::Instant::now();
        let blocks = secs * rate as usize / 1024;
        let mut sink = 0.0f32;
        for _ in 0..blocks {
            buf.copy_from_slice(&block);
            for f in buf.as_chunks_mut::<2>().0 {
                f[0] *= gain;
                f[1] *= gain;
                chain.process(f, false);
            }
            sink += buf[7];
        }
        let el = t.elapsed().as_secs_f64();
        let pct = el / secs as f64 * 100.0;
        println!("DSP chain: {secs} s of 96 kHz stereo in {el:.3} s = {pct:.2} % of one core ({sink})");
        assert!(pct < 3.0, "{pct:.2} %");
    }
}
