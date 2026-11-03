#!/usr/bin/env python3
"""Generate the fixed authoring DRT's GPU conformance resources using pinned OCIO."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def generate(output):
    output = output.resolve()
    if output.exists() or not output.is_relative_to((ROOT / ".cache").resolve()):
        raise ValueError("choose a fresh output under checkout .cache/")
    config = json.loads((ROOT / "assets/ocio/authoring-reference.json").read_text())
    prefix = Path(os.environ.get("OCIO_PREFIX", ROOT / ".cache/deps/ocio-install")).resolve()
    scratch = ROOT / ".cache/spektrafilm"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch) as temporary:
        helper = Path(temporary) / "reference-generator"
        subprocess.run(
            [
                os.environ.get("CXX", "/usr/bin/clang++"),
                "-std=c++17",
                "-O3",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-I" + str(prefix / "include"),
                str(ROOT / "native/authoring/generate_reference.cpp"),
                "-L" + str(prefix / "lib"),
                "-Wl,-rpath," + str(prefix / "lib"),
                "-lOpenColorIO",
                "-o",
                str(helper),
            ],
            check=True,
        )
        output.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [
                str(helper),
                str(output),
                *[format(v, ".17g") for v in config["reference"]["parameters"]],
            ],
            check=True,
        )
    metadata = {
        "ocio": config["ocio_version"],
        "reference": config["reference"],
        "space": "forward AP1/D60 to fixed reference; inverse fixed reference to AP1/D60",
        "shader_language": "GLSL_VK_4_6",
        "table_channels": {"reach.f32": 1, "cusp.f32": 3},
        "sha256": {
            p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(output.iterdir())
        },
    }
    (output / "provenance.json").write_text(json.dumps(metadata, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    generate(parser.parse_args().output)


if __name__ == "__main__":
    main()
