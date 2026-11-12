"""Validate native source-image cleanup decisions without reclassifying pixels."""

from dataclasses import dataclass

from .validation import finite_number, object_value, require_close

_DECISIONS = {
    "confirmed",
    "missing_support",
    "possible_continuation",
    "separate_ridges",
    "ambiguous_geometry",
    "image_unsupported",
}


@dataclass(frozen=True)
class CleanupWidth:
    relative_tolerance: float
    maximum_px: float


def validate_source_policy(value, unit):
    policy = object_value(value, "cleanup.policy.source")
    relative = finite_number(policy.get("relative_width_tolerance"), "relative_width_tolerance")
    maximum = finite_number(policy.get("max_ridge_width_px"), "max_ridge_width_px")
    require_close(relative, 0.2, "cleanup relative width tolerance")
    require_close(maximum, 1.2 * unit, "cleanup maximum ridge width")
    for name, expected in (
        ("dark_midpoint", 0.5),
        ("sample_spacing_px", 0.5),
        ("tangent_radius_px", unit / 2),
        ("max_backbone_extension_px", 2 * unit),
        ("terminal_lookahead_px", maximum),
        ("terminal_barrier_width_px", unit),
        ("terminal_barrier_depth_px", unit / 4),
        ("loop_flank_search_px", maximum),
        ("loop_min_valley_length_px", 1.0),
    ):
        require_close(
            finite_number(policy.get(name), name), expected, "cleanup source policy " + name
        )
    return CleanupWidth(relative, maximum)


def validate_spur_evidence(value, maximum):
    evidence = _evidence(value, "spur")
    for name in ("terminal_darkness", "min_barrier_lightness"):
        _optional_number(evidence, name)
    distance = _optional_number(evidence, "light_barrier_distance_px")
    if distance is not None and not 0 <= distance <= maximum:
        raise ValueError("terminal barrier distance must be within the source lookahead")
    if evidence["decision"] == "confirmed":
        _require_values(
            evidence,
            (
                "terminal_darkness",
                "light_barrier_distance_px",
                "min_barrier_lightness",
            ),
        )
        if evidence["min_barrier_lightness"] <= 0.5:
            raise ValueError("confirmed spur requires a light source barrier above midpoint")
    return evidence


def validate_loop_evidence(value, maximum):
    evidence = _evidence(value, "loop")
    scalar_names = (
        "max_width_px",
        "mean_interior_darkness",
        "max_valley_lightness",
        "min_flank_lightness",
        "longest_valley_length_px",
    )
    for name in scalar_names:
        number = _optional_number(evidence, name)
        if name.endswith("_px") and number is not None and number < 0:
            raise ValueError("loop source distances cannot be negative")
    for name in ("arm_darkness", "arm_center_distance_px"):
        if name not in evidence:
            raise ValueError("cleanup source evidence is missing " + name)
        pair = evidence.get(name)
        if pair is not None:
            if not isinstance(pair, list) or len(pair) != 2:
                raise ValueError("loop arm evidence must contain exactly two values")
            values = [finite_number(item, name) for item in pair]
            if name.endswith("_px") and min(values) < 0:
                raise ValueError("loop arm distances cannot be negative")
    if "preferred_arm" not in evidence:
        raise ValueError("cleanup source evidence is missing preferred_arm")
    preferred = evidence.get("preferred_arm")
    if preferred is not None and (type(preferred) is not int or preferred not in (0, 1)):
        raise ValueError("preferred loop arm must be index zero or one")
    if evidence["decision"] == "confirmed":
        _require_values(
            evidence, (*scalar_names, "arm_darkness", "arm_center_distance_px", "preferred_arm")
        )
        if not evidence["max_width_px"] < maximum:
            raise ValueError(
                "confirmed loop width must be strictly below the upper tolerance bound"
            )
        if evidence["mean_interior_darkness"] < 0.5 or evidence["min_flank_lightness"] <= 0.5:
            raise ValueError("confirmed loop needs a dark interior and light source flanks")
        if evidence["longest_valley_length_px"] >= 1:
            raise ValueError("confirmed loop cannot cross a resolved bright source valley")
    elif preferred is not None:
        raise ValueError("unconfirmed loop cannot prefer an arm")
    return evidence


def _evidence(value, kind):
    evidence = object_value(value, kind + " source evidence")
    decision = evidence.get("decision")
    if not isinstance(decision, str) or decision not in _DECISIONS:
        raise ValueError("unknown cleanup source decision")
    return evidence


def _optional_number(evidence, name):
    if name not in evidence:
        raise ValueError("cleanup source evidence is missing " + name)
    value = evidence[name]
    return None if value is None else finite_number(value, name)


def _require_values(evidence, names):
    if any(evidence.get(name) is None for name in names):
        raise ValueError("confirmed cleanup decision requires complete source evidence")
