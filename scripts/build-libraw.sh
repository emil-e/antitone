#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
./scripts/build-openmp.sh

cache=.cache/native-deps
archive="$cache/LibRaw-0.22.0.tar.gz"
source="$cache/LibRaw-0.22.0-openmp"
openmp="$PWD/$cache/openmp-22.1.8-install"
mkdir -p "$cache"

if [ ! -f "$archive" ]; then
    if [ -f .cache/deps/LibRaw-0.22.0.tar.gz ]; then
        cp .cache/deps/LibRaw-0.22.0.tar.gz "$archive"
    else
        curl --fail --location https://codeload.github.com/LibRaw/LibRaw/tar.gz/refs/tags/0.22.0 -o "$archive.tmp"
        mv "$archive.tmp" "$archive"
    fi
fi
printf '%s  %s\n' 5a11327a9cef2496d6a4335e8da30a1604460b6c545a30fe7588cf4c00a0fcae "$archive" | shasum -a 256 -c -
if [ ! -d "$source" ]; then
    mkdir -p "$source"
    tar -xf "$archive" -C "$source" --strip-components=1
fi

make -C "$source" -f Makefile.devel -j4 CC=/usr/bin/clang CXX=/usr/bin/clang++ \
    CFLAGS="-DLIBRAW_USE_AUTOPTR -DUSE_ZLIB -g -I. -pedantic -Wno-long-long -Wno-overflow -O3 -Wall -mmacosx-version-min=14.0 -Xclang -fopenmp -I$openmp/include" \
    lib/libraw_r.a
