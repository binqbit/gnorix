//! Versioned regional-grid interpretation shared by enrollment and reconstruction.

use std::error::Error;

use gnorix::{biometrics::fingerprint::CombinationConfig, merkle::Hash};
use sha2::{Digest, Sha256};

/// Identify candidate interpretation, excluding source pixels and search budgets.
/// Baseline-only enrollment and lazy signing uncertainty deliberately use a
/// new profile rather than accepting earlier expanded enrollment packages.
pub(super) fn profile_id(
    controls: &CombinationConfig,
    uncertainty_steps: u32,
) -> Result<Hash, Box<dyn Error + Send + Sync>> {
    controls.validate()?;
    let mut hasher = Sha256::new();
    for field in [
        b"gnorix.example.imported-grid-profile.v3".as_slice(),
        b"gnorix.feature-combinations.v2",
        b"gnorix.grid-candidate.v1",
        b"baseline-only-enrollment;lazy-per-subset-uncertainty-signing.v1",
        b"gnorix.adaptive-regions.v1;radius-in-ridge-units;pattern-and-eligible-point-bounds;row-major-cell-centers",
        b"longest-hull-edge;top-right-origin;mean-ridge-spacing;fixed-padding;retained-collisions",
        &(controls.min_points as u64).to_be_bytes(),
        &(controls.pool_multiplier as u64).to_be_bytes(),
        &controls.radius_units.to_be_bytes(),
        &(controls.combinations_per_region as u64).to_be_bytes(),
        &uncertainty_steps.to_be_bytes(),
    ] {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }
    Ok(hasher.finalize().into())
}
