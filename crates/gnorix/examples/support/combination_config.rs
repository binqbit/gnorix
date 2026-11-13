//! Explicit regional enrollment controls shared by the local examples.

use gnorix::biometrics::fingerprint::CombinationConfig;
use std::{collections::BTreeMap, env, error::Error, str::FromStr};

pub(super) const ENVIRONMENT: [&str; 5] = [
    "GNORIX_COMBINATION_MIN_POINTS",
    "GNORIX_COMBINATION_POOL_MULTIPLIER",
    "GNORIX_COMBINATION_DIAMETER_FRACTION",
    "GNORIX_COMBINATION_PER_REGION",
    "GNORIX_COMBINATION_MAX_WORK",
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
        pool_multiplier: number(ENVIRONMENT[1], value(ENVIRONMENT[1]), 2)?,
        diameter_fraction: number(ENVIRONMENT[2], value(ENVIRONMENT[2]), 0.5)?,
        combinations_per_region: number(ENVIRONMENT[3], value(ENVIRONMENT[3]), 12)?,
        max_work: number(ENVIRONMENT[4], value(ENVIRONMENT[4]), 10_000_000)?,
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
