#!/bin/sh
# Regenerate upstream OCIO CPU/GLSL/LUT references and gate finite/neutral/basis agreement.
# Requires pinned OCIO (OCIO_PREFIX or .cache/deps/ocio-install) and Apple Clang, not a GPU device.
# Writes .cache/probes; --camera regenerates authored inputs and requires LibRaw/Cargo GPU parameter types.
# These numerical references do not establish photographic or physical-display acceptance.
set -eu
cd "$(dirname "$0")/.."
case "$#:$*" in
    0:) ;;
    1:--camera) cargo run --locked --features raw,gpu --bin antitone-camera-reference ;;
    *) echo "Usage: $0 [--camera]" >&2; exit 1 ;;
esac
mkdir -p .cache/probes
prefix="${OCIO_PREFIX:-$PWD/.cache/deps/ocio-install}"
/usr/bin/clang++ -std=c++17 -O2 native/ocio_probe.cpp \
    -I"$prefix/include" -L"$prefix/lib" -lOpenColorIO \
    -Wl,-rpath,"$prefix/lib" -o .cache/probes/ocio-probe
.cache/probes/ocio-probe "$@"
