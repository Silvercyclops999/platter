pub mod archive;
pub mod artistinfo;
pub mod audio;
pub mod backend;
pub mod bookmarks;
pub mod bridge;
pub mod db;
pub mod discover;
pub mod doctor;
pub mod downloads;
pub mod error;
pub mod feed;
pub mod hidden;
pub mod identity;
pub mod import;
pub mod jellyfin;
pub mod libraries;
pub mod lidarr;
pub mod likes;
pub mod lyrics;
pub mod mixes;
pub mod model;
pub mod monitor;
pub mod notify;
pub mod platform;
pub mod playlists;
pub mod plex;
pub mod plexedit;
pub mod radar;
pub mod remote;
pub mod scrobble;
pub mod settings;
pub mod share;
pub mod sid;
pub mod smart;
pub mod social;
pub mod soundid;
pub mod subsonic;
pub mod stats;
pub mod sync;
pub mod tags;
pub mod themes;
pub mod tracking;
pub mod updates;
pub mod web;

#[cfg(test)]
mod plex_requests_tests;

use std::sync::Arc;
use std::time::Duration;

use tauri::{Listener, Manager};

use crate::audio::player::{PlayerConfig, PlayerHandle};
use crate::db::{ArtCache, DbHandle};
use crate::plex::PlexState;
use crate::plex::notifications::PlexEvent;

/// Keep the cache current from the server's notification socket:
///
/// - Items Plex finished adding or changing, or deleted (`timeline`
///   entries), go straight into the cache in batches gathered for
///   [`db::live::BATCH_WINDOW`] (`db::live`). Pages hear `library://changed`,
///   and new albums go to What's New. A batch too big to fetch item by item
///   (a large scan) runs a sync instead.
/// - The socket connecting (after being down), missed events, and a library
///   scan ending run the usual incremental sync once things are quiet for
///   `QUIET`.
/// - While the socket is down, an incremental sync runs every
///   `FALLBACK_EVERY` instead, since no events arrive.
const QUIET: Duration = Duration::from_secs(5);
const FALLBACK_EVERY: Duration = Duration::from_secs(15 * 60);

fn watch_library(app: tauri::AppHandle, plex: PlexState, db: DbHandle) {
    // When the last relevant change arrived; None = nothing pending.
    let pending: Arc<parking_lot::Mutex<Option<std::time::Instant>>> = Arc::default();
    // Wakes the sync timer when something changes, so it can sleep meanwhile.
    let wake = Arc::new(tokio::sync::Notify::new());
    let wake_events = wake.clone();
    let socket_up = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (live_tx, live_rx) = tokio::sync::mpsc::unbounded_channel::<db::live::LiveChange>();
    let mut events = plex.subscribe();
    let mark = pending.clone();
    let db_events = db.clone();
    let up = socket_up.clone();
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
            match &ev {
                Some(PlexEvent::Connected) => up.store(true, std::sync::atomic::Ordering::Release),
                Some(PlexEvent::Disconnected) => {
                    up.store(false, std::sync::atomic::Ordering::Release);
                    wake_events.notify_one();
                }
                Some(PlexEvent::LibraryUpdate { section_id, item_id, item_type, state }) => {
                    if let Some(c) = db::live::change_from_timeline(
                        section_id.as_deref(),
                        item_id.as_deref(),
                        *item_type,
                        *state,
                        music.as_deref(),
                    ) {
                        let _ = live_tx.send(c);
                    }
                    continue;
                }
                _ => {}
            }
            let relevant = match &ev {
                Some(ev) => marks_library_dirty(ev, music.as_deref()),
                None => true, // missed events; assume something changed
            };
            if relevant {
                match &ev {
                    Some(PlexEvent::Activity { kind, .. }) => tracing::debug!(kind, "library activity ended"),
                    Some(PlexEvent::Connected) => tracing::debug!("server connected; resync"),
                    _ => tracing::debug!("missed server events; resync"),
                }
                *mark.lock() = Some(std::time::Instant::now());
                wake_events.notify_one();
            }
        }
    });
    spawn_live_batches(app, db.clone(), live_rx, pending.clone(), wake.clone());
    tauri::async_runtime::spawn(async move {
        let mut last_fallback = std::time::Instant::now();
        loop {
            let down = !socket_up.load(std::sync::atomic::Ordering::Acquire);
            if down && last_fallback.elapsed() >= FALLBACK_EVERY {
                last_fallback = std::time::Instant::now();
                tracing::debug!("notification socket down; periodic sync");
                *pending.lock() = std::time::Instant::now().checked_sub(QUIET);
            }
            let due = matches!(*pending.lock(), Some(at) if at.elapsed() >= QUIET);
            let mut blocked = false;
            if due {
                blocked = true;
                if plex.client().await.is_ok() {
                    match db.start_sync(false) {
                        Ok(true) => {
                            *pending.lock() = None;
                            blocked = false;
                        }
                        Ok(false) => {} // a sync is running; try again shortly
                        Err(e) => tracing::warn!("sync start failed: {e}"),
                    }
                }
            }
            let age = pending.lock().map(|at| at.elapsed());
            // Nothing pending and the socket up: sleep until an event arrives.
            match library_wait(age, down, last_fallback.elapsed(), blocked) {
                Some(wait) => {
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = wake.notified() => {}
                    }
                }
                None => wake.notified().await,
            }
        }
    });
}

/// How long the library watcher sleeps: until a pending change has been
/// quiet long enough, until the next fallback sync while the socket is down,
/// a short retry when a due sync couldn't start, or (None) until woken.
fn library_wait(pending_age: Option<Duration>, socket_down: bool, since_fallback: Duration, blocked: bool) -> Option<Duration> {
    const RETRY: Duration = Duration::from_secs(5);
    if blocked {
        return Some(RETRY);
    }
    if let Some(age) = pending_age {
        return Some(QUIET.saturating_sub(age).max(Duration::from_millis(100)));
    }
    socket_down.then(|| FALLBACK_EVERY.saturating_sub(since_fallback).max(Duration::from_secs(1)))
}

/// Gather live changes for a moment, then write them into the cache. While
/// a sync runs, the batch waits (the sync is rewriting the same rows).
fn spawn_live_batches(
    app: tauri::AppHandle,
    db: DbHandle,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<db::live::LiveChange>,
    pending_sync: Arc<parking_lot::Mutex<Option<std::time::Instant>>>,
    wake: Arc<tokio::sync::Notify>,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(first) = rx.recv().await {
            let mut batch = db::live::Batch::default();
            batch.add(first);
            let until = tokio::time::Instant::now() + db::live::BATCH_WINDOW;
            loop {
                tokio::select! {
                    c = rx.recv() => match c {
                        Some(c) => batch.add(c),
                        None => break,
                    },
                    _ = tokio::time::sleep_until(until) => break,
                }
            }
            while db.sync_running() {
                tokio::time::sleep(Duration::from_millis(500)).await;
                while let Ok(c) = rx.try_recv() {
                    batch.add(c);
                }
            }
            let Some(source) = db.source() else { continue };
            match db::live::apply(&db, source.as_ref(), batch).await {
                Ok(db::live::LiveOutcome::Applied { updated, removed, new_albums }) => {
                    tracing::debug!(updated, removed, new = new_albums.len(), "live library update");
                    if !new_albums.is_empty()
                        && let Some(social) = app.try_state::<social::Social>()
                    {
                        let social = social.inner().clone();
                        let _ = tokio::task::spawn_blocking(move || social.library_changed()).await;
                    }
                }
                Ok(db::live::LiveOutcome::Nothing) => {}
                Ok(db::live::LiveOutcome::NeedsSync) => {
                    *pending_sync.lock() = Some(std::time::Instant::now());
                    wake.notify_one();
                }
                Err(e) => {
                    tracing::debug!("live library update failed, syncing instead: {e}");
                    *pending_sync.lock() = Some(std::time::Instant::now());
                    wake.notify_one();
                }
            }
        }
    });
}

/// Whether a Plex notification means the cache needs a sync. Item changes
/// (`timeline`) are applied live instead; changes in other libraries on the
/// server (movies, TV) are ignored once the music section is known.
fn marks_library_dirty(ev: &PlexEvent, music_section: Option<&str>) -> bool {
    match ev {
        PlexEvent::Connected => true,
        PlexEvent::Activity { event, kind, section_id, .. } => {
            event == "ended"
                && kind.starts_with("library.")
                && match (section_id.as_deref(), music_section) {
                    (Some(s), Some(m)) => s == m,
                    _ => true,
                }
        }
        _ => false,
    }
}

/// Release status marks and artist page sections another device changed:
/// load the marks again, work statuses out again and tell the pages.
fn follow_archive(app: tauri::AppHandle, store: &sync::SyncStore, db: DbHandle) {
    use tauri::Emitter;
    let mut rx = store.subscribe();
    let store = store.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let c = match rx.recv().await {
                Ok(c) if c.remote => c,
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => sync::Changed {
                    namespaces: sync::store::NAMESPACES.iter().map(|s| s.to_string()).collect(),
                    remote: true,
                },
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let has = |ns: &str| c.namespaces.iter().any(|n| n == ns);
            if has(sync::store::RELEASE_MARKS.name()) {
                let s = store.clone();
                if let Err(e) = s.run(archive::marks::load).await {
                    tracing::warn!("release marks not loaded: {e}");
                }
                let res = db
                    .write_async(|c| {
                        archive::in_tx(c, |c| {
                            crate::identity::index::rebuild(c)?;
                            archive::refresh_all(c)
                        })
                    })
                    .await;
                if let Err(e) = res {
                    tracing::warn!("archive refresh failed: {e}");
                }
            }
            if has(sync::store::RELEASE_MARKS.name()) || has(sync::store::ALBUM_SECTIONS.name()) {
                let _ = app.emit("archive://changed", serde_json::json!({}));
            }
        }
    });
}

/// Hidden items, bookmarks and Platter smart playlists another device
/// changed (through the sync file): read them again and tell the UI.
fn follow_sync_store(store: &sync::SyncStore, hidden: hidden::Hidden, bookmarks: bookmarks::Bookmarks, smart: smart::Smart) {
    let mut rx = store.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let c = match rx.recv().await {
                Ok(c) if c.remote => c,
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => sync::Changed {
                    namespaces: sync::store::NAMESPACES.iter().map(|s| s.to_string()).collect(),
                    remote: true,
                },
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let has = |ns: &str| c.namespaces.iter().any(|n| n == ns);
            let (h, b, s) = (hidden.clone(), bookmarks.clone(), smart.clone());
            let (hh, hb, hs) = (has(sync::store::HIDDEN.name()), has(sync::store::BOOKMARKS.name()), has(sync::store::SMART.name()));
            let _ = tokio::task::spawn_blocking(move || {
                if hh {
                    h.reload();
                }
                if hb {
                    let _ = b.changed();
                }
                if hs {
                    s.changed();
                }
            })
            .await;
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Two TLS crypto backends get compiled in (the updater pulls in ring), so
    // rustls can't pick one on its own. Use the same one as our HTTP client.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    {
        use tracing_subscriber::Layer;
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        let console = tracing_subscriber::fmt::layer().with_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,platter_lib=debug".into()),
        );
        // The log file (Help, Show Logs) keeps info and above.
        let file = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(platform::logs::LogFile)
            .with_filter(tracing_subscriber::filter::LevelFilter::INFO);
        tracing_subscriber::registry().with(console).with(file).init();
    }

    tauri::Builder::default()
        // First, so a second launch (a platter:// link on Windows) hands its
        // link to the copy already running and quits before doing anything.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| share::commands::focus_main(app)))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // Launch at login (a LaunchAgent on macOS, the default).
        .plugin(tauri_plugin_autostart::Builder::new().arg(platform::AUTOSTART_ARG).build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(updates::Updates::default())
        .manage(platform::miniplayer::Miniplayer::default())
        .register_asynchronous_uri_scheme_protocol("art", |ctx, req, responder| {
            let cache = ctx.app_handle().state::<ArtCache>().inner().clone();
            db::artcache::respond(cache, req.uri().to_string(), responder);
        })
        .setup(|app| {
            let root_dir = app.path().app_data_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            std::fs::create_dir_all(&root_dir)?;
            monitor::crash::install(&root_dir);
            platform::logs::open(app.handle());
            // The Plex Home user this run belongs to: their own folder and
            // keychain entries (the user who signed in keeps the top folder).
            let (profile, data_dir) = plex::home::startup(&root_dir);
            plex::keychain::set_profile(profile.clone());
            std::fs::create_dir_all(&data_dir)?;
            plex::keychain::init(&root_dir);
            let audio_dir = cache_dir.join("audio");

            let plex = PlexState::new(data_dir.clone())?;
            app.manage(plex::home::Home::new(
                root_dir.clone(),
                profile,
                plex.plex_tv().clone(),
                Arc::new(plex::home::KeyringProfiles),
            ));
            let db = DbHandle::open(data_dir.join("library.sqlite"))?;
            // A cache from before server-qualified ids: rewrite it now for
            // the server it came from (saved with the server choice), before
            // the queue, downloads or a sync read it. Otherwise the first
            // sync does it.
            if db.ids_pending()
                && let Some(m) = plex::discovery::DiscoveryCache::load(&data_dir).selected
            {
                let tag = sid::tag(&m);
                match db.qualify_legacy(&tag) {
                    Ok(_) => {
                        let n = audio::cache::qualify_files(&audio_dir, &tag);
                        if n > 0 {
                            tracing::info!(n, "cached songs now carry their server");
                        }
                    }
                    Err(e) => tracing::warn!("cache ids not rewritten yet: {e}"),
                }
            }
            // The music servers: the Plex account. Calls about an item go
            // to the server its id names.
            let stream_prefs: Arc<bridge::StreamPrefs> = Arc::default();
            let backends = backend::Backends::with_plex(backend::plex::PlexServer::new(
                plex.clone(),
                stream_prefs.clone(),
                Some(db.clone()),
            ));
            backends.set_event_sink(Arc::new(plex::state::TauriSink(app.handle().clone())));
            backends.keep_records_in(data_dir.clone());
            {
                // Sign-in and server changes: record the server, and the UI
                // asks what it can do again.
                let b = backends.clone();
                app.listen_any(plex::types::EVENT_AUTH_CHANGED, move |_| {
                    let b = b.clone();
                    tauri::async_runtime::spawn(async move {
                        b.remember_plex().await;
                        b.changed();
                    });
                });
            }
            // The main library comes from whichever server has it now.
            db.set_source(Arc::new(backend::routed::RoutedLibrary(backends.clone())));
            db.set_events(Arc::new(monitor::library::SyncHealth {
                inner: Arc::new(app.handle().clone()),
                plex: plex.clone(),
            }));
            db.set_remote_search(Arc::new(backend::routed::RoutedSearch(backends.clone())));
            let initial = settings::load(&db);
            // Artist info sources share one HTTP client with the art cache,
            // which also serves their images.
            // Go offline stops these as well.
            let info_http: Arc<dyn artistinfo::http::InfoHttp> = Arc::new(artistinfo::http::Switched {
                inner: Arc::new(artistinfo::http::ReqwestHttp::new()?),
                off: plex.offline_switch(),
            });
            let share_http = info_http.clone();
            let lyrics_http = info_http.clone();
            let discord_http = info_http.clone();
            // Downloads: the queue starts once the player's transfer gate exists.
            let downloads = downloads::Downloads::new(downloads::Deps {
                db: db.clone(),
                dir: initial.download_dir(&data_dir.join("downloads")),
                source: Arc::new(backend::routed::RoutedDownloads(backends.clone())),
                free_space: downloads::os_free_space(),
                art: Some(Arc::new(backend::routed::RoutedArt(backends.clone()))),
            });
            downloads.set_events(Arc::new(app.handle().clone()));
            downloads.set_prefs(initial.download_prefs());
            let local_files = audio::source::LocalFiles(Arc::new(downloads.clone()));
            let art = ArtCache::open(
                cache_dir.join("art"),
                initial.art_cache_bytes(),
                Arc::new(downloads::art::WithDownloads {
                    inner: Arc::new(artistinfo::art::RoutedArt {
                        plex: Arc::new(backend::routed::RoutedArt(backends.clone())),
                        http: info_http.clone(),
                    }),
                    downloads: downloads.clone(),
                }),
            )?;
            // Jellyfin servers saved earlier sign in with their tokens.
            let jellyfin = jellyfin::Jellyfin::new(jellyfin::manager::Deps {
                backends: backends.clone(),
                db: Some(db.clone()),
                dir: data_dir.clone(),
                store: Arc::new(plex::keychain::KeyringStore::new()),
                http: jellyfin::client::http(),
                device_id: plex.identity().client_id.clone(),
                prefs: stream_prefs.clone(),
                offline: plex.offline_switch(),
                live: true,
            });
            jellyfin.restore();
            app.manage(jellyfin);
            let live = settings::Live {
                art: Some(art.clone()),
                plex: Some(plex.clone()),
                backends: Some(backends.clone()),
                streams: stream_prefs.clone(),
                ..Default::default()
            };
            *live.downloads.lock() = Some(downloads.clone());
            // Navidrome and other Subsonic servers saved earlier sign in with
            // their keys, and are checked for changes every few minutes.
            live.subsonic_check_minutes.store(initial.subsonic_check_minutes, std::sync::atomic::Ordering::Relaxed);
            live.save_queue_on_server.store(initial.save_queue_on_server, std::sync::atomic::Ordering::Relaxed);
            let subsonic = subsonic::Subsonic::new(subsonic::manager::Deps {
                backends: backends.clone(),
                db: Some(db.clone()),
                store: Arc::new(plex::keychain::KeyringStore::new()),
                http: subsonic::client::http(),
                prefs: stream_prefs.clone(),
                offline: plex.offline_switch(),
                poll_minutes: live.subsonic_check_minutes.clone(),
                live: true,
            });
            subsonic.restore();
            app.manage(subsonic);
            // Mixes, radio and the daylist, made here from the cache.
            let mix_service = mixes::MixService::new(db.clone());
            mix_service.set_options(initial.mix_options());
            // App-only state that can travel in the sync file (sync::).
            let sync_store = sync::SyncStore::new(db.clone(), plex.identity().client_id.clone());
            // Release status marks feed the unreleased rule from the start.
            if let Err(e) = archive::marks::load(&sync_store) {
                tracing::warn!("release marks not loaded: {e}");
            }
            follow_archive(app.handle().clone(), &sync_store, db.clone());
            // Hidden songs and artists: mixes, radio, autoplay and shuffle skip them.
            let hidden = hidden::Hidden::new(
                Arc::new(hidden::SyncHidden(sync_store.clone())),
                hidden::HiddenIds::default(),
                Arc::new(plex::state::TauriSink(app.handle().clone())),
                Arc::new(db::queries::now_s),
            );
            mix_service.set_hidden_ids(hidden.ids());
            mix_service.set_events(Arc::new(app.handle().clone()));
            *live.mixes.lock() = Some(mix_service.clone());
            let artist_info = artistinfo::ArtistInfoService::new(
                db.clone(),
                info_http,
                Arc::new(plex::keychain::KeyringStore::new()),
                live.artist_info.clone(),
            );
            artist_info.set_mbid_lookup(Arc::new(backend::routed::RoutedMbids(backends.clone())));
            artist_info.set_events(Arc::new(app.handle().clone()));
            // Popular lists for the artists you play, ready before their pages open.
            tauri::async_runtime::spawn(artist_info.clone().watch_prefetch());
            // Albums Plex couldn't match: ask MusicBrainz whether Plex just missed them.
            tauri::async_runtime::spawn(artistinfo::releasecheck::watch(artist_info.clone()));
            // Launched at login: start hidden to the tray or minimised.
            let args: Vec<String> = std::env::args().collect();
            platform::apply_start_mode(app.handle(), platform::start_mode(&args, &initial.platform()));
            let os = platform::Platform::new();

            let net_route = audio::netsink::NetRoute::default();
            let player = PlayerHandle::spawn(
                tauri::async_runtime::handle().inner().clone(),
                Arc::new(backend::routed::RoutedStreams(backends.clone())),
                Arc::new(bridge::CacheLookup(
                    db.clone(),
                    Some(backends.clone()),
                    bridge::OfflinePlay::new(audio_dir.clone()).with_downloads(local_files.clone()),
                    Some(mix_service.clone()),
                )),
                {
                    use audio::output::AudioOutput;
                    let mut out = audio::output::CpalOutput::new();
                    out.select(initial.output_device.clone());
                    out.set_exclusive_devices(audio::hifi::bit_perfect_ids(&initial));
                    // AirPlay plays through a network output behind this one.
                    Box::new(audio::netsink::RoutedOutput::new(Box::new(out), net_route.clone()))
                },
                os.tap(Arc::new(audio::events::TauriSink(app.handle().clone()))),
                PlayerConfig {
                    persist_path: Some(data_dir.join("queue.json")),
                    cache_dir: Some(audio_dir.clone()),
                    shuffle_style: initial.shuffle_style,
                    resume_contexts: initial.resume_contexts,
                    precache_count: initial.precache_count as usize,
                    dsp: initial.dsp(),
                    local_files: Some(local_files.clone()),
                    autoplay: initial.queue_end == settings::QueueEnd::KeepPlaying,
                    smart_shuffle: initial.smart_shuffle,
                    ..Default::default()
                },
            );
            downloads.attach(player.transfer_gate(), Some(player.transfer_host()), audio::http::http_client());
            tauri::async_runtime::spawn(downloads.clone().run());
            // Downloaded items follow library changes straight away.
            for ev in downloads::FOLLOW_EVENTS {
                let d = downloads.clone();
                app.listen_any(ev, move |_| d.poke());
            }
            if initial.download_liked {
                // Keep Liked Songs downloaded (turned on before this item existed).
                let d = downloads.clone();
                tauri::async_runtime::spawn(async move {
                    let key = downloads::ItemKey::new(downloads::ItemKind::Liked, "");
                    if !d.is_on(key.clone()).await.unwrap_or(true)
                        && let Err(e) = d.add(key, "Liked Songs".into(), None).await
                    {
                        tracing::warn!("Liked Songs download: {e}");
                    }
                });
            }
            // File tags (explicit, versions, editions), read behind playback.
            let tag_scanner = tags::scan::TagScanner::new(
                db.clone(),
                Arc::new(backend::routed::RoutedParts(backends.clone())),
                tags::fetch::Reader::new(
                    audio::http::http_client(),
                    player.transfer_gate(),
                    Some(player.transfer_host()),
                ),
                Some(Arc::new(app.handle().clone())),
            );
            *live.tag_scanner.lock() = Some(tag_scanner.clone());
            let app_settings = settings::SettingsState::new(db.clone(), live.clone());
            // How each included server is reached (choosing between copies).
            libraries::spawn_ranks(plex.clone(), db.clone());
            tauri::async_runtime::spawn({
                let s = tag_scanner.clone();
                async move { s.spawn() }
            });
            app.manage(tag_scanner);
            app.manage(audio::autoeq::Fetcher::new(cache_dir.join("headphone-profiles")));
            tauri::async_runtime::spawn(audio::commands::follow_output(
                app.handle().clone(),
                player.clone(),
                app_settings.clone(),
            ));
            os.start(app.handle(), player.clone(), art.clone(), cache_dir.join("now-playing"), live.platform.clone());
            tracking::spawn(
                tracking::Tracker::new(backends.clone(), db.clone(), live.tracking.clone()),
                player.subscribe_tracking(),
            );
            // The queue kept on servers that hold one, for other apps to pick up.
            backend::resume::spawn_saver(backends.clone(), player.clone(), player.subscribe_tracking(), live.save_queue_on_server.clone());
            // Scrobbling to Last.fm and ListenBrainz, and the AI provider.
            let web_out: Arc<dyn web::Web> = Arc::new(web::Switched { inner: Arc::new(web::ReqwestWeb::new()?), off: plex.offline_switch() });
            let keychain: Arc<dyn plex::keychain::TokenStore> = Arc::new(plex::keychain::KeyringStore::new());
            {
                let lastfm = Arc::new(scrobble::lastfm::Lastfm::new(web_out.clone(), keychain.clone()));
                let listenbrainz = Arc::new(scrobble::listenbrainz::ListenBrainz::new(web_out.clone(), keychain.clone()));
                let scrobbler = scrobble::Scrobbler::new(scrobble::ScrobbleDeps {
                    db: db.clone(),
                    services: vec![lastfm.clone(), listenbrainz.clone()],
                    switches: live.scrobble.clone(),
                    unreleased: Arc::new(scrobble::CacheUnreleased(db.clone())),
                    notice: Arc::new(scrobble::commands::ToastNotice(plex.clone())),
                    events: Arc::new(plex::state::TauriSink(app.handle().clone())),
                    clock: Arc::new(db::queries::now_s),
                });
                scrobble::spawn(scrobbler.clone(), player.subscribe_tracking());
                app.manage(scrobble::commands::ScrobbleState::new(scrobbler, lastfm, listenbrainz, db.clone()));
            }
            // Lidarr: the setup check, requests and their background retry.
            let lidarr_service = lidarr::Lidarr::new(lidarr::LidarrDeps {
                web: web_out.clone(),
                keys: keychain.clone(),
                db: db.clone(),
                plex: Arc::new(backend::routed::RoutedFilePaths(backends.clone())),
                clock: Arc::new(db::queries::now_s),
                mb: Arc::new(lidarr::mbpick::MbCached {
                    http: artist_info.http(),
                    limiter: artist_info.musicbrainz_limiter(),
                    db: db.clone(),
                    clock: Arc::new(db::queries::now_s),
                    ttl: lidarr::mbpick::CACHE_TTL,
                }),
                artist_mbid: Arc::new(lidarr::InfoMbid(artist_info.clone())),
                album_wait: std::time::Duration::from_secs(2),
            });
            tauri::async_runtime::spawn(lidarr::request::watch(lidarr_service.clone()));
            let radar_lidarr = lidarr_service.clone();
            let discover_lidarr = lidarr_service.clone();
            app.manage(lidarr_service);
            let smart_service = smart::Smart::new(smart::SmartDeps {
                db: db.clone(),
                store: Arc::new(smart::SyncSmart { store: sync_store.clone(), db: db.clone() }),
                remote: Arc::new(backend::routed::RoutedSmart(backends.clone())),
                ai: Arc::new(smart::prompt::Anthropic::new(web_out.clone(), keychain.clone())),
                events: Arc::new(plex::state::TauriSink(app.handle().clone())),
                clock: Arc::new(db::queries::now_s),
            });
            let bookmarks_service = bookmarks::Bookmarks::new(
                Arc::new(bookmarks::SyncBookmarks(sync_store.clone())),
                Arc::new(plex::state::TauriSink(app.handle().clone())),
                Arc::new(db::queries::now_s),
            );
            follow_sync_store(&sync_store, hidden.clone(), bookmarks_service.clone(), smart_service.clone());
            app.manage(hidden);
            app.manage(smart_service);
            app.manage(bookmarks_service);
            notify::spawn(
                app.handle().clone(),
                player.subscribe_tracking(),
                notify::Switches {
                    enabled: live.notify_track_change.clone(),
                    private_session: live.tracking.private_session.clone(),
                },
                art.clone(),
                cache_dir.clone(),
            );

            let playlists = playlists::Playlists::new(
                db.clone(),
                Arc::new(backend::routed::RoutedPlaylists(backends.clone())),
                Arc::new(playlists::AppEvents { plex: plex.clone(), db: db.clone() }),
            );
            playlists::spawn(playlists.clone());
            app.manage(playlists);

            let likes = likes::Likes::new(
                db.clone(),
                Arc::new(backend::routed::RoutedLikes(backends.clone())),
                Arc::new(likes::AppLikeEvents { app: app.handle().clone(), plex: plex.clone() }),
            );
            likes::spawn(likes.clone());
            app.manage(likes);

            // Listen on: other Plex players, and Platter as one (setting).
            let remote = remote::RemoteService::new(
                Arc::new(backend::routed::RoutedRemote(backends.clone())),
                Arc::new(player.clone()),
                Arc::new(plex::state::TauriSink(app.handle().clone())),
                plex.identity(),
                live.remote.clone(),
                remote::RemoteConfig::default(),
            )?;
            {
                let id = plex.identity();
                remote::commands::spawn_target(
                    &remote,
                    remote::target::TargetInfo {
                        client_id: id.client_id.clone(),
                        name: id.device_name.clone(),
                        product: id.product.clone(),
                        version: id.version.clone(),
                        platform: id.platform.clone(),
                        platform_version: id.platform_version.clone(),
                    },
                );
            }
            {
                let id = plex.identity();
                remote::commands::spawn_peers(&remote, id.platform.clone(), id.version.clone());
            }
            // Listen along goes through a relay (ntfy), never computer to computer.
            let listen_name = social::commands::ListenName::default();
            let listen = social::listen::ListenAlong::new(social::listen::ListenDeps {
                player: Arc::new(player.clone()),
                relays: Arc::new(social::listen::relay::Ntfy::new().map_err(error::AppError::Internal)?),
                events: Arc::new(plex::state::TauriSink(app.handle().clone())),
                server: Arc::new(backend::routed::RoutedServerId(backends.clone())),
                library: Arc::new(social::commands::DbListenLibrary(db.clone())),
                name: {
                    let plex_name = listen_name.0.clone();
                    let chosen = live.social.listen.name.clone();
                    Arc::new(move || {
                        let c = chosen.read().trim().to_string();
                        if c.is_empty() { plex_name.lock().clone() } else { c }
                    })
                },
                settings: live.social.listen.clone(),
                default_relay: social::listen::DEFAULT_RELAY.into(),
                cfg: Default::default(),
                tap: None,
            });
            let discord_party: Arc<parking_lot::Mutex<Option<platform::discord::activity::Party>>> = Arc::default();
            {
                let mut rx = listen.subscribe_party();
                let slot = discord_party.clone();
                tauri::async_runtime::spawn(async move {
                    while rx.changed().await.is_ok() {
                        let p = rx.borrow().clone();
                        *slot.lock() = p.map(|p| platform::discord::activity::Party { id: p.id, size: p.size, max: p.max, join: None });
                    }
                });
            }
            app.manage(listen);
            app.manage(listen_name);
            remote.set_lan(remote::LanEnv {
                mdns: Arc::new(remote::lan::MdnsSd::new()),
                route: net_route.clone(),
                passwords: Arc::new(plex::keychain::KeyringStore::new()),
                tls: remote::cast::conn::tls_config()?,
            });
            app.manage(remote);

            // Share links, and platter:// links opened from elsewhere.
            let share = share::ShareService::new(
                db.clone(),
                share_http,
                Arc::new(backend::routed::RoutedServerId(backends.clone())),
                live.share_public_links.clone(),
            );
            share.set_events(Arc::new(app.handle().clone()));
            share::commands::install(app.handle(), &share);

            // Discord status follows the player on its own task.
            platform::discord::commands::start(
                app.handle(),
                Arc::new(backend::routed::RoutedMbids(backends.clone())),
                db.clone(),
                discord_http,
                art.clone(),
                share.clone(),
                &live.discord,
                artist_info.musicbrainz_limiter(),
                player.subscribe_tracking(),
                discord_party,
            )?;
            app.manage(share);

            // Friend activity, follows and What's New.
            let social = social::Social::new(social::SocialDeps {
                db: db.clone(),
                follows: {
                    // Follows live in the sync store; an older build's list moves over once.
                    match sync::follows::SyncFollows::migrate(&sync_store, &db) {
                        Ok(n) if n > 0 => tracing::info!(n, "followed artists moved to the sync store"),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("followed artists not moved: {e}"),
                    }
                    Arc::new(sync::follows::SyncFollows { store: sync_store.clone() })
                },
                activity: Arc::new(backend::routed::RoutedActivity(backends.clone())),
                events: Arc::new(plex::state::TauriSink(app.handle().clone())),
                notifier: Arc::new(social::commands::OsNotifier(app.handle().clone())),
                switches: live.social.clone(),
                clock: Arc::new(db::queries::now_s),
            });
            social::commands::spawn_watch(social.clone(), db.clone());
            app.manage(social);

            // Release radar: new releases on MusicBrainz, about once a day.
            {
                let mb = |ttl: i64| -> Arc<dyn lidarr::mbpick::MbGet> {
                    Arc::new(lidarr::mbpick::MbCached {
                        http: artist_info.http(),
                        limiter: artist_info.musicbrainz_limiter(),
                        db: db.clone(),
                        clock: Arc::new(db::queries::now_s),
                        ttl,
                    })
                };
                let info = artist_info.clone();
                let radar = radar::Radar::new(radar::RadarDeps {
                    db: db.clone(),
                    mb: mb(radar::MB_TTL),
                    mbids: Arc::new(radar::commands::RadarMbids {
                        info: Arc::new(lidarr::InfoMbid(artist_info.clone())),
                        picker: lidarr::mbpick::Picker { mb: mb(lidarr::mbpick::CACHE_TTL) },
                    }),
                    follows: Arc::new(sync::follows::SyncFollows { store: sync_store.clone() }),
                    top: Arc::new(radar::StatsTop(db.clone())),
                    lidarr: Arc::new(radar_lidarr),
                    notifier: Arc::new(social::commands::OsNotifier(app.handle().clone())),
                    events: Arc::new(plex::state::TauriSink(app.handle().clone())),
                    usable: Arc::new(move || info.usable(artistinfo::InfoSource::Musicbrainz)),
                    private_session: live.tracking.private_session.clone(),
                    clock: Arc::new(db::queries::now_s),
                });
                tauri::async_runtime::spawn(radar::watch(radar.clone()));
                app.manage(radar);

                // Discover: music that isn't in the library (search, catalogue
                // pages, Fans also like, New to you).
                let discover = discover::Discover::new(discover::DiscoverDeps {
                    db: db.clone(),
                    mb: mb(lidarr::mbpick::CACHE_TTL),
                    similar: Arc::new(discover::lb::LbCached {
                        http: artist_info.http(),
                        limiter: Arc::new(artistinfo::limit::Limiter::per_second(1.0)),
                        db: db.clone(),
                        clock: Arc::new(db::queries::now_s),
                    }),
                    mbids: Arc::new(radar::commands::RadarMbids {
                        info: Arc::new(lidarr::InfoMbid(artist_info.clone())),
                        picker: lidarr::mbpick::Picker { mb: mb(lidarr::mbpick::CACHE_TTL) },
                    }),
                    top: Arc::new(radar::StatsTop(db.clone())),
                    info: Arc::new(artist_info.clone()),
                    requests: Arc::new(discover_lidarr),
                    prefs: Arc::new(discover::SettingsPrefs(db.clone())),
                    offline: plex.offline_switch(),
                    clock: Arc::new(db::queries::now_s),
                });
                app.manage(discover);
            }

            // Lyrics: the song's server first, then LRCLIB when its setting is on.
            let lyrics_service = lyrics::LyricsService::new(
                db.clone(),
                lyrics_http,
                Arc::new(backend::routed::RoutedLyrics(backends.clone())),
                lyrics::Switches {
                    lrclib: live.lyrics_lrclib.clone(),
                    prefer_own: live.lyrics_prefer_own.clone(),
                    show_auto: live.lyrics_show_auto.clone(),
                    live_timing: live.lyrics_live_timing.clone(),
                },
            );
            // Lyrics transcribed on this computer (opt-in, model downloaded on request).
            {
                use lyrics::transcribe::{self as tr, commands as trc};
                let transcriber = tr::Transcriber::new(tr::Deps {
                    db: db.clone(),
                    models: tr::model::ModelStore::new(
                        data_dir.join("models"),
                        tr::model::catalogue(),
                        tr::model::os_free_space(),
                    ),
                    fetch: Arc::new(tr::model::HttpFetch(reqwest::Client::new())),
                    recognizer: tr::engine::default_recognizer(),
                    songs: Arc::new(trc::AppSongs {
                        player: player.clone(),
                        audio_dir: audio_dir.clone(),
                        parts: Arc::new(backend::routed::RoutedParts(backends.clone())),
                        reader: tags::fetch::Reader::new(
                            audio::http::http_client(),
                            player.transfer_gate(),
                            Some(player.transfer_host()),
                        ),
                    }),
                    other: Arc::new(trc::AppOther(lyrics_service.clone())),
                    conditions: Arc::new(trc::AppConditions { player: player.clone(), plex: plex.clone() }),
                    events: Arc::new(app.handle().clone()),
                });
                transcriber.set_options(initial.transcribe_lyrics, initial.transcribe_model);
                *live.transcriber.lock() = Some(transcriber.clone());
                tauri::async_runtime::spawn({
                    let t = transcriber.clone();
                    async move { t.spawn() }
                });
                app.manage(transcriber);
            }
            app.manage(lyrics_service);

            // App-only state: the store, the sync file, and copying keys.
            {
                use sync::commands as sc;
                let device = plex.identity().client_id.clone();
                let store = sync_store.clone();
                let remote = app.state::<remote::RemoteService>().inner().clone();
                let switches = remote.switches().clone();
                let engine = sync::engine::SyncEngine::new(
                    store.clone(),
                    Arc::new(sc::RemoteTag(remote.clone())),
                    live.sync.subscribe(),
                    Arc::new(move || switches.computer_name()),
                    Default::default(),
                );
                engine.set_events(Arc::new(app.handle().clone()));
                let vault: Arc<dyn sync::keycopy::KeyVault> = Arc::new(sc::AppVault {
                    app: app.handle().clone(),
                    keychain: Arc::new(plex::keychain::KeyringStore::new()),
                });
                let keys = sync::keycopy::KeyReceiver::new(device, vault.clone(), Arc::new(sync::store::system_ms));
                keys.set_events(Arc::new(app.handle().clone()));
                // Key copy answers on the control port (same account only).
                {
                    use remote::peer::keys::{KeyCopyRoutes, RouteList};
                    use remote::peer::server::ExtraRoutes;
                    let copy: Arc<dyn ExtraRoutes> = Arc::new(KeyCopyRoutes::new(
                        plex.identity().client_id.clone(),
                        remote.peer_keys(),
                        Arc::new(keys.clone()),
                    ));
                    let slot = remote.extra_routes();
                    let mut routes = vec![copy];
                    routes.extend(slot.read().clone());
                    *slot.write() = Some(Arc::new(RouteList(routes)));
                }
                sc::spawn_listener(
                    store.clone(),
                    sc::Appliers {
                        app: app.handle().clone(),
                        settings: app_settings.clone(),
                        player: player.clone(),
                        downloads: downloads.clone(),
                        mixes: mix_service.clone(),
                        db: db.clone(),
                    },
                );
                {
                    let (store, settings, db, engine) = (store.clone(), app_settings.clone(), db.clone(), engine.clone());
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = settings.attach_sync(store.clone()).await {
                            tracing::warn!("synced settings not set up: {e}");
                        }
                        if let Err(e) = store.run(move |s| sync::lyrics::seed(s, &db)).await {
                            tracing::warn!("lyrics not added to the sync store: {e}");
                        }
                        engine.run().await;
                    });
                }
                app.manage(sc::SyncService { store, engine, keys, vault });
            }

            plex.start(app.handle().clone());
            watch_library(app.handle().clone(), plex.clone(), db.clone());
            doctor::spawn(app.handle().clone(), db.clone(), Arc::new(backend::routed::RoutedParts(backends.clone())), tags::fetch::Reader::new(audio::http::http_client(), player.transfer_gate(), Some(player.transfer_host())));
            {
                // Identify by sound: off until turned on in Settings.
                let sound = soundid::SoundId::new(
                    db.clone(),
                    Arc::new(backend::routed::RoutedParts(backends.clone())),
                    tags::fetch::Reader::new(audio::http::http_client(), player.transfer_gate(), Some(player.transfer_host())),
                    Arc::new(soundid::acoustid::AcoustId::new(audio::http::http_client())),
                    Arc::new(plex::keychain::KeyringStore::new()),
                    soundid::app_key(),
                );
                let handle = app.handle().clone();
                sound.set_on_change(Arc::new(move || {
                    use tauri::Emitter;
                    let _ = handle.emit(soundid::EV_CHANGED, ());
                }));
                sound.spawn();
                app.manage(sound);
            }
            monitor::files::spawn(
                plex.clone(),
                db.clone(),
                player.clone(),
                Arc::new(backend::routed::RoutedFiles(backends.clone())),
            );
            monitor::net::spawn(plex.clone());
            monitor::disk::spawn(
                plex.clone(),
                audio_dir.clone(),
                art.clone(),
                live.art_cap.clone(),
            );
            app.manage(monitor::cached::AudioCacheDir(audio_dir));
            app.manage(downloads);
            mixes::service::spawn_schedule(mix_service.clone());
            app.manage(mix_service);

            app.manage(plexedit::PlexEditor::new(
                data_dir.join("plex-edits"),
                Arc::new(backend::routed::RoutedEdit(backends.clone())),
                Arc::new(plexedit::AppEditHooks { db: db.clone(), art: art.clone() }),
            ));
            app.manage(plex);
            app.manage(backends);
            app.manage(db);
            // The now-playing feed for widgets, on 127.0.0.1 while its setting is on.
            app.manage(feed::spawn(
                live.feed.clone(),
                Arc::new(feed::AppSource { player: player.clone(), art: art.clone() }),
                Arc::new(plex::state::TauriSink(app.handle().clone())),
            ));
            // The Stream Deck control API, on 127.0.0.1 while it's allowed.
            app.manage(feed::deck::spawn(
                live.deck.clone(),
                Arc::new(feed::deck::app::AppDeck::new(app.handle().clone(), player.clone(), art.clone())),
                Arc::new(feed::deck::pairing::Pairing::new(Arc::new(plex::keychain::KeyringStore::new()))),
                Arc::new(plex::state::TauriSink(app.handle().clone())),
            ));
            app.manage(art);
            app.manage(artist_info);
            app.manage(player);
            app.manage(audio::compare::CompareState::default());
            app.manage(app_settings);
            app.manage(live.platform.clone());
            updates::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            import::commands::import_overview, import::commands::import_choose, import::commands::import_run, import::commands::import_review_queue, import::commands::import_review, import::commands::import_list_page, import::commands::import_missing, import::commands::import_missing_albums, import::commands::import_missing_album_add, import::commands::import_missing_album_remove, import::commands::import_open_link, import::commands::import_forget, import::commands::import_list_remove,
            plex::commands::auth_status,
            plex::commands::auth_start,
            plex::commands::auth_cancel,
            plex::commands::auth_sign_out,
            plex::commands::servers_list,
            plex::commands::server_select,
            plex::commands::health,
            plex::commands::open_local_network_settings,
            plex::commands::home_users,
            plex::commands::home_switch,
            plex::commands::home_remember_set,
            libraries::library_sources,
            libraries::library_servers,
            backend::commands::backend_caps,
            jellyfin::commands::servers_saved,
            jellyfin::commands::server_set_primary,
            jellyfin::commands::server_remove,
            jellyfin::commands::server_libraries,
            jellyfin::commands::server_libraries_set,
            jellyfin::commands::jellyfin_check,
            jellyfin::commands::jellyfin_quick_start,
            jellyfin::commands::jellyfin_quick_poll,
            jellyfin::commands::jellyfin_quick_cancel,
            jellyfin::commands::jellyfin_sign_in_password,
            subsonic::commands::subsonic_check,
            subsonic::commands::subsonic_sign_in,
            backend::resume::resume_queues,
            backend::resume::resume_queue_play,
            backend::resume::resume_queue_save,
            plex::details::now_playing_details,
            lyrics::commands::lyrics_get,
            lyrics::commands::lyrics_own_get,
            lyrics::commands::lyrics_own_save,
            lyrics::commands::lyrics_own_delete,
            lyrics::commands::lyrics_own_ids,
            lyrics::commands::lyrics_export,
            lyrics::transcribe::commands::transcribe_status,
            lyrics::transcribe::commands::transcribe_model_download,
            lyrics::transcribe::commands::transcribe_model_cancel,
            lyrics::transcribe::commands::transcribe_model_delete,
            lyrics::transcribe::commands::transcribe_track,
            lyrics::transcribe::commands::transcribe_album,
            lyrics::transcribe::commands::transcribe_unreleased,
            lyrics::transcribe::commands::transcribe_cancel,
            lyrics::transcribe::commands::transcribe_found_nothing,
            db::commands::sync_status,
            db::commands::sync_now,
            db::commands::home,
            db::commands::home_shelf,
            db::commands::artists,
            db::commands::artist_names,
            db::commands::albums,
            db::commands::playlists,
            db::commands::library_items,
            db::commands::artist_page,
            db::commands::album_page,
            db::commands::recent_tracks,
            db::commands::recents,
            stats::commands::stats_overview, stats::commands::stats_span, stats::commands::stats_item, stats::commands::stats_month_recap, stats::commands::stats_play_ids, stats::commands::stats_compare_summary, stats::commands::stats_compare, stats::commands::stats_friends, stats::commands::stats_friend_remove,
            db::commands::playlist_page,
            mixes::commands::mixes_home,
            mixes::commands::mix_page,
            mixes::commands::mix_refresh,
            mixes::commands::radio_page,
            playlists::playlist_create,
            playlists::playlist_add,
            playlists::playlist_duplicates,
            playlists::playlist_remove,
            playlists::playlist_move,
            playlists::playlist_update,
            playlists::playlist_set_cover,
            playlists::playlist_delete,
            plexedit::commands::plex_edit_access,
            plexedit::commands::plex_edit_item,
            plexedit::commands::plex_edit_images,
            plexedit::commands::plex_edit_select_image,
            plexedit::commands::plex_edit_upload_image,
            plexedit::commands::plex_edit_image_link,
            plexedit::commands::plex_edit_matches,
            plexedit::commands::plex_edit_match,
            plexedit::commands::plex_edit_unmatch,
            plexedit::commands::plex_edit_details,
            plexedit::commands::plex_edit_merge,
            plexedit::commands::plex_edit_split,
            plexedit::commands::plex_edit_refresh,
            plexedit::commands::plex_edit_undo,
            plexedit::commands::plex_edit_history,
            likes::likes_set,
            likes::liked_ids,
            likes::liked_page,
            likes::playlists_with_track,
            remote::commands::remote_players,
            remote::commands::remote_state,
            remote::commands::remote_move,
            remote::commands::remote_return,
            remote::commands::remote_play_context,
            remote::commands::remote_queue_jump,
            remote::commands::remote_command,
            remote::commands::remote_move_with_password,
            db::commands::search,
            db::commands::search_completions,
            db::commands::search_recents,
            db::commands::search_recent_add,
            db::commands::search_recent_remove,
            db::commands::search_recents_clear,
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
            audio::commands::set_smart_shuffle,
            audio::commands::set_context_smart_shuffle,
            audio::commands::smart_suggestion_remove,
            audio::commands::smart_suggestion_keep,
            audio::commands::set_smooth_transitions,
            audio::commands::set_repeat,
            audio::commands::set_ab_loop,
            audio::commands::queue_add,
            audio::commands::queue_remove,
            audio::commands::queue_move,
            audio::commands::queue_clear,
            audio::commands::queue_jump,
            audio::commands::player_state,
            audio::commands::player_problems,
            audio::commands::output_devices,
            audio::commands::output_select,
            audio::commands::eq_profile_save,
            audio::commands::eq_profile_rename,
            audio::commands::eq_profile_delete,
            audio::commands::eq_use_for_output,
            audio::hifi::headphone_search,
            audio::hifi::headphone_correction_set,
            audio::hifi::headphone_correction_compare,
            audio::hifi::bit_perfect_set,
            settings::settings_get,
            settings::settings_set,
            settings::art_cache_info,
            settings::art_cache_clear,
            settings::app_info,
            settings::open_project_page,
            themes::themes_list,
            themes::themes_import,
            themes::themes_open_folder,
            themes::themes_import_text,
            themes::themes_save,
            themes::themes_export,
            artistinfo::commands::artist_info,
            artistinfo::commands::album_credits,
            artistinfo::commands::album_release_status,
            archive::commands::release_mark_set,
            archive::commands::track_versions,
            archive::commands::track_lineage,
            audio::compare::compare_prepare,
            audio::compare::compare_switch,
            audio::compare::compare_loop,
            audio::compare::compare_end,
            archive::commands::era_page,
            archive::commands::album_section_set,
            archive::commands::artist_sections,
            doctor::commands::doctor_report,
            doctor::commands::doctor_count,
            doctor::commands::doctor_ignore,
            doctor::commands::doctor_run,
            soundid::commands::sound_id_status,
            soundid::commands::sound_id_key_set,
            soundid::commands::track_sounds_like,
            plexedit::commands::plex_edit_merge_albums,
            feed::commands::feed_status,
            feed::deck::commands::deck_status,
            feed::deck::commands::deck_revoke,
            artistinfo::commands::artist_info_keys,
            artistinfo::commands::artist_info_key_set,
            artistinfo::commands::artist_info_cache_clear,
            artistinfo::commands::open_info_link,
            monitor::cached::cached_tracks,
            monitor::cached::cached_library,
            downloads::commands::downloads_status,
            downloads::commands::downloaded_tracks,
            downloads::commands::downloads_set,
            downloads::commands::downloads_retry,
            downloads::commands::downloads_remove_all,
            downloads::commands::downloads_choose_folder,
            updates::update_check,
            updates::update_install_on_restart,
            share::commands::share_info,
            share::commands::share_public_link,
            share::commands::deep_link_take,
            share::commands::share_card_qr,
            share::commands::share_card_copy,
            share::commands::share_card_save,
            platform::discord::commands::discord_status,
            platform::discord::commands::discord_imgur_key_set,
            tags::commands::tags_index,
            tags::commands::tag_scan_status,
            social::commands::friend_activity,
            social::commands::follows_list,
            social::commands::follow_set,
            social::commands::whats_new,
            social::commands::whats_new_seen,
            social::commands::listen_state,
            social::commands::listen_start,
            social::commands::listen_invite,
            social::commands::listen_stop,
            social::commands::listen_join,
            social::commands::listen_add,
            social::commands::listen_control,
            social::commands::listen_remove,
            sync::commands::sync_file_status,
            sync::commands::sync_file_now,
            sync::commands::sync_file_folders,
            sync::commands::sync_file_use_folder,
            sync::commands::sync_file_choose_folder,
            sync::commands::sync_file_replace,
            sync::commands::pins_get,
            sync::commands::pins_toggle,
            sync::commands::pins_import,
            sync::commands::folders_get,
            sync::commands::folder_create,
            sync::commands::folder_rename,
            sync::commands::folder_delete,
            sync::commands::folder_move,
            sync::commands::playlist_folder_set,
            sync::commands::keys_available,
            sync::commands::keys_receive_start,
            sync::commands::keys_receive_state,
            sync::commands::keys_receive_cancel,
            sync::commands::keys_send,
            hidden::hidden_list,
            hidden::hidden_set,
            bookmarks::bookmarks_list,
            bookmarks::bookmark_add,
            bookmarks::bookmark_remove,
            bookmarks::bookmark_open,
            smart::commands::smart_preview,
            smart::commands::smart_save,
            smart::commands::smart_list,
            smart::commands::smart_page,
            smart::commands::smart_delete,
            smart::commands::smart_plex_rules,
            smart::commands::smart_prompt,
            smart::commands::ai_status,
            smart::commands::ai_key_set,
            scrobble::commands::scrobble_status,
            scrobble::commands::lastfm_secret_set,
            scrobble::commands::lastfm_sign_in_start,
            scrobble::commands::lastfm_sign_in_finish,
            scrobble::commands::lastfm_sign_out,
            scrobble::commands::listenbrainz_check,
            lidarr::commands::lidarr_status,
            lidarr::commands::lidarr_key_set,
            lidarr::commands::lidarr_headers_set,
            lidarr::commands::lidarr_check,
            lidarr::commands::lidarr_confirm_root,
            lidarr::commands::lidarr_pick,
            lidarr::commands::lidarr_request_preview,
            lidarr::commands::lidarr_request,
            lidarr::commands::lidarr_requests,
            lidarr::commands::lidarr_request_cancel,
            lidarr::commands::lidarr_missing_releases,
            radar::commands::release_radar,
            discover::commands::catalogue_search,
            discover::commands::catalogue_artist,
            discover::commands::catalogue_album,
            discover::commands::catalogue_fans_also_like,
            discover::commands::new_to_you,
            radar::commands::release_radar_check,
            radar::commands::radar_auto_artists,
            radar::commands::radar_auto_set,
            platform::miniplayer::miniplayer_state,
            platform::miniplayer::miniplayer_set,
        ])
        .on_window_event(|window, event| {
            platform::miniplayer::on_window_event(window, event);
            platform::visibility::on_window_event(window, event);
            // Back in front: look for an update if the last check is old.
            if let tauri::WindowEvent::Focused(true) = event
                && window.label() == "main"
            {
                updates::window_shown(window.app_handle());
            }
            // Back from System Settings: pick up Local Network access at once.
            if let tauri::WindowEvent::Focused(true) = event
                && window.label() == "main"
                && let Some(plex) = window.try_state::<plex::PlexState>()
            {
                plex.recheck_if_local_network_blocked();
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
                && let Some(live) = window.try_state::<platform::Switches>()
                && platform::close_action(&live.get()) == platform::CloseAction::Hide
            {
                api.prevent_close();
                let _ = window.hide();
                if let Some(w) = window.app_handle().get_webview_window("main") {
                    platform::visibility::set_page_visible(&w, false);
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let player = app.state::<PlayerHandle>().inner().clone();
                tauri::async_runtime::block_on(player.flush());
                // A hosted session ends for everyone; a guest says it left.
                if let Some(listen) = app.try_state::<social::listen::ListenAlong>() {
                    let listen = listen.inner().clone();
                    tauri::async_runtime::block_on(async move {
                        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), listen.stop()).await;
                    });
                }
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
            item_type: Some(10),
            state: Some(5),
        }
    }

    fn scan_ended(section: Option<&str>) -> PlexEvent {
        PlexEvent::Activity {
            event: "ended".into(),
            uuid: "u".into(),
            kind: "library.update.section".into(),
            title: None,
            subtitle: None,
            progress: None,
            section_id: section.map(str::to_owned),
        }
    }

    #[test]
    fn library_watcher_sleeps_until_woken_when_nothing_is_pending() {
        let s = Duration::from_secs;
        // Socket up, nothing pending: no timer at all.
        assert_eq!(library_wait(None, false, s(0), false), None);
        // A change waits out the quiet period.
        assert_eq!(library_wait(Some(s(2)), false, s(0), false), Some(s(3)));
        // Socket down: the next fallback sync, not a 1 s poll.
        assert_eq!(library_wait(None, true, s(60), false), Some(FALLBACK_EVERY - s(60)));
        // A due sync that couldn't start retries in seconds, not per tick.
        assert_eq!(library_wait(Some(s(10)), false, s(0), true), Some(s(5)));
    }

    #[test]
    fn item_changes_are_applied_live_not_by_a_sync() {
        assert!(!marks_library_dirty(&update(Some("7")), Some("7")));
    }

    #[test]
    fn scans_ending_in_other_libraries_dont_trigger_a_music_sync() {
        assert!(!marks_library_dirty(&scan_ended(Some("2")), Some("7")));
        assert!(marks_library_dirty(&scan_ended(Some("7")), Some("7")));
    }

    #[test]
    fn unknown_sections_and_reconnects_still_count() {
        assert!(marks_library_dirty(&scan_ended(None), Some("7")));
        assert!(marks_library_dirty(&scan_ended(Some("2")), None));
        assert!(marks_library_dirty(&PlexEvent::Connected, Some("7")));
        assert!(!marks_library_dirty(&PlexEvent::Disconnected, Some("7")));
    }
}
