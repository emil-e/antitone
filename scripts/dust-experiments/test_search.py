import unittest

import cv2
import numpy as np

from experiment import positive_gaussian_score
from search import candidates, density_patch


class NativeSearch(unittest.TestCase):
    def test_tiled_search_finds_boundary_spot_and_is_deterministic(self):
        rng = np.random.default_rng(7)
        camera = np.clip(rng.normal(20000, 300, (256, 256, 3)), 1, 65535).astype(np.uint16)
        cv2.circle(camera, (127, 127), 5, (12000, 12000, 12000), -1)
        for polarity in ("both", "positive"):
            with self.subTest(polarity=polarity):
                first = candidates(camera, tile=128, polarity=polarity)
                self.assertEqual(first, candidates(camera, tile=128, polarity=polarity))
                nearest = min(first, key=lambda c: (c["x"] - 127) ** 2 + (c["y"] - 127) ** 2)
                self.assertLessEqual(abs(nearest["x"] - 127), 2)
                self.assertLessEqual(abs(nearest["y"] - 127), 2)
                self.assertEqual(
                    sum((c["x"] - 127) ** 2 + (c["y"] - 127) ** 2 < 20**2 for c in first), 1
                )

    def test_density_view_does_not_modify_source(self):
        camera = np.full((128, 128, 3), 20000, np.uint16)
        original = camera.copy()
        self.assertEqual(density_patch(camera, 64, 64, 32).size, (64, 64))
        np.testing.assert_array_equal(camera, original)

    def test_positive_score_distinguishes_transmission_polarity_at_center(self):
        for value, sign in ((0.15, 1), (0.3, -1)):
            camera = np.full((128, 128, 3), 0.2, np.float32)
            cv2.circle(camera, (64, 64), 6, (value, value, value), -1)
            score = positive_gaussian_score(camera)
            self.assertTrue(np.isfinite(score).all())
            self.assertGreater(sign * float(score[64, 64]), 0)


if __name__ == "__main__":
    unittest.main()
