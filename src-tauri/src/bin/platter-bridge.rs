//! Platter Bridge: Platter's home side without a window, for a server or a
//! container (docs/BRIDGE.md). `platter-bridge` runs it; `platter-bridge
//! health` asks a running one whether it's well (Docker's healthcheck).

use std::sync::Arc;

use platter_lib::homeside::{health, run};

fn main() {
    let args: Vec<String> = std::env::args().collect();
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
    tracing::info!("Music folder: {}", match parts.music {
        platter_lib::homeside::wire::MusicAccess::ReadWrite => "mounted read-write, so new protected albums can be added",
        platter_lib::homeside::wire::MusicAccess::ReadOnly => "mounted read-only, so additions wait for a desktop",
        platter_lib::homeside::wire::MusicAccess::None => "not mounted, so additions wait for a desktop",
    });
    let plex = parts.plex.clone();
    let bridge = run::Bridge::new(parts);
    if let Some(p) = plex
        && let Err(e) = p.restore().await
    {
        tracing::info!("Plex: {e}");
    }
    let h = bridge.clone();
    tokio::spawn(async move {
        if let Err(e) = health::serve(h, port).await {
            tracing::warn!("The health check couldn't listen on 127.0.0.1:{port}: {e}");
        }
    });
    if bridge.wire.links.all().is_empty() || bridge.p.env.pair_code.is_some() {
        match bridge.pair_from_env().await {
            Ok(true) | Ok(false) => {}
            Err(e) => tracing::error!("Pairing: {e}"),
        }
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
