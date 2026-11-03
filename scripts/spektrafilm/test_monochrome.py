"""Source and component integrity without the scientific generation environment."""

import copy
import json
import math
from pathlib import Path
import shutil
import tempfile
import unittest
import zlib

from . import monochrome


class Monochrome(unittest.TestCase):
    def test_display_names_use_catalogue_capitalization_without_professional(self):
        for path in monochrome.SOURCES:
            with self.subTest(path=path):
                name = json.loads(path.read_text())["material"]["name"]
                self.assertNotEqual(name, name.upper())
                self.assertNotIn("professional", name.casefold())

    def test_reconstructed_curves_fit_retained_readings_and_reject_changed_density(self):
        for path in monochrome.SOURCES:
            with self.subTest(path=path):
                source = json.loads(path.read_text())
                audit = monochrome.audit_source(source)
                self.assertEqual(audit["constraints"], len(source["constraints"]))
                self.assertGreater(audit["constraints"], 100)
                self.assertLessEqual(
                    audit["max_density_outside_ink_interval"],
                    source["reading_uncertainty"]["density"],
                )
                changed = copy.deepcopy(source)
                reconstruction = changed["reconstruction"]
                if reconstruction["family"] == "tabulated":
                    for row in reconstruction["samples"]:
                        row[1] += 0.1
                else:
                    reconstruction["parameters"][0][1] *= 1.2
                with self.assertRaisesRegex(ValueError, "outside retained graph-reading allowance"):
                    monochrome.audit_source(changed)

    def test_invalid_curve_and_sensitivity_coordinates_are_rejected(self):
        source = next(
            json.loads(p.read_text())
            for p in monochrome.SOURCES
            if p.parent.parent.name == "negatives"
        )
        changed = copy.deepcopy(source)
        changed["reconstruction"]["samples"][10][0] = changed["reconstruction"]["samples"][9][0]
        with self.assertRaisesRegex(ValueError, "invalid characteristic"):
            monochrome.audit_source(changed)
        changed = copy.deepcopy(source)
        changed["sensitivity"]["samples"][0][1] = float("nan")
        with self.assertRaisesRegex(ValueError, "invalid spectral"):
            monochrome.audit_source(changed)
        monochrome.audit_source(source)

    def test_flat_source_segments_remain_exact_without_rounding_dips(self):
        for density in [0.17777777777777776, 1.869047619047619]:
            source = {
                "reconstruction": {"family": "tabulated", "samples": [[-1, density], [1, density]]}
            }
            for i in range(2049):
                self.assertEqual(monochrome.source_density(source, -1 + i / 1024), density)

    def test_linear_and_log_sensitivity_are_distinct_and_no_source_spd_is_invented(self):
        linear = {
            "scale": "linear-relative",
            "samples": [[400, 0.2], [600, 1.0]],
            "source_illuminant_K": 2850,
        }
        logarithmic = {"scale": "log10-reciprocal-exposure", "samples": [[400, -1], [600, 1]]}
        self.assertAlmostEqual(monochrome.sensitivity_response(linear, 500), 0.6)
        self.assertAlmostEqual(monochrome.sensitivity_response(logarithmic, 500), 1.0)
        for source in [linear, logarithmic]:
            self.assertEqual(monochrome.sensitivity_response(source, 350), 0.0)
            self.assertEqual(monochrome.sensitivity_response(source, 650), 0.0)
        with self.assertRaisesRegex(ValueError, "unreviewed spectral-sensitivity scale"):
            monochrome.sensitivity_response({**linear, "scale": "unknown"}, 500)

    def test_ilford_graph_controls_and_published_processes_are_present(self):
        for prefix, minutes, toe, mid, high, blue in [
            ("ilford_fp4_plus_", 8, 0.1, 1.0, 1.9, 0.4),
            ("ilford_hp5_plus_", 6.5, 0.18, 1.1, 2.1, 0.7),
        ]:
            source = next(
                json.loads(p.read_text())
                for p in monochrome.SOURCES
                if json.loads(p.read_text())["material"]["id"].startswith(prefix)
            )
            with self.subTest(stock=prefix):
                self.assertEqual(source["conditions"]["development_s"], minutes * 60)
                self.assertEqual(source["conditions"]["dilution"], "1+31")
                self.assertEqual(source["sensitivity"]["scale"], "linear-relative")
                # Coarse readings from the published grids, independently of
                # the dense ink tracing and generated component tables.
                for h, expected in [(0.5, toe), (2.5, mid), (4.0, high)]:
                    self.assertAlmostEqual(
                        monochrome.source_density(source, h), expected, delta=0.05
                    )
                self.assertAlmostEqual(
                    monochrome.sensitivity_response(source["sensitivity"], 400), blue, delta=0.07
                )
                changed = copy.deepcopy(source)
                changed["sensitivity"]["samples"][10][1] = -0.1
                with self.assertRaisesRegex(ValueError, "invalid spectral-sensitivity"):
                    monochrome.audit_source(changed)

    def test_delta_and_tmax_400_curves_retain_the_plotted_process_and_spectral_scale(self):
        sources = {
            json.loads(p.read_text())["material"]["id"]: json.loads(p.read_text())
            for p in monochrome.SOURCES
        }
        # Coarse controls read from the original plots, not generated tables.
        for stock, developer, seconds, temperature, readings, sensitivity in [
            (
                "ilford_delta_100_id11_8m30s",
                "ILFORD ID-11",
                510,
                20,
                [(0.5, 0.08), (2, 0.32), (3, 1.0), (4, 1.8)],
                [(400, 0.65), (600, 0.85), (650, 0.08)],
            ),
            (
                "ilford_delta_400_id11_8m_24c",
                "ILFORD ID-11",
                480,
                24,
                [(0.5, 0.28), (2, 1.0), (3, 1.7), (4, 2.2)],
                [(400, 0.75), (600, 0.95), (650, 0.95)],
            ),
            (
                "kodak_tmax_400_d76_8m",
                "KODAK PROFESSIONAL D-76",
                480,
                20,
                [(-3, 0.25), (-2, 0.65), (-1, 1.3), (0, 1.95)],
                [(400, 2.9), (500, 2.75), (600, 2.8)],
            ),
        ]:
            source = sources[stock]
            with self.subTest(stock=stock):
                self.assertEqual(source["conditions"]["developer"], developer)
                self.assertEqual(source["conditions"]["dilution"], "stock")
                self.assertEqual(source["conditions"]["temperature_C"], temperature)
                self.assertEqual(source["conditions"]["development_s"], seconds)
                if temperature == 24:
                    self.assertRegex(source["material"]["name"], r"\b24\s*°?\s*C\b")
                for exposure, density in readings:
                    self.assertAlmostEqual(
                        monochrome.source_density(source, exposure), density, delta=0.08
                    )
                logarithmic = stock.startswith("kodak_")
                self.assertEqual(
                    source["sensitivity"]["scale"],
                    "log10-reciprocal-exposure" if logarithmic else "linear-relative",
                )
                for wavelength, expected in sensitivity:
                    response = monochrome.sensitivity_response(source["sensitivity"], wavelength)
                    ordinate = math.log10(response) if logarithmic else response
                    self.assertAlmostEqual(ordinate, expected, delta=0.12 if logarithmic else 0.08)

    def test_fb_classic_uses_the_filter_two_curve_and_its_plotted_development(self):
        source = json.loads(
            (
                monochrome.ROOT / "assets/papers/ilford-multigrade-fb-classic-filter-2/source.json"
            ).read_text()
        )
        self.assertEqual(source["conditions"]["filter"], "ILFORD MULTIGRADE 2")
        self.assertEqual(source["conditions"]["developer"], "ILFORD MULTIGRADE")
        self.assertEqual(source["conditions"]["dilution"], "1+9")
        self.assertEqual(source["conditions"]["temperature_C"], 20)
        self.assertEqual(source["conditions"]["development_s"], 120)
        # The cyan stroke's separate toe fragments lie on the zero-density baseline.
        self.assertAlmostEqual(monochrome.source_density(source, 0.05), 0.0, delta=0.005)
        # Published cyan2 differs from adjacent green1/red3 at mid exposure.
        for exposure, density in [(0.5, 0.2), (1.0, 1.25), (1.5, 2.06), (2.0, 2.08)]:
            self.assertAlmostEqual(monochrome.source_density(source, exposure), density, delta=0.06)

    def test_every_fixture_reflectance_is_one_total_neutral_density(self):
        fixture = json.loads(zlib.decompress(monochrome.FIXTURE.read_bytes()))
        sources = [json.loads(path.read_text()) for path in monochrome.SOURCES]
        papers = {s["material"]["id"] for s in sources if s["material"].get("kind") != "negative"}
        negatives = {
            s["material"]["id"] for s in sources if s["material"].get("kind") == "negative"
        }
        expected = {(n, p) for n in [None, *negatives] for p in papers}
        self.assertEqual({(c["negative"], c["paper"]) for c in fixture["cases"]}, expected)
        self.assertEqual(len(fixture["cases"]), len(expected))
        for case in fixture["cases"]:
            self.assertEqual(len(fixture["inputs"]), len(case["density"]))
            self.assertEqual(len(case["density"]), len(case["reflection_y"]))
            self.assertGreater(max(case["density"]) - min(case["density"]), 0.5)
            for density, reflection in zip(case["density"], case["reflection_y"]):
                self.assertTrue(math.isfinite(reflection))
                self.assertLess(abs(reflection - 10 ** (-density)), 1e-12)

    def test_current_components_index_and_fixture_bind_all_source_producers(self):
        monochrome.check(monochrome.ASSETS)

    def test_corruption_stale_producers_and_environment_changes_are_rejected(self):
        (monochrome.ROOT / ".cache").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix="monochrome-test-", dir=monochrome.ROOT / ".cache"
        ) as temporary:
            folder = Path(temporary)
            for name in ["provenance.json", "components.json.z", "spectral.f32.z", "index.json"]:
                shutil.copyfile(monochrome.ASSETS / name, folder / name)
            shutil.copyfile(monochrome.FIXTURE, folder / "conformance.json.z")
            monochrome.check(folder)
            for name in ["components.json.z", "spectral.f32.z", "index.json"]:
                original = (folder / name).read_bytes()
                (folder / name).write_bytes(original[:-1] + bytes([original[-1] ^ 1]))
                with self.assertRaisesRegex(ValueError, "paper output changed"):
                    monochrome.check(folder)
                (folder / name).write_bytes(original)
                monochrome.check(folder)
            provenance = json.loads((folder / "provenance.json").read_text())
            for section, key, pattern in [
                *[
                    ("inputs", str(path.relative_to(monochrome.ROOT)), "paper producer changed")
                    for path in monochrome.SOURCES
                ],
                ("dependencies", "numpy", "paper dependencies differ"),
                ("outputs", "conformance.json.z", "paper output changed"),
            ]:
                with self.subTest(section=section, key=key):
                    changed = copy.deepcopy(provenance)
                    changed[section][key] = "changed"
                    (folder / "provenance.json").write_text(json.dumps(changed))
                    with self.assertRaisesRegex(ValueError, pattern):
                        monochrome.check(folder)
                    (folder / "provenance.json").write_text(json.dumps(provenance))
                    monochrome.check(folder)


if __name__ == "__main__":
    unittest.main()
