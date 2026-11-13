//! Conservative enrollment capacity estimates, separate from biometric rules.
//!
//! Reserve 512 MiB for working data and 96 bytes per retained leaf, then budget
//! 512 bytes per generated variant for transient trees and buffers. This is
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
const BYTES_PER_VARIANT: u64 = 512;
const BYTES_PER_RETAINED_LEAF: u64 = 96;
const PACKAGE_ALLOWANCE: u64 = 128 * 1024;

pub fn default_memory_budget_gib() -> u8 {
    DEFAULT_MEMORY_GIB
}

#[derive(Clone, Copy, Debug)]
pub struct EnrollmentBudget {
    memory_gib: u8,
    budget_bytes: u64,
    reserved_bytes: u64,
    variant_limit: u64,
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
            .filter(|&bytes| bytes >= BYTES_PER_VARIANT)
            .ok_or_else(|| format!("the {memory_gib} GiB enrollment budget cannot cover the existing retained set and the working reserve; select a larger budget"))?;
        let variant_limit = available_bytes / BYTES_PER_VARIANT;
        Ok(Self {
            memory_gib,
            budget_bytes,
            reserved_bytes,
            variant_limit,
        })
    }

    pub fn memory_budget_gib(self) -> u8 {
        self.memory_gib
    }

    pub fn variant_limit(self) -> u64 {
        self.variant_limit
    }

    pub fn estimated_peak_bytes(self, variants: u64) -> Result<u64> {
        let estimate = variants
            .checked_mul(BYTES_PER_VARIANT)
            .and_then(|working| self.reserved_bytes.checked_add(working))
            .ok_or("enrollment memory estimate overflow")?;
        if estimate > self.budget_bytes {
            return Err(format!("enrollment requires {variants} variants with an estimated peak of {estimate} bytes; the selected {} GiB budget permits {} variants. Increase the enrollment memory budget or reduce combinations or grid uncertainty", self.memory_gib, self.variant_limit).into());
        }
        Ok(estimate)
    }

    pub fn package_limits(self) -> Result<PackageLimits> {
        let bytes = self
            .variant_limit
            .checked_mul(32)
            .and_then(|leaves| leaves.checked_add(PACKAGE_ALLOWANCE))
            .ok_or("enrollment package byte limit overflow")?;
        Ok(PackageLimits {
            max_bytes: NonZeroUsize::new(usize::try_from(bytes)?)
                .ok_or("package byte limit is zero")?,
            max_leaves: NonZeroUsize::new(usize::try_from(self.variant_limit)?)
                .ok_or("package leaf limit is zero")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnorix::biometrics::{
        fingerprint::{
            self, CombinationPoint, FeatureCombination, FeatureKind, GridShapeConfig,
            GridShapeError,
        },
        geometry::Point,
    };

    #[test]
    fn default_range_and_resource_arithmetic_are_explicit() {
        assert_eq!(default_memory_budget_gib(), 2);
        let minimum = EnrollmentBudget::new(MIN_MEMORY_GIB, 0).unwrap();
        assert_eq!(minimum.variant_limit(), 1_048_576);
        let default = EnrollmentBudget::new(DEFAULT_MEMORY_GIB, 0).unwrap();
        assert_eq!(default.variant_limit(), 3_145_728);
        for gib in [MIN_MEMORY_GIB, DEFAULT_MEMORY_GIB, MAX_MEMORY_GIB] {
            let budget = EnrollmentBudget::new(gib, 0).unwrap();
            assert_eq!(budget.memory_budget_gib(), gib);
            assert!(
                budget.estimated_peak_bytes(budget.variant_limit()).unwrap()
                    <= u64::from(gib) * GIB
            );
            assert!(
                budget
                    .estimated_peak_bytes(budget.variant_limit() + 1)
                    .is_err()
            );
            let package = budget.package_limits().unwrap();
            assert_eq!(package.max_leaves.get() as u64, budget.variant_limit());
            assert_eq!(
                package.max_bytes.get() as u64,
                budget.variant_limit() * 32 + PACKAGE_ALLOWANCE
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
        assert!(budget.variant_limit() < empty.variant_limit());
        assert_eq!(
            budget.estimated_peak_bytes(100).unwrap(),
            FIXED_RESERVE + 96_000_000 + 51_200
        );
        assert!(EnrollmentBudget::new(1, GIB / BYTES_PER_RETAINED_LEAF).is_err());
    }

    #[test]
    fn default_budget_prepares_the_reported_million_variant_case_without_enumeration() {
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
        let prepare = |limit| {
            fingerprint::prepare_grid_shapes(
                &points,
                &combinations,
                4.0,
                &GridShapeConfig {
                    uncertainty_steps: 1,
                    max_variants: limit,
                    max_work: 1_000_000,
                },
            )
        };
        let required = 1_075_648;
        assert_eq!(
            prepare(1_000_000).unwrap_err(),
            GridShapeError::VariantLimit {
                required,
                limit: 1_000_000
            }
        );
        let small = EnrollmentBudget::new(1, 0).unwrap();
        assert!(small.estimated_peak_bytes(required).is_err());
        assert!(prepare(small.variant_limit()).is_err());
        let default = EnrollmentBudget::new(2, 0).unwrap();
        let grid = prepare(default.variant_limit()).unwrap();
        assert_eq!(grid.variant_count(), required);
        assert_eq!(
            default.estimated_peak_bytes(required).unwrap(),
            FIXED_RESERVE + required * BYTES_PER_VARIANT
        );
    }
}
