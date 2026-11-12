"""Native feature evidence remains inspectable and rejects broken contracts."""

import copy
import unittest

import matplotlib.pyplot as plt
import numpy as np
from feature_fixture import feature_fixture
from PIL import Image
from test_structure import fixture

from gnorix_reports.feature_strength import validate_feature_strength
from gnorix_reports.feature_strength_render import candidate_colors, render_feature_strength


class FeatureStrengthTests(unittest.TestCase):
    def setUp(self):
        self.data = fixture()
        self.strength = feature_fixture(self.data)

    def validate(self, value=None):
        data = self.data
        return validate_feature_strength(
            value if value is not None else self.strength,
            data["nodes"],
            data["edges"],
            data["cycles"],
            data["width"],
            data["height"],
            data["scale"]["unit_px"],
            data["config"]["min_contrast"],
            data["config"]["min_coherence"],
        )

    def test_preserves_every_raw_candidate_and_distinguishes_unknown_from_zero(self):
        first, second = self.strength["candidates"][:2]
        first.update(source_support=0, strength=0)
        second.update(
            source_support=None,
            strength=None,
            source_coverage=0.5,
            reasons=["missing_source_support", "stress_sensitive"],
        )
        result = self.validate()
        self.assertEqual(len(result.candidates), len(self.data["nodes"]) + len(self.data["cycles"]))
        self.assertEqual(result.candidates[0].strength, 0)
        self.assertIsNone(result.candidates[1].strength)
        colors = candidate_colors(result.candidates[:2])
        self.assertFalse(np.array_equal(colors[0, :3], colors[1, :3]))
        np.testing.assert_allclose(colors[:, 3], 0.4)

    def test_rejects_omitted_duplicate_or_fabricated_raw_candidates(self):
        for mutation in (
            lambda v: v["candidates"].pop(),
            lambda v: v["candidates"].append(copy.deepcopy(v["candidates"][0])),
            lambda v: v["candidates"][1].update(id=0),
            lambda v: v["candidates"][0].update(source_node=999),
            lambda v: v["candidates"][0].update(source_node=10.0),
            lambda v: v["candidates"][0].update(source_cycle=7),
            lambda v: v["candidates"][1].update(kind="ending"),
            lambda v: v["candidates"][0].update(position=[4, 5]),
            lambda v: v["candidates"][-1].update(position=[20, 10]),
            lambda v: v["candidates"][0]["arms"].clear(),
        ):
            value = copy.deepcopy(self.strength)
            mutation(value)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.validate(value)

    def test_rejects_nonfinite_out_of_bounds_and_wrong_combined_evidence(self):
        for field, invalid in (
            ("source_support", float("nan")),
            ("geometry_support", 1.1),
            ("stability", True),
            ("strength", -0.1),
            ("strength", 0.6),
            ("strength", None),
            ("source_coverage", float("inf")),
            ("geometry_coverage", -1),
            ("source_contrast", 2),
        ):
            value = copy.deepcopy(self.strength)
            value["candidates"][0][field] = invalid
            with self.subTest(field=field, invalid=invalid), self.assertRaises(ValueError):
                self.validate(value)
        value = copy.deepcopy(self.strength)
        value["candidates"][0]["source_support"] = None
        with self.assertRaisesRegex(ValueError, "unknown Q"):
            self.validate(value)

    def test_validates_eight_exact_stress_cases_and_one_to_one_variant_ids(self):
        mutations = (
            lambda v: v["stress_cases"].pop(),
            lambda v: v["stress_cases"][0].update(directional_margin=0),
            lambda v: v["stress_cases"][0].update(min_coherence=0.1),
            lambda v: v["stress_cases"][0].update(node_count=True),
            lambda v: v["stress_cases"][0].update(cycle_count=10000),
            lambda v: v["candidates"][0]["stress_matches"].pop(),
            lambda v: v["candidates"][0]["stress_matches"].__setitem__(0, 999),
            lambda v: v["candidates"][1]["stress_matches"].__setitem__(0, 0),
            lambda v: v["candidates"][0]["stress_matches"].__setitem__(0, 5),
            lambda v: v["candidates"][0].update(stability=1),
        )
        for mutation in mutations:
            value = copy.deepcopy(self.strength)
            mutation(value)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.validate(value)

    def test_checks_declared_controls_reason_codes_and_arm_evidence(self):
        for mutation in (
            lambda v: v["policy"].update(reference_width_px=3),
            lambda v: v["policy"].update(contrast_reference=0.1),
            lambda v: v["stability_policy"].update(max_position_shift_px=5),
            lambda v: v["candidates"][0].update(reasons=["probably_real"]),
            lambda v: v["candidates"][0].update(reasons=["possible_continuation"] * 2),
            lambda v: v["candidates"][0]["arms"][0].update(edge=999),
            lambda v: v["candidates"][0]["arms"][0].update(length_px=5),
            lambda v: v["candidates"][0]["arms"][0].update(source_darkness=-0.2),
            lambda v: v["candidates"][0]["arms"][0].update(dark_fraction=None),
        ):
            value = copy.deepcopy(self.strength)
            mutation(value)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.validate(value)

    def test_stress_grid_remains_eight_distinct_cases_at_coherence_boundaries(self):
        for coherence in (0, 1):
            self.data["config"]["min_coherence"] = coherence
            value = feature_fixture(self.data)
            self.validate(value)
            self.assertEqual(
                len({(c["directional_margin"], c["min_coherence"]) for c in value["stress_cases"]}),
                8,
            )

    def test_cleanup_removed_candidates_remain_in_the_original_population(self):
        from test_cleanup import fixture as cleaned_fixture

        self.data, cleanup = cleaned_fixture()
        self.data.update(width=24, height=24, config={"min_contrast": 0.035, "min_coherence": 0.2})
        self.strength = feature_fixture(self.data)
        assessed = self.validate()
        retained_nodes = {node["source_node"] for node in cleanup["nodes"]}
        removed_nodes = {node["id"] for node in self.data["nodes"]} - retained_nodes
        self.assertEqual(len(removed_nodes), 2)
        self.assertTrue(
            removed_nodes <= {candidate.source_node for candidate in assessed.candidates}
        )

    def test_partial_source_and_bounded_light_gap_are_required(self):
        for change in (
            lambda v: v["candidates"][0].update(source_coverage=0.9),
            lambda v: v["candidates"][0]["arms"][0].update(longest_light_gap_px=5),
            lambda v: v["candidates"][0]["arms"][0].update(longest_light_gap_px=None),
            lambda v: v["candidates"][0].update(geometry_separation=1.1),
        ):
            value = copy.deepcopy(self.strength)
            change(value)
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.validate(value)

    def test_renders_all_candidates_fixed_scale_and_source_without_modification(self):
        result = self.validate()
        source = Image.fromarray(np.arange(32 * 24, dtype=np.uint8).reshape(24, 32))
        self.addCleanup(source.close)
        figure = render_feature_strength(result, source)
        self.addCleanup(plt.close, figure)
        axis = figure.axes[0]
        np.testing.assert_array_equal(axis.images[0].get_array(), np.asarray(source))
        self.assertEqual(axis.images[0].get_alpha(), 1)
        self.assertEqual(
            sum(len(c.get_offsets()) for c in axis.collections), len(result.candidates)
        )
        for collection in axis.collections:
            np.testing.assert_allclose(collection.get_facecolors()[:, 3], 0.4 + 0.5 * np.sqrt(0.45))
        text = " ".join(t.get_text() for a in figure.axes for t in a.texts)
        self.assertIn("All 6 original", axis.get_title())
        self.assertIn("P=0.600", " ".join(a.get_xlabel() for a in figure.axes))
        self.assertNotIn("probability=", text)


if __name__ == "__main__":
    unittest.main()
