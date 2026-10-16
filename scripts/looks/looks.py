#!/usr/bin/env python3
"""Independent authoring CLI; generated artifacts stay in the checkout cache."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from catalog import ROOT, common, digest, identifiers, load, payload_hash, require
from derive import validate

CACHE = ROOT / ".cache/looks"
NATIVE = ROOT / "native/looks"


def cpp(value):
    if isinstance(value, list):
        return "{" + ",".join(cpp(v) for v in value) + "}"
    return format(float(value), ".17g")


def header(definition):
    authoring, spectral, _ = common()
    model = definition.model
    fields = {"reference_parameters": authoring["reference"]["parameters"]}
    if definition.monochrome:
        lo, hi = model["log_exposure_range"]
        fields.update(log_exposure=[lo + (hi - lo) * i / 255 for i in range(256)],
                      mono_parameters=model["density_parameters"])
    else:
        fields.update(wavelengths=spectral["wavelengths"], observer=spectral["observer"],
                      illumination=spectral["illumination"], base=[0.] * 61,
                      dye=model["dye_density"], log_exposure=model["log_exposure"],
                      density_curve=model["effective_coefficients"])
    lines = ["// Generated from versioned independent look definitions. Do not edit.", "#pragma once"]
    if definition.monochrome:
        lines.append("#define LOOK_NEUTRAL_DENSITY 1")
    lines.extend(["namespace look_data {", "constexpr const char *look_id = " + json.dumps(definition.id) + ";",
                  "constexpr const char *look_name = " + json.dumps(definition.look["name"]) + ";",
                  "constexpr double exposure_compression = " + cpp(authoring["exposure_compression"]) + ";"])
    for name, values in fields.items():
        dims = f"[{len(values)}]" + (f"[{len(values[0])}]" if isinstance(values[0], list) else "")
        lines.append(f"constexpr double {name}{dims} = {cpp(values)};")
    lines.append("} // namespace look_data")
    return "\n".join(lines) + "\n"


def build(definition):
    prefix = Path(os.environ.get("OCIO_PREFIX", ROOT / ".cache/deps/ocio-install")).resolve()
    require((prefix / "include/OpenColorIO/OpenColorIO.h").is_file() and (prefix / "lib").is_dir(),
            "OCIO missing: use the normal scripts/setup-native.sh workflow or set OCIO_PREFIX to an existing pinned installation")
    compiler = os.environ.get("CXX", "/usr/bin/clang++")
    version = subprocess.run([compiler, "--version"], check=True, capture_output=True, text=True).stdout
    generated = header(definition)
    flags = ["-std=c++17", "-O3", "-Wall", "-Wextra", "-Werror"]
    key = hashlib.sha256((compiler + generated + str(prefix) + version + repr(flags)).encode())
    for path in sorted(NATIVE.glob("*")):
        if path.suffix in (".h", ".cpp"):
            key.update(path.name.encode())
            key.update(path.read_bytes())
    # A reused native prefix may be rebuilt in place. Do not reuse a binary
    # compiled against stale dependency headers even when its path is the same.
    dependency_hash = hashlib.sha256()
    for path in sorted((prefix / "include/OpenColorIO").glob("*.h")):
        dependency_hash.update(path.name.encode())
        dependency_hash.update(path.read_bytes())
    for path in sorted((prefix / "lib").glob("*OpenColorIO*")):
        if path.is_file() and not path.is_symlink():
            dependency_hash.update(path.name.encode())
            dependency_hash.update(path.read_bytes())
    key.update(dependency_hash.digest())
    folder = CACHE / "build" / definition.id / key.hexdigest()
    executable = folder / "look-tool"
    if not executable.is_file():
        folder.mkdir(parents=True, exist_ok=True)
        # Concurrent cold-cache bakes compile in separate directories. Publish
        # only complete files; a failed compiler must not leave a reusable exe.
        with tempfile.TemporaryDirectory(dir=folder, prefix="compile-") as staging:
            staging = Path(staging)
            (staging / "look_data.h").write_text(generated)
            candidate = staging / "look-tool"
            subprocess.run([compiler, *flags, "-iquote", str(staging), "-I" + str(prefix / "include"),
                            str(NATIVE / "tool.cpp"), "-L" + str(prefix / "lib"),
                            "-Wl,-rpath," + str(prefix / "lib"), "-lOpenColorIO", "-o", str(candidate)], check=True)
            os.replace(staging / "look_data.h", folder / "look_data.h")
            os.replace(candidate, executable)
    return executable, {"compiler": version.splitlines()[0], "flags": flags, "build_key": key.hexdigest(),
                        "model_sha256": definition.look["model_sha256"], "source_sha256": definition.look["source_sha256"],
                        "native_dependency_sha256": dependency_hash.hexdigest()}


def execute(executable, *args):
    result = subprocess.run([str(executable), *args], capture_output=True, text=True)
    require(result.returncode == 0, "native authoring failed: " + (result.stderr.strip() or result.stdout.strip()))
    report = json.loads(result.stdout)
    require(isinstance(report, dict) and report.get("status") == "passed", "native tool did not report successful validation")
    return report


def native_check(definition):
    executable, _ = build(definition)
    return execute(executable, "check")


def bake(definition, size, output):
    require(size in (65, 97), "supported cube sizes are 65 and 97")
    output = output.resolve()
    require(output.is_relative_to((ROOT / ".cache").resolve()), "generated outputs must be under this checkout's ignored .cache/")
    require(not output.exists(), "choose a fresh output directory")
    source_report = validate(definition)
    executable, provenance = build(definition)
    output.parent.mkdir(parents=True, exist_ok=True)
    owned = False
    try:
        # Claim atomically after preflight/build. A competing invocation may
        # have created the destination in the meantime; never write into it.
        output.mkdir()
        owned = True
        execute(executable, "bake", str(size), str(output))
        actual = payload_hash(output / "look.cube")
        expected = definition.look["reviewed_bakes"][str(size)]["numeric_payload_sha256"]
        require(actual == expected, "numeric LUT payload changed from the reviewed independent bake; do not update its identity without investigation/review")
        report = {"status": "passed", "look": definition.id, "version": definition.look["version"],
                  "selection_status": definition.look["status"], "release_clearance": definition.look["release_clearance"],
                  "numeric_payload_sha256": actual, "file_sha256": digest(output / "look.cube"),
                  "source_checks": source_report, "build": provenance,
                  "limits": "Content parity and numerical admission, not physical calibration, final approximation or release approval."}
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        if owned:
            (output / "validation.json").write_text(json.dumps({"status": "failed", "error": str(exc)}, indent=2) + "\n")
        raise
    (output / "validation.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    commands.add_parser("list", help="show all definitions, including shelved status")
    check = commands.add_parser("check", help="validate definitions/source equations; optionally check native synthetic invariants")
    selection = check.add_mutually_exclusive_group(required=True)
    selection.add_argument("--all", action="store_true")
    selection.add_argument("--look")
    check.add_argument("--native", action="store_true")
    bake_parser = commands.add_parser("bake", help="generate and validate a reviewed numeric cube payload")
    bake_parser.add_argument("look")
    bake_parser.add_argument("--size", type=int, choices=(65, 97), default=97)
    bake_parser.add_argument("--output", type=Path, help="fresh directory under .cache/; default .cache/looks/bakes/ID/SIZE")
    return result


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        if args.command == "list":
            common()
            for identifier in identifiers():
                item = load(identifier)
                print(f"{item.id:42} {item.look['status']:10} {item.model['family']}")
        elif args.command == "check":
            report = {}
            for identifier in identifiers() if args.all else [args.look]:
                item = load(identifier)
                report[identifier] = {"source": validate(item)}
                if args.native:
                    report[identifier]["native"] = native_check(item)
            print(json.dumps({"status": "passed", "looks": report}, indent=2))
        else:
            item = load(args.look)
            output = args.output or CACHE / "bakes" / item.id / str(args.size)
            print(json.dumps(bake(item, args.size, output), indent=2))
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as exc:
        print(f"look authoring failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
