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

/// The language songs are transcribed in when the model only knows English.
pub const LANGUAGE: &str = "en";
/// Asks the model to hear which language is sung.
pub const DETECT: &str = "auto";
/// A detected language this unsure falls back to English.
pub const DETECT_SURE: f32 = 0.5;

/// What a run is told about the song.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Hints {
    /// A language code, or [`DETECT`].
    pub language: String,
    /// Words the model reads before the song (its title and artist), so
    /// names come out spelled right. It guides the first stretches of the
    /// song most: the model's running context pushes it out after that.
    /// Empty for none.
    #[serde(default)]
    pub prompt: String,
}

impl Hints {
    /// English, no prompt.
    pub fn english() -> Self {
        Self { language: LANGUAGE.into(), prompt: String::new() }
    }

    /// For a song: the model's language (every language with the large
    /// model, English with the English-only ones) and its title and artist.
    pub fn for_song(kind: ModelKind, title: Option<&str>, artist: Option<&str>) -> Self {
        let language = if kind.multilingual() { DETECT } else { LANGUAGE };
        Self { language: language.into(), prompt: prompt_for(title, artist) }
    }
}

/// "Title by Artist." from whatever is known. A version note after the
/// title ("(Remastered 2011)", "- Live") is left off: it isn't sung.
pub fn prompt_for(title: Option<&str>, artist: Option<&str>) -> String {
    // A NUL (a tag separator) can't go to the speech library.
    let clean = |s: Option<&str>| s.map(|s| s.replace('\0', " ")).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let title = title.map(str::trim).map(|t| &t[..[" (", " [", " - "].iter().filter_map(|m| t.find(m)).min().unwrap_or(t.len())]);
    match (clean(title), clean(artist)) {
        (Some(t), Some(a)) => format!("{t} by {a}."),
        (Some(t), None) => format!("{t}."),
        (None, Some(a)) => format!("{a}."),
        (None, None) => String::new(),
    }
}

/// The language to transcribe in from a detection: the likeliest one when
/// it's sure enough, else English.
pub fn detected_language(best: Option<(&str, f32)>) -> &str {
    match best {
        Some((code, p)) if p >= DETECT_SURE => code,
        _ => LANGUAGE,
    }
}

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
/// times in milliseconds, the attention-based start when known and how sure
/// the model was of it (0 to 1).
#[derive(Debug, Clone, PartialEq)]
pub struct RawToken {
    pub bytes: Vec<u8>,
    pub start_ms: u32,
    pub end_ms: u32,
    pub dtw_ms: Option<u32>,
    pub p: f32,
}

/// Candidates decoded when a window is tried again at a higher temperature.
pub const FALLBACK_CANDIDATES: i32 = 2;
/// Temperature added per retry: 0.4 and 0.8, so at most two retries.
pub const FALLBACK_STEP: f32 = 0.4;

/// How stored transcriptions were timed. 1: a word started at its first
/// token's attention time, which marks where that token ends (late by about
/// one token). 2: [`token_times`].
pub const TIMING_VERSION: i64 = 2;

/// A token's sung length when the next one doesn't say.
const TOKEN_MS: u32 = 250;
/// Bounds on a token's length taken from the next token's time.
const TOKEN_MIN_MS: u32 = 80;
const TOKEN_MAX_MS: u32 = 600;
/// How much of the next token's length is taken off the attention time.
/// Measured on spoken lines over a music bed: line starts land within about
/// 0.2 s of the truth, with no lean either way.
const LEAD: f32 = 0.75;

/// When each token starts and ends, in ms.
///
/// whisper.cpp's attention time for a token is when the alignment moves on
/// from it, so it marks the token's end. A token's start is that time less
/// about its length (taken from the gap to the next token), never before the
/// previous token's end. Without attention times the token's own guesses
/// are used.
pub fn token_times(tokens: &[RawToken]) -> Vec<(u32, u32)> {
    let mut out = Vec::with_capacity(tokens.len());
    for (i, t) in tokens.iter().enumerate() {
        let Some(at) = t.dtw_ms else {
            out.push((t.start_ms, t.end_ms.max(t.start_ms)));
            continue;
        };
        let next = tokens.get(i + 1).and_then(|n| n.dtw_ms).filter(|n| *n > at && n - at <= 2000);
        let prev = if i > 0 { tokens[i - 1].dtw_ms.filter(|p| *p <= at) } else { None };
        let len = next.map_or(TOKEN_MS, |n| (n - at).clamp(TOKEN_MIN_MS, TOKEN_MAX_MS));
        let start = at.saturating_sub((len as f32 * LEAD).round() as u32).max(prev.unwrap_or(0));
        out.push((start, at));
    }
    out
}

/// Tokens to words. A word starts at a token beginning with a space; its
/// bytes are joined first and decoded once, so a character split across
/// tokens comes out whole. `times` are [`token_times`].
fn words_timed(tokens: &[RawToken], times: &[(u32, u32)]) -> Vec<Word> {
    let mut raw: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    for (t, &(start, end)) in tokens.iter().zip(times) {
        let new_word = raw.is_empty() || t.bytes.first() == Some(&b' ');
        match raw.last_mut() {
            Some(w) if !new_word => {
                w.2.extend_from_slice(&t.bytes);
                w.1 = w.1.max(end);
            }
            _ => raw.push((start, end.max(start), t.bytes.clone())),
        }
    }
    raw.into_iter()
        .map(|(start_ms, end_ms, bytes)| Word { start_ms, end_ms, text: String::from_utf8_lossy(&bytes).into_owned() })
        .collect()
}

/// One segment's tokens to words (no neighbours to time against).
pub fn words_from_tokens(tokens: &[RawToken]) -> Vec<Word> {
    words_timed(tokens, &token_times(tokens))
}

/// Every segment's tokens to segments of words. Tokens are timed across
/// segment edges, since one alignment covers each stretch of the song.
pub fn segments_from_tokens(segments: &[Vec<RawToken>]) -> Vec<Segment> {
    let flat: Vec<RawToken> = segments.iter().flatten().cloned().collect();
    let times = token_times(&flat);
    let mut out = Vec::new();
    let mut at = 0;
    for seg in segments {
        let words = words_timed(seg, &times[at..at + seg.len()]);
        at += seg.len();
        if !words.is_empty() {
            let sure = Some(seg.iter().map(|t| t.p).sum::<f32>() / seg.len() as f32);
            out.push(Segment { words, sure });
        }
    }
    out
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
        hints: &Hints,
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
    fn transcribe(&self, _: &Path, _: ModelKind, _: &[f32], _: usize, _: &Hints, _: Stop, _: Progress) -> Result<Vec<Segment>, RecognizeError> {
        Err(RecognizeError::Failed("not in this build".into()))
    }
}

#[cfg(feature = "transcribe")]
pub mod whisper {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};

    use parking_lot::Mutex;
    use whisper_rs::{
        DtwMode, DtwModelPreset, DtwParameters, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
    };

    use super::super::gpu;
    use super::super::lines::Segment;
    use super::super::model::ModelKind;
    use super::{
        DETECT, FALLBACK_CANDIDATES, FALLBACK_STEP, Hints, Progress, RawToken, RecognizeError, Recognizer, Step, Stop,
        detected_language, segments_from_tokens, unavailable_here,
    };

    /// The language is heard from this much of the song at a quarter, half
    /// and three quarters of the way in, and the three listens vote (one
    /// could land on a solo).
    const DETECT_MS: usize = 30_000;

    /// The language to transcribe `pcm` in: asked for, or heard.
    fn language(state: &mut whisper_rs::WhisperState, pcm: &[f32], threads: usize, hints: &Hints, stop: &Stop) -> String {
        if hints.language != DETECT {
            return hints.language.clone();
        }
        let len = DETECT_MS / 1000 * super::super::audio::RATE as usize;
        let mut total: Vec<f32> = Vec::new();
        let mut listens = 0;
        for at in [1, 2, 3] {
            if stop.is_set() {
                break;
            }
            let from = (pcm.len() * at / 4).min(pcm.len().saturating_sub(len));
            let slice = &pcm[from..(from + len).min(pcm.len())];
            let Ok((_, probs)) = state.pcm_to_mel(slice, threads).and_then(|_| state.lang_detect(0, threads)) else { continue };
            total.resize(probs.len(), 0.0);
            total.iter_mut().zip(&probs).for_each(|(t, p)| *t += p);
            listens += 1;
        }
        let best = total.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, p)| (i, p / listens.max(1) as f32));
        let heard = best.and_then(|(id, p)| Some((whisper_rs::get_lang_str(id as i32)?, p)));
        let code = detected_language(heard).to_owned();
        tracing::info!(language = %code, sure = heard.map(|h| h.1), listens, "language heard");
        code
    }

    /// whisper.cpp, keeping the last model loaded between songs.
    #[derive(Default)]
    pub struct Whisper {
        ctx: Mutex<Option<(PathBuf, WhisperContext)>>,
    }

    fn preset(kind: ModelKind) -> DtwModelPreset {
        match kind {
            ModelKind::Small => DtwModelPreset::SmallEn,
            ModelKind::Medium => DtwModelPreset::MediumEn,
            ModelKind::Turbo => DtwModelPreset::LargeV3Turbo,
        }
    }

    /// The graphics card this process may use (Metal on macOS, Vulkan on
    /// Windows and Linux), looked up once.
    fn card() -> Option<&'static gpu::Device> {
        static CARD: OnceLock<Option<gpu::Device>> = OnceLock::new();
        CARD.get_or_init(|| if gpu::allowed() { gpu::pick(&gpu::list_devices()).cloned() } else { None }).as_ref()
    }

    fn load(path: &Path, kind: ModelKind) -> Result<WhisperContext, RecognizeError> {
        // Much faster and cooler than the processor.
        if let Some(card) = card() {
            let api = if cfg!(target_os = "macos") { "Metal" } else { "Vulkan" };
            match load_on(path, kind, Some(card.index)) {
                Ok(ctx) => {
                    tracing::info!("speech model on the graphics card: {} ({api})", card.name);
                    return Ok(ctx);
                }
                Err(e) => tracing::warn!("speech model didn't load on the graphics card ({}), using the processor: {e:?}", card.name),
            }
        }
        tracing::info!("speech model on the processor");
        load_on(path, kind, None)
    }

    /// `card`: the graphics card's index, or None for the processor.
    fn load_on(path: &Path, kind: ModelKind, card: Option<usize>) -> Result<WhisperContext, RecognizeError> {
        let p = WhisperContextParameters {
            use_gpu: card.is_some(),
            gpu_device: card.unwrap_or(0) as i32,
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
            hints: &Hints,
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
            let language = language(&mut state, pcm, threads, hints, &stop);

            // A window the model is unsure of is decoded again hotter; on
            // music that happens often, so it's capped at two more tries with
            // two candidates each (the library's default is five of five).
            let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: FALLBACK_CANDIDATES });
            p.set_temperature_inc(FALLBACK_STEP);
            p.set_n_threads(threads as i32);
            p.set_language(Some(&language));
            if !hints.prompt.is_empty() {
                p.set_initial_prompt(&hints.prompt);
            }
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

            let started = std::time::Instant::now();
            let res = state.full(p, pcm);
            tracing::info!(
                audio_s = pcm.len() / 16_000,
                full_ms = started.elapsed().as_millis() as u64,
                threads,
                "speech model run"
            );
            if cancel.load(Ordering::Relaxed) {
                return Err(RecognizeError::Cancelled);
            }
            res.map_err(|e| RecognizeError::Failed(e.to_string()))?;

            let eot = ctx.token_eot();
            let mut all = Vec::new();
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
                        p: d.p,
                    });
                }
                all.push(tokens);
            }
            Ok(segments_from_tokens(&all))
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
        RawToken { bytes: bytes.to_vec(), start_ms: at, end_ms: at + 100, dtw_ms: None, p: 1.0 }
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
    fn the_attention_time_ends_a_word_when_known() {
        // The attention time is where the token ends; it starts about a token before.
        let words = words_from_tokens(&[RawToken { bytes: b" hey".to_vec(), start_ms: 500, end_ms: 900, dtw_ms: Some(620), p: 1.0 }]);
        assert_eq!((words[0].start_ms, words[0].end_ms), (432, 620));
    }

    #[test]
    fn songs_get_their_title_and_artist_and_the_models_language() {
        let h = Hints::for_song(ModelKind::Turbo, Some(" Golden Hour "), Some("Kacey"));
        assert_eq!(h, Hints { language: DETECT.into(), prompt: "Golden Hour by Kacey.".into() });
        assert_eq!(Hints::for_song(ModelKind::Small, Some("Song"), None).language, LANGUAGE, "English-only models stay English");
        assert_eq!(prompt_for(None, Some("Band")), "Band.");
        assert_eq!(prompt_for(Some(" "), None), "");
        assert_eq!(prompt_for(Some("Hey Jude (Remastered 2015)"), Some("The Beatles")), "Hey Jude by The Beatles.");
        assert_eq!(prompt_for(Some("Song - Live [Bonus]"), None), "Song.");
        assert_eq!(prompt_for(Some("(You Gotta) Fight"), None), "(You Gotta) Fight.", "a title starting with brackets stays");
        assert_eq!(prompt_for(Some("Song"), Some("A\0B")), "Song by A B.", "no NUL reaches the library");
    }

    #[test]
    fn an_unsure_language_falls_back_to_english() {
        assert_eq!(detected_language(Some(("es", 0.9))), "es");
        assert_eq!(detected_language(Some(("cy", 0.3))), "en");
        assert_eq!(detected_language(None), "en");
    }

    #[test]
    fn a_segment_is_as_sure_as_its_tokens() {
        let t = |b: &[u8], p: f32| RawToken { bytes: b.to_vec(), start_ms: 0, end_ms: 100, dtw_ms: None, p };
        let segs = segments_from_tokens(&[vec![t(b" a", 0.9), t(b" b", 0.5)]]);
        assert!((segs[0].sure.unwrap() - 0.7).abs() < 1e-6);
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
