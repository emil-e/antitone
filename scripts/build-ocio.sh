#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cache=.cache/deps
archive="$cache/OpenColorIO-2.6.0.tar.gz"
mkdir -p "$cache"
if [ ! -f "$archive" ]; then
    curl --fail --location https://codeload.github.com/AcademySoftwareFoundation/OpenColorIO/tar.gz/refs/tags/v2.6.0 -o "$archive"
fi
printf '%s  %s\n' 784ac37bde5b9c6e5dc15f23d1235bd1452d376cdc03627d6d6dd310f375a735 "$archive" | shasum -a 256 -c -
if [ ! -d "$cache/OpenColorIO-2.6.0" ]; then
    tar -xzf "$archive" -C "$cache"
fi
# Scope compiler selection to this build and its CMake ExternalProject children.
export CC=/usr/bin/clang CXX=/usr/bin/clang++
cmake -S "$cache/OpenColorIO-2.6.0" -B "$cache/ocio-build" -G Ninja \
    -DCMAKE_C_COMPILER="$CC" -DCMAKE_CXX_COMPILER="$CXX" \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PWD/$cache/ocio-install" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0 \
    -DOCIO_BUILD_APPS=OFF -DOCIO_BUILD_TESTS=OFF -DOCIO_BUILD_GPU_TESTS=OFF \
    -DOCIO_BUILD_PYTHON=OFF -DOCIO_BUILD_DOCS=OFF -DOCIO_INSTALL_EXT_PACKAGES=ALL
cmake --build "$cache/ocio-build" -j4
cmake --install "$cache/ocio-build"
