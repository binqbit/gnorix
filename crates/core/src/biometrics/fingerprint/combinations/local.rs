//! Compact enrollment witnesses independent of distant feature priorities.

use std::{
    cmp::Ordering,
    collections::{BTreeSet, BinaryHeap},
};

use super::{
    CombinationError, Point, WorkBudget,
    enumeration::{self, RankedSubset},
    spatial,
};

/// Keep the regional allocation intact and add at most one nearest valid group
/// containing each source feature. Every group still satisfies the original
/// spatial policy. A small neighborhood lacking valid anchors is expanded by
/// the lazy frontier rather than treated as an uncovered regional feature.
pub(super) fn supplement(
    priorities: &[f64],
    positions: &[Point],
    minimum: usize,
    maximum: usize,
    policy: spatial::SpatialPixels,
    selected: &mut Vec<RankedSubset>,
    work: &mut WorkBudget,
) -> Result<(), CombinationError> {
    let minimum = minimum.max(policy.anchors);
    let mut seen = BTreeSet::new();
    for subset in selected.iter() {
        work.spend(subset.ranks.len())?;
        seen.insert(subset.ranks.clone());
    }
    for center in 0..positions.len() {
        let neighbors = neighbors(center, priorities, positions, work)?;
        if let Some(ranks) = nearest_valid(&neighbors, minimum, maximum, positions, policy, work)? {
            work.spend(ranks.len())?;
            if seen.insert(ranks.clone()) {
                selected.push(enumeration::ranked_subset(priorities, ranks));
            }
        }
    }
    selected.sort_unstable_by(enumeration::compare_subsets);
    Ok(())
}

struct Neighbor {
    rank: usize,
    distance: f64,
}

fn neighbors(
    center: usize,
    priorities: &[f64],
    positions: &[Point],
    work: &mut WorkBudget,
) -> Result<Vec<Neighbor>, CombinationError> {
    work.spend(positions.len())?;
    let mut neighbors = positions
        .iter()
        .enumerate()
        .map(|(rank, point)| Neighbor {
            rank,
            distance: (point.x() - positions[center].x()).hypot(point.y() - positions[center].y()),
        })
        .collect::<Vec<_>>();
    if neighbors
        .iter()
        .any(|neighbor| !neighbor.distance.is_finite())
    {
        return Err(CombinationError::InvalidFingerprint(
            super::FingerprintError::InvalidMap("local point distance is not finite"),
        ));
    }
    let mut comparisons = 0_usize;
    neighbors.sort_unstable_by(|first, second| {
        comparisons = comparisons.saturating_add(1);
        (first.rank != center)
            .cmp(&(second.rank != center))
            .then_with(|| first.distance.total_cmp(&second.distance))
            .then_with(|| priorities[second.rank].total_cmp(&priorities[first.rank]))
            // Regional ranks already use priority and ascending source IDs.
            .then(first.rank.cmp(&second.rank))
    });
    work.spend(comparisons)?;
    Ok(neighbors)
}

fn nearest_valid(
    neighbors: &[Neighbor],
    minimum: usize,
    maximum: usize,
    positions: &[Point],
    policy: spatial::SpatialPixels,
    work: &mut WorkBudget,
) -> Result<Option<Vec<usize>>, CombinationError> {
    let mut pending = BinaryHeap::new();
    for size in minimum..=maximum {
        work.spend(size)?;
        pending.push(Pending::new((0..size).collect(), size, neighbors));
    }
    while let Some(current) = pending.pop() {
        work.spend(current.indices.len())?;
        let mut ranks = current
            .indices
            .iter()
            .map(|&index| neighbors[index].rank)
            .collect::<Vec<_>>();
        ranks.sort_unstable();
        if spatial::subset(positions, &ranks, policy, work)? {
            return Ok(Some(ranks));
        }

        // As with the ranked regional frontier, decrementing the first
        // nonminimal mutable index identifies each tuple's unique parent.
        // Index zero is fixed: every witness must contain its own center.
        let last = current.first_nonminimal.min(current.indices.len() - 1);
        work.spend(last + 1)?;
        for position in 1..=last {
            let limit = current
                .indices
                .get(position + 1)
                .copied()
                .unwrap_or(neighbors.len());
            if current.indices[position] + 1 >= limit {
                continue;
            }
            work.spend(current.indices.len())?;
            let mut child = current.indices.clone();
            child[position] += 1;
            pending.push(Pending::new(child, position, neighbors));
        }
    }
    Ok(None)
}

struct Pending {
    indices: Vec<usize>,
    radius: f64,
    total_distance: f64,
    first_nonminimal: usize,
}

impl Pending {
    fn new(indices: Vec<usize>, first_nonminimal: usize, neighbors: &[Neighbor]) -> Self {
        Self {
            radius: neighbors[*indices.last().expect("local group is nonempty")].distance,
            total_distance: indices.iter().map(|&index| neighbors[index].distance).sum(),
            indices,
            first_nonminimal,
        }
    }
}

impl PartialEq for Pending {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Pending {}
impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Pending {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .radius
            .total_cmp(&self.radius)
            .then_with(|| other.total_distance.total_cmp(&self.total_distance))
            .then_with(|| other.indices.cmp(&self.indices))
    }
}

#[cfg(test)]
mod tests;
