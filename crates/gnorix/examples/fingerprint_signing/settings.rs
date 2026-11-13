//! Search-only budgets, separated from the enrolled coordinate interpretation.

use super::{Result, combination_config, enrollment_profile};
use gnorix::{
    biometrics::fingerprint::SigningSearchConfig, merkle::Hash, package::PackageLimits,
    policy::Limits,
};
use std::{env, num::NonZeroUsize, str::FromStr};

pub(super) struct Controls {
    pub search: SigningSearchConfig,
    pub attempts: usize,
    pub profile_id: Hash,
}

pub(super) fn read() -> Result<Controls> {
    let combination = combination_config::read()?;
    let uncertainty_steps = read_number("GNORIX_GRID_UNCERTAINTY_STEPS", 1_u32)?;
    let attempts = number("GNORIX_SIGNING_MAX_ATTEMPTS", 1_000_000)?;
    let work = number("GNORIX_SIGNING_MAX_WORK", 1_000_000_000)?;
    let profile_id = enrollment_profile::profile_id(&combination, uncertainty_steps)?;
    let search = SigningSearchConfig {
        min_points: combination.min_points,
        radius_units: combination.radius_units,
        uncertainty_steps,
        max_work: work,
        max_descriptor_bytes: nonzero(1024 * 1024)?,
    };
    search.validate()?;
    Ok(Controls {
        search,
        attempts,
        profile_id,
    })
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
        .and_then(|bytes| bytes.checked_add(64 * 1024 + 128))
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
