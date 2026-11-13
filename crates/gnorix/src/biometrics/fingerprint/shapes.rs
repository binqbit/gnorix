//! Oriented point figures and complete positional variants on the measured grid.

mod frame;
mod grid;
mod variants;

#[cfg(test)]
mod tests;

use std::{collections::BTreeMap, error::Error, fmt};

use super::{CombinationPoint, FeatureCombination, Point};
use crate::biometrics::geometry::{GeometryError, GridPoint};

pub use variants::GridVariants;

/// Positional uncertainty is separate from the upstream subset selection budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridShapeConfig {
    pub uncertainty_steps: u32,
    /// Maximum complete Cartesian assignments across all supplied subsets.
    pub max_variants: u64,
    /// Preparation and iteration work entries, including each placed feature.
    pub max_work: usize,
}

impl GridShapeConfig {
    pub fn validate(&self) -> Result<(), GridShapeError> {
        if self.max_variants == 0 || self.max_work == 0 {
            return Err(GridShapeError::InvalidConfig("limits must be nonzero"));
        }
        variants::neighborhood_count(self.uncertainty_steps)?;
        Ok(())
    }
}

/// Nonnegative offset coordinates: columns grow left, rows grow down.
/// With the shape's phase, physical leftward distance in grid units is
/// `column + ((row + phase) % 2) / 2`; vertical distance is `sqrt(3) * row / 2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GridCell {
    pub column: u64,
    pub row: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridShapeFrame {
    /// Source IDs of the convex boundary; one/two vertices represent a point/line.
    pub boundary: Vec<usize>,
    pub top_edge: Option<[usize; 2]>,
    /// Virtual top-right bounding-box corner in the original coordinate frame.
    pub origin: Point,
    /// Rightward unit vector in the original frame; the downward axis is its normal.
    pub x_axis: Point,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridShape {
    pub combination_index: usize,
    /// All point arrays retain this ascending source-ID order, including collisions.
    pub feature_ids: Vec<usize>,
    pub frame: GridShapeFrame,
    pub aligned: Vec<Point>,
    /// Signed nearest nodes before boundary alignment and uncertainty padding.
    pub snapped: Vec<GridPoint>,
    pub baseline: Vec<GridCell>,
    /// Stagger of row zero, either zero or one half-step toward the left.
    pub phase: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridCollision {
    pub position: GridCell,
    pub feature_ids: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridVariant {
    pub shape_index: usize,
    pub positions: Vec<GridCell>,
    pub collisions: Vec<GridCollision>,
}

/// Validated prepared figures. Iteration keeps their common padded frames fixed.
#[derive(Clone, Debug)]
pub struct GridShapes {
    spacing_px: f64,
    uncertainty_steps: u32,
    shapes: Vec<GridShape>,
    offsets: Vec<GridPoint>,
    variant_count: u64,
    preparation_work: usize,
    remaining_work: usize,
}

impl GridShapes {
    pub fn spacing_px(&self) -> f64 {
        self.spacing_px
    }

    pub fn uncertainty_steps(&self) -> u32 {
        self.uncertainty_steps
    }

    pub fn shapes(&self) -> &[GridShape] {
        &self.shapes
    }

    pub fn variant_count(&self) -> u64 {
        self.variant_count
    }

    pub fn preparation_work(&self) -> usize {
        self.preparation_work
    }

    /// A new bounded traversal of every assignment, without per-variant recentering.
    /// A resource error is yielded explicitly and terminates that traversal.
    pub fn variants(&self) -> GridVariants<'_> {
        GridVariants::new(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridShapeError {
    InvalidConfig(&'static str),
    InvalidInput(&'static str),
    Geometry(GeometryError),
    CountOverflow,
    VariantLimit { required: u64, limit: u64 },
    WorkLimit,
}

impl fmt::Display for GridShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(reason) => write!(f, "invalid grid shape config: {reason}"),
            Self::InvalidInput(reason) => write!(f, "invalid grid shape input: {reason}"),
            Self::Geometry(error) => write!(f, "grid shape geometry: {error}"),
            Self::CountOverflow => f.write_str("complete variant count exceeds u64"),
            Self::VariantLimit { required, limit } => {
                write!(
                    f,
                    "complete traversal requires {required} variants; limit is {limit}"
                )
            }
            Self::WorkLimit => f.write_str("grid shape work limit exceeded"),
        }
    }
}

impl Error for GridShapeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Geometry(error) => Some(error),
            _ => None,
        }
    }
}

impl From<GeometryError> for GridShapeError {
    fn from(value: GeometryError) -> Self {
        Self::Geometry(value)
    }
}

/// Orients each subset by its longest convex-hull edge, snaps at measured ridge
/// width R, and prepares a complete Cartesian neighborhood traversal. Interior
/// points and coincident features remain present. Scale must come from the same
/// capture; this is not a cross-capture matching or key-encoding guarantee.
pub fn prepare_grid_shapes(
    points: &[CombinationPoint],
    combinations: &[FeatureCombination],
    unit_px: f64,
    config: &GridShapeConfig,
) -> Result<GridShapes, GridShapeError> {
    config.validate()?;
    if !unit_px.is_finite() || unit_px <= 0.0 {
        return Err(GridShapeError::Geometry(GeometryError::InvalidSpacing));
    }
    if combinations.is_empty() {
        return Err(GridShapeError::InvalidInput("no point combinations"));
    }
    let mut work = WorkBudget::new(config.max_work);
    let indexed = index_points(points, &mut work)?;
    let variant_count = count_variants(combinations, config, &mut work)?;
    let offsets = variants::neighborhood_offsets(config.uncertainty_steps, &mut work)?;
    let shapes = combinations
        .iter()
        .enumerate()
        .map(|(index, combination)| {
            prepare_shape(index, combination, &indexed, unit_px, config, &mut work)
        })
        .collect::<Result<_, _>>()?;

    Ok(GridShapes {
        spacing_px: unit_px,
        uncertainty_steps: config.uncertainty_steps,
        shapes,
        offsets,
        variant_count,
        preparation_work: config.max_work - work.remaining,
        remaining_work: work.remaining,
    })
}

fn index_points<'a>(
    points: &'a [CombinationPoint],
    work: &mut WorkBudget,
) -> Result<BTreeMap<usize, &'a CombinationPoint>, GridShapeError> {
    work.spend(points.len())?;
    let mut indexed = BTreeMap::new();
    for point in points {
        if indexed.insert(point.id, point).is_some() {
            return Err(GridShapeError::InvalidInput("duplicate source feature ID"));
        }
    }
    Ok(indexed)
}

fn count_variants(
    combinations: &[FeatureCombination],
    config: &GridShapeConfig,
    work: &mut WorkBudget,
) -> Result<u64, GridShapeError> {
    let choices = variants::neighborhood_count(config.uncertainty_steps)?;
    work.spend(combinations.len())?;
    let mut total = 0_u64;
    for combination in combinations {
        if combination.feature_ids.is_empty() {
            return Err(GridShapeError::InvalidInput("empty point combination"));
        }
        work.spend(combination.feature_ids.len())?;
        let count = combination.feature_ids.iter().try_fold(1_u64, |count, _| {
            count
                .checked_mul(choices)
                .ok_or(GridShapeError::CountOverflow)
        })?;
        total = total
            .checked_add(count)
            .ok_or(GridShapeError::CountOverflow)?;
    }
    if total > config.max_variants {
        return Err(GridShapeError::VariantLimit {
            required: total,
            limit: config.max_variants,
        });
    }
    Ok(total)
}

fn prepare_shape(
    combination_index: usize,
    combination: &FeatureCombination,
    points: &BTreeMap<usize, &CombinationPoint>,
    spacing: f64,
    config: &GridShapeConfig,
    work: &mut WorkBudget,
) -> Result<GridShape, GridShapeError> {
    work.spend(combination.feature_ids.len())?;
    let mut ids = combination.feature_ids.clone();
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(GridShapeError::InvalidInput(
            "a combination repeats a source ID",
        ));
    }
    let positions = ids
        .iter()
        .map(|id| {
            points
                .get(id)
                .map(|point| point.position)
                .ok_or(GridShapeError::InvalidInput(
                    "combination refers to an absent source point",
                ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let candidates = frame::candidate_frames(&positions, spacing, work)?;
    let selected = grid::choose_frame(candidates.candidates, work)?;
    let (baseline, phase) = grid::align_and_pad(&selected.snapped, config.uncertainty_steps)?;

    Ok(GridShape {
        combination_index,
        frame: GridShapeFrame {
            boundary: candidates.hull.iter().map(|&index| ids[index]).collect(),
            top_edge: selected.edge.map(|edge| [ids[edge[0]], ids[edge[1]]]),
            origin: selected.origin,
            x_axis: selected.x_axis,
        },
        feature_ids: ids,
        aligned: selected.aligned,
        snapped: selected.snapped,
        baseline,
        phase,
    })
}

struct WorkBudget {
    remaining: usize,
}

impl WorkBudget {
    fn new(limit: usize) -> Self {
        Self { remaining: limit }
    }

    fn spend(&mut self, amount: usize) -> Result<(), GridShapeError> {
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or(GridShapeError::WorkLimit)?;
        Ok(())
    }
}
