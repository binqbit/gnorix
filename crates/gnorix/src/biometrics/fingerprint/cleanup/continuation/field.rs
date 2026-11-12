//! Connected original-source inspection around a terminal path.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

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
    margin: f64,
    along: f64,
    across: f64,
}

#[derive(Clone, Copy)]
struct Pending {
    index: usize,
    support: f64,
}

impl PartialEq for Pending {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.support.total_cmp(&other.support) == Ordering::Equal
    }
}

impl Eq for Pending {}

impl Ord for Pending {
    fn cmp(&self, other: &Self) -> Ordering {
        self.support
            .total_cmp(&other.support)
            .then_with(|| other.index.cmp(&self.index))
    }
}

impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Field {
    /// Raster bounds conservatively contain every inspected escape and crossing.
    pub(super) fn source_bounds(&self) -> [f64; 4] {
        [
            self.left as f64,
            self.top as f64,
            (self.left + self.width - 1) as f64,
            (self.top + self.height - 1) as f64,
        ]
    }

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
                    margin: (2.0 * darkness - 1.0).clamp(0.0, 1.0),
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

    /// Each escape carries the strongest continuously supported source path
    /// reaching it. A path is only as strong as its weakest normalized link.
    /// Boundary crossings are not traversed beyond the inspection region.
    pub(super) fn weighted_escapes(
        &self,
        observed: &[Point],
        cache: &mut footprint::Cache<'_>,
        work: &mut Work,
    ) -> Result<Option<Vec<(Escape, f64)>>, FingerprintError> {
        let count = self.pixels.len();
        work.charge(count as f64 * 8.0)?;
        let mut links = vec![[None; 4]; count];
        let Some(seeds) = self.connected_core(observed, &mut links, cache, work)? else {
            return Ok(None);
        };
        let maximum_pending = count
            .checked_mul(8)
            .and_then(|value| value.checked_add(1))
            .ok_or(FingerprintError::WorkLimit)?;
        let mut strongest = vec![0.0; count];
        let mut settled = vec![false; count];
        let mut pending = BinaryHeap::new();
        let mut escapes = Vec::new();
        // All seeds are fixed before settling any vertex. This preserves the
        // widest-path invariant while avoiding a single weak boundary pixel.
        for index in seeds {
            let support = self.pixels[index].margin;
            if support > strongest[index] {
                strongest[index] = support;
                charge_heap(work, pending.len() + 1)?;
                pending.push(Pending { index, support });
            }
        }

        while !pending.is_empty() {
            charge_heap(work, pending.len())?;
            let Pending { index, support } = pending.pop().expect("nonempty source queue");
            if settled[index] || support < strongest[index] {
                continue;
            }
            settled[index] = true;
            work.charge(8.0)?;
            if index != self.anchor && self.pixels[index].across.abs() > self.unit / 2.0 {
                escapes.push((
                    Escape {
                        point: self.position(index),
                        boundary: false,
                    },
                    support,
                ));
            }
            for next in self.neighbors(index) {
                if settled[next] || !self.in_support(next) || self.pixels[next].margin <= 0.0 {
                    continue;
                }
                let Some(link) = self.cached_link(index, next, &mut links, cache, work)? else {
                    return Ok(None);
                };
                let candidate = support.min(link.margin);
                if candidate <= 0.0 {
                    continue;
                }
                let pixel = &self.pixels[next];
                if pixel.along < self.back_limit
                    || pixel.along > self.unit
                    || pixel.across.abs() > self.unit
                {
                    escapes.push((
                        Escape {
                            point: self.boundary_crossing(index, next),
                            boundary: true,
                        },
                        candidate,
                    ));
                    continue;
                }
                if candidate > strongest[next] {
                    // Every settled pixel relaxes at most eight links. Stale
                    // heap entries therefore remain bounded by eight per pixel.
                    if pending.len() >= maximum_pending {
                        return Err(FingerprintError::WorkLimit);
                    }
                    charge_heap(work, pending.len() + 1)?;
                    strongest[next] = candidate;
                    pending.push(Pending {
                        index: next,
                        support: candidate,
                    });
                }
            }
        }
        Ok(Some(escapes))
    }

    /// Restrict incoming-core seeds to the same continuously dark component
    /// as the tip. Midpoint links are connected even when their graded margin
    /// is zero; a light source gap still separates components.
    fn connected_core(
        &self,
        observed: &[Point],
        links: &mut [[Option<footprint::SourceLink>; 4]],
        cache: &mut footprint::Cache<'_>,
        work: &mut Work,
    ) -> Result<Option<Vec<usize>>, FingerprintError> {
        let core = self.incoming_core(observed, work)?;
        let mut reachable = vec![false; self.pixels.len()];
        let mut queue = VecDeque::from([self.anchor]);
        let mut seeds = vec![self.anchor];
        reachable[self.anchor] = true;
        while let Some(index) = queue.pop_front() {
            work.charge(8.0)?;
            if index != self.anchor && core[index] && self.pixels[index].margin > 0.0 {
                seeds.push(index);
            }
            for next in self.neighbors(index) {
                if reachable[next] || !self.in_support(next) || self.outside(next) {
                    continue;
                }
                let Some(link) = self.cached_link(index, next, links, cache, work)? else {
                    return Ok(None);
                };
                if link.dark {
                    reachable[next] = true;
                    queue.push_back(next);
                }
            }
        }
        Ok(Some(seeds))
    }

    /// Use only the actually observed terminal arc, matching the cap's dark
    /// reference class: R/2..3R/4 behind the tip and a narrow 0.2R half-width.
    fn incoming_core(
        &self,
        observed: &[Point],
        work: &mut Work,
    ) -> Result<Vec<bool>, FingerprintError> {
        let mut core = vec![false; self.pixels.len()];
        let (low, high, radius) = (self.unit / 2.0, 0.75 * self.unit, 0.2 * self.unit);
        let mut traveled = 0.0;
        work.charge(observed.len() as f64)?;
        for pair in observed.windows(2).rev() {
            let (start, end) = (pair[1], pair[0]);
            let delta = [end.x() - start.x(), end.y() - start.y()];
            let length = delta[0].hypot(delta[1]);
            if length <= 0.0 {
                continue;
            }
            let from = (low - traveled).max(0.0);
            let to = (high - traveled).min(length);
            traveled += length;
            if from < to {
                let direction = [delta[0] / length, delta[1] / length];
                let a = [
                    start.x() + direction[0] * from,
                    start.y() + direction[1] * from,
                ];
                let b = [start.x() + direction[0] * to, start.y() + direction[1] * to];
                let left = (a[0].min(b[0]) - radius).floor().max(self.left as f64) as usize;
                let right = (a[0].max(b[0]) + radius)
                    .ceil()
                    .min((self.left + self.width - 1) as f64) as usize;
                let top = (a[1].min(b[1]) - radius).floor().max(self.top as f64) as usize;
                let bottom = (a[1].max(b[1]) + radius)
                    .ceil()
                    .min((self.top + self.height - 1) as f64) as usize;
                if left <= right && top <= bottom {
                    work.charge(((right - left + 1) * (bottom - top + 1)) as f64 * 8.0)?;
                    for y in top..=bottom {
                        for x in left..=right {
                            let offset = [x as f64 - a[0], y as f64 - a[1]];
                            let along = offset[0] * direction[0] + offset[1] * direction[1];
                            let across = -offset[0] * direction[1] + offset[1] * direction[0];
                            if (0.0..=to - from).contains(&along) && across.abs() <= radius {
                                core[(y - self.top) * self.width + x - self.left] = true;
                            }
                        }
                    }
                }
            }
            if traveled >= high {
                break;
            }
        }
        Ok(core)
    }

    fn outside(&self, index: usize) -> bool {
        let pixel = &self.pixels[index];
        pixel.along < self.back_limit || pixel.along > self.unit || pixel.across.abs() > self.unit
    }

    fn cached_link(
        &self,
        from: usize,
        to: usize,
        links: &mut [[Option<footprint::SourceLink>; 4]],
        cache: &mut footprint::Cache<'_>,
        work: &mut Work,
    ) -> Result<Option<footprint::SourceLink>, FingerprintError> {
        let (first, second) = (from.min(to), from.max(to));
        // Four forward grid directions identify each undirected edge exactly.
        let slot = if first / self.width == second / self.width {
            0
        } else {
            (second % self.width + 2) - first % self.width
        };
        work.charge(1.0)?;
        if let Some(link) = links[first][slot] {
            return Ok(Some(link));
        }
        let link = if first % self.width == second % self.width
            || first / self.width == second / self.width
        {
            // On a grid edge signed source and positive reference spread are
            // linear. Endpoint values suffice for connectivity and margin.
            footprint::SourceLink {
                dark: self.pixels[first].dark && self.pixels[second].dark,
                margin: self.pixels[first].margin.min(self.pixels[second].margin),
            }
        } else {
            let Some(link) =
                cache.source_link(self.position(first), self.position(second), work)?
            else {
                return Ok(None);
            };
            link
        };
        links[first][slot] = Some(link);
        Ok(Some(link))
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

fn charge_heap(work: &mut Work, length: usize) -> Result<(), FingerprintError> {
    let levels = usize::BITS - length.leading_zeros();
    work.charge(f64::from(levels) + 1.0)
}

#[cfg(test)]
mod tests {
    use super::super::KnownEnd;
    use super::*;

    fn weighted_fixture(alternate: bool) -> (Capture, LocalPalette, Field) {
        let mut pixels = vec![200; 81];
        pixels[4 * 9 + 4] = 40;
        pixels[5 * 9 + 4] = 100;
        pixels[6 * 9 + 4] = 40;
        if alternate {
            for y in 4..=6 {
                pixels[y * 9 + 5] = 40;
            }
        }
        let capture = Capture::new(9, 9, pixels, None, None).unwrap();
        let palette = LocalPalette {
            radius_px: 2,
            ridge_gray: vec![40.0; 81],
            valley_gray: vec![200.0; 81],
            usable: vec![true; 81],
        };
        let tip = Point::new(4.0, 4.0).unwrap();
        let field = Field::read(
            &capture,
            &palette,
            tip,
            [1.0, 0.0],
            &[Point::new(2.0, 4.0).unwrap(), tip],
            2.0,
            &mut Work::default(),
        )
        .unwrap()
        .unwrap();
        (capture, palette, field)
    }

    #[test]
    fn weighted_escapes_use_the_strongest_route_and_its_weakest_link() {
        let target = Point::new(4.0, 6.0).unwrap();
        for (alternate, expected) in [(false, 0.25), (true, 1.0)] {
            let (capture, palette, field) = weighted_fixture(alternate);
            let escapes = field
                .weighted_escapes(
                    &[],
                    &mut footprint::Cache::new(&capture, &palette),
                    &mut Work::default(),
                )
                .unwrap()
                .unwrap();
            let support = escapes
                .iter()
                .filter(|(escape, _)| !escape.boundary && escape.point == target)
                .map(|(_, support)| *support)
                .fold(0.0_f64, f64::max);
            assert_eq!(support, expected);
        }
    }

    #[test]
    fn weighted_traversal_is_deterministic_and_cannot_bypass_work_limits() {
        let (capture, palette, field) = weighted_fixture(true);
        let describe = |escapes: Vec<(Escape, f64)>| {
            escapes
                .into_iter()
                .map(|(escape, support)| (escape.point, escape.boundary, support))
                .collect::<Vec<_>>()
        };
        let first = field
            .weighted_escapes(
                &[],
                &mut footprint::Cache::new(&capture, &palette),
                &mut Work::default(),
            )
            .unwrap()
            .unwrap();
        let second = field
            .weighted_escapes(
                &[],
                &mut footprint::Cache::new(&capture, &palette),
                &mut Work::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(describe(first), describe(second));
        let mut work = Work {
            estimated: super::super::super::MAX_WORK as f64 - 1.0,
        };
        assert!(matches!(
            field.weighted_escapes(
                &[],
                &mut footprint::Cache::new(&capture, &palette),
                &mut work
            ),
            Err(FingerprintError::WorkLimit),
        ));
    }

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
                margin: 1.0,
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
