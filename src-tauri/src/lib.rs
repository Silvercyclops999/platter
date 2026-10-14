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
use std::time::Duration;

use tauri::Manager;

use crate::audio::player::{PlayerConfig, PlayerHandle};
use crate::db::{ArtCache, DbHandle};
use crate::plex::PlexState;
use crate::plex::notifications::PlexEvent;

/// Re-sync the cache when the server connects and when Plex reports library
/// changes. Changes arrive in bursts during a scan, so they're coalesced:
/// a sync starts once no new change has arrived for `QUIET`.
fn watch_library(plex: PlexState, db: DbHandle) {
    const QUIET: Duration = Duration::from_secs(5);
    // When the last relevant change arrived; None = nothing pending.
    let pending: Arc<parking_lot::Mutex<Option<std::time::Instant>>> = Arc::default();
    let mut events = plex.subscribe();
    let mark = pending.clone();
    let db_events = db.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let ev = match events.recv().await {
                Ok(ev) => Some(ev),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let music = db_events
                .read(db::store::sync_state)
                .ok()
                .flatten()
                .map(|s| s.section_id);
            let relevant = match &ev {
                Some(ev) => marks_library_dirty(ev, music.as_deref()),
                None => true, // missed events; assume something changed
            };
            if relevant {
                match &ev {
                    Some(PlexEvent::LibraryUpdate { section_id, item_type, state, .. }) => {
                        tracing::debug!(?section_id, ?item_type, ?state, "library change")
                    }
                    Some(PlexEvent::Activity { kind, .. }) => tracing::debug!(kind, "library activity ended"),
                    Some(PlexEvent::Connected) => tracing::debug!("server connected; resync"),
                    _ => tracing::debug!("missed server events; resync"),
                }
                *mark.lock() = Some(std::time::Instant::now());
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let due = matches!(*pending.lock(), Some(at) if at.elapsed() >= QUIET);
            if due && plex.client().await.is_ok() {
                match db.start_sync(false) {
                    Ok(true) => *pending.lock() = None,
                    Ok(false) => {} // a sync is running; try again next tick
                    Err(e) => tracing::warn!("sync start failed: {e}"),
                }
            }
        }
    });
}

/// Whether a Plex notification means the music cache may be stale. Changes
/// in other libraries on the server (movies, TV) are ignored once the music
/// section is known.
fn marks_library_dirty(ev: &PlexEvent, music_section: Option<&str>) -> bool {
    match ev {
        PlexEvent::Connected => true,
        PlexEvent::LibraryUpdate { section_id, .. } => match (section_id.as_deref(), music_section) {
            (Some(s), Some(m)) => s == m,
            _ => true,
        },
        PlexEvent::Activity { event, kind, .. } => event == "ended" && kind.starts_with("library."),
        _ => false,
    }
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
            let initial = settings::load(&db);
            let art = ArtCache::open(
                cache_dir.join("art"),
                initial.art_cache_bytes(),
                Arc::new(bridge::PlexArt(plex.clone())),
            )?;
            let live = settings::Live {
                art: Some(art.clone()),
                ..Default::default()
            };

            let player = PlayerHandle::spawn(
                tauri::async_runtime::handle().inner().clone(),
                Arc::new(bridge::PlexStreams(plex.clone(), live.streams.clone())),
                Arc::new(bridge::CacheLookup(db.clone())),
                Box::new(audio::output::CpalOutput::new()),
                Arc::new(audio::events::TauriSink(app.handle().clone())),
                PlayerConfig {
                    persist_path: Some(data_dir.join("queue.json")),
                    cache_dir: Some(audio_dir.clone()),
                    shuffle_style: initial.shuffle_style,
                    resume_contexts: initial.resume_contexts,
                    precache_count: initial.precache_count as usize,
                    ..Default::default()
                },
            );
            let app_settings = settings::SettingsState::new(db.clone(), live.clone());
            tracking::spawn(
                tracking::Tracker::new(plex.clone(), db.clone(), live.tracking.clone()),
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
                live.art_cap.clone(),
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
            db::commands::artist_names,
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
            settings::art_cache_info,
            settings::art_cache_clear,
            settings::app_info,
            settings::open_project_page,
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

#[cfg(test)]
mod watch_tests {
    use super::*;

    fn update(section: Option<&str>) -> PlexEvent {
        PlexEvent::LibraryUpdate {
            section_id: section.map(str::to_owned),
            item_id: Some("1".into()),
            item_type: Some(1),
            state: Some(5),
        }
    }

    #[test]
    fn other_libraries_dont_trigger_a_music_sync() {
        assert!(!marks_library_dirty(&update(Some("2")), Some("7")));
        assert!(marks_library_dirty(&update(Some("7")), Some("7")));
    }

    #[test]
    fn unknown_sections_still_count() {
        assert!(marks_library_dirty(&update(None), Some("7")));
        assert!(marks_library_dirty(&update(Some("2")), None));
        assert!(marks_library_dirty(&PlexEvent::Connected, Some("7")));
    }
}
