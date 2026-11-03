"""Acquisition, environment and opt-in generation boundaries without downloads."""

import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from . import build


class GenerationWorkflow(unittest.TestCase):
    def test_cached_source_must_match_the_pin_and_be_clean(self):
        for revision, status, accepted in [
            (build.REVISION, "", True),
            ("a" * 40, "", False),
            (build.REVISION, "?? changed.py", False),
        ]:
            with (
                self.subTest(revision=revision, status=status),
                patch.object(build, "captured", side_effect=[revision, status]),
            ):
                if accepted:
                    self.assertEqual(build.verify_source(Path(".")), Path(".").resolve())
                else:
                    with self.assertRaises(ValueError):
                        build.verify_source(Path("."))

    def test_existing_verified_source_does_not_fetch_and_failed_acquisition_is_not_published(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            upstream = root / "upstream"
            upstream.mkdir()
            with (
                patch.object(build, "CACHE", root),
                patch.object(build, "UPSTREAM", upstream),
                patch.object(build, "verify_source", return_value=upstream) as verify,
                patch.object(build, "run") as run,
            ):
                self.assertEqual(build.acquire_source(), upstream)
                verify.assert_called_once_with(upstream)
                run.assert_not_called()
            upstream.rmdir()
            with (
                patch.object(build, "CACHE", root),
                patch.object(build, "UPSTREAM", upstream),
                patch.object(build, "run", side_effect=subprocess.CalledProcessError(1, "git")),
            ):
                with self.assertRaises(subprocess.CalledProcessError):
                    build.acquire_source()
                self.assertFalse(upstream.exists())
                self.assertEqual(list(root.iterdir()), [])

    def test_generation_is_explicit_and_checks_candidates_without_promoting_them(self):
        source = Path("source")
        output = Path("candidate")
        with (
            patch.object(build, "verify_source", return_value=source),
            patch.object(build.sys, "platform", "darwin"),
            patch.object(build, "run") as run,
            patch.object(build, "acquire_source") as fetch,
        ):
            build.generate(Path("python"), source, output)
            fetch.assert_not_called()
            self.assertEqual(run.call_count, 2)
            self.assertIn("--output", run.call_args_list[0].args)
            self.assertEqual(run.call_args_list[0].args[-1], output)
            self.assertEqual(run.call_args_list[1].args[-2:], ("--candidate", output))

    def test_locked_dependencies_require_wheel_hashes_and_actual_versions(self):
        pinned = build.requirements()
        self.assertGreater(len(pinned), 1)
        self.assertFalse(any(name.lower() in {"napari", "pyside6", "qtpy"} for name in pinned))
        with (
            patch.object(build.sys, "version_info", build.PYTHON),
            patch.object(
                build.importlib.metadata, "version", side_effect=lambda name: pinned[name]
            ),
        ):
            self.assertEqual(build.verify_environment(), pinned)
        with (
            patch.object(build.sys, "version_info", build.PYTHON),
            patch.object(build.importlib.metadata, "version", return_value="wrong"),
        ):
            with self.assertRaises(ValueError):
                build.verify_environment()
        with tempfile.TemporaryDirectory() as directory:
            lock = Path(directory) / "requirements.lock"
            lock.write_text("numpy==2.5.4\n")
            with patch.object(build, "LOCK", lock), self.assertRaises(ValueError):
                build.requirements()
