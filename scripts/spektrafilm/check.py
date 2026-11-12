#!/usr/bin/env python3
"""Validate generated spektrafilm data/provenance without an upstream environment."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[2]


def validate(folder, files=None):
    provenance = json.loads((folder / "provenance.json").read_text())
    for field, path in [
        ("generator_sha256", ROOT / "scripts/spektrafilm/export.py"),
        ("positive_driver_sha256", ROOT / "scripts/spektrafilm/positive.py"),
        ("reference_driver_sha256", ROOT / "scripts/spektrafilm/reference.py"),
        ("reference_helper_sha256", ROOT / "native/spektrafilm/reference.cpp"),
        ("environment_lock_sha256", ROOT / "scripts/spektrafilm/requirements.lock"),
        ("build_tool_sha256", ROOT / "scripts/spektrafilm/build.py"),
        ("authoring_reference_sha256", ROOT / "assets/ocio/authoring-reference.json"),
    ]:
        if hashlib.sha256(path.read_bytes()).hexdigest() != provenance[field]:
            raise ValueError("producer identity changed: " + field)
    if files is None:
        files = {
            name: (folder / name).read_bytes()
            for name in provenance["files"]
            if name != "conformance.json"
        }
    for name, record in provenance["files"].items():
        data = (
            (
                (ROOT / "tests/fixtures/spektrafilm/conformance.json")
                if folder.resolve() == (ROOT / "assets/spektrafilm").resolve()
                else folder / name
            ).read_bytes()
            if name == "conformance.json"
            else files[name]
        )
        if len(data) != record["bytes"] or hashlib.sha256(data).hexdigest() != record["sha256"]:
            raise ValueError("component identity changed: " + name)
    catalog = json.loads(zlib.decompress(files["catalog.json.z"]))
    labels = json.loads(files["selection.json"])
    if catalog["revision"] != provenance["revision"] or labels["revision"] != catalog["revision"]:
        raise ValueError("source pin mismatch")
    for kind in ["negative", "paper"]:
        if labels[kind] != [{"id": v["id"], "name": v["name"]} for v in catalog[kind]]:
            raise ValueError("label/data mismatch")
    negative = {v["id"] for v in catalog["negative"]}
    paper = {v["id"] for v in catalog["paper"]}
    if {(v["negative"], v["paper"]) for v in catalog["pairs"]} != {
        (f, p) for f in negative for p in paper
    }:
        raise ValueError("incomplete pairs")
    if len(catalog["pairs"]) != len(negative) * len(paper):
        raise ValueError("duplicate pairs")
    n = catalog["spectra_size"]
    w = catalog["wavelengths"]
    size = catalog["scanner_size"]
    spectra = zlib.decompress(files["irradiance_xy_tc.f16.z"])
    if len(spectra) != n * n * w * 2:
        raise ValueError("spectral family shape")
    if not all(float("-inf") < v[0] < float("inf") for v in struct.iter_unpack("<e", spectra)):
        raise ValueError("nonfinite spectral family")
    if (
        set(provenance["positive_drivers"]) != paper
        or set(provenance["paper_output_validation"]) != paper
    ):
        raise ValueError("incomplete reference entry/finishing coverage")
    for medium in catalog["paper"]:
        if medium["positive_driver"] != {
            key: provenance["positive_drivers"][medium["id"]][key]
            for key in ["input_luminance", "reference"]
        }:
            raise ValueError("reference driver/provenance mismatch")
        values = zlib.decompress(files[medium["scanner_file"]])
        if len(values) != size**3 * 12:
            raise ValueError("scanner component shape")
        if not all(float("-inf") < v[0] < float("inf") for v in struct.iter_unpack("<f", values)):
            raise ValueError("nonfinite scanner")
    for filename in catalog["remap"].values():
        if len(zlib.decompress(files[filename])) != n * n * 16:
            raise ValueError("input-gamut shape")
    return {
        "negative_profiles": len(negative),
        "print_media": len(paper),
        "pairs": len(catalog["pairs"]),
        "runtime_bytes": sum(len(v) for name, v in files.items() if name.endswith(".z")),
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path)
    args = parser.parse_args()
    print(json.dumps(validate(args.candidate or ROOT / "assets/spektrafilm"), indent=2))
