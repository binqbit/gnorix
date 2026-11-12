//! Inspect bounded regional feature combinations from one opaque PNG capture.

#[path = "support/image_input.rs"]
mod image_input;

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

use gnorix::biometrics::{
    Capture,
    fingerprint::{self, CombinationConfig, ExtractionConfig, FeatureCombinations, FeatureKind},
};
use serde_json::{Value, json};

const ENVIRONMENT: [&str; 5] = [
    "GNORIX_COMBINATION_MIN_POINTS",
    "GNORIX_COMBINATION_POOL_MULTIPLIER",
    "GNORIX_COMBINATION_DIAMETER_FRACTION",
    "GNORIX_COMBINATION_PER_REGION",
    "GNORIX_COMBINATION_MAX_WORK",
];

fn main() -> Result<(), Box<dyn Error>> {
    let (input, output) = arguments()?;
    if output.exists() {
        return Err("output file already exists".into());
    }
    let config = configuration()?;
    let extraction = ExtractionConfig::default();
    let source = image_input::read(&input, extraction.max_pixels)?;
    let capture = Capture::new(
        source.width(),
        source.height(),
        source.as_raw().clone(),
        None,
        None,
    )?;

    let fingerprint = fingerprint::extract(&capture, &extraction)?;
    let result = fingerprint::generate_combinations(&fingerprint, &config)?;
    let document = snapshot(
        &input,
        source.width(),
        source.height(),
        fingerprint.scale.unit_px,
        &config,
        &result,
    );
    write_new(&output, &document)?;
    println!(
        "{} scored features; {} regions; {} unique combinations; {} unscored features\n{}",
        result.points.len(),
        result.regions.len(),
        result.combinations.len(),
        result.unscored_features,
        output.display(),
    );
    Ok(())
}

fn arguments() -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        return Err("usage: feature_combinations <input.png> <new-output.json>".into());
    }
    Ok((PathBuf::from(&arguments[0]), PathBuf::from(&arguments[1])))
}

fn configuration() -> Result<CombinationConfig, Box<dyn Error>> {
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

fn parse_configuration<'a>(
    value: impl Fn(&str) -> Option<&'a str>,
) -> Result<CombinationConfig, Box<dyn Error>> {
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

fn number<T: FromStr>(name: &str, value: Option<&str>, default: T) -> Result<T, Box<dyn Error>> {
    match value {
        None => Ok(default),
        Some(raw) => raw
            .parse()
            .map_err(|_| format!("invalid numeric value for {name}").into()),
    }
}

fn snapshot(
    input: &Path,
    width: u32,
    height: u32,
    unit_px: f64,
    config: &CombinationConfig,
    result: &FeatureCombinations,
) -> Value {
    let (global_counts, region_counts) = occurrence_counts(result);
    let points: Vec<_> = result
        .points
        .iter()
        .map(|point| {
            json!({
                "id": point.id,
                "kind": kind_name(point.kind),
                "position": [point.position.x(), point.position.y()],
                "priority": point.priority,
                "combination_count": global_counts.get(&point.id).copied().unwrap_or(0),
            })
        })
        .collect();
    let regions: Vec<_> = result
        .regions
        .iter()
        .map(|region| {
            let counts = region_counts.get(&region.id);
            let occurrences: Vec<_> = region
                .pool
                .iter()
                .map(|id| {
                    json!({
                        "feature_id": id,
                        "count": counts.and_then(|values| values.get(id)).copied().unwrap_or(0),
                    })
                })
                .collect();
            json!({
                "id": region.id,
                "center": [region.center.x(), region.center.y()],
                "initial_radius_px": region.initial_radius,
                "radius_px": region.radius,
                "available_points": region.available_points,
                "pool": region.pool,
                "combination_count": region.combination_count,
                "point_occurrences": occurrences,
            })
        })
        .collect();
    let combinations: Vec<_> = result
        .combinations
        .iter()
        .map(|combination| {
            json!({
                "feature_ids": combination.feature_ids,
                "score": combination.score,
                "regions": combination.regions,
            })
        })
        .collect();

    json!({
        "schema": "gnorix.feature-combinations.v1",
        "source": {"image": input.to_string_lossy(), "width": width, "height": height},
        "unit_px": unit_px,
        "config": {
            "min_points": config.min_points,
            "pool_multiplier": config.pool_multiplier,
            "diameter_fraction": config.diameter_fraction,
            "combinations_per_region": config.combinations_per_region,
            "max_work": config.max_work,
        },
        "policy": {
            "region_layout": "3x3 cell centers of confirmed-pattern bounds",
            "point_weight": "0.1 + 0.9 * priority^2",
            "combination_score": "mean point weight",
            "subset_sizes": "min_points through regional pool size",
            "coverage": "each selected pool point occurs at least once per region",
            "identity": "original feature IDs within this capture",
        },
        "bounds": {
            "min": [result.bounds.min.x(), result.bounds.min.y()],
            "max": [result.bounds.max.x(), result.bounds.max.y()],
        },
        "unscored_features": result.unscored_features,
        "work_used": result.work_used,
        "points": points,
        "regions": regions,
        "combinations": combinations,
    })
}

type OccurrenceCounts = BTreeMap<usize, usize>;

fn occurrence_counts(
    result: &FeatureCombinations,
) -> (OccurrenceCounts, BTreeMap<u8, OccurrenceCounts>) {
    let mut global = BTreeMap::new();
    let mut regional = BTreeMap::<u8, OccurrenceCounts>::new();
    for combination in &result.combinations {
        for &id in &combination.feature_ids {
            *global.entry(id).or_default() += 1;
            for &region in &combination.regions {
                *regional.entry(region).or_default().entry(id).or_default() += 1;
            }
        }
    }
    (global, regional)
}

fn kind_name(kind: FeatureKind) -> &'static str {
    match kind {
        FeatureKind::Ending => "ending",
        FeatureKind::Bifurcation => "bifurcation",
        FeatureKind::Junction => "junction",
        FeatureKind::Isolated => "isolated",
        FeatureKind::Cycle => "cycle",
    }
}

fn write_new(path: &Path, document: &Value) -> Result<(), Box<dyn Error>> {
    let mut encoded = serde_json::to_vec_pretty(document)?;
    encoded.push(b'\n');
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Err(error) = file.write_all(&encoded) {
        drop(file);
        fs::remove_file(path)?;
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_are_explicit_and_can_be_overridden_without_global_environment() {
        let defaults = parse_configuration(|_| None).unwrap();
        assert_eq!(defaults.min_points, 3);
        assert_eq!(defaults.pool_multiplier, 2);
        assert_eq!(defaults.diameter_fraction, 0.5);
        assert_eq!(defaults.combinations_per_region, 12);
        assert_eq!(defaults.max_work, 10_000_000);

        let settings = BTreeMap::from([
            (ENVIRONMENT[0], "2"),
            (ENVIRONMENT[1], "3"),
            (ENVIRONMENT[2], "0.75"),
            (ENVIRONMENT[3], "20"),
            (ENVIRONMENT[4], "1000"),
        ]);
        let config = parse_configuration(|name| settings.get(name).copied()).unwrap();
        assert_eq!(config.min_points, 2);
        assert_eq!(config.pool_multiplier, 3);
        assert_eq!(config.diameter_fraction, 0.75);
        assert_eq!(config.combinations_per_region, 20);
        assert_eq!(config.max_work, 1000);
    }

    #[test]
    fn invalid_environment_values_fail_instead_of_falling_back_to_defaults() {
        for (name, invalid) in [
            (ENVIRONMENT[0], ""),
            (ENVIRONMENT[0], "-1"),
            (ENVIRONMENT[0], "1.5"),
            (ENVIRONMENT[0], "0"),
            (ENVIRONMENT[1], "0"),
            (ENVIRONMENT[2], "NaN"),
            (ENVIRONMENT[2], "inf"),
            (ENVIRONMENT[2], "-0.5"),
            (ENVIRONMENT[3], "0"),
            (ENVIRONMENT[4], "0"),
        ] {
            assert!(
                parse_configuration(|key| (key == name).then_some(invalid)).is_err(),
                "{name} accepted {invalid}"
            );
        }
        let huge = usize::MAX.to_string();
        assert!(
            parse_configuration(|name| (name == ENVIRONMENT[0]).then_some(huge.as_str())).is_err()
        );
    }
}
