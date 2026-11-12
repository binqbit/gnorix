//! Confirmed-pattern bounds and priority-ranked circular pools.

use super::{
    CombinationConfig, CombinationError, CombinationPoint, CombinationRegion, Fingerprint,
    FingerprintError, Point, RegionBounds, WorkBudget,
};

pub(super) fn pattern_bounds(
    fingerprint: &Fingerprint,
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
    support_bounds(support, map.width(), work)
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

pub(super) fn select(
    points: &[CombinationPoint],
    bounds: RegionBounds,
    config: &CombinationConfig,
    work: &mut WorkBudget,
) -> Result<Vec<CombinationRegion>, CombinationError> {
    let width = bounds.max.x() - bounds.min.x();
    let height = bounds.max.y() - bounds.min.y();
    let initial_radius = config.diameter_fraction * width.min(height) / 2.0;
    if !initial_radius.is_finite() || initial_radius <= 0.0 {
        return Err(CombinationError::InvalidConfig(
            "circle radius is not finite and positive",
        ));
    }
    let target = (config.min_points * config.pool_multiplier).min(points.len());
    (0..9)
        .map(|id| {
            let center = Point::new(
                bounds.min.x() + width * (f64::from(id % 3) + 0.5) / 3.0,
                bounds.min.y() + height * (f64::from(id / 3) + 0.5) / 3.0,
            )
            .expect("finite region center");
            select_pool(points, id, center, initial_radius, target, work)
        })
        .collect()
}

fn select_pool(
    points: &[CombinationPoint],
    id: u8,
    center: Point,
    initial_radius: f64,
    target: usize,
    work: &mut WorkBudget,
) -> Result<CombinationRegion, CombinationError> {
    work.spend(points.len())?;
    let mut distances: Vec<_> = points
        .iter()
        .map(|point| {
            (
                (point.position.x() - center.x()).hypot(point.position.y() - center.y()),
                point,
            )
        })
        .collect();
    distances.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
    let radius = initial_radius.max(distances[target - 1].0);
    let mut inside: Vec<_> = distances
        .iter()
        .take_while(|(distance, _)| *distance <= radius)
        .map(|(_, point)| *point)
        .collect();
    let available_points = inside.len();
    inside.sort_by(|a, b| b.priority.total_cmp(&a.priority).then(a.id.cmp(&b.id)));
    let pool = inside.iter().take(target).map(|point| point.id).collect();
    Ok(CombinationRegion {
        id,
        center,
        initial_radius,
        radius,
        available_points,
        pool,
        combination_count: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biometrics::fingerprint::FeatureKind;

    fn point(id: usize, x: f64, priority: f64) -> CombinationPoint {
        CombinationPoint {
            id,
            kind: FeatureKind::Ending,
            position: Point::new(x, 0.0).unwrap(),
            priority,
        }
    }

    #[test]
    fn grows_to_target_and_ranks_all_points_on_inclusive_boundary() {
        let points = vec![point(5, -3.0, 0.9), point(3, 1.0, 0.0), point(2, 3.0, 0.8)];
        let pool = select_pool(
            &points,
            0,
            Point::new(0.0, 0.0).unwrap(),
            1.0,
            2,
            &mut WorkBudget::new(100),
        )
        .unwrap();
        assert_eq!(pool.radius, 3.0);
        assert_eq!(pool.available_points, 3);
        assert_eq!(pool.pool, vec![5, 2]);
    }

    #[test]
    fn retains_initial_radius_and_uses_ids_to_break_equal_priority_ties() {
        let points = vec![point(8, 1.0, 0.2), point(2, 2.0, 0.2), point(3, 4.0, 1.0)];
        let pool = select_pool(
            &points,
            0,
            Point::new(0.0, 0.0).unwrap(),
            5.0,
            2,
            &mut WorkBudget::new(100),
        )
        .unwrap();
        assert_eq!(pool.radius, 5.0);
        assert_eq!(pool.pool, vec![3, 2]);
    }

    #[test]
    fn bounds_use_confirmed_pixel_edges_and_centers_visit_nine_cells() {
        let mut mask = vec![false; 100];
        mask[22] = true;
        mask[77] = true;
        let bounds = support_bounds(&mask, 10, &mut WorkBudget::new(100)).unwrap();
        assert_eq!(bounds.min, Point::new(1.5, 1.5).unwrap());
        assert_eq!(bounds.max, Point::new(7.5, 7.5).unwrap());
        let config = CombinationConfig {
            min_points: 1,
            pool_multiplier: 1,
            diameter_fraction: 0.5,
            combinations_per_region: 1,
            max_work: 100,
        };
        let result = select(
            &[point(0, 4.0, 1.0)],
            bounds,
            &config,
            &mut WorkBudget::new(100),
        )
        .unwrap();
        assert_eq!(result.len(), 9);
        assert_eq!(result[0].center, Point::new(2.5, 2.5).unwrap());
        assert_eq!(result[4].center, Point::new(4.5, 4.5).unwrap());
        assert_eq!(result[8].center, Point::new(6.5, 6.5).unwrap());
        assert!(result.iter().all(|r| r.initial_radius == 1.5));
    }

    #[test]
    fn missing_pattern_and_exhausted_work_are_explicit() {
        assert_eq!(
            support_bounds(&[false; 4], 2, &mut WorkBudget::new(4)),
            Err(CombinationError::PatternUnavailable)
        );
        assert_eq!(
            support_bounds(&[true; 4], 2, &mut WorkBudget::new(3)),
            Err(CombinationError::WorkLimit)
        );
    }
}
