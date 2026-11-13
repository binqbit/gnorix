//! Compact source measurements consumed by enrollment and signing.

mod projection;

use std::{collections::BTreeSet, fmt};

use super::{
    FeatureKind, FeatureStrength, Fingerprint, FingerprintError, FullScale, Point, RegionBounds,
};
pub(super) use projection::Projection;

/// A type-preserved original candidate surviving cleanup and reconstruction.
/// A missing priority is unknown evidence, not measured zero.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalyzedFeature {
    pub id: usize,
    pub kind: FeatureKind,
    pub position: Point,
    pub priority: Option<f64>,
}

/// Sensitive per-capture analysis without raster or diagnostic graph retention.
/// Distances use the measured mean dark-ridge width; bounds cover confirmed
/// pattern pixels only. Subset selection may extend them to eligible points.
#[derive(Clone, PartialEq)]
pub struct FingerprintFeatures {
    pub width: usize,
    pub height: usize,
    pub unit_px: f64,
    pub pattern_bounds: Option<RegionBounds>,
    pub features: Vec<AnalyzedFeature>,
}

impl fmt::Debug for FingerprintFeatures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FingerprintFeatures")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("biometrics", &"[REDACTED]")
            .finish()
    }
}

impl FingerprintFeatures {
    pub fn new(
        width: usize,
        height: usize,
        unit_px: f64,
        pattern_bounds: Option<RegionBounds>,
        features: Vec<AnalyzedFeature>,
    ) -> Result<Self, FingerprintError> {
        let result = Self {
            width,
            height,
            unit_px,
            pattern_bounds,
            features,
        };
        result.validate()?;
        Ok(result)
    }

    /// Revalidates public fields before they are consumed after modification.
    pub fn validate(&self) -> Result<(), FingerprintError> {
        if self
            .width
            .checked_mul(self.height)
            .is_none_or(|size| size == 0)
        {
            return Err(FingerprintError::InvalidMap(
                "analysis dimensions must be nonzero and addressable",
            ));
        }
        if !self.unit_px.is_finite() || self.unit_px <= 0.0 {
            return Err(FingerprintError::InvalidConfig(
                "analysis ridge width must be finite and positive",
            ));
        }
        if let Some(bounds) = self.pattern_bounds
            && (bounds.min.x() < -0.5
                || bounds.min.y() < -0.5
                || bounds.max.x() > self.width as f64 - 0.5
                || bounds.max.y() > self.height as f64 - 0.5
                || bounds.min.x() >= bounds.max.x()
                || bounds.min.y() >= bounds.max.y())
        {
            return Err(FingerprintError::InvalidMap(
                "confirmed pattern bounds must lie inside the source frame",
            ));
        }
        let mut ids = BTreeSet::new();
        for feature in &self.features {
            if !ids.insert(feature.id) {
                return Err(FingerprintError::InvalidTopology(
                    "analysis repeats an original feature ID",
                ));
            }
            if feature.position.x() < 0.0
                || feature.position.y() < 0.0
                || feature.position.x() > (self.width - 1) as f64
                || feature.position.y() > (self.height - 1) as f64
            {
                return Err(FingerprintError::InvalidTopology(
                    "analyzed feature lies outside the source frame",
                ));
            }
            if feature
                .priority
                .is_some_and(|priority| !priority.is_finite() || !(0.0..=1.0).contains(&priority))
            {
                return Err(FingerprintError::InvalidTopology(
                    "feature priority must be unknown or in [0, 1]",
                ));
            }
        }
        Ok(())
    }
}

impl Fingerprint {
    /// Projects full report diagnostics into the same compact signing input.
    pub fn features(&self) -> Result<FingerprintFeatures, FingerprintError> {
        let contours = &self.contours;
        summarize(
            (self.binary.width(), self.binary.height()),
            &self.scale,
            &self.feature_strength.candidates,
            Projection::new(
                &self.graph,
                &contours.cleanup,
                &contours.graph,
                &contours.node_sources,
                &contours.cycle_origins,
            ),
        )
    }
}

pub(super) fn summarize(
    dimensions: (usize, usize),
    scale: &FullScale,
    candidates: &[FeatureStrength],
    projection: Projection<'_>,
) -> Result<FingerprintFeatures, FingerprintError> {
    let (width, height) = dimensions;
    let pattern_bounds = confirmed_bounds(dimensions, scale)?;
    let features = projection::project(candidates, projection)?
        .into_iter()
        .map(|feature| AnalyzedFeature {
            id: feature.id,
            kind: feature.kind,
            position: feature.position,
            priority: feature.priority,
        })
        .collect();
    FingerprintFeatures::new(width, height, scale.unit_px, pattern_bounds, features)
}

fn confirmed_bounds(
    dimensions: (usize, usize),
    scale: &FullScale,
) -> Result<Option<RegionBounds>, FingerprintError> {
    let (width, height) = dimensions;
    if width.checked_mul(height) != Some(scale.pattern_support.len())
        || scale.skeleton.width() != width
        || scale.skeleton.height() != height
    {
        return Err(FingerprintError::InvalidMap(
            "pattern support dimensions differ from measurement skeleton",
        ));
    }
    Ok(support_bounds(&scale.pattern_support, width))
}

fn support_bounds(support: &[bool], width: usize) -> Option<RegionBounds> {
    let mut extent: Option<(usize, usize, usize, usize)> = None;
    for (index, &supported) in support.iter().enumerate() {
        if supported {
            let (x, y) = (index % width, index / width);
            extent = Some(match extent {
                None => (x, y, x, y),
                Some((left, top, right, bottom)) => {
                    (left.min(x), top.min(y), right.max(x), bottom.max(y))
                }
            });
        }
    }
    extent.map(|(left, top, right, bottom)| RegionBounds {
        min: Point::new(left as f64 - 0.5, top as f64 - 0.5).expect("finite source bounds"),
        max: Point::new(right as f64 + 0.5, bottom as f64 + 0.5).expect("finite source bounds"),
    })
}

#[cfg(test)]
mod tests {
    use super::super::{
        Capture, ExtractionConfig, MeasurementCoverage, RidgeMap, analyze, extract,
    };
    use super::*;

    fn bounds(left: f64, top: f64, right: f64, bottom: f64) -> RegionBounds {
        RegionBounds {
            min: Point::new(left, top).unwrap(),
            max: Point::new(right, bottom).unwrap(),
        }
    }

    #[test]
    fn confirmed_support_uses_pixel_edges_and_keeps_absence_explicit() {
        let mut support = vec![false; 100];
        assert_eq!(support_bounds(&support, 10), None);
        support[22] = true;
        support[77] = true;
        assert_eq!(
            support_bounds(&support, 10),
            Some(bounds(1.5, 1.5, 7.5, 7.5))
        );
        support[0] = true;
        support[99] = true;
        assert_eq!(
            support_bounds(&support, 10),
            Some(bounds(-0.5, -0.5, 9.5, 9.5))
        );
    }

    #[test]
    fn rejects_mismatched_pattern_and_skeleton_dimensions() {
        let mut scale = FullScale {
            unit_px: 2.0,
            coverage: MeasurementCoverage {
                total_length_px: 0.0,
                measured_length_px: 0.0,
                total_links: 0,
                measured_links: 0,
                isolated_pixels: 0,
            },
            observations: vec![],
            skeleton: RidgeMap::new(10, 10, vec![false; 100], vec![true; 100]).unwrap(),
            pattern_support: vec![false; 100],
        };
        assert_eq!(confirmed_bounds((10, 10), &scale).unwrap(), None);
        assert!(confirmed_bounds((5, 20), &scale).is_err());
        assert!(confirmed_bounds((usize::MAX, 2), &scale).is_err());
        scale.pattern_support.pop();
        assert!(confirmed_bounds((10, 10), &scale).is_err());
    }

    #[test]
    fn compact_validation_preserves_unknown_scores_and_rejects_invalid_inputs() {
        let feature = AnalyzedFeature {
            id: 1,
            kind: FeatureKind::Ending,
            position: Point::new(4.0, 4.0).unwrap(),
            priority: None,
        };
        let valid = FingerprintFeatures::new(10, 10, 2.0, None, vec![feature.clone()]).unwrap();
        assert_eq!(valid.features[0].priority, None);
        for unit in [0.0, -1.0, f64::INFINITY, f64::NAN] {
            assert!(FingerprintFeatures::new(10, 10, unit, None, vec![]).is_err());
        }
        assert!(FingerprintFeatures::new(0, 10, 2.0, None, vec![]).is_err());
        assert!(FingerprintFeatures::new(usize::MAX, 2, 2.0, None, vec![]).is_err());
        assert!(
            FingerprintFeatures::new(10, 10, 2.0, Some(bounds(-0.5, -0.5, 10.5, 9.5)), vec![])
                .is_err()
        );
        assert!(
            FingerprintFeatures::new(10, 10, 2.0, None, vec![feature.clone(), feature.clone()])
                .is_err()
        );
        let mut mutated = valid.clone();
        mutated.features[0].priority = Some(f64::NAN);
        assert!(mutated.validate().is_err());
        mutated.features[0].priority = Some(0.5);
        mutated.features[0].position = Point::new(10.0, 0.0).unwrap();
        assert!(mutated.validate().is_err());
        assert!(!format!("{valid:?}").contains("position"));
    }

    #[test]
    fn compact_analysis_matches_report_projection_and_preserves_mask_independence() {
        let size = 80;
        let pixels: Vec<_> = (0..size * size)
            .map(|index| if index % size % 12 < 6 { 25 } else { 225 })
            .collect();
        let mask: Vec<_> = (0..size * size)
            .map(|index| !(34..40).contains(&(index % size)))
            .collect();
        let mut hidden = pixels.clone();
        for (pixel, valid) in hidden.iter_mut().zip(&mask) {
            if !valid {
                *pixel = 17;
            }
        }
        let capture =
            Capture::new(size as u32, size as u32, pixels, Some(mask.clone()), None).unwrap();
        let changed = Capture::new(size as u32, size as u32, hidden, Some(mask), None).unwrap();
        let config = ExtractionConfig::default();
        let report = extract(&capture, &config).unwrap();
        let compact = analyze(&capture, &config).unwrap();
        assert_eq!(compact, report.features().unwrap());
        assert_eq!(compact, analyze(&changed, &config).unwrap());
        assert_eq!(compact.unit_px, report.scale.unit_px);
    }
}
