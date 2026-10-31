#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
cache=.cache/native-deps
archive="$cache/LibRaw-0.22.3.tar.gz"
source="$cache/LibRaw-0.22.3-openmp"
openmp="$PWD/$cache/openmp-22.1.8-install"
mkdir -p "$cache"
if [ ! -f "$openmp/include/omp.h" ] || [ ! -f "$openmp/lib/libomp.dylib" ]; then
    ./scripts/build-openmp.sh
fi

if [ ! -f "$archive" ]; then
    if [ -f .cache/deps/LibRaw-0.22.3.tar.gz ]; then
        cp .cache/deps/LibRaw-0.22.3.tar.gz "$archive"
    else
        curl --fail --location https://codeload.github.com/LibRaw/LibRaw/tar.gz/refs/tags/0.22.3 -o "$archive.tmp"
        mv "$archive.tmp" "$archive"
    fi
fi
printf '%s  %s\n' 7edf96c5b70cff88980a02ab0fc8af125cb6a4a377985573c654439d9d02355c "$archive" | shasum -a 256 -c -
if [ ! -d "$source" ]; then
    mkdir -p "$source"
    tar -xf "$archive" -C "$source" --strip-components=1
fi

make -C "$source" -f Makefile.devel -j4 CC=/usr/bin/clang CXX=/usr/bin/clang++ \
    CFLAGS="-DLIBRAW_USE_AUTOPTR -DUSE_ZLIB -g -I. -pedantic -Wno-long-long -Wno-overflow -O3 -Wall -mmacosx-version-min=14.0 -Xclang -fopenmp -I$openmp/include" \
    lib/libraw_r.a
