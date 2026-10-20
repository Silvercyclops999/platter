//! Platter Bridge: Platter's home side without a window, for a server or a
//! container (docs/BRIDGE.md). `platter-bridge` runs it; `platter-bridge
//! health` asks a running one whether it's well (Docker's healthcheck);
//! `platter-bridge status` prints what it's doing and its pair code.
//! `platter-bridge --speech-worker` is the speech helper the Bridge starts
//! for Lyrics for every song (as Platter on a computer does).

use std::sync::Arc;

use platter_lib::homeside::{health, run};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // The speech helper: before anything else starts.
    if let Some(code) = platter_lib::speech_worker(&args) {
        std::process::exit(code);
    }
    let env = run::Env::from_env();
    let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Platter Bridge couldn't start: {e}");
            std::process::exit(1);
        }
    };
    if args.get(1).map(String::as_str) == Some("health") {
        let ok = rt.block_on(health::check(env.health_port));
        std::process::exit(if ok { 0 } else { 1 });
    }
    if args.get(1).map(String::as_str) == Some("status") {
        match rt.block_on(health::status(env.health_port)) {
            Some(text) => {
                print!("{text}");
                std::process::exit(0);
            }
            None => {
                eprintln!("The Bridge isn't running here (nothing answered on 127.0.0.1:{}).", env.health_port);
                std::process::exit(1);
            }
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("BRIDGE_LOG").unwrap_or_else(|_| "info,platter_lib::db=warn".into()))
        .with_ansi(false)
        .with_target(false)
        .init();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    std::process::exit(rt.block_on(start(env)));
}

async fn start(env: run::Env) -> i32 {
    tracing::info!("Platter Bridge {} starting. Data folder: {}", platter_lib::homeside::VERSION, env.data.display());
    let port = env.health_port;
    let parts = match run::build(env) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Couldn't start: {e}");
            return 1;
        }
    };
    let mounted_unnamed = parts.env.music.is_none() && parts.env.music_dir.is_dir();
    tracing::info!("Music folder: {}", match parts.music {
        platter_lib::homeside::wire::MusicAccess::ReadWrite => "mounted read-write, so new protected albums can be added".to_string(),
        platter_lib::homeside::wire::MusicAccess::ReadOnly => "mounted read-only, so additions wait for a desktop".to_string(),
        // Mounted, but without MUSIC_PLEX_PATH the Bridge can't match it to Plex yet.
        platter_lib::homeside::wire::MusicAccess::None if mounted_unnamed => format!(
            "mounted at {}, but MUSIC_PLEX_PATH isn't set yet, so the Bridge waits for its suggestion once it has read Plex",
            parts.env.music_dir.display()
        ),
        platter_lib::homeside::wire::MusicAccess::None => "not mounted, so additions wait for a desktop".to_string(),
    });
    if !parts.env.set_here().is_empty() {
        tracing::info!("Using the Bridge's own settings for: {}. What a computer sends for these is ignored.", parts.env.set_here().join(", "));
    }
    // Lyrics for every song: the speech model and the queue, from the mounted music folder.
    let lyrics = platter_lib::homeside::lyrics::real(
        &parts.env.data,
        parts.db.clone(),
        parts.store.clone(),
        parts.env.music_dir.clone(),
        parts.env.music.as_ref().map(|(root, _)| root.clone()),
        parts.plex.clone(),
    );
    let bridge = run::Bridge::new(parts);
    match lyrics {
        Ok(l) => {
            l.spawn();
            bridge.set_lyrics(l);
        }
        Err(e) => tracing::warn!("Lyrics for every song can't run here: {e}"),
    }
    let h = bridge.clone();
    tokio::spawn(async move {
        if let Err(e) = health::serve(h, port).await {
            tracing::warn!("The health check couldn't listen on 127.0.0.1:{port}: {e}");
        }
    });
    // A used or expired code is said once and skipped; only a fresh one pairs.
    if let Err(e) = bridge.pair_from_env().await {
        tracing::error!("Pairing: {e}");
    }
    let (stop_tx, stop) = tokio::sync::watch::channel(false);
    let runner = tokio::spawn(Arc::clone(&bridge).run(stop));
    stopped().await;
    tracing::info!("Stopping.");
    let _ = stop_tx.send(true);
    let _ = runner.await;
    0
}

/// Ctrl-C, or `docker stop` (SIGTERM).
async fn stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
