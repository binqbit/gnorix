//! Continuous source support for a retained ridge's local transverse footprint.

mod cache;
pub(super) use cache::Cache;

use super::super::{photometry, raster};
use super::{Capture, FingerprintError, LocalPalette, Point, SourceDecision, SourcePolicy, Work};

#[derive(Clone, Copy)]
struct CellSource {
    signed: [f64; 4],
    spread: [f64; 4],
}

fn cell_source(
    capture: &Capture,
    palette: &LocalPalette,
    cell: raster::CellInterval,
) -> Option<CellSource> {
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    let right = (cell.x + 1).min(width - 1);
    let bottom = (cell.y + 1).min(height - 1);
    let corners = [
        cell.y * width + cell.x,
        cell.y * width + right,
        bottom * width + cell.x,
        bottom * width + right,
    ];
    let mut result = CellSource {
        signed: [0.0; 4],
        spread: [0.0; 4],
    };
    for (corner, index) in corners.into_iter().enumerate() {
        let (low, high) = (palette.ridge_gray[index], palette.valley_gray[index]);
        if !palette.usable[index]
            || capture.valid_mask().is_some_and(|mask| !mask[index])
            || !low.is_finite()
            || !high.is_finite()
            || !(0.0..=255.0).contains(&low)
            || !(0.0..=255.0).contains(&high)
            || low >= high
        {
            return None;
        }
        result.signed[corner] =
            (f64::from(low) + f64::from(high)) / 2.0 - f64::from(capture.pixels()[index]);
        result.spread[corner] = f64::from(high) - f64::from(low);
    }
    Some(result)
}

fn bilinear(values: [f64; 4], x: f64, y: f64) -> f64 {
    (values[0] * (1.0 - x) + values[1] * x) * (1.0 - y)
        + (values[2] * (1.0 - x) + values[3] * x) * y
}

/// Certifies the whole original-source segment, including every crossed cell.
/// At the established midpoint, darkness >= 0.5 is equivalent to
/// (local_dark + local_light)/2 - source_gray >= 0. This signed field is
/// bilinear within a source cell and therefore quadratic along a line.
pub(super) fn dark_segment(
    capture: &Capture,
    palette: &LocalPalette,
    start: Point,
    end: Point,
    work: &mut Work,
) -> Result<SourceDecision, FingerprintError> {
    if !segment_in_capture(capture, start, end) {
        return Ok(SourceDecision::MissingSupport);
    }
    let (dx, dy) = (end.x() - start.x(), end.y() - start.y());
    // Each grid event visits a constant number of cells, including corner ties.
    work.charge((dx.abs().ceil() + dy.abs().ceil() + 4.0) * 32.0)?;
    Ok(classify_segment(capture, palette, start, end, |cell| {
        cell_source(capture, palette, cell)
    }))
}

fn segment_in_capture(capture: &Capture, start: Point, end: Point) -> bool {
    [start, end].iter().all(|point| {
        point.x() >= 0.0
            && point.y() >= 0.0
            && point.x() <= f64::from(capture.width() - 1)
            && point.y() <= f64::from(capture.height() - 1)
    })
}

fn classify_segment(
    capture: &Capture,
    palette: &LocalPalette,
    start: Point,
    end: Point,
    mut read: impl FnMut(raster::CellInterval) -> Option<CellSource>,
) -> SourceDecision {
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    let (dx, dy) = (end.x() - start.x(), end.y() - start.y());
    let mut dark = true;
    let observed = raster::visit_cells(start, end, width, height, |cell| {
        let signed = read(cell)?.signed;
        if cell.start == cell.end {
            return Some(());
        }
        let sample = |t: f64| {
            let x = (start.x() + dx * t - cell.x as f64).clamp(0.0, 1.0);
            let y = (start.y() + dy * t - cell.y as f64).clamp(0.0, 1.0);
            bilinear(signed, x, y)
        };
        let left = sample(cell.start);
        let middle = sample(cell.start + (cell.end - cell.start) / 2.0);
        let right = sample(cell.end);
        if quadratic_minimum(left, middle, right) < 0.0 {
            dark = false;
        }
        Some(())
    });
    if observed.is_none() {
        return SourceDecision::MissingSupport;
    }
    // The traversal deliberately emits a zero-duration cell for a point query.
    if start == end {
        dark =
            photometry::sample_darkness(capture, palette, start).is_some_and(|value| value >= 0.5);
    }
    if dark {
        SourceDecision::Confirmed
    } else {
        SourceDecision::ImageUnsupported
    }
}

/// Conservative continuous support for a diagnostic source connection.
/// Cardinal links have a linear numerator and positive linear denominator,
/// so their normalized minimum occurs at an endpoint. Other links use the
/// exact quadratic signed-field minimum and the largest cell reference spread.
/// Every touched interpolation cell remains required, even at a corner tie.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SourceLink {
    pub(super) dark: bool,
    pub(super) margin: f64,
}

#[cfg(test)]
fn source_link(
    capture: &Capture,
    palette: &LocalPalette,
    start: Point,
    end: Point,
    work: &mut Work,
) -> Result<Option<SourceLink>, FingerprintError> {
    if !segment_in_capture(capture, start, end) {
        return Ok(None);
    }
    let (dx, dy) = (end.x() - start.x(), end.y() - start.y());
    work.charge((dx.abs().ceil() + dy.abs().ceil() + 4.0) * 32.0)?;
    Ok(classify_link(capture, palette, start, end, |cell| {
        cell_source(capture, palette, cell)
    }))
}

fn classify_link(
    capture: &Capture,
    palette: &LocalPalette,
    start: Point,
    end: Point,
    mut read: impl FnMut(raster::CellInterval) -> Option<CellSource>,
) -> Option<SourceLink> {
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    let (dx, dy) = (end.x() - start.x(), end.y() - start.y());
    let mut margin = 1.0_f64;
    let mut dark = true;
    let observed = raster::visit_cells(start, end, width, height, |cell| {
        let source = read(cell)?;
        if cell.start == cell.end {
            return Some(());
        }
        let sample = |values: [f64; 4], t: f64| {
            let x = (start.x() + dx * t - cell.x as f64).clamp(0.0, 1.0);
            let y = (start.y() + dy * t - cell.y as f64).clamp(0.0, 1.0);
            bilinear(values, x, y)
        };
        let left = sample(source.signed, cell.start);
        let right = sample(source.signed, cell.end);
        let bound = if dx == 0.0 || dy == 0.0 {
            (2.0 * left / sample(source.spread, cell.start))
                .min(2.0 * right / sample(source.spread, cell.end))
        } else {
            let middle = sample(source.signed, cell.start + (cell.end - cell.start) / 2.0);
            let spread = source.spread.into_iter().fold(0.0_f64, f64::max);
            2.0 * quadratic_minimum(left, middle, right) / spread
        };
        dark &= bound >= 0.0;
        margin = margin.min(bound.clamp(0.0, 1.0));
        Some(())
    });
    observed?;
    if start == end {
        return photometry::sample_darkness(capture, palette, start).map(|value| SourceLink {
            dark: value >= 0.5,
            margin: (2.0 * value - 1.0).clamp(0.0, 1.0),
        });
    }
    Some(SourceLink { dark, margin })
}

fn quadratic_minimum(left: f64, middle: f64, right: f64) -> f64 {
    let a = 2.0 * (left + right - 2.0 * middle);
    let b = 4.0 * middle - 3.0 * left - right;
    let mut minimum = left.min(right);
    if a > 0.0 {
        let at = -b / (2.0 * a);
        if (0.0..1.0).contains(&at) {
            minimum = minimum.min((a * at + b) * at + left);
        }
    }
    minimum
}

/// `center` is the caller's perpendicular projection on an observed or
/// source-certified local backbone axis. The caller proves its axis prefix.
/// This helper proves a dark transverse span and observed light on both sides;
/// the agreed upper width bound limits the search, not nominal R/2.
pub(super) fn normal_span(
    capture: &Capture,
    palette: &LocalPalette,
    center: Point,
    tangent: [f64; 2],
    escaped: Point,
    unit: f64,
    work: &mut Work,
) -> Result<SourceDecision, FingerprintError> {
    normal_span_with(
        capture,
        palette,
        center,
        tangent,
        escaped,
        unit,
        work,
        &mut |a, b, work| dark_segment(capture, palette, a, b, work),
    )
}

#[allow(clippy::too_many_arguments)]
fn normal_span_with(
    capture: &Capture,
    palette: &LocalPalette,
    center: Point,
    tangent: [f64; 2],
    escaped: Point,
    unit: f64,
    work: &mut Work,
    segment: &mut impl FnMut(Point, Point, &mut Work) -> Result<SourceDecision, FingerprintError>,
) -> Result<SourceDecision, FingerprintError> {
    let length = tangent[0].hypot(tangent[1]);
    if !length.is_finite() || length <= 0.0 || !unit.is_finite() || unit <= 0.0 {
        return Ok(SourceDecision::ImageUnsupported);
    }
    let maximum = SourcePolicy::for_unit(unit).max_ridge_width_px;
    let tangent = [tangent[0] / length, tangent[1] / length];
    let normal = [-tangent[1], tangent[0]];
    let delta = [escaped.x() - center.x(), escaped.y() - center.y()];
    let along = delta[0] * tangent[0] + delta[1] * tangent[1];
    let across = delta[0] * normal[0] + delta[1] * normal[1];
    let roundoff = 128.0 * f64::EPSILON * center.x().abs().max(center.y().abs()).max(unit).max(1.0);
    if along.abs() > roundoff || across.abs() > maximum {
        return Ok(SourceDecision::ImageUnsupported);
    }
    let span = segment(center, escaped, work)?;
    if span != SourceDecision::Confirmed {
        return Ok(span);
    }
    let left = light_boundary(capture, palette, center, normal, -maximum, work, segment)?;
    let right = light_boundary(capture, palette, center, normal, maximum, work, segment)?;
    Ok(
        if left == SourceDecision::MissingSupport || right == SourceDecision::MissingSupport {
            SourceDecision::MissingSupport
        } else if left == SourceDecision::Confirmed && right == SourceDecision::Confirmed {
            SourceDecision::Confirmed
        } else {
            SourceDecision::ImageUnsupported
        },
    )
}

fn light_boundary(
    capture: &Capture,
    palette: &LocalPalette,
    center: Point,
    normal: [f64; 2],
    extent: f64,
    work: &mut Work,
    segment: &mut impl FnMut(Point, Point, &mut Work) -> Result<SourceDecision, FingerprintError>,
) -> Result<SourceDecision, FingerprintError> {
    let intervals = (extent.abs() / 0.5).ceil().max(1.0);
    work.charge(intervals + 1.0)?;
    let mut previous = center;
    for index in 1..=intervals as usize {
        let distance = extent * index as f64 / intervals;
        let point = Point::new(
            center.x() + normal[0] * distance,
            center.y() + normal[1] * distance,
        )
        .map_err(|_| FingerprintError::InvalidCurve("source footprint coordinate overflow"))?;
        let Some(darkness) = photometry::sample_darkness(capture, palette, point) else {
            return Ok(SourceDecision::MissingSupport);
        };
        if darkness < 0.5 {
            // The last dark-to-light interval still needs complete original
            // support, even when a masked cell is only touched at a corner.
            work.charge(128.0)?;
            return Ok(
                if photometry::segment_observed(capture, palette, previous, point).is_some() {
                    SourceDecision::Confirmed
                } else {
                    SourceDecision::MissingSupport
                },
            );
        }
        match segment(previous, point, work)? {
            SourceDecision::Confirmed => {}
            SourceDecision::ImageUnsupported => {
                // An exact light dip between two dark sample stations is also
                // an observed boundary. The center-to-escape span was proved
                // dark separately, so this cannot excuse crossing that valley.
                return Ok(SourceDecision::Confirmed);
            }
            other => return Ok(other),
        }
        previous = point;
    }
    Ok(SourceDecision::ImageUnsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y).unwrap()
    }

    fn palette(size: usize) -> LocalPalette {
        LocalPalette {
            radius_px: 4,
            ridge_gray: vec![40.0; size],
            valley_gray: vec![200.0; size],
            usable: vec![true; size],
        }
    }

    #[test]
    fn diagonal_dark_vertices_do_not_hide_a_bright_saddle() {
        let capture = Capture::new(2, 2, vec![100, 200, 200, 100], None, None).unwrap();
        for (a, b) in [
            (point(0.0, 0.0), point(1.0, 1.0)),
            (point(1.0, 1.0), point(0.0, 0.0)),
        ] {
            assert_eq!(
                dark_segment(&capture, &palette(4), a, b, &mut Work::default()).unwrap(),
                SourceDecision::ImageUnsupported
            );
        }
        let capture = Capture::new(2, 2, vec![40, 150, 150, 40], None, None).unwrap();
        assert_eq!(
            dark_segment(
                &capture,
                &palette(4),
                point(0.0, 0.0),
                point(1.0, 1.0),
                &mut Work::default()
            )
            .unwrap(),
            SourceDecision::Confirmed
        );
    }

    #[test]
    fn continuous_link_margin_measures_the_diagonal_saddle_not_only_its_vertices() {
        for (middle, expected, dark) in [
            (220, 0.0, false),
            (200, 0.0, true),
            (150, 0.3125, true),
            (40, 1.0, true),
        ] {
            let capture = Capture::new(2, 2, vec![40, middle, middle, 40], None, None).unwrap();
            for (a, b) in [
                (point(0.0, 0.0), point(1.0, 1.0)),
                (point(1.0, 1.0), point(0.0, 0.0)),
            ] {
                assert_eq!(
                    source_link(&capture, &palette(4), a, b, &mut Work::default()).unwrap(),
                    Some(SourceLink {
                        dark,
                        margin: expected
                    }),
                );
            }
        }
    }

    #[test]
    fn cardinal_margin_preserves_normalization_when_local_reference_spreads_differ() {
        let capture = Capture::new(2, 2, vec![25, 90, 25, 90], None, None).unwrap();
        let references = LocalPalette {
            radius_px: 1,
            ridge_gray: vec![0.0; 4],
            valley_gray: vec![100.0, 200.0, 100.0, 200.0],
            usable: vec![true; 4],
        };
        for (a, b) in [
            (point(0.0, 0.0), point(1.0, 0.0)),
            (point(1.0, 0.0), point(0.0, 0.0)),
        ] {
            assert_eq!(
                source_link(&capture, &references, a, b, &mut Work::default()).unwrap(),
                Some(SourceLink {
                    dark: true,
                    margin: 0.1
                }),
            );
        }
    }

    #[test]
    fn graded_links_require_every_corner_and_share_the_work_limit() {
        let capture =
            Capture::new(2, 2, vec![40; 4], Some(vec![true, false, true, true]), None).unwrap();
        let (a, b) = (point(0.0, 0.0), point(1.0, 1.0));
        assert_eq!(
            source_link(&capture, &palette(4), a, b, &mut Work::default()).unwrap(),
            None,
        );
        let mut work = Work {
            estimated: super::super::MAX_WORK as f64 - 1.0,
        };
        assert!(matches!(
            source_link(&capture, &palette(4), a, b, &mut work),
            Err(FingerprintError::WorkLimit),
        ));
    }

    #[test]
    fn off_center_quadratic_extremum_is_checked() {
        // q(t)=(t-0.2)^2-0.01 is positive at t=0,0.5,1 but negative inside.
        assert!(quadratic_minimum(0.03, 0.08, 0.63) < 0.0);
        assert!(quadratic_minimum(0.63, 0.08, 0.03) < 0.0);
        assert_eq!(quadratic_minimum(0.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn tiny_corner_contact_preserves_missing_capture_or_palette() {
        let mut mask = vec![true; 9];
        mask[6] = false;
        let capture = Capture::new(3, 3, vec![40; 9], Some(mask), None).unwrap();
        let (a, b) = (point(0.0, 0.0), point(1.4999999999999998, 1.5));
        assert_eq!(
            dark_segment(&capture, &palette(9), a, b, &mut Work::default()).unwrap(),
            SourceDecision::MissingSupport
        );
        assert_eq!(
            source_link(&capture, &palette(9), a, b, &mut Work::default()).unwrap(),
            None,
        );
        let capture = Capture::new(3, 3, vec![40; 9], None, None).unwrap();
        let mut palette = palette(9);
        palette.usable[6] = false;
        assert_eq!(
            dark_segment(&capture, &palette, b, a, &mut Work::default()).unwrap(),
            SourceDecision::MissingSupport
        );
        assert_eq!(
            source_link(&capture, &palette, b, a, &mut Work::default()).unwrap(),
            None,
        );
    }

    #[test]
    fn observed_normal_uses_actual_boundaries_and_rejects_a_light_valley() {
        let pixels = (0_usize..64 * 64)
            .map(|i| {
                if (26..=38).contains(&(i / 64)) {
                    40
                } else {
                    200
                }
            })
            .collect();
        let capture = Capture::new(64, 64, pixels, None, None).unwrap();
        let references = palette(64 * 64);
        let center = point(32.0, 32.0);
        assert_eq!(
            normal_span(
                &capture,
                &references,
                center,
                [1.0, 0.0],
                point(32.0, 37.0),
                8.0,
                &mut Work::default()
            )
            .unwrap(),
            SourceDecision::Confirmed
        );
        assert_eq!(
            normal_span(
                &capture,
                &references,
                center,
                [1.0, 0.0],
                point(32.0, 40.0),
                8.0,
                &mut Work::default()
            )
            .unwrap(),
            SourceDecision::ImageUnsupported
        );
        let mut interrupted = capture.pixels().to_vec();
        for x in 0..64 {
            interrupted[35 * 64 + x] = 200;
        }
        let capture = Capture::new(64, 64, interrupted, None, None).unwrap();
        assert_eq!(
            normal_span(
                &capture,
                &references,
                center,
                [1.0, 0.0],
                point(32.0, 37.0),
                8.0,
                &mut Work::default()
            )
            .unwrap(),
            SourceDecision::ImageUnsupported
        );
    }

    #[test]
    fn a_light_boundary_between_scan_stations_is_observed_but_cannot_be_crossed() {
        let pixels = (0_usize..64 * 64)
            .map(|index| match index / 64 {
                33 => 121,
                31..=40 => 40,
                _ => 200,
            })
            .collect();
        let capture = Capture::new(64, 64, pixels, None, None).unwrap();
        let references = palette(64 * 64);
        let center = point(32.0, 32.25);
        // Samples y=32.75 and 33.25 are dark, while the source pixel y=33 is light.
        // The exact per-cell minimum identifies the boundary before the latter.
        assert_eq!(
            normal_span(
                &capture,
                &references,
                center,
                [1.0, 0.0],
                point(32.0, 32.5),
                8.0,
                &mut Work::default(),
            )
            .unwrap(),
            SourceDecision::Confirmed
        );
        assert_eq!(
            normal_span(
                &capture,
                &references,
                center,
                [1.0, 0.0],
                point(32.0, 34.0),
                8.0,
                &mut Work::default(),
            )
            .unwrap(),
            SourceDecision::ImageUnsupported
        );
    }
    #[test]
    fn global_tolerance_accepts_observed_borders_between_nominal_and_upper_width() {
        for (half_width, expected) in [
            (11, SourceDecision::Confirmed),
            (12, SourceDecision::ImageUnsupported),
        ] {
            let pixels = (0_usize..64 * 64)
                .map(|index| {
                    if (index / 64).abs_diff(32) <= half_width {
                        40
                    } else {
                        200
                    }
                })
                .collect();
            let capture = Capture::new(64, 64, pixels, None, None).unwrap();
            let decision = normal_span(
                &capture,
                &palette(64 * 64),
                point(32.0, 32.0),
                [1.0, 0.0],
                point(32.0, 42.75),
                10.0,
                &mut Work::default(),
            )
            .unwrap();
            assert_eq!(decision, expected);
        }
    }
}
