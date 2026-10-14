pub mod audio;
pub mod bridge;
pub mod db;
pub mod error;
pub mod model;
pub mod monitor;
pub mod platform;
pub mod plex;
pub mod settings;
pub mod tracking;
pub mod updates;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::Manager;

use crate::audio::player::{PlayerConfig, PlayerHandle};
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
    // Two TLS crypto backends get compiled in (the updater pulls in ring), so
    // rustls can't pick one on its own. Use the same one as our HTTP client.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,platter_lib=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::Updates::default())
        .register_asynchronous_uri_scheme_protocol("art", |ctx, req, responder| {
            let cache = ctx.app_handle().state::<ArtCache>().inner().clone();
            db::artcache::respond(cache, req.uri().to_string(), responder);
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            monitor::crash::install(&data_dir);
            let audio_dir = cache_dir.join("audio");

            let plex = PlexState::new(data_dir.clone())?;
            let db = DbHandle::open(data_dir.join("library.sqlite"))?;
            db.set_source(Arc::new(bridge::PlexLibrary::new(plex.clone())));
            db.set_events(Arc::new(monitor::library::SyncHealth {
                inner: Arc::new(app.handle().clone()),
                plex: plex.clone(),
            }));
            db.set_remote_search(Arc::new(bridge::PlexSearch(plex.clone())));
            let art = ArtCache::open(
                cache_dir.join("art"),
                db::artcache::DEFAULT_CAP_BYTES,
                Arc::new(bridge::PlexArt(plex.clone())),
            )?;

            let player = PlayerHandle::spawn(
                tauri::async_runtime::handle().inner().clone(),
                Arc::new(bridge::PlexStreams(plex.clone())),
                Arc::new(bridge::CacheLookup(db.clone())),
                Box::new(audio::output::CpalOutput::new()),
                Arc::new(audio::events::TauriSink(app.handle().clone())),
                PlayerConfig {
                    persist_path: Some(data_dir.join("queue.json")),
                    cache_dir: Some(audio_dir.clone()),
                    shuffle_style: settings::load(&db).shuffle_style,
                    ..Default::default()
                },
            );
            let tracking_settings = tracking::TrackingSettings::default();
            let app_settings = settings::SettingsState::new(db.clone(), tracking_settings.clone());
            tracking::spawn(
                tracking::Tracker::new(plex.clone(), db.clone(), tracking_settings),
                player.subscribe_tracking(),
            );

            plex.start(app.handle().clone());
            watch_library(plex.clone(), db.clone());
            monitor::files::spawn(plex.clone(), db.clone(), player.clone());
            monitor::net::spawn(plex.clone());
            monitor::disk::spawn(
                plex.clone(),
                audio_dir.clone(),
                art.clone(),
                db::artcache::DEFAULT_CAP_BYTES,
            );
            app.manage(monitor::cached::AudioCacheDir(audio_dir));

            app.manage(plex);
            app.manage(db);
            app.manage(art);
            app.manage(player);
            app.manage(app_settings);
            updates::start(app.handle().clone());
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
            audio::commands::play_context,
            audio::commands::play_toggle,
            audio::commands::play_pause,
            audio::commands::play_resume,
            audio::commands::play_next,
            audio::commands::play_previous,
            audio::commands::play_seek,
            audio::commands::set_volume,
            audio::commands::set_muted,
            audio::commands::set_shuffle,
            audio::commands::set_context_shuffle,
            audio::commands::set_repeat,
            audio::commands::queue_add,
            audio::commands::queue_remove,
            audio::commands::queue_move,
            audio::commands::queue_clear,
            audio::commands::queue_jump,
            audio::commands::player_state,
            audio::commands::player_problems,
            settings::settings_get,
            settings::settings_set,
            monitor::cached::cached_tracks,
            updates::update_check,
            updates::update_install_on_restart,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let player = app.state::<PlayerHandle>().inner().clone();
                tauri::async_runtime::block_on(player.flush());
            }
        });
}
