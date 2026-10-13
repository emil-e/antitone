#!/bin/sh
# Gate production CPU/OCIO agreement and independent Core Image/ColorSync ICC conversion.
# Requires native rendering setup and fresh OCIO artifacts from probe-ocio.sh in .cache/probes.
# Writes profile fixtures/ICC files and a reference executable there; failures propagate.
# Does not establish photographic or physical-display acceptance.
set -eu
cd "$(dirname "$0")/.."
mkdir -p .cache/probes
cargo run --locked --offline --features render --bin antitone-rendering-probe
/usr/bin/clang++ -std=c++17 -O2 native/profile_reference.mm \
    -framework Foundation -framework CoreGraphics -framework CoreImage \
    -o .cache/probes/profile-reference
.cache/probes/profile-reference
