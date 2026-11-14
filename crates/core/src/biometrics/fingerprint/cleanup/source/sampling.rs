//! Bounded source sampling and observed raw-path coordinates.

use super::{Capture, FingerprintError, LocalPalette, Point, SourcePolicy};

const MAX_WORK: usize = 4_000_000;

pub(super) fn section_fractions(
    first: &Path<'_>,
    second: &Path<'_>,
    spacing: f64,
) -> Result<Vec<f64>, FingerprintError> {
    let intervals = steps(first.length().max(second.length()), spacing)?;
    check_work(
        (intervals + 1)
            .checked_add(first.points.len())
            .and_then(|n| n.checked_add(second.points.len())),
    )?;
    let mut fractions: Vec<_> = (0..=intervals)
        .map(|i| i as f64 / intervals as f64)
        .collect();
    for path in [first, second] {
        fractions.extend(
            path.positions
                .iter()
                .map(|&position| position / path.length()),
        );
    }
    fractions.sort_by(f64::total_cmp);
    fractions.dedup();
    Ok(fractions)
}

pub(super) struct Path<'a> {
    points: &'a [Point],
    positions: Vec<f64>,
}

impl<'a> Path<'a> {
    pub(super) fn new(points: &'a [Point]) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        let mut positions = vec![0.0];
        for pair in points.windows(2) {
            let length = distance(pair[0], pair[1]);
            if !length.is_finite() {
                return None;
            }
            positions.push(positions.last()? + length);
        }
        (*positions.last()? > 0.0).then_some(Self { points, positions })
    }

    pub(super) fn length(&self) -> f64 {
        *self.positions.last().unwrap()
    }

    pub(super) fn at(&self, position: f64) -> Point {
        let next = self.positions.partition_point(|&value| value <= position);
        if next == self.points.len() {
            return *self.points.last().unwrap();
        }
        if next == 0 {
            return self.points[0];
        }
        lerp(
            self.points[next - 1],
            self.points[next],
            (position - self.positions[next - 1])
                / (self.positions[next] - self.positions[next - 1]),
        )
    }
}

pub(super) struct Bounds {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Bounds {
    pub(super) fn of(points: &[Point]) -> Self {
        let mut bounds = Self {
            left: f64::INFINITY,
            top: f64::INFINITY,
            right: f64::NEG_INFINITY,
            bottom: f64::NEG_INFINITY,
        };
        for &point in points {
            bounds.include(point);
        }
        bounds
    }
    pub(super) fn include(&mut self, point: Point) {
        self.left = self.left.min(point.x());
        self.right = self.right.max(point.x());
        self.top = self.top.min(point.y());
        self.bottom = self.bottom.max(point.y());
    }
    pub(super) fn expand(&mut self, amount: f64) {
        self.left -= amount;
        self.top -= amount;
        self.right += amount;
        self.bottom += amount;
    }
}

/// The conservative enclosing rectangle includes every interpolation corner,
/// including pixels between sampling stations and outside the sampled centerline.
pub(super) fn observed_area(
    capture: &Capture,
    palette: &LocalPalette,
    area: Bounds,
) -> Result<bool, FingerprintError> {
    let (width, height) = (capture.width() as usize, capture.height() as usize);
    if area.left < 0.0
        || area.top < 0.0
        || area.right > (width - 1) as f64
        || area.bottom > (height - 1) as f64
    {
        return Ok(false);
    }
    let (left, top) = (area.left.floor() as usize, area.top.floor() as usize);
    let (right, bottom) = (
        (area.right.floor() as usize + 1).min(width - 1),
        (area.bottom.floor() as usize + 1).min(height - 1),
    );
    check_work((right - left + 1).checked_mul(bottom - top + 1))?;
    for y in top..=bottom {
        for x in left..=right {
            let index = y * width + x;
            let (dark, light) = (palette.ridge_gray[index], palette.valley_gray[index]);
            if !palette.usable[index]
                || capture.valid_mask().is_some_and(|mask| !mask[index])
                || !dark.is_finite()
                || !light.is_finite()
                || !(0.0..=255.0).contains(&dark)
                || !(0.0..=255.0).contains(&light)
                || dark >= light
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(super) fn validate(
    capture: &Capture,
    palette: &LocalPalette,
    unit: f64,
    points: usize,
) -> Result<(), FingerprintError> {
    if !unit.is_finite() || unit <= 0.0 {
        return Err(FingerprintError::InvalidConfig(
            "source verification needs a finite positive ridge width",
        ));
    }
    let size = capture.pixels().len();
    if palette.usable.len() != size
        || palette.ridge_gray.len() != size
        || palette.valley_gray.len() != size
    {
        return Err(FingerprintError::InvalidConfig(
            "source palette dimensions differ from capture",
        ));
    }
    steps(unit, SourcePolicy::for_unit(unit).sample_spacing_px)?;
    check_work(Some(points))
}

pub(super) fn check_work(work: Option<usize>) -> Result<(), FingerprintError> {
    work.filter(|&work| work <= MAX_WORK)
        .map(|_| ())
        .ok_or(FingerprintError::WorkLimit)
}

pub(super) fn steps(length: f64, spacing: f64) -> Result<usize, FingerprintError> {
    let count = (length / spacing).ceil().max(1.0);
    if !count.is_finite() || count > (MAX_WORK - 1) as f64 {
        return Err(FingerprintError::WorkLimit);
    }
    Ok(count as usize)
}

pub(super) fn distance(a: Point, b: Point) -> f64 {
    (a.x() - b.x()).hypot(a.y() - b.y())
}
pub(super) fn direction(a: Point, b: Point) -> Option<[f64; 2]> {
    let length = distance(a, b);
    (length > 0.0).then_some([(b.x() - a.x()) / length, (b.y() - a.y()) / length])
}
pub(super) fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x() + (b.x() - a.x()) * t, a.y() + (b.y() - a.y()) * t).unwrap()
}
pub(super) fn offset(origin: Point, a: [f64; 2], x: f64, b: [f64; 2], y: f64) -> Point {
    Point::new(
        origin.x() + a[0] * x + b[0] * y,
        origin.y() + a[1] * x + b[1] * y,
    )
    .unwrap()
}
