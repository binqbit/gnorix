//! Local interactive fixture laboratory. Run in release mode for extraction.

#[path = "support/authorization_json.rs"]
#[allow(
    dead_code,
    reason = "HTTP supplies actions; the shared codec also supports CLI action files"
)]
mod authorization_json;
#[path = "support/enrollment_profile.rs"]
mod enrollment_profile;
#[path = "support/output.rs"]
mod output;
#[path = "support/example_crypto.rs"]
mod provider;

#[path = "fingerprint_lab/api.rs"]
mod api;
#[path = "fingerprint_lab/artifacts.rs"]
mod artifacts;
#[path = "fingerprint_lab/capture.rs"]
mod capture;
#[path = "fingerprint_lab/engine.rs"]
mod engine;
#[path = "fingerprint_lab/model.rs"]
mod model;
#[path = "fingerprint_lab/resources.rs"]
mod resources;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> Result<()> {
    let port = port()?;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let image =
        image::load_from_memory(include_bytes!("../tests/assets/test-fingerprint.png"))?.to_luma8();
    let engine = engine::Engine::new(image, root.join("reports/fingerprint-lab"))?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let address = listener.local_addr()?;
    let app = api::router(engine, address.port())?;
    println!("Gnorix fingerprint lab: http://{address}\nStop with Ctrl+C.");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

fn port() -> Result<u16> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => Ok(8765),
        [flag, value] if flag == "--port" => Ok(value.parse()?),
        _ => Err("usage: fingerprint_lab [--port <port>]".into()),
    }
}
