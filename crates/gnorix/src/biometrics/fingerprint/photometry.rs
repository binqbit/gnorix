//! Raw grayscale line integrals over the already confirmed source spans.

use super::{Capture, FingerprintError};
use crate::biometrics::geometry::Point;

/// Integrates the piecewise-bilinear source image, with no normalization or
/// intensity threshold. All intersected cells must have observed source support.
/// Splitting at pixel-grid crossings makes Simpson's rule exact within each
/// cell: a bilinear image restricted to a straight line is quadratic.
pub(super) fn mean_gray(
    capture: &Capture,
    start: Point,
    end: Point,
) -> Result<f64, FingerprintError> {
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    let in_bounds = |p: Point| {
        p.x() >= 0.0 && p.y() >= 0.0 && p.x() <= (width - 1) as f64 && p.y() <= (height - 1) as f64
    };
    if !in_bounds(start) || !in_bounds(end) {
        return Err(FingerprintError::ScaleUnavailable(
            "photometry span is outside source pixels",
        ));
    }
    let (dx, dy) = (end.x() - start.x(), end.y() - start.y());
    let length = dx.hypot(dy);
    if !length.is_finite() || length <= 0.0 {
        return Err(FingerprintError::ScaleUnavailable(
            "photometry span must have positive finite length",
        ));
    }
    let mut breaks = vec![0.0, 1.0];
    add_crossings(&mut breaks, start.x(), end.x());
    add_crossings(&mut breaks, start.y(), end.y());
    breaks.sort_by(f64::total_cmp);
    breaks.dedup();
    let (mut integral, mut correction) = (0.0, 0.0);
    for interval in breaks.windows(2) {
        let (left, right) = (interval[0], interval[1]);
        let middle = left + (right - left) / 2.0;
        let x = (start.x() + dx * middle).clamp(0.0, (width - 1) as f64);
        let y = (start.y() + dy * middle).clamp(0.0, (height - 1) as f64);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
        let corners = [
            y0 * width + x0,
            y0 * width + x1,
            y1 * width + x0,
            y1 * width + x1,
        ];
        if capture
            .valid_mask()
            .is_some_and(|mask| corners.iter().any(|&i| !mask[i]))
        {
            return Err(FingerprintError::ScaleUnavailable(
                "photometry span crosses unobserved source pixels",
            ));
        }
        let sample = |t: f64| {
            // Validated endpoints define an in-bounds convex segment. Clamping
            // local fractions only absorbs arithmetic roundoff at cell borders.
            let tx = (start.x() + dx * t - x0 as f64).clamp(0.0, 1.0);
            let ty = (start.y() + dy * t - y0 as f64).clamp(0.0, 1.0);
            let gray = |i| f64::from(capture.pixels()[corners[i]]);
            (gray(0) * (1.0 - tx) + gray(1) * tx) * (1.0 - ty)
                + (gray(2) * (1.0 - tx) + gray(3) * tx) * ty
        };
        // Parameter t has constant speed, so length cancels when dividing the
        // line integral by span length. Retain compensated summation across cells.
        let area = (right - left) * (sample(left) + 4.0 * sample(middle) + sample(right)) / 6.0;
        let adjusted = area - correction;
        let next = integral + adjusted;
        correction = (next - integral) - adjusted;
        integral = next;
    }
    Ok(integral.clamp(0.0, 255.0))
}

fn add_crossings(breaks: &mut Vec<f64>, start: f64, end: f64) {
    if start == end {
        return;
    }
    let first = start.min(end).floor() as usize + 1;
    let last = start.max(end).ceil() as usize;
    // Each crossing is an actual source-grid boundary. Thus work is bounded
    // by source dimensions; the enclosing scale stage already enforces pixels
    // and profile budgets and its maximum finite profile extent.
    for grid in first..last {
        let t = (grid as f64 - start) / (end - start);
        if t > 0.0 && t < 1.0 {
            breaks.push(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y).unwrap()
    }
    fn image(width: u32, height: u32, values: Vec<u8>, mask: Option<Vec<bool>>) -> Capture {
        Capture::new(width, height, values, mask, None).unwrap()
    }

    #[test]
    fn constant_and_linear_fields_have_closed_form_means_in_both_directions() {
        let constant = image(6, 5, vec![73; 30], None);
        assert!(
            (mean_gray(&constant, point(0.2, 0.4), point(4.8, 3.7)).unwrap() - 73.0).abs() < 1e-12
        );
        let ramp = image(
            6,
            5,
            (0..30)
                .map(|i| (7 + 3 * (i % 6) + 11 * (i / 6)) as u8)
                .collect(),
            None,
        );
        let (a, b) = (point(0.2, 0.4), point(4.8, 3.7));
        let expected = 7.0 + 3.0 * (a.x() + b.x()) / 2.0 + 11.0 * (a.y() + b.y()) / 2.0;
        assert!((mean_gray(&ramp, a, b).unwrap() - expected).abs() < 1e-12);
        assert!((mean_gray(&ramp, b, a).unwrap() - expected).abs() < 1e-12);
    }

    #[test]
    fn rotated_bilinear_field_integrates_exactly_across_many_cells() {
        let image = image(
            7,
            6,
            (0..42)
                .map(|i| (10 + 2 * (i % 7) + 3 * (i / 7) + 4 * (i % 7) * (i / 7)) as u8)
                .collect(),
            None,
        );
        let (a, b) = (point(0.3, 0.7), point(5.6, 4.8));
        let (dx, dy) = (b.x() - a.x(), b.y() - a.y());
        let product = a.x() * a.y() + (a.x() * dy + a.y() * dx) / 2.0 + dx * dy / 3.0;
        let expected =
            10.0 + 2.0 * (a.x() + b.x()) / 2.0 + 3.0 * (a.y() + b.y()) / 2.0 + 4.0 * product;
        assert!((mean_gray(&image, a, b).unwrap() - expected).abs() < 1e-11);
        assert!((mean_gray(&image, b, a).unwrap() - expected).abs() < 1e-11);
    }

    #[test]
    fn nonuniform_zones_use_line_length_and_exclude_outside_background() {
        let zones = image(4, 1, vec![20, 20, 20, 220], None);
        assert!(
            (mean_gray(&zones, point(0.0, 0.0), point(3.0, 0.0)).unwrap() - 160.0 / 3.0).abs()
                < 1e-12
        );
        let background = image(
            7,
            2,
            vec![
                255, 255, 40, 40, 40, 255, 255, 255, 255, 40, 40, 40, 255, 255,
            ],
            None,
        );
        assert_eq!(
            mean_gray(&background, point(2.0, 0.5), point(4.0, 0.5)).unwrap(),
            40.0
        );
    }

    #[test]
    fn unsupported_cells_and_outside_spans_are_errors_without_hidden_value_use() {
        let mut mask = vec![true; 16];
        mask[5] = false;
        let a = image(4, 4, vec![10; 16], Some(mask.clone()));
        let mut changed = vec![10; 16];
        changed[5] = 255;
        let b = image(4, 4, changed, Some(mask));
        for input in [&a, &b] {
            assert!(matches!(
                mean_gray(input, point(0.2, 0.2), point(2.8, 2.8)),
                Err(FingerprintError::ScaleUnavailable(_))
            ));
            assert_eq!(
                mean_gray(input, point(2.2, 2.2), point(2.8, 2.8)).unwrap(),
                10.0
            );
            assert!(mean_gray(input, point(-0.1, 0.0), point(1.0, 1.0)).is_err());
            assert!(mean_gray(input, point(1.0, 1.0), point(1.0, 1.0)).is_err());
        }
    }
}
