//! Tauri side of Discord status (docs/IPC.md "Discord"): the commands, the
//! status event, and the glue to Plex, the art cache and sharing.

use std::sync::Arc;

use rusqlite::{OptionalExtension, params};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tokio::sync::broadcast;

use super::art::{AlbumMbid, ArtResolver, CoverBytes, ReqwestUpload};
use super::{Config, DiscordHandle, DiscordStatus, DiscordSwitches, Library, StatusSink, app_id};
use crate::artistinfo::http::InfoHttp;
use crate::artistinfo::service::KeyState;
use crate::audio::events::TrackingEvent;
use crate::db::release::is_unreleased;
use crate::db::{ArtCache, DbHandle};
use crate::error::AppResult;
use crate::plex::PlexState;
use crate::plex::keychain::KeyringStore;
use crate::share::{ShareKind, ShareService};

pub const EV_STATUS: &str = "discord://status";

/// The album's release id from Plex's album detail (`Guid` `mbid://…`).
pub struct PlexAlbumMbid(pub PlexState);

#[async_trait::async_trait]
impl AlbumMbid for PlexAlbumMbid {
    async fn release_mbid(&self, album_id: &str) -> AppResult<Option<String>> {
        Ok(self.0.client().await?.metadata(album_id).await?.mbid())
    }
}

/// Covers come from the art cache (which fetches from Plex), as bytes.
pub struct CachedCovers(pub ArtCache);

#[async_trait::async_trait]
impl CoverBytes for CachedCovers {
    async fn cover(&self, thumb: &str, px: u32) -> AppResult<(Vec<u8>, String)> {
        let img = self.0.get_or_fetch(thumb, px, px).await?;
        Ok((img.bytes.to_vec(), img.content_type.clone()))
    }
}

/// Unreleased checks from the library cache; links from sharing.
pub struct AppLibrary {
    pub db: DbHandle,
    pub share: ShareService,
}

#[async_trait::async_trait]
impl Library for AppLibrary {
    async fn unreleased(&self, album_id: &str) -> bool {
        let id = album_id.to_owned();
        let row = self
            .db
            .read_async(move |c| {
                Ok(c.query_row(
                    "SELECT al.format, al.subformats, at.version FROM albums al
                     LEFT JOIN album_tags at ON at.album_id = al.id WHERE al.id = ?1",
                    params![id],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()?)
            })
            .await
            .ok()
            .flatten();
        match row {
            Some((format, subs, version)) => {
                let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
                is_unreleased(format.as_deref(), &subs, version.as_deref())
            }
            // Not in the cache: treat as unreleased, the cautious choice.
            None => true,
        }
    }

    async fn song_link(&self, track_id: &str) -> Option<String> {
        match self.share.public_link(ShareKind::Track, track_id).await {
            Ok(u) => Some(u),
            Err(e) => {
                tracing::debug!("no song link for Discord: {e}");
                None
            }
        }
    }
}

impl<R: Runtime> StatusSink for AppHandle<R> {
    fn changed(&self, status: &DiscordStatus) {
        let _ = self.emit(EV_STATUS, status);
    }
}

/// Build the resolver and start the service (or a stub when this build has
/// no Discord app id). The handle is managed for the commands.
#[allow(clippy::too_many_arguments)]
pub fn start<R: Runtime>(
    app: &AppHandle<R>,
    plex: PlexState,
    db: DbHandle,
    http: Arc<dyn InfoHttp>,
    art: ArtCache,
    share: ShareService,
    switches: &DiscordSwitches,
    events: broadcast::Receiver<TrackingEvent>,
) -> AppResult<()> {
    let resolver = Arc::new(ArtResolver::new(
        db.clone(),
        http,
        Arc::new(ReqwestUpload::new()?),
        Arc::new(PlexAlbumMbid(plex)),
        Arc::new(CachedCovers(art)),
        Arc::new(KeyringStore::new()),
    ));
    let sink: Arc<dyn StatusSink> = Arc::new(app.clone());
    let handle = match app_id() {
        Some(id) => {
            let deps = super::Deps {
                connector: Arc::new(super::ipc::LocalDiscord),
                art: resolver.clone(),
                library: Arc::new(AppLibrary { db, share }),
                status: sink,
            };
            // Enter Tauri's runtime so the service's task lands there.
            let _guard = tauri::async_runtime::handle().inner().enter();
            super::spawn(Config::new(id), deps, events, switches.subscribe()).0
        }
        None => super::unavailable(resolver.clone(), sink),
    };
    app.manage(handle);
    app.manage(resolver);
    Ok(())
}

#[tauri::command]
pub async fn discord_status(discord: State<'_, DiscordHandle>) -> AppResult<DiscordStatus> {
    let d = discord.inner().clone();
    // The first read of the key touches the keychain.
    tokio::task::spawn_blocking(move || d.status())
        .await
        .map_err(|e| crate::error::AppError::Internal(e.to_string()))
}

/// Save the Imgur client id in the keychain (null removes it). It's checked
/// with one request that uploads nothing.
#[tauri::command]
pub async fn discord_imgur_key_set(
    discord: State<'_, DiscordHandle>,
    resolver: State<'_, Arc<ArtResolver>>,
    key: Option<String>,
) -> AppResult<KeyState> {
    let st = resolver.set_key(key).await?;
    discord.key_changed();
    Ok(st)
}
