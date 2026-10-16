import unittest

import numpy as np

import experiment as e
from donor_study import reconstruction_metrics
from stage_study import density_heal


class RefinementStudy(unittest.TestCase):
    def test_identity_metrics_and_covariance(self):
        rng = np.random.default_rng(21)
        image = rng.normal(size=(48, 48, 3)).astype(np.float32)
        mask = np.zeros((48, 48), bool)
        mask[20:28, 20:28] = True
        metrics = reconstruction_metrics(image, image, mask)
        self.assertEqual(metrics["masked_structure_gradient_rmse"], 0)
        self.assertEqual(metrics["masked_rmse"], 0)
        self.assertEqual(metrics["masked_highpass_ratio"], 1)
        covariance = np.asarray(metrics["reference_highpass_rgb_covariance"])
        np.testing.assert_array_equal(covariance, covariance.T)
        np.testing.assert_array_equal(covariance, metrics["repaired_highpass_rgb_covariance"])
        self.assertGreaterEqual(float(np.linalg.eigvalsh(covariance).min()), 0)

    def test_refined_log_camera_ignores_hidden_values_and_keeps_camera_bits(self):
        y, x = np.mgrid[:96, :96]
        image = np.repeat((0.03 + x * 0.0001 + y * 0.0001)[:, :, None], 3, axis=2).astype(
            np.float32
        )
        mask = np.zeros((96, 96), bool)
        mask[44:52, 44:52] = True
        a, b = image.copy(), image.copy()
        a[mask], b[mask] = 0, -100
        for method in e.DONOR_REFINEMENTS:
            with self.subTest(method=method):
                result, donors = density_heal(a, mask, method=method)
                other, repeated = density_heal(b, mask, method=method)
                np.testing.assert_array_equal(result.view(np.uint32), other.view(np.uint32))
                self.assertEqual(donors, repeated)
                self.assertTrue(e.outside_bit_exact(result, image, mask))
                self.assertTrue(np.isfinite(result).all())
                self.assertGreater(float(result[mask].min()), 0)


if __name__ == "__main__":
    unittest.main()
