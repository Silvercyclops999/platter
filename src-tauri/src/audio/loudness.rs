//! How loud a song is over its whole length, measured while it plays, for
//! songs the server has no loudness for. The measure is the broadcast
//! standard one (ITU-R BS.1770): a filter that hears roughly the way ears
//! do, 400 ms blocks every 100 ms, quiet blocks left out (under -70, and
//! under 10 below the song's own level), and the mean of the rest. The
//! leveling gain is then a single volume change for the whole song, so
//! nothing about how the song was made changes.

/// Blocks under this (LUFS) never count.
const ABSOLUTE_GATE: f64 = -70.0;
/// Blocks this far under the song's level don't count either.
const RELATIVE_GATE: f64 = -10.0;
/// Too short to say how loud it is.
const MIN_BLOCKS: usize = 10;

/// One biquad, direct form II transposed.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn new(b: [f64; 3], a: [f64; 2]) -> Self {
        Self { b, a, z: [0.0; 2] }
    }

    #[inline]
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// The two filter stages for `rate` (a high shelf near 1.7 kHz and a high
/// pass near 38 Hz), worked out for any sample rate.
fn k_weighting(rate: f64) -> (Biquad, Biquad) {
    use std::f64::consts::PI;
    let (f0, g, q) = (1_681.974_450_955_533, 3.999_843_853_973_347, 0.707_175_236_955_419_6);
    let k = (PI * f0 / rate).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad::new(
        [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    );
    let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = (PI * f0 / rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let high = Biquad::new([1.0, -2.0, 1.0], [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0]);
    (shelf, high)
}

/// Measures integrated loudness and the sample peak of interleaved audio.
pub struct LoudnessMeter {
    ch: usize,
    /// Per channel: its two filter stages and its weight (0 for LFE).
    filters: Vec<(Biquad, Biquad, f64)>,
    /// Frames in 100 ms.
    step: usize,
    acc: f64,
    count: usize,
    /// The last three 100 ms energies (a block is four of them).
    recent: [f64; 3],
    seen: usize,
    /// Mean energy of each 400 ms block over the absolute gate.
    blocks: Vec<f64>,
    peak: f32,
}

impl LoudnessMeter {
    pub fn new(rate: u32, channels: usize) -> Self {
        let ch = channels.max(1);
        let rate = rate.max(8_000);
        let (shelf, high) = k_weighting(rate as f64);
        let filters = (0..ch)
            .map(|i| {
                // 5.1 and 7.1 carry the low effects channel fourth: it
                // isn't counted, and surrounds count a little more.
                let w = match (ch, i) {
                    (6 | 8, 3) => 0.0,
                    (6 | 8, i) if i >= 4 => 1.41,
                    _ => 1.0,
                };
                (shelf, high, w)
            })
            .collect();
        Self {
            ch,
            filters,
            step: (rate as usize / 10).max(1),
            acc: 0.0,
            count: 0,
            recent: [0.0; 3],
            seen: 0,
            blocks: Vec::new(),
            peak: 0.0,
        }
    }

    pub fn push(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.ch) {
            let mut e = 0.0;
            for (s, (shelf, high, w)) in frame.iter().zip(self.filters.iter_mut()) {
                self.peak = self.peak.max(s.abs());
                let y = high.run(shelf.run(*s as f64));
                e += *w * y * y;
            }
            self.acc += e;
            self.count += 1;
            if self.count == self.step {
                self.close_step();
            }
        }
    }

    fn close_step(&mut self) {
        let e = self.acc / self.count as f64;
        self.acc = 0.0;
        self.count = 0;
        self.seen += 1;
        if self.seen >= 4 {
            let block = (self.recent.iter().sum::<f64>() + e) / 4.0;
            if lufs(block) > ABSOLUTE_GATE {
                self.blocks.push(block);
            }
        }
        self.recent = [self.recent[1], self.recent[2], e];
    }

    /// The song's integrated loudness (LUFS), when enough of it was loud
    /// enough to measure.
    pub fn integrated(&self) -> Option<f64> {
        if self.blocks.len() < MIN_BLOCKS {
            return None;
        }
        let mean = |it: &mut dyn Iterator<Item = &f64>| {
            let (sum, n) = it.fold((0.0, 0usize), |(s, n), e| (s + e, n + 1));
            (n > 0).then(|| sum / n as f64)
        };
        let gate = lufs(mean(&mut self.blocks.iter())?) + RELATIVE_GATE;
        let l = lufs(mean(&mut self.blocks.iter().filter(|e| lufs(**e) > gate))?);
        l.is_finite().then_some(l)
    }

    /// How much audio has been measured (seconds).
    pub fn seconds(&self) -> f64 {
        self.seen as f64 / 10.0
    }

    /// The highest sample (linear).
    pub fn peak(&self) -> f32 {
        self.peak
    }
}

fn lufs(energy: f64) -> f64 {
    -0.691 + 10.0 * energy.max(1e-20).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, ch: usize, hz: f64, amp: f64, secs: f64) -> Vec<f32> {
        let n = (rate as f64 * secs) as usize;
        let mut v = Vec::with_capacity(n * ch);
        for i in 0..n {
            let s = (amp * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin()) as f32;
            v.extend(std::iter::repeat_n(s, ch));
        }
        v
    }

    fn db(x: f64) -> f64 {
        10f64.powf(x / 20.0)
    }

    #[test]
    fn a_stereo_1k_tone_at_minus_23_reads_minus_23() {
        for rate in [44_100, 48_000, 96_000] {
            let mut m = LoudnessMeter::new(rate, 2);
            m.push(&sine(rate, 2, 1000.0, db(-23.0), 20.0));
            let l = m.integrated().unwrap();
            assert!((l + 23.0).abs() < 0.1, "{rate}: {l}");
        }
    }

    #[test]
    fn quiet_parts_dont_pull_the_level_down() {
        // A loud part, then a long near silent one: the gates leave the
        // quiet part out, so the song reads as loud as its loud part.
        let rate = 44_100;
        let mut m = LoudnessMeter::new(rate, 2);
        m.push(&sine(rate, 2, 1000.0, db(-20.0), 20.0));
        m.push(&sine(rate, 2, 1000.0, db(-60.0), 60.0));
        let l = m.integrated().unwrap();
        assert!((l + 20.0).abs() < 0.2, "{l}");
    }

    #[test]
    fn louder_songs_read_louder_by_the_same_amount() {
        let rate = 48_000;
        let mut a = LoudnessMeter::new(rate, 2);
        a.push(&sine(rate, 2, 440.0, db(-6.0), 10.0));
        let mut b = LoudnessMeter::new(rate, 2);
        b.push(&sine(rate, 2, 440.0, db(-18.0), 10.0));
        let d = a.integrated().unwrap() - b.integrated().unwrap();
        assert!((d - 12.0).abs() < 0.05, "{d}");
        assert!((a.peak() as f64 - db(-6.0)).abs() < 1e-3);
    }

    #[test]
    fn silence_and_tiny_clips_have_no_level() {
        let mut m = LoudnessMeter::new(44_100, 2);
        m.push(&vec![0.0; 44_100 * 2 * 10]);
        assert_eq!(m.integrated(), None);
        let mut m = LoudnessMeter::new(44_100, 2);
        m.push(&sine(44_100, 2, 1000.0, 0.5, 0.5));
        assert_eq!(m.integrated(), None);
    }

    #[test]
    fn mono_reads_3_db_under_the_same_tone_in_stereo() {
        let rate = 44_100;
        let mut mono = LoudnessMeter::new(rate, 1);
        mono.push(&sine(rate, 1, 1000.0, db(-20.0), 10.0));
        let mut st = LoudnessMeter::new(rate, 2);
        st.push(&sine(rate, 2, 1000.0, db(-20.0), 10.0));
        let d = st.integrated().unwrap() - mono.integrated().unwrap();
        assert!((d - 3.01).abs() < 0.05, "{d}");
    }
}
