//! Search-only budgets, separated from the enrolled coordinate interpretation.

use super::{Result, combination_config, enrollment_profile, grid_config};
use gnorix::{
    biometrics::fingerprint::SigningSearchConfig, merkle::Hash, package::PackageLimits,
    policy::Limits,
};
use std::{env, num::NonZeroUsize};

pub(super) struct Controls {
    pub search: SigningSearchConfig,
    pub attempts: usize,
    pub profile_id: Hash,
}

pub(super) fn read() -> Result<Controls> {
    let combination = combination_config::read()?;
    let grid = grid_config::read()?;
    let attempts = number("GNORIX_SIGNING_MAX_ATTEMPTS", 1_000_000)?;
    let work = number("GNORIX_SIGNING_MAX_WORK", 1_000_000_000)?;
    let profile_id = enrollment_profile::profile_id(&combination, grid.uncertainty_steps)?;
    Ok(Controls {
        search: SigningSearchConfig {
            min_points: combination.min_points,
            enrollment_padding: grid.uncertainty_steps,
            max_work: work,
            max_descriptor_bytes: nonzero(1024 * 1024)?,
        },
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
    Ok(PackageLimits {
        max_bytes: nonzero(64 * 1024 * 1024)?,
        max_leaves: nonzero(1_000_000)?,
    })
}

fn number(name: &str, default: usize) -> Result<usize> {
    let value = match env::var(name) {
        Ok(raw) => raw.parse()?,
        Err(env::VarError::NotPresent) => default,
        Err(env::VarError::NotUnicode(_)) => {
            return Err(format!("{name} must be UTF-8 numeric text").into());
        }
    };
    nonzero(value)?;
    Ok(value)
}

fn nonzero(value: usize) -> Result<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "example resource limit must be nonzero".into())
}
