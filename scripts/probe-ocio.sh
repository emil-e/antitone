#!/bin/sh
# Generate upstream OCIO CPU/GLSL/LUT references, then gate their Metal agreement.
# Requires pinned OCIO (OCIO_PREFIX or .cache/deps/ocio-install), Cargo and Metal; --camera also needs LibRaw.
# Writes .cache/probes artifacts; --camera regenerates authored RAW inputs for the optional extension.
# Numerical failures propagate; this does not establish photographic or display acceptance.
set -eu

cd "$(dirname "$0")/.."

./scripts/generate-ocio-reference.sh "$@"
cargo run --locked --features gpu --bin antitone-gpu-probe -- "$@"
