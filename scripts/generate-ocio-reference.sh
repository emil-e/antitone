#!/bin/sh
# Regenerate upstream OCIO CPU/GLSL/LUT references and gate finite/neutral/basis agreement.
# Requires pinned OCIO (OCIO_PREFIX or .cache/deps/ocio-install) and Apple Clang, not a GPU device.
# Writes upstream-generated artifacts to .cache/probes.
# These numerical references do not establish photographic or physical-display acceptance.
set -eu

cd "$(dirname "$0")/.."

if [ "$#" -ne 0 ]; then
    echo "Usage: $0" >&2
    exit 1
fi

mkdir -p .cache/probes
prefix="${OCIO_PREFIX:-$PWD/.cache/deps/ocio-install}"
/usr/bin/clang++ -std=c++17 -O2 native/ocio_probe.cpp \
    -I"$prefix/include" -L"$prefix/lib" -lOpenColorIO \
    -Wl,-rpath,"$prefix/lib" -o .cache/probes/ocio-probe
.cache/probes/ocio-probe
