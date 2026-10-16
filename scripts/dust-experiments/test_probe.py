import json
from pathlib import Path
import subprocess
import tempfile
import unittest

import cv2
import numpy as np
from PIL import Image


class DevelopmentProbe(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.probe = Path("target/release/antitone-dust-source-probe").resolve()
        if not cls.probe.exists():
            raise unittest.SkipTest(
                "Build the release source probe to run native integration tests"
            )

    def setUp(self):
        root = Path(".cache/dust")
        root.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=root)
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.info = {
            "width": 2,
            "height": 2,
            "stride": 1,
            "processing_model": 4,
            "dmin": [0.2] * 3,
            "dmax": [0.002] * 3,
            "camera_to_ap1_columns": [1, 0, 0, 0, 1, 0, 0, 0, 1],
            "roll": {
                "response": {"kind": "film", "toe": 2.5, "shoulder": 0},
                "gamma": 0.65,
                "balance": {"warmth": 0, "tint": 0},
                "exposure": 0,
            },
            "photo": {
                "exposure": 0,
                "wb": {"warmth": 0, "tint": 0},
                "contrast": 1,
                "pivot": 0.18,
                "contrast_colour_mix": 0,
                "saturation": 1,
                "saturation_density": 0,
            },
        }
        self.base = np.array(
            [[-0.0, -0.1, 2], [4, 5, 6], [-1, 0.0, 1.2], [3, -0.2, -0.0]], dtype="<f4"
        )
        self.camera = np.full((4, 3), np.nan, dtype="<f4")
        self.camera[1] = (0.1, 0.05, 0.3)
        self.mask = np.array([0, 1, 0, 0], np.uint8)

    def run_probe(self, name="result", success=True):
        (self.directory / "source.json").write_text(json.dumps(self.info))
        self.base.tofile(self.directory / "working.ap1-f32")
        self.camera.tofile(self.directory / "camera-f32")
        self.mask.tofile(self.directory / "mask-u8")
        output = self.directory / (name + ".ap1-f32")
        result = subprocess.run(
            [
                str(self.probe),
                "develop",
                str(self.directory),
                str(self.directory / "camera-f32"),
                str(self.directory / "mask-u8"),
                str(output),
            ],
            capture_output=True,
            text=True,
        )
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
            return np.fromfile(output, dtype="<f4").reshape(4, 3), json.loads(result.stdout)
        self.assertNotEqual(result.returncode, 0)
        return output

    def test_only_selected_pixels_develop_and_masked_base_is_unused(self):
        a, _ = self.run_probe("a")
        self.base[1] = -100
        b, _ = self.run_probe("b")
        np.testing.assert_array_equal(a.view(np.uint32), b.view(np.uint32))
        np.testing.assert_array_equal(
            a[self.mask == 0].view(np.uint32), self.base[self.mask == 0].view(np.uint32)
        )
        self.assertTrue(np.isfinite(a).all())
        self.assertLess(a[1, 2], 0)

    def test_empty_mask_is_byte_identity(self):
        self.mask[:] = 0
        result, statistics = self.run_probe()
        self.assertEqual(result.tobytes(), self.base.tobytes())
        self.assertEqual(statistics["masked_pixels"], 0)

    def test_production_floor_is_reported(self):
        self.camera[1, 1] = 0
        _, statistics = self.run_probe()
        self.assertEqual(statistics["floor_pixels"], 1)
        self.assertEqual(statistics["floor_channels"], [0, 1, 0])

    def test_nonfinite_selected_camera_rejects_without_output(self):
        self.camera[1, 0] = np.nan
        self.assertFalse(self.run_probe(success=False).exists())

    def test_unsupported_model_or_proxy_rejects_without_output(self):
        for field, value in (("processing_model", 3), ("stride", 7)):
            old = self.info[field]
            self.info[field] = value
            self.assertFalse(self.run_probe(name=field, success=False).exists())
            self.info[field] = old

    def test_mismatched_mask_rejects_without_output(self):
        self.mask = self.mask[:3]
        self.assertFalse(self.run_probe(success=False).exists())

    def test_existing_output_is_untouched(self):
        output = self.directory / "result.ap1-f32"
        output.write_bytes(b"existing output")
        self.run_probe(success=False)
        self.assertEqual(output.read_bytes(), b"existing output")

    def run_tiff(self, name="input", target="srgb", success=True):
        (self.directory / "source.json").write_text(json.dumps(self.info))
        working = np.array(
            [[0.01, 0.02, 0.03], [0.1, 0.2, 0.3], [0.5, 0.4, 0.2], [1.0, 0.8, 0.6]], dtype="<f4"
        )
        working.tofile(self.directory / "tiff-input.ap1-f32")
        output = self.directory / (name + ".tif")
        result = subprocess.run(
            [
                str(self.probe),
                "tiff",
                str(self.directory),
                str(self.directory / "tiff-input.ap1-f32"),
                str(output),
                target,
            ],
            capture_output=True,
            text=True,
        )
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
        return output

    def test_production_tiff_is_rgb16_with_icc_and_description(self):
        output = self.run_tiff()
        decoded = cv2.imread(str(output), cv2.IMREAD_UNCHANGED)
        self.assertEqual(decoded.shape, (2, 2, 3))
        self.assertEqual(decoded.dtype, np.uint16)
        with Image.open(output) as image:
            self.assertEqual(image.size, (2, 2))
            self.assertTrue(image.tag_v2.get(34675))
            description = image.tag_v2.get(270)
            self.assertIn("Antitone model 4", description)
            self.assertIn("Srgb", description)
            self.assertIn("film toe=2.5", description)

    def test_tiff_rejects_proxy_target_mismatch_and_existing_output(self):
        self.info["stride"] = 7
        self.assertFalse(self.run_tiff(name="proxy", success=False).exists())
        self.info["stride"] = 1
        self.assertFalse(self.run_tiff(name="target", target="unknown", success=False).exists())
        output = self.directory / "existing.tif"
        output.write_bytes(b"existing output")
        self.run_tiff(name="existing", success=False)
        self.assertEqual(output.read_bytes(), b"existing output")


if __name__ == "__main__":
    unittest.main()
