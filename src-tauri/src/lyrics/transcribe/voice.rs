//! The voice separated from the music before the speech model listens.
//!
//! Most made-up words in a transcription come from the band: the speech
//! model hears guitars and drums as syllables. HTDemucs v4 (Meta's Demucs,
//! MIT, run by demucs-core in pure Rust on the graphics card through wgpu)
//! pulls the singing out first, and the speech model hears only that. It
//! runs in the speech helper, and only while a graphics card is in use: on
//! the processor it would take longer than the song. Any trouble falls back
//! to the full mix, logged, so the song is still written down, just less
//! cleanly.

use std::path::Path;

use super::audio::Decoded;
use super::engine::{Progress, Stop};
use super::model::ModelSpec;

/// The separation model's sample rate.
pub const RATE: u32 = 44_100;
/// Longest song separated: twenty minutes (a longer one is heard as it is,
/// and isn't decoded to stereo at all).
pub const MAX_SECONDS: usize = 20 * 60;

/// The voice model (there is one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Voice {
    Demucs,
}

/// The HTDemucs v4 weights as safetensors, pinned to one commit of the
/// repository that hosts the conversion and checked against their SHA-256.
pub fn catalogue() -> Vec<ModelSpec<Voice>> {
    vec![ModelSpec {
        kind: Voice::Demucs,
        file: "htdemucs.safetensors".into(),
        url: "https://huggingface.co/set-soft/audio_separation/resolve/939723568b2dca203e61cc7294317ba38549964f/Demucs/htdemucs.safetensors".into(),
        size: 84_030_696,
        sha256: "8193504cdfb3943adaf039b8acb524a46e87ebf232c383ac7a32c80a6578423e".into(),
    }]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceError {
    Cancelled,
    Failed(String),
    /// The helper process stopped in the middle of it.
    Crashed,
}

/// Separates the singing from a song.
pub trait Separator: Send + Sync {
    /// The singing in `song` (interleaved stereo at [`RATE`]) as 16 kHz
    /// mono, the speech model's input. Blocking; call from a blocking
    /// thread. `stop` ends it between stretches of the song.
    fn vocals(&self, weights: &Path, song: &[f32], stop: Stop, progress: Progress) -> Result<Vec<f32>, VoiceError>;
    /// Free the loaded model.
    fn unload(&self) {}
}

/// How the speech model heard a song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heard {
    /// The singing alone.
    Voice,
    /// The whole mix: no separation was asked for, or the song is too long.
    Mix,
    /// The whole mix because the separation went wrong.
    MixAfterTrouble,
}

/// The speech model's input for a song: its singing when it can be
/// separated, else the whole mix. Only a cancel is an error.
pub fn voice_or_mix(
    sep: Option<(&dyn Separator, &Path)>,
    song: Decoded,
    stop: &Stop,
    progress: Progress,
) -> Result<(Vec<f32>, Heard), VoiceError> {
    let mut heard = Heard::Mix;
    if let (Some((sep, weights)), Some(stereo)) = (sep, song.stereo.as_deref()) {
        match sep.vocals(weights, stereo, stop.clone(), progress) {
            Ok(v) => return Ok((v, Heard::Voice)),
            Err(VoiceError::Cancelled) => return Err(VoiceError::Cancelled),
            Err(e) => {
                tracing::warn!("the voice wasn't separated, transcribing the whole mix: {e:?}");
                heard = Heard::MixAfterTrouble;
            }
        }
    }
    if stop.is_set() {
        return Err(VoiceError::Cancelled);
    }
    Ok((song.speech, heard))
}

/// The separator this build has, in this process (the helper runs it).
pub fn in_process() -> Option<std::sync::Arc<dyn Separator>> {
    #[cfg(feature = "transcribe")]
    {
        Some(std::sync::Arc::new(demucs::Demucs::default()))
    }
    #[cfg(not(feature = "transcribe"))]
    {
        None
    }
}

#[cfg(feature = "transcribe")]
pub mod demucs {
    use std::path::{Path, PathBuf};

    use demucs_core::listener::{ForwardEvent, ForwardListener};
    use demucs_core::model::metadata::StemId;
    use demucs_core::{ModelOptions, num_chunks};
    use parking_lot::Mutex;

    use super::super::engine::{Progress, Step, Stop};
    use super::{RATE, Separator, VoiceError};

    type B = burn::backend::wgpu::Wgpu;

    /// HTDemucs on the graphics card, keeping the model loaded between songs.
    #[derive(Default)]
    pub struct Demucs {
        loaded: Mutex<Option<(PathBuf, demucs_core::Demucs<B>)>>,
        ready: Mutex<bool>,
    }

    /// Passes the stop flag in and the chunks done out.
    struct Follow {
        stop: Stop,
        progress: Progress,
    }

    impl ForwardListener for Follow {
        fn on_event(&mut self, event: ForwardEvent) {
            if let ForwardEvent::ChunkDone { index, total } = event {
                (self.progress)(Step::Percent((((index + 1) * 100) / total.max(1)).min(100) as u8));
            }
        }
        fn is_cancelled(&self) -> bool {
            self.stop.is_set()
        }
    }

    impl Demucs {
        fn load(&self, weights: &Path) -> Result<(), VoiceError> {
            let mut slot = self.loaded.lock();
            if slot.as_ref().is_some_and(|(p, _)| p == weights) {
                return Ok(());
            }
            *slot = None;
            let device = burn::backend::wgpu::WgpuDevice::default();
            let mut ready = self.ready.lock();
            if !*ready {
                burn::backend::wgpu::init_setup::<burn::backend::wgpu::graphics::AutoGraphicsApi>(&device, Default::default());
                *ready = true;
            }
            let bytes = std::fs::read(weights).map_err(|e| VoiceError::Failed(format!("voice model unreadable: {e}")))?;
            let model = demucs_core::Demucs::<B>::from_bytes(ModelOptions::FourStem, &bytes, device)
                .map_err(|e| VoiceError::Failed(format!("voice model didn't load: {e}")))?;
            *slot = Some((weights.to_path_buf(), model));
            Ok(())
        }

        fn run(&self, weights: &Path, song: &[f32], stop: Stop, progress: Progress) -> Result<Vec<f32>, VoiceError> {
            progress(Step::Loading);
            self.load(weights)?;
            let (left, right): (Vec<f32>, Vec<f32>) = song.chunks_exact(2).map(|f| (f[0], f[1])).unzip();
            tracing::info!(chunks = num_chunks(left.len()), "separating the voice");
            progress(Step::Percent(0));
            let slot = self.loaded.lock();
            let (_, model) = slot.as_ref().expect("just loaded");
            let started = std::time::Instant::now();
            let mut follow = Follow { stop: stop.clone(), progress };
            let stems = pollster::block_on(model.separate_with_listener(&left, &right, RATE, &mut follow));
            if stop.is_set() {
                return Err(VoiceError::Cancelled);
            }
            let stems = stems.map_err(|e| VoiceError::Failed(e.to_string()))?;
            let vocals = stems.into_iter().find(|s| s.id == StemId::Vocals).ok_or_else(|| VoiceError::Failed("no vocals stem".into()))?;
            tracing::info!(audio_s = left.len() / RATE as usize, took_ms = started.elapsed().as_millis() as u64, "voice separated");
            let stereo: Vec<f32> = vocals.left.iter().zip(&vocals.right).flat_map(|(l, r)| [*l, *r]).collect();
            Ok(to_speech(&stereo))
        }
    }

    /// Interleaved stereo at [`RATE`] to 16 kHz mono.
    pub fn to_speech(stereo: &[f32]) -> Vec<f32> {
        use crate::audio::convert::Converter;
        let Ok(mut conv) = Converter::new(RATE, 2, super::super::audio::RATE, 1) else { return Vec::new() };
        let mut out = Vec::with_capacity(stereo.len() / 2 * super::super::audio::RATE as usize / RATE as usize + 1);
        for block in stereo.chunks(2 * 4096) {
            conv.push(block, &mut out);
        }
        conv.finish(&mut out);
        out
    }

    impl Separator for Demucs {
        fn vocals(&self, weights: &Path, song: &[f32], stop: Stop, progress: Progress) -> Result<Vec<f32>, VoiceError> {
            // The graphics library panics rather than failing on some
            // drivers: that's a failed separation, not a lost helper.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run(weights, song, stop, progress))) {
                Ok(r) => r,
                Err(_) => {
                    *self.loaded.lock() = None;
                    Err(VoiceError::Failed("the graphics card stopped while separating the voice".into()))
                }
            }
        }

        fn unload(&self) {
            *self.loaded.lock() = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Fake {
        result: Result<Vec<f32>, VoiceError>,
        runs: AtomicUsize,
    }

    impl Separator for Fake {
        fn vocals(&self, _: &Path, _: &[f32], _: Stop, _: Progress) -> Result<Vec<f32>, VoiceError> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            self.result.clone()
        }
    }

    fn song(seconds: usize) -> Decoded {
        let stereo = (0..seconds * RATE as usize * 2).map(|i| ((i / 2) as f32 * 0.01).sin() * 0.5).collect();
        Decoded { speech: vec![0.1; seconds * 16_000], stereo: Some(stereo) }
    }

    fn none() -> Progress {
        Arc::new(|_| {})
    }

    #[test]
    fn the_voice_is_heard_when_it_separates() {
        let f = Fake { result: Ok(vec![0.25; 16_000]), runs: AtomicUsize::new(0) };
        let (pcm, heard) = voice_or_mix(Some((&f, Path::new("v.safetensors"))), song(1), &Stop::new(), none()).unwrap();
        assert_eq!(heard, Heard::Voice);
        assert_eq!(pcm, vec![0.25; 16_000]);
    }

    #[test]
    fn a_failed_separation_falls_back_to_the_mix() {
        for e in [VoiceError::Failed("out of memory".into()), VoiceError::Crashed] {
            let f = Fake { result: Err(e), runs: AtomicUsize::new(0) };
            let (pcm, heard) = voice_or_mix(Some((&f, Path::new("v"))), song(2), &Stop::new(), none()).unwrap();
            assert_eq!(heard, Heard::MixAfterTrouble);
            assert_eq!(pcm, vec![0.1; 32_000]);
        }
    }

    #[test]
    fn without_a_separator_or_the_stereo_the_mix_is_heard() {
        let (_, heard) = voice_or_mix(None, song(1), &Stop::new(), none()).unwrap();
        assert_eq!(heard, Heard::Mix);
        let f = Fake { result: Ok(vec![0.0]), runs: AtomicUsize::new(0) };
        let long = Decoded { speech: vec![0.0; 16_000], stereo: None };
        let (_, heard) = voice_or_mix(Some((&f, Path::new("v"))), long, &Stop::new(), none()).unwrap();
        assert_eq!(heard, Heard::Mix);
        assert_eq!(f.runs.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_cancel_stops_it() {
        let f = Fake { result: Err(VoiceError::Cancelled), runs: AtomicUsize::new(0) };
        assert_eq!(voice_or_mix(Some((&f, Path::new("v"))), song(1), &Stop::new(), none()).unwrap_err(), VoiceError::Cancelled);
        let stop = Stop::new();
        stop.now();
        assert_eq!(voice_or_mix(None, song(1), &stop, none()).unwrap_err(), VoiceError::Cancelled);
    }

    #[test]
    fn the_voice_model_is_pinned_and_checksummed() {
        let c = catalogue();
        assert_eq!(c.len(), 1);
        assert!(c[0].url.starts_with("https://huggingface.co/") && c[0].url.contains("/resolve/939723568b2dca203e61cc7294317ba38549964f/"));
        assert_eq!(c[0].sha256.len(), 64);
        assert!(c[0].size > 80_000_000);
    }
}
