"""The distributed plug-in is versioned, complete and free of local configuration."""

import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
import zipfile

SPEC = importlib.util.spec_from_file_location(
    "lightroom_package", Path(__file__).with_name("lightroom_package.py")
)
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)
ROOT = Path(__file__).resolve().parents[1]


class PluginPackage(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir=ROOT / ".cache")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        bundle = self.source / "integrations/lightroom/Antitone.lrplugin"
        shutil.copytree(ROOT / "integrations/lightroom/Antitone.lrplugin", bundle)
        shutil.copyfile(ROOT / "LICENSE", self.source / "LICENSE")
        (bundle / "Config.local.lua").write_text('return { executable = "/private/build" }')
        (bundle / "scratch.txt").write_text("not a release input")
        for name in ("Sessions.lua", "Finish.lua", "Reveal.lua"):
            (bundle / name).write_text("obsolete source must not ship")

    def test_payload_is_allowlisted_and_versioned_without_modifying_source(self):
        path = package.package_plugin(self.source, self.root / "output", "1.2.3", 3)
        package.verify_plugin(path, "1.2.3", 3)
        with zipfile.ZipFile(path) as archive:
            names = archive.namelist()
            self.assertNotIn("Antitone.lrplugin/Config.local.lua", names)
            self.assertNotIn("Antitone.lrplugin/scratch.txt", names)
            for obsolete in ("Sessions.lua", "Finish.lua", "Reveal.lua"):
                self.assertNotIn(f"Antitone.lrplugin/{obsolete}", names)
            info = archive.read("Antitone.lrplugin/Info.lua").decode()
            self.assertIn("VERSION = { major = 1, minor = 2, revision = 3 }", info)
            self.assertIn("com.antitone.lightroom.experiment", info)
            self.assertIn(
                "Copyright (c) 2019 rxi", archive.read("Antitone.lrplugin/json.lua").decode()
            )
            manifest = json.loads(archive.read("Antitone.lrplugin/package.json"))
            self.assertEqual(manifest["protocol"], 3)
        original = (self.source / "integrations/lightroom/Antitone.lrplugin/Info.lua").read_text()
        self.assertIn("major = 0, minor = 1", original)

    def test_an_incompatible_application_cannot_stamp_a_valid_manifest(self):
        with self.assertRaisesRegex(ValueError, "protocol does not match"):
            package.package_plugin(self.source, self.root / "output", "1.2.3", 2)
        self.assertFalse((self.root / "output").exists())

    def test_relocated_archive_rejects_a_mismatched_application_or_changed_payload(self):
        path = package.package_plugin(self.source, self.root / "output", "1.2.3", 3)
        relocated = self.root / "outside checkout.zip"
        shutil.copyfile(path, relocated)
        for version, protocol in [("1.2.4", 3), ("1.2.3", 2)]:
            with self.assertRaisesRegex(ValueError, "versions do not match"):
                package.verify_plugin(relocated, version, protocol)
        with zipfile.ZipFile(relocated) as archive:
            contents = {name: archive.read(name) for name in archive.namelist()}
        contents["Antitone.lrplugin/Config.lua"] += b"\n-- changed\n"
        with zipfile.ZipFile(relocated, "w") as archive:
            for name, data in contents.items():
                archive.writestr(name, data)
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            package.verify_plugin(relocated, "1.2.3", 3)


if __name__ == "__main__":
    unittest.main()
