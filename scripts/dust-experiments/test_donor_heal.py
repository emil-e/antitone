import unittest

import numpy as np

import experiment as e
from experiment import nearest_competitive_candidate


class DonorRefinements(unittest.TestCase):
    def setUp(self):
        y, x = np.mgrid[:96, :96]
        self.image = np.stack(
            (0.04 + x * 0.0001, -0.02 + y * 0.0002, 1.1 + x * 0.0003), axis=2
        ).astype(np.float32)
        self.image[0, 0] = (-0.0, 0.0, -0.0)
        self.mask = e.mask_from_shapes(
            self.image.shape, [{"kind": "circle", "center": [48, 48], "radius": 6}]
        )

    def test_masked_truth_independence_known_bits_and_signed_hdr(self):
        a, b = self.image.copy(), self.image.copy()
        a[self.mask], b[self.mask] = -100, 100
        for method in e.DONOR_REFINEMENTS:
            with self.subTest(method=method):
                result, donors = e.repair(a, self.mask, method)
                other, repeated = e.repair(b, self.mask, method)
                np.testing.assert_array_equal(result.view(np.uint32), other.view(np.uint32))
                self.assertEqual(donors, repeated)
                self.assertTrue(e.outside_bit_exact(result, self.image, self.mask))
                self.assertTrue(np.isfinite(result).all())
                self.assertLess(result[self.mask, 1].max(), 0)
                self.assertGreater(result[self.mask, 2].min(), 1)

    def test_near_source_can_overlap_unused_context_but_not_required_pixels(self):
        with self.assertRaisesRegex(ValueError, "intersects unknown"):
            e.repair(self.image, self.mask, "donor-heal", donor_offset=[17, 0])
        for method in e.DONOR_REFINEMENTS:
            with self.subTest(method=method):
                result, donors = e.repair(self.image, self.mask, method, donor_offset=[17, 0])
                np.testing.assert_allclose(
                    result[self.mask], self.image[self.mask], atol=3e-7, rtol=0
                )
                self.assertTrue(e.outside_bit_exact(result, self.image, self.mask))
                self.assertEqual(donors[0]["source"][0], donors[0]["target"][0] + 17)

    def test_invalid_sources_reject(self):
        for method in e.DONOR_REFINEMENTS:
            for offset in ([0, 0], [14, 0], [1000, 0], [0.5, 0]):
                with self.subTest(method=method, offset=offset), self.assertRaises(ValueError):
                    e.repair(self.image, self.mask, method, donor_offset=offset)

    def test_close_masks_are_jointly_solved_without_hidden_boundary_values(self):
        mask = np.zeros(self.mask.shape, bool)
        mask[46:49, 46:49] = True
        mask[46:49, 50:53] = True
        a, b = self.image.copy(), self.image.copy()
        a[mask], b[mask] = -100, 100
        for method in e.DONOR_REFINEMENTS:
            with self.subTest(method=method):
                result, donors = e.repair(a, mask, method, donor_offset=[-25, 0])
                other, _ = e.repair(b, mask, method, donor_offset=[-25, 0])
                self.assertEqual(len(donors), 1)
                np.testing.assert_array_equal(result.view(np.uint32), other.view(np.uint32))
                np.testing.assert_allclose(result[mask], self.image[mask], atol=3e-7, rtol=0)

    def test_structural_variants_use_identical_automatic_donors(self):
        _, harmonic = e.repair(self.image, self.mask, "donor-heal-structure")
        for method in ("donor-heal-refined", "donor-heal-boundary"):
            _, donors = e.repair(self.image, self.mask, method)
            self.assertEqual(len(harmonic), len(donors))
            for a, b in zip(harmonic, donors):
                self.assertEqual(a["source"], b["source"])
                self.assertEqual(a["target"], b["target"])
                self.assertEqual(a["matching_score"], b["matching_score"])

    def test_crossing_wire_keeps_source_alignment(self):
        y, x = np.mgrid[:96, :96]
        image = np.repeat((0.04 + y * 0.0001)[:, :, None], 3, axis=2).astype(np.float32)
        image[:, 47:50] += 0.08
        mask = e.mask_from_shapes(
            image.shape, [{"kind": "circle", "center": [48, 48], "radius": 4}]
        )
        for method in (
            "donor-heal-structure",
            "donor-heal-refined",
            "donor-heal-boundary",
            "donor-heal-ranked",
        ):
            with self.subTest(method=method):
                result, donors = e.repair(image, mask, method)
                self.assertEqual(donors[0]["source"][0], donors[0]["target"][0])
                np.testing.assert_allclose(result[mask], image[mask], atol=3e-7, rtol=0)

    def test_empty_identity_and_full_rejection(self):
        for method in e.DONOR_REFINEMENTS:
            result, donors = e.repair(self.image, np.zeros(self.mask.shape, bool), method)
            np.testing.assert_array_equal(result.view(np.uint32), self.image.view(np.uint32))
            self.assertEqual(donors, [])
            with self.assertRaises(ValueError):
                e.repair(self.image, np.ones(self.mask.shape, bool), method)

    def test_nearest_candidate_wins_only_within_score_tolerance(self):
        ranked = [(1.0, 20, 20, 20.0), (1.009, 5, 5, 5.0), (1.02, 1, 1, 1.0)]
        self.assertEqual(nearest_competitive_candidate(ranked), ranked[1])
        self.assertEqual(nearest_competitive_candidate(ranked, tolerance=0), ranked[0])

    def test_pair_scores_ignore_missing_source_and_target_samples(self):
        rng = np.random.default_rng(20)
        search = rng.normal(size=(12, 13, 3)).astype(np.float32)
        template = rng.normal(size=(4, 5, 3)).astype(np.float32)
        source_known = rng.random(search.shape[:2]) > 0.2
        target_known = rng.random(template.shape[:2]) > 0.2
        a, b = search.copy(), template.copy()
        a[~source_known], b[~target_known] = 1000, -1000
        weights = target_known.astype(np.float32)
        weights[0] *= 0.25
        scores, counts = e.paired_scores(a, source_known, b, weights)
        h, w = target_known.shape
        for y in range(scores.shape[0]):
            for x in range(scores.shape[1]):
                paired = target_known & source_known[y : y + h, x : x + w]
                paired_weights = weights[paired].astype(np.float64)
                self.assertAlmostEqual(counts[y, x], paired_weights.sum(), places=5)
                delta = search[y : y + h, x : x + w][paired].astype(np.float64) - template[paired]
                expected = np.sum(delta**2 * paired_weights[:, None]) / (paired_weights.sum() * 3)
                self.assertAlmostEqual(scores[y, x], expected, places=5)


class Biharmonic(unittest.TestCase):
    def test_quadratic_reconstruction_and_known_bytes(self):
        y, x = np.mgrid[:64, :64]
        image = np.stack(
            (0.000025 * ((x - 32) ** 2 + (y - 32) ** 2), -0.02 + x * 0.0001, 1.1 + y * 0.0002),
            axis=2,
        ).astype(np.float32)
        image[0, 0] = (-0.0, 0.0, -0.0)
        mask = (x - 32) ** 2 + (y - 32) ** 2 <= 10**2
        a, b = image.copy(), image.copy()
        a[mask], b[mask] = -100, 100
        result = e.biharmonic_fill(a, mask)
        other = e.biharmonic_fill(b, mask)
        np.testing.assert_array_equal(result.view(np.uint32), other.view(np.uint32))
        np.testing.assert_allclose(result[mask], image[mask], atol=2e-6, rtol=0)
        self.assertTrue(e.outside_bit_exact(result, image, mask))
        harmonic = e.harmonic_fill(a, mask)
        self.assertGreater(float(np.abs(harmonic[mask, 0] - image[mask, 0]).mean()), 0.001)

    def test_limited_refinement_is_deterministic_and_keeps_known_bits(self):
        y, x = np.mgrid[:64, :64]
        image = np.repeat(
            (0.000025 * ((x - 32) ** 2 + (y - 32) ** 2))[:, :, None], 3, axis=2
        ).astype(np.float32)
        image[0, 0] = (-0.0, 0.0, -0.0)
        mask = (x - 32) ** 2 + (y - 32) ** 2 <= 10**2
        a, b = image.copy(), image.copy()
        a[mask], b[mask] = -100, 100
        result = e.biharmonic_fill(a, mask, iterations=8)
        other = e.biharmonic_fill(b, mask, iterations=8)
        np.testing.assert_array_equal(result.view(np.uint32), other.view(np.uint32))
        self.assertTrue(e.outside_bit_exact(result, image, mask))
        self.assertLess(
            float(np.mean((result[mask] - image[mask]) ** 2)),
            float(np.mean((e.harmonic_fill(a, mask)[mask] - image[mask]) ** 2)),
        )

    def test_missing_exterior_rejects_and_empty_is_identity(self):
        image = np.zeros((16, 16, 3), np.float32)
        mask = np.zeros((16, 16), bool)
        np.testing.assert_array_equal(e.biharmonic_fill(image, mask), image)
        for y, x in ((0, 5), (1, 5), (14, 5), (5, 15)):
            mask[:] = False
            mask[y, x] = True
            with self.assertRaisesRegex(ValueError, "two-pixel exterior"):
                e.biharmonic_fill(image, mask)


if __name__ == "__main__":
    unittest.main()
