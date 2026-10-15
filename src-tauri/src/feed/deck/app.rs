//! The real [`DeckHost`]: the player, the art cache, likes, Lidarr and
//! Listen along. Request album and Listen along bring the window up and let
//! the UI show the usual confirmation (`deck://action`).

use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use super::{DeckHost, DeckNow};
use crate::audio::events::PlayerStatus;
use crate::audio::player::PlayerHandle;
use crate::db::ArtCache;
use crate::db::source::ArtBytes;
use crate::error::{AppError, AppResult};
use crate::lidarr::Lidarr;
use crate::likes::Likes;
use crate::model::Track;

/// How long "Lidarr is set up" is remembered (it reads the keychain).
const READY_TTL: Duration = Duration::from_secs(30);

pub struct AppDeck {
    pub app: AppHandle,
    pub player: PlayerHandle,
    pub art: ArtCache,
    ready: Mutex<Option<(Instant, bool)>>,
}

impl AppDeck {
    pub fn new(app: AppHandle, player: PlayerHandle, art: ArtCache) -> Self {
        Self { app, player, art, ready: Mutex::new(None) }
    }

    fn likes(&self) -> AppResult<Likes> {
        self.app.try_state::<Likes>().map(|l| l.inner().clone()).ok_or_else(|| AppError::Internal("likes not ready".into()))
    }

    fn action(&self, payload: serde_json::Value) {
        crate::platform::show_window(&self.app);
        if let Err(e) = self.app.emit("deck://action", payload) {
            tracing::warn!("emit deck://action failed: {e}");
        }
    }
}

#[async_trait]
impl DeckHost for AppDeck {
    async fn now(&self) -> Option<DeckNow> {
        let s = self.player.state().await.ok()?;
        Some(DeckNow {
            playing: matches!(s.status, PlayerStatus::Playing | PlayerStatus::Buffering),
            volume: s.volume,
            muted: s.muted,
            track: s.track,
        })
    }

    async fn liked(&self, track_id: &str) -> bool {
        match self.likes() {
            Ok(l) => l.is_liked(track_id).await.unwrap_or(false),
            Err(_) => false,
        }
    }

    async fn cover(&self, thumb: &str, size: u32) -> AppResult<ArtBytes> {
        self.art.get_or_fetch(thumb, size, size).await
    }

    fn can_request(&self) -> bool {
        let mut r = self.ready.lock();
        if let Some((at, v)) = *r
            && at.elapsed() < READY_TTL
        {
            return v;
        }
        let v = self.app.try_state::<Lidarr>().is_some_and(|l| {
            let s = l.status();
            (s.url_set || s.away_set) && s.has_key
        });
        *r = Some((Instant::now(), v));
        v
    }

    async fn toggle(&self) -> AppResult<()> {
        self.player.toggle().await
    }

    async fn next(&self) -> AppResult<()> {
        self.player.next().await
    }

    async fn previous(&self) -> AppResult<()> {
        self.player.previous().await
    }

    async fn set_liked(&self, track_id: &str, liked: bool) -> AppResult<()> {
        self.likes()?.set(vec![track_id.to_owned()], liked).await.map(|_| ())
    }

    async fn set_volume(&self, volume: f32) -> AppResult<()> {
        self.player.set_volume(volume).await?;
        if volume > 0.0 && self.player.state().await.is_ok_and(|s| s.muted) {
            self.player.set_muted(false).await?;
        }
        Ok(())
    }

    async fn request_album(&self, track: &Track) -> AppResult<()> {
        let lidarr = self.app.try_state::<Lidarr>().map(|l| l.inner().clone()).ok_or_else(|| AppError::Invalid("Lidarr isn't set up.".into()))?;
        let artist = crate::feed::artist_line(track);
        let picked = lidarr.pick(&track.artist_title, None, &track.title, true, None).await?;
        let Some(group) = picked.group else {
            return Err(AppError::Invalid(picked.reason.unwrap_or_else(|| format!("Nothing to request for {artist}."))));
        };
        self.action(serde_json::json!({
            "kind": "requestAlbum",
            "input": { "releaseGroupId": group.id, "title": group.title, "artist": picked.artist, "artistId": null },
        }));
        Ok(())
    }

    async fn listen_along(&self) -> AppResult<()> {
        self.action(serde_json::json!({ "kind": "listenAlong" }));
        Ok(())
    }
}
