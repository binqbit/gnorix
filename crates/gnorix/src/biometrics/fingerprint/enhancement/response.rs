//! Frozen directional source response reused by threshold-only stress cases.

use super::{FingerprintError, Normalized, Orientation, RidgeMap, interpolate};

pub(in super::super) struct DirectionalResponse {
    width: usize,
    height: usize,
    margin: Vec<f64>,
    coherence: Vec<f64>,
    observed: Vec<bool>,
}

impl DirectionalResponse {
    pub(in super::super) fn binarize(
        &self,
        margin: f64,
        min_coherence: f64,
    ) -> Result<RidgeMap, FingerprintError> {
        if !margin.is_finite()
            || !min_coherence.is_finite()
            || !(0.0..=1.0).contains(&min_coherence)
        {
            return Err(FingerprintError::InvalidConfig(
                "invalid response thresholds",
            ));
        }
        let usable: Vec<_> = self
            .observed
            .iter()
            .enumerate()
            .map(|(i, &known)| known && self.coherence[i] >= min_coherence)
            .collect();
        let ridges = usable
            .iter()
            .enumerate()
            .map(|(i, &known)| known && self.margin[i] > margin)
            .collect();
        RidgeMap::new(self.width, self.height, ridges, usable)
    }
}

pub(super) fn collect(
    local: &Normalized,
    field: &Orientation,
    width: usize,
    height: usize,
    period: f64,
) -> Result<DirectionalResponse, FingerprintError> {
    let mut response = DirectionalResponse {
        width,
        height,
        margin: vec![0.0; local.values.len()],
        coherence: field.exact_coherence.clone(),
        observed: vec![false; local.values.len()],
    };
    let intervals = period.ceil() as usize;
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if !field.observed[index] {
                continue;
            }
            let (sin, cos) = f64::from(field.angles[index]).sin_cos();
            let mut lines = [0.0; 3];
            let mut complete = true;
            for (line, offset) in [-0.5, 0.0, 0.5].into_iter().enumerate() {
                for sample in 0..=intervals {
                    let along = -period / 2.0 + sample as f64 * period / intervals as f64;
                    let across = offset * period;
                    let sx = x as f64 + along * cos - across * sin;
                    let sy = y as f64 + along * sin + across * cos;
                    let Some(value) = interpolate(local, width, height, sx, sy) else {
                        complete = false;
                        break;
                    };
                    lines[line] += value;
                }
                if !complete {
                    break;
                }
            }
            if complete {
                // The two strict baseline inequalities are equivalent to a
                // positive minimum margin. Normalization encodes a full local
                // dark/light spread as two units; divide by the sample count
                // and two to express perturbations in that original spread.
                response.margin[index] = (-lines[1]).min((lines[0] + lines[2]) / 2.0 - lines[1])
                    / (2.0 * (intervals + 1) as f64);
                response.observed[index] = true;
            }
        }
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn cached_zero_margin_is_identical_to_original_strict_response() {
        let (width, height) = (72, 72);
        let pixels = (0..width * height)
            .map(|i| {
                let x = (i % width) as f64;
                let y = (i / width) as f64;
                (128.0 + 65.0 * (x / 3.0 + y / 7.0).sin() + 35.0 * (x / 13.0 - y / 5.0).cos())
                    .round() as u8
            })
            .collect();
        let capture = Capture::new(width as u32, height as u32, pixels, None, None).unwrap();
        for coherence in [0.0, 0.2, 1.0] {
            let standardized = normalization::standardize(&capture, 6, 0.035).unwrap();
            let tensor = orientation(&standardized, width, height, 6, coherence).unwrap();
            let palette = normalization::estimate_palette(&capture, &standardized, 6).unwrap();
            let local = normalization::apply_palette(&capture, &palette);
            let filtered = denoise_intensity(&local, width, height, 0.6);
            let original = directional_ridges(&filtered, &tensor, width, height, 12.0).unwrap();
            let response = response::collect(&filtered, &tensor, width, height, 12.0).unwrap();
            assert_eq!(response.binarize(0.0, coherence).unwrap(), original);
        }
    }

    #[test]
    fn confidence_changes_reuse_observed_orientation_and_preserve_missing_support() {
        let config = ExtractionConfig::default();
        let pixels = (0..80 * 80)
            .map(|i| (128.0 + 80.0 * ((i % 80) as f64 / 3.0).cos()).round() as u8)
            .collect();
        let mut mask = vec![true; 80 * 80];
        mask[40 * 80 + 40] = false;
        let capture = Capture::new(80, 80, pixels, Some(mask), None).unwrap();
        let prepared = preprocess(&capture, &config, 12.0).unwrap();
        let relaxed = prepared.response.binarize(-0.05, 0.0).unwrap();
        let strict = prepared.response.binarize(0.05, 0.22).unwrap();
        for i in 0..80 * 80 {
            assert!(!strict.ridges[i] || relaxed.ridges[i]);
            assert!(!strict.usable[i] || relaxed.usable[i]);
        }
        assert!(!relaxed.usable[40 * 80 + 40]);
    }
}
