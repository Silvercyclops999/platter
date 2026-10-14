//! Controller tests: commands → engine → events, with a capture output and
//! a local HTTP server. Covers the spec 04 queue/playback rules end to end
//! and the spec 06 tracking signals.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::broadcast;

use super::engine::EngineConfig;
use super::events::{
    ErrorPayload, EventSink, PlayerState, PlayerStatus, PositionPayload, SKIPPED_MESSAGE,
    TrackingEvent,
};
use super::http::HttpConfig;
use super::output::{CaptureHandle, CaptureOutput, OutputFormat};
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
}

impl EventSink for Sink {
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
    assert_eq!(tid(&s.track), "c");
    assert_eq!(s.position_ms, 0);
    assert_eq!(r.cap.samples(), concat(&r, &["a", "b", "c"]));
    let hist: Vec<String> = s.queue.history.iter().map(|i| i.track.id.clone()).collect();
    assert_eq!(hist, vec!["b", "a"]);
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
    assert_eq!(r.cap.samples(), concat(&r, &["a", "c"]));
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
    assert_eq!(r.cap.samples(), concat(&r, &["b"]));
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
    tokio::time::sleep(Duration::from_millis(400)).await;
    let pos = r.sink.positions.lock().clone();
    assert!(pos.len() >= 3, "{}", pos.len());
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
