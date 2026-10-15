//! Controller tests: commands → engine → events, with a capture output and
//! a local HTTP server. Covers the queue/playback rules end to end
//! and the play tracking signals.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::broadcast;

use super::engine::EngineConfig;
use super::events::{
    AbLoop, ErrorPayload, EventSink, OutputLost, PlayerState, PlayerStatus, PositionPayload,
    SKIPPED_MESSAGE, TrackingEvent,
};
use super::http::HttpConfig;
use super::output::{CaptureHandle, CaptureOutput, OutputDevice, OutputFormat, OutputState};
use super::player::{PlayerConfig, PlayerHandle};
use super::queue::{AddAt, Repeat};
use super::source::{ContextKind, PlayContext};
use super::testutil::*;
use crate::model::Track;

const RATE: u32 = 44_100;

#[derive(Default)]
struct Sink {
    states: Mutex<Vec<PlayerState>>,
    positions: Mutex<Vec<PositionPayload>>,
    errors: Mutex<Vec<ErrorPayload>>,
    outputs: Mutex<Vec<OutputState>>,
    lost: Mutex<Vec<String>>,
}

impl EventSink for Sink {
    fn output(&self, s: &OutputState) {
        self.outputs.lock().push(s.clone());
    }
    fn output_lost(&self, l: &OutputLost) {
        self.lost.lock().push(l.device.clone());
    }
    fn state(&self, s: &PlayerState) {
        self.states.lock().push(s.clone());
    }
    fn position(&self, p: &PositionPayload) {
        self.positions.lock().push(*p);
    }
    fn error(&self, e: &ErrorPayload) {
        self.errors.lock().push(e.clone());
    }
}

struct Rig {
    server: TestServer,
    resolver: Arc<FakeResolver>,
    player: PlayerHandle,
    sink: Arc<Sink>,
    cap: CaptureHandle,
    tracking: broadcast::Receiver<TrackingEvent>,
    bodies: Vec<(String, Vec<u8>)>,
}

fn engine_cfg() -> EngineConfig {
    EngineConfig {
        buffer_ahead_ms: 2_000,
        ramp_ms: 0,
        declick_ms: 0,
        http: HttpConfig {
            min_backoff: Duration::from_millis(10),
            retry_window: Duration::from_secs(2),
            ..Default::default()
        },
    }
}

/// Tracks "a".."" each `secs` long (distinct tones), served at /t/{id}.
async fn rig(ids: &[&str], secs: f32, realtime: bool, config: PlayerConfig) -> Rig {
    let server = TestServer::start().await;
    let resolver = FakeResolver::new(&server);
    let mut bodies = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let frames = (secs * RATE as f32) as usize;
        let body = wav_tone(RATE, 2, frames, 220.0 + 110.0 * i as f32, 0.5);
        server.add(&format!("/t/{id}"), Route { body: body.clone(), ..Default::default() });
        let mut t = track(id, if i % 2 == 0 { "A" } else { "B" }, &format!("/p/{id}"));
        t.duration_ms = (secs * 1000.0) as u64;
        resolver.tracks.lock().insert(id.to_string(), t);
        bodies.push((id.to_string(), body));
    }
    let (out, cap) = CaptureOutput::new(OutputFormat {
        sample_rate: RATE,
        channels: 2,
    });
    let out = if realtime { out.realtime() } else { out };
    let sink = Arc::new(Sink::default());
    let player = PlayerHandle::spawn(
        tokio::runtime::Handle::current(),
        resolver.clone(),
        resolver.clone(),
        Box::new(out),
        sink.clone(),
        PlayerConfig {
            engine: engine_cfg(),
            seed: Some(1),
            position_interval: Duration::from_millis(50),
            persist_debounce: Duration::from_millis(20),
            ..config
        },
    );
    let tracking = player.subscribe_tracking();
    Rig {
        server,
        resolver,
        player,
        sink,
        cap,
        tracking,
        bodies,
    }
}

fn ctx(ids: &[&str]) -> PlayContext {
    PlayContext {
        kind: ContextKind::Tracks,
        id: None,
        track_ids: Some(ids.iter().map(|s| s.to_string()).collect()),
    }
}

async fn next_tracking(
    rx: &mut broadcast::Receiver<TrackingEvent>,
    pred: impl Fn(&TrackingEvent) -> bool,
) -> TrackingEvent {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Ok(e)) if pred(&e) => return e,
            Ok(Ok(_)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            _ => panic!("timed out waiting for tracking event"),
        }
    }
}

fn started(id: &'static str) -> impl Fn(&TrackingEvent) -> bool {
    move |e| matches!(e, TrackingEvent::TrackStarted { track, .. } if track.id == id)
}

fn ended(id: &'static str) -> impl Fn(&TrackingEvent) -> bool {
    move |e| matches!(e, TrackingEvent::TrackEnded { track, .. } if track.id == id)
}

async fn wait_state(player: &PlayerHandle, pred: impl Fn(&PlayerState) -> bool) -> PlayerState {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let s = player.state().await.unwrap();
        if pred(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "state never matched; last {s:?}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn concat(r: &Rig, ids: &[&str]) -> Vec<f32> {
    let mut out = Vec::new();
    for id in ids {
        let body = &r.bodies.iter().find(|b| b.0 == *id).unwrap().1;
        out.extend(decode_all(body, "wav").0);
    }
    out
}

fn tid(t: &Option<Track>) -> String {
    t.as_ref().map(|t| t.id.clone()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plays_a_context_gaplessly_with_tracking_signals() {
    let mut r = rig(&["a", "b", "c"], 0.3, false, PlayerConfig::default()).await;
    r.player
        .play_context(ctx(&["a", "b", "c"]), None, Some(false))
        .await
        .unwrap();
    let mut seq = Vec::new();
    loop {
        let e = next_tracking(&mut r.tracking, |_| true).await;
        match &e {
            TrackingEvent::TrackStarted { track, context, .. } => {
                assert_eq!(context.as_ref().unwrap().kind, ContextKind::Tracks);
                seq.push(format!("start {}", track.id));
            }
            TrackingEvent::TrackEnded {
                track,
                completed,
                skipped_at_ms,
                listened_ms,
                ..
            } => {
                assert!(*completed);
                assert!(skipped_at_ms.is_none());
                assert_eq!(*listened_ms, 300);
                seq.push(format!("end {}", track.id));
                if track.id == "c" {
                    break;
                }
            }
            _ => {}
        }
    }
    assert_eq!(
        seq,
        vec!["start a", "end a", "start b", "end b", "start c", "end c"]
    );
    let s = wait_state(&r.player, |s| s.status == PlayerStatus::Paused).await;
    // The context ended (repeat off): back to its first track, paused.
    assert_eq!(tid(&s.track), "a");
    assert_eq!(s.position_ms, 0);
    let want = concat(&r, &["a", "b", "c"]);
    assert_eq!(settled_samples(&r.cap, want.len()).await, want);
    let hist: Vec<String> = s.queue.history.iter().map(|i| i.track.id.clone()).collect();
    assert_eq!(hist, vec!["c", "b", "a"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_track_and_user_queue_order() {
    let mut r = rig(&["a", "b", "c", "q1", "q2"], 0.2, false, PlayerConfig::default()).await;
    // Queue before starting: add-to-queue then play-next.
    // Queued into an empty player: q2 becomes the (paused) current entry.
    r.player.queue_add(vec!["q2".into()], AddAt::End).await.unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!(s.status, PlayerStatus::Paused);
    assert_eq!(tid(&s.track), "q2");
    r.player.queue_add(vec!["q1".into()], AddAt::Next).await.unwrap();
    // A new context keeps the user queue and starts at the chosen track.
    r.player
        .play_context(ctx(&["a", "b", "c"]), Some("b".into()), Some(false))
        .await
        .unwrap();
    let mut order = Vec::new();
    while order.len() < 3 {
        if let TrackingEvent::TrackStarted { track, .. } =
            next_tracking(&mut r.tracking, |e| matches!(e, TrackingEvent::TrackStarted { .. })).await
        {
            order.push(track.id);
        }
    }
    assert_eq!(order, vec!["b", "q1", "c"]);
    assert_eq!(r.resolver.transcode_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn play_next_beats_add_to_queue_beats_context() {
    let mut r = rig(&["a", "b", "c", "e1", "n1"], 1.5, true, PlayerConfig::default()).await;
    r.player
        .play_context(ctx(&["a", "b", "c"]), None, Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    r.player.queue_add(vec!["e1".into()], AddAt::End).await.unwrap();
    r.player.queue_add(vec!["n1".into()], AddAt::Next).await.unwrap();
    let s = r.player.state().await.unwrap();
    let uq: Vec<String> = s.queue.user_queue.iter().map(|i| i.track.id.clone()).collect();
    assert_eq!(uq, vec!["n1", "e1"]);
    let nu: Vec<String> = s.queue.next_up.iter().map(|i| i.track.id.clone()).collect();
    assert_eq!(nu, vec!["b", "c"]);
    // Skip through and watch the order.
    let mut order = vec![];
    for _ in 0..3 {
        r.player.next().await.unwrap();
        if let TrackingEvent::TrackStarted { track, .. } =
            next_tracking(&mut r.tracking, |e| matches!(e, TrackingEvent::TrackStarted { .. })).await
        {
            order.push(track.id);
        }
    }
    assert_eq!(order, vec!["n1", "e1", "b"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn next_reports_a_skip_and_previous_follows_the_3s_rule() {
    let mut r = rig(&["a", "b"], 5.0, true, PlayerConfig::default()).await;
    r.player
        .play_context(ctx(&["a", "b"]), None, Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    r.player.next().await.unwrap();
    match next_tracking(&mut r.tracking, ended("a")).await {
        TrackingEvent::TrackEnded {
            completed,
            skipped_at_ms,
            ..
        } => {
            assert!(!completed);
            assert!(skipped_at_ms.is_some());
        }
        _ => unreachable!(),
    }
    next_tracking(&mut r.tracking, started("b")).await;
    // Past 3 s: previous restarts the track.
    r.player.pause().await.unwrap();
    r.player.seek(3_500).await.unwrap();
    match next_tracking(&mut r.tracking, |e| matches!(e, TrackingEvent::Seeked { .. })).await {
        TrackingEvent::Seeked { to_ms, .. } => assert_eq!(to_ms, 3_500),
        _ => unreachable!(),
    }
    r.player.previous().await.unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!(tid(&s.track), "b");
    assert_eq!(s.position_ms, 0);
    // Within 3 s: previous goes back to "a".
    r.player.previous().await.unwrap();
    let s = wait_state(&r.player, |s| tid(&s.track) == "a").await;
    assert_eq!(s.position_ms, 0);
    next_tracking(&mut r.tracking, started("a")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_file_is_skipped_with_error_and_problem_logged() {
    let mut r = rig(&["a", "b", "c"], 0.3, false, PlayerConfig::default()).await;
    r.server.state.routes.lock().remove("/t/b");
    r.player
        .play_context(ctx(&["a", "b", "c"]), None, Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, ended("c")).await;
    let errs = r.sink.errors.lock().clone();
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!(errs[0].message, SKIPPED_MESSAGE);
    assert_eq!(errs[0].track_id.as_deref(), Some("b"));
    assert_eq!(errs[0].kind, "notFound");
    let probs = r.player.problems();
    assert_eq!(probs.len(), 1);
    assert_eq!(probs[0].track_id, "b");
    // a then c, back to back.
    let want = concat(&r, &["a", "c"]);
    assert_eq!(settled_samples(&r.cap, want.len()).await, want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_first_track_skips_to_the_next() {
    let mut r = rig(&["a", "b"], 0.3, false, PlayerConfig::default()).await;
    r.server.state.routes.lock().remove("/t/a");
    r.player
        .play_context(ctx(&["a", "b"]), None, Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, ended("b")).await;
    assert_eq!(r.player.problems().len(), 1);
    let want = concat(&r, &["b"]);
    assert_eq!(settled_samples(&r.cap, want.len()).await, want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_transfer_server_plays_queue_with_precache() {
    let dir = tempfile::tempdir().unwrap();
    let cache_dir = dir.path().join("cache");
    let mut r = rig(
        &["a", "b", "c"],
        1.0,
        true,
        PlayerConfig {
            cache_dir: Some(cache_dir.clone()),
            precache_count: 5,
            ..Default::default()
        },
    )
    .await;
    // The server aborts any transfer when a newer one starts, and is slowish.
    r.server.state.exclusive.store(true, std::sync::atomic::Ordering::SeqCst);
    for route in r.server.state.routes.lock().values_mut() {
        route.chunk_delay = Some(Duration::from_millis(15));
    }
    r.player
        .play_context(ctx(&["a", "b", "c"]), None, Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, ended("c")).await;
    let want = concat(&r, &["a", "b", "c"]);
    assert_eq!(settled_samples(&r.cap, want.len()).await, want);
    assert!(r.sink.errors.lock().is_empty(), "{:?}", r.sink.errors.lock());
    assert!(r.player.problems().is_empty());
    // Never two transfers at once: the server never had to cut one off.
    assert_eq!(r.server.state.killed.load(std::sync::atomic::Ordering::SeqCst), 0);

    // Whatever landed in the cache is complete and intact, and the upcoming
    // tracks were pre-cached while "a" played.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let names: Vec<String> = std::fs::read_dir(&cache_dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let has = |id: &str| names.iter().any(|n| n.starts_with(&format!("{id}-")) && n.ends_with(".audio"));
        if has("b") && has("c") && !names.iter().any(|n| n.ends_with(".part")) {
            break;
        }
        assert!(Instant::now() < deadline, "cache incomplete: {names:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for (id, body) in &r.bodies {
        let path = cache_dir.join(format!(
            "{}.audio",
            super::cache::cache_key(r.resolver.tracks.lock().get(id).unwrap())
        ));
        if path.exists() {
            assert_eq!(&std::fs::read(&path).unwrap(), body, "cached {id}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn precache_count_setting_limits_how_far_ahead_is_fetched() {
    let dir = tempfile::tempdir().unwrap();
    let cache_dir = dir.path().join("cache");
    let mut r = rig(
        &["a", "b", "c", "d", "e"],
        8.0,
        true,
        PlayerConfig {
            cache_dir: Some(cache_dir.clone()),
            ..Default::default()
        },
    )
    .await;
    let cached = |id: &str| {
        std::fs::read_dir(&cache_dir).into_iter().flatten().flatten().any(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with(&format!("{id}-")) && n.ends_with(".audio")
        })
    };
    let wait_for = |ids: &'static [&'static str]| {
        let cached = &cached;
        async move {
            let deadline = Instant::now() + Duration::from_secs(15);
            while !ids.iter().all(|id| cached(id)) {
                assert!(Instant::now() < deadline, "never cached {ids:?}");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    };

    r.player.set_precache_count(1).await.unwrap();
    r.player
        .play_context(ctx(&["a", "b", "c", "d", "e"]), None, Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    wait_for(&["b"]).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(!cached("c"), "only one track ahead");

    r.player.set_precache_count(3).await.unwrap();
    wait_for(&["c", "d"]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!cached("e"), "three tracks ahead, not four");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeat_one_replays_the_same_entry() {
    let mut r = rig(&["a", "b"], 0.2, false, PlayerConfig::default()).await;
    r.player.set_repeat(Repeat::One).await.unwrap();
    r.player
        .play_context(ctx(&["a", "b"]), None, Some(false))
        .await
        .unwrap();
    let mut starts = Vec::new();
    while starts.len() < 3 {
        if let TrackingEvent::TrackStarted { track, item_id, .. } =
            next_tracking(&mut r.tracking, |e| matches!(e, TrackingEvent::TrackStarted { .. })).await
        {
            starts.push((track.id, item_id));
        }
    }
    assert!(starts.iter().all(|s| s == &starts[0]), "{starts:?}");
    r.player.set_repeat(Repeat::Off).await.unwrap();
    next_tracking(&mut r.tracking, started("b")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_and_position_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.json");
    let cfg = || PlayerConfig {
        persist_path: Some(path.clone()),
        ..Default::default()
    };
    let before;
    {
        let mut r = rig(&["a", "b", "c", "u"], 5.0, true, cfg()).await;
        r.player
            .play_context(ctx(&["a", "b", "c"]), Some("b".into()), Some(false))
            .await
            .unwrap();
        next_tracking(&mut r.tracking, started("b")).await;
        r.player.queue_add(vec!["u".into()], AddAt::End).await.unwrap();
        r.player.set_volume(0.4).await.unwrap();
        r.player.pause().await.unwrap();
        r.player.seek(2_345).await.unwrap();
        r.player.flush().await;
        before = r.player.state().await.unwrap();
    }
    let mut r = rig(&["a", "b", "c", "u"], 5.0, true, cfg()).await;
    let after = r.player.state().await.unwrap();
    assert_eq!(after.status, PlayerStatus::Paused);
    assert_eq!(after.queue, before.queue);
    assert_eq!(after.position_ms, 2_345);
    assert_eq!(after.volume, 0.4);
    assert_eq!(tid(&after.track), "b");
    // Resume continues from the saved position.
    r.player.resume().await.unwrap();
    next_tracking(&mut r.tracking, started("b")).await;
    let s = r.player.state().await.unwrap();
    assert!(s.position_ms >= 2_345, "{}", s.position_ms);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistence_is_debounced_to_disk_on_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.json");
    let r = rig(
        &["a", "b"],
        5.0,
        true,
        PlayerConfig {
            persist_path: Some(path.clone()),
            ..Default::default()
        },
    )
    .await;
    r.player.queue_add(vec!["a".into(), "b".into()], AddAt::End).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < deadline, "queue file never written");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let p = super::persist::load(&path).unwrap();
    assert_eq!(p.queue.view().current.unwrap().track.id, "a");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_loss_pauses_and_resume_reopens() {
    let mut r = rig(&["a"], 3.0, true, PlayerConfig::default()).await;
    r.player.play_context(ctx(&["a"]), None, None).await.unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    r.cap.lose_device();
    next_tracking(&mut r.tracking, |e| matches!(e, TrackingEvent::Paused { .. })).await;
    let s = r.player.state().await.unwrap();
    assert_eq!(s.status, PlayerStatus::Paused);
    let frames = r.cap.len_frames(2);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(r.cap.len_frames(2), frames, "silent after device loss");
    r.player.resume().await.unwrap();
    wait_state(&r.player, |s| s.status == PlayerStatus::Playing).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while r.cap.len_frames(2) <= frames {
        assert!(Instant::now() < deadline, "no audio after resume");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn position_events_flow_while_playing_and_volume_state_updates() {
    let mut r = rig(&["a"], 2.0, true, PlayerConfig::default()).await;
    r.player.play_context(ctx(&["a"]), None, None).await.unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while r.sink.positions.lock().len() < 3 {
        assert!(Instant::now() < deadline, "no position events");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pos = r.sink.positions.lock().clone();
    assert!(pos.iter().all(|p| p.duration_ms == 2_000));
    assert!(pos.last().unwrap().buffered_ms >= pos.last().unwrap().position_ms);
    r.player.set_volume(0.25).await.unwrap();
    r.player.set_muted(true).await.unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!(s.volume, 0.25);
    assert!(s.muted);
    next_tracking(&mut r.tracking, |e| matches!(e, TrackingEvent::Progress { .. })).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jump_and_remove_via_item_ids() {
    let mut r = rig(&["a", "b", "c", "d"], 3.0, true, PlayerConfig::default()).await;
    r.player
        .play_context(ctx(&["a", "b", "c", "d"]), None, Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    let s = r.player.state().await.unwrap();
    let b = s.queue.next_up[0].item_id.clone();
    let d = s.queue.next_up[2].item_id.clone();
    r.player.queue_remove(b).await.unwrap();
    r.player.queue_jump(d).await.unwrap();
    next_tracking(&mut r.tracking, started("d")).await;
    let s = r.player.state().await.unwrap();
    assert!(s.queue.next_up.is_empty());
    assert!(r.player.queue_jump("missing".into()).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shuffle_play_starts_with_a_shuffled_context() {
    let ids = ["a", "b", "c", "d", "e", "f"];
    let mut r = rig(&ids, 3.0, true, PlayerConfig::default()).await;
    r.player.play_context(ctx(&ids), None, Some(true)).await.unwrap();
    next_tracking(&mut r.tracking, |e| matches!(e, TrackingEvent::TrackStarted { .. })).await;
    let s = r.player.state().await.unwrap();
    assert!(s.shuffle);
    let mut all: Vec<String> = s.queue.next_up.iter().map(|i| i.track.id.clone()).collect();
    all.push(tid(&s.track));
    all.sort();
    assert_eq!(all, ids.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    r.player.set_shuffle(false).await.unwrap();
    let s = r.player.state().await.unwrap();
    let cur = tid(&s.track);
    let idx = ids.iter().position(|i| *i == cur).unwrap();
    let up: Vec<String> = s.queue.next_up.iter().map(|i| i.track.id.clone()).collect();
    assert_eq!(up, ids[idx + 1..].iter().map(|s| s.to_string()).collect::<Vec<_>>());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shuffle_play_skips_hidden_songs_and_artists_but_not_the_picked_one() {
    let ids = ["a", "b", "c", "d", "e", "f"];
    let mut r = rig(&ids, 3.0, true, PlayerConfig::default()).await;
    let mut hidden = crate::hidden::HiddenSet::default();
    hidden.tracks.insert("c".into());
    hidden.tracks.insert("e".into());
    *r.resolver.hidden.lock() = Some(Arc::new(hidden));
    // Picked directly: "c" plays, the other hidden one is left out.
    r.player.play_context(ctx(&ids), Some("c".into()), Some(true)).await.unwrap();
    next_tracking(&mut r.tracking, |e| matches!(e, TrackingEvent::TrackStarted { .. })).await;
    let s = r.player.state().await.unwrap();
    assert_eq!(tid(&s.track), "c");
    let mut all: Vec<String> = s.queue.next_up.iter().map(|i| i.track.id.clone()).collect();
    all.sort();
    assert_eq!(all, ["a", "b", "d", "f"]);
    // In order (no shuffle), everything plays as listed.
    r.player.play_context(ctx(&ids), None, Some(false)).await.unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!(s.queue.next_up.len(), 5);
}

/// A crash never runs the exit flush: the queue file as it was a moment
/// after the last change must be enough to restore the queue and position.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_without_flush_restores_queue_and_position() {
    let dir = tempfile::tempdir().unwrap();
    let live = dir.path().join("queue.json");
    let mut r = rig(
        &["a", "b", "c"],
        5.0,
        true,
        PlayerConfig {
            persist_path: Some(live.clone()),
            ..Default::default()
        },
    )
    .await;
    r.player
        .play_context(ctx(&["a", "b", "c"]), Some("b".into()), Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, started("b")).await;
    r.player.pause().await.unwrap();
    r.player.seek(1_500).await.unwrap();
    // Wait for the debounced write, then take the file as a crash leaves it.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(p) = super::persist::load(&live)
            && p.position_ms == 1_500
        {
            break;
        }
        assert!(Instant::now() < deadline, "position never persisted");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let crashed = dir.path().join("after-crash.json");
    std::fs::copy(&live, &crashed).unwrap();
    let r2 = rig(
        &["a", "b", "c"],
        5.0,
        true,
        PlayerConfig {
            persist_path: Some(crashed),
            ..Default::default()
        },
    )
    .await;
    let s = r2.player.state().await.unwrap();
    assert_eq!(s.status, PlayerStatus::Paused);
    assert_eq!(tid(&s.track), "b");
    assert_eq!(s.position_ms, 1_500);
    assert_eq!(s.queue.next_up.len(), 1);
}

fn album(id: &str, ids: &[&str]) -> PlayContext {
    PlayContext {
        kind: ContextKind::Album,
        id: Some(id.into()),
        track_ids: Some(ids.iter().map(|s| s.to_string()).collect()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previous_at_exactly_3s_restarts_and_keeps_repeat_one() {
    let mut r = rig(&["a", "b", "c"], 5.0, true, PlayerConfig::default()).await;
    r.player
        .play_context(ctx(&["a", "b", "c"]), Some("b".into()), Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, started("b")).await;
    r.player.set_repeat(Repeat::One).await.unwrap();
    r.player.pause().await.unwrap();
    r.player.seek(3_000).await.unwrap();
    r.player.previous().await.unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!(tid(&s.track), "b");
    assert_eq!(s.position_ms, 0);
    assert_eq!(s.repeat, Repeat::One, "a late Previous keeps repeat-one");
    // Now under 3 s: back to "a", and repeat-one drops to repeat-all.
    r.player.previous().await.unwrap();
    let s = wait_state(&r.player, |s| tid(&s.track) == "a").await;
    assert_eq!(s.repeat, Repeat::All);
    // First track, early Previous: restarts.
    r.player.previous().await.unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!(tid(&s.track), "a");
    assert_eq!(s.position_ms, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previous_on_a_queued_song_drops_it() {
    let mut r = rig(&["a", "b", "u"], 5.0, true, PlayerConfig::default()).await;
    r.player
        .play_context(ctx(&["a", "b"]), None, Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    r.player.queue_add(vec!["u".into()], AddAt::End).await.unwrap();
    r.player.next().await.unwrap();
    next_tracking(&mut r.tracking, started("u")).await;
    r.player.previous().await.unwrap();
    let s = wait_state(&r.player, |s| tid(&s.track) == "a").await;
    assert!(s.queue.user_queue.is_empty());
    assert_eq!(s.queue.next_up.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn next_in_repeat_one_drops_to_repeat_all() {
    let mut r = rig(&["a", "b"], 5.0, true, PlayerConfig::default()).await;
    r.player
        .play_context(ctx(&["a", "b"]), None, Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    r.player.set_repeat(Repeat::One).await.unwrap();
    r.player.next().await.unwrap();
    let s = wait_state(&r.player, |s| tid(&s.track) == "b").await;
    assert_eq!(s.repeat, Repeat::All);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_end_rewinds_to_the_first_track_paused() {
    let mut r = rig(&["a", "b", "c"], 0.2, false, PlayerConfig::default()).await;
    r.player
        .play_context(ctx(&["a", "b", "c"]), Some("b".into()), Some(false))
        .await
        .unwrap();
    next_tracking(&mut r.tracking, ended("c")).await;
    let s = wait_state(&r.player, |s| s.status == PlayerStatus::Paused).await;
    assert_eq!(tid(&s.track), "a");
    assert_eq!(s.position_ms, 0);
    let up: Vec<String> = s.queue.next_up.iter().map(|i| i.track.id.clone()).collect();
    assert_eq!(up, vec!["b", "c"]);
    // Play starts the context from the top.
    r.player.resume().await.unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_context_resumes_where_it_was_left_even_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.json");
    let cfg = || PlayerConfig {
        persist_path: Some(path.clone()),
        ..Default::default()
    };
    let ids = ["a", "b", "c", "x"];
    {
        let mut r = rig(&ids, 5.0, true, cfg()).await;
        r.player
            .play_context(album("al", &["a", "b", "c"]), Some("b".into()), None)
            .await
            .unwrap();
        next_tracking(&mut r.tracking, started("b")).await;
        r.player.pause().await.unwrap();
        r.player.seek(2_000).await.unwrap();
        // Something else plays, then the app quits.
        r.player
            .play_context(album("other", &["x"]), None, None)
            .await
            .unwrap();
        next_tracking(&mut r.tracking, started("x")).await;
        r.player.flush().await;
    }
    let mut r = rig(&ids, 5.0, true, cfg()).await;
    r.player
        .play_context(album("al", &["a", "b", "c"]), None, None)
        .await
        .unwrap();
    next_tracking(&mut r.tracking, started("b")).await;
    let s = r.player.state().await.unwrap();
    assert!(s.position_ms >= 2_000, "{}", s.position_ms);
    // A context never played before starts at track 1.
    r.player
        .play_context(album("fresh", &["c", "a"]), None, None)
        .await
        .unwrap();
    next_tracking(&mut r.tracking, started("c")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shuffle_is_per_context_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.json");
    let cfg = || PlayerConfig {
        persist_path: Some(path.clone()),
        ..Default::default()
    };
    let ids = ["a", "b", "c", "d"];
    {
        let mut r = rig(&ids, 5.0, true, cfg()).await;
        r.player
            .play_context(album("one", &["a", "b"]), None, None)
            .await
            .unwrap();
        next_tracking(&mut r.tracking, started("a")).await;
        // Set for a context that isn't playing: nothing changes now.
        r.player.set_context_shuffle("two".into(), true).await.unwrap();
        let s = r.player.state().await.unwrap();
        assert!(!s.shuffle);
        assert_eq!(s.shuffled_contexts, vec!["two".to_string()]);
        r.player.flush().await;
    }
    let mut r = rig(&ids, 5.0, true, cfg()).await;
    assert_eq!(
        r.player.state().await.unwrap().shuffled_contexts,
        vec!["two".to_string()]
    );
    r.player
        .play_context(album("two", &["c", "d"]), None, None)
        .await
        .unwrap();
    next_tracking(&mut r.tracking, |e| matches!(e, TrackingEvent::TrackStarted { .. })).await;
    assert!(r.player.state().await.unwrap().shuffle);
    // The playing context's toggle applies now and is remembered.
    r.player.set_context_shuffle("two".into(), false).await.unwrap();
    let s = r.player.state().await.unwrap();
    assert!(!s.shuffle);
    assert!(s.shuffled_contexts.is_empty());
    r.player
        .play_context(album("one", &["a", "b"]), Some("a".into()), None)
        .await
        .unwrap();
    assert!(!r.player.state().await.unwrap().shuffle);
}

// ---------------------------------------------------------------- output devices

fn devices(r: &Rig) {
    let dev = |id: &str, name: &str, default: bool| OutputDevice {
        id: id.into(),
        name: name.into(),
        is_default: default,
    };
    r.cap.set_devices(vec![
        dev("spk", "Speakers", true),
        dev("usb", "USB DAC", false),
    ]);
}

/// Waits until playback has moved past `ms` (audio is flowing).
async fn wait_position_past(r: &Rig, ms: u64) -> u64 {
    wait_state(&r.player, |s| s.status == PlayerStatus::Playing && s.position_ms > ms)
        .await
        .position_ms
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_devices_are_listed_with_the_default_current() {
    let r = rig(&["a"], 1.0, true, PlayerConfig::default()).await;
    devices(&r);
    let s = r.player.output_state().await.unwrap();
    assert_eq!(s.devices.len(), 2);
    assert_eq!(s.selected, None);
    assert_eq!(s.current.unwrap().id, "spk");
    let err = r.player.select_output(Some("nope".into())).await.unwrap_err();
    assert!(matches!(err, crate::error::AppError::NotFound(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_output_mid_track_keeps_position_and_play_state() {
    let mut r = rig(&["a"], 4.0, true, PlayerConfig::default()).await;
    devices(&r);
    r.player.play_context(ctx(&["a"]), None, None).await.unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    let before = wait_position_past(&r, 500).await;
    assert_eq!(r.cap.opened_id().as_deref(), Some("spk"));

    let s = r.player.select_output(Some("usb".into())).await.unwrap();
    assert_eq!(s.selected.as_deref(), Some("usb"));
    assert_eq!(s.current.as_ref().unwrap().id, "usb");
    assert_eq!(r.cap.opened_id().as_deref(), Some("usb"));
    assert_eq!(r.cap.opens(), 2);
    let now = r.player.state().await.unwrap();
    assert_eq!(now.status, PlayerStatus::Playing);
    assert!(now.position_ms >= before, "position went back: {} < {before}", now.position_ms);
    assert!(now.position_ms < before + 1_000, "position jumped: {}", now.position_ms);
    // And it plays on from there.
    wait_position_past(&r, now.position_ms + 200).await;
    assert!(r.sink.outputs.lock().iter().any(|o| o.selected.as_deref() == Some("usb")));
    assert!(r.sink.lost.lock().is_empty());

    // Paused: switching keeps it paused.
    r.player.pause().await.unwrap();
    r.player.select_output(None).await.unwrap();
    assert_eq!(r.cap.opened_id().as_deref(), Some("spk"));
    assert_eq!(r.player.state().await.unwrap().status, PlayerStatus::Paused);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chosen_output_unplugged_falls_back_to_the_default_with_a_notice() {
    let mut r = rig(&["a"], 4.0, true, PlayerConfig::default()).await;
    devices(&r);
    r.player.select_output(Some("usb".into())).await.unwrap();
    r.player.play_context(ctx(&["a"]), None, None).await.unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    assert_eq!(r.cap.opened_id().as_deref(), Some("usb"));
    let at = wait_position_past(&r, 300).await;

    r.cap.remove_device("usb");
    next_tracking(&mut r.tracking, |e| matches!(e, TrackingEvent::Paused { .. })).await;
    let s = wait_state(&r.player, |s| s.status == PlayerStatus::Paused).await;
    assert!(s.position_ms >= at);
    let deadline = Instant::now() + Duration::from_secs(5);
    while r.sink.lost.lock().is_empty() {
        assert!(Instant::now() < deadline, "no output://lost");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(r.sink.lost.lock().clone(), vec!["USB DAC".to_string()]);
    let o = r.player.output_state().await.unwrap();
    assert_eq!(o.selected, None);
    assert_eq!(o.current.unwrap().id, "spk");

    // Play resumes on the system default.
    r.player.resume().await.unwrap();
    wait_position_past(&r, s.position_ms + 100).await;
    assert_eq!(r.cap.opened_id().as_deref(), Some("spk"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn system_default_changes_are_followed_without_stopping() {
    let mut r = rig(&["a"], 4.0, true, PlayerConfig::default()).await;
    devices(&r);
    r.player.play_context(ctx(&["a"]), None, None).await.unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    let before = wait_position_past(&r, 300).await;
    r.cap.set_default("usb");
    let deadline = Instant::now() + Duration::from_secs(5);
    while r.cap.opens() < 2 {
        assert!(Instant::now() < deadline, "never reopened");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(r.cap.opened_id().as_deref(), Some("usb"));
    let p = wait_position_past(&r, before + 100).await;
    assert!(p < before + 2_000);
    assert!(r.sink.lost.lock().is_empty());
    assert_eq!(r.player.state().await.unwrap().status, PlayerStatus::Playing);
    // A chosen device ignores default changes.
    r.player.select_output(Some("spk".into())).await.unwrap();
    let opens = r.cap.opens();
    r.cap.set_default("spk");
    r.cap.set_default("usb");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(r.cap.opens(), opens);
    assert_eq!(r.cap.opened_id().as_deref(), Some("spk"));
}

// ------------------------------------------------------------ A-B loop

fn ab(a_ms: u64, b_ms: u64) -> Option<AbLoop> {
    Some(AbLoop { a_ms, b_ms })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ab_loop_wraps_in_the_core_and_points_are_checked() {
    let mut r = rig(&["a", "b"], 2.0, true, PlayerConfig::default()).await;
    r.player.play_context(ctx(&["a", "b"]), None, Some(false)).await.unwrap();
    // B before A swaps them.
    r.player.set_ab_loop(ab(1_200, 100)).await.unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!(s.ab_loop, ab(100, 1_200));
    // Under 1 s apart is refused and leaves the loop alone.
    assert!(r.player.set_ab_loop(ab(300, 1_000)).await.is_err());
    assert_eq!(r.player.state().await.unwrap().ab_loop, ab(100, 1_200));
    next_tracking(&mut r.tracking, started("a")).await;

    // The captured output is A..B over and over, with no gap.
    let all = concat(&r, &["a"]);
    let f = |ms: usize| ms * RATE as usize / 1000 * 2;
    let mut want = all[..f(1_200)].to_vec();
    for _ in 0..2 {
        want.extend_from_slice(&all[f(100)..f(1_200)]);
    }
    let got = settled_samples(&r.cap, want.len()).await;
    assert_eq!(&got[..want.len()], &want[..]);
    let s = r.player.state().await.unwrap();
    assert_eq!(tid(&s.track), "a");
    assert!(s.position_ms <= 1_200, "position {}", s.position_ms);

    // A seek inside the span keeps it; outside clears it.
    r.player.seek(600).await.unwrap();
    assert_eq!(r.player.state().await.unwrap().ab_loop, ab(100, 1_200));
    r.player.seek(1_500).await.unwrap();
    assert_eq!(r.player.state().await.unwrap().ab_loop, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ab_loop_clears_on_track_change_and_on_request() {
    let mut r = rig(&["a", "b"], 3.0, true, PlayerConfig::default()).await;
    r.player.play_context(ctx(&["a", "b"]), None, Some(false)).await.unwrap();
    next_tracking(&mut r.tracking, started("a")).await;
    r.player.set_ab_loop(ab(0, 2_000)).await.unwrap();
    r.player.set_ab_loop(None).await.unwrap();
    assert_eq!(r.player.state().await.unwrap().ab_loop, None);
    r.player.set_ab_loop(ab(0, 2_000)).await.unwrap();
    r.player.next().await.unwrap();
    let s = wait_state(&r.player, |s| tid(&s.track) == "b").await;
    assert_eq!(s.ab_loop, None);
    // Pausing and resuming the same track keeps it.
    next_tracking(&mut r.tracking, started("b")).await;
    r.player.set_ab_loop(ab(500, 2_500)).await.unwrap();
    r.player.pause().await.unwrap();
    r.player.resume().await.unwrap();
    assert_eq!(r.player.state().await.unwrap().ab_loop, ab(500, 2_500));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_and_jump_can_start_at_a_position_paused() {
    let r = rig(&["a", "b", "c"], 3.0, true, PlayerConfig::default()).await;
    r.player
        .play_context_at(ctx(&["a", "b", "c"]), Some("b".into()), 1_500, false)
        .await
        .unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!(s.status, PlayerStatus::Paused);
    assert_eq!(tid(&s.track), "b");
    assert_eq!(s.position_ms, 1_500);
    let c = s.queue.next_up[0].item_id.clone();
    r.player.queue_jump_at(c, 1_000, false).await.unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!((tid(&s.track), s.status, s.position_ms), ("c".to_string(), PlayerStatus::Paused, 1_000));
    assert!(r.player.queue_jump_at("missing".into(), 0, true).await.is_err());
}

fn is_offline_refusal(e: &crate::error::AppError) -> bool {
    matches!(e, crate::error::AppError::Invalid(m) if m == crate::audio::player::OFFLINE_NOTHING_PLAYS)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_play_leaves_out_songs_that_need_the_server() {
    let r = rig(&["a", "b", "c", "d"], 0.2, false, PlayerConfig::default()).await;
    *r.resolver.playable.lock() = Some(["a", "c"].iter().map(|s| s.to_string()).collect());
    // Clicking a song that can't play starts at the next one that can.
    r.player
        .play_context(ctx(&["a", "b", "c", "d"]), Some("b".into()), Some(false))
        .await
        .unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!(tid(&s.track), "c");
    let upcoming: Vec<String> = s.queue.next_up.iter().map(|i| i.track.id.clone()).collect();
    assert!(!upcoming.iter().any(|id| id == "b" || id == "d"), "{upcoming:?}");
    assert_eq!(*r.resolver.left_out.lock(), vec![2]);
    // Asking only for songs that can't play is refused with a reason.
    let e = r.player.queue_add(vec!["d".into()], AddAt::End).await.unwrap_err();
    assert!(is_offline_refusal(&e), "{e}");
    let e = r.player.play_context(ctx(&["b", "d"]), None, Some(false)).await.unwrap_err();
    assert!(is_offline_refusal(&e), "{e}");
    // Back online: everything plays again.
    *r.resolver.playable.lock() = None;
    r.player
        .play_context(ctx(&["a", "b", "c", "d"]), Some("b".into()), Some(false))
        .await
        .unwrap();
    assert_eq!(tid(&r.player.state().await.unwrap().track), "b");
}

#[test]
fn playable_part_wraps_round_to_find_a_start() {
    use crate::audio::player::playable_part;
    let tracks: Vec<Track> = ["a", "b", "c"].iter().map(|id| track(id, "A", &format!("/p/{id}"))).collect();
    let ok: std::collections::HashSet<String> = ["a"].iter().map(|s| s.to_string()).collect();
    assert_eq!(playable_part(&tracks, Some("c"), &ok), (Some("a".into()), 2));
    assert_eq!(playable_part(&tracks, Some("a"), &ok), (Some("a".into()), 2));
    assert_eq!(playable_part(&tracks, None, &ok), (None, 2));
}

fn radio(ids: &[&str]) -> PlayContext {
    PlayContext {
        kind: ContextKind::Radio,
        id: Some("track:a".into()),
        track_ids: Some(ids.iter().map(|s| s.to_string()).collect()),
    }
}

fn next_ids(s: &PlayerState) -> Vec<String> {
    s.queue.next_up.iter().map(|i| i.track.id.clone()).collect()
}

#[tokio::test]
async fn radio_refills_when_it_runs_low_without_repeats() {
    let r = rig(&["a", "b", "c", "d", "e", "f", "g", "h"], 30.0, true, PlayerConfig::default()).await;
    *r.resolver.more_batches.lock() = vec![vec!["d".into(), "e".into(), "f".into(), "g".into(), "h".into()]];
    r.player.play_context(radio(&["a", "b", "c"]), None, Some(false)).await.unwrap();
    let s = wait_state(&r.player, |s| s.queue.next_up.len() == 7).await;
    assert_eq!(next_ids(&s), vec!["b", "c", "d", "e", "f", "g", "h"]);
    let calls = r.resolver.more_calls.lock().clone();
    assert_eq!(calls.len(), 1, "one refill at a time");
    assert_eq!(calls[0].kind, ContextKind::Radio);
    assert_eq!(calls[0].id.as_deref(), Some("track:a"));
    assert_eq!(calls[0].n, super::player::REFILL_BATCH);
    for id in ["a", "b", "c"] {
        assert!(calls[0].avoid.iter().any(|x| x == id), "queued songs are avoided: {:?}", calls[0].avoid);
    }
}

#[tokio::test]
async fn a_radio_that_finds_nothing_is_not_asked_again() {
    let r = rig(&["a", "b"], 30.0, true, PlayerConfig::default()).await;
    r.player.play_context(radio(&["a", "b"]), None, Some(false)).await.unwrap();
    wait_state(&r.player, |_| !r.resolver.more_calls.lock().is_empty()).await;
    r.player.next().await.unwrap();
    wait_state(&r.player, |s| tid(&s.track) == "b").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(r.resolver.more_calls.lock().len(), 1);
}

#[tokio::test]
async fn similar_music_follows_the_last_song_when_keep_playing_is_on() {
    let r = rig(&["a", "b", "c", "d"], 30.0, true, PlayerConfig::default()).await;
    *r.resolver.more_batches.lock() = vec![vec!["c".into(), "d".into()]];
    r.player.play_context(ctx(&["a", "b"]), None, Some(false)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(r.resolver.more_calls.lock().is_empty(), "not before the last song");
    r.player.next().await.unwrap();
    let s = wait_state(&r.player, |s| s.queue.next_up.len() == 2).await;
    assert_eq!(next_ids(&s), vec!["c", "d"]);
    assert!(s.queue.next_up.iter().all(|i| i.source == super::queue::ItemSource::Autoplay));
    let calls = r.resolver.more_calls.lock().clone();
    assert_eq!(calls[0].kind, ContextKind::Tracks);
    assert_eq!(calls[0].seed_ids, vec!["a".to_string(), "b".to_string()], "the songs that played are the seed");
}

#[tokio::test]
async fn stop_at_the_end_asks_for_nothing() {
    let r = rig(&["a", "b"], 30.0, true, PlayerConfig { autoplay: false, ..Default::default() }).await;
    r.player.play_context(ctx(&["a", "b"]), Some("b".into()), Some(false)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(r.resolver.more_calls.lock().is_empty());
    // Turning it on while the last song plays asks at once.
    r.player.set_autoplay(true).await.unwrap();
    wait_state(&r.player, |_| !r.resolver.more_calls.lock().is_empty()).await;
    // Repeat on: the context loops, nothing is added.
    let r = rig(&["a", "b"], 30.0, true, PlayerConfig::default()).await;
    r.player.set_repeat(Repeat::All).await.unwrap();
    r.player.play_context(ctx(&["a", "b"]), Some("b".into()), Some(false)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(r.resolver.more_calls.lock().is_empty());
}

fn playlist(id: &str, ids: &[&str]) -> PlayContext {
    PlayContext {
        kind: ContextKind::Playlist,
        id: Some(id.into()),
        track_ids: Some(ids.iter().map(|s| s.to_string()).collect()),
    }
}

fn is_smart(i: &super::queue::QueueItem) -> bool {
    i.source == super::queue::ItemSource::Smart
}

/// Upcoming kinds with the playing song first: C a playlist song, S a suggestion.
fn rhythm(s: &PlayerState) -> String {
    let cur = s.queue.current.as_ref().map_or("", |c| if is_smart(c) { "S" } else { "C" });
    let rest: String = s.queue.next_up.iter().map(|i| if is_smart(i) { 'S' } else { 'C' }).collect();
    format!("{cur}{rest}")
}

const SONGS: [&str; 7] = ["a", "b", "c", "d", "e", "f", "g"];
const ALL: [&str; 11] = ["a", "b", "c", "d", "e", "f", "g", "x", "y", "z", "w"];

#[tokio::test]
async fn smart_shuffle_is_for_playlists_and_liked_songs_only() {
    let r = rig(&ALL, 30.0, true, PlayerConfig::default()).await;
    r.player.play_context(album("al", &SONGS), Some("a".into()), Some(true)).await.unwrap();
    assert!(r.player.set_smart_shuffle(true).await.is_err(), "not on an album");
    let s = r.player.state().await.unwrap();
    assert!(s.shuffle && !s.smart_shuffle);
    // A smart preference stored for an album id does nothing either.
    r.player.set_context_smart_shuffle("al".into(), true).await.unwrap_or(());
    r.player.play_context(album("al", &SONGS), Some("a".into()), Some(true)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(r.resolver.more_calls.lock().iter().all(|m| !m.smart));
    // Liked Songs has it.
    let liked = PlayContext { kind: ContextKind::Liked, id: Some("liked".into()), track_ids: Some(SONGS.iter().map(|s| s.to_string()).collect()) };
    *r.resolver.more_batches.lock() = vec![vec!["x".into(), "y".into(), "z".into()]];
    r.player.play_context(liked, Some("a".into()), Some(true)).await.unwrap();
    r.player.set_smart_shuffle(true).await.unwrap();
    let s = wait_state(&r.player, |s| s.queue.next_up.iter().any(is_smart)).await;
    assert!(s.smart_shuffle && s.smart_contexts.contains(&"liked".to_string()));
}

#[tokio::test]
async fn smart_shuffle_builds_one_fixed_list_two_songs_to_one_suggestion() {
    let r = rig(&ALL, 30.0, true, PlayerConfig::default()).await;
    *r.resolver.more_batches.lock() = vec![vec!["x".into(), "y".into(), "z".into()], vec!["w".into()]];
    r.player.set_context_smart_shuffle("pl".into(), true).await.unwrap();
    assert!(r.player.state().await.unwrap().shuffled_contexts.contains(&"pl".to_string()));
    r.player.play_context(playlist("pl", &SONGS), None, None).await.unwrap();
    let s = wait_state(&r.player, |s| s.queue.next_up.iter().any(is_smart)).await;
    assert!(s.shuffle && s.smart_shuffle);
    assert_eq!(rhythm(&s), "CCSCCSCCSC");
    let req = r.resolver.more_calls.lock()[0].clone();
    assert!(req.smart && req.kind == ContextKind::Playlist);
    assert_eq!(req.n, 3);
    assert!(SONGS.iter().all(|t| req.avoid.contains(&t.to_string())), "not already in the playlist");
    assert_eq!(req.avoid_artists.len(), 2, "the playlist's artists are left out");
    // Songs get used up: nothing new is asked for.
    r.player.next().await.unwrap();
    r.player.next().await.unwrap();
    r.player.next().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(r.resolver.more_calls.lock().iter().filter(|m| m.smart).count(), 1);
    assert_eq!(r.player.state().await.unwrap().queue.next_up.iter().filter(|i| is_smart(i)).count(), 2);
}

#[tokio::test]
async fn removing_a_suggestion_skips_it_and_builds_new_ones() {
    let r = rig(&ALL, 30.0, true, PlayerConfig::default()).await;
    *r.resolver.more_batches.lock() = vec![vec!["x".into(), "y".into(), "z".into()], vec!["w".into(), "y".into()]];
    r.player.set_context_smart_shuffle("pl".into(), true).await.unwrap();
    r.player.play_context(playlist("pl", &SONGS), None, None).await.unwrap();
    wait_state(&r.player, |s| s.queue.next_up.iter().any(is_smart)).await;
    assert!(r.player.smart_remove().await.is_err(), "a playlist song is playing");
    r.player.next().await.unwrap();
    r.player.next().await.unwrap();
    let s = r.player.state().await.unwrap();
    assert_eq!(tid(&s.track), "x");
    r.player.smart_remove().await.unwrap();
    let s = wait_state(&r.player, |s| s.queue.next_up.iter().any(|i| i.track.id == "w")).await;
    assert!(!is_smart(s.queue.current.as_ref().unwrap()), "on to the next playlist song");
    assert_eq!(r.resolver.more_calls.lock().iter().filter(|m| m.smart).count(), 2);
    assert!(s.queue.next_up.iter().all(|i| i.track.id != "z"), "the old suggestions went");
    assert_eq!(rhythm(&s), "CCSCCSC");
}

#[tokio::test]
async fn a_kept_suggestion_loses_its_badge_and_keeps_playing() {
    let r = rig(&ALL, 30.0, true, PlayerConfig::default()).await;
    *r.resolver.more_batches.lock() = vec![vec!["x".into(), "y".into(), "z".into()]];
    r.player.set_context_smart_shuffle("pl".into(), true).await.unwrap();
    r.player.play_context(playlist("pl", &SONGS), None, None).await.unwrap();
    wait_state(&r.player, |s| s.queue.next_up.iter().any(is_smart)).await;
    r.player.next().await.unwrap();
    r.player.next().await.unwrap();
    let cur = r.player.state().await.unwrap().queue.current.unwrap();
    assert!(is_smart(&cur));
    r.player.smart_keep(cur.item_id.clone()).await.unwrap();
    let s = r.player.state().await.unwrap();
    let now = s.queue.current.unwrap();
    assert_eq!((now.item_id.clone(), is_smart(&now)), (cur.item_id.clone(), false));
    assert!(s.status != PlayerStatus::Idle);
    assert!(r.player.smart_keep(cur.item_id).await.is_err());
}

#[tokio::test]
async fn turning_smart_shuffle_off_mid_play_keeps_the_song_and_the_order() {
    let r = rig(&ALL, 30.0, true, PlayerConfig::default()).await;
    *r.resolver.more_batches.lock() = vec![vec!["x".into(), "y".into(), "z".into()]];
    r.player.set_context_smart_shuffle("pl".into(), true).await.unwrap();
    r.player.play_context(playlist("pl", &SONGS), None, None).await.unwrap();
    wait_state(&r.player, |s| s.queue.next_up.iter().any(is_smart)).await;
    r.player.next().await.unwrap();
    r.player.next().await.unwrap();
    let before = r.player.state().await.unwrap();
    let playing = before.queue.current.clone().unwrap();
    assert!(is_smart(&playing));
    let order: Vec<String> = before.queue.next_up.iter().filter(|i| !is_smart(i)).map(|i| i.track.id.clone()).collect();
    r.player.set_smart_shuffle(false).await.unwrap();
    let s = r.player.state().await.unwrap();
    assert!(!s.shuffle && !s.smart_shuffle && s.smart_contexts.is_empty());
    assert_eq!(s.queue.current.unwrap().item_id, playing.item_id, "the suggestion carries on, badge and all");
    let after: Vec<String> = s.queue.next_up.iter().map(|i| i.track.id.clone()).collect();
    assert_eq!(after, order, "no suggestions left, shuffled order kept");
}

#[tokio::test]
async fn the_setting_takes_smart_shuffle_out_everywhere() {
    let r = rig(&ALL, 30.0, true, PlayerConfig::default()).await;
    *r.resolver.more_batches.lock() = vec![vec!["x".into(), "y".into(), "z".into()], vec!["w".into()]];
    r.player.set_context_smart_shuffle("pl".into(), true).await.unwrap();
    r.player.play_context(playlist("pl", &SONGS), None, None).await.unwrap();
    wait_state(&r.player, |s| s.queue.next_up.iter().any(is_smart)).await;
    r.player.set_smart_allowed(false).await.unwrap();
    let s = r.player.state().await.unwrap();
    assert!(s.shuffle && !s.smart_shuffle, "plain shuffle stays");
    assert!(!s.queue.next_up.iter().any(is_smart));
    r.player.set_smart_allowed(true).await.unwrap();
    wait_state(&r.player, |s| s.smart_shuffle && s.queue.next_up.iter().any(is_smart)).await;
}

#[tokio::test]
async fn autoplay_takes_over_after_the_last_song_without_badges() {
    let r = rig(&ALL, 30.0, true, PlayerConfig::default()).await;
    *r.resolver.more_batches.lock() = vec![vec!["x".into()], vec!["w".into()]];
    r.player.set_context_smart_shuffle("pl".into(), true).await.unwrap();
    r.player.play_context(playlist("pl", &["a", "b", "c"]), Some("a".into()), None).await.unwrap();
    let s = wait_state(&r.player, |s| s.queue.next_up.iter().any(is_smart)).await;
    assert_eq!(rhythm(&s), "CCSC");
    r.player.next().await.unwrap();
    r.player.next().await.unwrap();
    r.player.next().await.unwrap();
    let s = wait_state(&r.player, |s| s.queue.next_up.iter().any(|i| i.track.id == "w")).await;
    let w = s.queue.next_up.iter().find(|i| i.track.id == "w").unwrap();
    assert_eq!(w.source, super::queue::ItemSource::Autoplay);
    let calls = r.resolver.more_calls.lock().clone();
    assert_eq!((calls.len(), calls[1].smart), (2, false));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn smart_shuffle_is_remembered_per_playlist_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.json");
    let cfg = || PlayerConfig { persist_path: Some(path.clone()), ..Default::default() };
    {
        let r = rig(&ALL, 5.0, true, cfg()).await;
        r.player.set_context_smart_shuffle("one".into(), true).await.unwrap();
        r.player.set_context_shuffle("two".into(), true).await.unwrap();
        r.player.set_context_smart_shuffle("liked".into(), true).await.unwrap();
        r.player.set_context_smart_shuffle("liked".into(), false).await.unwrap();
        r.player.flush().await;
    }
    let r = rig(&ALL, 5.0, true, cfg()).await;
    let s = r.player.state().await.unwrap();
    assert_eq!(s.smart_contexts, vec!["one".to_string()]);
    assert!(s.shuffled_contexts.contains(&"one".to_string()) && s.shuffled_contexts.contains(&"two".to_string()));
    assert!(!s.shuffled_contexts.contains(&"liked".to_string()), "smart shuffle off is off");
    *r.resolver.more_batches.lock() = vec![vec!["x".into(), "y".into(), "z".into()]];
    r.player.play_context(playlist("one", &SONGS), None, None).await.unwrap();
    wait_state(&r.player, |s| s.smart_shuffle && s.queue.next_up.iter().any(is_smart)).await;
}

#[tokio::test]
async fn smooth_transitions_last_for_the_context_they_were_turned_on_for() {
    let r = rig(&["a", "b", "c"], 30.0, true, PlayerConfig::default()).await;
    r.player.play_context(album("al", &["a", "b"]), None, Some(false)).await.unwrap();
    r.player.set_smooth_transitions(true).await.unwrap();
    assert!(r.player.state().await.unwrap().smooth_transitions);
    r.player.play_context(ctx(&["c"]), None, Some(false)).await.unwrap();
    assert!(!r.player.state().await.unwrap().smooth_transitions, "another context: off again");
}
