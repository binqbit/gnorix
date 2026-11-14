//! Adapter configuration, input validation and complete-output guarantees.

use std::{
    env,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use gnorix::biometrics::fingerprint::{self, GridShapeConfig, GridShapes};
use serde_json::{Value, json};

use super::{
    ENVIRONMENT, MAX_JSON_BYTES,
    input::{CombinationInput, parse as parse_input, read as read_input},
    output::{manifest, write_new},
    parse_configuration,
};

fn fixture() -> Value {
    json!({
        "schema": "gnorix.feature-combinations.v6", "unit_px": 4.0,
        "source": {"image": "test.png", "width": 20, "height": 20},
        "points": [
            {"id": 2, "kind": "ending", "position": [4,4], "priority": 0.8},
            {"id": 8, "kind": "bifurcation", "position": [4,4], "priority": 0.3},
        ],
        "combinations": [{"feature_ids": [2,8], "score": 0.4, "regions": [0]}],
    })
}

fn prepared(input: &CombinationInput, config: &GridShapeConfig) -> GridShapes {
    fingerprint::prepare_grid_shapes(&input.points, &input.combinations, input.unit_px, config)
        .unwrap()
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "gnorix-grid-{}-{timestamp}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn settings_reject_invalid_values_without_reading_global_environment() {
    let defaults = parse_configuration(|_| None).unwrap();
    assert_eq!(defaults.uncertainty_steps, 1);
    assert_eq!(defaults.max_variants, 1_000_000);
    assert_eq!(defaults.max_work, 10_000_000);
    assert_eq!(
        parse_configuration(|name| (name == ENVIRONMENT[0]).then_some("0"))
            .unwrap()
            .uncertainty_steps,
        0
    );
    for (name, value) in [
        (ENVIRONMENT[0], "-1"),
        (ENVIRONMENT[0], "1.5"),
        (ENVIRONMENT[0], "4294967296"),
        (ENVIRONMENT[1], "0"),
        (ENVIRONMENT[2], "0"),
    ] {
        assert!(parse_configuration(|key| (key == name).then_some(value)).is_err());
    }
}

#[test]
fn validates_schema_numeric_fields_coordinates_and_feature_kinds() {
    for (pointer, invalid) in [
        ("/schema", json!("different.v1")),
        ("/schema", json!("gnorix.feature-combinations.v1")),
        ("/schema", json!("gnorix.feature-combinations.v2")),
        ("/schema", json!("gnorix.feature-combinations.v3")),
        ("/schema", json!("gnorix.feature-combinations.v4")),
        ("/schema", json!("gnorix.feature-combinations.v5")),
        ("/unit_px", json!(0)),
        ("/points/0/id", json!(-1)),
        ("/points/0/id", json!(1.5)),
        ("/points/0/kind", json!("unknown")),
        ("/points/0/position", json!([1])),
        ("/points/0/position/0", json!("NaN")),
        ("/points/0/priority", json!(1.01)),
        ("/combinations/0/score", json!(null)),
        ("/combinations/0/regions/0", json!(-1)),
        ("/combinations/0/regions/0", json!(1.5)),
    ] {
        let mut document = fixture();
        *document.pointer_mut(pointer).unwrap() = invalid;
        assert!(parse_input(document).is_err(), "accepted {pointer}");
    }
    let mut missing = fixture();
    missing.as_object_mut().unwrap().remove("points");
    assert!(parse_input(missing).is_err());
}

#[test]
fn adaptive_region_identifiers_are_not_limited_to_a_fixed_grid() {
    let mut document = fixture();
    document["combinations"][0]["regions"] = json!([0, 9, 256, 65536]);
    let input = parse_input(document).unwrap();
    assert_eq!(input.combinations[0].regions, vec![0, 9, 256, 65536]);
    let config = parse_configuration(|_| None).unwrap();
    let shapes = prepared(&input, &config);
    let document = manifest(Path::new("input.json"), &input, &config, &shapes);
    assert_eq!(
        document["combinations"][0]["regions"],
        json!([0, 9, 256, 65536])
    );
}

#[test]
fn bounded_reader_rejects_oversize_malformed_and_nonfinite_json() {
    let encoded = serde_json::to_vec(&fixture()).unwrap();
    assert!(read_input(&mut encoded.as_slice(), encoded.len()).is_ok());
    assert!(
        read_input(&mut encoded.as_slice(), encoded.len() - 1)
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
    assert!(read_input(&mut b"{broken".as_slice(), 100).is_err());
    assert!(read_input(&mut b"{\"unit_px\":1e999}".as_slice(), 100).is_err());
}

#[test]
fn writes_complete_json_and_preserves_collisions_and_source_ids() {
    let temporary = TemporaryDirectory::new();
    let output = temporary.0.join("complete.json");
    let input = parse_input(fixture()).unwrap();
    let config = parse_configuration(|_| None).unwrap();
    let prepared = prepared(&input, &config);
    let manifest = manifest(Path::new("input.json"), &input, &config, &prepared);
    let written = write_new(&output, &manifest, &prepared, MAX_JSON_BYTES).unwrap();
    let document: Value = serde_json::from_reader(File::open(&output).unwrap()).unwrap();
    assert_eq!(written.variants, 49);
    assert_eq!(written.colliding_variants, 7);
    assert_eq!(document["variants"].as_array().unwrap().len(), 49);
    assert_eq!(document["planned_variant_count"], 49);
    assert_eq!(document["variant_count"], 49);
    assert_eq!(document["colliding_variants"], 7);
    assert_eq!(document["shapes"][0]["feature_ids"], json!([2, 8]));
    for variant in document["variants"].as_array().unwrap() {
        assert_eq!(variant["shape_index"], 0);
        for collision in variant["collisions"].as_array().unwrap() {
            assert_eq!(collision["feature_ids"], json!([2, 8]));
        }
    }
    assert_eq!(written.bytes as u64, fs::metadata(&output).unwrap().len());
    assert!(write_new(&output, &manifest, &prepared, MAX_JSON_BYTES).is_err());
    assert_eq!(
        serde_json::from_reader::<_, Value>(File::open(&output).unwrap()).unwrap(),
        document
    );
}

#[test]
fn removes_partial_output_on_size_limit_and_late_traversal_failure() {
    let temporary = TemporaryDirectory::new();
    let output = temporary.0.join("partial.json");
    let input = parse_input(fixture()).unwrap();
    let mut config = parse_configuration(|_| None).unwrap();
    let shapes = prepared(&input, &config);
    let manifest = manifest(Path::new("input.json"), &input, &config, &shapes);
    assert!(
        write_new(&output, &manifest, &shapes, 100)
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
    assert!(!output.exists());

    config.max_work = shapes.preparation_work() + 1;
    let limited = prepared(&input, &config);
    assert!(write_new(&output, &manifest, &limited, MAX_JSON_BYTES).is_err());
    assert!(!output.exists());
}
