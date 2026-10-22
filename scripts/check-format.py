#!/usr/bin/env python3
"""Check authored source formatting; --write applies the configured formatters."""

import argparse
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / ".cache/format-tools"
CLANG_VERSION = "clang-format version 22.1.8"


def tool(name, local=None):
    override = os.environ.get(name.upper().replace("-", "_"))
    candidate = override or (str(local) if local and local.is_file() else name)
    executable = shutil.which(candidate)
    if executable is None:
        raise RuntimeError(f"Missing {name}; see docs/development.md#source-formatting.")
    return str(Path(executable).resolve())


def sources():
    result = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    paths = {os.fsdecode(path) for path in result.stdout.split(b"\0") if path}
    return sorted(
        path for path in paths if not path.startswith("vendor/") and (ROOT / path).is_file()
    )


def run(*command):
    print("+ " + shlex.join(command), flush=True)
    subprocess.run(command, cwd=ROOT, check=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--check", action="store_true", help="check without modifying source (default)"
    )
    mode.add_argument(
        "--write", action="store_true", help="apply formatting, then run syntax/lint checks"
    )
    args = parser.parse_args(argv)

    paths = sources()
    native = [path for path in paths if Path(path).suffix in {".cpp", ".h", ".m", ".mm"}]
    python = [path for path in paths if path.endswith(".py")]
    shaders = [path for path in paths if path.endswith(".wgsl")]
    swift = [path for path in paths if path.endswith(".swift")]
    shell = [path for path in paths if path.endswith(".sh")]

    clang = tool("clang-format", TOOLS / "venv/bin/clang-format")
    ruff = tool("ruff", TOOLS / "venv/bin/ruff")
    wgslfmt = tool("wgslfmt", TOOLS / "bin/wgslfmt")
    version = subprocess.run([clang, "--version"], check=True, capture_output=True, text=True)
    if version.stdout.strip() != CLANG_VERSION:
        raise RuntimeError(f"Expected {CLANG_VERSION}; found {version.stdout.strip()}.")

    run("cargo", "fmt", "--all", *([] if args.write else ["--", "--check"]))
    if native:
        run(
            clang,
            "--style=file",
            "--fail-on-incomplete-format",
            *(["-i"] if args.write else ["--dry-run", "--Werror"]),
            *native,
        )
    if python:
        run(ruff, "format", *([] if args.write else ["--check"]), *python)
        run(ruff, "check", *python)
    if shaders:
        run(
            wgslfmt,
            "--config",
            "max_line_width=100",
            *([] if args.write else ["--check"]),
            *shaders,
        )
    if swift:
        run(
            "xcrun",
            "swift-format",
            *(["format", "--in-place"] if args.write else ["lint", "--strict"]),
            "--configuration",
            str(ROOT / ".swift-format"),
            *swift,
        )
    for path in shell:
        run("sh", "-n", path)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)
    except subprocess.CalledProcessError as error:
        raise SystemExit(error.returncode)
