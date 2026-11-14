//! Local interactive fixture laboratory. Run in release mode for extraction.

use gnorix_example_support::{
    authorization_json, enrollment_profile, example_crypto as provider, output,
};

mod api;
mod engine;
mod gallery;
mod model;
mod resources;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let options = options(std::env::args().skip(1), &root)?;
    let gallery = gallery::Gallery::load(&options.gallery)?;
    let engine = engine::Engine::new(
        gallery.default_image().image.clone(),
        root.join("reports/fingerprint-lab"),
    )?;
    let listener =
        tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, options.port)).await?;
    let address = listener.local_addr()?;
    let app = api::router(engine, gallery, address.port())?;
    println!("Gnorix fingerprint lab: http://{address}\nStop with Ctrl+C.");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

struct Options {
    port: u16,
    gallery: std::path::PathBuf,
}

fn options(mut args: impl Iterator<Item = String>, root: &std::path::Path) -> Result<Options> {
    let mut port = None;
    let mut gallery = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--port" if port.is_none() => {
                port = Some(args.next().ok_or("--port needs a value")?.parse()?)
            }
            "--gallery" if gallery.is_none() => {
                gallery = Some(std::path::PathBuf::from(
                    args.next().ok_or("--gallery needs a directory")?,
                ))
            }
            _ => {
                return Err("usage: gnorix-lab [--port <port>] [--gallery <directory>]".into());
            }
        }
    }
    Ok(Options {
        port: port.unwrap_or(8765),
        gallery: gallery.unwrap_or_else(|| root.join("test-data/fingerprint-gallery")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_options_keep_default_fixture_and_accept_explicit_gallery() {
        let root = std::path::Path::new("project");
        let defaults = options(std::iter::empty(), root).unwrap();
        assert_eq!(defaults.port, 8765);
        assert_eq!(defaults.gallery, root.join("test-data/fingerprint-gallery"));
        let args = ["--gallery", "local research", "--port", "9000"].map(str::to_owned);
        let configured = options(args.into_iter(), root).unwrap();
        assert_eq!(configured.port, 9000);
        assert_eq!(configured.gallery, std::path::Path::new("local research"));
        for args in [
            vec!["--gallery"],
            vec!["--unknown"],
            vec!["--port", "1", "--port", "2"],
        ] {
            assert!(options(args.into_iter().map(str::to_owned), root).is_err());
        }
    }
}
