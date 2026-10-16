#!/usr/bin/env python3
"""Extract compact runtime conformance samples from verified native 65-cube bakes."""

import argparse
import json
import os
import random
import struct
import subprocess
from pathlib import Path

from catalog import ROOT, load, require
from looks import bake, build

IDS = (
    "kodak-portra-endura",
    "kodak-pro-image-ii",
    "kodak-2383",
    "ilford-multigrade-rc-deluxe-filter-2",
)


def generate(output):
    output = output.resolve()
    require(
        output.is_relative_to((ROOT / ".cache").resolve()),
        "generated outputs must be under this checkout's ignored .cache/",
    )
    output.mkdir(parents=True)  # Atomic ownership; never overwrite a previous result.
    indices = {i + 65 * i + 65 * 65 * i for i in range(65)}
    indices.update(r + 65 * g + 65 * 65 * b for r in (0, 64) for g in (0, 64) for b in (0, 64))
    for i in (1, 8, 16, 32, 38, 48, 63):
        indices.update(
            (i + 65 * 38 + 65 * 65 * 38, 38 + 65 * i + 65 * 65 * 38, 38 + 65 * 38 + 65 * 65 * i)
        )
    randomizer = random.Random(0x65ACE5)
    while len(indices) < 256:
        indices.add(randomizer.randrange(65**3))
    results = {}
    for identifier in IDS:
        definition = load(identifier)
        folder = output / identifier
        report = bake(
            definition, 65, folder
        )  # Includes all native gates and reviewed payload identity.
        rows = []
        for line in (folder / "look.cube").read_text().splitlines():
            line = line.strip()
            if not line or line.startswith(("#", "TITLE", "LUT_3D_SIZE", "DOMAIN_")):
                continue
            row = [float(value) for value in line.split()]
            require(len(row) == 3, "invalid native sample")
            rows.append(row)
        require(len(rows) == 65**3, "incomplete native sample grid")
        executable, provenance = build(definition)
        prefix = Path(os.environ.get("OCIO_PREFIX", ROOT / ".cache/deps/ocio-install")).resolve()
        conformance = folder / "conformance"
        subprocess.run(
            [
                os.environ.get("CXX", "/usr/bin/clang++"),
                *provenance["flags"],
                "-iquote",
                str(executable.parent),
                "-I" + str(prefix / "include"),
                str(ROOT / "native/looks/conformance.cpp"),
                "-L" + str(prefix / "lib"),
                "-Wl,-rpath," + str(prefix / "lib"),
                "-lOpenColorIO",
                "-o",
                str(conformance),
            ],
            check=True,
        )
        mapped = subprocess.run([str(conformance)], capture_output=True, check=True).stdout
        require(len(mapped) == 65**3 * 24, "incomplete native material/reference samples")
        results[identifier] = {
            "model_sha256": definition.look["model_sha256"],
            "numeric_payload_sha256": report["numeric_payload_sha256"],
            "samples": [
                [index, *rows[index], *struct.unpack_from("=ddd", mapped, index * 24)]
                for index in sorted(indices)
            ],
        }
    result = {
        "schema": 1,
        "size": 65,
        "columns": [
            "index",
            "r_cct",
            "g_cct",
            "b_cct",
            "r_reference",
            "g_reference",
            "b_reference",
        ],
        "reference": "Frozen native C++ material + OCIO 2.6.0 CPU authoring; not runtime-generated targets",
        "looks": results,
    }
    (output / "native-65.json").write_text(
        json.dumps(result, separators=(",", ":"), allow_nan=False) + "\n"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="fresh directory under .cache/")
    generate(parser.parse_args().output)
