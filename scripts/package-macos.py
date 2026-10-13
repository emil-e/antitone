#!/usr/bin/env python3
"""Package the built editor as a checkout-local macOS application."""

import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys


def main():
    if sys.platform != "darwin":
        raise SystemExit("This packaging command requires macOS.")
    root = Path(__file__).resolve().parents[1]
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version", "1"],
        cwd=root, text=True,
    ))
    package = next(package for package in metadata["packages"]
                   if Path(package["manifest_path"]) == root / "Cargo.toml")
    binary = Path(metadata["target_directory"]) / "release" / "antitone"
    if not binary.is_file():
        raise SystemExit("Build the editor first: cargo build-app")
    bundle = root / ".cache" / "app" / "Antitone.app"
    if bundle.exists():
        shutil.rmtree(bundle)
    contents = bundle / "Contents"
    executable = contents / "MacOS"
    resources = contents / "Resources"
    executable.mkdir(parents=True)
    resources.mkdir()
    shutil.copy2(binary, executable / "antitone")
    shutil.copy2(root / "assets" / "branding" / "icon.icns", resources / "Antitone.icns")
    info = {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": "Antitone",
        "CFBundleExecutable": "antitone",
        "CFBundleIconFile": "Antitone.icns",
        "CFBundleIdentifier": "local.antitone.editor",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": "Antitone",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": package["version"],
        "CFBundleVersion": package["version"],
        "LSMinimumSystemVersion": "14.0",
        "NSHighResolutionCapable": True,
    }
    with (contents / "Info.plist").open("wb") as output:
        plistlib.dump(info, output)
    print(bundle)


if __name__ == "__main__":
    main()
