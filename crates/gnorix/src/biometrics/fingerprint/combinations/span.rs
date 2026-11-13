//! Source-space span admission, independent of snapping and uncertainty.

use super::{CombinationError, CombinationPoint, Point, RegionBounds, WorkBudget};

/// Whether at least one source-point pair spans `minimum_px` inclusively.
/// Zero disables the rule. Work bounds the inspected pairs, including failures.
pub fn satisfies_min_pair_span(
    positions: &[Point],
    minimum_px: f64,
    max_work: usize,
) -> Result<bool, CombinationError> {
    validate(minimum_px)?;
    if max_work == 0 {
        return Err(CombinationError::InvalidConfig(
            "work limit must be nonzero",
        ));
    }
    matches(
        positions.len(),
        |index| positions[index],
        minimum_px,
        &mut WorkBudget::new(max_work),
    )
}

pub(super) fn validate(minimum: f64) -> Result<(), CombinationError> {
    if !minimum.is_finite() || minimum < 0.0 {
        return Err(CombinationError::InvalidConfig(
            "minimum pair span must be finite and nonnegative",
        ));
    }
    Ok(())
}

/// Derive the fixed enrollment span from its observed pattern/eligible bounds.
pub fn derive_min_pair_span_units(
    bounds: RegionBounds,
    unit: f64,
    fraction: f64,
) -> Result<f64, CombinationError> {
    if !unit.is_finite() || unit <= 0.0 {
        return Err(CombinationError::InvalidFingerprint(
            super::FingerprintError::ScaleUnavailable("ridge width must be finite and positive"),
        ));
    }
    if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
        return Err(CombinationError::InvalidConfig(
            "minimum span fraction must be finite and in [0, 1]",
        ));
    }
    if bounds.max.x() < bounds.min.x() || bounds.max.y() < bounds.min.y() {
        return Err(CombinationError::InvalidConfig(
            "span bounds must not be inverted",
        ));
    }
    let shorter = (bounds.max.x() - bounds.min.x()).min(bounds.max.y() - bounds.min.y());
    let units = fraction * shorter / unit;
    validate(units)?;
    Ok(units)
}

pub(super) fn pixels(units: f64, unit: f64) -> Result<f64, CombinationError> {
    let minimum = units * unit;
    validate(minimum)?;
    Ok(minimum)
}

fn separated(
    a: Point,
    b: Point,
    minimum: f64,
    work: &mut WorkBudget,
) -> Result<bool, CombinationError> {
    work.spend(1)?;
    Ok((a.x() - b.x()).hypot(a.y() - b.y()) >= minimum)
}

fn matches(
    count: usize,
    position: impl Fn(usize) -> Point,
    minimum: f64,
    work: &mut WorkBudget,
) -> Result<bool, CombinationError> {
    if minimum == 0.0 {
        return Ok(true);
    }
    for first in 0..count {
        for second in first + 1..count {
            if separated(position(first), position(second), minimum, work)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) fn subset(
    positions: &[Point],
    ranks: &[usize],
    minimum: f64,
    work: &mut WorkBudget,
) -> Result<bool, CombinationError> {
    matches(ranks.len(), |index| positions[ranks[index]], minimum, work)
}

pub(super) fn point_subset(
    points: &[CombinationPoint],
    ranks: &[usize],
    minimum: f64,
    work: &mut WorkBudget,
) -> Result<bool, CombinationError> {
    matches(
        ranks.len(),
        |index| points[ranks[index]].position,
        minimum,
        work,
    )
}

/// Complete required ranks with the strongest remaining points, then restore rank order.
fn complete(
    required: &[usize],
    size: usize,
    available: usize,
    work: &mut WorkBudget,
) -> Result<Vec<usize>, CombinationError> {
    work.spend(required.len())?;
    let mut ranks = required.to_vec();
    ranks.sort_unstable();
    ranks.dedup();
    for rank in 0..available {
        if ranks.len() == size {
            break;
        }
        work.spend(ranks.len() + 1)?;
        if !ranks.contains(&rank) {
            ranks.push(rank);
        }
    }
    work.spend(ranks.len())?;
    ranks.sort_unstable();
    Ok(ranks)
}

/// Preserve a valid top-priority pool. Otherwise select the lexicographically
/// strongest rank list containing a qualifying pair, without enlarging the pool.
pub(super) fn pool(
    points: &[CombinationPoint],
    indices: &[usize],
    size: usize,
    minimum: f64,
    work: &mut WorkBudget,
) -> Result<Option<Vec<usize>>, CombinationError> {
    let top = &indices[..size];
    if point_subset(points, top, minimum, work)? {
        return Ok(Some(top.to_vec()));
    }
    if size < 2 {
        return Ok(None);
    }
    let mut best = None;
    for first in 0..indices.len() {
        for second in first + 1..indices.len() {
            if !separated(
                points[indices[first]].position,
                points[indices[second]].position,
                minimum,
                work,
            )? {
                continue;
            }
            let ranks = complete(&[first, second], size, indices.len(), work)?;
            work.spend(ranks.len())?;
            if best.as_ref().is_none_or(|previous| ranks < *previous) {
                best = Some(ranks);
            }
        }
    }
    Ok(best.map(|ranks| ranks.into_iter().map(|rank| indices[rank]).collect()))
}

/// Best minimal-size valid coverage subset containing the required point.
pub(super) fn coverage(
    positions: &[Point],
    priorities: &[f64],
    required: usize,
    minimum_size: usize,
    minimum: f64,
    work: &mut WorkBudget,
) -> Result<Vec<usize>, CombinationError> {
    let mut best: Option<(Vec<usize>, f64)> = None;
    for first in 0..positions.len() {
        for second in first + 1..positions.len() {
            if !separated(positions[first], positions[second], minimum, work)? {
                continue;
            }
            let required_count = if required == first || required == second {
                2
            } else {
                3
            };
            let size = minimum_size.max(required_count);
            let ranks = complete(&[required, first, second], size, positions.len(), work)?;
            work.spend(ranks.len())?;
            let score = ranks
                .iter()
                .map(|&rank| priorities[rank].powi(2))
                .sum::<f64>();
            if best.as_ref().is_none_or(|(previous, previous_score)| {
                ranks.len() < previous.len()
                    || (ranks.len() == previous.len()
                        && (score > *previous_score
                            || (score == *previous_score && ranks < *previous)))
            }) {
                best = Some((ranks, score));
            }
        }
    }
    best.map(|(ranks, _)| ranks)
        .ok_or(CombinationError::InsufficientSpan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y).unwrap()
    }

    #[test]
    fn one_inclusive_pair_is_enough_and_the_rule_is_disabled_at_zero() {
        let positions = [p(0.0, 0.0), p(3.0, 4.0), p(0.01, 0.0), p(0.02, 0.0)];
        assert!(satisfies_min_pair_span(&positions, 5.0, 100).unwrap());
        assert!(!satisfies_min_pair_span(&positions[..2], 5.000_001, 100).unwrap());
        assert!(!satisfies_min_pair_span(&positions[..1], 0.1, 100).unwrap());
        assert!(satisfies_min_pair_span(&positions[..1], 0.0, 1).unwrap());
        assert!(matches!(
            satisfies_min_pair_span(&positions, 100.0, 1),
            Err(CombinationError::WorkLimit)
        ));
        for minimum in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                satisfies_min_pair_span(&positions, minimum, 100),
                Err(CombinationError::InvalidConfig(_))
            ));
        }
    }

    #[test]
    fn enrollment_units_are_scale_invariant_and_derive_from_the_shorter_bound() {
        let bounds = RegionBounds {
            min: p(10.0, 20.0),
            max: p(70.0, 50.0),
        };
        let units = derive_min_pair_span_units(bounds, 2.0, 1.0 / 3.0).unwrap();
        assert_eq!(units, 5.0);
        let scaled = RegionBounds {
            min: p(100.0, 200.0),
            max: p(280.0, 290.0),
        };
        assert_eq!(
            derive_min_pair_span_units(scaled, 6.0, 1.0 / 3.0).unwrap(),
            units
        );
        for fraction in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            assert!(derive_min_pair_span_units(bounds, 2.0, fraction).is_err());
        }
        for unit in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(derive_min_pair_span_units(bounds, unit, 0.5).is_err());
        }
    }

    #[test]
    fn compact_pool_replacement_preserves_the_best_complete_rank_list() {
        let mut points: Vec<_> = [0.0, 4.0, 4.5, -1.0, 5.0]
            .into_iter()
            .enumerate()
            .map(|(id, x)| CombinationPoint {
                id,
                kind: super::super::super::FeatureKind::Ending,
                position: p(x, 0.0),
                priority: 1.0 - id as f64 * 0.1,
            })
            .collect();
        let indices = [0, 1, 2, 3, 4];
        // Anchoring rank0 with its first compatible point would choose rank4;
        // pair1/3 instead retains the lexicographically better pool0/1/3.
        assert_eq!(
            pool(&points, &indices, 3, 5.0, &mut WorkBudget::new(1000)).unwrap(),
            Some(vec![0, 1, 3])
        );
        points[1].position = p(5.0, 0.0);
        assert_eq!(
            pool(&points, &indices, 3, 5.0, &mut WorkBudget::new(1000)).unwrap(),
            Some(vec![0, 1, 2])
        );
        assert_eq!(
            pool(&points, &indices, 1, 5.0, &mut WorkBudget::new(1000)).unwrap(),
            None
        );
    }
}
