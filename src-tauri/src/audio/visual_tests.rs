//! The visualizer's tap through the whole engine, on a capture output: it
//! only runs while watched, reads after the sound settings and before the
//! volume, and never changes what goes out (bit-perfect included).

use std::io::Cursor;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedReceiver;

use super::dsp::{self, DspSettings, Leveling};
use super::engine::{Engine, EngineConfig, EngineDeps, EngineEvent};
use super::gate::TransferGate;
use super::http::{HttpConfig, http_client};
use super::output::{CaptureHandle, CaptureOutput, OutputDevice, OutputFormat};
use super::testutil::*;
use super::visual::{BANDS, Spectrum, band_edge};
use crate::model::Track;

const RATE: u32 = 48_000;

struct Rig {
    server: TestServer,
    engine: Engine,
    events: UnboundedReceiver<EngineEvent>,
    cap: CaptureHandle,
}

async fn rig() -> Rig {
    let server = TestServer::start().await;
    let res = FakeResolver::new(&server);
    let (out, cap) = CaptureOutput::new(OutputFormat { sample_rate: RATE, channels: 2 });
    cap.set_devices(vec![OutputDevice { id: "spk".into(), name: "Speakers".into(), is_default: true }]);
    let (engine, events) = Engine::new(EngineDeps {
        resolver: res,
        cache: None,
        local: None,
        client: http_client(),
        rt: tokio::runtime::Handle::current(),
        output: Box::new(out),
        config: EngineConfig { buffer_ahead_ms: 2_000, ramp_ms: 0, declick_ms: 0, http: HttpConfig::default(), ..Default::default() },
        gate: TransferGate::new(),
    });
    Rig { server, engine, events, cap }
}

/// A 1 kHz tone at half scale, the right channel inverted (a wide mix).
fn sample(i: usize) -> i16 {
    ((2.0 * std::f32::consts::PI * 1000.0 * i as f32 / RATE as f32).sin() * 16_384.0) as i16
}

fn wav(frames: usize) -> Vec<u8> {
    let spec = hound::WavSpec { channels: 2, sample_rate: RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut cur = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
        for i in 0..frames {
            w.write_sample(sample(i)).unwrap();
            w.write_sample(sample(i).wrapping_neg()).unwrap();
        }
        w.finalize().unwrap();
    }
    cur.into_inner()
}

fn add(r: &Rig, id: &str, frames: usize) -> Track {
    r.server.add(&format!("/t/{id}"), Route { body: wav(frames), ..Default::default() });
    let mut t = track(id, "Artist", &format!("/library/parts/{id}/file.wav"));
    t.sample_rate = Some(RATE);
    t.duration_ms = frames as u64 * 1000 / RATE as u64;
    t
}

/// Plays `t` to its end; the samples that went out and the visualizer
/// frames that came.
async fn play(r: &mut Rig, t: Track) -> (Vec<f32>, Vec<Spectrum>) {
    r.cap.clear();
    r.engine.load(t, "i".into(), 0, true).unwrap();
    let mut frames = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, r.events.recv()).await {
            Ok(Some(EngineEvent::Drained)) => break,
            Ok(Some(EngineEvent::Spectrum(s))) => frames.push(*s),
            Ok(Some(_)) => {}
            _ => panic!("timed out"),
        }
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    while let Ok(e) = r.events.try_recv() {
        if let EngineEvent::Spectrum(s) = e {
            frames.push(*s);
        }
    }
    (r.cap.samples(), frames)
}

/// The frames that show the song (not the silence around it).
fn playing(frames: &[Spectrum]) -> Vec<&Spectrum> {
    frames.iter().filter(|f| f.peak[0] > -10.0).collect()
}

fn band_of(freq: f32) -> usize {
    (0..BANDS).find(|&k| freq >= band_edge(k) && freq < band_edge(k + 1)).unwrap()
}

fn loud_settings() -> DspSettings {
    DspSettings {
        leveling: Leveling::Off,
        preamp_db: -3.0,
        limiter: true,
        eq: Some([2.0; dsp::EQ_BANDS]),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_is_sent_while_nobody_watches() {
    let mut r = rig().await;
    let t = add(&r, "a", RATE as usize / 2);
    let (_, frames) = play(&mut r, t).await;
    assert!(frames.is_empty(), "{} frames", frames.len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watched_song_sends_frames_at_the_asked_rate() {
    let mut r = rig().await;
    let t = add(&r, "a", RATE as usize);
    r.engine.visualizer().watch(true, 30, true);
    let (_, frames) = play(&mut r, t).await;
    // A second of audio at 30 a second (the capture pulls faster than real
    // time, and the watch only counts audio).
    let song = playing(&frames);
    assert!((28..=32).contains(&song.len()), "{} frames", song.len());
    let last = song[song.len() / 2];
    let k = band_of(1000.0);
    let loudest = (0..BANDS).max_by(|&a, &b| last.bands[a].total_cmp(&last.bands[b])).unwrap();
    assert_eq!(loudest, k);
    // Half scale: about -6 dB.
    assert!((-8.0..=-5.5).contains(&last.bands[k]), "{}", last.bands[k]);
    // The wide mix spreads out on the scope.
    let scope = last.scope.as_ref().unwrap();
    assert!(scope.chunks(2).any(|p| (p[0] - p[1]).abs() > 0.5));
    // The clock moves on with the audio.
    assert!(frames.windows(2).all(|w| w[1].t > w[0].t));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_picture_keeps_its_size_at_any_volume() {
    let mut r = rig().await;
    let a = add(&r, "a", RATE as usize / 2);
    let b = add(&r, "b", RATE as usize / 2);
    r.engine.visualizer().watch(true, 30, false);
    r.engine.set_gain_now(1.0);
    let (_, full) = play(&mut r, a).await;
    r.engine.visualizer().watch(true, 30, false);
    r.engine.set_gain_now(0.05);
    let (quiet_out, quiet) = play(&mut r, b).await;
    let k = band_of(1000.0);
    let (full, quiet) = (playing(&full), playing(&quiet));
    let (f, q) = (full[full.len() / 2], quiet[quiet.len() / 2]);
    assert!((f.bands[k] - q.bands[k]).abs() < 0.01, "{} vs {}", f.bands[k], q.bands[k]);
    assert!((f.peak[0] - q.peak[0]).abs() < 0.01);
    // While what went out really was quiet.
    let out_peak = quiet_out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(out_peak < 0.03, "{out_peak}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_tap_never_changes_the_output() {
    // Two fresh engines, so nothing but the watch differs.
    let mut outs = Vec::new();
    for watched in [false, true] {
        let mut r = rig().await;
        let t = add(&r, "a", RATE as usize / 2);
        r.engine.set_dsp(loud_settings());
        r.engine.set_gain_now(0.6);
        if watched {
            r.engine.visualizer().watch(true, 60, true);
        }
        let (out, frames) = play(&mut r, t).await;
        assert_eq!(frames.is_empty(), !watched);
        outs.push(out);
    }
    assert_eq!(outs[0].len(), outs[1].len());
    assert!(outs[0] == outs[1], "the watched song came out different");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bit_perfect_output_is_untouched_and_still_drawn() {
    let mut r = rig().await;
    let t = add(&r, "a", RATE as usize / 2);
    r.engine.set_dsp(loud_settings());
    r.engine.set_gain_now(0.3);
    r.engine.set_exclusive_devices(vec!["spk".into()]).unwrap();
    r.engine.visualizer().watch(true, 60, true);
    let (got, frames) = play(&mut r, t).await;
    assert!(r.engine.bit_perfect());
    let want: Vec<f32> =
        (0..RATE as usize / 2).flat_map(|i| [sample(i) as f32 / 32_768.0, sample(i).wrapping_neg() as f32 / 32_768.0]).collect();
    assert_eq!(got.len(), want.len());
    assert!(got == want, "samples changed on the way out");
    let song = playing(&frames);
    assert!(song.len() >= 25, "{}", song.len());
    let k = band_of(1000.0);
    assert!(song[song.len() / 2].bands[k] > -8.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_watch_sends_nothing() {
    let mut r = rig().await;
    let t = add(&r, "a", RATE as usize / 2);
    let vis = r.engine.visualizer();
    vis.set_watch_for(Duration::from_millis(1));
    vis.watch(true, 60, false);
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!vis.active());
    let (_, frames) = play(&mut r, t).await;
    assert!(frames.is_empty(), "{} frames", frames.len());
}
