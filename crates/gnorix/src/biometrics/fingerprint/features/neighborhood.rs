//! Bounded observed crowding. This changes priority, never intrinsic strength.

use std::collections::BTreeMap;

use super::sampling::Work;
use super::{Capture, FeatureStrength, FingerprintError, Point};

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureNeighborhood {
    pub neighbor_count: usize,
    /// Sum of squared, linearly tapered weights inside the inspection radius.
    pub weighted_count: f64,
    /// Available source pixel centers divided by all centers in the full disk.
    /// Outside-frame and masked centers count as unavailable.
    pub observed_fraction: f64,
    pub factor: f64,
}

impl Default for FeatureNeighborhood {
    fn default() -> Self {
        Self {
            neighbor_count: 0,
            weighted_count: 0.0,
            observed_fraction: 0.0,
            factor: 0.5,
        }
    }
}

pub(super) fn rank(
    capture: &Capture,
    candidates: &mut [FeatureStrength],
    radius: f64,
) -> Result<(), FingerprintError> {
    let positions: Vec<_> = candidates.iter().map(|feature| feature.position).collect();
    let neighborhoods = assess(capture, &positions, radius)?;
    for (candidate, neighborhood) in candidates.iter_mut().zip(neighborhoods) {
        candidate.priority = candidate.strength.map(|score| score * neighborhood.factor);
        candidate.neighborhood = neighborhood;
    }
    Ok(())
}

fn assess(
    capture: &Capture,
    positions: &[Point],
    radius: f64,
) -> Result<Vec<FeatureNeighborhood>, FingerprintError> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err(FingerprintError::InvalidConfig(
            "feature neighborhood requires a finite positive radius",
        ));
    }
    let mut work = Work::default();
    work.charge(positions.len())?;
    let mut cells: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (id, &point) in positions.iter().enumerate() {
        cells.entry(cell(point, radius)).or_default().push(id);
    }

    let mut result = Vec::with_capacity(positions.len());
    for (id, &point) in positions.iter().enumerate() {
        let mut neighbors = FeatureNeighborhood::default();
        let (cx, cy) = cell(point, radius);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let Some(nearby) = cells.get(&(cx + dx, cy + dy)) else {
                    continue;
                };
                work.charge(nearby.len())?;
                for &other in nearby {
                    if other == id {
                        continue;
                    }
                    let distance = super::geometry::distance(point, positions[other]);
                    if distance < radius {
                        neighbors.neighbor_count += 1;
                        neighbors.weighted_count += (1.0 - distance / radius).powi(2);
                    }
                }
            }
        }
        neighbors.observed_fraction = observed_fraction(capture, point, radius, &mut work)?;
        neighbors.factor =
            0.5 + 0.5 * neighbors.observed_fraction / (1.0 + neighbors.weighted_count);
        result.push(neighbors);
    }
    Ok(result)
}

fn cell(point: Point, radius: f64) -> (i64, i64) {
    (
        (point.x() / radius).floor() as i64,
        (point.y() / radius).floor() as i64,
    )
}

fn observed_fraction(
    capture: &Capture,
    center: Point,
    radius: f64,
    work: &mut Work,
) -> Result<f64, FingerprintError> {
    let side = (2.0 * radius).ceil() + 3.0;
    work.charge((side * side) as usize)?;
    let (width, height) = (i64::from(capture.width()), i64::from(capture.height()));
    let (mut observed, mut total) = (0_usize, 0_usize);
    for y in (center.y() - radius).ceil() as i64..=(center.y() + radius).floor() as i64 {
        for x in (center.x() - radius).ceil() as i64..=(center.x() + radius).floor() as i64 {
            if (x as f64 - center.x()).hypot(y as f64 - center.y()) > radius {
                continue;
            }
            total += 1;
            if x >= 0 && y >= 0 && x < width && y < height {
                let index = (y * width + x) as usize;
                observed += usize::from(capture.valid_mask().is_none_or(|mask| mask[index]));
            }
        }
    }
    Ok(if total == 0 {
        0.0
    } else {
        observed as f64 / total as f64
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y).unwrap()
    }

    fn capture(mask: Option<Vec<bool>>) -> Capture {
        Capture::new(64, 64, vec![128; 64 * 64], mask, None).unwrap()
    }

    #[test]
    fn nearby_candidates_reduce_priority_without_promoting_isolated_evidence() {
        let image = capture(None);
        let center = point(32.0, 32.0);
        let isolated = assess(&image, &[center], 12.0).unwrap();
        assert_eq!(isolated[0].factor, 1.0);
        let near = assess(&image, &[center, point(35.0, 32.0)], 12.0).unwrap();
        let far = assess(&image, &[center, point(41.0, 32.0)], 12.0).unwrap();
        assert!(near[0].factor < far[0].factor);
        assert!(far[0].factor < 1.0);
        assert_eq!(near[0].neighbor_count, 1);
        let boundary = assess(&image, &[center, point(44.0, 32.0)], 12.0).unwrap();
        assert_eq!(boundary[0], isolated[0]);
        let crowded = assess(&image, &[center; 100], 12.0).unwrap();
        assert_eq!(crowded[0].neighbor_count, 99);
        assert!(crowded[0].factor >= 0.5);
        assert!(crowded[0].factor < near[0].factor);
    }

    #[test]
    fn hidden_or_outside_context_does_not_earn_full_isolation_credit() {
        let image = capture(None);
        let boundary = assess(&image, &[point(0.0, 32.0)], 12.0).unwrap();
        assert!(boundary[0].observed_fraction < 0.6);
        assert!(boundary[0].factor < 0.8);
        let hidden = capture(Some(vec![false; 64 * 64]));
        let empty = assess(&hidden, &[point(32.0, 32.0)], 12.0).unwrap();
        assert_eq!(empty[0].observed_fraction, 0.0);
        assert_eq!(empty[0].factor, 0.5);
    }

    #[test]
    fn crowding_depends_on_relative_scale_and_not_input_order() {
        let image = capture(None);
        let a = [point(16.0, 16.0), point(18.0, 17.0), point(24.0, 16.0)];
        let b = a.map(|p| point(p.x() * 2.0, p.y() * 2.0));
        let small = assess(&image, &a, 6.0).unwrap();
        let large = assess(&image, &b, 12.0).unwrap();
        assert_eq!(small, large);
        let reverse = assess(&image, &[a[2], a[1], a[0]], 6.0).unwrap();
        assert_eq!(small, reverse.into_iter().rev().collect::<Vec<_>>());
    }

    #[test]
    fn oversized_neighborhood_fails_before_raster_iteration() {
        assert!(matches!(
            assess(&capture(None), &[point(32.0, 32.0)], 1e20),
            Err(FingerprintError::WorkLimit)
        ));
    }
}
