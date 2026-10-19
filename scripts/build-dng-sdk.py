#!/usr/bin/env python3
"""Build/reuse the explicitly supplied DNG SDK dependency; never download it."""

import argparse
import hashlib
from pathlib import Path
import shlex
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PREFIX = ROOT / ".cache/dng-sdk"
SHA256 = "c1d3a545ed45b150e59af2e6aa0695c1551f0e2bed34ad87436898a05194d2b1"


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    supplied = parser.add_mutually_exclusive_group(required=True)
    supplied.add_argument(
        "--archive", type=Path, help="SDK 1.7.1 build 2768 ZIP; use accepts its license"
    )
    supplied.add_argument(
        "--source", type=Path, help="reuse that SDK's existing source/build directory"
    )
    parser.add_argument(
        "--reuse-build",
        action="store_true",
        help="package existing validator objects without invoking Xcode",
    )
    args = parser.parse_args()
    PREFIX.mkdir(parents=True, exist_ok=True)
    if args.archive:
        require(
            hashlib.sha256(args.archive.read_bytes()).hexdigest() == SHA256,
            "SDK archive checksum mismatch",
        )
        destination = PREFIX / "source"
        if not destination.exists():
            with tempfile.TemporaryDirectory(dir=PREFIX) as scratch:
                staged = Path(scratch) / "source"
                with zipfile.ZipFile(args.archive) as archive:
                    for entry in archive.infolist():
                        path = Path(entry.filename)
                        require(
                            not path.is_absolute() and ".." not in path.parts, "Unsafe archive path"
                        )
                    archive.extractall(staged)
                    for entry in archive.infolist():
                        mode = (entry.external_attr >> 16) & 0o777
                        if mode:
                            (staged / entry.filename).chmod(mode)
                staged.rename(destination)
        sdk = destination / "dng_sdk_1_7_1"
    else:
        sdk = args.source.resolve()
    flags_header = (sdk / "dng_sdk/source/dng_flags.h").read_text()
    require('#define kDNGSDK_VersionString "1.7.1"' in flags_header, "Unsupported SDK version")
    require("#define kDNGSDK_BuildVersion 2768" in flags_header, "Unsupported SDK build")
    project = sdk / "dng_sdk/projects/mac"
    if not args.reuse_build:
        subprocess.run(
            [
                "xcodebuild",
                "-project",
                str(project / "dng_validate.xcodeproj"),
                "-scheme",
                "dng_validate release",
                "-derivedDataPath",
                str(PREFIX / "xcode-build"),
                "-disableAutomaticPackageResolution",
                "-jobs",
                "4",
                "ARCHS=arm64",
                "ONLY_ACTIVE_ARCH=YES",
                "MACOSX_DEPLOYMENT_TARGET=14.0",
                "CODE_SIGNING_ALLOWED=NO",
                "CC=/usr/bin/clang",
                "CPLUSPLUS=/usr/bin/clang++",
                "LD=/usr/bin/clang++",
                "LDPLUSPLUS=/usr/bin/clang++",
                "build",
            ],
            check=True,
            cwd=ROOT,
        )
    objects = (
        project / "build/dng_validate.build/Default/dng_validate release.build/Objects-normal/arm64"
    )
    files = [
        p
        for p in (objects / "dng_validate.LinkFileList").read_text().splitlines()
        if not p.endswith("/dng_validate.o")
    ]
    require(files, "SDK build produced no objects")
    libraries = [
        sdk
        / "xmp/toolkit/public/libraries/macintosh/intel_64_libcpp/Release/libXMPFilesStatic_Release.a",
        sdk
        / "xmp/toolkit/public/libraries/macintosh/intel_64_libcpp/Release/libXMPCoreStatic_Release.a",
        sdk / "libjxl/client_projects/mac/build/Default/libjxl_release.a",
    ]
    output = PREFIX / "lib/libantitone_dng_sdk.a"
    output.parent.mkdir(exist_ok=True)
    subprocess.run(
        ["/usr/bin/libtool", "-static", "-o", str(output), *files, *map(str, libraries)], check=True
    )
    responses = list(objects.glob("*common-args.resp"))
    define_sets = {
        tuple(sorted(flag[2:] for flag in shlex.split(path.read_text()) if flag.startswith("-D")))
        for path in responses
    }
    require(len(define_sets) == 1, "Missing or inconsistent SDK compile definitions")
    defines = next(iter(define_sets))
    (PREFIX / "source-path.txt").write_text(str(sdk) + "\n")
    (PREFIX / "defines.txt").write_text("\n".join(defines) + "\n")
    print(f"DNG SDK dependency ready: {PREFIX}")


if __name__ == "__main__":
    main()
