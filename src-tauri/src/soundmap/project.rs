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

/// A vector the map can use.
pub fn usable(v: &[f32]) -> bool {
    v.len() == DIMS && v.iter().all(|x| x.is_finite())
}

fn standard(v: &[f32], mean: &[f64], scale: &[f64]) -> [f64; DIMS] {
    let mut out = [0.0; DIMS];
    for i in 0..DIMS {
        out[i] = (v[i] as f64 - mean[i]) / scale[i] * WEIGHTS[i];
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
    // The same way round every time: its largest part positive.
    let big = v.iter().copied().fold(0.0f64, |m, x| if x.abs() > m.abs() { x } else { m });
    if big < 0.0 {
        v.iter_mut().for_each(|x| *x = -*x);
    }
    (v, lambda.max(0.0))
}

/// Works out the projection for these vectors (all [`usable`]).
pub fn fit(vectors: &[&[f32]]) -> Option<Basis> {
    let n = vectors.len();
    if n == 0 {
        return None;
    }
    let mut mean = vec![0.0; DIMS];
    for v in vectors {
        for i in 0..DIMS {
            mean[i] += v[i] as f64;
        }
    }
    mean.iter_mut().for_each(|m| *m /= n as f64);
    let mut scale = vec![0.0; DIMS];
    for v in vectors {
        for i in 0..DIMS {
            let d = v[i] as f64 - mean[i];
            scale[i] += d * d;
        }
    }
    // A feature that doesn't vary isn't stretched out of nothing.
    scale.iter_mut().for_each(|s| *s = (*s / n as f64).sqrt().max(0.02));
    let mut cov = [[0.0; DIMS]; DIMS];
    for v in vectors {
        let s = standard(v, &mean, &scale);
        for (row, si) in cov.iter_mut().zip(&s) {
            for (c, sj) in row.iter_mut().zip(&s) {
                *c += si * sj;
            }
        }
    }
    cov.iter_mut().flatten().for_each(|x| *x /= n as f64);
    let (a, l1) = top_axis(&cov, None, 0);
    let (b, _) = top_axis(&cov, Some(&a), 7);
    let spread = l1.sqrt();
    Some(Basis { mean, scale, axes: [a, b], spread: if spread > 1e-6 { spread } else { 1.0 }, count: n })
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
