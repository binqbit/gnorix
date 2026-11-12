//! Full traversal of a raw, adaptive grayscale measurement skeleton.
//!
//! Bootstrap scale selects bounded processing support, never the final unit.
//! Every unique skeleton link contributes attempted arc length, including links
//! inside junction clusters and the closing links of cycles. Only its own raw
//! ridge span can contribute a measurement; neighboring ridges confirm context.

mod support;

use std::fmt;

use support::RegionRaster;

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

fn distance(a: Point, b: Point) -> f64 {
    (b.x() - a.x()).hypot(b.y() - a.y())
}

#[cfg(test)]
mod tests;
