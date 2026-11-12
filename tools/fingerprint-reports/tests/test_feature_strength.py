"""Native feature evidence remains inspectable and rejects broken contracts."""

import copy
import unittest
from types import SimpleNamespace

import matplotlib.pyplot as plt
import numpy as np
from feature_fixture import feature_fixture
from PIL import Image
from test_structure import fixture

from gnorix_reports.feature_strength import validate_feature_strength
from gnorix_reports.feature_strength_context import (
    Neighborhood,
    validate_context,
    validate_neighbor_positions,
)
from gnorix_reports.feature_strength_render import (
    candidate_colors,
    detail_examples,
    render_feature_strength,
)


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
        first.update(strength=0, priority=0)
        first["context"]["support"] = 0
        first["reasons"].append("unconfirmed_context")
        second.update(
            source_support=None,
            strength=None,
            priority=None,
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
            ("priority", -0.1),
            ("priority", None),
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
            lambda v: v["policy"].update(context_horizon_px=2),
            lambda v: v["policy"].update(neighborhood_radius_px=2),
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
            expected = {
                candidate.position: 0.4 + 0.5 * np.sqrt(candidate.priority)
                for candidate in result.candidates
            }
            for position, color in zip(collection.get_offsets(), collection.get_facecolors()):
                self.assertAlmostEqual(color[3], expected[tuple(position)])
        text = " ".join(t.get_text() for a in figure.axes for t in a.texts)
        self.assertIn("All 6 original", axis.get_title())
        self.assertIn("P=0.600", " ".join(a.get_xlabel() for a in figure.axes))
        self.assertIn("4R=0.667", " ".join(a.get_xlabel() for a in figure.axes))
        self.assertIn("combined continuation=0.100", " ".join(a.get_xlabel() for a in figure.axes))
        self.assertIn("forward=0.100; curved=0.050", " ".join(a.get_xlabel() for a in figure.axes))
        self.assertIn("priority=", " ".join(a.get_xlabel() for a in figure.axes))
        self.assertNotIn("probability=", text)

    def test_context_controls_quality_and_missing_context_stays_unavailable(self):
        first, second, third = self.strength["candidates"][:3]
        first.update(source_support=0, geometry_support=0, strength=0.45)
        first["context"]["support"] = 0.8
        first["priority"] = 0.45 * first["neighborhood"]["factor"]
        second.update(strength=0, priority=0)
        second["context"]["support"] = 0
        second["reasons"].append("unconfirmed_context")
        third.update(strength=None, priority=None)
        third["context"].update(support=None, support_2r=None, source_coverage=0.5)
        third["reasons"].append("missing_context_support")
        assessed = self.validate().candidates
        self.assertAlmostEqual(assessed[0].strength, 0.45)
        self.assertEqual(assessed[1].strength, 0)
        self.assertIsNone(assessed[2].strength)

    def test_neighborhood_changes_priority_without_changing_quality(self):
        candidate = self.strength["candidates"][0]
        original_quality = candidate["strength"]
        candidate["neighborhood"]["observed_fraction"] = 0
        candidate["neighborhood"]["factor"] = 0.5
        candidate["priority"] = original_quality / 2
        result = self.validate().candidates[0]
        self.assertEqual(result.strength, original_quality)
        self.assertEqual(result.priority, original_quality / 2)

    def test_rejects_malformed_context_and_neighborhood_evidence(self):
        mutations = (
            lambda c: c["context"].update(support=float("nan")),
            lambda c: c["context"].update(support_2r=1.1),
            lambda c: c["context"].update(continuation=float("nan")),
            lambda c: c["context"].update(continuation=-0.1),
            lambda c: c["context"].update(continuation=1.1),
            lambda c: c["context"].update(forward_continuation=float("nan")),
            lambda c: c["context"].update(curved_continuation=-0.1),
            lambda c: c["context"].update(curved_continuation=1.1),
            lambda c: c["context"].update(source_coverage=0.5),
            lambda c: c["context"].update(unknown=0),
            lambda c: c["context"]["arms"].clear(),
            lambda c: c["context"]["arms"][0].update(edge=999),
            lambda c: c["context"]["arms"][0].update(edges=[]),
            lambda c: c["context"]["arms"][0].update(edges=[999]),
            lambda c: c["context"]["arms"][0].update(edges=[0, 0]),
            lambda c: c["context"]["arms"][0].update(length_px=9),
            lambda c: c["context"]["arms"][0].update(supported_length_px=9),
            lambda c: c["context"]["arms"][0].update(longest_unsupported_px=9),
            lambda c: c["context"]["arms"][0].update(continuity=-0.1),
            lambda c: c["context"]["arms"][0].update(continuity=None),
            lambda c: c["context"]["arms"][0].update(ridge_separation=1.1),
            lambda c: c["context"]["arms"][0].update(stop="invented"),
            lambda c: c["context"]["arms"][0].update(stop=[]),
            lambda c: c["context"]["arms"][0].update(stop="missing_source"),
            lambda c: c["neighborhood"].update(neighbor_count=True),
            lambda c: c["neighborhood"].update(neighbor_count=6),
            lambda c: c["neighborhood"].update(weighted_count=-0.1),
            lambda c: c["neighborhood"].update(observed_fraction=1.1),
            lambda c: c["neighborhood"].update(factor=0.4),
            lambda c: c["neighborhood"].update(unknown=0),
            lambda c: c.update(priority=0.9),
        )
        for mutate in mutations:
            value = copy.deepcopy(self.strength)
            mutate(value["candidates"][0])
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                self.validate(value)

    def test_rejects_legacy_method_and_unknown_quality_with_known_priority(self):
        value = copy.deepcopy(self.strength)
        value["method"] = "source-geometry-stress-v1"
        with self.assertRaisesRegex(ValueError, "method"):
            self.validate(value)
        candidate = self.strength["candidates"][0]
        candidate["context"]["support"] = None
        candidate["strength"] = None
        with self.assertRaisesRegex(ValueError, "unknown feature priority"):
            self.validate()

    def test_neighbor_validation_includes_coincident_candidates_but_excludes_self_and_radius(self):
        candidates = [
            SimpleNamespace(id=i, position=point, kind=kind, neighborhood=None)
            for i, point, kind in (
                (0, (0, 0), "bifurcation"),
                (1, (0, 0), "isolated"),
                (2, (3, 0), "ending"),
                (3, (6, 0), "cycle"),
            )
        ]
        for candidate, count, weight in zip(candidates, (2, 2, 3, 1), (1.25, 1.25, 0.75, 0.25)):
            candidate.neighborhood = Neighborhood(count, weight, 1, 0.5 + 0.5 / (1 + weight))
        validate_neighbor_positions(candidates, 6)
        candidates[0].neighborhood = Neighborhood(3, 1.25, 1, 0.5 + 0.5 / 2.25)
        with self.assertRaisesRegex(ValueError, "neighbor count"):
            validate_neighbor_positions(candidates, 6)
        candidates[0].neighborhood = Neighborhood(2, 1, 1, 0.75)
        with self.assertRaisesRegex(ValueError, "weighted neighbor count"):
            validate_neighbor_positions(candidates, 6)

    def test_context_paths_follow_incidence_and_horizon_stops_require_full_extent(self):
        context = copy.deepcopy(self.strength["candidates"][0]["context"])
        arm = context["arms"][0]
        # The first edge may be traversed in reverse, and a self edge stays at its node.
        edges = {8: (20, 10), 9: (20, 30), 11: (20, 20), 100: (20, 50)}
        arm["edges"] = [8, 11, 9]
        validate_context(context, [8], edges, 10, 8, kind="ending")
        arm["edges"] = [8, 9, 100]
        with self.assertRaisesRegex(ValueError, "form a chain"):
            validate_context(context, [8], edges, 10, 8, kind="ending")
        arm.update(edges=[8], stop="horizon")
        with self.assertRaisesRegex(ValueError, "horizon stop length"):
            validate_context(context, [8], edges, 10, 8, kind="ending")
        arm["length_px"] = 8
        validate_context(context, [8], edges, 10, 8, kind="ending")

    def test_graded_ending_components_bound_both_context_scales_and_only_apply_to_endings(self):
        context = copy.deepcopy(self.strength["candidates"][0]["context"])
        context.update(support=0.5, support_2r=0.5, continuation=0.5, forward_continuation=0.5)
        edges = {8: (20, 10)}
        result = validate_context(context, [8], edges, 10, 8, kind="ending")
        self.assertEqual(result.continuation, 0.5)
        for field in ("support", "support_2r"):
            contradictory = copy.deepcopy(context)
            contradictory[field] = 0.6
            with (
                self.subTest(field=field),
                self.assertRaisesRegex(ValueError, "continuation evidence"),
            ):
                validate_context(contradictory, [8], edges, 10, 8, kind="ending")
        for kind in ("bifurcation", "junction", "cycle", "isolated"):
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, "only an ending"):
                validate_context(context, [8], edges, 10, 8, kind=kind)

        context.update(termination=0.3, support=0.3, support_2r=0.3)
        result = validate_context(context, [8], edges, 10, 8, kind="ending")
        self.assertEqual(result.support, 0.3)
        context["support_2r"] = 0.4
        with self.assertRaisesRegex(ValueError, "termination or continuation evidence"):
            validate_context(context, [8], edges, 10, 8, kind="ending")

    def test_combined_continuation_uses_the_stronger_measured_component(self):
        context = copy.deepcopy(self.strength["candidates"][0]["context"])
        edges = {8: (20, 10)}
        for forward, curved in ((0.2, 0.7), (0.7, 0.2), (0.7, 0.7)):
            context.update(
                forward_continuation=forward,
                curved_continuation=curved,
                continuation=0.7,
                support=0.3,
                support_2r=0.3,
            )
            with self.subTest(forward=forward, curved=curved):
                result = validate_context(context, [8], edges, 10, 8, kind="ending")
                self.assertEqual(result.continuation, 0.7)
                context["continuation"] = 0.6
                with self.assertRaisesRegex(ValueError, "combined context continuation"):
                    validate_context(context, [8], edges, 10, 8, kind="ending")

    def test_missing_ending_components_preserve_partial_diagnostics_without_inventing_support(self):
        edges = {8: (20, 10)}
        for missing in ("termination", "forward_continuation", "curved_continuation"):
            context = copy.deepcopy(self.strength["candidates"][0]["context"])
            context[missing] = None
            if missing != "termination":
                with self.assertRaisesRegex(ValueError, "unavailable combined evidence"):
                    validate_context(context, [8], edges, 10, 8, kind="ending")
                context["continuation"] = None
            with self.subTest(missing=missing):
                with self.assertRaisesRegex(ValueError, "missing ending evidence"):
                    validate_context(context, [8], edges, 10, 8, kind="ending")
                context.update(support=None, support_2r=None)
                result = validate_context(context, [8], edges, 10, 8, kind="ending")
                self.assertIsNone(getattr(result, missing))
                self.assertIsNone(result.support)
                if missing != "termination":
                    self.assertEqual(result.termination, 0.8)

        context = copy.deepcopy(self.strength["candidates"][0]["context"])
        context["continuation"] = None
        with self.assertRaisesRegex(ValueError, "require their combined evidence"):
            validate_context(context, [8], edges, 10, 8, kind="ending")

    def test_detail_cards_explain_quality_forks_disagreement_and_unknown_deterministically(self):
        def candidate(identifier, kind, priority, context):
            return SimpleNamespace(
                id=identifier,
                kind=kind,
                priority=priority,
                context=SimpleNamespace(support=context, support_2r=context),
                stability=1,
                source_support=0.2,
                geometry_support=0.6,
            )

        candidates = [
            candidate(20, "ending", 0.9, 0.95),
            candidate(11, "bifurcation", 0.8, 0.9),
            candidate(5, "bifurcation", 0.8, 0.9),
            candidate(9, "ending", 0, 0),
            candidate(3, "ending", None, None),
        ]
        expected = [20, 5, 9, 3]
        for order in (candidates, candidates[::-1]):
            self.assertEqual([item.id for _, item in detail_examples(order)], expected)
        zeroes = [candidate(i, "ending", 0, 0) for i in (5, 2, 9, 8)]
        self.assertEqual([item.id for _, item in detail_examples(zeroes)], [2, 5, 8, 9])


if __name__ == "__main__":
    unittest.main()
