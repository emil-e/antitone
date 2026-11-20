#!/bin/sh
set -eu

cd "$(dirname "$0")/.."

cache=.cache/native-deps
pins=native/onnxruntime/pins.txt
version=$(awk -F= '$1 == "version" {print $2}' "$pins")
hash=$(awk -F= '$1 == "nuget_sha256" {print $2}' "$pins")
package="microsoft.ml.onnxruntime"
name="$package.$version.nupkg"
archive="$cache/$name"
prefix="$cache/onnxruntime-$version"
mkdir -p "$cache"

if [ ! -f "$archive" ]; then
    curl --fail --location "https://api.nuget.org/v3-flatcontainer/$package/$version/$name" -o "$archive.tmp"
    mv "$archive.tmp" "$archive"
fi
printf '%s  %s\n' "$hash" "$archive" | shasum -a 256 -c -
if [ -f "$prefix/libonnxruntime.dylib" ] && [ -f "$prefix/LICENSE" ] && [ -f "$prefix/ThirdPartyNotices.txt" ]; then
    exit 0
fi

stage=$(mktemp -d "$cache/.onnxruntime-install.XXXXXX")
trap 'rm -rf "$stage"' EXIT
tar -xf "$archive" -C "$stage" runtimes/osx-arm64/native/libonnxruntime.dylib \
    LICENSE ThirdPartyNotices.txt
mkdir -p "$prefix"
mv "$stage/LICENSE" "$stage/ThirdPartyNotices.txt" "$prefix/"
mv "$stage/runtimes/osx-arm64/native/libonnxruntime.dylib" "$prefix/libonnxruntime.dylib"
