//! The service against an in-memory Discord, and the cover lookups against
//! fake HTTP. Nothing here talks to a real Discord client or uploads.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::broadcast;

use super::activity::APP_ICON;
use super::art::*;
use super::ipc::fake::{FAKE_SIGNATURE, FakeConnector, FakeDiscord};
use super::*;
use crate::artistinfo::http::fake::FakeHttp;
use crate::artistinfo::http::HttpResponse;
use crate::audio::testutil::track;
use crate::model::Track;
use crate::plex::keychain::{MemoryStore, TokenStore};

const COVER: &str = "https://coverartarchive.org/release/0b1e2a3c-0000-4000-8000-000000000001/front-500";
const LINK: &str = "https://song.link/i/42";
const MBID: &str = "0b1e2a3c-0000-4000-8000-000000000001";

// ---------------------------------------------------------------- fakes

#[derive(Default)]
struct FakeLib {
    unreleased_albums: HashSet<String>,
    link_calls: AtomicUsize,
}

#[async_trait]
impl Library for FakeLib {
    async fn unreleased(&self, album_id: &str) -> bool {
        self.unreleased_albums.contains(album_id)
    }
    async fn song_link(&self, _: &str) -> Option<String> {
        self.link_calls.fetch_add(1, Ordering::SeqCst);
        Some(LINK.into())
    }
}

struct FakeArt {
    outcome: Mutex<ArtOutcome>,
    delay: Duration,
    calls: AtomicUsize,
    /// What a re-upload gives, and the failed links each one was asked with.
    replacement: Mutex<ArtOutcome>,
    replaced: Mutex<Vec<Vec<String>>>,
}

impl FakeArt {
    fn new(outcome: ArtOutcome) -> Self {
        Self {
            outcome: Mutex::new(outcome),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
            replacement: Mutex::new(ArtOutcome::None),
            replaced: Mutex::default(),
        }
    }
}

#[async_trait]
impl CoverArt for FakeArt {
    async fn resolve(&self, _: &ArtRequest) -> ArtOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.outcome.lock().clone()
    }
    async fn replace(&self, _: &ArtRequest, failed: &[String]) -> ArtOutcome {
        self.replaced.lock().push(failed.to_vec());
        self.replacement.lock().clone()
    }
    fn key_state(&self) -> KeyState {
        KeyState::default()
    }
}

#[derive(Default)]
struct Statuses(Mutex<Vec<DiscordStatus>>);

impl StatusSink for Statuses {
    fn changed(&self, s: &DiscordStatus) {
        self.0.lock().push(*s);
    }
}

/// Discord's image proxy: answers from a script, then a fixed answer
/// (an image by default).
struct FakeProxy {
    script: Mutex<std::collections::VecDeque<(u16, &'static str)>>,
    then: Mutex<(u16, &'static str)>,
    log: Mutex<Vec<(String, tokio::time::Instant)>>,
}

impl Default for FakeProxy {
    fn default() -> Self {
        Self { script: Mutex::default(), then: Mutex::new((200, "image/jpeg")), log: Mutex::default() }
    }
}

impl FakeProxy {
    fn calls(&self) -> usize {
        self.log.lock().len()
    }
}

#[async_trait]
impl crate::artistinfo::http::InfoHttp for FakeProxy {
    async fn get(&self, url: &str, _: &[(&'static str, String)]) -> crate::error::AppResult<HttpResponse> {
        self.log.lock().push((url.to_owned(), tokio::time::Instant::now()));
        let (status, ct) = self.script.lock().pop_front().unwrap_or(*self.then.lock());
        if status == 0 {
            return Err(crate::error::AppError::Network("connection reset".into()));
        }
        Ok(HttpResponse { status, content_type: Some(ct.into()), body: "x".into(), ..Default::default() })
    }
}

struct Rig {
    proxy: Arc<FakeProxy>,
    discord: Arc<FakeDiscord>,
    events: broadcast::Sender<TrackingEvent>,
    switches: DiscordSwitches,
    handle: DiscordHandle,
    art: Arc<FakeArt>,
    lib: Arc<FakeLib>,
    statuses: Arc<Statuses>,
    task: tokio::task::JoinHandle<()>,
}

fn rig_with(art: FakeArt, lib: FakeLib, settings: DiscordSettings) -> Rig {
    rig_cfg(art, lib, settings, |_| {})
}

fn rig_cfg(art: FakeArt, lib: FakeLib, settings: DiscordSettings, tweak: impl FnOnce(&mut Config)) -> Rig {
    let discord = FakeDiscord::new();
    let (events, rx) = broadcast::channel(64);
    let switches = DiscordSwitches::default();
    switches.set(settings);
    let art = Arc::new(art);
    let lib = Arc::new(lib);
    let statuses = Arc::new(Statuses::default());
    let proxy = Arc::new(FakeProxy::default());
    let deps = Deps {
        connector: Arc::new(FakeConnector(discord.clone())),
        art: art.clone(),
        library: lib.clone(),
        status: statuses.clone(),
        http: proxy.clone(),
    };
    let mut cfg = Config::new("1234567890");
    cfg.clock = Arc::new(now_ms);
    tweak(&mut cfg);
    let (handle, task) = spawn(cfg, deps, rx, switches.subscribe());
    Rig { proxy, discord, events, switches, handle, art, lib, statuses, task }
}

fn rig() -> Rig {
    rig_with(FakeArt::new(ArtOutcome::Url(COVER.into())), FakeLib::default(), DiscordSettings::default())
}

fn song(id: &str) -> Track {
    let mut t = track(id, "Band", &format!("/library/parts/{id}/1/file.flac"));
    t.title = format!("Song {id}");
    t.album_id = format!("al{id}");
    t.album_title = "Record".into();
    t.duration_ms = 200_000;
    t.thumb = Some(format!("/library/metadata/{id}/thumb/1"));
    t
}

/// The paused tokio clock as unix ms, so positions and time agree.
/// The base is per thread: each test runs on its own current-thread
/// runtime with its own paused clock, and one shared base would tie a test's
/// clock to whichever test happened to read it first.
fn now_ms() -> i64 {
    thread_local! {
        static BASE: tokio::time::Instant = tokio::time::Instant::now();
    }
    let base = BASE.with(|b| *b);
    1_700_000_000_000 + tokio::time::Instant::now().saturating_duration_since(base).as_millis() as i64
}

fn started(t: Track) -> TrackingEvent {
    TrackingEvent::TrackStarted { track: t, item_id: "q1".into(), at: now_ms(), context: None }
}

async fn wait(secs: f64) {
    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
}

fn last(d: &FakeDiscord) -> Option<Value> {
    d.activities().last().cloned().flatten()
}

// ---------------------------------------------------------------- service

#[tokio::test(start_paused = true)]
async fn status_appears_within_2s_with_cover_and_find_this_song() {
    let r = rig();
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    let acts = r.discord.activities();
    assert_eq!(acts.len(), 1, "{acts:?}");
    let a = acts[0].clone().unwrap();
    assert_eq!(a["type"], 2, "Listening");
    assert_eq!(a["details"], "Song 1");
    assert_eq!(a["state"], "by Band");
    assert_eq!(a["status_display_type"], 0);
    assert!(a["assets"].get("small_image").is_none());
    assert_eq!(a["assets"]["large_image"], COVER);
    assert_eq!(a["assets"]["large_text"], "Record");
    assert_eq!(a["buttons"][0]["url"], LINK);
    let end = a["timestamps"]["end"].as_i64().unwrap() - a["timestamps"]["start"].as_i64().unwrap();
    assert_eq!(end, 200_000);
    assert!(r.handle.status().connected);
}

#[tokio::test(start_paused = true)]
async fn a_slow_cover_never_holds_the_status_back() {
    let mut art = FakeArt::new(ArtOutcome::Url(COVER.into()));
    art.delay = Duration::from_secs(8);
    let r = rig_with(art, FakeLib::default(), DiscordSettings::default());
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], APP_ICON);
    // The cover lands at 8 s; the next update is due 15 s after the first.
    wait(15.0).await;
    assert_eq!(r.discord.activities().len(), 2);
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], COVER);
}

#[tokio::test(start_paused = true)]
async fn updates_are_coalesced_to_one_per_15s_with_the_latest_state() {
    let r = rig();
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    for id in ["2", "3", "4", "5"] {
        r.events.send(started(song(id))).unwrap();
        wait(1.0).await;
    }
    wait(8.0).await; // 14 s in: still only the first
    assert_eq!(r.discord.activities().len(), 1);
    wait(10.0).await;
    let acts = r.discord.activities();
    assert_eq!(acts.len(), 2, "one update for four changes");
    assert_eq!(acts[1].as_ref().unwrap()["details"], "Song 5");
    wait(30.0).await;
    assert_eq!(r.discord.activities().len(), 2, "nothing new to say");
}

#[tokio::test(start_paused = true)]
async fn pausing_clears_the_status_by_default() {
    let r = rig();
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    r.events.send(TrackingEvent::Paused { track_id: "1".into(), position_ms: 30_000 }).unwrap();
    wait(16.0).await;
    assert_eq!(r.discord.activities().last(), Some(&None), "cleared");
    r.events.send(TrackingEvent::Resumed { track_id: "1".into(), position_ms: 30_000 }).unwrap();
    wait(16.0).await;
    let a = last(&r.discord).unwrap();
    let start = a["timestamps"]["start"].as_i64().unwrap();
    assert!((now_ms() - start - 30_000).abs() < 20_000, "resumes from 0:30");
}

#[tokio::test(start_paused = true)]
async fn pausing_can_show_paused_instead() {
    let r = rig_with(
        FakeArt::new(ArtOutcome::Url(COVER.into())),
        FakeLib::default(),
        DiscordSettings { clear_on_pause: false, ..Default::default() },
    );
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    r.events.send(TrackingEvent::Paused { track_id: "1".into(), position_ms: 30_000 }).unwrap();
    wait(16.0).await;
    let a = last(&r.discord).unwrap();
    assert!(a.get("timestamps").is_none(), "no progress bar while paused");
    assert_eq!(a["assets"]["large_text"], "Record (paused)");
    assert_eq!(a["details"], "Song 1");
}

#[tokio::test(start_paused = true)]
async fn seeking_moves_the_progress_bar() {
    let r = rig();
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    let before = last(&r.discord).unwrap()["timestamps"]["start"].as_i64().unwrap();
    r.events.send(TrackingEvent::Seeked { track_id: "1".into(), from_ms: 2_000, to_ms: 120_000 }).unwrap();
    wait(16.0).await;
    let after = last(&r.discord).unwrap()["timestamps"]["start"].as_i64().unwrap();
    assert!(before - after > 100_000, "start moved back by the seek: {before} {after}");
}

#[tokio::test(start_paused = true)]
async fn ordinary_progress_ticks_send_nothing() {
    let r = rig();
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    for s in 2..40u64 {
        r.events
            .send(TrackingEvent::Progress { track_id: "1".into(), position_ms: s * 1000, duration_ms: 200_000, listened_ms: s * 1000 })
            .unwrap();
        wait(1.0).await;
    }
    assert_eq!(r.discord.activities().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn stopping_clears_but_a_gapless_change_does_not_flash() {
    let r = rig();
    r.events.send(started(song("1"))).unwrap();
    wait(16.0).await;
    // Track 1 ends and 2 starts at once: one update, straight to song 2.
    r.events
        .send(TrackingEvent::TrackEnded { track: song("1"), item_id: "q1".into(), listened_ms: 1, completed: true, skipped_at_ms: None })
        .unwrap();
    r.events.send(started(song("2"))).unwrap();
    wait(2.0).await;
    let acts = r.discord.activities();
    assert_eq!(acts.len(), 2);
    assert_eq!(acts[1].as_ref().unwrap()["details"], "Song 2");
    // Stop: the status goes.
    r.events
        .send(TrackingEvent::TrackEnded { track: song("2"), item_id: "q2".into(), listened_ms: 1, completed: false, skipped_at_ms: Some(1) })
        .unwrap();
    wait(16.0).await;
    assert_eq!(r.discord.activities().last(), Some(&None));
}

#[tokio::test(start_paused = true)]
async fn a_private_session_hides_everything_and_turning_off_leaves_discord() {
    let r = rig();
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    r.switches.set(DiscordSettings { private_session: true, ..Default::default() });
    wait(16.0).await;
    assert_eq!(r.discord.activities().last(), Some(&None));
    r.switches.set(DiscordSettings::default());
    wait(16.0).await;
    assert!(last(&r.discord).is_some(), "back after the private session");
    r.switches.set(DiscordSettings { enabled: false, ..Default::default() });
    wait(16.0).await;
    assert_eq!(r.discord.activities().last(), Some(&None));
    assert!(!r.handle.status().connected, "off disconnects");
    let n = r.discord.frames.lock().len();
    r.events.send(started(song("2"))).unwrap();
    wait(30.0).await;
    assert_eq!(r.discord.frames.lock().len(), n, "nothing sent while off");
}

#[tokio::test(start_paused = true)]
async fn unreleased_songs_follow_the_settings_and_never_look_up_links() {
    let lib = FakeLib { unreleased_albums: ["al1".to_string()].into(), ..Default::default() };
    let r = rig_with(FakeArt::new(ArtOutcome::Url(COVER.into())), lib, DiscordSettings::default());
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    let a = last(&r.discord).unwrap();
    assert!(a.get("buttons").is_none());
    assert_eq!(r.lib.link_calls.load(Ordering::SeqCst), 0, "no link search for unreleased music");
    r.switches.set(DiscordSettings { show_unreleased: false, ..Default::default() });
    wait(16.0).await;
    assert_eq!(r.discord.activities().last(), Some(&None), "nothing while an unreleased song plays");
}

#[tokio::test(start_paused = true)]
async fn hidden_unreleased_songs_never_reach_the_cover_lookups() {
    let lib = FakeLib { unreleased_albums: ["al1".to_string()].into(), ..Default::default() };
    let s = DiscordSettings { unreleased_art: false, ..Default::default() };
    let r = rig_with(FakeArt::new(ArtOutcome::Url(COVER.into())), lib, s);
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], APP_ICON);
    assert_eq!(r.art.calls.load(Ordering::SeqCst), 0, "no upload or lookup for a hidden cover");
}

#[tokio::test(start_paused = true)]
async fn no_token_or_server_address_is_ever_sent_to_discord() {
    // Even if a lookup handed back a server URL, it must not get through.
    let art = FakeArt::new(ArtOutcome::Url("https://192.0.2.7:32400/library/metadata/1/thumb?X-Plex-Token=SECRET".into()));
    let r = rig_with(art, FakeLib::default(), DiscordSettings { clear_on_pause: false, ..Default::default() });
    let mut t = song("1");
    t.thumb = Some("/library/metadata/1/thumb/1?X-Plex-Token=SECRET".into());
    t.part_key = "https://192.0.2.7:32400/library/parts/1/file.flac?X-Plex-Token=SECRET".into();
    r.events.send(started(t)).unwrap();
    wait(2.0).await;
    r.events.send(TrackingEvent::Paused { track_id: "1".into(), position_ms: 5_000 }).unwrap();
    wait(16.0).await;
    assert_eq!(r.discord.activities().len(), 2);
    let wire = r.discord.wire_text();
    for bad in ["SECRET", "X-Plex-Token", "192.0.2.", "32400", "plex.direct", "/library/", "/photo/"] {
        assert!(!wire.contains(bad), "{bad} reached Discord: {wire}");
    }
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], APP_ICON);
}

#[tokio::test(start_paused = true)]
async fn discord_closing_mid_song_never_touches_playback_and_reconnects() {
    let r = rig();
    r.discord.hang_up_after.store(1, Ordering::SeqCst);
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    assert_eq!(r.discord.activities().len(), 1);
    // Discord quits. Player events keep flowing without blocking.
    for s in 0..100u64 {
        r.events
            .send(TrackingEvent::Progress { track_id: "1".into(), position_ms: 3000 + s, duration_ms: 200_000, listened_ms: 3000 + s })
            .expect("the player's events never fail because of Discord");
    }
    r.events.send(TrackingEvent::Seeked { track_id: "1".into(), from_ms: 3_000, to_ms: 60_000 }).unwrap();
    // The loss shows at the next update (15 s after the last).
    wait(16.0).await;
    assert!(!r.handle.status().connected);
    assert!(!r.task.is_finished(), "the service carries on");
    // Discord comes back: the latest status is sent again.
    r.discord.down.store(false, Ordering::SeqCst);
    wait(31.0).await;
    assert!(r.handle.status().connected);
    assert!(r.discord.connects.load(Ordering::SeqCst) >= 2);
    assert_eq!(last(&r.discord).unwrap()["details"], "Song 1");
    let statuses = r.statuses.0.lock().clone();
    assert!(statuses.iter().any(|s| !s.connected), "Settings heard about the disconnect");
}

#[tokio::test(start_paused = true)]
async fn discord_not_running_is_quiet_and_looks_up_nothing() {
    let r = rig();
    r.discord.down.store(true, Ordering::SeqCst);
    r.events.send(started(song("1"))).unwrap();
    wait(40.0).await;
    assert!(!r.handle.status().connected);
    assert_eq!(r.art.calls.load(Ordering::SeqCst), 0, "no cover lookups without Discord");
    assert_eq!(r.lib.link_calls.load(Ordering::SeqCst), 0);
    // Discord starts: picked up within the retry time.
    r.discord.down.store(false, Ordering::SeqCst);
    wait(17.0).await;
    assert!(r.handle.status().connected);
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], COVER);
}

#[tokio::test(start_paused = true)]
async fn a_client_that_refuses_listening_gets_playing() {
    let r = rig();
    r.discord.refuse_listening.store(true, Ordering::SeqCst);
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    let acts = r.discord.activities();
    assert_eq!(acts.len(), 3, "tried Listening, without the status line, then Playing");
    assert_eq!(acts[2].as_ref().unwrap()["type"], 0);
}

#[tokio::test(start_paused = true)]
async fn a_client_that_refuses_the_status_line_gets_it_left_out() {
    let r = rig_with(
        FakeArt::new(ArtOutcome::Url(COVER.into())),
        FakeLib::default(),
        DiscordSettings { status_line: StatusLine::Artist, ..Default::default() },
    );
    r.discord.refuse_status_line.store(true, Ordering::SeqCst);
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    let acts = r.discord.activities();
    assert_eq!(acts.len(), 2, "{acts:?}");
    assert_eq!(acts[0].as_ref().unwrap()["status_display_type"], 1);
    let a = acts[1].as_ref().unwrap();
    assert!(a.get("status_display_type").is_none());
    assert_eq!(a["type"], 2, "still Listening");
    // The next song goes straight to the accepted shape.
    r.events.send(started(song("2"))).unwrap();
    wait(16.0).await;
    assert!(last(&r.discord).unwrap().get("status_display_type").is_none());
    assert_eq!(r.discord.activities().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn changing_the_status_line_updates_discord() {
    let r = rig();
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    assert_eq!(last(&r.discord).unwrap()["status_display_type"], 0);
    r.switches.set(DiscordSettings { status_line: StatusLine::Song, ..Default::default() });
    wait(16.0).await;
    assert_eq!(last(&r.discord).unwrap()["status_display_type"], 2);
}

#[tokio::test(start_paused = true)]
async fn a_failed_upload_shows_the_app_icon_and_tells_settings() {
    let art = FakeArt::new(ArtOutcome::UploadFailed("Imgur answered 500".into()));
    let r = rig_with(art, FakeLib::default(), DiscordSettings { upload_art: true, ..Default::default() });
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], APP_ICON);
    assert!(r.handle.status().upload_failed);
    assert!(r.statuses.0.lock().iter().any(|s| s.upload_failed));
    r.handle.key_changed();
    assert!(!r.handle.status().upload_failed, "a new key clears it");
}

// ---------------------------------------------------------------- proxied cover

fn proxied_cover() -> String {
    format!("{MEDIA_PROXY}external/{FAKE_SIGNATURE}/https/{}", COVER.trim_start_matches("https://"))
}

#[tokio::test(start_paused = true)]
async fn a_cover_discord_can_show_is_checked_once_and_never_resent() {
    let r = rig();
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    assert_eq!(r.discord.activities().len(), 1);
    assert_eq!(r.proxy.calls(), 1);
    assert_eq!(r.proxy.log.lock()[0].0, proxied_cover(), "the path after mp: on Discord's proxy");
    // A later update of the same song doesn't check the cover again.
    r.events.send(TrackingEvent::Seeked { track_id: "1".into(), from_ms: 2_000, to_ms: 120_000 }).unwrap();
    wait(16.0).await;
    assert_eq!(r.discord.activities().len(), 2);
    wait(70.0).await;
    assert_eq!(r.discord.activities().len(), 2, "no resend");
    assert_eq!(r.proxy.calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_cover_discord_shows_only_later_is_resent_exactly_once() {
    let r = rig();
    r.proxy.script.lock().push_back((404, "text/html"));
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    assert_eq!(r.discord.activities().len(), 1);
    assert_eq!(r.proxy.calls(), 1);
    wait(5.0).await; // 7 s in: the second check worked
    assert_eq!(r.proxy.calls(), 2);
    let log = r.proxy.log.lock().clone();
    assert_eq!(log[1].1 - log[0].1, PROXY_RECHECKS[0]);
    assert_eq!(r.discord.activities().len(), 1, "the resend waits for the gap");
    wait(12.0).await;
    let acts = r.discord.activities();
    assert_eq!(acts.len(), 2, "{acts:?}");
    assert_eq!(acts[1], acts[0], "the same status again");
    assert_eq!(acts[1].as_ref().unwrap()["assets"]["large_image"], COVER);
    wait(70.0).await;
    assert_eq!(r.discord.activities().len(), 2, "exactly one resend");
    assert_eq!(r.proxy.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_cover_discord_never_shows_falls_back_to_the_app_icon_for_that_play() {
    let r = rig();
    // Not an image, a dropped connection, a server error, then not found.
    r.proxy.script.lock().extend([(200, "text/html"), (0, ""), (503, "text/plain")]);
    *r.proxy.then.lock() = (404, "text/html");
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], COVER);
    wait(50.0).await;
    assert_eq!(r.proxy.calls(), 3);
    assert_eq!(r.discord.activities().len(), 1);
    wait(15.0).await; // past the last check, about a minute in
    assert_eq!(r.proxy.calls(), 4);
    let acts = r.discord.activities();
    assert_eq!(acts.len(), 2, "{acts:?}");
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], APP_ICON);
    assert_eq!(last(&r.discord).unwrap()["details"], "Song 1");
    wait(60.0).await;
    assert_eq!(r.proxy.calls(), 4, "no more checks");
    assert_eq!(r.discord.activities().len(), 2);
    // Not remembered as bad: the next play tries the cover again.
    *r.proxy.then.lock() = (200, "image/jpeg");
    r.events.send(started(song("1"))).unwrap();
    wait(16.0).await;
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], COVER);
    assert_eq!(r.proxy.calls(), 5);
}

#[tokio::test(start_paused = true)]
async fn the_app_icon_and_a_private_session_are_never_checked() {
    let r = rig_with(FakeArt::new(ArtOutcome::None), FakeLib::default(), DiscordSettings::default());
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], APP_ICON);
    assert_eq!(r.proxy.calls(), 0);

    // A cover that recovers while a private session is on isn't resent.
    let r = rig();
    r.proxy.script.lock().push_back((404, "text/html"));
    r.events.send(started(song("1"))).unwrap();
    wait(2.0).await;
    r.switches.set(DiscordSettings { private_session: true, ..Default::default() });
    wait(30.0).await;
    assert_eq!(r.discord.activities().last(), Some(&None));
    let n = r.discord.activities().len();
    wait(60.0).await;
    assert_eq!(r.discord.activities().len(), n, "nothing sent in a private session");
}

// ---------------------------------------------------------------- covers

struct Mbids(Option<String>, AtomicUsize);

#[async_trait]
impl AlbumMbid for Mbids {
    async fn release_mbid(&self, _: &str) -> crate::error::AppResult<Option<String>> {
        self.1.fetch_add(1, Ordering::SeqCst);
        Ok(self.0.clone())
    }
}

struct Covers;

#[async_trait]
impl CoverBytes for Covers {
    async fn cover(&self, thumb: &str, px: u32) -> crate::error::AppResult<(Vec<u8>, String)> {
        assert_eq!(px, COVER_PX);
        assert!(thumb.starts_with("/library/"));
        Ok((b"\xff\xd8JPEGDATA".to_vec(), "image/jpeg".into()))
    }
}

type Post = (String, Vec<(&'static str, String)>, String, Vec<u8>);

#[derive(Default)]
struct FakeUpload {
    answer: Mutex<Option<HttpResponse>>,
    /// Answers by upload URL, ahead of `answer` (None: the host is down).
    routes: Mutex<Vec<(&'static str, Option<HttpResponse>)>>,
    log: Mutex<Vec<Post>>,
}

impl FakeUpload {
    fn route(&self, url: &'static str, r: Option<HttpResponse>) {
        self.routes.lock().retain(|(u, _)| *u != url);
        self.routes.lock().push((url, r));
    }
    fn urls(&self) -> Vec<String> {
        self.log.lock().iter().map(|p| p.0.clone()).collect()
    }
}

#[async_trait]
impl UploadHttp for FakeUpload {
    async fn post(&self, url: &str, headers: &[(&'static str, String)], ct: &str, body: Vec<u8>) -> crate::error::AppResult<HttpResponse> {
        self.log.lock().push((url.into(), headers.to_vec(), ct.into(), body));
        let routed = self.routes.lock().iter().find(|(u, _)| *u == url).map(|(_, r)| r.clone());
        match routed.unwrap_or_else(|| self.answer.lock().clone()) {
            Some(r) => Ok(r),
            None => Err(crate::error::AppError::Network("connection refused".into())),
        }
    }
}

fn imgur_ok() -> HttpResponse {
    HttpResponse {
        status: 200,
        body: r#"{"data":{"link":"https://i.imgur.com/AbC123.jpg","deletehash":"x"},"success":true,"status":200}"#.into(),
        ..Default::default()
    }
}

struct Covers3 {
    http: Arc<FakeHttp>,
    up: Arc<FakeUpload>,
    mbids: Arc<Mbids>,
    keys: Arc<MemoryStore>,
    r: ArtResolver,
    db: DbHandle,
    _dir: tempfile::TempDir,
}

fn resolver(mbid: Option<&str>) -> Covers3 {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("d.db")).unwrap();
    let http = Arc::new(FakeHttp::new());
    let up = Arc::new(FakeUpload::default());
    let mbids = Arc::new(Mbids(mbid.map(str::to_owned), AtomicUsize::new(0)));
    let keys = Arc::new(MemoryStore::new());
    let r = ArtResolver::new(db.clone(), http.clone(), up.clone(), mbids.clone(), Arc::new(Covers), keys.clone())
        .with_musicbrainz_limiter(Arc::new(crate::artistinfo::limit::Limiter::per_second(1000.0)));
    Covers3 { http, up, mbids, keys, r, db, _dir: dir }
}

use crate::db::DbHandle;

fn req(upload: bool) -> ArtRequest {
    ArtRequest {
        album_id: "al1".into(),
        thumb: Some("/library/metadata/9/thumb/1".into()),
        artist: "Band".into(),
        album: "Record (Deluxe Edition)".into(),
        unreleased: false,
        use_archive: true,
        upload,
        anon: false,
        hosts: CoverHost::Auto,
    }
}

const GROUP: &str = "7c3e2a3c-0000-4000-8000-000000000002";
const GROUP_COVER: &str = "https://coverartarchive.org/release-group/7c3e2a3c-0000-4000-8000-000000000002/front-500";

fn mb_reply(title: &str, artist: &str, score: u64) -> String {
    serde_json::json!({ "release-groups": [{
        "id": GROUP, "score": score, "title": title,
        "artist-credit": [{ "name": artist, "artist": { "name": artist } }]
    }]})
    .to_string()
}

fn catbox_ok() -> HttpResponse {
    HttpResponse { status: 200, body: "https://files.catbox.moe/ab12cd.jpg".into(), ..Default::default() }
}

#[tokio::test]
async fn a_sure_musicbrainz_match_gives_the_release_group_cover() {
    let c = resolver(None);
    c.http.route("musicbrainz.org/ws/2/release-group", 200, &mb_reply("Record", "Band", 100));
    c.http.route("coverartarchive.org/release-group/", 200, r#"{"images":[{"front":true}]}"#);
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::Url(GROUP_COVER.into()));
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::Url(GROUP_COVER.into()));
    assert_eq!(c.http.calls("musicbrainz.org"), 1, "cached after the first");
    let search = &c.http.urls()[0];
    let search = search.to_lowercase();
    assert!(search.contains("record") && !search.contains("deluxe"), "edition notes aren't sent: {search}");
    assert!(search.contains("band"));
}

#[tokio::test]
async fn near_misses_are_refused() {
    for (title, artist, score) in [
        ("Record Two", "Band", 100),
        ("Record", "Band Tribute", 100),
        ("Record", "Band", 92),
    ] {
        let c = resolver(None);
        c.http.route("musicbrainz.org/ws/2/release-group", 200, &mb_reply(title, artist, score));
        c.http.route("coverartarchive.org/release-group/", 200, r#"{"images":[{"front":true}]}"#);
        assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::None, "{title} / {artist} / {score}");
        assert_eq!(c.http.calls("coverartarchive.org"), 0);
    }
    // Two albums of the same name by the same artist: no telling which.
    let both = serde_json::json!({ "release-groups": [
        { "id": GROUP, "score": 100, "title": "Record", "artist-credit": [{ "name": "Band" }] },
        { "id": MBID, "score": 100, "title": "Record", "artist-credit": [{ "name": "Band" }] },
    ]});
    assert_eq!(crate::artistinfo::musicbrainz::pick_release_group(&both, "Band", "Record"), None);
}

#[tokio::test]
async fn a_release_group_without_a_front_cover_is_a_cached_miss() {
    let c = resolver(None);
    c.http.route("musicbrainz.org/ws/2/release-group", 200, &mb_reply("Record", "Band", 100));
    c.http.route("coverartarchive.org/release-group/", 404, "Not Found");
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::None);
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::None);
    assert_eq!(c.http.calls("musicbrainz.org"), 1, "the miss is remembered");
    assert_eq!(c.http.calls("coverartarchive.org"), 1);
}

#[tokio::test]
async fn a_failed_search_is_not_cached() {
    let c = resolver(None);
    c.http.fail("musicbrainz.org");
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::None);
    c.http.route("musicbrainz.org", 200, &mb_reply("Record", "Band", 100));
    c.http.route("coverartarchive.org/release-group/", 200, r#"{"images":[{"front":true}]}"#);
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::Url(GROUP_COVER.into()));
}

#[tokio::test]
async fn unreleased_albums_are_never_looked_up_only_uploaded() {
    let c = resolver(Some(MBID));
    c.http.route("coverartarchive.org", 200, r#"{"images":[{"front":true}]}"#);
    c.http.route("musicbrainz.org", 200, &mb_reply("Record", "Band", 100));
    let demo = ArtRequest { unreleased: true, ..req(false) };
    assert_eq!(c.r.resolve(&demo).await, ArtOutcome::None);
    assert!(c.http.urls().is_empty(), "nothing sent: {:?}", c.http.urls());
    assert_eq!(c.mbids.1.load(Ordering::SeqCst), 0);
    *c.up.answer.lock() = Some(catbox_ok());
    let demo = ArtRequest { anon: true, hosts: CoverHost::Catbox, ..demo };
    assert_eq!(c.r.resolve(&demo).await, ArtOutcome::Url("https://files.catbox.moe/ab12cd.jpg".into()));
    assert_eq!(c.http.urls(), vec!["https://files.catbox.moe/ab12cd.jpg".to_string()], "only the fresh upload is fetched");
}

#[tokio::test]
async fn order_is_release_id_then_search_then_upload_then_icon() {
    // The release id wins: no search.
    let c = resolver(Some(MBID));
    c.http.route("coverartarchive.org/release/", 200, r#"{"images":[{"front":true}]}"#);
    assert_eq!(c.r.resolve(&req(true)).await, ArtOutcome::Url(COVER.into()));
    assert_eq!(c.http.calls("musicbrainz.org"), 0);

    // No cover by id: the search is next.
    let c = resolver(Some(MBID));
    c.http.route("coverartarchive.org/release/", 404, "");
    c.http.route("musicbrainz.org/ws/2/release-group", 200, &mb_reply("Record", "Band", 100));
    c.http.route("coverartarchive.org/release-group/", 200, r#"{"images":[{"front":true}]}"#);
    let urls = |c: &Covers3| c.http.urls();
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::Url(GROUP_COVER.into()));
    let u = urls(&c);
    assert!(u[0].contains("/release/") && u[1].contains("musicbrainz.org") && u[2].contains("/release-group/"), "{u:?}");

    // Nothing public: the upload host; Imgur over catbox when both can.
    let c = resolver(None);
    c.http.route("musicbrainz.org", 200, r#"{"release-groups":[]}"#);
    c.keys.set(IMGUR_ACCOUNT, &crate::plex::token::Token::new("k").unwrap()).unwrap();
    *c.up.answer.lock() = Some(imgur_ok());
    let both = ArtRequest { anon: true, hosts: CoverHost::Catbox, ..req(true) };
    assert_eq!(c.r.resolve(&both).await, ArtOutcome::Url("https://i.imgur.com/AbC123.jpg".into()));
    assert_eq!(c.up.log.lock()[0].0, IMGUR_UPLOAD);

    // Neither upload on: the icon.
    let c = resolver(None);
    c.http.route("musicbrainz.org", 200, r#"{"release-groups":[]}"#);
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::None);
    assert!(c.up.log.lock().is_empty());
}

#[tokio::test]
async fn catbox_uploads_once_without_a_key() {
    let c = resolver(None);
    c.http.route("musicbrainz.org", 200, r#"{"release-groups":[]}"#);
    *c.up.answer.lock() = Some(catbox_ok());
    let r = ArtRequest { anon: true, hosts: CoverHost::Catbox, ..req(false) };
    assert_eq!(c.r.resolve(&r).await, ArtOutcome::Url("https://files.catbox.moe/ab12cd.jpg".into()));
    assert_eq!(c.r.resolve(&r).await, ArtOutcome::Url("https://files.catbox.moe/ab12cd.jpg".into()));
    assert_eq!(c.up.log.lock().len(), 1, "one upload per album");
    let (url, headers, ct, body) = c.up.log.lock()[0].clone();
    assert_eq!(url, CATBOX_UPLOAD);
    assert!(headers.is_empty());
    assert!(ct.starts_with("multipart/form-data; boundary="));
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("name=\"reqtype\"\r\n\r\nfileupload\r\n") && text.contains("name=\"fileToUpload\""));
    assert!(text.contains("JPEGDATA") && !text.contains("Record") && !text.contains("/library/"));
    // With Imgur's client id refused, catbox still works.
    let c = resolver(None);
    c.http.route("api.imgur.com/3/credits", 403, "{}");
    c.r.set_key(Some("bad".into())).await.unwrap();
    *c.up.answer.lock() = Some(catbox_ok());
    let both = ArtRequest { anon: true, hosts: CoverHost::Catbox, ..req(true) };
    assert_eq!(c.r.resolve(&both).await, ArtOutcome::Url("https://files.catbox.moe/ab12cd.jpg".into()));
}

#[tokio::test]
async fn a_catbox_answer_that_isnt_its_file_link_is_not_used() {
    for body in ["https://evil.example/a.jpg", "Something went wrong", "https://catbox.moe/a.jpg"] {
        let c = resolver(None);
        *c.up.answer.lock() = Some(HttpResponse { status: 200, body: body.to_owned().into(), ..Default::default() });
        let r = ArtRequest { anon: true, hosts: CoverHost::Catbox, use_archive: false, ..req(false) };
        assert!(matches!(c.r.resolve(&r).await, ArtOutcome::UploadFailed(_)), "{body}");
    }
    let c = resolver(None);
    *c.up.answer.lock() = Some(HttpResponse { status: 412, ..Default::default() });
    let r = ArtRequest { anon: true, hosts: CoverHost::Catbox, use_archive: false, ..req(false) };
    assert!(matches!(c.r.resolve(&r).await, ArtOutcome::UploadFailed(_)));
    assert!(matches!(c.r.resolve(&r).await, ArtOutcome::UploadFailed(_)));
    assert_eq!(c.up.log.lock().len(), 1, "not retried this session");
}

#[tokio::test]
async fn cover_art_archive_is_used_by_release_id_and_cached() {
    let c = resolver(Some(MBID));
    c.http.route("coverartarchive.org/release/", 200, r#"{"images":[{"front":true,"image":"x"}]}"#);
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::Url(COVER.into()));
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::Url(COVER.into()));
    assert_eq!(c.http.calls("coverartarchive.org"), 1, "cached after the first");
    assert_eq!(c.http.urls()[0], format!("{CAA}/{MBID}"), "only the release id is sent");
}

#[tokio::test]
async fn albums_the_archive_lacks_are_uploaded_once_to_imgur() {
    let c = resolver(Some(MBID));
    c.http.route("coverartarchive.org/release/", 404, "Not Found");
    c.keys.set(IMGUR_ACCOUNT, &crate::plex::token::Token::new("client123").unwrap()).unwrap();
    *c.up.answer.lock() = Some(imgur_ok());
    assert_eq!(c.r.resolve(&req(true)).await, ArtOutcome::Url("https://i.imgur.com/AbC123.jpg".into()));
    assert_eq!(c.r.resolve(&req(true)).await, ArtOutcome::Url("https://i.imgur.com/AbC123.jpg".into()));
    assert_eq!(c.up.log.lock().len(), 1, "one upload per album");
    assert_eq!(c.http.calls("coverartarchive.org"), 1, "the miss is remembered");
    let (url, headers, ct, body) = c.up.log.lock()[0].clone();
    assert_eq!(url, IMGUR_UPLOAD);
    assert_eq!(headers, vec![("Authorization", "Client-ID client123".to_string())]);
    assert!(ct.starts_with("multipart/form-data; boundary="));
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("JPEGDATA") && text.contains("name=\"image\""));
    assert!(!text.contains("/library/") && !text.contains("Record"), "only the image is sent");
}

#[tokio::test]
async fn without_a_key_or_mbid_there_is_no_public_cover() {
    let c = resolver(None);
    assert_eq!(c.r.resolve(&req(true)).await, ArtOutcome::None);
    assert_eq!(c.http.calls("coverartarchive.org"), 0);
    assert!(c.up.log.lock().is_empty());
    assert_eq!(c.mbids.1.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn image_upload_failed_is_reported_and_not_retried_until_a_new_key() {
    let c = resolver(None);
    c.keys.set(IMGUR_ACCOUNT, &crate::plex::token::Token::new("client123").unwrap()).unwrap();
    *c.up.answer.lock() = Some(HttpResponse { status: 500, ..Default::default() });
    assert!(matches!(c.r.resolve(&req(true)).await, ArtOutcome::UploadFailed(_)));
    assert!(matches!(c.r.resolve(&req(true)).await, ArtOutcome::UploadFailed(_)));
    assert_eq!(c.up.log.lock().len(), 1);
    // Offline also fails cleanly.
    *c.up.answer.lock() = None;
    c.http.route("api.imgur.com/3/credits", 200, "{}");
    c.r.set_key(Some("client456".into())).await.unwrap();
    assert!(matches!(c.r.resolve(&req(true)).await, ArtOutcome::UploadFailed(_)));
    *c.up.answer.lock() = Some(imgur_ok());
    c.r.set_key(Some("client456".into())).await.unwrap();
    assert!(matches!(c.r.resolve(&req(true)).await, ArtOutcome::Url(_)));
}

#[tokio::test]
async fn a_refused_client_id_is_flagged_on_save_and_on_upload() {
    let c = resolver(None);
    c.http.route("api.imgur.com/3/credits", 403, "{}");
    let st = c.r.set_key(Some(" bad ".into())).await.unwrap();
    assert!(st.saved && st.rejected);
    assert_eq!(c.keys.get(IMGUR_ACCOUNT).unwrap().unwrap().expose(), "bad");
    assert!(c.up.log.lock().is_empty(), "checking a key uploads nothing");
    c.http.route("api.imgur.com/3/credits", 200, "{}");
    c.r.set_key(Some("good".into())).await.unwrap();
    *c.up.answer.lock() = Some(HttpResponse { status: 403, ..Default::default() });
    assert!(matches!(c.r.resolve(&req(true)).await, ArtOutcome::UploadFailed(_)));
    assert!(c.r.key_state().rejected);
    let st = c.r.set_key(None).await.unwrap();
    assert!(!st.saved && !st.rejected);
}

#[tokio::test]
async fn an_upload_answer_off_the_public_hosts_is_not_used() {
    let c = resolver(None);
    c.keys.set(IMGUR_ACCOUNT, &crate::plex::token::Token::new("k").unwrap()).unwrap();
    *c.up.answer.lock() = Some(HttpResponse { status: 200, body: r#"{"data":{"link":"https://evil.example/a.jpg"}}"#.into(), ..Default::default() });
    assert!(matches!(c.r.resolve(&req(true)).await, ArtOutcome::UploadFailed(_)));
}

#[tokio::test]
async fn turning_the_archive_off_skips_its_cached_cover() {
    let c = resolver(Some(MBID));
    c.http.route("coverartarchive.org/release/", 200, r#"{"images":[{"front":true}]}"#);
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::Url(COVER.into()));
    let off = ArtRequest { use_archive: false, ..req(false) };
    assert_eq!(c.r.resolve(&off).await, ArtOutcome::None);
}

#[tokio::test]
async fn a_fresh_upload_is_fetched_once_before_use_and_a_cached_one_is_not() {
    let c = resolver(None);
    c.http.route("musicbrainz.org", 200, r#"{"release-groups":[]}"#);
    *c.up.answer.lock() = Some(catbox_ok());
    let r = ArtRequest { anon: true, hosts: CoverHost::Catbox, ..req(false) };
    assert_eq!(c.r.resolve(&r).await, ArtOutcome::Url("https://files.catbox.moe/ab12cd.jpg".into()));
    assert_eq!(c.http.urls().last().unwrap(), "https://files.catbox.moe/ab12cd.jpg");
    assert_eq!(c.r.resolve(&r).await, ArtOutcome::Url("https://files.catbox.moe/ab12cd.jpg".into()));
    assert_eq!(c.http.calls("files.catbox.moe"), 1, "the cached cover isn't fetched again");

    // The same for Imgur, without the client id on the fetch.
    let c = resolver(None);
    c.http.route("musicbrainz.org", 200, r#"{"release-groups":[]}"#);
    c.keys.set(IMGUR_ACCOUNT, &crate::plex::token::Token::new("client123").unwrap()).unwrap();
    *c.up.answer.lock() = Some(imgur_ok());
    c.r.resolve(&req(true)).await;
    c.r.resolve(&req(true)).await;
    assert_eq!(c.http.calls("i.imgur.com/AbC123.jpg"), 1);
    let log = c.http.log.lock().clone();
    let (_, headers) = log.iter().find(|(u, _)| u.contains("i.imgur.com")).unwrap();
    assert!(headers.is_empty(), "nothing secret on the fetch");

    // A failed fetch changes nothing.
    let c = resolver(None);
    c.http.fail("files.catbox.moe");
    *c.up.answer.lock() = Some(catbox_ok());
    let r = ArtRequest { anon: true, hosts: CoverHost::Catbox, use_archive: false, ..req(false) };
    assert_eq!(c.r.resolve(&r).await, ArtOutcome::Url("https://files.catbox.moe/ab12cd.jpg".into()));

    // Cover Art Archive covers aren't uploads: never fetched by us.
    let c = resolver(Some(MBID));
    c.http.route("coverartarchive.org/release/", 200, r#"{"images":[{"front":true}]}"#);
    assert_eq!(c.r.resolve(&req(false)).await, ArtOutcome::Url(COVER.into()));
    assert!(!c.http.urls().iter().any(|u| u.ends_with("front-500")));
}

#[test]
fn multipart_body_has_the_type_and_the_image() {
    let (body, ct) = multipart(b"IMG", "image/png");
    let b = ct.split("boundary=").nth(1).unwrap();
    let text = String::from_utf8(body).unwrap();
    assert!(text.starts_with(&format!("--{b}\r\n")));
    assert!(text.contains("filename=\"cover.png\"\r\nContent-Type: image/png\r\n\r\nIMG\r\n"));
    assert!(text.ends_with(&format!("--{b}--\r\n")));
}

#[test]
fn an_empty_build_time_id_means_unavailable() {
    // The test build has no id unless one is set in the environment.
    if let Some(id) = app_id() {
        assert!(id.bytes().all(|b| b.is_ascii_digit()));
    }
}

// ---------------------------------------------------------------- upload hosts

use super::hosts::{HostAdapter, LITTERBOX_UPLOAD, UGUU_UPLOAD, X0_UPLOAD, direct_image};
use crate::db::queries::now_s;

const LITTER: &str = "https://litter.catbox.moe/ab12cd.jpg";
const X0_LINK: &str = "https://x0.at/Qr7.jpg";
const UGUU_LINK: &str = "https://n.uguu.se/xYzAbC.jpg";
const CATBOX_LINK: &str = "https://files.catbox.moe/ab12cd.jpg";

fn text(body: &str) -> Option<HttpResponse> {
    Some(HttpResponse { status: 200, body: body.to_owned().into(), ..Default::default() })
}

fn uguu_ok() -> Option<HttpResponse> {
    text(&format!(r#"{{"success":true,"files":[{{"hash":"h","filename":"xYzAbC.jpg","url":"{UGUU_LINK}","size":9}}]}}"#))
}

fn server_error() -> Option<HttpResponse> {
    Some(HttpResponse { status: 502, ..Default::default() })
}

/// Keyless uploads on, automatic hosts, no Cover Art Archive.
fn keyless(hosts: CoverHost) -> ArtRequest {
    ArtRequest { anon: true, hosts, use_archive: false, unreleased: true, ..req(false) }
}

fn all_hosts_ok(c: &Covers3) {
    c.up.route(LITTERBOX_UPLOAD, text(LITTER));
    c.up.route(X0_UPLOAD, text(X0_LINK));
    c.up.route(UGUU_UPLOAD, uguu_ok());
    c.up.route(CATBOX_UPLOAD, text(CATBOX_LINK));
}

#[tokio::test]
async fn keyless_hosts_go_litterbox_then_x0_then_uguu_then_catbox() {
    let c = resolver(None);
    all_hosts_ok(&c);
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url(LITTER.into()));
    assert_eq!(c.up.urls(), vec![LITTERBOX_UPLOAD.to_string()]);
    let body = String::from_utf8_lossy(&c.up.log.lock()[0].3).into_owned();
    assert!(body.contains("name=\"reqtype\"\r\n\r\nfileupload\r\n") && body.contains("name=\"time\"\r\n\r\n72h\r\n"));
    assert!(body.contains("name=\"fileToUpload\"") && body.contains("JPEGDATA") && !body.contains("Record"));

    // Each host down in turn: the next one.
    let c = resolver(None);
    all_hosts_ok(&c);
    c.up.route(LITTERBOX_UPLOAD, server_error());
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url(X0_LINK.into()));
    assert!(String::from_utf8_lossy(&c.up.log.lock()[1].3).contains("name=\"file\""));

    let c = resolver(None);
    all_hosts_ok(&c);
    c.up.route(LITTERBOX_UPLOAD, None);
    c.up.route(X0_UPLOAD, server_error());
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url(UGUU_LINK.into()));
    assert!(String::from_utf8_lossy(&c.up.log.lock()[2].3).contains("name=\"files[]\""));

    let c = resolver(None);
    all_hosts_ok(&c);
    c.up.route(LITTERBOX_UPLOAD, None);
    c.up.route(X0_UPLOAD, None);
    c.up.route(UGUU_UPLOAD, server_error());
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url(CATBOX_LINK.into()));
    assert_eq!(c.up.urls(), vec![LITTERBOX_UPLOAD, X0_UPLOAD, UGUU_UPLOAD, CATBOX_UPLOAD]);

    // All down: a failure, and not tried again this session.
    let c = resolver(None);
    assert!(matches!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::UploadFailed(_)));
    assert_eq!(c.up.urls().len(), 4);
    assert!(matches!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::UploadFailed(_)));
    assert_eq!(c.up.urls().len(), 4);

    // One host picked: only that one.
    let c = resolver(None);
    all_hosts_ok(&c);
    assert_eq!(c.r.resolve(&keyless(CoverHost::Uguu)).await, ArtOutcome::Url(UGUU_LINK.into()));
    assert_eq!(c.up.urls(), vec![UGUU_UPLOAD]);
}

#[test]
fn each_host_only_takes_a_direct_image_link_on_itself() {
    let ok = |b: &str| HttpResponse { status: 200, body: b.to_owned().into(), ..Default::default() };
    assert_eq!(super::hosts::Litterbox.link(&ok(" https://litter.catbox.moe/a1.jpg\n")).unwrap(), "https://litter.catbox.moe/a1.jpg");
    assert_eq!(super::hosts::X0.link(&ok("http://x0.at/Ab.png")).unwrap(), "https://x0.at/Ab.png");
    for (host, bad) in [
        (Host::Litterbox, "https://files.catbox.moe/a.jpg"),
        (Host::Litterbox, "https://litter.catbox.moe/"),
        (Host::Litterbox, "https://litter.catbox.moe/a.html"),
        (Host::X0, "https://x0.at/a"),
        (Host::X0, "https://x0.at/dir/a.jpg"),
        (Host::X0, "https://x0.at/a.jpg?x=1"),
        (Host::Uguu, "https://uguu.se.evil.example/a.jpg"),
        (Host::Catbox, "https://catbox.moe/a.jpg"),
        (Host::Catbox, "Something went wrong"),
    ] {
        assert!(!direct_image(bad, host), "{bad}");
    }
    assert!(super::hosts::X0.link(&ok("File too large")).is_err());
    assert!(super::hosts::Litterbox.link(&HttpResponse { status: 412, ..Default::default() }).is_err());
    let uguu = |b: &str| super::hosts::Uguu.link(&ok(b));
    assert_eq!(uguu(std::str::from_utf8(&uguu_ok().unwrap().body).unwrap()).unwrap(), UGUU_LINK);
    assert!(uguu(r#"{"success":false,"files":[{"url":"https://n.uguu.se/a.jpg"}]}"#).is_err());
    assert!(uguu(r#"{"success":true,"files":[{"url":"https://evil.example/a.jpg"}]}"#).is_err());
    assert!(uguu("<html>busy</html>").is_err());
    // How long each keeps a file.
    assert_eq!(super::hosts::Litterbox.keeps(50_000), Some(72 * 3600));
    assert_eq!(super::hosts::Uguu.keeps(50_000), Some(3 * 3600));
    assert_eq!(super::hosts::Catbox.keeps(50_000), None);
    let small = super::hosts::X0.keeps(60_000).unwrap();
    assert!(small > 99 * 86_400 && small <= 100 * 86_400, "{small}");
    assert_eq!(super::hosts::X0.keeps(usize::MAX), Some(3 * 86_400));
}

#[tokio::test]
async fn a_host_that_answers_with_a_page_is_skipped() {
    let c = resolver(None);
    all_hosts_ok(&c);
    c.http.route_resp(
        "litter.catbox.moe",
        HttpResponse { status: 200, content_type: Some("text/html".into()), body: "<html>".into(), ..Default::default() },
    );
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url(X0_LINK.into()));
}

#[tokio::test]
async fn imgur_stays_first_when_its_client_id_is_set() {
    let c = resolver(None);
    all_hosts_ok(&c);
    c.keys.set(IMGUR_ACCOUNT, &crate::plex::token::Token::new("client123").unwrap()).unwrap();
    c.up.route(IMGUR_UPLOAD, Some(imgur_ok()));
    let both = ArtRequest { upload: true, ..keyless(CoverHost::Auto) };
    assert_eq!(c.r.resolve(&both).await, ArtOutcome::Url("https://i.imgur.com/AbC123.jpg".into()));
    assert_eq!(c.up.urls(), vec![IMGUR_UPLOAD]);

    // Imgur failing hands over to the keyless hosts.
    let c = resolver(None);
    all_hosts_ok(&c);
    c.keys.set(IMGUR_ACCOUNT, &crate::plex::token::Token::new("client123").unwrap()).unwrap();
    c.up.route(IMGUR_UPLOAD, server_error());
    assert_eq!(c.r.resolve(&both).await, ArtOutcome::Url(LITTER.into()));
    assert_eq!(c.up.urls(), vec![IMGUR_UPLOAD, LITTERBOX_UPLOAD]);
}

fn put_link(c: &Covers3, url: &'static str, host: Host, fetched: i64, ends: Option<i64>) {
    c.db.write(|db| put_upload(db, "al1", url, host, fetched, ends)).unwrap();
}

#[tokio::test]
async fn short_lived_links_are_uploaded_again_before_they_go() {
    let now = now_s();
    let h = 3600;
    // Litterbox: plenty left, used as is.
    let c = resolver(None);
    all_hosts_ok(&c);
    put_link(&c, "https://litter.catbox.moe/old.jpg", Host::Litterbox, now - 10 * h, Some(now + 62 * h));
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url("https://litter.catbox.moe/old.jpg".into()));
    assert!(c.up.urls().is_empty());
    // Under 12 hours left: uploaded again, and the new link kept with its end.
    put_link(&c, "https://litter.catbox.moe/old.jpg", Host::Litterbox, now - 61 * h, Some(now + 11 * h));
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url(LITTER.into()));
    assert_eq!(c.up.urls().len(), 1);
    let row = c.db.read(|db| get(db, "al1")).unwrap().unwrap();
    assert_eq!((row.source.as_str(), row.host.as_deref()), ("litterbox", Some("litterbox")));
    let ends = row.link_expires_at.unwrap() - row.fetched_at;
    assert_eq!(ends, 72 * h);
    // Gone already: uploaded again.
    put_link(&c, "https://litter.catbox.moe/old.jpg", Host::Litterbox, now - 80 * h, Some(now - 8 * h));
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url(LITTER.into()));
    assert_eq!(c.up.urls().len(), 2);
    // uguu.se keeps 3 hours: used for the first half of that.
    put_link(&c, "https://n.uguu.se/old.jpg", Host::Uguu, now - h, Some(now + 2 * h));
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url("https://n.uguu.se/old.jpg".into()));
    put_link(&c, "https://n.uguu.se/old.jpg", Host::Uguu, now - 2 * h, Some(now + h));
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url(LITTER.into()));
    assert_eq!(c.up.urls().len(), 3);
}

#[tokio::test]
async fn earlier_catbox_links_are_kept() {
    let c = resolver(None);
    all_hosts_ok(&c);
    // As an earlier build saved it (and the migration filled in the host).
    c.db.write(|db| {
        db.execute(
            "INSERT INTO discord_art (album_id, url, source, fetched_at, expires_at, host) VALUES ('al1', ?1, 'catbox', 1, ?2, 'catbox')",
            rusqlite::params![CATBOX_LINK, i64::MAX],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(c.r.resolve(&keyless(CoverHost::Auto)).await, ArtOutcome::Url(CATBOX_LINK.into()));
    assert!(c.up.urls().is_empty(), "no new upload");
    assert!(c.http.urls().is_empty(), "not fetched again");
    // With one other host picked, that host's link is made instead.
    assert_eq!(c.r.resolve(&keyless(CoverHost::X0)).await, ArtOutcome::Url(X0_LINK.into()));
    // Uploads off: no uploaded link is used.
    let off = ArtRequest { anon: false, ..keyless(CoverHost::Auto) };
    assert_eq!(c.r.resolve(&off).await, ArtOutcome::None);
}

#[tokio::test]
async fn replacing_a_cover_moves_down_the_chain() {
    let c = resolver(None);
    all_hosts_ok(&c);
    let r = keyless(CoverHost::Auto);
    assert_eq!(c.r.resolve(&r).await, ArtOutcome::Url(LITTER.into()));
    assert_eq!(c.r.replace(&r, &[LITTER.into()]).await, ArtOutcome::Url(X0_LINK.into()));
    assert_eq!(c.db.read(|db| get(db, "al1")).unwrap().unwrap().url.as_deref(), Some(X0_LINK), "the new link is kept");
    assert_eq!(c.r.resolve(&r).await, ArtOutcome::Url(X0_LINK.into()));
    assert_eq!(c.r.replace(&r, &[LITTER.into(), X0_LINK.into()]).await, ArtOutcome::Url(UGUU_LINK.into()));
    assert_eq!(
        c.r.replace(&r, &[LITTER.into(), X0_LINK.into(), UGUU_LINK.into()]).await,
        ArtOutcome::Url(CATBOX_LINK.into())
    );
    let n = c.up.urls().len();
    let all = [LITTER.into(), X0_LINK.into(), UGUU_LINK.into(), CATBOX_LINK.into()];
    assert_eq!(c.r.replace(&r, &all).await, ArtOutcome::None, "nothing left");
    // A kept catbox link that fails starts the chain from the top.
    assert_eq!(c.r.replace(&r, &[CATBOX_LINK.into()]).await, ArtOutcome::Url(LITTER.into()));
    // Not an upload: nothing to replace.
    assert_eq!(c.r.replace(&r, &[COVER.into()]).await, ArtOutcome::None);
    assert_eq!(c.up.urls().len(), n + 1);
}

#[test]
fn covers_are_sent_as_small_jpegs() {
    // A big PNG becomes a 512 px JPEG.
    let img = image::RgbImage::from_pixel(1200, 900, image::Rgb([200, 40, 40]));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let (out, ty) = prepare_cover(png, "image/png".into());
    assert_eq!(ty, "image/jpeg");
    let back = image::load_from_memory(&out).unwrap();
    assert_eq!((back.width(), back.height()), (512, 384));
    assert_eq!(image::guess_format(&out).unwrap(), image::ImageFormat::Jpeg);
    // A small JPEG goes as it is.
    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::new(300, 300))
        .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
        .unwrap();
    assert_eq!(prepare_cover(jpeg.clone(), "image/jpeg".into()), (jpeg, "image/jpeg".to_string()));
    // Something unreadable goes as it came.
    assert_eq!(prepare_cover(b"nope".to_vec(), "image/jpeg".into()), (b"nope".to_vec(), "image/jpeg".to_string()));
}

// ---------------------------------------------------------------- proxy failure and the send limit

#[tokio::test(start_paused = true)]
async fn a_cover_the_proxy_refuses_is_uploaded_to_the_next_host_and_sent_once_more() {
    let art = FakeArt::new(ArtOutcome::Url(LITTER.into()));
    *art.replacement.lock() = ArtOutcome::Url(X0_LINK.into());
    let r = rig_with(art, FakeLib::default(), DiscordSettings::default());
    r.proxy.script.lock().push_back((502, "text/plain"));
    r.events.send(started(song("1"))).unwrap();
    wait(3.0).await;
    let acts = r.discord.activities();
    assert_eq!(acts.len(), 2, "sent again at once, not after the gap: {acts:?}");
    assert_eq!(acts[0].as_ref().unwrap()["assets"]["large_image"], LITTER);
    assert_eq!(acts[1].as_ref().unwrap()["assets"]["large_image"], X0_LINK);
    assert_eq!(*r.art.replaced.lock(), vec![vec![LITTER.to_string()]]);
    assert!(r.proxy.log.lock()[1].0.contains("x0.at"), "the new link is checked");
    // The first link's own recheck still runs, but changes nothing now.
    wait(70.0).await;
    assert_eq!(r.discord.activities().len(), 2, "exactly one resend");
    assert_eq!(r.art.replaced.lock().len(), 1);
    assert_eq!(r.proxy.calls(), 3);
}

#[tokio::test(start_paused = true)]
async fn when_no_host_is_left_the_song_shows_the_app_icon() {
    let art = FakeArt::new(ArtOutcome::Url(LITTER.into()));
    *art.replacement.lock() = ArtOutcome::Url(X0_LINK.into());
    let r = rig_with(art, FakeLib::default(), DiscordSettings::default());
    *r.proxy.then.lock() = (502, "text/plain");
    r.events.send(started(song("1"))).unwrap();
    wait(3.0).await;
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], X0_LINK);
    // The second host fails too, and nothing is left.
    *r.art.replacement.lock() = ArtOutcome::None;
    wait(70.0).await;
    assert_eq!(*r.art.replaced.lock(), vec![vec![LITTER.to_string()], vec![LITTER.to_string(), X0_LINK.to_string()]]);
    assert_eq!(last(&r.discord).unwrap()["assets"]["large_image"], APP_ICON);
    assert_eq!(last(&r.discord).unwrap()["details"], "Song 1");
}

#[tokio::test(start_paused = true)]
async fn with_no_other_host_a_link_that_works_later_is_sent_again() {
    let art = FakeArt::new(ArtOutcome::Url(LITTER.into()));
    *art.replacement.lock() = ArtOutcome::None;
    let r = rig_with(art, FakeLib::default(), DiscordSettings::default());
    r.proxy.script.lock().push_back((502, "text/plain"));
    r.events.send(started(song("1"))).unwrap();
    wait(30.0).await;
    let acts = r.discord.activities();
    assert_eq!(acts.len(), 2, "{acts:?}");
    assert_eq!(acts[1], acts[0], "the same status again");
}

#[tokio::test(start_paused = true)]
async fn no_more_than_five_updates_in_20s_and_the_last_state_still_goes() {
    let r = rig_cfg(FakeArt::new(ArtOutcome::Url(COVER.into())), FakeLib::default(), DiscordSettings::default(), |c| {
        c.gap = Duration::from_millis(100);
    });
    for i in 1..=12 {
        r.events.send(started(song(&i.to_string()))).unwrap();
        wait(1.0).await;
    }
    assert_eq!(r.discord.activities().len(), 5);
    wait(7.0).await; // 19 s in
    assert_eq!(r.discord.activities().len(), 5, "held back by the limit");
    wait(3.0).await;
    let acts = r.discord.activities();
    assert_eq!(acts.len(), 6, "{acts:?}");
    assert_eq!(last(&r.discord).unwrap()["details"], "Song 12", "newest wins");
    wait(60.0).await;
    assert_eq!(r.discord.activities().len(), 6);
}

#[tokio::test]
async fn an_album_plex_missed_gets_the_cover_of_the_release_the_check_found() {
    use crate::artistinfo::musicbrainz::FoundRelease;
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("d.db")).unwrap();
    db.write(|c| {
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, matched)
             VALUES ('al1', 'Record (Deluxe Edition)', 'Record', 'a1', 'Band', 0)",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    // Unmatched and unchecked: probably unreleased, so only ever uploaded.
    assert_eq!(db.read(|c| super::commands::album_unreleased(c, "al1")).unwrap(), Some(true));
    db.write(|c| {
        let found = FoundRelease { release_group_id: GROUP.into(), release_id: MBID.into() };
        crate::artistinfo::releasecheck::put(c, "al1", Some(&found), crate::db::queries::now_s())
    })
    .unwrap();
    // Found on MusicBrainz: the one rule says released.
    assert_eq!(db.read(|c| super::commands::album_unreleased(c, "al1")).unwrap(), Some(false));

    let http = Arc::new(FakeHttp::new());
    let mbids = Arc::new(Mbids(None, AtomicUsize::new(0)));
    let r = ArtResolver::new(
        db.clone(),
        http.clone(),
        Arc::new(FakeUpload::default()),
        mbids.clone(),
        Arc::new(Covers),
        Arc::new(MemoryStore::new()),
    )
    .with_musicbrainz_limiter(Arc::new(crate::artistinfo::limit::Limiter::per_second(1000.0)));
    http.route("coverartarchive.org/release/", 200, r#"{"images":[{"front":true}]}"#);
    assert_eq!(r.resolve(&req(false)).await, ArtOutcome::Url(COVER.into()), "the stored release's cover");
    assert_eq!(http.calls("musicbrainz.org"), 0, "no new search");
    assert_eq!(mbids.1.load(Ordering::SeqCst), 0, "Plex isn't asked for an id it doesn't have");

    // No cover for that release: the stored release group's.
    db.write(|c| {
        c.execute("DELETE FROM discord_art", [])?;
        Ok(())
    })
    .unwrap();
    http.route("coverartarchive.org/release/", 404, "Not Found");
    http.route("coverartarchive.org/release-group/", 200, r#"{"images":[{"front":true}]}"#);
    assert_eq!(r.resolve(&req(false)).await, ArtOutcome::Url(GROUP_COVER.into()));
    assert_eq!(http.calls("musicbrainz.org"), 0);
}

#[test]
fn the_pushed_status_leaves_the_imgur_key_unread_until_something_needs_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("d.db")).unwrap();
    let keys = Arc::new(crate::plex::keychain::CountingStore::default());
    keys.set(super::art::IMGUR_ACCOUNT, &crate::plex::token::Token::new("cid").unwrap()).unwrap();
    let r = ArtResolver::new(
        db,
        Arc::new(FakeHttp::new()),
        Arc::new(FakeUpload::default()),
        Arc::new(Mbids(None, AtomicUsize::new(0))),
        Arc::new(Covers),
        keys.clone(),
    );
    assert_eq!(r.known_key_state(), None);
    assert_eq!(keys.total_reads(), 0);
    // The Settings page asks: one read, then it's known.
    assert!(r.key_state().saved);
    assert_eq!(r.known_key_state().map(|k| k.saved), Some(true));
    assert!(r.key_state().saved);
    assert_eq!(keys.reads_of(super::art::IMGUR_ACCOUNT), 1);
}
