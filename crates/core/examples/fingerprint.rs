//! Extract one PNG capture to inspectable local stages and graph geometry.

#[path = "fingerprint/clarity.rs"]
mod clarity;
#[path = "fingerprint/cleanup.rs"]
mod cleanup;
#[path = "fingerprint/contours.rs"]
mod contours;
#[path = "fingerprint/features.rs"]
mod features;
#[path = "support/image_input.rs"]
mod image_input;
#[path = "fingerprint/images.rs"]
mod images;
#[path = "fingerprint/snapshot.rs"]
mod snapshot;

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gnorix::biometrics::{
    Capture,
    fingerprint::{self, ExtractionConfig, Fingerprint},
};
use image::GrayImage;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (input, output) = arguments()?;
    if output.exists() {
        return Err("output directory already exists".into());
    }
    let config = ExtractionConfig::default();
    let source = image_input::read(&input, config.max_pixels)?;
    let capture = Capture::new(
        source.width(),
        source.height(),
        source.as_raw().clone(),
        None,
        None,
    )?;

    let started = Instant::now();
    let result = fingerprint::extract(&capture, &config)?;
    let elapsed = started.elapsed();
    fs::create_dir_all(&output)?;
    save_bundle(&output, &source, &result, &config)?;
    print_summary(&result, elapsed, &output);
    Ok(())
}

fn print_summary(result: &Fingerprint, elapsed: Duration, output: &Path) {
    println!(
        "Full-source unit: {:.3}px; {:.3}px measured / {:.3}px traversed ridge length; {} / {} links measured",
        result.scale.unit_px,
        result.scale.coverage.measured_length_px,
        result.scale.coverage.total_length_px,
        result.scale.coverage.measured_links,
        result.scale.coverage.total_links,
    );
    println!(
        "Provisional bootstrap: period {:.3}px from {} profiles; final width observations come from the full pass",
        result.bootstrap.ridge_period_px,
        result.bootstrap.profiles.len(),
    );
    println!(
        "{} nodes, {} edges, {} isolated cycles; {:.3}s extraction\n{}",
        result.graph.nodes.len(),
        result.graph.edges.len(),
        result.graph.cycles.len(),
        elapsed.as_secs_f64(),
        output.display()
    );
    contours::print_summary(&result.contours);
    features::print_summary(&result.feature_strength);
}

fn arguments() -> Result<(PathBuf, PathBuf), Box<dyn Error + Send + Sync>> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        return Err("usage: fingerprint <input.png> <new-output-directory>".into());
    }
    Ok((PathBuf::from(&arguments[0]), PathBuf::from(&arguments[1])))
}

fn save_bundle(
    root: &Path,
    source: &GrayImage,
    result: &Fingerprint,
    config: &ExtractionConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    images::save(root, source, result)?;
    let snapshot = snapshot::build(result, config)?;
    fs::write(
        root.join("features.json"),
        serde_json::to_vec_pretty(&snapshot)?,
    )?;
    Ok(())
}
