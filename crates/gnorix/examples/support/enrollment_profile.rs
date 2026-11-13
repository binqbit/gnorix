//! Canonical, retained fingerprint reconstruction parameters shared by the examples.

use std::error::Error;

use gnorix::{biometrics::fingerprint::CombinationConfig, merkle::Hash};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const DOMAIN: &str = "gnorix.example.imported-grid-profile.v5";
const SCHEMA: &str = "gnorix.feature-combinations.v4";
const SPAN_FRACTION: f64 = 1.0 / 3.0;

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
    min_span_fraction: f64,
    min_pair_span_units: f64,
    uncertainty_steps: u32,
}

#[allow(
    dead_code,
    reason = "Shared enrollment, signing and Lab entry points use different methods."
)]
impl ProfileParameters {
    pub(super) fn new(
        controls: &CombinationConfig,
        uncertainty_steps: u32,
        min_pair_span_units: f64,
    ) -> Result<Self> {
        controls.validate()?;
        let record = Record {
            domain: DOMAIN.into(),
            schema: SCHEMA.into(),
            min_points: controls.min_points,
            min_priority: canonical_zero(controls.min_priority),
            pool_multiplier: controls.pool_multiplier,
            radius_units: controls.radius_units,
            combinations_per_region: controls.combinations_per_region,
            min_span_fraction: controls.min_span_fraction,
            min_pair_span_units,
            uncertainty_steps,
        };
        Self::from_record(record)
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
        if record.min_span_fraction != SPAN_FRACTION
            || !record.min_pair_span_units.is_finite()
            || record.min_pair_span_units <= 0.0
            || record.uncertainty_steps > i32::MAX as u32
        {
            return Err("invalid fingerprint reconstruction geometry".into());
        }
        record.min_priority = canonical_zero(record.min_priority);
        let value = Self {
            record,
            encoded: Vec::new(),
        };
        value.controls(1)?.validate()?;
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
            min_span_fraction: self.record.min_span_fraction,
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
    pub(super) fn min_pair_span_units(&self) -> f64 {
        self.record.min_pair_span_units
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
            min_points: 2,
            min_priority: 0.5,
            pool_multiplier: 2,
            radius_units: 30.0,
            combinations_per_region: 12,
            min_span_fraction: 1.0 / 3.0,
            max_work: 1000,
        }
    }

    #[test]
    fn canonical_parameters_roundtrip_nontrivial_float_values_exactly() {
        for (span, priority, radius) in [
            (21.52098361288588, 0.5, 30.0),
            (f64::MIN_POSITIVE, 0.12345678901234568, 17.333333333333332),
            (123.45678901234567, 0.9999999999999999, 200.00000000000003),
            (1.0, 0.0, 1.0),
        ] {
            let mut config = controls();
            config.min_priority = priority;
            config.radius_units = radius;
            let value = ProfileParameters::new(&config, 1, span).unwrap();
            let restored = ProfileParameters::decode(value.encoded(), value.profile_id()).unwrap();
            assert_eq!(restored.min_pair_span_units().to_bits(), span.to_bits());
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
    fn canonical_parameters_reject_unknown_fields_versions_and_noncanonical_json() {
        let value = ProfileParameters::new(&controls(), 1, 21.52098361288588).unwrap();
        for (key, replacement) in [
            ("domain", serde_json::json!("old.v4")),
            (
                "schema",
                serde_json::json!("gnorix.feature-combinations.v3"),
            ),
            ("extra", serde_json::json!(true)),
            ("min_span_fraction", serde_json::json!(0.5)),
        ] {
            let mut document: serde_json::Value = serde_json::from_slice(value.encoded()).unwrap();
            document[key] = replacement;
            let encoded = serde_json::to_vec(&document).unwrap();
            assert!(ProfileParameters::decode(&encoded, Sha256::digest(&encoded).into()).is_err());
        }
        let encoded = serde_json::to_vec_pretty(
            &serde_json::from_slice::<serde_json::Value>(value.encoded()).unwrap(),
        )
        .unwrap();
        assert!(ProfileParameters::decode(&encoded, Sha256::digest(&encoded).into()).is_err());
    }
}
