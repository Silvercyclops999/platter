//! Converts decoded audio to the output device's layout: channel up/down-mix,
//! then sample-rate conversion (rubato FFT resampler) when rates differ.
//!
//! The converter is exact about length: the resampler's startup delay is
//! trimmed and the tail is flushed, so `n` input frames always become
//! `round(n * out_rate / in_rate)` output frames. That keeps the position
//! clock honest and makes track joins seamless.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};

/// Mixes interleaved `input` with `cin` channels into `cout` channels,
/// appending to `out`.
pub fn mix_channels(input: &[f32], cin: usize, cout: usize, out: &mut Vec<f32>) {
    if cin == 0 || cout == 0 {
        return;
    }
    if cin == cout {
        out.extend_from_slice(input);
        return;
    }
    const M3DB: f32 = std::f32::consts::FRAC_1_SQRT_2;
    out.reserve(input.len() / cin * cout);
    for frame in input.chunks_exact(cin) {
        if cin == 1 {
            out.extend(std::iter::repeat_n(frame[0], cout));
        } else if cout == 1 {
            out.push(frame.iter().sum::<f32>() / cin as f32);
        } else if cout == 2 {
            // Standard order: FL FR FC LFE BL BR SL SR. LFE is dropped.
            let c = frame.get(2).copied().unwrap_or(0.0);
            let (bl, br) = (
                frame.get(4).copied().unwrap_or(0.0),
                frame.get(5).copied().unwrap_or(0.0),
            );
            let (sl, sr) = (
                frame.get(6).copied().unwrap_or(0.0),
                frame.get(7).copied().unwrap_or(0.0),
            );
            let mut l = frame[0];
            let mut r = frame[1];
            if cin > 2 {
                l += M3DB * (c + bl + sl);
                r += M3DB * (c + br + sr);
                let norm = 1.0 / (1.0 + M3DB * 2.0);
                l *= norm;
                r *= norm;
            }
            out.push(l);
            out.push(r);
        } else {
            // Stereo (or more) into a wider layout: copy what matches, silence the rest.
            out.extend(frame.iter().copied().chain(std::iter::repeat(0.0)).take(cout));
        }
    }
}

const CHUNK: usize = 1024;

struct Rs {
    rs: Fft<f32>,
    /// Pending input frames (interleaved, `channels` wide).
    pending: Vec<f32>,
    out_buf: Vec<f32>,
    /// Output frames still to drop (resampler delay).
    to_trim: usize,
    /// Output frames owed to the previous track (continued resampling).
    carry: u64,
    in_frames: u64,
    out_frames: u64,
    in_rate: u32,
    out_rate: u32,
}

/// Streaming converter from one source format to the device format.
pub struct Converter {
    cin: usize,
    cout: usize,
    rs: Option<Rs>,
    mixed: Vec<f32>,
}

impl Converter {
    pub fn new(in_rate: u32, in_channels: usize, out_rate: u32, out_channels: usize) -> Self {
        let rs = (in_rate != out_rate && in_rate > 0).then(|| {
            let rs = Fft::<f32>::new(
                in_rate as usize,
                out_rate as usize,
                CHUNK,
                out_channels,
                FixedSync::Input,
            )
            .expect("valid resampler parameters");
            let to_trim = rs.output_delay();
            let out_cap = rs.output_frames_max();
            Rs {
                rs,
                pending: Vec::with_capacity(CHUNK * out_channels),
                out_buf: vec![0.0; out_cap * out_channels],
                to_trim,
                carry: 0,
                in_frames: 0,
                out_frames: 0,
                in_rate,
                out_rate,
            }
        });
        Self {
            cin: in_channels,
            cout: out_channels,
            rs,
            mixed: Vec::new(),
        }
    }

    pub fn is_passthrough(&self) -> bool {
        self.rs.is_none() && self.cin == self.cout
    }

    /// Converts interleaved input, appending device-format samples to `out`.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let Some(rs) = self.rs.as_mut() else {
            mix_channels(input, self.cin, self.cout, out);
            return;
        };
        self.mixed.clear();
        mix_channels(input, self.cin, self.cout, &mut self.mixed);
        rs.in_frames += (self.mixed.len() / self.cout) as u64;
        let ch = self.cout;
        let mut off = 0;
        while off < self.mixed.len() {
            let need = (rs.rs.input_frames_next() * ch).saturating_sub(rs.pending.len());
            let take = need.min(self.mixed.len() - off);
            rs.pending.extend_from_slice(&self.mixed[off..off + take]);
            off += take;
            if rs.pending.len() == rs.rs.input_frames_next() * ch {
                Self::run(rs, ch, None, out);
                rs.pending.clear();
            }
        }
    }

    fn run(rs: &mut Rs, ch: usize, partial: Option<usize>, out: &mut Vec<f32>) {
        let frames_in = rs.rs.input_frames_next();
        if rs.pending.len() < frames_in * ch {
            rs.pending.resize(frames_in * ch, 0.0);
        }
        let out_frames_cap = rs.out_buf.len() / ch;
        let input = InterleavedSlice::new(&rs.pending[..], ch, frames_in).expect("input layout");
        let mut output =
            InterleavedSlice::new_mut(&mut rs.out_buf[..], ch, out_frames_cap).expect("output layout");
        let mut idx = Indexing::new();
        idx.partial_len = partial;
        let (_, written) = rs
            .rs
            .process_into_buffer(&input, &mut output, Some(&idx))
            .expect("resample");
        let mut start = 0;
        if rs.to_trim > 0 {
            start = rs.to_trim.min(written);
            rs.to_trim -= start;
        }
        let expected_total = Self::expected(rs);
        let room = expected_total.saturating_sub(rs.out_frames) as usize;
        let n = (written - start).min(room);
        out.extend_from_slice(&rs.out_buf[start * ch..(start + n) * ch]);
        rs.out_frames += n as u64;
    }

    fn expected(rs: &Rs) -> u64 {
        // carry + round(in * out_rate / in_rate)
        rs.carry + (rs.in_frames * rs.out_rate as u64 + rs.in_rate as u64 / 2) / rs.in_rate as u64
    }

    /// Source format this converter accepts.
    pub fn input_format(&self) -> (u32, usize) {
        (self.rs.as_ref().map_or(0, |r| r.in_rate), self.cin)
    }

    pub fn output_rate(&self) -> u32 {
        self.rs.as_ref().map_or(0, |r| r.out_rate)
    }

    /// Gapless resampling across tracks: instead of flushing at the end of
    /// a track (which would add filter edge transients at the join), keep
    /// the resampler running into the next track. Returns how many of the
    /// next output frames still belong to the previous track.
    pub fn continue_track(&mut self) -> u64 {
        let Some(rs) = self.rs.as_mut() else { return 0 };
        let owed = Self::expected(rs).saturating_sub(rs.out_frames);
        rs.in_frames = 0;
        rs.out_frames = 0;
        rs.carry = owed;
        owed
    }

    /// End of stream: flushes the resampler so the output length is exact.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        let ch = self.cout;
        let Some(rs) = self.rs.as_mut() else { return };
        let partial = rs.pending.len() / ch;
        if partial > 0 {
            Self::run(rs, ch, Some(partial), out);
        }
        rs.pending.clear();
        // Feed silence until all real samples have come out.
        let mut guard = 0;
        while rs.out_frames < Self::expected(rs) && guard < 64 {
            Self::run(rs, ch, Some(0), out);
            guard += 1;
        }
    }

    /// Forget all state (after a seek).
    pub fn reset(&mut self) {
        if let Some(rs) = self.rs.as_mut() {
            rs.rs.reset();
            rs.pending.clear();
            rs.to_trim = rs.rs.output_delay();
            rs.carry = 0;
            rs.in_frames = 0;
            rs.out_frames = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_to_stereo_duplicates() {
        let mut out = Vec::new();
        mix_channels(&[0.5, -0.25], 1, 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.25, -0.25]);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let mut out = Vec::new();
        mix_channels(&[1.0, 0.0, 0.5, 0.5], 2, 1, &mut out);
        assert_eq!(out, vec![0.5, 0.5]);
    }

    #[test]
    fn surround_downmix_stays_in_range() {
        let mut out = Vec::new();
        mix_channels(&[1.0; 6], 6, 2, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|s| *s <= 1.0001 && *s > 0.5));
    }

    #[test]
    fn continued_resampling_keeps_total_length_and_owes_the_tail() {
        let (rin, rout) = (44_100u32, 48_000u32);
        let exp = |n: u64| (n * rout as u64 + rin as u64 / 2) / rin as u64;
        let sine = |from: usize, n: usize| -> Vec<f32> {
            (from..from + n)
                .flat_map(|i| {
                    let s = (2.0 * std::f64::consts::PI * 300.0 * i as f64 / rin as f64).sin() as f32;
                    [s, s]
                })
                .collect()
        };
        let (na, nb) = (50_000usize, 40_000usize);
        let mut c = Converter::new(rin, 2, rout, 2);
        let mut a_out = Vec::new();
        for p in sine(0, na).chunks(2 * 4096) {
            c.push(p, &mut a_out);
        }
        let owed = c.continue_track();
        assert_eq!(a_out.len() as u64 / 2 + owed, exp(na as u64));
        let mut b_out = Vec::new();
        for p in sine(na, nb).chunks(2 * 4096) {
            c.push(p, &mut b_out);
        }
        c.finish(&mut b_out);
        assert_eq!((a_out.len() + b_out.len()) as u64 / 2, exp(na as u64) + exp(nb as u64));
        // Continuous across the join.
        let all: Vec<f32> = a_out.iter().chain(b_out.iter()).step_by(2).copied().collect();
        let j = a_out.len() / 2;
        let step = all[j - 50..j + 50]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(step < 0.05, "step {step}");
    }

    #[test]
    fn resample_length_is_exact_and_signal_survives() {
        for (rin, rout) in [(44_100u32, 48_000u32), (48_000, 44_100), (96_000, 48_000)] {
            let n = 10_007usize;
            let freq = 440.0;
            let input: Vec<f32> = (0..n)
                .flat_map(|i| {
                    let s = (2.0 * std::f32::consts::PI * freq * i as f32 / rin as f32).sin() * 0.5;
                    [s, s]
                })
                .collect();
            let mut c = Converter::new(rin, 2, rout, 2);
            let mut out = Vec::new();
            // Feed in odd-sized pieces.
            for piece in input.chunks(2 * 333) {
                c.push(piece, &mut out);
            }
            c.finish(&mut out);
            let expected = (n as u64 * rout as u64 + rin as u64 / 2) / rin as u64;
            assert_eq!(out.len() as u64 / 2, expected, "{rin}->{rout}");
            // Compare against the ideal sine in the middle (delay trimmed => aligned).
            let mid = out.len() / 4;
            for f in mid..mid + 200 {
                let ideal = (2.0 * std::f32::consts::PI * freq * f as f32 / rout as f32).sin() * 0.5;
                assert!((out[f * 2] - ideal).abs() < 0.02, "{rin}->{rout} frame {f}");
            }
        }
    }
}
