"""Photo-free catalog/source tests; opt-in native checks use existing OCIO setup."""
from contextlib import contextmanager
from dataclasses import replace
from pathlib import Path
import copy
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import catalog
import derive
import looks

MONO = "ilford-multigrade-rc-deluxe-filter-2"
PORTRA = "kodak-portra-endura"


@contextmanager
def isolated_assets(identifier=PORTRA):
    looks.CACHE.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=looks.CACHE) as folder:
        root = Path(folder)
        shutil.copytree(catalog.ASSETS / "common", root / "common")
        shutil.copytree(catalog.ASSETS / identifier, root / identifier)
        yield root


def change_data(assets, identifier, filename, transform, update_digest=True):
    path = assets / identifier / filename
    value = catalog.read_json(path)
    transform(value)
    path.write_text(json.dumps(value) + "\n")
    if update_digest:
        manifest = assets / identifier / "look.json"
        look = catalog.read_json(manifest)
        look[filename.removesuffix(".json") + "_sha256"] = catalog.digest(path)
        manifest.write_text(json.dumps(look) + "\n")


class CatalogTests(unittest.TestCase):
    def test_canonical_set_and_selection(self):
        self.assertEqual(set(catalog.identifiers()), {PORTRA, MONO, "kodak-2383", "kodak-pro-image-ii",
                         "kodak-endura-premier", "kodak-ektacolor-edge", "kodak-endura-metallic",
                         "kodak-endura-premier-metallic"})
        statuses = {i: catalog.load(i).look["status"] for i in catalog.identifiers()}
        self.assertEqual(statuses[PORTRA], "selected")
        self.assertEqual(statuses[MONO], "accepted")
        self.assertEqual(statuses["kodak-2383"], "accepted")
        self.assertEqual(statuses["kodak-pro-image-ii"], "accepted")
        self.assertEqual({i for i, s in statuses.items() if s == "shelved"},
                         {"kodak-endura-metallic", "kodak-endura-premier-metallic"})

    def test_all_source_ledgers_and_frozen_models(self):
        for identifier in catalog.identifiers():
            with self.subTest(look=identifier):
                result = derive.validate(catalog.load(identifier))
                self.assertLessEqual(result["source_interval_error"], .025)
                self.assertLessEqual(result["source_coverage_error"], .025)

    def test_portra_is_source_reconstructed_not_old_tabulated_definition(self):
        item = catalog.load(PORTRA)
        self.assertEqual(item.source["reconstruction"]["family"], "generalized-logistic-5")
        self.assertIn("Canonical source-reconstructed Portra", item.look["notes"][0])

    def test_unresolved_spectral_tails_keep_censor_metadata(self):
        rows = catalog.load(PORTRA).source["dye_readings"]["yellow"]
        self.assertTrue(rows[-1]["censored_below_graph_resolution"])
        self.assertEqual(rows[-1]["density"], 0.)
        with isolated_assets() as assets:
            change_data(assets, PORTRA, "source.json", lambda s: s["dye_readings"]["yellow"][-1].pop("censored_below_graph_resolution"))
            with self.assertRaises(ValueError):
                catalog.load(PORTRA, assets)

    def test_monochrome_is_scalar_and_analytic(self):
        item = catalog.load(MONO)
        self.assertTrue(item.monochrome)
        self.assertNotIn("dye_density", item.model)
        self.assertEqual(set(item.source["reading_uncertainty"]), {"density", "log_exposure"})
        self.assertNotIn("densitometry", item.source["conditions"])
        self.assertEqual(item.model["density_parameters"], item.source["reconstruction"]["parameters"][0])
        generated = looks.header(item)
        self.assertIn("#define LOOK_NEUTRAL_DENSITY 1", generated)
        self.assertIn("mono_parameters[5]", generated)
        self.assertNotIn("scalar_density[", generated)
        self.assertNotIn("observer[", generated)

    def test_generated_spectral_headers_use_common_data(self):
        for identifier in catalog.identifiers():
            item = catalog.load(identifier)
            if not item.monochrome:
                generated = looks.header(item)
                self.assertIn("observer[61][3]", generated)
                self.assertIn("density_curve[256][3]", generated)
                self.assertNotIn("LOOK_NEUTRAL_DENSITY", generated)

    def test_wrong_source_channel_identity_is_rejected(self):
        item = catalog.load(PORTRA)
        source = copy.deepcopy(item.source)
        params = source["reconstruction"]["parameters"]
        params[0] = params[1] = params[2]
        with self.assertRaises(ValueError):
            derive.validate(replace(item, source=source))

    def test_wrong_monochrome_response_is_rejected(self):
        item = catalog.load(MONO)
        source = copy.deepcopy(item.source)
        source["reconstruction"]["parameters"][0][3] += .2
        with self.assertRaises(ValueError):
            derive.validate(replace(item, source=source))

    def test_wrong_effective_coefficients_are_rejected(self):
        item = catalog.load(PORTRA)
        model = copy.deepcopy(item.model)
        model["effective_coefficients"][100][0] += .02
        with self.assertRaisesRegex(ValueError, "source densities"):
            derive.validate(replace(item, model=model))

    def test_shared_observer_is_generated_not_arbitrary_table(self):
        _, spectral, _ = catalog.common()
        derive.common_equations(spectral)
        spectral["observer"][20][0] += .001
        with self.assertRaisesRegex(ValueError, "observer"):
            derive.common_equations(spectral)

    def test_changed_frozen_data_requires_explicit_identity_update(self):
        with isolated_assets() as assets:
            change_data(assets, PORTRA, "model.json", lambda m: m["effective_coefficients"][0].__setitem__(0, .2), False)
            with self.assertRaisesRegex(ValueError, "changed frozen"):
                catalog.load(PORTRA, assets)

    def test_negative_density_is_rejected_even_with_updated_digest(self):
        with isolated_assets() as assets:
            change_data(assets, PORTRA, "model.json", lambda m: m["dye_density"][0].__setitem__(0, -.1))
            with self.assertRaisesRegex(ValueError, "negative"):
                catalog.load(PORTRA, assets)

    def test_boolean_and_wrong_shape_are_not_numeric_model_data(self):
        for invalid in (True, [1, 2]):
            with isolated_assets() as assets:
                change_data(assets, PORTRA, "model.json", lambda m: m["effective_coefficients"].__setitem__(0, invalid))
                with self.assertRaises(ValueError):
                    catalog.load(PORTRA, assets)

    def test_unknown_model_family_is_rejected(self):
        with isolated_assets() as assets:
            change_data(assets, PORTRA, "model.json", lambda m: m.__setitem__("family", "unknown"))
            with self.assertRaisesRegex(ValueError, "unsupported"):
                catalog.load(PORTRA, assets)

    def test_nonuniform_exposure_grid_is_rejected(self):
        with isolated_assets() as assets:
            change_data(assets, PORTRA, "model.json", lambda m: m["log_exposure"].__setitem__(10, m["log_exposure"][10] + .001))
            with self.assertRaisesRegex(ValueError, "nonuniform"):
                catalog.load(PORTRA, assets)

    def test_reversed_source_interval_is_rejected(self):
        with isolated_assets() as assets:
            change_data(assets, PORTRA, "source.json", lambda s: s["constraints"][0]["channel_intervals"].__setitem__(0, [1., 0.]))
            with self.assertRaisesRegex(ValueError, "reversed"):
                catalog.load(PORTRA, assets)

    def test_malformed_json_constants_and_duplicate_fields_are_rejected(self):
        looks.CACHE.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=looks.CACHE) as folder:
            path = Path(folder) / "input.json"
            for text in ('{"x": NaN}', '{"x": Infinity}', '{"x":1,"x":2}', '{"x":1e999}', '{"x":' + '1' + '0' * 400 + '}'):
                path.write_text(text)
                with self.assertRaises(ValueError):
                    catalog.read_json(path)

    def test_schema_and_look_revisions_are_integers_not_booleans_or_floats(self):
        for invalid in (True, 1.0):
            with isolated_assets() as assets:
                path = assets / PORTRA / "look.json"
                data = catalog.read_json(path)
                data["version"] = invalid
                path.write_text(json.dumps(data))
                with self.assertRaises(ValueError):
                    catalog.load(PORTRA, assets)

    def test_changed_authoring_gates_and_unknown_fields_are_rejected(self):
        for key, value in (("reference_roundtrip_limit", 1.), ("unexpected", 1.)):
            with isolated_assets() as assets:
                path = assets / "common/authoring.json"
                data = catalog.read_json(path)
                data[key] = value
                path.write_text(json.dumps(data))
                with self.assertRaises(ValueError):
                    catalog.common(assets)

    def test_invalid_ids_cannot_traverse_or_inject_paths(self):
        for identifier in ("../common", "", "/tmp/look", "x;echo", "A", "x/y"):
            with self.assertRaises(ValueError):
                catalog.load(identifier)

    def test_maintained_assets_are_text_and_have_no_private_path_dependencies(self):
        for path in catalog.ASSETS.rglob("*"):
            if path.is_file():
                self.assertIn(path.suffix, (".json", ".md"))
                text = path.read_text()
                self.assertNotIn("/Users/", text)
                self.assertNotIn(".worktrees/", text)
                self.assertNotIn(".pfm", text)

    def test_cli_list_and_invalid_size_need_no_native_setup(self):
        cli = Path(looks.__file__)
        listed = subprocess.run([sys.executable, str(cli), "list"], capture_output=True, text=True)
        self.assertEqual(listed.returncode, 0, listed.stderr)
        self.assertIn("shelved", listed.stdout)
        failed = subprocess.run([sys.executable, str(cli), "bake", PORTRA, "--size", "33"], capture_output=True)
        self.assertNotEqual(failed.returncode, 0)

    def test_bake_refuses_existing_or_tracked_output_before_building(self):
        item = catalog.load(PORTRA)
        looks.CACHE.mkdir(parents=True, exist_ok=True)
        for output in (catalog.ROOT / "assets/looks/new-bake", looks.CACHE):
            with mock.patch.object(looks, "build") as build:
                with self.assertRaises(ValueError):
                    looks.bake(item, 65, output)
                build.assert_not_called()

    def test_destination_claim_race_preserves_other_invocation(self):
        item = catalog.load(PORTRA)
        looks.CACHE.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=looks.CACHE) as folder:
            output = Path(folder) / "raced"

            def competitor(_):
                output.mkdir()
                (output / "validation.json").write_text('{"status":"passed","owner":"other"}\n')
                return Path("unused-native-tool"), {}

            with mock.patch.object(looks, "build", side_effect=competitor), mock.patch.object(looks.subprocess, "run") as run:
                with self.assertRaises(FileExistsError):
                    looks.bake(item, 65, output)
                run.assert_not_called()
            self.assertEqual(catalog.read_json(output / "validation.json"), {"status": "passed", "owner": "other"})
            self.assertEqual({p.name for p in output.iterdir()}, {"validation.json"})

    def test_failed_compile_does_not_publish_partial_cache_executable(self):
        looks.CACHE.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=looks.CACHE) as folder:
            root = Path(folder)
            prefix = root / "dependency"
            (prefix / "include/OpenColorIO").mkdir(parents=True)
            (prefix / "include/OpenColorIO/OpenColorIO.h").write_text("// test header\n")
            (prefix / "lib").mkdir()

            def compiler(command, **_):
                if command[-1] == "--version":
                    return subprocess.CompletedProcess(command, 0, stdout="test compiler\n")
                candidate = Path(command[command.index("-o") + 1])
                candidate.write_bytes(b"partial")
                raise subprocess.CalledProcessError(1, command)

            with mock.patch.dict(os.environ, {"OCIO_PREFIX": str(prefix), "CXX": "test-compiler"}), mock.patch.object(looks, "CACHE", root / "build-cache"), mock.patch.object(looks.subprocess, "run", side_effect=compiler):
                with self.assertRaises(subprocess.CalledProcessError):
                    looks.build(catalog.load(PORTRA))
            self.assertEqual(list((root / "build-cache").rglob("look-tool")), [])
            self.assertEqual(list((root / "build-cache").rglob("compile-*")), [])

    def test_native_failure_reason_is_preserved_and_false_success_rejected(self):
        failed = subprocess.CompletedProcess([], 1, stdout="", stderr="reference admission failed")
        with mock.patch.object(looks.subprocess, "run", return_value=failed):
            with self.assertRaisesRegex(ValueError, "reference admission failed"):
                looks.execute(Path("native-tool"), "check")
        false_pass = subprocess.CompletedProcess([], 0, stdout='{"status":"failed"}', stderr="")
        with mock.patch.object(looks.subprocess, "run", return_value=false_pass):
            with self.assertRaises(ValueError):
                looks.execute(Path("native-tool"), "check")

    def test_numeric_payload_identity_ignores_only_descriptive_header(self):
        looks.CACHE.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=looks.CACHE) as folder:
            a, b = Path(folder) / "a.cube", Path(folder) / "b.cube"
            payload = "LUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n0 0 0\n"
            a.write_text('# comment\nTITLE "A"\n' + payload)
            b.write_text('TITLE "B"\n' + payload)
            self.assertEqual(catalog.payload_hash(a), catalog.payload_hash(b))
            b.write_text('TITLE "B"\n' + payload.replace("0 0 0\n", "0.1 0 0\n"))
            self.assertNotEqual(catalog.payload_hash(a), catalog.payload_hash(b))


@unittest.skipUnless(os.environ.get("LOOKS_NATIVE_TESTS") == "1", "opt-in native tests require existing pinned OCIO")
class NativeTests(unittest.TestCase):
    def test_all_native_synthetic_contracts(self):
        for identifier in catalog.identifiers():
            with self.subTest(look=identifier):
                report = looks.native_check(catalog.load(identifier))
                self.assertEqual(report["status"], "passed")
                self.assertLess(report["gray_ap1_error"], 2e-5)
                self.assertLess(report["synthetic_reference_roundtrip_max"], 2e-4)

    def test_native_refuses_nonempty_output_without_replacing_files(self):
        executable, _ = looks.build(catalog.load(PORTRA))
        with tempfile.TemporaryDirectory(dir=looks.CACHE) as folder:
            output = Path(folder)
            sentinel = output / "validation.json"
            sentinel.write_text('{"owner":"other"}\n')
            result = subprocess.run([str(executable), "bake", "65", str(output)], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(sentinel.read_text(), '{"owner":"other"}\n')
            self.assertEqual({p.name for p in output.iterdir()}, {"validation.json"})

    def test_source_only_copy_needs_no_worktrees_photographs_or_app_code(self):
        with tempfile.TemporaryDirectory(dir=looks.CACHE) as folder:
            root = Path(folder)
            for relative in ("assets/looks", "native/looks", "scripts/looks"):
                shutil.copytree(catalog.ROOT / relative, root / relative, ignore=shutil.ignore_patterns("__pycache__"))
            cli = root / "scripts/looks/looks.py"
            checked = subprocess.run([sys.executable, str(cli), "check", "--all"], capture_output=True, text=True)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            self.assertEqual(json.loads(checked.stdout)["status"], "passed")
            env = os.environ.copy()
            # The source-only copy deliberately has no dependency installation.
            # Reuse the parent's normal prefix, including its default location.
            env["OCIO_PREFIX"] = str(Path(env.get("OCIO_PREFIX", catalog.ROOT / ".cache/deps/ocio-install")).resolve())
            baked = subprocess.run([sys.executable, str(cli), "bake", PORTRA, "--size", "65"], capture_output=True, text=True, env=env)
            self.assertEqual(baked.returncode, 0, baked.stderr)
            self.assertEqual(json.loads(baked.stdout)["status"], "passed")
            self.assertFalse((root / "native/rendering_transform.h").exists())
            report = catalog.read_json(root / ".cache/looks/bakes" / PORTRA / "65/validation.json")
            self.assertEqual(report["numeric_payload_sha256"], catalog.load(PORTRA).look["reviewed_bakes"]["65"]["numeric_payload_sha256"])

    def test_native_invalid_arguments_do_not_create_outputs(self):
        executable, _ = looks.build(catalog.load(PORTRA))
        looks.CACHE.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=looks.CACHE) as folder:
            output = Path(folder) / "invalid"
            for args in ([], ["check", "extra"], ["bake", "65junk", str(output)], ["bake", "2", str(output)]):
                result = subprocess.run([str(executable), *args], capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
