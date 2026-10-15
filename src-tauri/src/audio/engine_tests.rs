//! Engine tests against a local HTTP server and a capture output. No sound
//! device, no network beyond 127.0.0.1, all audio generated at test time.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedReceiver;

use super::engine::{Engine, EngineConfig, EngineDeps, EngineEvent, Origin};
use super::gate::{Priority, TransferGate};
use super::http::{HttpConfig, HttpSource, http_client};
use super::output::{CaptureHandle, CaptureOutput, OutputFormat};
use super::source::StreamSource;
use super::testutil::*;
use crate::error::AppError;
use crate::model::Track;

const RATE: u32 = 44_100;

fn cfg() -> EngineConfig {
    EngineConfig {
        buffer_ahead_ms: 2_000,
        ramp_ms: 0,
        declick_ms: 0,
        http: HttpConfig {
            min_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(50),
            retry_window: Duration::from_secs(5),
            ..Default::default()
        },
    }
}

struct Rig {
    server: TestServer,
    resolver: Arc<FakeResolver>,
    engine: Engine,
    events: UnboundedReceiver<EngineEvent>,
    cap: CaptureHandle,
    channels: usize,
    gate: Arc<TransferGate>,
}

async fn rig_full(
    rate: u32,
    config: EngineConfig,
    cache: Option<Arc<super::cache::DiskCache>>,
    paced: bool,
) -> Rig {
    rig_local(rate, config, cache, None, paced).await
}

async fn rig_local(
    rate: u32,
    config: EngineConfig,
    cache: Option<Arc<super::cache::DiskCache>>,
    local: Option<super::source::LocalFiles>,
    paced: bool,
) -> Rig {
    let server = TestServer::start().await;
    let resolver = FakeResolver::new(&server);
    let (out, cap) = CaptureOutput::new(OutputFormat {
        sample_rate: rate,
        channels: 2,
    });
    let out = if paced { out.realtime() } else { out };
    let gate = TransferGate::new();
    let (engine, events) = Engine::new(EngineDeps {
        resolver: resolver.clone(),
        cache,
        local,
        client: http_client(),
        rt: tokio::runtime::Handle::current(),
        output: Box::new(out),
        config,
        gate: gate.clone(),
    });
    Rig {
        server,
        resolver,
        engine,
        events,
        cap,
        channels: 2,
        gate,
    }
}

async fn rig_with(rate: u32, config: EngineConfig, cache: Option<Arc<super::cache::DiskCache>>) -> Rig {
    rig_full(rate, config, cache, false).await
}

async fn rig_paced(rate: u32, config: EngineConfig) -> Rig {
    rig_full(rate, config, None, true).await
}

async fn rig() -> Rig {
    rig_with(RATE, cfg(), None).await
}

fn t(id: &str) -> Track {
    let mut tr = track(id, "Artist", &format!("/library/parts/{id}/file.wav"));
    tr.duration_ms = 3_000;
    tr
}

/// Collects events until `stop` matches (inclusive) or the timeout.
async fn collect_until(
    rx: &mut UnboundedReceiver<EngineEvent>,
    stop: impl Fn(&EngineEvent) -> bool,
    timeout: Duration,
) -> Vec<EngineEvent> {
    let deadline = Instant::now() + timeout;
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
            _ => panic!("timed out waiting for event; got {out:?}"),
        }
    }
}

fn is_drained(e: &EngineEvent) -> bool {
    matches!(e, EngineEvent::Drained)
}

fn assert_same(got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len(), "sample count");
    if let Some(i) = got.iter().zip(want).position(|(a, b)| a != b) {
        panic!("first difference at sample {i}: {} vs {}", got[i], want[i]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gapless_back_to_back_is_sample_exact() {
    let mut r = rig().await;
    let a = wav_tone(RATE, 2, RATE as usize + 123, 440.0, 0.5);
    let b = wav_tone(RATE, 2, 30_011, 660.0, 0.4);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    r.server.add("/t/b", Route { body: b.clone(), ..Default::default() });

    let key = r.engine.load(t("a"), "ia".into(), 0, true).unwrap();
    let next_key = r.engine.set_next(Some((t("b"), "ib".into())), key).unwrap();
    let ev = collect_until(&mut r.events, is_drained, Duration::from_secs(20)).await;

    let (ra, _, _) = decode_all(&a, "wav");
    let (rb, _, _) = decode_all(&b, "wav");
    let mut want = ra.clone();
    want.extend_from_slice(&rb);
    let got = settled_samples(&r.cap, want.len()).await;
    assert_same(&got, &want);
    assert_eq!(got.len() / r.channels, RATE as usize + 123 + 30_011);

    let order: Vec<String> = ev
        .iter()
        .filter_map(|e| match e {
            EngineEvent::Started { key, .. } => Some(format!("start{key}")),
            EngineEvent::Ended { key, .. } => Some(format!("end{key}")),
            EngineEvent::Drained => Some("drained".into()),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        vec![
            format!("start{key}"),
            format!("end{key}"),
            format!("start{next_key}"),
            format!("end{next_key}"),
            "drained".to_string()
        ]
    );
    // Listened time is reported per track.
    let listened: Vec<u64> = ev
        .iter()
        .filter_map(|e| match e {
            EngineEvent::Ended { listened_ms, .. } => Some(*listened_ms),
            _ => None,
        })
        .collect();
    assert_eq!(listened, vec![1003, 681]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gapless_with_resampling_has_exact_length_and_no_discontinuity() {
    // Device at 48 kHz, files at 44.1 kHz; B continues A's sine exactly, so
    // any gap or pop would show up as a jump at the boundary.
    let mut r = rig_with(48_000, cfg(), None).await;
    let na = 50_000usize;
    let nb = 40_000usize;
    let a = wav_tone_from(RATE, 2, 0, na, 300.0, 0.5);
    let b = wav_tone_from(RATE, 2, na, nb, 300.0, 0.5);
    r.server.add("/t/a", Route { body: a, ..Default::default() });
    r.server.add("/t/b", Route { body: b, ..Default::default() });
    let key = r.engine.load(t("a"), "ia".into(), 0, true).unwrap();
    r.engine.set_next(Some((t("b"), "ib".into())), key);
    collect_until(&mut r.events, is_drained, Duration::from_secs(20)).await;

    let exp = |n: usize| (n as u64 * 48_000 + RATE as u64 / 2) / RATE as u64;
    let got = settled_samples(&r.cap, (exp(na) + exp(nb)) as usize * 2).await;
    let frames = got.len() / 2;
    assert_eq!(frames as u64, exp(na) + exp(nb));

    // Largest step anywhere in the steady-state middle of A...
    let left: Vec<f32> = got.iter().step_by(2).copied().collect();
    let max_step = left[1000..exp(na) as usize - 1000]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    // ...and around the join.
    let j = exp(na) as usize;
    let join_step = left[j - 200..j + 200]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        join_step <= max_step * 1.25 + 1e-3,
        "join step {join_step} vs normal {max_step}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_connection_resumes_without_changing_audio() {
    let mut r = rig().await;
    let a = wav_tone(RATE, 2, RATE as usize * 3, 440.0, 0.5);
    r.server.add(
        "/t/a",
        Route {
            body: a.clone(),
            drop_after: Some(100_000),
            drops: 3,
            ..Default::default()
        },
    );
    r.engine.load(t("a"), "ia".into(), 0, true).unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(20)).await;
    let (want, _, _) = decode_all(&a, "wav");
    assert_same(&settled_samples(&r.cap, want.len()).await, &want);
    let ranges = r.server.state.ranges.lock().clone();
    assert!(
        ranges.iter().filter(|r| !r.starts_with("bytes=0-")).count() >= 3,
        "expected resumed range requests, got {ranges:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_source_resumes_byte_exact() {
    let server = TestServer::start().await;
    let body: Vec<u8> = (0..700_000u32).map(|i| (i * 31 % 251) as u8).collect();
    server.add(
        "/f",
        Route {
            body: body.clone(),
            drop_after: Some(64_000),
            drops: 5,
            ..Default::default()
        },
    );
    let src = StreamSource {
        url: server.url("/f"),
        ..Default::default()
    };
    let rt = tokio::runtime::Handle::current();
    let got = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Seek, SeekFrom};
        let cfg = HttpConfig {
            min_backoff: Duration::from_millis(5),
            ..Default::default()
        };
        let mut s = HttpSource::open(rt, http_client(), src, cfg, TransferGate::new(), Priority::Current.cell()).unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).unwrap();
        // Seek back and far forward.
        s.seek(SeekFrom::Start(10)).unwrap();
        let mut b = [0u8; 4];
        s.read_exact(&mut b).unwrap();
        s.seek(SeekFrom::End(-4)).unwrap();
        let mut e = [0u8; 4];
        s.read_exact(&mut e).unwrap();
        let st = s.stats();
        (all, b, e, st)
    })
    .await
    .unwrap();
    assert_eq!(got.0, body);
    assert_eq!(&got.1, &body[10..14]);
    assert_eq!(&got.2, &body[body.len() - 4..]);
    assert!(got.3.resumes >= 5, "resumes {}", got.3.resumes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_gone_for_good_fails_after_retry_window() {
    let server = TestServer::start().await;
    let body = vec![7u8; 300_000];
    server.add(
        "/f",
        Route {
            body,
            drop_after: Some(50_000),
            drops: 1,
            ..Default::default()
        },
    );
    let url = server.url("/f");
    let rt = tokio::runtime::Handle::current();
    let state = server.state.clone();
    let res = tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let cfg = HttpConfig {
            min_backoff: Duration::from_millis(5),
            max_backoff: Duration::from_millis(20),
            retry_window: Duration::from_millis(300),
            ..Default::default()
        };
        let mut s = HttpSource::open(
            rt,
            http_client(),
            StreamSource {
                url,
                ..Default::default()
            },
            cfg,
            TransferGate::new(),
            Priority::Current.cell(),
        )
        .unwrap();
        // The server "goes away" after the first response.
        state.routes.lock().clear();
        let mut all = Vec::new();
        s.read_to_end(&mut all)
    })
    .await
    .unwrap();
    assert!(res.is_err(), "should fail once the file is gone");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undecodable_original_falls_back_to_transcode() {
    let mut r = rig().await;
    let garbage: Vec<u8> = (0..60_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    let good = wav_tone(RATE, 2, 20_000, 500.0, 0.5);
    r.server.add("/t/a", Route { body: garbage, ..Default::default() });
    r.server.add("/x/a", Route { body: good.clone(), ..Default::default() });
    let mut tr = t("a");
    tr.codec = Some("flac".into());
    tr.container = Some("flac".into());
    r.engine.load(tr, "ia".into(), 0, true).unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(20)).await;
    assert_eq!(r.resolver.transcode_calls.load(Ordering::SeqCst), 1);
    let (want, _, _) = decode_all(&good, "wav");
    assert_same(&settled_samples(&r.cap, want.len()).await, &want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transcode_also_failing_reports_failure() {
    let mut r = rig().await;
    r.server.add("/t/a", Route { body: vec![1u8; 5000], ..Default::default() });
    r.server.add("/x/a", Route { body: vec![2u8; 5000], ..Default::default() });
    let key = r.engine.load(t("a"), "ia".into(), 0, true).unwrap();
    let ev = collect_until(
        &mut r.events,
        |e| matches!(e, EngineEvent::Failed { .. }),
        Duration::from_secs(10),
    )
    .await;
    match ev.last().unwrap() {
        EngineEvent::Failed { key: k, track_id, error, .. } => {
            assert_eq!(*k, key);
            assert_eq!(track_id, "a");
            assert!(matches!(error, AppError::Decode(_)), "{error:?}");
        }
        _ => unreachable!(),
    }
    assert_eq!(r.resolver.transcode_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_file_fails_without_transcode() {
    let mut r = rig().await;
    r.engine.load(t("nope"), "i".into(), 0, true).unwrap();
    let ev = collect_until(
        &mut r.events,
        |e| matches!(e, EngineEvent::Failed { .. }),
        Duration::from_secs(10),
    )
    .await;
    assert!(matches!(
        ev.last().unwrap(),
        EngineEvent::Failed { error: AppError::NotFound(_), .. }
    ));
    assert_eq!(r.resolver.transcode_calls.load(Ordering::SeqCst), 0);
}

async fn wait_buffered(engine: &Engine, ms: u64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(p) = engine.position()
            && p.buffered_ms >= ms
        {
            return;
        }
        assert!(Instant::now() < deadline, "never buffered {ms} ms");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seek_within_buffer_is_sample_accurate() {
    let mut config = cfg();
    config.buffer_ahead_ms = 10_000;
    let mut r = rig_with(RATE, config, None).await;
    let a = wav_tone(RATE, 2, RATE as usize * 3, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    r.engine.load(t("a"), "ia".into(), 0, false).unwrap();
    wait_buffered(&r.engine, 2_500).await;
    assert!(r.engine.seek(1_500));
    assert_eq!(r.engine.position().unwrap().position_ms, 1_500);
    r.engine.play().unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(10)).await;
    let (want, _, _) = decode_all(&a, "wav");
    let from = (1_500 * RATE as usize / 1000) * 2;
    assert_same(&settled_samples(&r.cap, want.len() - from).await, &want[from..]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seek_beyond_buffer_uses_range_and_is_sample_accurate() {
    let mut config = cfg();
    config.buffer_ahead_ms = 200;
    config.http.max_ahead = 64 * 1024;
    let mut r = rig_with(RATE, config, None).await;
    let a = wav_tone(RATE, 2, RATE as usize * 6, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    let mut tr = t("a");
    tr.duration_ms = 6_000;
    r.engine.load(tr, "ia".into(), 0, false).unwrap();
    wait_buffered(&r.engine, 150).await;
    let started = Instant::now();
    assert!(r.engine.seek(4_321));
    // Wait for audio at the new position to be ready.
    loop {
        let p = r.engine.position().unwrap();
        if p.position_ms == 4_321 && p.buffered_ms > 4_400 {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let latency = started.elapsed();
    assert!(latency < Duration::from_millis(2_000), "seek took {latency:?}");
    r.engine.play().unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(10)).await;
    let (want, _, _) = decode_all(&a, "wav");
    let from = (4_321 * RATE as usize / 1000) * 2;
    assert_same(&settled_samples(&r.cap, want.len() - from).await, &want[from..]);
    let ranges = r.server.state.ranges.lock().clone();
    assert!(ranges.iter().any(|x| x != "bytes=0-"), "{ranges:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_position_and_back_seek() {
    let mut r = rig().await;
    let a = wav_tone(RATE, 2, RATE as usize * 3, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    // Restore-style start at 2 s, then seek back to 0.5 s.
    r.engine.load(t("a"), "ia".into(), 2_000, false).unwrap();
    wait_buffered(&r.engine, 2_500).await;
    assert_eq!(r.engine.position().unwrap().position_ms, 2_000);
    r.engine.seek(500);
    r.engine.play().unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(10)).await;
    let (want, _, _) = decode_all(&a, "wav");
    let from = (500 * RATE as usize / 1000) * 2;
    assert_same(&settled_samples(&r.cap, want.len() - from).await, &want[from..]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_loss_pauses_and_play_reopens() {
    let mut r = rig().await;
    let a = wav_tone(RATE, 2, 40_000, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    r.engine.load(t("a"), "ia".into(), 0, false).unwrap();
    r.cap.lose_device();
    collect_until(
        &mut r.events,
        |e| matches!(e, EngineEvent::DeviceLost { .. }),
        Duration::from_secs(5),
    )
    .await;
    assert!(!r.engine.is_playing());
    r.engine.close_output();
    r.engine.play().unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(10)).await;
    let (want, _, _) = decode_all(&a, "wav");
    assert_same(&settled_samples(&r.cap, want.len()).await, &want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_ramps_down_and_resume_continues_seamlessly() {
    let mut config = cfg();
    config.ramp_ms = 10;
    // Real-time pacing so the pause lands mid-track on any machine.
    let mut r = rig_paced(RATE, config).await;
    let total = RATE as usize * 2;
    let a = wav_tone(RATE, 2, total, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    r.engine.load(t("a"), "ia".into(), 0, true).unwrap();
    // Let some audio play, then pause.
    let deadline = Instant::now() + Duration::from_secs(20);
    while r.cap.len_frames(2) < 4_000 {
        assert!(Instant::now() < deadline, "playback never started");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    r.engine.pause();
    // The ramp finishes, then nothing more is consumed.
    let at_pause = wait_capture_still(&r.cap).await / 2;
    assert!(at_pause < total, "paused before the end");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(r.cap.len_frames(2), at_pause, "nothing consumed while paused");
    r.engine.play().unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(20)).await;
    // Every source frame was played exactly once (ramps scale, never skip).
    assert_eq!(settled_samples(&r.cap, total * 2).await.len(), total * 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plays_from_disk_cache_when_server_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Arc::new(super::cache::DiskCache::new(dir.path(), 50 << 20).unwrap());
    let a = wav_tone(RATE, 2, 20_000, 440.0, 0.5);
    let tr = t("a");
    cache.insert_bytes(&super::cache::cache_key(&tr), &a).unwrap();
    let mut r = rig_with(RATE, cfg(), Some(cache)).await;
    // No route on the server at all: only the cache can serve it.
    r.engine.load(tr, "ia".into(), 0, true).unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(10)).await;
    let (want, _, _) = decode_all(&a, "wav");
    assert_same(&settled_samples(&r.cap, want.len()).await, &want);
    assert_eq!(r.server.state.requests.load(Ordering::SeqCst), 0);
}

/// Downloaded files, by track id.
struct Downloaded(std::collections::HashMap<String, std::path::PathBuf>);

impl super::source::LocalAudio for Downloaded {
    fn path(&self, track: &Track) -> Option<std::path::PathBuf> {
        self.0.get(&track.id).cloned()
    }
    fn ids(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plays_a_downloaded_file_before_the_cache_or_server() {
    let dir = tempfile::tempdir().unwrap();
    let a = wav_tone(RATE, 2, 20_000, 440.0, 0.5);
    // A different tone sits in the pre-cache and on the server.
    let b = wav_tone(RATE, 2, 20_000, 220.0, 0.5);
    let tr = t("a");
    let file = dir.path().join("a.wav");
    std::fs::write(&file, &a).unwrap();
    let cache = Arc::new(super::cache::DiskCache::new(dir.path().join("cache"), 50 << 20).unwrap());
    cache.insert_bytes(&super::cache::cache_key(&tr), &b).unwrap();
    let local = super::source::LocalFiles(Arc::new(Downloaded([("a".to_string(), file)].into_iter().collect())));
    let mut r = rig_local(RATE, cfg(), Some(cache), Some(local), false).await;
    r.server.add("/t/a", Route { body: b.clone(), ..Default::default() });
    r.engine.load(tr, "ia".into(), 0, true).unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(10)).await;
    let (want, _, _) = decode_all(&a, "wav");
    assert_same(&settled_samples(&r.cap, want.len()).await, &want);
    assert_eq!(r.server.state.requests.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fully_streamed_track_is_saved_to_cache() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Arc::new(super::cache::DiskCache::new(dir.path(), 50 << 20).unwrap());
    let mut r = rig_with(RATE, cfg(), Some(cache.clone())).await;
    let a = wav_tone(RATE, 2, 20_000, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    let tr = t("a");
    r.engine.load(tr.clone(), "ia".into(), 0, true).unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(10)).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cache.contains(&super::cache::cache_key(&tr)) {
        assert!(Instant::now() < deadline, "not cached");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        std::fs::read(cache.get(&super::cache::cache_key(&tr)).unwrap()).unwrap(),
        a
    );
    let _ = Origin::Cache;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn precacher_downloads_wanted_tracks_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Arc::new(super::cache::DiskCache::new(dir.path(), 50 << 20).unwrap());
    let server = TestServer::start().await;
    let resolver = FakeResolver::new(&server);
    let a = wav_tone(RATE, 2, 10_000, 440.0, 0.5);
    let b = wav_tone(RATE, 2, 12_000, 550.0, 0.5);
    server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    server.add("/t/b", Route { body: b.clone(), ..Default::default() });
    let pre = super::cache::Precacher::spawn(
        &tokio::runtime::Handle::current(),
        cache.clone(),
        resolver,
        http_client(),
        TransferGate::new(),
        None,
    );
    // "missing" 404s and is dropped from the wish list; the rest still land.
    pre.set_wanted(vec![t("a"), t("missing"), t("b")]);
    let deadline = Instant::now() + Duration::from_secs(10);
    for (tr, body) in [(t("a"), &a), (t("b"), &b)] {
        let key = super::cache::cache_key(&tr);
        while !cache.contains(&key) {
            assert!(Instant::now() < deadline, "{} not cached", tr.id);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(&std::fs::read(cache.get(&key).unwrap()).unwrap(), body);
    }
    assert!(!cache.contains(&super::cache::cache_key(&t("missing"))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seek_preempts_a_precache_transfer_on_a_one_transfer_server() {
    let mut config = cfg();
    config.buffer_ahead_ms = 200;
    config.http.max_ahead = 64 * 1024;
    let mut r = rig_with(RATE, config, None).await;
    r.server.state.exclusive.store(true, Ordering::SeqCst);
    let a = wav_tone(RATE, 2, RATE as usize * 6, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    // A slow background download (~2.5 s if left alone).
    let big: Vec<u8> = (0..1_048_576u32).map(|i| (i % 253) as u8).collect();
    r.server.add(
        "/t/big",
        Route {
            body: big.clone(),
            chunk_delay: Some(Duration::from_millis(40)),
            ..Default::default()
        },
    );
    let dir = tempfile::tempdir().unwrap();
    let cache = Arc::new(super::cache::DiskCache::new(dir.path(), 50 << 20).unwrap());
    let pre = super::cache::Precacher::spawn(
        &tokio::runtime::Handle::current(),
        cache.clone(),
        r.resolver.clone(),
        http_client(),
        r.gate.clone(),
        Some(r.engine.precache_host()),
    );

    let mut tr = t("a");
    tr.duration_ms = 6_000;
    r.engine.load(tr, "ia".into(), 0, false).unwrap();
    wait_buffered(&r.engine, 150).await;
    pre.set_wanted(vec![t("big")]);
    // Wait until the background download is under way.
    let deadline = Instant::now() + Duration::from_secs(20);
    while r.server.state.ranges_for("/t/big").is_empty() {
        assert!(Instant::now() < deadline, "precache never started");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(r.gate.holder_priority(), Some(Priority::Precache as u8));

    // A far seek needs the network now: it must not wait for the download.
    let started = Instant::now();
    assert!(r.engine.seek(4_321));
    loop {
        let p = r.engine.position().unwrap();
        if p.position_ms == 4_321 && p.buffered_ms > 4_400 {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "seek never buffered");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let latency = started.elapsed();
    assert!(latency < Duration::from_millis(1_500), "seek waited {latency:?}");
    assert!(r.gate.preemptions() >= 1);
    assert_eq!(r.server.state.killed.load(Ordering::SeqCst), 0);

    // Playback from the seek point is exact.
    r.engine.play().unwrap();
    collect_until(&mut r.events, is_drained, Duration::from_secs(20)).await;
    let (want, _, _) = decode_all(&a, "wav");
    let from = (4_321 * RATE as usize / 1000) * 2;
    assert_same(&settled_samples(&r.cap, want.len() - from).await, &want[from..]);

    // The download resumed by range and completed intact.
    let key = super::cache::cache_key(&t("big"));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cache.contains(&key) {
        assert!(Instant::now() < deadline, "precache never finished");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(std::fs::read(cache.get(&key).unwrap()).unwrap(), big);
    let ranges = r.server.state.ranges_for("/t/big");
    assert!(
        ranges.iter().any(|x| x.as_deref().is_some_and(|x| x != "bytes=0-")),
        "{ranges:?}"
    );
}

// ------------------------------------------------------------ A-B loop

const fn frames(ms: usize) -> usize {
    ms * RATE as usize / 1000
}

/// `all[from..to]` in ms (stereo samples).
fn span(all: &[f32], from_ms: usize, to_ms: usize) -> &[f32] {
    &all[frames(from_ms) * 2..frames(to_ms) * 2]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ab_loop_set_before_a_replays_sample_exact_with_no_gap() {
    let r = rig().await;
    let a = wav_tone(RATE, 2, RATE as usize * 2, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    let key = r.engine.load(t("a"), "ia".into(), 0, true).unwrap();
    assert!(r.engine.set_loop(key, Some((500, 1_500))));
    let (all, _, _) = decode_all(&a, "wav");
    let mut want = span(&all, 0, 1_500).to_vec();
    for _ in 0..3 {
        want.extend_from_slice(span(&all, 500, 1_500));
    }
    let got = settled_samples(&r.cap, want.len()).await;
    assert_same(&got[..want.len()], &want);
    // Still looping: the position stays inside the span.
    let p = r.engine.position().unwrap().position_ms;
    assert!((500..=1_500).contains(&p), "position {p}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ab_loop_set_inside_the_span_goes_back_to_a_then_keeps_looping() {
    let r = rig().await;
    let a = wav_tone(RATE, 2, RATE as usize * 2, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    let key = r.engine.load(t("a"), "ia".into(), 1_000, true).unwrap();
    assert!(r.engine.set_loop(key, Some((500, 1_500))));
    let (all, _, _) = decode_all(&a, "wav");
    // The first wrap re-decodes from A; later ones replay from memory.
    let mut want = span(&all, 1_000, 1_500).to_vec();
    for _ in 0..3 {
        want.extend_from_slice(span(&all, 500, 1_500));
    }
    let got = settled_samples(&r.cap, want.len()).await;
    assert_same(&got[..want.len()], &want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ab_loop_with_b_past_the_audio_wraps_at_the_end() {
    let r = rig().await;
    // The tags say 3 s; the file holds 2 s.
    let a = wav_tone(RATE, 2, RATE as usize * 2, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    let key = r.engine.load(t("a"), "ia".into(), 0, true).unwrap();
    assert!(r.engine.set_loop(key, Some((500, 2_900))));
    let (all, _, _) = decode_all(&a, "wav");
    let mut want = span(&all, 0, 2_000).to_vec();
    for _ in 0..2 {
        want.extend_from_slice(span(&all, 500, 2_000));
    }
    let got = settled_samples(&r.cap, want.len()).await;
    assert_same(&got[..want.len()], &want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clearing_the_loop_plays_on_to_the_end() {
    let mut r = rig().await;
    let a = wav_tone(RATE, 2, RATE as usize * 2, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    let key = r.engine.load(t("a"), "ia".into(), 0, true).unwrap();
    assert!(r.engine.set_loop(key, Some((500, 1_500))));
    let (all, _, _) = decode_all(&a, "wav");
    // Let it loop at least twice, then clear.
    let two = frames(1_500) * 2 + 2 * frames(1_000) * 2;
    settled_samples(&r.cap, two).await;
    assert!(r.engine.set_loop(key, None));
    collect_until(&mut r.events, is_drained, Duration::from_secs(20)).await;
    let n = wait_capture_still(&r.cap).await;
    let got = r.cap.samples();
    let got = &got[..n];
    let tail = span(&all, 1_500, 2_000);
    assert!(got.len() >= two + tail.len());
    assert_same(&got[got.len() - tail.len()..], tail);
    // Everything before was the first pass and then whole or partial passes
    // of the loop, all the track's own audio.
    let body = &got[..got.len() - tail.len()];
    let first = span(&all, 0, 1_500);
    assert_same(&body[..first.len()], first);
    let pass = span(&all, 500, 1_500);
    for chunk in body[first.len()..].chunks(pass.len()) {
        assert_same(chunk, &pass[..chunk.len()]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seeking_during_a_replay_lands_on_the_right_audio() {
    let mut r = rig_paced(RATE, cfg()).await;
    let a = wav_tone(RATE, 2, RATE as usize * 3, 440.0, 0.5);
    r.server.add("/t/a", Route { body: a.clone(), ..Default::default() });
    let key = r.engine.load(t("a"), "ia".into(), 0, true).unwrap();
    assert!(r.engine.set_loop(key, Some((200, 700))));
    // Wait until the second pass (a replay) is under way.
    let deadline = Instant::now() + Duration::from_secs(10);
    while r.cap.len_frames(2) < frames(700) + frames(500) + frames(100) {
        assert!(Instant::now() < deadline, "never looped");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let p = r.engine.position().unwrap().position_ms;
    assert!((200..=700).contains(&p), "position {p}");
    r.engine.set_loop(key, None);
    r.engine.seek(2_000);
    r.cap.clear();
    collect_until(&mut r.events, is_drained, Duration::from_secs(10)).await;
    let (all, _, _) = decode_all(&a, "wav");
    let want = span(&all, 2_000, 3_000);
    let got = settled_samples(&r.cap, want.len()).await;
    // A block may have been rendered before the clear.
    assert!(got.len() >= want.len());
    assert_same(&got[got.len() - want.len()..], want);
}
