"""Source identities and current packed components, independent of the Rust reader."""

import json
import unittest
from .check import ROOT, validate


class Components(unittest.TestCase):
    def test_current_payloads_labels_and_cartesian_pairs_match_the_recorded_source(self):
        result = validate(ROOT / "assets/spektrafilm")
        self.assertGreater(result["negative_profiles"], 1)
        self.assertGreater(result["print_media"], 1)
        self.assertEqual(result["pairs"], result["negative_profiles"] * result["print_media"])

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
