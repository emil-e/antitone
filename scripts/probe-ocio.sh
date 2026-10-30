#!/bin/sh
# Generate upstream OCIO CPU/GLSL/LUT references, then gate their Metal agreement.
# Requires pinned OCIO (OCIO_PREFIX or .cache/deps/ocio-install), Cargo and Metal.
# Writes upstream-generated artifacts to .cache/probes.
# Numerical failures propagate; this does not establish photographic or display acceptance.
set -eu

cd "$(dirname "$0")/.."

./scripts/generate-ocio-reference.sh "$@"
cargo run --locked --features gpu --bin antitone-gpu-probe -- "$@"
