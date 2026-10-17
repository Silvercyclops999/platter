//! Headphone correction and bit-perfect output: the filters on their own,
//! and through the whole engine on a capture output with fake devices.

use std::io::Cursor;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedReceiver;

use super::dsp::{self, Chain, Correction, CorrectionStage, DspSettings, FilterKind, Leveling, PeqFilter};
use super::engine::{Engine, EngineConfig, EngineDeps, EngineEvent};
use super::gate::TransferGate;
use super::http::{HttpConfig, http_client};
use super::output::{CaptureHandle, CaptureOutput, Exclusive, OutputDevice, OutputFormat};
use super::testutil::*;
use crate::model::Track;

const RATE: u32 = 44_100;

fn pk(freq: f32, gain_db: f32, q: f32) -> PeqFilter {
    PeqFilter { kind: FilterKind::Peaking, freq, gain_db, q }
}

// ------------------------------------------------------------ the filters

#[test]
fn peaking_filters_hit_their_gain_at_the_centre() {
    let c = Correction::new(0.0, &[pk(1000.0, 6.0, 1.41)]);
    assert!((c.response_db(48_000, 1000.0) - 6.0).abs() < 0.01);
    // An octave-ish away it's much less, and far away nothing.
    assert!(c.response_db(48_000, 4000.0) < 1.0);
    assert!(c.response_db(48_000, 50.0).abs() < 0.05);
    // A cut is the mirror image.
    let c = Correction::new(0.0, &[pk(1000.0, -6.0, 1.41)]);
    assert!((c.response_db(48_000, 1000.0) + 6.0).abs() < 0.01);
}

#[test]
fn shelves_reach_their_gain_past_the_corner() {
    let low = Correction::new(0.0, &[PeqFilter { kind: FilterKind::LowShelf, freq: 105.0, gain_db: 6.0, q: 0.7 }]);
    assert!((low.response_db(48_000, 20.0) - 6.0).abs() < 0.5);
    assert!((low.response_db(48_000, 105.0) - 3.0).abs() < 0.2, "half the gain at the corner");
    assert!(low.response_db(48_000, 5000.0).abs() < 0.1);
    let high = Correction::new(0.0, &[PeqFilter { kind: FilterKind::HighShelf, freq: 10_000.0, gain_db: -4.0, q: 0.7 }]);
    assert!((high.response_db(48_000, 20_000.0) + 4.0).abs() < 0.6);
    assert!((high.response_db(48_000, 10_000.0) + 2.0).abs() < 0.2);
    assert!(high.response_db(48_000, 200.0).abs() < 0.05);
}

#[test]
fn the_preamp_is_honoured_and_filters_past_nyquist_are_left_out() {
    let c = Correction::new(-6.0, &[pk(3000.0, 6.0, 1.0), pk(30_000.0, 9.0, 1.0)]);
    assert!((c.response_db(44_100, 3000.0)).abs() < 0.05);
    assert!((c.response_db(44_100, 100.0) + 6.0).abs() < 0.1);
    assert!(c.max_gain(44_100) <= 1.07);
    // A positive preamp would clip; it's held at 0 dB.
    assert_eq!(Correction::new(4.0, &[]).preamp_db, 0.0);
    // Too many filters: the extra ones go.
    let many = vec![pk(1000.0, 1.0, 1.0); dsp::CORRECTION_MAX + 5];
    assert_eq!(Correction::new(0.0, &many).filters().len(), dsp::CORRECTION_MAX);
}

/// The stage itself, on a steady sine: the measured level matches the curve.
#[test]
fn the_stage_filters_audio_like_its_curve() {
    let c = Correction::new(-3.0, &[pk(1000.0, 6.0, 1.0)]);
    let rate = 48_000u32;
    for (freq, want) in [(1000.0f64, 3.0f64), (100.0, -3.0)] {
        let mut st = CorrectionStage::new(rate, 1);
        st.set(Some(c));
        let mut peak = 0.0f32;
        for i in 0..rate as usize {
            let mut f = [(2.0 * std::f64::consts::PI * freq * i as f64 / rate as f64).sin() as f32 * 0.25];
            st.process(&mut f);
            if i > rate as usize / 2 {
                peak = peak.max(f[0].abs());
            }
        }
        let got = 20.0 * (peak as f64 / 0.25).log10();
        assert!((got - want).abs() < 0.2, "{freq} Hz: {got} dB, want {want}");
    }
}

#[test]
fn no_correction_changes_nothing() {
    let mut st = CorrectionStage::new(48_000, 2);
    let mut f = [0.123f32, -0.456];
    st.process(&mut f);
    assert_eq!(f, [0.123, -0.456]);
    let mut chain = Chain::new(48_000, 2);
    chain.set_correction(None, false);
    let mut f = [0.123f32, -0.456];
    chain.process(&mut f, true);
    assert_eq!(f, [0.123, -0.456]);
}

#[test]
fn a_live_correction_change_swaps_at_the_bottom_of_a_dip() {
    let c = Correction::new(-6.0, &[pk(1000.0, 6.0, 1.0)]);
    let mut chain = Chain::new(48_000, 2);
    chain.set_correction(Some(c), true);
    assert_eq!(chain.correction(), Some(c));
    // The level dips to nothing and comes back within a few hundred frames.
    let mut lowest = f32::MAX;
    for _ in 0..2000 {
        let mut f = [0.5f32, 0.5];
        chain.process(&mut f, true);
        lowest = lowest.min(f[0].abs());
    }
    assert!(lowest < 0.01, "dipped to {lowest}");
    let mut f = [0.5f32, 0.5];
    chain.process(&mut f, true);
    // Now through the -6 dB preamp (DC passes the 1 kHz peak untouched).
    assert!((f[0] - 0.5 * dsp::db_to_lin(-6.0)).abs() < 0.01, "{}", f[0]);
}

// ------------------------------------------------------------ the engine

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
    cap.set_devices(vec![
        OutputDevice { id: "spk".into(), name: "Speakers".into(), is_default: true },
        OutputDevice { id: "dac".into(), name: "USB DAC".into(), is_default: false },
    ]);
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

/// 16-bit stereo at `rate`, values k / 32768 exactly once decoded.
fn wav(frames: usize, rate: u32, f: impl Fn(usize) -> i16) -> Vec<u8> {
    let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut cur = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
        for i in 0..frames {
            w.write_sample(f(i)).unwrap();
            w.write_sample(f(i).wrapping_neg()).unwrap();
        }
        w.finalize().unwrap();
    }
    cur.into_inner()
}

fn sample(i: usize) -> i16 {
    ((2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin() * 20_000.0) as i16
}

fn add(r: &Rig, id: &str, rate: u32, frames: usize) -> Track {
    r.server.add(&format!("/t/{id}"), Route { body: wav(frames, rate, sample), ..Default::default() });
    let mut t = track(id, "Artist", &format!("/library/parts/{id}/file.wav"));
    t.sample_rate = Some(rate);
    t.duration_ms = frames as u64 * 1000 / rate as u64;
    t
}

async fn until(rx: &mut UnboundedReceiver<EngineEvent>, stop: impl Fn(&EngineEvent) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(e)) if stop(&e) => return,
            Ok(Some(_)) => {}
            _ => panic!("timed out"),
        }
    }
}

fn drained(e: &EngineEvent) -> bool {
    matches!(e, EngineEvent::Drained)
}

/// Every sound setting on, loud, plus a volume well under full.
fn everything() -> DspSettings {
    DspSettings {
        leveling: Leveling::Track,
        preamp_db: 6.0,
        limiter: true,
        eq: Some([6.0; dsp::EQ_BANDS]),
        correction: Some(Correction::new(-6.0, &[pk(3000.0, 6.0, 1.0)])),
        ..Default::default()
    }
}

async fn play(r: &mut Rig, t: Track) -> Vec<f32> {
    r.cap.clear();
    r.engine.load(t, "i".into(), 0, true).unwrap();
    until(&mut r.events, drained).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    r.cap.samples()
}

fn peak(s: &[f32]) -> f32 {
    s.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bit_perfect_sends_the_samples_untouched() {
    let mut r = rig().await;
    let t = add(&r, "a", RATE, 22_050);
    r.engine.set_dsp(everything());
    r.engine.set_gain_now(0.3);
    r.engine.set_exclusive_devices(vec!["spk".into()]).unwrap();
    let got = play(&mut r, t).await;
    assert!(r.engine.bit_perfect());
    assert_eq!(r.engine.output_state().exclusive, Exclusive::On { rate: RATE, bits: 32 });
    let want: Vec<f32> = (0..22_050).flat_map(|i| [sample(i) as f32 / 32_768.0, sample(i).wrapping_neg() as f32 / 32_768.0]).collect();
    assert_eq!(got.len(), want.len());
    assert!(got == want, "samples changed on the way out");
    // The user's settings are kept for when it's off again.
    assert_eq!(r.engine.dsp(), DspSettings::untouched());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bit_perfect_still_mutes() {
    let mut r = rig().await;
    let t = add(&r, "a", RATE, 4_410);
    r.engine.set_exclusive_devices(vec!["spk".into()]).unwrap();
    r.engine.set_gain_now(0.0);
    let got = play(&mut r, t).await;
    assert_eq!(peak(&got), 0.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turning_bit_perfect_off_brings_the_sound_settings_back() {
    let mut r = rig().await;
    let t = add(&r, "a", RATE, 8_820);
    r.engine.set_dsp(DspSettings { preamp_db: -6.0, ..Default::default() });
    r.engine.set_exclusive_devices(vec!["spk".into()]).unwrap();
    let on = play(&mut r, t.clone()).await;
    r.engine.set_exclusive_devices(Vec::new()).unwrap();
    assert!(!r.engine.bit_perfect());
    assert_eq!(r.engine.output_state().exclusive, Exclusive::Off);
    let off = play(&mut r, t).await;
    let ratio = peak(&off) / peak(&on);
    assert!((ratio - dsp::db_to_lin(-6.0)).abs() < 0.01, "ratio {ratio}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_chosen_device_plays_bit_perfect() {
    let mut r = rig().await;
    let t = add(&r, "a", RATE, 4_410);
    r.engine.set_dsp(DspSettings { preamp_db: -6.0, ..Default::default() });
    r.engine.set_exclusive_devices(vec!["dac".into()]).unwrap();
    let spk = play(&mut r, t.clone()).await;
    assert!(!r.engine.bit_perfect());
    // Moving to the DAC reopens it exclusively.
    r.engine.select_output(Some("dac".into())).unwrap();
    let dac = play(&mut r, t).await;
    assert!(r.engine.bit_perfect());
    assert_eq!(r.cap.opened_id().as_deref(), Some("dac"));
    assert!(peak(&dac) > peak(&spk) * 1.9, "preamp bypassed on the DAC");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_busy_device_falls_back_to_shared_with_a_reason() {
    let mut r = rig().await;
    let t = add(&r, "a", RATE, 4_410);
    r.cap.fail_exclusive(Some("Another app is using the device on its own."));
    r.engine.set_dsp(DspSettings { preamp_db: -6.0, ..Default::default() });
    r.engine.set_exclusive_devices(vec!["spk".into()]).unwrap();
    let got = play(&mut r, t).await;
    assert!(!r.engine.bit_perfect());
    assert_eq!(
        r.engine.output_state().exclusive,
        Exclusive::Fallback { reason: "Another app is using the device on its own.".into() }
    );
    // Shared mode keeps the sound settings.
    assert!(peak(&got) < 0.62 * 0.55, "peak {}", peak(&got));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bit_perfect_opens_at_each_songs_own_rate() {
    let mut r = rig().await;
    let a = add(&r, "a", 96_000, 9_600);
    let b = add(&r, "b", RATE, 4_410);
    r.engine.set_exclusive_devices(vec!["spk".into()]).unwrap();
    play(&mut r, a.clone()).await;
    assert_eq!(r.engine.format().sample_rate, 96_000);
    assert_eq!(r.engine.output_state().exclusive, Exclusive::On { rate: 96_000, bits: 32 });
    play(&mut r, b.clone()).await;
    assert_eq!(r.engine.format().sample_rate, RATE);
    // A next song at another rate isn't joined gaplessly; it loads on its own.
    let key = r.engine.load(a, "ia".into(), 0, false).unwrap();
    assert_eq!(r.engine.set_next(Some((b, "ib".into())), key), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn correction_applies_and_ab_bypasses_it() {
    let mut r = rig().await;
    let t = add(&r, "a", RATE, 8_820);
    let with = DspSettings { correction: Some(Correction::new(-6.0, &[pk(3000.0, 6.0, 1.0)])), ..Default::default() };
    let plain = play(&mut r, t.clone()).await;
    r.engine.set_dsp(with);
    let corrected = play(&mut r, t.clone()).await;
    // 440 Hz is far below the peak: it gets the -6 dB preamp, give or take.
    let ratio = peak(&corrected) / peak(&plain);
    assert!((20.0 * ratio.log10() + 6.0).abs() < 0.8, "{} dB", 20.0 * ratio.log10());
    r.engine.set_correction_bypass(true);
    assert!(r.engine.correction_bypassed());
    assert_eq!(r.engine.dsp().correction, None);
    let bypassed = play(&mut r, t.clone()).await;
    assert!((peak(&bypassed) - peak(&plain)).abs() < 1e-3);
    r.engine.set_correction_bypass(false);
    assert_eq!(r.engine.dsp(), with);
}
