pub mod audio;
pub mod bridge;
pub mod db;
pub mod error;
pub mod model;
pub mod platform;
pub mod plex;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::Manager;

use crate::db::{ArtCache, DbHandle};
use crate::plex::PlexState;
use crate::plex::notifications::PlexEvent;

/// Re-sync the cache when the server connects and when Plex reports library
/// changes. Changes arrive in bursts during a scan, so they're coalesced.
fn watch_library(plex: PlexState, db: DbHandle) {
    let dirty = Arc::new(AtomicBool::new(false));
    let mut events = plex.subscribe();
    let flag = dirty.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(PlexEvent::Connected) | Ok(PlexEvent::LibraryUpdate { .. }) => {
                    flag.store(true, Ordering::Relaxed)
                }
                Ok(PlexEvent::Activity { event, kind, .. })
                    if event == "ended" && kind.starts_with("library.") =>
                {
                    flag.store(true, Ordering::Relaxed)
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    flag.store(true, Ordering::Relaxed)
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if dirty.load(Ordering::Relaxed) && plex.client().await.is_ok() {
                match db.start_sync(false) {
                    Ok(true) => dirty.store(false, Ordering::Relaxed),
                    Ok(false) => {} // a sync is running; try again next tick
                    Err(e) => tracing::warn!("sync start failed: {e}"),
                }
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,platter_lib=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("art", |ctx, req, responder| {
            let cache = ctx.app_handle().state::<ArtCache>().inner().clone();
            db::artcache::respond(cache, req.uri().to_string(), responder);
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let plex = PlexState::new(data_dir.clone())?;
            let db = DbHandle::open(data_dir.join("library.sqlite"))?;
            db.set_source(Arc::new(bridge::PlexLibrary::new(plex.clone())));
            db.set_events(Arc::new(app.handle().clone()));
            let art = ArtCache::open(
                cache_dir.join("art"),
                db::artcache::DEFAULT_CAP_BYTES,
                Arc::new(bridge::PlexArt(plex.clone())),
            )?;

            plex.start(app.handle().clone());
            watch_library(plex.clone(), db.clone());

            app.manage(plex);
            app.manage(db);
            app.manage(art);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            plex::commands::auth_status,
            plex::commands::auth_start,
            plex::commands::auth_cancel,
            plex::commands::auth_sign_out,
            plex::commands::servers_list,
            plex::commands::server_select,
            plex::commands::health,
            db::commands::sync_status,
            db::commands::sync_now,
            db::commands::home,
            db::commands::artists,
            db::commands::albums,
            db::commands::playlists,
            db::commands::artist_page,
            db::commands::album_page,
            db::commands::playlist_page,
            db::commands::search,
            db::commands::recent_searches,
            db::commands::recent_search_add,
            db::commands::recent_searches_clear,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
