//! The speech recognizer behind a trait, so the queue and its tests don't
//! need a model. The app's one is the helper process ([`super::helper`]),
//! which runs whisper.cpp (the `transcribe` build feature) in its own
//! low-priority process; builds without it say transcription isn't
//! available.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::lines::{Segment, Word};
use super::model::ModelKind;

/// The language songs are transcribed in.
pub const LANGUAGE: &str = "en";

/// How a run is getting on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "step", content = "percent", rename_all = "camelCase")]
pub enum Step {
    /// Loading the model from disk (before the first percent).
    Loading,
    /// Listening: the percent done (0 to 100). The speech library reports
    /// it once per 30 s stretch of the song.
    Percent(u8),
}

/// Called as a run moves on.
pub type Progress = Arc<dyn Fn(Step) + Send + Sync>;

/// Stops a run early. Asked to give way, a run stops at the speech
/// library's next check (the end of the step it's on), keeping its helper
/// and the loaded model; stopped now, a helper that doesn't stop within
/// about a second is ended.
#[derive(Debug, Clone, Default)]
pub struct Stop {
    flag: Arc<AtomicBool>,
    now: Arc<AtomicBool>,
}

impl Stop {
    pub fn new() -> Self {
        Self::default()
    }
    /// Stop at the next check (another song comes first).
    pub fn give_way(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }
    /// Stop as soon as possible (the user cancelled).
    pub fn now(&self) {
        self.now.store(true, Ordering::Relaxed);
        self.flag.store(true, Ordering::Relaxed);
    }
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
    pub fn is_now(&self) -> bool {
        self.now.load(Ordering::Relaxed)
    }
    /// The flag the speech library's abort check reads.
    pub fn flag(&self) -> &Arc<AtomicBool> {
        &self.flag
    }
}

/// One token as the recognizer gave it: its raw text bytes (a token can
/// end partway through a character, such as the three bytes of "♪"), its
/// times in milliseconds and the attention-based start when known.
#[derive(Debug, Clone, PartialEq)]
pub struct RawToken {
    pub bytes: Vec<u8>,
    pub start_ms: u32,
    pub end_ms: u32,
    pub dtw_ms: Option<u32>,
}

/// A segment's tokens to words. A word starts at a token beginning with a
/// space; its bytes are joined first and decoded once, so a character
/// split across tokens comes out whole.
pub fn words_from_tokens(tokens: &[RawToken]) -> Vec<Word> {
    let mut raw: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    for t in tokens {
        let new_word = raw.is_empty() || t.bytes.first() == Some(&b' ');
        match raw.last_mut() {
            Some(w) if !new_word => {
                w.2.extend_from_slice(&t.bytes);
                w.1 = w.1.max(t.end_ms);
            }
            _ => {
                let start = t.dtw_ms.unwrap_or(t.start_ms);
                raw.push((start, t.end_ms.max(start), t.bytes.clone()));
            }
        }
    }
    raw.into_iter()
        .map(|(start_ms, end_ms, bytes)| Word { start_ms, end_ms, text: String::from_utf8_lossy(&bytes).into_owned() })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecognizeError {
    Cancelled,
    Failed(String),
    /// The helper process stopped in the middle of the run.
    Crashed,
}

/// One run: 16 kHz mono samples in, timed words grouped into segments out.
pub trait Recognizer: Send + Sync {
    /// Why transcription can't run on this computer, if it can't.
    fn unavailable(&self) -> Option<String>;
    /// Blocking; call from a blocking thread. `cancel` stops it early.
    #[allow(clippy::too_many_arguments)]
    fn transcribe(
        &self,
        model: &Path,
        kind: ModelKind,
        pcm: &[f32],
        threads: usize,
        language: &str,
        cancel: Stop,
        progress: Progress,
    ) -> Result<Vec<Segment>, RecognizeError>;
    /// Free the loaded model (the queue is empty).
    fn unload(&self) {}
    /// Another caller is waiting for this recognizer (a lyrics check): a
    /// background song in progress gives way to it.
    fn wanted(&self) -> bool {
        false
    }
    /// The recognizer crashed often enough this session that background
    /// work should stop.
    fn crashed_too_often(&self) -> bool {
        false
    }
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

/// Why this build or computer can't transcribe, if it can't.
pub fn unavailable_here() -> Option<String> {
    #[cfg(feature = "transcribe")]
    {
        (!cpu_supported()).then(|| "Transcription needs a processor with AVX2 (most from 2013 on).".to_string())
    }
    #[cfg(not(feature = "transcribe"))]
    {
        Some("This build of Platter doesn't include transcription.".into())
    }
}

/// The recognizer this build has, in this process (the helper process runs it).
pub fn in_process() -> Arc<dyn Recognizer> {
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
        unavailable_here()
    }
    fn transcribe(&self, _: &Path, _: ModelKind, _: &[f32], _: usize, _: &str, _: Stop, _: Progress) -> Result<Vec<Segment>, RecognizeError> {
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

    use super::super::lines::Segment;
    use super::super::model::ModelKind;
    use super::{Progress, RawToken, RecognizeError, Recognizer, Step, Stop, unavailable_here, words_from_tokens};

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

    /// The GPU whisper.cpp picks when asked for one, if this build has one.
    fn gpu_name() -> Option<String> {
        use whisper_rs::whisper_rs_sys as sys;
        // SAFETY: the device registry is static; names are C strings it owns.
        unsafe {
            (0..sys::ggml_backend_dev_count()).map(|i| sys::ggml_backend_dev_get(i)).find_map(|dev| {
                let kind = sys::ggml_backend_dev_type(dev);
                let gpu = kind == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU
                    || kind == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU;
                let name = sys::ggml_backend_dev_description(dev);
                (gpu && !name.is_null()).then(|| std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned())
            })
        }
    }

    fn load(path: &Path, kind: ModelKind) -> Result<WhisperContext, RecognizeError> {
        // Metal on macOS (much cooler than the processor); the processor elsewhere.
        let gpu = if cfg!(target_os = "macos") { gpu_name() } else { None };
        if let Some(name) = &gpu {
            match load_on(path, kind, true) {
                Ok(ctx) => {
                    tracing::info!("speech model on the GPU ({name})");
                    return Ok(ctx);
                }
                Err(e) => tracing::warn!("speech model didn't load on the GPU ({name}), using the processor: {e:?}"),
            }
        } else if cfg!(target_os = "macos") {
            tracing::info!("no GPU for speech, using the processor");
        }
        load_on(path, kind, false)
    }

    fn load_on(path: &Path, kind: ModelKind, use_gpu: bool) -> Result<WhisperContext, RecognizeError> {
        let p = WhisperContextParameters {
            use_gpu,
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
            unavailable_here()
        }

        fn transcribe(
            &self,
            model: &Path,
            kind: ModelKind,
            pcm: &[f32],
            threads: usize,
            language: &str,
            stop: Stop,
            progress: Progress,
        ) -> Result<Vec<Segment>, RecognizeError> {
            let cancel = stop.flag().clone();
            let mut slot = self.ctx.lock();
            if slot.as_ref().is_none_or(|(p, _)| p != model) {
                *slot = None;
                progress(Step::Loading);
                *slot = Some((model.to_path_buf(), load(model, kind)?));
            }
            let (_, ctx) = slot.as_ref().expect("just loaded");
            let mut state = ctx.create_state().map_err(|e| RecognizeError::Failed(e.to_string()))?;
            progress(Step::Percent(0));

            let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 5 });
            p.set_n_threads(threads as i32);
            p.set_language(Some(language));
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
            p.set_progress_callback_safe(move |pct: i32| progress(Step::Percent(pct.clamp(0, 100) as u8)));

            let res = state.full(p, pcm);
            if cancel.load(Ordering::Relaxed) {
                return Err(RecognizeError::Cancelled);
            }
            res.map_err(|e| RecognizeError::Failed(e.to_string()))?;

            let eot = ctx.token_eot();
            let mut out = Vec::new();
            for seg in state.as_iter() {
                let mut tokens = Vec::new();
                for i in 0..seg.n_tokens() {
                    let Some(tok) = seg.get_token(i) else { continue };
                    // Special and timestamp tokens come after end-of-text.
                    if tok.token_id() >= eot {
                        continue;
                    }
                    // Raw bytes: a token can hold part of a character.
                    let Ok(bytes) = tok.to_bytes() else { continue };
                    let d = tok.token_data();
                    tokens.push(RawToken {
                        bytes: bytes.to_vec(),
                        start_ms: (d.t0.max(0) * 10) as u32,
                        end_ms: (d.t1.max(0) * 10) as u32,
                        dtw_ms: (d.t_dtw >= 0).then(|| (d.t_dtw * 10) as u32),
                    });
                }
                let words = words_from_tokens(&tokens);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(bytes: &[u8], at: u32) -> RawToken {
        RawToken { bytes: bytes.to_vec(), start_ms: at, end_ms: at + 100, dtw_ms: None }
    }

    #[test]
    fn a_character_split_across_tokens_comes_out_whole() {
        let note = "♪".as_bytes();
        let tokens = [tok(b" ", 0), tok(&note[..2], 0), tok(&note[2..], 100), tok(b" I'm", 200), tok(b" fine", 300), tok(&note[..1], 400), tok(&note[1..], 450)];
        let words = words_from_tokens(&tokens);
        let text: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(text, vec![" ♪", " I'm", " fine♪"]);
        assert!(words.iter().all(|w| !w.text.contains('\u{fffd}')));
        assert_eq!((words[0].start_ms, words[0].end_ms), (0, 200), "a word spans its tokens");
    }

    #[test]
    fn the_attention_time_starts_a_word_when_known() {
        let words = words_from_tokens(&[RawToken { bytes: b" hey".to_vec(), start_ms: 500, end_ms: 900, dtw_ms: Some(620) }]);
        assert_eq!((words[0].start_ms, words[0].end_ms), (620, 900));
    }

    #[test]
    fn give_way_and_now() {
        let s = Stop::new();
        assert!(!s.is_set());
        s.give_way();
        assert!(s.is_set() && !s.is_now());
        let s = Stop::new();
        s.clone().now();
        assert!(s.is_set() && s.is_now(), "clones share the flags");
    }
}
