"""Analytic spectral controls for the paper-only reference, without upstream Python."""

import copy
import math
import unittest
from unittest.mock import patch

from . import reference
from .characteristic import PARAMETERS, density
from .positive import prepare


def diagonal():
    film = {
        "id": reference.NEGATIVE,
        "support": [True] * 3,
        "dyes": [[2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 4.0]],
        "base": [0.2, 0.3, 0.4],
    }
    pair = {
        "negative": reference.NEGATIVE,
        "paper": "analytic",
        "weights": [[2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 4.0]],
        "preflash": [0.1, 0.2, 0.3],
        "print_exposure": 2.5,
    }
    definition = reference.definition()
    definition.update(
        exposure_rows=[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        dye_rows=[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        density_gain=1.0,
    )
    return film, pair, definition


class ReferenceFormation(unittest.TestCase):
    def test_separate_dye_bands_have_analytic_transmission_and_print_exposure(self):
        film, pair, definition = diagonal()
        with patch.object(reference, "definition", return_value=definition):
            model = reference.Reference(film, pair)
        for rgb in [[0.0, 0.0, 0.0], [0.003, 0.02, 0.18], [4.0, 0.008, 0.03]]:
            actual = model.expose(rgb)
            for c, weight in enumerate([2.0, 3.0, 4.0]):
                raw = weight * 10 ** -(film["base"][c] + density(rgb[c])) + pair["preflash"][c]
                expected = math.log10((raw + 1e-10) * 2.5 + 1e-10)
                self.assertAlmostEqual(actual[c], expected, places=13)
        self.assertEqual(model.dyes, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])

    def test_common_neutral_filtration_and_lamp_compensation_preserve_dose(self):
        film, pair, definition = diagonal()
        models = []
        for offset in [0.0, 1.25]:
            setting = copy.deepcopy(definition)
            setting["base_density_offset"] = offset
            with patch.object(reference, "definition", return_value=setting):
                models.append(reference.Reference(film, pair))
        for rgb in [[0.18] * 3, [3.0, 0.2, 0.03], [-0.01, 0.3, 0.02]]:
            for a, b in zip(models[0].expose(rgb), models[1].expose(rgb)):
                self.assertAlmostEqual(a, b, places=13)

    def test_signed_effective_coordinates_are_not_clipped(self):
        film, pair, definition = diagonal()
        with patch.object(reference, "definition", return_value=definition):
            model = reference.Reference(film, pair)
        a = [-0.15, 0.3, 0.4]
        actual = model.transmission(a)
        for c in range(3):
            self.assertAlmostEqual(actual[c], 10 ** -(film["base"][c] + 1.25 + a[c]), places=14)
        self.assertGreater(actual[0], model.transmission([0.0, 0.3, 0.4])[0])
        self.assertTrue(all(0 < t <= 1 for t in actual))

    def test_pre_curve_exposure_basis_and_post_curve_dye_basis_do_not_collapse(self):
        film, pair, definition = diagonal()
        definition["exposure_rows"] = [[0.5, 0.5, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        definition["dye_rows"] = [[2.0, -1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        with patch.object(reference, "definition", return_value=definition):
            model = reference.Reference(film, pair)
        rgb = [0.03, 0.003, 0.18]
        actual = model.amounts(rgb)
        self.assertAlmostEqual(actual[0], 2 * density(0.0165) - density(0.003), places=14)
        collapsed = reference.multiply(
            definition["dye_rows"], reference.multiply(definition["exposure_rows"], rgb)
        )
        self.assertGreater(abs(actual[0] - density(collapsed[0])), 0.01)

    def test_printing_balance_is_fixed_and_gray_is_a_calibration_not_a_pixel_branch(self):
        film, pair, definition = diagonal()
        with patch.object(reference, "definition", return_value=definition):
            driver = prepare(film, pair, [-0.1, 0.2, 0.3])
        gray = driver.expose([[PARAMETERS["gray"]] * 3])[0]
        for actual, expected in zip(gray, [-0.1, 0.2, 0.3]):
            self.assertAlmostEqual(actual, expected, places=14)
        before = driver.print_log_h[:]
        driver.expose([[0.003, 0.18, 2.0], [-0.02, 0.3, 0.01]])
        self.assertEqual(driver.print_log_h, before)

    def test_definition_records_neutral_preserving_matrices_and_reviewed_units(self):
        d = reference.definition()
        for key in ["exposure_rows", "dye_rows"]:
            for row in d[key]:
                self.assertAlmostEqual(sum(row), 1.0, places=12)
        self.assertGreater(d["density_gain"], 0)
        self.assertGreater(d["base_density_offset"], 0)
