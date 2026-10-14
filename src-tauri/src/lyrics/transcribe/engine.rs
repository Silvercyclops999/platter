//! The speech recognizer behind a trait, so the queue and its tests don't
//! need a model. The real one is whisper.cpp (the `transcribe` build
//! feature); builds without it say transcription isn't available.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::lines::Segment;
use super::model::ModelKind;

/// Called with the percent done (0 to 100).
pub type Progress = Arc<dyn Fn(u8) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecognizeError {
    Cancelled,
    Failed(String),
}

/// One run: 16 kHz mono samples in, timed words grouped into segments out.
pub trait Recognizer: Send + Sync {
    /// Why transcription can't run on this computer, if it can't.
    fn unavailable(&self) -> Option<String>;
    /// Blocking; call from a blocking thread. `cancel` stops it early.
    fn transcribe(
        &self,
        model: &Path,
        kind: ModelKind,
        pcm: &[f32],
        threads: usize,
        cancel: Arc<AtomicBool>,
        progress: Progress,
    ) -> Result<Vec<Segment>, RecognizeError>;
    /// Free the loaded model (the queue is empty).
    fn unload(&self) {}
}

/// Threads for a run: about half the cores for a song asked for now, a
/// quarter for the background batch, so playback and the UI keep theirs.
pub fn threads(background: bool) -> usize {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    if background { (n / 4).clamp(1, 4) } else { (n / 2).clamp(1, 8) }
}

/// The CPU features the shipped whisper.cpp build assumes on x86.
#[cfg(target_arch = "x86_64")]
pub fn cpu_supported() -> bool {
    std::is_x86_feature_detected!("avx2")
        && std::is_x86_feature_detected!("fma")
        && std::is_x86_feature_detected!("f16c")
        && std::is_x86_feature_detected!("bmi2")
}

#[cfg(not(target_arch = "x86_64"))]
pub fn cpu_supported() -> bool {
    true
}

/// The recognizer this build has.
pub fn default_recognizer() -> Arc<dyn Recognizer> {
    #[cfg(feature = "transcribe")]
    {
        Arc::new(whisper::Whisper::default())
    }
    #[cfg(not(feature = "transcribe"))]
    {
        Arc::new(Missing)
    }
}

/// A build without the recognizer.
#[cfg(not(feature = "transcribe"))]
struct Missing;

#[cfg(not(feature = "transcribe"))]
impl Recognizer for Missing {
    fn unavailable(&self) -> Option<String> {
        Some("This build of Platter doesn't include transcription.".into())
    }
    fn transcribe(&self, _: &Path, _: ModelKind, _: &[f32], _: usize, _: Arc<AtomicBool>, _: Progress) -> Result<Vec<Segment>, RecognizeError> {
        Err(RecognizeError::Failed("not in this build".into()))
    }
}

#[cfg(feature = "transcribe")]
pub mod whisper {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use parking_lot::Mutex;
    use whisper_rs::{
        DtwMode, DtwModelPreset, DtwParameters, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
    };

    use super::super::lines::{Segment, Word};
    use super::super::model::ModelKind;
    use super::{Progress, RecognizeError, Recognizer, cpu_supported};

    /// whisper.cpp, keeping the last model loaded between songs.
    #[derive(Default)]
    pub struct Whisper {
        ctx: Mutex<Option<(PathBuf, WhisperContext)>>,
    }

    fn preset(kind: ModelKind) -> DtwModelPreset {
        match kind {
            ModelKind::Small => DtwModelPreset::SmallEn,
            ModelKind::Medium => DtwModelPreset::MediumEn,
        }
    }

    fn load(path: &Path, kind: ModelKind) -> Result<WhisperContext, RecognizeError> {
        let p = WhisperContextParameters {
            // CPU on every platform for now; the GPU builds need more toolchain.
            use_gpu: false,
            // Word times from the attention heads, steadier than token guesses.
            dtw_parameters: DtwParameters { mode: DtwMode::ModelPreset { model_preset: preset(kind) }, ..Default::default() },
            ..Default::default()
        };
        let fail = |e: whisper_rs::WhisperError| RecognizeError::Failed(format!("model didn't load: {e}"));
        match path.to_str().filter(|s| s.is_ascii()) {
            Some(s) => WhisperContext::new_with_params(s, p).map_err(fail),
            // whisper.cpp opens files with the ANSI code page on Windows, so
            // a folder with other characters in its name is read here.
            None => {
                let bytes = std::fs::read(path).map_err(|e| RecognizeError::Failed(format!("model unreadable: {e}")))?;
                WhisperContext::new_from_buffer_with_params(&bytes, p).map_err(fail)
            }
        }
    }

    impl Recognizer for Whisper {
        fn unavailable(&self) -> Option<String> {
            (!cpu_supported()).then(|| "Transcription needs a processor with AVX2 (most from 2013 on).".to_string())
        }

        fn transcribe(
            &self,
            model: &Path,
            kind: ModelKind,
            pcm: &[f32],
            threads: usize,
            cancel: Arc<AtomicBool>,
            progress: Progress,
        ) -> Result<Vec<Segment>, RecognizeError> {
            let mut slot = self.ctx.lock();
            if slot.as_ref().is_none_or(|(p, _)| p != model) {
                *slot = None;
                *slot = Some((model.to_path_buf(), load(model, kind)?));
            }
            let (_, ctx) = slot.as_ref().expect("just loaded");
            let mut state = ctx.create_state().map_err(|e| RecognizeError::Failed(e.to_string()))?;

            let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 5 });
            p.set_n_threads(threads as i32);
            p.set_language(Some("en"));
            p.set_token_timestamps(true);
            // Each window on its own: carrying text over makes the model loop
            // on a chorus.
            p.set_no_context(true);
            p.set_suppress_nst(true);
            p.set_print_progress(false);
            p.set_print_realtime(false);
            p.set_print_special(false);
            p.set_print_timestamps(false);
            // whisper-rs 0.16's safe abort callback casts its closure to the
            // wrong type, so the flag is handed over directly. `cancel` lives
            // until `full` returns, which is the only time it's read.
            unsafe extern "C" fn should_abort(data: *mut std::ffi::c_void) -> bool {
                // SAFETY: `data` is the AtomicBool inside `cancel`, alive for the run.
                unsafe { (*(data as *const AtomicBool)).load(Ordering::Relaxed) }
            }
            // SAFETY: see above; the pointer outlives the call to `full`.
            unsafe {
                p.set_abort_callback(Some(should_abort));
                p.set_abort_callback_user_data(Arc::as_ptr(&cancel) as *mut std::ffi::c_void);
            }
            p.set_progress_callback_safe(move |pct: i32| progress(pct.clamp(0, 100) as u8));

            let res = state.full(p, pcm);
            if cancel.load(Ordering::Relaxed) {
                return Err(RecognizeError::Cancelled);
            }
            res.map_err(|e| RecognizeError::Failed(e.to_string()))?;

            let eot = ctx.token_eot();
            let mut out = Vec::new();
            for seg in state.as_iter() {
                let mut words: Vec<Word> = Vec::new();
                for i in 0..seg.n_tokens() {
                    let Some(tok) = seg.get_token(i) else { continue };
                    // Special and timestamp tokens come after end-of-text.
                    if tok.token_id() >= eot {
                        continue;
                    }
                    let Ok(text) = tok.to_str_lossy() else { continue };
                    let d = tok.token_data();
                    let (t0, t1) = ((d.t0.max(0) * 10) as u32, (d.t1.max(0) * 10) as u32);
                    let start = if d.t_dtw >= 0 { (d.t_dtw * 10) as u32 } else { t0 };
                    let new_word = words.is_empty() || text.starts_with(' ');
                    match words.last_mut() {
                        Some(w) if !new_word => {
                            w.text.push_str(&text);
                            w.end_ms = w.end_ms.max(t1);
                        }
                        _ => words.push(Word { start_ms: start, end_ms: t1.max(start), text: text.into_owned() }),
                    }
                }
                if !words.is_empty() {
                    out.push(Segment { words });
                }
            }
            Ok(out)
        }

        fn unload(&self) {
            *self.ctx.lock() = None;
        }
    }
}
