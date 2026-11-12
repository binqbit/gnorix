"""Final markers follow cleanup and joins without inventing native evidence."""

import unittest

from feature_fixture import feature_fixture
from test_cleanup import confirm_spur
from test_cleanup import fixture as spur_fixture
from test_cleanup_sources import loop_fixture
from test_contours import fixture as join_fixture
from test_structure import unchanged_contours

from gnorix_reports.cleanup import validate_cleanup
from gnorix_reports.contours import validate_contours
from gnorix_reports.feature_strength import validate_feature_strength
from gnorix_reports.structure_features import project_final_structure


def project(raw, cleaned, final=None):
    unit = raw["scale"]["unit_px"]
    cleanup = validate_cleanup(cleaned, raw["nodes"], raw["edges"], raw["cycles"], 40, 32, unit)
    if final is None:
        final = unchanged_contours(
            {
                "scale": raw["scale"],
                "nodes": cleanup.nodes,
                "edges": cleanup.edges,
                "cycles": cleanup.cycles,
            }
        )
    contours = validate_contours(
        final,
        cleanup.nodes,
        cleanup.edges,
        cleanup.cycles,
        40,
        32,
        unit,
        cleanup_width=cleanup.width,
    )
    raw["config"] = {"min_contrast": 0.035, "min_coherence": 0.2}
    strength = validate_feature_strength(
        feature_fixture(raw), raw["nodes"], raw["edges"], raw["cycles"], 40, 32, unit, 0.035, 0.2
    )
    return project_final_structure(strength, cleanup, contours, final["edges"], 0), strength


class FinalStructureTests(unittest.TestCase):
    def test_removed_spurs_and_duplicate_loop_nodes_do_not_reappear(self):
        for factory in (spur_fixture, loop_fixture):
            with self.subTest(factory=factory.__name__):
                raw, cleaned = factory()
                final, original = project(raw, cleaned)
                self.assertEqual(len(original.candidates), 4)
                self.assertEqual(len(final.markers), 2)
                self.assertTrue(all(marker.kind == "ending" for marker in final.markers))
                original_priority = {item.position: item.priority for item in original.candidates}
                self.assertTrue(
                    all(
                        marker.priority == original_priority[marker.position]
                        for marker in final.markers
                    )
                )
                self.assertEqual(
                    {marker.position for marker in final.markers},
                    {(node["x"], node["y"]) for node in cleaned["nodes"]},
                )

    def test_accepted_gap_consumes_only_its_two_endings(self):
        for accepted, expected in ((False, 4), (True, 2)):
            with self.subTest(accepted=accepted):
                raw, contours = join_fixture(accepted)
                final, original = project(raw, raw["cleanup"], contours)
                self.assertEqual(len(original.candidates), 4)
                self.assertEqual(len(final.markers), expected)
                if accepted:
                    self.assertEqual(
                        {marker.position for marker in final.markers}, {(3, 15), (31, 15)}
                    )

    def test_changed_junction_degree_is_unassessed_instead_of_reusing_original_q(self):
        raw, _ = spur_fixture()
        raw["nodes"].append(
            {"id": 50, "x": 10, "y": 15, "kind": "ending", "touches_unobserved": False}
        )
        raw["edges"].append(
            {"id": 15, "start": 20, "end": 50, "points": [[10, y] for y in range(10, 16)]}
        )
        unchanged_contours(raw)
        cleaned = raw["cleanup"]
        cleaned["nodes"] = [node for node in cleaned["nodes"] if node["id"] != 40]
        cleaned["edges"] = [edge for edge in cleaned["edges"] if edge["id"] != 7]
        cleaned["removed_spurs"] = [
            {"source_edge": 7, "attachment_node": 20, "terminal_node": 40, "arc_length_px": 2}
        ]
        confirm_spur(cleaned, 7)
        final, original = project(raw, cleaned)
        old = next(candidate for candidate in original.candidates if candidate.source_node == 20)
        marker = next(marker for marker in final.markers if marker.position == (10, 10))
        self.assertEqual((old.kind, old.strength), ("junction", 0.45))
        self.assertEqual(marker.kind, "bifurcation")
        self.assertIsNone(marker.priority)
        self.assertTrue(marker.unassessed)
        self.assertEqual(len(final.markers), 4)

    def test_new_cycle_from_gap_is_geometry_without_an_invented_feature_score(self):
        raw = {
            "scale": {"unit_px": 4},
            "nodes": [
                {"id": i, "x": x, "y": 10, "kind": "ending", "touches_unobserved": False}
                for i, x in ((10, 10), (20, 13))
            ],
            "edges": [
                {
                    "id": 5,
                    "start": 10,
                    "end": 20,
                    "points": [[10, 10], [10, 20], [20, 20], [13, 10]],
                }
            ],
            "cycles": [],
        }
        contours = unchanged_contours(raw)
        join = join_fixture()[1]["joins"][0]
        join.update(start_node=20, end_node=10, points=[[13, 10], [12, 10], [11, 10], [10, 10]])
        contours.update(
            nodes=[],
            edges=[],
            joins=[join],
            cycles=[
                {
                    "id": 3,
                    "points": [*raw["edges"][0]["points"], [12, 10], [11, 10]],
                    "segment_clarity": [1] * 6,
                    "source_edges": [5],
                    "source_cycles": [],
                    "source_joins": [7],
                }
            ],
        )
        final, original = project(raw, raw["cleanup"], contours)
        self.assertEqual(len(original.candidates), 2)
        self.assertEqual(final.markers, ())
        self.assertEqual(final.unassessed_cycles, 1)
