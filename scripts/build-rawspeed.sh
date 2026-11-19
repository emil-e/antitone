#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cache="$PWD/.cache/native-deps"
revision=$(awk -F= '$1 == "revision" {print $2}' native/rawspeed/pins.txt)
raw_hash=$(awk -F= '$1 == "sha256" {print $2}' native/rawspeed/pins.txt)
pugi_version=$(awk -F= '$1 == "pugixml_version" {print $2}' native/rawspeed/pins.txt)
pugi_hash=$(awk -F= '$1 == "pugixml_sha256" {print $2}' native/rawspeed/pins.txt)
omp_version=$(awk -F= '$1 == "version" {print $2}' native/openmp/pins.txt)
openmp="$cache/openmp-$omp_version-install"
prefix="$cache/rawspeed-$(printf '%.12s' "$revision")-macos-arm64"
source="$cache/rawspeed-$revision"
pugi="$cache/pugixml-$pugi_version"
mkdir -p "$cache"

prepare() {
    archive=$1; url=$2; hash=$3; destination=$4
    if [ ! -f "$archive" ]; then
        curl --fail --location "$url" -o "$archive.tmp"
        mv "$archive.tmp" "$archive"
    fi
    printf '%s  %s\n' "$hash" "$archive" | shasum -a 256 -c -
    if [ ! -d "$destination" ]; then
        mkdir -p "$destination"
        tar -xf "$archive" -C "$destination" --strip-components=1
    fi
}
prepare "$cache/rawspeed-$revision.tar.gz" \
    "https://codeload.github.com/darktable-org/rawspeed/tar.gz/$revision" "$raw_hash" "$source"
prepare "$cache/pugixml-$pugi_version.tar.gz" \
    "https://codeload.github.com/zeux/pugixml/tar.gz/refs/tags/v$pugi_version" "$pugi_hash" "$pugi"
if [ ! -f "$openmp/include/omp.h" ] || [ ! -f "$openmp/lib/libomp.dylib" ]; then
    ./scripts/build-openmp.sh
fi
cmake -S native/rawspeed -B "$prefix/build" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
    -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0 -DCMAKE_INSTALL_PREFIX="$prefix" \
    -DRAWSPEED_SOURCE="$source" -DPUGIXML_SOURCE="$pugi" -DRAWSPEED_REVISION="$revision" \
    '-DOpenMP_CXX_FLAGS=-Xclang -fopenmp' -DOpenMP_CXX_LIB_NAMES=omp \
    -DOpenMP_omp_LIBRARY="$openmp/lib/libomp.dylib" -DOpenMP_CXX_INCLUDE_DIR="$openmp/include"
cmake --build "$prefix/build" --parallel 4
cmake --install "$prefix/build"
"$prefix/build/adapter_tests" "$PWD/tests/fixtures/raw/patches.dng"
