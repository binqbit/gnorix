//! Conservative enrollment capacity estimates, separate from biometric rules.
//!
//! Reserve 512 MiB for working data and 96 bytes per retained leaf, then budget
//! 512 bytes per baseline candidate for transient trees and buffers. This is
//! admission control for the local fixture workflow, not an OS memory limit.
//! The same capacity governs traversal, key derivation and package readback.

use std::num::NonZeroUsize;

use gnorix::package::PackageLimits;

use crate::Result;

pub const MIN_MEMORY_GIB: u8 = 1;
pub const DEFAULT_MEMORY_GIB: u8 = 2;
pub const MAX_MEMORY_GIB: u8 = 10;

const GIB: u64 = 1024 * 1024 * 1024;
const FIXED_RESERVE: u64 = 512 * 1024 * 1024;
const BYTES_PER_CANDIDATE: u64 = 512;
const BYTES_PER_RETAINED_LEAF: u64 = 96;
const PACKAGE_ALLOWANCE: u64 = 128 * 1024 + 128;

pub fn default_memory_budget_gib() -> u8 {
    DEFAULT_MEMORY_GIB
}

#[derive(Clone, Copy, Debug)]
pub struct EnrollmentBudget {
    memory_gib: u8,
    budget_bytes: u64,
    reserved_bytes: u64,
    candidate_limit: u64,
}

impl EnrollmentBudget {
    pub fn new(memory_gib: u8, retained_leaves: u64) -> Result<Self> {
        if !(MIN_MEMORY_GIB..=MAX_MEMORY_GIB).contains(&memory_gib) {
            return Err(format!(
                "enrollment memory budget must be between {MIN_MEMORY_GIB} and {MAX_MEMORY_GIB} GiB"
            )
            .into());
        }
        let budget_bytes = u64::from(memory_gib)
            .checked_mul(GIB)
            .ok_or("memory budget overflow")?;
        let reserved_bytes = retained_leaves
            .checked_mul(BYTES_PER_RETAINED_LEAF)
            .and_then(|retained| FIXED_RESERVE.checked_add(retained))
            .ok_or("retained enrollment estimate overflow")?;
        let available_bytes = budget_bytes.checked_sub(reserved_bytes)
            .filter(|&bytes| bytes >= BYTES_PER_CANDIDATE)
            .ok_or_else(|| format!("the {memory_gib} GiB enrollment budget cannot cover the existing retained set and the working reserve; select a larger budget"))?;
        let candidate_limit = available_bytes / BYTES_PER_CANDIDATE;
        Ok(Self {
            memory_gib,
            budget_bytes,
            reserved_bytes,
            candidate_limit,
        })
    }

    pub fn memory_budget_gib(self) -> u8 {
        self.memory_gib
    }

    pub fn candidate_limit(self) -> u64 {
        self.candidate_limit
    }

    pub fn estimated_peak_bytes(self, candidates: u64) -> Result<u64> {
        let estimate = candidates
            .checked_mul(BYTES_PER_CANDIDATE)
            .and_then(|working| self.reserved_bytes.checked_add(working))
            .ok_or("enrollment memory estimate overflow")?;
        if estimate > self.budget_bytes {
            return Err(format!("enrollment requires {candidates} candidates with an estimated peak of {estimate} bytes; the selected {} GiB budget permits {} candidates. Increase the enrollment memory budget or reduce selected combinations", self.memory_gib, self.candidate_limit).into());
        }
        Ok(estimate)
    }

    pub fn package_limits(self) -> Result<PackageLimits> {
        let bytes = self
            .candidate_limit
            .checked_mul(32)
            .and_then(|leaves| leaves.checked_add(PACKAGE_ALLOWANCE))
            .ok_or("enrollment package byte limit overflow")?;
        Ok(PackageLimits {
            max_bytes: NonZeroUsize::new(usize::try_from(bytes)?)
                .ok_or("package byte limit is zero")?,
            max_leaves: NonZeroUsize::new(usize::try_from(self.candidate_limit)?)
                .ok_or("package leaf limit is zero")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnorix::biometrics::{
        fingerprint::{
            self, CombinationPoint, FeatureCombination, FeatureKind, GridBaselineConfig,
            GridShapeConfig, GridShapeError,
        },
        geometry::Point,
    };

    #[test]
    fn default_range_and_resource_arithmetic_are_explicit() {
        assert_eq!(default_memory_budget_gib(), 2);
        let minimum = EnrollmentBudget::new(MIN_MEMORY_GIB, 0).unwrap();
        assert_eq!(minimum.candidate_limit(), 1_048_576);
        let default = EnrollmentBudget::new(DEFAULT_MEMORY_GIB, 0).unwrap();
        assert_eq!(default.candidate_limit(), 3_145_728);
        for gib in [MIN_MEMORY_GIB, DEFAULT_MEMORY_GIB, MAX_MEMORY_GIB] {
            let budget = EnrollmentBudget::new(gib, 0).unwrap();
            assert_eq!(budget.memory_budget_gib(), gib);
            assert!(
                budget
                    .estimated_peak_bytes(budget.candidate_limit())
                    .unwrap()
                    <= u64::from(gib) * GIB
            );
            assert!(
                budget
                    .estimated_peak_bytes(budget.candidate_limit() + 1)
                    .is_err()
            );
            let package = budget.package_limits().unwrap();
            assert_eq!(package.max_leaves.get() as u64, budget.candidate_limit());
            assert_eq!(
                package.max_bytes.get() as u64,
                budget.candidate_limit() * 32 + PACKAGE_ALLOWANCE
            );
        }
        for gib in [0, MAX_MEMORY_GIB + 1, u8::MAX] {
            assert!(EnrollmentBudget::new(gib, 0).is_err());
        }
        assert!(EnrollmentBudget::new(2, u64::MAX).is_err());
        assert!(default.estimated_peak_bytes(u64::MAX).is_err());
    }

    #[test]
    fn retained_set_reduces_available_capacity() {
        let budget = EnrollmentBudget::new(2, 1_000_000).unwrap();
        let empty = EnrollmentBudget::new(2, 0).unwrap();
        assert!(budget.candidate_limit() < empty.candidate_limit());
        assert_eq!(
            budget.estimated_peak_bytes(100).unwrap(),
            FIXED_RESERVE + 96_000_000 + 51_200
        );
        assert!(EnrollmentBudget::new(1, GIB / BYTES_PER_RETAINED_LEAF).is_err());
    }

    #[test]
    fn baseline_enrollment_avoids_the_reported_million_variant_expansion() {
        let points: Vec<_> = (0..14)
            .map(|id| CombinationPoint {
                id,
                kind: FeatureKind::Ending,
                position: Point::new(id as f64 * 13.0, (id * id) as f64).unwrap(),
                priority: 1.0,
            })
            .collect();
        let mut combinations: Vec<_> = (5..14)
            .map(|extra| FeatureCombination {
                feature_ids: (0..5).chain(std::iter::once(extra)).collect(),
                score: 1.0,
                regions: vec![0],
            })
            .collect();
        combinations.push(FeatureCombination {
            feature_ids: (0..5).collect(),
            score: 1.0,
            regions: vec![0],
        });
        let expanded = fingerprint::prepare_grid_shapes(
            &points,
            &combinations,
            4.0,
            &GridShapeConfig {
                uncertainty_steps: 1,
                max_variants: 1_000_000,
                max_work: 1_000_000,
            },
        );
        assert_eq!(
            expanded.unwrap_err(),
            GridShapeError::VariantLimit {
                required: 1_075_648,
                limit: 1_000_000,
            }
        );
        let budget = EnrollmentBudget::new(1, 0).unwrap();
        let baseline = fingerprint::prepare_grid_baselines(
            &points,
            &combinations,
            4.0,
            &GridBaselineConfig {
                padding_steps: 1,
                max_baselines: budget.candidate_limit(),
                max_work: 1_000_000,
            },
        )
        .unwrap();
        assert_eq!(baseline.baseline_count(), 10);
        assert_eq!(
            baseline
                .candidates(NonZeroUsize::new(1024).unwrap())
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
                .len(),
            10
        );
        assert_eq!(
            budget
                .estimated_peak_bytes(baseline.baseline_count())
                .unwrap(),
            FIXED_RESERVE + 10 * BYTES_PER_CANDIDATE
        );
    }
}
