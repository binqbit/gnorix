"""Explicit synthetic native feature DTOs for report contract tests."""

import math


def feature_fixture(data):
    unit = data["scale"]["unit_px"]
    count = len(data["nodes"]) + len(data["cycles"])
    coherence = data["config"]["min_coherence"]
    delta = max(0.02, 0.1 * coherence)
    low, high = max(0, coherence - delta), min(1, coherence + delta)
    if low == coherence:
        low = min(1, coherence + 2 * delta)
    if high == coherence:
        high = max(0, coherence - 2 * delta)
    cases = [
        {
            "directional_margin": margin,
            "min_coherence": threshold,
            "node_count": len(data["nodes"]),
            "cycle_count": len(data["cycles"]),
        }
        for margin in (-0.05, 0, 0.05)
        for threshold in sorted((low, coherence, high))
        if not (margin == 0 and threshold == coherence)
    ]
    candidates = []
    for identifier in range(count):
        node = data["nodes"][identifier] if identifier < len(data["nodes"]) else None
        cycle = None if node is not None else data["cycles"][identifier - len(data["nodes"])]
        arms = []
        for edge in data["edges"]:
            for end in ("start", "end"):
                if node is not None and edge[end] == node["id"]:
                    length = sum(
                        math.dist(a, b) for a, b in zip(edge["points"], edge["points"][1:])
                    )
                    arms.append(
                        {
                            "edge": edge["id"],
                            "length_px": min(length, 2 * unit),
                            "source_darkness": 0.8,
                            "dark_fraction": 0.75,
                            "source_contrast": 0.4,
                            "longest_light_gap_px": 0,
                        }
                    )
        kind = (
            "cycle"
            if node is None
            else ("bifurcation" if node["kind"] == "junction" and len(arms) == 3 else node["kind"])
        )
        if node is not None:
            position = [node["x"], node["y"]]
        else:
            points = cycle["points"]
            total = x = y = 0
            for a, b in zip(points, points[1:] + points[:1], strict=True):
                length = math.dist(a, b)
                total += length
                x += 0.5 * (a[0] + b[0]) * length
                y += 0.5 * (a[1] + b[1]) * length
            position = [x / total, y / total]
        candidates.append(
            {
                "id": identifier,
                "kind": kind,
                "source_node": node["id"] if node else None,
                "source_cycle": cycle["id"] if cycle else None,
                "position": position,
                "source_support": 0.6,
                "geometry_support": 0.8,
                "stability": 0.5,
                "strength": 0.45,
                "source_coverage": 1,
                "geometry_coverage": 1,
                "source_contrast": 0.4,
                "arms": arms,
                "separation": None,
                "geometry_separation": None,
                "termination": None,
                "continuation": None,
                "reasons": ["stress_sensitive"],
                "stress_matches": [identifier] * 4 + [None] * 4,
            }
        )
    return {
        "method": "source-geometry-stress-v1",
        "score": "min(P,G) * (0.5 + 0.5*T)",
        "policy": {
            "reference_width_px": unit,
            "branch_horizon_px": 2 * unit,
            "tangent_support_px": unit / 2,
            "sample_spacing_px": 0.5,
            "source_search_px": 1.2 * unit,
            "continuation_search_px": 3 * unit,
            "terminal_barrier_width_px": unit,
            "terminal_barrier_depth_px": unit / 4,
            "contrast_reference": data["config"]["min_contrast"],
        },
        "stability_policy": {
            "max_position_shift_px": unit,
            "arm_radius_px": unit,
            "max_arm_angle_radians": math.pi / 6,
            "max_arm_shape_error_px": unit / 4,
            "max_isolated_shift_px": unit / 4,
            "cycle_relative_tolerance": 0.2,
        },
        "stress_cases": cases,
        "candidates": candidates,
    }
