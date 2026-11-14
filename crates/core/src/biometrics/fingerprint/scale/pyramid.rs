//! Source image pyramid for bootstrap orientation and profile search.

use super::Capture;

pub(super) struct Image {
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) factor: usize,
    pub(super) values: Vec<f64>,
    pub(super) valid: Vec<bool>,
}

impl Image {
    fn downsample(&self) -> Option<Self> {
        let (width, height) = (self.width / 2, self.height / 2);
        if width < 3 || height < 3 {
            return None;
        }
        let mut image = Self {
            width,
            height,
            factor: self.factor * 2,
            values: vec![0.0; width * height],
            valid: vec![false; width * height],
        };
        for y in 0..height {
            for x in 0..width {
                let indices = [
                    2 * y * self.width + 2 * x,
                    2 * y * self.width + 2 * x + 1,
                    (2 * y + 1) * self.width + 2 * x,
                    (2 * y + 1) * self.width + 2 * x + 1,
                ];
                if indices.iter().all(|&i| self.valid[i]) {
                    image.values[y * width + x] =
                        indices.iter().map(|&i| self.values[i]).sum::<f64>() / 4.0;
                    image.valid[y * width + x] = true;
                }
            }
        }
        Some(image)
    }

    pub(super) fn sample(&self, x: f64, y: f64) -> Option<f64> {
        if x < 0.0 || y < 0.0 || x > (self.width - 1) as f64 || y > (self.height - 1) as f64 {
            return None;
        }
        let (left, top) = (x.floor() as usize, y.floor() as usize);
        let (right, bottom) = (
            (left + 1).min(self.width - 1),
            (top + 1).min(self.height - 1),
        );
        let indices = [
            top * self.width + left,
            top * self.width + right,
            bottom * self.width + left,
            bottom * self.width + right,
        ];
        if indices.iter().any(|&i| !self.valid[i]) {
            return None;
        }
        let (dx, dy) = (x - left as f64, y - top as f64);
        Some(
            (self.values[indices[0]] * (1.0 - dx) + self.values[indices[1]] * dx) * (1.0 - dy)
                + (self.values[indices[2]] * (1.0 - dx) + self.values[indices[3]] * dx) * dy,
        )
    }
}

pub(super) fn build(capture: &Capture) -> Vec<Image> {
    let source = Image {
        width: capture.width() as usize,
        height: capture.height() as usize,
        factor: 1,
        values: capture
            .pixels()
            .iter()
            .map(|&v| f64::from(v) / 255.0)
            .collect(),
        valid: capture
            .valid_mask()
            .map_or_else(|| vec![true; capture.pixels().len()], <[bool]>::to_vec),
    };
    let mut pyramid = vec![source];
    for _ in 0..4 {
        let Some(next) = pyramid.last().unwrap().downsample() else {
            break;
        };
        pyramid.push(next);
    }
    pyramid
}
