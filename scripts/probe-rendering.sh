#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p .cache/probes
cargo run --locked --offline --features render --bin antitone-rendering-probe
/usr/bin/clang++ -std=c++17 -O2 native/profile_reference.mm \
    -framework Foundation -framework CoreGraphics -framework CoreImage \
    -o .cache/probes/profile-reference
.cache/probes/profile-reference
