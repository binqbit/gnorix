//! Public enrollment input, resource bounds and the imported-grid profile.

use std::{
    collections::BTreeMap,
    env,
    io::Read,
    num::{NonZeroU64, NonZeroUsize},
    str::FromStr,
};

use gnorix::{
    biometrics::{
        fingerprint::{self, CombinationConfig, GridBaselineConfig, RegionBounds},
        geometry::Point,
    },
    enrollment::{Domain, EnrollmentSpec},
    merkle::Hash,
    package::PackageLimits,
    policy::{Limits, SecretPolicy},
};
use serde_json::{Map, Value};

use super::{Result, enrollment_profile::ProfileParameters, input::CombinationInput};

pub(super) const MAX_CONTEXT_JSON_BYTES: usize = 64 * 1024;
pub(super) const MAX_DESCRIPTOR_BYTES: usize = 1024 * 1024;

pub(super) fn baseline_config() -> Result<GridBaselineConfig> {
    let config = GridBaselineConfig {
        padding_steps: number("GNORIX_GRID_UNCERTAINTY_STEPS", 1)?,
        max_baselines: number("GNORIX_ENROLLMENT_MAX_CANDIDATES", 1_000_000)?,
        max_work: number("GNORIX_GRID_MAX_WORK", 10_000_000)?,
    };
    config.validate()?;
    Ok(config)
}

fn number<T: FromStr>(name: &str, default: T) -> Result<T> {
    match env::var(name) {
        Ok(raw) => raw
            .parse()
            .map_err(|_| format!("invalid numeric value for {name}").into()),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => {
            Err(format!("{name} must be UTF-8 numeric text").into())
        }
    }
}

pub(super) fn limits(grid: &GridBaselineConfig) -> Result<Limits> {
    Ok(Limits {
        candidate_attempts: nonzero(usize::try_from(grid.max_baselines)?)?,
        candidate_bytes: nonzero(32)?,
        public_key_bytes: nonzero(32)?,
        signature_bytes: nonzero(64)?,
        action_bytes: nonzero(64 * 1024)?,
        context_bytes: nonzero(MAX_CONTEXT_JSON_BYTES)?,
    })
}

pub(super) fn package_limits(grid: &GridBaselineConfig) -> Result<PackageLimits> {
    let maximum_bytes = grid
        .max_baselines
        .checked_mul(32)
        .and_then(|bytes| bytes.checked_add(2 * MAX_CONTEXT_JSON_BYTES as u64 + 128))
        .ok_or("enrollment package byte limit overflow")?;
    Ok(PackageLimits {
        max_bytes: nonzero(usize::try_from(maximum_bytes)?)?,
        max_leaves: nonzero(usize::try_from(grid.max_baselines)?)?,
    })
}

pub(super) fn nonzero(value: usize) -> Result<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "resource capacity must be nonzero".into())
}

pub(super) fn read_context(reader: &mut impl Read) -> Result<EnrollmentSpec> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_CONTEXT_JSON_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CONTEXT_JSON_BYTES {
        return Err("enrollment context JSON exceeds its byte limit".into());
    }
    parse_context(serde_json::from_slice(&bytes)?)
}

fn parse_context(value: Value) -> Result<EnrollmentSpec> {
    let fields = value.as_object().ok_or("context must be a JSON object")?;
    let expected = [
        "network",
        "verifier",
        "subject",
        "id",
        "revision",
        "public_salt",
        "secret_policy",
    ];
    if fields.len() != expected.len()
        || fields.keys().any(|name| !expected.contains(&name.as_str()))
    {
        return Err("context must contain exactly the documented public fields".into());
    }
    let id: Hash = hex(text(fields, "id")?)?
        .try_into()
        .map_err(|_| "id must encode exactly 32 bytes")?;
    let revision = fields
        .get("revision")
        .and_then(Value::as_u64)
        .and_then(NonZeroU64::new)
        .ok_or("revision must be a nonzero u64 integer")?;
    let secret_policy = match text(fields, "secret_policy")? {
        "absent" => SecretPolicy::Absent,
        "required" => SecretPolicy::Required,
        _ => return Err("secret_policy must be absent or required".into()),
    };
    Ok(EnrollmentSpec {
        domain: Domain {
            network: text(fields, "network")?.to_owned(),
            verifier: text(fields, "verifier")?.to_owned(),
        },
        subject: text(fields, "subject")?.to_owned(),
        id,
        revision,
        public_salt: hex(fields
            .get("public_salt")
            .and_then(Value::as_str)
            .ok_or("public_salt must be hexadecimal text")?)?,
        secret_policy,
    })
}

fn text<'a>(fields: &'a Map<String, Value>, name: &str) -> Result<&'a str> {
    fields
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} must be a nonempty string").into())
}

fn hex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("hexadecimal public fields must contain complete byte pairs".into());
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = char::from(pair[0])
                .to_digit(16)
                .ok_or("invalid hexadecimal digit")?;
            let low = char::from(pair[1])
                .to_digit(16)
                .ok_or("invalid hexadecimal digit")?;
            Ok((high * 16 + low) as u8)
        })
        .collect()
}

/// Validate the imported selected subsets and retain their exact reconstruction policy.
pub(super) fn parameters(
    input: &CombinationInput,
    grid: &GridBaselineConfig,
) -> Result<ProfileParameters> {
    let controls = combination_controls(&input.metadata)?;
    if controls.min_span_fraction != 1.0 / 3.0 {
        return Err("fingerprint span fraction must equal one third".into());
    }
    let bounds = input
        .metadata
        .get("bounds")
        .and_then(Value::as_object)
        .ok_or("combination report requires bounds")?;
    let bounds = RegionBounds {
        min: point(bounds.get("min"))?,
        max: point(bounds.get("max"))?,
    };
    let derived =
        fingerprint::derive_min_pair_span_units(bounds, input.unit_px, controls.min_span_fraction)?;
    let declared = input
        .metadata
        .get("min_pair_span_units")
        .and_then(Value::as_f64)
        .ok_or("combination report requires numeric min_pair_span_units")?;
    if declared != derived {
        return Err(
            "declared minimum pair span differs from enrolled bounds and ridge unit".into(),
        );
    }
    validate_selected_features(input, &controls, declared * input.unit_px)?;
    ProfileParameters::new(&controls, grid.padding_steps, declared)
}

fn point(value: Option<&Value>) -> Result<Point> {
    let coordinates = value
        .and_then(Value::as_array)
        .filter(|items| items.len() == 2)
        .ok_or("bounds must contain two numeric coordinates")?;
    Ok(Point::new(
        coordinates[0].as_f64().ok_or("bound must be numeric")?,
        coordinates[1].as_f64().ok_or("bound must be numeric")?,
    )?)
}

fn combination_controls(metadata: &Map<String, Value>) -> Result<CombinationConfig> {
    let config = metadata
        .get("config")
        .and_then(Value::as_object)
        .ok_or("combination report requires config")?;
    let expected = [
        "min_points",
        "min_priority",
        "pool_multiplier",
        "radius_units",
        "combinations_per_region",
        "min_span_fraction",
        "max_work",
    ];
    if config.len() != expected.len()
        || config.keys().any(|name| !expected.contains(&name.as_str()))
    {
        return Err("unsupported combination configuration fields".into());
    }
    let controls = CombinationConfig {
        min_points: integer(config, "min_points")?,
        min_priority: numeric(config, "min_priority")?,
        pool_multiplier: integer(config, "pool_multiplier")?,
        radius_units: numeric(config, "radius_units")?,
        combinations_per_region: integer(config, "combinations_per_region")?,
        min_span_fraction: numeric(config, "min_span_fraction")?,
        max_work: integer(config, "max_work")?,
    };
    controls.validate()?;
    Ok(controls)
}

fn numeric(fields: &Map<String, Value>, name: &str) -> Result<f64> {
    fields
        .get(name)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{name} must be numeric").into())
}

fn validate_selected_features(
    input: &CombinationInput,
    controls: &CombinationConfig,
    minimum_px: f64,
) -> Result<()> {
    let mut remaining_work = controls.max_work;
    reserve_validation_work(&mut remaining_work, input.points.len())?;
    let mut points = BTreeMap::new();
    for point in &input.points {
        if points.insert(point.id, point).is_some() {
            return Err(format!("duplicate feature ID {}", point.id).into());
        }
    }
    for (index, combination) in input.combinations.iter().enumerate() {
        let count = combination.feature_ids.len();
        if count < 2 {
            return Err(format!("combination {index} lacks the required source-point pair").into());
        }
        let pair_checks = count
            .checked_mul(count - 1)
            .map(|value| value / 2)
            .ok_or(fingerprint::CombinationError::WorkLimit)?;
        let reserved = count
            .checked_add(pair_checks)
            .ok_or(fingerprint::CombinationError::WorkLimit)?;
        // Reserve the complete pair scan even when the helper finds a pair early.
        reserve_validation_work(&mut remaining_work, reserved)?;
        let mut positions = Vec::with_capacity(count);
        for id in &combination.feature_ids {
            let point = points
                .get(id)
                .ok_or_else(|| format!("combination {index} selects unknown feature {id}"))?;
            if point.priority < controls.min_priority {
                return Err(format!(
                    "combination {index} selects feature {id} below the configured minimum priority"
                )
                .into());
            }
            positions.push(point.position);
        }
        if !fingerprint::satisfies_min_pair_span(&positions, minimum_px, pair_checks)? {
            return Err(format!("combination {index} lacks the required source-point span").into());
        }
    }
    Ok(())
}

fn reserve_validation_work(remaining: &mut usize, amount: usize) -> Result<()> {
    *remaining = remaining
        .checked_sub(amount)
        .ok_or(fingerprint::CombinationError::WorkLimit)?;
    Ok(())
}

fn integer(fields: &Map<String, Value>, name: &str) -> Result<usize> {
    usize::try_from(
        fields
            .get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{name} must be an unsigned integer"))?,
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn context() -> Value {
        json!({"network":"local", "verifier":"example", "subject":"fixture", "id":"01".repeat(32),
            "revision":1, "public_salt":"02".repeat(32), "secret_policy":"absent"})
    }

    #[test]
    fn context_rejects_missing_unknown_malformed_or_unbounded_fields() {
        assert!(parse_context(context()).is_ok());
        for (field, value) in [
            ("network", json!("")),
            ("id", json!("01")),
            ("id", json!("😃")),
            ("revision", json!(0)),
            ("revision", json!(1.5)),
            ("secret_policy", json!("optional")),
            ("public_salt", json!("abc")),
            ("private_key", json!("not-allowed")),
        ] {
            let mut document = context();
            document[field] = value;
            assert!(parse_context(document).is_err(), "accepted {field}");
        }
        assert!(read_context(&mut &[b' '; MAX_CONTEXT_JSON_BYTES + 1][..]).is_err());
        assert!(read_context(&mut &b"{"[..]).is_err());
    }

    #[test]
    fn package_capacity_tracks_baselines_and_rejects_byte_overflow() {
        let mut config = GridBaselineConfig {
            padding_steps: 3,
            max_baselines: 3_000_000,
            max_work: 10_000_000,
        };
        let package = package_limits(&config).unwrap();
        assert_eq!(package.max_leaves.get(), 3_000_000);
        assert_eq!(
            package.max_bytes.get(),
            3_000_000 * 32 + 2 * MAX_CONTEXT_JSON_BYTES + 128
        );
        assert_eq!(limits(&config).unwrap().candidate_attempts.get(), 3_000_000);

        config.max_baselines = u64::MAX;
        assert!(package_limits(&config).is_err());
    }

    fn imported() -> CombinationInput {
        crate::input::parse(json!({
            "schema":"gnorix.feature-combinations.v4", "unit_px":4.0,
            "bounds":{"min":[0,0],"max":[12,12]}, "min_pair_span_units":1.0,
            "config":{"min_points":2,"min_priority":0.5,"pool_multiplier":2,
                "radius_units":30.0,"combinations_per_region":12,"min_span_fraction":1.0/3.0,"max_work":1000},
            "points":[
                {"id":0,"kind":"ending","position":[0,0],"priority":0.5},
                {"id":1,"kind":"ending","position":[10,0],"priority":0.9},
                {"id":2,"kind":"ending","position":[5,10],"priority":0.49}],
            "combinations":[{"feature_ids":[0,1],"score":0.5,"regions":[0]}]
        })).unwrap()
    }

    fn grid() -> GridBaselineConfig {
        GridBaselineConfig {
            padding_steps: 1,
            max_baselines: 10,
            max_work: 1000,
        }
    }

    #[test]
    fn retained_policy_is_canonical_bound_and_independent_of_resource_limits() {
        let mut input = imported();
        let first = parameters(&input, &grid()).unwrap();
        let loaded = ProfileParameters::decode(first.encoded(), first.profile_id()).unwrap();
        assert_eq!(loaded.min_pair_span_units(), 1.0);
        assert_eq!(loaded.uncertainty_steps(), 1);
        assert_eq!(loaded.controls(50).unwrap().min_priority, 0.5);
        input.metadata["config"]["max_work"] = json!(2000);
        assert_eq!(
            first.profile_id(),
            parameters(&input, &grid()).unwrap().profile_id()
        );
        input.metadata["config"]["min_priority"] = json!(0.0);
        let zero = parameters(&input, &grid()).unwrap();
        assert_ne!(first.profile_id(), zero.profile_id());
        input.metadata["config"]["min_priority"] = json!(-0.0);
        assert_eq!(
            zero.profile_id(),
            parameters(&input, &grid()).unwrap().profile_id()
        );
        assert!(ProfileParameters::decode(first.encoded(), [0; 32]).is_err());
        let mut padded = first.encoded().to_vec();
        padded.push(b' ');
        let hash: Hash = <sha2::Sha256 as sha2::Digest>::digest(&padded).into();
        assert!(ProfileParameters::decode(&padded, hash).is_err());
    }

    #[test]
    fn imported_subsets_must_meet_priority_and_span_without_silent_filtering() {
        let mut input = imported();
        assert!(parameters(&input, &grid()).is_ok());
        input.combinations[0].feature_ids = vec![1, 2];
        assert!(
            parameters(&input, &grid())
                .unwrap_err()
                .to_string()
                .contains("priority")
        );
        let mut input = imported();
        input.points[1].position = Point::new(1.0, 0.0).unwrap();
        assert!(
            parameters(&input, &grid())
                .unwrap_err()
                .to_string()
                .contains("span")
        );
    }

    #[test]
    fn imports_reject_wrong_fraction_derived_distance_and_old_policy() {
        let mut input = imported();
        input.metadata["min_pair_span_units"] = json!(1.1);
        assert!(parameters(&input, &grid()).is_err());
        let mut input = imported();
        input.metadata["config"]["min_span_fraction"] = json!(0.5);
        assert!(parameters(&input, &grid()).is_err());
        let mut input = imported();
        input.metadata["config"]
            .as_object_mut()
            .unwrap()
            .remove("min_span_fraction");
        assert!(parameters(&input, &grid()).is_err());
    }

    #[test]
    fn imported_span_validation_has_one_aggregate_work_budget() {
        let mut input = imported();
        input.points[2].priority = 0.8;
        input.combinations.push(input.combinations[0].clone());
        input.combinations[1].feature_ids = vec![1, 2];
        // Three source lookups plus two subsets, each reserving two point
        // lookups and one pair check, require exactly nine work entries.
        input.metadata["config"]["max_work"] = json!(8);
        let error = parameters(&input, &grid()).unwrap_err();
        assert_eq!(
            error.downcast_ref::<fingerprint::CombinationError>(),
            Some(&fingerprint::CombinationError::WorkLimit)
        );
        input.metadata["config"]["max_work"] = json!(9);
        assert!(parameters(&input, &grid()).is_ok());
    }
}
