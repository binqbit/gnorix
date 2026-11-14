//! Deterministic topology-preserving thinning of binary ridge maps.
//!
//! Uses Zhang & Suen (1984), doi:10.1145/357994.358023, with a safeguard
//! against erasing an entire foreground component and local boundary provenance.

use super::{FingerprintError, RidgeMap};

pub(super) fn thin(map: &RidgeMap) -> Result<RidgeMap, FingerprintError> {
    let mut ridges = map.ridges.clone();
    let mut boundary_origin = map.boundary_origin.clone();
    let mut removed = Vec::new();
    let mut deletion_state = vec![0_u8; ridges.len()];
    let mut pending = Vec::new();
    let mut boundary_owners = vec![0_usize; ridges.len()];
    let iteration_limit = map
        .width
        .checked_add(map.height)
        .ok_or(FingerprintError::WorkLimit)?;
    for _ in 0..iteration_limit {
        let mut changed = false;
        for second_pass in [false, true] {
            removed.clear();
            for y in 0..map.height {
                for x in 0..map.width {
                    let index = y * map.width + x;
                    if !ridges[index] {
                        continue;
                    }
                    let n = neighbors(&ridges, map.width, map.height, x, y);
                    let count = n.iter().filter(|&&pixel| pixel).count();
                    let transitions = (0..8).filter(|&i| !n[i] && n[(i + 1) % 8]).count();
                    let directional = if second_pass {
                        !(n[0] && n[2] && n[6]) && !(n[0] && n[4] && n[6])
                    } else {
                        !(n[0] && n[2] && n[4]) && !(n[2] && n[4] && n[6])
                    };
                    if (2..=6).contains(&count) && transitions == 1 && directional {
                        removed.push(index);
                    }
                }
            }
            protect_nonempty_components(
                &ridges,
                map.width,
                map.height,
                &removed,
                &mut deletion_state,
                &mut pending,
            );
            transfer_boundary_origins(
                &ridges,
                (map.width, map.height),
                &removed,
                &mut deletion_state,
                &mut boundary_origin,
                &mut pending,
                &mut boundary_owners,
            )?;
            for &index in &removed {
                if deletion_state[index] != 0 {
                    ridges[index] = false;
                    boundary_origin[index] = false;
                    changed = true;
                }
                deletion_state[index] = 0;
            }
        }
        if !changed {
            let mut output = RidgeMap::new(map.width, map.height, ridges, map.usable.clone())?;
            output.boundary_origin = boundary_origin;
            return Ok(output);
        }
    }
    Err(FingerprintError::WorkLimit)
}

/// A boundary point can move inward while a thick ridge is thinned. Transfer
/// its provenance to its nearest retained foreground support, measured along
/// the deleted pixels. A multi-source breadth-first traversal keeps provenance
/// local instead of spreading it along an entire deleted boundary strip.
/// Equal-distance ties prefer the first row-major surviving pixel.
fn transfer_boundary_origins(
    ridges: &[bool],
    dimensions: (usize, usize),
    removed: &[usize],
    state: &mut [u8],
    origins: &mut [bool],
    pending: &mut Vec<usize>,
    owners: &mut [usize],
) -> Result<(), FingerprintError> {
    let (width, height) = dimensions;
    pending.clear();

    // Seed each deleted boundary pixel with its nearest adjacent survivor.
    // The previous component safeguard guarantees at least one seed for every
    // deleted region. Owners always refer to pixels retained in this phase.
    for &index in removed {
        if state[index] != 2 {
            continue;
        }
        let owner = neighbor_indices(width, height, index % width, index / width)
            .into_iter()
            .flatten()
            .filter(|&next| ridges[next] && state[next] == 0)
            .min();
        if let Some(owner) = owner {
            owners[index] = owner;
            state[index] = 3;
            pending.push(index);
        }
    }
    pending.sort_unstable_by_key(|&index| (owners[index], index));

    let mut cursor = 0;
    while cursor < pending.len() {
        let index = pending[cursor];
        for next in neighbor_indices(width, height, index % width, index / width)
            .into_iter()
            .flatten()
        {
            if state[next] == 2 {
                owners[next] = owners[index];
                state[next] = 3;
                pending.push(next);
            }
        }
        cursor += 1;
    }

    if removed.iter().any(|&index| state[index] == 2) {
        return Err(FingerprintError::InvalidTopology(
            "deleted ridge region has no retained support",
        ));
    }

    // Read only deleted pixels' original flags. Writing retained owners cannot
    // feed a newly assigned flag back into this phase's provenance traversal.
    for &index in pending.iter() {
        if origins[index] {
            origins[owners[index]] = true;
        }
    }
    Ok(())
}

/// Parallel Zhang-Suen can remove every pixel of an isolated 2x2 block.
/// Retain its first row-major pixel, or the first pixel of any other component
/// wholly selected for deletion. Components with any survivor are unchanged.
fn protect_nonempty_components(
    ridges: &[bool],
    width: usize,
    height: usize,
    removed: &[usize],
    state: &mut [u8],
    pending: &mut Vec<usize>,
) {
    for &index in removed {
        state[index] = 1;
    }
    for &start in removed {
        if state[start] != 1 {
            continue;
        }
        pending.clear();
        pending.push(start);
        state[start] = 2;
        let mut first = start;
        let mut has_survivor = false;
        while let Some(index) = pending.pop() {
            first = first.min(index);
            for next in neighbor_indices(width, height, index % width, index / width)
                .into_iter()
                .flatten()
            {
                if ridges[next] {
                    if state[next] == 0 {
                        has_survivor = true;
                    } else if state[next] == 1 {
                        state[next] = 2;
                        pending.push(next);
                    }
                }
            }
        }
        if !has_survivor {
            state[first] = 0;
        }
    }
}

fn neighbors(ridges: &[bool], width: usize, height: usize, x: usize, y: usize) -> [bool; 8] {
    neighbor_indices(width, height, x, y).map(|index| index.is_some_and(|i| ridges[i]))
}

fn neighbor_indices(width: usize, height: usize, x: usize, y: usize) -> [Option<usize>; 8] {
    // Clockwise from north, matching the Zhang-Suen p2..p9 convention.
    [
        (0, -1),
        (1, -1),
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
    ]
    .map(|(dx, dy)| {
        let nx = x.checked_add_signed(dx)?;
        let ny = y.checked_add_signed(dy)?;
        (nx < width && ny < height).then_some(ny * width + nx)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinning_retains_a_closed_ring_and_disjoint_components() {
        let (width, height) = (31, 31);
        let mut pixels = vec![false; width * height];
        for y in 0..height {
            for x in 0..width {
                let distance = (x as f64 - 15.0).hypot(y as f64 - 15.0);
                pixels[y * width + x] = (7.0..=10.0).contains(&distance);
            }
        }
        pixels[1] = true;
        let map = RidgeMap::new(width, height, pixels, vec![true; width * height]).unwrap();
        let skeleton = thin(&map).unwrap();
        assert!(skeleton.ridges[1]);
        assert!(!skeleton.ridges[15 * width + 15]);
        assert!(skeleton.ridges.iter().filter(|&&v| v).count() >= 35);
        assert!(
            skeleton
                .ridges
                .iter()
                .enumerate()
                .all(|(i, &v)| !v || map.ridges[i])
        );
        // Each ring pixel still has at least two neighbors: no loop was opened.
        for y in 3..28 {
            for x in 3..28 {
                if skeleton.ridges[y * width + x] {
                    assert!(
                        neighbors(&skeleton.ridges, width, height, x, y)
                            .iter()
                            .filter(|&&v| v)
                            .count()
                            >= 2
                    );
                }
            }
        }
        assert_eq!(thin(&skeleton).unwrap().ridges, skeleton.ridges);
    }

    #[test]
    fn thinning_preserves_one_pixel_endpoints_and_mask() {
        let (width, height) = (9, 7);
        let mut pixels = vec![false; width * height];
        for x in 1..8 {
            pixels[3 * width + x] = true;
        }
        let mut mask = vec![true; width * height];
        mask[0] = false;
        let map = RidgeMap::new(width, height, pixels, mask).unwrap();
        let skeleton = thin(&map).unwrap();
        assert_eq!(skeleton.ridges, map.ridges);
        assert_eq!(skeleton.usable, map.usable);
    }

    #[test]
    fn thinning_retains_boundary_provenance_when_a_clipped_end_retracts() {
        let (width, height) = (9, 8);
        for invalid_column in [true, false] {
            let left = usize::from(invalid_column);
            let mut pixels = vec![false; width * height];
            for y in 3..5 {
                for x in left..7 {
                    pixels[y * width + x] = true;
                }
            }
            let mut usable = vec![true; width * height];
            if invalid_column {
                for y in 0..height {
                    usable[y * width] = false;
                }
            }
            let map = RidgeMap::new(width, height, pixels, usable).unwrap();
            assert!(map.boundary_origin[3 * width + left]);
            let output = thin(&map).unwrap();
            let first = (0..output.ridges.len())
                .filter(|&i| output.ridges[i])
                .min_by_key(|&i| i % width)
                .unwrap();
            assert!(
                first % width > left,
                "the regression requires an actual retraction"
            );
            assert!(output.boundary_origin[first]);
            assert_eq!(output.usable, map.usable);
            assert!(
                output
                    .boundary_origin
                    .iter()
                    .enumerate()
                    .all(|(i, &v)| !v || output.ridges[i])
            );
            assert_eq!(
                thin(&output).unwrap().boundary_origin,
                output.boundary_origin
            );
        }
    }

    #[test]
    fn thinning_does_not_invent_boundary_provenance_for_interior_components() {
        let (width, height) = (9, 8);
        let mut pixels = vec![false; width * height];
        for y in 3..5 {
            for x in 2..7 {
                pixels[y * width + x] = true;
            }
        }
        let map = RidgeMap::new(width, height, pixels, vec![true; width * height]).unwrap();
        assert!(map.boundary_origin.iter().all(|&v| !v));
        assert!(thin(&map).unwrap().boundary_origin.iter().all(|&v| !v));
    }

    #[test]
    fn clipped_end_provenance_does_not_reach_the_other_end_of_a_long_ridge() {
        let (width, height) = (64, 9);
        let mut pixels = vec![false; width * height];
        for y in 3..5 {
            for x in 1..61 {
                pixels[y * width + x] = true;
            }
        }
        let mut usable = vec![true; width * height];
        for y in 0..height {
            usable[y * width] = false;
        }
        let map = RidgeMap::new(width, height, pixels, usable).unwrap();
        let output = thin(&map).unwrap();
        let ridge_pixels: Vec<_> = (0..output.ridges.len())
            .filter(|&index| output.ridges[index])
            .collect();
        let first = *ridge_pixels.iter().min_by_key(|&&i| i % width).unwrap();
        let last = *ridge_pixels.iter().max_by_key(|&&i| i % width).unwrap();

        assert!(first % width > 1, "the clipped end must actually retract");
        assert!(output.boundary_origin[first]);
        assert!(!output.boundary_origin[last]);
        assert!(ridge_pixels.iter().all(|&index| {
            !output.boundary_origin[index] || index % width <= first % width + 1
        }));
        assert_eq!(output.usable, map.usable);
        let repeated = thin(&output).unwrap();
        assert_eq!(repeated.ridges, output.ridges);
        assert_eq!(repeated.boundary_origin, output.boundary_origin);
    }

    #[test]
    fn equally_near_boundary_support_prefers_the_first_surviving_pixel() {
        let (width, height) = (5, 3);
        let (left, middle, right) = (width + 1, width + 2, width + 3);
        let mut ridges = vec![false; width * height];
        for index in [left, middle, right] {
            ridges[index] = true;
        }
        let mut origins = vec![false; ridges.len()];
        origins[middle] = true;
        let mut state = vec![0; ridges.len()];
        state[middle] = 2;
        let mut owners = vec![0; ridges.len()];
        transfer_boundary_origins(
            &ridges,
            (width, height),
            &[middle],
            &mut state,
            &mut origins,
            &mut Vec::new(),
            &mut owners,
        )
        .unwrap();
        assert!(origins[left]);
        assert!(!origins[right]);
    }

    fn components(pixels: &[bool], width: usize, height: usize, diagonal: bool) -> usize {
        let mut visited = vec![false; pixels.len()];
        let mut count = 0;
        let mut pending = Vec::new();
        for start in 0..pixels.len() {
            if !pixels[start] || visited[start] {
                continue;
            }
            count += 1;
            pending.push(start);
            visited[start] = true;
            while let Some(index) = pending.pop() {
                for (direction, next) in
                    neighbor_indices(width, height, index % width, index / width)
                        .into_iter()
                        .enumerate()
                {
                    if !diagonal && direction % 2 != 0 {
                        continue;
                    }
                    if let Some(next) = next
                        && pixels[next]
                        && !visited[next]
                    {
                        visited[next] = true;
                        pending.push(next);
                    }
                }
            }
        }
        count
    }

    fn background_components(pixels: &[bool], width: usize, height: usize) -> usize {
        let (padded_width, padded_height) = (width + 2, height + 2);
        let mut background = vec![true; padded_width * padded_height];
        for y in 0..height {
            for x in 0..width {
                background[(y + 1) * padded_width + x + 1] = !pixels[y * width + x];
            }
        }
        components(&background, padded_width, padded_height, false)
    }

    #[test]
    fn thinning_keeps_a_deterministic_pixel_in_each_isolated_two_by_two_block() {
        let (width, height) = (8, 5);
        let mut pixels = vec![false; width * height];
        for left in [1, 5] {
            for y in 1..3 {
                for x in left..left + 2 {
                    pixels[y * width + x] = true;
                }
            }
        }
        let map = RidgeMap::new(width, height, pixels, vec![true; width * height]).unwrap();
        let output = thin(&map).unwrap();
        assert_eq!(output.ridges.iter().filter(|&&v| v).count(), 2);
        assert!(output.ridges[width + 1]);
        assert!(output.ridges[width + 5]);
        assert_eq!(components(&output.ridges, width, height, true), 2);
    }

    #[test]
    fn every_three_by_three_pattern_retains_components_and_holes() {
        for pattern in 0..512 {
            let pixels: Vec<_> = (0..9).map(|bit| pattern & (1 << bit) != 0).collect();
            let map = RidgeMap::new(3, 3, pixels, vec![true; 9]).unwrap();
            let output = thin(&map).unwrap();
            assert_eq!(
                components(&map.ridges, 3, 3, true),
                components(&output.ridges, 3, 3, true),
                "foreground components for pattern {pattern:09b}",
            );
            assert_eq!(
                background_components(&map.ridges, 3, 3),
                background_components(&output.ridges, 3, 3),
                "background components for pattern {pattern:09b}",
            );
            assert!(
                output
                    .ridges
                    .iter()
                    .enumerate()
                    .all(|(i, &v)| !v || map.ridges[i])
            );
        }
    }
}
