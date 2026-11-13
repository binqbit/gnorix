//! Search-only budgets, separated from the enrolled coordinate interpretation.

use super::{Result, enrollment_profile::ProfileParameters};
use gnorix::{
    biometrics::fingerprint::SigningSearchConfig, merkle::Hash, package::EnrollmentPackage,
    package::PackageLimits, policy::Limits,
};
use std::{env, num::NonZeroUsize, str::FromStr};

pub(super) struct Controls {
    pub search: SigningSearchConfig,
    pub profile_id: Hash,
}

pub(super) fn attempt_limit() -> Result<usize> {
    number("GNORIX_SIGNING_MAX_ATTEMPTS", 1_000_000)
}

pub(super) fn parameters(package: &EnrollmentPackage) -> Result<ProfileParameters> {
    ProfileParameters::decode(
        package
            .profile_parameters()
            .ok_or("fingerprint package lacks retained reconstruction parameters")?,
        package.commitment().context.profile_id,
    )
}

pub(super) fn read(package: &EnrollmentPackage) -> Result<Controls> {
    let retained = parameters(package)?;
    let work = number("GNORIX_SIGNING_MAX_WORK", 1_000_000_000)?;
    let combination = retained.controls(work)?;
    require_override("GNORIX_COMBINATION_MIN_POINTS", combination.min_points)?;
    require_override("GNORIX_COMBINATION_MIN_PRIORITY", combination.min_priority)?;
    require_override(
        "GNORIX_COMBINATION_POOL_MULTIPLIER",
        combination.pool_multiplier,
    )?;
    require_override("GNORIX_COMBINATION_RADIUS_UNITS", combination.radius_units)?;
    require_override(
        "GNORIX_COMBINATION_PER_REGION",
        combination.combinations_per_region,
    )?;
    require_override(
        "GNORIX_GRID_UNCERTAINTY_STEPS",
        retained.uncertainty_steps(),
    )?;
    let search = SigningSearchConfig {
        min_points: combination.min_points,
        min_priority: combination.min_priority,
        radius_units: combination.radius_units,
        min_pair_span_units: retained.min_pair_span_units(),
        uncertainty_steps: retained.uncertainty_steps(),
        max_work: work,
        max_descriptor_bytes: nonzero(1024 * 1024)?,
    };
    search.validate()?;
    Ok(Controls {
        search,
        profile_id: retained.profile_id(),
    })
}

fn require_override<T: FromStr + PartialEq + Copy>(name: &str, retained: T) -> Result<()> {
    let value = read_number(name, retained)?;
    if value != retained {
        return Err(format!("{name} differs from the saved fingerprint policy").into());
    }
    Ok(())
}

pub(super) fn limits(attempts: usize) -> Result<Limits> {
    Ok(Limits {
        candidate_attempts: nonzero(attempts)?,
        candidate_bytes: nonzero(32)?,
        public_key_bytes: nonzero(32)?,
        signature_bytes: nonzero(64)?,
        action_bytes: nonzero(64 * 1024)?,
        context_bytes: nonzero(64 * 1024)?,
    })
}

pub(super) fn package_limits() -> Result<PackageLimits> {
    let max_leaves = number("GNORIX_ENROLLMENT_MAX_CANDIDATES", 1_000_000)?;
    let max_bytes = max_leaves
        .checked_mul(32)
        .and_then(|bytes| bytes.checked_add(2 * 64 * 1024 + 128))
        .ok_or("package byte limit overflow")?;
    Ok(PackageLimits {
        max_bytes: nonzero(max_bytes)?,
        max_leaves: nonzero(max_leaves)?,
    })
}

fn number(name: &str, default: usize) -> Result<usize> {
    let value = read_number(name, default)?;
    nonzero(value)?;
    Ok(value)
}

fn read_number<T: FromStr>(name: &str, default: T) -> Result<T> {
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

fn nonzero(value: usize) -> Result<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "example resource limit must be nonzero".into())
}
