//! Starts the real Platter binary as the speech helper and talks to it.
//! Ignored by default (it needs the built binary):
//! `cargo test --features transcribe --test speech_helper -- --ignored`
//! With `PLATTER_WHISPER_MODEL=path` it also transcribes a second of silence.
//! `the_real_helper_on_the_graphics_card` is the one to run on a computer
//! with a graphics card (CI has none).

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

/// Lists the cards with the real probe, then (with `PLATTER_WHISPER_MODEL`,
/// best the turbo model) times half a minute of audio on the card and on
/// the processor:
/// `PLATTER_WHISPER_MODEL=path cargo test --release --features transcribe
/// --test speech_helper graphics -- --ignored --nocapture`
#[test]
#[ignore]
fn the_real_helper_on_the_graphics_card() {
    use platter_lib::lyrics::transcribe::gpu::{Gpu, GpuMode, pick, run_probe};
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_platter"));
    let cards = run_probe(&exe);
    println!("cards: {cards:#?}");
    println!("picked: {:?}", pick(&cards).map(|c| &c.name));
    let Ok(model) = std::env::var("PLATTER_WHISPER_MODEL") else { return };
    let kind = if model.contains("turbo") {
        ModelKind::Turbo
    } else if model.contains("medium") {
        ModelKind::Medium
    } else {
        ModelKind::Small
    };
    let probe_exe = exe.clone();
    let gpu = Gpu::new(Arc::new(move || run_probe(&probe_exe)));
    gpu.wait(Duration::from_secs(30));
    // A tone with a little noise, so the model has something to chew on.
    let pcm: Vec<f32> = (0..16_000 * 30).map(|i| ((i as f32 * 0.07).sin() * 0.1) + ((i * 7919 % 101) as f32 / 1000.0)).collect();
    for mode in [GpuMode::Auto, GpuMode::Off] {
        gpu.set_mode(mode);
        let helper = Helper::new(Arc::new(ProcessLaunch { exe: exe.clone() }), Arc::new(|| true), Duration::from_secs(5)).with_gpu(gpu.clone());
        let started = std::time::Instant::now();
        let res = helper.transcribe(Path::new(&model), kind, &pcm, 2, LANGUAGE, Stop::new(), Arc::new(|_| {}));
        println!("{:?} on {:?}: {:.1} s, {:?}", kind, gpu.active().map(|c| c.name), started.elapsed().as_secs_f32(), res.map(|s| s.len()));
    }
}
