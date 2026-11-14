//! One local entry point: web UI, fingerprint signing, and a managed NEAR sandbox.

mod api;
mod capture;
mod chain;
mod config;
mod fingerprint;
mod model;
mod runtime;
mod service;
mod sessions;

use anyhow::{Context, Result};

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run());
    // Fingerprint extraction is CPU-bound and cannot be interrupted mid-stage.
    // The chain is already dropped by stop(); do not make process exit wait for
    // a detached extraction worker after Ctrl+C.
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    result
}

async fn run() -> Result<()> {
    let config = config::Config::parse(std::env::args().skip(1))?;
    if let Some(points) = config.points_per_combination {
        println!(
            "Fingerprint test policy: exactly {points} points per combination, at least 3 anchors separated by 5R"
        );
    }
    let port = config.port;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .with_context(|| format!("cannot listen on http://127.0.0.1:{port}"))?;
    if let Some(origin) = config.public_origin() {
        println!("Public reverse-proxy origin: {origin}");
    }
    let app = sessions::Demo::new(config)?;
    app.start();

    println!("Gnorix transaction signing demo: http://127.0.0.1:{port}");
    println!("Preparing the shared NEAR runtime. The page creates your one-hour wallet session.");
    let result = axum::serve(listener, api::router(app.clone()))
        .with_graceful_shutdown(shutdown_signal())
        .await;
    app.stop().await;
    result.context("HTTP server failed")
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install termination signal handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
