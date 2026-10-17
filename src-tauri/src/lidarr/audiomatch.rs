//! Whether two files hold the same audio: their decoded waveforms,
//! correlated. One key among several for a swap without ISRCs (a censored
//! edit can still score above 0.99, so it never decides alone).
//!
//! Both files are decoded with the player's decoder, mixed to mono and
//! brought to [`RATE`] (a box filter, then every n-th sample: enough for
//! comparing waveforms, not for listening). The best lag is found on a
//! coarser copy first, then refined, so an encoder's few milliseconds of
//! delay don't count against a match.

use std::path::Path;

use crate::audio::decode::TrackDecoder;
use crate::error::{AppError, AppResult};

/// Samples per second compared.
pub const RATE: u32 = 8_000;
/// Seconds of each file compared, from the start.
pub const SECONDS: u32 = 40;
/// Largest lag searched either way.
const MAX_LAG_MS: i64 = 250;
/// The coarse pass keeps every n-th sample.
const COARSE: usize = 8;

fn decode_err(p: &Path, e: impl std::fmt::Display) -> AppError {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    AppError::Invalid(format!("Platter couldn't decode {name} to compare it ({e})."))
}

/// Up to [`SECONDS`] of a file, mono, at [`RATE`].
pub fn load(path: &Path) -> AppResult<Vec<f32>> {
    crate::error::no_panic(path, || load_unguarded(path))
}

fn load_unguarded(path: &Path) -> AppResult<Vec<f32>> {
    let file = std::fs::File::open(path).map_err(|e| decode_err(path, e))?;
    let hint = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    let mut dec = TrackDecoder::open(Box::new(file), hint.as_deref()).map_err(|e| decode_err(path, e))?;
    let (rate, ch) = (dec.rate.max(1) as u64, dec.channels.max(1));
    let want_frames = rate * SECONDS as u64;
    let mut out = Vec::with_capacity((RATE * SECONDS) as usize);
    let (mut acc, mut n, mut frame) = (0f64, 0u32, 0u64);
    'blocks: while let Some(block) = dec.next_block().map_err(|e| decode_err(path, e))? {
        for f in block.chunks_exact(ch) {
            if frame >= want_frames {
                break 'blocks;
            }
            acc += f.iter().map(|x| *x as f64).sum::<f64>() / ch as f64;
            n += 1;
            frame += 1;
            // Close an output sample once its slice of input time is over.
            if frame * RATE as u64 / rate > out.len() as u64 {
                out.push((acc / n as f64) as f32);
                acc = 0.0;
                n = 0;
            }
        }
    }
    Ok(out)
}

/// Pearson correlation of `a` against `b` shifted by `lag` (b[i + lag]).
fn pearson(a: &[f32], b: &[f32], lag: i64) -> f64 {
    let (start_a, start_b) = if lag >= 0 { (0usize, lag as usize) } else { ((-lag) as usize, 0usize) };
    if start_a >= a.len() || start_b >= b.len() {
        return 0.0;
    }
    let n = (a.len() - start_a).min(b.len() - start_b);
    if n < 16 {
        return 0.0;
    }
    let (a, b) = (&a[start_a..start_a + n], &b[start_b..start_b + n]);
    let (ma, mb) = (a.iter().map(|x| *x as f64).sum::<f64>() / n as f64, b.iter().map(|x| *x as f64).sum::<f64>() / n as f64);
    let (mut sab, mut saa, mut sbb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        let (dx, dy) = (*x as f64 - ma, *y as f64 - mb);
        sab += dx * dy;
        saa += dx * dx;
        sbb += dy * dy;
    }
    if saa <= f64::EPSILON || sbb <= f64::EPSILON { 0.0 } else { sab / (saa * sbb).sqrt() }
}

fn every_nth(v: &[f32], n: usize) -> Vec<f32> {
    v.chunks(n).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect()
}

/// The best correlation of two signals at [`RATE`] over the allowed lags.
pub fn best_correlation(a: &[f32], b: &[f32]) -> f64 {
    let max_lag = MAX_LAG_MS * RATE as i64 / 1000;
    let (ca, cb) = (every_nth(a, COARSE), every_nth(b, COARSE));
    let coarse_max = max_lag / COARSE as i64;
    let best_coarse = (-coarse_max..=coarse_max).max_by(|x, y| pearson(&ca, &cb, *x).total_cmp(&pearson(&ca, &cb, *y))).unwrap_or(0);
    let centre = best_coarse * COARSE as i64;
    let span = COARSE as i64 * 2;
    ((centre - span)..=(centre + span)).map(|lag| pearson(a, b, lag)).fold(0.0, f64::max)
}

/// How alike two files' audio is, from -1 to 1.
pub fn correlation(a: &Path, b: &Path) -> AppResult<f64> {
    let (x, y) = (load(a)?, load(b)?);
    if x.len() < RATE as usize || y.len() < RATE as usize {
        return Err(AppError::Invalid("A file is too short to compare.".into()));
    }
    Ok(best_correlation(&x, &y))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A WAV of noise-like music from `seed`, `ms` long, starting `delay` samples late.
    pub fn wav(path: &Path, seed: u32, rate: u32, ms: u32, delay: usize) {
        let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        let frames = (rate as u64 * ms as u64 / 1000) as usize;
        let mut s = seed.wrapping_mul(2_654_435_761).max(1);
        let mut low = 0f32;
        for i in 0..frames + delay {
            // Low-passed noise with a slow melody on top.
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let noise = (s as f32 / u32::MAX as f32) * 2.0 - 1.0;
            low = low * 0.97 + noise * 0.03;
            let t = i.saturating_sub(delay) as f32 / rate as f32;
            let v = if i < delay { 0.0 } else { low * 4.0 + 0.2 * (t * 220.0 * std::f32::consts::TAU * (1.0 + (seed % 3) as f32)).sin() };
            let v = (v.clamp(-1.0, 1.0) * 20_000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn the_same_audio_correlates_across_rates_and_a_small_delay() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (dir.path().join("a.wav"), dir.path().join("b.wav"), dir.path().join("c.wav"));
        wav(&a, 7, 44_100, 3_000, 0);
        wav(&b, 7, 44_100, 3_000, 900); // about 20 ms late, like an encoder's delay
        wav(&c, 8, 44_100, 3_000, 0);
        assert!(correlation(&a, &b).unwrap() >= 0.99, "{}", correlation(&a, &b).unwrap());
        assert!(correlation(&a, &c).unwrap() < 0.5, "different audio: {}", correlation(&a, &c).unwrap());
        assert!(correlation(&a, &a).unwrap() > 0.999);
    }

    #[test]
    fn too_short_or_not_audio_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.wav"), dir.path().join("b.txt"));
        wav(&a, 1, 8_000, 200, 0);
        std::fs::write(&b, b"not audio").unwrap();
        assert!(correlation(&a, &a).is_err());
        assert!(correlation(&a, &b).is_err());
    }
}
