//! Play tracking (spec 06): turns the player's tracking events into Plex
//! timeline/scrobble reports and rows in the local listen log.
//!
//! Plex gets `/:/timeline` on play, pause, resume, seek, stop and every 10 s
//! while playing, and `/:/scrobble` once per play when the listen log says the
//! threshold was crossed. Scrobbles that fail (offline) are queued in SQLite
//! and replayed when the server is reachable again. Plex's scrobble has no
//! timestamp parameter, so replayed plays count at replay time.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::audio::events::TrackingEvent;
use crate::audio::queue::ContextInfo;
use crate::audio::source::ContextKind as PlayContextKind;
use crate::db::DbHandle;
use crate::db::listens::{self, ContextKind, NewReport, PlayStart, ReportKind};
use crate::db::queries::now_s;
use crate::plex::PlexState;
use crate::error::AppResult;
use crate::plex::client::{PlaybackState, TimelineReport};

const TIMELINE_EVERY_MS: u64 = 10_000;

/// Shared switch for the private session stub: when on, nothing is written to
/// the local listen log. Plex reporting continues unless `report_to_plex` is
/// off too.
#[derive(Clone)]
pub struct TrackingSettings {
    pub private_session: Arc<AtomicBool>,
    pub report_to_plex: Arc<AtomicBool>,
}

impl Default for TrackingSettings {
    fn default() -> Self {
        Self {
            private_session: Arc::new(AtomicBool::new(false)),
            report_to_plex: Arc::new(AtomicBool::new(true)),
        }
    }
}

fn log_context(ctx: &Option<ContextInfo>) -> (Option<ContextKind>, Option<String>) {
    match ctx {
        None => (None, None),
        Some(c) => (
            Some(match c.kind {
                PlayContextKind::Album => ContextKind::Album,
                PlayContextKind::Playlist => ContextKind::Playlist,
                PlayContextKind::Artist => ContextKind::Artist,
                PlayContextKind::Tracks => ContextKind::Tracks,
                PlayContextKind::Search => ContextKind::Search,
            }),
            c.id.clone(),
        ),
    }
}

/// What to send to Plex for one tracking event. Pure, so it's testable.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    Timeline(TimelineReport),
}

/// Per-play state kept between events.
#[derive(Default)]
struct Current {
    rating_key: String,
    item_id: String,
    duration_ms: u64,
    play_id: Option<i64>,
    last_timeline_ms: Option<u64>,
}

fn timeline(c: &Current, state: PlaybackState, time_ms: u64) -> Report {
    Report::Timeline(TimelineReport {
        rating_key: c.rating_key.clone(),
        state,
        time_ms,
        duration_ms: c.duration_ms,
        play_queue_item_id: None,
    })
}

/// Decide the Plex reports for an event, updating `cur`.
fn reports_for(cur: &mut Current, ev: &TrackingEvent) -> Vec<Report> {
    match ev {
        TrackingEvent::TrackStarted { track, item_id, .. } => {
            *cur = Current {
                rating_key: track.id.clone(),
                item_id: item_id.clone(),
                duration_ms: track.duration_ms,
                play_id: None,
                last_timeline_ms: Some(0),
            };
            vec![timeline(cur, PlaybackState::Playing, 0)]
        }
        TrackingEvent::Progress { track_id, position_ms, .. } if *track_id == cur.rating_key => {
            let due = cur
                .last_timeline_ms
                .is_none_or(|last| position_ms.abs_diff(last) >= TIMELINE_EVERY_MS);
            if due {
                cur.last_timeline_ms = Some(*position_ms);
                vec![timeline(cur, PlaybackState::Playing, *position_ms)]
            } else {
                vec![]
            }
        }
        TrackingEvent::Paused { track_id, position_ms } if *track_id == cur.rating_key => {
            vec![timeline(cur, PlaybackState::Paused, *position_ms)]
        }
        TrackingEvent::Resumed { track_id, position_ms } if *track_id == cur.rating_key => {
            cur.last_timeline_ms = Some(*position_ms);
            vec![timeline(cur, PlaybackState::Playing, *position_ms)]
        }
        TrackingEvent::Seeked { track_id, to_ms, .. } if *track_id == cur.rating_key => {
            cur.last_timeline_ms = Some(*to_ms);
            vec![timeline(cur, PlaybackState::Playing, *to_ms)]
        }
        TrackingEvent::TrackEnded { track, item_id, listened_ms, completed, skipped_at_ms } => {
            if *item_id != cur.item_id {
                return vec![];
            }
            let at = if *completed { track.duration_ms } else { skipped_at_ms.unwrap_or(*listened_ms) };
            vec![timeline(cur, PlaybackState::Stopped, at)]
        }
        _ => vec![],
    }
}

/// Where play reports go. The app uses the connected Plex server; tests use
/// a recorder.
#[async_trait]
pub trait PlayReporter: Send + Sync {
    /// True when a server is connected right now.
    async fn online(&self) -> bool;
    async fn timeline(&self, r: &TimelineReport) -> AppResult<()>;
    async fn scrobble(&self, rating_key: &str) -> AppResult<()>;
}

#[async_trait]
impl PlayReporter for PlexState {
    async fn online(&self) -> bool {
        self.client().await.is_ok()
    }

    async fn timeline(&self, r: &TimelineReport) -> AppResult<()> {
        self.client().await?.timeline(r).await
    }

    async fn scrobble(&self, rating_key: &str) -> AppResult<()> {
        self.client().await?.scrobble(rating_key).await
    }
}

pub struct Tracker {
    plex: Arc<dyn PlayReporter>,
    db: DbHandle,
    settings: TrackingSettings,
}

impl Tracker {
    pub fn new(plex: PlexState, db: DbHandle, settings: TrackingSettings) -> Self {
        Self::with_reporter(Arc::new(plex), db, settings)
    }

    pub fn with_reporter(plex: Arc<dyn PlayReporter>, db: DbHandle, settings: TrackingSettings) -> Self {
        Self { plex, db, settings }
    }

    fn reporting(&self) -> bool {
        self.settings.report_to_plex.load(Ordering::Relaxed)
    }

    async fn send_timeline(&self, r: TimelineReport) {
        if !self.reporting() {
            return;
        }
        if let Err(e) = self.plex.timeline(&r).await {
            tracing::debug!("timeline not sent: {}", e.kind());
        }
    }

    async fn scrobble(&self, play_id: Option<i64>, rating_key: String) {
        if !self.reporting() {
            return;
        }
        let sent = self.plex.scrobble(&rating_key).await.is_ok();
        let db = self.db.clone();
        let _ = db
            .write_async(move |c| {
                if sent {
                    if let Some(id) = play_id {
                        listens::mark_reported(c, id)?;
                    }
                } else {
                    listens::enqueue_report(
                        c,
                        &NewReport {
                            play_id,
                            kind: ReportKind::Scrobble,
                            rating_key,
                            state: None,
                            time_ms: None,
                            duration_ms: None,
                            at: now_s(),
                        },
                    )?;
                }
                Ok(())
            })
            .await;
    }

    /// Send queued scrobbles, oldest first. Stops at the first failure.
    pub async fn replay_pending(&self) {
        if !self.reporting() || !self.plex.online().await {
            return;
        }
        let pending = match self.db.read_async(|c| listens::pending_reports(c, 200)).await {
            Ok(p) => p,
            Err(_) => return,
        };
        let mut sent = Vec::new();
        for r in pending {
            let ok = match r.kind {
                ReportKind::Scrobble => self.plex.scrobble(&r.rating_key).await.is_ok(),
                ReportKind::Timeline => true, // stale timelines are dropped
            };
            if !ok {
                break;
            }
            sent.push(r.id);
        }
        if !sent.is_empty() {
            let _ = self
                .db
                .write_async(move |c| listens::mark_sent(c, &sent, now_s()))
                .await;
        }
    }

    pub async fn run(self: Arc<Self>, mut events: broadcast::Receiver<TrackingEvent>) {
        let mut cur = Current::default();
        loop {
            let ev = match events.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            };
            self.handle(&mut cur, &ev).await;
        }
    }

    async fn handle(&self, cur: &mut Current, ev: &TrackingEvent) {
        for Report::Timeline(r) in reports_for(cur, ev) {
            self.send_timeline(r).await;
        }
        if let Err(e) = self.log(cur, ev).await {
            tracing::warn!("listen log: {e}");
        }
    }

    /// Local listen log + scrobble decisions.
    async fn log(&self, cur: &mut Current, ev: &TrackingEvent) -> AppResult<()> {
        let private = self.settings.private_session.load(Ordering::Relaxed);
        match ev {
            TrackingEvent::TrackStarted { track, at, context, .. } if !private => {
                let (kind, id) = log_context(context);
                let start = PlayStart {
                    rating_key: track.id.clone(),
                    part_key: Some(track.part_key.clone()),
                    started_at: at / 1000,
                    duration_ms: track.duration_ms,
                    context_kind: kind,
                    context_id: id,
                    device: None,
                    offline: !self.plex.online().await,
                };
                cur.play_id = Some(self.db.write_async(move |c| listens::start_play(c, &start)).await?);
            }
            TrackingEvent::Progress { track_id, position_ms, .. } if *track_id == cur.rating_key => {
                if let Some(id) = cur.play_id {
                    let ms = *position_ms;
                    let upd = self.db.write_async(move |c| listens::progress(c, id, ms)).await?;
                    if upd.scrobble_now {
                        self.scrobble(Some(id), cur.rating_key.clone()).await;
                    }
                }
            }
            TrackingEvent::TrackEnded { track, item_id, listened_ms, completed, skipped_at_ms }
                if *item_id == cur.item_id =>
            {
                if let Some(id) = cur.play_id.take() {
                    let (ms, done) = (*listened_ms, *completed);
                    let pos = skipped_at_ms.unwrap_or(ms);
                    let upd = self
                        .db
                        .write_async(move |c| listens::end_play(c, id, now_s(), ms, pos, done))
                        .await?;
                    if upd.scrobble_now {
                        self.scrobble(Some(id), track.id.clone()).await;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Start the tracker plus a loop that replays queued scrobbles every 30 s.
pub fn spawn(tracker: Tracker, events: broadcast::Receiver<TrackingEvent>) {
    let tracker = Arc::new(tracker);
    tauri::async_runtime::spawn(tracker.clone().run(events));
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            tracker.replay_pending().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Track;

    fn track(id: &str) -> Track {
        Track {
            id: id.into(),
            title: "t".into(),
            album_id: "a".into(),
            album_title: "A".into(),
            artist_id: "r".into(),
            artist_title: "R".into(),
            track_artist: None,
            track_number: 1,
            disc_number: 1,
            duration_ms: 200_000,
            thumb: None,
            part_key: "/library/parts/1/2/file.flac".into(),
            codec: None,
            container: None,
            bitrate: None,
            bit_depth: None,
            sample_rate: None,
            user_rating: None,
            view_count: 0,
            last_viewed_at: None,
        }
    }

    fn states(rs: &[Report]) -> Vec<(PlaybackState, u64)> {
        rs.iter().map(|Report::Timeline(t)| (t.state, t.time_ms)).collect()
    }

    #[test]
    fn timeline_on_start_pause_resume_seek_stop_and_every_10s() {
        let mut cur = Current::default();
        let t = track("1");
        let started = TrackingEvent::TrackStarted { track: t.clone(), item_id: "q1".into(), at: 0, context: None };
        assert_eq!(states(&reports_for(&mut cur, &started)), vec![(PlaybackState::Playing, 0)]);

        let progress = |ms| TrackingEvent::Progress { track_id: "1".into(), position_ms: ms, duration_ms: 200_000 };
        assert!(reports_for(&mut cur, &progress(5_000)).is_empty());
        assert_eq!(states(&reports_for(&mut cur, &progress(10_000))), vec![(PlaybackState::Playing, 10_000)]);
        assert!(reports_for(&mut cur, &progress(15_000)).is_empty());

        let paused = TrackingEvent::Paused { track_id: "1".into(), position_ms: 16_000 };
        assert_eq!(states(&reports_for(&mut cur, &paused)), vec![(PlaybackState::Paused, 16_000)]);
        let resumed = TrackingEvent::Resumed { track_id: "1".into(), position_ms: 16_000 };
        assert_eq!(states(&reports_for(&mut cur, &resumed)), vec![(PlaybackState::Playing, 16_000)]);
        let seeked = TrackingEvent::Seeked { track_id: "1".into(), from_ms: 16_000, to_ms: 90_000 };
        assert_eq!(states(&reports_for(&mut cur, &seeked)), vec![(PlaybackState::Playing, 90_000)]);
        assert!(reports_for(&mut cur, &progress(95_000)).is_empty());

        let ended = TrackingEvent::TrackEnded {
            track: t,
            item_id: "q1".into(),
            listened_ms: 40_000,
            completed: false,
            skipped_at_ms: Some(96_000),
        };
        assert_eq!(states(&reports_for(&mut cur, &ended)), vec![(PlaybackState::Stopped, 96_000)]);
    }

    #[test]
    fn events_for_another_track_are_ignored() {
        let mut cur = Current::default();
        let started = TrackingEvent::TrackStarted { track: track("1"), item_id: "q1".into(), at: 0, context: None };
        reports_for(&mut cur, &started);
        let stale = TrackingEvent::Paused { track_id: "2".into(), position_ms: 1 };
        assert!(reports_for(&mut cur, &stale).is_empty());
        let stale_end = TrackingEvent::TrackEnded {
            track: track("0"),
            item_id: "q0".into(),
            listened_ms: 1,
            completed: true,
            skipped_at_ms: None,
        };
        assert!(reports_for(&mut cur, &stale_end).is_empty());
    }

    // ------------------------------------------------------------ settings

    #[derive(Default)]
    struct Recorder {
        timelines: parking_lot::Mutex<Vec<PlaybackState>>,
        scrobbles: parking_lot::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl PlayReporter for Recorder {
        async fn online(&self) -> bool {
            true
        }
        async fn timeline(&self, r: &TimelineReport) -> AppResult<()> {
            self.timelines.lock().push(r.state);
            Ok(())
        }
        async fn scrobble(&self, rating_key: &str) -> AppResult<()> {
            self.scrobbles.lock().push(rating_key.to_string());
            Ok(())
        }
    }

    struct Rig {
        tracker: Tracker,
        rec: Arc<Recorder>,
        db: DbHandle,
        _dir: tempfile::TempDir,
    }

    fn rig(private: bool, report: bool) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("t.db")).unwrap();
        let rec = Arc::new(Recorder::default());
        let settings = TrackingSettings::default();
        settings.private_session.store(private, Ordering::Relaxed);
        settings.report_to_plex.store(report, Ordering::Relaxed);
        let tracker = Tracker::with_reporter(rec.clone(), db.clone(), settings);
        Rig { tracker, rec, db, _dir: dir }
    }

    /// Start a 200 s track, play 150 s of it and let it finish.
    async fn full_play(t: &Tracker) {
        let mut cur = Current::default();
        let tr = track("7");
        let evs = [
            TrackingEvent::TrackStarted { track: tr.clone(), item_id: "q1".into(), at: 1_000_000, context: None },
            TrackingEvent::Progress { track_id: "7".into(), position_ms: 150_000, duration_ms: 200_000 },
            TrackingEvent::TrackEnded {
                track: tr,
                item_id: "q1".into(),
                listened_ms: 200_000,
                completed: true,
                skipped_at_ms: None,
            },
        ];
        for ev in &evs {
            t.handle(&mut cur, ev).await;
        }
    }

    fn listen_rows(db: &DbHandle) -> i64 {
        db.read(|c| Ok(c.query_row("SELECT count(*) FROM listen_log", [], |r| r.get(0))?)).unwrap()
    }

    #[tokio::test]
    async fn normal_play_logs_and_reports() {
        let r = rig(false, true);
        full_play(&r.tracker).await;
        assert_eq!(listen_rows(&r.db), 1);
        assert_eq!(*r.rec.scrobbles.lock(), vec!["7".to_string()]);
        assert!(!r.rec.timelines.lock().is_empty());
    }

    #[tokio::test]
    async fn private_session_writes_no_listen_log_but_still_reports() {
        let r = rig(true, true);
        full_play(&r.tracker).await;
        assert_eq!(listen_rows(&r.db), 0);
        assert!(!r.rec.timelines.lock().is_empty());
    }

    #[tokio::test]
    async fn report_off_makes_no_plex_calls() {
        let r = rig(false, false);
        full_play(&r.tracker).await;
        assert_eq!(listen_rows(&r.db), 1);
        assert!(r.rec.timelines.lock().is_empty());
        assert!(r.rec.scrobbles.lock().is_empty());
        // Nothing queued for later either.
        let pending = r.db.read(|c| listens::pending_reports(c, 10)).unwrap();
        assert!(pending.is_empty());
        r.tracker.replay_pending().await;
        assert!(r.rec.scrobbles.lock().is_empty());
    }

    #[tokio::test]
    async fn flags_change_take_effect_immediately() {
        let r = rig(false, true);
        r.tracker.settings.private_session.store(true, Ordering::Relaxed);
        r.tracker.settings.report_to_plex.store(false, Ordering::Relaxed);
        full_play(&r.tracker).await;
        assert_eq!(listen_rows(&r.db), 0);
        assert!(r.rec.timelines.lock().is_empty());
        assert!(r.rec.scrobbles.lock().is_empty());
    }
}
