#!/usr/bin/env python3
"""Validate a release tag and assemble verified platform ZIPs with one checksum manifest."""

import argparse
import hashlib
from pathlib import Path
import shutil
import tomllib
import zipfile


PACKAGES = ("Antitone-macos-arm64.zip", "Antitone-windows-x64.zip")


def validate_tag(tag: str, manifest: Path) -> None:
    version = tomllib.loads(manifest.read_text(encoding="utf-8"))["package"]["version"]
    if tag != f"v{version}":
        raise ValueError(f"Tag {tag!r} does not match Cargo.toml version {version}")


def verify_package(directory: Path, name: str) -> str:
    """Require the named platform's manifest, matching bytes and an intact ZIP container."""
    fields = (directory / "SHA256SUMS").read_text(encoding="ascii").split()
    if len(fields) != 2 or fields[1] != name:
        raise ValueError(f"Expected exactly one checksum for {name} in {directory}")
    path = directory / name
    with path.open("rb") as stream:
        actual = hashlib.file_digest(stream, "sha256").hexdigest()
    if fields[0] != actual:
        raise ValueError(f"SHA-256 mismatch for {name}")
    with zipfile.ZipFile(path) as archive:
        if not archive.namelist():
            raise ValueError(f"Empty package: {name}")
        damaged = archive.testzip()
        if damaged:
            raise ValueError(f"Damaged ZIP member in {name}: {damaged}")
    return actual


def assemble(macos: Path, windows: Path, output: Path) -> None:
    # Validate both inputs before creating anything publishable. A fresh output directory prevents
    # stale assets from an earlier run from being picked up by a release upload.
    inputs = list(zip((macos, windows), PACKAGES, strict=True))
    hashes = [verify_package(directory, name) for directory, name in inputs]
    output.mkdir(parents=True, exist_ok=False)
    try:
        for directory, name in inputs:
            shutil.copyfile(directory / name, output / name)
        checksums = "".join(
            f"{digest}  {name}\n" for digest, name in zip(hashes, PACKAGES, strict=True)
        )
        (output / "SHA256SUMS").write_text(checksums, encoding="ascii")
    except Exception:
        shutil.rmtree(output)
        raise


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--manifest", type=Path, default=Path("Cargo.toml"))
    parser.add_argument("--check-tag", action="store_true")
    parser.add_argument("--macos", type=Path)
    parser.add_argument("--windows", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    validate_tag(args.tag, args.manifest)
    if args.check_tag:
        return
    if any(value is None for value in (args.macos, args.windows, args.output)):
        parser.error("assembly requires --macos, --windows and --output")
    assemble(args.macos, args.windows, args.output)


if __name__ == "__main__":
    main()
