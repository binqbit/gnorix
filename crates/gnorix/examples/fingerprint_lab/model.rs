use gnorix::biometrics::fingerprint::CombinationConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;

pub const MAX_WORK: usize = 1_000_000_000;
pub const MAX_ATTEMPTS: usize = 1_000_000_000;
pub const ARTIFACT_GENERATOR: &str = "gnorix.test-artifacts.v3";

pub fn default_work_limit() -> usize {
    MAX_WORK
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Circle {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabConfig {
    pub min_points: usize,
    pub min_priority: f64,
    pub pool_multiplier: usize,
    pub radius_units: f64,
    pub combinations_per_region: usize,
    pub uncertainty_steps: u32,
}

impl Default for LabConfig {
    fn default() -> Self {
        Self {
            min_points: 3,
            min_priority: 0.5,
            pool_multiplier: 2,
            radius_units: 30.0,
            combinations_per_region: 12,
            uncertainty_steps: 1,
        }
    }
}

impl LabConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=8).contains(&self.min_points)
            || !self.min_priority.is_finite()
            || !(0.0..=1.0).contains(&self.min_priority)
            || !(1..=4).contains(&self.pool_multiplier)
            || !(1..=256).contains(&self.combinations_per_region)
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
            min_priority: self.min_priority,
            min_span_fraction: 1.0 / 3.0,
            pool_multiplier: self.pool_multiplier,
            radius_units: self.radius_units,
            combinations_per_region: self.combinations_per_region,
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
    pub attempt_limit: usize,
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
    pub attempt_limit: usize,
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
    pub min_pair_span_units: f64,
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
    pub pool: Vec<usize>,
    pub available_points: usize,
}

#[derive(Clone, Serialize)]
pub struct EnrollmentView {
    pub min_pair_span_units: f64,
    pub root: String,
    pub leaf_count: u64,
    pub config: LabConfig,
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
    fn rejects_unbounded_invalid_and_insufficient_pool_configs() {
        assert!(LabConfig::default().validate().is_ok());
        for invalid in [
            LabConfig {
                min_points: 0,
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
                radius_units: f64::NAN,
                ..LabConfig::default()
            },
            LabConfig {
                combinations_per_region: 1,
                ..LabConfig::default()
            },
            LabConfig {
                uncertainty_steps: 4,
                ..LabConfig::default()
            },
        ] {
            assert!(invalid.validate().is_err());
        }
        assert!(
            serde_json::from_value::<PreparedId>(serde_json::json!({
                "prepared_id": 1, "path": "/unexpected"
            }))
            .is_err()
        );
    }

    #[test]
    fn preparation_defaults_to_two_gib_without_changing_biometric_config() {
        let config = LabConfig::default();
        assert_eq!(config.min_priority, 0.5);
        for min_priority in [0.0, 1.0] {
            LabConfig {
                min_priority,
                ..config
            }
            .validate()
            .unwrap();
        }
        let value =
            serde_json::json!({ "kind": "enrollment", "config": config, "selection": null });
        let request: PrepareRequest = serde_json::from_value(value).unwrap();
        assert_eq!(request.memory_budget_gib, 2);
        assert_eq!(request.config, config);
    }
}
