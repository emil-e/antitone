#!/bin/sh
# Generate upstream OCIO CPU/GLSL/LUT references, then gate their Metal agreement.
# Requires pinned OCIO (OCIO_PREFIX or .cache/deps/ocio-install), Cargo and Metal; --camera also needs LibRaw.
# Writes .cache/probes artifacts; --camera regenerates authored RAW inputs for the optional extension.
# Numerical failures propagate; this does not establish photographic or display acceptance.
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
cargo run --locked --features gpu --bin antitone-gpu-probe -- "$@"
