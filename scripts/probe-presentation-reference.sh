#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p .cache/probes
/usr/bin/clang++ -std=c++17 -O2 -fobjc-arc native/presentation_reference.mm \
    -framework Foundation -framework CoreGraphics -framework CoreImage \
    -o .cache/probes/presentation-reference
.cache/probes/presentation-reference
