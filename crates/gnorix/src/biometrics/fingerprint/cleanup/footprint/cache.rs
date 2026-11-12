//! Reuse validated interpolation cells within one diagnostic source field.

use std::collections::HashMap;

use super::{
    Capture, CellSource, FingerprintError, LocalPalette, Point, SourceDecision, SourceLink, Work,
};

// Bounded auxiliary memory; eviction changes only repeated work, never evidence.
const MAX_CELLS: usize = 65_536;
const MAX_SEGMENTS: usize = 65_536;

pub(in super::super) struct Cache<'a> {
    capture: &'a Capture,
    palette: &'a LocalPalette,
    cells: HashMap<(usize, usize), Option<CellSource>>,
    segments: HashMap<[u64; 4], SourceDecision>,
}

impl<'a> Cache<'a> {
    pub(in super::super) fn new(capture: &'a Capture, palette: &'a LocalPalette) -> Self {
        Self {
            capture,
            palette,
            cells: HashMap::new(),
            segments: HashMap::new(),
        }
    }

    pub(in super::super) fn dark_segment(
        &mut self,
        start: Point,
        end: Point,
        work: &mut Work,
    ) -> Result<SourceDecision, FingerprintError> {
        if !super::segment_in_capture(self.capture, start, end) {
            return Ok(SourceDecision::MissingSupport);
        }
        // Keep endpoint order and exact float bits: no quantized cache key or
        // reversed arithmetic may change a near-midpoint source decision.
        let key = [
            start.x().to_bits(),
            start.y().to_bits(),
            end.x().to_bits(),
            end.y().to_bits(),
        ];
        work.charge(4.0)?;
        if let Some(&result) = self.segments.get(&key) {
            return Ok(result);
        }
        // Traversal and quadratic evaluation remain real work even on a hit.
        // Validate/load each cell once, instead of repeating its four palette
        // and capture-corner checks for every nearby backbone projection.
        let visits = (end.x() - start.x()).abs().ceil() + (end.y() - start.y()).abs().ceil() + 4.0;
        work.charge(visits * 8.0)?;
        let mut failure = None;
        let result = super::classify_segment(self.capture, self.palette, start, end, |cell| {
            match self.read(cell, work) {
                Ok(value) => value,
                Err(error) => {
                    failure = Some(error);
                    None
                }
            }
        });
        if let Some(error) = failure {
            return Err(error);
        }
        if self.segments.len() == MAX_SEGMENTS {
            self.segments.clear();
        }
        self.segments.insert(key, result);
        Ok(result)
    }

    pub(in super::super) fn source_link(
        &mut self,
        start: Point,
        end: Point,
        work: &mut Work,
    ) -> Result<Option<SourceLink>, FingerprintError> {
        if !super::segment_in_capture(self.capture, start, end) {
            return Ok(None);
        }
        let visits = (end.x() - start.x()).abs().ceil() + (end.y() - start.y()).abs().ceil() + 4.0;
        work.charge(visits * 8.0)?;
        let mut failure = None;
        let result =
            super::classify_link(self.capture, self.palette, start, end, |cell| {
                match self.read(cell, work) {
                    Ok(value) => value,
                    Err(error) => {
                        failure = Some(error);
                        None
                    }
                }
            });
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(result)
    }

    fn read(
        &mut self,
        cell: super::raster::CellInterval,
        work: &mut Work,
    ) -> Result<Option<CellSource>, FingerprintError> {
        let key = (cell.x, cell.y);
        let cached = self.cells.get(&key).copied();
        work.charge(if cached.is_some() { 4.0 } else { 28.0 })?;
        if let Some(value) = cached {
            return Ok(value);
        }
        let value = super::cell_source(self.capture, self.palette, cell);
        if self.cells.len() == MAX_CELLS {
            self.cells.clear();
        }
        self.cells.insert(key, value);
        Ok(value)
    }

    pub(in super::super) fn normal_span(
        &mut self,
        center: Point,
        tangent: [f64; 2],
        escaped: Point,
        unit: f64,
        work: &mut Work,
    ) -> Result<SourceDecision, FingerprintError> {
        super::normal_span_with(
            self.capture,
            self.palette,
            center,
            tangent,
            escaped,
            unit,
            work,
            &mut |a, b, work| self.dark_segment(a, b, work),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y).unwrap()
    }

    fn palette(size: usize) -> LocalPalette {
        LocalPalette {
            radius_px: 4,
            ridge_gray: vec![40.0; size],
            valley_gray: vec![200.0; size],
            usable: vec![true; size],
        }
    }

    #[test]
    fn cached_proofs_preserve_saddles_corner_ties_masks_and_point_queries() {
        let palette = palette(9);
        for hidden in [false, true] {
            let capture = Capture::new(
                3,
                3,
                vec![40, 200, 100, 150, 40, 160, 40, 200, 40],
                hidden.then(|| vec![true, true, true, true, true, true, false, true, true]),
                None,
            )
            .unwrap();
            let mut cache = Cache::new(&capture, &palette);
            for (a, b) in [
                (point(0.0, 0.0), point(1.0, 1.0)),
                (point(0.0, 0.0), point(1.4999999999999998, 1.5)),
                (point(0.25, 0.5), point(1.75, 1.25)),
                (point(1.0, 1.0), point(1.0, 1.0)),
            ] {
                for (start, end) in [(a, b), (b, a)] {
                    let expected = super::super::dark_segment(
                        &capture,
                        &palette,
                        start,
                        end,
                        &mut Work::default(),
                    )
                    .unwrap();
                    let expected_link = super::super::source_link(
                        &capture,
                        &palette,
                        start,
                        end,
                        &mut Work::default(),
                    )
                    .unwrap();
                    for _ in 0..2 {
                        assert_eq!(
                            cache
                                .dark_segment(start, end, &mut Work::default())
                                .unwrap(),
                            expected
                        );
                        assert_eq!(
                            cache.source_link(start, end, &mut Work::default()).unwrap(),
                            expected_link
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cached_normal_proofs_reuse_cells_and_respect_the_work_limit() {
        let capture = Capture::new(
            64,
            64,
            (0..4096)
                .map(|i| {
                    if (26..=38).contains(&(i / 64)) {
                        40
                    } else {
                        200
                    }
                })
                .collect(),
            None,
            None,
        )
        .unwrap();
        let palette = palette(4096);
        let mut cache = Cache::new(&capture, &palette);
        let (center, end) = (point(32.0, 32.0), point(32.0, 37.0));
        let mut cold = Work::default();
        let expected = super::super::normal_span(
            &capture,
            &palette,
            center,
            [1.0, 0.0],
            end,
            8.0,
            &mut Work::default(),
        )
        .unwrap();
        assert_eq!(
            cache
                .normal_span(center, [1.0, 0.0], end, 8.0, &mut cold)
                .unwrap(),
            expected
        );
        let mut warm = Work::default();
        assert_eq!(
            cache
                .normal_span(center, [1.0, 0.0], end, 8.0, &mut warm)
                .unwrap(),
            expected
        );
        assert!(warm.estimated < cold.estimated);
        let mut exhausted = Work {
            estimated: super::super::super::MAX_WORK as f64 - 1.0,
        };
        assert!(matches!(
            cache.dark_segment(center, end, &mut exhausted),
            Err(FingerprintError::WorkLimit)
        ));
    }

    #[test]
    fn cell_load_exhaustion_is_an_error_and_missing_palette_is_cached_as_unknown() {
        let capture = Capture::new(3, 3, vec![40; 9], None, None).unwrap();
        let mut references = palette(9);
        let mut cache = Cache::new(&capture, &references);
        let origin = point(0.0, 0.0);
        // The traversal fits; loading the first cell exceeds the remainder.
        let mut work = Work {
            estimated: super::super::super::MAX_WORK as f64 - 40.0,
        };
        assert!(matches!(
            cache.dark_segment(origin, origin, &mut work),
            Err(FingerprintError::WorkLimit)
        ));
        assert_eq!(
            cache
                .dark_segment(origin, origin, &mut Work::default())
                .unwrap(),
            SourceDecision::Confirmed
        );

        references.usable[6] = false;
        let mut cache = Cache::new(&capture, &references);
        for _ in 0..2 {
            assert_eq!(
                cache
                    .dark_segment(origin, point(1.4999999999999998, 1.5), &mut Work::default())
                    .unwrap(),
                SourceDecision::MissingSupport
            );
        }
    }
}
