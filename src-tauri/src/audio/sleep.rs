//! The sleep timer: playback fades out and pauses after a number of
//! minutes, at the end of the song playing, or at the end of the album
//! playing. It runs here in the core, so it keeps time while the window is
//! hidden or the phone is locked. The volume comes back up after the pause,
//! so the next play starts at the level it had. Per device, never synced,
//! gone when Platter quits.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::time::Instant;

use super::events::PlayerStatus;
use super::player::PlayerHandle;
use crate::error::{AppError, AppResult};
use crate::plex::state::EventSink;

/// Sent with the timer (or null) whenever it's set, cleared or runs out.
pub const EVENT_SLEEP: &str = "sleep://changed";
/// How long the fade at the end takes.
pub const FADE_MS: u64 = 6_000;
/// A song counts as ended this close to its end.
const END_SLACK_MS: u64 = 350;
/// Custom lengths, in minutes.
pub const MIN_MINUTES: u32 = 1;
pub const MAX_MINUTES: u32 = 12 * 60;
/// How often a running timer looks at the player, and how often near the end.
const POLL: Duration = Duration::from_millis(1000);
const POLL_NEAR: Duration = Duration::from_millis(200);
/// Steps in the fade.
const FADE_STEP: Duration = Duration::from_millis(200);

/// What the timer is asked to do (`sleep_set`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SleepChoice {
    Minutes { minutes: u32 },
    Track,
    Album,
}

/// A running timer, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SleepTimer {
    /// `ends_at_ms`: Unix ms when playback has paused.
    Minutes { minutes: u32, ends_at_ms: i64 },
    Track { track_id: String },
    Album { album_id: String },
}

/// What the timer needs to know about the player.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Now {
    pub playing: bool,
    pub track_id: Option<String>,
    pub album_id: Option<String>,
    /// Time left in the song playing.
    pub left_ms: u64,
    /// The album of the song after this one, if any.
    pub next_album_id: Option<String>,
    pub volume: f32,
}

/// The player, as the timer drives it.
#[async_trait]
pub trait SleepPlayer: Send + Sync {
    async fn now(&self) -> Option<Now>;
    async fn set_volume(&self, volume: f32) -> AppResult<()>;
    async fn pause(&self) -> AppResult<()>;
}

#[async_trait]
impl SleepPlayer for PlayerHandle {
    async fn now(&self) -> Option<Now> {
        let s = self.state().await.ok()?;
        let next = s.queue.user_queue.first().or(s.queue.next_up.first());
        Some(Now {
            playing: matches!(s.status, PlayerStatus::Playing | PlayerStatus::Buffering | PlayerStatus::Loading),
            track_id: s.track.as_ref().map(|t| t.id.clone()),
            album_id: s.track.as_ref().map(|t| t.album_id.clone()),
            left_ms: s.duration_ms.saturating_sub(s.position_ms),
            next_album_id: next.map(|q| q.track.album_id.clone()),
            volume: s.volume,
        })
    }
    async fn set_volume(&self, volume: f32) -> AppResult<()> {
        PlayerHandle::set_volume(self, volume).await
    }
    async fn pause(&self) -> AppResult<()> {
        PlayerHandle::pause(self).await
    }
}

/// Whether a song or album timer is due: its song is ending (or another one
/// started), or the album's last song is ending (or another album started).
pub fn song_due(t: &SleepTimer, now: &Now) -> bool {
    let ending = now.left_ms <= END_SLACK_MS + FADE_MS;
    match t {
        SleepTimer::Minutes { .. } => false,
        SleepTimer::Track { track_id } => now.track_id.as_deref() != Some(track_id) || ending,
        SleepTimer::Album { album_id } => {
            if now.album_id.as_deref() != Some(album_id) {
                return true;
            }
            let last = now.next_album_id.as_deref() != Some(album_id);
            last && ending
        }
    }
}

struct Running {
    /// Tells a stale task it was replaced or cancelled.
    id: u64,
    timer: SleepTimer,
}

struct Inner {
    player: Arc<dyn SleepPlayer>,
    events: Arc<dyn EventSink>,
    /// Unix ms, for the window's countdown.
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    running: Mutex<Option<Running>>,
    next_id: Mutex<u64>,
}

/// The sleep timer service. Cheap to clone; `manage` one.
#[derive(Clone)]
pub struct Sleep {
    inner: Arc<Inner>,
}

impl Sleep {
    pub fn new(player: Arc<dyn SleepPlayer>, events: Arc<dyn EventSink>, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        Self {
            inner: Arc::new(Inner { player, events, clock, running: Mutex::new(None), next_id: Mutex::new(0) }),
        }
    }

    pub fn get(&self) -> Option<SleepTimer> {
        self.inner.running.lock().as_ref().map(|r| r.timer.clone())
    }

    fn emit(&self) {
        let v = serde_json::to_value(self.get()).unwrap_or(serde_json::Value::Null);
        self.inner.events.emit(EVENT_SLEEP, v);
    }

    fn current(&self, id: u64) -> bool {
        self.inner.running.lock().as_ref().is_some_and(|r| r.id == id)
    }

    /// Clears the timer if it's still `id`. True when it was.
    fn clear(&self, id: u64) -> bool {
        let mut r = self.inner.running.lock();
        if r.as_ref().is_some_and(|r| r.id == id) {
            *r = None;
            return true;
        }
        false
    }

    /// Start (or replace) the timer.
    pub async fn set(&self, choice: SleepChoice) -> AppResult<SleepTimer> {
        let now = self.inner.player.now().await;
        let (timer, deadline) = match choice {
            SleepChoice::Minutes { minutes } => {
                if !(MIN_MINUTES..=MAX_MINUTES).contains(&minutes) {
                    return Err(AppError::Invalid(format!("Pick from {MIN_MINUTES} minute to {} hours.", MAX_MINUTES / 60)));
                }
                let ms = u64::from(minutes) * 60_000;
                let ends_at_ms = (self.inner.clock)() + ms as i64;
                (SleepTimer::Minutes { minutes, ends_at_ms }, Some(Instant::now() + Duration::from_millis(ms)))
            }
            SleepChoice::Track => match now.and_then(|n| n.track_id) {
                Some(track_id) => (SleepTimer::Track { track_id }, None),
                None => return Err(AppError::Invalid("Nothing is playing.".into())),
            },
            SleepChoice::Album => match now.and_then(|n| n.album_id).filter(|a| !a.is_empty()) {
                Some(album_id) => (SleepTimer::Album { album_id }, None),
                None => return Err(AppError::Invalid("Nothing is playing.".into())),
            },
        };
        let id = {
            let mut n = self.inner.next_id.lock();
            *n += 1;
            *n
        };
        *self.inner.running.lock() = Some(Running { id, timer: timer.clone() });
        self.emit();
        let me = self.clone();
        let t = timer.clone();
        tokio::spawn(async move { me.run(id, t, deadline).await });
        Ok(timer)
    }

    /// Stop the timer. A fade under way stops and the volume goes back.
    pub fn cancel(&self) {
        let had = self.inner.running.lock().take().is_some();
        if had {
            self.emit();
        }
    }

    async fn run(&self, id: u64, timer: SleepTimer, deadline: Option<Instant>) {
        // Wait until the fade should start.
        if let Some(end) = deadline {
            let fade_at = end.checked_sub(Duration::from_millis(FADE_MS)).unwrap_or(end);
            tokio::time::sleep_until(fade_at).await;
        } else {
            loop {
                if !self.current(id) {
                    return;
                }
                let Some(now) = self.inner.player.now().await else { break };
                if song_due(&timer, &now) {
                    break;
                }
                let near = now.left_ms < END_SLACK_MS + FADE_MS + 2_000;
                tokio::time::sleep(if near { POLL_NEAR } else { POLL }).await;
            }
        }
        if !self.current(id) {
            return;
        }
        self.fade_out(id).await;
    }

    /// Fade, pause, put the volume back. Paused already: just clear.
    async fn fade_out(&self, id: u64) {
        let now = self.inner.player.now().await;
        let Some(now) = now.filter(|n| n.playing) else {
            if self.clear(id) {
                self.emit();
            }
            return;
        };
        let from = now.volume;
        let steps = (FADE_MS / FADE_STEP.as_millis() as u64).max(1);
        for i in 1..=steps {
            tokio::time::sleep(FADE_STEP).await;
            if !self.current(id) {
                // Cancelled mid-fade.
                let _ = self.inner.player.set_volume(from).await;
                return;
            }
            let k = i as f32 / steps as f32;
            let _ = self.inner.player.set_volume((from * (1.0 - k)).max(0.0)).await;
        }
        if !self.clear(id) {
            let _ = self.inner.player.set_volume(from).await;
            return;
        }
        if let Err(e) = self.inner.player.pause().await {
            tracing::warn!("sleep timer couldn't pause: {e}");
        }
        let _ = self.inner.player.set_volume(from).await;
        self.emit();
    }
}

/// `sleep_get`: the running timer, or null.
#[tauri::command]
pub async fn sleep_get(sleep: State<'_, Sleep>) -> AppResult<Option<SleepTimer>> {
    Ok(sleep.get())
}

/// `sleep_set`: start (or replace) the timer.
#[tauri::command]
pub async fn sleep_set(sleep: State<'_, Sleep>, choice: SleepChoice) -> AppResult<SleepTimer> {
    sleep.set(choice).await
}

/// `sleep_cancel`: stop it.
#[tauri::command]
pub async fn sleep_cancel(sleep: State<'_, Sleep>) -> AppResult<()> {
    sleep.cancel();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        now: Mutex<Option<Now>>,
        calls: Mutex<Vec<String>>,
        fail_pause: Mutex<bool>,
    }

    #[async_trait]
    impl SleepPlayer for Fake {
        async fn now(&self) -> Option<Now> {
            self.now.lock().clone()
        }
        async fn set_volume(&self, v: f32) -> AppResult<()> {
            self.calls.lock().push(format!("volume {v:.2}"));
            if let Some(n) = self.now.lock().as_mut() {
                n.volume = v;
            }
            Ok(())
        }
        async fn pause(&self) -> AppResult<()> {
            self.calls.lock().push("pause".into());
            if *self.fail_pause.lock() {
                return Err(AppError::Internal("device gone".into()));
            }
            if let Some(n) = self.now.lock().as_mut() {
                n.playing = false;
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<serde_json::Value>>);
    impl EventSink for Events {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            assert_eq!(event, EVENT_SLEEP);
            self.0.lock().push(payload);
        }
    }

    fn playing(track: &str, album: &str, left_ms: u64, next_album: Option<&str>) -> Now {
        Now {
            playing: true,
            track_id: Some(track.into()),
            album_id: Some(album.into()),
            left_ms,
            next_album_id: next_album.map(Into::into),
            volume: 0.8,
        }
    }

    fn setup(now: Option<Now>) -> (Sleep, Arc<Fake>, Arc<Events>) {
        let fake = Arc::new(Fake::default());
        *fake.now.lock() = now;
        let events = Arc::new(Events::default());
        let sleep = Sleep::new(fake.clone(), events.clone(), Arc::new(|| 1_000_000));
        (sleep, fake, events)
    }

    fn calls(f: &Fake) -> Vec<String> {
        f.calls.lock().clone()
    }

    /// Let the timer task run for `ms` of (paused) time.
    async fn pass(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    #[test]
    fn songs_and_albums_fall_due_at_their_end() {
        let track = SleepTimer::Track { track_id: "t1".into() };
        assert!(!song_due(&track, &playing("t1", "a", 60_000, Some("a"))));
        assert!(song_due(&track, &playing("t1", "a", FADE_MS, Some("a"))), "the fade ends with the song");
        assert!(song_due(&track, &playing("t2", "a", 60_000, Some("a"))), "another song started");
        let album = SleepTimer::Album { album_id: "a".into() };
        assert!(!song_due(&album, &playing("t1", "a", FADE_MS, Some("a"))), "more of the album follows");
        assert!(song_due(&album, &playing("t9", "a", FADE_MS, Some("b"))), "its last song is ending");
        assert!(song_due(&album, &playing("t9", "a", FADE_MS, None)), "the queue ends with it");
        assert!(!song_due(&album, &playing("t9", "a", 60_000, None)));
        assert!(song_due(&album, &playing("x", "b", 60_000, Some("b"))), "another album started");
    }

    #[tokio::test(start_paused = true)]
    async fn minutes_fade_pause_and_put_the_volume_back() {
        let (sleep, fake, events) = setup(Some(playing("t1", "a", 600_000, Some("a"))));
        let t = sleep.set(SleepChoice::Minutes { minutes: 1 }).await.unwrap();
        assert_eq!(t, SleepTimer::Minutes { minutes: 1, ends_at_ms: 1_000_000 + 60_000 });
        assert_eq!(sleep.get(), Some(t));
        pass(60_000 - FADE_MS - 100).await;
        assert!(calls(&fake).is_empty(), "nothing before the fade");
        pass(FADE_MS + 500).await;
        let c = calls(&fake);
        assert_eq!(c.iter().filter(|c| c.starts_with("volume")).count(), 31, "30 fade steps and the level back");
        assert_eq!(&c[c.len() - 3..], ["volume 0.00", "pause", "volume 0.80"]);
        assert_eq!(sleep.get(), None);
        let ev = events.0.lock().clone();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0]["kind"], "minutes");
        assert_eq!(ev[0]["endsAtMs"], 1_060_000);
        assert!(ev[1].is_null());
    }

    #[tokio::test(start_paused = true)]
    async fn end_of_song_pauses_as_it_ends() {
        let (sleep, fake, _) = setup(Some(playing("t1", "a", 20_000, Some("a"))));
        sleep.set(SleepChoice::Track).await.unwrap();
        pass(5_000).await;
        assert!(calls(&fake).is_empty());
        fake.now.lock().as_mut().unwrap().left_ms = FADE_MS;
        pass(FADE_MS + 1_500).await;
        assert!(calls(&fake).contains(&"pause".to_string()));
        assert_eq!(sleep.get(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn end_of_album_waits_for_the_last_song() {
        let (sleep, fake, _) = setup(Some(playing("t1", "a", FADE_MS, Some("a"))));
        sleep.set(SleepChoice::Album).await.unwrap();
        pass(3_000).await;
        assert!(calls(&fake).is_empty(), "not the last song yet");
        *fake.now.lock() = Some(playing("t9", "a", FADE_MS, Some("b")));
        pass(FADE_MS + 1_500).await;
        assert!(calls(&fake).contains(&"pause".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn skipping_to_another_album_ends_it_there() {
        let (sleep, fake, _) = setup(Some(playing("t1", "a", 200_000, Some("a"))));
        sleep.set(SleepChoice::Album).await.unwrap();
        *fake.now.lock() = Some(playing("x1", "b", 200_000, Some("b")));
        pass(FADE_MS + 2_000).await;
        assert!(calls(&fake).contains(&"pause".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn paused_already_just_clears() {
        let mut now = playing("t1", "a", 600_000, Some("a"));
        now.playing = false;
        let (sleep, fake, events) = setup(Some(now));
        sleep.set(SleepChoice::Minutes { minutes: 1 }).await.unwrap();
        pass(61_000).await;
        assert!(calls(&fake).is_empty());
        assert_eq!(sleep.get(), None);
        assert!(events.0.lock().last().unwrap().is_null());
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_mid_fade_puts_the_volume_back_and_keeps_playing() {
        let (sleep, fake, events) = setup(Some(playing("t1", "a", 600_000, Some("a"))));
        sleep.set(SleepChoice::Minutes { minutes: 1 }).await.unwrap();
        pass(60_000 - FADE_MS + 1_000).await;
        assert!(!calls(&fake).is_empty(), "fading");
        sleep.cancel();
        pass(10_000).await;
        let c = calls(&fake);
        assert!(!c.contains(&"pause".to_string()));
        assert_eq!(c.last().unwrap(), "volume 0.80");
        assert!(events.0.lock().last().unwrap().is_null());
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_timer_replaces_the_old() {
        let (sleep, fake, _) = setup(Some(playing("t1", "a", 600_000, Some("a"))));
        sleep.set(SleepChoice::Minutes { minutes: 1 }).await.unwrap();
        sleep.set(SleepChoice::Minutes { minutes: 5 }).await.unwrap();
        pass(120_000).await;
        assert!(calls(&fake).is_empty(), "the one-minute timer is gone");
        pass(200_000).await;
        assert!(calls(&fake).contains(&"pause".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_pause_still_clears_and_puts_the_volume_back() {
        let (sleep, fake, _) = setup(Some(playing("t1", "a", 600_000, Some("a"))));
        *fake.fail_pause.lock() = true;
        sleep.set(SleepChoice::Minutes { minutes: 1 }).await.unwrap();
        pass(61_000).await;
        assert_eq!(sleep.get(), None);
        assert_eq!(calls(&fake).last().unwrap(), "volume 0.80");
    }

    #[tokio::test]
    async fn refuses_what_it_cant_do() {
        let (sleep, _, events) = setup(None);
        assert!(matches!(sleep.set(SleepChoice::Track).await, Err(AppError::Invalid(_))));
        assert!(matches!(sleep.set(SleepChoice::Album).await, Err(AppError::Invalid(_))));
        assert!(matches!(sleep.set(SleepChoice::Minutes { minutes: 0 }).await, Err(AppError::Invalid(_))));
        assert!(matches!(sleep.set(SleepChoice::Minutes { minutes: MAX_MINUTES + 1 }).await, Err(AppError::Invalid(_))));
        assert!(events.0.lock().is_empty());
        sleep.cancel();
        assert!(events.0.lock().is_empty(), "nothing to cancel says nothing");
    }

    #[test]
    fn choices_and_timers_cross_the_bridge_in_camel_case() {
        let c: SleepChoice = serde_json::from_value(serde_json::json!({ "kind": "minutes", "minutes": 30 })).unwrap();
        assert_eq!(c, SleepChoice::Minutes { minutes: 30 });
        let c: SleepChoice = serde_json::from_value(serde_json::json!({ "kind": "album" })).unwrap();
        assert_eq!(c, SleepChoice::Album);
        let t = serde_json::to_value(SleepTimer::Track { track_id: "t1".into() }).unwrap();
        assert_eq!(t, serde_json::json!({ "kind": "track", "trackId": "t1" }));
    }
}
