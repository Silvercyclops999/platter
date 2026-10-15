//! Bit-perfect output: the device opened for this app alone, at the music's
//! own sample rate, so samples reach it untouched.
//!
//! Windows: a WASAPI exclusive-mode stream of our own (cpal only opens shared
//! streams), event driven, on the output thread. macOS: the device is taken
//! in hog mode at the right nominal rate, and cpal plays on it. Anything that
//! stops either (the device busy, the rate unsupported, exclusive use turned
//! off in the system settings) comes back as a plain reason, and the output
//! falls back to shared mode.

#[cfg(not(target_os = "windows"))]
use std::sync::Arc;
#[cfg(not(target_os = "windows"))]
use std::sync::mpsc::Receiver;

#[cfg(not(target_os = "windows"))]
use super::output::{Renderer, StreamCtl};

#[cfg(target_os = "windows")]
mod wasapi;
#[cfg(target_os = "windows")]
pub(super) use wasapi::OwnStream;

#[cfg(target_os = "macos")]
mod coreaudio;
#[cfg(target_os = "macos")]
pub(super) use coreaudio::DeviceGuard;

/// Never made where the system has no exclusive stream of our own.
#[cfg(not(target_os = "windows"))]
pub(super) enum OwnStream {}

#[cfg(not(target_os = "windows"))]
impl OwnStream {
    pub(super) fn run(self, _: Arc<dyn Renderer>, _: Receiver<StreamCtl>, _: Box<dyn FnOnce(String) + Send>) {
        match self {}
    }
}

/// Never made where the device isn't taken separately from the stream.
#[cfg(not(target_os = "macos"))]
pub(super) enum DeviceGuard {}

/// What an exclusive open got.
pub(super) enum Opened {
    /// A stream of our own, already running its format (Windows).
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Own { stream: OwnStream, rate: u32, channels: u16, bits: u16 },
    /// The device is ours and set to `rate`; play on it through cpal while
    /// the guard lives (macOS).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Device { guard: DeviceGuard, rate: u32, bits: u16 },
}

/// Opens `device_id` (the id from the device list) exclusively at `rate`.
#[allow(unused_variables)]
pub(super) fn open(device_id: &str, rate: u32, channels: u16) -> Result<Opened, String> {
    // The list's ids are "<host>:<system id>".
    let system_id = device_id.split_once(':').map_or(device_id, |(_, id)| id);
    #[cfg(target_os = "windows")]
    {
        wasapi::open(system_id, rate, channels)
    }
    #[cfg(target_os = "macos")]
    {
        coreaudio::open(system_id, rate)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Err("Bit-perfect output isn't available on this system.".into())
    }
}

/// Writes interleaved f32 samples into the device's integer or float
/// layout. Exact for 16 and 24-bit music: those samples are k / 2^15 or
/// k / 2^23, which these scale back to k.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) enum SampleLayout {
    I16,
    /// 24 bits packed in 3 bytes.
    I24,
    /// 24 bits in a 32-bit word (low byte zero).
    I24In32,
    I32,
    F32,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
impl SampleLayout {
    pub(crate) fn bytes(self) -> usize {
        match self {
            Self::I16 => 2,
            Self::I24 => 3,
            Self::I24In32 | Self::I32 | Self::F32 => 4,
        }
    }

    pub(crate) fn valid_bits(self) -> u16 {
        match self {
            Self::I16 => 16,
            Self::I24 | Self::I24In32 => 24,
            Self::I32 | Self::F32 => 32,
        }
    }

    /// `out` holds `src.len() * self.bytes()` bytes.
    pub(crate) fn write(self, src: &[f32], out: &mut [u8]) {
        fn int(s: f32, scale: f64, min: f64, max: f64) -> i64 {
            (s as f64 * scale).round().clamp(min, max) as i64
        }
        let n = self.bytes();
        for (s, o) in src.iter().zip(out.chunks_exact_mut(n)) {
            match self {
                Self::I16 => o.copy_from_slice(&(int(*s, 32_768.0, -32_768.0, 32_767.0) as i16).to_le_bytes()),
                Self::I24 => o.copy_from_slice(&(int(*s, 8_388_608.0, -8_388_608.0, 8_388_607.0) as i32).to_le_bytes()[..3]),
                Self::I24In32 => {
                    let v = int(*s, 8_388_608.0, -8_388_608.0, 8_388_607.0) as i32;
                    o.copy_from_slice(&(v << 8).to_le_bytes());
                }
                Self::I32 => {
                    o.copy_from_slice(&(int(*s, 2_147_483_648.0, -2_147_483_648.0, 2_147_483_647.0) as i32).to_le_bytes())
                }
                Self::F32 => o.copy_from_slice(&s.to_le_bytes()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SampleLayout as L;

    fn write(l: L, s: &[f32]) -> Vec<u8> {
        let mut out = vec![0u8; s.len() * l.bytes()];
        l.write(s, &mut out);
        out
    }

    #[test]
    fn sixteen_bit_music_comes_back_exactly() {
        for k in [-32_768i32, -12_345, -1, 0, 1, 255, 32_767] {
            let s = k as f32 / 32_768.0;
            assert_eq!(write(L::I16, &[s]), (k as i16).to_le_bytes());
            // Into a wider container it's the same value, padded.
            assert_eq!(write(L::I32, &[s]), (k << 16).to_le_bytes());
            assert_eq!(write(L::I24In32, &[s]), (k << 16).to_le_bytes());
        }
    }

    #[test]
    fn twenty_four_bit_music_comes_back_exactly() {
        for k in [-8_388_608i32, -1_234_567, -1, 0, 1, 8_388_607] {
            let s = k as f32 / 8_388_608.0;
            assert_eq!(write(L::I24, &[s]), k.to_le_bytes()[..3]);
            assert_eq!(write(L::I24In32, &[s]), (k << 8).to_le_bytes());
            assert_eq!(write(L::I32, &[s]), (k << 8).to_le_bytes());
        }
    }

    #[test]
    fn overs_clip_instead_of_wrapping() {
        assert_eq!(write(L::I16, &[1.5]), 32_767i16.to_le_bytes());
        assert_eq!(write(L::I16, &[-1.5]), (-32_768i16).to_le_bytes());
        assert_eq!(write(L::I32, &[1.0]), i32::MAX.to_le_bytes());
        assert_eq!(write(L::F32, &[0.25]), 0.25f32.to_le_bytes());
    }
}
