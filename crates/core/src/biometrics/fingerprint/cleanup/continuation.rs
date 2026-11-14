//! Guard a forward light cap against source-connected bends around the terminal arc.

mod field;

use field::Field;

use super::super::photometry::sample_darkness;
use super::{
    Capture, FingerprintError, LocalPalette, Point, RidgeGraph, SourceDecision, Work, footprint,
    source,
};

pub(super) struct Backbone<'a> {
    paths: Vec<BackbonePath<'a>>,
    known_ends: Vec<KnownEnd<'a>>,
}

struct KnownEnd<'a> {
    points: &'a [Point],
    component: usize,
}

struct BackbonePath<'a> {
    points: &'a [Point],
    component: usize,
    bounds: [f64; 4],
    positions: Vec<f64>,
}

struct PathSelection {
    path: usize,
    stations: Vec<Station>,
}

struct Station {
    index: usize,
    vertex: bool,
    segment: bool,
}

impl<'a> Backbone<'a> {
    pub(super) fn new(
        graph: &'a RidgeGraph,
        excluded: &[bool],
        components: &[usize],
        confirmed: &[usize],
    ) -> Self {
        let paths = graph
            .edges
            .iter()
            .enumerate()
            .filter(|(id, _)| !excluded[*id])
            .map(|(_, edge)| (&edge.points[..], components[edge.start]));
        let mut backbone = Self::from_paths(paths);
        backbone.known_ends = confirmed
            .iter()
            .map(|&id| KnownEnd {
                points: &graph.edges[id].points,
                component: components[graph.edges[id].start],
            })
            .collect();
        backbone
    }

    pub(super) fn from_paths(paths: impl Iterator<Item = (&'a [Point], usize)>) -> Self {
        let paths = paths
            .map(|(points, component)| {
                let mut bounds = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for point in points {
                    bounds[0] = bounds[0].min(point.x());
                    bounds[1] = bounds[1].min(point.y());
                    bounds[2] = bounds[2].max(point.x());
                    bounds[3] = bounds[3].max(point.y());
                }
                let mut positions = vec![0.0];
                for pair in points.windows(2) {
                    positions.push(
                        positions.last().copied().unwrap() + super::distance(pair[0], pair[1]),
                    );
                }
                BackbonePath {
                    points,
                    component,
                    bounds,
                    positions,
                }
            })
            .collect();
        Self {
            paths,
            known_ends: Vec::new(),
        }
    }
}

pub(super) fn check(
    capture: &Capture,
    palette: &LocalPalette,
    points: &[Point],
    unit: f64,
    backbone: &Backbone<'_>,
    component: usize,
    work: &mut Work,
) -> Result<SourceDecision, FingerprintError> {
    let (tip, tangent) = match last_dark_frame(capture, palette, points, unit, work)? {
        Anchor::Observed(tip, tangent) => (tip, tangent),
        Anchor::Unresolved(decision) => return Ok(decision),
    };
    let Some(field) = Field::read(capture, palette, tip, tangent, points, unit, work)? else {
        return Ok(SourceDecision::MissingSupport);
    };
    let Some(escapes) = field.escapes(capture, palette, work)? else {
        return Ok(SourceDecision::MissingSupport);
    };
    let known = field.contained_ends(backbone, component, work)?;
    let mut candidates = None;
    for escape in escapes {
        let point = escape.point;
        work.charge(known.iter().map(|points| points.len()).sum::<usize>() as f64)?;
        // Exact, independently cap-confirmed geometry is known finite support,
        // not a ridge-width or extension inference. The flood already traversed
        // through it: source continuation outside these points still needs proof.
        if !escape.boundary && known.iter().any(|points| points.contains(&point)) {
            continue;
        }
        if candidates.is_none() {
            candidates = Some(backbone.local_candidates(&field, component, work)?);
        }
        match backbone.explains(
            capture,
            palette,
            point,
            &field,
            candidates.as_deref().expect("prepared backbone candidates"),
            None,
            work,
        )? {
            SourceDecision::Confirmed => {}
            SourceDecision::MissingSupport => return Ok(SourceDecision::MissingSupport),
            _ => return Ok(SourceDecision::PossibleContinuation),
        }
    }
    Ok(SourceDecision::Confirmed)
}

struct Escape {
    point: Point,
    boundary: bool,
}

/// Graded contradiction for diagnostic feature ranking. Cleanup still uses
/// `check`, including its original categorical source predicates.
#[allow(clippy::too_many_arguments)]
pub(super) fn score(
    capture: &Capture,
    palette: &LocalPalette,
    points: &[Point],
    unit: f64,
    backbone: &Backbone<'_>,
    component: usize,
    reference_support: f64,
    explain: &mut impl FnMut(Point) -> Result<Option<f64>, FingerprintError>,
    work: &mut Work,
) -> Result<Option<f64>, FingerprintError> {
    if !reference_support.is_finite() || reference_support <= 0.0 {
        return Ok(None);
    }
    let (tip, tangent) = match last_dark_frame(capture, palette, points, unit, work)? {
        Anchor::Observed(tip, tangent) => (tip, tangent),
        Anchor::Unresolved(SourceDecision::ImageUnsupported) => return Ok(Some(0.0)),
        Anchor::Unresolved(_) => return Ok(None),
    };
    let Some(field) = Field::read(capture, palette, tip, tangent, points, unit, work)? else {
        return Ok(None);
    };
    let mut cache = footprint::Cache::new(capture, palette);
    let Some(mut escapes) = field.weighted_escapes(points, &mut cache, work)? else {
        return Ok(None);
    };
    work.charge(escapes.len() as f64 * (escapes.len().max(1).ilog2() as f64 + 1.0))?;
    escapes.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut strongest = 0.0_f64;
    let mut proofs = None;
    for (escape, connected) in escapes {
        // A consistently faint ridge is still a strong continuation relative
        // to its own observed incoming signal. A weak leak from a strong ridge
        // retains its small connected support; no fixed score floor is used.
        let connected = (connected / reference_support).clamp(0.0, 1.0);
        if connected <= strongest {
            continue;
        }
        let incoming = explain(escape.point)?;
        if incoming.is_some_and(|value| connected * (1.0 - value) <= strongest) {
            // The independently observed incoming ribbon already bounds this
            // contradiction. Further backbone evidence can only reduce it.
            continue;
        }
        if proofs.is_none() {
            proofs = Some(backbone.local_candidates(&field, component, work)?);
        }
        let candidates = proofs.as_deref().expect("prepared source proof region");
        let explanation = match backbone.explains(
            capture,
            palette,
            escape.point,
            &field,
            candidates,
            Some(&mut cache),
            work,
        )? {
            SourceDecision::Confirmed => 1.0,
            SourceDecision::MissingSupport => return Ok(None),
            _ => match incoming {
                Some(value) => value,
                None => return Ok(None),
            },
        };
        strongest = strongest.max(connected * (1.0 - explanation));
    }
    Ok(Some(strongest))
}

struct Axis {
    origin: Point,
    center: Point,
    tangent: [f64; 2],
    distance: f64,
}

impl Backbone<'_> {
    fn local_candidates(
        &self,
        field: &Field,
        component: usize,
        work: &mut Work,
    ) -> Result<Vec<PathSelection>, FingerprintError> {
        let maximum = source::SourcePolicy::for_unit(field.unit).max_backbone_extension_px;
        let bounds = expanded_bounds(field.source_bounds(), maximum);
        let mut selected = Vec::new();
        work.charge(self.paths.len() as f64)?;
        for (index, path) in self.paths.iter().enumerate() {
            if path.component != component || !bounds_overlap(path.bounds, bounds) {
                continue;
            }
            let stations = path.local_stations(bounds, work)?;
            if !stations.is_empty() {
                selected.push(PathSelection {
                    path: index,
                    stations,
                });
            }
        }
        Ok(selected)
    }

    #[allow(clippy::too_many_arguments)]
    fn explains(
        &self,
        capture: &Capture,
        palette: &LocalPalette,
        point: Point,
        field: &Field,
        candidates: &[PathSelection],
        mut cache: Option<&mut footprint::Cache<'_>>,
        work: &mut Work,
    ) -> Result<SourceDecision, FingerprintError> {
        let unit = field.unit;
        let maximum = source::SourcePolicy::for_unit(unit).max_backbone_extension_px;
        let mut axes = Vec::new();
        work.charge(candidates.len() as f64)?;
        for candidate in candidates {
            let path = &self.paths[candidate.path];
            // A normal can reach R from the locally continued axis. Its origin
            // can be another R away at an observed endpoint, never farther.
            if point.x() < path.bounds[0] - maximum
                || point.y() < path.bounds[1] - maximum
                || point.x() > path.bounds[2] + maximum
                || point.y() > path.bounds[3] + maximum
            {
                continue;
            }
            path.append_axes(&candidate.stations, point, field.tip, unit, &mut axes, work)?;
        }
        axes.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        let mut missing = false;
        for axis in axes {
            let prefix = match cache.as_deref_mut() {
                Some(cache) => cache.dark_segment(axis.origin, axis.center, work)?,
                None => footprint::dark_segment(capture, palette, axis.origin, axis.center, work)?,
            };
            match prefix {
                SourceDecision::Confirmed => {}
                SourceDecision::MissingSupport => {
                    missing = true;
                    continue;
                }
                _ => continue,
            }
            // An exact point on the retained path is already observed backbone,
            // not an inferred transverse footprint. The source-dark prefix
            // above still validates it and every capture/palette corner.
            if point == axis.origin {
                return Ok(SourceDecision::Confirmed);
            }
            let span = match cache.as_deref_mut() {
                Some(cache) => cache.normal_span(axis.center, axis.tangent, point, unit, work)?,
                None => footprint::normal_span(
                    capture,
                    palette,
                    axis.center,
                    axis.tangent,
                    point,
                    unit,
                    work,
                )?,
            };
            match span {
                SourceDecision::Confirmed => return Ok(SourceDecision::Confirmed),
                SourceDecision::MissingSupport => missing = true,
                _ => {}
            }
        }
        Ok(if missing {
            SourceDecision::MissingSupport
        } else {
            SourceDecision::ImageUnsupported
        })
    }
}

impl BackbonePath<'_> {
    fn local_stations(
        &self,
        bounds: [f64; 4],
        work: &mut Work,
    ) -> Result<Vec<Station>, FingerprintError> {
        // Preserve original indices: clipping or rebuilding the path would
        // change its R/2 tangent and could invent an extensible endpoint.
        work.charge(self.points.len() as f64 * 4.0)?;
        let mut stations = Vec::new();
        for (index, &point) in self.points.iter().enumerate() {
            let vertex = point_in_bounds(point, bounds);
            let segment = self.points.get(index + 1).is_some_and(|&next| {
                bounds_overlap(
                    [
                        point.x().min(next.x()),
                        point.y().min(next.y()),
                        point.x().max(next.x()),
                        point.y().max(next.y()),
                    ],
                    bounds,
                )
            });
            if vertex || segment {
                stations.push(Station {
                    index,
                    vertex,
                    segment,
                });
            }
        }
        Ok(stations)
    }

    fn append_axes(
        &self,
        stations: &[Station],
        point: Point,
        anchor: Point,
        unit: f64,
        axes: &mut Vec<Axis>,
        work: &mut Work,
    ) -> Result<(), FingerprintError> {
        let maximum = source::SourcePolicy::for_unit(unit).max_backbone_extension_px;
        // Field candidates were selected once. Keep the exact per-query tests;
        // the conservative index bounds never admit additional evidence axes.
        work.charge(stations.len() as f64 * 4.0)?;
        // Segment boundaries remain valid stations even when the nearest point
        // lies inside an adjacent segment with a different scale-aware tangent.
        // Visit each boundary once, then add only strictly interior projections.
        for station in stations {
            let index = station.index;
            if station.vertex && super::distance(self.points[index], point) <= maximum {
                work.charge(16.0)?;
                if let Some(axis) = self.vertex_axis(index, point, anchor, unit) {
                    axes.push(axis);
                }
            }
            if station.segment
                && segment_near(self.points[index], self.points[index + 1], point, maximum)
            {
                work.charge(8.0)?;
                if let Some(axis) = self.segment_axis(index, point, unit, work)? {
                    axes.push(axis);
                }
            }
        }
        Ok(())
    }

    fn vertex_axis(&self, index: usize, point: Point, anchor: Point, unit: f64) -> Option<Axis> {
        let origin = self.points[index];
        let position = self.positions[index];
        let maximum = source::SourcePolicy::for_unit(unit).max_backbone_extension_px;
        let extension = (unit + super::distance(anchor, origin)).min(maximum);
        let low = if index == 0 {
            -extension
        } else {
            self.positions[index - 1] - position
        };
        let high = if index + 1 == self.points.len() {
            extension
        } else {
            self.positions[index + 1] - position
        };
        self.station_axis(origin, position, [low, high], point, unit)
    }

    fn segment_axis(
        &self,
        index: usize,
        point: Point,
        unit: f64,
        work: &mut Work,
    ) -> Result<Option<Axis>, FingerprintError> {
        let first = self.points[index];
        let last = self.points[index + 1];
        let span = self.positions[index + 1] - self.positions[index];
        if span <= 0.0 {
            return Ok(None);
        }
        let direction = [(last.x() - first.x()) / span, (last.y() - first.y()) / span];
        let offset = ((point.x() - first.x()) * direction[0]
            + (point.y() - first.y()) * direction[1])
            .clamp(0.0, span);
        if offset <= 0.0 || offset >= span {
            return Ok(None);
        }
        let Ok(origin) = Point::new(
            first.x() + offset * direction[0],
            first.y() + offset * direction[1],
        ) else {
            return Ok(None);
        };
        let maximum = source::SourcePolicy::for_unit(unit).max_backbone_extension_px;
        if super::distance(origin, point) > maximum {
            return Ok(None);
        }
        work.charge(16.0)?;
        Ok(self.station_axis(
            origin,
            self.positions[index] + offset,
            [-offset, span - offset],
            point,
            unit,
        ))
    }

    fn station_axis(
        &self,
        origin: Point,
        position: f64,
        bounds: [f64; 2],
        point: Point,
        unit: f64,
    ) -> Option<Axis> {
        let maximum = source::SourcePolicy::for_unit(unit).max_backbone_extension_px;
        if super::distance(origin, point) > maximum {
            return None;
        }
        let length = *self.positions.last()?;
        let before = self.at((position - unit / 2.0).max(0.0))?;
        let after = self.at((position + unit / 2.0).min(length))?;
        let norm = super::distance(before, after);
        if norm <= 0.0 {
            return None;
        }
        let tangent = [
            (after.x() - before.x()) / norm,
            (after.y() - before.y()) / norm,
        ];
        let delta = [point.x() - origin.x(), point.y() - origin.y()];
        let along = delta[0] * tangent[0] + delta[1] * tangent[1];
        // Cover complete adjacent segments instead of disjoint half-arc slabs.
        // Only actual path endpoints may extend beyond the observed path, and
        // every displacement from the station still requires source proof.
        if along < bounds[0] || along > bounds[1] {
            return None;
        }
        let center = Point::new(
            origin.x() + along * tangent[0],
            origin.y() + along * tangent[1],
        )
        .ok()?;
        let distance = super::distance(center, point);
        (distance <= source::SourcePolicy::for_unit(unit).max_ridge_width_px).then_some(Axis {
            origin,
            center,
            tangent,
            distance,
        })
    }

    fn at(&self, target: f64) -> Option<Point> {
        let index = self.positions.partition_point(|&value| value < target);
        if index == 0 {
            return self.points.first().copied();
        }
        if index == self.positions.len() {
            return self.points.last().copied();
        }
        let one = self.points[index - 1];
        let two = self.points[index];
        let ratio = (target - self.positions[index - 1])
            / (self.positions[index] - self.positions[index - 1]);
        Point::new(
            one.x() + ratio * (two.x() - one.x()),
            one.y() + ratio * (two.y() - one.y()),
        )
        .ok()
    }
}

fn expanded_bounds(bounds: [f64; 4], maximum: f64) -> [f64; 4] {
    // Only the candidate index is padded for floating-point roundoff. Every
    // query still applies its original exact distance and projection bounds.
    let scale = bounds
        .into_iter()
        .map(f64::abs)
        .fold(maximum.max(1.0), f64::max);
    let extent = maximum + 32.0 * f64::EPSILON * scale;
    [
        bounds[0] - extent,
        bounds[1] - extent,
        bounds[2] + extent,
        bounds[3] + extent,
    ]
}

fn bounds_overlap(first: [f64; 4], second: [f64; 4]) -> bool {
    first[0] <= second[2] && first[1] <= second[3] && first[2] >= second[0] && first[3] >= second[1]
}

fn point_in_bounds(point: Point, bounds: [f64; 4]) -> bool {
    point.x() >= bounds[0]
        && point.x() <= bounds[2]
        && point.y() >= bounds[1]
        && point.y() <= bounds[3]
}

fn segment_near(first: Point, last: Point, point: Point, maximum: f64) -> bool {
    point.x() >= first.x().min(last.x()) - maximum
        && point.x() <= first.x().max(last.x()) + maximum
        && point.y() >= first.y().min(last.y()) - maximum
        && point.y() <= first.y().max(last.y()) + maximum
}

enum Anchor {
    Observed(Point, [f64; 2]),
    Unresolved(SourceDecision),
}

fn last_dark_frame(
    capture: &Capture,
    palette: &LocalPalette,
    points: &[Point],
    unit: f64,
    work: &mut Work,
) -> Result<Anchor, FingerprintError> {
    let mut last = None;
    for (segment, pair) in points.windows(2).enumerate() {
        let length = (pair[1].x() - pair[0].x()).hypot(pair[1].y() - pair[0].y());
        let intervals = (length / 0.5).ceil().max(1.0);
        work.charge(intervals + 1.0)?;
        for step in 0..=intervals as usize {
            let fraction = step as f64 / intervals;
            let point = Point::new(
                pair[0].x() + fraction * (pair[1].x() - pair[0].x()),
                pair[0].y() + fraction * (pair[1].y() - pair[0].y()),
            )
            .map_err(|_| FingerprintError::InvalidCurve("source anchor coordinate overflow"))?;
            let Some(darkness) = sample_darkness(capture, palette, point) else {
                return Ok(Anchor::Unresolved(SourceDecision::MissingSupport));
            };
            if darkness >= 0.5 {
                last = Some((segment, point));
            }
        }
    }
    let Some((segment, point)) = last else {
        return Ok(Anchor::Unresolved(SourceDecision::ImageUnsupported));
    };
    let mut prefix = points[..=segment].to_vec();
    if prefix.last() != Some(&point) {
        prefix.push(point);
    }
    Ok(match source::terminal_frame(&prefix, unit) {
        Some((anchor, tangent)) => match source_anchor(capture, palette, anchor) {
            Some(pixel) => Anchor::Observed(pixel, tangent),
            None => Anchor::Unresolved(SourceDecision::ImageUnsupported),
        },
        None => Anchor::Unresolved(SourceDecision::ImageUnsupported),
    })
}

fn source_anchor(capture: &Capture, palette: &LocalPalette, point: Point) -> Option<Point> {
    let (x, y) = (point.x().floor(), point.y().floor());
    let (dx, dy) = (point.x() - x, point.y() - y);
    // The last dark interpolated position may lie on the bright-facing edge of
    // a source pixel. Anchor to an actually contributing dark pixel, so its
    // sideways continuation is not excluded by an artificial half-pixel plane.
    [
        (x, y, (1.0 - dx) * (1.0 - dy)),
        (x + 1.0, y, dx * (1.0 - dy)),
        (x, y + 1.0, (1.0 - dx) * dy),
        (x + 1.0, y + 1.0, dx * dy),
    ]
    .into_iter()
    .filter(|&(_, _, weight)| weight > 0.0)
    .filter_map(|(x, y, _)| Point::new(x, y).ok())
    .filter(|&pixel| sample_darkness(capture, palette, pixel).is_some_and(|value| value >= 0.5))
    .min_by(|a, b| {
        (a.x() - point.x())
            .hypot(a.y() - point.y())
            .total_cmp(&(b.x() - point.x()).hypot(b.y() - point.y()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y).unwrap()
    }

    fn local_axes(
        path: &BackbonePath<'_>,
        escaped: Point,
        anchor: Point,
        unit: f64,
        work: &mut Work,
    ) -> Vec<Axis> {
        let maximum = source::SourcePolicy::for_unit(unit).max_backbone_extension_px;
        let bounds = expanded_bounds(
            [escaped.x(), escaped.y(), escaped.x(), escaped.y()],
            maximum,
        );
        let stations = path.local_stations(bounds, work).unwrap();
        let mut axes = Vec::new();
        path.append_axes(&stations, escaped, anchor, unit, &mut axes, work)
            .unwrap();
        axes
    }

    fn signature(axes: Vec<Axis>) -> Vec<(Point, Point, [f64; 2], f64)> {
        axes.into_iter()
            .map(|axis| (axis.origin, axis.center, axis.tangent, axis.distance))
            .collect()
    }

    #[test]
    fn local_bounds_keep_interior_stations_between_distant_endpoints() {
        let points = [
            point(-100.0, 0.0),
            point(100.0, 0.0),
            point(100.0, 100.0),
            point(101.0, 100.0),
        ];
        let backbone = Backbone::from_paths(std::iter::once((&points[..], 0)));
        let path = &backbone.paths[0];
        let escaped = point(0.0, 3.0);
        let anchor = point(0.0, 0.0);
        let mut work = Work::default();
        let actual = local_axes(path, escaped, anchor, 8.0, &mut work);
        let mut expected = Vec::new();
        for index in 0..points.len() {
            expected.extend(path.vertex_axis(index, escaped, anchor, 8.0));
            if index + 1 < points.len() {
                expected.extend(
                    path.segment_axis(index, escaped, 8.0, &mut Work::default())
                        .unwrap(),
                );
            }
        }
        assert_eq!(actual.len(), 1);
        assert_eq!(signature(actual), signature(expected));
        assert!(work.estimated < points.len() as f64 * 32.0);
    }

    #[test]
    fn one_field_selection_preserves_full_scan_axes_at_pixels_and_fractional_boundaries() {
        let paths = [
            vec![point(0.0, 128.0), point(255.0, 128.0)],
            vec![
                point(90.0, 115.0),
                point(110.0, 120.0),
                point(128.0, 125.0),
                point(140.0, 140.0),
                point(160.0, 148.0),
                point(230.0, 180.0),
            ],
            vec![point(20.0, 20.0), point(40.0, 30.0)],
        ];
        let backbone = Backbone::from_paths(
            paths
                .iter()
                .map(|points| (&points[..], 0))
                .chain(std::iter::once((&paths[0][..], 1))),
        );
        let capture = Capture::new(256, 256, vec![20; 256 * 256], None, None).unwrap();
        let palette = LocalPalette {
            radius_px: 8,
            ridge_gray: vec![20.0; 256 * 256],
            valley_gray: vec![220.0; 256 * 256],
            usable: vec![true; 256 * 256],
        };
        let tip = point(128.0, 128.0);
        let field = Field::read(
            &capture,
            &palette,
            tip,
            [1.0, 0.0],
            &[point(112.0, 128.0), tip],
            8.0,
            &mut Work::default(),
        )
        .unwrap()
        .unwrap();
        let candidates = backbone
            .local_candidates(&field, 0, &mut Work::default())
            .unwrap();
        assert_eq!(
            candidates.iter().map(|item| item.path).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(candidates[0].stations.len(), 1);
        assert!(!candidates[0].stations[0].vertex);
        assert!(candidates[0].stations[0].segment);
        assert!(candidates[1].stations.len() < paths[1].len());

        let [left, top, right, bottom] = field.source_bounds();
        for half_y in (2 * top as usize)..=(2 * bottom as usize) {
            for half_x in (2 * left as usize)..=(2 * right as usize) {
                let escaped = point(half_x as f64 / 2.0, half_y as f64 / 2.0);
                let mut actual = Vec::new();
                for selection in &candidates {
                    backbone.paths[selection.path]
                        .append_axes(
                            &selection.stations,
                            escaped,
                            tip,
                            8.0,
                            &mut actual,
                            &mut Work::default(),
                        )
                        .unwrap();
                }
                let mut expected = Vec::new();
                for path in backbone.paths.iter().filter(|path| path.component == 0) {
                    for index in 0..path.points.len() {
                        expected.extend(path.vertex_axis(index, escaped, tip, 8.0));
                        if index + 1 < path.points.len() {
                            expected.extend(
                                path.segment_axis(index, escaped, 8.0, &mut Work::default())
                                    .unwrap(),
                            );
                        }
                    }
                }
                assert_eq!(signature(actual), signature(expected), "query {escaped:?}");
            }
        }

        let mut exhausted = Work {
            estimated: super::super::MAX_WORK as f64 - 1.0,
        };
        assert!(matches!(
            backbone.local_candidates(&field, 0, &mut exhausted),
            Err(FingerprintError::WorkLimit)
        ));
    }

    #[test]
    fn a_clipped_raster_port_has_no_half_arc_gap_in_its_source_footprint() {
        let source = vec![
            point(440.3289777250817, 550.0),
            point(440.0, 550.0),
            point(440.0, 551.0),
            point(439.0, 551.0),
            point(438.0, 551.0),
            point(438.0, 552.0),
            point(437.0, 552.0),
            point(436.0, 553.0),
            point(435.0, 553.0),
            point(434.0, 554.0),
        ];
        let escaped = point(437.83915480111364, 546.1608451988864);
        let unit = 16.820792511697714;
        for reverse in [false, true] {
            let mut points = source.clone();
            if reverse {
                points.reverse();
            }
            let backbone = Backbone::from_paths(std::iter::once((&points[..], 0)));
            let path = &backbone.paths[0];
            let axes = local_axes(
                path,
                escaped,
                point(466.0, 535.0),
                unit,
                &mut Work::default(),
            );
            let axis = axes
                .iter()
                .find(|axis| axis.origin == point(440.0, 550.0))
                .expect("the station next to the fractional clipped port must remain covered");
            let delta = [escaped.x() - axis.center.x(), escaped.y() - axis.center.y()];
            assert!((delta[0] * axis.tangent[0] + delta[1] * axis.tangent[1]).abs() < 1e-10);
            assert!(axis.distance < unit / 2.0);
            assert!(super::super::distance(axis.origin, axis.center) < 0.329);
        }
    }

    #[test]
    fn a_valid_boundary_station_survives_interior_projection_and_resampling() {
        let source = [
            point(867.0, 721.4994493996644),
            point(867.0, 722.0),
            point(867.0, 723.0),
            point(868.0, 723.0),
            point(868.0, 724.0),
            point(868.0, 725.0),
            point(868.0, 726.0),
            point(869.0, 727.0),
            point(869.0, 728.0),
            point(869.0, 729.0),
            point(870.0, 730.0),
            point(870.0, 731.0),
        ];
        let escaped = point(864.0, 722.8668213548855);
        let unit = 16.820792511697714;
        for subdivisions in [1, 2] {
            let mut sampled = vec![source[0]];
            for pair in source.windows(2) {
                for step in 1..=subdivisions {
                    let fraction = step as f64 / subdivisions as f64;
                    sampled.push(point(
                        pair[0].x() + fraction * (pair[1].x() - pair[0].x()),
                        pair[0].y() + fraction * (pair[1].y() - pair[0].y()),
                    ));
                }
            }
            for reverse in [false, true] {
                let mut points = sampled.clone();
                if reverse {
                    points.reverse();
                }
                let backbone = Backbone::from_paths(std::iter::once((&points[..], 0)));
                let axes = local_axes(
                    &backbone.paths[0],
                    escaped,
                    point(855.0, 694.0),
                    unit,
                    &mut Work::default(),
                );
                let axis = axes
                    .iter()
                    .find(|axis| axis.origin == point(867.0, 722.0))
                    .expect("a valid boundary station cannot be replaced by its interior neighbor");
                assert!((axis.center.x() - 867.0165554867039).abs() < 1e-10);
                assert!((axis.center.y() - 722.062056034184).abs() < 1e-10);
                assert!((axis.distance - 3.122059292480976).abs() < 1e-10);
            }
        }
    }
}
