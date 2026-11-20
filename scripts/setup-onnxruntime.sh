#!/bin/sh
set -eu

cd "$(dirname "$0")/.."

cache=.cache/native-deps
pins=native/onnxruntime/pins.txt
version=$(awk -F= '$1 == "version" {print $2}' "$pins")
hash=$(awk -F= '$1 == "macos_arm64_sha256" {print $2}' "$pins")
release="onnxruntime-osx-arm64-$version"
archive="$cache/$release.tgz"
prefix="$cache/onnxruntime-$version"
mkdir -p "$cache"

if [ ! -f "$archive" ]; then
    curl --fail --location "https://github.com/microsoft/onnxruntime/releases/download/v$version/$release.tgz" -o "$archive.tmp"
    mv "$archive.tmp" "$archive"
fi
printf '%s  %s\n' "$hash" "$archive" | shasum -a 256 -c -
if [ -f "$prefix/libonnxruntime.dylib" ] && [ -f "$prefix/LICENSE" ] && [ -f "$prefix/ThirdPartyNotices.txt" ]; then
    exit 0
fi

stage=$(mktemp -d "$cache/.onnxruntime-install.XXXXXX")
trap 'rm -rf "$stage"' EXIT
tar -xf "$archive" -C "$stage" "./$release/lib/libonnxruntime.$version.dylib" \
    "./$release/LICENSE" "./$release/ThirdPartyNotices.txt"
mkdir -p "$prefix"
mv "$stage/$release/LICENSE" "$stage/$release/ThirdPartyNotices.txt" "$prefix/"
mv "$stage/$release/lib/libonnxruntime.$version.dylib" "$prefix/libonnxruntime.dylib"
