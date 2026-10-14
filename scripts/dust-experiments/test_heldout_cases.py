import unittest

from heldout_cases import cases_for_crop


class HeldOutCases(unittest.TestCase):
    def test_generation_is_deterministic_bounded_and_name_dependent(self):
        a = cases_for_crop('first', 1024, 768, 'crops')
        self.assertEqual(a, cases_for_crop('first', 1024, 768, 'crops'))
        self.assertNotEqual(a, cases_for_crop('second', 1024, 768, 'crops'))
        self.assertEqual([case['name'] for case in a],
                         ['first-validation-circle', 'first-validation-line'])
        for case in a:
            self.assertEqual(case['crop'], 'crops/first')
            x0, y0, x1, y1 = case['zoom']
            self.assertGreaterEqual(x0, 0)
            self.assertGreaterEqual(y0, 0)
            self.assertLessEqual(x1, 1024)
            self.assertLessEqual(y1, 768)
            self.assertEqual((x1 - x0, y1 - y0), (128, 128))

    def test_small_crop_rejects(self):
        with self.assertRaises(ValueError):
            cases_for_crop('small', 320, 320, 'crops')


if __name__ == '__main__':
    unittest.main()
