import unittest

import numpy as np

import experiment as e
from context_study import context_filter
from stage_study import density_heal


class Study(unittest.TestCase):
    def test_log_heal_ignores_unknowns_and_preserves_known_camera(self):
        camera = np.full((96, 96, 3), 0.02, np.float32)
        mask = e.mask_from_shapes(
            camera.shape, [{"kind": "circle", "center": [48, 48], "radius": 6}]
        )
        other = camera.copy()
        other[mask] = -100
        a, _ = density_heal(camera, mask, donor_offset=[-32, 0])
        b, _ = density_heal(other, mask, donor_offset=[-32, 0])
        np.testing.assert_array_equal(a.view(np.uint32), b.view(np.uint32))
        self.assertTrue(e.outside_bit_exact(a, camera, mask))
        np.testing.assert_allclose(a[mask], 0.02, rtol=1e-6, atol=0)
        self.assertTrue((a > 0).all())

    def test_log_heal_rejects_nonpositive_known_transmission(self):
        camera = np.full((96, 96, 3), 0.02, np.float32)
        camera[0, 0] = 0
        mask = e.mask_from_shapes(
            camera.shape, [{"kind": "circle", "center": [48, 48], "radius": 6}]
        )
        with self.assertRaises(ValueError):
            density_heal(camera, mask)

    def test_context_veto_rejects_compact_target_in_coherent_background(self):
        _, x = np.mgrid[:128, :128]
        camera = np.repeat(np.exp(-0.005 * x).astype(np.float32)[:, :, None], 3, axis=2)
        mask = np.zeros((128, 128), bool)
        mask[62:66, 62:66] = True
        shape, features = context_filter(camera, mask)
        context, _ = context_filter(camera, mask, coherence_limit=0.8)
        np.testing.assert_array_equal(shape, mask)
        self.assertGreater(features[0]["context_coherence"], 0.99)
        self.assertFalse(context.any())

    def test_shape_filter_rejects_long_component_without_mutating_input(self):
        camera = np.full((128, 128, 3), 0.2, np.float32)
        mask = np.zeros((128, 128), bool)
        mask[50:52, 40:90] = True
        original = mask.copy()
        filtered, features = context_filter(camera, mask)
        self.assertFalse(filtered.any())
        self.assertGreater(features[0]["aspect"], 4)
        np.testing.assert_array_equal(mask, original)


if __name__ == "__main__":
    unittest.main()
