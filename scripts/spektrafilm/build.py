#!/usr/bin/env python3
"""Explicit acquisition and regeneration of the checked-in spektrafilm components."""

import argparse
import importlib.metadata
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
CACHE = ROOT / ".cache/spektrafilm"
UPSTREAM = CACHE / "upstream"
ENVIRONMENT = CACHE / "venv"
SOURCE = "https://github.com/andreavolpato/spektrafilm.git"
REVISION = "3bb2c2d2801ff68b92019cf1dbcbb133d60832bc"
PYTHON = (3, 14, 6)
LOCK = Path(__file__).with_name("requirements.lock")


def run(*args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def captured(*args):
    return subprocess.check_output([str(arg) for arg in args], text=True).strip()


def requirements():
    result = {}
    for line in LOCK.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        match = re.fullmatch(r"([A-Za-z0-9_.-]+)==([^\s]+) --hash=sha256:[0-9a-f]{64}", line)
        if not match:
            raise ValueError("invalid generation dependency lock entry: " + line)
        name, version = match.groups()
        if name in result:
            raise ValueError("duplicate generation dependency: " + name)
        result[name] = version
    return result


def verify_environment():
    if sys.version_info[:3] != PYTHON:
        raise ValueError(f"generation requires Python {'.'.join(map(str, PYTHON))}")
    actual = {name: importlib.metadata.version(name) for name in requirements()}
    if actual != requirements():
        raise ValueError("generation dependencies differ from requirements.lock")
    return actual


def verify_source(source):
    source = source.resolve()
    if captured("git", "-C", source, "rev-parse", "HEAD") != REVISION:
        raise ValueError("spektrafilm source revision differs from the reviewed pin")
    if captured("git", "-C", source, "status", "--porcelain"):
        raise ValueError("spektrafilm source checkout is modified")
    return source


def acquire_source():
    if UPSTREAM.exists():
        return verify_source(UPSTREAM)
    CACHE.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="source-", dir=CACHE) as temporary:
        staging = Path(temporary) / "upstream"
        run("git", "init", "--quiet", staging)
        run("git", "-C", staging, "config", "core.autocrlf", "false")
        run("git", "-C", staging, "remote", "add", "origin", SOURCE)
        run("git", "-C", staging, "fetch", "--quiet", "--depth=1", "origin", REVISION)
        run("git", "-C", staging, "checkout", "--quiet", "--detach", "FETCH_HEAD")
        verify_source(staging)
        staging.rename(UPSTREAM)
    return UPSTREAM


def environment_python():
    return ENVIRONMENT / ("Scripts/python.exe" if os.name == "nt" else "bin/python")


def setup(python):
    version = json.loads(
        captured(python, "-c", "import sys,json;print(json.dumps(sys.version_info[:3]))")
    )
    if tuple(version) != PYTHON:
        raise ValueError(
            f"supply Python {'.'.join(map(str, PYTHON))}; setup does not install an interpreter"
        )
    acquire_source()
    CACHE.mkdir(parents=True, exist_ok=True)
    if not environment_python().is_file():
        run(python, "-m", "venv", ENVIRONMENT)
    env = dict(os.environ, PIP_CACHE_DIR=str(CACHE / "pip-cache"), PIP_REQUIRE_VIRTUALENV="true")
    run(
        environment_python(),
        "-m",
        "pip",
        "install",
        "--only-binary=:all:",
        "--no-deps",
        "--require-hashes",
        "-r",
        LOCK,
        env=env,
    )
    run(environment_python(), "-m", "pip", "check", env=env)
    run(
        environment_python(),
        "-c",
        f"import sys;sys.path.insert(0,{str(LOCK.parent)!r});from build import verify_environment;verify_environment()",
    )


def generate(python, upstream, output, apd_responsivities):
    if sys.platform != "darwin":
        raise ValueError(
            "asset regeneration is currently supported on macOS; application builds use the checked-in components"
        )
    source = verify_source(upstream)
    env = dict(os.environ, PYTHONNOUSERSITE="1", PYTHONDONTWRITEBYTECODE="1")
    run(
        python,
        LOCK.with_name("export.py"),
        "--upstream",
        source,
        "--output",
        output,
        "--apd-responsivities",
        apd_responsivities,
        env=env,
    )
    run(sys.executable, LOCK.with_name("check.py"), "--candidate", output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare = commands.add_parser(
        "setup", help="fetch pinned source and install locked headless dependencies into .cache/"
    )
    prepare.add_argument(
        "--python", default=sys.executable, help="existing Python 3.14.6 interpreter"
    )
    export = commands.add_parser(
        "generate", help="write a fresh candidate; never replaces checked-in assets"
    )
    export.add_argument("--python", type=Path, default=environment_python())
    export.add_argument(
        "--upstream",
        type=Path,
        default=UPSTREAM,
        help="existing clean pinned checkout, or setup's cached source",
    )
    export.add_argument("--output", type=Path, required=True)
    export.add_argument("--apd-responsivities", type=Path, required=True)
    reference = commands.add_parser(
        "reference", help="regenerate pinned authoring-DRT GPU conformance resources"
    )
    reference.add_argument("--output", type=Path, required=True)
    commands.add_parser(
        "check",
        help="validate checked-in components without upstream/Python scientific dependencies",
    )
    args = parser.parse_args()
    if args.command == "setup":
        setup(args.python)
    elif args.command == "generate":
        generate(args.python, args.upstream, args.output, args.apd_responsivities)
    elif args.command == "reference":
        run(sys.executable, LOCK.with_name("generate_reference.py"), "--output", args.output)
    else:
        run(sys.executable, LOCK.with_name("check.py"))


if __name__ == "__main__":
    try:
        main()
    except (
        OSError,
        ValueError,
        subprocess.CalledProcessError,
        importlib.metadata.PackageNotFoundError,
    ) as error:
        raise SystemExit(str(error)) from error
