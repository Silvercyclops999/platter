//! Apple Lossless frames in "escape" (uncompressed) mode: the cheapest valid
//! ALAC a receiver accepts. The PCM goes through as-is behind a 23-bit
//! header, so there's no encoder to get wrong.
//!
//! ```text
//! 3 bits  element type (1 = channel pair)
//! 4 bits  element instance (0)
//! 12 bits unused (0)
//! 1 bit   partial frame (0: the full frame length from the stream setup)
//! 2 bits  shifted bytes (0)
//! 1 bit   escape (1: samples follow uncompressed)
//! then each sample, 16 bits big-endian, interleaved left/right
//! 3 bits  end element (7)
//! ```

/// Frames per packet (the stream setup's frame length).
pub const FRAMES: usize = 352;

struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn with_capacity(bytes: usize) -> Self {
        Self { out: Vec::with_capacity(bytes), acc: 0, n: 0 }
    }
    fn put(&mut self, value: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.acc = (self.acc << 1) | ((value >> i) & 1);
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc << (8 - self.n)) as u8);
        }
        self.out
    }
}

/// One stereo frame of exactly [`FRAMES`] interleaved samples (shorter input
/// is padded with silence).
pub fn encode_stereo(pcm: &[i16]) -> Vec<u8> {
    let mut b = Bits::with_capacity(FRAMES * 4 + 4);
    b.put(1, 3);
    b.put(0, 4);
    b.put(0, 12);
    b.put(0, 1);
    b.put(0, 2);
    b.put(1, 1);
    for i in 0..FRAMES * 2 {
        b.put(u32::from(pcm.get(i).copied().unwrap_or(0) as u16), 16);
    }
    b.put(7, 3);
    b.finish()
}

/// The samples back out of an escape frame (tests and the fake receiver).
#[cfg(test)]
pub fn decode_stereo(frame: &[u8]) -> Option<Vec<i16>> {
    let bit = |i: usize| -> Option<u32> { Some(u32::from((frame.get(i / 8)? >> (7 - i % 8)) & 1)) };
    let read = |at: &mut usize, n: usize| -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | bit(*at)?;
            *at += 1;
        }
        Some(v)
    };
    let mut at = 0;
    if read(&mut at, 3)? != 1 {
        return None;
    }
    read(&mut at, 16)?;
    let (partial, shift, escape) = (read(&mut at, 1)?, read(&mut at, 2)?, read(&mut at, 1)?);
    if partial != 0 || shift != 0 || escape != 1 {
        return None;
    }
    let mut out = Vec::with_capacity(FRAMES * 2);
    for _ in 0..FRAMES * 2 {
        out.push(read(&mut at, 16)? as u16 as i16);
    }
    (read(&mut at, 3)? == 7).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_have_the_right_size() {
        let pcm: Vec<i16> = (0..FRAMES * 2).map(|i| (i as i16).wrapping_mul(97) - 300).collect();
        let f = encode_stereo(&pcm);
        // 23 header bits + 11264 sample bits + 3 end bits = 11290 bits.
        assert_eq!(f.len(), 1412);
        assert_eq!(f[0] >> 5, 1, "channel pair element");
        assert_eq!(decode_stereo(&f).unwrap(), pcm);
    }

    #[test]
    fn short_input_is_padded_with_silence() {
        let f = encode_stereo(&[5, -5]);
        let d = decode_stereo(&f).unwrap();
        assert_eq!(&d[..3], &[5, -5, 0]);
        assert!(d[2..].iter().all(|s| *s == 0));
    }
}
