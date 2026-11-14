//! Canonical, retained fingerprint reconstruction parameters shared by the examples.

use gnorix::{
    biometrics::fingerprint::{CombinationConfig, SpatialPolicy},
    merkle::Hash,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Result;

const DOMAIN: &str = "gnorix.example.imported-grid-profile.v7";
const SCHEMA: &str = "gnorix.feature-combinations.v6";

/// Validated public policy and its exact canonical package representation.
#[derive(Clone, Debug)]
pub struct ProfileParameters {
    record: Record,
    encoded: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    domain: String,
    schema: String,
    min_points: usize,
    max_points: usize,
    min_priority: f64,
    radius_units: f64,
    spatial: SpatialRecord,
    uncertainty_steps: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpatialRecord {
    min_anchor_points: usize,
    min_anchor_distance_units: f64,
}

impl SpatialRecord {
    fn new(value: SpatialPolicy) -> Self {
        Self {
            min_anchor_points: value.min_anchor_points,
            min_anchor_distance_units: canonical_zero(value.min_anchor_distance_units),
        }
    }

    fn core(&self) -> SpatialPolicy {
        SpatialPolicy {
            min_anchor_points: self.min_anchor_points,
            min_anchor_distance_units: self.min_anchor_distance_units,
        }
    }
}

impl ProfileParameters {
    pub fn new(controls: &CombinationConfig, uncertainty_steps: u32) -> Result<Self> {
        controls.validate()?;
        Self::from_record(Record {
            domain: DOMAIN.into(),
            schema: SCHEMA.into(),
            min_points: controls.min_points,
            max_points: controls.max_points,
            min_priority: canonical_zero(controls.min_priority),
            radius_units: controls.radius_units,
            spatial: SpatialRecord::new(controls.spatial),
            uncertainty_steps,
        })
    }

    /// Reject unsupported policy, unknown fields and noncanonical serialized bytes.
    pub fn decode(encoded: &[u8], expected_profile: Hash) -> Result<Self> {
        if <Hash>::from(Sha256::digest(encoded)) != expected_profile {
            return Err("fingerprint parameter hash differs from the enrolled profile".into());
        }
        let value = Self::from_record(serde_json::from_slice(encoded)?)?;
        if value.encoded != encoded {
            return Err("fingerprint parameters are not canonically encoded".into());
        }
        Ok(value)
    }

    fn from_record(mut record: Record) -> Result<Self> {
        if record.domain != DOMAIN || record.schema != SCHEMA {
            return Err("unsupported fingerprint reconstruction parameters".into());
        }
        if record.uncertainty_steps > i32::MAX as u32 {
            return Err("unsupported grid uncertainty level".into());
        }
        record.min_priority = canonical_zero(record.min_priority);
        record.spatial = SpatialRecord::new(record.spatial.core());
        let mut value = Self {
            record,
            encoded: Vec::new(),
        };
        value.controls(1)?;
        value.encoded = serde_json::to_vec(&value.record)?;
        Ok(value)
    }

    pub fn controls(&self, max_work: usize) -> Result<CombinationConfig> {
        let value = CombinationConfig {
            min_points: self.record.min_points,
            max_points: self.record.max_points,
            min_priority: self.record.min_priority,
            radius_units: self.record.radius_units,
            spatial: self.record.spatial.core(),
            max_work,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }
    pub fn profile_id(&self) -> Hash {
        Sha256::digest(&self.encoded).into()
    }
    pub fn uncertainty_steps(&self) -> u32 {
        self.record.uncertainty_steps
    }
    pub fn spatial(&self) -> SpatialPolicy {
        self.record.spatial.core()
    }
}

fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls() -> CombinationConfig {
        CombinationConfig {
            min_points: 3,
            max_points: 6,
            min_priority: 0.5,
            radius_units: 30.0,
            spatial: SpatialPolicy {
                min_anchor_points: 3,
                min_anchor_distance_units: 6.0,
            },
            max_work: 1000,
        }
    }

    #[test]
    fn canonical_parameters_roundtrip_exact_values_and_direct_ridge_units() {
        for (distance, priority, radius) in [
            (21.52098361288588, 0.5, 30.0),
            (f64::MIN_POSITIVE, 0.12345678901234568, 17.333333333333332),
            (123.45678901234567, 0.9999999999999999, 200.00000000000003),
            (0.0, 0.0, 1.0),
        ] {
            let mut config = controls();
            config.min_priority = priority;
            config.radius_units = radius;
            config.spatial.min_anchor_distance_units = distance;
            let value = ProfileParameters::new(&config, 1).unwrap();
            let restored = ProfileParameters::decode(value.encoded(), value.profile_id()).unwrap();
            assert_eq!(restored.controls(config.max_work).unwrap(), config);
            assert_eq!(restored.spatial(), config.spatial);
            assert_eq!(restored.encoded(), value.encoded());
        }
    }

    #[test]
    fn all_reconstruction_controls_are_authenticated_but_work_budget_is_not() {
        let config = controls();
        let baseline = ProfileParameters::new(&config, 1).unwrap();
        for changed in [
            CombinationConfig {
                min_points: 4,
                ..config
            },
            CombinationConfig {
                max_points: 7,
                ..config
            },
            CombinationConfig {
                min_priority: 0.6,
                ..config
            },
            CombinationConfig {
                radius_units: 31.0,
                ..config
            },
            CombinationConfig {
                spatial: SpatialPolicy {
                    min_anchor_points: 4,
                    ..config.spatial
                },
                ..config
            },
            CombinationConfig {
                spatial: SpatialPolicy {
                    min_anchor_distance_units: 7.0,
                    ..config.spatial
                },
                ..config
            },
        ] {
            assert_ne!(
                baseline.profile_id(),
                ProfileParameters::new(&changed, 1).unwrap().profile_id()
            );
        }
        assert_ne!(
            baseline.profile_id(),
            ProfileParameters::new(&config, 2).unwrap().profile_id()
        );
        let changed_work = CombinationConfig {
            max_work: 2000,
            ..config
        };
        assert_eq!(
            baseline.profile_id(),
            ProfileParameters::new(&changed_work, 1)
                .unwrap()
                .profile_id()
        );
    }

    #[test]
    fn signed_zero_has_one_canonical_representation() {
        let mut config = controls();
        config.min_priority = 0.0;
        config.spatial.min_anchor_distance_units = 0.0;
        let first = ProfileParameters::new(&config, 1).unwrap();
        config.min_priority = -0.0;
        config.spatial.min_anchor_distance_units = -0.0;
        assert_eq!(
            first.profile_id(),
            ProfileParameters::new(&config, 1).unwrap().profile_id()
        );
    }

    #[test]
    fn parameters_reject_old_profiles_invalid_controls_and_noncanonical_json() {
        let value = ProfileParameters::new(&controls(), 1).unwrap();
        for (pointer, replacement) in [
            (
                "/domain",
                serde_json::json!("gnorix.example.imported-grid-profile.v6"),
            ),
            (
                "/schema",
                serde_json::json!("gnorix.feature-combinations.v5"),
            ),
            ("/max_points", serde_json::json!(2)),
            ("/spatial/min_anchor_points", serde_json::json!(0)),
            ("/spatial/min_anchor_points", serde_json::json!(7)),
            (
                "/spatial/min_anchor_distance_units",
                serde_json::json!(-1.0),
            ),
        ] {
            let mut document: serde_json::Value = serde_json::from_slice(value.encoded()).unwrap();
            *document.pointer_mut(pointer).unwrap() = replacement;
            let encoded = serde_json::to_vec(&document).unwrap();
            assert!(ProfileParameters::decode(&encoded, Sha256::digest(&encoded).into()).is_err());
        }
        let mut document: serde_json::Value = serde_json::from_slice(value.encoded()).unwrap();
        document["spatial"]["legacy_fraction"] = serde_json::json!(0.5);
        let encoded = serde_json::to_vec(&document).unwrap();
        assert!(ProfileParameters::decode(&encoded, Sha256::digest(&encoded).into()).is_err());
        let document: serde_json::Value = serde_json::from_slice(value.encoded()).unwrap();
        let encoded = serde_json::to_vec_pretty(&document).unwrap();
        assert!(ProfileParameters::decode(&encoded, Sha256::digest(&encoded).into()).is_err());
        assert!(ProfileParameters::decode(value.encoded(), [0; 32]).is_err());
    }
}
