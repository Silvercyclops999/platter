//! Manual smoke test for the real output device. Ignored by default:
//!
//! ```sh
//! cd src-tauri && cargo test --lib audio::smoke -- --ignored --nocapture
//! ```
//!
//! Plays 5 s of a quiet 440 Hz tone through the system default device.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use super::output::{AudioOutput, CpalOutput, OutputEvent, OutputFormat, Renderer};

struct Tone {
    fmt: Mutex<Option<OutputFormat>>,
    frame: AtomicU64,
}

impl Renderer for Tone {
    fn render(&self, out: &mut [f32]) -> usize {
        let Some(fmt) = *self.fmt.lock() else {
            out.fill(0.0);
            return 0;
        };
        let ch = fmt.channels as usize;
        let frames = out.len() / ch;
        let start = self.frame.fetch_add(frames as u64, Ordering::Relaxed);
        for (i, f) in out.chunks_exact_mut(ch).enumerate() {
            let t = (start + i as u64) as f64 / fmt.sample_rate as f64;
            let s = ((2.0 * std::f64::consts::PI * 440.0 * t).sin() * 0.1) as f32;
            f.fill(s);
        }
        frames
    }
}

/// Full pipeline: local HTTP → decode → resample/mix → cpal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a real audio device; run manually"]
async fn engine_plays_two_generated_tracks_gaplessly_on_the_default_device() {
    use super::engine::{Engine, EngineConfig, EngineDeps, EngineEvent};
    use super::testutil::*;
    let server = TestServer::start().await;
    let resolver = FakeResolver::new(&server);
    let rate = 44_100;
    server.add(
        "/t/a",
        Route {
            body: wav_tone_from(rate, 2, 0, rate as usize * 2, 330.0, 0.1),
            ..Default::default()
        },
    );
    server.add(
        "/t/b",
        Route {
            body: wav_tone_from(rate, 2, rate as usize * 2, rate as usize * 2, 330.0, 0.1),
            ..Default::default()
        },
    );
    let (engine, mut rx) = Engine::new(EngineDeps {
        resolver,
        cache: None,
        client: super::http::http_client(),
        rt: tokio::runtime::Handle::current(),
        output: Box::new(CpalOutput::new()),
        config: EngineConfig::default(),
        gate: super::gate::TransferGate::new(),
    });
    let mut a = track("a", "x", "/a");
    a.duration_ms = 2000;
    let mut b = track("b", "x", "/b");
    b.duration_ms = 2000;
    let key = engine.load(a, "a".into(), 0, true).expect("load");
    engine.set_next(Some((b, "b".into())), key);
    println!("output format: {:?}", engine.format());
    let started = std::time::Instant::now();
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(EngineEvent::Drained)) => break,
            Ok(Some(e)) => println!("{:>6} ms  {e:?}", started.elapsed().as_millis()),
            other => panic!("unexpected: {other:?}"),
        }
    }
    let secs = started.elapsed().as_secs_f64();
    println!("played 4 s of audio in {secs:.2} s");
    assert!((3.8..5.0).contains(&secs));
}

#[test]
#[ignore = "needs a real audio device; run manually"]
fn plays_five_seconds_on_the_default_device() {
    let tone = Arc::new(Tone {
        fmt: Mutex::new(None),
        frame: AtomicU64::new(0),
    });
    let mut out = CpalOutput::new();
    let fmt = out
        .open(
            tone.clone(),
            Arc::new(|e: OutputEvent| eprintln!("device event: {e:?}")),
        )
        .expect("open default output device");
    *tone.fmt.lock() = Some(fmt);
    let dev = out.last_device.lock().clone();
    println!(
        "opened {:?}: {} Hz, {} ch, sample format {:?}",
        dev.as_ref().map(|d| &d.0),
        fmt.sample_rate,
        fmt.channels,
        dev.as_ref().map(|d| &d.1)
    );
    std::thread::sleep(Duration::from_secs(5));
    let frames = tone.frame.load(Ordering::Relaxed);
    out.close();
    println!(
        "rendered {frames} frames ({:.2} s)",
        frames as f64 / fmt.sample_rate as f64
    );
    assert!(
        frames as f64 > fmt.sample_rate as f64 * 4.0,
        "device pulled too little audio"
    );
}
