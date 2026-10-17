//! Automix blends through the whole engine, on a capture output: where a
//! blend starts, the tempo change, the handover at the midpoint, Next and
//! seeking during a blend, and a different next song mid-blend. All audio
//! is generated at test time; A plays on the left, B on the right.

use std::io::Cursor;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedReceiver;

use super::automix::{BlendPlan, Curve};
use super::engine::{Engine, EngineConfig, EngineDeps, EngineEvent};
use super::gate::TransferGate;
use super::http::{HttpConfig, http_client};
use super::output::{CaptureHandle, CaptureOutput, OutputFormat};
use super::testutil::*;
use crate::model::Track;

const RATE: u32 = 44_100;

struct Rig {
    server: TestServer,
    engine: Engine,
    events: UnboundedReceiver<EngineEvent>,
    cap: CaptureHandle,
}

async fn rig(paced: bool) -> Rig {
    let server = TestServer::start().await;
    let (out, cap) = CaptureOutput::new(OutputFormat { sample_rate: RATE, channels: 2 });
    let out = if paced { out.realtime() } else { out };
    let (engine, events) = Engine::new(EngineDeps {
        resolver: FakeResolver::new(&server),
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

fn tr(id: &str, secs: f64) -> Track {
    let mut t = track(id, "Artist", &format!("/library/parts/{id}/file.wav"));
    t.album_id = format!("album-{id}");
    t.duration_ms = (secs * 1000.0) as u64;
    t
}

fn secs(s: f64) -> usize {
    (s * RATE as f64) as usize
}

fn tone(i: f64, amp: f32) -> f32 {
    ((2.0 * std::f64::consts::PI * 220.0 * i / RATE as f64).sin() as f32) * amp
}

fn wav(frames: usize, f: impl Fn(usize) -> (f32, f32)) -> Vec<u8> {
    let spec = hound::WavSpec { channels: 2, sample_rate: RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
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

fn add(r: &Rig, id: &str, body: Vec<u8>) {
    r.server.add(&format!("/t/{id}"), Route { body, ..Default::default() });
}

async fn until(rx: &mut UnboundedReceiver<EngineEvent>, stop: impl Fn(&EngineEvent) -> bool) -> Vec<EngineEvent> {
    let deadline = Instant::now() + Duration::from_secs(30);
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

async fn wait_for(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !f() {
        assert!(Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

async fn wait_decoded(engine: &Engine, n: usize) {
    wait_for("decoded", || {
        let (segs, done) = engine.decoded();
        segs >= n && done
    })
    .await;
}

fn order(ev: &[EngineEvent]) -> Vec<String> {
    ev.iter()
        .filter_map(|e| match e {
            EngineEvent::Started { key, .. } => Some(format!("start{key}")),
            EngineEvent::Ended { key, listened_ms, .. } => Some(format!("end{key}:{listened_ms}")),
            _ => None,
        })
        .collect()
}

fn plan(a_start_ms: f64, len_ms: f64, rate: f64, curve: Curve) -> BlendPlan {
    BlendPlan { a_start_ms: Some(a_start_ms), len_ms, b_start_ms: 0.0, rate, ramp_ms: 0.0, curve, beat_ms: 500.0, beatmatched: false }
}

const LVL: f32 = 0.5 * 16383.0 / 16384.0;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_blend_starts_where_planned_and_hands_over_at_its_midpoint() {
    let mut r = rig(false).await;
    add(&r, "a", wav(secs(4.0), |_| (0.5, 0.0)));
    add(&r, "b", wav(secs(2.5), |_| (0.0, 0.5)));
    let key = r.engine.load(tr("a", 4.0), "ia".into(), 0, false).unwrap();
    let nk = r.engine.set_next(Some((tr("b", 2.5), "ib".into())), key).unwrap();
    r.engine.set_blend(key, nk, Some(plan(2_500.0, 1_500.0, 1.0, Curve::Blend)));
    assert!(r.engine.blend_of(key).is_some());
    wait_decoded(&r.engine, 2).await;
    r.engine.play().unwrap();
    let ev = until(&mut r.events, drained).await;
    // A alone for 2.5 s, both for 1.5 s, then the rest of B: 5 s in all.
    let got = settled_samples(&r.cap, secs(5.0) * 2).await;
    assert_eq!(got.len() / 2, secs(5.0));
    assert!((got[secs(2.4) * 2] - LVL).abs() < 1e-4 && got[secs(2.4) * 2 + 1] == 0.0, "A alone before");
    for i in [secs(2.7), secs(3.25), secs(3.9)] {
        let (l, rr) = (got[i * 2], got[i * 2 + 1]);
        assert!(((l * l + rr * rr) - LVL * LVL).abs() < 2e-3, "equal power at {i}");
    }
    assert!(got[secs(4.5) * 2] == 0.0 && (got[secs(4.5) * 2 + 1] - LVL).abs() < 1e-4, "B alone after");
    // B takes over at the midpoint: A had played 3.25 s by then.
    assert_eq!(order(&ev), vec![format!("start{key}"), format!("end{key}:3250"), format!("start{nk}"), format!("end{nk}:2500")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_outgoing_song_speeds_up_to_match_the_tempo() {
    let mut r = rig(false).await;
    add(&r, "a", wav(secs(4.0), |i| (tone(i as f64, 0.5), 0.0)));
    add(&r, "b", wav(secs(2.5), |_| (0.0, 0.25)));
    let key = r.engine.load(tr("a", 4.0), "ia".into(), 0, false).unwrap();
    let nk = r.engine.set_next(Some((tr("b", 2.5), "ib".into())), key).unwrap();
    let p = plan(2_500.0, 1_400.0, 1.05, Curve::Lift);
    r.engine.set_blend(key, nk, Some(p));
    wait_decoded(&r.engine, 2).await;
    r.engine.play().unwrap();
    until(&mut r.events, drained).await;
    let got = settled_samples(&r.cap, secs(5.0) * 2).await;
    assert_eq!(got.len() / 2, secs(2.5) + secs(2.5), "B plays whole from the blend's start");
    // During the first half (Lift: A at full volume) A is 5% faster.
    let start = secs(2.5);
    for k in (100..secs(0.65)).step_by(997) {
        let want = tone(start as f64 + 1.05 * k as f64, 0.5);
        let l = got[(start + k) * 2];
        assert!((l - want).abs() < 0.01, "frame {k}: {l} vs {want}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn next_during_a_blend_jumps_straight_to_the_next_song() {
    let mut r = rig(true).await;
    add(&r, "a", wav(secs(4.0), |_| (0.5, 0.0)));
    add(&r, "b", wav(secs(3.0), |_| (0.0, 0.5)));
    let key = r.engine.load(tr("a", 4.0), "ia".into(), 0, false).unwrap();
    let nk = r.engine.set_next(Some((tr("b", 3.0), "ib".into())), key).unwrap();
    r.engine.set_blend(key, nk, Some(plan(1_000.0, 2_400.0, 1.0, Curve::Blend)));
    wait_decoded(&r.engine, 2).await;
    r.engine.play().unwrap();
    wait_for("blending", || r.engine.blending()).await;
    // Before the midpoint the outgoing song is still the one playing.
    let p = r.engine.position().unwrap();
    assert_eq!(p.key, key);
    assert!(p.position_ms >= 1_000 && p.position_ms < 2_300, "{}", p.position_ms);
    assert_eq!(r.engine.next_key(), Some(nk));
    assert!(!r.engine.skip_blend(nk), "only the outgoing song's blend");
    let at = r.cap.len_frames(2);
    assert!(r.engine.skip_blend(key));
    assert!(!r.engine.skip_blend(key), "once");
    let ev = until(&mut r.events, |e| matches!(e, EngineEvent::Started { key: k, .. } if *k == nk)).await;
    assert!(matches!(ev[ev.len() - 2], EngineEvent::Ended { key: k, .. } if k == key));
    wait_for("blend over", || !r.engine.blending()).await;
    assert_eq!(r.engine.position().unwrap().key, nk);
    until(&mut r.events, drained).await;
    let got = settled_samples(&r.cap, 0).await;
    // A is gone within a moment of the skip (allowing for the device
    // buffer), long before the blend would have ended; B is at full level.
    let after = at + secs(0.25);
    assert!(after < secs(3.0), "the skip came mid-blend");
    for f in got.chunks(2).skip(after) {
        assert!(f[0].abs() < 1e-6, "A still playing");
    }
    assert!((got[(after + 10) * 2 + 1] - LVL).abs() < 1e-4, "B at full volume");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seeking_during_a_blend_cancels_it() {
    let mut r = rig(true).await;
    add(&r, "a", wav(secs(4.0), |_| (0.5, 0.0)));
    add(&r, "b", wav(secs(2.0), |_| (0.0, 0.5)));
    let key = r.engine.load(tr("a", 4.0), "ia".into(), 0, false).unwrap();
    let nk = r.engine.set_next(Some((tr("b", 2.0), "ib".into())), key).unwrap();
    r.engine.set_blend(key, nk, Some(plan(1_000.0, 2_800.0, 1.0, Curve::Blend)));
    wait_decoded(&r.engine, 2).await;
    r.engine.play().unwrap();
    wait_for("blending", || r.engine.blending()).await;
    // Forward within the outgoing song: the blend stops, A plays on alone.
    assert!(r.engine.seek(3_000));
    assert!(!r.engine.blending());
    let p = r.engine.position().unwrap();
    assert_eq!(p.key, key);
    assert!((3_000..3_100).contains(&p.position_ms), "{}", p.position_ms);
    let at = r.cap.len_frames(2);
    // B comes back as the next song, from its start, once A ends.
    let ev = until(&mut r.events, drained).await;
    assert_eq!(order(&ev).iter().filter(|e| e.starts_with(&format!("start{nk}"))).count(), 1);
    let got = settled_samples(&r.cap, 0).await;
    let b_first = (at..got.len() / 2).find(|&i| got[i * 2 + 1] != 0.0).expect("B played");
    let a_last = (at..got.len() / 2).rfind(|&i| got[i * 2] != 0.0).expect("A played");
    assert!(b_first > a_last, "no blend after the seek: B from {b_first}, A until {a_last}");
    assert_eq!(got.len() / 2 - b_first, secs(2.0), "the whole of B");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_different_next_song_mid_blend_ends_the_blend() {
    let mut r = rig(true).await;
    add(&r, "a", wav(secs(3.0), |_| (0.5, 0.0)));
    add(&r, "b", wav(secs(2.0), |_| (0.0, 0.5)));
    add(&r, "c", wav(secs(1.0), |_| (0.0, 0.3)));
    let key = r.engine.load(tr("a", 3.0), "ia".into(), 0, false).unwrap();
    let nk = r.engine.set_next(Some((tr("b", 2.0), "ib".into())), key).unwrap();
    r.engine.set_blend(key, nk, Some(plan(1_000.0, 1_900.0, 1.0, Curve::Blend)));
    wait_decoded(&r.engine, 2).await;
    r.engine.play().unwrap();
    wait_for("blending", || r.engine.blending()).await;
    // The same next song carries on.
    assert_eq!(r.engine.set_next(Some((tr("b", 2.0), "ib".into())), key), Some(nk));
    assert!(r.engine.blending());
    // Another one: back to A alone, C next.
    let ck = r.engine.set_next(Some((tr("c", 1.0), "ic".into())), key).unwrap();
    assert!(!r.engine.blending());
    assert_eq!(r.engine.position().unwrap().key, key);
    let ev = until(&mut r.events, drained).await;
    let o = order(&ev);
    assert!(o.contains(&format!("start{ck}")), "{o:?}");
    assert!(!o.contains(&format!("start{nk}")), "{o:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_late_beatmatched_blend_is_skipped_for_the_usual_crossfade() {
    let mut r = rig(false).await;
    add(&r, "a", wav(secs(3.0), |_| (0.5, 0.0)));
    add(&r, "b", wav(secs(2.0), |_| (0.0, 0.5)));
    // Planned to start at 1 s, but playback starts at 1.5 s: no blend.
    let key = r.engine.load(tr("a", 3.0), "ia".into(), 1_500, false).unwrap();
    let nk = r.engine.set_next(Some((tr("b", 2.0), "ib".into())), key).unwrap();
    let mut p = plan(1_000.0, 1_000.0, 1.0, Curve::Blend);
    p.beatmatched = true;
    r.engine.set_blend(key, nk, Some(p));
    wait_decoded(&r.engine, 2).await;
    r.engine.play().unwrap();
    until(&mut r.events, drained).await;
    let got = settled_samples(&r.cap, (secs(1.5) + secs(2.0)) * 2).await;
    assert_eq!(got.len() / 2, secs(1.5) + secs(2.0), "back to back, crossfade off");
}

// ------------------------------------------------------------------ player

mod player {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use parking_lot::Mutex;
    use tokio::sync::broadcast;

    use super::super::automix::{AutomixOptions, Feel, MixInfoSource, MixStyle, Mixed, MixedPlaylist, Reach, gap_key};
    use super::super::events::{ErrorPayload, EventSink, PlayerState, PositionPayload, TrackingEvent};
    use super::super::output::{CaptureHandle, CaptureOutput, OutputFormat};
    use super::super::player::{PlayerConfig, PlayerHandle};
    use super::super::queue::Repeat;
    use super::super::source::{ContextKind, PlayContext};
    use super::super::testutil::*;
    use super::{RATE, secs, wait_for};
    use crate::analysis::features::{Analysis, Grid};

    struct Quiet;
    impl EventSink for Quiet {
        fn state(&self, _: &PlayerState) {}
        fn position(&self, _: &PositionPayload) {}
        fn error(&self, _: &ErrorPayload) {}
    }

    #[derive(Default)]
    struct Infos {
        known: Mutex<HashMap<String, Analysis>>,
        wanted: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl MixInfoSource for Infos {
        async fn infos(&self, ids: &[String]) -> crate::error::AppResult<HashMap<String, Analysis>> {
            let k = self.known.lock();
            Ok(ids.iter().filter_map(|id| k.get(id).map(|a| (id.clone(), a.clone()))).collect())
        }
        fn want(&self, ids: &[String]) {
            self.wanted.lock().extend(ids.iter().cloned());
        }
    }

    /// An analysis of a minute-long song whose music ends at `cue_out_ms`.
    fn info(cue_out_ms: u64, grid: bool) -> Analysis {
        let g = Grid { downbeat_ms: 100.0, beat_ms: 500.0, conf: 0.9, downbeat_conf: 0.9 };
        Analysis {
            duration_ms: 60_000,
            bpm: Some(120.0),
            beat_conf: 0.9,
            intro: grid.then_some(g),
            outro: grid.then_some(Grid { downbeat_ms: 30_100.0, ..g }),
            key: None,
            key_conf: 0.0,
            energy: 0.5,
            loudness_db: -12.0,
            cue_in_ms: 0,
            cue_out_ms,
            intro_end_ms: 0,
            outro_start_ms: 0,
            starts_hot: false,
            ends_hot: false,
            sections: vec![],
            features: vec![],
        }
    }

    struct Rig {
        _server: TestServer,
        player: PlayerHandle,
        cap: CaptureHandle,
        tracking: broadcast::Receiver<TrackingEvent>,
    }

    /// Songs a, b, c: `audio_secs` of audio each, but a minute long as far
    /// as the library knows (so a blend isn't cut down by their length).
    async fn rig(audio_secs: f32, realtime: bool) -> Rig {
        let server = TestServer::start().await;
        let resolver = FakeResolver::new(&server);
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            let frames = (audio_secs * RATE as f32) as usize;
            server.add(&format!("/t/{id}"), Route { body: wav_tone(RATE, 2, frames, 220.0 + 110.0 * i as f32, 0.5), ..Default::default() });
            let mut t = track(id, "Artist", &format!("/p/{id}"));
            t.album_id = format!("album-{id}");
            t.duration_ms = 60_000;
            resolver.tracks.lock().insert(id.to_string(), t);
        }
        let (out, cap) = CaptureOutput::new(OutputFormat { sample_rate: RATE, channels: 2 });
        let out = if realtime { out.realtime() } else { out };
        let player = PlayerHandle::spawn(
            tokio::runtime::Handle::current(),
            resolver.clone(),
            resolver,
            Box::new(out),
            Arc::new(Quiet),
            PlayerConfig {
                engine: super::super::engine::EngineConfig { buffer_ahead_ms: 2_000, ramp_ms: 0, declick_ms: 0, http: Default::default(), ..Default::default() },
                seed: Some(1),
                position_interval: Duration::from_millis(50),
                automix: AutomixOptions { on: true, reach: Reach::PlaylistsAndMixes, feel: Feel::Smooth, fade_ms: 0 },
                ..Default::default()
            },
        );
        let tracking = player.subscribe_tracking();
        Rig { _server: server, player, cap, tracking }
    }

    fn ctx(kind: ContextKind, id: &str) -> PlayContext {
        PlayContext { kind, id: Some(id.into()), track_ids: Some(vec!["a".into(), "b".into(), "c".into()]) }
    }

    async fn load(r: &Rig, kind: ContextKind, id: &str) {
        r.player.play_context_at(ctx(kind, id), None, 0, false).await.unwrap();
    }

    fn mixed(id: &str, styles: &[(&str, &str, MixStyle)]) -> Mixed {
        let mut p = MixedPlaylist::default();
        for (a, b, s) in styles {
            p.styles.insert(gap_key(a, b), *s);
        }
        HashMap::from([(id.to_string(), p)])
    }

    fn opts(on: bool, reach: Reach) -> AutomixOptions {
        AutomixOptions { on, reach, feel: Feel::Smooth, fade_ms: 0 }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automix_covers_mixes_and_mixed_playlists_never_albums() {
        let r = rig(0.5, false).await;
        load(&r, ContextKind::Mix, "m").await;
        let p = r.player.peek_blend().await.expect("a mix blends");
        assert!(!p.beatmatched && (p.len_ms - 6_000.0).abs() < 1.0, "{p:?}");
        load(&r, ContextKind::Radio, "track:a").await;
        assert!(r.player.peek_blend().await.is_some(), "radio blends");
        load(&r, ContextKind::Tracks, "t").await;
        assert!(r.player.peek_blend().await.is_none(), "only playlists and mixes");
        r.player.set_automix(opts(true, Reach::AllButAlbums)).await.unwrap();
        assert!(r.player.peek_blend().await.is_some(), "everywhere but albums");
        load(&r, ContextKind::Album, "al").await;
        assert!(r.player.peek_blend().await.is_none(), "never an album");
        r.player.set_automix(opts(true, Reach::PlaylistsAndMixes)).await.unwrap();
        load(&r, ContextKind::Playlist, "pl").await;
        assert!(r.player.peek_blend().await.is_none(), "Mix is off on this playlist");
        r.player.set_mixed(mixed("pl", &[])).await.unwrap();
        assert!(r.player.peek_blend().await.is_some(), "Mix on");
        r.player.set_mixed(mixed("pl", &[("a", "b", MixStyle::None)])).await.unwrap();
        assert!(r.player.peek_blend().await.is_none(), "No transition picked for this gap");
        r.player.set_mixed(mixed("pl", &[("a", "b", MixStyle::Echo)])).await.unwrap();
        assert_eq!(r.player.peek_blend().await.unwrap().curve, super::Curve::Echo);
        r.player.set_repeat(Repeat::One).await.unwrap();
        assert!(r.player.peek_blend().await.is_none(), "repeating one song");
        r.player.set_repeat(Repeat::Off).await.unwrap();
        r.player.set_automix(opts(false, Reach::AllButAlbums)).await.unwrap();
        assert!(r.player.peek_blend().await.is_none(), "Automix off");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blends_follow_the_analyses_and_unknown_songs_are_asked_for() {
        let r = rig(0.5, false).await;
        let infos = Arc::new(Infos::default());
        r.player.set_mix_source(infos.clone()).await.unwrap();
        load(&r, ContextKind::Mix, "m").await;
        wait_for("songs asked for", || infos.wanted.lock().len() >= 2).await;
        assert!(infos.wanted.lock().contains(&"a".to_string()) && infos.wanted.lock().contains(&"b".to_string()));
        // Analysed since: the blend is planned again, now on the beat.
        infos.known.lock().insert("a".into(), info(59_000, true));
        infos.known.lock().insert("b".into(), info(59_000, true));
        r.player.analysed(vec!["a".into(), "b".into()]);
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if r.player.peek_blend().await.is_some_and(|p| p.beatmatched) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "never beat-matched");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn started_b(r: &mut Rig) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match tokio::time::timeout(left, r.tracking.recv()).await {
                Ok(Ok(TrackingEvent::TrackStarted { track, .. })) if track.id == "b" => return,
                Ok(_) => {}
                Err(_) => panic!("b never started"),
            }
        }
    }

    /// A mixed playlist whose songs' music ends at 3 s, fading over 1 s.
    async fn blending_rig() -> Rig {
        let r = rig(3.0, true).await;
        let infos = Arc::new(Infos::default());
        for id in ["a", "b", "c"] {
            infos.known.lock().insert(id.into(), info(3_000, false));
        }
        r.player.set_mix_source(infos).await.unwrap();
        r.player.set_automix(AutomixOptions { fade_ms: 1_000, ..opts(true, Reach::PlaylistsAndMixes) }).await.unwrap();
        r.player.set_mixed(mixed("pl", &[])).await.unwrap();
        r.player.play_context(ctx(ContextKind::Playlist, "pl"), None, Some(false)).await.unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while r.player.peek_blend().await.and_then(|p| p.a_start_ms) != Some(2_000.0) {
            assert!(std::time::Instant::now() < deadline, "never planned from the analysis");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        r
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_next_song_takes_over_at_the_blends_midpoint() {
        let mut r = blending_rig().await;
        started_b(&mut r).await;
        // The blend runs from 2 s to 3 s: b took over halfway, not at its start.
        let at = r.cap.len_frames(2);
        assert!(at >= secs(2.3), "took over at {at} frames");
        let s = r.player.state().await.unwrap();
        assert_eq!(s.track.unwrap().id, "b");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn next_mid_blend_carries_on_with_the_next_song() {
        let mut r = blending_rig().await;
        wait_for("the blend", || r.cap.len_frames(2) >= secs(2.2)).await;
        r.player.next().await.unwrap();
        started_b(&mut r).await;
        let s = r.player.state().await.unwrap();
        assert_eq!(s.track.unwrap().id, "b");
        // b went on from where it was in the blend, not from its start.
        assert!(s.position_ms >= 150, "b at {} ms", s.position_ms);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn next_mid_blend_counts_the_outgoing_song_as_skipped() {
        let mut r = blending_rig().await;
        wait_for("the blend", || r.cap.len_frames(2) >= secs(2.2)).await;
        r.player.next().await.unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut ends = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match tokio::time::timeout(left, r.tracking.recv()).await {
                Ok(Ok(TrackingEvent::TrackEnded { track, completed, skipped_at_ms, .. })) => ends.push((track.id, completed, skipped_at_ms)),
                Ok(Ok(TrackingEvent::TrackStarted { track, .. })) if track.id == "b" => break,
                Ok(_) => {}
                Err(_) => panic!("b never started"),
            }
        }
        assert_eq!(ends.len(), 1, "{ends:?}");
        let (id, completed, at) = &ends[0];
        assert_eq!(id, "a");
        assert!(!completed, "skipped, not finished");
        assert!(at.is_some_and(|ms| ms >= 2_000), "{at:?}");
    }
}
