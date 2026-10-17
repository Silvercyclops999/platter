//! Play tracking: turns the player's tracking events into now-playing and
//! play reports for each track's server, and rows in the local listen log.
//!
//! The server hears about play, pause, resume, seek, stop and every 10 s
//! while playing (Plex: `/:/timeline`), and gets one finished play per play
//! when the listen log says the threshold was crossed (Plex: `/:/scrobble`).
//! Plays that fail (offline) are queued in SQLite and replayed when the
//! server is reachable again, with the time they happened. Plex's scrobble
//! has no timestamp parameter, so replayed plays count at replay time there.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::audio::events::TrackingEvent;
use crate::audio::queue::ContextInfo;
use crate::audio::source::ContextKind as PlayContextKind;
use crate::db::DbHandle;
use crate::db::listens::{self, ContextKind, NewReport, PlaySnapshot, PlayStart, ReportKind};
use crate::db::queries::now_s;
use crate::plex::PlexState;
use crate::error::AppResult;
pub use crate::plex::client::PlaybackState;
use crate::plex::client::TimelineReport;

const TIMELINE_EVERY_MS: u64 = 10_000;

/// Shared switches. A private session writes nothing to the local listen
/// log (so nothing is scrobbled) and sends no now-playing timelines, so the
/// server shows no live session for friends to see. "Share my activity" off
/// stops the timelines too; finished plays still reach Plex's history.
#[derive(Clone)]
pub struct TrackingSettings {
    pub private_session: Arc<AtomicBool>,
    pub report_plays: Arc<AtomicBool>,
    pub share_activity: Arc<AtomicBool>,
}

impl Default for TrackingSettings {
    fn default() -> Self {
        Self {
            private_session: Arc::new(AtomicBool::new(false)),
            report_plays: Arc::new(AtomicBool::new(true)),
            share_activity: Arc::new(AtomicBool::new(true)),
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
                PlayContextKind::Liked => ContextKind::Liked,
                PlayContextKind::Mix => ContextKind::Mix,
                PlayContextKind::Radio => ContextKind::Radio,
            }),
            c.id.clone(),
        ),
    }
}

/// Where a track is in its play, for its server's now-playing view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayReport {
    pub track_id: String,
    pub state: PlaybackState,
    pub time_ms: u64,
    pub duration_ms: u64,
}

/// What to send to the server for one tracking event. Pure, so it's testable.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    Timeline(PlayReport),
}

/// Per-play state kept between events.
#[derive(Default)]
struct Current {
    track_id: String,
    item_id: String,
    duration_ms: u64,
    play_id: Option<i64>,
    last_timeline_ms: Option<u64>,
}

fn timeline(c: &Current, state: PlaybackState, time_ms: u64) -> Report {
    Report::Timeline(PlayReport {
        track_id: c.track_id.clone(),
        state,
        time_ms,
        duration_ms: c.duration_ms,
    })
}

/// Decide the server reports for an event, updating `cur`.
fn reports_for(cur: &mut Current, ev: &TrackingEvent) -> Vec<Report> {
    match ev {
        TrackingEvent::TrackStarted { track, item_id, .. } => {
            *cur = Current {
                track_id: track.id.clone(),
                item_id: item_id.clone(),
                duration_ms: track.duration_ms,
                play_id: None,
                last_timeline_ms: Some(0),
            };
            vec![timeline(cur, PlaybackState::Playing, 0)]
        }
        TrackingEvent::Progress { track_id, position_ms, .. } if *track_id == cur.track_id => {
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
        TrackingEvent::Paused { track_id, position_ms } if *track_id == cur.track_id => {
            vec![timeline(cur, PlaybackState::Paused, *position_ms)]
        }
        TrackingEvent::Resumed { track_id, position_ms } if *track_id == cur.track_id => {
            cur.last_timeline_ms = Some(*position_ms);
            vec![timeline(cur, PlaybackState::Playing, *position_ms)]
        }
        TrackingEvent::Seeked { track_id, to_ms, .. } if *track_id == cur.track_id => {
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

/// Where play reports go. The app sends each to the track's server
/// (`crate::backend`); tests use a recorder.
#[async_trait]
pub trait PlayReporter: Send + Sync {
    /// True when a server is connected right now.
    async fn online(&self) -> bool;
    /// Now playing, paused or stopped, and where.
    async fn timeline(&self, r: &PlayReport) -> AppResult<()>;
    /// A finished play. `at` (unix seconds) is when it happened, for a play
    /// sent later; servers that can't take a date count it now.
    async fn scrobble(&self, track_id: &str, at: Option<i64>) -> AppResult<()>;
}

#[async_trait]
impl PlayReporter for PlexState {
    async fn online(&self) -> bool {
        self.client().await.is_ok()
    }

    async fn timeline(&self, r: &PlayReport) -> AppResult<()> {
        let report = TimelineReport {
            rating_key: r.track_id.clone(),
            state: r.state,
            time_ms: r.time_ms,
            duration_ms: r.duration_ms,
            play_queue_item_id: None,
        };
        self.client_for(&r.track_id).await?.timeline(&report).await
    }

    /// Plex's scrobble has no date: a play sent later counts now.
    async fn scrobble(&self, track_id: &str, _at: Option<i64>) -> AppResult<()> {
        self.client_for(track_id).await?.scrobble(track_id).await
    }
}

pub struct Tracker {
    plex: Arc<dyn PlayReporter>,
    db: DbHandle,
    settings: TrackingSettings,
    /// Wakes the retry loop when a scrobble is queued for later.
    queued: tokio::sync::Notify,
}

impl Tracker {
    /// Reports go to each track's server.
    pub fn new(backends: crate::backend::Backends, db: DbHandle, settings: TrackingSettings) -> Self {
        Self::with_reporter(Arc::new(crate::backend::routed::RoutedPlays(backends)), db, settings)
    }

    pub fn with_reporter(plex: Arc<dyn PlayReporter>, db: DbHandle, settings: TrackingSettings) -> Self {
        Self { plex, db, settings, queued: tokio::sync::Notify::new() }
    }

    fn reporting(&self) -> bool {
        self.settings.report_plays.load(Ordering::Relaxed)
    }

    /// Now-playing timelines go out (they make the live session friends see).
    fn sharing_now(&self) -> bool {
        self.settings.share_activity.load(Ordering::Relaxed) && !self.settings.private_session.load(Ordering::Relaxed)
    }

    async fn send_timeline(&self, r: PlayReport) {
        if !self.reporting() || !self.sharing_now() {
            return;
        }
        if let Err(e) = self.plex.timeline(&r).await {
            tracing::debug!("timeline not sent: {}", e.kind());
        }
    }

    async fn scrobble(&self, play_id: Option<i64>, track_id: String) {
        if !self.reporting() {
            return;
        }
        let sent = self.plex.scrobble(&track_id, None).await.is_ok();
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
                            rating_key: track_id,
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
        if !sent {
            self.queued.notify_one();
        }
    }

    /// Send queued scrobbles, oldest first. Stops at the first failure.
    /// Returns whether any are still waiting (so the retry loop keeps going).
    pub async fn replay_pending(&self) -> bool {
        const BATCH: u32 = 200;
        if !self.reporting() {
            return false;
        }
        let pending = match self.db.read_async(|c| listens::pending_reports(c, BATCH)).await {
            Ok(p) => p,
            Err(_) => return true,
        };
        if pending.is_empty() {
            return false;
        }
        if !self.plex.online().await {
            return true;
        }
        let total = pending.len();
        let mut sent = Vec::new();
        for r in pending {
            let ok = match r.kind {
                ReportKind::Scrobble => self.plex.scrobble(&r.rating_key, Some(r.at)).await.is_ok(),
                ReportKind::Timeline => true, // stale timelines are dropped
            };
            if !ok {
                break;
            }
            sent.push(r.id);
        }
        let all = sent.len() == total && total < BATCH as usize;
        if !sent.is_empty() {
            let _ = self
                .db
                .write_async(move |c| listens::mark_sent(c, &sent, now_s()))
                .await;
        }
        !all
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
                    // Unix ms, like every reader of the listen log expects.
                    started_at: *at,
                    duration_ms: track.duration_ms,
                    context_kind: kind,
                    context_id: id,
                    device: None,
                    offline: !self.plex.online().await,
                };
                let snap = PlaySnapshot::of_track(track);
                cur.play_id =
                    Some(self.db.write_async(move |c| listens::start_play_with(c, &start, Some(&snap))).await?);
            }
            // Time listened, not the position: a seek to the end isn't a play.
            TrackingEvent::Progress { track_id, listened_ms, .. } if *track_id == cur.track_id => {
                if let Some(id) = cur.play_id {
                    let ms = *listened_ms;
                    let upd = self.db.write_async(move |c| listens::progress(c, id, ms)).await?;
                    if upd.scrobble_now {
                        self.scrobble(Some(id), cur.track_id.clone()).await;
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
                        .write_async(move |c| listens::end_play(c, id, now_s() * 1000, ms, pos, done))
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
        let mut backfilling = true;
        let mut waiting = true;
        loop {
            // Backfill done and nothing queued: sleep until a scrobble fails.
            if !waiting && !backfilling {
                tracker.queued.notified().await;
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
            waiting = tracker.replay_pending().await;
            if backfilling {
                backfilling = tracker.backfill_step().await;
            }
        }
    });
}

impl Tracker {
    /// One bounded pass filling the snapshot on plays logged before plays
    /// kept one. Returns false once there is nothing left to fill.
    async fn backfill_step(&self) -> bool {
        match self.db.write_async(|c| listens::backfill_snapshots(c, listens::BACKFILL_BATCH)).await {
            Ok(n) => n > 0,
            Err(e) => {
                tracing::warn!("listen log backfill: {e}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Track;

    fn track(id: &str) -> Track {
        Track { mbid: None,
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
            rating_count: None,
            file: None,
            other_versions: None,
            song_status: None,
            file_release: None,
            rated_at: None,
            genres: Vec::new(),
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

        let progress =
            |ms| TrackingEvent::Progress { track_id: "1".into(), position_ms: ms, duration_ms: 200_000, listened_ms: ms };
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
        async fn timeline(&self, r: &PlayReport) -> AppResult<()> {
            self.timelines.lock().push(r.state);
            Ok(())
        }
        async fn scrobble(&self, track_id: &str, _at: Option<i64>) -> AppResult<()> {
            self.scrobbles.lock().push(track_id.to_string());
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
        settings.report_plays.store(report, Ordering::Relaxed);
        let tracker = Tracker::with_reporter(rec.clone(), db.clone(), settings);
        Rig { tracker, rec, db, _dir: dir }
    }

    /// Start a 200 s track, play 150 s of it and let it finish.
    async fn full_play(t: &Tracker) {
        let mut cur = Current::default();
        let tr = track("7");
        let evs = [
            TrackingEvent::TrackStarted { track: tr.clone(), item_id: "q1".into(), at: 1_000_000, context: None },
            TrackingEvent::Progress {
                track_id: "7".into(),
                position_ms: 150_000,
                duration_ms: 200_000,
                listened_ms: 150_000,
            },
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
    async fn seeking_near_the_end_and_finishing_is_not_a_play() {
        // 5 s heard, then a seek to 80% and on to the end: the position
        // passes every threshold, the time listened doesn't.
        let r = rig(false, true);
        let mut cur = Current::default();
        let tr = track("7");
        let evs = [
            TrackingEvent::TrackStarted { track: tr.clone(), item_id: "q1".into(), at: 1_000_000, context: None },
            TrackingEvent::Progress { track_id: "7".into(), position_ms: 5_000, duration_ms: 200_000, listened_ms: 5_000 },
            TrackingEvent::Seeked { track_id: "7".into(), from_ms: 5_000, to_ms: 160_000 },
            TrackingEvent::Progress { track_id: "7".into(), position_ms: 161_000, duration_ms: 200_000, listened_ms: 6_000 },
            TrackingEvent::Progress { track_id: "7".into(), position_ms: 199_000, duration_ms: 200_000, listened_ms: 44_000 },
            TrackingEvent::TrackEnded {
                track: tr,
                item_id: "q1".into(),
                listened_ms: 45_000,
                completed: true,
                skipped_at_ms: None,
            },
        ];
        for ev in &evs {
            r.tracker.handle(&mut cur, ev).await;
        }
        assert!(r.rec.scrobbles.lock().is_empty());
        let heard: i64 =
            r.db.read(|c| Ok(c.query_row("SELECT ms_listened FROM listen_log", [], |r| r.get(0))?)).unwrap();
        assert_eq!(heard, 45_000);
    }

    #[tokio::test]
    async fn private_session_writes_no_listen_log_and_shows_no_live_session() {
        let r = rig(true, true);
        full_play(&r.tracker).await;
        assert_eq!(listen_rows(&r.db), 0);
        // No timelines: friends on the server don't see it playing.
        assert!(r.rec.timelines.lock().is_empty());
        assert!(r.rec.scrobbles.lock().is_empty());
    }

    #[tokio::test]
    async fn private_session_plays_stay_out_of_wrapped() {
        let year = |db: &DbHandle| {
            db.write(|c| crate::stats::refresh(c).map(|_| ())).unwrap();
            db.read(|c| crate::stats::wrapped::wrapped(c, 1970)).unwrap().totals.plays
        };
        let open = rig(false, false);
        full_play(&open.tracker).await;
        assert_eq!(year(&open.db), 1, "a normal play counts");
        let private = rig(true, false);
        full_play(&private.tracker).await;
        assert_eq!(year(&private.db), 0);
    }

    #[tokio::test]
    async fn sharing_off_hides_the_live_session_but_keeps_history() {
        let r = rig(false, true);
        r.tracker.settings.share_activity.store(false, Ordering::Relaxed);
        full_play(&r.tracker).await;
        assert!(r.rec.timelines.lock().is_empty());
        assert_eq!(*r.rec.scrobbles.lock(), vec!["7".to_string()]);
        assert_eq!(listen_rows(&r.db), 1);
    }

    #[tokio::test]
    async fn a_play_keeps_what_the_song_was_even_when_it_is_not_in_the_cache() {
        let r = rig(false, true);
        full_play(&r.tracker).await;
        let snap = r
            .db
            .read(|c| {
                Ok(c.query_row(
                    "SELECT snap_title, snap_artist, snap_album, snap_album_artist FROM listen_log",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)),
                )?)
            })
            .unwrap();
        assert_eq!(snap, ("t".into(), "R".into(), "A".into(), "R".into()));
        assert!(!r.tracker.backfill_step().await, "nothing left to fill");
    }

    #[tokio::test]
    async fn replay_says_when_the_retry_loop_can_stop() {
        let r = rig(false, true);
        // Nothing queued: the loop can park.
        assert!(!r.tracker.replay_pending().await);
        r.db
            .write(|c| {
                listens::enqueue_report(
                    c,
                    &NewReport {
                        play_id: None,
                        kind: ReportKind::Scrobble,
                        rating_key: "7".into(),
                        state: None,
                        time_ms: None,
                        duration_ms: None,
                        at: 1,
                    },
                )
            })
            .unwrap();
        // Sent: nothing left, so it stops again.
        assert!(!r.tracker.replay_pending().await);
        assert_eq!(r.rec.scrobbles.lock().clone(), vec!["7".to_string()]);
        // Reporting off: nothing to retry.
        let off = rig(false, false);
        assert!(!off.tracker.replay_pending().await);
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
        r.tracker.settings.report_plays.store(false, Ordering::Relaxed);
        full_play(&r.tracker).await;
        assert_eq!(listen_rows(&r.db), 0);
        assert!(r.rec.timelines.lock().is_empty());
        assert!(r.rec.scrobbles.lock().is_empty());
    }
}
