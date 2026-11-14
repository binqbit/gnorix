//! Complete positional assignments in each figure's fixed padded grid frame.

use std::{collections::BTreeMap, iter::FusedIterator, num::NonZeroUsize};

use super::{
    GeometryError, GridCell, GridCollision, GridPoint, GridShape, GridShapeError, GridShapes,
    GridVariant, WorkBudget,
};
use crate::{biometrics::Candidate, error::AlgorithmError};

/// Own one subset's traversal without evaluating its Cartesian product size.
pub(in super::super) struct CandidateVariants {
    baseline: Vec<GridCell>,
    phase: u8,
    offsets: Vec<GridPoint>,
    choices: Vec<usize>,
    work: WorkBudget,
    descriptor_bytes: usize,
    max_descriptor_bytes: NonZeroUsize,
    first: bool,
    finished: bool,
    baseline_padding: Option<u32>,
}

impl CandidateVariants {
    pub(super) fn new(
        baseline: Vec<GridCell>,
        phase: u8,
        offsets: Vec<GridPoint>,
        remaining_work: usize,
        descriptor_bytes: usize,
        max_descriptor_bytes: NonZeroUsize,
        baseline_padding: Option<u32>,
    ) -> Result<Self, AlgorithmError> {
        if baseline.is_empty() || offsets.is_empty() || phase > 1 {
            return Err(Box::new(GridShapeError::InvalidInput(
                "invalid positional traversal",
            )));
        }
        let mut work = WorkBudget::new(remaining_work);
        work.spend(baseline.len())?;
        let mut choices = Vec::new();
        choices
            .try_reserve_exact(baseline.len())
            .map_err(|_| GridShapeError::WorkLimit)?;
        choices.resize(baseline.len(), 0);
        Ok(Self {
            baseline,
            phase,
            offsets,
            choices,
            work,
            descriptor_bytes,
            max_descriptor_bytes,
            first: true,
            finished: false,
            baseline_padding,
        })
    }

    pub(in super::super) fn remaining_work(&self) -> usize {
        self.work.remaining
    }

    fn advance(&mut self) -> Result<Option<Candidate>, AlgorithmError> {
        loop {
            if !self.first && !advance_choices(&mut self.choices, self.offsets.len()) {
                return Ok(None);
            }
            self.first = false;
            let positions = place_positions(
                &self.baseline,
                self.phase,
                &self.offsets,
                &self.choices,
                &mut self.work,
            )?;
            if let Some(padding) = self.baseline_padding {
                self.work.spend(positions.len())?;
                if !has_baseline_bounds(&positions, self.phase, padding) {
                    continue;
                }
            }
            self.work.spend(self.descriptor_bytes)?;
            return super::grid_candidate(&positions, self.phase, self.max_descriptor_bytes)
                .map(Some)
                .map_err(Into::into);
        }
    }
}

/// `align_and_pad` always puts an enrolled baseline's upper/right extrema on
/// these exact boundaries. Inspect the physical staggered horizontal coordinate
/// rather than the column alone. Wide arithmetic also handles extreme cells.
fn has_baseline_bounds(positions: &[GridCell], phase: u8, padding: u32) -> bool {
    let mut top = u64::MAX;
    let mut right = u128::MAX;
    for position in positions {
        top = top.min(position.row);
        let stagger = (u128::from(position.row) + u128::from(phase)) % 2;
        right = right.min(2 * u128::from(position.column) + stagger);
    }
    top == u64::from(padding) && right == 2 * u128::from(padding)
}

impl Iterator for CandidateVariants {
    type Item = Result<Candidate, AlgorithmError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match self.advance() {
            Ok(Some(candidate)) => Some(Ok(candidate)),
            Ok(None) => {
                self.finished = true;
                None
            }
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }
}

impl FusedIterator for CandidateVariants {}

pub(super) fn neighborhood_count(level: u32) -> Result<u64, GridShapeError> {
    let radius = u64::from(level);
    let count = radius
        .checked_mul(radius + 1)
        .and_then(|value| value.checked_mul(3))
        .and_then(|value| value.checked_add(1))
        .ok_or(GridShapeError::CountOverflow)?;
    if level > i32::MAX as u32 {
        return Err(GridShapeError::InvalidConfig(
            "neighborhood offsets exceed the integer coordinate range",
        ));
    }
    Ok(count)
}

/// Center first, then complete hexagonal rings in ascending `(q, r)` order.
pub(super) fn neighborhood_offsets(
    level: u32,
    work: &mut WorkBudget,
) -> Result<Vec<GridPoint>, GridShapeError> {
    let count =
        usize::try_from(neighborhood_count(level)?).map_err(|_| GridShapeError::CountOverflow)?;
    work.spend(count)?;
    let mut offsets = Vec::new();
    offsets
        .try_reserve_exact(count)
        .map_err(|_| GridShapeError::WorkLimit)?;
    offsets.push(GridPoint { q: 0, r: 0 });
    for radius in 1..=i64::from(level) {
        append_ring(&mut offsets, radius);
    }
    Ok(offsets)
}

fn append_ring(offsets: &mut Vec<GridPoint>, radius: i64) {
    // At the extreme q columns the entire interval is on the boundary.
    // Other columns contribute just their two boundary endpoints: 6r nodes.
    for q in -radius..=radius {
        let first = (-radius).max(-q - radius);
        let last = radius.min(-q + radius);
        if q.abs() == radius {
            for r in first..=last {
                offsets.push(GridPoint {
                    q: q as i32,
                    r: r as i32,
                });
            }
        } else {
            for r in [first, last] {
                offsets.push(GridPoint {
                    q: q as i32,
                    r: r as i32,
                });
            }
        }
    }
}

/// Lazy mixed-radix traversal: the final feature varies fastest.
///
/// Every point keeps its identity, even when several points occupy one cell.
/// Positions remain in the prepared figure's padded frame; variants are never
/// individually translated or deduplicated. Errors terminate the traversal.
pub struct GridVariants<'a> {
    shapes: &'a [GridShape],
    offsets: &'a [GridPoint],
    shape_index: usize,
    choices: Vec<usize>,
    work: WorkBudget,
    finished: bool,
}

impl<'a> GridVariants<'a> {
    pub(super) fn new(prepared: &'a GridShapes) -> Self {
        Self {
            shapes: &prepared.shapes,
            offsets: &prepared.offsets,
            shape_index: 0,
            choices: Vec::new(),
            work: WorkBudget::new(prepared.remaining_work),
            finished: false,
        }
    }

    fn next_assignment(&mut self) -> Result<GridVariant, GridShapeError> {
        let shape = &self.shapes[self.shape_index];
        if self.choices.is_empty() {
            self.start_shape(shape)?;
        }
        let variant = place_assignment(
            self.shape_index,
            shape,
            self.offsets,
            &self.choices,
            &mut self.work,
        )?;
        if !advance_choices(&mut self.choices, self.offsets.len()) {
            self.shape_index += 1;
            self.choices.clear();
        }
        Ok(variant)
    }

    fn start_shape(&mut self, shape: &GridShape) -> Result<(), GridShapeError> {
        if self.offsets.is_empty() || shape.baseline.is_empty() {
            return Err(GridShapeError::InvalidInput("empty positional traversal"));
        }
        if shape.baseline.len() != shape.feature_ids.len() || shape.phase > 1 {
            return Err(GridShapeError::InvalidInput("invalid prepared figure"));
        }
        self.work.spend(shape.baseline.len())?;
        self.choices
            .try_reserve_exact(shape.baseline.len())
            .map_err(|_| GridShapeError::WorkLimit)?;
        self.choices.resize(shape.baseline.len(), 0);
        Ok(())
    }
}

impl Iterator for GridVariants<'_> {
    type Item = Result<GridVariant, GridShapeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished || self.shape_index == self.shapes.len() {
            self.finished = true;
            return None;
        }
        let variant = self.next_assignment();
        if variant.is_err() {
            self.finished = true;
        }
        Some(variant)
    }
}

impl FusedIterator for GridVariants<'_> {}

fn advance_choices(choices: &mut [usize], radix: usize) -> bool {
    for choice in choices.iter_mut().rev() {
        *choice += 1;
        if *choice < radix {
            return true;
        }
        *choice = 0;
    }
    false
}

fn place_assignment(
    shape_index: usize,
    shape: &GridShape,
    offsets: &[GridPoint],
    choices: &[usize],
    work: &mut WorkBudget,
) -> Result<GridVariant, GridShapeError> {
    // Each feature contributes one output position and one collision-index entry.
    work.spend(shape.baseline.len())?;
    let positions = place_positions(&shape.baseline, shape.phase, offsets, choices, work)?;
    let collisions = collision_groups(&shape.feature_ids, &positions);
    Ok(GridVariant {
        shape_index,
        positions,
        collisions,
    })
}

fn place_positions(
    baseline: &[GridCell],
    phase: u8,
    offsets: &[GridPoint],
    choices: &[usize],
    work: &mut WorkBudget,
) -> Result<Vec<GridCell>, GridShapeError> {
    work.spend(baseline.len())?;
    baseline
        .iter()
        .zip(choices)
        .map(|(&cell, &choice)| shift_cell(cell, phase, offsets[choice]))
        .collect()
}

fn shift_cell(cell: GridCell, phase: u8, offset: GridPoint) -> Result<GridCell, GridShapeError> {
    let row = i128::from(cell.row);
    let left2 = 2 * i128::from(cell.column) + (row + i128::from(phase)) % 2;
    let shifted_left = left2 - 2 * i128::from(offset.q) - i128::from(offset.r);
    let shifted_row = row + i128::from(offset.r);
    if shifted_left < 0 || shifted_row < 0 {
        return Err(GridShapeError::InvalidInput(
            "positional variant leaves the padded coordinate frame",
        ));
    }
    if shifted_left % 2 != (shifted_row + i128::from(phase)) % 2 {
        return Err(GridShapeError::InvalidInput(
            "positional variant changes grid phase",
        ));
    }
    Ok(GridCell {
        column: u64::try_from(shifted_left / 2).map_err(|_| GeometryError::CoordinateOverflow)?,
        row: u64::try_from(shifted_row).map_err(|_| GeometryError::CoordinateOverflow)?,
    })
}

fn collision_groups(ids: &[usize], positions: &[GridCell]) -> Vec<GridCollision> {
    let mut occupied: BTreeMap<GridCell, Vec<usize>> = BTreeMap::new();
    for (&id, &position) in ids.iter().zip(positions) {
        occupied.entry(position).or_default().push(id);
    }
    occupied
        .into_iter()
        .filter_map(|(position, mut feature_ids)| {
            if feature_ids.len() < 2 {
                return None;
            }
            feature_ids.sort_unstable();
            Some(GridCollision {
                position,
                feature_ids,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::{GridShapeFrame, Point};
    use super::*;

    fn cell(column: u64, row: u64) -> GridCell {
        GridCell { column, row }
    }

    fn shape(ids: &[usize], baseline: &[GridCell], phase: u8) -> GridShape {
        GridShape {
            combination_index: 0,
            feature_ids: ids.to_vec(),
            frame: GridShapeFrame {
                boundary: ids.to_vec(),
                top_edge: None,
                origin: Point::new(0.0, 0.0).unwrap(),
                x_axis: Point::new(1.0, 0.0).unwrap(),
            },
            aligned: vec![Point::new(0.0, 0.0).unwrap(); ids.len()],
            snapped: vec![GridPoint { q: 0, r: 0 }; ids.len()],
            baseline: baseline.to_vec(),
            phase,
        }
    }

    fn prepared(level: u32, shapes: Vec<GridShape>, remaining_work: usize) -> GridShapes {
        let offsets = neighborhood_offsets(level, &mut WorkBudget::new(1000)).unwrap();
        let variant_count = shapes
            .iter()
            .map(|shape| (offsets.len() as u64).pow(shape.feature_ids.len() as u32))
            .sum();
        GridShapes {
            spacing_px: 1.0,
            uncertainty_steps: level,
            shapes,
            offsets,
            variant_count,
            preparation_work: 0,
            remaining_work,
        }
    }

    #[test]
    fn baseline_filter_reports_budget_exhaustion_while_skipping_impossible_assignments() {
        let baseline = vec![
            cell(28, 1),
            cell(21, 2),
            cell(11, 11),
            cell(5, 2),
            cell(1, 1),
        ];
        let offsets = neighborhood_offsets(1, &mut WorkBudget::new(100)).unwrap();
        let bytes = NonZeroUsize::new(4096).unwrap();
        let descriptor = super::super::encoding::descriptor_size(baseline.len(), bytes).unwrap();
        let mut candidates =
            CandidateVariants::new(baseline, 1, offsets, 170, descriptor, bytes, Some(1)).unwrap();
        // Constructor costs 5, the baseline costs 155, and the next impossible
        // assignment costs 10. Reaching the next placement must report a limit.
        assert!(candidates.next().unwrap().is_ok());
        assert!(matches!(
            candidates
                .next()
                .unwrap()
                .unwrap_err()
                .downcast_ref::<GridShapeError>(),
            Some(GridShapeError::WorkLimit)
        ));
        assert_eq!(candidates.remaining_work(), 0);
        assert!(candidates.next().is_none());
    }

    #[test]
    fn baseline_bounds_use_physical_stagger_and_handle_extreme_coordinates() {
        assert!(has_baseline_bounds(&[cell(1, 1)], 1, 1));
        assert!(!has_baseline_bounds(&[cell(1, 1)], 0, 1));
        assert!(!has_baseline_bounds(&[cell(u64::MAX, u64::MAX)], 1, 1));
    }

    #[test]
    fn baseline_filter_does_not_hide_coordinate_overflow() {
        let bytes = NonZeroUsize::new(4096).unwrap();
        let mut candidates = CandidateVariants::new(
            vec![cell(u64::MAX, u64::MAX)],
            0,
            vec![GridPoint { q: 0, r: 0 }, GridPoint { q: -1, r: 0 }],
            1000,
            super::super::encoding::descriptor_size(1, bytes).unwrap(),
            bytes,
            Some(1),
        )
        .unwrap();
        assert!(matches!(
            candidates
                .next()
                .unwrap()
                .unwrap_err()
                .downcast_ref::<GridShapeError>(),
            Some(GridShapeError::Geometry(GeometryError::CoordinateOverflow))
        ));
        assert!(candidates.next().is_none());
    }

    #[test]
    fn neighborhoods_are_complete_unique_and_ordered_hexagonal_rings() {
        for (level, count) in [(0, 1), (1, 7), (2, 19), (3, 37)] {
            let mut work = WorkBudget::new(count);
            let offsets = neighborhood_offsets(level, &mut work).unwrap();
            assert_eq!(offsets.len(), count);
            assert_eq!(neighborhood_count(level), Ok(count as u64));
            assert_eq!(work.remaining, 0);
            assert_eq!(offsets[0], GridPoint { q: 0, r: 0 });
            let unique: BTreeSet<_> = offsets.iter().copied().collect();
            assert_eq!(unique.len(), count);
            let key = |point: GridPoint| {
                let distance = point
                    .q
                    .abs()
                    .max(point.r.abs())
                    .max((point.q + point.r).abs());
                (distance, point.q, point.r)
            };
            assert!(offsets.windows(2).all(|pair| key(pair[0]) < key(pair[1])));
            let radius = level as i32;
            for q in -radius..=radius {
                for r in -radius..=radius {
                    assert_eq!(
                        unique.contains(&GridPoint { q, r }),
                        q.abs().max(r.abs()).max((q + r).abs()) <= radius
                    );
                }
            }
        }
    }

    #[test]
    fn cartesian_traversal_keeps_all_seven_cubed_assignments() {
        let input = prepared(
            1,
            vec![shape(&[3, 5, 9], &[cell(1, 1), cell(3, 2), cell(5, 3)], 1)],
            10_000,
        );
        let variants = input.variants().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(variants.len(), 7_usize.pow(3));
        assert_eq!(variants.len() as u64, input.variant_count);
        assert!(variants.iter().all(|variant| variant.shape_index == 0));
        assert_eq!(variants[0].positions, input.shapes[0].baseline);
        let unique: BTreeSet<_> = variants.iter().map(|variant| &variant.positions).collect();
        assert_eq!(unique.len(), variants.len());
        for index in 0..3 {
            let positions: BTreeSet<_> = variants
                .iter()
                .map(|variant| variant.positions[index])
                .collect();
            assert_eq!(positions.len(), 7);
        }
        // The final feature changes fastest, independently of the first two.
        for variant in &variants[..7] {
            assert_eq!(variant.positions[..2], input.shapes[0].baseline[..2]);
        }
        assert_ne!(variants[7].positions[1], variants[0].positions[1]);
    }

    #[test]
    fn traversal_preserves_each_figures_fixed_frame_and_order() {
        let input = prepared(
            1,
            vec![
                shape(&[10], &[cell(1, 1)], 1),
                shape(&[20, 30], &[cell(4, 4), cell(6, 5)], 0),
            ],
            1000,
        );
        let variants = input.variants().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(variants.len(), 7 + 49);
        assert!(variants[..7].iter().all(|variant| variant.shape_index == 0));
        assert!(variants[7..].iter().all(|variant| variant.shape_index == 1));
        let first_positions: BTreeSet<_> = variants[..7]
            .iter()
            .map(|variant| variant.positions[0])
            .collect();
        // A per-variant recenter would incorrectly collapse all seven singletons.
        assert_eq!(first_positions.len(), 7);
        assert_eq!(
            first_positions.iter().map(|cell| cell.column).min(),
            Some(0)
        );
        assert_eq!(first_positions.iter().map(|cell| cell.row).min(), Some(0));
        assert_eq!(variants[7].positions, input.shapes[1].baseline);
        for (&offset, variant) in input.offsets.iter().zip(&variants[..7]) {
            let position = variant.positions[0];
            let left2 = 2 * position.column + (position.row + 1) % 2;
            assert_eq!(
                left2 as i64 - 2,
                -2 * i64::from(offset.q) - i64::from(offset.r)
            );
            assert_eq!(position.row as i64 - 1, i64::from(offset.r));
        }
    }

    #[test]
    fn zero_uncertainty_retains_all_colliding_features_and_sorted_groups() {
        let baseline = [cell(4, 3), cell(1, 2), cell(4, 3), cell(1, 2)];
        let input = prepared(0, vec![shape(&[9, 8, 3, 2], &baseline, 0)], 100);
        let mut iterator = input.variants();
        let variant = iterator.next().unwrap().unwrap();
        assert_eq!(variant.positions, baseline);
        assert_eq!(
            variant.collisions,
            vec![
                GridCollision {
                    position: cell(1, 2),
                    feature_ids: vec![2, 8]
                },
                GridCollision {
                    position: cell(4, 3),
                    feature_ids: vec![3, 9]
                },
            ]
        );
        assert!(iterator.next().is_none());
        assert!(iterator.next().is_none());
    }

    #[test]
    fn neighboring_movements_can_collide_without_dropping_assignments() {
        let input = prepared(1, vec![shape(&[3, 9], &[cell(1, 1), cell(2, 1)], 1)], 1000);
        let variants = input.variants().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(variants.len(), 49);
        assert!(variants.iter().any(|variant| {
            variant.positions == [cell(2, 1), cell(2, 1)]
                && variant.collisions
                    == [GridCollision {
                        position: cell(2, 1),
                        feature_ids: vec![3, 9],
                    }]
        }));
        assert!(variants.iter().all(|variant| variant.positions.len() == 2));
    }

    #[test]
    fn work_exhaustion_is_an_explicit_fused_error_including_before_first_variant() {
        for (work, successes) in [(0, 0), (1, 0), (7, 3)] {
            let input = prepared(1, vec![shape(&[3], &[cell(1, 1)], 1)], work);
            let mut iterator = input.variants();
            for _ in 0..successes {
                assert!(iterator.next().unwrap().is_ok());
            }
            assert_eq!(iterator.next(), Some(Err(GridShapeError::WorkLimit)));
            assert!(iterator.next().is_none());
            assert!(iterator.next().is_none());
        }
    }

    #[test]
    fn missing_padding_fails_without_clipping_or_recentring() {
        let input = prepared(1, vec![shape(&[3], &[cell(0, 0)], 0)], 100);
        let mut iterator = input.variants();
        assert_eq!(iterator.next().unwrap().unwrap().positions, [cell(0, 0)]);
        let error = iterator.by_ref().find_map(Result::err).unwrap();
        assert!(matches!(error, GridShapeError::InvalidInput(_)));
        assert!(iterator.next().is_none());
        assert_eq!(
            shift_cell(cell(u64::MAX, 0), 0, GridPoint { q: -1, r: 0 }),
            Err(GridShapeError::Geometry(GeometryError::CoordinateOverflow))
        );
        assert_eq!(
            shift_cell(cell(1, u64::MAX), 0, GridPoint { q: 0, r: 1 }),
            Err(GridShapeError::Geometry(GeometryError::CoordinateOverflow))
        );
    }

    #[test]
    fn neighborhood_limits_are_checked_before_reserving_or_enumerating() {
        let mut work = WorkBudget::new(6);
        assert_eq!(
            neighborhood_offsets(1, &mut work),
            Err(GridShapeError::WorkLimit)
        );
        assert_eq!(work.remaining, 6);
        assert_eq!(
            neighborhood_count(u32::MAX),
            Err(GridShapeError::CountOverflow)
        );
        assert!(matches!(
            neighborhood_count(i32::MAX as u32 + 1),
            Err(GridShapeError::InvalidConfig(_))
        ));
        assert_eq!(
            neighborhood_offsets(i32::MAX as u32, &mut WorkBudget::new(0)),
            Err(GridShapeError::WorkLimit)
        );
    }
}
