"""Build and verify the Lightroom plug-in shipped with the macOS application."""

import hashlib
import json
from pathlib import Path
import re
import zipfile

ARCHIVE_NAME = "Antitone-Lightroom.zip"
BUNDLE = "Antitone.lrplugin"
FILES = (
    "Bridge.lua",
    "Config.lua",
    "Edit.lua",
    "Info.lua",
    "Metadata.lua",
    "PluginInfo.lua",
    "Projects.lua",
    "Protocol.lua",
    "Run.lua",
    "json.lua",
)


def version_table(version: str) -> str:
    major, minor, patch = map(int, version.split("-", 1)[0].split("+", 1)[0].split("."))
    return f"VERSION = {{ major = {major}, minor = {minor}, revision = {patch} }}"


def package_plugin(root: Path, output: Path, version: str, protocol: int) -> Path:
    source = root / "integrations/lightroom" / BUNDLE
    payload = {name: (source / name).read_bytes() for name in FILES}
    declared = re.fullmatch(r"\s*return\s+(\d+)\s*", payload["Protocol.lua"].decode())
    if not declared or int(declared[1]) != protocol:
        raise ValueError("Plug-in transport protocol does not match application")
    info, count = re.subn(
        r"VERSION\s*=\s*\{[^}]+\}", version_table(version), payload["Info.lua"].decode()
    )
    if count != 1:
        raise ValueError("Expected one plug-in version table")
    payload["Info.lua"] = info.encode()
    payload["LICENSE"] = (root / "LICENSE").read_bytes()
    manifest = {
        "version": version,
        "protocol": protocol,
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in payload.items()},
    }
    payload["package.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    output.mkdir(parents=True, exist_ok=True)
    archive = output / ARCHIVE_NAME
    temporary = archive.with_suffix(".zip.tmp")
    try:
        with zipfile.ZipFile(temporary, "w", zipfile.ZIP_DEFLATED) as stream:
            for name, data in payload.items():
                stream.writestr(f"{BUNDLE}/{name}", data)
        verify_plugin(temporary, version, protocol)
        temporary.replace(archive)
    finally:
        temporary.unlink(missing_ok=True)
    return archive


def verify_plugin(path: Path, version: str, protocol: int) -> None:
    with zipfile.ZipFile(path) as archive:
        expected = {f"{BUNDLE}/{name}" for name in (*FILES, "LICENSE", "package.json")}
        if set(archive.namelist()) != expected or len(archive.namelist()) != len(expected):
            raise ValueError("Unexpected or missing plug-in files")
        manifest = json.loads(archive.read(f"{BUNDLE}/package.json"))
        if manifest["version"] != version or manifest["protocol"] != protocol:
            raise ValueError("Plug-in and application versions do not match")
        if set(manifest["sha256"]) != {*FILES, "LICENSE"}:
            raise ValueError("Incomplete plug-in checksum inventory")
        for name, digest in manifest["sha256"].items():
            if hashlib.sha256(archive.read(f"{BUNDLE}/{name}")).hexdigest() != digest:
                raise ValueError(f"Plug-in checksum mismatch: {name}")
        declared = re.fullmatch(
            r"\s*return\s+(\d+)\s*", archive.read(f"{BUNDLE}/Protocol.lua").decode()
        )
        if not declared or int(declared[1]) != protocol:
            raise ValueError("Plug-in transport protocol does not match application")
        if version_table(version) not in archive.read(f"{BUNDLE}/Info.lua").decode():
            raise ValueError("Incorrect Plug-in Manager version")
