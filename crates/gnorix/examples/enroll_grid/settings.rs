//! Public enrollment input, resource bounds and the imported-grid profile.

use std::{
    env,
    io::Read,
    num::{NonZeroU64, NonZeroUsize},
    str::FromStr,
};

use gnorix::{
    biometrics::fingerprint::{CombinationConfig, GridBaselineConfig},
    enrollment::{Domain, EnrollmentSpec},
    merkle::Hash,
    package::PackageLimits,
    policy::{Limits, SecretPolicy},
};
use serde_json::{Map, Value};

use super::{Result, enrollment_profile};

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
        .and_then(|bytes| bytes.checked_add(MAX_CONTEXT_JSON_BYTES as u64 + 128))
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

/// Identify the imported-combination interpretation, not an entire capture engine.
/// Sampling IDs, image scale and resource-only work limits do not enter this ID.
pub(super) fn profile_id(metadata: &Map<String, Value>, grid: &GridBaselineConfig) -> Result<Hash> {
    let config = metadata
        .get("config")
        .and_then(Value::as_object)
        .ok_or("combination report requires config")?;
    let expected = [
        "min_points",
        "pool_multiplier",
        "radius_units",
        "combinations_per_region",
        "max_work",
    ];
    if config.len() != expected.len()
        || config.keys().any(|name| !expected.contains(&name.as_str()))
    {
        return Err("unsupported combination configuration fields".into());
    }
    let controls = CombinationConfig {
        min_points: integer(config, "min_points")?,
        pool_multiplier: integer(config, "pool_multiplier")?,
        radius_units: config
            .get("radius_units")
            .and_then(Value::as_f64)
            .ok_or("radius_units must be numeric")?,
        combinations_per_region: integer(config, "combinations_per_region")?,
        max_work: integer(config, "max_work")?,
    };
    enrollment_profile::profile_id(&controls, grid.padding_steps)
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
            3_000_000 * 32 + MAX_CONTEXT_JSON_BYTES + 128
        );
        assert_eq!(limits(&config).unwrap().candidate_attempts.get(), 3_000_000);

        config.max_baselines = u64::MAX;
        assert!(package_limits(&config).is_err());
    }

    #[test]
    fn profile_tracks_selection_and_uncertainty_but_not_pixels_or_limits() {
        let mut metadata = json!({"config":{"min_points":3,"pool_multiplier":2,
            "radius_units":30.0,"combinations_per_region":12,"max_work":10000000},
            "source":{"image":"a.png"},"unit_px":16.8})
        .as_object()
        .unwrap()
        .clone();
        let mut grid = GridBaselineConfig {
            padding_steps: 1,
            max_baselines: 1000000,
            max_work: 10000000,
        };
        let baseline = profile_id(&metadata, &grid).unwrap();
        let previous_profile =
            hex("8bd0d14924be772d55dc471ce7c448f72d8a926c29f70022e98b59627413494b").unwrap();
        assert_ne!(baseline.as_slice(), previous_profile);
        let expanded_profile =
            hex("9285906b7fb81701a2059b4e59dc5b7b27028b1299f1feac02e848cf63f141dc").unwrap();
        assert_ne!(baseline.as_slice(), expanded_profile);
        metadata.insert("unit_px".into(), json!(33.6));
        metadata.insert("source".into(), json!({"image":"b.png"}));
        metadata.get_mut("config").unwrap()["max_work"] = json!(20000000);
        grid.max_work *= 2;
        grid.max_baselines *= 2;
        assert_eq!(baseline, profile_id(&metadata, &grid).unwrap());
        grid.padding_steps = 2;
        assert_ne!(baseline, profile_id(&metadata, &grid).unwrap());
        grid.padding_steps = 1;
        metadata.get_mut("config").unwrap()["radius_units"] = json!(31.0);
        assert_ne!(baseline, profile_id(&metadata, &grid).unwrap());
    }

    #[test]
    fn profile_rejects_obsolete_or_invalid_region_configuration() {
        let metadata = json!({"config":{"min_points":3,"pool_multiplier":2,
            "radius_units":30.0,"combinations_per_region":12,"max_work":10000000}})
        .as_object()
        .unwrap()
        .clone();
        let grid = GridBaselineConfig {
            padding_steps: 1,
            max_baselines: 1000000,
            max_work: 10000000,
        };
        for invalid in [json!(0), json!(-1), json!("30"), json!(null)] {
            let mut changed = metadata.clone();
            changed.get_mut("config").unwrap()["radius_units"] = invalid;
            assert!(profile_id(&changed, &grid).is_err());
        }
        let mut obsolete = metadata;
        let config = obsolete.get_mut("config").unwrap().as_object_mut().unwrap();
        config.remove("radius_units");
        config.insert("diameter_fraction".into(), json!(0.5));
        assert!(profile_id(&obsolete, &grid).is_err());
    }
}
