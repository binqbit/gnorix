//! Shared adaptive circles, followed by bounded enrollment pool selection.

mod layout;

#[cfg(test)]
mod tests;

use super::{
    CombinationConfig, CombinationError, CombinationPoint, CombinationRegion, Fingerprint,
    FingerprintError, Point, RegionBounds, WorkBudget, enumeration, spatial,
};

pub(super) struct RegionCandidates {
    pub id: usize,
    pub center: Point,
    pub initial_radius: f64,
    pub radius: f64,
    /// All in-circle indices into the source point slice, ranked by priority/ID.
    pub indices: Vec<usize>,
}

pub(super) fn pattern_bounds(
    fingerprint: &Fingerprint,
    points: &[CombinationPoint],
    work: &mut WorkBudget,
) -> Result<RegionBounds, CombinationError> {
    let map = &fingerprint.scale.skeleton;
    let support = &fingerprint.scale.pattern_support;
    if map.width().checked_mul(map.height()) != Some(support.len())
        || map.width() != fingerprint.binary.width()
        || map.height() != fingerprint.binary.height()
    {
        return Err(CombinationError::InvalidFingerprint(
            FingerprintError::InvalidMap(
                "pattern support dimensions differ from measurement skeleton",
            ),
        ));
    }
    let bounds = support_bounds(support, map.width(), work)?;
    cover_points(bounds, points, work)
}

fn support_bounds(
    support: &[bool],
    width: usize,
    work: &mut WorkBudget,
) -> Result<RegionBounds, CombinationError> {
    work.spend(support.len())?;
    let mut extent: Option<(usize, usize, usize, usize)> = None;
    for (index, &supported) in support.iter().enumerate() {
        if !supported {
            continue;
        }
        let (x, y) = (index % width, index / width);
        extent = Some(match extent {
            None => (x, y, x, y),
            Some((left, top, right, bottom)) => {
                (left.min(x), top.min(y), right.max(x), bottom.max(y))
            }
        });
    }
    let (left, top, right, bottom) = extent.ok_or(CombinationError::PatternUnavailable)?;
    Ok(RegionBounds {
        min: Point::new(left as f64 - 0.5, top as f64 - 0.5).expect("finite pixel bounds"),
        max: Point::new(right as f64 + 0.5, bottom as f64 + 0.5).expect("finite pixel bounds"),
    })
}

fn cover_points(
    bounds: RegionBounds,
    points: &[CombinationPoint],
    work: &mut WorkBudget,
) -> Result<RegionBounds, CombinationError> {
    work.spend(points.len())?;
    let mut min = [bounds.min.x(), bounds.min.y()];
    let mut max = [bounds.max.x(), bounds.max.y()];
    for point in points {
        min[0] = min[0].min(point.position.x());
        min[1] = min[1].min(point.position.y());
        max[0] = max[0].max(point.position.x());
        max[1] = max[1].max(point.position.y());
    }
    Ok(RegionBounds {
        min: Point::new(min[0], min[1]).expect("finite source bounds"),
        max: Point::new(max[0], max[1]).expect("finite source bounds"),
    })
}

pub(super) fn select(
    points: &[CombinationPoint],
    bounds: RegionBounds,
    unit_px: f64,
    config: &CombinationConfig,
    spatial: spatial::SpatialPixels,
    work: &mut WorkBudget,
) -> Result<Vec<CombinationRegion>, CombinationError> {
    let target = config
        .min_points
        .checked_mul(config.pool_multiplier)
        .ok_or(CombinationError::InvalidConfig("point pool size overflows"))?
        .min(points.len());
    circular_regions(points, bounds, unit_px, config.radius_units, target, work)?
        .into_iter()
        .map(|region| {
            let pool = select_pool(points, &region, config, target, spatial, work)?;
            Ok(CombinationRegion {
                id: region.id,
                center: region.center,
                initial_radius: region.initial_radius,
                radius: region.radius,
                available_points: region.indices.len(),
                pool: pool.into_iter().map(|index| points[index].id).collect(),
                combination_count: 0,
            })
        })
        .collect()
}

fn pool_data(
    points: &[CombinationPoint],
    indices: &[usize],
    work: &mut WorkBudget,
) -> Result<(Vec<f64>, Vec<Point>), CombinationError> {
    work.spend(indices.len())?;
    work.spend(indices.len())?;
    Ok((
        indices
            .iter()
            .map(|&index| points[index].priority)
            .collect(),
        indices
            .iter()
            .map(|&index| points[index].position)
            .collect(),
    ))
}

/// Never discard an uncovered selected point. Rescue is allowed only when the
/// original top-priority pool has no valid subset at all.
fn select_pool(
    points: &[CombinationPoint],
    region: &RegionCandidates,
    config: &CombinationConfig,
    target: usize,
    spatial: spatial::SpatialPixels,
    work: &mut WorkBudget,
) -> Result<Vec<usize>, CombinationError> {
    let top = &region.indices[..target];
    if spatial.unconstrained() {
        work.spend(target)?;
        return Ok(top.to_vec());
    }
    let failure = |index: usize| CombinationError::UncoverablePoint {
        region: region.id,
        feature_id: points[index].id,
    };
    if spatial.anchors > target {
        return Err(failure(top[0]));
    }
    let (priorities, positions) = pool_data(points, top, work)?;
    let coverage =
        enumeration::coverage_spatial(&priorities, &positions, config.min_points, spatial, work)?;
    if coverage.uncovered.is_empty() {
        work.spend(target)?;
        return Ok(top.to_vec());
    }
    if !coverage.subsets.is_empty() {
        return Err(failure(top[coverage.uncovered[0]]));
    }
    let (priorities, positions) = pool_data(points, &region.indices, work)?;
    let mut ranked = enumeration::RankedSubsets::new(
        &priorities,
        config.min_points.max(spatial.anchors),
        target,
        work,
    )?;
    while let Some(seed) = ranked.next(&priorities, work)? {
        if !spatial::subset(&positions, &seed.ranks, spatial, work)? {
            continue;
        }
        work.spend(region.indices.len())?;
        let mut ranks = seed.ranks;
        for rank in 0..region.indices.len() {
            if ranks.len() == target {
                break;
            }
            work.spend(ranks.len() + 1)?;
            if !ranks.contains(&rank) {
                ranks.push(rank);
            }
        }
        ranks.sort_unstable();
        let pool: Vec<_> = ranks.into_iter().map(|rank| region.indices[rank]).collect();
        let (priorities, positions) = pool_data(points, &pool, work)?;
        let coverage = enumeration::coverage_spatial(
            &priorities,
            &positions,
            config.min_points,
            spatial,
            work,
        )?;
        if let Some(&uncovered) = coverage.uncovered.first() {
            return Err(failure(pool[uncovered]));
        }
        return Ok(pool);
    }
    Err(failure(top[0]))
}

/// Keep every in-circle point for signing. Enrollment alone truncates this list
/// to its target pool. Sparse circles grow just enough to reach the shared target.
pub(super) fn circular_regions(
    points: &[CombinationPoint],
    bounds: RegionBounds,
    unit_px: f64,
    radius_units: f64,
    target: usize,
    work: &mut WorkBudget,
) -> Result<Vec<RegionCandidates>, CombinationError> {
    let radius = measured_radius(unit_px, radius_units)?;
    if target == 0 || points.is_empty() {
        return Err(CombinationError::InvalidConfig(
            "circle point target must be nonzero",
        ));
    }
    let target = target.min(points.len());
    let width = bounds.max.x() - bounds.min.x();
    let height = bounds.max.y() - bounds.min.y();
    let layout = layout::covering_grid(width, height, radius, work)?;
    // Every circle must visit the source points and retain at least its target.
    // Reject an impossible budget before reserving the whole region vector.
    let minimum_work = points
        .len()
        .checked_add(target)
        .and_then(|count| count.checked_add(1))
        .and_then(|count| count.checked_mul(layout.count()))
        .ok_or(CombinationError::WorkLimit)?;
    if minimum_work > work.remaining {
        return Err(CombinationError::WorkLimit);
    }
    work.spend(layout.count())?;
    let mut regions = Vec::with_capacity(layout.count());
    for id in 0..layout.count() {
        let center = Point::new(
            bounds.min.x() + width * ((id % layout.columns) as f64 + 0.5) / layout.columns as f64,
            bounds.min.y() + height * ((id / layout.columns) as f64 + 0.5) / layout.rows as f64,
        )
        .map_err(|_| {
            CombinationError::InvalidFingerprint(FingerprintError::InvalidMap(
                "region center is not finite",
            ))
        })?;
        regions.push(circle_candidates(points, id, center, radius, target, work)?);
    }
    Ok(regions)
}

fn measured_radius(unit_px: f64, radius_units: f64) -> Result<f64, CombinationError> {
    if !unit_px.is_finite() || unit_px <= 0.0 {
        return Err(CombinationError::InvalidFingerprint(
            FingerprintError::ScaleUnavailable("ridge width must be finite and positive"),
        ));
    }
    let radius = radius_units * unit_px;
    if !radius_units.is_finite() || radius_units <= 0.0 || !radius.is_finite() || radius <= 0.0 {
        return Err(CombinationError::InvalidConfig(
            "circle radius must be finite and positive",
        ));
    }
    Ok(radius)
}

fn circle_candidates(
    points: &[CombinationPoint],
    id: usize,
    center: Point,
    initial_radius: f64,
    target: usize,
    work: &mut WorkBudget,
) -> Result<RegionCandidates, CombinationError> {
    work.spend(points.len())?;
    let mut distances: Vec<_> = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            (
                (point.position.x() - center.x()).hypot(point.position.y() - center.y()),
                index,
            )
        })
        .collect();
    distances.sort_unstable_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(points[a.1].id.cmp(&points[b.1].id))
    });
    let radius = initial_radius.max(distances[target - 1].0);
    if !radius.is_finite() {
        return Err(CombinationError::InvalidFingerprint(
            FingerprintError::InvalidMap("point distance from circle center is not finite"),
        ));
    }
    let inside = distances.partition_point(|(distance, _)| *distance <= radius);
    work.spend(inside)?;
    let mut indices = distances
        .into_iter()
        .take(inside)
        .map(|(_, index)| index)
        .collect::<Vec<_>>();
    indices.sort_unstable_by(|&a, &b| {
        points[b]
            .priority
            .total_cmp(&points[a].priority)
            .then(points[a].id.cmp(&points[b].id))
    });
    Ok(RegionCandidates {
        id,
        center,
        initial_radius,
        radius,
        indices,
    })
}
