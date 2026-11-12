//! Full traversal of a raw, adaptive grayscale measurement skeleton.
//!
//! Bootstrap scale selects bounded processing support, never the final unit.
//! Every unique skeleton link contributes attempted arc length, including links
//! inside junction clusters and the closing links of cycles. Only its own raw
//! ridge span can contribute a measurement; neighboring ridges confirm context.

use std::fmt;

use super::{
    BootstrapScale, Capture, ExtractionConfig, FingerprintError, RidgeMap, RidgeSpan, ScaleProfile,
    enhancement::{self, MeasurementInput},
    graph, photometry, scale, thinning,
};
use crate::biometrics::geometry::Point;

#[derive(Clone, Debug, PartialEq)]
pub struct RidgeObservation {
    /// Canonical source pixel indices, first < second, in row-major order.
    pub link: [usize; 2],
    /// The original grayscale width span containing this link's midpoint.
    pub span: RidgeSpan,
    /// Source skeleton arc length represented by this observation: 1 or sqrt(2).
    pub represented_length_px: f64,
    /// Exact piecewise-bilinear raw grayscale line mean, in [0, 255].
    pub mean_gray: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasurementCoverage {
    pub total_length_px: f64,
    pub measured_length_px: f64,
    pub total_links: usize,
    pub measured_links: usize,
    /// Isolated points have no defined arc length or reliable local tangent.
    pub isolated_pixels: usize,
}

#[derive(PartialEq)]
pub struct FullScale {
    /// Arc-length-weighted arithmetic mean of every measurable own-ridge width.
    pub unit_px: f64,
    pub coverage: MeasurementCoverage,
    pub observations: Vec<RidgeObservation>,
    pub skeleton: RidgeMap,
    /// Union of local observed three-ridge ribbons; not anatomical segmentation.
    pub pattern_support: Vec<bool>,
}

impl fmt::Debug for FullScale {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FullScale")
            .field("unit_px", &self.unit_px)
            .field("coverage", &self.coverage)
            .field("observations", &"[REDACTED]")
            .field("images", &"[REDACTED]")
            .finish()
    }
}

struct Draft {
    link: [usize; 2],
    span: RidgeSpan,
    length: f64,
}

pub(super) fn measure(
    capture: &Capture,
    config: &ExtractionConfig,
    bootstrap: &BootstrapScale,
) -> Result<FullScale, FingerprintError> {
    config.validate()?;
    if capture.pixels().len() > config.max_pixels {
        return Err(FingerprintError::WorkLimit);
    }
    let input = enhancement::measurement_input(capture, config, bootstrap.ridge_period_px)?;
    let count = capture.pixels().len();
    if input.map.width() != capture.width() as usize
        || input.map.height() != capture.height() as usize
        || input.normal_angles.len() != count
        || input.normal_usable.len() != count
    {
        return Err(FingerprintError::InvalidMap(
            "measurement input must use source dimensions",
        ));
    }
    let skeleton = thinning::thin(&input.map)?;
    let mut regions = RegionRaster::new(skeleton.width(), skeleton.height())?;
    let mut drafts = Vec::new();
    let coverage = visit_links(&skeleton, |a, b, length| {
        let Some(angle) = midpoint_normal(&input, a, b) else {
            return Ok(());
        };
        let center = midpoint(&skeleton, a, b)?;
        let Some(profile) = scale::sample_width(
            capture,
            center.x(),
            center.y(),
            angle,
            bootstrap.ridge_period_px,
            config.min_contrast,
        )?
        else {
            return Ok(());
        };
        if let Some(confirmation) = profile.confirmation {
            regions.add(&confirmation)?;
        }
        drafts.push(Draft {
            link: [a, b],
            span: profile.ridge,
            length,
        });
        Ok(())
    })?;
    let pattern_support = regions.finish(capture);
    finish(capture, skeleton, coverage, drafts, pattern_support)
}

fn finish(
    capture: &Capture,
    skeleton: RidgeMap,
    mut coverage: MeasurementCoverage,
    drafts: Vec<Draft>,
    pattern_support: Vec<bool>,
) -> Result<FullScale, FingerprintError> {
    let mut observations = Vec::new();
    let mut weighted_width = 0.0;
    for draft in drafts {
        // Requiring both link pixels is conservative midpoint support. The raw
        // width span itself additionally requires complete source observation.
        if !pattern_support[draft.link[0]] || !pattern_support[draft.link[1]] {
            continue;
        }
        let gray = match photometry::mean_gray(capture, draft.span.start, draft.span.end) {
            Ok(gray) => gray,
            Err(FingerprintError::ScaleUnavailable(_)) => continue,
            Err(error) => return Err(error),
        };
        let width = distance(draft.span.start, draft.span.end);
        if !width.is_finite() || width <= 0.0 {
            return Err(FingerprintError::InvalidCurve(
                "raw ridge width must be finite and positive",
            ));
        }
        weighted_width += width * draft.length;
        coverage.measured_length_px += draft.length;
        coverage.measured_links += 1;
        observations.push(RidgeObservation {
            link: draft.link,
            span: draft.span,
            represented_length_px: draft.length,
            mean_gray: gray,
        });
    }
    if coverage.measured_length_px == 0.0 {
        return Err(FingerprintError::ScaleUnavailable(
            "no measurable raw ridge links in confirmed pattern regions",
        ));
    }
    let unit_px = weighted_width / coverage.measured_length_px;
    if !unit_px.is_finite() || unit_px <= 0.0 {
        return Err(FingerprintError::ScaleUnavailable(
            "full ridge mean is not finite and positive",
        ));
    }
    Ok(FullScale {
        unit_px,
        coverage,
        observations,
        skeleton,
        pattern_support,
    })
}

fn visit_links(
    skeleton: &RidgeMap,
    mut visit: impl FnMut(usize, usize, f64) -> Result<(), FingerprintError>,
) -> Result<MeasurementCoverage, FingerprintError> {
    let mut coverage = MeasurementCoverage {
        total_length_px: 0.0,
        measured_length_px: 0.0,
        total_links: 0,
        measured_links: 0,
        isolated_pixels: 0,
    };
    for pixel in 0..skeleton.ridges().len() {
        if !skeleton.ridges()[pixel] || !skeleton.usable()[pixel] {
            continue;
        }
        let neighbors = graph::neighbors(skeleton, pixel);
        if neighbors.iter().all(Option::is_none) {
            coverage.isolated_pixels += 1;
        }
        for next in neighbors.into_iter().flatten().filter(|&next| next > pixel) {
            let length = ((next % skeleton.width()) as f64 - (pixel % skeleton.width()) as f64)
                .hypot((next / skeleton.width()) as f64 - (pixel / skeleton.width()) as f64);
            coverage.total_length_px += length;
            coverage.total_links += 1;
            visit(pixel, next, length)?;
        }
    }
    Ok(coverage)
}

fn midpoint(map: &RidgeMap, a: usize, b: usize) -> Result<Point, FingerprintError> {
    Point::new(
        ((a % map.width()) + (b % map.width())) as f64 / 2.0,
        ((a / map.width()) + (b / map.width())) as f64 / 2.0,
    )
    .map_err(|_| FingerprintError::InvalidMap("nonfinite measurement midpoint"))
}

fn midpoint_normal(input: &MeasurementInput, a: usize, b: usize) -> Option<f64> {
    let width = input.map.width();
    let height = input.map.height();
    let x = ((a % width) + (b % width)) as f64 / 2.0;
    let y = ((a / width) + (b / width)) as f64 / 2.0;
    let (left, top) = (x.floor() as usize, y.floor() as usize);
    let (right, bottom) = ((left + 1).min(width - 1), (top + 1).min(height - 1));
    let (dx, dy) = (x - left as f64, y - top as f64);
    let samples = [
        (top * width + left, (1.0 - dx) * (1.0 - dy)),
        (top * width + right, dx * (1.0 - dy)),
        (bottom * width + left, (1.0 - dx) * dy),
        (bottom * width + right, dx * dy),
    ];
    let (mut cosine, mut sine) = (0.0, 0.0);
    for (i, weight) in samples {
        if weight == 0.0 {
            continue;
        }
        if !input.normal_usable[i] || !input.normal_angles[i].is_finite() {
            return None;
        }
        let (s, c) = (2.0 * f64::from(input.normal_angles[i])).sin_cos();
        cosine += weight * c;
        sine += weight * s;
    }
    // An axial cancellation has no defined normal; do not choose a digital
    // link direction or substitute a neighbor's direction in its place.
    (cosine.hypot(sine) > 8.0 * f64::from(f32::EPSILON)).then(|| sine.atan2(cosine) / 2.0)
}

/// Rotated ribbons contribute row intervals to a difference image. Union work
/// is O(ribbon height), followed by one O(pixels) scan, rather than repeatedly
/// painting O(period²) pixels for every adjacent confirmed link.
struct RegionRaster {
    width: usize,
    height: usize,
    stride: usize,
    difference: Vec<i32>,
}

impl RegionRaster {
    fn new(width: usize, height: usize) -> Result<Self, FingerprintError> {
        let stride = width.checked_add(1).ok_or(FingerprintError::WorkLimit)?;
        let count = stride
            .checked_mul(height)
            .ok_or(FingerprintError::WorkLimit)?;
        Ok(Self {
            width,
            height,
            stride,
            difference: vec![0; count],
        })
    }

    fn add(&mut self, profile: &ScaleProfile) -> Result<(), FingerprintError> {
        let (start, end) = (profile.ridges[0].start, profile.ridges[2].end);
        let extent = distance(start, end);
        let centers = profile.ridges.each_ref().map(|span| {
            (
                (span.start.x() + span.end.x()) / 2.0,
                (span.start.y() + span.end.y()) / 2.0,
            )
        });
        let period = ((centers[1].0 - centers[0].0).hypot(centers[1].1 - centers[0].1)
            + (centers[2].0 - centers[1].0).hypot(centers[2].1 - centers[1].1))
            / 2.0;
        if !extent.is_finite() || extent <= 0.0 || !period.is_finite() || period <= 0.0 {
            return Err(FingerprintError::InvalidCurve(
                "confirmation ribbon must have finite positive extent",
            ));
        }
        let (nx, ny) = (
            (end.x() - start.x()) / extent,
            (end.y() - start.y()) / extent,
        );
        let (tx, ty) = (-ny, nx);
        let (cx, cy) = ((start.x() + end.x()) / 2.0, (start.y() + end.y()) / 2.0);
        let (half_normal, half_tangent) = (extent / 2.0, period / 2.0);
        let epsilon = 32.0 * f64::EPSILON * (cx.abs() + cy.abs() + extent + period + 1.0);
        let vertical = ny.abs() * half_normal + ty.abs() * half_tangent;
        let low_y = (cy - vertical - epsilon).ceil().max(0.0);
        let high_y = (cy + vertical + epsilon)
            .floor()
            .min((self.height - 1) as f64);
        if high_y < low_y {
            return Ok(());
        }
        for y in low_y as usize..=high_y as usize {
            let dy = y as f64 - cy;
            let Some((a, b)) = axis_interval(nx, ny, half_normal, dy, epsilon) else {
                continue;
            };
            let Some((c, d)) = axis_interval(tx, ty, half_tangent, dy, epsilon) else {
                continue;
            };
            let low = (cx + a.max(c) - epsilon).ceil().max(0.0);
            let high = (cx + b.min(d) + epsilon)
                .floor()
                .min((self.width - 1) as f64);
            if high < low {
                continue;
            }
            let start = y * self.stride + low as usize;
            let end = y * self.stride + high as usize + 1;
            self.difference[start] = self.difference[start]
                .checked_add(1)
                .ok_or(FingerprintError::WorkLimit)?;
            self.difference[end] = self.difference[end]
                .checked_sub(1)
                .ok_or(FingerprintError::WorkLimit)?;
        }
        Ok(())
    }

    fn finish(self, capture: &Capture) -> Vec<bool> {
        let mut support = vec![false; self.width * self.height];
        for y in 0..self.height {
            let mut active = 0_i64;
            for x in 0..self.width {
                active += i64::from(self.difference[y * self.stride + x]);
                let index = y * self.width + x;
                support[index] = active > 0 && capture.valid_mask().is_none_or(|mask| mask[index]);
            }
        }
        support
    }
}

fn axis_interval(a: f64, b: f64, half: f64, dy: f64, epsilon: f64) -> Option<(f64, f64)> {
    if a.abs() <= 8.0 * f64::EPSILON {
        return ((b * dy).abs() <= half + epsilon).then_some((f64::NEG_INFINITY, f64::INFINITY));
    }
    let (first, last) = ((-half - b * dy) / a, (half - b * dy) / a);
    Some((first.min(last), first.max(last)))
}

fn distance(a: Point, b: Point) -> f64 {
    (b.x() - a.x()).hypot(b.y() - a.y())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI, SQRT_2};

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y).unwrap()
    }

    fn map(width: usize, height: usize, points: &[(usize, usize)]) -> RidgeMap {
        let mut ridges = vec![false; width * height];
        for &(x, y) in points {
            ridges[y * width + x] = true;
        }
        RidgeMap::new(width, height, ridges, vec![true; width * height]).unwrap()
    }

    fn capture(width: usize, height: usize, mask: Option<Vec<bool>>) -> Capture {
        Capture::new(
            width as u32,
            height as u32,
            vec![50; width * height],
            mask,
            None,
        )
        .unwrap()
    }

    fn profile() -> ScaleProfile {
        ScaleProfile {
            ridges: std::array::from_fn(|i| RidgeSpan {
                start: point(2.0 + 4.0 * i as f64, 5.0),
                end: point(4.0 + 4.0 * i as f64, 5.0),
            }),
            ridge_mean_gray: [30.0; 3],
            valley_mean_gray: [220.0; 2],
        }
    }

    #[test]
    fn unique_links_include_cycles_diagonals_and_report_isolates_separately() {
        let skeleton = map(
            8,
            8,
            &[
                (3, 1),
                (4, 2),
                (3, 3),
                (2, 2),
                (0, 6),
                (1, 6),
                (2, 6),
                (7, 7),
            ],
        );
        let mut links = Vec::new();
        let coverage = visit_links(&skeleton, |a, b, length| {
            links.push(([a, b], length));
            Ok(())
        })
        .unwrap();
        assert_eq!(coverage.total_links, 6);
        assert_eq!(coverage.isolated_pixels, 1);
        assert!((coverage.total_length_px - (4.0 * SQRT_2 + 2.0)).abs() < 1e-12);
        assert!(links.iter().all(|(pair, _)| pair[0] < pair[1]));
        links.sort_by_key(|(pair, _)| *pair);
        assert!(links.windows(2).all(|pair| pair[0].0 != pair[1].0));
    }

    #[test]
    fn junction_internal_links_are_not_lost_by_cluster_contraction() {
        let skeleton = map(8, 7, &[(2, 3), (3, 3), (4, 3), (5, 3), (3, 2), (4, 4)]);
        let mut links = Vec::new();
        let coverage = visit_links(&skeleton, |a, b, _| {
            links.push([a, b]);
            Ok(())
        })
        .unwrap();
        assert_eq!(coverage.total_links, 5);
        assert_eq!(coverage.total_length_px, 5.0);
        assert!(links.contains(&[3 * 8 + 3, 3 * 8 + 4]));
    }

    #[test]
    fn midpoint_normals_are_axial_and_do_not_fall_back_to_the_digital_link() {
        let mut input = MeasurementInput {
            map: map(2, 1, &[(0, 0), (1, 0)]),
            normal_angles: vec![0.0, PI as f32],
            normal_usable: vec![true; 2],
        };
        assert!(midpoint_normal(&input, 0, 1).unwrap().abs() < 1e-6);
        input.normal_angles[1] = FRAC_PI_2 as f32;
        assert_eq!(midpoint_normal(&input, 0, 1), None);
        input.normal_angles[1] = 0.0;
        input.normal_usable[1] = false;
        assert_eq!(midpoint_normal(&input, 0, 1), None);
    }

    #[test]
    fn confirmation_is_a_local_observed_ribbon_not_a_component_flood() {
        let mut source_mask = vec![true; 16 * 12];
        source_mask[5 * 16 + 7] = false;
        let source = capture(16, 12, Some(source_mask));
        let mut raster = RegionRaster::new(16, 12).unwrap();
        raster.add(&profile()).unwrap();
        let support = raster.finish(&source);
        for y in 0..12 {
            for x in 0..16 {
                let expected = (2..=12).contains(&x) && (3..=7).contains(&y) && (x, y) != (7, 5);
                assert_eq!(support[y * 16 + x], expected, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn rotated_ribbon_row_union_matches_its_continuous_rectangle() {
        let triple = ScaleProfile {
            ridges: std::array::from_fn(|i| RidgeSpan {
                start: point(2.0 + 3.0 * i as f64, 2.0 + 3.0 * i as f64),
                end: point(3.0 + 3.0 * i as f64, 3.0 + 3.0 * i as f64),
            }),
            ridge_mean_gray: [30.0; 3],
            valley_mean_gray: [220.0; 2],
        };
        let source = capture(16, 16, None);
        let mut raster = RegionRaster::new(16, 16).unwrap();
        raster.add(&triple).unwrap();
        // Duplicate observations change neither the union nor its mask.
        raster.add(&triple).unwrap();
        let support = raster.finish(&source);
        for y in 0..16 {
            for x in 0..16 {
                let normal = ((x as f64 - 5.5) + (y as f64 - 5.5)) / SQRT_2;
                let tangent = (-(x as f64 - 5.5) + (y as f64 - 5.5)) / SQRT_2;
                let expected = normal.abs() <= 7.0 * SQRT_2 / 2.0 + 1e-12
                    && tangent.abs() <= 3.0 * SQRT_2 / 2.0 + 1e-12;
                assert_eq!(support[y * 16 + x], expected, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn all_link_length_is_accounted_and_width_mean_uses_arclength_weights() {
        let skeleton = map(20, 5, &[(1, 1), (2, 1), (5, 1), (6, 2), (12, 3), (13, 3)]);
        let coverage = visit_links(&skeleton, |_, _, _| Ok(())).unwrap();
        assert_eq!(coverage.total_links, 3);
        let drafts = vec![
            Draft {
                link: [21, 22],
                span: RidgeSpan {
                    start: point(0.5, 1.0),
                    end: point(2.5, 1.0),
                },
                length: 1.0,
            },
            Draft {
                link: [25, 46],
                span: RidgeSpan {
                    start: point(2.5, 1.5),
                    end: point(8.5, 1.5),
                },
                length: SQRT_2,
            },
            Draft {
                link: [72, 73],
                span: RidgeSpan {
                    start: point(11.5, 3.0),
                    end: point(13.5, 3.0),
                },
                length: 1.0,
            },
        ];
        let mut support = vec![false; 100];
        for i in [21, 22, 25, 46] {
            support[i] = true;
        }
        let full = finish(&capture(20, 5, None), skeleton, coverage, drafts, support).unwrap();
        assert!((full.unit_px - (2.0 + 6.0 * SQRT_2) / (1.0 + SQRT_2)).abs() < 1e-12);
        assert!((full.coverage.total_length_px - (2.0 + SQRT_2)).abs() < 1e-12);
        assert!((full.coverage.measured_length_px - (1.0 + SQRT_2)).abs() < 1e-12);
        assert_eq!(full.coverage.measured_links, 2);
        assert!(full.observations.iter().all(|o| o.mean_gray == 50.0));
        let debug = format!("{full:?}");
        assert!(debug.contains("REDACTED") && !debug.contains("RidgeSpan"));
    }

    #[test]
    fn no_confirmed_region_is_an_explicit_failure_not_a_fallback_unit() {
        let skeleton = map(5, 3, &[(1, 1), (2, 1)]);
        let coverage = visit_links(&skeleton, |_, _, _| Ok(())).unwrap();
        let drafts = vec![Draft {
            link: [6, 7],
            span: RidgeSpan {
                start: point(0.5, 1.0),
                end: point(2.5, 1.0),
            },
            length: 1.0,
        }];
        assert!(matches!(
            finish(
                &capture(5, 3, None),
                skeleton,
                coverage,
                drafts,
                vec![false; 15]
            ),
            Err(FingerprintError::ScaleUnavailable(_))
        ));
    }

    #[test]
    fn bootstrap_profile_budget_does_not_truncate_the_full_ridge_pass() {
        let (width, height) = (96, 64);
        let pixels = (0..width * height)
            .map(|i| if i % width % 12 < 4 { 20 } else { 220 })
            .collect();
        let source = Capture::new(width as u32, height as u32, pixels, None, None).unwrap();
        let config = ExtractionConfig {
            bootstrap_grid_locations: 1,
            ..ExtractionConfig::default()
        };
        let bootstrap = scale::estimate(&source, &config).unwrap();
        let full = measure(&source, &config, &bootstrap).unwrap();
        assert!(full.coverage.total_links > config.bootstrap_grid_locations);
        assert!(full.coverage.measured_links > config.bootstrap_grid_locations);
        let expected = visit_links(&full.skeleton, |_, _, _| Ok(())).unwrap();
        assert_eq!(full.coverage.total_links, expected.total_links);
        assert_eq!(full.coverage.total_length_px, expected.total_length_px);
        assert!(full.coverage.measured_length_px <= full.coverage.total_length_px);
        assert!((3.5..4.5).contains(&full.unit_px));
    }

    #[test]
    fn hidden_mask_values_cannot_change_full_measurement_or_confirm_a_hole() {
        let (width, height) = (120, 64);
        let mut first: Vec<_> = (0..width * height)
            .map(|i| if i % width % 12 < 4 { 20 } else { 220 })
            .collect();
        let mut second = first.clone();
        let mut mask = vec![true; width * height];
        for y in 0..height {
            for x in 56..65 {
                let i = y * width + x;
                mask[i] = false;
                first[i] = 0;
                second[i] = 255;
            }
        }
        let a = Capture::new(width as u32, height as u32, first, Some(mask.clone()), None).unwrap();
        let b = Capture::new(
            width as u32,
            height as u32,
            second,
            Some(mask.clone()),
            None,
        )
        .unwrap();
        let config = ExtractionConfig {
            bootstrap_grid_locations: 16,
            ..ExtractionConfig::default()
        };
        let bootstrap = scale::estimate(&a, &config).unwrap();
        let first = measure(&a, &config, &bootstrap).unwrap();
        let second = measure(&b, &config, &bootstrap).unwrap();
        assert_eq!(first, second);
        for (i, &valid) in mask.iter().enumerate() {
            if !valid {
                assert!(!first.pattern_support[i]);
                assert!(!first.skeleton.ridges()[i]);
            }
        }
        assert!(
            first
                .observations
                .iter()
                .all(|o| mask[o.link[0]] && mask[o.link[1]])
        );
    }

    fn smooth_stripe_capture(angle: f64, phase: f64) -> Capture {
        let (width, height) = (180, 180);
        let (sin, cos) = angle.sin_cos();
        let pixels = (0..width * height)
            .map(|i| {
                let axis = (i % width) as f64 * cos + (i / width) as f64 * sin + phase;
                let distance = ((axis + 6.0).rem_euclid(12.0) - 6.0).abs();
                (120.0 + 100.0 * ((distance - 2.0) / 0.35).tanh())
                    .round()
                    .clamp(0.0, 255.0) as u8
            })
            .collect();
        Capture::new(width as u32, height as u32, pixels, None, None).unwrap()
    }

    fn unequal_rectangle_capture() -> Capture {
        let (width, height) = (200, 240);
        let mut pixels = vec![220; width * height];
        for (centers, thickness, end) in [([30, 50, 70], 4, 180), ([130, 150, 170], 8, 100)] {
            for center in centers {
                for y in 20..end {
                    for x in center - thickness / 2..center + thickness / 2 {
                        pixels[y * width + x] = 20;
                    }
                }
            }
        }
        Capture::new(width as u32, height as u32, pixels, None, None).unwrap()
    }

    fn full_capture(capture: &Capture) -> FullScale {
        let config = ExtractionConfig::default();
        let bootstrap = scale::estimate(capture, &config).unwrap();
        measure(capture, &config, &bootstrap).unwrap()
    }

    fn group_statistics(full: &FullScale, right: bool) -> (f64, f64, f64) {
        let width = full.skeleton.width();
        let belongs = |link: [usize; 2]| {
            ((((link[0] % width) + (link[1] % width)) as f64 / 2.0) >= 100.0) == right
        };
        let mut attempted = 0.0;
        visit_links(&full.skeleton, |a, b, length| {
            if belongs([a, b]) {
                attempted += length;
            }
            Ok(())
        })
        .unwrap();
        let (mut measured, mut weighted) = (0.0, 0.0);
        for observation in &full.observations {
            if belongs(observation.link) {
                measured += observation.represented_length_px;
                weighted += distance(observation.span.start, observation.span.end)
                    * observation.represented_length_px;
            }
        }
        (weighted / measured, attempted, measured)
    }

    #[test]
    fn full_pass_width_is_stable_across_rotations_and_phases() {
        let mut units = Vec::new();
        for angle in [0.0, PI / 5.0, FRAC_PI_2] {
            for phase in [0.25, 1.1] {
                let full = full_capture(&smooth_stripe_capture(angle, phase));
                // The four-pixel dark body is resolved by eight half-pixel
                // profile intervals. Allow 0.1px for sampled grayscale edges
                // and source-normal interpolation, not an identity tolerance.
                assert!(
                    (full.unit_px - 4.0).abs() < 0.1,
                    "angle {angle}, phase {phase}: {}",
                    full.unit_px
                );
                assert!(full.coverage.measured_length_px > 0.0);
                let attempted = visit_links(&full.skeleton, |_, _, _| Ok(())).unwrap();
                assert_eq!(full.coverage.total_links, attempted.total_links);
                assert_eq!(full.coverage.total_length_px, attempted.total_length_px);
                units.push(full.unit_px);
            }
        }
        let low = units.iter().copied().fold(f64::INFINITY, f64::min);
        let high = units.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(high - low < 0.05, "angle/phase spread: {}", high - low);
    }

    #[test]
    fn full_pass_uses_all_lengths_in_unequal_thickness_groups() {
        let full = full_capture(&unequal_rectangle_capture());
        let (thin_width, thin_total, thin_measured) = group_statistics(&full, false);
        let (thick_width, thick_total, thick_measured) = group_statistics(&full, true);
        assert!((thin_width - 4.0).abs() < 0.1);
        assert!((thick_width - 8.0).abs() < 0.1);
        assert_eq!(thin_measured, thin_total);
        assert_eq!(thick_measured, thick_total);
        assert_eq!(
            full.coverage.measured_length_px,
            thin_measured + thick_measured
        );
        // The source rectangles have lengths 160 and 80. Their raster skeleton
        // caps retract farther for thickness 8 than for thickness 4; their actual
        // represented lengths, not nominal rectangles or equal curve votes,
        // define the arc-length mean. Keep this near the intended 2:1 balance.
        assert!((thick_total / thin_total - 0.5).abs() < 0.06);
        let expected = (4.0 * thin_total + 8.0 * thick_total) / (thin_total + thick_total);
        assert!((full.unit_px - expected).abs() < 0.1);
        let nominal = (4.0 * 160.0 + 8.0 * 80.0) / 240.0;
        assert!((full.unit_px - nominal).abs() < 0.15);
        assert!(
            (full.unit_px - 6.0).abs() > 0.5,
            "must not give each whole curve one vote"
        );
        assert!(
            (full.unit_px - 4.0).abs() > 1.0,
            "must not retain the dominant median as the unit"
        );
    }
}
