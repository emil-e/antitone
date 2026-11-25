#!/usr/bin/env python3
"""Validate a release tag and assemble verified platform ZIPs with one checksum manifest."""

import argparse
import hashlib
from pathlib import Path
import shutil
import tomllib
import zipfile


PACKAGES = ("Antitone-macos-arm64.zip", "Antitone-windows-x64.zip")
LIGHTROOM = "Antitone-Lightroom.zip"


def validate_tag(tag: str, manifest: Path) -> None:
    version = tomllib.loads(manifest.read_text(encoding="utf-8"))["package"]["version"]
    if tag != f"v{version}":
        raise ValueError(f"Tag {tag!r} does not match Cargo.toml version {version}")


def read_checksums(directory: Path, expected: tuple[str, ...]) -> dict[str, str]:
    rows = [
        line.split() for line in (directory / "SHA256SUMS").read_text(encoding="ascii").splitlines()
    ]
    if any(len(row) != 2 for row in rows):
        raise ValueError(f"Invalid checksum manifest in {directory}")
    checksums = {name: digest for digest, name in rows}
    if len(rows) != len(expected) or set(checksums) != set(expected):
        raise ValueError(f"Expected exactly one checksum for each of {expected} in {directory}")
    return checksums


def verify_package(directory: Path, name: str, digest: str) -> str:
    """Require matching bytes and an intact ZIP container."""
    path = directory / name
    with path.open("rb") as stream:
        actual = hashlib.file_digest(stream, "sha256").hexdigest()
    if digest != actual:
        raise ValueError(f"SHA-256 mismatch for {name}")
    with zipfile.ZipFile(path) as archive:
        if not archive.namelist():
            raise ValueError(f"Empty package: {name}")
        damaged = archive.testzip()
        if damaged:
            raise ValueError(f"Damaged ZIP member in {name}: {damaged}")
    return actual


def assemble(macos: Path, windows: Path, output: Path, *, tag: str) -> None:
    # Validate both inputs before creating anything publishable. A fresh output directory prevents
    # stale assets from an earlier run from being picked up by a release upload.
    mac_checksums = read_checksums(macos, (PACKAGES[0], LIGHTROOM))
    win_checksums = read_checksums(windows, (PACKAGES[1],))
    inputs = [(macos, PACKAGES[0]), (macos, LIGHTROOM), (windows, PACKAGES[1])]
    hashes = [
        verify_package(
            directory, name, (mac_checksums if directory == macos else win_checksums)[name]
        )
        for directory, name in inputs
    ]
    output.mkdir(parents=True, exist_ok=False)
    try:
        checksums = []
        for (directory, name), digest in zip(inputs, hashes, strict=True):
            asset_name = f"{Path(name).stem}-{tag}.zip"
            shutil.copyfile(directory / name, output / asset_name)
            checksums.append(f"{digest}  {asset_name}\n")
        (output / "SHA256SUMS").write_text("".join(checksums), encoding="ascii")
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
    assemble(args.macos, args.windows, args.output, tag=args.tag)


if __name__ == "__main__":
    main()
