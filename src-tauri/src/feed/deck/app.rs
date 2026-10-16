//! The real [`DeckHost`]: the player, the art cache, likes, Lidarr and
//! Listen along. Request album and Listen along bring the window up and let
//! the UI show the usual confirmation (`deck://action`).

use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use super::{DeckDevice, DeckExtras, DeckHost, DeckItem, DeckLink, DeckNow, DeckQueued, LOCAL_DEVICE, LibraryKind};
use crate::audio::queue::Repeat;
use crate::audio::source::PlayContext;
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
        self.window_action(payload, true);
    }

    fn db(&self) -> AppResult<crate::db::DbHandle> {
        self.app.try_state::<crate::db::DbHandle>().map(|d| d.inner().clone()).ok_or_else(|| AppError::Internal("library not ready".into()))
    }

    fn remote(&self) -> Option<crate::remote::RemoteService> {
        self.app.try_state::<crate::remote::RemoteService>().map(|r| r.inner().clone())
    }

    /// The playlists the library shows, as the window filters them.
    async fn playlists(&self) -> AppResult<Vec<crate::model::Playlist>> {
        let db = self.db()?;
        let server = db.server_view();
        db.read_async(move |c| {
            let mut out = crate::db::queries::playlists(c)?;
            crate::db::merge::ViewFilter::load(c, &server)?.playlists(&mut out);
            Ok(out)
        })
        .await
    }
}

#[async_trait]
impl DeckHost for AppDeck {
    async fn now(&self) -> Option<DeckNow> {
        let s = self.player.state().await.ok()?;
        let next = s.queue.user_queue.first().or(s.queue.next_up.first()).map(|q| q.track.clone());
        let previous = s.queue.history.first().map(|q| q.track.clone());
        Some(DeckNow {
            playing: matches!(s.status, PlayerStatus::Playing | PlayerStatus::Buffering),
            volume: s.volume,
            muted: s.muted,
            position_ms: s.position_ms,
            duration_ms: s.duration_ms,
            shuffle: s.shuffle,
            smart_shuffle: s.smart_shuffle,
            repeat: s.repeat,
            next,
            previous,
            context: s.queue.context.map(|c| (c.kind, c.id)),
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

    async fn pause(&self) -> AppResult<()> {
        self.player.pause().await
    }

    async fn resume(&self) -> AppResult<()> {
        self.player.resume().await
    }

    async fn play_playlist(&self, query: &str, shuffle: bool) -> AppResult<()> {
        let lists = self.playlists().await?;
        let p = super::automation::pick_playlist(&lists, query).ok_or_else(|| AppError::NotFound(format!("No playlist called {query}.")))?;
        let ctx = crate::audio::source::PlayContext { kind: crate::audio::source::ContextKind::Playlist, id: Some(p.id.clone()), track_ids: None };
        self.player.play_context(ctx, None, Some(shuffle)).await
    }

    async fn show(&self, route: serde_json::Value) -> AppResult<()> {
        self.action(serde_json::json!({ "kind": "open", "route": route }));
        Ok(())
    }

    fn notice(&self, message: &str) {
        if let Err(e) = self.app.emit("deck://action", serde_json::json!({ "kind": "notice", "message": message })) {
            tracing::warn!("emit deck://action failed: {e}");
        }
    }

    fn raise(&self) {
        crate::platform::show_window(&self.app);
    }
    async fn extras(&self, track: Option<&Track>) -> DeckExtras {
        let mut x = DeckExtras {
            miniplayer: crate::platform::miniplayer::is_open(&self.app),
            window: self.app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false)),
            ..Default::default()
        };
        if let Some(s) = self.app.try_state::<crate::settings::SettingsState>()
            && let Ok(s) = s.get().await
        {
            x.crossfade = s.crossfade != crate::audio::dsp::CrossfadeMode::Off;
        }
        x.device = self.remote().and_then(|r| r.state().session.map(|s| s.player_name));
        if let Some(t) = track {
            x.can_rate = self
                .app
                .try_state::<crate::backend::Backends>()
                .and_then(|b| b.for_id(&t.id).ok())
                .is_some_and(|s| s.kind() == crate::backend::BackendKind::Plex);
            if let Ok(l) = self.likes() {
                x.stars = l.stars(&t.id).await.unwrap_or(0);
                x.playlists = l.playlists_with(t.id.clone()).await.unwrap_or_default();
            }
        }
        x
    }

    async fn seek(&self, position_ms: u64) -> AppResult<()> {
        self.player.seek(position_ms).await
    }

    async fn set_shuffle(&self, on: bool) -> AppResult<()> {
        self.player.set_shuffle(on).await
    }

    async fn set_smart_shuffle(&self, on: bool) -> AppResult<()> {
        self.player.set_smart_shuffle(on).await
    }

    async fn set_repeat(&self, repeat: Repeat) -> AppResult<()> {
        self.player.set_repeat(repeat).await
    }

    async fn set_muted(&self, muted: bool) -> AppResult<()> {
        self.player.set_muted(muted).await
    }

    async fn set_settings(&self, patch: serde_json::Value) -> AppResult<()> {
        let patch: crate::settings::SettingsPatch = serde_json::from_value(patch).map_err(|e| AppError::Internal(e.to_string()))?;
        let settings = self.app.try_state::<crate::settings::SettingsState>().ok_or_else(|| AppError::Internal("settings not ready".into()))?;
        let downloads = self.app.try_state::<crate::downloads::Downloads>().ok_or_else(|| AppError::Internal("downloads not ready".into()))?;
        let mixes = self.app.try_state::<crate::mixes::MixService>().ok_or_else(|| AppError::Internal("mixes not ready".into()))?;
        crate::settings::apply_patch(&settings, &self.player, &downloads, &mixes, patch).await?;
        let s = settings.get().await?;
        if let Err(e) = self.app.emit("settings://changed", &s) {
            tracing::warn!("emit settings://changed failed: {e}");
        }
        Ok(())
    }

    async fn rate(&self, track_id: &str, stars: Option<u8>) -> AppResult<()> {
        self.likes()?.rate(track_id, stars).await
    }

    async fn playlist_add(&self, playlist_id: &str, track_id: &str) -> AppResult<()> {
        let p = self.app.try_state::<crate::playlists::Playlists>().ok_or_else(|| AppError::Internal("playlists not ready".into()))?;
        p.add(playlist_id.to_owned(), vec![track_id.to_owned()], crate::db::playlists::Place::End).await.map(|_| ())
    }

    async fn playlist_remove(&self, playlist_id: &str, track_id: &str) -> AppResult<()> {
        let p = self.app.try_state::<crate::playlists::Playlists>().ok_or_else(|| AppError::Internal("playlists not ready".into()))?;
        let id = playlist_id.to_owned();
        let page = self.db()?.read_async(move |c| crate::db::queries::playlist_page(c, &id)).await?;
        let items: Vec<String> =
            page.tracks.iter().zip(page.item_ids).filter(|(t, _)| t.id == track_id).map(|(_, item)| item).collect();
        if items.is_empty() {
            return Err(AppError::Invalid("The song isn't in that playlist.".into()));
        }
        p.remove(playlist_id.to_owned(), items).await.map(|_| ())
    }

    async fn play(&self, ctx: PlayContext, start: Option<String>, start_ms: u64, shuffle: bool) -> AppResult<()> {
        if start_ms > 0 {
            self.player.play_context_at(ctx, start, start_ms, true).await
        } else {
            self.player.play_context(ctx, start, Some(shuffle)).await
        }
    }

    async fn queue_context(&self, ctx: PlayContext) -> AppResult<()> {
        self.player.queue_context(ctx).await
    }

    async fn resolve_link(&self, link: &str) -> Option<DeckLink> {
        use crate::share::{DeepLinkStatus, ShareKind};
        let share = self.app.try_state::<crate::share::ShareService>()?;
        let o = share.resolve(link).await;
        if o.status != DeepLinkStatus::Open {
            return None;
        }
        let id = o.id?;
        Some(match o.kind? {
            ShareKind::Album => DeckLink::Album(id),
            ShareKind::Artist => DeckLink::Artist(id),
            ShareKind::Playlist => DeckLink::Playlist(id),
            ShareKind::Track => DeckLink::Track { id, album_id: o.album_id?, at_seconds: o.at_seconds },
        })
    }

    async fn search_tracks(&self, query: &str, limit: usize) -> AppResult<Vec<String>> {
        let db = self.db()?;
        let out = crate::db::search::search_with_fallback(&db, query, crate::db::types::SearchFilter::Songs, Some(limit as u32)).await?;
        Ok(out.songs.into_iter().take(limit).map(|t| t.id).collect())
    }

    async fn mix_of_kind(&self, kind: &str) -> AppResult<Option<String>> {
        let want = crate::mixes::MixKind::from_key(kind);
        let mixes = self.app.try_state::<crate::mixes::MixService>().ok_or_else(|| AppError::Internal("mixes not ready".into()))?;
        let shelves = mixes.shelves().await?;
        Ok(all_mixes(shelves).into_iter().find(|m| Some(m.kind) == want).map(|m| m.id))
    }

    async fn library(&self, kind: LibraryKind) -> AppResult<Vec<DeckItem>> {
        let item = |id: String, title: String, subtitle: Option<String>, thumb: Option<String>| DeckItem { id, title, subtitle, thumb, cover_key: None };
        Ok(match kind {
            LibraryKind::Playlists => self
                .playlists()
                .await?
                .into_iter()
                .map(|p| {
                    let n = format!("{} {}", p.track_count, if p.track_count == 1 { "song" } else { "songs" });
                    item(p.id, p.title, Some(n), p.thumb)
                })
                .collect(),
            LibraryKind::Albums | LibraryKind::Artists => {
                let db = self.db()?;
                let server = db.server_view();
                let view = db.tag_view();
                let max = super::LIBRARY_MAX as u32;
                if kind == LibraryKind::Albums {
                    let page = db
                        .read_async(move |c| {
                            crate::db::queries::albums_view_on(c, crate::db::types::AlbumSort::Recent, 0, max, None, view, &server)
                        })
                        .await?;
                    page.items.into_iter().map(|a| item(a.id, a.title, Some(a.artist_title), a.thumb)).collect()
                } else {
                    let page = db
                        .read_async(move |c| crate::db::queries::artists_on(c, crate::db::types::ArtistSort::Plays, 0, max, &server))
                        .await?;
                    page.items.into_iter().map(|a| item(a.id, a.title, None, a.thumb)).collect()
                }
            }
            LibraryKind::Mixes => {
                let mixes = self.app.try_state::<crate::mixes::MixService>().ok_or_else(|| AppError::Internal("mixes not ready".into()))?;
                let shelves = mixes.shelves().await?;
                all_mixes(shelves)
                    .into_iter()
                    .map(|m| item(m.id, m.title, Some(m.description), m.covers.into_iter().next().or(m.art)))
                    .collect()
            }
        })
    }

    async fn queue(&self) -> Vec<DeckQueued> {
        let Ok(s) = self.player.state().await else { return Vec::new() };
        s.queue
            .user_queue
            .into_iter()
            .chain(s.queue.next_up)
            .take(super::QUEUE_MAX)
            .map(|q| DeckQueued { item_id: q.item_id, track: q.track })
            .collect()
    }

    async fn queue_jump(&self, item_id: &str) -> AppResult<()> {
        self.player.queue_jump(item_id.to_owned()).await
    }

    async fn devices(&self) -> Vec<DeckDevice> {
        let Some(r) = self.remote() else { return Vec::new() };
        let on = r.state().session.map(|s| s.player_id);
        let mut out = vec![DeckDevice { id: LOCAL_DEVICE.into(), name: "This computer".into(), active: on.is_none(), needs_window: false }];
        out.extend(r.players().await.into_iter().map(|p| DeckDevice {
            active: on.as_deref() == Some(p.id.as_str()),
            needs_window: p.needs_password || p.unsupported.is_some(),
            id: p.id,
            name: p.name,
        }));
        out
    }

    async fn listen_on(&self, device: &str) -> AppResult<()> {
        let r = self.remote().ok_or_else(|| AppError::Internal("devices not ready".into()))?;
        if device == LOCAL_DEVICE { r.return_here().await } else { r.move_to(device).await }.map(|_| ())
    }

    async fn set_miniplayer(&self, open: bool) -> AppResult<()> {
        crate::platform::miniplayer::miniplayer_set(self.app.clone(), open).await.map(|_| ())
    }

    async fn set_window(&self, visible: bool) -> AppResult<()> {
        if visible {
            crate::platform::show_window(&self.app);
        } else if let Some(w) = self.app.get_webview_window("main") {
            w.hide().map_err(|e| AppError::Internal(e.to_string()))?;
        }
        Ok(())
    }

    fn window_action(&self, payload: serde_json::Value, bring_up: bool) {
        if bring_up {
            crate::platform::show_window(&self.app);
        }
        if let Err(e) = self.app.emit("deck://action", payload) {
            tracing::warn!("emit deck://action failed: {e}");
        }
    }

    async fn bookmark(&self, track: &Track, position_ms: u64) -> AppResult<()> {
        let b = self.app.try_state::<crate::bookmarks::Bookmarks>().ok_or_else(|| AppError::Internal("bookmarks not ready".into()))?;
        b.add(crate::bookmarks::NewBookmark {
            kind: crate::bookmarks::BookmarkKind::Track,
            target: track.id.clone(),
            page: None,
            title: track.title.clone(),
            subtitle: Some(crate::feed::artist_line(track)),
            thumb: track.thumb.clone(),
            position_ms: Some(position_ms),
        })
        .map(|_| ())
    }
}

/// Every mix on Home, made for you first.
fn all_mixes(s: crate::mixes::MixShelves) -> Vec<crate::mixes::MixSummary> {
    s.made_for_you.into_iter().chain(s.moods).chain(s.genres).collect()
}
