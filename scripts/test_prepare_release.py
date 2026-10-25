"""Release assembly rejects incomplete or mismatched inputs before publication."""

import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile


SPEC = importlib.util.spec_from_file_location(
    "prepare_release", Path(__file__).with_name("prepare-release.py")
)
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.macos = self.root / "macos"
        self.windows = self.root / "windows"
        self.output = self.root / "release"
        for directory, name in zip((self.macos, self.windows), release.PACKAGES, strict=True):
            directory.mkdir()
            with zipfile.ZipFile(directory / name, "w") as archive:
                archive.writestr("package/application", name.encode())
            self.checksum(directory, name)

    @staticmethod
    def checksum(directory, name):
        digest = hashlib.sha256((directory / name).read_bytes()).hexdigest()
        (directory / "SHA256SUMS").write_text(f"{digest}  {name}\n", encoding="ascii")

    def test_both_platforms_are_preserved_with_a_combined_verifiable_manifest(self):
        release.assemble(self.macos, self.windows, self.output)
        self.assertEqual(
            set(p.name for p in self.output.iterdir()), {*release.PACKAGES, "SHA256SUMS"}
        )
        recorded = dict(
            (name, digest)
            for digest, name in (
                line.split() for line in (self.output / "SHA256SUMS").read_text().splitlines()
            )
        )
        for directory, name in zip((self.macos, self.windows), release.PACKAGES, strict=True):
            source = (directory / name).read_bytes()
            self.assertEqual((self.output / name).read_bytes(), source)
            self.assertEqual(recorded[name], hashlib.sha256(source).hexdigest())

    def test_missing_windows_archive_never_creates_publishable_output(self):
        (self.windows / release.PACKAGES[1]).unlink()
        with self.assertRaises(FileNotFoundError):
            release.assemble(self.macos, self.windows, self.output)
        self.assertFalse(self.output.exists())

    def test_changed_archive_is_rejected_even_when_it_is_still_a_valid_zip(self):
        for directory, name in zip((self.macos, self.windows), release.PACKAGES, strict=True):
            with self.subTest(platform=name):
                original = (directory / name).read_bytes()
                with zipfile.ZipFile(directory / name, "a") as archive:
                    archive.writestr("unexpected", b"changed")
                with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                    release.assemble(self.macos, self.windows, self.output)
                self.assertFalse(self.output.exists())
                (directory / name).write_bytes(original)

    def test_wrong_package_and_extra_checksum_entries_are_rejected(self):
        manifest = self.windows / "SHA256SUMS"
        original = manifest.read_text()
        for text in [original.replace(release.PACKAGES[1], "other.zip"), original * 2]:
            with self.subTest(manifest=text):
                manifest.write_text(text)
                with self.assertRaisesRegex(ValueError, "exactly one checksum"):
                    release.assemble(self.macos, self.windows, self.output)
                self.assertFalse(self.output.exists())

    def test_valid_digest_does_not_make_an_empty_or_invalid_zip_a_package(self):
        name = release.PACKAGES[1]
        for invalid_zip in [False, True]:
            with self.subTest(invalid_zip=invalid_zip):
                if invalid_zip:
                    (self.windows / name).write_bytes(b"not a zip")
                else:
                    with zipfile.ZipFile(self.windows / name, "w"):
                        pass
                self.checksum(self.windows, name)
                with self.assertRaises((ValueError, zipfile.BadZipFile)):
                    release.assemble(self.macos, self.windows, self.output)
                self.assertFalse(self.output.exists())

    def test_existing_output_is_never_overwritten(self):
        self.output.mkdir()
        sentinel = self.output / "existing"
        sentinel.write_bytes(b"keep")
        with self.assertRaises(FileExistsError):
            release.assemble(self.macos, self.windows, self.output)
        self.assertEqual(sentinel.read_bytes(), b"keep")

    def test_tag_matches_the_package_not_another_workspace_member(self):
        manifest = self.root / "Cargo.toml"
        manifest.write_text('[package]\nversion = "1.2.3"\n[workspace]\nmembers = ["tool"]\n')
        release.validate_tag("v1.2.3", manifest)
        for tag in ["1.2.3", "v1.2.4", "v", "v1.2.3-extra"]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.validate_tag(tag, manifest)


if __name__ == "__main__":
    unittest.main()
