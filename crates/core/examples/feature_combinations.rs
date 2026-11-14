//! Inspect ranked regional feature combinations from one opaque PNG capture.

#[path = "support/combination_config.rs"]
mod combination_config;
#[path = "support/image_input.rs"]
mod image_input;

use std::{
    env,
    error::Error,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use combination_config::read as configuration;
#[cfg(test)]
use combination_config::{ENVIRONMENT, parse_configuration};
use gnorix::biometrics::{
    Capture,
    fingerprint::{self, CombinationConfig, ExtractionConfig, FeatureCombinations, FeatureKind},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (input, output) = arguments()?;
    if output.exists() {
        return Err("output file already exists".into());
    }
    let config = configuration()?;
    let extraction = ExtractionConfig::default();
    let source = image_input::read(&input, extraction.max_pixels)?;
    let (width, height) = source.dimensions();
    let capture = Capture::new(width, height, source.into_raw(), None, None)?;

    let fingerprint = fingerprint::analyze(&capture, &extraction)?;
    let result = fingerprint::generate_combinations(&fingerprint, &config)?;
    let document = snapshot(&input, width, height, fingerprint.unit_px, &config, &result);
    write_new(&output, &document)?;
    println!(
        "{} eligible features; {} regions; {} unique combinations; {} unscored features\n{}",
        result.points.len(),
        result.regions.len(),
        result.combinations.len(),
        result.unscored_features,
        output.display(),
    );
    Ok(())
}

fn arguments() -> Result<(PathBuf, PathBuf), Box<dyn Error + Send + Sync>> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        return Err("usage: feature_combinations <input.png> <new-output.json>".into());
    }
    Ok((PathBuf::from(&arguments[0]), PathBuf::from(&arguments[1])))
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
                .feature_ids
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
                "feature_ids": region.feature_ids,
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
        "schema": "gnorix.feature-combinations.v6",
        "source": {"image": input.to_string_lossy(), "width": width, "height": height},
        "unit_px": unit_px,
        "config": {
            "min_points": config.min_points,
            "max_points": config.max_points,
            "min_priority": config.min_priority,
            "spatial": {
                "min_anchor_points": config.spatial.min_anchor_points,
                "min_anchor_distance_units": config.spatial.min_anchor_distance_units,
            },
            "radius_units": config.radius_units,
            "max_work": config.max_work,
        },
        "policy": {
            "point_eligibility": "known final priority >= min_priority",
            "subset_geometry": "at least min_anchor_points pairwise-separated anchors; all other points inside their convex hull (inclusive), before grid alignment",
            "region_layout": "adaptive row-major cell centers covering confirmed-pattern and eligible-point bounds",
            "region_coverage": "minimum cell count whose half-diagonals fit the initial radius; closest-to-square cells break ties",
            "initial_radius": "radius_units * measured mean ridge width R",
            "point_weight": "0.1 + 0.9 * priority^2",
            "combination_score": "mean point weight",
            "selection": "rank bands by strongest included point; quotas N, ceil(N/2), ... with a floor of one, plus coverage witnesses",
            "subset_sizes": "min_points through max_points, bounded by regional availability",
            "coverage": "each eligible regional point occurs at least once per region",
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
) -> (OccurrenceCounts, BTreeMap<usize, OccurrenceCounts>) {
    let mut global = BTreeMap::new();
    let mut regional = BTreeMap::<usize, OccurrenceCounts>::new();
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

fn write_new(path: &Path, document: &Value) -> Result<(), Box<dyn Error + Send + Sync>> {
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
    fn settings_use_explicit_point_range_and_ridge_unit_distance() {
        let defaults = parse_configuration(|_| None).unwrap();
        assert_eq!((defaults.min_points, defaults.max_points), (3, 6));
        assert_eq!(defaults.min_priority, 0.5);
        assert_eq!(defaults.spatial.min_anchor_points, 3);
        assert_eq!(defaults.spatial.min_anchor_distance_units, 6.0);
        assert_eq!(defaults.radius_units, 30.0);
        assert_eq!(defaults.max_work, 10_000_000);

        let settings = BTreeMap::from([
            (ENVIRONMENT[0], "2"),
            (ENVIRONMENT[1], "7"),
            (ENVIRONMENT[2], "24.5"),
            (ENVIRONMENT[3], "1000"),
            (ENVIRONMENT[4], "0.35"),
            (ENVIRONMENT[5], "4"),
            (ENVIRONMENT[6], "8.5"),
        ]);
        let config = parse_configuration(|name| settings.get(name).copied()).unwrap();
        assert_eq!((config.min_points, config.max_points), (2, 7));
        assert_eq!(config.min_priority, 0.35);
        assert_eq!(config.spatial.min_anchor_points, 4);
        assert_eq!(config.spatial.min_anchor_distance_units, 8.5);
        assert_eq!(config.radius_units, 24.5);
        assert_eq!(config.max_work, 1000);
        for value in ["0", "-0", "1"] {
            assert!(parse_configuration(|name| (name == ENVIRONMENT[4]).then_some(value)).is_ok());
        }
    }

    #[test]
    fn export_records_the_required_eligibility_policy() {
        let config = parse_configuration(|_| None).unwrap();
        let origin = gnorix::biometrics::geometry::Point::new(0.0, 0.0).unwrap();
        let result = FeatureCombinations {
            bounds: fingerprint::RegionBounds {
                min: origin,
                max: origin,
            },
            points: Vec::new(),
            unscored_features: 0,
            regions: Vec::new(),
            combinations: Vec::new(),
            work_used: 0,
        };
        let document = snapshot(Path::new("capture.png"), 1, 1, 4.0, &config, &result);
        assert_eq!(document["schema"], "gnorix.feature-combinations.v6");
        assert_eq!(document["config"]["min_priority"], 0.5);
        assert_eq!(document["config"]["max_points"], 6);
        assert_eq!(document["config"]["spatial"]["min_anchor_points"], 3);
        assert_eq!(
            document["config"]["spatial"]["min_anchor_distance_units"],
            6.0
        );
        assert!(document.get("spatial").is_none());
        assert_eq!(
            document["policy"]["point_eligibility"],
            "known final priority >= min_priority"
        );
    }

    #[test]
    fn invalid_environment_values_fail_instead_of_falling_back_to_defaults() {
        for (name, invalid) in [
            (ENVIRONMENT[0], ""),
            (ENVIRONMENT[0], "-1"),
            (ENVIRONMENT[0], "1.5"),
            (ENVIRONMENT[0], "0"),
            (ENVIRONMENT[0], "7"),
            (ENVIRONMENT[1], "2"),
            (ENVIRONMENT[2], "NaN"),
            (ENVIRONMENT[2], "inf"),
            (ENVIRONMENT[2], "-0.5"),
            (ENVIRONMENT[2], "0"),
            (ENVIRONMENT[3], "0"),
            (ENVIRONMENT[4], "NaN"),
            (ENVIRONMENT[4], "inf"),
            (ENVIRONMENT[4], "-0.01"),
            (ENVIRONMENT[4], "1.01"),
            (ENVIRONMENT[5], "0"),
            (ENVIRONMENT[5], "2.5"),
            (ENVIRONMENT[5], "7"),
            (ENVIRONMENT[6], "inf"),
            (ENVIRONMENT[6], "-0.1"),
        ] {
            assert!(
                parse_configuration(|key| (key == name).then_some(invalid)).is_err(),
                "{name} accepted {invalid}"
            );
        }
    }
}
