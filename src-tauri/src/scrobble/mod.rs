//! Scrobbling to Last.fm and ListenBrainz.
//!
//! The scrobbler follows the player's tracking events on its own task:
//! - a song starting tells each service it's playing now (best effort);
//! - a song that ends counts as a scrobble when it's longer than 30 s and
//!   was heard for half its length or 4 minutes, whichever comes first
//!   (Last.fm's rule, used for both services);
//! - scrobbles that can't be sent (offline, the service down) wait in a
//!   queue on this computer and go out oldest first when it can reach the
//!   service again;
//! - a private session never scrobbles, and unreleased music only does
//!   when its setting is on.
//!
//! When a service refuses the sign-in (Last.fm session, ListenBrainz token)
//! the scrobbles wait, Settings shows it and a toast says so once.

pub mod commands;
pub mod lastfm;
pub mod listenbrainz;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::audio::events::TrackingEvent;
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::plex::state::EventSink;

pub const EVENT_SCROBBLE: &str = "scrobble://status";
/// Songs this short never scrobble.
pub const MIN_TRACK_MS: u64 = 30_000;
/// Heard this long, a song scrobbles even if it's longer than twice this.
pub const MAX_NEEDED_MS: u64 = 240_000;
/// Most scrobbles kept waiting.
pub const QUEUE_MAX: usize = 5000;
/// Last.fm turns away scrobbles older than two weeks; both services get
/// the same rule so the queue never fills with ones that can't go out.
pub const QUEUE_MAX_AGE_S: i64 = 14 * 24 * 3600;

const KEY_QUEUE: &str = "scrobble.queue.v1";

/// Whether a play of `duration_ms` heard for `listened_ms` counts.
pub fn counts(duration_ms: u64, listened_ms: u64) -> bool {
    duration_ms > MIN_TRACK_MS && listened_ms >= (duration_ms / 2).min(MAX_NEEDED_MS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Service {
    Lastfm,
    Listenbrainz,
}

impl Service {
    pub const ALL: [Service; 2] = [Service::Lastfm, Service::Listenbrainz];

    pub fn display(self) -> &'static str {
        match self {
            Service::Lastfm => "Last.fm",
            Service::Listenbrainz => "ListenBrainz",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Service::Lastfm => "lastfm",
            Service::Listenbrainz => "listenbrainz",
        }
    }
}

/// What a service gets about one play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listen {
    pub artist: String,
    pub title: String,
    pub album: String,
    /// The album artist, when the song's own artist differs.
    #[serde(default)]
    pub album_artist: Option<String>,
    pub duration_s: u32,
    pub track_number: u32,
    /// When it started, unix seconds.
    pub started_at: i64,
}

impl Listen {
    pub fn from_track(t: &Track, started_at: i64) -> Self {
        let artist = t.track_artist.clone().filter(|a| !a.trim().is_empty()).unwrap_or_else(|| t.artist_title.clone());
        let album_artist = (artist != t.artist_title).then(|| t.artist_title.clone());
        Listen {
            artist,
            title: t.title.clone(),
            album: t.album_title.clone(),
            album_artist,
            duration_s: (t.duration_ms / 1000) as u32,
            track_number: t.track_number,
            started_at,
        }
    }
}

/// Why a send didn't go through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// Not signed in (nothing to send with).
    NotSignedIn,
    /// The service refused the session or token.
    Rejected,
    /// Couldn't reach it, or it's having trouble: try again later.
    Later(String),
    /// It refused these scrobbles themselves: drop them.
    Refused(String),
}

/// One service.
#[async_trait]
pub trait ScrobbleService: Send + Sync {
    fn service(&self) -> Service;
    /// Signed in (a session or token is saved).
    fn signed_in(&self) -> bool;
    async fn now_playing(&self, l: &Listen) -> Result<(), SendError>;
    /// Up to 50 at once.
    async fn scrobble(&self, ls: &[Listen]) -> Result<(), SendError>;
}

/// A scrobble waiting to be sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queued {
    pub service: Service,
    pub listen: Listen,
}

/// The settings the scrobbler reads directly.
#[derive(Clone)]
pub struct ScrobbleSwitches {
    pub lastfm: Arc<AtomicBool>,
    pub listenbrainz: Arc<AtomicBool>,
    pub unreleased: Arc<AtomicBool>,
    /// Shared with play tracking.
    pub private_session: Arc<AtomicBool>,
}

impl Default for ScrobbleSwitches {
    fn default() -> Self {
        Self {
            lastfm: Arc::new(AtomicBool::new(false)),
            listenbrainz: Arc::new(AtomicBool::new(false)),
            unreleased: Arc::new(AtomicBool::new(false)),
            private_session: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ScrobbleSwitches {
    pub fn on(&self, s: Service) -> bool {
        match s {
            Service::Lastfm => self.lastfm.load(Ordering::Relaxed),
            Service::Listenbrainz => self.listenbrainz.load(Ordering::Relaxed),
        }
    }
}

/// Tells the user when a service refuses the sign-in.
pub trait ScrobbleNotice: Send + Sync {
    fn rejected(&self, service: Service);
}

/// Whether a song is unreleased (from the library cache).
pub trait Unreleased: Send + Sync {
    fn unreleased(&self, track: &Track) -> bool;
}

/// Unreleased checks from the cache; a song it doesn't know counts as
/// unreleased (the cautious choice).
pub struct CacheUnreleased(pub DbHandle);

impl Unreleased for CacheUnreleased {
    fn unreleased(&self, track: &Track) -> bool {
        let album = track.album_id.clone();
        let part = track.part_key.clone();
        let row = self.0.read(move |c| {
            Ok(c.query_row(
                "SELECT al.format, al.subformats, coalesce(f.version, at.version), al.studio, al.matched, mc.found FROM albums al
                 LEFT JOIN album_tags at ON at.album_id = al.id
                 LEFT JOIN album_mb_check mc ON mc.album_id = al.id
                 LEFT JOIN file_tags f ON f.part_key = ?2 AND f.status = 'ok'
                 WHERE al.id = ?1",
                rusqlite::params![album, part],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<bool>>(4)?,
                        r.get::<_, Option<bool>>(5)?,
                    ))
                },
            )
            .optional()?)
        });
        match row {
            Ok(Some((format, subs, version, studio, matched, mb))) => {
                let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
                crate::db::release::is_unreleased(format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref(), mb)
            }
            _ => true,
        }
    }
}

pub struct ScrobbleDeps {
    pub db: DbHandle,
    pub services: Vec<Arc<dyn ScrobbleService>>,
    pub switches: ScrobbleSwitches,
    pub unreleased: Arc<dyn Unreleased>,
    pub notice: Arc<dyn ScrobbleNotice>,
    pub events: Arc<dyn EventSink>,
    /// Unix seconds now (fixed in tests).
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

struct Inner {
    db: DbHandle,
    services: Vec<Arc<dyn ScrobbleService>>,
    switches: ScrobbleSwitches,
    unreleased: Arc<dyn Unreleased>,
    notice: Arc<dyn ScrobbleNotice>,
    events: Arc<dyn EventSink>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// Services that refused the sign-in (until the next sign-in).
    rejected: Mutex<Vec<Service>>,
    /// The play in progress: its listen, when it's not to be scrobbled
    /// (private session, unreleased), and its queue item.
    current: Mutex<Option<(String, Listen)>>,
    sending: tokio::sync::Mutex<()>,
}

/// Cheap to clone.
#[derive(Clone)]
pub struct Scrobbler {
    inner: Arc<Inner>,
}

impl Scrobbler {
    pub fn new(d: ScrobbleDeps) -> Self {
        let rejected = Service::ALL
            .into_iter()
            .filter(|s| d.db.read(|c| kv_get(c, &rejected_key(*s))).ok().flatten().is_some())
            .collect();
        Self {
            inner: Arc::new(Inner {
                db: d.db,
                services: d.services,
                switches: d.switches,
                unreleased: d.unreleased,
                notice: d.notice,
                events: d.events,
                clock: d.clock,
                rejected: Mutex::new(rejected),
                current: Mutex::new(None),
                sending: Default::default(),
            }),
        }
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    pub fn service(&self, s: Service) -> Option<&Arc<dyn ScrobbleService>> {
        self.inner.services.iter().find(|x| x.service() == s)
    }

    pub fn is_rejected(&self, s: Service) -> bool {
        self.inner.rejected.lock().contains(&s)
    }

    /// A new sign-in: the service is trusted again and waiting scrobbles go.
    pub fn signed_in(&self, s: Service) {
        self.inner.rejected.lock().retain(|x| *x != s);
        let _ = self.inner.db.write(|c| {
            c.execute("DELETE FROM kv WHERE key = ?1", [rejected_key(s)])?;
            Ok(())
        });
        self.emit_status();
    }

    /// The service refused the sign-in (found outside a send).
    pub fn reject_now(&self, s: Service) {
        self.reject(s);
    }

    fn reject(&self, s: Service) {
        let first = {
            let mut r = self.inner.rejected.lock();
            let first = !r.contains(&s);
            if first {
                r.push(s);
            }
            first
        };
        if first {
            tracing::info!("{} refused the sign-in; scrobbles wait", s.display());
            let _ = self.inner.db.write(|c| kv_set(c, &rejected_key(s), "1"));
            self.inner.notice.rejected(s);
            self.emit_status();
        }
    }

    /// Services that should get this play: on, signed in, not refused.
    fn targets(&self) -> Vec<Arc<dyn ScrobbleService>> {
        self.inner
            .services
            .iter()
            .filter(|s| self.inner.switches.on(s.service()) && s.signed_in() && !self.is_rejected(s.service()))
            .cloned()
            .collect()
    }

    /// Whether this song may go out at all right now.
    fn allowed(&self, t: &Track) -> bool {
        !self.inner.switches.private_session.load(Ordering::Relaxed)
            && (self.inner.switches.unreleased.load(Ordering::Relaxed) || !self.inner.unreleased.unreleased(t))
    }

    // ---- the queue -------------------------------------------------------------

    pub fn queued(&self) -> AppResult<Vec<Queued>> {
        let raw = self.inner.db.read(|c| kv_get(c, KEY_QUEUE))?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    fn save_queue(&self, q: &[Queued]) -> AppResult<()> {
        let json = serde_json::to_string(q).map_err(|e| AppError::Internal(e.to_string()))?;
        self.inner.db.write(|c| kv_set(c, KEY_QUEUE, &json))
    }

    fn enqueue(&self, items: Vec<Queued>) {
        let mut q = self.queued().unwrap_or_default();
        q.extend(items);
        if q.len() > QUEUE_MAX {
            let drop = q.len() - QUEUE_MAX;
            q.drain(..drop);
        }
        if let Err(e) = self.save_queue(&q) {
            tracing::warn!("scrobble queue: {e}");
        }
        self.emit_status();
    }

    fn emit_status(&self) {
        let n = self.queued().map(|q| q.len()).unwrap_or(0);
        self.inner.events.emit(EVENT_SCROBBLE, serde_json::json!({ "queued": n }));
    }

    // ---- events ------------------------------------------------------------------

    /// Follow one tracking event.
    pub async fn handle(&self, ev: &TrackingEvent) {
        match ev {
            TrackingEvent::TrackStarted { track, item_id, at, .. } => {
                if !self.allowed(track) {
                    *self.inner.current.lock() = None;
                    return;
                }
                let listen = Listen::from_track(track, at / 1000);
                *self.inner.current.lock() = Some((item_id.clone(), listen.clone()));
                for s in self.targets() {
                    match s.now_playing(&listen).await {
                        Err(SendError::Rejected) => self.reject(s.service()),
                        Err(e) => tracing::debug!("{} now playing not sent: {e:?}", s.service().display()),
                        Ok(()) => {}
                    }
                }
            }
            TrackingEvent::TrackEnded { track, item_id, listened_ms, .. } => {
                let cur = self.inner.current.lock().take();
                let Some((cur_item, listen)) = cur else { return };
                // A private session that started mid-song still keeps it out.
                if cur_item != *item_id || !counts(track.duration_ms, *listened_ms) || !self.allowed(track) {
                    return;
                }
                self.scrobble(listen).await;
            }
            _ => {}
        }
    }

    /// Send one scrobble to every service that should get it; whatever
    /// can't go now waits in the queue.
    pub async fn scrobble(&self, listen: Listen) {
        let mut later = Vec::new();
        let mut flush = false;
        for s in self.targets() {
            // Anything already waiting for this service goes first, in order.
            let waiting = self.queued().unwrap_or_default().iter().any(|q| q.service == s.service());
            if waiting {
                later.push(Queued { service: s.service(), listen: listen.clone() });
                flush = true;
                continue;
            }
            match s.scrobble(std::slice::from_ref(&listen)).await {
                Ok(()) => {}
                Err(SendError::Refused(why)) => tracing::info!("{} refused a scrobble: {why}", s.service().display()),
                Err(SendError::Rejected) => {
                    self.reject(s.service());
                    later.push(Queued { service: s.service(), listen: listen.clone() });
                }
                Err(SendError::Later(_) | SendError::NotSignedIn) => later.push(Queued { service: s.service(), listen: listen.clone() }),
            }
        }
        // A service that's on but signed out or refused keeps the play for later too.
        for s in &self.inner.services {
            let svc = s.service();
            if self.inner.switches.on(svc) && (self.is_rejected(svc) || !s.signed_in()) && !later.iter().any(|q| q.service == svc) {
                later.push(Queued { service: svc, listen: listen.clone() });
            }
        }
        if !later.is_empty() {
            self.enqueue(later);
        }
        if flush {
            self.replay().await;
        }
    }

    /// Send waiting scrobbles, oldest first, 50 at a time per service.
    /// Stops for a service at its first failure. Too old ones are dropped.
    pub async fn replay(&self) {
        let _one = self.inner.sending.lock().await;
        let now = self.now();
        let Ok(mut q) = self.queued() else { return };
        // Nothing waiting: don't touch the services (and their keychain entries).
        if q.is_empty() {
            return;
        }
        let before = q.len();
        q.retain(|x| now - x.listen.started_at <= QUEUE_MAX_AGE_S);
        for s in self.targets() {
            let svc = s.service();
            loop {
                let batch: Vec<Listen> = q.iter().filter(|x| x.service == svc).take(50).map(|x| x.listen.clone()).collect();
                if batch.is_empty() {
                    break;
                }
                let done = match s.scrobble(&batch).await {
                    Ok(()) => true,
                    Err(SendError::Refused(why)) => {
                        tracing::info!("{} refused waiting scrobbles: {why}", svc.display());
                        true
                    }
                    Err(SendError::Rejected) => {
                        self.reject(svc);
                        false
                    }
                    Err(_) => false,
                };
                if !done {
                    break;
                }
                let mut n = batch.len();
                q.retain(|x| {
                    if n > 0 && x.service == svc {
                        n -= 1;
                        false
                    } else {
                        true
                    }
                });
            }
        }
        if q.len() != before {
            let _ = self.save_queue(&q);
            self.emit_status();
        }
    }

    pub async fn run(self, mut events: broadcast::Receiver<TrackingEvent>) {
        loop {
            let ev = match events.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            };
            self.handle(&ev).await;
        }
    }
}

fn rejected_key(s: Service) -> String {
    format!("scrobble.{}.rejected", s.key())
}

/// Follow the player, and send waiting scrobbles every minute.
pub fn spawn(scrobbler: Scrobbler, events: broadcast::Receiver<TrackingEvent>) {
    tauri::async_runtime::spawn(scrobbler.clone().run(events));
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            scrobbler.replay().await;
        }
    });
}
