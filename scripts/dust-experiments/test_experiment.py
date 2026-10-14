import json
import unittest

import cv2
import numpy as np

from experiment import REPAIRS, analysis_gray, candidate_mask, harmonic_fill, mask_from_shapes, repair


class Repairs(unittest.TestCase):
    def setUp(self):
        y, x = np.mgrid[:96, :96]
        self.image = np.stack((0.04 + x * 0.0001, -0.02 + y * 0.0002, 1.1 + x * 0.0003), axis=2).astype(np.float32)
        self.image[0, 0] = (-0.0, 0.0, -0.0)
        self.mask = mask_from_shapes(self.image.shape, [{"kind": "circle", "center": [48, 48], "radius": 6}])

    def test_all_methods_ignore_masked_input_and_preserve_outside(self):
        a, b = self.image.copy(), self.image.copy()
        a[self.mask], b[self.mask] = -100, 100
        for method in REPAIRS:
            with self.subTest(method=method):
                result, _ = repair(a, self.mask, method)
                other, _ = repair(b, self.mask, method)
                np.testing.assert_array_equal(result, other)
                np.testing.assert_array_equal(result[~self.mask].view(np.uint32), self.image[~self.mask].view(np.uint32))
                self.assertTrue(np.isfinite(result).all())
                self.assertLess(result[self.mask, 1].max(), 0)  # Signed green retained.
                self.assertGreater(result[self.mask, 2].min(), 1)  # HDR blue retained.

    def test_harmonic_reconstructs_affine_field(self):
        damaged = self.image.copy()
        damaged[self.mask] = 0
        filled = harmonic_fill(damaged, self.mask)
        np.testing.assert_allclose(filled[self.mask], self.image[self.mask], atol=2e-7, rtol=0)

    def test_harmonic_reconstructs_constant_and_edge_touching_masks(self):
        image = np.full((32, 32, 3), -0.02, np.float32)
        mask = np.zeros((32, 32), bool)
        mask[:8, :8] = True
        damaged = image.copy()
        damaged[mask] = 42
        np.testing.assert_allclose(harmonic_fill(damaged, mask), image, atol=1e-8, rtol=0)

    def test_donor_never_contains_unknown_pixels(self):
        mask = self.mask.copy()
        mask[65:70, 65:70] = True
        for method in ("donor-clone", "donor-heal"):
            _, donors = repair(self.image, mask, method)
            json.dumps(donors)
            for donor in donors:
                _, _, w, h = donor["target"]
                x, y = donor["source"]
                self.assertFalse(mask[y:y + h, x:x + w].any())

    def test_fixed_donor_is_used_and_masked_truth_is_ignored(self):
        a, b = self.image.copy(), self.image.copy()
        a[self.mask], b[self.mask] = -100, 100
        for method in ("donor-clone", "donor-heal"):
            result, donors = repair(a, self.mask, method, donor_offset=[-32, 0])
            other, _ = repair(b, self.mask, method, donor_offset=[-32, 0])
            np.testing.assert_array_equal(result.view(np.uint32), other.view(np.uint32))
            np.testing.assert_array_equal(result[~self.mask].view(np.uint32), self.image[~self.mask].view(np.uint32))
            self.assertEqual(donors[0]["source"], [donors[0]["target"][0] - 32, donors[0]["target"][1]])
        np.testing.assert_allclose(result[self.mask], self.image[self.mask], atol=2e-7, rtol=0)

    def test_invalid_fixed_donors_reject(self):
        for offset in ([0, 0], [1000, 0], [0.5, 0]):
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                repair(self.image, self.mask, "donor-heal", donor_offset=offset)

    def test_empty_mask_is_identity(self):
        for method in REPAIRS:
            result, _ = repair(self.image, np.zeros(self.mask.shape, bool), method)
            np.testing.assert_array_equal(result, self.image)

    def test_full_mask_rejects(self):
        for method in REPAIRS:
            with self.assertRaises(ValueError):
                repair(self.image, np.ones(self.mask.shape, bool), method)

    def test_unconditioned_opencv_telea_float_limitation(self):
        # Pinned OpenCV 4.10.0: a constant normalized float field is not preserved.
        image = np.full((64, 64), 0.02, np.float32)
        mask = np.zeros((64, 64), np.uint8)
        cv2.circle(mask, (32, 32), 6, 1, -1)
        filled = cv2.inpaint(image, mask, 3, cv2.INPAINT_TELEA)
        self.assertGreater(float(np.abs(filled[mask > 0] - 0.02).max()), 0.1)


class Detection(unittest.TestCase):
    def test_analysis_finite_at_camera_floor(self):
        image = np.zeros((32, 32, 3), np.float32)
        self.assertTrue(np.isfinite(analysis_gray(image)).all())

    def test_component_limits_and_border(self):
        score = np.zeros((128, 128), np.float32)
        score[40:43, 40:43] = 10
        score[60, 60] = 10
        score[:4, :4] = 10
        mask = candidate_mask(score, 6)
        self.assertEqual(int(mask.sum()), 9)
        self.assertTrue(mask[41, 41])


if __name__ == "__main__":
    unittest.main()
