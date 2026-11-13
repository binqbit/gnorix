//! Stable imported-grid interpretation shared by enrollment and reconstruction.

use std::error::Error;

use gnorix::{biometrics::fingerprint::CombinationConfig, merkle::Hash};
use sha2::{Digest, Sha256};

/// Identify candidate interpretation, excluding source pixels and search budgets.
/// This intentionally preserves the existing imported-grid profile byte framing.
pub(super) fn profile_id(
    controls: &CombinationConfig,
    uncertainty_steps: u32,
) -> Result<Hash, Box<dyn Error + Send + Sync>> {
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
        &uncertainty_steps.to_be_bytes(),
    ] {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }
    Ok(hasher.finalize().into())
}
