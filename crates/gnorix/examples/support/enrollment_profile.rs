//! Canonical, retained fingerprint reconstruction parameters shared by the examples.

use std::error::Error;

use gnorix::{
    biometrics::fingerprint::{CombinationConfig, SpatialPolicy, SpatialRequirements},
    merkle::Hash,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const DOMAIN: &str = "gnorix.example.imported-grid-profile.v6";
const SCHEMA: &str = "gnorix.feature-combinations.v5";

/// Validated public policy and its exact canonical package representation.
#[derive(Clone, Debug)]
pub(super) struct ProfileParameters {
    record: Record,
    encoded: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    domain: String,
    schema: String,
    min_points: usize,
    min_priority: f64,
    pool_multiplier: usize,
    radius_units: f64,
    combinations_per_region: usize,
    spatial_policy: SpatialPolicyRecord,
    spatial_requirements: SpatialRequirementsRecord,
    uncertainty_steps: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpatialPolicyRecord {
    min_diameter_fraction: f64,
    min_anchor_points: usize,
    min_anchor_separation_fraction: f64,
}

impl SpatialPolicyRecord {
    fn new(value: SpatialPolicy) -> Self {
        Self {
            min_diameter_fraction: canonical_zero(value.min_diameter_fraction),
            min_anchor_points: value.min_anchor_points,
            min_anchor_separation_fraction: canonical_zero(value.min_anchor_separation_fraction),
        }
    }

    fn core(&self) -> SpatialPolicy {
        SpatialPolicy {
            min_diameter_fraction: self.min_diameter_fraction,
            min_anchor_points: self.min_anchor_points,
            min_anchor_separation_fraction: self.min_anchor_separation_fraction,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpatialRequirementsRecord {
    min_diameter_units: f64,
    min_anchor_points: usize,
    min_anchor_separation_units: f64,
}

impl SpatialRequirementsRecord {
    fn new(value: SpatialRequirements) -> Self {
        Self {
            min_diameter_units: canonical_zero(value.min_diameter_units),
            min_anchor_points: value.min_anchor_points,
            min_anchor_separation_units: canonical_zero(value.min_anchor_separation_units),
        }
    }

    fn core(&self) -> SpatialRequirements {
        SpatialRequirements {
            min_diameter_units: self.min_diameter_units,
            min_anchor_points: self.min_anchor_points,
            min_anchor_separation_units: self.min_anchor_separation_units,
        }
    }
}

#[allow(
    dead_code,
    reason = "Shared enrollment, signing and Lab entry points use different methods."
)]
impl ProfileParameters {
    pub(super) fn new(
        controls: &CombinationConfig,
        uncertainty_steps: u32,
        spatial: SpatialRequirements,
    ) -> Result<Self> {
        controls.validate()?;
        Self::from_record(Record {
            domain: DOMAIN.into(),
            schema: SCHEMA.into(),
            min_points: controls.min_points,
            min_priority: canonical_zero(controls.min_priority),
            pool_multiplier: controls.pool_multiplier,
            radius_units: controls.radius_units,
            combinations_per_region: controls.combinations_per_region,
            spatial_policy: SpatialPolicyRecord::new(controls.spatial),
            spatial_requirements: SpatialRequirementsRecord::new(spatial),
            uncertainty_steps,
        })
    }

    /// Reject unsupported policy, unknown fields and noncanonical serialized bytes.
    pub(super) fn decode(encoded: &[u8], expected_profile: Hash) -> Result<Self> {
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
        let policy = record.spatial_policy.core();
        let spatial = record.spatial_requirements.core();
        policy.validate()?;
        spatial.validate()?;
        validate_resolved_spatial(&policy, &spatial)?;
        let signing_capacity = record
            .min_points
            .checked_mul(2)
            .ok_or("signing subset capacity overflow")?;
        if policy.min_anchor_points != spatial.min_anchor_points
            || spatial.min_anchor_points > signing_capacity
            || record.uncertainty_steps > i32::MAX as u32
        {
            return Err("inconsistent fingerprint reconstruction geometry".into());
        }
        record.min_priority = canonical_zero(record.min_priority);
        record.spatial_policy = SpatialPolicyRecord::new(policy);
        record.spatial_requirements = SpatialRequirementsRecord::new(spatial);
        let value = Self {
            record,
            encoded: Vec::new(),
        };
        value.controls(1)?;
        let encoded = serde_json::to_vec(&value.record)?;
        Ok(Self { encoded, ..value })
    }

    pub(super) fn controls(&self, max_work: usize) -> Result<CombinationConfig> {
        let value = CombinationConfig {
            min_points: self.record.min_points,
            min_priority: self.record.min_priority,
            pool_multiplier: self.record.pool_multiplier,
            radius_units: self.record.radius_units,
            combinations_per_region: self.record.combinations_per_region,
            spatial: self.record.spatial_policy.core(),
            max_work,
        };
        value.validate()?;
        Ok(value)
    }

    pub(super) fn encoded(&self) -> &[u8] {
        &self.encoded
    }
    pub(super) fn profile_id(&self) -> Hash {
        Sha256::digest(&self.encoded).into()
    }
    pub(super) fn uncertainty_steps(&self) -> u32 {
        self.record.uncertainty_steps
    }
    pub(super) fn spatial(&self) -> SpatialRequirements {
        self.record.spatial_requirements.core()
    }
}

fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

fn validate_resolved_spatial(policy: &SpatialPolicy, spatial: &SpatialRequirements) -> Result<()> {
    let diameter_scale = resolved_scale(policy.min_diameter_fraction, spatial.min_diameter_units)?;
    let anchor_scale = resolved_scale(
        policy.min_anchor_separation_fraction,
        spatial.min_anchor_separation_units,
    )?;
    if let (Some(diameter), Some(anchors)) = (diameter_scale, anchor_scale) {
        // Both distances derive from the same enrolled shorter side and R.
        // Allow only the roundoff from multiplication/division, not scan error.
        if (diameter - anchors).abs() / diameter.max(anchors) > 8.0 * f64::EPSILON {
            return Err("resolved spatial distances do not share one enrolled scale".into());
        }
    }
    Ok(())
}

fn resolved_scale(fraction: f64, units: f64) -> Result<Option<f64>> {
    if fraction == 0.0 && units == 0.0 {
        return Ok(None);
    }
    if fraction == 0.0 || units == 0.0 {
        return Err("declared and resolved spatial distances disagree at zero".into());
    }
    let scale = units / fraction;
    if !scale.is_finite() || scale <= 0.0 {
        return Err("enrolled spatial scale must be finite and positive".into());
    }
    Ok(Some(scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls() -> CombinationConfig {
        CombinationConfig {
            min_points: 3,
            min_priority: 0.5,
            pool_multiplier: 2,
            radius_units: 30.0,
            combinations_per_region: 12,
            spatial: SpatialPolicy {
                min_diameter_fraction: 1.0 / 3.0,
                min_anchor_points: 3,
                min_anchor_separation_fraction: 1.0 / 6.0,
            },
            max_work: 1000,
        }
    }

    fn spatial(diameter: f64) -> SpatialRequirements {
        SpatialRequirements {
            min_diameter_units: diameter,
            min_anchor_points: 3,
            min_anchor_separation_units: diameter / 2.0,
        }
    }

    #[test]
    fn canonical_parameters_roundtrip_nontrivial_float_values_exactly() {
        for (diameter, priority, radius) in [
            (21.52098361288588, 0.5, 30.0),
            (f64::MIN_POSITIVE, 0.12345678901234568, 17.333333333333332),
            (123.45678901234567, 0.9999999999999999, 200.00000000000003),
            (0.0, 0.0, 1.0),
        ] {
            let mut config = controls();
            config.min_priority = priority;
            config.radius_units = radius;
            if diameter == 0.0 {
                config.spatial.min_diameter_fraction = 0.0;
                config.spatial.min_anchor_separation_fraction = 0.0;
            }
            let value = ProfileParameters::new(&config, 1, spatial(diameter)).unwrap();
            let restored = ProfileParameters::decode(value.encoded(), value.profile_id()).unwrap();
            assert_eq!(restored.spatial(), spatial(diameter));
            assert_eq!(
                restored.controls(10).unwrap().min_priority.to_bits(),
                priority.to_bits()
            );
            assert_eq!(
                restored.controls(10).unwrap().radius_units.to_bits(),
                radius.to_bits()
            );
            assert_eq!(restored.encoded(), value.encoded());
        }
    }

    #[test]
    fn spatial_controls_are_authenticated_and_signed_zero_is_canonical() {
        let config = controls();
        let baseline = ProfileParameters::new(&config, 1, spatial(20.0)).unwrap();
        for changed in [
            SpatialPolicy {
                min_diameter_fraction: 0.4,
                ..config.spatial
            },
            SpatialPolicy {
                min_anchor_points: 4,
                ..config.spatial
            },
            SpatialPolicy {
                min_anchor_separation_fraction: 0.2,
                ..config.spatial
            },
        ] {
            let mut controls = config;
            controls.spatial = changed;
            let requirements = SpatialRequirements {
                min_diameter_units: changed.min_diameter_fraction * 60.0,
                min_anchor_points: changed.min_anchor_points,
                min_anchor_separation_units: changed.min_anchor_separation_fraction * 60.0,
            };
            assert_ne!(
                baseline.profile_id(),
                ProfileParameters::new(&controls, 1, requirements)
                    .unwrap()
                    .profile_id()
            );
        }
        let mut zero = config;
        zero.min_priority = 0.0;
        zero.spatial.min_diameter_fraction = 0.0;
        zero.spatial.min_anchor_separation_fraction = 0.0;
        let first = ProfileParameters::new(&zero, 1, spatial(0.0)).unwrap();
        zero.min_priority = -0.0;
        zero.spatial.min_diameter_fraction = -0.0;
        zero.spatial.min_anchor_separation_fraction = -0.0;
        let mut requirements = spatial(0.0);
        requirements.min_diameter_units = -0.0;
        requirements.min_anchor_separation_units = -0.0;
        assert_eq!(
            first.profile_id(),
            ProfileParameters::new(&zero, 1, requirements)
                .unwrap()
                .profile_id()
        );
        let changed = spatial(24.0);
        assert_ne!(
            baseline.profile_id(),
            ProfileParameters::new(&config, 1, changed)
                .unwrap()
                .profile_id()
        );
    }

    #[test]
    fn canonical_parameters_reject_unknown_fields_versions_and_noncanonical_json() {
        let value = ProfileParameters::new(&controls(), 1, spatial(21.52098361288588)).unwrap();
        for (pointer, replacement) in [
            ("/domain", serde_json::json!("old.v5")),
            (
                "/schema",
                serde_json::json!("gnorix.feature-combinations.v4"),
            ),
            (
                "/spatial_policy/min_diameter_fraction",
                serde_json::json!(1.1),
            ),
            ("/spatial_policy/min_anchor_points", serde_json::json!(0)),
            (
                "/spatial_requirements/min_diameter_units",
                serde_json::json!(-1.0),
            ),
            (
                "/spatial_requirements/min_anchor_points",
                serde_json::json!(4),
            ),
        ] {
            let mut document: serde_json::Value = serde_json::from_slice(value.encoded()).unwrap();
            *document.pointer_mut(pointer).unwrap() = replacement;
            let encoded = serde_json::to_vec(&document).unwrap();
            assert!(ProfileParameters::decode(&encoded, Sha256::digest(&encoded).into()).is_err());
        }
        let mut document: serde_json::Value = serde_json::from_slice(value.encoded()).unwrap();
        document["spatial_policy"]["extra"] = serde_json::json!(true);
        let encoded = serde_json::to_vec(&document).unwrap();
        assert!(ProfileParameters::decode(&encoded, Sha256::digest(&encoded).into()).is_err());
        let document: serde_json::Value = serde_json::from_slice(value.encoded()).unwrap();
        let encoded = serde_json::to_vec_pretty(&document).unwrap();
        assert!(ProfileParameters::decode(&encoded, Sha256::digest(&encoded).into()).is_err());
        assert!(ProfileParameters::decode(value.encoded(), [0; 32]).is_err());
        let mut impossible = controls();
        impossible.spatial.min_anchor_points = 2 * impossible.min_points + 1;
        let mut requirements = spatial(20.0);
        requirements.min_anchor_points = impossible.spatial.min_anchor_points;
        assert!(ProfileParameters::new(&impossible, 1, requirements).is_err());
    }

    #[test]
    fn resolved_geometry_cannot_contradict_declared_fractions() {
        let config = controls();
        assert!(ProfileParameters::new(&config, 1, spatial(0.0)).is_err());
        let mut inconsistent = spatial(20.0);
        inconsistent.min_anchor_separation_units = 12.0;
        assert!(ProfileParameters::new(&config, 1, inconsistent).is_err());
        let mut zero = config;
        zero.spatial.min_anchor_separation_fraction = 0.0;
        assert!(ProfileParameters::new(&zero, 1, spatial(20.0)).is_err());
        let mut roundoff = spatial(20.0);
        roundoff.min_anchor_separation_units = f64::from_bits(10.0_f64.to_bits() + 1);
        assert!(ProfileParameters::new(&config, 1, roundoff).is_ok());
    }
}
