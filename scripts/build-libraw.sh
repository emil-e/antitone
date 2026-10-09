#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cache=.cache/deps
archive="$cache/LibRaw-0.22.0.tar.gz"
mkdir -p "$cache"
if [ ! -f "$archive" ]; then
    curl --fail --location https://codeload.github.com/LibRaw/LibRaw/tar.gz/refs/tags/0.22.0 -o "$archive"
fi
printf '%s  %s\n' 5a11327a9cef2496d6a4335e8da30a1604460b6c545a30fe7588cf4c00a0fcae "$archive" | shasum -a 256 -c -
if [ ! -d "$cache/LibRaw-0.22.0" ]; then
    tar -xzf "$archive" -C "$cache"
fi
make -C "$cache/LibRaw-0.22.0" -f Makefile.devel -j4 lib/libraw_r.a
