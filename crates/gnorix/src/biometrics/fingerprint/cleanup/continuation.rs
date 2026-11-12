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
            .map(|(_, edge)| {
                let mut bounds = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for point in &edge.points {
                    bounds[0] = bounds[0].min(point.x());
                    bounds[1] = bounds[1].min(point.y());
                    bounds[2] = bounds[2].max(point.x());
                    bounds[3] = bounds[3].max(point.y());
                }
                let mut positions = vec![0.0];
                for pair in edge.points.windows(2) {
                    positions.push(
                        positions.last().copied().unwrap() + super::distance(pair[0], pair[1]),
                    );
                }
                BackbonePath {
                    points: &edge.points,
                    component: components[edge.start],
                    bounds,
                    positions,
                }
            })
            .collect();
        let known_ends = confirmed
            .iter()
            .map(|&id| KnownEnd {
                points: &graph.edges[id].points,
                component: components[graph.edges[id].start],
            })
            .collect();
        Self { paths, known_ends }
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
    for escape in escapes {
        let point = escape.point;
        work.charge(known.iter().map(|points| points.len()).sum::<usize>() as f64)?;
        // Exact, independently cap-confirmed geometry is known finite support,
        // not a ridge-width or extension inference. The flood already traversed
        // through it: source continuation outside these points still needs proof.
        if !escape.boundary && known.iter().any(|points| points.contains(&point)) {
            continue;
        }
        match backbone.explains(capture, palette, point, &field, component, work)? {
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

struct Axis {
    origin: Point,
    center: Point,
    tangent: [f64; 2],
    distance: f64,
}

impl Backbone<'_> {
    fn explains(
        &self,
        capture: &Capture,
        palette: &LocalPalette,
        point: Point,
        field: &Field,
        component: usize,
        work: &mut Work,
    ) -> Result<SourceDecision, FingerprintError> {
        let unit = field.unit;
        let maximum = source::SourcePolicy::for_unit(unit).max_backbone_extension_px;
        let mut axes = Vec::new();
        work.charge(self.paths.len() as f64)?;
        for path in &self.paths {
            // A normal can reach R from the locally continued axis. Its origin
            // can be another R away at an observed endpoint, never farther.
            if path.component != component
                || point.x() < path.bounds[0] - maximum
                || point.y() < path.bounds[1] - maximum
                || point.x() > path.bounds[2] + maximum
                || point.y() > path.bounds[3] + maximum
            {
                continue;
            }
            work.charge(path.points.len() as f64 * 8.0)?;
            for index in 0..path.points.len() {
                if let Some(axis) = path.axis(index, point, field.tip, unit) {
                    axes.push(axis);
                }
            }
        }
        axes.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        let mut missing = false;
        for axis in axes {
            match footprint::dark_segment(capture, palette, axis.origin, axis.center, work)? {
                SourceDecision::Confirmed => {}
                SourceDecision::MissingSupport => {
                    missing = true;
                    continue;
                }
                _ => continue,
            }
            // An exact retained path vertex is already observed backbone,
            // not an inferred transverse footprint. The source-dark prefix
            // above still validates it and every capture/palette corner.
            if point == axis.origin {
                return Ok(SourceDecision::Confirmed);
            }
            match footprint::normal_span(
                capture,
                palette,
                axis.center,
                axis.tangent,
                point,
                unit,
                work,
            )? {
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
    fn axis(&self, index: usize, point: Point, anchor: Point, unit: f64) -> Option<Axis> {
        let origin = self.points[index];
        let maximum = source::SourcePolicy::for_unit(unit).max_backbone_extension_px;
        if super::distance(origin, point) > maximum {
            return None;
        }
        let length = *self.positions.last()?;
        let position = self.positions[index];
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
        // The observed endpoint and the source inspection anchor need not
        // coincide. Cover their displacement, while bounding verification by
        // two measured units and continuously proving every added source pixel.
        let extension = (unit + super::distance(anchor, origin)).min(maximum);
        let low = if index == 0 {
            -extension
        } else {
            -(position - self.positions[index - 1]) / 2.0
        };
        let high = if index + 1 == self.points.len() {
            extension
        } else {
            (self.positions[index + 1] - position) / 2.0
        };
        if along < low || along > high {
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
