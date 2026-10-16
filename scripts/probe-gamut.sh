#!/bin/sh
# Build/run the broad native AP1 gamut gate with pinned OCIO; no RAW/cache inputs are needed.
# Stores the executable in .cache/probes; nonfinite/collapsed outputs fail in either SDR target.
# This is a bounded numerical check, not photographic or physical-monitor acceptance.
set -eu

cd "$(dirname "$0")/.."

mkdir -p .cache/probes
prefix="${OCIO_PREFIX:-$PWD/.cache/deps/ocio-install}"
/usr/bin/clang++ -std=c++17 -O2 native/gamut_probe.cpp \
    -I"$prefix/include" -L"$prefix/lib" -lOpenColorIO \
    -Wl,-rpath,"$prefix/lib" -o .cache/probes/gamut-probe
.cache/probes/gamut-probe "$@"
