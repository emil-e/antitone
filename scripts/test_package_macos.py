import hashlib
import importlib.util
import json
from pathlib import Path
import plistlib
import subprocess
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "package_macos", Path(__file__).with_name("package-macos.py")
)
packager = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packager)


class RuntimePaths(unittest.TestCase):
    def test_dependency_names_keep_spaces_but_not_otool_annotations(self):
        output = """Antitone.app/Contents/MacOS/antitone:
    @rpath/libomp.dylib (compatibility version 5.0.0, current version 5.0.0)
    /a checkout/native/libOther.dylib (compatibility version 0.0.0, current version 0.0.0)
"""
        self.assertEqual(
            packager.dependencies(output),
            ["@rpath/libomp.dylib", "/a checkout/native/libOther.dylib"],
        )

    def test_rpath_extraction_ignores_other_load_commands(self):
        output = """Load command 0
          cmd LC_RPATH
      cmdsize 48
         path /a checkout/lib (offset 12)
Load command 1
          cmd LC_LOAD_DYLIB
         name /usr/lib/libSystem.B.dylib (offset 24)
Load command 2
          cmd LC_RPATH
         path @executable_path/../Frameworks (offset 12)
"""
        self.assertEqual(
            packager.rpaths(output), ["/a checkout/lib", "@executable_path/../Frameworks"]
        )

    def test_only_system_libraries_and_the_bundled_runtime_are_accepted(self):
        packager.audit_dependencies(
            ["/usr/lib/libSystem.B.dylib", "/System/Library/Frameworks/Metal.framework/Metal"],
            bundled=False,
        )
        packager.audit_dependencies(["@rpath/libomp.dylib"], bundled=True)
        for name in [
            "/opt/homebrew/lib/libomp.dylib",
            "@rpath/libOpenColorIO.dylib",
            "@rpath/libMissing.dylib",
            "/usr/library/libFake.dylib",
            "/usr/lib/../../opt/homebrew/lib/libFake.dylib",
            "@loader_path/../../outside.dylib",
        ]:
            with self.subTest(name=name), self.assertRaisesRegex(RuntimeError, "dependency"):
                packager.audit_dependencies([name], bundled=True)
        with self.assertRaises(RuntimeError):
            packager.audit_dependencies(["@rpath/libomp.dylib"], bundled=False)


class NoticeCollection(unittest.TestCase):
    def test_libraw_notices_follow_the_corrected_source_revision_or_explicit_override(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "native/libraw/dht-hashes.txt"
            manifest.parent.mkdir(parents=True)
            with (
                patch.object(packager, "ROOT", root),
                patch.dict(packager.os.environ, {}, clear=True),
            ):
                for digest in ["a" * 64, "b" * 64]:
                    manifest.write_text(f"upstream={'0' * 64}\npatched={digest}\n")
                    expected = root / f".cache/native-deps/LibRaw-0.22.3-{digest[:12]}-openmp"
                    self.assertEqual(packager.libraw_source(), expected)
                manifest.unlink()
                with patch.dict(packager.os.environ, {"LIBRAW_SOURCE": str(root / "shared")}):
                    self.assertEqual(packager.libraw_source(), root / "shared")

    def test_colour_material_notices_include_the_academy_terms_without_runtime_blobs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "assets/spektrafilm"
            source.mkdir(parents=True)
            files = {
                "LICENSE-code.txt": b"code terms\n",
                "LICENSE-data.txt": b"data terms\n",
                "NOTICE-Academy-APD.txt": "Academy notice: ©2010\n".encode(),
                "CHANGELOG.txt": b"modifications\n",
                "provenance.json": b'{"source": "historical reference"}\n',
            }
            for name, data in files.items():
                (source / name).write_bytes(data)
            (source / "catalog.json.z").write_bytes(b"compiled material data")
            output = root / "notices"
            with patch.object(packager, "ROOT", root):
                packager.copy_spektrafilm_notices(output)
            retained = output / "spektrafilm"
            self.assertEqual({path.name for path in retained.iterdir()}, set(files))
            for name, data in files.items():
                self.assertEqual((retained / name).read_bytes(), data)

    def test_material_notices_follow_bound_paper_and_negative_sources(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            files = {
                "assets/papers/README.md": b"material assumptions",
                "assets/papers/paper/source.json": b"paper source",
                "assets/negatives/negative/source.json": b"negative source",
                "assets/negatives/unbound/source.json": b"not in the compiled catalogue",
            }
            for name, data in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            inputs = {
                name: hashlib.sha256(files[name]).hexdigest()
                for name in [
                    "assets/papers/paper/source.json",
                    "assets/negatives/negative/source.json",
                ]
            }
            inputs["native/helper.cpp"] = "producer hash, not a source notice"
            provenance = root / "assets/monochrome/provenance.json"
            provenance.parent.mkdir(parents=True)
            provenance.write_text(json.dumps({"inputs": inputs}))
            output = root / "notices"
            with patch.object(packager, "ROOT", root):
                packager.copy_monochrome_notices(output)
            self.assertEqual((output / "papers/paper/source.json").read_bytes(), b"paper source")
            self.assertEqual(
                (output / "negatives/negative/source.json").read_bytes(), b"negative source"
            )
            self.assertEqual(
                (output / "monochrome/provenance.json").read_bytes(), provenance.read_bytes()
            )
            self.assertEqual((output / "papers/README.md").read_bytes(), b"material assumptions")
            self.assertFalse((output / "negatives/unbound").exists())
            self.assertFalse((output / "native").exists())

    def test_nested_codec_and_font_notices_are_retained_without_binary_assets(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            crate = root / "crate"
            crate.mkdir()
            (crate / "Cargo.toml").touch()
            for name in [
                "LICENSE.MIT",
                "LICENSE.APACHE",
                "vendor/LICENSE",
                "fonts/OFL.txt",
                "fonts/UFL.txt",
                "fonts/Hack-Regular.txt",
            ]:
                path = crate / name
                path.parent.mkdir(exist_ok=True)
                path.write_text(f"copyright and permission for {name}")
            (crate / "fonts/font.ttf").write_bytes(b"font data")
            (crate / "copying.rs").write_text("// Not a license document")
            package = {
                "name": "fixture",
                "version": "1",
                "manifest_path": str(crate / "Cargo.toml"),
            }
            output = root / "output"
            files = packager.collect_rust_notices(package, output, [])
            self.assertEqual(
                set(files),
                {
                    "LICENSE.MIT",
                    "LICENSE.APACHE",
                    "vendor/LICENSE",
                    "fonts/OFL.txt",
                    "fonts/UFL.txt",
                    "fonts/Hack-Regular.txt",
                },
            )
            for name in files:
                self.assertEqual((output / name).read_bytes(), (crate / name).read_bytes())
            self.assertFalse((output / "fonts/font.ttf").exists())

    def test_missing_notices_stay_unresolved_and_supplements_are_version_specific(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "Cargo.toml"
            manifest.touch()
            package = {"name": "fixture", "version": "2", "manifest_path": str(manifest)}
            supplement = [
                {
                    "packages": ["fixture@1"],
                    "copyright": "Copyright fixture author",
                    "source": "reviewed upstream",
                }
            ]
            licenses = root / "licenses"
            licenses.mkdir()
            shutil.copy2(packager.ROOT / "licenses/MIT-permission.txt", licenses)
            self.assertEqual(
                packager.collect_rust_notices(package, root / "out", supplement, licenses),
                [],
            )
            package["version"] = "1"
            files = packager.collect_rust_notices(package, root / "out", supplement, licenses)
            self.assertIn("LICENSE-MIT-supplemental.txt", files)
            notice = (root / "out/LICENSE-MIT-supplemental.txt").read_text()
            self.assertIn("Copyright fixture author", notice)
            self.assertIn("Permission is hereby granted", notice)
            self.assertIn("THE SOFTWARE IS PROVIDED", notice)
            # No MIT template exists in the injected licenses directory yet;
            # the template is a real repository license file.
            self.assertTrue((packager.ROOT / "licenses/MIT-permission.txt").is_file())

    def test_text_mode_supplements_pin_upstream_notice_bytes_with_provenance(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "Cargo.toml"
            manifest.touch()
            package = {"name": "fixture", "version": "1", "manifest_path": str(manifest)}
            supplement = [
                {
                    "packages": ["fixture@1"],
                    "mode": "text",
                    "file": "fixture-LICENSE.md",
                    "source": "https://example/fixture/blob/deadbeef/LICENSE.md",
                    "note": "pinned VCS commit rewritten away upstream",
                }
            ]
            notice = root / "licenses" / "fixture-LICENSE.md"
            notice.parent.mkdir()
            notice.write_text("upstream pointer notice bytes\n")
            files = packager.collect_rust_notices(
                package, root / "out", supplement, root / "licenses"
            )
            self.assertIn("fixture-LICENSE.md", files)
            self.assertEqual((root / "out/fixture-LICENSE.md").read_bytes(), notice.read_bytes())
            self.assertFalse((root / "out/LICENSE-MIT-supplemental.txt").exists())
            provenance = json.loads((root / "out/notice-provenance.json").read_text())
            self.assertEqual(provenance["packages"], ["fixture@1"])
            self.assertEqual(provenance["mode"], "text")
            self.assertEqual(
                provenance["source"], "https://example/fixture/blob/deadbeef/LICENSE.md"
            )
            self.assertIn("rewritten away", provenance["note"])


class VendoredSource(unittest.TestCase):
    def test_upstream_bytes_and_complete_manifest_are_preserved(self):
        root = Path(__file__).resolve().parents[1] / "vendor/dng-sdk"
        recorded = set()
        for line in (root / "SHA256SUMS").read_text().splitlines():
            digest, name = line.split("  ", 1)
            self.assertNotIn(name, recorded)
            recorded.add(name)
            self.assertEqual(hashlib.sha256((root / name).read_bytes()).hexdigest(), digest, name)
        actual = {str(p.relative_to(root)) for p in root.rglob("*") if p.is_file()}
        self.assertEqual(actual - {"SHA256SUMS", "PROVENANCE.md"}, recorded)


@unittest.skipUnless(sys.platform == "darwin", "Requires Apple's loader and signing tools")
class RelocatedBundle(unittest.TestCase):
    def test_verifier_runs_a_signed_bundle_and_rejects_checkout_rpaths(self):
        cache = Path(__file__).resolve().parents[1] / ".cache"
        cache.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="package test ", dir=cache) as directory:
            root = Path(directory)
            bundle = root / "Antitone.app"
            binary = bundle / "Contents/MacOS/antitone"
            runtime = bundle / "Contents/Frameworks/libomp.dylib"
            binary.parent.mkdir(parents=True)
            runtime.parent.mkdir(parents=True)
            (root / "runtime.c").write_text("int fixture_value(void) { return 7; }\n")
            (root / "main.c").write_text(
                "#include <stdio.h>\nint fixture_value(void);\n"
                "int main(void) { if (fixture_value() != 7) return 1; "
                'puts("Antitone fixture"); return 0; }\n'
            )

            def run(*args):
                subprocess.run(list(map(str, args)), check=True)

            run(
                "clang",
                "-dynamiclib",
                root / "runtime.c",
                "-o",
                runtime,
                "-Wl,-install_name,@rpath/libomp.dylib",
            )
            run(
                "clang",
                root / "main.c",
                runtime,
                "-o",
                binary,
                "-Wl,-rpath,@executable_path/../Frameworks",
                "-Wl,-headerpad_max_install_names",
            )
            with (bundle / "Contents/Info.plist").open("wb") as file:
                plistlib.dump(
                    {
                        "CFBundleExecutable": "antitone",
                        "CFBundleIdentifier": "local.antitone.fixture",
                        "CFBundlePackageType": "APPL",
                    },
                    file,
                )
            run("codesign", "--force", "--sign", "-", runtime)
            run("codesign", "--force", "--sign", "-", bundle)
            packager.verify_bundle(bundle, "fixture")
            run("install_name_tool", "-add_rpath", str(root / "checkout"), binary)
            with self.assertRaisesRegex(RuntimeError, "search paths"):
                packager.verify_bundle(bundle, "fixture")
            runtime.unlink()
            with self.assertRaisesRegex(RuntimeError, "missing"):
                packager.verify_bundle(bundle, "fixture")
