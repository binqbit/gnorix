//! Minimum regular grid whose cell corners fit the circle radius.

use super::{CombinationError, FingerprintError, WorkBudget};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    pub columns: usize,
    pub rows: usize,
}

impl Layout {
    pub fn count(self) -> usize {
        // Every layout is constructed with a checked product.
        self.columns * self.rows
    }
}

pub(super) fn covering_grid(
    width: f64,
    height: f64,
    radius: f64,
    work: &mut WorkBudget,
) -> Result<Layout, CombinationError> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(CombinationError::InvalidFingerprint(
            FingerprintError::InvalidMap("region bounds must have finite positive extents"),
        ));
    }
    if !radius.is_finite() || radius <= 0.0 {
        return Err(CombinationError::InvalidConfig(
            "circle radius must be finite and positive",
        ));
    }
    work.spend(1)?;
    if covers(
        width,
        height,
        radius,
        Layout {
            columns: 1,
            rows: 1,
        },
    ) {
        return Ok(Layout {
            columns: 1,
            rows: 1,
        });
    }
    // Search the shorter axis to bound work for long, thin capture rectangles.
    if width > height {
        let result = search_grid(height, width, radius, true, work)?;
        Ok(Layout {
            columns: result.rows,
            rows: result.columns,
        })
    } else {
        search_grid(width, height, radius, false, work)
    }
}

fn search_grid(
    width: f64,
    height: f64,
    radius: f64,
    transposed: bool,
    work: &mut WorkBudget,
) -> Result<Layout, CombinationError> {
    // Normalized half-extents keep the solving arithmetic scale-independent.
    let horizontal = (width / 2.0) / radius;
    let vertical = (height / 2.0) / radius;
    let mut best = checked_layout(
        ceiling(std::f64::consts::SQRT_2 * horizontal)?,
        ceiling(std::f64::consts::SQRT_2 * vertical)?,
    )?;
    while !covers(width, height, radius, best) {
        work.spend(1)?;
        best = checked_layout(
            best.columns,
            best.rows
                .checked_add(1)
                .ok_or(CombinationError::WorkLimit)?,
        )?;
    }

    let minimum_rows = strict_axis_count(vertical)?;
    let mut columns = strict_axis_count(horizontal)?;
    if checked_layout(columns, minimum_rows)?.count() > work.remaining {
        return Err(CombinationError::WorkLimit);
    }
    while columns <= best.count() / minimum_rows {
        work.spend(1)?;
        if let Some(candidate) =
            candidate_for_columns(width, height, radius, horizontal, vertical, columns, work)?
            && better(candidate, best, width, height, transposed)
        {
            best = candidate;
        }
        columns = columns.checked_add(1).ok_or(CombinationError::WorkLimit)?;
    }
    Ok(best)
}

fn candidate_for_columns(
    width: f64,
    height: f64,
    radius: f64,
    horizontal: f64,
    vertical: f64,
    columns: usize,
    work: &mut WorkBudget,
) -> Result<Option<Layout>, CombinationError> {
    let side = horizontal / columns as f64;
    let remaining = (1.0 - side) * (1.0 + side);
    if remaining <= 0.0 {
        return Ok(None);
    }
    let mut candidate = checked_layout(columns, ceiling(vertical / remaining.sqrt())?)?;
    while !covers(width, height, radius, candidate) {
        work.spend(1)?;
        candidate = checked_layout(
            columns,
            candidate
                .rows
                .checked_add(1)
                .ok_or(CombinationError::WorkLimit)?,
        )?;
    }
    while candidate.rows > 1
        && covers(
            width,
            height,
            radius,
            Layout {
                rows: candidate.rows - 1,
                ..candidate
            },
        )
    {
        work.spend(1)?;
        candidate.rows -= 1;
    }
    Ok(Some(candidate))
}

fn covers(width: f64, height: f64, radius: f64, layout: Layout) -> bool {
    (width / layout.columns as f64 / 2.0).hypot(height / layout.rows as f64 / 2.0) <= radius
}

fn better(candidate: Layout, current: Layout, width: f64, height: f64, transposed: bool) -> bool {
    let aspect =
        |layout: Layout| (width / layout.columns as f64 - height / layout.rows as f64).abs();
    let axes = |layout: Layout| {
        if transposed {
            (layout.rows, layout.columns)
        } else {
            (layout.columns, layout.rows)
        }
    };
    candidate
        .count()
        .cmp(&current.count())
        .then_with(|| aspect(candidate).total_cmp(&aspect(current)))
        .then(axes(candidate).cmp(&axes(current)))
        .is_lt()
}

fn checked_layout(columns: usize, rows: usize) -> Result<Layout, CombinationError> {
    columns
        .checked_mul(rows)
        .ok_or(CombinationError::WorkLimit)?;
    Ok(Layout { columns, rows })
}

fn ceiling(value: f64) -> Result<usize, CombinationError> {
    if !value.is_finite() || value >= usize::MAX as f64 {
        return Err(CombinationError::WorkLimit);
    }
    Ok((value.ceil() as usize).max(1))
}

fn strict_axis_count(ratio: f64) -> Result<usize, CombinationError> {
    if !ratio.is_finite() || ratio >= usize::MAX as f64 {
        return Err(CombinationError::WorkLimit);
    }
    (ratio.floor() as usize)
        .checked_add(1)
        .ok_or(CombinationError::WorkLimit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_layout_search_matches_exhaustive_integer_grids_and_ties() {
        for width in 1..=20 {
            for height in 1..=20 {
                for radius in [0.75, 1.0, 1.4, 2.5, 5.0, 10.0] {
                    let (width, height) = (width as f64, height as f64);
                    let mut expected = None;
                    for columns in 1..=40 {
                        for rows in 1..=40 {
                            if (width / columns as f64 / 2.0).hypot(height / rows as f64 / 2.0)
                                > radius
                            {
                                continue;
                            }
                            let candidate = (
                                columns * rows,
                                (width / columns as f64 - height / rows as f64).abs(),
                                columns,
                                rows,
                            );
                            if expected.is_none_or(|old| candidate < old) {
                                expected = Some(candidate);
                            }
                        }
                    }
                    let (_, _, columns, rows) = expected.unwrap();
                    let result =
                        covering_grid(width, height, radius, &mut WorkBudget::new(1000)).unwrap();
                    assert_eq!(
                        result,
                        Layout { columns, rows },
                        "{width} by {height}, radius {radius}"
                    );
                }
            }
        }
    }

    #[test]
    fn measured_examples_adapt_to_capture_size_and_are_scale_invariant() {
        for (width, height, radius, columns, rows) in [
            (300.0, 400.0, 30.0 * 16.8, 1, 1),
            (1086.0, 1448.0, 30.0 * 16.8207925, 2, 2),
            (760.0, 1014.0, 30.0 * 16.19527, 1, 2),
            (1086.0, 1448.0, 16.0 * 16.8207925, 3, 4),
        ] {
            for scale in [1e-100, 0.1, 1.0, 3.7, 1e100] {
                assert_eq!(
                    covering_grid(
                        width * scale,
                        height * scale,
                        radius * scale,
                        &mut WorkBudget::new(100)
                    )
                    .unwrap(),
                    Layout { columns, rows }
                );
            }
        }
    }

    #[test]
    fn handles_exact_circle_boundaries_without_inventing_extra_cells() {
        assert_eq!(
            covering_grid(6.0, 8.0, 5.0, &mut WorkBudget::new(1)).unwrap(),
            Layout {
                columns: 1,
                rows: 1
            }
        );
        let below = f64::from_bits(5.0_f64.to_bits() - 1);
        assert!(
            covering_grid(6.0, 8.0, below, &mut WorkBudget::new(100))
                .unwrap()
                .count()
                > 1
        );
    }

    #[test]
    fn very_wide_thin_bounds_search_the_short_axis_without_allocating_regions() {
        let limit = 1_000_000_000;
        let mut work = WorkBudget::new(limit);
        let result = covering_grid(100_000_000.0, 1.0, 1.0, &mut work).unwrap();
        assert_eq!(result.rows, 1);
        assert_eq!(
            result.columns,
            (100_000_000.0 / 3.0_f64.sqrt()).ceil() as usize
        );
        assert!(limit - work.remaining < 10);
    }
}
