//! Flattening each song's feature vector to a point on the map: a principal
//! component projection (plain arithmetic, no libraries). The two directions
//! the library varies most in become the map's axes, so songs with similar
//! vectors land near each other.
//!
//! The features are standardised first and weighted, so the twelve pitch
//! class values (which mostly say what key a song is in) count about as
//! much as two of the others together, and energy and brightness a little
//! more. Without that the map would mostly sort songs by key.

use serde::{Deserialize, Serialize};

/// Places in a feature vector (`analysis::features::feature_names`).
pub const DIMS: usize = 20;

/// Bumped when the projection changes, so stored layouts are redone.
pub const LAYOUT_VERSION: i64 = 1;

/// tempo, energy, loudness, onsets, brightness, dynamics, major, tonal, then
/// the twelve pitch classes.
const WEIGHTS: [f64; DIMS] = [
    1.0, 1.4, 1.0, 1.0, 1.4, 1.0, 0.8, 1.0, 0.35, 0.35, 0.35, 0.35, 0.35, 0.35, 0.35, 0.35, 0.35, 0.35, 0.35, 0.35,
];

const ITERATIONS: usize = 300;

/// How far from the middle one feature can pull a point, in standard
/// deviations. A feature that hardly varied when the projection was worked
/// out has a tiny spread, so a later song that differs in it would
/// otherwise land far off the map.
pub const CLAMP: f64 = 5.0;

/// What turns a vector into a point. Kept, so songs analysed later land
/// where the same projection puts them and the map doesn't shift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Basis {
    pub mean: Vec<f64>,
    pub scale: Vec<f64>,
    pub axes: [Vec<f64>; 2],
    /// Spread along the first axis; points are divided by three of these,
    /// so most of the map sits within -1 to 1.
    pub spread: f64,
    /// Songs the basis was worked out from.
    pub count: usize,
}

impl Basis {
    /// Every part the right length, so placing with it can't go out of bounds.
    pub fn whole(&self) -> bool {
        self.mean.len() == DIMS && self.scale.len() == DIMS && self.axes.iter().all(|a| a.len() == DIMS)
    }
}

/// A vector the map can use.
pub fn usable(v: &[f32]) -> bool {
    v.len() == DIMS && v.iter().all(|x| x.is_finite())
}

fn standard(v: &[f32], mean: &[f64], scale: &[f64]) -> [f64; DIMS] {
    let mut out = [0.0; DIMS];
    for i in 0..DIMS {
        out[i] = ((v[i] as f64 - mean[i]) / scale[i]).clamp(-CLAMP, CLAMP) * WEIGHTS[i];
    }
    out
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn normalise(v: &mut [f64]) -> bool {
    let n = dot(v, v).sqrt();
    if n < 1e-12 {
        return false;
    }
    v.iter_mut().for_each(|x| *x /= n);
    true
}

/// The strongest direction of `cov` that's at right angles to `against`.
fn top_axis(cov: &[[f64; DIMS]; DIMS], against: Option<&[f64]>, seed: usize) -> (Vec<f64>, f64) {
    // A fixed start that isn't at right angles to anything likely.
    let mut v: Vec<f64> = (0..DIMS).map(|i| 1.0 / (1.0 + ((i + seed) % DIMS) as f64)).collect();
    let orth = |v: &mut Vec<f64>| {
        if let Some(a) = against {
            let d = dot(v, a);
            v.iter_mut().zip(a).for_each(|(x, y)| *x -= d * y);
        }
    };
    orth(&mut v);
    if !normalise(&mut v) {
        v = vec![0.0; DIMS];
        v[seed % DIMS] = 1.0;
    }
    let mut lambda = 0.0;
    for _ in 0..ITERATIONS {
        let mut next: Vec<f64> = (0..DIMS).map(|i| dot(&cov[i], &v)).collect();
        orth(&mut next);
        lambda = dot(&next, &v);
        if !normalise(&mut next) {
            break;
        }
        let change: f64 = next.iter().zip(&v).map(|(a, b)| (a - b).abs()).sum();
        v = next;
        if change < 1e-10 {
            break;
        }
    }
    (v, lambda.max(0.0))
}

/// The same way round every time: its largest part positive.
fn fixed_sign(v: &mut [f64]) {
    let big = v.iter().copied().fold(0.0f64, |m, x| if x.abs() > m.abs() { x } else { m });
    if big < 0.0 {
        v.iter_mut().for_each(|x| *x = -*x);
    }
}

/// Turns new axes to match the ones in use, so working the projection out
/// again doesn't mirror or turn the map: swaps them when each lines up
/// better with the other's old axis, then flips any that points the other
/// way. With nothing to match (the first time), each gets a fixed sign.
fn align(a: &mut Vec<f64>, b: &mut Vec<f64>, previous: Option<&Basis>) {
    let Some(p) = previous.filter(|p| p.axes.iter().all(|x| x.len() == DIMS)) else {
        fixed_sign(a);
        fixed_sign(b);
        return;
    };
    let straight = dot(a, &p.axes[0]).abs() + dot(b, &p.axes[1]).abs();
    let crossed = dot(a, &p.axes[1]).abs() + dot(b, &p.axes[0]).abs();
    if crossed > straight {
        std::mem::swap(a, b);
    }
    for (v, old) in [(a, &p.axes[0]), (b, &p.axes[1])] {
        if dot(v, old) < 0.0 {
            v.iter_mut().for_each(|x| *x = -*x);
        }
    }
}

/// Works the projection out one vector at a time, so the whole library
/// needn't be held at once. Keeps sums and products of the features
/// (measured from the first vector, which keeps the sums small), from
/// which each feature's spread and how the features vary together follow.
#[derive(Debug, Clone)]
pub struct Fit {
    n: usize,
    origin: [f64; DIMS],
    sum: [f64; DIMS],
    products: [[f64; DIMS]; DIMS],
}

impl Default for Fit {
    fn default() -> Self {
        Self { n: 0, origin: [0.0; DIMS], sum: [0.0; DIMS], products: [[0.0; DIMS]; DIMS] }
    }
}

impl Fit {
    /// Counts one vector (which must be [`usable`]).
    pub fn add(&mut self, v: &[f32]) {
        if self.n == 0 {
            self.origin.iter_mut().zip(v).for_each(|(o, x)| *o = *x as f64);
        }
        self.n += 1;
        let mut d = [0.0; DIMS];
        for i in 0..DIMS {
            d[i] = v[i] as f64 - self.origin[i];
            self.sum[i] += d[i];
        }
        for (row, di) in self.products.iter_mut().zip(&d) {
            for (p, dj) in row.iter_mut().zip(&d) {
                *p += di * dj;
            }
        }
    }

    /// The projection, turned to match `previous` when there is one.
    pub fn finish(&self, previous: Option<&Basis>) -> Option<Basis> {
        let n = self.n;
        if n == 0 {
            return None;
        }
        let nf = n as f64;
        let m: Vec<f64> = self.sum.iter().map(|s| s / nf).collect();
        let raw = |i: usize, j: usize| self.products[i][j] / nf - m[i] * m[j];
        // A feature that doesn't vary isn't stretched out of nothing.
        let scale: Vec<f64> = (0..DIMS).map(|i| raw(i, i).max(0.0).sqrt().max(0.02)).collect();
        let mut cov = [[0.0; DIMS]; DIMS];
        for (i, row) in cov.iter_mut().enumerate() {
            for (j, c) in row.iter_mut().enumerate() {
                *c = raw(i, j) / (scale[i] * scale[j]) * WEIGHTS[i] * WEIGHTS[j];
            }
        }
        let mean = (0..DIMS).map(|i| self.origin[i] + m[i]).collect();
        let (mut a, l1) = top_axis(&cov, None, 0);
        let (mut b, _) = top_axis(&cov, Some(&a), 7);
        align(&mut a, &mut b, previous);
        let spread = l1.sqrt();
        Some(Basis { mean, scale, axes: [a, b], spread: if spread > 1e-6 { spread } else { 1.0 }, count: n })
    }
}

/// Works out the projection for these vectors (all [`usable`]), turned to
/// match `previous` when there is one.
pub fn fit(vectors: &[&[f32]], previous: Option<&Basis>) -> Option<Basis> {
    let mut f = Fit::default();
    vectors.iter().for_each(|v| f.add(v));
    f.finish(previous)
}

/// Where a vector goes on the map.
pub fn place(b: &Basis, v: &[f32]) -> (f32, f32) {
    let s = standard(v, &b.mean, &b.scale);
    let k = 3.0 * b.spread;
    ((dot(&s, &b.axes[0]) / k) as f32, (dot(&s, &b.axes[1]) / k) as f32)
}

/// Worth working the projection out again: there's none yet, it's from an
/// older version, or the library has grown by half since.
pub fn needs_refit(basis: Option<&Basis>, version: i64, analysed: usize) -> bool {
    match basis {
        None => analysed > 0,
        Some(_) if version != LAYOUT_VERSION => true,
        Some(b) => analysed >= b.count + b.count.div_ceil(2).max(1),
    }
}
