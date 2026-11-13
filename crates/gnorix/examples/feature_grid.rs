//! Stream complete positional variants from an existing feature-combination report.

#[path = "feature_grid/input.rs"]
mod input;
#[path = "feature_grid/output.rs"]
mod output;
#[cfg(test)]
#[path = "feature_grid/tests.rs"]
mod tests;

use std::{collections::BTreeMap, env, error::Error, fs::File, path::PathBuf, str::FromStr};

use gnorix::biometrics::fingerprint::{self, GridShapeConfig};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const MAX_JSON_BYTES: usize = 64 * 1024 * 1024;
const ENVIRONMENT: [&str; 3] = [
    "GNORIX_GRID_UNCERTAINTY_STEPS",
    "GNORIX_GRID_MAX_VARIANTS",
    "GNORIX_GRID_MAX_WORK",
];

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

fn configuration() -> Result<GridShapeConfig> {
    let mut values = BTreeMap::new();
    for name in ENVIRONMENT {
        match env::var(name) {
            Ok(value) => {
                values.insert(name, value);
            }
            Err(env::VarError::NotPresent) => {}
            Err(env::VarError::NotUnicode(_)) => {
                return Err(format!("{name} must contain UTF-8 numeric text").into());
            }
        }
    }
    parse_configuration(|name| values.get(name).map(String::as_str))
}

fn parse_configuration<'a>(value: impl Fn(&str) -> Option<&'a str>) -> Result<GridShapeConfig> {
    let config = GridShapeConfig {
        uncertainty_steps: number(ENVIRONMENT[0], value(ENVIRONMENT[0]), 1)?,
        max_variants: number(ENVIRONMENT[1], value(ENVIRONMENT[1]), 1_000_000)?,
        max_work: number(ENVIRONMENT[2], value(ENVIRONMENT[2]), 10_000_000)?,
    };
    config.validate()?;
    Ok(config)
}

fn number<T: FromStr>(name: &str, value: Option<&str>, default: T) -> Result<T> {
    match value {
        None => Ok(default),
        Some(raw) => raw
            .parse()
            .map_err(|_| format!("invalid numeric value for {name}").into()),
    }
}
