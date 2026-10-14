//! Crossfade, leveling, EQ and limiter through the whole engine, on a
//! capture output. All audio is generated at test time.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedReceiver;

use super::dsp::{self, Crossfade, DspSettings, Leveling, TrackAudio};
use super::engine::{Engine, EngineConfig, EngineDeps, EngineEvent};
use super::gate::TransferGate;
use super::http::{HttpConfig, http_client};
use super::output::{CaptureHandle, CaptureOutput, OutputFormat};
use super::source::{StreamSource, TrackResolver};
use super::testutil::*;
use crate::error::AppResult;
use crate::model::Track;

const RATE: u32 = 44_100;

/// The fake resolver plus loudness data and a record of saved silence.
struct InfoResolver {
    inner: Arc<FakeResolver>,
    info: Mutex<HashMap<String, TrackAudio>>,
    saved: Mutex<Vec<(String, u64, u64)>>,
}

#[async_trait]
impl TrackResolver for InfoResolver {
    async fn stream(&self, track: &Track) -> AppResult<StreamSource> {
        self.inner.stream(track).await
    }
    async fn transcode(&self, track: &Track) -> AppResult<StreamSource> {
        self.inner.transcode(track).await
    }
    async fn audio_info(&self, track: &Track) -> Option<TrackAudio> {
        self.info.lock().get(&track.id).copied()
    }
    fn save_silence(&self, track_id: &str, head_ms: u64, tail_ms: u64) {
        self.saved.lock().push((track_id.into(), head_ms, tail_ms));
    }
}

struct Rig {
    server: TestServer,
    res: Arc<InfoResolver>,
    engine: Engine,
    events: UnboundedReceiver<EngineEvent>,
    cap: CaptureHandle,
}

async fn rig(paced: bool) -> Rig {
    let server = TestServer::start().await;
    let res = Arc::new(InfoResolver {
        inner: FakeResolver::new(&server),
        info: Mutex::new(HashMap::new()),
        saved: Mutex::new(Vec::new()),
    });
    let (out, cap) = CaptureOutput::new(OutputFormat {
        sample_rate: RATE,
        channels: 2,
    });
    let out = if paced { out.realtime() } else { out };
    let (engine, events) = Engine::new(EngineDeps {
        resolver: res.clone(),
        cache: None,
        client: http_client(),
        rt: tokio::runtime::Handle::current(),
        output: Box::new(out),
        config: EngineConfig {
            buffer_ahead_ms: 2_000,
            ramp_ms: 0,
            declick_ms: 0,
            http: HttpConfig::default(),
        },
        gate: TransferGate::new(),
    });
    Rig {
        server,
        res,
        engine,
        events,
        cap,
    }
}

fn tr(id: &str, album: &str, number: u32, secs: f64) -> Track {
    let mut t = track(id, "Artist", &format!("/library/parts/{id}/file.wav"));
    t.album_id = album.into();
    t.track_number = number;
    t.duration_ms = (secs * 1000.0) as u64;
    t
}

/// A stereo 16-bit WAV from a per-frame function.
fn wav(frames: usize, f: impl Fn(usize) -> (f32, f32)) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cur = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
        for i in 0..frames {
            let (l, r) = f(i);
            w.write_sample((l * i16::MAX as f32) as i16).unwrap();
            w.write_sample((r * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
    }
    cur.into_inner()
}

fn secs(s: f64) -> usize {
    (s * RATE as f64) as usize
}

fn tone(i: usize, amp: f32) -> f32 {
    (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin() * amp
}

async fn until(rx: &mut UnboundedReceiver<EngineEvent>, stop: impl Fn(&EngineEvent) -> bool) -> Vec<EngineEvent> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut out = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(e)) => {
                let done = stop(&e);
                out.push(e);
                if done {
                    return out;
                }
            }
            _ => panic!("timed out; got {out:?}"),
        }
    }
}

fn drained(e: &EngineEvent) -> bool {
    matches!(e, EngineEvent::Drained)
}

/// Loads `a` paused with `b` next, lets both decode, then plays to the end.
async fn play_pair(r: &mut Rig, a: Track, b: Track, s: DspSettings) -> (Vec<EngineEvent>, u64, u64) {
    r.engine.set_dsp(s);
    let key = r.engine.load(a, "ia".into(), 0, false).unwrap();
    let nk = r.engine.set_next(Some((b, "ib".into())), key).unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await;
    r.engine.play().unwrap();
    (until(&mut r.events, drained).await, key, nk)
}

fn add(r: &Rig, id: &str, body: Vec<u8>) {
    r.server.add(&format!("/t/{id}"), Route { body, ..Default::default() });
}

fn fixed(ms: u64) -> DspSettings {
    DspSettings {
        crossfade: Crossfade::Fixed { ms },
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fixed_crossfade_keeps_constant_power() {
    let mut r = rig(false).await;
    // A on the left only, B on the right only: during the fade the
    // left/right levels are the two curves.
    add(&r, "a", wav(secs(2.0), |_| (0.5, 0.0)));
    add(&r, "b", wav(secs(2.0), |_| (0.0, 0.5)));
    let (ev, key, nk) = play_pair(&mut r, tr("a", "x", 1, 2.0), tr("b", "y", 1, 2.0), fixed(1_000)).await;
    let total = secs(3.0);
    let got = settled_samples(&r.cap, total * 2).await;
    assert_eq!(got.len() / 2, total, "a 1 s overlap");
    let lvl = 0.5f32 * 16383.0 / 16384.0;
    for i in secs(1.0)..secs(2.0) {
        let (l, rr) = (got[i * 2], got[i * 2 + 1]);
        let p = l * l + rr * rr;
        assert!((p - lvl * lvl).abs() < 1e-3, "frame {i}: power {p}");
    }
    // Before and after the overlap, each track alone at full level.
    assert!((got[secs(0.5) * 2] - lvl).abs() < 1e-4 && got[secs(0.5) * 2 + 1] == 0.0);
    assert!(got[secs(2.5) * 2] == 0.0 && (got[secs(2.5) * 2 + 1] - lvl).abs() < 1e-4);
    // The outgoing track ends (with its full listened time) as the next
    // starts.
    let order: Vec<String> = ev
        .iter()
        .filter_map(|e| match e {
            EngineEvent::Started { key, .. } => Some(format!("start{key}")),
            EngineEvent::Ended { key, listened_ms, .. } => Some(format!("end{key}:{listened_ms}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        vec![format!("start{key}"), format!("end{key}:2000"), format!("start{nk}"), format!("end{nk}:2000")]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn smart_crossfade_overlaps_the_measured_silence() {
    let mut r = rig(false).await;
    // A: 3 s of tone then 2 s of silence. B: 1.5 s of silence then 3 s.
    add(&r, "a", wav(secs(5.0), |i| if i < secs(3.0) { (tone(i, 0.5), 0.0) } else { (0.0, 0.0) }));
    add(&r, "b", wav(secs(4.5), |i| if i >= secs(1.5) { (0.0, tone(i, 0.5)) } else { (0.0, 0.0) }));
    let s = DspSettings {
        crossfade: Crossfade::Smart,
        ..Default::default()
    };
    play_pair(&mut r, tr("a", "x", 1, 5.0), tr("b", "y", 1, 4.5), s).await;
    let got = settled_samples(&r.cap, secs(6.0) * 2).await;
    // 3.5 s of silence cut: A's tone runs straight into B's.
    let frames = got.len() / 2;
    let win = secs(0.05);
    assert!(frames.abs_diff(secs(6.0)) <= win, "{frames} frames");
    // A's tone is untouched; B's tone follows right after.
    let a_ref: Vec<f32> = (0..secs(3.0)).map(|i| tone(i, 0.5)).collect();
    for i in (0..secs(3.0)).step_by(97) {
        assert!((got[i * 2] - a_ref[i]).abs() < 1e-3, "A at {i}");
    }
    let b_start = (0..frames).find(|&i| got[i * 2 + 1].abs() > 1e-3).unwrap();
    assert!(b_start.abs_diff(secs(3.0)) <= win, "B starts at {b_start}");
    // Both tracks' silence was measured and handed over for saving.
    let saved = r.res.saved.lock().clone();
    let a = saved.iter().find(|s| s.0 == "a").expect("A saved");
    let b = saved.iter().find(|s| s.0 == "b").expect("B saved");
    assert_eq!((a.1, a.2), (0, 2000));
    assert_eq!((b.1, b.2), (1500, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn smart_crossfade_is_capped_at_eight_seconds() {
    let mut r = rig(false).await;
    // 6 s of quiet tail plus 5 s of quiet head: only 8 s overlap.
    add(&r, "a", wav(secs(9.0), |i| if i < secs(3.0) { (tone(i, 0.5), 0.0) } else { (0.0, 0.0) }));
    add(&r, "b", wav(secs(9.0), |i| if i >= secs(5.0) { (0.0, tone(i, 0.5)) } else { (0.0, 0.0) }));
    let s = DspSettings {
        crossfade: Crossfade::Smart,
        ..Default::default()
    };
    play_pair(&mut r, tr("a", "x", 1, 9.0), tr("b", "y", 1, 9.0), s).await;
    let got = settled_samples(&r.cap, secs(10.0) * 2).await;
    assert_eq!(got.len() / 2, secs(10.0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_endings_are_not_faded() {
    let mut r = rig(false).await;
    // A ends at full level; B has a 1 s quiet lead-in.
    let a = wav(secs(2.0), |i| (tone(i, 0.5), tone(i, 0.4)));
    add(&r, "a", a.clone());
    add(&r, "b", wav(secs(3.0), |i| if i >= secs(1.0) { (tone(i, 0.5), 0.0) } else { (0.0, 0.0) }));
    let s = DspSettings {
        crossfade: Crossfade::Smart,
        ..Default::default()
    };
    play_pair(&mut r, tr("a", "x", 1, 2.0), tr("b", "y", 1, 3.0), s).await;
    let got = settled_samples(&r.cap, secs(4.0) * 2).await;
    assert_eq!(got.len() / 2, secs(4.0), "B's lead-in plays under A's end");
    let (ra, _, _) = decode_all(&a, "wav");
    assert_eq!(&got[..ra.len()], &ra[..], "A plays sample for sample");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn album_tracks_in_order_stay_gapless() {
    let mut r = rig(false).await;
    let a = wav(secs(2.0), |i| (tone(i, 0.5), 0.0));
    let b = wav(secs(2.0), |i| (0.0, tone(i, 0.5)));
    add(&r, "a", a.clone());
    add(&r, "b", b.clone());
    play_pair(&mut r, tr("a", "al", 1, 2.0), tr("b", "al", 2, 2.0), fixed(1_000)).await;
    let (mut want, _, _) = decode_all(&a, "wav");
    want.extend(decode_all(&b, "wav").0);
    let got = settled_samples(&r.cap, want.len()).await;
    assert_eq!(got, want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn album_rule_off_crossfades_album_tracks() {
    let mut r = rig(false).await;
    add(&r, "a", wav(secs(2.0), |i| (tone(i, 0.5), 0.0)));
    add(&r, "b", wav(secs(2.0), |i| (0.0, tone(i, 0.5))));
    let s = DspSettings {
        album_gapless: false,
        ..fixed(1_000)
    };
    play_pair(&mut r, tr("a", "al", 1, 2.0), tr("b", "al", 2, 2.0), s).await;
    let got = settled_samples(&r.cap, secs(3.0) * 2).await;
    assert_eq!(got.len() / 2, secs(3.0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_skip_cuts_without_a_fade() {
    let mut r = rig(true).await;
    add(&r, "a", wav(secs(3.0), |i| (tone(i, 0.5), 0.0)));
    let b = wav(secs(1.0), |i| (0.0, tone(i, 0.5) + 0.01));
    add(&r, "b", b.clone());
    r.engine.set_dsp(fixed(2_000));
    r.engine.load(tr("a", "x", 1, 3.0), "ia".into(), 0, true).unwrap();
    while r.cap.len_frames(2) < secs(0.3) {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    r.engine.load(tr("b", "y", 1, 1.0), "ib".into(), 0, true).unwrap();
    until(&mut r.events, drained).await;
    let got = settled_samples(&r.cap, 0).await;
    let first_b = (0..got.len() / 2).find(|&i| got[i * 2 + 1] != 0.0).unwrap();
    let after = &got[first_b * 2..];
    assert!(after.chunks(2).all(|f| f[0] == 0.0), "no A under B after the skip");
    assert_eq!(after, &decode_all(&b, "wav").0[..], "B from its start at full level");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ab_loop_wrap_never_crossfades() {
    let mut r = rig(false).await;
    add(&r, "a", wav(secs(2.0), |i| (tone(i, 0.5), 0.0)));
    add(&r, "b", wav(secs(2.0), |i| (0.0, tone(i, 0.5))));
    r.engine.set_dsp(fixed(1_000));
    let key = r.engine.load(tr("a", "x", 1, 2.0), "ia".into(), 0, false).unwrap();
    assert!(r.engine.set_loop(key, Some((500, 1_900))));
    let nk = r.engine.set_next(Some((tr("b", "y", 1, 2.0), "ib".into())), key).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    r.engine.play().unwrap();
    // Many wraps later B still hasn't started.
    while r.cap.len_frames(2) < secs(8.0) {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    r.engine.pause();
    let got = r.cap.samples();
    assert!(got.chunks(2).all(|f| f[1] == 0.0), "nothing of B is heard");
    while let Ok(e) = r.events.try_recv() {
        if let EngineEvent::Started { key, .. } = e {
            assert_ne!(key, nk);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leveling_uses_album_gain_in_album_order_and_track_gain_otherwise() {
    let mut r = rig(false).await;
    add(&r, "a", wav(secs(1.0), |i| (tone(i, 0.5), tone(i, 0.5))));
    r.res.info.lock().insert(
        "a".into(),
        TrackAudio {
            gain_db: Some(-6.0),
            album_gain_db: Some(-3.0),
            peak: Some(0.5),
            album_peak: Some(0.6),
            ..Default::default()
        },
    );
    let s = DspSettings {
        leveling: Leveling::Smart,
        ..Default::default()
    };
    r.engine.set_dsp(s);
    async fn gain_after_load(r: &Rig) -> f32 {
        r.engine.load(tr("a", "al", 1, 1.0), "ia".into(), 0, false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let g = r.engine.front_gain().unwrap();
            if g != 1.0 || Instant::now() > deadline {
                return g;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    r.engine.set_album_context(Some("al".into()));
    assert!((gain_after_load(&r).await - dsp::db_to_lin(-3.0)).abs() < 1e-6);
    r.engine.set_album_context(None);
    assert!((gain_after_load(&r).await - dsp::db_to_lin(-6.0)).abs() < 1e-6);
    r.engine.set_dsp(DspSettings { leveling: Leveling::Album, ..s });
    assert!((gain_after_load(&r).await - dsp::db_to_lin(-3.0)).abs() < 1e-6);
    // And it's what plays.
    r.engine.set_dsp(DspSettings { leveling: Leveling::Track, ..s });
    r.cap.clear();
    gain_after_load(&r).await;
    r.engine.play().unwrap();
    until(&mut r.events, drained).await;
    let got = settled_samples(&r.cap, secs(1.0) * 2).await;
    let peak = got.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!((peak - 0.5 * dsp::db_to_lin(-6.0)).abs() < 2e-3, "peak {peak}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preamp_eq_boost_and_limiter_stay_under_full_scale() {
    let mut r = rig(false).await;
    add(&r, "a", wav(secs(1.0), |i| (tone(i, 0.9), tone(i, 0.9))));
    r.engine.set_dsp(DspSettings {
        leveling: Leveling::Smart,
        preamp_db: 12.0,
        limiter: true,
        eq: Some([12.0; dsp::EQ_BANDS]),
        ..Default::default()
    });
    r.engine.load(tr("a", "x", 1, 1.0), "ia".into(), 0, true).unwrap();
    until(&mut r.events, drained).await;
    let got = settled_samples(&r.cap, secs(1.0) * 2).await;
    // The limiter's 1.5 ms delay is flushed at the end: nothing is lost.
    assert_eq!(got.len() / 2, secs(1.0) + 66);
    let peak = got.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak <= dsp::LIMIT_CEILING + 1e-4, "peak {peak}");
    assert!(peak > 0.9, "it's loud, just not clipping ({peak})");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn setting_changes_ramp_without_clicks() {
    let mut r = rig(true).await;
    let amp = 0.2;
    add(&r, "a", wav(secs(3.0), |i| (tone(i, amp), tone(i, amp))));
    r.engine.set_dsp(DspSettings {
        leveling: Leveling::Smart,
        ..Default::default()
    });
    r.engine.load(tr("a", "x", 1, 3.0), "ia".into(), 0, true).unwrap();
    while r.cap.len_frames(2) < secs(0.8) {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    r.engine.set_dsp(DspSettings {
        leveling: Leveling::Smart,
        preamp_db: 6.0,
        eq: Some([0.0, 0.0, 0.0, 0.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        limiter: true,
        ..Default::default()
    });
    until(&mut r.events, drained).await;
    let got = settled_samples(&r.cap, secs(3.0) * 2).await;
    // A 440 Hz tone moves at most this much per sample at the final level
    // (+6 dB preamp, +6 dB at 500 Hz: under 4x).
    let slope = 2.0 * std::f32::consts::PI * 440.0 / RATE as f32 * amp * 4.0;
    let mut worst = 0.0f32;
    for w in got.chunks(2).collect::<Vec<_>>().windows(2) {
        worst = worst.max((w[1][0] - w[0][0]).abs());
    }
    assert!(worst < slope * 1.2, "a jump of {worst} (limit {slope})");
    // The new level did arrive.
    let end = &got[got.len() - secs(0.2) * 2..];
    let peak = end.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > amp * 2.5, "peak {peak}");
}
