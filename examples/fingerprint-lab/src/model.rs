use gnorix::biometrics::fingerprint::{CombinationConfig, SpatialPolicy};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;

pub const MAX_WORK: usize = 1_000_000_000;
pub const ARTIFACT_GENERATOR: &str = "gnorix.test-artifacts.v3";

pub fn default_work_limit() -> usize {
    MAX_WORK
}

pub use gnorix_example_support::fingerprint::Circle;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabConfig {
    pub min_points: usize,
    pub max_points: usize,
    pub min_priority: f64,
    pub min_anchor_points: usize,
    pub min_anchor_distance_units: f64,
    pub radius_units: f64,
    pub uncertainty_steps: u32,
}

impl Default for LabConfig {
    fn default() -> Self {
        Self {
            min_points: 3,
            max_points: 6,
            min_priority: 0.5,
            min_anchor_points: 3,
            min_anchor_distance_units: 6.0,
            radius_units: 30.0,
            uncertainty_steps: 1,
        }
    }
}

impl LabConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=16).contains(&self.min_points)
            || !self.min_priority.is_finite()
            || !(0.0..=1.0).contains(&self.min_priority)
            || !(1..=16).contains(&self.min_anchor_points)
            || !(1..=16).contains(&self.max_points)
            || self.min_points > self.max_points
            || self.min_anchor_points > self.max_points
            || !self.min_anchor_distance_units.is_finite()
            || !(0.0..=200.0).contains(&self.min_anchor_distance_units)
            || !self.radius_units.is_finite()
            || !(1.0..=200.0).contains(&self.radius_units)
            || self.uncertainty_steps > 3
        {
            return Err("configuration exceeds the local test stand's limits".into());
        }
        self.combinations()?.validate()?;
        Ok(())
    }

    pub fn combinations(&self) -> Result<CombinationConfig> {
        Ok(CombinationConfig {
            min_points: self.min_points,
            max_points: self.max_points,
            min_priority: self.min_priority,
            spatial: SpatialPolicy {
                min_anchor_points: self.min_anchor_points,
                min_anchor_distance_units: self.min_anchor_distance_units,
            },
            radius_units: self.radius_units,
            max_work: MAX_WORK,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRequest {
    pub kind: String,
    pub config: LabConfig,
    pub selection: Option<Circle>,
    #[serde(default = "default_work_limit")]
    pub work_limit: usize,
    #[serde(default)]
    pub artifact_revision: Option<u64>,
    #[serde(default = "super::resources::default_memory_budget_gib")]
    pub memory_budget_gib: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRequest {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRequest {
    pub level: u8,
    #[serde(default)]
    pub randomize: bool,
}

#[derive(Clone, Default, Serialize)]
pub struct ArtifactView {
    pub revision: u64,
    pub level: u8,
    pub seed: u32,
    pub image_url: String,
    pub generator_version: String,
    pub image_sha256: String,
    /// Clean enrollment ridge width used to generate this frame.
    pub ridge_width_px: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestRequest {
    pub config: LabConfig,
    pub selection: Circle,
    pub artifact_revision: u64,
    pub payload: String,
    pub work_limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedId {
    pub prepared_id: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignRequest {
    pub prepared_id: u64,
    pub payload: String,
    pub work_limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyRequest {
    pub payload: String,
}

#[derive(Clone, Default, Serialize)]
pub struct LabData {
    pub artifact: ArtifactView,
    pub prepared: Option<PreparedView>,
    pub enrollment: Option<EnrollmentView>,
    pub result: Option<RunResult>,
}

#[derive(Clone, Serialize)]
pub struct PreparedView {
    pub id: u64,
    pub kind: String,
    pub config: LabConfig,
    pub selection: Option<Circle>,
    pub work_limit: usize,
    pub artifact_revision: Option<u64>,
    pub unit_px: f64,
    pub spatial: SpatialView,
    pub points: Vec<PointView>,
    pub regions: Vec<RegionView>,
    pub combination_count: usize,
    pub candidate_count: Option<u64>,
    pub memory_budget_gib: Option<u8>,
    pub candidate_limit: Option<u64>,
    pub estimated_peak_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
pub struct PointView {
    pub id: usize,
    pub x: f64,
    pub y: f64,
    pub priority: f64,
    pub kind: String,
}

#[derive(Clone, Serialize)]
pub struct RegionView {
    pub id: usize,
    pub x: f64,
    pub y: f64,
    pub initial_radius: f64,
    pub radius: f64,
    pub feature_ids: Vec<usize>,
}

#[derive(Clone, Serialize)]
pub struct EnrollmentView {
    pub spatial: SpatialView,
    pub root: String,
    pub leaf_count: u64,
    pub config: LabConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SpatialView {
    pub min_anchor_points: usize,
    pub min_anchor_distance_units: f64,
}

impl From<SpatialPolicy> for SpatialView {
    fn from(requirements: SpatialPolicy) -> Self {
        Self {
            min_anchor_points: requirements.min_anchor_points,
            min_anchor_distance_units: requirements.min_anchor_distance_units,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct RunResult {
    pub status: String,
    pub message: String,
    pub attempts: usize,
    pub feature_ids: Vec<usize>,
    pub authorization: Option<Value>,
    pub verified: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_ranges_and_spatial_controls() {
        assert!(LabConfig::default().validate().is_ok());
        for invalid in [
            LabConfig {
                min_points: 0,
                ..LabConfig::default()
            },
            LabConfig {
                min_points: 7,
                max_points: 6,
                ..LabConfig::default()
            },
            LabConfig {
                max_points: 0,
                ..LabConfig::default()
            },
            LabConfig {
                max_points: 17,
                ..LabConfig::default()
            },
            LabConfig {
                min_priority: f64::NAN,
                ..LabConfig::default()
            },
            LabConfig {
                min_priority: -0.01,
                ..LabConfig::default()
            },
            LabConfig {
                min_priority: 1.01,
                ..LabConfig::default()
            },
            LabConfig {
                min_anchor_points: 0,
                ..LabConfig::default()
            },
            LabConfig {
                min_anchor_points: 7,
                ..LabConfig::default()
            },
            LabConfig {
                min_anchor_distance_units: f64::INFINITY,
                ..LabConfig::default()
            },
            LabConfig {
                min_anchor_distance_units: -0.01,
                ..LabConfig::default()
            },
            LabConfig {
                min_anchor_distance_units: 201.0,
                ..LabConfig::default()
            },
            LabConfig {
                radius_units: f64::NAN,
                ..LabConfig::default()
            },
            LabConfig {
                uncertainty_steps: 4,
                ..LabConfig::default()
            },
        ] {
            assert!(invalid.validate().is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn preparation_defaults_preserve_exact_biometric_controls() {
        let config = LabConfig::default();
        assert_eq!((config.min_points, config.max_points), (3, 6));
        assert_eq!(config.min_priority, 0.5);
        assert_eq!(config.min_anchor_points, 3);
        assert_eq!(config.min_anchor_distance_units, 6.0);
        for min_priority in [0.0, 0.43137304374477875, 1.0] {
            let config = LabConfig {
                min_priority,
                ..config
            };
            config.validate().unwrap();
            let value =
                serde_json::json!({ "kind": "enrollment", "config": config, "selection": null });
            let request: PrepareRequest = serde_json::from_value(value).unwrap();
            assert_eq!(request.memory_budget_gib, 2);
            assert_eq!(request.work_limit, MAX_WORK);
            assert_eq!(request.config, config);
        }
    }

    #[test]
    fn explicit_range_and_ridge_distance_reach_the_core_without_conversion() {
        let config = LabConfig {
            min_points: 2,
            max_points: 5,
            min_anchor_points: 4,
            min_anchor_distance_units: 7.25,
            ..LabConfig::default()
        };
        config.validate().unwrap();
        let controls = config.combinations().unwrap();
        assert_eq!((controls.min_points, controls.max_points), (2, 5));
        assert_eq!(controls.spatial.min_anchor_points, 4);
        assert_eq!(controls.spatial.min_anchor_distance_units, 7.25);
        let encoded = serde_json::to_vec(&config).unwrap();
        let decoded: LabConfig = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, config);
        for value in [0.0, 200.0] {
            LabConfig {
                min_anchor_distance_units: value,
                ..config
            }
            .validate()
            .unwrap();
        }
        LabConfig {
            min_points: 16,
            max_points: 16,
            ..config
        }
        .validate()
        .unwrap();
        let mut malformed = serde_json::to_value(config).unwrap();
        malformed["min_anchor_points"] = serde_json::json!(3.5);
        assert!(serde_json::from_value::<LabConfig>(malformed).is_err());
    }

    #[test]
    fn obsolete_controls_are_rejected_instead_of_silently_ignored() {
        for name in [
            "pool_multiplier",
            "combinations_per_region",
            "min_diameter_fraction",
            "min_anchor_separation_fraction",
        ] {
            let mut value = serde_json::to_value(LabConfig::default()).unwrap();
            value[name] = serde_json::json!(1);
            assert!(serde_json::from_value::<LabConfig>(value).is_err());
        }
        let request = serde_json::json!({"prepared_id": 1, "payload": "test", "work_limit": 1000});
        assert!(serde_json::from_value::<SignRequest>(request.clone()).is_ok());
        let mut obsolete = request;
        obsolete["attempt_limit"] = serde_json::json!(100);
        assert!(serde_json::from_value::<SignRequest>(obsolete).is_err());
        assert!(
            serde_json::from_value::<PreparedId>(serde_json::json!({
                "prepared_id": 1, "path": "/unexpected"
            }))
            .is_err()
        );
    }
}
