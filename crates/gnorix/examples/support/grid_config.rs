//! Explicit controls for the complete positional-grid diagnostic export.

use std::{collections::BTreeMap, env, str::FromStr};

use gnorix::biometrics::fingerprint::GridShapeConfig;

use super::Result;

pub(super) const ENVIRONMENT: [&str; 3] = [
    "GNORIX_GRID_UNCERTAINTY_STEPS",
    "GNORIX_GRID_MAX_VARIANTS",
    "GNORIX_GRID_MAX_WORK",
];

pub(super) fn read() -> Result<GridShapeConfig> {
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
    parse(|name| values.get(name).map(String::as_str))
}

pub(super) fn parse<'a>(value: impl Fn(&str) -> Option<&'a str>) -> Result<GridShapeConfig> {
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
