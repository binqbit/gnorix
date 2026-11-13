//! Canonical integer-grid descriptors and their SHA-256 candidate digests.

use std::num::NonZeroUsize;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{GridCell, GridShapes};
use crate::{
    Error, Result,
    biometrics::{Candidate, Candidates},
    encoding,
    error::{AlgorithmError, Resource},
};

const DOMAIN: &[u8] = b"gnorix.grid-candidate.v1";
const COORDINATE_BYTES: usize = 16;
// Four length-prefixed fields: domain, phase, count and coordinate records.
const DESCRIPTOR_OVERHEAD: usize = 4 * 8 + DOMAIN.len() + 1 + 8;

/// Hash a positional variant independently of feature IDs or input order.
///
/// Records are sorted by `(row, column)` and encoded as consecutive
/// `(column:u64be, row:u64be)` pairs, including repeats. The three fields are
/// phase (one byte), record count (`u64be`) and the coordinate records. Each
/// field and the `gnorix.grid-candidate.v1` domain has a `u64be` byte-length
/// prefix. SHA-256 of those framed bytes becomes the 32-byte candidate.
///
/// Phase must be zero or one. No translation, deduplication, feature metadata
/// or source-pixel scale is applied. `max_descriptor_bytes` bounds the complete
/// framed descriptor before any sorting or allocation.
pub fn grid_candidate(
    positions: &[GridCell],
    phase: u8,
    max_descriptor_bytes: NonZeroUsize,
) -> Result<Candidate> {
    let descriptor = canonical_descriptor(positions, phase, max_descriptor_bytes)?;
    let digest = Zeroizing::new(<[u8; 32]>::from(Sha256::digest(&descriptor)));

    Candidate::new(digest.to_vec())
}

pub(super) fn candidates(
    shapes: &GridShapes,
    max_descriptor_bytes: NonZeroUsize,
) -> Candidates<'_> {
    let mut variants = shapes.variants();
    let mut failed = false;

    Box::new(std::iter::from_fn(move || {
        if failed {
            return None;
        }
        let result =
            variants
                .next()?
                .map_err(|error| -> AlgorithmError { Box::new(error) })
                .and_then(|variant| {
                    let shape = shapes.shapes().get(variant.shape_index).ok_or_else(
                        || -> AlgorithmError {
                            Box::new(Error::InvalidInput(
                                "variant refers to an absent grid shape",
                            ))
                        },
                    )?;
                    grid_candidate(&variant.positions, shape.phase, max_descriptor_bytes)
                        .map_err(Into::into)
                });
        failed = result.is_err();
        Some(result)
    }))
}

fn canonical_descriptor(
    positions: &[GridCell],
    phase: u8,
    max_bytes: NonZeroUsize,
) -> Result<Zeroizing<Vec<u8>>> {
    if positions.is_empty() {
        return Err(Error::InvalidInput("grid candidate has no points"));
    }
    if phase > 1 {
        return Err(Error::InvalidInput(
            "grid candidate phase must be zero or one",
        ));
    }
    let record_bytes = checked_record_bytes(positions.len(), max_bytes)?;
    let count = u64::try_from(positions.len())
        .map_err(|_| Error::LimitExceeded(Resource::CandidateBytes))?;
    let mut ordered = Zeroizing::new(
        positions
            .iter()
            .map(|point| [point.row, point.column])
            .collect::<Vec<_>>(),
    );
    ordered.sort_unstable();
    let mut records = Zeroizing::new(Vec::with_capacity(record_bytes));
    for &[row, column] in ordered.iter() {
        records.extend_from_slice(&column.to_be_bytes());
        records.extend_from_slice(&row.to_be_bytes());
    }

    encoding::encode(
        DOMAIN,
        &[&[phase], &count.to_be_bytes(), &records],
        max_bytes,
        Resource::CandidateBytes,
    )
    .map(Zeroizing::new)
}

fn checked_record_bytes(count: usize, max_bytes: NonZeroUsize) -> Result<usize> {
    let records = count
        .checked_mul(COORDINATE_BYTES)
        .ok_or(Error::LimitExceeded(Resource::CandidateBytes))?;
    records
        .checked_add(DESCRIPTOR_OVERHEAD)
        .filter(|&size| size <= max_bytes.get())
        .ok_or(Error::LimitExceeded(Resource::CandidateBytes))?;
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biometrics::{
        fingerprint::{
            CombinationPoint, FeatureCombination, FeatureKind, GridShapeConfig, GridShapeError,
            prepare_grid_shapes,
        },
        geometry::Point,
    };

    fn limit(bytes: usize) -> NonZeroUsize {
        NonZeroUsize::new(bytes).unwrap()
    }

    fn cell(column: u64, row: u64) -> GridCell {
        GridCell { column, row }
    }

    fn digest(points: &[GridCell], phase: u8) -> Candidate {
        grid_candidate(points, phase, limit(4096)).unwrap()
    }

    fn decode_hex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    fn prepared(level: u32) -> GridShapes {
        let points = [(0.0, 0.0), (12.0, 0.0), (4.0, 5.0)]
            .into_iter()
            .enumerate()
            .map(|(id, (x, y))| CombinationPoint {
                id,
                kind: FeatureKind::Ending,
                position: Point::new(x, y).unwrap(),
                priority: 1.0,
            })
            .collect::<Vec<_>>();
        let combinations =
            [vec![0], vec![0, 1, 2], vec![0]].map(|feature_ids| FeatureCombination {
                feature_ids,
                score: 1.0,
                regions: vec![0],
            });
        prepare_grid_shapes(
            &points,
            &combinations,
            1.0,
            &GridShapeConfig {
                uncertainty_steps: level,
                max_variants: 1000,
                max_work: 10_000,
            },
        )
        .unwrap()
    }

    #[test]
    fn exact_descriptor_and_hash_match_an_independent_vector() {
        // Generated independently with Python struct.pack('>Q', ...) and hashlib.
        let positions = [cell(3, 2), cell(1, 0), cell(3, 2)];
        let expected = decode_hex(concat!(
            "0000000000000018676e6f7269782e677269642d63616e6469646174652e7631",
            "000000000000000101",
            "00000000000000080000000000000003",
            "0000000000000030",
            "00000000000000010000000000000000",
            "00000000000000030000000000000002",
            "00000000000000030000000000000002",
        ));
        let descriptor = canonical_descriptor(&positions, 1, limit(113)).unwrap();
        assert_eq!(*descriptor, expected);
        assert_eq!(
            digest(&positions, 1).as_bytes(),
            decode_hex("fd02c9d332471c501d0d7f7b71fbc02737ac95f5ac6a41682c37ba65509db01c"),
        );
    }

    #[test]
    fn coordinate_order_is_irrelevant_but_phase_multiplicity_and_translation_are_not() {
        let positions = [cell(3, 2), cell(1, 0), cell(3, 2)];
        let base = digest(&positions, 0);
        for reordered in [
            [positions[1], positions[0], positions[2]],
            [positions[2], positions[0], positions[1]],
        ] {
            assert_eq!(base.as_bytes(), digest(&reordered, 0).as_bytes());
        }
        assert_ne!(base.as_bytes(), digest(&positions, 1).as_bytes());
        assert_ne!(base.as_bytes(), digest(&positions[..2], 0).as_bytes());
        let translated = positions.map(|p| cell(p.column + 1, p.row + 2));
        assert_ne!(base.as_bytes(), digest(&translated, 0).as_bytes());
        assert_ne!(
            digest(&[cell(1, 23)], 0).as_bytes(),
            digest(&[cell(12, 3)], 0).as_bytes(),
        );
    }

    #[test]
    fn validates_phase_points_and_full_descriptor_size_before_allocation() {
        assert!(matches!(
            grid_candidate(&[], 0, limit(4096)),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            grid_candidate(&[cell(0, 0)], 2, limit(4096)),
            Err(Error::InvalidInput(_))
        ));
        let points = [cell(1, 2), cell(u64::MAX, u64::MAX), cell(0, 1)];
        assert!(matches!(
            grid_candidate(&points, 0, limit(112)),
            Err(Error::LimitExceeded(Resource::CandidateBytes))
        ));
        assert_eq!(
            grid_candidate(&points, 0, limit(113))
                .unwrap()
                .as_bytes()
                .len(),
            32
        );
        assert!(matches!(
            checked_record_bytes(usize::MAX, limit(usize::MAX)),
            Err(Error::LimitExceeded(Resource::CandidateBytes))
        ));
    }

    #[test]
    fn source_ids_feature_kinds_and_scores_do_not_enter_candidate_bytes() {
        let mut points = [(2.13, 1.74), (29.7, 1.74), (10.1, 13.2)]
            .into_iter()
            .enumerate()
            .map(|(id, (x, y))| CombinationPoint {
                id,
                kind: FeatureKind::Ending,
                position: Point::new(x, y).unwrap(),
                priority: 1.0,
            })
            .collect::<Vec<_>>();
        let config = GridShapeConfig {
            uncertainty_steps: 0,
            max_variants: 1,
            max_work: 1000,
        };
        let original = prepare_grid_shapes(
            &points,
            &[FeatureCombination {
                feature_ids: vec![0, 1, 2],
                score: 1.0,
                regions: vec![0],
            }],
            2.3,
            &config,
        )
        .unwrap();

        for (point, id) in points.iter_mut().zip([80, 2, 35]) {
            point.id = id;
            point.kind = FeatureKind::Bifurcation;
            point.priority = 0.2;
        }
        points.reverse();
        let relabeled = prepare_grid_shapes(
            &points,
            &[FeatureCombination {
                feature_ids: vec![35, 80, 2],
                score: 0.4,
                regions: vec![8],
            }],
            2.3,
            &config,
        )
        .unwrap();
        let first = original.candidates(limit(4096)).next().unwrap().unwrap();
        let second = relabeled.candidates(limit(4096)).next().unwrap().unwrap();
        assert_eq!(first.as_bytes(), second.as_bytes());
    }

    #[test]
    fn stream_hashes_every_variant_in_its_own_phase_and_retains_duplicates() {
        let shapes = prepared(1);
        let candidates = shapes
            .candidates(limit(4096))
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(candidates.len() as u64, shapes.variant_count());
        for (candidate, variant) in candidates.iter().zip(shapes.variants()) {
            let variant = variant.unwrap();
            let phase = shapes.shapes()[variant.shape_index].phase;
            assert_eq!(
                candidate.as_bytes(),
                digest(&variant.positions, phase).as_bytes()
            );
        }

        let hashes: Vec<_> = candidates.iter().map(Candidate::as_bytes).collect();
        assert_eq!(hashes[..7], hashes[hashes.len() - 7..]);
    }

    #[test]
    fn stream_propagates_late_work_and_descriptor_failures_then_stops() {
        let mut shapes = prepared(1);
        // One feature for initialization, then two entries for its first placement.
        shapes.remaining_work = 3;
        let mut stream = shapes.candidates(limit(4096));
        assert!(stream.next().unwrap().is_ok());
        let error = stream.next().unwrap().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<GridShapeError>(),
            Some(GridShapeError::WorkLimit)
        ));
        assert!(stream.next().is_none());
        assert!(stream.next().is_none());

        let shapes = prepared(0);
        let mut stream = shapes.candidates(limit(81));
        assert!(stream.next().unwrap().is_ok());
        let error = stream.next().unwrap().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Error>(),
            Some(Error::LimitExceeded(Resource::CandidateBytes))
        ));
        assert!(stream.next().is_none());
    }
}
