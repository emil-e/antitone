"""Source identities and current packed components, independent of the Rust reader."""

import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from . import monochrome
from .check import ROOT, validate


class Components(unittest.TestCase):
    def test_current_payloads_labels_and_cartesian_pairs_match_the_recorded_source(self):
        result = validate(ROOT / "assets/spektrafilm")
        self.assertGreater(result["negative_profiles"], 1)
        self.assertGreater(result["print_media"], 1)
        self.assertEqual(result["pairs"], result["negative_profiles"] * result["print_media"])

    def test_monochrome_bytes_survive_windows_checkout_conversion(self):
        paths = [*monochrome.PRODUCERS, *monochrome.ASSETS.iterdir(), monochrome.FIXTURE]
        files = {path.relative_to(ROOT).as_posix(): path.read_bytes() for path in paths}
        (ROOT / ".cache").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix="monochrome-checkout-", dir=ROOT / ".cache"
        ) as temporary:
            checkout = Path(temporary)
            git = ["git", "-C", temporary, "-c", "core.autocrlf=true", "-c", "core.safecrlf=false"]
            subprocess.run([*git, "init", "--quiet"], check=True, capture_output=True)
            attributes = (ROOT / ".gitattributes").read_bytes()
            (checkout / ".gitattributes").write_bytes(
                attributes + b"\ncheckout-control.txt text !eol\n"
            )
            control = "checkout-control.txt"
            for name, data in {**files, control: b"first\nsecond\n"}.items():
                path = checkout / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            subprocess.run([*git, "add", "--all"], check=True, capture_output=True)
            # Prove checkout filters really convert an unpinned text control.
            self.assertEqual(
                subprocess.check_output([*git, "cat-file", "--filters", ":" + control]),
                b"first\r\nsecond\r\n",
            )
            for name, expected in files.items():
                with self.subTest(path=name):
                    actual = subprocess.check_output([*git, "cat-file", "--filters", ":" + name])
                    self.assertEqual(actual, expected, "checkout changed provenance-bound bytes")

    def test_corruption_is_rejected_without_mutating_any_asset(self):
        folder = ROOT / "assets/spektrafilm"
        provenance = json.loads((folder / "provenance.json").read_text())
        files = {
            name: (folder / name).read_bytes()
            for name in provenance["files"]
            if name != "conformance.json"
        }
        self.assertGreater(validate(folder, files)["pairs"], 1)
        key = "catalog.json.z"
        changed = dict(files)
        raw = changed[key]
        changed[key] = raw[:-1] + bytes([raw[-1] ^ 1])
        with self.assertRaisesRegex(ValueError, "component identity changed"):
            validate(folder, changed)


if __name__ == "__main__":
    unittest.main()
