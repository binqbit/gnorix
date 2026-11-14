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

/// Keep the regional allocation intact and add one nearest valid group around
/// each source feature and pair midpoint. Feature-centered witnesses contain
/// their source feature; midpoint witnesses cover compact groups between them.
/// Every group still satisfies the original spatial policy. A neighborhood
/// lacking valid anchors expands through the lazy frontier.
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
    check_center_budget(positions.len(), work)?;
    let mut seen = BTreeSet::new();
    for subset in selected.iter() {
        work.spend(subset.ranks.len())?;
        seen.insert(subset.ranks.clone());
    }
    let feature_centers = positions
        .iter()
        .enumerate()
        .map(|(rank, &position)| (position, Some(rank)));
    let midpoints = positions.iter().enumerate().flat_map(|(first, &a)| {
        positions[first + 1..]
            .iter()
            .map(move |&b| (midpoint(a, b), None))
    });
    for (center, required_rank) in feature_centers.chain(midpoints) {
        work.spend(1)?;
        let neighbors = neighbors(center, required_rank, priorities, positions, work)?;
        if let Some(ranks) = nearest_valid(
            &neighbors,
            required_rank.is_some(),
            minimum,
            maximum,
            positions,
            policy,
            work,
        )? {
            work.spend(ranks.len())?;
            if seen.insert(ranks.clone()) {
                selected.push(enumeration::ranked_subset(priorities, ranks));
            }
        }
    }
    selected.sort_unstable_by(enumeration::compare_subsets);
    Ok(())
}

fn check_center_budget(count: usize, work: &WorkBudget) -> Result<(), CombinationError> {
    let pairs = count
        .checked_mul(count.saturating_sub(1))
        .map(|pairs| pairs / 2)
        .ok_or(CombinationError::WorkLimit)?;
    let minimum_work = pairs
        .checked_add(count)
        .and_then(|centers| count.checked_add(1)?.checked_mul(centers))
        .ok_or(CombinationError::WorkLimit)?;
    // Each lazy center visits every position, before sorting or searching.
    if minimum_work > work.remaining {
        return Err(CombinationError::WorkLimit);
    }
    Ok(())
}

fn midpoint(a: Point, b: Point) -> Point {
    Point::new(a.x() / 2.0 + b.x() / 2.0, a.y() / 2.0 + b.y() / 2.0)
        .expect("halved finite coordinates have a finite midpoint")
}

struct Neighbor {
    rank: usize,
    distance: f64,
}

fn neighbors(
    center: Point,
    required_rank: Option<usize>,
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
            distance: (point.x() - center.x()).hypot(point.y() - center.y()),
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
        (Some(first.rank) != required_rank)
            .cmp(&(Some(second.rank) != required_rank))
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
    fixed_first: bool,
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
        // Feature witnesses fix index zero. Midpoint witnesses have no required
        // member, so the same frontier also advances their first index.
        let last = current.first_nonminimal.min(current.indices.len() - 1);
        work.spend(last + 1)?;
        for position in usize::from(fixed_first)..=last {
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
