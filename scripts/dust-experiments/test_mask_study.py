import unittest

import numpy as np

from mask_study import score_support


class RepairSupport(unittest.TestCase):
    def test_growth_stays_inside_annotation_and_removes_singletons(self):
        annotation = np.zeros((32, 32), bool)
        annotation[5:27, 5:27] = True
        score = np.zeros((32, 32), np.float32)
        score[14:17, 14:17] = 12
        score[8, 8] = 20
        core, support = score_support(annotation, score, threshold=10, growth=3)
        self.assertEqual(int(core.sum()), 9)
        self.assertFalse(core[8, 8])
        self.assertTrue(support[12, 12])
        self.assertFalse(np.any(support & ~annotation))

    def test_threshold_and_empty_core(self):
        annotation = np.ones((16, 16), bool)
        score = np.zeros((16, 16), np.float32)
        score[6:9, 6:9] = 8
        with self.assertRaises(ValueError):
            score_support(annotation, score, threshold=10, growth=2)
        core, _ = score_support(annotation, score, threshold=8, growth=2)
        self.assertEqual(int(core.sum()), 9)


if __name__ == "__main__":
    unittest.main()
