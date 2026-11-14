//! One local entry point: web UI, fingerprint signing, and a managed NEAR sandbox.

mod api;
mod capture;
mod chain;
mod fingerprint;
mod model;
mod runtime;
mod service;

use anyhow::{Context, Result, bail};

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
    let port = port()?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .with_context(|| format!("cannot listen on http://127.0.0.1:{port}"))?;
    let app = service::App::new(port)?;
    app.start();

    println!("Gnorix transaction signing demo: http://127.0.0.1:{port}");
    println!("Starting local contracts and fingerprint enrollments. The page shows progress.");
    let result = axum::serve(listener, api::router(app.clone()))
        .with_graceful_shutdown(shutdown_signal())
        .await;
    app.stop().await;
    result.context("HTTP server failed")
}

fn port() -> Result<u16> {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next(), args.next()) {
        (None, None, None) => Ok(8780),
        (Some("--port"), Some(value), None) => {
            let port: u16 = value.parse().context("port must be within 1..=65535")?;
            if port == 0 {
                bail!("port must be within 1..=65535");
            }
            Ok(port)
        }
        _ => bail!("usage: gnorix-near-demo [--port PORT]"),
    }
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
