//! Export native local clarity without changing source geometry or measurements.

use std::{error::Error, path::Path};

use gnorix::biometrics::{
    fingerprint::{ClarityMap, Fingerprint},
    geometry::Point,
};
use serde_json::{Value, json};

pub(super) fn save(root: &Path, result: &Fingerprint) -> Result<(), Box<dyn Error>> {
    let map = &result.clarity;
    for (filename, values) in [
        ("clarity.png", &map.values),
        ("clarity-contrast.png", &map.contrast),
        ("clarity-separation.png", &map.separation),
        ("clarity-repetition.png", &map.repetition),
    ] {
        super::save_values(
            root,
            filename,
            result,
            values.iter().map(|&value| (value * 255.0).round() as u8),
        )?;
    }
    super::save_values(
        root,
        "clarity-support.png",
        result,
        map.supported
            .iter()
            .map(|&valid| if valid { 255 } else { 0 }),
    )
}

pub(super) fn snapshot(map: &ClarityMap) -> Value {
    let supported_pixels = map.supported.iter().filter(|&&valid| valid).count();
    let sum: f64 = map
        .values
        .iter()
        .zip(&map.supported)
        .filter(|(_, valid)| **valid)
        .map(|(&value, _)| f64::from(value))
        .sum();
    let mean = (supported_pixels > 0).then(|| sum / supported_pixels as f64);

    json!({
        "method":"local-ridge-separability-v1",
        "period_px":map.period_px,
        "grid_spacing_px":map.grid_spacing_px,
        "score":"interpolated(contrast * separation * repetition)",
        "segment_sampling":"bilinear-map-line-mean",
        "encoding":"rounded-u8",
        "summary":{
            "supported_pixels":supported_pixels,
            "total_pixels":map.supported.len(),
            "mean_score":mean,
        },
    })
}

/// Open paths have n-1 segments; cycles also include the last-to-first segment.
/// An explicitly repeated cycle seam keeps its final zero-length segment.
pub(super) fn path_scores(map: &ClarityMap, points: &[Point], closed: bool) -> Vec<Option<f64>> {
    let mut scores: Vec<_> = points
        .windows(2)
        .map(|pair| map.segment_mean(pair[0], pair[1]))
        .collect();
    if closed && let (Some(&first), Some(&last)) = (points.first(), points.last()) {
        scores.push(map.segment_mean(last, first));
    }
    scores
}
