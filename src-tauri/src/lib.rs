// The graphics stack nests deeply enough to pass the default limit when
// the compiler checks it is safe to share across threads.
#![recursion_limit = "256"]

pub mod analysis;
pub mod archive;
pub mod artistinfo;
pub mod asks;
pub mod audio;
pub mod backend;
pub mod carplay;
pub mod bookmarks;
pub mod bridge;
pub mod db;
pub mod devices;
pub mod diary;
pub mod digging;
pub mod discover;
pub mod doctor;
pub mod downloads;
pub mod error;
pub mod feed;
pub mod hidden;
pub mod homeside;
pub mod explicit;
pub mod identity;
pub mod import;
pub mod jellyfin;
pub mod keyhost;
pub mod libraries;
pub mod lidarr;
pub mod likes;
pub mod lyrics;
pub mod mixes;
pub mod model;
pub mod musicshare;
pub mod monitor;
pub mod notify;
pub mod physical;
pub mod platform;
pub mod playhistory;
pub mod playlists;
pub mod plex;
pub mod plexedit;
pub mod radar;
pub mod remote;
pub mod scrobble;
pub mod settings;
pub mod share;
pub mod sid;
pub mod sleeve;
pub mod smart;
pub mod soundmap;
pub mod social;
pub mod soundid;
pub mod subsonic;
pub mod stats;
pub mod sync;
pub mod tagedit;
pub mod tags;
pub mod tapplay;
pub mod themes;
pub mod tracking;
#[cfg(desktop)]
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
///   `FALLBACK_EVERY` instead, since no events arrive. While it is up, one
///   runs every `SAFETY_EVERY` anyway: a server can keep the socket open
///   and still not send library events (to an account that isn't its
///   owner, say).
/// - Every `SCAN_CHECK_EVERY` the music library's last scan time is asked
///   for; a scan since the last look runs a sync, so new music shows within
///   minutes even when the scan's notification never came.
const QUIET: Duration = Duration::from_secs(5);
const FALLBACK_EVERY: Duration = Duration::from_secs(15 * 60);
const SAFETY_EVERY: Duration = Duration::from_secs(60 * 60);
/// How often the music library's last scan time is asked for (one small
/// request): a scan that added music runs a sync within minutes even when
/// its notification never arrived.
const SCAN_CHECK_EVERY: Duration = Duration::from_secs(3 * 60);

/// Whether the library was scanned since the last look: `scanned` is the
/// section's last scan time now, `seen` the one from the previous look
/// (None the first time, when it's compared with the last sync instead).
/// Only a move forward counts, so a server clock that runs ahead of this
/// computer's can't cause a sync on every look.
fn scanned_since(scanned: Option<i64>, seen: Option<i64>, last_sync: Option<i64>) -> bool {
    let Some(scanned) = scanned else { return false };
    match seen {
        Some(seen) => scanned > seen,
        None => last_sync.is_none_or(|at| scanned > at),
    }
}

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
        let mut last_scan_check = std::time::Instant::now();
        let mut seen_scan: Option<i64> = None;
        loop {
            let down = !socket_up.load(std::sync::atomic::Ordering::Acquire);
            if last_scan_check.elapsed() >= SCAN_CHECK_EVERY && pending.lock().is_none() {
                last_scan_check = std::time::Instant::now();
                let state = db.read(db::store::sync_state).ok().flatten();
                if let (Some(state), Ok(client)) = (state, plex.client().await) {
                    match client.sections().await {
                        Ok(sections) => {
                            let scanned = sections.iter().find(|s| s.id == state.section_id).and_then(|s| s.scanned_at);
                            if scanned_since(scanned, seen_scan, state.last_sync_at) {
                                tracing::debug!("music library scanned since the last look; sync");
                                *pending.lock() = std::time::Instant::now().checked_sub(QUIET);
                            }
                            if scanned.is_some() {
                                seen_scan = scanned;
                            }
                        }
                        Err(e) => tracing::debug!("library scan check: {e}"),
                    }
                }
            }
            if last_fallback.elapsed() >= if down { FALLBACK_EVERY } else { SAFETY_EVERY } {
                last_fallback = std::time::Instant::now();
                tracing::debug!(down, "periodic sync");
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
            let wait = library_wait(age, down, last_fallback.elapsed(), blocked)
                .min(SCAN_CHECK_EVERY.saturating_sub(last_scan_check.elapsed()).max(Duration::from_secs(1)));
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = wake.notified() => {}
            }
        }
    });
}

/// How long the library watcher sleeps (an event wakes it sooner): until a
/// pending change has been quiet long enough, a short retry when a due sync
/// couldn't start, else until the next periodic sync.
fn library_wait(pending_age: Option<Duration>, socket_down: bool, since_fallback: Duration, blocked: bool) -> Duration {
    const RETRY: Duration = Duration::from_secs(5);
    if blocked {
        return RETRY;
    }
    if let Some(age) = pending_age {
        return QUIET.saturating_sub(age).max(Duration::from_millis(100));
    }
    let every = if socket_down { FALLBACK_EVERY } else { SAFETY_EVERY };
    every.saturating_sub(since_fallback).max(Duration::from_secs(1))
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

/// `platter --speech-worker`: the helper process speech-model work runs in
/// (lyrics transcription and the lyrics check), and `--speech-probe`, which
/// lists the graphics cards for it. Checked first thing in
/// `main`, before anything of the app starts. Returns the exit code.
pub fn speech_worker(args: &[String]) -> Option<i32> {
    match args.get(1).map(String::as_str) {
        Some(lyrics::transcribe::helper::WORKER_FLAG) => Some(lyrics::transcribe::helper::worker::main()),
        // `platter --speech-probe`: lists the graphics cards and exits.
        Some(lyrics::transcribe::gpu::PROBE_FLAG) => Some(lyrics::transcribe::gpu::probe_main()),
        _ => None,
    }
}

/// The desktop-only plugins: one running copy, updates, global media keys
/// and launch at login. A phone build has none of them.
#[cfg(desktop)]
fn desktop_plugins(b: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    b
        // First, so a second launch (a platter:// link on Windows) hands its
        // link to the copy already running and quits before doing anything.
        // A command link (platter://next) leaves the window where it is.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|a| feed::deck::automation::is_command_link(a)) {
                share::commands::focus_main(app)
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // Launch at login (a LaunchAgent on macOS, the default).
        .plugin(tauri_plugin_autostart::Builder::new().arg(platform::AUTOSTART_ARG).build())
        .manage(updates::Updates::default())
}

#[cfg(mobile)]
fn desktop_plugins(b: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    b
}

/// Links by Plex account, kept for the life of the app.
struct AccountLinks {
    _looker: Arc<homeside::account::Looker>,
    _keep: tokio::sync::watch::Sender<bool>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `platter --next` and the rest talk to the running app and exit.
    if let Some(code) = feed::deck::cli::main(&std::env::args().collect::<Vec<_>>()) {
        std::process::exit(code);
    }
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
    #[cfg(target_os = "ios")]
    platform::audio_session::start();
    #[cfg(target_os = "ios")]
    carplay::ios::install();
    #[cfg(target_os = "ios")]
    platform::foreground::watch_app_state();

    desktop_plugins(tauri::Builder::default())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
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
            #[cfg(target_os = "ios")]
            platform::ios_webview::settle(app.handle());
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
            let (db, recovered) = DbHandle::open_or_recover(data_dir.join("library.sqlite"))?;
            if let Some(r) = recovered {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Emitter;
                    // Once the window is up and listening.
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    let toast = plex::types::Toast { kind: "error".into(), message: r.message() };
                    let _ = handle.emit(plex::types::EVENT_TOAST, toast);
                });
            }
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
                #[cfg(desktop)]
                auto_update: app.state::<updates::Updates>().auto.clone(),
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
            // Keys saved before their hosts were recorded: bound to the
            // addresses set now, before synced settings can change them.
            keyhost::startup_adopt(&db);
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
            // Songs whose files don't say: ask Deezer which are explicit.
            tauri::async_runtime::spawn(explicit::watch(db.clone(), artist_info.http(), {
                let h = app.handle().clone();
                Arc::new(move || {
                    use tauri::Emitter;
                    let _ = h.emit(tags::commands::EV_CHANGED, ());
                })
            }));
            // The window opens hidden and is shown once the page has drawn
            // (launched at login: kept in the tray or minimised).
            let args: Vec<String> = std::env::args().collect();
            platform::reveal::install(app.handle(), &root_dir, platform::start_mode(&args, &initial.platform()));
            let os = platform::Platform::new();

            let net_route = audio::netsink::NetRoute::default();
            // A tapped song can play from its finished download (lidarr::tap).
            let arriving = tapplay::Arriving::default();
            let player_files = audio::source::LocalFiles(Arc::new(tapplay::ArrivingFiles { inner: local_files.0.clone(), arriving: arriving.clone() }));
            let player = PlayerHandle::spawn(
                tauri::async_runtime::handle().inner().clone(),
                Arc::new(backend::routed::RoutedStreams(backends.clone())),
                Arc::new(tapplay::ArrivingLookup {
                    inner: Arc::new(bridge::CacheLookup(
                        db.clone(),
                        Some(backends.clone()),
                        bridge::OfflinePlay::new(audio_dir.clone()).with_downloads(local_files.clone()),
                        Some(mix_service.clone()),
                    )),
                    arriving: arriving.clone(),
                }),
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
                    local_files: Some(player_files),
                    autoplay: initial.queue_end == settings::QueueEnd::KeepPlaying,
                    smart_shuffle: initial.smart_shuffle,
                    automix: initial.automix_options(),
                    ..Default::default()
                },
            );
            #[cfg(target_os = "ios")]
            platform::audio_session::watch(player.clone(), app.handle().clone());
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
            // Sync to a device: songs through the same transfer gate, at
            // background priority, converted on this computer.
            let devices = devices::run::Devices::new(devices::run::Deps {
                db: db.clone(),
                files: Arc::new(devices::commands::ServerFiles {
                    source: Arc::new(backend::routed::RoutedDownloads(backends.clone())),
                    local: Arc::new(downloads.clone()),
                    gate: player.transfer_gate(),
                    host: player.transfer_host(),
                    client: audio::http::http_client(),
                    art: Some(Arc::new(backend::routed::RoutedArt(backends.clone()))),
                }),
                volumes: Arc::new(devices::commands::SystemVolumes),
                io: Arc::new(devices::io::StdIo),
                pace: Arc::new(devices::commands::SystemPace {
                    host: player.transfer_host(),
                    load: Default::default(),
                }),
                scratch: cache_dir.join("device-sync"),
            });
            devices.set_events(Arc::new(app.handle().clone()));
            app.manage(devices);
            // The sound map, kept up to date as songs are analysed.
            let sound_map = soundmap::SoundMap::new(db.clone());
            {
                let h = app.handle().clone();
                sound_map.set_on_changed(Arc::new(move || {
                    use tauri::Emitter;
                    let _ = h.emit(soundmap::EV_CHANGED, ());
                }));
            }
            sound_map.spawn();
            app.manage(sound_map.clone());
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
            os.start(app.handle(), player.clone(), art.clone(), cache_dir.join("now-playing"), live.platform.clone(), db.clone());
            // The listening diary: albums played start to finish, kept here
            // (and in the sync file only when its own setting is on).
            // Where an entry was heard: the name set for this computer, or its
            // own name. Never a server's idea of it, so every backend gets it.
            let diary = diary::Diary::new(db.clone(), sync_store.clone(), live.diary.clone(), String::new()).with_device_name({
                let db = db.clone();
                Arc::new(move || {
                    let n = settings::load(&db).computer_name;
                    if n.trim().is_empty() { remote::peer::os_computer_name() } else { n }
                })
            });
            diary.set_events(Arc::new(app.handle().clone()));
            diary.set_place(Arc::new(player.clone()));
            app.manage(diary.clone());
            tracking::spawn(
                tracking::Tracker::new(backends.clone(), db.clone(), live.tracking.clone()).with_play_ended(Arc::new(diary)),
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
            // Library import from a connected streaming account (playlists).
            {
                let handle = app.handle().clone();
                let cfg_db = db.clone();
                let account = import::account::AccountImport::new(import::account::Deps {
                    web: web_out.clone(),
                    keys: keychain.clone(),
                    db: db.clone(),
                    events: Arc::new(plex::state::TauriSink(app.handle().clone())),
                    open: Arc::new(move |url: &str| {
                        use tauri_plugin_opener::OpenerExt;
                        handle.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
                    }),
                    config: Arc::new(move || import::account::Config::from_settings(&settings::load(&cfg_db))),
                    clock: Arc::new(|| db::queries::now_s() * 1000),
                    ends: import::account::api::Endpoints::default(),
                    sign_in_wait: import::account::callback::WAIT,
                });
                account.spawn_recheck();
                app.manage(account);
            }
            // Play history from before Platter and other apps: the Plex
            // server's, Tautulli's and Last.fm's.
            {
                let server: Arc<dyn playhistory::plex::PlexServer> = Arc::new(playhistory::plex::LivePlex { plex: plex.clone(), db: db.clone() });
                let tautulli = Arc::new(playhistory::tautulli::Tautulli { web: web_out.clone(), keys: keychain.clone(), db: db.clone(), plex: server.clone() });
                let history = playhistory::PlayHistory::new(playhistory::Deps {
                    db: db.clone(),
                    feeds: vec![
                        Arc::new(playhistory::plex::PlexFeed::new(server)),
                        tautulli.clone(),
                        Arc::new(playhistory::lastfm::LastfmFeed { web: web_out.clone(), keys: keychain.clone(), db: db.clone() }),
                    ],
                    events: Arc::new(plex::state::TauriSink(app.handle().clone())),
                    clock: Arc::new(|| db::queries::now_s() * 1000),
                    pause: playhistory::PAGE_PAUSE,
                    page_size: playhistory::PAGE_SIZE,
                });
                tauri::async_runtime::spawn(playhistory::watch(history.clone()));
                app.manage(playhistory::commands::PlayHistoryState { history, tautulli });
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
                shared: lidarr::shared::Shared::new(sync_store.clone(), {
                    let db = db.clone();
                    Arc::new(move || {
                        let n = settings::load(&db).computer_name;
                        if n.trim().is_empty() { remote::peer::os_computer_name() } else { n }
                    })
                }),
                plex_server: Arc::new(lidarr::plexside::PlexLive(plex.clone())),
            });
            {
                // Only the Plex server's owner gets music through Lidarr.
                // Signed out of Plex (or its sign-in expired) is nobody's;
                // with only other kinds of server, Lidarr is whoever set it
                // up here.
                let (plex, backends) = (plex.clone(), backends.clone());
                lidarr_service.set_owner_check(Arc::new(move || plex.owns_current_server_now().unwrap_or_else(|| !backends.plex_set_up())));
            }
            tauri::async_runtime::spawn(lidarr::request::watch(lidarr_service.clone()));
            // Tap a song the library doesn't have: it's got and played.
            let taps = lidarr::tap::Taps::new(
                lidarr_service.clone(),
                Arc::new(tapplay::AppTapHost {
                    app: app.handle().clone(),
                    player: player.clone(),
                    arriving: arriving.clone(),
                    db: db.clone(),
                    last_refresh: Default::default(),
                }),
            );
            tauri::async_runtime::spawn(taps.clone().run());
            let asks_taps = taps.clone();
            app.manage(taps);
            // Bring in my listening: albums from Played a lot, a few a day.
            let bring_in = {
                let (day_db, offset_db, hidden_store) = (db.clone(), db.clone(), sync_store.clone());
                lidarr::waves::Waves::new(lidarr::waves::WavesDeps {
                    lidarr: lidarr_service.clone(),
                    notifier: Arc::new(social::commands::OsNotifier(app.handle().clone())),
                    day_of: Arc::new(move |at| lidarr::waves::local_day(&day_db, at)),
                    utc_offset: Some(Arc::new(move |at| lidarr::waves::local_offset(&offset_db, at))),
                    hidden: Arc::new(move || {
                        hidden_store.list(&sync::store::PLAYED_HIDDEN).map(|v| v.into_iter().map(|(k, _)| k).collect()).unwrap_or_default()
                    }),
                })
            };
            tauri::async_runtime::spawn(lidarr::waves::watch(bring_in.clone()));
            {
                // A settings change (the switch, waves, pace) runs a pass soon and reaches a Bridge.
                let w = bring_in.clone();
                app.listen_any("settings://changed", move |_| w.settings_changed());
            }
            let bring_in_for_bridge = bring_in.clone();
            let bring_over_waves = bring_in.clone();
            let bring_over_lidarr = lidarr_service.clone();
            app.manage(bring_in);
            // Upgrades: better copies through the staging folder, swapped on this computer.
            let upgrades = lidarr::upgrade::Upgrades::new(lidarr::upgrade::UpgradeDeps {
                lidarr: lidarr_service.clone(),
                db: db.clone(),
                hosts: Arc::new(lidarr::upgrade::DesktopHosts { db: db.clone() }),
                plex: Arc::new(lidarr::upgrade::PlexRescan(plex.clone())),
                clock: Arc::new(db::queries::now_s),
                status: Arc::new(lidarr::provenance::TagStatus),
            });
            tauri::async_runtime::spawn(lidarr::upgrade::watch(upgrades.clone()));
            // Managed and protected artists: catching up with what Lidarr did,
            // adding protected albums from staging, and telling the user.
            lidarr_service.set_hosts(Arc::new(lidarr::upgrade::DesktopHosts { db: db.clone() }));
            lidarr_service.set_announcer(Arc::new(lidarr::commands::AppAnnouncer { app: app.handle().clone(), plex: plex.clone() }));
            tauri::async_runtime::spawn(lidarr::catchup::watch(lidarr_service.clone()));
            app.manage(upgrades);
            // Platter Bridge (optional): pairing, and the link to any paired Bridge.
            let bridge_desktop = homeside::desktop::Desktop::new(homeside::desktop::DesktopDeps {
                db: db.clone(),
                keys: keychain.clone(),
                relays: Arc::new(social::listen::relay::Ntfy::new().map_err(error::AppError::Internal)?),
                clock: Arc::new(sync::store::system_ms),
                store: sync_store.clone(),
                lidarr: lidarr_service.clone(),
                plex: Some(plex.clone()),
                name: {
                    let db = db.clone();
                    Arc::new(move || {
                        let n = settings::load(&db).computer_name;
                        if n.trim().is_empty() { remote::peer::os_computer_name() } else { n }
                    })
                },
            });
            {
                let app = app.handle().clone();
                bridge_desktop.set_notify(Arc::new(move || {
                    use tauri::Emitter;
                    let _ = app.emit("bridge://changed", ());
                }));
            }
            bridge_desktop.set_waves(bring_in_for_bridge);
            {
                // Lyrics a Bridge transcribed show at once.
                let app = app.handle().clone();
                bridge_desktop.set_on_lyrics(Arc::new(move |changed| {
                    for (id, found) in changed {
                        lyrics::transcribe::TranscribeEvents::done(&app, &id, found);
                    }
                }));
            }
            tauri::async_runtime::spawn(bridge_desktop.clone().run());
            let bridge_for_lyrics = bridge_desktop.clone();
            app.manage(bridge_desktop);
            let radar_lidarr = lidarr_service.clone();
            let discover_lidarr = lidarr_service.clone();
            let asks_lidarr = lidarr_service.clone();
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
            let circle_playlists = playlists.clone();
            app.manage(playlists);

            let likes = likes::Likes::new(
                db.clone(),
                Arc::new(backend::routed::RoutedLikes(backends.clone())),
                Arc::new(likes::AppLikeEvents { app: app.handle().clone(), plex: plex.clone() }),
            );
            likes::spawn(likes.clone());
            // Bringing music over: likes and playlists go out through the
            // queues above, missing albums through the Get albums queue.
            let bring_over = import::bring::Bring::new(import::bring::BringDeps {
                db: db.clone(),
                server: Arc::new(import::bring::AppServer { likes: likes.clone(), playlists: circle_playlists.clone() }),
                getter: Arc::new(import::bring::AppGetter { lidarr: bring_over_lidarr, waves: bring_over_waves }),
                clock_ms: Arc::new(|| db::queries::now_s() * 1000),
            });
            tauri::async_runtime::spawn(import::bring::watch(bring_over.clone()));
            {
                // Albums arriving show up as library changes.
                let b = bring_over.clone();
                app.listen_any("library://changed", move |_| b.poke());
            }
            app.manage(bring_over);
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
            // Sharing music through Plex (Settings, Sharing, and in listening sessions).
            let sharing = Arc::new(social::sharing::Sharing::new(
                Arc::new(social::sharing::PlexHost { plex: plex.clone(), db: db.clone(), settings: app_settings.clone() }),
                db.clone(),
                live.social.sharing.clone(),
                Arc::new(plex::state::TauriSink(app.handle().clone())),
                Arc::new(social::commands::OsNotifier(app.handle().clone())),
                social::sharing::links::LinkDeps {
                    relays: Arc::new(social::listen::relay::Ntfy::new().map_err(error::AppError::Internal)?),
                    relay: live.social.listen.relay.clone(),
                    default_relay: social::listen::DEFAULT_RELAY.into(),
                    answer_timeout: std::time::Duration::from_secs(90),
                    retry: std::time::Duration::from_secs(2),
                },
            ));
            // Requests from people the server is shared with (they ask, the
            // owner says yes; only the owner's Platter reaches Lidarr).
            {
                let requests = asks::Asks::new(
                    Arc::new(asks::live::LiveAsksHost {
                        plex: plex.clone(),
                        lidarr: asks_lidarr.clone(),
                        sharing: sharing.clone(),
                        db: db.clone(),
                        events: Arc::new(plex::state::TauriSink(app.handle().clone())),
                        notifier: Arc::new(social::commands::OsNotifier(app.handle().clone())),
                    }),
                    db.clone(),
                    Arc::new(social::listen::relay::Ntfy::new().map_err(error::AppError::Internal)?),
                    live.social.listen.relay.clone(),
                    social::listen::DEFAULT_RELAY.into(),
                );
                tauri::async_runtime::spawn(requests.clone().run());
                for ev in ["settings://changed", plex::types::EVENT_AUTH_CHANGED] {
                    let r = requests.clone();
                    app.listen_any(ev, move |_| r.poke());
                }
                asks_taps.set_requests(requests.clone());
                app.manage(requests);
            }
            // Listen along goes through a relay (ntfy), never computer to computer.
            let listen_name = social::commands::ListenName::default();
            let listen = social::listen::ListenAlong::new(social::listen::ListenDeps {
                player: Arc::new(player.clone()),
                relays: Arc::new(social::listen::relay::Ntfy::new().map_err(error::AppError::Internal)?),
                events: Arc::new(plex::state::TauriSink(app.handle().clone())),
                server: Arc::new(backend::routed::RoutedServerId(backends.clone())),
                library: Arc::new(social::commands::DbListenLibrary(db.clone())),
                offer: sharing.clone(),
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
            // Collaborative playlists and Blends: the same relay, sealed the same way.
            let circle_avatar: Arc<parking_lot::Mutex<Option<String>>> = Arc::default();
            let circle = social::circle::Circle::new(social::circle::CircleDeps {
                relays: Arc::new(social::listen::relay::Ntfy::new().map_err(error::AppError::Internal)?),
                events: Arc::new(plex::state::TauriSink(app.handle().clone())),
                store: Arc::new(social::circle::commands::KvStore(db.clone())),
                library: Arc::new(social::circle::commands::DbLibrary {
                    db: db.clone(),
                    liked_min: {
                        let db = db.clone();
                        Arc::new(move || settings::load(&db).liked_min_rating())
                    },
                }),
                playlists: Arc::new(social::circle::commands::AppPlaylists { db: db.clone(), playlists: circle_playlists }),
                people: Arc::new(social::circle::commands::PlexPeople {
                    plex: plex.clone(),
                    home: app.state::<plex::home::Home>().inner().clone(),
                    activity: Arc::new(backend::routed::RoutedActivity(backends.clone())),
                }),
                name: {
                    let plex_name = listen_name.0.clone();
                    let chosen = live.social.listen.name.clone();
                    Arc::new(move || {
                        let c = chosen.read().trim().to_string();
                        if c.is_empty() { plex_name.lock().clone() } else { c }
                    })
                },
                avatar: {
                    let a = circle_avatar.clone();
                    Arc::new(move || a.lock().clone())
                },
                settings: live.social.circle.clone(),
                default_relay: social::listen::DEFAULT_RELAY.into(),
                cfg: Default::default(),
                clock: Arc::new(remote::peer::auth::now_ms),
                tap: None,
            });
            {
                let circle = circle.clone();
                let plex = plex.clone();
                let name = listen_name.0.clone();
                tauri::async_runtime::spawn(async move {
                    // Who you are to others, then start (it listens only once
                    // there's an invite or someone on your server).
                    loop {
                        let a = plex.auth_status().await;
                        if let Some(n) = a.username.filter(|n| !n.trim().is_empty()) {
                            *name.lock() = n;
                        }
                        *circle_avatar.lock() = a.avatar;
                        circle.start().await;
                        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                    }
                });
            }
            app.manage(circle);
            social::commands::spawn_sharing((*sharing).clone(), plex.clone(), listen.clone(), db.clone());
            app.manage((*sharing).clone());
            app.manage(listen);
            app.manage(listen_name);
            remote.set_lan(remote::LanEnv {
                mdns: Arc::new(remote::lan::MdnsSd::new()),
                route: net_route.clone(),
                passwords: Arc::new(plex::keychain::KeyringStore::new()),
                tls: remote::cast::conn::tls_config()?,
            });
            remote.set_dlna(Arc::new(remote::dlna::Ssdp::new(audio::http::http_client())));
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
                    requests: Arc::new(discover_lidarr.clone()),
                    lookup: Arc::new(discover_lidarr),
                    prefs: Arc::new(discover::SettingsPrefs(db.clone())),
                    offline: plex.offline_switch(),
                    clock: Arc::new(db::queries::now_s),
                });
                app.manage(discover);
            }

            // The physical collection: owned albums in the sync store, and the
            // optional Discogs import (token in the keychain).
            let physical = physical::commands::Physical::new(
                db.clone(),
                sync_store.clone(),
                Arc::new(plex::keychain::KeyringStore::new()),
                physical::discogs::DiscogsClient {
                    http: artist_info.http(),
                    limiter: Arc::new(artistinfo::limit::Limiter::per_second(1.0)),
                },
                Arc::new(plex::state::TauriSink(app.handle().clone())),
                Arc::new(db::queries::now_s),
            );
            app.manage(physical.clone());

            // Sleeve notes: the files' tags, Plex, and with lookups on,
            // MusicBrainz (paced with the artist info) and Discogs.
            let sleeve = sleeve::service::SleeveService::new(db.clone(), artist_info.clone(), live.sleeve.clone());
            sleeve.set_discogs(Arc::new(physical));
            sleeve.set_plex(Arc::new(sleeve::commands::PlexSongCredits(plex.clone())));
            sleeve.set_events(Arc::new(app.handle().clone()));
            app.manage(sleeve);
            // Record sides: from a vinyl edition the notes read, or split by hand (synced).
            app.manage(sleeve::sides::Sides::new(db.clone(), sync_store.clone()));
            // "Same artist as...": two names counted as one artist in stats (synced).
            app.manage(stats::aliases::Aliases::new(sync_store.clone()));

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
                    fix_timing: live.lyrics_fix_timing.clone(),
                    released_ok: live.lyrics_released_ok.clone(),
                    official_first: live.lyrics_official_first.clone(),
                },
            );
            // Lyrics transcribed on this computer (opt-in, model downloaded on
            // request), and lyrics checked against the song with the same model.
            {
                use lyrics::transcribe::{self as tr, commands as trc};
                let models = tr::model::ModelStore::new(data_dir.join("models"), tr::model::catalogue(), tr::model::os_free_space());
                // Threads, the busy check and the breather, shared by both.
                let pacer = tr::pace::Pacer::new(Arc::new(tr::pace::SystemSignals::default()), tr::pace::Timing::default());
                // The graphics card, looked for once transcription is on.
                let gpu = tr::gpu::Gpu::for_this_build();
                gpu.set_mode(initial.transcribe_gpu);
                // One transcription helper (ends a minute after its last run),
                // on the graphics card while one is in use.
                let helper = |idle| {
                    let p = pacer.clone();
                    tr::helper::Helper::new(Arc::new(tr::helper::ProcessLaunch::current()), Arc::new(move || p.light()), idle)
                };
                let transcription = Arc::new(helper(tr::helper::IDLE_EXIT).with_gpu(gpu.clone()));
                let recognizer: Arc<dyn tr::engine::Recognizer> = transcription.clone();
                // The check runs ahead of background songs in this helper for
                // a bigger model (never loaded twice) and for anything on the
                // graphics card (one model on the card). The small model on
                // the processor has a short lane of its own.
                let check_recognizer: Arc<dyn tr::engine::Recognizer> = Arc::new(tr::gpu::CheckRoute {
                    gpu: gpu.clone(),
                    own: Arc::new(helper(tr::helper::CHECK_IDLE_EXIT).only(tr::model::ModelKind::Small)),
                    shared: transcription.priority(),
                });
                let songs: Arc<dyn tr::SongReader> = Arc::new(trc::AppSongs {
                    player: player.clone(),
                    audio_dir: audio_dir.clone(),
                    parts: Arc::new(backend::routed::RoutedParts(backends.clone())),
                    reader: tags::fetch::Reader::new(
                        audio::http::http_client(),
                        player.transfer_gate(),
                        Some(player.transfer_host()),
                    ),
                });
                let transcriber = tr::Transcriber::new(tr::Deps {
                    db: db.clone(),
                    models: models.clone(),
                    fetch: Arc::new(tr::model::HttpFetch::new()),
                    recognizer: recognizer.clone(),
                    songs: songs.clone(),
                    other: Arc::new(trc::AppOther(lyrics_service.clone())),
                    conditions: Arc::new(trc::AppConditions),
                    events: Arc::new(app.handle().clone()),
                    pacer: pacer.clone(),
                    gpu,
                    speeds: Arc::new(tr::speed::Speeds::load(Some(data_dir.join("models").join("speeds.json")))),
                    voice: cfg!(feature = "transcribe").then(|| tr::VoiceDeps {
                        separator: transcription.clone(),
                        models: tr::model::ModelStore::new(data_dir.join("models"), tr::voice::catalogue(), tr::model::os_free_space()),
                    }),
                });
                // Song analysis for Automix: at idle, with the same busy check.
                {
                    let analyzer = analysis::Analyzer::new(db.clone(), songs.clone(), Some(pacer.clone()));
                    let (handle, p, map) = (app.handle().clone(), player.clone(), sound_map.clone());
                    analyzer.set_on_done(Arc::new(move |ids: Vec<String>| {
                        use tauri::Emitter;
                        let _ = handle.emit(analysis::EV_CHANGED, &ids);
                        map.kick();
                        p.analysed(ids);
                    }));
                    analyzer.spawn();
                    let (p, a) = (player.clone(), analyzer.clone());
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = p.set_mix_source(Arc::new(a)).await {
                            tracing::warn!("automix has no analyses: {e}");
                        }
                    });
                    app.manage(analyzer);
                }
                transcriber.set_options(initial.transcribe_lyrics, initial.transcribe_model);
                // A paired Bridge doing Lyrics for every song: the background batch waits.
                transcriber.set_defer(Arc::new(move || bridge_for_lyrics.lyrics_takes_over()));
                transcriber.set_pace(initial.transcribe_processor, initial.transcribe_pause_busy, initial.transcribe_faster_away);
                *live.transcriber.lock() = Some(transcriber.clone());
                // Songs without lyrics are written down as they play.
                lyrics::transcribe::commands::follow_playback(transcriber.clone(), player.clone(), player.subscribe_tracking());
                tauri::async_runtime::spawn({
                    let t = transcriber.clone();
                    async move { t.spawn() }
                });
                let checker = lyrics::checker::Checker::new(lyrics::checker::CheckerDeps {
                    db: db.clone(),
                    models,
                    recognizer: check_recognizer,
                    audio: Arc::new(lyrics::checker::DecodedSongs(songs)),
                    events: Arc::new(app.handle().clone()),
                    enabled: live.lyrics_check_audio.clone(),
                    // Nothing found matched: transcribe the song, when transcription is on.
                    none_matched: Some({
                        let t = transcriber.clone();
                        Arc::new(move |id: &str| t.transcribe_quietly(id))
                    }),
                    pacer,
                });
                tauri::async_runtime::spawn({
                    let c = checker.clone();
                    async move { c.spawn() }
                });
                lyrics_service.set_checker(Arc::new(checker.clone()));
                app.manage(checker);
                app.manage(transcriber);
            }
            app.manage(lyrics_service);
            app.manage(lyrics::share::LyricsShare::new(lyrics::publish::Publisher::new()));

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
                        transcripts: live.sync_transcriptions.clone(),
                    },
                );
                if let Some(t) = live.transcriber.lock().as_ref() {
                    t.set_shared(Arc::new(sync::transcripts::AppShared {
                        store: store.clone(),
                        db: db.clone(),
                        on: live.sync_transcriptions.clone(),
                    }));
                }
                {
                    let (store, settings, db, engine) = (store.clone(), app_settings.clone(), db.clone(), engine.clone());
                    let mix_player = player.clone();
                    let transcripts = live.sync_transcriptions.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = settings.attach_sync(store.clone()).await {
                            tracing::warn!("synced settings not set up: {e}");
                        }
                        // Playlists with Mix on reach the player.
                        match store.run(sync::mixed::list).await {
                            Ok(m) => {
                                if let Err(e) = mix_player.set_mixed(m).await {
                                    tracing::warn!("mixed playlists not set: {e}");
                                }
                            }
                            Err(e) => tracing::warn!("mixed playlists not read: {e}"),
                        }
                        let db2 = db.clone();
                        if let Err(e) = store.run(move |s| sync::lyrics::seed(s, &db2)).await {
                            tracing::warn!("lyrics not added to the sync store: {e}");
                        }
                        // Transcriptions: this computer's go in, other devices' come out.
                        if transcripts.load(std::sync::atomic::Ordering::Relaxed) {
                            let r = store
                                .run(move |s| {
                                    sync::transcripts::seed(s, &db)?;
                                    sync::transcripts::apply(s, &db)
                                })
                                .await;
                            if let Err(e) = r {
                                tracing::warn!("transcriptions not synced: {e}");
                            }
                        }
                        engine.run().await;
                    });
                }
                app.manage(sc::SyncService { store, engine, keys, vault });
            }

            // Keep my devices in sync: paired devices swap the store's
            // entries directly, over the local network at home and through
            // the sealed relay away. No server, and the sync file stays
            // optional.
            {
                let id = plex.identity();
                let devices = sync::devices::commands::start(sync::devices::commands::Setup {
                    app: app.handle().clone(),
                    store: sync_store.clone(),
                    db: db.clone(),
                    remote: app.state::<remote::RemoteService>().inner().clone(),
                    groups: live.sync.clone(),
                    on: live.device_sync.clone(),
                    away: live.device_sync_away.clone(),
                    wifi_only: live.device_sync_wifi_only.clone(),
                    relays: Arc::new(social::listen::relay::Ntfy::new().map_err(error::AppError::Internal)?),
                    relay: live.social.listen.relay.clone(),
                    downloads: downloads.clone(),
                    platform: id.platform.clone(),
                    version: id.version.clone(),
                });
                // The home side: this computer answers Lidarr for the user's
                // other devices over their device links (while Let my other
                // devices use Lidarr through this computer is on), and Lidarr
                // here can go through a computer at home that does.
                {
                    let lidarr = app.state::<lidarr::Lidarr>().inner().clone();
                    let on = live.home_side_lidarr.clone();
                    devices.sync.peer.set_home(Some(Arc::new(homeside::computer::ComputerServes {
                        lidarr: lidarr.clone(),
                        db: db.clone(),
                        on: Arc::new(move || on.load(std::sync::atomic::Ordering::Relaxed)),
                    })));
                    let weak = Arc::downgrade(&devices.sync);
                    lidarr.set_computer_route(Some(Arc::new(move || weak.upgrade().and_then(|d| d.computer_route()))));
                }
                // Sign in once: Bridges and the user's other devices on the
                // same Plex account link by themselves, with no codes.
                {
                    let bridges = app.state::<Arc<homeside::desktop::Desktop>>().inner().clone();
                    let looker = homeside::account::Looker::new(
                        Arc::new(homeside::account::AppAccount { plex: plex.clone(), bridges: bridges.clone(), devices: devices.sync.clone() }),
                        Arc::new(social::listen::relay::Ntfy::new().map_err(error::AppError::Internal)?),
                        homeside::DEFAULT_RELAY,
                        Arc::new(sync::store::system_ms),
                    );
                    let again = looker.clone();
                    bridges.set_relook(Arc::new(move || again.look_now()));
                    let (keep, stop) = tokio::sync::watch::channel(false);
                    tauri::async_runtime::spawn(looker.clone().run(stop));
                    app.manage(AccountLinks { _looker: looker, _keep: keep });
                }
                app.manage(devices);
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

            app.manage::<tagedit::commands::Editor>(Arc::new(tagedit::editor::TagEditor::new(
                db.clone(),
                Arc::new(tagedit::plexside::AppPlex(plex.clone())),
                Arc::new(tagedit::write::NoHooks),
                Some(data_dir.join("tag-edits.json")),
            )));
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
            #[cfg(target_os = "ios")]
            carplay::ios::attach(app.handle());
            app.manage(audio::compare::CompareState::default());
            app.manage(app_settings);
            app.manage(live.platform.clone());
            #[cfg(desktop)]
            updates::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            import::commands::import_overview, import::commands::import_choose, import::commands::import_run, import::commands::import_review_queue, import::commands::import_review, import::commands::import_list_page, import::commands::import_missing, import::commands::import_missing_albums, import::commands::import_missing_album_add, import::commands::import_missing_album_remove, import::commands::import_open_link, import::commands::import_forget, import::commands::import_list_remove, import::commands::import_played, import::commands::import_played_hide, import::commands::import_played_hidden, import::commands::bring_status, import::commands::bring_read, import::commands::bring_adopt, import::commands::bring_get, import::commands::bring_lidarr_guesses, import::commands::bring_waiting_rows, import::commands::bring_read_dropped, import::account::commands::streaming_account_status, import::account::commands::streaming_account_sign_in, import::account::commands::streaming_account_read, import::account::commands::streaming_account_cancel, import::account::commands::streaming_account_sign_out, import::account::commands::streaming_account_client_id_set,
            platform::route_picker::show_route_picker,
            platform::haptics::haptic,
            audio::hifi::headphone_detect,
            platform::audio_route::audio_route,
            platform::status_bar::set_status_bar_style,
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
            subsonic::commands::subsonic_change_address,
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
            lyrics::commands::lyrics_candidates,
            lyrics::commands::lyrics_check_status,
            lyrics::commands::lyrics_choose,
            lyrics::commands::lyrics_look_again,
            lyrics::commands::lyrics_offset,
            lyrics::commands::lyrics_choices_count,
            lyrics::commands::lyrics_choices_clear,
            lyrics::commands::lyrics_pronounce,
            lyrics::commands::lyrics_translate,
            lyrics::share::lyrics_share_info,
            lyrics::share::lyrics_share_reviewed,
            lyrics::share::lyrics_share_publish,
            lyrics::share::lyrics_share_cancel,
            lyrics::transcribe::commands::transcribe_status,
            lyrics::transcribe::commands::transcribe_model_download,
            lyrics::transcribe::commands::transcribe_model_cancel,
            lyrics::transcribe::commands::transcribe_model_delete,
            lyrics::transcribe::commands::transcribe_track,
            lyrics::transcribe::commands::transcribe_album,
            lyrics::transcribe::commands::transcribe_unreleased,
            lyrics::transcribe::commands::transcribe_cancel,
            lyrics::transcribe::commands::transcribe_pause,
            lyrics::transcribe::commands::transcribe_redo,
            lyrics::transcribe::commands::transcribe_found_nothing,
            db::commands::sync_status,
            db::commands::sync_now,
            db::commands::home,
            db::commands::home_records,
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
            stats::commands::stats_overview, stats::commands::stats_span, stats::commands::stats_moments, stats::commands::stats_days, stats::commands::stats_item, stats::commands::stats_month_recap, stats::commands::stats_play_ids, stats::commands::stats_compare_summary, stats::commands::stats_compare, stats::commands::stats_friends, stats::commands::stats_friend_remove, stats::commands::top_songs_index, stats::commands::top_songs_list, stats::commands::stats_wrapped, stats::aliases::artist_aliases, stats::aliases::artist_alias_set, stats::aliases::artist_alias_apart, stats::aliases::artist_alias_auto_pairs, stats::aliases::stats_played_artists,
            db::commands::playlist_page,
            stats::commands::home_on_this_day,
            stats::commands::home_time_travel,
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
            audio::commands::signal_path,
            audio::commands::visualizer_watch,
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
            settings::settings_defaults,
            settings::settings_export,
            settings::settings_import,
            settings::help_open_link,
            settings::open_project_page,
            themes::themes_list,
            themes::themes_import,
            themes::themes_open_folder,
            themes::themes_import_text,
            themes::themes_save,
            themes::themes_export,
            artistinfo::commands::artist_info,
            artistinfo::commands::album_credits,
            sleeve::commands::sleeve_notes,
            sleeve::commands::song_credits,
            sleeve::commands::credit_page,
            digging::commands::crates,
            digging::commands::crate_page,
            digging::commands::album_crates,
            sleeve::sides::album_sides,
            sleeve::sides::album_sides_set,
            diary::diary_entries,
            diary::diary_entry_edit,
            diary::diary_entry_delete,
            diary::diary_log_album,
            diary::album_first_heard,
            diary::diary_export,
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
            analysis::commands::analysis_status,
            analysis::commands::automix_mixed_get,
            analysis::commands::automix_mix_set,
            analysis::commands::automix_style_set,
            analysis::commands::automix_view,
            soundid::commands::sound_id_key_set,
            soundid::commands::track_sounds_like,
            plexedit::commands::plex_edit_merge_albums,
            feed::commands::feed_status,
            feed::deck::commands::deck_status,
            feed::deck::commands::deck_revoke,
            feed::deck::commands::deck_ui_report,
            feed::deck::commands::automation_revoke,
            feed::deck::commands::deck_pair_prompt,
            feed::deck::commands::deck_pair_answer,
            feed::deck::commands::deck_plugin_status,
            feed::deck::commands::deck_plugin_install,
            feed::deck::commands::deck_app_download,
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
            devices::commands::devices_list,
            devices::commands::devices_add,
            devices::commands::devices_choose_folder,
            devices::commands::devices_update,
            devices::commands::devices_select,
            devices::commands::devices_forget,
            devices::commands::devices_sync,
            devices::commands::devices_pause,
            devices::commands::devices_cancel,
            devices::commands::devices_status,
            soundmap::commands::soundmap_info,
            soundmap::commands::soundmap_points,
            soundmap::commands::soundmap_find,
            soundmap::commands::soundmap_song,
            #[cfg(desktop)]
            updates::update_check,
            #[cfg(desktop)]
            updates::update_install_on_restart,
            #[cfg(desktop)]
            updates::update_put_off,
            share::commands::share_info,
            share::commands::share_public_link,
            share::commands::deep_link_take,
            share::commands::share_card_qr,
            share::commands::share_card_copy,
            share::commands::share_card_save,
            platform::discord::commands::discord_status,
            platform::discord::commands::discord_imgur_key_set,
            tags::commands::tags_index,
            tagedit::commands::tagedit_pick_music_folder,
            tagedit::commands::tagedit_set_music_folder,
            tagedit::commands::tagedit_clear_music_folder,
            tagedit::commands::tagedit_music_folder_status,
            tagedit::commands::tagedit_open,
            tagedit::commands::tagedit_save,
            tagedit::commands::tagedit_undo,
            tagedit::commands::tagedit_history,
            tagedit::commands::tagedit_check,
            tagedit::commands::tagedit_check_fine,
            tagedit::commands::tagedit_fix_preview,
            tagedit::commands::tagedit_isrc_releases,
            tagedit::commands::tagedit_isrc_preview,
            tagedit::commands::tagedit_explicit_preview,
            tagedit::commands::tagedit_library_settings,
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
            social::circle::commands::circle_state,
            social::circle::commands::circle_people,
            social::circle::commands::circle_join,
            social::circle::commands::circle_accept,
            social::circle::commands::circle_dismiss,
            social::circle::commands::collab_info,
            social::circle::commands::collab_invite_link,
            social::circle::commands::collab_invite_person,
            social::circle::commands::collab_page,
            social::circle::commands::collab_add,
            social::circle::commands::collab_remove,
            social::circle::commands::collab_leave,
            social::circle::commands::collab_set_in_library,
            social::circle::commands::collab_set_role,
            social::circle::commands::collab_remove_person,
            social::circle::commands::collab_set_public,
            social::circle::commands::blend_create,
            social::circle::commands::blend_invite_link,
            social::circle::commands::blend_invite_person,
            social::circle::commands::blend_page,
            social::circle::commands::blend_hide,
            social::circle::commands::blend_refresh,
            social::circle::commands::blend_leave,
            social::commands::listen_offer_server,
            social::commands::sharing_overview,
            social::commands::sharing_share,
            social::commands::sharing_people,
            social::commands::sharing_remove,
            social::commands::sharing_link,
            social::commands::sharing_open_link,
            social::commands::sharing_link_asks,
            social::commands::sharing_link_approve,
            social::commands::sharing_link_deny,
            social::commands::sharing_invites,
            social::commands::sharing_accept,
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
            sync::devices::commands::device_sync_status,
            sync::devices::commands::device_sync_now,
            sync::devices::commands::device_sync_forget,
            sync::devices::commands::device_sync_quarantined,
            sync::devices::commands::device_sync_take_quarantined,
            sync::devices::commands::device_pair_start,
            sync::devices::commands::device_pair_cancel,
            sync::devices::commands::device_pair_confirm,
            sync::devices::commands::device_pair_state,
            sync::devices::commands::device_pair_join,
            sync::devices::commands::device_download_suggestions,
            sync::devices::commands::device_download_dismiss,
            sync::devices::commands::pair_copy_settings,
            sync::devices::commands::pair_copy_cancel,
            sync::devices::commands::device_sync_network,
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
            smart::commands::smart_describe,
            smart::commands::ai_status,
            smart::commands::ai_key_set,
            scrobble::commands::scrobble_status,
            scrobble::commands::lastfm_secret_set,
            scrobble::commands::lastfm_sign_in_start,
            scrobble::commands::lastfm_sign_in_finish,
            scrobble::commands::lastfm_sign_out,
            scrobble::commands::listenbrainz_check,
            homeside::commands::bridge_overview,
            homeside::commands::bridge_pair_start,
            homeside::commands::bridge_pair_cancel,
            homeside::commands::bridge_join,
            homeside::commands::bridge_unpair,
            homeside::commands::bridge_queued_apply,
            homeside::commands::bridge_queued_dismiss,
            homeside::commands::bridge_send_settings,
            homeside::commands::lidarr_home_sides,
            lidarr::commands::lidarr_status,
            lidarr::commands::lidarr_key_set,
            lidarr::commands::lidarr_headers_set,
            lidarr::commands::lidarr_check,
            lidarr::commands::lidarr_confirm_root,
            lidarr::commands::lidarr_pick,
            lidarr::commands::lidarr_request_preview,
            lidarr::commands::lidarr_request,
            lidarr::commands::lidarr_get,
            lidarr::commands::lidarr_tap,
            asks::commands::asks_view,
            asks::commands::asks_request,
            asks::commands::asks_cancel,
            asks::commands::asks_incoming,
            asks::commands::asks_answer,
            lidarr::commands::lidarr_taps,
            lidarr::commands::lidarr_last_get,
            lidarr::commands::lidarr_search_again,
            lidarr::commands::get_albums_unqueue,
            lidarr::commands::lidarr_request_batch,
            lidarr::commands::bring_in_status,
            lidarr::commands::bring_in_pause,
            lidarr::commands::bring_in_needs_you,
            lidarr::commands::lidarr_requests,
            lidarr::commands::lidarr_request_cancel,
            lidarr::commands::lidarr_missing_releases,
            lidarr::commands::lidarr_upgrade_album,
            lidarr::commands::lidarr_upgrade_request,
            lidarr::commands::lidarr_upgrade_bulk,
            lidarr::commands::lidarr_upgrades,
            lidarr::commands::lidarr_upgrade_offer,
            lidarr::commands::lidarr_upgrade_swap,
            lidarr::commands::lidarr_upgrade_undo,
            lidarr::commands::lidarr_upgrade_clear,
            lidarr::commands::lidarr_upgrade_candidates,
            lidarr::commands::lidarr_upgrade_confirm,
            lidarr::commands::lidarr_allowlist,
            lidarr::commands::lidarr_allowlist_set,
            lidarr::commands::lidarr_modes,
            lidarr::commands::lidarr_mode_set,
            lidarr::commands::lidarr_artist_choices,
            lidarr::commands::lidarr_artist_choices_set,
            lidarr::commands::lidarr_change_preview,
            lidarr::commands::lidarr_hires_preview,
            lidarr::commands::lidarr_change_apply,
            lidarr::commands::lidarr_changes,
            lidarr::commands::lidarr_change_put_back,
            lidarr::commands::lidarr_rematch_preview,
            lidarr::commands::lidarr_rematch_apply,
            lidarr::commands::lidarr_work,
            lidarr::commands::lidarr_catch_up,
            lidarr::commands::lidarr_resume_protected,
            lidarr::commands::lidarr_staged_add,
            lidarr::commands::lidarr_staged_undo,
            lidarr::commands::lidarr_hunt,
            lidarr::commands::music_share_get,
            lidarr::commands::music_share_set,
            lidarr::commands::music_share_plex_folders,
            radar::commands::release_radar,
            discover::commands::catalogue_search,
            discover::commands::catalogue_artist,
            discover::commands::catalogue_album,
            discover::commands::catalogue_fans_also_like,
            discover::commands::new_to_you,
            discover::commands::catalogue_search_part,
            discover::commands::catalogue_artist_photo,
            discover::commands::featured_artist,
            discover::commands::featured_artist_photo,
            discover::commands::catalogue_get_albums_plan,
            discover::commands::catalogue_get_albums,
            physical::commands::physical_list,
            physical::commands::physical_set,
            physical::commands::physical_stats,
            physical::commands::physical_find_release,
            physical::commands::discogs_status,
            physical::commands::discogs_token_set,
            physical::commands::discogs_import,
            playhistory::commands::play_history_status,
            playhistory::commands::play_history_update,
            playhistory::commands::play_history_cancel,
            playhistory::commands::play_history_remove,
            playhistory::commands::tautulli_status,
            playhistory::commands::tautulli_key_set,
            radar::commands::release_radar_check,
            radar::commands::radar_auto_artists,
            radar::commands::radar_auto_set,
            platform::miniplayer::miniplayer_state,
            platform::miniplayer::miniplayer_set,
            platform::miniplayer::miniplayer_go,
            platform::reveal::window_ready,
        ])
        .on_window_event(|window, event| {
            platform::miniplayer::on_window_event(window, event);
            platform::visibility::on_window_event(window, event);
            platform::reveal::on_window_event(window, event);
            // Back in front: look for an update if the last check is old.
            #[cfg(desktop)]
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
            #[cfg(desktop)]
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
            // Back in front (the phone opened again): catch up with the
            // other devices now. While playing it keeps running and catches
            // up as it goes.
            #[cfg(any(target_os = "ios", target_os = "android"))]
            if let tauri::RunEvent::Resumed = event
                && let Some(d) = app.try_state::<sync::devices::commands::DeviceService>()
            {
                d.sync.kick();
            }
            // Quitting from the Dock, logging out or shutting down on a Mac
            // skips ExitRequested: install a waiting update here too (a
            // second call finds nothing waiting).
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Exit = event {
                updates::quitting(app, None);
            }
            if let tauri::RunEvent::ExitRequested { code, .. } = event {
                // So a log that ends shows whether the app closed or stopped.
                tracing::info!("Platter is closing");
                platform::reveal::save(app);
                let player = app.state::<PlayerHandle>().inner().clone();
                tauri::async_runtime::block_on(player.flush());
                // A hosted session ends for everyone; a guest says it left.
                if let Some(listen) = app.try_state::<social::listen::ListenAlong>() {
                    let listen = listen.inner().clone();
                    tauri::async_runtime::block_on(async move {
                        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), listen.stop()).await;
                    });
                }
                // Last: on Windows the installer takes over and the process ends here.
                #[cfg(desktop)]
                updates::quitting(app, code);
                #[cfg(mobile)]
                let _ = code;
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

    #[test]
    fn a_scan_counts_once_and_only_when_it_moves_forward() {
        // First look: compared with the last sync.
        assert!(scanned_since(Some(200), None, Some(100)));
        assert!(!scanned_since(Some(100), None, Some(200)));
        assert!(scanned_since(Some(100), None, None));
        // Later looks: only a newer scan than the last one seen.
        assert!(!scanned_since(Some(200), Some(200), Some(100)));
        assert!(scanned_since(Some(201), Some(200), Some(100)));
        // A server clock far ahead of this computer's: one sync, not one per look.
        assert!(scanned_since(Some(9_000), None, Some(100)));
        assert!(!scanned_since(Some(9_000), Some(9_000), Some(150)));
        // The server didn't say.
        assert!(!scanned_since(None, Some(5), Some(1)));
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
    fn the_watcher_syncs_now_and_then_even_with_the_socket_up() {
        let s = Duration::from_secs;
        // Socket up, nothing pending: the hourly sync, not a poll.
        assert_eq!(library_wait(None, false, s(0), false), SAFETY_EVERY);
        assert_eq!(library_wait(None, true, s(60), false), FALLBACK_EVERY - s(60));
        assert_eq!(library_wait(None, false, s(60), false), SAFETY_EVERY - s(60));
        assert_eq!(library_wait(None, false, SAFETY_EVERY * 2, false), s(1));
        // A change waits out the quiet period; a due sync that couldn't
        // start retries in seconds.
        assert_eq!(library_wait(Some(s(2)), false, s(0), false), QUIET - s(2));
        assert_eq!(library_wait(Some(s(10)), false, s(0), true), s(5));
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
