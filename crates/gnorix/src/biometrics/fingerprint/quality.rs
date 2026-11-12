//! Width consistency around the relevant measured reference thickness.
//!
//! These fractions describe measured width agreement, not identity confidence,
//! anatomical correctness, independent evidence, or a reason to discard ridges.

use super::{BootstrapScale, FingerprintError, FullScale};

const RELATIVE_TOLERANCES: [f64; 3] = [0.1, 0.2, 0.3];

/// The fraction of observed ridge widths within one reference-width band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidthConsistency {
    /// Symmetric relative band: 0.1, 0.2 or 0.3 for ten, twenty or thirty percent.
    pub relative_tolerance: f64,
    /// Measurements inside this band, including its endpoints.
    pub accepted_measurements: usize,
    /// All widths in this measurement population, without further filtering.
    pub total_measurements: usize,
    /// Accepted divided by total; this is not a confidence probability.
    pub fraction: f64,
}

impl BootstrapScale {
    /// Counts confirmed widths in the 10%, 20% and 30% bands around the measured
    /// median, without running later extraction stages or removing observations.
    pub fn width_consistency(&self) -> Result<[WidthConsistency; 3], FingerprintError> {
        let widths = self.profiles.iter().flat_map(|profile| {
            profile.ridges.iter().map(|ridge| {
                (ridge.end.x() - ridge.start.x()).hypot(ridge.end.y() - ridge.start.y())
            })
        });
        summarize_widths(self.ridge_width_px, widths)
    }
}

impl FullScale {
    /// Counts all full-pass own-ridge widths around the arithmetic mean unit.
    /// Fractions count measurements; source-length coverage is reported separately.
    pub fn width_consistency(&self) -> Result<[WidthConsistency; 3], FingerprintError> {
        summarize_widths(
            self.unit_px,
            self.observations.iter().map(|observation| {
                (observation.span.end.x() - observation.span.start.x())
                    .hypot(observation.span.end.y() - observation.span.start.y())
            }),
        )
    }
}

fn summarize_widths(
    reference: f64,
    widths: impl Iterator<Item = f64>,
) -> Result<[WidthConsistency; 3], FingerprintError> {
    if !reference.is_finite() || reference <= 0.0 {
        return Err(FingerprintError::ScaleUnavailable(
            "width consistency requires a finite positive reference thickness",
        ));
    }
    let mut bands = RELATIVE_TOLERANCES.map(|relative_tolerance| WidthConsistency {
        relative_tolerance,
        accepted_measurements: 0,
        total_measurements: 0,
        fraction: 0.0,
    });
    let mut total = 0_usize;

    for width in widths {
        if !width.is_finite() || width <= 0.0 {
            return Err(FingerprintError::ScaleUnavailable(
                "width consistency requires finite positive ridge measurements",
            ));
        }
        total = total.checked_add(1).ok_or(FingerprintError::WorkLimit)?;
        for band in &mut bands {
            if within_band(width, reference, band.relative_tolerance) {
                band.accepted_measurements += 1;
            }
        }
    }
    if total == 0 {
        return Err(FingerprintError::ScaleUnavailable(
            "width consistency requires confirmed ridge measurements",
        ));
    }

    for band in &mut bands {
        band.total_measurements = total;
        band.fraction = band.accepted_measurements as f64 / total as f64;
    }
    Ok(bands)
}

fn within_band(width: f64, median: f64, relative_tolerance: f64) -> bool {
    // A tiny arithmetic allowance keeps decimal boundary values inclusive
    // despite subtraction rounding, e.g. 40.7 around 37 at ten percent.
    // This is floating-point roundoff, not an additional statistical tolerance.
    let roundoff = 8.0 * f64::EPSILON * width.max(median);
    (width - median).abs() <= relative_tolerance * median + roundoff
}

#[cfg(test)]
mod tests {
    use super::super::{RidgeSpan, ScaleProfile};
    use super::*;
    use crate::biometrics::geometry::Point;

    #[test]
    fn every_exported_span_contributes_its_euclidean_source_pixel_width() {
        let profiles = [[10.0, 10.0, 10.0], [10.0, 10.0, 40.0]]
            .into_iter()
            .enumerate()
            .map(|(index, widths)| {
                let origin = index as f64 * 100.0;
                let mut cursor = 0.0;
                ScaleProfile {
                    ridges: widths.map(|width| {
                        let start =
                            Point::new(origin + 0.6 * cursor, origin + 0.8 * cursor).unwrap();
                        cursor += width;
                        let end = Point::new(origin + 0.6 * cursor, origin + 0.8 * cursor).unwrap();
                        cursor += 5.0;
                        RidgeSpan { start, end }
                    }),
                    ridge_mean_gray: [64.0; 3],
                    valley_mean_gray: [192.0; 2],
                }
            })
            .collect();
        let scale = BootstrapScale {
            ridge_width_px: 10.0,
            valley_width_px: 5.0,
            ridge_period_px: 15.0,
            ridge_width_mad_px: 0.0,
            ridge_period_mad_px: 0.0,
            mean_ridge_width_px: 15.0,
            mean_valley_width_px: 5.0,
            mean_ridge_gray: 64.0,
            mean_valley_gray: 192.0,
            examined_locations: 2,
            profiles,
        };
        let bands = scale.width_consistency().unwrap();
        assert!(bands.iter().all(|band| {
            band.total_measurements == 6
                && band.accepted_measurements == 5
                && band.fraction == 5.0 / 6.0
        }));

        let mut empty = scale;
        empty.profiles.clear();
        assert!(matches!(
            empty.width_consistency(),
            Err(FingerprintError::ScaleUnavailable(_))
        ));
    }

    #[test]
    fn nested_bands_include_both_threshold_endpoints() {
        let widths = [7.0, 8.0, 9.0, 10.0, 10.0, 10.0, 11.0, 12.0, 13.0];
        let bands = summarize_widths(10.0, widths.into_iter()).unwrap();
        for (band, accepted) in bands.iter().zip([5, 7, 9]) {
            assert_eq!(band.accepted_measurements, accepted);
            assert_eq!(band.total_measurements, 9);
            assert_eq!(band.fraction, accepted as f64 / 9.0);
        }
        assert_eq!(bands.map(|band| band.relative_tolerance), [0.1, 0.2, 0.3]);
    }

    #[test]
    fn thick_artifacts_remain_in_the_denominator() {
        let widths = [10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 40.0, 40.0, 40.0];
        let bands = summarize_widths(10.0, widths.into_iter()).unwrap();
        assert!(bands.iter().all(|band| {
            band.accepted_measurements == 6
                && band.total_measurements == 9
                && band.fraction == 2.0 / 3.0
        }));
    }

    #[test]
    fn separated_groups_are_compared_with_the_precomputed_global_median() {
        // The median of all six widths is twenty. Neither group is selected or
        // discarded, and the median is not recalculated after band membership.
        let widths = [10.0, 10.0, 10.0, 30.0, 30.0, 30.0];
        let bands = summarize_widths(20.0, widths.into_iter()).unwrap();
        assert!(bands.iter().all(|band| {
            band.accepted_measurements == 0 && band.total_measurements == 6 && band.fraction == 0.0
        }));
    }

    #[test]
    fn decimal_boundary_roundoff_does_not_widen_the_band_materially() {
        let widths = [33.3, 40.7, 40.70001];
        let bands = summarize_widths(37.0, widths.into_iter()).unwrap();
        assert_eq!(bands[0].accepted_measurements, 2);
        assert_eq!(bands[0].total_measurements, 3);
        assert_eq!(bands[1].accepted_measurements, 3);
    }

    #[test]
    fn missing_or_invalid_evidence_is_an_error_not_zero_quality() {
        assert!(matches!(
            summarize_widths(10.0, std::iter::empty()),
            Err(FingerprintError::ScaleUnavailable(_))
        ));
        for median in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                summarize_widths(median, [10.0].into_iter()),
                Err(FingerprintError::ScaleUnavailable(_))
            ));
        }
        for width in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                summarize_widths(10.0, [width].into_iter()),
                Err(FingerprintError::ScaleUnavailable(_))
            ));
        }
    }
}
