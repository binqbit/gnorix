//! Geometric measurements of an already traced path; no biometric decisions.
//!
//! Distances use the input coordinate units. Positive angles turn x toward y.
//! The turning profile is directed and anchored at the supplied first point.
//! Reversing a path reverses its profile and rebases angles at its new start;
//! moving a closed path's seam changes the phase of that profile. Neither is
//! silently canonicalized. A closed profile includes the seam again at fraction
//! one, retaining its accumulated turn rather than resetting it to zero.
//!
//! Sampling uses equal arc-length intervals no larger than the requested spacing.
//! Gaussian support is four sigma, renormalized at open boundaries and wrapped
//! at closed boundaries. Tangents use centered differences, except for one-sided
//! differences at open endpoints. Unwrapping chooses the shortest angular change
//! between neighboring samples; callers must select enough spatial resolution
//! for the features they intend to measure. Smoothing is not gap repair.

use super::{CurveConfig, CurveGeometry, FingerprintError, TurnSample};
use crate::biometrics::geometry::Point;

const MAX_POINTS: usize = 100_000;
const MAX_SMOOTHING_WORK: usize = 20_000_000;
const GAUSSIAN_SUPPORT: f64 = 4.0;

#[derive(Clone, Copy)]
struct Coordinates {
    x: f64,
    y: f64,
}

struct Path {
    points: Vec<Point>,
    cumulative: Vec<f64>,
    length: f64,
    closed: bool,
}

pub(super) fn describe(
    points: &[Point],
    closed: bool,
    config: CurveConfig,
) -> Result<CurveGeometry, FingerprintError> {
    validate_config(config)?;
    let path = Path::new(points, closed)?;
    let (samples, spacing) = path.resample(config.sample_spacing)?;
    let smoothed = smooth(&samples, spacing, config.smoothing_sigma, closed)?;
    let turning = turning_profile(&smoothed, closed)?;

    measurements(&path, turning)
}

/// Measures a supplied geometric contour without smoothing it a second time.
pub(super) fn describe_smoothed(
    points: &[Point],
    closed: bool,
    sample_spacing: f64,
) -> Result<CurveGeometry, FingerprintError> {
    validate_spacing(sample_spacing)?;
    let path = Path::new(points, closed)?;
    let (samples, _) = path.resample(sample_spacing)?;
    let turning = turning_profile(&samples, closed)?;

    measurements(&path, turning)
}

fn measurements(path: &Path, turning: Vec<TurnSample>) -> Result<CurveGeometry, FingerprintError> {
    Ok(CurveGeometry {
        arc_length: path.length,
        chord_length: if path.closed {
            0.0
        } else {
            distance(path.points[0], path.points[path.points.len() - 1])?
        },
        midpoint: path.point_at(path.length / 2.0)?,
        turning,
    })
}

fn validate_config(config: CurveConfig) -> Result<(), FingerprintError> {
    validate_spacing(config.sample_spacing)?;

    if !config.smoothing_sigma.is_finite() || config.smoothing_sigma <= 0.0 {
        return Err(FingerprintError::InvalidConfig(
            "curve smoothing sigma must be finite and positive",
        ));
    }

    Ok(())
}

fn validate_spacing(spacing: f64) -> Result<(), FingerprintError> {
    if !spacing.is_finite() || spacing <= 0.0 {
        return Err(FingerprintError::InvalidConfig(
            "curve sample spacing must be finite and positive",
        ));
    }
    Ok(())
}

impl Path {
    fn new(input: &[Point], closed: bool) -> Result<Self, FingerprintError> {
        if input.len() > MAX_POINTS {
            return Err(FingerprintError::WorkLimit);
        }

        let mut points = input.to_vec();
        points.dedup();
        if closed && points.len() > 1 && points.first() == points.last() {
            points.pop();
        }

        if points.len() < if closed { 3 } else { 2 } {
            return Err(FingerprintError::InvalidCurve(
                "curve has too few distinct consecutive points",
            ));
        }

        let segments = points.len() - usize::from(!closed);
        let mut cumulative = Vec::with_capacity(segments + 1);
        cumulative.push(0.0);
        let mut length = 0.0;

        for index in 0..segments {
            let next = length + distance(points[index], points[(index + 1) % points.len()])?;
            if !next.is_finite() || next <= length {
                return Err(FingerprintError::InvalidCurve(
                    "curve length exceeds floating-point range or precision",
                ));
            }
            cumulative.push(next);
            length = next;
        }

        Ok(Self {
            points,
            cumulative,
            length,
            closed,
        })
    }

    fn segment_at(&self, distance: f64) -> (usize, f64) {
        let segment = self
            .cumulative
            .partition_point(|position| *position <= distance)
            .saturating_sub(1)
            .min(self.cumulative.len() - 2);
        let fraction = (distance - self.cumulative[segment])
            / (self.cumulative[segment + 1] - self.cumulative[segment]);
        (segment, fraction)
    }

    fn point_at(&self, distance: f64) -> Result<Point, FingerprintError> {
        let (segment, fraction) = self.segment_at(distance);
        let start = self.points[segment];
        let end = self.points[(segment + 1) % self.points.len()];

        Point::new(
            start.x() + fraction * (end.x() - start.x()),
            start.y() + fraction * (end.y() - start.y()),
        )
        .map_err(|_| FingerprintError::InvalidCurve("interpolated point is not finite"))
    }

    fn resample(
        &self,
        requested_spacing: f64,
    ) -> Result<(Vec<Coordinates>, f64), FingerprintError> {
        let intervals = (self.length / requested_spacing)
            .ceil()
            .max(if self.closed { 3.0 } else { 1.0 });
        if intervals > (MAX_POINTS - 1) as f64 {
            return Err(FingerprintError::WorkLimit);
        }

        let intervals = intervals as usize;
        let count = intervals + usize::from(!self.closed);
        let spacing = self.length / intervals as f64;
        if spacing == 0.0 {
            return Err(FingerprintError::InvalidCurve(
                "resampling spacing is below floating-point range",
            ));
        }

        // Work in a translated, length-normalized frame so convolution cannot
        // overflow and does not mix a large image origin into small tangents.
        let origin = self.points[0];
        let mut samples = Vec::with_capacity(count);
        for index in 0..count {
            let (segment, fraction) =
                self.segment_at(self.length * (index as f64 / intervals as f64));
            let start = self.points[segment];
            let end = self.points[(segment + 1) % self.points.len()];
            let local_start = Coordinates {
                x: (start.x() - origin.x()) / self.length,
                y: (start.y() - origin.y()) / self.length,
            };
            let local_end = Coordinates {
                x: (end.x() - origin.x()) / self.length,
                y: (end.y() - origin.y()) / self.length,
            };
            samples.push(Coordinates {
                x: local_start.x + fraction * (local_end.x - local_start.x),
                y: local_start.y + fraction * (local_end.y - local_start.y),
            });
        }

        Ok((samples, spacing))
    }
}

fn distance(start: Point, end: Point) -> Result<f64, FingerprintError> {
    let length = (end.x() - start.x()).hypot(end.y() - start.y());
    if !length.is_finite() {
        return Err(FingerprintError::InvalidCurve(
            "point distance exceeds floating-point range",
        ));
    }
    Ok(length)
}

fn smooth(
    samples: &[Coordinates],
    spacing: f64,
    sigma: f64,
    closed: bool,
) -> Result<Vec<Coordinates>, FingerprintError> {
    let sigma_samples = sigma / spacing;
    let support = (GAUSSIAN_SUPPORT * sigma_samples).ceil();
    if support > MAX_SMOOTHING_WORK as f64 {
        return Err(FingerprintError::WorkLimit);
    }
    let radius = support as usize;
    let work = samples.len().checked_mul(2 * radius + 1);
    if work.is_none_or(|work| work > MAX_SMOOTHING_WORK) {
        return Err(FingerprintError::WorkLimit);
    }

    let mut weights = Vec::with_capacity(radius + 1);
    weights.push(1.0);
    for offset in 1..=radius {
        let scaled = offset as f64 / sigma_samples;
        weights.push((-0.5 * scaled * scaled).exp());
    }

    let mut result = Vec::with_capacity(samples.len());
    for index in 0..samples.len() {
        let mut sum = Coordinates { x: 0.0, y: 0.0 };
        let mut mass = 0.0;
        for offset in -(radius as isize)..=radius as isize {
            let neighbor = index as isize + offset;
            let neighbor = if closed {
                neighbor.rem_euclid(samples.len() as isize) as usize
            } else if (0..samples.len() as isize).contains(&neighbor) {
                neighbor as usize
            } else {
                continue;
            };
            let weight = weights[offset.unsigned_abs()];
            sum.x += weight * samples[neighbor].x;
            sum.y += weight * samples[neighbor].y;
            mass += weight;
        }
        result.push(Coordinates {
            x: sum.x / mass,
            y: sum.y / mass,
        });
    }

    Ok(result)
}

fn turning_profile(
    samples: &[Coordinates],
    closed: bool,
) -> Result<Vec<TurnSample>, FingerprintError> {
    let mut directions = Vec::with_capacity(samples.len());
    for index in 0..samples.len() {
        let previous = if index > 0 {
            index - 1
        } else if closed {
            samples.len() - 1
        } else {
            0
        };
        let next = if index + 1 < samples.len() {
            index + 1
        } else if closed {
            0
        } else {
            index
        };
        let dx = samples[next].x - samples[previous].x;
        let dy = samples[next].y - samples[previous].y;
        // This is a numerical-resolution guard in a unit-length frame, not a
        // biometric curvature threshold. Collapsed curves have no tangent.
        if !dx.is_finite() || !dy.is_finite() || dx.hypot(dy) <= 32.0 * f64::EPSILON {
            return Err(FingerprintError::InvalidCurve(
                "smoothed curve has an unresolved tangent",
            ));
        }
        directions.push(dy.atan2(dx));
    }

    let intervals = samples.len() - usize::from(!closed);
    let mut turning = Vec::with_capacity(intervals + 1);
    let mut angle = 0.0;
    let mut previous = directions[0];
    for index in 0..=intervals {
        let direction = directions[index % directions.len()];
        let delta = direction - previous;
        angle += delta.sin().atan2(delta.cos());
        turning.push(TurnSample {
            arc_fraction: index as f64 / intervals as f64,
            angle_radians: angle,
        });
        previous = direction;
    }

    Ok(turning)
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, TAU};

    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y).unwrap()
    }

    fn config() -> CurveConfig {
        CurveConfig {
            sample_spacing: 0.17,
            smoothing_sigma: 0.23,
        }
    }

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
    }

    #[test]
    fn measures_original_path_even_when_smoothing_rounds_its_corner() {
        let path = [point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)];
        let geometry = describe(&path, false, config()).unwrap();

        close(geometry.arc_length, 7.0);
        close(geometry.chord_length, 5.0);
        assert_eq!(geometry.midpoint, point(3.0, 0.5));
        close(geometry.turning[0].angle_radians, 0.0);
        close(geometry.turning.last().unwrap().angle_radians, FRAC_PI_2);
        assert!(geometry.turning.windows(2).all(|pair| {
            pair[1].arc_fraction > pair[0].arc_fraction
                && pair[1].angle_radians + 1e-10 >= pair[0].angle_radians
        }));
        assert!(geometry.turning.iter().any(|sample| {
            sample.angle_radians > 0.1 && sample.angle_radians < FRAC_PI_2 - 0.1
        }));
    }

    #[test]
    fn final_contour_measurements_do_not_apply_another_gaussian() {
        let path = [point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)];
        let geometry = describe_smoothed(&path, false, 0.1).unwrap();
        close(geometry.arc_length, 7.0);
        close(geometry.chord_length, 5.0);
        assert_eq!(geometry.midpoint, point(3.0, 0.5));
        // Away from the corner, tangents are exactly those of the supplied
        // straight segments. A second Gaussian would spread the turn farther.
        for sample in &geometry.turning {
            if sample.arc_fraction < 2.8 / 7.0 {
                close(sample.angle_radians, 0.0);
            } else if sample.arc_fraction > 3.2 / 7.0 {
                close(sample.angle_radians, FRAC_PI_2);
            }
        }
    }

    #[test]
    fn nonuniform_input_density_and_repeated_vertices_do_not_weight_smoothing() {
        let sparse = [point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)];
        let dense = [
            point(0.0, 0.0),
            point(0.0, 0.0),
            point(0.1, 0.0),
            point(2.8, 0.0),
            point(3.0, 0.0),
            point(3.0, 0.3),
            point(3.0, 4.0),
        ];
        let left = describe(&sparse, false, config()).unwrap();
        let right = describe(&dense, false, config()).unwrap();

        close(left.arc_length, right.arc_length);
        assert_eq!(left.turning.len(), right.turning.len());
        for (a, b) in left.turning.iter().zip(&right.turning) {
            close(a.angle_radians, b.angle_radians);
        }
    }

    #[test]
    fn directed_profile_is_invariant_to_translation_and_rotation() {
        let path = [point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)];
        let (sin, cos) = 2.8_f64.sin_cos();
        let transformed: Vec<_> = path
            .iter()
            .map(|p| {
                point(
                    40.0 + cos * p.x() - sin * p.y(),
                    -70.0 + sin * p.x() + cos * p.y(),
                )
            })
            .collect();
        let original = describe(&path, false, config()).unwrap();
        let moved = describe(&transformed, false, config()).unwrap();

        close(original.arc_length, moved.arc_length);
        close(original.chord_length, moved.chord_length);
        assert_eq!(original.turning.len(), moved.turning.len());
        for (a, b) in original.turning.iter().zip(&moved.turning) {
            close(a.angle_radians, b.angle_radians);
        }
    }

    #[test]
    fn large_origin_does_not_quantize_the_resampled_profile() {
        let path = [point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)];
        let offset = 2.0_f64.powi(48);
        let translated: Vec<_> = path
            .iter()
            .map(|p| point(p.x() + offset, p.y() - offset))
            .collect();
        let original = describe(&path, false, config()).unwrap();
        let moved = describe(&translated, false, config()).unwrap();

        assert_eq!(original.turning.len(), moved.turning.len());
        for (a, b) in original.turning.iter().zip(&moved.turning) {
            close(a.angle_radians, b.angle_radians);
        }
    }

    #[test]
    fn reversal_rebases_the_reversed_profile_without_changing_lengths() {
        let path = [point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)];
        let reversed: Vec<_> = path.iter().copied().rev().collect();
        let forward = describe(&path, false, config()).unwrap();
        let backward = describe(&reversed, false, config()).unwrap();
        let total = forward.turning.last().unwrap().angle_radians;

        close(forward.arc_length, backward.arc_length);
        close(forward.chord_length, backward.chord_length);
        assert_eq!(forward.midpoint, backward.midpoint);
        for (a, b) in forward.turning.iter().rev().zip(&backward.turning) {
            close(a.angle_radians - total, b.angle_radians);
        }
    }

    #[test]
    fn closed_seam_retains_winding_and_accepts_an_explicit_duplicate_endpoint() {
        let square = [
            point(0.0, 0.0),
            point(2.0, 0.0),
            point(2.0, 2.0),
            point(0.0, 2.0),
        ];
        let mut duplicated = square.to_vec();
        duplicated.push(square[0]);
        let shape = describe(&square, true, config()).unwrap();
        let duplicate = describe(&duplicated, true, config()).unwrap();
        let reverse = [square[0], square[3], square[2], square[1]];
        let reversed = describe(&reverse, true, config()).unwrap();

        close(shape.arc_length, 8.0);
        close(shape.chord_length, 0.0);
        assert_eq!(shape.midpoint, point(2.0, 2.0));
        close(shape.turning.last().unwrap().arc_fraction, 1.0);
        close(shape.turning.last().unwrap().angle_radians, TAU);
        close(reversed.turning.last().unwrap().angle_radians, -TAU);
        assert_eq!(shape.turning.len(), duplicate.turning.len());
        for (a, b) in shape.turning.iter().zip(&duplicate.turning) {
            close(a.angle_radians, b.angle_radians);
        }
    }

    #[test]
    fn closed_midpoint_follows_the_supplied_seam() {
        let first = [
            point(0.0, 0.0),
            point(2.0, 0.0),
            point(2.0, 2.0),
            point(0.0, 2.0),
        ];
        let shifted = [first[1], first[2], first[3], first[0]];
        let a = describe(&first, true, config()).unwrap();
        let b = describe(&shifted, true, config()).unwrap();

        assert_eq!(a.midpoint, first[2]);
        assert_eq!(b.midpoint, first[3]);
        close(a.arc_length, b.arc_length);
        close(
            a.turning.last().unwrap().angle_radians,
            b.turning.last().unwrap().angle_radians,
        );
    }

    #[test]
    fn straight_path_has_no_turn_even_with_only_two_requested_samples() {
        let shape = describe(
            &[point(10.0, 10.0), point(13.0, 14.0)],
            false,
            CurveConfig {
                sample_spacing: 100.0,
                smoothing_sigma: 0.2,
            },
        )
        .unwrap();

        assert_eq!(shape.turning.len(), 2);
        close(shape.arc_length, 5.0);
        assert_eq!(shape.midpoint, point(11.5, 12.0));
        assert!(
            shape
                .turning
                .iter()
                .all(|sample| sample.angle_radians == 0.0)
        );
    }

    #[test]
    fn invalid_configuration_and_degenerate_paths_are_rejected() {
        let line = [point(0.0, 0.0), point(1.0, 0.0)];
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            for bad in [
                CurveConfig {
                    sample_spacing: value,
                    ..config()
                },
                CurveConfig {
                    smoothing_sigma: value,
                    ..config()
                },
            ] {
                assert!(matches!(
                    describe(&line, false, bad),
                    Err(FingerprintError::InvalidConfig(_))
                ));
            }
        }
        for path in [vec![], vec![line[0]], vec![line[0], line[0]]] {
            assert!(matches!(
                describe(&path, false, config()),
                Err(FingerprintError::InvalidCurve(_))
            ));
        }
        assert!(matches!(
            describe(&line, true, config()),
            Err(FingerprintError::InvalidCurve(_))
        ));
    }

    #[test]
    fn numerical_overflow_and_unbounded_sampling_or_smoothing_are_rejected() {
        let huge = [point(-f64::MAX, 0.0), point(f64::MAX, 0.0)];
        assert!(matches!(
            describe(&huge, false, config()),
            Err(FingerprintError::InvalidCurve(_))
        ));
        let line = [point(0.0, 0.0), point(1.0, 0.0)];
        for limited in [
            CurveConfig {
                sample_spacing: f64::MIN_POSITIVE,
                ..config()
            },
            CurveConfig {
                smoothing_sigma: f64::MAX,
                ..config()
            },
        ] {
            assert!(matches!(
                describe(&line, false, limited),
                Err(FingerprintError::WorkLimit)
            ));
        }
        let collapsed = CurveConfig {
            sample_spacing: 1.0,
            smoothing_sigma: 100.0,
        };
        let retraced = [point(0.0, 0.0), point(1.0, 0.0), point(0.0, 0.0)];
        assert!(matches!(
            describe(&retraced, false, collapsed),
            Err(FingerprintError::InvalidCurve(_))
        ));
    }
}
