//! Social: friend activity, following artists with What's New, and Listen
//! along.
//!
//! - [`activity`]: who else on the Plex server listened lately (server
//!   sessions and history, owner only).
//! - [`follows`]: the follow list (behind a store a sync file can replace),
//!   What's New, and the new-release announcement.
//! - [`listen`]: listening sessions through a relay, end to end sealed.
//! - [`sharing`]: sharing your music through Plex, and invites to you.
//! - [`commands`]: Tauri commands and the Plex, player and notification glue.

pub mod activity;
pub mod circle;
pub mod commands;
pub mod follows;
pub mod listen;
pub mod sharing;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tokio::time::Instant;

use crate::db::DbHandle;
use crate::error::AppResult;
use crate::plex::state::EventSink;
use activity::{ActivityPanel, ActivitySource, ActivityState};
use follows::{FollowStore, FollowedArtist, WhatsNew};

pub const EVENT_FOLLOWS: &str = "follows://changed";
pub const EVENT_WHATS_NEW: &str = "whatsnew://changed";

/// Friend activity is read again at most this often.
pub const ACTIVITY_TTL: Duration = Duration::from_secs(20);

/// The settings this module reads directly.
#[derive(Clone)]
pub struct SocialSwitches {
    pub show_activity: Arc<AtomicBool>,
    pub whats_new: Arc<AtomicBool>,
    pub notify_releases: Arc<AtomicBool>,
    /// Listen along settings.
    pub listen: listen::ListenSettings,
    /// Sharing settings.
    pub sharing: sharing::ShareSettings,
    /// A private session: no announcements.
    pub private_session: Arc<AtomicBool>,
    /// Collaborative playlists and Blends (sharing listen along's relay,
    /// invite lifetime and the private session).
    pub circle: circle::CircleSettings,
}

impl Default for SocialSwitches {
    fn default() -> Self {
        let listen = listen::ListenSettings::default();
        let private_session = Arc::new(AtomicBool::new(false));
        let circle = circle::CircleSettings {
            private_session: private_session.clone(),
            relay: listen.relay.clone(),
            invite_hours: listen.invite_hours.clone(),
            ..Default::default()
        };
        Self {
            show_activity: Arc::new(AtomicBool::new(true)),
            whats_new: Arc::new(AtomicBool::new(true)),
            notify_releases: Arc::new(AtomicBool::new(false)),
            listen,
            sharing: sharing::ShareSettings::default(),
            private_session,
            circle,
        }
    }
}

/// Shows desktop notifications (the OS in the app, a recorder in tests).
pub trait Notifier: Send + Sync {
    fn notify(&self, title: &str, body: &str);
}

/// `whatsnew://changed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNewBadge {
    pub unseen: u32,
}

struct Inner {
    db: DbHandle,
    follows: Arc<dyn FollowStore>,
    activity: Arc<dyn ActivitySource>,
    events: Arc<dyn EventSink>,
    notifier: Arc<dyn Notifier>,
    switches: SocialSwitches,
    cache: Mutex<Option<(Instant, ActivityPanel)>>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

/// Friend activity, follows and What's New. Cheap to clone.
#[derive(Clone)]
pub struct Social {
    inner: Arc<Inner>,
}

pub struct SocialDeps {
    pub db: DbHandle,
    pub follows: Arc<dyn FollowStore>,
    pub activity: Arc<dyn ActivitySource>,
    pub events: Arc<dyn EventSink>,
    pub notifier: Arc<dyn Notifier>,
    pub switches: SocialSwitches,
    /// Unix seconds now (fixed in tests).
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl Social {
    pub fn new(d: SocialDeps) -> Self {
        Self {
            inner: Arc::new(Inner {
                db: d.db,
                follows: d.follows,
                activity: d.activity,
                events: d.events,
                notifier: d.notifier,
                switches: d.switches,
                cache: Mutex::new(None),
                clock: d.clock,
            }),
        }
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    pub fn switches(&self) -> &SocialSwitches {
        &self.inner.switches
    }

    /// The friend activity panel (read again after [`ACTIVITY_TTL`], or
    /// when `fresh`).
    pub async fn activity(&self, fresh: bool) -> ActivityPanel {
        if !self.inner.switches.show_activity.load(Ordering::Relaxed) {
            return ActivityPanel::empty(ActivityState::Off);
        }
        if !fresh
            && let Some((at, p)) = self.inner.cache.lock().clone()
            && at.elapsed() < ACTIVITY_TTL
        {
            return p;
        }
        let mut panel = activity::read(self.inner.activity.as_ref(), self.now()).await;
        let mut friends = panel.friends.clone();
        match self.inner.db.read_async(move |c| activity::place(c, &mut friends).map(|_| friends)).await {
            Ok(f) => panel.friends = f,
            Err(e) => {
                // Shown without links rather than with ids that may not be here.
                tracing::debug!(kind = e.kind(), "friend activity couldn't be matched to the library");
                for f in &mut panel.friends {
                    (f.track_id, f.album_id, f.artist_id, f.in_library) = (None, None, None, false);
                }
            }
        }
        *self.inner.cache.lock() = Some((Instant::now(), panel.clone()));
        panel
    }

    pub fn follows(&self) -> AppResult<Vec<FollowedArtist>> {
        self.inner.follows.list()
    }

    /// Follow or unfollow an artist; tells the UI and refreshes the badge.
    pub fn set_follow(&self, id: String, title: String, thumb: Option<String>, on: bool) -> AppResult<Vec<FollowedArtist>> {
        let artist = FollowedArtist { id, title, thumb, followed_at: self.now(), just_show: false };
        let all = follows::set_follow(self.inner.follows.as_ref(), artist, on)?;
        self.inner.events.emit(EVENT_FOLLOWS, serde_json::to_value(&all).unwrap_or_default());
        self.badge();
        Ok(all)
    }

    /// Get a followed artist's new albums, or just show them.
    pub fn set_just_show(&self, id: &str, just_show: bool) -> AppResult<Vec<FollowedArtist>> {
        let all = follows::set_just_show(self.inner.follows.as_ref(), id, just_show)?;
        self.inner.events.emit(EVENT_FOLLOWS, serde_json::to_value(&all).unwrap_or_default());
        Ok(all)
    }

    pub fn whats_new(&self) -> AppResult<WhatsNew> {
        follows::load(&self.inner.db, self.inner.follows.as_ref(), self.now())
    }

    /// The page was seen: what's on it is no longer new.
    pub fn whats_new_seen(&self) -> AppResult<()> {
        follows::mark_seen(&self.inner.db, self.now())?;
        self.emit_badge(0);
        Ok(())
    }

    fn emit_badge(&self, unseen: u32) {
        let unseen = if self.inner.switches.whats_new.load(Ordering::Relaxed) { unseen } else { 0 };
        self.inner.events.emit(EVENT_WHATS_NEW, serde_json::to_value(WhatsNewBadge { unseen }).unwrap_or_default());
    }

    /// Tell the UI how many What's New items are unseen.
    pub fn badge(&self) {
        match self.whats_new() {
            Ok(w) => self.emit_badge(w.unseen),
            Err(e) => tracing::debug!("what's new: {e}"),
        }
    }

    /// After the library changed: announce new releases by followed artists
    /// (when that's on and no private session is), and refresh the badge.
    pub fn library_changed(&self) {
        match follows::check(&self.inner.db, self.inner.follows.as_ref(), self.now()) {
            Ok(albums) => {
                let s = &self.inner.switches;
                if s.notify_releases.load(Ordering::Relaxed)
                    && !s.private_session.load(Ordering::Relaxed)
                    && let Some((title, body)) = follows::notice(&albums)
                {
                    self.inner.notifier.notify(&title, &body);
                }
            }
            Err(e) => tracing::debug!("new release check: {e}"),
        }
        self.badge();
    }
}
