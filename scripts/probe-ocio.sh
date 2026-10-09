#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p .cache/probes
prefix="$PWD/.cache/deps/ocio-install"
/usr/bin/clang++ -std=c++17 -O2 native/ocio_probe.cpp \
    -I"$prefix/include" -L"$prefix/lib" -lOpenColorIO \
    -Wl,-rpath,"$prefix/lib" -o .cache/probes/ocio-probe
.cache/probes/ocio-probe
./scripts/cargo.sh run --features gpu --bin antitone-gpu-probe
