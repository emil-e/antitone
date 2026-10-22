#!/usr/bin/env python3
"""Create and verify a relocatable local macOS app and ZIP (not a public release)."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import posixpath
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / ".cache" / "app"
APP = OUTPUT / "Antitone.app"
ARCHIVE = OUTPUT / "Antitone-macos-arm64.zip"
OPENMP = ROOT / ".cache/native-deps/openmp-22.1.8-install/lib/libomp.dylib"


def run(*command, **kwargs):
    print("+ " + " ".join(map(str, command)), flush=True)
    return subprocess.run(list(map(str, command)), check=True, **kwargs)


def text(*command):
    return subprocess.check_output(list(map(str, command)), text=True, cwd=ROOT)


def metadata(*, dependencies=False):
    args = ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"]
    if dependencies:
        args += ["--features", "editor", "--filter-platform", "aarch64-apple-darwin"]
    else:
        args += ["--no-deps"]
    return json.loads(text(*args))


def package_metadata(data):
    return next(p for p in data["packages"] if Path(p["manifest_path"]) == ROOT / "Cargo.toml")


def dependencies(output):
    """otool's annotation is separated from the install name, which may contain spaces."""
    return [line.strip().rsplit(" (", 1)[0] for line in output.splitlines()[1:] if line.strip()]


def rpaths(output):
    result = []
    is_rpath = False
    for line in output.splitlines():
        words = line.strip().split()
        if words[:1] == ["cmd"]:
            is_rpath = words == ["cmd", "LC_RPATH"]
        elif is_rpath and line.strip().startswith("path "):
            result.append(line.strip()[5:].rsplit(" (offset ", 1)[0])
            is_rpath = False
    return result


def system_library(name):
    return posixpath.normpath(name).startswith(("/System/Library/", "/usr/lib/"))


def audit_dependencies(names, *, bundled):
    for name in names:
        if system_library(name):
            continue
        if name == "@rpath/libomp.dylib" and bundled:
            continue
        raise RuntimeError(f"Unexpected non-system dependency: {name}")


def remove_rpaths(binary):
    for path in rpaths(text("otool", "-l", binary)):
        run("install_name_tool", "-delete_rpath", path, binary)


def copy_notice(source, destination, name=None):
    source = ROOT / source
    target = destination / (name or source.name)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)


def collect_rust_notices(package, destination, supplements):
    directory = Path(package["manifest_path"]).parent
    paths = set()
    for path in directory.rglob("*"):
        name = path.name.lower()
        if path.is_file() and path.suffix.lower() not in (
            ".rs",
            ".c",
            ".cc",
            ".cpp",
            ".h",
            ".hpp",
            ".py",
        ):
            if (
                "license" in name
                or name.startswith(("copying", "copyright", "notice"))
                or name in ("ofl.txt", "ufl.txt", "hack-regular.txt")
            ):
                paths.add(path)
    if package.get("license_file"):
        paths.add(directory / package["license_file"])
    files = []
    for path in sorted(paths):
        relative = path.relative_to(directory)
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)
        files.append(str(relative))
    key = f"{package['name']}@{package['version']}"
    supplement = next((item for item in supplements if key in item["packages"]), None)
    if supplement:
        destination.mkdir(parents=True, exist_ok=True)
        filename = "LICENSE-MIT-supplemental.txt"
        (destination / filename).write_text(
            supplement["copyright"] + "\n\n" + (ROOT / "licenses/MIT-permission.txt").read_text()
        )
        (destination / "notice-provenance.json").write_text(json.dumps(supplement, indent=2) + "\n")
        files += [filename, "notice-provenance.json"]
    return sorted(files)


def write_notices(resources):
    notices = resources / "Licenses"
    notices.mkdir()
    for path in [
        "LICENSE",
        "NOTICE.md",
        "assets/fonts/OFL.txt",
        "assets/ocio/LICENSE-OpenColorIO.txt",
        "licenses/BSL-1.0.txt",
    ]:
        copy_notice(path, notices)
    sdk = "vendor/dng-sdk/"
    for path in [
        "LICENSE.txt",
        "libjpeg/README",
        "libjxl/libjxl/LICENSE",
        "libjxl/libjxl/third_party/skcms/LICENSE",
        "libjxl/libjxl/third_party/highway/LICENSE",
        "libjxl/libjxl/third_party/highway/LICENSE-BSD3",
        "libjxl/libjxl/third_party/brotli/LICENSE",
        "xmp/toolkit/XMPCore/third-party/expat/public/lib/expat.h",
        "xmp/toolkit/XMPCore/third-party/uuid/source/COPYING",
        "xmp/toolkit/third-party/zlib/zlib.h",
        "xmp/toolkit/third-party/zuid/sources/MD5.cpp",
        "xmp/toolkit/third-party/zuid/sources/ZUIDSysDep.cpp",
    ]:
        copy_notice(sdk + path, notices / "DNG-SDK", path)
    copy_notice("vendor/dng-sdk/PROVENANCE.md", notices / "DNG-SDK")
    raw = Path(os.environ.get("LIBRAW_SOURCE", ROOT / ".cache/native-deps/LibRaw-0.22.0-openmp"))
    for name in ["COPYRIGHT", "LICENSE.LGPL", "LICENSE.CDDL"]:
        copy_notice(raw / name, notices / "LibRaw")
    copy_notice(
        ".cache/native-deps/openmp-22.1.8-install/share/licenses/LLVM-OpenMP.txt",
        notices / "OpenMP",
    )

    # Include the selected production dependency graph's declared licenses and
    # shipped notice files. This is an inventory, not automated legal clearance.
    data = metadata(dependencies=True)
    nodes = {n["id"]: n for n in data["resolve"]["nodes"]}
    selected = set()
    pending = [package_metadata(data)["id"]]
    while pending:
        package_id = pending.pop()
        if package_id in selected:
            continue
        selected.add(package_id)
        pending.extend(d["pkg"] for d in nodes[package_id]["deps"])
    inventory = []
    supplements = json.loads((ROOT / "licenses/rust-notices.json").read_text())
    for package in sorted(data["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] not in selected or package["name"] == "antitone":
            continue
        target = notices / "Rust" / f"{package['name']}-{package['version']}"
        files = collect_rust_notices(package, target, supplements)
        if not files:
            print(
                f"WARNING: unresolved upstream notice files: {package['name']} {package['version']}; local test package only",
                file=sys.stderr,
            )
        inventory.append(
            {
                "name": package["name"],
                "version": package["version"],
                "license": package["license"],
                "repository": package["repository"],
                "source": package["source"],
                "notice_files": files,
                "notice_status": "collected; review required"
                if files
                else "unresolved: upstream notice files missing",
                "authors": package["authors"],
            }
        )
    (notices / "rust-dependencies.json").write_text(json.dumps(inventory, indent=2) + "\n")


def verify_bundle(bundle, version):
    binary = bundle / "Contents/MacOS/antitone"
    runtime = bundle / "Contents/Frameworks/libomp.dylib"
    if not runtime.is_file():
        raise RuntimeError("Packaged OpenMP runtime is missing")
    for path in [binary, runtime]:
        audit_dependencies(dependencies(text("otool", "-L", path)), bundled=True)
        allowed = ["@executable_path/../Frameworks"] if path == binary else []
        if rpaths(text("otool", "-l", path)) != allowed:
            raise RuntimeError(f"Unexpected runtime search paths in {path}")
    run("codesign", "--verify", "--deep", "--strict", bundle)
    with tempfile.TemporaryDirectory(prefix="antitone-package-home-") as home:
        # No developer/Cargo paths or inherited DYLD overrides can rescue a bad bundle.
        environment = {
            "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
            "HOME": home,
            "TMPDIR": home,
            "DYLD_PRINT_LIBRARIES": "1",
        }
        result = run(
            binary,
            "--version",
            cwd=home,
            env=environment,
            capture_output=True,
            text=True,
            timeout=30,
        )
        if result.stdout.strip() != f"Antitone {version}":
            raise RuntimeError(f"Unexpected packaged executable response: {result.stdout!r}")
        if str(runtime) not in result.stderr:
            raise RuntimeError("Loader did not report the bundled OpenMP runtime")


def verify_archive(version):
    # Relocate outside the checkout, including when its path contains spaces.
    with tempfile.TemporaryDirectory(prefix="Antitone package ", dir="/private/tmp") as temporary:
        run("ditto", "-x", "-k", ARCHIVE, temporary)
        verify_bundle(Path(temporary) / "Antitone.app", version)
    print(f"Verified relocated package: {ARCHIVE}")


def package(data):
    item = package_metadata(data)
    binary = Path(data["target_directory"]) / "aarch64-apple-darwin/release/antitone"
    if not binary.is_file():
        raise RuntimeError("Build the production editor first: cargo xtask package")
    if text("lipo", "-archs", binary).strip() != "arm64":
        raise RuntimeError("Expected the Apple Silicon production executable")
    OUTPUT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="staging-", dir=OUTPUT) as temporary:
        bundle = Path(temporary) / "Antitone.app"
        contents = bundle / "Contents"
        executable = contents / "MacOS/antitone"
        runtime = contents / "Frameworks/libomp.dylib"
        resources = contents / "Resources"
        for directory in [executable.parent, runtime.parent, resources]:
            directory.mkdir(parents=True, exist_ok=True)
        shutil.copy2(binary, executable)
        shutil.copy2(OPENMP, runtime)
        shutil.copy2(ROOT / "assets/branding/icon.icns", resources / "Antitone.icns")
        for name in dependencies(text("otool", "-L", executable)):
            if system_library(name):
                continue
            if name not in ("@rpath/libomp.dylib", str(OPENMP), str(OPENMP.resolve())):
                raise RuntimeError(f"Production binary has an unexpected dependency: {name}")
            if name != "@rpath/libomp.dylib":
                run("install_name_tool", "-change", name, "@rpath/libomp.dylib", executable)
        remove_rpaths(executable)
        remove_rpaths(runtime)
        run("install_name_tool", "-add_rpath", "@executable_path/../Frameworks", executable)
        run("install_name_tool", "-id", "@rpath/libomp.dylib", runtime)
        info = {
            "CFBundleDevelopmentRegion": "en",
            "CFBundleDisplayName": "Antitone",
            "CFBundleExecutable": "antitone",
            "CFBundleIconFile": "Antitone.icns",
            "CFBundleIdentifier": "local.antitone.editor",
            "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundleName": "Antitone",
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": item["version"],
            "CFBundleVersion": item["version"],
            "LSMinimumSystemVersion": "14.0",
            "NSHighResolutionCapable": True,
        }
        with (contents / "Info.plist").open("wb") as output:
            plistlib.dump(info, output)
        write_notices(resources)
        provenance = {
            "version": item["version"],
            "source_commit": text("git", "rev-parse", "HEAD").strip(),
            "source_status": text("git", "status", "--porcelain").splitlines(),
            "binary_before_packaging_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "cargo_lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(),
            "rustc": text("rustc", "-vV"),
            "clang": text("xcrun", "clang", "--version"),
            "sdk": text("xcrun", "--show-sdk-version").strip(),
            "dng_source_manifest_sha256": hashlib.sha256(
                (ROOT / "vendor/dng-sdk/SHA256SUMS").read_bytes()
            ).hexdigest(),
            "libraw_source_override": os.environ.get("LIBRAW_SOURCE"),
            "cmake": text("cmake", "--version"),
            "ninja": text("ninja", "--version").strip(),
            "build": "cargo build --locked --release --target aarch64-apple-darwin --features editor --bin antitone",
            "distribution": "local test package; ad-hoc signed, not Developer ID signed or notarized",
        }
        (resources / "build-info.json").write_text(json.dumps(provenance, indent=2) + "\n")
        for path in [runtime, bundle]:
            run("codesign", "--force", "--sign", "-", "--timestamp=none", path)
        verify_bundle(bundle, item["version"])
        if APP.exists():
            shutil.rmtree(APP)
        bundle.rename(APP)
    staged_archive = OUTPUT / "Antitone-macos-arm64.zip.tmp"
    if staged_archive.exists():
        staged_archive.unlink()
    run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", APP, staged_archive)
    staged_archive.replace(ARCHIVE)
    (OUTPUT / "SHA256SUMS").write_text(
        f"{hashlib.sha256(ARCHIVE.read_bytes()).hexdigest()}  {ARCHIVE.name}\n"
    )
    print(APP)
    print(ARCHIVE)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--verify", action="store_true", help="verify the existing ZIP outside the checkout"
    )
    args = parser.parse_args()
    if sys.platform != "darwin":
        raise RuntimeError("This packaging command requires macOS")
    data = metadata()
    if args.verify:
        verify_archive(package_metadata(data)["version"])
    else:
        package(data)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, subprocess.SubprocessError) as error:
        raise SystemExit(str(error)) from error
