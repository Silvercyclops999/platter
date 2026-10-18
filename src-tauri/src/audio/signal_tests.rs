//! Signal path: the snapshot for each kind of playback, what the engine
//! records when a decoder and the output open, and the event the player
//! sends when the path changes.

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedReceiver;

use super::dsp::{self, Correction, Crossfade, DspSettings, Leveling, TrackAudio};
use super::engine::{Engine, EngineConfig, EngineDeps, EngineEvent};
use super::events::{ErrorPayload, EventSink, PlayerState, PositionPayload};
use super::gate::TransferGate;
use super::http::{HttpConfig, http_client};
use super::output::{CaptureHandle, CaptureOutput, Exclusive, OutputDevice, OutputFormat, OutputState};
use super::player::{PlayerConfig, PlayerHandle};
use super::signal::{self, Inputs, SignalPath, SourceFrom, SourceInfo, VerdictKind};
use super::source::{ContextKind, PlayContext};
use super::testutil::*;
use crate::model::Track;

fn dev(id: &str, name: &str, default: bool) -> OutputDevice {
    OutputDevice { id: id.into(), name: name.into(), is_default: default }
}

fn out(exclusive: Exclusive) -> OutputState {
    let mut s = OutputState::build(vec![dev("spk", "Speakers", true), dev("dac", "USB DAC", false)], Some("dac".into()), None);
    s.exclusive = exclusive;
    s
}

fn flac(bits: u32, rate: u32) -> Track {
    let mut t = track("a", "Artist", "/p/a");
    t.codec = Some("flac".into());
    t.container = Some("flac".into());
    t.bit_depth = Some(bits);
    t.sample_rate = Some(rate);
    t.bitrate = Some(2_900);
    t
}

fn source(rate: u32) -> SourceInfo {
    SourceInfo {
        from: SourceFrom::Server,
        transcoded: false,
        fallback: false,
        codec: Some("flac".into()),
        rate,
        channels: 2,
        audio: None,
    }
}

fn fmt(rate: u32) -> Option<OutputFormat> {
    Some(OutputFormat { sample_rate: rate, channels: 2 })
}

fn inputs<'a>(t: &'a Track, s: &'a SourceInfo, o: &'a OutputState, format: Option<OutputFormat>) -> Inputs<'a> {
    Inputs {
        track: Some(t),
        source: Some(s),
        dsp: DspSettings { leveling: Leveling::Off, ..Default::default() },
        album_in_order: false,
        output: Some(o),
        format,
        volume: 1.0,
        muted: false,
        automix: false,
    }
}

// ---------------------------------------------------------------- snapshots

#[test]
fn lossless_original_in_shared_mode_is_resampled_to_the_device_rate() {
    let t = flac(24, 96_000);
    let s = source(96_000);
    let o = out(Exclusive::Off);
    let p = signal::build(&inputs(&t, &s, &o, fmt(48_000)));
    assert_eq!(p.verdict.kind, VerdictKind::Lossless);
    assert_eq!(p.verdict.text, "Lossless, resampled to 48 kHz");
    let src = p.source.as_ref().unwrap();
    assert_eq!((src.codec.as_deref(), src.bit_depth, src.sample_rate), (Some("FLAC"), Some(24), Some(96_000)));
    assert!(src.lossless && !src.transcoded);
    assert_eq!(src.from, Some(SourceFrom::Server));
    let r = p.resample.as_ref().unwrap();
    assert_eq!((r.from_rate, r.to_rate), (96_000, 48_000));
    assert_eq!(p.output.mode, "shared");
    assert_eq!(p.output.device.as_deref(), Some("USB DAC"));
    assert!(!p.output.volume_bypassed);
    // The fix is bit-perfect output for this device.
    assert_eq!(p.why.as_ref().unwrap().setting, Some("bitPerfectOutputs"));
    // 44.1 kHz family reads with its decimal.
    let p = signal::build(&inputs(&t, &s, &o, fmt(44_100)));
    assert_eq!(p.verdict.text, "Lossless, resampled to 44.1 kHz");
}

#[test]
fn exclusive_at_the_songs_own_rate_is_bit_perfect_and_bypasses_everything() {
    let t = flac(24, 96_000);
    let s = source(96_000);
    let o = out(Exclusive::On { rate: 96_000, bits: 24 });
    let mut i = inputs(&t, &s, &o, fmt(96_000));
    i.dsp = DspSettings {
        leveling: Leveling::Track,
        preamp_db: 3.0,
        eq: Some([2.0; dsp::EQ_BANDS]),
        correction: Some(Correction::new(-6.0, &[])),
        crossfade: Crossfade::Fixed { ms: 6_000 },
        ..Default::default()
    };
    i.volume = 0.4;
    i.automix = true;
    let p = signal::build(&i);
    assert_eq!(p.verdict.kind, VerdictKind::BitPerfect);
    assert_eq!(p.verdict.text, "Bit-perfect");
    assert!(p.why.is_none());
    assert!(p.resample.is_none());
    assert!(!p.loudness.on && !p.eq.eq && !p.eq.correction);
    assert_eq!(p.eq.preamp_db, 0.0);
    assert_eq!(p.blend.crossfade, "off");
    assert!(!p.blend.automix);
    assert_eq!((p.output.mode, p.output.bits, p.output.rate), ("exclusive", Some(24), Some(96_000)));
    assert!(p.output.volume_bypassed);
}

#[test]
fn a_transcoded_stream_is_lossy_and_points_at_streaming_quality() {
    let t = flac(16, 44_100);
    let s = SourceInfo { transcoded: true, codec: Some("mp3".into()), ..source(44_100) };
    let o = out(Exclusive::Off);
    let p = signal::build(&inputs(&t, &s, &o, fmt(44_100)));
    assert_eq!(p.verdict.kind, VerdictKind::Lossy);
    assert_eq!(p.verdict.text, "Lossy stream (MP3)");
    let src = p.source.as_ref().unwrap();
    assert!(src.transcoded && !src.lossless);
    // The file's own depth and bitrate don't describe the transcode.
    assert_eq!((src.bit_depth, src.bitrate), (None, None));
    assert_eq!(p.why.as_ref().unwrap().setting, Some("qualityHome"));
    // A transcode because the file wouldn't play has no setting to change.
    let s = SourceInfo { fallback: true, ..s };
    assert_eq!(signal::build(&inputs(&t, &s, &o, fmt(44_100))).why.unwrap().setting, None);
}

#[test]
fn a_lossy_file_says_so_with_its_bitrate() {
    let mut t = track("m", "Artist", "/p/m");
    t.codec = Some("mp3".into());
    t.bitrate = Some(320);
    let s = SourceInfo { codec: None, ..source(44_100) };
    let o = out(Exclusive::Off);
    let p = signal::build(&inputs(&t, &s, &o, fmt(44_100)));
    assert_eq!(p.verdict.text, "Lossy source (MP3 320)");
    assert_eq!(p.why.unwrap().setting, None);
}

#[test]
fn eq_and_leveling_applied_are_named_with_the_gain() {
    let t = flac(16, 44_100);
    let s = SourceInfo { audio: Some(TrackAudio { gain_db: Some(-7.5), peak: Some(0.9), ..Default::default() }), ..source(44_100) };
    let o = out(Exclusive::Off);
    let mut i = inputs(&t, &s, &o, fmt(44_100));
    i.dsp = DspSettings { leveling: Leveling::Track, eq: Some([3.0; dsp::EQ_BANDS]), preamp_db: 0.0, ..Default::default() };
    let p = signal::build(&i);
    assert!(p.loudness.on);
    assert_eq!(p.loudness.mode, "track");
    assert!((p.loudness.gain_db.unwrap() + 7.5).abs() < 0.01);
    assert!(p.eq.eq && !p.eq.correction);
    assert!(p.resample.is_none());
    assert_eq!(p.verdict.text, "Lossless, volume and EQ applied");
    // With headphone correction too, and a preamp that isn't counted as leveling.
    i.dsp.correction = Some(Correction::new(-4.0, &[]));
    i.dsp.preamp_db = 2.0;
    let p = signal::build(&i);
    assert_eq!(p.verdict.text, "Lossless, volume, EQ and headphone correction applied");
    assert_eq!(p.eq.correction_preamp_db, Some(-4.0));
    assert!((p.loudness.gain_db.unwrap() + 7.5).abs() < 0.01);
    // Nothing applied and the rate matches: still the system mixer.
    let p = signal::build(&inputs(&t, &s, &o, fmt(44_100)));
    assert_eq!(p.verdict.text, "Lossless, shared with other apps");
}

#[test]
fn a_shared_fallback_says_why() {
    let t = flac(24, 192_000);
    let s = source(192_000);
    let o = out(Exclusive::Fallback { reason: "Another app is using the device on its own.".into() });
    let p = signal::build(&inputs(&t, &s, &o, fmt(48_000)));
    assert_eq!(p.output.mode, "fallback");
    let why = p.why.unwrap();
    assert!(why.text.contains("Another app is using the device"), "{}", why.text);
    assert_eq!(why.setting, Some("bitPerfectOutputs"));
}

#[test]
fn nothing_playing_is_idle_and_the_json_is_camel_case() {
    let o = out(Exclusive::Off);
    let p = signal::build(&Inputs {
        track: None,
        source: None,
        dsp: DspSettings::default(),
        album_in_order: false,
        output: Some(&o),
        format: None,
        volume: 0.5,
        muted: true,
        automix: false,
    });
    assert_eq!(p.verdict.kind, VerdictKind::Idle);
    assert!(p.source.is_none() && p.decode.is_none());
    assert_eq!(p.output.volume, 0.0);
    let t = flac(24, 96_000);
    let s = source(96_000);
    let v = serde_json::to_value(signal::build(&inputs(&t, &s, &o, fmt(48_000)))).unwrap();
    assert_eq!(v["verdict"]["kind"], "lossless");
    assert_eq!(v["source"]["bitDepth"], 24);
    assert_eq!(v["source"]["from"], "server");
    assert_eq!(v["resample"]["toRate"], 48_000);
    assert_eq!(v["output"]["volumeBypassed"], false);
    assert_eq!(v["why"]["setting"], "bitPerfectOutputs");
}

// ---------------------------------------------------------------- the engine

const RATE: u32 = 44_100;

struct EngineRig {
    server: TestServer,
    engine: Engine,
    events: UnboundedReceiver<EngineEvent>,
    cap: CaptureHandle,
}

async fn engine_rig() -> EngineRig {
    let server = TestServer::start().await;
    let res = FakeResolver::new(&server);
    let (out, cap) = CaptureOutput::new(OutputFormat { sample_rate: RATE, channels: 2 });
    cap.set_devices(vec![dev("spk", "Speakers", true)]);
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
    EngineRig { server, engine, events, cap }
}

fn wav(frames: usize, rate: u32) -> Vec<u8> {
    let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut cur = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
        for i in 0..frames {
            let v = ((i % 100) as i16 - 50) * 100;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
    }
    cur.into_inner()
}

fn add(r: &EngineRig, id: &str, rate: u32, frames: usize) -> Track {
    r.server.add(&format!("/t/{id}"), Route { body: wav(frames, rate), ..Default::default() });
    let mut t = track(id, "Artist", &format!("/library/parts/{id}/file.wav"));
    t.sample_rate = Some(rate);
    t.bit_depth = Some(16);
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

/// Plays `t` until the source is known and audio is heard; returns its key.
async fn play(r: &mut EngineRig, t: Track) -> u64 {
    let key = r.engine.load(t, "i".into(), 0, true).unwrap();
    until(&mut r.events, |e| matches!(e, EngineEvent::Started { key: k, .. } if *k == key)).await;
    key
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_engine_records_the_source_and_device_formats_while_shared() {
    let mut r = engine_rig().await;
    let t = add(&r, "a", 96_000, 96_000);
    let key = play(&mut r, t.clone()).await;
    let s = r.engine.published_source(key).expect("source recorded");
    assert_eq!((s.rate, s.channels, s.from, s.transcoded), (96_000, 2, SourceFrom::Server, false));
    assert_eq!(r.engine.published_format(), fmt(RATE));
    let o = r.engine.output_state();
    let p = signal::build(&Inputs { format: r.engine.published_format(), ..inputs(&t, &s, &o, None) });
    assert_eq!(p.verdict.text, "Lossless, resampled to 44.1 kHz");
    // Reads never wait on the render lock.
    assert!(r.engine.signal_reads_skip_render_lock(key));
    // Closed, there's no device format.
    r.engine.stop();
    r.engine.close_output();
    assert_eq!(r.engine.published_format(), None);
    assert!(r.cap.opens() >= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_engine_records_an_exclusive_stream_at_the_songs_rate() {
    let mut r = engine_rig().await;
    let t = add(&r, "a", 96_000, 48_000);
    r.engine.set_exclusive_devices(vec!["spk".into()]).unwrap();
    let key = play(&mut r, t.clone()).await;
    let s = r.engine.published_source(key).unwrap();
    assert_eq!(r.engine.published_format(), fmt(96_000));
    let o = r.engine.output_state();
    assert!(o.exclusive.is_on());
    let p = signal::build(&Inputs { format: r.engine.published_format(), ..inputs(&t, &s, &o, None) });
    assert_eq!(p.verdict.kind, VerdictKind::BitPerfect);
    assert!(r.engine.signal_reads_skip_render_lock(key));
}

// ---------------------------------------------------------------- the player

#[derive(Default)]
struct Sink {
    paths: Mutex<Vec<SignalPath>>,
}

impl EventSink for Sink {
    fn state(&self, _: &PlayerState) {}
    fn position(&self, _: &PositionPayload) {}
    fn error(&self, _: &ErrorPayload) {}
    fn signal_path(&self, p: &SignalPath) {
        self.paths.lock().push(p.clone());
    }
}

/// Waits until a sent signal path matches.
async fn sent(sink: &Sink, what: &str, pred: impl Fn(&SignalPath) -> bool) -> SignalPath {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(p) = sink.paths.lock().iter().rev().find(|p| pred(p)) {
            return p.clone();
        }
        assert!(Instant::now() < deadline, "no signal path with {what}; sent {:?}", sink.paths.lock().last());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_player_sends_the_path_on_track_device_and_settings_changes() {
    let server = TestServer::start().await;
    let resolver = FakeResolver::new(&server);
    server.add("/t/a", Route { body: wav_tone(RATE, 2, RATE as usize * 4, 220.0, 0.5), ..Default::default() });
    let mut t = track("a", "Artist", "/p/a");
    t.duration_ms = 4_000;
    resolver.tracks.lock().insert("a".into(), t);
    let (out, cap) = CaptureOutput::new(OutputFormat { sample_rate: RATE, channels: 2 });
    cap.set_devices(vec![dev("spk", "Speakers", true), dev("usb", "USB DAC", false)]);
    let sink = Arc::new(Sink::default());
    let player = PlayerHandle::spawn(
        tokio::runtime::Handle::current(),
        resolver.clone(),
        resolver.clone(),
        Box::new(out.realtime()),
        sink.clone(),
        PlayerConfig {
            engine: EngineConfig { buffer_ahead_ms: 2_000, ramp_ms: 0, declick_ms: 0, ..Default::default() },
            seed: Some(1),
            position_interval: Duration::from_millis(50),
            ..PlayerConfig::default()
        },
    );
    let ctx = PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(vec!["a".into()]) };
    player.play_context(ctx, None, None).await.unwrap();

    // The track: its source, decoded and on the default device.
    let p = sent(&sink, "the track's source", |p| p.track_id.as_deref() == Some("a") && p.source.as_ref().is_some_and(|s| s.channels.is_some())).await;
    assert_eq!(p.output.device.as_deref(), Some("Speakers"));
    assert_eq!(p.source.unwrap().sample_rate, Some(RATE));
    // The snapshot command agrees with what was sent.
    assert_eq!(player.signal_path().await.unwrap().track_id.as_deref(), Some("a"));

    // Another device.
    player.select_output(Some("usb".into())).await.unwrap();
    sent(&sink, "the new device", |p| p.output.device.as_deref() == Some("USB DAC")).await;

    // A sound setting.
    player.set_dsp(DspSettings { eq: Some([4.0; dsp::EQ_BANDS]), leveling: Leveling::Off, ..Default::default() }).await.unwrap();
    let p = sent(&sink, "the EQ on", |p| p.eq.eq).await;
    assert!(p.verdict.text.contains("EQ applied"), "{}", p.verdict.text);

    // Unchanged state isn't sent again.
    let n = sink.paths.lock().len();
    player.state().await.unwrap();
    player.signal_path().await.unwrap();
    assert_eq!(sink.paths.lock().len(), n);
    player.pause().await.unwrap();
}
