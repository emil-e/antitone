#!/usr/bin/env python3
"""Regenerate pinned authoring-reference GPU algorithms into an ignored fresh directory."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from catalog import ROOT, common, digest, require


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        require(
            output.is_relative_to((ROOT / ".cache").resolve()) and not output.exists(),
            "choose a fresh output under checkout .cache/",
        )
        require(sys.byteorder == "little", "reference generation requires a little-endian host")
        authoring, _, _ = common()
        prefix = Path(os.environ.get("OCIO_PREFIX", ROOT / ".cache/deps/ocio-install")).resolve()
        require(
            (prefix / "include/OpenColorIO/OpenColorIO.h").is_file(),
            "use normal native setup or OCIO_PREFIX for the pinned installation",
        )
        build = ROOT / ".cache/looks/reference-generator"
        build.mkdir(parents=True, exist_ok=True)
        compiler = os.environ.get("CXX", "/usr/bin/clang++")
        with tempfile.TemporaryDirectory(dir=build) as staging:
            executable = Path(staging) / "generate-reference"
            subprocess.run(
                [
                    compiler,
                    "-std=c++17",
                    "-O3",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "-I" + str(prefix / "include"),
                    str(ROOT / "native/looks/generate_reference.cpp"),
                    "-L" + str(prefix / "lib"),
                    "-Wl,-rpath," + str(prefix / "lib"),
                    "-lOpenColorIO",
                    "-o",
                    str(executable),
                ],
                check=True,
            )
            output.parent.mkdir(parents=True, exist_ok=True)
            parameters = authoring["reference"]["parameters"]
            subprocess.run(
                [str(executable), str(output), *[format(v, ".17g") for v in parameters]], check=True
            )
        metadata = {
            "ocio": "2.6.0",
            "reference": authoring["reference"],
            "space": "forward AP1/D60 to fixed reference; inverse fixed reference to AP1/D60",
            "shader_language": "GLSL_VK_4_6",
            "table_channels": {"reach.f32": 1, "cusp.f32": 3},
            "sha256": {p.name: digest(p) for p in sorted(output.iterdir())},
        }
        (output / "provenance.json").write_text(json.dumps(metadata, indent=2) + "\n")
        print(output)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"reference generation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
