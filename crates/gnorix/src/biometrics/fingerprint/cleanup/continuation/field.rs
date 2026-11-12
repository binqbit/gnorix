//! Connected original-source inspection around a terminal path.

use std::collections::VecDeque;

use super::{
    Backbone, Capture, Escape, FingerprintError, LocalPalette, Point, SourceDecision, Work,
    footprint, sample_darkness,
};

const MAX_PIXELS: usize = 4_000_000;

pub(super) struct Field {
    left: usize,
    top: usize,
    width: usize,
    height: usize,
    pub(super) unit: f64,
    back_limit: f64,
    pub(super) tip: Point,
    tangent: [f64; 2],
    anchor: usize,
    pixels: Vec<Pixel>,
}

struct Pixel {
    dark: bool,
    along: f64,
    across: f64,
}

impl Field {
    pub(super) fn read(
        capture: &Capture,
        palette: &LocalPalette,
        tip: Point,
        tangent: [f64; 2],
        observed: &[Point],
        unit: f64,
        work: &mut Work,
    ) -> Result<Option<Self>, FingerprintError> {
        let normal = [-tangent[1], tangent[0]];
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        // One raster diagonal beyond an inspection boundary is enough to
        // observe the first crossing, even for a subpixel measured unit.
        let margin = std::f64::consts::SQRT_2;
        let back_limit = observed
            .iter()
            .map(|p| (p.x() - tip.x()) * tangent[0] + (p.y() - tip.y()) * tangent[1])
            .fold(-unit / 2.0, f64::min);
        let across_extent = unit + margin;
        for along in [back_limit - margin, unit + margin] {
            for across in [-across_extent, across_extent] {
                let x = tip.x() + along * tangent[0] + across * normal[0];
                let y = tip.y() + along * tangent[1] + across * normal[1];
                bounds[0] = bounds[0].min(x);
                bounds[1] = bounds[1].min(y);
                bounds[2] = bounds[2].max(x);
                bounds[3] = bounds[3].max(y);
            }
        }
        if bounds[0] < 0.0
            || bounds[1] < 0.0
            || bounds[2] > f64::from(capture.width() - 1)
            || bounds[3] > f64::from(capture.height() - 1)
        {
            return Ok(None);
        }
        let (left, top, right, bottom) = (
            bounds[0].floor() as usize,
            bounds[1].floor() as usize,
            bounds[2].ceil() as usize,
            bounds[3].ceil() as usize,
        );
        let (width, height) = (right - left + 1, bottom - top + 1);
        let count = width
            .checked_mul(height)
            .filter(|&count| count <= MAX_PIXELS)
            .ok_or(FingerprintError::WorkLimit)?;
        work.charge(count as f64 * 24.0)?;
        let mut pixels = Vec::with_capacity(count);
        for y in top..=bottom {
            for x in left..=right {
                let point = Point::new(x as f64, y as f64).map_err(|_| {
                    FingerprintError::InvalidMap("continuation coordinate overflow")
                })?;
                let Some(darkness) = sample_darkness(capture, palette, point) else {
                    return Ok(None);
                };
                let delta = [point.x() - tip.x(), point.y() - tip.y()];
                pixels.push(Pixel {
                    dark: darkness >= 0.5,
                    along: delta[0] * tangent[0] + delta[1] * tangent[1],
                    across: delta[0] * normal[0] + delta[1] * normal[1],
                });
            }
        }
        Ok(Some(Self {
            left,
            top,
            width,
            height,
            unit,
            back_limit,
            tip,
            tangent,
            anchor: (tip.y() as usize - top) * width + tip.x() as usize - left,
            pixels,
        }))
    }

    pub(super) fn contained_ends<'a>(
        &self,
        backbone: &Backbone<'a>,
        component: usize,
        work: &mut Work,
    ) -> Result<Vec<&'a [Point]>, FingerprintError> {
        let mut known = Vec::new();
        work.charge(backbone.known_ends.len() as f64)?;
        for path in &backbone.known_ends {
            if path.component != component {
                continue;
            }
            work.charge(path.points.len() as f64)?;
            if path.points.iter().all(|point| {
                let dx = point.x() - self.tip.x();
                let dy = point.y() - self.tip.y();
                let along = dx * self.tangent[0] + dy * self.tangent[1];
                let across = -dx * self.tangent[1] + dy * self.tangent[0];
                along >= self.back_limit && along <= self.unit && across.abs() <= self.unit
            }) {
                known.push(path.points);
            }
        }
        Ok(known)
    }

    fn in_support(&self, index: usize) -> bool {
        let pixel = &self.pixels[index];
        pixel.dark
            && pixel.along >= self.back_limit - std::f64::consts::SQRT_2
            && pixel.along <= self.unit + std::f64::consts::SQRT_2
            && pixel.across.abs() <= self.unit + std::f64::consts::SQRT_2
    }

    pub(super) fn escapes(
        &self,
        capture: &Capture,
        palette: &LocalPalette,
        work: &mut Work,
    ) -> Result<Option<Vec<Escape>>, FingerprintError> {
        let mut escapes = Vec::new();
        let mut visited = vec![false; self.pixels.len()];
        let mut queue = VecDeque::new();
        visited[self.anchor] = true;
        queue.push_back(self.anchor);
        while let Some(index) = queue.pop_front() {
            for next in self.neighbors(index) {
                if visited[next] || !self.in_support(next) {
                    continue;
                }
                if index % self.width != next % self.width
                    && index / self.width != next / self.width
                {
                    match footprint::dark_segment(
                        capture,
                        palette,
                        self.position(index),
                        self.position(next),
                        work,
                    )? {
                        SourceDecision::Confirmed => {}
                        SourceDecision::MissingSupport => return Ok(None),
                        _ => continue,
                    }
                }
                let pixel = &self.pixels[next];
                if pixel.along < self.back_limit
                    || pixel.along > self.unit
                    || pixel.across.abs() > self.unit
                {
                    escapes.push(Escape {
                        point: self.boundary_crossing(index, next),
                        boundary: true,
                    });
                    continue;
                }
                if pixel.across.abs() > self.unit / 2.0 {
                    escapes.push(Escape {
                        point: self.position(next),
                        boundary: false,
                    });
                }
                visited[next] = true;
                queue.push_back(next);
            }
        }
        Ok(Some(escapes))
    }

    fn boundary_crossing(&self, from: usize, to: usize) -> Point {
        let start = &self.pixels[from];
        let end = &self.pixels[to];
        let mut fraction = 1.0_f64;
        for (one, two, low, high) in [
            (start.along, end.along, self.back_limit, self.unit),
            (start.across, end.across, -self.unit, self.unit),
        ] {
            if two < low {
                fraction = fraction.min((one - low) / (one - two));
            }
            if two > high {
                fraction = fraction.min((high - one) / (two - one));
            }
        }
        let start = self.position(from);
        let end = self.position(to);
        Point::new(
            start.x() + (end.x() - start.x()) * fraction,
            start.y() + (end.y() - start.y()) * fraction,
        )
        .expect("bounded source crossing")
    }

    fn position(&self, index: usize) -> Point {
        Point::new(
            (self.left + index % self.width) as f64,
            (self.top + index / self.width) as f64,
        )
        .expect("bounded raster coordinate")
    }

    fn neighbors(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let (x, y) = (index % self.width, index / self.width);
        [
            (-1_isize, -1_isize),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ]
        .into_iter()
        .filter_map(move |(dx, dy)| {
            let (x, y) = (x.checked_add_signed(dx)?, y.checked_add_signed(dy)?);
            (x < self.width && y < self.height).then_some(y * self.width + x)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::KnownEnd;
    use super::*;

    #[test]
    fn unrelated_known_paths_cannot_bypass_the_shared_work_budget() {
        let point = Point::new(0.0, 0.0).unwrap();
        let path = [point, Point::new(1.0, 0.0).unwrap()];
        let backbone = Backbone {
            paths: Vec::new(),
            known_ends: (1..=3)
                .map(|component| KnownEnd {
                    points: &path,
                    component,
                })
                .collect(),
        };
        let field = Field {
            left: 0,
            top: 0,
            width: 1,
            height: 1,
            unit: 1.0,
            back_limit: -0.5,
            tip: point,
            tangent: [1.0, 0.0],
            anchor: 0,
            pixels: vec![Pixel {
                dark: true,
                along: 0.0,
                across: 0.0,
            }],
        };
        let mut work = Work {
            estimated: super::super::super::MAX_WORK as f64 - 2.0,
        };
        assert!(matches!(
            field.contained_ends(&backbone, 0, &mut work),
            Err(FingerprintError::WorkLimit)
        ));
    }
}
