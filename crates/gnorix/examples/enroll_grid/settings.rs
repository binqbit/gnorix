//! Public enrollment input, resource bounds and the imported-grid profile.

use std::{
    env,
    io::Read,
    num::{NonZeroU64, NonZeroUsize},
};

use gnorix::{
    biometrics::fingerprint::{CombinationConfig, GridShapeConfig},
    crypto::SecretInput,
    enrollment::{Domain, EnrollmentSpec},
    merkle::Hash,
    package::PackageLimits,
    policy::{Limits, SecretPolicy},
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::Result;

pub(super) const MAX_CONTEXT_JSON_BYTES: usize = 64 * 1024;
pub(super) const MAX_SECRET_BYTES: usize = 4096;
pub(super) const MAX_DESCRIPTOR_BYTES: usize = 1024 * 1024;

pub(super) fn limits(grid: &GridShapeConfig) -> Result<Limits> {
    Ok(Limits {
        candidate_attempts: nonzero(usize::try_from(grid.max_variants)?)?,
        candidate_bytes: nonzero(32)?,
        public_key_bytes: nonzero(32)?,
        signature_bytes: nonzero(64)?,
        action_bytes: nonzero(64 * 1024)?,
        context_bytes: nonzero(MAX_CONTEXT_JSON_BYTES)?,
    })
}

pub(super) fn package_limits(grid: &GridShapeConfig) -> Result<PackageLimits> {
    Ok(PackageLimits {
        max_bytes: nonzero(64 * 1024 * 1024)?,
        max_leaves: nonzero(usize::try_from(grid.max_variants)?)?,
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

pub(super) fn read_secret(policy: SecretPolicy) -> Result<Option<SecretInput>> {
    let secret = match env::var("GNORIX_ENROLL_SECRET") {
        Ok(value) => {
            let secret = SecretInput::new(value.into_bytes())?;
            if secret.as_bytes().len() > MAX_SECRET_BYTES {
                return Err("secret input exceeds the example byte limit".into());
            }
            Some(secret)
        }
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => return Err("secret input must be UTF-8".into()),
    };
    if !matches!(
        (policy, &secret),
        (SecretPolicy::Absent, None) | (SecretPolicy::Required, Some(_))
    ) {
        return Err(gnorix::Error::SecretPolicyMismatch.into());
    }
    Ok(secret)
}

/// Identify the imported-combination interpretation, not an entire capture engine.
/// Sampling IDs, image scale and resource-only work limits do not enter this ID.
pub(super) fn profile_id(metadata: &Map<String, Value>, grid: &GridShapeConfig) -> Result<Hash> {
    let config = metadata
        .get("config")
        .and_then(Value::as_object)
        .ok_or("combination report requires config")?;
    let expected = [
        "min_points",
        "pool_multiplier",
        "diameter_fraction",
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
        diameter_fraction: config
            .get("diameter_fraction")
            .and_then(Value::as_f64)
            .ok_or("diameter_fraction must be numeric")?,
        combinations_per_region: integer(config, "combinations_per_region")?,
        max_work: integer(config, "max_work")?,
    };
    controls.validate()?;
    let mut hasher = Sha256::new();
    for field in [
        b"gnorix.example.imported-grid-profile.v1".as_slice(),
        b"gnorix.feature-combinations.v1",
        b"gnorix.grid-candidate.v1",
        b"longest-hull-edge;top-right-origin;mean-ridge-spacing;fixed-padding;retained-collisions",
        &(controls.min_points as u64).to_be_bytes(),
        &(controls.pool_multiplier as u64).to_be_bytes(),
        &controls.diameter_fraction.to_be_bytes(),
        &(controls.combinations_per_region as u64).to_be_bytes(),
        &grid.uncertainty_steps.to_be_bytes(),
    ] {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }
    Ok(hasher.finalize().into())
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
    fn profile_tracks_selection_and_uncertainty_but_not_pixels_or_limits() {
        let mut metadata = json!({"config":{"min_points":3,"pool_multiplier":2,
            "diameter_fraction":0.5,"combinations_per_region":12,"max_work":10000000},
            "source":{"image":"a.png"},"unit_px":16.8})
        .as_object()
        .unwrap()
        .clone();
        let mut grid = GridShapeConfig {
            uncertainty_steps: 1,
            max_variants: 1000000,
            max_work: 10000000,
        };
        let baseline = profile_id(&metadata, &grid).unwrap();
        metadata.insert("unit_px".into(), json!(33.6));
        metadata.insert("source".into(), json!({"image":"b.png"}));
        metadata.get_mut("config").unwrap()["max_work"] = json!(20000000);
        grid.max_work *= 2;
        grid.max_variants *= 2;
        assert_eq!(baseline, profile_id(&metadata, &grid).unwrap());
        grid.uncertainty_steps = 2;
        assert_ne!(baseline, profile_id(&metadata, &grid).unwrap());
        grid.uncertainty_steps = 1;
        metadata.get_mut("config").unwrap()["diameter_fraction"] = json!(0.6);
        assert_ne!(baseline, profile_id(&metadata, &grid).unwrap());
    }
}
