//! Stream complete positional variants from an existing feature-combination report.

#[path = "support/grid_config.rs"]
mod grid_config;
#[path = "feature_grid/input.rs"]
mod input;
#[path = "feature_grid/output.rs"]
mod output;
#[cfg(test)]
#[path = "feature_grid/tests.rs"]
mod tests;

use std::{env, error::Error, fs::File, path::PathBuf};

use gnorix::biometrics::fingerprint;
use grid_config::read as configuration;
#[cfg(test)]
use grid_config::{ENVIRONMENT, parse as parse_configuration};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const MAX_JSON_BYTES: usize = 64 * 1024 * 1024;

fn main() -> Result<()> {
    let (input_path, output_path) = arguments()?;
    output::require_new_path(&output_path)?;
    let config = configuration()?;
    let input = input::read(&mut File::open(&input_path)?, MAX_JSON_BYTES)?;
    let prepared = fingerprint::prepare_grid_shapes(
        &input.points,
        &input.combinations,
        input.unit_px,
        &config,
    )?;
    let manifest = output::manifest(&input_path, &input, &config, &prepared);
    let written = output::write_new(&output_path, &manifest, &prepared, MAX_JSON_BYTES)?;
    println!(
        "{} shapes; {} variants; {} colliding variants; {} bytes\n{}",
        prepared.shapes().len(),
        written.variants,
        written.colliding_variants,
        written.bytes,
        output_path.display(),
    );
    Ok(())
}

fn arguments() -> Result<(PathBuf, PathBuf)> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        return Err("usage: feature_grid <input.json> <new-output.json>".into());
    }
    Ok((PathBuf::from(&arguments[0]), PathBuf::from(&arguments[1])))
}
