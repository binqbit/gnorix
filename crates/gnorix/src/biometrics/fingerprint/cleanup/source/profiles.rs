//! Source-normal profiles, ridge boundaries and separating valleys.

use super::sample_darkness;
use super::sampling::{Path, direction, distance, lerp, offset, steps};
use super::{
    Capture, FingerprintError, LocalPalette, LoopEvidence, Point, SourceDecision, SourcePolicy,
};

pub(super) struct Section {
    pub(super) interior_darkness: f64,
    pub(super) valley_lightness: f64,
    pub(super) flank_lightness: f64,
    pub(super) arm_darkness: [f64; 2],
    pub(super) center_distance: [f64; 2],
}

pub(super) fn section(
    capture: &Capture,
    palette: &LocalPalette,
    a: Point,
    b: Point,
    normal: [f64; 2],
    policy: SourcePolicy,
) -> Result<Option<Section>, FingerprintError> {
    let middle = lerp(a, b, 0.5);
    let signed_a = (a.x() - middle.x()) * normal[0] + (a.y() - middle.y()) * normal[1];
    let signed_b = -signed_a;
    let (low, high) = (signed_a.min(signed_b), signed_a.max(signed_b));
    let width = high - low;
    let count = steps(width, policy.sample_spacing_px)?;
    let mut interior = 0.0;
    let mut interior_profile = Vec::with_capacity(count + 1);
    for index in 0..=count {
        let position = low + width * index as f64 / count as f64;
        let point = offset(middle, normal, position, [0.0, 0.0], 0.0);
        let Some(darkness) = sample_darkness(capture, palette, point) else {
            return Ok(None);
        };
        let weight = if index == 0 || index == count {
            0.5
        } else {
            1.0
        };
        interior += darkness * weight / count as f64;
        interior_profile.push(darkness);
    }
    let Some((left, left_dark)) =
        light_boundary(capture, palette, middle, normal, low, -1.0, policy)?
    else {
        return Ok(None);
    };
    let Some((right, right_dark)) =
        light_boundary(capture, palette, middle, normal, high, 1.0, policy)?
    else {
        return Ok(None);
    };
    let profile_start = left;
    let profile_width = right - left;
    let count = steps(profile_width, policy.sample_spacing_px)?;
    let (mut mass, mut moment) = (0.0, 0.0);
    for index in 0..=count {
        let position = profile_start + profile_width * index as f64 / count as f64;
        let Some(darkness) = sample_darkness(
            capture,
            palette,
            offset(middle, normal, position, [0.0, 0.0], 0.0),
        ) else {
            return Ok(None);
        };
        let weight = (darkness - policy.dark_midpoint).max(0.0)
            * if index == 0 || index == count {
                0.5
            } else {
                1.0
            };
        mass += weight;
        moment += weight * position;
    }
    let (Some(a_dark), Some(b_dark)) = (
        sample_darkness(capture, palette, a),
        sample_darkness(capture, palette, b),
    ) else {
        return Ok(None);
    };
    let center = if mass > 0.0 { moment / mass } else { 0.0 };
    Ok(Some(Section {
        interior_darkness: interior,
        // Only light enclosed by observed dark masses is a separating valley.
        // An outer bright shoulder of a false arm is not a second ridge.
        valley_lightness: enclosed_valley(&interior_profile, policy.dark_midpoint),
        flank_lightness: 1.0 - left_dark.max(right_dark),
        arm_darkness: [a_dark, b_dark],
        center_distance: [(signed_a - center).abs(), (signed_b - center).abs()],
    }))
}

fn light_boundary(
    capture: &Capture,
    palette: &LocalPalette,
    middle: Point,
    normal: [f64; 2],
    start: f64,
    sign: f64,
    policy: SourcePolicy,
) -> Result<Option<(f64, f64)>, FingerprintError> {
    let steps = steps(policy.loop_flank_search_px, policy.sample_spacing_px)?;
    for index in 0..=steps {
        let position = start + sign * policy.loop_flank_search_px * index as f64 / steps as f64;
        let Some(darkness) = sample_darkness(
            capture,
            palette,
            offset(middle, normal, position, [0.0, 0.0], 0.0),
        ) else {
            return Ok(None);
        };
        if darkness < policy.dark_midpoint || index == steps {
            return Ok(Some((position, darkness)));
        }
    }
    unreachable!("the finite boundary search always visits its final station")
}

fn enclosed_valley(profile: &[f64], midpoint: f64) -> f64 {
    let Some(first_dark) = profile.iter().position(|&dark| dark > midpoint) else {
        return 0.0;
    };
    let Some(last_dark) = profile.iter().rposition(|&dark| dark > midpoint) else {
        return 0.0;
    };
    if first_dark == last_dark {
        return 0.0;
    }
    profile[first_dark..=last_dark]
        .iter()
        .filter(|&&dark| dark < midpoint)
        .map(|&dark| 1.0 - dark)
        .fold(0.0, f64::max)
}

pub(super) fn common_normal(
    first: &Path<'_>,
    second: &Path<'_>,
    fraction: f64,
    policy: SourcePolicy,
) -> Option<[f64; 2]> {
    let mean_length = (first.length() + second.length()) / 2.0;
    let delta = policy.tangent_radius_px / mean_length;
    let midpoint = |at: f64| {
        lerp(
            first.at(at * first.length()),
            second.at(at * second.length()),
            0.5,
        )
    };
    let before = midpoint((fraction - delta).max(0.0));
    let center = midpoint(fraction);
    let after = midpoint((fraction + delta).min(1.0));
    // Unequal arm lengths can move the paired midpoint slightly backward at a
    // raster port before it advances. Such motion below the source sampling
    // resolution cannot establish a reversal; source profiles must still prove
    // the ridge. A resolved turn on both sides remains ambiguous geometry.
    if distance(before, center) >= policy.sample_spacing_px
        && distance(center, after) >= policy.sample_spacing_px
        && let (Some(incoming), Some(outgoing)) =
            (direction(before, center), direction(center, after))
        && incoming[0] * outgoing[0] + incoming[1] * outgoing[1] < 0.0
    {
        return None;
    }
    let tangent = direction(before, after)?;
    Some([-tangent[1], tangent[0]])
}

pub(super) fn summarize_loop(
    sections: &[Section],
    positions: &[f64],
    width: f64,
    policy: SourcePolicy,
) -> LoopEvidence {
    let length = positions.last().unwrap() - positions[0];
    let (mut mean, mut arms, mut center) = (0.0, [0.0; 2], [0.0; 2]);
    let (mut valley_max, mut flank_min) = (f64::NEG_INFINITY, f64::INFINITY);
    let (mut run, mut longest) = (0.0_f64, 0.0_f64);
    for (index, section) in sections.iter().enumerate() {
        let before = if index == 0 {
            0.0
        } else {
            positions[index] - positions[index - 1]
        };
        let after = if index + 1 == positions.len() {
            0.0
        } else {
            positions[index + 1] - positions[index]
        };
        let weight = (before + after) / (2.0 * length);
        mean += section.interior_darkness * weight;
        for arm in 0..2 {
            arms[arm] += section.arm_darkness[arm] * weight;
            center[arm] += section.center_distance[arm] * weight;
        }
        valley_max = valley_max.max(section.valley_lightness);
        flank_min = flank_min.min(section.flank_lightness);
        if index > 0 {
            let previous = sections[index - 1].valley_lightness;
            let current = section.valley_lightness;
            let above = |value: f64| value > policy.dark_midpoint;
            match (above(previous), above(current)) {
                (true, true) => run += before,
                (false, true) => {
                    run = before * (current - policy.dark_midpoint) / (current - previous)
                }
                (true, false) => {
                    run += before * (previous - policy.dark_midpoint) / (previous - current);
                    longest = longest.max(run);
                    run = 0.0;
                }
                (false, false) => run = 0.0,
            }
            longest = longest.max(run);
        }
    }
    let supported = mean >= policy.dark_midpoint && flank_min > policy.dark_midpoint;
    let confirmed = supported && longest < policy.loop_min_valley_length_px;
    // Equal scores have no unique source-supported representative. Retain the
    // loop instead of introducing an edge-ID or shortest-path preference.
    let error = |values: [f64; 2]| {
        128.0 * f64::EPSILON * sections.len() as f64 * values[0].abs().max(values[1].abs()).max(1.0)
    };
    let preference = if center[1] - center[0] > error(center) {
        Some(0)
    } else if center[0] - center[1] > error(center) {
        Some(1)
    } else if arms[0] - arms[1] > error(arms) {
        Some(0)
    } else if arms[1] - arms[0] > error(arms) {
        Some(1)
    } else {
        None
    };
    LoopEvidence {
        decision: if confirmed && preference.is_some() {
            SourceDecision::Confirmed
        } else if confirmed {
            SourceDecision::AmbiguousGeometry
        } else if longest >= policy.loop_min_valley_length_px {
            SourceDecision::SeparateRidges
        } else {
            SourceDecision::ImageUnsupported
        },
        max_width_px: Some(width),
        mean_interior_darkness: Some(mean),
        max_valley_lightness: Some(valley_max),
        longest_valley_length_px: Some(longest),
        min_flank_lightness: Some(flank_min),
        arm_darkness: Some(arms),
        arm_center_distance_px: Some(center),
        preferred_arm: confirmed.then_some(preference).flatten(),
    }
}
