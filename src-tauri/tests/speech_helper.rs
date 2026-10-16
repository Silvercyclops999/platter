//! Starts the real Platter binary as the speech helper and talks to it.
//! Ignored by default (it needs the built binary):
//! `cargo test --features transcribe --test speech_helper -- --ignored`
//! With `PLATTER_WHISPER_MODEL=path` it also transcribes a second of silence.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use platter_lib::lyrics::transcribe::engine::{LANGUAGE, RecognizeError, Recognizer, Stop};
use platter_lib::lyrics::transcribe::helper::{Helper, ProcessLaunch};
use platter_lib::lyrics::transcribe::model::ModelKind;

#[test]
#[ignore]
fn the_real_helper_starts_answers_and_stops() {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_platter"));
    let helper = Helper::new(Arc::new(ProcessLaunch { exe }), Arc::new(|| true), Duration::from_millis(500));
    let never = Stop::new;
    let res = helper.transcribe(Path::new("/nowhere/missing.bin"), ModelKind::Small, &[0.0; 16_000], 2, LANGUAGE, never(), Arc::new(|_| {}));
    assert!(matches!(res, Err(RecognizeError::Failed(_))), "{res:?}");
    assert!(helper.running(), "a failed run keeps the helper");
    if let Ok(model) = std::env::var("PLATTER_WHISPER_MODEL") {
        let res = helper.transcribe(Path::new(&model), ModelKind::Small, &[0.0; 16_000], 2, LANGUAGE, never(), Arc::new(|_| {}));
        assert!(res.is_ok(), "{res:?}");
    }
    std::thread::sleep(Duration::from_secs(3));
    assert!(!helper.running(), "it ends after the idle time");
    assert!(!helper.crashed_too_often());
}
