//! Boundary alignment and stagger-preserving offset coordinates.

use super::{GridCell, GridPoint, GridShapeError, WorkBudget, frame::FrameCandidate};

pub(super) fn align_and_pad(
    points: &[GridPoint],
    padding: u32,
) -> Result<(Vec<GridCell>, u8), GridShapeError> {
    let top = points
        .iter()
        .map(|p| i64::from(p.r))
        .min()
        .ok_or(GridShapeError::InvalidInput("empty snapped figure"))?;
    let right = points
        .iter()
        .map(|p| 2 * i64::from(p.q) + i64::from(p.r))
        .max()
        .ok_or(GridShapeError::InvalidInput("empty snapped figure"))?;
    let padding = i64::from(padding);
    let phase = (right + top + padding).rem_euclid(2) as u8;
    let cells = points
        .iter()
        .map(|point| {
            let left2 = right - (2 * i64::from(point.q) + i64::from(point.r)) + 2 * padding;
            let row = i64::from(point.r) - top + padding;
            GridCell {
                column: (left2 / 2) as u64,
                row: row as u64,
            }
        })
        .collect();
    Ok((cells, phase))
}

/// Equal maximal edges use only geometric data, never source IDs or priorities.
pub(super) fn choose_frame(
    candidates: Vec<FrameCandidate>,
    work: &mut WorkBudget,
) -> Result<FrameCandidate, GridShapeError> {
    let mut best = None;
    for candidate in candidates {
        work.spend(candidate.snapped.len())?;
        let (cells, phase) = align_and_pad(&candidate.snapped, 0)?;
        let mut key: Vec<_> = cells
            .iter()
            .map(|cell| {
                (
                    cell.row,
                    2 * cell.column + (cell.row + u64::from(phase)) % 2,
                )
            })
            .collect();
        key.sort_unstable();
        if best.as_ref().is_none_or(|(best_key, _)| key < *best_key) {
            best = Some((key, candidate));
        }
    }
    best.map(|(_, frame)| frame)
        .ok_or(GridShapeError::InvalidInput("no alignment frame"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_actual_horizontal_extrema_instead_of_axial_minima() {
        let points = [GridPoint { q: 0, r: 0 }, GridPoint { q: 0, r: 1 }];
        let (cells, phase) = align_and_pad(&points, 0).unwrap();
        assert_eq!(phase, 1);
        assert_eq!(
            cells,
            [
                GridCell { column: 0, row: 0 },
                GridCell { column: 0, row: 1 }
            ]
        );
        let left2: Vec<_> = cells
            .iter()
            .map(|p| 2 * p.column + (p.row + u64::from(phase)) % 2)
            .collect();
        assert_eq!(left2, [1, 0]);
    }

    #[test]
    fn whole_lattice_translation_does_not_change_aligned_positions() {
        let points = [
            GridPoint { q: -4, r: -2 },
            GridPoint { q: 1, r: 3 },
            GridPoint { q: -2, r: 4 },
        ];
        let shifted: Vec<_> = points
            .iter()
            .map(|p| GridPoint {
                q: p.q + 19,
                r: p.r - 7,
            })
            .collect();
        for padding in [0, 1, 2, 3] {
            assert_eq!(
                align_and_pad(&points, padding),
                align_and_pad(&shifted, padding)
            );
        }
    }

    #[test]
    fn padding_adds_full_horizontal_steps_and_rows_without_straightening_the_grid() {
        let points = [GridPoint { q: -4, r: 0 }, GridPoint { q: 0, r: 1 }];
        let (base, phase) = align_and_pad(&points, 0).unwrap();
        for padding in 0..4 {
            let (padded, padded_phase) = align_and_pad(&points, padding).unwrap();
            for (a, b) in base.iter().zip(&padded) {
                let a_left = 2 * a.column + (a.row + u64::from(phase)) % 2;
                let b_left = 2 * b.column + (b.row + u64::from(padded_phase)) % 2;
                assert_eq!(b_left, a_left + 2 * u64::from(padding));
                assert_eq!(b.row, a.row + u64::from(padding));
            }
        }
    }
}
