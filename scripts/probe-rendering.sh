#!/bin/sh
# Gate production CPU/OCIO agreement and independent Core Image/ColorSync ICC conversion.
# Requires pinned OCIO and macOS frameworks, not Metal; --camera additionally needs LibRaw/Cargo GPU types.
# Regenerates OCIO/profile fixtures and reference executables in .cache/probes; failures propagate.
# Does not establish photographic or physical-display acceptance.
set -eu
cd "$(dirname "$0")/.."
./scripts/generate-ocio-reference.sh "$@"
cargo run --locked --offline --features render --bin antitone-rendering-probe
/usr/bin/clang++ -std=c++17 -O2 native/profile_reference.mm \
    -framework Foundation -framework CoreGraphics -framework CoreImage \
    -o .cache/probes/profile-reference
.cache/probes/profile-reference
