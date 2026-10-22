import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "check_format", Path(__file__).with_name("check-format.py")
)
check_format = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_format)


class FormatChecks(unittest.TestCase):
    def commands(self, *arguments):
        paths = [
            "assets/ocio/generated.glsl",
            "native/bridge.cpp",
            "native/artwork.swift",
            "scripts/tool.py",
            "scripts/setup.sh",
            "src/shaders/processing.wgsl",
        ]
        commands = []
        with (
            mock.patch.object(check_format, "sources", return_value=paths),
            mock.patch.object(check_format, "tool", side_effect=lambda name, local=None: name),
            mock.patch.object(
                check_format, "run", side_effect=lambda *command: commands.append(command)
            ),
            mock.patch.object(
                check_format.subprocess,
                "run",
                return_value=SimpleNamespace(stdout=check_format.CLANG_VERSION),
            ),
        ):
            self.assertEqual(check_format.main(list(arguments)), 0)
        return commands

    def test_default_checks_every_supported_language_without_rewriting(self):
        commands = self.commands()
        self.assertEqual(commands[0], ("cargo", "fmt", "--all", "--", "--check"))
        native = next(command for command in commands if command[0] == "clang-format")
        self.assertIn("--dry-run", native)
        self.assertIn("--Werror", native)
        self.assertIn(("ruff", "format", "--check", "scripts/tool.py"), commands)
        self.assertIn(("ruff", "check", "scripts/tool.py"), commands)
        shader = next(command for command in commands if command[0] == "wgslfmt")
        self.assertIn("--check", shader)
        swift = next(command for command in commands if command[0] == "xcrun")
        self.assertIn("--strict", swift)
        self.assertIn(("sh", "-n", "scripts/setup.sh"), commands)
        self.assertFalse(any("assets/ocio/generated.glsl" in command for command in commands))

    def test_write_mode_formats_but_keeps_python_and_shell_checks(self):
        commands = self.commands("--write")
        self.assertEqual(commands[0], ("cargo", "fmt", "--all"))
        self.assertIn("-i", next(command for command in commands if command[0] == "clang-format"))
        self.assertIn(("ruff", "format", "scripts/tool.py"), commands)
        self.assertIn(("ruff", "check", "scripts/tool.py"), commands)
        self.assertNotIn(
            "--check", next(command for command in commands if command[0] == "wgslfmt")
        )
        self.assertIn("--in-place", next(command for command in commands if command[0] == "xcrun"))
        self.assertIn(("sh", "-n", "scripts/setup.sh"), commands)

    def test_source_inventory_deduplicates_tracked_and_new_paths(self):
        result = SimpleNamespace(stdout=b"src/lib.rs\0scripts/new.py\0src/lib.rs\0")
        with (
            mock.patch.object(check_format.subprocess, "run", return_value=result),
            mock.patch.object(Path, "is_file", return_value=True),
        ):
            self.assertEqual(check_format.sources(), ["scripts/new.py", "src/lib.rs"])

    def test_source_inventory_ignores_unstaged_deletions_but_keeps_new_files(self):
        result = SimpleNamespace(stdout=b"scripts/deleted.py\0scripts/new.py\0")
        with (
            mock.patch.object(check_format.subprocess, "run", return_value=result),
            mock.patch.object(
                Path, "is_file", autospec=True, side_effect=lambda path: path.name != "deleted.py"
            ),
        ):
            self.assertEqual(check_format.sources(), ["scripts/new.py"])

    def test_vendored_sources_are_not_reformatted(self):
        result = SimpleNamespace(stdout=b"native/bridge.cpp\0vendor/dng-sdk/source.cpp\0")
        with (
            mock.patch.object(check_format.subprocess, "run", return_value=result),
            mock.patch.object(Path, "is_file", return_value=True),
        ):
            self.assertEqual(check_format.sources(), ["native/bridge.cpp"])

    def test_missing_tool_fails_instead_of_skipping_its_language(self):
        with mock.patch.dict(check_format.os.environ, {}, clear=True):
            with mock.patch.object(check_format.shutil, "which", return_value=None):
                with self.assertRaisesRegex(RuntimeError, "Missing ruff"):
                    check_format.tool("ruff")
