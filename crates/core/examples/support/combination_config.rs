//! Explicit regional enrollment controls shared by the local examples.

use gnorix_core::biometrics::fingerprint::{CombinationConfig, SpatialPolicy};
use std::{collections::BTreeMap, env, error::Error, str::FromStr};

pub(super) const ENVIRONMENT: [&str; 7] = [
    "GNORIX_COMBINATION_MIN_POINTS",
    "GNORIX_COMBINATION_MAX_POINTS",
    "GNORIX_COMBINATION_RADIUS_UNITS",
    "GNORIX_COMBINATION_MAX_WORK",
    "GNORIX_COMBINATION_MIN_PRIORITY",
    "GNORIX_COMBINATION_MIN_ANCHOR_POINTS",
    "GNORIX_COMBINATION_MIN_ANCHOR_DISTANCE_UNITS",
];

pub(super) fn read() -> Result<CombinationConfig, Box<dyn Error + Send + Sync>> {
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

pub(super) fn parse_configuration<'a>(
    value: impl Fn(&str) -> Option<&'a str>,
) -> Result<CombinationConfig, Box<dyn Error + Send + Sync>> {
    // Small example settings exercise the mechanism; applications supply their own policy.
    let config = CombinationConfig {
        min_points: number(ENVIRONMENT[0], value(ENVIRONMENT[0]), 3)?,
        max_points: number(ENVIRONMENT[1], value(ENVIRONMENT[1]), 6)?,
        min_priority: number(ENVIRONMENT[4], value(ENVIRONMENT[4]), 0.5)?,
        spatial: SpatialPolicy {
            min_anchor_points: number(ENVIRONMENT[5], value(ENVIRONMENT[5]), 3)?,
            min_anchor_distance_units: number(ENVIRONMENT[6], value(ENVIRONMENT[6]), 6.0)?,
        },
        radius_units: number(ENVIRONMENT[2], value(ENVIRONMENT[2]), 30.0)?,
        max_work: number(ENVIRONMENT[3], value(ENVIRONMENT[3]), 10_000_000)?,
    };
    config.validate()?;
    Ok(config)
}

fn number<T: FromStr>(
    name: &str,
    value: Option<&str>,
    default: T,
) -> Result<T, Box<dyn Error + Send + Sync>> {
    match value {
        None => Ok(default),
        Some(raw) => raw
            .parse()
            .map_err(|_| format!("invalid numeric value for {name}").into()),
    }
}
