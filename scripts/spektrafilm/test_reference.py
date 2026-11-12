"""Reference input boundaries without acquiring or redistributing standard data."""

from pathlib import Path
import tempfile
import unittest

from .reference import receiver_weights, responsivities


class ReferenceInput(unittest.TestCase):
    def test_relative_filter_receiver_has_independent_asymmetric_channel_sums(self):
        weights = receiver_weights(
            [400.0, 500.0, 600.0],
            [0.2, 0.5, 0.8],
            [390.0, 450.0, 550.0, 610.0],
            [0.1, 0.4, 0.6, 0.9],
            [[2.0, 1.0, 0.0], [0.0, 3.0, 1.0], [1.0, 0.0, 4.0], [5.0, 2.0, 1.0]],
        )
        # Relative filter .5,.4,.8 gives site transmissions .05,.18,.36,.72.
        for c, expected in enumerate([4.06, 2.03, 2.34]):
            observed = sum(w[c] * t for w, t in zip(weights, [0.1, 0.2, 0.64]))
            self.assertAlmostEqual(observed, expected, places=13)

    def test_wrong_shape_wavelengths_and_unreviewed_numeric_table_reject(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "reference.csv"
            header = "wavelength_nm,red,green,blue\n"
            for body in [
                "360,1,1,1\n",
                "".join(f"{w},0,0,0\n" for w in range(360, 731, 2)),
                "".join(f"{w},1,1,1\n" for w in range(361, 732, 2)),
            ]:
                with self.subTest(body=body[:30]):
                    path.write_text(header + body)
                    with self.assertRaises(ValueError):
                        responsivities(path)
