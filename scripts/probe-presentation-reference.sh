#!/bin/sh
# Build/run the independent Core Image signed transfer gate with macOS frameworks.
# Writes .cache/probes reference fixtures for the managed-presentation probe; errors fail.
# No desktop window is opened; numerical transfer is not physical-monitor acceptance.
set -eu
cd "$(dirname "$0")/.."
mkdir -p .cache/probes
/usr/bin/clang++ -std=c++17 -O2 -fobjc-arc native/presentation_reference.mm \
    -framework Foundation -framework CoreGraphics -framework CoreImage \
    -o .cache/probes/presentation-reference
.cache/probes/presentation-reference
