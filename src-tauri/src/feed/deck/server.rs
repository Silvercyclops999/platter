//! The control API's listener and HTTP handling: one request per
//! connection, small JSON bodies, loopback only.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::ask::{AskError, AskState, Asks};
use super::pairing::{PairError, Pairing, hex};
use super::{
    API_VERSION, BURST, COVER_SIZE, COVER_SIZE_MAX, Command, ContextRef, DeckConfig, DeckHost, DeckLink, DeckState, DeckStatus,
    LibraryKind, Listening, NextUp, PAIR_BURST, PAIR_RATE, QUEUE_MAX, RATE, READ_TIMEOUT, RateLimit, SleepView, UiReport,
};
use crate::audio::queue::Repeat;
use crate::audio::source::{ContextKind, PlayContext};
use crate::error::AppError;
use crate::feed::artist_line;
use crate::feed::server::host_allowed;
use crate::model::Track;
use crate::plex::state::EventSink;

const MAX_HEAD: usize = 8 * 1024;
const MAX_BODY: usize = 4 * 1024;

/// One parsed request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn bearer(&self) -> Option<&str> {
        let v = self.header("authorization")?;
        let (scheme, token) = v.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
    }
}

/// One JSON answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: serde_json::Value,
}

impl Reply {
    fn ok(body: serde_json::Value) -> Self {
        Self { status: 200, body }
    }
    fn error(status: u16, message: &str) -> Self {
        Self { status, body: serde_json::json!({ "error": message }) }
    }
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    }
}

struct Inner {
    host: Arc<dyn DeckHost>,
    pairing: Arc<Pairing>,
    automation: Arc<Pairing>,
    events: Arc<dyn EventSink>,
    listening: Mutex<Listening>,
    config: Mutex<DeckConfig>,
    limit: Mutex<RateLimit>,
    pair_limit: Mutex<RateLimit>,
    asks: Mutex<Asks>,
    /// Art paths behind the cover keys handed out, for `/v1/cover`.
    covers: Mutex<std::collections::HashMap<String, String>>,
    ui: Mutex<UiReport>,
    sleep: Mutex<Option<Sleep>>,
}

/// Most cover keys remembered for `/v1/cover` (forgotten all at once past it).
const COVERS_MAX: usize = 4096;
/// How often an end-of-song sleep timer looks at the player, and how often
/// near the end.
const SLEEP_POLL: Duration = Duration::from_millis(1000);
const SLEEP_POLL_NEAR: Duration = Duration::from_millis(100);
/// The longest sleep timer, in minutes.
const SLEEP_MAX_MINUTES: f64 = 24.0 * 60.0;
/// Songs a search key plays.
const SEARCH_SONGS: usize = 50;

/// A running sleep timer.
#[derive(Debug, Clone)]
struct Sleep {
    /// Tells a stale timer task it was replaced or cancelled.
    id: u64,
    until: Option<tokio::time::Instant>,
    /// The song to finish, for an end-of-song timer.
    song: Option<String>,
}

/// The control API server. Cheap to clone.
#[derive(Clone)]
pub struct DeckServer {
    inner: Arc<Inner>,
}

/// A short name for a cover, so a client can skip one it has.
pub fn cover_key(thumb: &str) -> String {
    hex(&Sha256::digest(thumb.as_bytes())[..8])
}

impl DeckServer {
    pub fn new(host: Arc<dyn DeckHost>, pairing: Arc<Pairing>, events: Arc<dyn EventSink>) -> Self {
        Self {
            inner: Arc::new(Inner {
                host,
                automation: Arc::new(pairing.sibling(super::pairing::AUTOMATION_ACCOUNT)),
                pairing,
                events,
                listening: Mutex::new(Listening::Off),
                config: Mutex::new(DeckConfig::default()),
                limit: Mutex::new(RateLimit::new(RATE, BURST)),
                pair_limit: Mutex::new(RateLimit::new(PAIR_RATE, PAIR_BURST)),
                asks: Mutex::new(Asks::default()),
                covers: Mutex::new(Default::default()),
                ui: Mutex::new(UiReport::default()),
                sleep: Mutex::new(None),
            }),
        }
    }

    pub fn status(&self) -> DeckStatus {
        let listening = self.inner.listening.lock().clone();
        let on = matches!(listening, Listening::On { .. });
        let c = *self.inner.config.lock();
        DeckStatus {
            code: (on && self.deck_on(&c)).then(|| self.inner.pairing.code()),
            paired: self.inner.pairing.paired(),
            automation_code: (on && c.automation).then(|| self.inner.automation.code()),
            automation_paired: self.inner.automation.paired(),
            listening,
        }
    }

    /// The Stream Deck side is on (it's the only side when neither is set,
    /// as in a server not yet following settings).
    fn deck_on(&self, c: &DeckConfig) -> bool {
        c.enabled || !c.automation
    }

    /// Forget the paired script or command line and show a new code.
    pub fn revoke_automation(&self) -> crate::error::AppResult<DeckStatus> {
        self.inner.automation.revoke()?;
        self.emit_status();
        Ok(self.status())
    }

    /// Run a `platter://` command link (only while automation is on).
    pub async fn run_link(&self, cmd: Command) {
        if !self.inner.config.lock().automation {
            self.inner.host.notice("Turn on Allow automation in Settings to control Platter from links.");
            return;
        }
        let r = self.command(cmd).await;
        if r.status != 200 {
            let m = r.body["error"].as_str().unwrap_or("That didn't work.").to_owned();
            self.inner.host.notice(&m);
        }
    }

    fn emit_status(&self) {
        self.inner.events.emit("deck://status", serde_json::to_value(self.status()).unwrap_or_default());
    }

    fn set_listening(&self, l: Listening) {
        let changed = {
            let mut cur = self.inner.listening.lock();
            let changed = *cur != l;
            *cur = l;
            changed
        };
        if changed {
            self.emit_status();
        }
    }

    /// The "Allow Stream Deck to control Platter?" prompt showing now, if any.
    pub fn pair_prompt(&self) -> Option<String> {
        self.inner.asks.lock().current(Instant::now())
    }

    /// The person answered the prompt `id` in the window. False when that
    /// prompt had already gone.
    pub fn pair_answer(&self, id: &str, allow: bool) -> crate::error::AppResult<bool> {
        self.pair_answer_at(id, allow, Instant::now())
    }

    pub fn pair_answer_at(&self, id: &str, allow: bool, now: Instant) -> crate::error::AppResult<bool> {
        let pairing = &self.inner.pairing;
        let done = self.inner.asks.lock().answer(id, allow, now, || pairing.approve()).map_err(|e| match e {
            PairError::Keychain(m) => AppError::Internal(format!("couldn't save the Stream Deck pairing: {m}")),
            PairError::WrongCode => AppError::Internal("pairing failed".into()),
        })?;
        self.emit_prompt(None);
        if done && allow {
            self.emit_status();
        }
        Ok(done)
    }

    fn emit_prompt(&self, id: Option<&str>) {
        self.inner.events.emit("deck://pair-request", serde_json::json!({ "id": id }));
    }

    /// Forget the paired Stream Deck and show a new code.
    pub fn revoke(&self) -> crate::error::AppResult<DeckStatus> {
        self.inner.pairing.revoke()?;
        self.emit_status();
        Ok(self.status())
    }

    #[cfg(test)]
    pub fn set_config(&self, c: DeckConfig) {
        *self.inner.config.lock() = c;
    }

    #[cfg(test)]
    pub fn automation_code_for_tests(&self) -> String {
        self.inner.automation.code()
    }

    /// What the window says about itself (`deck_ui_report`).
    pub fn set_ui(&self, r: UiReport) {
        *self.inner.ui.lock() = r;
    }

    /// A cover key for an art path, remembered for `/v1/cover`.
    fn key_for(&self, thumb: &str) -> String {
        let key = cover_key(thumb);
        let mut m = self.inner.covers.lock();
        if m.len() >= COVERS_MAX && !m.contains_key(&key) {
            m.clear();
        }
        m.insert(key.clone(), thumb.to_owned());
        key
    }

    async fn cover_data(&self, thumb: &str, size: u32) -> Option<String> {
        let a = self.inner.host.cover(thumb, size).await.ok()?;
        Some(format!("data:{};base64,{}", a.content_type, base64::engine::general_purpose::STANDARD.encode(&a.bytes)))
    }

    /// `/v1/cover`: a cover named by a key handed out before.
    pub async fn cover(&self, key: &str, size: u32) -> Reply {
        let thumb = self.inner.covers.lock().get(key).cloned();
        let Some(thumb) = thumb else { return Reply::error(404, "No such cover.") };
        match self.cover_data(&thumb, size).await {
            Some(cover) => Reply::ok(serde_json::json!({ "key": key, "cover": cover })),
            None => Reply::error(409, "Platter couldn't get that cover."),
        }
    }

    /// The sleep timer as the state shows it.
    fn sleep_view(&self) -> Option<SleepView> {
        let s = self.inner.sleep.lock().clone()?;
        Some(SleepView {
            remaining_ms: s.until.map(|u| u.saturating_duration_since(tokio::time::Instant::now()).as_millis() as u64),
            end_of_song: s.song.is_some(),
        })
    }

    /// `/v1/state`; `have` names the cover the client already shows, `size`
    /// is the cover's size in pixels.
    pub async fn state(&self, have: Option<&str>, size: u32) -> DeckState {
        let host = &self.inner.host;
        let private = self.inner.config.lock().private_session;
        let ui = *self.inner.ui.lock();
        let mut out = DeckState {
            api: API_VERSION,
            private,
            can_request: host.can_request(),
            lyrics: ui.lyrics,
            sleep: self.sleep_view(),
            ..Default::default()
        };
        let now = host.now().await;
        let extras = host.extras(now.as_ref().and_then(|n| n.track.as_ref())).await;
        out.crossfade = extras.crossfade;
        out.miniplayer = extras.miniplayer;
        out.window = extras.window;
        out.device = extras.device;
        let Some(now) = now else { return out };
        out.playing = now.playing;
        out.volume = (now.volume.clamp(0.0, 1.0) * 100.0).round() as u8;
        out.muted = now.muted;
        out.shuffle = now.shuffle;
        out.smart_shuffle = now.smart_shuffle;
        out.repeat = now.repeat;
        out.position_ms = now.position_ms;
        out.duration_ms = now.duration_ms;
        out.context = now.context.map(|(kind, id)| ContextRef { kind, id });
        let Some(t) = now.track else { return out };
        out.liked = host.liked(&t.id).await;
        out.stars = extras.stars;
        out.can_rate = extras.can_rate;
        out.playlists = extras.playlists;
        if private {
            return out;
        }
        out.title = Some(t.title.clone());
        out.artist = Some(artist_line(&t));
        out.album = Some(t.album_title.clone()).filter(|a| !a.is_empty());
        out.track_id = Some(t.id.clone());
        let up = |n: Track| NextUp { title: n.title.clone(), artist: artist_line(&n), cover_key: n.thumb.as_deref().map(|th| self.key_for(th)) };
        out.next = now.next.map(up);
        out.previous = now.previous.map(up);
        if let Some(thumb) = t.thumb.as_deref() {
            let key = self.key_for(thumb);
            if have != Some(key.as_str()) && have != Some("none") {
                out.cover = self.cover_data(thumb, size).await;
            }
            out.cover_key = Some(key);
        }
        out
    }

    /// `/v1/library`.
    pub async fn library(&self, kind: LibraryKind) -> Reply {
        match self.inner.host.library(kind).await {
            Ok(mut items) => {
                items.truncate(super::LIBRARY_MAX);
                for i in &mut items {
                    i.cover_key = i.thumb.as_deref().map(|t| self.key_for(t));
                }
                Reply::ok(serde_json::json!({ "kind": kind, "items": items }))
            }
            Err(e) => Reply::error(409, &message(&e)),
        }
    }

    /// `/v1/queue`: the songs coming up (left out in a private session).
    pub async fn queue(&self) -> Reply {
        if self.inner.config.lock().private_session {
            return Reply::ok(serde_json::json!({ "items": [] }));
        }
        let items: Vec<serde_json::Value> = self
            .inner
            .host
            .queue()
            .await
            .into_iter()
            .take(QUEUE_MAX)
            .map(|q| {
                serde_json::json!({
                    "itemId": q.item_id,
                    "title": q.track.title,
                    "artist": artist_line(&q.track),
                    "coverKey": q.track.thumb.as_deref().map(|t| self.key_for(t)),
                })
            })
            .collect();
        Reply::ok(serde_json::json!({ "items": items }))
    }

    /// Start (or replace) the sleep timer.
    async fn start_sleep(&self, minutes: Option<f64>, end_of_song: bool) -> crate::error::AppResult<()> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let sleep = if end_of_song {
            let song = self.inner.host.now().await.and_then(|n| n.track).map(|t| t.id);
            let Some(song) = song else { return Err(nothing()) };
            Sleep { id, until: None, song: Some(song) }
        } else {
            let m = minutes.filter(|m| m.is_finite() && *m > 0.0).ok_or_else(|| AppError::Invalid("Say how many minutes.".into()))?;
            let d = Duration::from_secs_f64(m.min(SLEEP_MAX_MINUTES) * 60.0);
            Sleep { id, until: Some(tokio::time::Instant::now() + d), song: None }
        };
        *self.inner.sleep.lock() = Some(sleep.clone());
        let me = self.clone();
        tokio::spawn(async move { me.run_sleep(sleep).await });
        Ok(())
    }

    /// Whether this timer is still the one set.
    fn sleep_current(&self, id: u64) -> bool {
        self.inner.sleep.lock().as_ref().is_some_and(|s| s.id == id)
    }

    async fn run_sleep(&self, s: Sleep) {
        if let Some(until) = s.until {
            tokio::time::sleep_until(until).await;
        } else if let Some(song) = &s.song {
            loop {
                if !self.sleep_current(s.id) {
                    return;
                }
                let now = self.inner.host.now().await;
                let on = now.as_ref().and_then(|n| n.track.as_ref()).map(|t| t.id.as_str());
                if on != Some(song.as_str()) {
                    // The song ended: stop what follows at its start.
                    if self.sleep_current(s.id) && now.as_ref().is_some_and(|n| n.playing) {
                        let _ = self.inner.host.pause().await;
                        let _ = self.inner.host.seek(0).await;
                    }
                    break;
                }
                let left = now.as_ref().map(|n| n.duration_ms.saturating_sub(n.position_ms)).unwrap_or(u64::MAX);
                tokio::time::sleep(if left < 2000 { SLEEP_POLL_NEAR } else { SLEEP_POLL }).await;
            }
        }
        let ours = {
            let mut cur = self.inner.sleep.lock();
            let ours = cur.as_ref().is_some_and(|c| c.id == s.id);
            if ours {
                *cur = None;
            }
            ours
        };
        if ours && s.until.is_some() {
            let _ = self.inner.host.pause().await;
        }
    }

    async fn command(&self, cmd: Command) -> Reply {
        let host = &self.inner.host;
        let now = host.now().await;
        let track = now.as_ref().and_then(|n| n.track.clone());
        let r = match cmd {
            Command::PlayPause => host.toggle().await,
            Command::Next => host.next().await,
            Command::Previous => host.previous().await,
            Command::Like => match &track {
                Some(t) => {
                    let liked = host.liked(&t.id).await;
                    host.set_liked(&t.id, !liked).await
                }
                None => Err(nothing()),
            },
            Command::VolumeSet { value } => host.set_volume(percent(value)).await,
            Command::VolumeStep { value } => {
                let cur = now.as_ref().map(|n| if n.muted { 0.0 } else { n.volume }).unwrap_or(0.0) as f64 * 100.0;
                host.set_volume(percent(cur + value)).await
            }
            Command::RequestAlbum => match &track {
                Some(_) if !host.can_request() => Err(AppError::Invalid("Set up Lidarr in Platter's Settings first.".into())),
                Some(t) => host.request_album(t).await,
                None => Err(nothing()),
            },
            Command::ListenAlong => host.listen_along().await,
            Command::Play => host.resume().await,
            Command::Pause => host.pause().await,
            Command::PlayPlaylist { playlist, shuffle } => match playlist.trim() {
                "" => Err(AppError::Invalid("Name a playlist to play.".into())),
                p => host.play_playlist(p, shuffle).await,
            },
            Command::Search { query } => host.show(serde_json::json!({ "name": "search", "query": query.trim() })).await,
            Command::Open { page } => match super::automation::page_route(&page) {
                Some(route) => host.show(route).await,
                None => Err(AppError::Invalid(format!("Platter has no page called {}.", page.trim()))),
            },
            Command::SeekBy { value } => match &now {
                Some(n) if n.track.is_some() && value.is_finite() => {
                    let to = (n.position_ms as f64 + value * 1000.0).max(0.0) as u64;
                    let to = if n.duration_ms > 0 { to.min(n.duration_ms.saturating_sub(500)) } else { to };
                    host.seek(to).await
                }
                _ => Err(nothing()),
            },
            Command::Shuffle { smart: false } => host.set_shuffle(!now.as_ref().is_some_and(|n| n.shuffle)).await,
            Command::Shuffle { smart: true } => host.set_smart_shuffle(!now.as_ref().is_some_and(|n| n.smart_shuffle)).await,
            Command::Repeat => host.set_repeat(next_repeat(now.as_ref().map(|n| n.repeat).unwrap_or_default())).await,
            Command::Mute => host.set_muted(!now.as_ref().is_some_and(|n| n.muted)).await,
            Command::Crossfade => {
                let on = host.extras(None).await.crossfade;
                host.set_settings(serde_json::json!({ "crossfade": if on { "off" } else { "smart" } })).await
            }
            Command::PrivateSession => {
                let on = self.inner.config.lock().private_session;
                host.set_settings(serde_json::json!({ "privateSession": !on })).await
            }
            Command::AddToPlaylist { playlist } => match (&track, playlist.as_deref().map(str::trim)) {
                (None, _) => Err(nothing()),
                (Some(t), None | Some("")) => {
                    host.window_action(serde_json::json!({ "kind": "addToPlaylist", "trackIds": [t.id] }), true);
                    Ok(())
                }
                (Some(t), Some(p)) => {
                    if host.extras(Some(t)).await.playlists.iter().any(|id| id == p) {
                        Err(AppError::Invalid("The song is in that playlist already.".into()))
                    } else {
                        host.playlist_add(p, &t.id).await
                    }
                }
            },
            Command::RemoveFromPlaylist { playlist } => match &track {
                None => Err(nothing()),
                Some(t) if !host.extras(Some(t)).await.playlists.contains(&playlist) => {
                    Err(AppError::Invalid("The song isn't in that playlist.".into()))
                }
                Some(t) => host.playlist_remove(&playlist, &t.id).await,
            },
            Command::Rate { stars } => match &track {
                None => Err(nothing()),
                Some(_) if !(1..=5).contains(&stars) => Err(AppError::Invalid("A rating is 1 to 5 stars.".into())),
                Some(t) => {
                    let x = host.extras(Some(t)).await;
                    if !x.can_rate {
                        Err(AppError::Invalid("This song's server has no star ratings.".into()))
                    } else {
                        host.rate(&t.id, (x.stars != stars).then_some(stars)).await
                    }
                }
            },
            Command::PlayItem { kind, id, shuffle, queue, start } => match id.trim() {
                "" => Err(AppError::Invalid("Pick something to play.".into())),
                id => {
                    let ctx = PlayContext { kind: kind.context(), id: Some(id.to_owned()), track_ids: None };
                    if queue {
                        host.queue_context(ctx).await
                    } else {
                        let at = start.filter(|s| s.is_finite() && *s > 0.0).map_or(0, |s| (s * 1000.0) as u64);
                        host.play(ctx, None, at, shuffle).await
                    }
                }
            },
            Command::PlayLink { link } => match host.resolve_link(&link).await {
                None => Err(AppError::Invalid("That link isn't to something in your library.".into())),
                Some(l) => {
                    let (kind, id, start, at) = match l {
                        DeckLink::Album(id) => (ContextKind::Album, id, None, 0),
                        DeckLink::Artist(id) => (ContextKind::Artist, id, None, 0),
                        DeckLink::Playlist(id) => (ContextKind::Playlist, id, None, 0),
                        DeckLink::Track { id, album_id, at_seconds } => {
                            (ContextKind::Album, album_id, Some(id), u64::from(at_seconds.unwrap_or(0)) * 1000)
                        }
                    };
                    host.play(PlayContext { kind, id: Some(id), track_ids: None }, start, at, false).await
                }
            },
            Command::PlaySearch { query } => match query.trim() {
                "" => Err(AppError::Invalid("Type what to search for.".into())),
                q => match host.search_tracks(q, SEARCH_SONGS).await {
                    Ok(ids) if ids.is_empty() => Err(AppError::NotFound(format!("No songs found for {q}."))),
                    Ok(ids) => host.play(PlayContext { kind: ContextKind::Search, id: Some(q.to_owned()), track_ids: Some(ids) }, None, 0, false).await,
                    Err(e) => Err(e),
                },
            },
            Command::Radio => match &track {
                Some(t) => host.play(PlayContext { kind: ContextKind::Radio, id: Some(format!("track:{}", t.id)), track_ids: None }, None, 0, false).await,
                None => Err(nothing()),
            },
            Command::PlayShortcut { name } => match name.as_str() {
                "liked" => host.play(PlayContext { kind: ContextKind::Liked, id: Some("liked".into()), track_ids: None }, None, 0, false).await,
                "daylist" | "discover" => match host.mix_of_kind(&name).await {
                    Ok(Some(id)) => host.play(PlayContext { kind: ContextKind::Mix, id: Some(id), track_ids: None }, None, 0, false).await,
                    Ok(None) => Err(AppError::NotFound(format!("There's no {name} yet. Platter makes it after the library has been played a while."))),
                    Err(e) => Err(e),
                },
                _ => Err(AppError::Invalid(format!("Platter has no shortcut called {name}."))),
            },
            Command::QueueJump { item_id } => host.queue_jump(&item_id).await,
            Command::ListenOn { device } => match device.as_deref().map(str::trim) {
                None | Some("") => {
                    host.window_action(serde_json::json!({ "kind": "listenOn" }), true);
                    Ok(())
                }
                Some(d) => match host.devices().await.into_iter().find(|x| x.id == d) {
                    None => Err(AppError::NotFound("That device isn't around right now.".into())),
                    Some(x) if x.active => Ok(()),
                    Some(x) if x.needs_window => {
                        host.window_action(serde_json::json!({ "kind": "listenOn", "device": x.id }), true);
                        Ok(())
                    }
                    Some(x) => host.listen_on(&x.id).await,
                },
            },
            Command::Lyrics => {
                let open = self.inner.ui.lock().lyrics;
                host.window_action(serde_json::json!({ "kind": "lyrics", "open": !open }), !open);
                Ok(())
            }
            Command::Miniplayer => host.set_miniplayer(!host.extras(None).await.miniplayer).await,
            Command::Window => host.set_window(!host.extras(None).await.window).await,
            Command::NowPlaying => {
                host.window_action(serde_json::json!({ "kind": "nowPlaying" }), true);
                Ok(())
            }
            Command::Queue => {
                host.window_action(serde_json::json!({ "kind": "queue" }), true);
                Ok(())
            }
            Command::Sleep { minutes, end_of_song } => self.start_sleep(minutes, end_of_song).await,
            Command::SleepCancel => {
                *self.inner.sleep.lock() = None;
                Ok(())
            }
            Command::Bookmark => match now.as_ref().and_then(|n| n.track.as_ref().map(|t| (t, n.position_ms))) {
                Some((t, at)) => host.bookmark(t, at).await,
                None => Err(nothing()),
            },
        };
        match r {
            Ok(()) => Reply::ok(serde_json::json!({ "ok": true })),
            Err(e) => Reply::error(409, &message(&e)),
        }
    }

    /// Answer one request from `peer`, at `now`.
    pub async fn respond(&self, req: &Request, peer: IpAddr, port: u16, now: Instant) -> Reply {
        if !peer.is_loopback() {
            return Reply::error(403, "Only this computer can use Platter's control API.");
        }
        if !host_allowed(req.header("host"), port) {
            return Reply::error(403, "Use 127.0.0.1 or localhost.");
        }
        // Web pages always send one of these; the Stream Deck plugin never does.
        if req.header("origin").is_some() || req.header("sec-fetch-site").is_some_and(|v| v != "none") {
            return Reply::error(403, "Web pages can't use Platter's control API.");
        }
        if !self.inner.limit.lock().allow(now) {
            return Reply::error(429, "Too many requests. Slow down.");
        }
        let (path, query) = match req.target.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (req.target.as_str(), None),
        };
        let c = *self.inner.config.lock();
        let authed = req
            .bearer()
            .is_some_and(|t| (self.deck_on(&c) && self.inner.pairing.check(t)) || (c.automation && self.inner.automation.check(t)));
        match (req.method.as_str(), path) {
            ("GET", "/v1/hello") => Reply::ok(serde_json::json!({ "app": "Platter", "api": API_VERSION, "paired": authed })),
            ("POST", "/v1/pair") => {
                if !self.inner.pair_limit.lock().allow(now) {
                    return Reply::error(429, "Too many tries. Wait a moment.");
                }
                let body = serde_json::from_slice::<serde_json::Value>(&req.body).unwrap_or_default();
                let code = body.get("code").and_then(|c| c.as_str()).unwrap_or_default().to_owned();
                let automation = body.get("client").and_then(|c| c.as_str()) == Some("automation");
                let pairing = match automation {
                    true if !c.automation => return Reply::error(403, "Turn on Allow automation in Platter's Settings first."),
                    true => &self.inner.automation,
                    false if !self.deck_on(&c) => return Reply::error(403, "Turn on Allow Stream Deck in Platter's Settings first."),
                    false => &self.inner.pairing,
                };
                let r = pairing.pair(&code);
                self.emit_status();
                match r {
                    Ok(token) => Reply::ok(serde_json::json!({ "token": token })),
                    Err(PairError::WrongCode) => Reply::error(401, "That code doesn't match the one in Platter's Settings."),
                    Err(PairError::Keychain(_)) => Reply::error(500, "Platter couldn't save the pairing in the keychain."),
                }
            }
            ("POST", "/v1/pair/request") => {
                if !self.deck_on(&c) {
                    return Reply::error(403, "Turn on Allow Stream Deck in Platter's Settings first.");
                }
                if !self.inner.pair_limit.lock().allow(now) {
                    return Reply::error(429, "Too many tries. Wait a moment.");
                }
                let r = self.inner.asks.lock().open(now);
                match r {
                    Ok(id) => {
                        self.inner.host.raise();
                        self.emit_prompt(Some(&id));
                        Reply::ok(serde_json::json!({ "request": id }))
                    }
                    Err(AskError::Busy) => Reply::error(409, "Platter is already asking. Answer it in Platter's window."),
                    Err(AskError::TooSoon) => Reply::error(429, "Wait a moment before asking again."),
                }
            }
            ("POST", "/v1/pair/status") => {
                let body = serde_json::from_slice::<serde_json::Value>(&req.body).unwrap_or_default();
                let id = body.get("request").and_then(|c| c.as_str()).unwrap_or_default();
                let state = self.inner.asks.lock().status(id, now);
                match state {
                    AskState::Waiting => Reply::ok(serde_json::json!({ "state": "waiting" })),
                    AskState::Allowed(token) => Reply::ok(serde_json::json!({ "state": "allowed", "token": token })),
                    AskState::Denied => Reply::ok(serde_json::json!({ "state": "denied" })),
                    AskState::Gone => Reply::ok(serde_json::json!({ "state": "gone" })),
                }
            }
            (_, "/v1/state" | "/v1/command" | "/v1/cover" | "/v1/library" | "/v1/queue" | "/v1/devices") if !authed => {
                Reply::error(401, "Pair with Platter first.")
            }
            ("GET", "/v1/state") => {
                let have = param(query, "cover");
                Reply::ok(serde_json::to_value(self.state(have.as_deref(), cover_size(query)).await).unwrap_or_default())
            }
            ("GET", "/v1/cover") => match param(query, "key") {
                Some(key) => self.cover(&key, cover_size(query)).await,
                None => Reply::error(400, "Name a cover."),
            },
            ("GET", "/v1/library") => match param(query, "kind").as_deref().and_then(LibraryKind::parse) {
                Some(kind) => self.library(kind).await,
                None => Reply::error(400, "Ask for playlists, albums, artists or mixes."),
            },
            ("GET", "/v1/queue") => self.queue().await,
            ("GET", "/v1/devices") => Reply::ok(serde_json::json!({ "items": self.inner.host.devices().await })),
            ("POST", "/v1/command") => match serde_json::from_slice::<Command>(&req.body) {
                Ok(cmd) => self.command(cmd).await,
                Err(_) => Reply::error(400, "Unknown command."),
            },
            (
                _,
                "/v1/hello" | "/v1/pair" | "/v1/pair/request" | "/v1/pair/status" | "/v1/state" | "/v1/command" | "/v1/cover" | "/v1/library"
                | "/v1/queue" | "/v1/devices",
            ) => {
                Reply::error(405, "Wrong method.")
            }
            _ => Reply::error(404, "Not found."),
        }
    }

    async fn serve_conn(&self, mut stream: TcpStream, peer: IpAddr, port: u16) {
        let reply = match tokio::time::timeout(READ_TIMEOUT, read_request(&mut stream)).await {
            Ok(Ok(req)) => self.respond(&req, peer, port, Instant::now()).await,
            Ok(Err(status)) => Reply::error(status, "Bad request."),
            Err(_) => return,
        };
        let body = serde_json::to_vec(&reply.body).unwrap_or_default();
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
            reply.status,
            reason(reply.status),
            body.len()
        );
        let _ = stream.write_all(head.as_bytes()).await;
        let _ = stream.write_all(&body).await;
        let _ = stream.shutdown().await;
    }

    /// Bind 127.0.0.1:`port`. Never any other address.
    pub async fn bind(port: u16) -> std::io::Result<TcpListener> {
        TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await
    }

    /// Follow the settings: open or close the port as they change.
    pub async fn follow(self, mut cfg: watch::Receiver<DeckConfig>, port: u16) {
        loop {
            let c = *cfg.borrow_and_update();
            *self.inner.config.lock() = c;
            if !c.listening() {
                self.set_listening(Listening::Off);
                if cfg.changed().await.is_err() {
                    return;
                }
                continue;
            }
            let listener = match Self::bind(port).await {
                Ok(l) => l,
                Err(e) => {
                    let l = if e.kind() == std::io::ErrorKind::AddrInUse || e.kind() == std::io::ErrorKind::PermissionDenied {
                        Listening::PortInUse { port }
                    } else {
                        Listening::Failed { port, message: e.to_string() }
                    };
                    tracing::info!(port, "stream deck control not started: {e}");
                    self.set_listening(l);
                    if cfg.changed().await.is_err() {
                        return;
                    }
                    continue;
                }
            };
            let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
            self.set_listening(Listening::On { port });
            loop {
                tokio::select! {
                    r = listener.accept() => match r {
                        Ok((s, addr)) => {
                            let me = self.clone();
                            tokio::spawn(async move { me.serve_conn(s, addr.ip(), port).await });
                        }
                        Err(e) => {
                            tracing::debug!("stream deck accept failed: {e}");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    },
                    r = cfg.changed() => {
                        if r.is_err() {
                            return;
                        }
                        let next = *cfg.borrow();
                        if next.listening() == c.listening() {
                            *self.inner.config.lock() = next;
                            // The codes shown follow which sides are on.
                            self.emit_status();
                            continue;
                        }
                        break;
                    }
                }
            }
            drop(listener);
        }
    }
}

/// One query parameter.
fn param(query: Option<&str>, name: &str) -> Option<String> {
    query.and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == name).map(|(_, v)| v.into_owned()))
}

/// The cover size asked for: 144 px unless `size` asks for more (up to 288).
pub fn cover_size(query: Option<&str>) -> u32 {
    param(query, "size").and_then(|v| v.parse::<u32>().ok()).map_or(COVER_SIZE, |s| s.clamp(COVER_SIZE, COVER_SIZE_MAX))
}

/// Off, all, one, then off again.
pub fn next_repeat(r: Repeat) -> Repeat {
    match r {
        Repeat::Off => Repeat::All,
        Repeat::All => Repeat::One,
        Repeat::One => Repeat::Off,
    }
}

fn nothing() -> AppError {
    AppError::Invalid("Nothing is playing.".into())
}

/// 0..100 percent to the player's 0..1.
fn percent(v: f64) -> f32 {
    if v.is_finite() { (v / 100.0).clamp(0.0, 1.0) as f32 } else { 0.0 }
}

/// The message a key shows (or logs) for a failed command.
fn message(e: &AppError) -> String {
    match e {
        AppError::Invalid(m) | AppError::NotFound(m) => m.clone(),
        AppError::Network(_) | AppError::Server(_) => "Platter couldn't reach the server.".into(),
        _ => "That didn't work.".into(),
    }
}

/// Read one request: head, then a body up to [`MAX_BODY`]. Err is the status to answer.
async fn read_request(stream: &mut TcpStream) -> Result<Request, u16> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let end = loop {
        let n = stream.read(&mut chunk).await.map_err(|_| 400u16)?;
        if n == 0 {
            return Err(400);
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return Err(413);
        }
    };
    let mut req = parse_head(&String::from_utf8_lossy(&buf[..end])).ok_or(400u16)?;
    let len: usize = req.header("content-length").map(|v| v.trim().parse().map_err(|_| 400u16)).transpose()?.unwrap_or(0);
    if len > MAX_BODY {
        return Err(413);
    }
    let mut body = buf[end..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).await.map_err(|_| 400u16)?;
        if n == 0 {
            return Err(400);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    req.body = body;
    Ok(req)
}

/// The request line and headers.
pub fn parse_head(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    if !target.starts_with('/') {
        return None;
    }
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
    Some(Request { method, target, headers, body: vec![] })
}
