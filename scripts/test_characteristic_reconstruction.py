"""New-material authoring: bounded smooth reconstruction and review-only publication."""

import copy
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from xml.etree import ElementTree

if __package__:
    from .spektrafilm import monochrome, reconstruct
else:
    from spektrafilm import monochrome, reconstruct


def source_for(response, allowance=0.01):
    samples = [[-1 + 3 * i / 256, response(i / 256)] for i in range(257)]
    return {
        "material": {
            "id": "new_material",
            "name": 'New <film> & "paper"',
            "log_exposure_range": [-1, 2],
        },
        "publication": "Authored graph fixture",
        "conditions": {"developer": "retained process"},
        "reading_uncertainty": {"density": allowance, "log_exposure": 0.01},
        "reconstruction": {
            "family": "tabulated",
            "samples": samples,
            "method": "Retained authored graph readings",
        },
        "constraints": [
            {"log_exposure": h, "coverage_intervals": [[d - 0.0005, d + 0.0005]]}
            for h, d in samples
        ],
    }


def smooth(t):
    return 0.1 + 2 * (3 * t**2 - 2 * t**3)


def noisy(t):
    return smooth(t) + 0.004 * math.sin(40 * math.pi * t) * 16 * t**2 * (1 - t) ** 2


class Reconstruction(unittest.TestCase):
    def test_monotone_projection_pools_violations_instead_of_only_clipping(self):
        self.assertEqual(
            reconstruct.project_monotone([-1, 0.5, 0.25, 0.375, 2], 0, 1),
            [0, 0.375, 0.375, 0.375, 1],
        )

    def test_least_squares_recovers_an_independent_cubic_polynomial(self):
        controls = [0.1, 0.25, 1.1, 1.8]

        def polynomial(t):
            return (
                (1 - t) ** 3 * controls[0]
                + 3 * (1 - t) ** 2 * t * controls[1]
                + 3 * (1 - t) * t**2 * controls[2]
                + t**3 * controls[3]
            )

        samples = [[-1 + 3 * i / 64, polynomial(i / 64)] for i in range(65)]
        fitted, _ = reconstruct.fit_controls(samples, [-1, 2], 1)
        for actual, expected in zip(fitted, controls):
            self.assertAlmostEqual(actual, expected, delta=2e-8)

    def test_new_trace_gets_a_smoother_mean_without_losing_readings_or_process(self):
        source = source_for(noisy)
        original = copy.deepcopy(source)
        candidate, audit = reconstruct.reconstruct(source)
        self.assertEqual(source, original)
        self.assertEqual(
            candidate["reconstruction"]["samples"], source["reconstruction"]["samples"]
        )
        self.assertEqual(
            candidate["reconstruction"]["trace_method"], source["reconstruction"]["method"]
        )
        for key in source.keys() - {"reconstruction"}:
            self.assertEqual(candidate[key], source[key])
        controls = candidate["reconstruction"]["control_values"]
        self.assertEqual(controls[0], source["reconstruction"]["samples"][0][1])
        self.assertEqual(controls[-1], source["reconstruction"]["samples"][-1][1])
        self.assertTrue(all(a <= b for a, b in zip(controls, controls[1:])))
        fit_error = trace_error = 0.0
        for i in range(513):
            t = i / 512
            fitted = monochrome.source_density(candidate, -1 + 3 * t)
            fit_error += (fitted - smooth(t)) ** 2
            trace_error += (noisy(t) - smooth(t)) ** 2
        # The authored noise has 20 cycles across a coarse smooth response.
        # This distinguishes reconstruction from merely copying the traced points.
        self.assertLess(fit_error, trace_error / 10)
        self.assertLessEqual(audit["attempts"][-1]["max_density_change_from_trace"], 0.01)
        monochrome.audit_source(candidate)

    def test_two_broad_development_stages_are_not_forced_into_one_s_shape(self):
        def step(t):
            t = min(1, max(0, t))
            return 3 * t**2 - 2 * t**3

        def response(t):
            return 0.1 + 1.2 * step(t / 0.45) + 0.8 * step((t - 0.6) / 0.4)

        source = source_for(response)
        candidate, _ = reconstruct.reconstruct(source)
        for t in [0.2, 0.5, 0.8]:
            self.assertAlmostEqual(
                monochrome.source_density(candidate, -1 + 3 * t), response(t), delta=0.01
            )
        self.assertAlmostEqual(monochrome.source_density(candidate, 0.5), 1.3, delta=0.01)

    def test_sparse_linear_trace_does_not_acquire_artificial_endpoint_curvature(self):
        source = source_for(lambda t: 0.1 + 2 * t)
        samples = source["reconstruction"]["samples"]
        source["reconstruction"]["samples"] = [samples[0], samples[-1]]
        candidate, _ = reconstruct.reconstruct(source)
        for i in range(33):
            t = i / 32
            self.assertAlmostEqual(monochrome.source_density(candidate, -1 + 3 * t), 0.1 + 2 * t)

    def test_flat_reading_stays_exact_and_has_a_finite_review_plot(self):
        source = source_for(lambda _: 0.5)
        candidate, _ = reconstruct.reconstruct(source)
        controls = candidate["reconstruction"]["control_values"]
        self.assertTrue(controls)
        self.assertTrue(all(v == 0.5 for v in controls))
        svg = ElementTree.fromstring(reconstruct.overlay(source, candidate))
        for line in svg.findall("{http://www.w3.org/2000/svg}polyline"):
            coordinates = line.attrib["points"].replace(",", " ").split()
            self.assertTrue(coordinates)
            self.assertTrue(all(math.isfinite(float(v)) for v in coordinates))

    def test_unresolved_sharp_trace_is_rejected_instead_of_loosening_its_allowance(self):
        source = source_for(lambda t: 0.1 if t < 0.5 else 2.1, allowance=0.001)
        original = copy.deepcopy(source)
        with self.assertRaisesRegex(ValueError, "no coarse reconstruction fits"):
            reconstruct.reconstruct(source)
        self.assertEqual(source, original)

    def test_ink_constraints_are_checked_in_addition_to_distance_from_the_trace(self):
        source = source_for(noisy)
        candidate, _ = reconstruct.reconstruct(source)
        point = max(
            source["constraints"],
            key=lambda p: (
                monochrome.source_density(source, p["log_exposure"])
                - monochrome.source_density(candidate, p["log_exposure"])
            ),
        )
        h = point["log_exposure"]
        d = monochrome.source_density(source, h)
        self.assertGreater(d - monochrome.source_density(candidate, h), 0.002)
        point["coverage_intervals"] = [[d + 0.009, d + 0.01]]
        monochrome.audit_source(source)
        with patch.object(reconstruct, "INTERVALS", (6,)):
            with self.assertRaisesRegex(ValueError, "no coarse reconstruction fits"):
                reconstruct.reconstruct(source)

    def test_bad_domains_allowances_and_existing_fits_are_rejected(self):
        source = source_for(smooth)
        for domain, allowance in [
            ([2, -1], 0.01),
            ([0, 1], 0.01),
            ([-1, 2], float("nan")),
            ([-1, 2], 0),
        ]:
            changed = copy.deepcopy(source)
            changed["material"]["log_exposure_range"] = domain
            changed["reading_uncertainty"]["density"] = allowance
            with self.assertRaises(ValueError):
                reconstruct.reconstruct(changed)
        candidate, _ = reconstruct.reconstruct(source)
        with self.assertRaisesRegex(ValueError, "existing fits stay fixed"):
            reconstruct.reconstruct(candidate)

    def test_cli_proposes_review_artifacts_without_changing_input_or_existing_output(self):
        (monochrome.ROOT / ".cache").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix="reconstruct-test-", dir=monochrome.ROOT / ".cache"
        ) as temporary:
            root = Path(temporary)
            path = root / "input.json"
            original = json.dumps(source_for(noisy)).encode()
            path.write_bytes(original)
            output = root / "candidate"
            result = subprocess.run(
                [
                    sys.executable,
                    str(Path(reconstruct.__file__)),
                    str(path),
                    "--output",
                    str(output),
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            candidate = json.loads((output / "source.json").read_text())
            monochrome.audit_source(candidate)
            audit = json.loads((output / "audit.json").read_text())
            self.assertEqual(audit["material"], "new_material")
            svg = ElementTree.parse(output / "curve.svg").getroot()
            self.assertEqual(
                svg.find("{http://www.w3.org/2000/svg}text").text, 'New <film> & "paper"'
            )
            self.assertEqual(path.read_bytes(), original)
            before = {p.name: p.read_bytes() for p in output.iterdir()}
            with self.assertRaisesRegex(ValueError, "fresh output directory"):
                reconstruct.propose(path, output)
            self.assertEqual(before, {p.name: p.read_bytes() for p in output.iterdir()})
            with self.assertRaisesRegex(ValueError, "fresh output directory"):
                reconstruct.propose(path, monochrome.ROOT / "assets/new-material")
            failed_input = root / "unresolved.json"
            failed_input.write_text(
                json.dumps(source_for(lambda t: 0.1 if t < 0.5 else 2.1, 0.001))
            )
            failed_output = root / "failed"
            with self.assertRaisesRegex(ValueError, "no coarse reconstruction fits"):
                reconstruct.propose(failed_input, failed_output)
            self.assertFalse(failed_output.exists())
            second = root / "second"
            reconstruct.propose(path, second)
            self.assertEqual(before, {p.name: p.read_bytes() for p in second.iterdir()})


if __name__ == "__main__":
    unittest.main()
