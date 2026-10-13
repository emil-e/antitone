#!/bin/sh
# Generate upstream OCIO CPU/GLSL/LUT references, then gate their Metal agreement.
# Requires pinned OCIO (OCIO_PREFIX or .cache/deps/ocio-install), Cargo and Metal.
# Writes .cache/probes artifacts; existing camera fixtures enable an optional RAW-derived extension.
# Numerical failures propagate; this does not establish photographic or display acceptance.
set -eu
cd "$(dirname "$0")/.."
mkdir -p .cache/probes
prefix="${OCIO_PREFIX:-$PWD/.cache/deps/ocio-install}"
/usr/bin/clang++ -std=c++17 -O2 native/ocio_probe.cpp \
    -I"$prefix/include" -L"$prefix/lib" -lOpenColorIO \
    -Wl,-rpath,"$prefix/lib" -o .cache/probes/ocio-probe
.cache/probes/ocio-probe
cargo run --locked --features gpu --bin antitone-gpu-probe
